use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::canvas::renderer::ShapeCanvas;
use crate::canvas::shapes::{base_shapes, Shape};
use crate::game::feedback::{AnswerFeedback, Flash};
use crate::game::theme;
use crate::game::{column_index, contains, Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "pattern_fill";

const CHOICE_COUNT: usize = 4;
const GRID_SIZE: usize = 9;

/// 描画エリアを「グリッド」「選択肢」「フッター」に分割する
fn split_areas(area: Rect) -> (Rect, Rect, Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(11),
            Constraint::Length(7),
            Constraint::Length(3),
        ])
        .split(area);
    (rows[0], rows[1], rows[2])
}

/// グリッド1マス分の内容(どの図形を何度回転させて置くか)
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cell {
    shape_index: usize,
    angle_deg: f64,
}

struct Question {
    /// 3x3グリッド(row-major)。blank_indexの位置も規則通りの値を保持しておき、
    /// 「本来ここに入るべき図形」の判定に使う(描画時のみblank_indexを隠す)
    cells: [Cell; GRID_SIZE],
    blank_index: usize,
    choices: [Cell; CHOICE_COUNT],
    correct_index: usize,
}

/// 出題の規則。マス(row, col)は「行rowの図形種類」を「列colの回転角度」で回転させたものになる
#[derive(Debug, Clone, Copy)]
struct Rules {
    /// 行の規則: 行ごとの図形種類(3行とも別の図形)
    row_shapes: [usize; 3],
    /// 列の規則: 列ごとの回転角度(度)
    col_angles: [f64; 3],
}

impl Rules {
    fn cell(&self, row: usize, col: usize) -> Cell {
        Cell {
            shape_index: self.row_shapes[row],
            angle_deg: self.col_angles[col],
        }
    }

    fn cells(&self) -> [Cell; GRID_SIZE] {
        std::array::from_fn(|i| self.cell(i / 3, i % 3))
    }
}

/// 重複しない図形種類を3つ選ぶ(行の規則用)
fn pick_row_shapes(rng: &mut impl Rng, shape_count: usize) -> [usize; 3] {
    let mut indices: Vec<usize> = (0..shape_count).collect();
    indices.shuffle(rng);
    [indices[0], indices[1], indices[2]]
}

/// 基準角度から一定刻みで増える3列分の角度を作る(列の規則用)。
/// 刻みの最大値×2が180度未満なので、点対称な図形でも列同士の見た目が重ならない
fn pick_col_angles(rng: &mut impl Rng, step_pool: &[f64], base_pool: &[f64]) -> [f64; 3] {
    let step = step_pool[rng.gen_range(0..step_pool.len())];
    let base = base_pool[rng.gen_range(0..base_pool.len())];
    std::array::from_fn(|col| normalize_angle(base + col as f64 * step))
}

/// Beginner: 行の規則(図形種類)のみ。列は全て0度で1軸だけの出題
fn generate_beginner(rng: &mut impl Rng, shapes: &[Shape]) -> Rules {
    Rules {
        row_shapes: pick_row_shapes(rng, shapes.len()),
        col_angles: [0.0; 3],
    }
}

/// Intermediate: 行の規則(図形種類)+列の規則(45度か60度刻みの回転)
fn generate_intermediate(rng: &mut impl Rng, shapes: &[Shape]) -> Rules {
    const STEP_POOL_DEG: [f64; 2] = [45.0, 60.0];
    const BASE_POOL_DEG: [f64; 4] = [0.0, 90.0, 180.0, 270.0];
    Rules {
        row_shapes: pick_row_shapes(rng, shapes.len()),
        col_angles: pick_col_angles(rng, &STEP_POOL_DEG, &BASE_POOL_DEG),
    }
}

/// Advanced: 行の規則+列の規則。回転の刻みを20度か30度に狭め、基準角度も中途半端にして見分けにくくする
fn generate_advanced(rng: &mut impl Rng, shapes: &[Shape]) -> Rules {
    const STEP_POOL_DEG: [f64; 2] = [20.0, 30.0];
    let base_pool: Vec<f64> = (0..24).map(|i| i as f64 * 15.0).collect();
    Rules {
        row_shapes: pick_row_shapes(rng, shapes.len()),
        col_angles: pick_col_angles(rng, &STEP_POOL_DEG, &base_pool),
    }
}

fn normalize_angle(angle_deg: f64) -> f64 {
    let mut a = angle_deg % 360.0;
    if a < 0.0 {
        a += 360.0;
    }
    a
}

/// 2つのマスが画面上で同じ見た目になるか。
/// 回転後の頂点集合で比べるので、点対称な図形の180度差のような見た目の一致も同一とみなす
fn looks_same(shapes: &[Shape], a: Cell, b: Cell) -> bool {
    const EPS: f64 = 1e-6;
    let pa = shapes[a.shape_index].rotated(a.angle_deg.to_radians()).points;
    let pb = shapes[b.shape_index].rotated(b.angle_deg.to_radians()).points;
    pa.len() == pb.len()
        && pa.iter().all(|&(x, y)| {
            pb.iter()
                .any(|&(u, v)| (x - u).abs() < EPS && (y - v).abs() < EPS)
        })
}

/// 候補の中から、既存の選択肢と見た目が被らないものを1つランダムに選んで追加する
fn push_distinct(
    rng: &mut impl Rng,
    shapes: &[Shape],
    chosen: &mut Vec<Cell>,
    candidates: &[Cell],
) -> bool {
    let mut pool: Vec<Cell> = candidates
        .iter()
        .copied()
        .filter(|c| !chosen.iter().any(|x| looks_same(shapes, *x, *c)))
        .collect();
    pool.shuffle(rng);
    match pool.first() {
        Some(&c) => {
            chosen.push(c);
            true
        }
        None => false,
    }
}

/// 誤答3つを作る。chosenの先頭は正解
fn build_distractors(
    rng: &mut impl Rng,
    shapes: &[Shape],
    difficulty: Difficulty,
    rules: &Rules,
    row: usize,
    col: usize,
) -> Vec<Cell> {
    let answer = rules.cell(row, col);
    let mut chosen = vec![answer];
    let other_rows: Vec<usize> = (0..3).filter(|&r| r != row).collect();
    let other_cols: Vec<usize> = (0..3).filter(|&c| c != col).collect();

    // 行の規則(図形種類)だけ正しい: 同じ図形を別の列の角度で
    let shape_only: Vec<Cell> = other_cols.iter().map(|&c| rules.cell(row, c)).collect();
    // 列の規則(回転角度)だけ正しい: 別の行の図形を同じ角度で
    let angle_only: Vec<Cell> = other_rows.iter().map(|&r| rules.cell(r, col)).collect();
    // 両方とも違う(グリッド上に見えている別のマス)
    let both_wrong: Vec<Cell> = other_rows
        .iter()
        .flat_map(|&r| other_cols.iter().map(move |&c| (r, c)))
        .map(|(r, c)| rules.cell(r, c))
        .collect();

    match difficulty {
        Difficulty::Beginner => {
            // 角度は全て0度なので、グリッドの他の行の図形+グリッドに無い図形で埋める
            let others: Vec<Cell> = rules
                .row_shapes
                .iter()
                .enumerate()
                .filter(|&(r, _)| r != row)
                .map(|(_, &s)| Cell {
                    shape_index: s,
                    angle_deg: 0.0,
                })
                .collect();
            for c in others {
                chosen.push(c);
            }
        }
        Difficulty::Intermediate => {
            push_distinct(rng, shapes, &mut chosen, &shape_only);
            push_distinct(rng, shapes, &mut chosen, &angle_only);
            let rest: Vec<Cell> = shape_only
                .iter()
                .chain(&angle_only)
                .chain(&both_wrong)
                .copied()
                .collect();
            push_distinct(rng, shapes, &mut chosen, &rest);
        }
        Difficulty::Advanced => {
            push_distinct(rng, shapes, &mut chosen, &shape_only);
            push_distinct(rng, shapes, &mut chosen, &angle_only);
            // 同じ図形で、列の角度の並びを1歩はみ出した(グリッドに無い)角度。
            // 角度の刻みが狭いので、どの列の角度かを正確に見ないと区別できない
            let step = normalize_angle(rules.col_angles[1] - rules.col_angles[0]);
            let off_grid: Vec<Cell> = [-step, 3.0 * step]
                .iter()
                .map(|&d| Cell {
                    shape_index: answer.shape_index,
                    angle_deg: normalize_angle(rules.col_angles[0] + d),
                })
                .collect();
            if !push_distinct(rng, shapes, &mut chosen, &off_grid) {
                let rest: Vec<Cell> = shape_only.iter().chain(&both_wrong).copied().collect();
                push_distinct(rng, shapes, &mut chosen, &rest);
            }
        }
    }

    // 候補不足時のフォールバック: グリッドに登場しない図形を正解と同じ角度で足す
    let mut unused: Vec<usize> = (0..shapes.len())
        .filter(|s| !rules.row_shapes.contains(s))
        .collect();
    unused.shuffle(rng);
    for s in unused {
        if chosen.len() >= CHOICE_COUNT {
            break;
        }
        let c = Cell {
            shape_index: s,
            angle_deg: answer.angle_deg,
        };
        if !chosen.iter().any(|x| looks_same(shapes, *x, c)) {
            chosen.push(c);
        }
    }
    chosen.truncate(CHOICE_COUNT);
    chosen.split_off(1)
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let shapes = base_shapes();
    let blank_index = rng.gen_range(0..GRID_SIZE);
    let (row, col) = (blank_index / 3, blank_index % 3);

    let rules = match difficulty {
        Difficulty::Beginner => generate_beginner(rng, &shapes),
        Difficulty::Intermediate => generate_intermediate(rng, &shapes),
        Difficulty::Advanced => generate_advanced(rng, &shapes),
    };
    let cells = rules.cells();
    let correct_cell = cells[blank_index];
    let distractors = build_distractors(rng, &shapes, difficulty, &rules, row, col);

    // (Cell, is_correct) のペアで作ってからシャッフルし、最後にcorrect_indexを特定する
    let mut tagged: Vec<(Cell, bool)> = vec![(correct_cell, true)];
    tagged.extend(distractors.into_iter().map(|c| (c, false)));
    tagged.shuffle(rng);
    let correct_index = tagged.iter().position(|(_, is_correct)| *is_correct).unwrap();
    let choices: [Cell; CHOICE_COUNT] = tagged
        .into_iter()
        .map(|(c, _)| c)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();

    Question {
        cells,
        blank_index,
        choices,
        correct_index,
    }
}

pub struct PatternFillGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
    /// 直前の回答の正誤表示(描画専用)
    feedback: AnswerFeedback,
    /// グリッドの各マス用(空欄マスの回はそのマスに対応するものは使わない)
    grid_canvases: [ShapeCanvas; GRID_SIZE],
    choice_canvases: [ShapeCanvas; CHOICE_COUNT],
}

impl PatternFillGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            current: generate_question(&mut rng, difficulty),
            question_started_at: Instant::now(),
            feedback: AnswerFeedback::new(),
            grid_canvases: std::array::from_fn(|_| ShapeCanvas::new()),
            choice_canvases: std::array::from_fn(|_| ShapeCanvas::new()),
        }
    }

    fn advance_question(&mut self, answered_index: usize) {
        let is_correct = answered_index == self.current.correct_index;
        let latency_ms = self.question_started_at.elapsed().as_millis() as f64;
        self.tracker.record(is_correct, latency_ms);
        self.feedback.record(
            is_correct,
            format!("こたえ: {}番", self.current.correct_index + 1),
        );
        audio::play_se(if is_correct {
            SeKind::Correct
        } else {
            SeKind::Incorrect
        });
        if !self.tracker.is_session_finished() {
            let mut rng = rand::thread_rng();
            self.current = generate_question(&mut rng, self.difficulty);
            self.question_started_at = Instant::now();
        }
    }
}

impl Game for PatternFillGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        if let KeyCode::Char(c @ '1'..='4') = key.code {
            let index = c.to_digit(10).unwrap() as usize - 1;
            self.advance_question(index);
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        if self.tracker.is_session_finished() {
            return;
        }
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let (_, choices_area, _) = split_areas(area);
        if !contains(choices_area, mouse.column, mouse.row) {
            return;
        }
        if let Some(index) = column_index(choices_area, mouse.column, CHOICE_COUNT as u16) {
            self.advance_question(index);
        }
    }

    fn update(&mut self, dt: Duration) {
        self.feedback.tick(dt);
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (grid_area, choices_area, footer_area) = split_areas(area);
        // HUDはクリック判定の無いグリッドエリアの上端から切り出す(選択肢の位置は変えない)
        let (hud_area, grid_area) = theme::split_hud(grid_area);
        theme::render_hud(
            frame,
            hud_area,
            "パターン補完",
            self.difficulty,
            self.tracker.total(),
            &self.feedback,
        );

        draw_grid(
            frame,
            grid_area,
            &self.grid_canvases,
            &self.current,
            self.feedback.current(),
        );

        // 選択肢はクリック判定(column_index)と同じ4等分の帯に描く
        let shapes = base_shapes();
        for (i, col_area) in theme::column_bands(choices_area, CHOICE_COUNT as u16)
            .into_iter()
            .enumerate()
        {
            let cell = self.current.choices[i];
            let shape = shapes[cell.shape_index].rotated(cell.angle_deg.to_radians());
            draw_choice(frame, col_area, &self.choice_canvases[i], i + 1, &shape);
        }

        theme::render_hint_footer(
            frame,
            footer_area,
            &[("1〜4", "？に当てはまる図形を回答"), ("q", "終了")],
        );
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

/// グリッドを3x3個の個別Rectに分割する(row-major、draw_gridと同じ分割をテストでも使う)
fn grid_cell_areas(inner: Rect) -> [Rect; GRID_SIZE] {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 3); 3])
        .split(inner);
    let mut cells = [Rect::default(); GRID_SIZE];
    for (row, row_area) in rows.iter().enumerate() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 3); 3])
            .split(*row_area);
        for (col, col_area) in cols.iter().enumerate() {
            cells[row * 3 + col] = *col_area;
        }
    }
    cells
}

fn draw_grid(
    frame: &mut Frame,
    area: Rect,
    canvases: &[ShapeCanvas; GRID_SIZE],
    question: &Question,
    flash: Option<&Flash>,
) {
    let shapes = base_shapes();
    let block = theme::focus_panel(" この規則に当てはまる図形は？ ", flash);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    for (i, cell_area) in grid_cell_areas(inner).into_iter().enumerate() {
        if i == question.blank_index {
            draw_blank_cell(frame, cell_area);
        } else {
            let cell = question.cells[i];
            let shape = shapes[cell.shape_index].rotated(cell.angle_deg.to_radians());
            canvases[i].render(
                frame,
                cell_area,
                Block::default(),
                &shape,
                ([-1.0, 1.0], [-1.0, 1.0]),
                theme::ACCENT_STRONG,
            );
        }
    }
}

/// 空欄マスは画像化せず、枠と「?」のテキストのみで描く
/// (画像プロトコルの上にテキストを重ね書きすると端末依存で表示が崩れる恐れがあるため)
fn draw_blank_cell(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::HIGHLIGHT));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = Paragraph::new(Span::styled(
        "?",
        Style::default()
            .fg(theme::HIGHLIGHT)
            .add_modifier(Modifier::BOLD),
    ))
    .alignment(Alignment::Center);
    frame.render_widget(text, theme::vertical_center(inner, 1));
}

fn draw_choice(frame: &mut Frame, area: Rect, canvas: &ShapeCanvas, number: usize, shape: &Shape) {
    // 枠の上辺に番号をキー風に出す(押すキーが一目で分かるように)
    let title = Line::from(Span::styled(
        format!(" {number} "),
        Style::default()
            .fg(Color::Black)
            .bg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    ))
    .centered();
    let block = theme::sub_panel().title(title);
    canvas.render(
        frame,
        area,
        block,
        shape,
        ([-1.0, 1.0], [-1.0, 1.0]),
        theme::ACCENT,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn grid_cell_areas_covers_the_whole_inner_area_without_overlap() {
        let inner = Rect::new(0, 0, 30, 21);
        let cells = grid_cell_areas(inner);
        // 9マスの面積の合計がinner全体の面積と一致すること(重なりも隙間も無い)
        let total_area: u32 = cells.iter().map(|c| c.width as u32 * c.height as u32).sum();
        assert_eq!(total_area, inner.width as u32 * inner.height as u32);
        // 全マスがinnerの範囲内に収まっていること
        for cell in cells {
            assert!(cell.x >= inner.x && cell.y >= inner.y);
            assert!(cell.x + cell.width <= inner.x + inner.width);
            assert!(cell.y + cell.height <= inner.y + inner.height);
        }
    }

    #[test]
    fn grid_cell_areas_orders_cells_row_major() {
        let inner = Rect::new(0, 0, 30, 21);
        let cells = grid_cell_areas(inner);
        // 0,1,2が同じ行(y座標が同じ)で、0,3,6が同じ列(x座標が同じ)であること
        assert_eq!(cells[0].y, cells[1].y);
        assert_eq!(cells[1].y, cells[2].y);
        assert_eq!(cells[0].x, cells[3].x);
        assert_eq!(cells[3].x, cells[6].x);
        assert!(cells[0].x < cells[1].x);
        assert!(cells[0].y < cells[3].y);
    }

    #[test]
    fn choices_always_have_exactly_one_correct_and_are_in_range() {
        let mut rng = StdRng::seed_from_u64(10);
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            for _ in 0..50 {
                let q = generate_question(&mut rng, difficulty);
                assert!(q.correct_index < CHOICE_COUNT);
                assert!(q.blank_index < GRID_SIZE);
            }
        }
    }

    const ALL_DIFFICULTIES: [Difficulty; 3] = [
        Difficulty::Beginner,
        Difficulty::Intermediate,
        Difficulty::Advanced,
    ];

    /// 角度が360度を法として等しいか
    fn same_angle(a: f64, b: f64) -> bool {
        let d = normalize_angle(a - b);
        d < 1e-6 || (360.0 - d) < 1e-6
    }

    /// 行ごとの図形種類(同じ行の3マスは同じ図形であることも検証する)
    fn row_shapes_of(q: &Question) -> [usize; 3] {
        std::array::from_fn(|row| {
            let expected = q.cells[row * 3].shape_index;
            for col in 0..3 {
                assert_eq!(
                    q.cells[row * 3 + col].shape_index,
                    expected,
                    "行{row}の図形種類が揃っていない"
                );
            }
            expected
        })
    }

    /// 列ごとの回転角度(同じ列の3マスは同じ角度であることも検証する)
    fn col_angles_of(q: &Question) -> [f64; 3] {
        std::array::from_fn(|col| {
            let expected = q.cells[col].angle_deg;
            for row in 0..3 {
                assert!(
                    same_angle(q.cells[row * 3 + col].angle_deg, expected),
                    "列{col}の回転角度が揃っていない"
                );
            }
            expected
        })
    }

    #[test]
    fn beginner_uses_only_the_row_rule_with_all_angles_zero() {
        let mut rng = StdRng::seed_from_u64(11);
        for _ in 0..100 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            let mut shapes = row_shapes_of(&q).to_vec();
            shapes.sort();
            shapes.dedup();
            assert_eq!(shapes.len(), 3, "行ごとに別の図形種類であること");
            assert!(q.cells.iter().all(|c| c.angle_deg == 0.0));
            assert!(q.choices.iter().all(|c| c.angle_deg == 0.0));
        }
    }

    #[test]
    fn beginner_choices_have_no_duplicate_shapes() {
        let mut rng = StdRng::seed_from_u64(12);
        for _ in 0..100 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            let mut indices: Vec<usize> = q.choices.iter().map(|c| c.shape_index).collect();
            indices.sort();
            indices.dedup();
            assert_eq!(indices.len(), CHOICE_COUNT, "選択肢の図形が重複している");
        }
    }

    #[test]
    fn intermediate_and_advanced_follow_both_row_and_column_rules() {
        let mut rng = StdRng::seed_from_u64(13);
        for difficulty in [Difficulty::Intermediate, Difficulty::Advanced] {
            for _ in 0..100 {
                let q = generate_question(&mut rng, difficulty);
                let mut shapes = row_shapes_of(&q).to_vec();
                shapes.sort();
                shapes.dedup();
                assert_eq!(shapes.len(), 3, "{difficulty:?}: 行の図形種類が重複している");

                let angles = col_angles_of(&q);
                for i in 0..3 {
                    for j in (i + 1)..3 {
                        let d = normalize_angle(angles[i] - angles[j]);
                        assert!(
                            !same_angle(d, 0.0) && !same_angle(d, 180.0),
                            "{difficulty:?}: 列{i}と列{j}の角度が同一か180度差で見分けがつかない"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn advanced_uses_finer_angle_steps_than_intermediate() {
        // Advancedは隣接列の角度差を小さくして見分けにくくする
        let min_step = |q: &Question| {
            let a = col_angles_of(q);
            (0..2)
                .map(|i| {
                    let d = normalize_angle(a[i + 1] - a[i]);
                    d.min(360.0 - d)
                })
                .fold(f64::MAX, f64::min)
        };
        let mut rng = StdRng::seed_from_u64(14);
        for _ in 0..100 {
            let inter = generate_question(&mut rng, Difficulty::Intermediate);
            let adv = generate_question(&mut rng, Difficulty::Advanced);
            assert!(min_step(&inter) >= 45.0 - 1e-6);
            assert!(min_step(&adv) <= 30.0 + 1e-6);
            assert!(min_step(&adv) > 0.0);
        }
    }

    #[test]
    fn correct_choice_satisfies_the_blank_row_and_column_rules() {
        let mut rng = StdRng::seed_from_u64(15);
        for difficulty in ALL_DIFFICULTIES {
            for _ in 0..100 {
                let q = generate_question(&mut rng, difficulty);
                let row = q.blank_index / 3;
                let col = q.blank_index % 3;
                let correct = q.choices[q.correct_index];
                // 空欄以外の同じ行のマスから図形種類、同じ列のマスから角度が決まる
                let other_in_row = (0..3).find(|&c| c != col).unwrap();
                let other_in_col = (0..3).find(|&r| r != row).unwrap();
                assert_eq!(correct.shape_index, q.cells[row * 3 + other_in_row].shape_index);
                assert!(same_angle(
                    correct.angle_deg,
                    q.cells[other_in_col * 3 + col].angle_deg
                ));
                assert_eq!(correct, q.cells[q.blank_index]);
            }
        }
    }

    #[test]
    fn exactly_one_choice_looks_like_the_answer_and_choices_are_distinct() {
        let shapes = base_shapes();
        let mut rng = StdRng::seed_from_u64(16);
        for difficulty in ALL_DIFFICULTIES {
            for _ in 0..200 {
                let q = generate_question(&mut rng, difficulty);
                let answer = q.cells[q.blank_index];
                let matching = q
                    .choices
                    .iter()
                    .filter(|c| looks_same(&shapes, **c, answer))
                    .count();
                assert_eq!(matching, 1, "{difficulty:?}: 正解と同じ見た目の選択肢が1つでない");
                for i in 0..CHOICE_COUNT {
                    for j in (i + 1)..CHOICE_COUNT {
                        assert!(
                            !looks_same(&shapes, q.choices[i], q.choices[j]),
                            "{difficulty:?}: 選択肢{i}と{j}が同じ見た目"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn intermediate_and_advanced_mix_in_one_axis_only_distractors() {
        let mut rng = StdRng::seed_from_u64(17);
        for difficulty in [Difficulty::Intermediate, Difficulty::Advanced] {
            for _ in 0..100 {
                let q = generate_question(&mut rng, difficulty);
                let correct = q.choices[q.correct_index];
                let wrongs: Vec<Cell> = (0..CHOICE_COUNT)
                    .filter(|&i| i != q.correct_index)
                    .map(|i| q.choices[i])
                    .collect();
                // 行の規則(図形種類)だけ正しい誤答
                assert!(
                    wrongs.iter().any(|c| c.shape_index == correct.shape_index
                        && !same_angle(c.angle_deg, correct.angle_deg)),
                    "{difficulty:?}: 図形種類だけ正しい誤答が無い"
                );
                // 列の規則(回転角度)だけ正しい誤答
                assert!(
                    wrongs.iter().any(|c| c.shape_index != correct.shape_index
                        && same_angle(c.angle_deg, correct.angle_deg)),
                    "{difficulty:?}: 回転角度だけ正しい誤答が無い"
                );
            }
        }
    }

    #[test]
    fn looks_same_compares_rendered_appearance() {
        // 点対称な図形(平行四辺形)は180度回転しても同じ見た目になる
        let symmetric = vec![Shape::new(vec![
            (-0.6, -0.4),
            (0.4, -0.4),
            (0.6, 0.4),
            (-0.4, 0.4),
        ])];
        let a = Cell { shape_index: 0, angle_deg: 30.0 };
        let b = Cell { shape_index: 0, angle_deg: 210.0 };
        assert!(looks_same(&symmetric, a, b));
        let shapes = base_shapes();
        // 非対称な直角三角形は180度回転で見た目が変わる
        let t0 = Cell { shape_index: 0, angle_deg: 30.0 };
        let t1 = Cell { shape_index: 0, angle_deg: 210.0 };
        assert!(!looks_same(&shapes, t0, t1));
        // 360度回転は同じ見た目
        let t2 = Cell { shape_index: 0, angle_deg: 390.0 };
        assert!(looks_same(&shapes, t0, t2));
        // 別の図形は同じ見た目にならない
        let other = Cell { shape_index: 1, angle_deg: 30.0 };
        assert!(!looks_same(&shapes, t0, other));
    }

    #[test]
    fn normalize_angle_wraps_into_0_to_360_range() {
        assert_eq!(normalize_angle(0.0), 0.0);
        assert!((normalize_angle(370.0) - 10.0).abs() < 1e-9);
        assert!((normalize_angle(-30.0) - 330.0).abs() < 1e-9);
        assert!((normalize_angle(720.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn pressing_number_key_answers_with_that_choice() {
        let mut game = PatternFillGame::new(Difficulty::Intermediate);
        let correct = game.current.correct_index;
        let key = KeyEvent::new(
            KeyCode::Char(char::from_digit(correct as u32 + 1, 10).unwrap()),
            crossterm::event::KeyModifiers::NONE,
        );
        game.handle_key(key);
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
        // 範囲外のキーは無視される
        game.handle_key(KeyEvent::new(
            KeyCode::Char('5'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn advance_question_records_correct_answer() {
        let mut game = PatternFillGame::new(Difficulty::Beginner);
        let answer = game.current.correct_index;
        game.advance_question(answer);
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn advance_question_records_incorrect_answer() {
        let mut game = PatternFillGame::new(Difficulty::Beginner);
        let wrong = (game.current.correct_index + 1) % CHOICE_COUNT;
        game.advance_question(wrong);
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn session_finishes_after_configured_question_count() {
        let mut game = PatternFillGame::new(Difficulty::Beginner);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            let answer = game.current.correct_index;
            game.advance_question(answer);
        }
        assert!(game.is_finished());
    }

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    #[test]
    fn clicking_a_choice_column_selects_that_index() {
        let mut game = PatternFillGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 21);
        let (_, choices_area, _) = split_areas(area);
        let correct = game.current.correct_index;
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(25); 4])
            .split(choices_area);
        let target = cols[correct];
        game.handle_mouse(left_click(target.x, target.y), area);
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_outside_choices_area_does_nothing() {
        let mut game = PatternFillGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 21);
        let (grid_area, _, _) = split_areas(area);
        game.handle_mouse(left_click(grid_area.x, grid_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn answering_shows_feedback_with_the_correct_choice_number() {
        let mut game = PatternFillGame::new(Difficulty::Beginner);
        let correct = game.current.correct_index;
        game.advance_question(correct);
        let flash = game.feedback.current().expect("回答直後は正誤を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Correct);
        assert_eq!(flash.detail, format!("こたえ: {}番", correct + 1));
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
    }

    #[test]
    fn clicking_anywhere_inside_a_drawn_choice_panel_selects_it() {
        // 選択肢パネルの描画位置(column_bands)とクリック判定(column_index)が一致すること
        let area = Rect::new(0, 0, 43, 24);
        let (_, choices_area, _) = split_areas(area);
        for (i, band) in crate::game::theme::column_bands(choices_area, CHOICE_COUNT as u16)
            .into_iter()
            .enumerate()
        {
            for column in [band.x, band.x + band.width - 1] {
                let mut game = PatternFillGame::new(Difficulty::Beginner);
                game.current.correct_index = i;
                game.handle_mouse(left_click(column, band.y), area);
                let result = game.tracker.to_result(GAME_ID, game.difficulty);
                assert_eq!(result.correct, 1, "選択肢{}の列{column}", i + 1);
            }
        }
    }

    #[test]
    fn render_does_not_panic_for_every_difficulty() {
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let game = PatternFillGame::new(difficulty);
            let backend = ratatui::backend::TestBackend::new(60, 24);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| game.render(frame, frame.area()))
                .unwrap();
        }
    }
}
