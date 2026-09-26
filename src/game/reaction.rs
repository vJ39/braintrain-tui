use std::cell::RefCell;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use image::{DynamicImage, RgbaImage};
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
use crate::game::feedback::{AnswerFeedback, Verdict};
use crate::game::mark_display::{compose_glyph_image, glyph_area, MarkRenderer};
#[cfg(test)]
use crate::game::mark_display::{CORRECT_MARK, INCORRECT_MARK};
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

/// イロピッタンの1セッションの問題数。6問(初級相当)→7問(中級相当)→7問(上級相当)
const SESSION_LENGTH: u32 = 20;

/// 結果・HUDに出す難易度。問題が進むと難易度が上がるため、最後の区間の上級を代表値にする
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;

/// 何問目(0始まり)の問題の難易度。1〜6問目=初級、7〜13問目=中級、14〜20問目=上級
fn question_difficulty(question_index: u32) -> Difficulty {
    match question_index {
        0..=5 => Difficulty::Beginner,
        6..=12 => Difficulty::Intermediate,
        _ => Difficulty::Advanced,
    }
}

/// 正誤判定時に鳴らす効果音
fn verdict_se(is_correct: bool) -> SeKind {
    if is_correct {
        SeKind::Correct
    } else {
        SeKind::Incorrect
    }
}

/// 出題の文字の表記
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notation {
    Kanji,
    Katakana,
}

/// パレットの1色。漢字/カタカナの表記と、背景に塗る色を持つ
#[derive(Clone, Copy, Debug)]
struct PaletteColor {
    kanji: &'static str,
    katakana: &'static str,
    color: Color,
}

impl PaletteColor {
    fn label(&self, notation: Notation) -> &'static str {
        match notation {
            Notation::Kanji => self.kanji,
            Notation::Katakana => self.katakana,
        }
    }
}

const fn palette_color(kanji: &'static str, katakana: &'static str, color: Color) -> PaletteColor {
    PaletteColor {
        kanji,
        katakana,
        color,
    }
}

/// 全色。先頭から初級は2色・中級は4色・上級は全9色を使う。
/// 端末の名前付き色が無い茶・橙・桃はRGBで持つ(画像表示の背景色もこのRGBになる)
const PALETTE: [PaletteColor; 9] = [
    palette_color("赤", "レッド", Color::Red),
    palette_color("青", "ブルー", Color::Blue),
    palette_color("緑", "グリーン", Color::Green),
    palette_color("黄", "イエロー", Color::Yellow),
    palette_color("紫", "パープル", Color::Magenta),
    palette_color("白", "ホワイト", Color::White),
    palette_color("茶", "ブラウン", Color::Rgb(180, 120, 70)),
    palette_color("橙", "オレンジ", Color::Rgb(255, 150, 30)),
    palette_color("桃", "ピンク", Color::Rgb(255, 160, 200)),
];

fn color_pool(difficulty: Difficulty) -> &'static [PaletteColor] {
    match difficulty {
        Difficulty::Beginner => &PALETTE[..2],
        Difficulty::Intermediate => &PALETTE[..4],
        Difficulty::Advanced => &PALETTE[..],
    }
}

/// 出題の表記を選ぶ。上級のみ漢字/カタカナを出題ごとにランダムに選び、初級/中級は常に漢字
fn pick_notation(rng: &mut impl Rng, difficulty: Difficulty) -> Notation {
    match difficulty {
        Difficulty::Advanced if rng.gen_bool(0.5) => Notation::Katakana,
        _ => Notation::Kanji,
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

/// 出題を作る。背景色は前回の出題の背景色(previous_color)を除いた色から選ぶので、
/// 同じ背景色が連続しない
fn generate_question(
    rng: &mut impl Rng,
    difficulty: Difficulty,
    previous_color: Option<Color>,
) -> Question {
    let pool = color_pool(difficulty);
    let display_candidates: Vec<&PaletteColor> = pool
        .iter()
        .filter(|p| Some(p.color) != previous_color)
        .collect();
    let display = display_candidates.choose(rng).copied().unwrap_or(&pool[0]);
    let is_match = rng.gen_bool(0.5);
    let notation = pick_notation(rng, difficulty);
    let label_entry = if is_match {
        display
    } else {
        let label_candidates: Vec<&PaletteColor> =
            pool.iter().filter(|p| p.color != display.color).collect();
        label_candidates.choose(rng).copied().unwrap_or(display)
    };
    Question {
        label: label_entry.label(notation),
        display_color: display.color,
        is_match,
    }
}

pub struct ReactionGame {
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
    elapsed_in_question: Duration,
    /// 直前の回答の正誤表示(描画専用)
    feedback: AnswerFeedback,
    /// 回答した瞬間の背景色。フィードバック表示中はこの色を使い続ける
    /// (advance_questionで即座に次の問題へ切り替わるため、currentの色をそのまま使うと
    /// ◯/✗表示中に次の問題の背景色になってしまう)
    answered_display_color: Option<Color>,
    /// 出題の文字の描画器(描画専用)
    label_renderer: LabelRenderer,
    /// 正誤の記号(◯/✗)の描画器(描画専用)
    mark_renderer: MarkRenderer,
}

impl Default for ReactionGame {
    fn default() -> Self {
        Self::new()
    }
}

impl ReactionGame {
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        // 端末への問い合わせは1回にし、出題文字と正誤の記号で同じPickerを使う
        let picker = detect_picker();
        Self {
            tracker: ScoreTracker::with_session_length(SESSION_LENGTH),
            // 初回は前回の出題が無い
            current: generate_question(&mut rng, question_difficulty(0), None),
            question_started_at: Instant::now(),
            elapsed_in_question: Duration::ZERO,
            feedback: AnswerFeedback::new(),
            answered_display_color: None,
            label_renderer: LabelRenderer::with_picker(picker.clone()),
            mark_renderer: MarkRenderer::with_picker(picker),
        }
    }

    /// いま出題中の問題の難易度
    fn current_difficulty(&self) -> Difficulty {
        question_difficulty(self.tracker.total())
    }

    fn next_question(&mut self) {
        let mut rng = rand::thread_rng();
        self.current = generate_question(
            &mut rng,
            self.current_difficulty(),
            Some(self.current.display_color),
        );
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
        self.feedback
            .record(is_correct, format!("こたえ: {answer}"));
        self.answered_display_color = Some(self.current.display_color);
        audio::play_se(verdict_se(is_correct));
        if !self.tracker.is_session_finished() {
            self.next_question();
        }
    }

    /// 出題エリアに出す正誤の記号。正誤表示中はSome(正解か)、それ以外は出題の文字を出すのでNone
    fn displayed_mark(&self) -> Option<bool> {
        self.feedback
            .current()
            .map(|flash| flash.verdict == Verdict::Correct)
    }

    /// 出題エリアの背景色。フィードバック表示中は回答した瞬間の色を維持し、
    /// 表示が終わったら現在の出題の色に戻る
    fn displayed_background_color(&self) -> Color {
        match self.feedback.current() {
            Some(_) => self.answered_display_color.unwrap_or(self.current.display_color),
            None => self.current.display_color,
        }
    }
}

impl Game for ReactionGame {
    fn handle_key(&mut self, key: KeyEvent) {
        // 正誤フィードバック(◯✗)表示中の入力は、次の問題への回答として扱わない
        if self.tracker.is_session_finished() || self.feedback.current().is_some() {
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
        // 正誤フィードバック(◯✗)表示中の入力は、次の問題への回答として扱わない
        if self.tracker.is_session_finished() || self.feedback.current().is_some() {
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
        if let Some(limit) = time_limit(self.current_difficulty()) {
            if self.elapsed_in_question >= limit {
                self.advance_question(false);
            }
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (label_area, footer_area) = split_areas(area);
        // HUDはクリック判定の無いラベルエリアの上端から切り出す(フッターの位置は変えない)
        let (hud_area, label_area) = theme::split_hud(label_area);
        theme::render_hud_with_session_length(
            frame,
            hud_area,
            "イロピッタン",
            SESSION_DIFFICULTY,
            self.tracker.total(),
            self.tracker.session_length(),
            &self.feedback,
        );

        // 出題の色は背景全面の塗りで示し、文字は黒で大きく表示する。
        // 画像表示の時は、画像の背景と周りのセルの背景が同じ色になるようRGBで塗る
        let display_color = self.displayed_background_color();
        let image_bg = self
            .label_renderer
            .uses_image()
            .then(|| background_rgb(display_color));
        let background = match image_bg {
            Some([r, g, b]) => Color::Rgb(r, g, b),
            None => display_color,
        };

        // 枠の色も出題の色そのもの(背景と同じ色になり枠が見えなくなっても構わない)
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(display_color))
            .style(Style::default().bg(background))
            .title(Line::from(" この背景色と文字の意味は一致？ ").style(theme::title_style()));
        if let Some(limit) = time_limit(self.current_difficulty()) {
            block = block.title_bottom(time_limit_line(self.elapsed_in_question, limit).centered());
        }
        let inner = block.inner(label_area);
        frame.render_widget(block, label_area);

        // 正誤表示中は出題文字の代わりに◯/✗を出す(背景色は回答した瞬間の出題の色のまま)
        if let Some(is_correct) = self.displayed_mark() {
            self.mark_renderer.render(frame, inner, is_correct, background);
        } else {
            let label = self.current.label;
            let drawn_as_image = image_bg
                .is_some_and(|bg| self.label_renderer.render_image(frame, inner, label, bg));
            if !drawn_as_image {
                render_label_text(frame, inner, label, background);
            }
        }

        // フッターはcolumn_index(2列)と同じ分割の2ボタン
        theme::render_choice_buttons(frame, footer_area, &[("←", "一致"), ("→", "不一致")]);
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
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
// 正誤の記号(◯/✗)の表示はmark_display::MarkRendererが受け持つ。

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
        "紫" => Some("murasaki.png"),
        "白" => Some("shiro.png"),
        "茶" => Some("cha.png"),
        "橙" => Some("daidai.png"),
        "桃" => Some("momo.png"),
        // カタカナは横長の画像
        "レッド" => Some("reddo.png"),
        "ブルー" => Some("buruu.png"),
        "グリーン" => Some("guriin.png"),
        "イエロー" => Some("ieroo.png"),
        "パープル" => Some("paapuru.png"),
        "ホワイト" => Some("howaito.png"),
        "ブラウン" => Some("buraun.png"),
        "オレンジ" => Some("orenji.png"),
        "ピンク" => Some("pinku.png"),
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
        Color::Magenta => [170, 110, 230],
        Color::White => [245, 245, 245],
        // 茶・橙・桃はパレットでRGBを持つのでそのまま使う
        Color::Rgb(r, g, b) => [r, g, b],
        _ => [128, 128, 128],
    }
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
    /// 直前に読み込んだ文字の画像(文字, 画像)。描画範囲の計算に画像の寸法が要るので、
    /// 同じ文字の間は毎フレームPNGを読み直さないよう持っておく
    glyph: RefCell<Option<(&'static str, RgbaImage)>>,
    cache: RefCell<Option<LabelCache>>,
}

impl LabelRenderer {
    /// PickerはReactionGame::newで調べ、正誤の記号の描画器と同じものを渡す
    fn with_picker(picker: Option<Picker>) -> Self {
        Self {
            picker,
            glyph: RefCell::new(None),
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
        // 描画範囲は画像の縦横比で決まるので、先に画像を読み込む
        let mut glyph_cache = self.glyph.borrow_mut();
        if !matches!(glyph_cache.as_ref(), Some((cached, _)) if *cached == label) {
            let Some(image) = load_label_image(label) else {
                return false;
            };
            *glyph_cache = Some((label, image));
        }
        let Some((_, glyph)) = glyph_cache.as_ref() else {
            return false;
        };
        let area = glyph_area(
            inner.intersection(frame.area()),
            picker.font_size(),
            glyph.dimensions(),
        );
        if area.is_empty() {
            return false;
        }
        let mut cache = self.cache.borrow_mut();
        let needs_regen = !matches!(
            cache.as_ref(),
            Some(cached) if cached.label == label && cached.bg == bg && cached.area == area
        );
        if needs_regen {
            let composed =
                compose_glyph_image(glyph, area.width, area.height, picker.font_size(), bg);
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
        let mut game = ReactionGame::new();
        game.current.is_match = true;
        // 一致の問題に「不一致」(→)と答える
        game.handle_key(KeyEvent::from(KeyCode::Right));
        let flash = game.feedback.current().expect("回答直後は正誤を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert_eq!(flash.detail, "こたえ: 一致");
    }

    #[test]
    fn feedback_disappears_after_hold_time() {
        let mut game = ReactionGame::new();
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
    fn key_input_during_feedback_is_ignored() {
        let mut game = ReactionGame::new();
        let is_match = game.current.is_match;
        game.handle_key(KeyEvent::from(if is_match {
            KeyCode::Left
        } else {
            KeyCode::Right
        }));
        assert!(game.feedback.current().is_some(), "回答直後は正誤表示中");
        // 正誤表示が消える前の追加入力は次の問題への回答として扱わない
        game.handle_key(KeyEvent::from(KeyCode::Left));
        game.handle_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(game.tracker.total(), 1, "表示中の入力は無視される");
    }

    #[test]
    fn mouse_input_during_feedback_is_ignored() {
        let mut game = ReactionGame::new();
        let area = Rect::new(0, 0, 40, 10);
        let (_, footer_area) = split_areas(area);
        let is_match = game.current.is_match;
        let column = if is_match {
            footer_area.x
        } else {
            footer_area.x + footer_area.width - 1
        };
        game.handle_mouse(left_click(column, footer_area.y), area);
        assert!(game.feedback.current().is_some(), "回答直後は正誤表示中");
        game.handle_mouse(left_click(column, footer_area.y), area);
        assert_eq!(game.tracker.total(), 1, "表示中のクリックは無視される");
    }

    #[test]
    fn input_is_accepted_again_after_feedback_hold_time() {
        let mut game = ReactionGame::new();
        let is_match = game.current.is_match;
        game.handle_key(KeyEvent::from(if is_match {
            KeyCode::Left
        } else {
            KeyCode::Right
        }));
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
        let next_is_match = game.current.is_match;
        game.handle_key(KeyEvent::from(if next_is_match {
            KeyCode::Left
        } else {
            KeyCode::Right
        }));
        assert_eq!(game.tracker.total(), 2, "表示が消えたら次の回答を受け付ける");
    }

    const ALL_DIFFICULTIES: [Difficulty; 3] = [
        Difficulty::Beginner,
        Difficulty::Intermediate,
        Difficulty::Advanced,
    ];

    /// 出題の文字(漢字/カタカナ)から、パレットの色と表記を引く
    fn lookup(label: &str) -> (PaletteColor, Notation) {
        PALETTE
            .iter()
            .find_map(|p| {
                if p.kanji == label {
                    Some((*p, Notation::Kanji))
                } else if p.katakana == label {
                    Some((*p, Notation::Katakana))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("「{label}」はパレットの文字であること"))
    }

    fn pool_kanji(difficulty: Difficulty) -> Vec<&'static str> {
        color_pool(difficulty).iter().map(|p| p.kanji).collect()
    }

    #[test]
    fn color_pool_per_difficulty() {
        assert_eq!(pool_kanji(Difficulty::Beginner), ["赤", "青"]);
        assert_eq!(
            pool_kanji(Difficulty::Intermediate),
            ["赤", "青", "緑", "黄"]
        );
        assert_eq!(
            pool_kanji(Difficulty::Advanced),
            ["赤", "青", "緑", "黄", "紫", "白", "茶", "橙", "桃"]
        );
    }

    #[test]
    fn palette_pairs_kanji_with_katakana() {
        let pairs: Vec<(&str, &str)> = PALETTE.iter().map(|p| (p.kanji, p.katakana)).collect();
        assert_eq!(
            pairs,
            [
                ("赤", "レッド"),
                ("青", "ブルー"),
                ("緑", "グリーン"),
                ("黄", "イエロー"),
                ("紫", "パープル"),
                ("白", "ホワイト"),
                ("茶", "ブラウン"),
                ("橙", "オレンジ"),
                ("桃", "ピンク"),
            ]
        );
        assert_eq!(PALETTE[0].label(Notation::Kanji), "赤");
        assert_eq!(PALETTE[0].label(Notation::Katakana), "レッド");
    }

    #[test]
    fn palette_display_colors_are_distinct() {
        for (i, a) in PALETTE.iter().enumerate() {
            for b in &PALETTE[i + 1..] {
                assert_ne!(a.color, b.color, "{}と{}は別の色", a.kanji, b.kanji);
            }
        }
    }

    #[test]
    fn match_question_uses_labels_own_color() {
        for difficulty in ALL_DIFFICULTIES {
            let mut rng = StdRng::seed_from_u64(20);
            let mut saw_match = false;
            for _ in 0..100 {
                let q = generate_question(&mut rng, difficulty, None);
                if q.is_match {
                    saw_match = true;
                    assert_eq!(q.display_color, lookup(q.label).0.color);
                }
            }
            assert!(saw_match);
        }
    }

    #[test]
    fn mismatch_question_never_uses_labels_own_color() {
        for difficulty in ALL_DIFFICULTIES {
            let mut rng = StdRng::seed_from_u64(21);
            let mut saw_mismatch = false;
            for _ in 0..100 {
                let q = generate_question(&mut rng, difficulty, None);
                if !q.is_match {
                    saw_mismatch = true;
                    assert_ne!(q.display_color, lookup(q.label).0.color);
                }
            }
            assert!(saw_mismatch);
        }
    }

    #[test]
    fn question_colors_stay_within_difficulty_pool() {
        for difficulty in ALL_DIFFICULTIES {
            let pool = color_pool(difficulty);
            let mut rng = StdRng::seed_from_u64(22);
            for _ in 0..200 {
                let q = generate_question(&mut rng, difficulty, None);
                assert!(pool.iter().any(|p| p.color == q.display_color));
                let (entry, _) = lookup(q.label);
                assert!(pool.iter().any(|p| p.kanji == entry.kanji));
            }
        }
    }

    #[test]
    fn question_never_repeats_previous_display_color() {
        for difficulty in ALL_DIFFICULTIES {
            for previous in color_pool(difficulty) {
                let mut rng = StdRng::seed_from_u64(23);
                for _ in 0..100 {
                    let q = generate_question(&mut rng, difficulty, Some(previous.color));
                    assert_ne!(
                        q.display_color, previous.color,
                        "前回と同じ背景色にならない"
                    );
                }
            }
        }
    }

    #[test]
    fn consecutive_questions_change_display_color() {
        for difficulty in ALL_DIFFICULTIES {
            let mut rng = StdRng::seed_from_u64(24);
            let mut previous = generate_question(&mut rng, difficulty, None).display_color;
            let mut seen = vec![previous];
            for _ in 0..300 {
                let q = generate_question(&mut rng, difficulty, Some(previous));
                assert_ne!(q.display_color, previous);
                previous = q.display_color;
                if !seen.contains(&previous) {
                    seen.push(previous);
                }
            }
            // 前回の色を除いても、プールの全色が出題される
            assert_eq!(seen.len(), color_pool(difficulty).len());
        }
    }

    #[test]
    fn game_next_question_never_repeats_display_color() {
        // 初級・中級・上級それぞれの区間の先頭(1問目・4問目・8問目)から確かめる
        for start in [0, 3, 7] {
            let mut game = ReactionGame::new();
            advance_to_question(&mut game, start);
            for _ in 0..200 {
                let before = game.current.display_color;
                game.next_question();
                assert_ne!(game.current.display_color, before);
            }
        }
    }

    #[test]
    fn beginner_and_intermediate_always_use_kanji() {
        for difficulty in [Difficulty::Beginner, Difficulty::Intermediate] {
            let mut rng = StdRng::seed_from_u64(25);
            for _ in 0..200 {
                let q = generate_question(&mut rng, difficulty, None);
                assert_eq!(lookup(q.label).1, Notation::Kanji, "「{}」は漢字", q.label);
            }
        }
    }

    #[test]
    fn advanced_mixes_kanji_and_katakana() {
        let mut rng = StdRng::seed_from_u64(26);
        let (mut kanji, mut katakana) = (0, 0);
        for _ in 0..400 {
            let q = generate_question(&mut rng, Difficulty::Advanced, None);
            match lookup(q.label).1 {
                Notation::Kanji => kanji += 1,
                Notation::Katakana => katakana += 1,
            }
        }
        // 出題ごとに半々で選ぶので、どちらも十分な回数出る
        assert!(kanji > 100, "漢字 {kanji} 回");
        assert!(katakana > 100, "カタカナ {katakana} 回");
    }

    #[test]
    fn advanced_difficulty_auto_fails_after_time_limit() {
        let mut game = advanced_game();
        game.update(Duration::from_secs(4));
        assert_eq!(game.tracker.total(), 14);
    }

    // ---- 6問(初級相当)→7問(中級相当)→7問(上級相当)の固定20問構成 ----

    /// 何問目(0始まり)ごとの難易度
    const EXPECTED_DIFFICULTIES: [Difficulty; 20] = [
        Difficulty::Beginner,
        Difficulty::Beginner,
        Difficulty::Beginner,
        Difficulty::Beginner,
        Difficulty::Beginner,
        Difficulty::Beginner,
        Difficulty::Intermediate,
        Difficulty::Intermediate,
        Difficulty::Intermediate,
        Difficulty::Intermediate,
        Difficulty::Intermediate,
        Difficulty::Intermediate,
        Difficulty::Intermediate,
        Difficulty::Advanced,
        Difficulty::Advanced,
        Difficulty::Advanced,
        Difficulty::Advanced,
        Difficulty::Advanced,
        Difficulty::Advanced,
        Difficulty::Advanced,
    ];

    /// 記録だけを積んで、index問目(0始まり)を出題中の状態にする
    fn advance_to_question(game: &mut ReactionGame, index: u32) {
        while game.tracker.total() < index {
            game.tracker.record(true, 0.0);
        }
        game.next_question();
    }

    /// 上級相当の区間(14問目)を出題中のゲーム
    fn advanced_game() -> ReactionGame {
        let mut game = ReactionGame::new();
        advance_to_question(&mut game, 13);
        game
    }

    /// 正誤表示が消えるまで待ってから、一致(←)で回答する
    fn answer_and_wait(game: &mut ReactionGame) {
        game.handle_key(KeyEvent::from(KeyCode::Left));
        game.update(crate::game::feedback::FEEDBACK_HOLD);
    }

    #[test]
    fn question_difficulty_follows_six_seven_seven_questions() {
        for (index, expected) in EXPECTED_DIFFICULTIES.iter().enumerate() {
            assert_eq!(
                question_difficulty(index as u32),
                *expected,
                "{}問目",
                index + 1
            );
        }
    }

    #[test]
    fn session_difficulty_is_advanced_and_recorded_in_result() {
        assert_eq!(SESSION_DIFFICULTY, Difficulty::Advanced);
        let mut game = ReactionGame::new();
        assert_eq!(game.result().difficulty, SESSION_DIFFICULTY);
        answer_and_wait(&mut game);
        assert_eq!(game.result().difficulty, SESSION_DIFFICULTY);
    }

    #[test]
    fn questions_use_parameters_of_their_position_in_session() {
        // 何セッションか通して遊び、何問目にどの色・表記が出たかを集める
        let mut colors: Vec<Vec<Color>> = vec![Vec::new(); 20];
        let mut katakana = [false; 20];
        for _ in 0..40 {
            let mut game = ReactionGame::new();
            for index in 0..20 {
                assert_eq!(game.tracker.total(), index as u32);
                let q = &game.current;
                colors[index].push(q.display_color);
                colors[index].push(lookup(q.label).0.color);
                if lookup(q.label).1 == Notation::Katakana {
                    katakana[index] = true;
                }
                answer_and_wait(&mut game);
            }
            assert!(game.is_finished());
        }
        for (index, difficulty) in EXPECTED_DIFFICULTIES.iter().enumerate() {
            let pool = color_pool(*difficulty);
            assert!(
                colors[index].iter().all(|c| pool.iter().any(|p| p.color == *c)),
                "{}問目は{difficulty:?}の色だけを使う",
                index + 1
            );
            // 初級/中級は漢字のみ、上級は漢字/カタカナ混在
            assert_eq!(
                katakana[index],
                *difficulty == Difficulty::Advanced,
                "{}問目のカタカナ",
                index + 1
            );
        }
        // 区間が進むと使う色が増える(初級2色→中級4色→上級9色)
        let seen = |range: std::ops::Range<usize>| {
            let mut seen: Vec<Color> = Vec::new();
            for c in colors[range].iter().flatten() {
                if !seen.contains(c) {
                    seen.push(*c);
                }
            }
            seen.len()
        };
        assert_eq!(seen(0..6), 2);
        assert_eq!(seen(6..13), 4);
        assert_eq!(seen(13..20), 9);
    }

    #[test]
    fn time_limit_applies_only_from_fourteenth_question() {
        let mut game = ReactionGame::new();
        for index in 0..13 {
            // 1〜13問目は時間制限が無く、待っても自動で不正解にならない
            game.update(Duration::from_secs(10));
            assert_eq!(game.tracker.total(), index, "{}問目は時間制限なし", index + 1);
            answer_and_wait(&mut game);
        }
        // 14問目からは3秒で自動的に不正解になり次の問題へ進む
        game.update(Duration::from_secs(3));
        assert_eq!(game.tracker.total(), 14, "14問目は時間切れ");
    }

    #[test]
    fn time_limit_bar_is_shown_only_in_advanced_questions() {
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        let screen_text = |game: &ReactionGame| {
            let (buffer, _) = render_game(game, 60, 20);
            let text: String = buffer.content().iter().map(|c| c.symbol()).collect();
            text.replace(' ', "")
        };
        let game = ReactionGame::new();
        assert!(!screen_text(&game).contains("残り"), "初級相当では残り時間を出さない");
        let game = advanced_game();
        assert!(screen_text(&game).contains("残り"), "上級相当では残り時間を出す");
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
        let mut game = ReactionGame::new();
        let area = Rect::new(0, 0, 40, 10);
        let (_, footer_area) = split_areas(area);
        game.current.is_match = true;
        game.handle_mouse(left_click(footer_area.x, footer_area.y), area);
        let result = game.tracker.to_result(GAME_ID, SESSION_DIFFICULTY);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_right_half_answers_mismatch() {
        let mut game = ReactionGame::new();
        let area = Rect::new(0, 0, 40, 10);
        let (_, footer_area) = split_areas(area);
        game.current.is_match = false;
        let right_column = footer_area.x + footer_area.width - 1;
        game.handle_mouse(left_click(right_column, footer_area.y), area);
        let result = game.tracker.to_result(GAME_ID, SESSION_DIFFICULTY);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_outside_footer_area_does_nothing() {
        let mut game = ReactionGame::new();
        let area = Rect::new(0, 0, 40, 10);
        let (label_area, _) = split_areas(area);
        game.handle_mouse(left_click(label_area.x, label_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }

    // ---- 出題の表示(背景色で色を示し、文字は黒の大きな画像) ----

    #[test]
    fn all_label_images_are_embedded() {
        for entry in PALETTE.iter() {
            let kanji = load_label_image(entry.kanji)
                .unwrap_or_else(|| panic!("「{}」の画像が埋め込まれていること", entry.kanji));
            assert_eq!(kanji.dimensions(), (512, 512), "漢字は正方形");
            let katakana = load_label_image(entry.katakana)
                .unwrap_or_else(|| panic!("「{}」の画像が埋め込まれていること", entry.katakana));
            assert_eq!(katakana.height(), 512);
            assert!(katakana.width() > katakana.height(), "カタカナは横長");
        }
    }

    #[test]
    fn label_maps_to_its_image_file() {
        let expected = [
            ("赤", "aka.png"),
            ("青", "ao.png"),
            ("緑", "midori.png"),
            ("黄", "ki.png"),
            ("紫", "murasaki.png"),
            ("白", "shiro.png"),
            ("茶", "cha.png"),
            ("橙", "daidai.png"),
            ("桃", "momo.png"),
            ("レッド", "reddo.png"),
            ("ブルー", "buruu.png"),
            ("グリーン", "guriin.png"),
            ("イエロー", "ieroo.png"),
            ("パープル", "paapuru.png"),
            ("ホワイト", "howaito.png"),
            ("ブラウン", "buraun.png"),
            ("オレンジ", "orenji.png"),
            ("ピンク", "pinku.png"),
        ];
        for (label, file) in expected {
            assert_eq!(label_image_file(label), Some(file), "「{label}」");
        }
        assert_eq!(label_image_file("黒"), None);
        assert!(load_label_image("黒").is_none());
    }

    #[test]
    fn every_pool_color_has_distinct_background_rgb() {
        let rgbs: Vec<[u8; 3]> = PALETTE.iter().map(|p| background_rgb(p.color)).collect();
        for (i, a) in rgbs.iter().enumerate() {
            for b in &rgbs[i + 1..] {
                assert_ne!(a, b, "色ごとに違う背景色になる");
            }
        }
    }

    #[test]
    fn background_rgb_is_bright_enough_for_black_text() {
        for entry in PALETTE.iter() {
            let [r, g, b] = background_rgb(entry.color);
            let luma = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
            // 既存で一番暗い赤(約88)以上なら黒い文字が読める
            assert!(
                luma >= 85.0,
                "{}の背景 {:?} は明るさ{luma:.0}",
                entry.kanji,
                [r, g, b]
            );
        }
    }

    #[test]
    fn composed_label_image_is_opaque_background_with_black_glyph() {
        let glyph = load_label_image("緑").unwrap();
        let bg = [10, 200, 30];
        let image = compose_glyph_image(&glyph, 20, 10, (10, 20), bg);
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

    /// 出題文字・正誤の記号の両方を、指定した画像プロトコルで描くようにする
    fn set_picker(game: &mut ReactionGame, picker: Picker) {
        game.label_renderer = LabelRenderer::with_picker(Some(picker.clone()));
        game.mark_renderer = MarkRenderer::with_picker(Some(picker));
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
        let mut game = ReactionGame::new();
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
            let mut game = advanced_game();
            set_picker(&mut game, picker);
            assert!(game.label_renderer.uses_image());
            // 全18文字を描くのはHalfblocksだけにし(画像のエンコードが重いため)、他のプロトコルは
            // 画像の形(正方形・3文字カタカナ・4文字カタカナ)ごとの代表で確かめる
            let labels: Vec<(&'static str, Color)> = if protocol == ProtocolType::Halfblocks {
                PALETTE
                    .iter()
                    .flat_map(|p| {
                        [Notation::Kanji, Notation::Katakana].map(|n| (p.label(n), p.color))
                    })
                    .collect()
            } else {
                vec![
                    ("紫", Color::Magenta),
                    ("ピンク", Color::White),
                    ("オレンジ", Color::Red),
                ]
            };
            for (label, color) in labels {
                fixed_question(&mut game, label, color);
                render_game(&game, 60, 20);
                // 極端に小さい画面でもパニックしない
                render_game(&game, 4, 4);
                render_game(&game, 12, 9);
            }
        }
    }

    #[test]
    fn image_mode_paints_background_rgb_around_glyph() {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        let mut game = ReactionGame::new();
        set_picker(&mut game, picker);
        fixed_question(&mut game, "青", Color::Red);
        let (buffer, inner) = render_game(&game, 60, 20);
        let [r, g, b] = background_rgb(Color::Red);
        // 文字画像の外(枠の内側の左端)は背景色で塗られている
        assert_eq!(buffer[(inner.x, inner.y)].bg, Color::Rgb(r, g, b));
    }

    #[test]
    fn image_mode_draws_katakana_in_wide_area() {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        let mut game = advanced_game();
        set_picker(&mut game, picker);
        let size = |game: &ReactionGame| {
            let cache = game.label_renderer.cache.borrow();
            let area = cache.as_ref().expect("画像で描かれていること").area;
            (u32::from(area.width) * 10, u32::from(area.height) * 20)
        };

        fixed_question(&mut game, "ブラウン", Color::White);
        let (buffer, inner) = render_game(&game, 80, 24);
        let (width, height) = size(&game);
        assert!(
            width > height * 2,
            "横長の画像は横長の範囲に描く({width}x{height})"
        );
        let [r, g, b] = background_rgb(Color::White);
        assert_eq!(buffer[(inner.x, inner.y)].bg, Color::Rgb(r, g, b));

        // 漢字に切り替えると正方形の範囲に戻る
        fixed_question(&mut game, "茶", Color::White);
        render_game(&game, 80, 24);
        let (width, height) = size(&game);
        assert!(
            width.abs_diff(height) < 20,
            "正方形の画像は正方形の範囲({width}x{height})"
        );
    }

    #[test]
    fn fallback_shows_katakana_label_as_text() {
        let mut game = advanced_game();
        fixed_question(&mut game, "パープル", Color::Yellow);
        let (buffer, inner) = render_game(&game, 40, 16);
        let text: String = (inner.y..inner.bottom())
            .flat_map(|y| (inner.x..inner.right()).map(move |x| (x, y)))
            .map(|pos| buffer[pos].symbol().to_string())
            .collect();
        assert!(
            text.contains('パ') && text.contains('ル'),
            "カタカナの文字が描かれる"
        );
    }

    #[test]
    fn fallback_render_does_not_panic_on_tiny_area() {
        let game = advanced_game();
        render_game(&game, 4, 4);
        render_game(&game, 1, 1);
    }

    // ---- 問題数(20問) ----

    #[test]
    fn session_finishes_after_twenty_questions() {
        let mut game = ReactionGame::new();
        for i in 0..SESSION_LENGTH - 1 {
            game.handle_key(KeyEvent::from(KeyCode::Left));
            game.update(crate::game::feedback::FEEDBACK_HOLD);
            assert!(!game.is_finished(), "{}問目では終わらない", i + 1);
        }
        game.handle_key(KeyEvent::from(KeyCode::Left));
        assert!(game.is_finished(), "20問で終わる");
        assert_eq!(game.result().total, 20);
        // 終わった後の入力は記録しない
        game.handle_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(game.result().total, 20);
    }

    #[test]
    fn session_length_is_twenty() {
        assert_eq!(SESSION_LENGTH, 20);
        let game = ReactionGame::new();
        assert_eq!(game.tracker.session_length(), 20);
    }

    #[test]
    fn hud_shows_progress_out_of_twenty() {
        let mut game = ReactionGame::new();
        for _ in 0..5 {
            game.handle_key(KeyEvent::from(KeyCode::Left));
            game.update(crate::game::feedback::FEEDBACK_HOLD);
        }
        let (buffer, _) = render_game(&game, 60, 20);
        let text: String = buffer.content().iter().map(|c| c.symbol()).collect();
        // 問題番号は2桁幅で右寄せする
        assert!(text.contains("Q 6/20"), "HUDは20問中の番号を出す");
    }

    // ---- 正誤の効果音 ----

    #[test]
    fn verdict_se_uses_common_correct_and_incorrect_sounds() {
        assert_eq!(verdict_se(true), SeKind::Correct);
        assert_eq!(verdict_se(false), SeKind::Incorrect);
    }

    // ---- 正誤の表示(出題文字の代わりに◯/✗) ----

    /// 出題エリアの内側の文字を1本の文字列にする
    fn inner_text(buffer: &ratatui::buffer::Buffer, inner: Rect) -> String {
        (inner.y..inner.bottom())
            .flat_map(|y| (inner.x..inner.right()).map(move |x| (x, y)))
            .map(|pos| buffer[pos].symbol().to_string())
            .collect()
    }

    /// 出題を一致の問題にして答え、正誤表示中の状態にする
    fn answer(game: &mut ReactionGame, correct: bool) {
        game.current.is_match = true;
        game.handle_key(KeyEvent::from(if correct {
            KeyCode::Left
        } else {
            KeyCode::Right
        }));
        assert!(game.feedback.current().is_some());
    }

    #[test]
    fn displayed_mark_is_shown_only_while_feedback_is_shown() {
        let mut game = ReactionGame::new();
        assert_eq!(game.displayed_mark(), None, "通常は出題文字");

        answer(&mut game, true);
        assert_eq!(game.displayed_mark(), Some(true), "正解は◯");

        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
        assert_eq!(game.displayed_mark(), None, "表示後は出題文字に戻る");

        answer(&mut game, false);
        assert_eq!(game.displayed_mark(), Some(false), "不正解は✗");
    }

    #[test]
    fn marks_are_not_label_images_of_this_game() {
        // ◯/✗の画像はmark_displayが持つので、出題文字の画像としては引かない
        assert_eq!(label_image_file(CORRECT_MARK), None);
        assert_eq!(label_image_file(INCORRECT_MARK), None);
    }

    #[test]
    fn fallback_shows_correct_mark_instead_of_label() {
        let mut game = ReactionGame::new();
        fixed_question(&mut game, "赤", Color::Blue);
        answer(&mut game, true);
        let (buffer, inner) = render_game(&game, 40, 16);
        let text = inner_text(&buffer, inner);
        assert!(text.contains(CORRECT_MARK), "◯が描かれる: {text:?}");
        assert!(!text.contains('赤'), "出題文字は描かない");
        let cell = (inner.y..inner.bottom())
            .flat_map(|y| (inner.x..inner.right()).map(move |x| (x, y)))
            .map(|pos| &buffer[pos])
            .find(|c| c.symbol() == CORRECT_MARK)
            .unwrap();
        assert_eq!(cell.fg, Color::Black, "記号も黒");
        // 回答後に次の問題へ切り替わっていても、背景は回答した瞬間の色のまま
        assert_eq!(buffer[(inner.x, inner.y)].bg, Color::Blue);
    }

    #[test]
    fn fallback_shows_incorrect_mark_instead_of_label() {
        let mut game = ReactionGame::new();
        fixed_question(&mut game, "青", Color::Red);
        answer(&mut game, false);
        let (buffer, inner) = render_game(&game, 40, 16);
        let text = inner_text(&buffer, inner);
        assert!(text.contains(INCORRECT_MARK), "✗が描かれる: {text:?}");
        assert!(!text.contains('青'), "出題文字は描かない");
        assert_eq!(buffer[(inner.x, inner.y)].bg, Color::Red);
    }

    #[test]
    fn fallback_returns_to_label_after_feedback_hold() {
        let mut game = ReactionGame::new();
        answer(&mut game, true);
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        fixed_question(&mut game, "赤", Color::Blue);
        let (buffer, inner) = render_game(&game, 40, 16);
        let text = inner_text(&buffer, inner);
        assert!(text.contains('赤'), "出題文字に戻る");
        assert!(!text.contains(CORRECT_MARK) && !text.contains(INCORRECT_MARK));
    }

    #[test]
    fn image_mode_shows_mark_image_with_question_background() {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        let mut game = ReactionGame::new();
        set_picker(&mut game, picker);
        let cached_label = |game: &ReactionGame| {
            let cache = game.label_renderer.cache.borrow();
            let cache = cache.as_ref().expect("画像で描かれていること");
            (cache.label, cache.bg)
        };

        for correct in [true, false] {
            fixed_question(&mut game, "青", Color::Red);
            answer(&mut game, correct);
            let (buffer, inner) = render_game(&game, 60, 20);
            let rgb = background_rgb(Color::Red);
            let (cached_correct, cached_bg, drawn) =
                game.mark_renderer.cached_mark().expect("記号が画像で描かれていること");
            assert_eq!(
                (cached_correct, cached_bg),
                (correct, rgb),
                "正誤の記号を回答した瞬間の背景色で描く"
            );
            assert!(drawn.x >= inner.x && drawn.right() <= inner.right());
            assert!(drawn.y >= inner.y && drawn.bottom() <= inner.bottom());
            let [r, g, b] = rgb;
            assert_eq!(buffer[(inner.x, inner.y)].bg, Color::Rgb(r, g, b));

            // 表示時間が過ぎたら、次の問題の画像に戻る(回答時点の色に引き戻されない)
            fixed_question(&mut game, "緑", Color::Green);
            game.update(crate::game::feedback::FEEDBACK_HOLD);
            render_game(&game, 60, 20);
            assert_eq!(cached_label(&game), ("緑", background_rgb(Color::Green)));
        }
    }

    #[test]
    fn mark_render_does_not_panic_with_image_protocols() {
        for protocol in [
            ProtocolType::Halfblocks,
            ProtocolType::Sixel,
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
        ] {
            let mut picker = Picker::from_fontsize((10, 20));
            picker.set_protocol_type(protocol);
            let mut game = ReactionGame::new();
            set_picker(&mut game, picker);
            for correct in [true, false] {
                answer(&mut game, correct);
                render_game(&game, 60, 20);
                render_game(&game, 4, 4);
                render_game(&game, 12, 9);
            }
        }
    }
}
