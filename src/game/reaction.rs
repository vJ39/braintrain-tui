use std::cell::RefCell;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use image::imageops::{self, FilterType};
use image::{DynamicImage, Rgba, RgbaImage};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;
use rust_embed::RustEmbed;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::theme;
use crate::game::{column_index, contains, Difficulty, Game, GameResult, ScoreTracker};

/// このゲームの描画エリアを「ラベル表示」と「選択肢フッター」に分割する
fn split_areas(area: Rect) -> (Rect, Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);
    (rows[0], rows[1])
}

pub const GAME_ID: &str = "reaction";

const COLORS_BEGINNER: [(&str, Color); 2] = [("赤", Color::Red), ("青", Color::Blue)];
const COLORS_FULL: [(&str, Color); 4] = [
    ("赤", Color::Red),
    ("青", Color::Blue),
    ("緑", Color::Green),
    ("黄", Color::Yellow),
];

fn color_pool(difficulty: Difficulty) -> Vec<(&'static str, Color)> {
    match difficulty {
        Difficulty::Beginner => COLORS_BEGINNER.to_vec(),
        Difficulty::Intermediate | Difficulty::Advanced => COLORS_FULL.to_vec(),
    }
}

/// 上級のみ、この時間内に回答しないと不正解として次の問題へ進む
fn time_limit(difficulty: Difficulty) -> Option<Duration> {
    match difficulty {
        Difficulty::Advanced => Some(Duration::from_secs(3)),
        _ => None,
    }
}

struct Question {
    label: &'static str,
    display_color: Color,
    is_match: bool,
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let pool = color_pool(difficulty);
    let (label, label_color) = pool[rng.gen_range(0..pool.len())];
    let is_match = rng.gen_bool(0.5);
    let display_color = if is_match {
        label_color
    } else {
        let mut candidates: Vec<Color> = pool
            .iter()
            .map(|&(_, c)| c)
            .filter(|&c| c != label_color)
            .collect();
        candidates.shuffle(rng);
        candidates[0]
    };
    Question {
        label,
        display_color,
        is_match,
    }
}

pub struct ReactionGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
    elapsed_in_question: Duration,
    /// 直前の回答の正誤表示(描画専用)
    feedback: AnswerFeedback,
    /// 出題の文字の描画器(描画専用)
    label_renderer: LabelRenderer,
}

impl ReactionGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            current: generate_question(&mut rng, difficulty),
            question_started_at: Instant::now(),
            elapsed_in_question: Duration::ZERO,
            feedback: AnswerFeedback::new(),
            label_renderer: LabelRenderer::new(),
        }
    }

    fn next_question(&mut self) {
        let mut rng = rand::thread_rng();
        self.current = generate_question(&mut rng, self.difficulty);
        self.question_started_at = Instant::now();
        self.elapsed_in_question = Duration::ZERO;
    }

    fn advance_question(&mut self, is_correct: bool) {
        let latency_ms = self.question_started_at.elapsed().as_millis() as f64;
        self.tracker.record(is_correct, latency_ms);
        let answer = if self.current.is_match {
            "一致"
        } else {
            "不一致"
        };
        self.feedback.record(is_correct, format!("こたえ: {answer}"));
        audio::play_se(if is_correct {
            SeKind::Correct
        } else {
            SeKind::Incorrect
        });
        if !self.tracker.is_session_finished() {
            self.next_question();
        }
    }
}

impl Game for ReactionGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        match key.code {
            KeyCode::Left => {
                let is_correct = self.current.is_match;
                self.advance_question(is_correct);
            }
            KeyCode::Right => {
                let is_correct = !self.current.is_match;
                self.advance_question(is_correct);
            }
            _ => {}
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        if self.tracker.is_session_finished() {
            return;
        }
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let (_, footer_area) = split_areas(area);
        if !contains(footer_area, mouse.column, mouse.row) {
            return;
        }
        if let Some(col) = column_index(footer_area, mouse.column, 2) {
            let is_correct = if col == 0 {
                self.current.is_match
            } else {
                !self.current.is_match
            };
            self.advance_question(is_correct);
        }
    }

    fn update(&mut self, dt: Duration) {
        if self.tracker.is_session_finished() {
            return;
        }
        self.feedback.tick(dt);
        self.elapsed_in_question += dt;
        if let Some(limit) = time_limit(self.difficulty) {
            if self.elapsed_in_question >= limit {
                self.advance_question(false);
            }
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (label_area, footer_area) = split_areas(area);
        // HUDはクリック判定の無いラベルエリアの上端から切り出す(フッターの位置は変えない)
        let (hud_area, label_area) = theme::split_hud(label_area);
        theme::render_hud(
            frame,
            hud_area,
            "イロピッタン",
            self.difficulty,
            self.tracker.total(),
            &self.feedback,
        );

        // 出題の色は背景全面の塗りで示し、文字は黒で大きく表示する。
        // 画像表示の時は、画像の背景と周りのセルの背景が同じ色になるようRGBで塗る
        let image_bg = self
            .label_renderer
            .uses_image()
            .then(|| background_rgb(self.current.display_color));
        let background = match image_bg {
            Some([r, g, b]) => Color::Rgb(r, g, b),
            None => self.current.display_color,
        };

        // 枠の色も出題の色そのもの(背景と同じ色になり枠が見えなくなっても構わない)
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(self.current.display_color))
            .style(Style::default().bg(background))
            .title(Line::from(" この背景色と文字の意味は一致？ ").style(theme::title_style()));
        if let Some(limit) = time_limit(self.difficulty) {
            block = block.title_bottom(time_limit_line(self.elapsed_in_question, limit).centered());
        }
        let inner = block.inner(label_area);
        frame.render_widget(block, label_area);

        let drawn_as_image = image_bg.is_some_and(|bg| {
            self.label_renderer
                .render_image(frame, inner, self.current.label, bg)
        });
        if !drawn_as_image {
            render_label_text(frame, inner, self.current.label, background);
        }

        // フッターはcolumn_index(2列)と同じ分割の2ボタン
        theme::render_choice_buttons(frame, footer_area, &[("←", "一致"), ("→", "不一致")]);
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

/// 残り時間の割合に応じた色。残り2/3超=通常、1/3超=注意、それ以下=警告
fn time_limit_color(remaining: Duration, limit: Duration) -> Color {
    if remaining <= limit / 3 {
        theme::INCORRECT
    } else if remaining <= limit * 2 / 3 {
        theme::HIGHLIGHT
    } else {
        theme::ACCENT_STRONG
    }
}

/// 制限時間付きの難易度で、残り時間をバーと秒数で示す1行
fn time_limit_line(elapsed: Duration, limit: Duration) -> Line<'static> {
    const BAR_WIDTH: usize = 12;
    let remaining = limit.saturating_sub(elapsed);
    let color = time_limit_color(remaining, limit);
    let bar = theme::progress_bar(
        remaining.as_millis() as u32,
        limit.as_millis() as u32,
        BAR_WIDTH,
    );
    Line::from(vec![
        Span::styled(" 残り ", Style::default().fg(theme::MUTED)),
        Span::styled(bar, Style::default().fg(color)),
        Span::styled(
            format!(" {:.1}秒 ", remaining.as_secs_f64()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

// ---- 出題の文字の描画 ----
// 文字は黒・透明背景の画像(assets/image/reaction/)を背景色の上に重ねて大きく表示する。
// sixel/kitty/iTerm2の画像プロトコルに対応した端末では画像を、非対応の端末では
// 通常サイズの黒い文字をテキストで表示する。

#[derive(RustEmbed)]
#[folder = "assets/image/reaction/"]
struct LabelAssets;

/// 出題の文字(label)に対応する画像ファイル名。対応する画像が無ければNone
fn label_image_file(label: &str) -> Option<&'static str> {
    match label {
        "赤" => Some("aka.png"),
        "青" => Some("ao.png"),
        "緑" => Some("midori.png"),
        "黄" => Some("ki.png"),
        _ => None,
    }
}

/// 出題の文字の画像を読み込む。該当する画像が無い/読めない場合はNone
fn load_label_image(label: &str) -> Option<RgbaImage> {
    let file = LabelAssets::get(label_image_file(label)?)?;
    let image = image::load_from_memory(&file.data).ok()?;
    Some(image.to_rgba8())
}

/// 画像表示の時の背景色(RGB)。端末の色設定に左右されず、画像の背景と周りのセルを
/// 同じ色で塗れるようにRGBで持つ。黒い文字が読みやすい明るさにしている
fn background_rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Red => [230, 50, 50],
        Color::Blue => [50, 120, 255],
        Color::Green => [40, 190, 70],
        Color::Yellow => [240, 210, 0],
        Color::Rgb(r, g, b) => [r, g, b],
        _ => [128, 128, 128],
    }
}

/// inner(セル単位)の中央に置く、ピクセル換算で正方形になる範囲。
/// 一辺はinnerの幅と高さ(ピクセル)の短い方。font_sizeは1セルのピクセル数(幅, 高さ)
fn glyph_area(inner: Rect, font_size: (u16, u16)) -> Rect {
    let cell_width = u32::from(font_size.0.max(1));
    let cell_height = u32::from(font_size.1.max(1));
    let side = u32::min(
        u32::from(inner.width) * cell_width,
        u32::from(inner.height) * cell_height,
    );
    // sideはinnerの幅・高さ(ピクセル)以下なので、セル数もinnerに収まる
    let cols = (side / cell_width) as u16;
    let rows = (side / cell_height) as u16;
    Rect::new(
        inner.x + (inner.width - cols) / 2,
        inner.y + (inner.height - rows) / 2,
        cols,
        rows,
    )
}

/// cols x rows セル分のピクセル画像を背景色(不透明)で塗り、文字の画像を縦横比を保ったまま
/// 収まる最大の大きさで中央に重ねる。透明な部分が残らないので、端末ごとの透明の扱いの違い
/// (sixelでは白になる等)に左右されない
fn compose_label_image(
    glyph: &RgbaImage,
    cols: u16,
    rows: u16,
    font_size: (u16, u16),
    bg: [u8; 3],
) -> RgbaImage {
    let width = u32::from(cols) * u32::from(font_size.0.max(1));
    let height = u32::from(rows) * u32::from(font_size.1.max(1));
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([bg[0], bg[1], bg[2], 255]));
    if width == 0 || height == 0 || glyph.width() == 0 || glyph.height() == 0 {
        return canvas;
    }
    let scale = f64::min(
        f64::from(width) / f64::from(glyph.width()),
        f64::from(height) / f64::from(glyph.height()),
    );
    let glyph_width = ((f64::from(glyph.width()) * scale).round() as u32).clamp(1, width);
    let glyph_height = ((f64::from(glyph.height()) * scale).round() as u32).clamp(1, height);
    let scaled = imageops::resize(glyph, glyph_width, glyph_height, FilterType::Triangle);
    imageops::overlay(
        &mut canvas,
        &scaled,
        i64::from((width - glyph_width) / 2),
        i64::from((height - glyph_height) / 2),
    );
    // 不透明な背景に重ねたので結果も不透明のはずだが、合成時の小数の切り捨てで
    // 文字の輪郭のアルファが254になることがあるため、255にそろえる
    for pixel in canvas.pixels_mut() {
        pixel.0[3] = 255;
    }
    canvas
}

/// 画像プロトコル非対応の端末向け表示。通常サイズの黒い文字を上下中央に置く
fn render_label_text(frame: &mut Frame, inner: Rect, label: &str, background: Color) {
    let vertical_padding = inner.height.saturating_sub(1) / 2;
    let mut lines: Vec<Line> = (0..vertical_padding).map(|_| Line::from("")).collect();
    let label_style = Style::default()
        .fg(Color::Black)
        .bg(background)
        .add_modifier(Modifier::BOLD);
    lines.push(Line::from(Span::styled(
        format!("  {label}  "),
        label_style,
    )));
    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .style(Style::default().bg(background));
    frame.render_widget(paragraph, inner);
}

/// 直前に作った文字画像。文字・背景色・描画範囲が同じなら再エンコードを省く
struct LabelCache {
    label: &'static str,
    bg: [u8; 3],
    area: Rect,
    protocol: StatefulProtocol,
}

/// 出題の文字の描画器。画像プロトコルが使える端末では文字を画像で大きく表示する
struct LabelRenderer {
    picker: Option<Picker>,
    cache: RefCell<Option<LabelCache>>,
}

impl LabelRenderer {
    fn new() -> Self {
        Self::with_picker(detect_picker())
    }

    fn with_picker(picker: Option<Picker>) -> Self {
        Self {
            picker,
            cache: RefCell::new(None),
        }
    }

    /// 画像プロトコルを使うか(false=テキスト表示)
    fn uses_image(&self) -> bool {
        self.picker.is_some()
    }

    /// inner(枠の内側)の中央に文字の画像を描く。画像プロトコルが使えない/画像が読めない/
    /// 描く場所が無い場合は何もせずfalse(呼び出し側がテキスト表示に切り替える)
    fn render_image(
        &self,
        frame: &mut Frame,
        inner: Rect,
        label: &'static str,
        bg: [u8; 3],
    ) -> bool {
        let Some(picker) = &self.picker else {
            return false;
        };
        let area = glyph_area(inner.intersection(frame.area()), picker.font_size());
        if area.is_empty() {
            return false;
        }
        let mut cache = self.cache.borrow_mut();
        let needs_regen = !matches!(
            cache.as_ref(),
            Some(cached) if cached.label == label && cached.bg == bg && cached.area == area
        );
        if needs_regen {
            let Some(glyph) = load_label_image(label) else {
                return false;
            };
            let composed =
                compose_label_image(&glyph, area.width, area.height, picker.font_size(), bg);
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(composed));
            *cache = Some(LabelCache {
                label,
                bg,
                area,
                protocol,
            });
        }
        let Some(cached) = cache.as_mut() else {
            return false;
        };
        frame.render_stateful_widget(StatefulImage::default(), area, &mut cached.protocol);
        true
    }
}

/// 端末の画像プロトコルを調べる。sixel/kitty/iTerm2のどれかが使える時だけSome。
/// テストでは端末に問い合わせず、常にテキスト表示にする(実行環境で結果が変わらないように)
fn detect_picker() -> Option<Picker> {
    if cfg!(test) {
        return None;
    }
    Picker::from_query_stdio().ok().filter(|picker| {
        matches!(
            picker.protocol_type(),
            ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use ratatui_image::picker::ProtocolType;

    #[test]
    fn time_limit_color_turns_warning_as_time_runs_out() {
        let limit = Duration::from_secs(3);
        assert_eq!(
            time_limit_color(Duration::from_secs(3), limit),
            theme::ACCENT_STRONG
        );
        assert_eq!(
            time_limit_color(Duration::from_millis(1500), limit),
            theme::HIGHLIGHT
        );
        assert_eq!(
            time_limit_color(Duration::from_millis(500), limit),
            theme::INCORRECT
        );
        assert_eq!(time_limit_color(Duration::ZERO, limit), theme::INCORRECT);
    }

    #[test]
    fn answering_shows_feedback_with_the_correct_answer() {
        let mut game = ReactionGame::new(Difficulty::Beginner);
        game.current.is_match = true;
        // 一致の問題に「不一致」(→)と答える
        game.handle_key(KeyEvent::from(KeyCode::Right));
        let flash = game.feedback.current().expect("回答直後は正誤を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert_eq!(flash.detail, "こたえ: 一致");
    }

    #[test]
    fn feedback_disappears_after_hold_time() {
        let mut game = ReactionGame::new(Difficulty::Beginner);
        let is_match = game.current.is_match;
        game.handle_key(KeyEvent::from(if is_match {
            KeyCode::Left
        } else {
            KeyCode::Right
        }));
        assert!(game.feedback.current().is_some());
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
        // 正解数の表示は消えない
        assert_eq!(game.feedback.correct(), 1);
    }

    #[test]
    fn match_question_uses_labels_own_color() {
        let mut rng = StdRng::seed_from_u64(20);
        let mut saw_match = false;
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            if q.is_match {
                saw_match = true;
                let expected = COLORS_FULL
                    .iter()
                    .find(|&&(label, _)| label == q.label)
                    .unwrap()
                    .1;
                assert_eq!(q.display_color, expected);
            }
        }
        assert!(saw_match);
    }

    #[test]
    fn mismatch_question_never_uses_labels_own_color() {
        let mut rng = StdRng::seed_from_u64(21);
        let mut saw_mismatch = false;
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            if !q.is_match {
                saw_mismatch = true;
                let label_color = COLORS_FULL
                    .iter()
                    .find(|&&(label, _)| label == q.label)
                    .unwrap()
                    .1;
                assert_ne!(q.display_color, label_color);
            }
        }
        assert!(saw_mismatch);
    }

    #[test]
    fn advanced_difficulty_auto_fails_after_time_limit() {
        let mut game = ReactionGame::new(Difficulty::Advanced);
        game.update(Duration::from_secs(4));
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn beginner_difficulty_has_no_time_limit() {
        assert_eq!(time_limit(Difficulty::Beginner), None);
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
    fn clicking_left_half_answers_match() {
        let mut game = ReactionGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 10);
        let (_, footer_area) = split_areas(area);
        game.current.is_match = true;
        game.handle_mouse(left_click(footer_area.x, footer_area.y), area);
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_right_half_answers_mismatch() {
        let mut game = ReactionGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 10);
        let (_, footer_area) = split_areas(area);
        game.current.is_match = false;
        let right_column = footer_area.x + footer_area.width - 1;
        game.handle_mouse(left_click(right_column, footer_area.y), area);
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_outside_footer_area_does_nothing() {
        let mut game = ReactionGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 10);
        let (label_area, _) = split_areas(area);
        game.handle_mouse(left_click(label_area.x, label_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }

    // ---- 出題の表示(背景色で色を示し、文字は黒の大きな画像) ----

    #[test]
    fn all_four_label_images_are_embedded() {
        for &(label, _) in COLORS_FULL.iter() {
            let image = load_label_image(label)
                .unwrap_or_else(|| panic!("「{label}」の画像が埋め込まれていること"));
            assert_eq!(image.dimensions(), (512, 512));
        }
    }

    #[test]
    fn label_maps_to_its_image_file() {
        assert_eq!(label_image_file("赤"), Some("aka.png"));
        assert_eq!(label_image_file("青"), Some("ao.png"));
        assert_eq!(label_image_file("緑"), Some("midori.png"));
        assert_eq!(label_image_file("黄"), Some("ki.png"));
        assert_eq!(label_image_file("紫"), None);
        assert!(load_label_image("紫").is_none());
    }

    #[test]
    fn every_pool_color_has_distinct_background_rgb() {
        let rgbs: Vec<[u8; 3]> = COLORS_FULL
            .iter()
            .map(|&(_, c)| background_rgb(c))
            .collect();
        for (i, a) in rgbs.iter().enumerate() {
            for b in &rgbs[i + 1..] {
                assert_ne!(a, b, "色ごとに違う背景色になる");
            }
        }
    }

    #[test]
    fn glyph_area_is_centered_square_in_pixels() {
        // 40x10セル、1セル10x20ピクセル => 400x200ピクセル。正方形の一辺は200ピクセル=20x10セル
        let inner = Rect::new(2, 3, 40, 10);
        let glyph = glyph_area(inner, (10, 20));
        assert_eq!((glyph.width, glyph.height), (20, 10));
        assert_eq!((glyph.x, glyph.y), (12, 3), "横方向の中央に置く");
        // 縦長のエリアでも収まる
        let tall = Rect::new(0, 0, 10, 30);
        let glyph = glyph_area(tall, (10, 20));
        assert_eq!((glyph.width, glyph.height), (10, 5));
        assert!(glyph.y >= tall.y && glyph.bottom() <= tall.bottom());
        // 空のエリアは空のまま
        assert!(glyph_area(Rect::new(0, 0, 0, 5), (10, 20)).is_empty());
    }

    #[test]
    fn composed_label_image_is_opaque_background_with_black_glyph() {
        let glyph = load_label_image("緑").unwrap();
        let bg = [10, 200, 30];
        let image = compose_label_image(&glyph, 20, 10, (10, 20), bg);
        assert_eq!(image.dimensions(), (200, 200));
        assert!(
            image.pixels().all(|p| p.0[3] == 255),
            "透明な部分は残らない(背景色で埋まる)"
        );
        assert_eq!(image.get_pixel(0, 0).0, [10, 200, 30, 255], "隅は背景色");
        let black = image
            .pixels()
            .filter(|p| p.0[..3].iter().all(|&c| c <= 50))
            .count();
        assert!(black > 1000, "文字の黒が描かれる");
    }

    /// 出題を固定した状態で描き、バッファと出題エリアの内側(枠の内側)を返す
    fn render_game(
        game: &ReactionGame,
        width: u16,
        height: u16,
    ) -> (ratatui::buffer::Buffer, Rect) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        let (label_area, _) = split_areas(Rect::new(0, 0, width, height));
        let (_, label_area) = theme::split_hud(label_area);
        let inner = Block::default().borders(Borders::ALL).inner(label_area);
        (terminal.backend().buffer().clone(), inner)
    }

    fn fixed_question(game: &mut ReactionGame, label: &'static str, display_color: Color) {
        game.current = Question {
            label,
            display_color,
            is_match: false,
        };
    }

    #[test]
    fn fallback_fills_background_with_display_color_and_black_text() {
        // テスト環境では画像プロトコルを検出しないのでテキスト表示になる
        let mut game = ReactionGame::new(Difficulty::Beginner);
        assert!(!game.label_renderer.uses_image());
        fixed_question(&mut game, "赤", Color::Blue);
        let (buffer, inner) = render_game(&game, 40, 16);
        for y in inner.y..inner.bottom() {
            for x in inner.x..inner.right() {
                // 全角文字の右半分のセルはratatuiが中身を空にする(端末では全角文字が2セル分を
                // 自分の背景色で描く)ので対象外
                if x > inner.x && buffer[(x - 1, y)].symbol() == "赤" {
                    continue;
                }
                assert_eq!(buffer[(x, y)].bg, Color::Blue, "({x},{y})の背景は文字色");
            }
        }
        let cell = (inner.y..inner.bottom())
            .flat_map(|y| (inner.x..inner.right()).map(move |x| (x, y)))
            .map(|pos| &buffer[pos])
            .find(|c| c.symbol() == "赤")
            .expect("出題の文字が描かれること");
        assert_eq!(cell.fg, Color::Black, "文字は黒");
    }

    #[test]
    fn render_does_not_panic_with_image_protocols() {
        for protocol in [
            ProtocolType::Halfblocks,
            ProtocolType::Sixel,
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
        ] {
            let mut picker = Picker::from_fontsize((10, 20));
            picker.set_protocol_type(protocol);
            let mut game = ReactionGame::new(Difficulty::Advanced);
            game.label_renderer = LabelRenderer::with_picker(Some(picker));
            assert!(game.label_renderer.uses_image());
            for &(label, color) in COLORS_FULL.iter() {
                fixed_question(&mut game, label, color);
                render_game(&game, 60, 20);
            }
            // 極端に小さい画面でもパニックしない
            render_game(&game, 4, 4);
        }
    }

    #[test]
    fn image_mode_paints_background_rgb_around_glyph() {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        let mut game = ReactionGame::new(Difficulty::Beginner);
        game.label_renderer = LabelRenderer::with_picker(Some(picker));
        fixed_question(&mut game, "青", Color::Red);
        let (buffer, inner) = render_game(&game, 60, 20);
        let [r, g, b] = background_rgb(Color::Red);
        // 文字画像の外(枠の内側の左端)は背景色で塗られている
        assert_eq!(buffer[(inner.x, inner.y)].bg, Color::Rgb(r, g, b));
    }

    #[test]
    fn fallback_render_does_not_panic_on_tiny_area() {
        let game = ReactionGame::new(Difficulty::Advanced);
        render_game(&game, 4, 4);
        render_game(&game, 1, 1);
    }
}
