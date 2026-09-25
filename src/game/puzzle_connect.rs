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
use crate::canvas::shapes::{
    edges_interlock, jigsaw_piece_left, jigsaw_piece_right, jigsaw_profile_count,
    jigsaw_profile_name, jigsaw_similar_profiles, JigsawEdge, Knob, Shape,
};
use crate::game::feedback::{AnswerFeedback, Flash};
use crate::game::theme;
use crate::game::{column_index, contains, Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "puzzle_connect";

const CHOICE_COUNT: usize = 4;

/// 図形の外接矩形から描画範囲の端までの余白
const BOUNDS_MARGIN: f64 = 0.2;
/// 端末の1セルの縦横比(縦/横)。文字セルはおおむね横1:縦2なので、
/// 描画範囲をこの比率で合わせると図形が縦横に伸び縮みしない
const CELL_HEIGHT_PER_WIDTH: f64 = 2.0;

/// 描画エリアを「お手本」「選択肢」「フッター」に分割する。
/// 選択肢は凹凸の細かな違いを見比べる必要があるので、お手本と同程度の高さを確保する
fn split_areas(area: Rect) -> (Rect, Rect, Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Percentage(45),
            Constraint::Length(3),
        ])
        .split(area);
    (rows[0], rows[1], rows[2])
}

/// 選択肢エリアを横一列の4パネルに分ける。クリック判定(column_index)と同じ境界になる
fn choice_areas(choices_area: Rect) -> Vec<Rect> {
    theme::column_bands(choices_area, CHOICE_COUNT as u16)
}

/// 難易度ごとに出題に使う形状番号の数(0..この値の形状番号を使う)。
/// 形状番号は shapes.rs で「基本形 → そのサイズ違い → 紛らわしい形」の順に並んでいる
fn profile_pool_size(difficulty: Difficulty) -> usize {
    match difficulty {
        // 三角・四角・丸だけ。形の違いが一目で分かる
        Difficulty::Beginner => 3,
        // サイズ違い(小さな三角・四角・丸)を加える
        Difficulty::Intermediate => 6,
        // あり型・キノコ型・二つ山など輪郭の近い形まで全部使う
        Difficulty::Advanced => jigsaw_profile_count(),
    }
}

struct Question {
    /// お手本のピースAの右辺
    piece_a_edge: JigsawEdge,
    /// 選択肢として提示するピースBの左辺4つ(重複なし)
    choices: [JigsawEdge; CHOICE_COUNT],
    /// choices中で正解(ピースAの右辺と噛み合う左辺)がある位置
    correct_choice_position: usize,
}

impl Question {
    /// お手本のピースA(右辺に接続部を持つ正方形)
    fn piece_a(&self) -> Shape {
        jigsaw_piece_right(self.piece_a_edge)
    }

    /// 選択肢positionのピースB(左辺に接続部を持つ正方形。ピースAには接続しない)
    fn choice_piece(&self, position: usize) -> Shape {
        jigsaw_piece_left(self.choices[position])
    }
}

/// 誤答の左辺を3つ選ぶ。難易度が上がるほど紛らわしいものを混ぜる
fn pick_decoys(
    rng: &mut impl Rng,
    difficulty: Difficulty,
    piece_a_edge: JigsawEdge,
) -> Vec<JigsawEdge> {
    let pool = profile_pool_size(difficulty);
    let similar: Vec<usize> = jigsaw_similar_profiles(piece_a_edge.profile)
        .iter()
        .copied()
        .filter(|&p| p < pool)
        .collect();
    let mut decoys = Vec::with_capacity(CHOICE_COUNT - 1);

    // 上級: 同じ形で凹凸も同じ(タブ同士・ブランク同士で噛み合わない)ものを必ず1つ
    if difficulty == Difficulty::Advanced {
        decoys.push(piece_a_edge);
    }
    // 中級以上: 似た形(サイズ違い等)で凹凸の向きは合っているものを必ず1つ
    if difficulty != Difficulty::Beginner {
        if let Some(&p) = similar.choose(rng) {
            decoys.push(JigsawEdge::new(p, piece_a_edge.knob.opposite()));
        }
    }

    // 残りは別の形から選ぶ。初級は似た形を避け、はっきり違う形だけにする
    let mut rest: Vec<JigsawEdge> = (0..pool)
        .filter(|&p| p != piece_a_edge.profile)
        .filter(|p| difficulty != Difficulty::Beginner || !similar.contains(p))
        .flat_map(|p| {
            [
                JigsawEdge::new(p, Knob::Tab),
                JigsawEdge::new(p, Knob::Blank),
            ]
        })
        .filter(|e| !decoys.contains(e))
        .collect();
    rest.shuffle(rng);
    let needed = CHOICE_COUNT - 1 - decoys.len();
    decoys.extend(rest.into_iter().take(needed));
    decoys
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let profile = rng.gen_range(0..profile_pool_size(difficulty));
    let knob = if rng.gen_bool(0.5) {
        Knob::Tab
    } else {
        Knob::Blank
    };
    let piece_a_edge = JigsawEdge::new(profile, knob);
    // 正解: 同じ形で凹凸が逆の左辺
    let correct = JigsawEdge::new(profile, knob.opposite());

    let mut choices = pick_decoys(rng, difficulty, piece_a_edge);
    choices.push(correct);
    choices.shuffle(rng);
    // 正解の位置は噛み合わせ判定で決める(誤答はどれも噛み合わない)
    let correct_choice_position = choices
        .iter()
        .position(|&e| edges_interlock(piece_a_edge, e))
        .expect("正解は必ずchoicesに含まれる");

    Question {
        piece_a_edge,
        choices: choices
            .try_into()
            .expect("選択肢は必ずCHOICE_COUNT個そろう"),
        correct_choice_position,
    }
}

/// お手本と全選択肢のピースをまとめて収める描画範囲(原点中心で上下左右対称)。
/// 全エリアで同じ範囲を使い、選択肢どうし・お手本との間で図形の縮尺をそろえる
fn question_bounds(q: &Question) -> ([f64; 2], [f64; 2]) {
    let shapes: Vec<Shape> = std::iter::once(q.piece_a())
        .chain((0..CHOICE_COUNT).map(|p| q.choice_piece(p)))
        .collect();
    let (mut x_extent, mut y_extent) = (0.0_f64, 0.0_f64);
    for &(x, y) in shapes.iter().flat_map(|s| s.points.iter()) {
        x_extent = x_extent.max(x.abs());
        y_extent = y_extent.max(y.abs());
    }
    let (x, y) = (x_extent + BOUNDS_MARGIN, y_extent + BOUNDS_MARGIN);
    ([-x, x], [-y, y])
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

/// 回答後に出す正解の説明(例: 「こたえ: 3番 (丸のくぼみ)」)
fn answer_detail(q: &Question) -> String {
    let edge = q.choices[q.correct_choice_position];
    let knob = match edge.knob {
        Knob::Tab => "出っ張り",
        Knob::Blank => "くぼみ",
    };
    format!(
        "こたえ: {}番 ({}の{knob})",
        q.correct_choice_position + 1,
        jigsaw_profile_name(edge.profile)
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
        self.feedback
            .record(is_correct, answer_detail(&self.current));
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
            &self.current.piece_a(),
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
                &self.current.choice_piece(position),
                bounds,
            );
        }

        theme::render_hint_footer(
            frame,
            footer_area,
            &[("1〜4", "右辺にぴったりはまるピースを回答"), ("q", "終了")],
        );
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

/// お手本のピースAを描く色
const PIECE_A_COLOR: Color = theme::ACCENT_STRONG;
/// 選択肢のピースBを描く色
const PIECE_B_COLOR: Color = theme::HIGHLIGHT;

fn draw_demo(
    frame: &mut Frame,
    area: Rect,
    canvas: &ShapeCanvas,
    piece_a: &Shape,
    bounds: ([f64; 2], [f64; 2]),
    flash: Option<&Flash>,
) {
    let block = theme::focus_panel(" お手本: このピースの右辺にはまるのは? ", flash);
    let bounds = fit_bounds_to_cells(bounds, block.inner(area));
    canvas.render_many(frame, area, block, &[(piece_a, PIECE_A_COLOR)], bounds);
}

/// 選択肢1つ分のピースB(左辺に接続部を持つ正方形)を描く
fn draw_choice(
    frame: &mut Frame,
    area: Rect,
    canvas: &ShapeCanvas,
    number: usize,
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
    canvas.render_many(frame, area, block, &[(piece_b, PIECE_B_COLOR)], bounds);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use std::collections::HashSet;

    const EPS: f64 = 1e-9;

    const ALL_DIFFICULTIES: [Difficulty; 3] = [
        Difficulty::Beginner,
        Difficulty::Intermediate,
        Difficulty::Advanced,
    ];

    /// 各難易度で多めに問題を生成する(乱数の偏りで条件を取りこぼさないように)
    fn sample_questions(difficulty: Difficulty, seed: u64) -> Vec<Question> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..300)
            .map(|_| generate_question(&mut rng, difficulty))
            .collect()
    }

    /// 誤答の選択肢(正解位置以外)
    fn decoys(q: &Question) -> Vec<JigsawEdge> {
        (0..CHOICE_COUNT)
            .filter(|&p| p != q.correct_choice_position)
            .map(|p| q.choices[p])
            .collect()
    }

    /// 同じ形状番号で凹凸も同じ(タブ同士・ブランク同士)の誤答か
    fn is_same_knob_decoy(q: &Question, e: JigsawEdge) -> bool {
        e == q.piece_a_edge
    }

    /// ピースAの形状に似た形(サイズ違い等)の誤答か
    fn is_similar_decoy(q: &Question, e: JigsawEdge) -> bool {
        jigsaw_similar_profiles(q.piece_a_edge.profile).contains(&e.profile)
    }

    // --- 出題 ---

    #[test]
    fn every_question_has_exactly_one_interlocking_choice_at_the_correct_position() {
        for difficulty in ALL_DIFFICULTIES {
            for q in sample_questions(difficulty, 1) {
                let fitting: Vec<usize> = (0..CHOICE_COUNT)
                    .filter(|&p| edges_interlock(q.piece_a_edge, q.choices[p]))
                    .collect();
                assert_eq!(fitting, vec![q.correct_choice_position], "{difficulty:?}");
            }
        }
    }

    #[test]
    fn correct_choice_is_the_same_profile_with_the_opposite_knob() {
        for difficulty in ALL_DIFFICULTIES {
            for q in sample_questions(difficulty, 2) {
                let correct = q.choices[q.correct_choice_position];
                assert_eq!(correct.profile, q.piece_a_edge.profile);
                assert_eq!(correct.knob, q.piece_a_edge.knob.opposite());
            }
        }
    }

    #[test]
    fn choices_have_no_duplicates() {
        for difficulty in ALL_DIFFICULTIES {
            for q in sample_questions(difficulty, 3) {
                let unique: HashSet<JigsawEdge> = q.choices.iter().copied().collect();
                assert_eq!(
                    unique.len(),
                    CHOICE_COUNT,
                    "{difficulty:?}: {:?}",
                    q.choices
                );
            }
        }
    }

    #[test]
    fn questions_only_use_profiles_within_the_difficulty_pool() {
        for difficulty in ALL_DIFFICULTIES {
            let pool = profile_pool_size(difficulty);
            assert!(pool <= jigsaw_profile_count());
            for q in sample_questions(difficulty, 4) {
                assert!(q.piece_a_edge.profile < pool);
                assert!(q.choices.iter().all(|e| e.profile < pool), "{difficulty:?}");
            }
        }
    }

    #[test]
    fn profile_variations_increase_with_difficulty() {
        let beginner = profile_pool_size(Difficulty::Beginner);
        let intermediate = profile_pool_size(Difficulty::Intermediate);
        let advanced = profile_pool_size(Difficulty::Advanced);
        assert!((2..=3).contains(&beginner), "初級は2〜3種");
        assert!(beginner < intermediate && intermediate < advanced);
        // 実際に出題される形状も、各難易度のプール全体に行き渡る
        for difficulty in ALL_DIFFICULTIES {
            let used: HashSet<usize> = sample_questions(difficulty, 5)
                .iter()
                .map(|q| q.piece_a_edge.profile)
                .collect();
            assert_eq!(used.len(), profile_pool_size(difficulty), "{difficulty:?}");
        }
    }

    #[test]
    fn piece_a_appears_with_both_tabs_and_blanks() {
        for difficulty in ALL_DIFFICULTIES {
            let knobs: HashSet<Knob> = sample_questions(difficulty, 6)
                .iter()
                .map(|q| q.piece_a_edge.knob)
                .collect();
            assert_eq!(knobs.len(), 2, "{difficulty:?}");
        }
    }

    #[test]
    fn correct_position_is_spread_over_all_choices() {
        let positions: HashSet<usize> = sample_questions(Difficulty::Beginner, 7)
            .iter()
            .map(|q| q.correct_choice_position)
            .collect();
        assert_eq!(positions.len(), CHOICE_COUNT);
    }

    #[test]
    fn beginner_decoys_are_clearly_different_shapes() {
        // 初級: 誤答は形がはっきり違うものだけ(同じ形・似た形は出さない)
        for q in sample_questions(Difficulty::Beginner, 8) {
            for e in decoys(&q) {
                assert_ne!(e.profile, q.piece_a_edge.profile, "{:?}", q.choices);
                assert!(!is_similar_decoy(&q, e), "{:?}", q.choices);
            }
        }
    }

    #[test]
    fn intermediate_always_mixes_in_a_similar_shape_decoy() {
        // 中級: 似た形(サイズ違い等)の誤答を必ず混ぜる。同じ形で凹凸違いはまだ出さない
        for q in sample_questions(Difficulty::Intermediate, 9) {
            let d = decoys(&q);
            assert!(
                d.iter().any(|&e| is_similar_decoy(&q, e)),
                "{:?}",
                q.choices
            );
            assert!(
                d.iter().all(|&e| e.profile != q.piece_a_edge.profile),
                "{:?}",
                q.choices
            );
        }
    }

    #[test]
    fn advanced_always_mixes_in_a_same_shape_same_knob_decoy() {
        // 上級: 同じ形状番号で凹凸も同じ(タブ同士・ブランク同士で噛み合わない)誤答を必ず含める
        for q in sample_questions(Difficulty::Advanced, 10) {
            let d = decoys(&q);
            assert!(
                d.iter().any(|&e| is_same_knob_decoy(&q, e)),
                "{:?}",
                q.choices
            );
            assert!(
                d.iter().any(|&e| is_similar_decoy(&q, e)),
                "{:?}",
                q.choices
            );
        }
    }

    #[test]
    fn confusing_decoys_increase_with_difficulty() {
        // 紛らわしい誤答(同じ形 or 似た形)の1問あたりの数は、難易度が上がるほど増える
        let confusing = |difficulty| -> usize {
            sample_questions(difficulty, 11)
                .iter()
                .map(|q| {
                    decoys(q)
                        .into_iter()
                        .filter(|&e| is_same_knob_decoy(q, e) || is_similar_decoy(q, e))
                        .count()
                })
                .sum()
        };
        let (b, i, a) = (
            confusing(Difficulty::Beginner),
            confusing(Difficulty::Intermediate),
            confusing(Difficulty::Advanced),
        );
        assert!(b < i && i < a, "初級{b} 中級{i} 上級{a}");
    }

    #[test]
    fn piece_a_has_the_knob_on_its_right_and_choices_on_their_left() {
        let mut rng = StdRng::seed_from_u64(12);
        for _ in 0..20 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            assert_eq!(q.piece_a(), jigsaw_piece_right(q.piece_a_edge));
            for p in 0..CHOICE_COUNT {
                assert_eq!(q.choice_piece(p), jigsaw_piece_left(q.choices[p]));
            }
        }
    }

    // --- 描画範囲 ---

    #[test]
    fn question_bounds_contain_the_demo_and_every_choice_and_are_centered() {
        let mut rng = StdRng::seed_from_u64(40);
        for _ in 0..20 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            let ([x_min, x_max], [y_min, y_max]) = question_bounds(&q);
            assert!((x_min + x_max).abs() < EPS && (y_min + y_max).abs() < EPS);
            let mut all = vec![q.piece_a()];
            all.extend((0..CHOICE_COUNT).map(|p| q.choice_piece(p)));
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
        for inner in [
            Rect::new(0, 0, 18, 7),
            Rect::new(0, 0, 78, 6),
            Rect::new(3, 2, 10, 10),
        ] {
            let ([x_min, x_max], [y_min, y_max]) = fit_bounds_to_cells(bounds, inner);
            let world_ratio = (x_max - x_min) / (y_max - y_min);
            let cell_ratio = inner.width as f64 / (inner.height as f64 * 2.0);
            assert!((world_ratio - cell_ratio).abs() < EPS, "{inner:?}");
            assert!(x_min <= -1.0 && x_max >= 3.0 && y_min <= -1.0 && y_max >= 1.0);
            assert!(
                ((x_min + x_max) / 2.0 - 1.0).abs() < EPS,
                "横方向は中央そろえ"
            );
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
        for area in [
            Rect::new(0, 0, 80, 24),
            Rect::new(0, 0, 43, 20),
            Rect::new(5, 3, 61, 30),
        ] {
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
                    assert_eq!(
                        game.tracker.total(),
                        1,
                        "{area:?} 選択肢{} ({column},{row})",
                        i + 1
                    );
                    assert_eq!(
                        game.result().correct,
                        1,
                        "{area:?} 選択肢{} ({column},{row})",
                        i + 1
                    );
                }
            }
        }
    }

    #[test]
    fn choice_areas_split_the_choices_area_into_four_panels_without_overlap() {
        let choices_area = Rect::new(2, 10, 61, 9);
        let panels = choice_areas(choices_area);
        assert_eq!(panels.len(), CHOICE_COUNT);
        let total: u32 = panels
            .iter()
            .map(|r| r.width as u32 * r.height as u32)
            .sum();
        assert_eq!(
            total,
            choices_area.width as u32 * choices_area.height as u32
        );
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
    fn answering_shows_feedback_with_the_correct_choice_and_shape() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        let correct = game.current.correct_choice_position;
        let edge = game.current.choices[correct];
        game.advance_question((correct + 1) % CHOICE_COUNT);
        let flash = game.feedback.current().expect("回答直後は正誤を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert!(
            flash.detail.contains(&format!("{}番", correct + 1)),
            "{}",
            flash.detail
        );
        assert!(
            flash.detail.contains(jigsaw_profile_name(edge.profile)),
            "{}",
            flash.detail
        );
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
    }

    #[test]
    fn answer_detail_names_the_knob_direction() {
        let q = Question {
            piece_a_edge: JigsawEdge::new(0, Knob::Tab),
            choices: [
                JigsawEdge::new(1, Knob::Blank),
                JigsawEdge::new(0, Knob::Blank),
                JigsawEdge::new(2, Knob::Tab),
                JigsawEdge::new(1, Knob::Tab),
            ],
            correct_choice_position: 1,
        };
        assert_eq!(
            answer_detail(&q),
            format!("こたえ: 2番 ({}のくぼみ)", jigsaw_profile_name(0))
        );
    }

    // --- 描画 ---

    fn rendered_text(game: &PuzzleConnectGame, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|pos| buffer[pos].symbol().to_string())
            .collect::<String>()
            .replace(' ', "")
    }

    #[test]
    fn choices_do_not_show_shape_names() {
        // 選択肢は図形で描き、形状名のテキストは出さない(回答前はフィードバックも無い)
        let game = PuzzleConnectGame::new(Difficulty::Advanced);
        let text = rendered_text(&game, 80, 24);
        for profile in 0..jigsaw_profile_count() {
            let name = jigsaw_profile_name(profile);
            assert!(!text.contains(name), "形状名「{name}」が描かれている");
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
            assert!(
                top.contains(&format!(" {} ", i + 1)),
                "選択肢{}の枠: {top}",
                i + 1
            );
        }
    }

    /// お手本と4つの選択肢を画像(sixel)経路にしたゲーム
    fn game_with_image_canvases(difficulty: Difficulty) -> PuzzleConnectGame {
        let mut game = PuzzleConnectGame::new(difficulty);
        game.demo_canvas = ShapeCanvas::with_sixel_for_test();
        game.choice_canvases = std::array::from_fn(|_| ShapeCanvas::with_sixel_for_test());
        game
    }

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
        ratatui::Terminal::new(ImageDedupBackend::new(RecordingBackend::new(
            TERM_W, TERM_H,
        )))
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
                terminal
                    .backend()
                    .inner()
                    .last_payload_positions()
                    .is_empty(),
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
        game.feedback.record(false, "こたえ: 1番".to_string());
        terminal.draw(|f| game.render(f, f.area())).unwrap();
        assert!(terminal
            .backend()
            .inner()
            .last_payload_positions()
            .is_empty());
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        terminal.draw(|f| game.render(f, f.area())).unwrap();
        assert!(terminal
            .backend()
            .inner()
            .last_payload_positions()
            .is_empty());
    }

    #[test]
    fn all_images_are_sent_again_when_the_question_changes() {
        // 前の問題と全ピースの形が異なる問題に切り替えると、5枚とも送り直す
        let mut game = game_with_image_canvases(Difficulty::Advanced);
        let mut terminal = dedup_terminal();
        game.current = Question {
            piece_a_edge: JigsawEdge::new(0, Knob::Tab),
            choices: [
                JigsawEdge::new(0, Knob::Blank),
                JigsawEdge::new(1, Knob::Blank),
                JigsawEdge::new(2, Knob::Blank),
                JigsawEdge::new(0, Knob::Tab),
            ],
            correct_choice_position: 0,
        };
        terminal.draw(|f| game.render(f, f.area())).unwrap();
        game.current = Question {
            piece_a_edge: JigsawEdge::new(1, Knob::Blank),
            choices: [
                JigsawEdge::new(2, Knob::Tab),
                JigsawEdge::new(1, Knob::Tab),
                JigsawEdge::new(0, Knob::Tab),
                JigsawEdge::new(1, Knob::Blank),
            ],
            correct_choice_position: 1,
        };
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
