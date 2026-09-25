use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::canvas::renderer::ShapeCanvas;
use crate::canvas::shapes::{base_shapes, Shape};
use crate::game::feedback::{AnswerFeedback, Flash};
use crate::game::theme;
use crate::game::{column_index, contains, Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "puzzle_connect";

const CHOICE_COUNT: usize = 4;

/// ピースAの右端とピースBの左端の間にあける隙間
const CONNECT_GAP: f64 = 0.1;
/// 図形の外接矩形から描画範囲の端までの余白
const BOUNDS_MARGIN: f64 = 0.2;
/// 端末の1セルの縦横比(縦/横)。文字セルはおおむね横1:縦2なので、
/// 描画範囲をこの比率で合わせると図形が縦横に伸び縮みしない
const CELL_HEIGHT_PER_WIDTH: f64 = 2.0;

/// 描画エリアを「お手本」「選択肢」「フッター」に分割する。
/// 完成形は横長なので、横に4つ並ぶ選択肢は幅で大きさが決まる。高さは控えめにしてお手本へ回す
fn split_areas(area: Rect) -> (Rect, Rect, Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Percentage(35),
            Constraint::Length(3),
        ])
        .split(area);
    (rows[0], rows[1], rows[2])
}

/// 選択肢エリアを横一列の4パネルに分ける。クリック判定(column_index)と同じ境界になる
fn choice_areas(choices_area: Rect) -> Vec<Rect> {
    theme::column_bands(choices_area, CHOICE_COUNT as u16)
}

/// base_shapes()のインデックス順と対応する図形名(回答後のフィードバック文言に使う)
const SHAPE_NAMES: [&str; 8] = [
    "三角形", "矢印", "L字", "T字", "稲妻", "旗", "階段", "フック",
];

/// ピースAの右端とピースBの左端が接するように、ピースBをX軸方向へ平行移動する。
/// Shape自体は編集せず、このファイル内だけで完結するローカルヘルパー。
fn translate_x(shape: &Shape, dx: f64) -> Shape {
    let points = shape.points.iter().map(|&(x, y)| (x + dx, y)).collect();
    Shape::new(points)
}

fn shape_max_x(shape: &Shape) -> f64 {
    shape
        .points
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max)
}

fn shape_min_x(shape: &Shape) -> f64 {
    shape
        .points
        .iter()
        .map(|p| p.0)
        .fold(f64::INFINITY, f64::min)
}

/// 回転済みのピースBを、ピースAの右端+隙間の位置へ平行移動する
fn connect_to_right_of(piece_a: &Shape, piece_b_rotated: &Shape) -> Shape {
    let dx = shape_max_x(piece_a) - shape_min_x(piece_b_rotated) + CONNECT_GAP;
    translate_x(piece_b_rotated, dx)
}

/// 難易度に応じた回転角(ラジアン)を1つ決める
fn rotation_angle_rad(rng: &mut impl Rng, difficulty: Difficulty) -> f64 {
    match difficulty {
        Difficulty::Beginner => 0.0,
        Difficulty::Intermediate => {
            let step = rng.gen_range(0..8); // 45度刻み x 8方向
            (step as f64 * 45.0).to_radians()
        }
        Difficulty::Advanced => {
            let deg = rng.gen_range(0..360); // 1度刻み
            (deg as f64).to_radians()
        }
    }
}

struct Question {
    /// お手本描画用: 回転後のピースA(選択肢の完成形でも同じものを使う)
    demo_piece_a: Shape,
    /// お手本描画用: 回転+平行移動後のピースB
    demo_piece_b: Shape,
    /// お手本のピースBの回転角。選択肢の候補ピースBも全てこの角度で回す
    angle_b: f64,
    /// 選択肢として提示するbase_shapes()インデックス4つ(重複なし)
    choices: [usize; CHOICE_COUNT],
    /// choices中で正解ピースBが入っている位置
    correct_choice_position: usize,
}

impl Question {
    /// 選択肢positionの完成形に使う候補ピースB。お手本と同じ角度で回転させ、
    /// ピースAの右端に接続する。正解の位置ではdemo_piece_bと同じ形になる
    fn choice_piece_b(&self, position: usize) -> Shape {
        let rotated = base_shapes()[self.choices[position]].rotated(self.angle_b);
        connect_to_right_of(&self.demo_piece_a, &rotated)
    }
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let shapes = base_shapes();
    let shape_count = shapes.len();

    let piece_a_idx = rng.gen_range(0..shape_count);
    let piece_b_idx = rng.gen_range(0..shape_count);

    let angle_a = rotation_angle_rad(rng, difficulty);
    let angle_b = rotation_angle_rad(rng, difficulty);

    let demo_piece_a = shapes[piece_a_idx].rotated(angle_a);
    let demo_piece_b = connect_to_right_of(&demo_piece_a, &shapes[piece_b_idx].rotated(angle_b));

    // 選択肢: 正解ピースB + 重複しない別図形3つ
    let mut choices = vec![piece_b_idx];
    while choices.len() < CHOICE_COUNT {
        let candidate = rng.gen_range(0..shape_count);
        if !choices.contains(&candidate) {
            choices.push(candidate);
        }
    }
    choices.shuffle(rng);
    let correct_choice_position = choices
        .iter()
        .position(|&idx| idx == piece_b_idx)
        .expect("正解ピースBは必ずchoicesに含まれる");

    Question {
        demo_piece_a,
        demo_piece_b,
        angle_b,
        choices: choices.try_into().unwrap(),
        correct_choice_position,
    }
}

/// 図形群の外接矩形に余白を足した描画範囲
fn content_bounds<'a>(shapes: impl IntoIterator<Item = &'a Shape>) -> ([f64; 2], [f64; 2]) {
    let (mut x_min, mut x_max) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut y_min, mut y_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(x, y) in shapes.into_iter().flat_map(|s| s.points.iter()) {
        x_min = x_min.min(x);
        x_max = x_max.max(x);
        y_min = y_min.min(y);
        y_max = y_max.max(y);
    }
    (
        [x_min - BOUNDS_MARGIN, x_max + BOUNDS_MARGIN],
        [y_min - BOUNDS_MARGIN, y_max + BOUNDS_MARGIN],
    )
}

/// お手本と全選択肢の完成形をまとめて収める描画範囲。
/// 全エリアで同じ範囲を使い、選択肢どうし・お手本との間で図形の縮尺をそろえる
fn question_bounds(q: &Question) -> ([f64; 2], [f64; 2]) {
    let choice_pieces: Vec<Shape> = (0..CHOICE_COUNT).map(|p| q.choice_piece_b(p)).collect();
    content_bounds(
        [&q.demo_piece_a, &q.demo_piece_b]
            .into_iter()
            .chain(choice_pieces.iter()),
    )
}

/// 描画範囲を、描画先(inner、セル単位)の見た目の縦横比に合わせて中央基準で広げる。
/// 画像・brailleとも範囲をエリア全体へ引き伸ばして描くため、比率を合わせないと
/// エリアの形によって図形がつぶれ、お手本と選択肢でシルエットが違って見えてしまう
fn fit_bounds_to_cells(bounds: ([f64; 2], [f64; 2]), inner: Rect) -> ([f64; 2], [f64; 2]) {
    if inner.width == 0 || inner.height == 0 {
        return bounds;
    }
    let ([x_min, x_max], [y_min, y_max]) = bounds;
    let (width, height) = (x_max - x_min, y_max - y_min);
    let area_ratio = inner.width as f64 / (inner.height as f64 * CELL_HEIGHT_PER_WIDTH);
    let (new_width, new_height) = if width / height < area_ratio {
        (height * area_ratio, height)
    } else {
        (width, width / area_ratio)
    };
    let (cx, cy) = ((x_min + x_max) / 2.0, (y_min + y_max) / 2.0);
    (
        [cx - new_width / 2.0, cx + new_width / 2.0],
        [cy - new_height / 2.0, cy + new_height / 2.0],
    )
}

pub struct PuzzleConnectGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
    /// 直前の回答の正誤表示(描画専用)
    feedback: AnswerFeedback,
    demo_canvas: ShapeCanvas,
    /// 選択肢ごとのキャンバス。ShapeCanvasは直前の1枚だけをキャッシュするため、
    /// 1つを使い回すと毎フレーム再エンコードになる。選択肢ごとに分けて持つ
    choice_canvases: [ShapeCanvas; CHOICE_COUNT],
}

impl PuzzleConnectGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            current: generate_question(&mut rng, difficulty),
            question_started_at: Instant::now(),
            feedback: AnswerFeedback::new(),
            demo_canvas: ShapeCanvas::new(),
            choice_canvases: std::array::from_fn(|_| ShapeCanvas::new()),
        }
    }

    fn advance_question(&mut self, answered_position: usize) {
        let is_correct = answered_position == self.current.correct_choice_position;
        let latency_ms = self.question_started_at.elapsed().as_millis() as f64;
        self.tracker.record(is_correct, latency_ms);
        let answer = SHAPE_NAMES[self.current.choices[self.current.correct_choice_position]];
        self.feedback.record(is_correct, format!("こたえ: {answer}"));
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

impl Game for PuzzleConnectGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        if let KeyCode::Char(c @ '1'..='4') = key.code {
            let position = c.to_digit(10).unwrap() as usize - 1;
            self.advance_question(position);
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
        let (demo_area, choices_area, footer_area) = split_areas(area);
        // HUDはクリック判定の無いお手本エリアの上端から切り出す(選択肢の位置は変えない)
        let (hud_area, demo_area) = theme::split_hud(demo_area);
        theme::render_hud(
            frame,
            hud_area,
            "組み合わせパズル",
            self.difficulty,
            self.tracker.total(),
            &self.feedback,
        );

        let bounds = question_bounds(&self.current);
        draw_demo(
            frame,
            demo_area,
            &self.demo_canvas,
            &self.current,
            bounds,
            self.feedback.current(),
        );

        // 選択肢はクリック判定(column_index)と同じ4等分のパネルに1つずつ描く
        for (position, panel) in choice_areas(choices_area).into_iter().enumerate() {
            draw_choice(
                frame,
                panel,
                &self.choice_canvases[position],
                position + 1,
                &self.current.demo_piece_a,
                &self.current.choice_piece_b(position),
                bounds,
            );
        }

        theme::render_hint_footer(
            frame,
            footer_area,
            &[("1〜4", "お手本と同じ組み合わせを回答"), ("q", "終了")],
        );
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

/// 1つ目のピースを描く色(お手本・選択肢共通)
const PIECE_A_COLOR: Color = theme::ACCENT_STRONG;
/// 2つ目のピースを描く色(お手本・選択肢共通)
const PIECE_B_COLOR: Color = theme::HIGHLIGHT;

fn draw_demo(
    frame: &mut Frame,
    area: Rect,
    canvas: &ShapeCanvas,
    question: &Question,
    bounds: ([f64; 2], [f64; 2]),
    flash: Option<&Flash>,
) {
    // 色の凡例を枠の下辺に出す
    let legend = Line::from(vec![
        Span::styled(" ━ ", Style::default().fg(PIECE_A_COLOR)),
        Span::styled("1つ目   ", Style::default().fg(theme::TEXT)),
        Span::styled("━ ", Style::default().fg(PIECE_B_COLOR)),
        Span::styled("2つ目 ", Style::default().fg(theme::TEXT)),
    ]);
    let block = theme::focus_panel(" お手本: この2つを組み合わせた完成形 ", flash)
        .title_bottom(legend.centered());
    let bounds = fit_bounds_to_cells(bounds, block.inner(area));
    canvas.render_many(
        frame,
        area,
        block,
        &[
            (&question.demo_piece_a, PIECE_A_COLOR),
            (&question.demo_piece_b, PIECE_B_COLOR),
        ],
        bounds,
    );
}

/// 選択肢1つ分の完成形(ピースA+候補ピースB)を、お手本と同じ配色で描く
fn draw_choice(
    frame: &mut Frame,
    area: Rect,
    canvas: &ShapeCanvas,
    number: usize,
    piece_a: &Shape,
    piece_b: &Shape,
    bounds: ([f64; 2], [f64; 2]),
) {
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
    let bounds = fit_bounds_to_cells(bounds, block.inner(area));
    canvas.render_many(
        frame,
        area,
        block,
        &[(piece_a, PIECE_A_COLOR), (piece_b, PIECE_B_COLOR)],
        bounds,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    const EPS: f64 = 1e-9;

    fn assert_points_eq(actual: &Shape, expected: &Shape, context: &str) {
        assert_eq!(actual.points.len(), expected.points.len(), "{context}: 頂点数");
        for (a, e) in actual.points.iter().zip(&expected.points) {
            assert!(
                (a.0 - e.0).abs() < EPS && (a.1 - e.1).abs() < EPS,
                "{context}: {a:?} != {e:?}"
            );
        }
    }

    #[test]
    fn translate_x_shifts_only_x_axis() {
        let shape = Shape::new(vec![(0.0, 0.5), (1.0, -0.5)]);
        let translated = translate_x(&shape, 2.0);
        assert_eq!(translated.points, vec![(2.0, 0.5), (3.0, -0.5)]);
    }

    #[test]
    fn beginner_rotation_angle_is_always_zero() {
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..20 {
            assert_eq!(rotation_angle_rad(&mut rng, Difficulty::Beginner), 0.0);
        }
    }

    #[test]
    fn generate_question_places_piece_b_directly_after_piece_a_with_margin() {
        let mut rng = StdRng::seed_from_u64(10);
        for _ in 0..30 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            let a_max_x = shape_max_x(&q.demo_piece_a);
            let b_min_x = shape_min_x(&q.demo_piece_b);
            assert!(
                (b_min_x - (a_max_x + 0.1)).abs() < EPS,
                "ピースBはピースAの右端+マージン0.1の位置から始まるべき"
            );
        }
    }

    #[test]
    fn generate_question_choices_have_no_duplicates() {
        let mut rng = StdRng::seed_from_u64(20);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            let mut sorted = q.choices.to_vec();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), CHOICE_COUNT, "選択肢に重複がある");
        }
    }

    #[test]
    fn generate_question_correct_position_is_within_choices() {
        let mut rng = StdRng::seed_from_u64(21);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            assert!(q.correct_choice_position < CHOICE_COUNT);
            // correct_choice_positionの位置にある値は必ずchoices内に実在する図形インデックス
            let idx = q.choices[q.correct_choice_position];
            assert!(idx < base_shapes().len());
        }
    }

    // --- 選択肢の完成形(ピースA+候補ピースB) ---

    #[test]
    fn correct_choice_forms_exactly_the_same_shape_as_the_demo() {
        // 正解の選択肢はお手本と同じ点集合(=同じシルエット)になる
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let mut rng = StdRng::seed_from_u64(30);
            for _ in 0..30 {
                let q = generate_question(&mut rng, difficulty);
                let piece_b = q.choice_piece_b(q.correct_choice_position);
                assert_points_eq(&piece_b, &q.demo_piece_b, "正解の候補ピースB");
            }
        }
    }

    #[test]
    fn wrong_choices_form_a_different_shape_from_the_demo() {
        let mut rng = StdRng::seed_from_u64(31);
        for _ in 0..30 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            for position in (0..CHOICE_COUNT).filter(|&p| p != q.correct_choice_position) {
                assert_ne!(
                    q.choice_piece_b(position).points,
                    q.demo_piece_b.points,
                    "不正解の選択肢{}がお手本と同じ形になっている",
                    position + 1
                );
            }
        }
    }

    #[test]
    fn every_choice_piece_b_is_rotated_by_the_same_angle_as_the_demo() {
        // 全選択肢の候補ピースBは、お手本のピースBと同じ角度(angle_b)で回転し、
        // ピースAの右端+0.1に接続されている(選択肢ごとに回転を変えない)
        let shapes = base_shapes();
        let mut rng = StdRng::seed_from_u64(32);
        let mut saw_rotation = false;
        for _ in 0..30 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            saw_rotation |= q.angle_b != 0.0;
            // お手本のピースBがangle_bで回転したものであること(angle_bがお手本と一致する根拠)
            let demo_expected = shapes[q.choices[q.correct_choice_position]].rotated(q.angle_b);
            let dx = q.demo_piece_b.points[0].0 - demo_expected.points[0].0;
            assert_points_eq(
                &q.demo_piece_b,
                &translate_x(&demo_expected, dx),
                "お手本のピースB",
            );
            for position in 0..CHOICE_COUNT {
                let rotated = shapes[q.choices[position]].rotated(q.angle_b);
                let a_max_x = shape_max_x(&q.demo_piece_a);
                let expected = translate_x(&rotated, a_max_x + 0.1 - shape_min_x(&rotated));
                assert_points_eq(
                    &q.choice_piece_b(position),
                    &expected,
                    &format!("選択肢{}", position + 1),
                );
            }
        }
        assert!(saw_rotation, "上級で一度も回転しなかった");
    }

    // --- 描画範囲 ---

    #[test]
    fn question_bounds_contain_the_demo_and_every_choice() {
        let mut rng = StdRng::seed_from_u64(40);
        for _ in 0..20 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            let ([x_min, x_max], [y_min, y_max]) = question_bounds(&q);
            let mut all: Vec<Shape> = vec![q.demo_piece_a.clone(), q.demo_piece_b.clone()];
            all.extend((0..CHOICE_COUNT).map(|p| q.choice_piece_b(p)));
            for shape in &all {
                for &(x, y) in &shape.points {
                    assert!(x > x_min && x < x_max && y > y_min && y < y_max);
                }
            }
        }
    }

    #[test]
    fn fit_bounds_to_cells_keeps_the_aspect_ratio_of_the_area() {
        // 1セルは横1:縦2の比率とみなす。広げた後の範囲の縦横比がエリアの見た目の縦横比と一致し、
        // 元の範囲を中央に含む(図形が縦横に伸び縮みしない)
        let bounds = ([-1.0, 3.0], [-1.0, 1.0]);
        for inner in [Rect::new(0, 0, 18, 7), Rect::new(0, 0, 78, 6), Rect::new(3, 2, 10, 10)] {
            let ([x_min, x_max], [y_min, y_max]) = fit_bounds_to_cells(bounds, inner);
            let world_ratio = (x_max - x_min) / (y_max - y_min);
            let cell_ratio = inner.width as f64 / (inner.height as f64 * 2.0);
            assert!((world_ratio - cell_ratio).abs() < EPS, "{inner:?}");
            assert!(x_min <= -1.0 && x_max >= 3.0 && y_min <= -1.0 && y_max >= 1.0);
            assert!(((x_min + x_max) / 2.0 - 1.0).abs() < EPS, "横方向は中央そろえ");
            assert!(((y_min + y_max) / 2.0).abs() < EPS, "縦方向は中央そろえ");
        }
    }

    #[test]
    fn fit_bounds_to_cells_returns_bounds_unchanged_for_empty_area() {
        let bounds = ([-1.0, 3.0], [-1.0, 1.0]);
        assert_eq!(fit_bounds_to_cells(bounds, Rect::new(0, 0, 0, 5)), bounds);
        assert_eq!(fit_bounds_to_cells(bounds, Rect::new(0, 0, 5, 0)), bounds);
    }

    // --- 回答 ---

    #[test]
    fn advance_question_with_correct_position_records_success() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        let correct = game.current.correct_choice_position;
        game.advance_question(correct);
        assert_eq!(game.tracker.total(), 1);
        assert_eq!(game.result().correct, 1);
    }

    #[test]
    fn advance_question_with_wrong_position_records_failure() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        let correct = game.current.correct_choice_position;
        let wrong = (correct + 1) % CHOICE_COUNT;
        game.advance_question(wrong);
        assert_eq!(game.tracker.total(), 1);
        assert_eq!(game.result().correct, 0);
    }

    #[test]
    fn session_finishes_after_configured_question_count() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            let position = game.current.correct_choice_position;
            game.advance_question(position);
        }
        assert!(game.is_finished());
        assert_eq!(game.result().correct, crate::game::QUESTIONS_PER_SESSION);
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn number_keys_select_the_matching_choice() {
        for position in 0..CHOICE_COUNT {
            let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
            game.current.correct_choice_position = position;
            let c = char::from_digit(position as u32 + 1, 10).unwrap();
            game.handle_key(key(c));
            assert_eq!(game.tracker.total(), 1);
            assert_eq!(game.result().correct, 1, "キー{c}で選択肢{c}を選ぶ");
        }
    }

    #[test]
    fn other_keys_do_not_answer() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        for c in ['0', '5', 'a'] {
            game.handle_key(key(c));
        }
        assert_eq!(game.tracker.total(), 0);
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
    fn clicking_anywhere_inside_a_drawn_choice_panel_selects_it() {
        // 選択肢パネルの描画位置(choice_areas)とクリック判定が一致すること(四隅で確認)
        for area in [Rect::new(0, 0, 80, 24), Rect::new(0, 0, 43, 20), Rect::new(5, 3, 61, 30)] {
            let (_, choices_area, _) = split_areas(area);
            for (i, panel) in choice_areas(choices_area).into_iter().enumerate() {
                let corners = [
                    (panel.x, panel.y),
                    (panel.x + panel.width - 1, panel.y),
                    (panel.x, panel.y + panel.height - 1),
                    (panel.x + panel.width - 1, panel.y + panel.height - 1),
                ];
                for (column, row) in corners {
                    let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
                    game.current.correct_choice_position = i;
                    game.handle_mouse(left_click(column, row), area);
                    assert_eq!(game.tracker.total(), 1, "{area:?} 選択肢{} ({column},{row})", i + 1);
                    assert_eq!(game.result().correct, 1, "{area:?} 選択肢{} ({column},{row})", i + 1);
                }
            }
        }
    }

    #[test]
    fn choice_areas_split_the_choices_area_into_four_panels_without_overlap() {
        let choices_area = Rect::new(2, 10, 61, 9);
        let panels = choice_areas(choices_area);
        assert_eq!(panels.len(), CHOICE_COUNT);
        let total: u32 = panels.iter().map(|r| r.width as u32 * r.height as u32).sum();
        assert_eq!(total, choices_area.width as u32 * choices_area.height as u32);
        for pair in panels.windows(2) {
            assert!(pair[0].intersection(pair[1]).is_empty());
        }
    }

    #[test]
    fn clicking_outside_choices_area_does_nothing() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 15);
        let (demo_area, _, footer_area) = split_areas(area);
        game.handle_mouse(left_click(demo_area.x, demo_area.y), area);
        game.handle_mouse(left_click(footer_area.x, footer_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn answering_shows_feedback_with_the_correct_shape_name() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        let correct = game.current.correct_choice_position;
        let answer = SHAPE_NAMES[game.current.choices[correct]];
        game.advance_question((correct + 1) % CHOICE_COUNT);
        let flash = game.feedback.current().expect("回答直後は正誤を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert_eq!(flash.detail, format!("こたえ: {answer}"));
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
    }

    // --- 描画 ---

    fn rendered_text(game: &PuzzleConnectGame, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| game.render(frame, frame.area())).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|pos| buffer[pos].symbol().to_string())
            .collect::<String>()
            .replace(' ', "")
    }

    #[test]
    fn choices_do_not_show_shape_names() {
        // 選択肢は図形で描き、図形名のテキストは出さない(回答前はフィードバックも無い)
        let game = PuzzleConnectGame::new(Difficulty::Beginner);
        let text = rendered_text(&game, 80, 24);
        for name in SHAPE_NAMES {
            assert!(!text.contains(name), "図形名「{name}」が描かれている");
        }
    }

    #[test]
    fn every_choice_number_is_drawn_inside_its_click_panel() {
        let area = Rect::new(0, 0, 80, 24);
        let game = PuzzleConnectGame::new(Difficulty::Beginner);
        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| game.render(frame, area)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let (_, choices_area, _) = split_areas(area);
        for (i, panel) in choice_areas(choices_area).into_iter().enumerate() {
            // 番号は枠の上辺に描く
            let top: String = (panel.x..panel.x + panel.width)
                .map(|x| buffer[(x, panel.y)].symbol().to_string())
                .collect();
            assert!(top.contains(&format!(" {} ", i + 1)), "選択肢{}の枠: {top}", i + 1);
        }
    }

    /// お手本と4つの選択肢を画像(sixel)経路にしたゲーム
    fn game_with_image_canvases(difficulty: Difficulty) -> PuzzleConnectGame {
        let mut game = PuzzleConnectGame::new(difficulty);
        game.demo_canvas = ShapeCanvas::with_sixel_for_test();
        game.choice_canvases = std::array::from_fn(|_| ShapeCanvas::with_sixel_for_test());
        game
    }

    const ALL_DIFFICULTIES: [Difficulty; 3] = [
        Difficulty::Beginner,
        Difficulty::Intermediate,
        Difficulty::Advanced,
    ];
    const RENDER_SIZES: [(u16, u16); 5] = [(80, 24), (120, 40), (40, 15), (20, 8), (4, 3)];

    #[test]
    fn render_does_not_panic_without_image_protocol() {
        for difficulty in ALL_DIFFICULTIES {
            let game = PuzzleConnectGame::new(difficulty);
            for (w, h) in RENDER_SIZES {
                rendered_text(&game, w, h);
            }
        }
    }

    #[test]
    fn render_does_not_panic_with_image_protocol() {
        for difficulty in ALL_DIFFICULTIES {
            let game = game_with_image_canvases(difficulty);
            for (w, h) in RENDER_SIZES {
                rendered_text(&game, w, h);
            }
        }
    }

    const TERM_W: u16 = 80;
    const TERM_H: u16 = 24;

    fn dedup_terminal() -> ratatui::Terminal<
        crate::image_backend::ImageDedupBackend<crate::image_backend::RecordingBackend>,
    > {
        use crate::image_backend::{ImageDedupBackend, RecordingBackend};
        ratatui::Terminal::new(ImageDedupBackend::new(RecordingBackend::new(TERM_W, TERM_H)))
            .unwrap()
    }

    #[test]
    fn first_frame_sends_the_demo_and_all_four_choice_images() {
        let game = game_with_image_canvases(Difficulty::Advanced);
        let mut terminal = dedup_terminal();
        terminal.draw(|f| game.render(f, f.area())).unwrap();
        assert_eq!(
            terminal.backend().inner().last_payload_positions().len(),
            1 + CHOICE_COUNT
        );
    }

    #[test]
    fn images_are_not_resent_while_question_is_unchanged() {
        let game = game_with_image_canvases(Difficulty::Advanced);
        let mut terminal = dedup_terminal();
        terminal.draw(|f| game.render(f, f.area())).unwrap();
        for _ in 0..5 {
            terminal.draw(|f| game.render(f, f.area())).unwrap();
            assert!(
                terminal.backend().inner().last_payload_positions().is_empty(),
                "問題が変わらない間は画像を送り直さない"
            );
        }
    }

    #[test]
    fn images_are_not_resent_when_only_the_feedback_border_changes() {
        // 正誤表示でお手本の枠色が変わり、表示時間後に元へ戻っても、図形の画像は送り直さない
        let mut game = game_with_image_canvases(Difficulty::Advanced);
        let mut terminal = dedup_terminal();
        terminal.draw(|f| game.render(f, f.area())).unwrap();
        game.feedback.record(false, "こたえ: 三角形".to_string());
        terminal.draw(|f| game.render(f, f.area())).unwrap();
        assert!(terminal.backend().inner().last_payload_positions().is_empty());
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        terminal.draw(|f| game.render(f, f.area())).unwrap();
        assert!(terminal.backend().inner().last_payload_positions().is_empty());
    }

    #[test]
    fn all_images_are_sent_again_when_the_question_changes() {
        let mut game = game_with_image_canvases(Difficulty::Advanced);
        let mut terminal = dedup_terminal();
        game.current = generate_question(&mut StdRng::seed_from_u64(50), Difficulty::Advanced);
        terminal.draw(|f| game.render(f, f.area())).unwrap();
        game.current = generate_question(&mut StdRng::seed_from_u64(51), Difficulty::Advanced);
        terminal.draw(|f| game.render(f, f.area())).unwrap();

        let sent = terminal.backend().inner().last_payload_positions();
        assert_eq!(sent.len(), 1 + CHOICE_COUNT, "{sent:?}");
        // 選択肢の画像はそれぞれのパネルの中に1枚ずつ出る
        let (_, choices_area, _) = split_areas(Rect::new(0, 0, TERM_W, TERM_H));
        for (i, panel) in choice_areas(choices_area).into_iter().enumerate() {
            let inside = sent.iter().filter(|&&(x, y)| contains(panel, x, y)).count();
            assert_eq!(inside, 1, "選択肢{}のパネル: {sent:?}", i + 1);
        }
    }
}
