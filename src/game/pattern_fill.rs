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

/// Beginner: 全9マス同じ図形の繰り返し
fn generate_beginner(rng: &mut impl Rng, shapes: &[Shape]) -> ([Cell; GRID_SIZE], Cell) {
    let shape_index = rng.gen_range(0..shapes.len());
    let cell = Cell {
        shape_index,
        angle_deg: 0.0,
    };
    ([cell; GRID_SIZE], cell)
}

/// Intermediate: 行ごとに図形が変わる(各行は同じ図形、3行で3種)
fn generate_intermediate(
    rng: &mut impl Rng,
    shapes: &[Shape],
    blank_index: usize,
) -> ([Cell; GRID_SIZE], Cell) {
    let mut row_indices: Vec<usize> = Vec::with_capacity(3);
    while row_indices.len() < 3 {
        let idx = rng.gen_range(0..shapes.len());
        if !row_indices.contains(&idx) {
            row_indices.push(idx);
        }
    }
    let mut cells = [Cell {
        shape_index: 0,
        angle_deg: 0.0,
    }; GRID_SIZE];
    for row in 0..3 {
        for col in 0..3 {
            cells[row * 3 + col] = Cell {
                shape_index: row_indices[row],
                angle_deg: 0.0,
            };
        }
    }
    (cells, cells[blank_index])
}

/// Advanced: 各セルごとに一定角度ずつ回転していく(0度→step度→2*step度…)
fn generate_advanced(rng: &mut impl Rng, shapes: &[Shape]) -> ([Cell; GRID_SIZE], Cell) {
    // 8ステップ分(i=0..=8)を360度未満に収め、かつ全セルの角度が重複しないステップのみ採用
    const STEP_POOL_DEG: [f64; 3] = [30.0, 40.0, 50.0];
    let step_deg = STEP_POOL_DEG[rng.gen_range(0..STEP_POOL_DEG.len())];
    let shape_index = rng.gen_range(0..shapes.len());
    let mut cells = [Cell {
        shape_index: 0,
        angle_deg: 0.0,
    }; GRID_SIZE];
    for (i, cell) in cells.iter_mut().enumerate() {
        *cell = Cell {
            shape_index,
            angle_deg: normalize_angle(i as f64 * step_deg),
        };
    }
    let blank = cells[0]; // placeholder, replaced by caller with correct index
    (cells, blank)
}

fn normalize_angle(angle_deg: f64) -> f64 {
    let mut a = angle_deg % 360.0;
    if a < 0.0 {
        a += 360.0;
    }
    a
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let shapes = base_shapes();
    let blank_index = rng.gen_range(0..GRID_SIZE);

    let (cells, correct_cell) = match difficulty {
        Difficulty::Beginner => generate_beginner(rng, &shapes),
        Difficulty::Intermediate => generate_intermediate(rng, &shapes, blank_index),
        Difficulty::Advanced => {
            let (cells, _) = generate_advanced(rng, &shapes);
            (cells, cells[blank_index])
        }
    };

    // (Cell, is_correct) のペアで作ってからシャッフルし、最後にcorrect_indexを特定する
    let mut tagged: Vec<(Cell, bool)> = vec![(correct_cell, true)];
    match difficulty {
        Difficulty::Beginner => {
            let mut used = vec![correct_cell.shape_index];
            while tagged.len() < CHOICE_COUNT {
                let idx = rng.gen_range(0..shapes.len());
                if !used.contains(&idx) {
                    used.push(idx);
                    tagged.push((
                        Cell {
                            shape_index: idx,
                            angle_deg: 0.0,
                        },
                        false,
                    ));
                }
            }
        }
        Difficulty::Intermediate => {
            // グリッドに使われている行の図形(正解含む)を集め、正解以外の行の図形2つ+
            // グリッドに登場しない図形1つを不正解選択肢にする
            let mut row_shape_indices: Vec<usize> = (0..3).map(|row| cells[row * 3].shape_index).collect();
            row_shape_indices.dedup();
            for &idx in &row_shape_indices {
                if idx != correct_cell.shape_index && tagged.len() < CHOICE_COUNT - 1 + 1 {
                    tagged.push((
                        Cell {
                            shape_index: idx,
                            angle_deg: 0.0,
                        },
                        false,
                    ));
                }
            }
            let mut used: Vec<usize> = row_shape_indices.clone();
            while tagged.len() < CHOICE_COUNT {
                let idx = rng.gen_range(0..shapes.len());
                if !used.contains(&idx) {
                    used.push(idx);
                    tagged.push((
                        Cell {
                            shape_index: idx,
                            angle_deg: 0.0,
                        },
                        false,
                    ));
                }
            }
        }
        Difficulty::Advanced => {
            let mut candidate_angles: Vec<f64> = cells
                .iter()
                .filter(|c| (c.angle_deg - correct_cell.angle_deg).abs() > 1e-9)
                .map(|c| c.angle_deg)
                .collect();
            candidate_angles.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
            candidate_angles.shuffle(rng);
            for angle in candidate_angles.into_iter().take(CHOICE_COUNT - 1) {
                tagged.push((
                    Cell {
                        shape_index: correct_cell.shape_index,
                        angle_deg: angle,
                    },
                    false,
                ));
            }
            // グリッド内に十分な数の異なる角度が無い場合のフォールバック
            while tagged.len() < CHOICE_COUNT {
                let extra = normalize_angle(correct_cell.angle_deg + rng.gen_range(10..350) as f64);
                if !tagged.iter().any(|(c, _)| (c.angle_deg - extra).abs() < 1e-9) {
                    tagged.push((
                        Cell {
                            shape_index: correct_cell.shape_index,
                            angle_deg: extra,
                        },
                        false,
                    ));
                }
            }
        }
    }

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

    #[test]
    fn beginner_grid_is_a_single_repeated_shape() {
        let mut rng = StdRng::seed_from_u64(11);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            let expected = q.cells[0].shape_index;
            assert!(q.cells.iter().all(|c| c.shape_index == expected));
            assert!(q.cells.iter().all(|c| c.angle_deg == 0.0));
            let correct = q.choices[q.correct_index];
            assert_eq!(correct.shape_index, expected);
        }
    }

    #[test]
    fn beginner_choices_have_no_duplicate_shapes() {
        let mut rng = StdRng::seed_from_u64(12);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            let mut indices: Vec<usize> = q.choices.iter().map(|c| c.shape_index).collect();
            indices.sort();
            indices.dedup();
            assert_eq!(indices.len(), CHOICE_COUNT, "選択肢の図形が重複している");
        }
    }

    #[test]
    fn intermediate_rows_share_the_same_shape() {
        let mut rng = StdRng::seed_from_u64(13);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            for row in 0..3 {
                let expected = q.cells[row * 3].shape_index;
                for col in 0..3 {
                    assert_eq!(q.cells[row * 3 + col].shape_index, expected);
                }
            }
            let blank_row = q.blank_index / 3;
            let expected_shape = q.cells[blank_row * 3].shape_index;
            let correct = q.choices[q.correct_index];
            assert_eq!(correct.shape_index, expected_shape);
        }
    }

    #[test]
    fn intermediate_uses_three_distinct_row_shapes() {
        let mut rng = StdRng::seed_from_u64(14);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            let mut row_shapes: Vec<usize> = (0..3).map(|row| q.cells[row * 3].shape_index).collect();
            row_shapes.sort();
            row_shapes.dedup();
            assert_eq!(row_shapes.len(), 3, "行の図形が重複している");
        }
    }

    #[test]
    fn advanced_cells_rotate_by_a_constant_step() {
        let mut rng = StdRng::seed_from_u64(15);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            let shape_index = q.cells[0].shape_index;
            assert!(q.cells.iter().all(|c| c.shape_index == shape_index));

            let step = {
                // i=1のセルの角度を基準ステップとみなす(全セル同一ステップである前提)
                let mut d = q.cells[1].angle_deg - q.cells[0].angle_deg;
                if d < 0.0 {
                    d += 360.0;
                }
                d
            };
            for i in 0..GRID_SIZE {
                let expected = normalize_angle(i as f64 * step);
                let actual = q.cells[i].angle_deg;
                assert!(
                    (expected - actual).abs() < 1e-6,
                    "セル{i}の角度が規則から外れている: expected={expected}, actual={actual}"
                );
            }

            let correct = q.choices[q.correct_index];
            assert_eq!(correct.shape_index, shape_index);
            assert!((correct.angle_deg - q.cells[q.blank_index].angle_deg).abs() < 1e-6);
        }
    }

    #[test]
    fn advanced_choices_have_distinct_angles() {
        let mut rng = StdRng::seed_from_u64(16);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            let mut angles: Vec<i64> = q
                .choices
                .iter()
                .map(|c| (c.angle_deg.round()) as i64)
                .collect();
            angles.sort();
            angles.dedup();
            assert_eq!(angles.len(), CHOICE_COUNT, "選択肢の回転角が重複している");
        }
    }

    #[test]
    fn normalize_angle_wraps_into_0_to_360_range() {
        assert_eq!(normalize_angle(0.0), 0.0);
        assert!((normalize_angle(370.0) - 10.0).abs() < 1e-9);
        assert!((normalize_angle(-30.0) - 330.0).abs() < 1e-9);
        assert!((normalize_angle(720.0) - 0.0).abs() < 1e-9);
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
