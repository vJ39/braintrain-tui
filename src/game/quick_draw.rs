//! ハヤウチ(quick_draw): いつ来るかわからない合図を待ち、合図が出た瞬間に反応する。
//! 仕様は docs/quick-draw-spec.md・docs/quick-draw-fixed-rounds-spec.md

use std::cell::RefCell;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use image::{DynamicImage, RgbaImage};
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::mark_display::{compose_glyph_image, glyph_area, MarkRenderer};
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};
use crate::ui::countdown::{self, CountdownState};

pub const GAME_ID: &str = "quick_draw";

/// 1セッションのラウンド数
pub const ROUNDS_PER_SESSION: u32 = 10;

/// 結果に記録する難易度・HUDに出す難易度。ハヤウチは難易度を選ばないので代表値にする
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;

/// 各ラウンドで超短時間パターンを選ぶ確率
pub const VERY_SHORT_RATE: f64 = 0.2;

/// フェイントを入れる対象になるのは、この番目(0始まり)以降のラウンド(後半のみ)
pub const FEINT_MIN_ROUND_INDEX: u32 = ROUNDS_PER_SESSION / 2;
/// 対象ラウンドでフェイントを入れる確率
pub const FEINT_RATE: f64 = 0.5;
/// フェイント表示の持続時間
pub const FEINT_DURATION: Duration = Duration::from_millis(300);
/// フェイントの背景色。本物の合図(SIGNAL_BG、明るい緑)と紛らわしいが、
/// 見比べれば分かる程度にくすんだ黄緑にする
pub const FEINT_BG: Color = Color::Rgb(150, 180, 40);

/// 待機中の背景(暗いグレー)
pub const WAITING_BG: Color = Color::Rgb(48, 48, 48);
/// 合図の背景(明るい緑)
pub const SIGNAL_BG: Color = Color::Rgb(0, 230, 64);
/// 待機中の表示
pub const WAITING_TEXT: &str = "まだ待て";
/// 合図の表示
pub const SIGNAL_TEXT: &str = "撃て!";
/// フェイントの表示。本物の合図(SIGNAL_TEXT)とは違う文言にし、それでも一瞬で見分けにくい
/// 紛らわしさは背景色(FEINT_BG)で出す
pub const FEINT_TEXT: &str = "撃つな";

/// 合図の文字と文字の間の区切り。全角スペースで間隔を広げ、目立つ見た目にする
const SIGNAL_TEXT_GAP: &str = "　";

/// 見出し用に、文字間を広げた文字列を作る("撃　て　！"のように)
fn spaced_headline(text: &str) -> String {
    text.chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(SIGNAL_TEXT_GAP)
}

/// 合図表示用に、文字間を広げた見出し文字列("撃　て　！")を作る
fn signal_headline() -> String {
    spaced_headline(SIGNAL_TEXT)
}

/// フェイント表示用に、文字間を広げた見出し文字列を作る
fn feint_headline() -> String {
    spaced_headline(FEINT_TEXT)
}

/// 合図(「撃て!」)の画像。黒い文字・透明背景の正方形
const SIGNAL_PNG: &[u8] = include_bytes!("../../assets/image/quick_draw/utte.png");
/// フェイント(「撃つな」)の画像。本物の合図と同じフォントサイズで見分けられるようにする
const FEINT_PNG: &[u8] = include_bytes!("../../assets/image/quick_draw/utsuna.png");

/// 見出しの画像(bytes)を読み込む。読めない場合はNone
fn load_glyph_image(bytes: &[u8]) -> Option<RgbaImage> {
    let image = image::load_from_memory(bytes).ok()?;
    Some(image.to_rgba8())
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

/// 直前に作った合図の画像。背景色・描画範囲が同じなら再エンコードを省く
struct SignalCache {
    bg: [u8; 3],
    area: Rect,
    protocol: StatefulProtocol,
}

/// 見出し画像(「撃て!」・「撃つな」)の描画器。画像プロトコルが使える端末では画像で大きく表示する。
/// 表示する画像(image_bytes)を指定して作るので、本物の合図・フェイントで共用できる
struct SignalRenderer {
    picker: Option<Picker>,
    image_bytes: &'static [u8],
    /// 直前に読み込んだ画像。描画範囲の計算に画像の寸法が要るので、
    /// 毎フレームPNGを読み直さないよう持っておく
    glyph: RefCell<Option<RgbaImage>>,
    cache: RefCell<Option<SignalCache>>,
}

impl SignalRenderer {
    fn new(image_bytes: &'static [u8]) -> Self {
        Self::with_picker(detect_picker(), image_bytes)
    }

    /// 画像プロトコルを指定して作る(None=テキスト表示)。テストで使う
    fn with_picker(picker: Option<Picker>, image_bytes: &'static [u8]) -> Self {
        Self {
            picker,
            image_bytes,
            glyph: RefCell::new(None),
            cache: RefCell::new(None),
        }
    }

    /// 画像プロトコルを使うか(false=テキスト表示)。テストでの確認用
    #[cfg(test)]
    fn uses_image(&self) -> bool {
        self.picker.is_some()
    }

    /// areaの中央に見出しの画像を描く。画像プロトコルが使えない/画像が読めない/
    /// 描く場所が無い場合は何もせずfalse(呼び出し側がテキスト表示に切り替える)
    fn render(&self, frame: &mut Frame, area: Rect, background: Color) -> bool {
        let Color::Rgb(r, g, b) = background else {
            return false;
        };
        let Some(picker) = &self.picker else {
            return false;
        };
        let mut glyph_cache = self.glyph.borrow_mut();
        if glyph_cache.is_none() {
            let Some(image) = load_glyph_image(self.image_bytes) else {
                return false;
            };
            *glyph_cache = Some(image);
        }
        let Some(glyph) = glyph_cache.as_ref() else {
            return false;
        };
        let drawn = glyph_area(area, picker.font_size(), glyph.dimensions());
        if drawn.is_empty() {
            return false;
        }
        let bg = [r, g, b];
        let mut cache = self.cache.borrow_mut();
        let needs_regen =
            !matches!(cache.as_ref(), Some(cached) if cached.bg == bg && cached.area == drawn);
        if needs_regen {
            let composed =
                compose_glyph_image(glyph, drawn.width, drawn.height, picker.font_size(), bg);
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(composed));
            *cache = Some(SignalCache {
                bg,
                area: drawn,
                protocol,
            });
        }
        let Some(cached) = cache.as_mut() else {
            return false;
        };
        frame.render_stateful_widget(StatefulImage::default(), drawn, &mut cached.protocol);
        true
    }

    /// 直前に画像で描いた範囲。テストで描画内容を確かめる用
    #[cfg(test)]
    fn cached_area(&self) -> Option<Rect> {
        self.cache.borrow().as_ref().map(|cached| cached.area)
    }
}

/// 押した後の結果(◯/✗)を表示し続ける時間。この間は次のラウンドへ進まない
const RESULT_HOLD: Duration = Duration::from_millis(1000);

/// 結果表示(◯/✗)のエリアを塗る色。画像表示の時は画像の背景と周りのセルを同じ色で塗れる
/// ようRGBにし、テキスト表示の時は端末の名前付き色にする。記号は黒なので、黒が読みやすい
/// 明るさの緑(成功)/赤(フライング)にする
fn result_background(is_correct: bool, uses_image: bool) -> Color {
    match (is_correct, uses_image) {
        (true, true) => Color::Rgb(40, 190, 70),
        (false, true) => Color::Rgb(230, 50, 50),
        (true, false) => Color::Green,
        (false, false) => Color::Red,
    }
}

/// 合図までの待機時間のパターン。ラウンドごとにランダムで選ぶ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitPattern {
    /// 通常の待機時間
    Normal,
    /// 超短時間。合図がほぼ待たずに来る
    VeryShort,
}

impl WaitPattern {
    /// 待機時間の範囲(最小, 最大)
    pub fn wait_range(self) -> (Duration, Duration) {
        let (min_ms, max_ms) = match self {
            WaitPattern::Normal => (800, 4000),
            WaitPattern::VeryShort => (200, 500),
        };
        (Duration::from_millis(min_ms), Duration::from_millis(max_ms))
    }

    /// フライング時に反応時間の代わりに記録する固定ペナルティ値(ms)。
    /// このパターンの最大待機時間と同じ値にする
    pub fn fail_latency_ms(self) -> f64 {
        let (_, max) = self.wait_range();
        max.as_millis() as f64
    }
}

/// ラウンドの待機時間パターンをランダムに選ぶ(超短時間はVERY_SHORT_RATEの確率)
fn random_pattern(rng: &mut impl Rng) -> WaitPattern {
    if rng.gen_bool(VERY_SHORT_RATE) {
        WaitPattern::VeryShort
    } else {
        WaitPattern::Normal
    }
}

/// 待機時間をパターンの範囲内からランダムに決める
fn random_wait(rng: &mut impl Rng, pattern: WaitPattern) -> Duration {
    let (min, max) = pattern.wait_range();
    Duration::from_millis(rng.gen_range(min.as_millis() as u64..=max.as_millis() as u64))
}

/// この待機時間remainingのラウンドにフェイントを入れるか決める。
/// 後半(round_index >= FEINT_MIN_ROUND_INDEX)のラウンドのみ対象で、フェイント表示
/// (FEINT_DURATION)を挟んでも合図までまだ間が残る(remaining >= FEINT_DURATION * 2)ことが条件
fn should_feint(round_index: u32, remaining: Duration, rng: &mut impl Rng) -> bool {
    round_index >= FEINT_MIN_ROUND_INDEX
        && remaining >= FEINT_DURATION * 2
        && rng.gen_bool(FEINT_RATE)
}

/// 待機時間remainingの中で、フェイントを始めるまでの残り時間(合図までの残り時間基準)を決める。
/// 待機の30%が過ぎた頃から、フェイントが終わってもまだ合図まで間がある範囲でランダムに選ぶ
fn feint_delay(rng: &mut impl Rng, remaining: Duration) -> Duration {
    let low = remaining.mul_f64(0.3).as_millis() as u64;
    let high = (remaining - FEINT_DURATION).as_millis() as u64;
    Duration::from_millis(rng.gen_range(low..=high.max(low)))
}

/// ラウンド内の状態
enum Phase {
    /// ラウンド冒頭の「3.2.1.GO!!」。終わると合図待ちへ進む
    Countdown { state: CountdownState },
    /// 合図待ち。残りの待機時間と、フェイントを始めるまでの残り時間
    /// (Noneならこのラウンドはフェイント無し)
    Waiting {
        remaining: Duration,
        feint_at: Option<Duration>,
    },
    /// フェイント表示中。本物の合図と同じ見出しを紛らわしい色で一瞬出し、フライングを誘う。
    /// 終わったらresume_waitingの残り時間でWaitingへ戻る(1ラウンドにつき最大1回)
    Feint {
        remaining: Duration,
        resume_waiting: Duration,
    },
    /// 合図が出ている。合図が出た時刻
    Signal { shown_at: Instant },
    /// 押した後の結果表示。成功時のみ反応時間(ms)を持つ。
    /// この表示が終わるまで次のラウンドへは進まず、入力も受け付けない
    Result {
        is_correct: bool,
        latency_ms: Option<f64>,
        elapsed: Duration,
    },
}

pub struct QuickDrawGame {
    tracker: ScoreTracker,
    phase: Phase,
    /// 現在のラウンドの待機時間パターン(ラウンド開始時に選ぶ)
    pattern: WaitPattern,
    /// 直前のラウンドの結果表示(HUD用の小さい表示)
    feedback: AnswerFeedback,
    /// 結果表示(◯/✗の大表示)の描画器
    mark_renderer: MarkRenderer,
    /// 合図(「撃て!」)の描画器
    signal_renderer: SignalRenderer,
    /// フェイント(「撃つな」)の描画器
    feint_renderer: SignalRenderer,
}

impl QuickDrawGame {
    pub fn new() -> Self {
        let mut game = Self {
            tracker: ScoreTracker::with_session_length(ROUNDS_PER_SESSION),
            phase: Phase::Countdown {
                state: CountdownState::new(),
            },
            pattern: WaitPattern::Normal,
            feedback: AnswerFeedback::new(),
            mark_renderer: MarkRenderer::new(),
            signal_renderer: SignalRenderer::new(SIGNAL_PNG),
            feint_renderer: SignalRenderer::new(FEINT_PNG),
        };
        game.start_round();
        game
    }

    /// 新しいラウンドを始める。待機時間パターンを選び直し、カウントダウンから始める
    fn start_round(&mut self) {
        self.pattern = random_pattern(&mut rand::thread_rng());
        let state = CountdownState::new();
        // 最初のフェーズ「3」の音
        if let Some(phase) = state.phase() {
            audio::play_se(phase.se());
        }
        self.phase = Phase::Countdown { state };
    }

    /// キー/クリックで押された時の処理。カウントダウン中の入力は受け付けない(無視する)。
    /// 待機中ならフライング、合図後なら反応時間を記録する。押した後はまず結果表示(◯/✗)に入り、
    /// 結果表示中の入力は無視する(次のラウンドへの誤入力を防ぐ)
    fn press(&mut self) {
        if self.is_finished() {
            return;
        }
        let (is_correct, latency_ms) = match &self.phase {
            Phase::Countdown { .. } | Phase::Result { .. } => return,
            Phase::Waiting { .. } | Phase::Feint { .. } => {
                self.tracker.record(false, self.pattern.fail_latency_ms());
                self.feedback.record(false, "フライング");
                audio::play_se(SeKind::Incorrect);
                (false, None)
            }
            Phase::Signal { shown_at } => {
                let latency_ms = shown_at.elapsed().as_millis() as f64;
                self.tracker.record(true, latency_ms);
                self.feedback.record(true, format!("{latency_ms:.0}ms"));
                audio::play_se(SeKind::Correct);
                (true, Some(latency_ms))
            }
        };
        self.phase = Phase::Result {
            is_correct,
            latency_ms,
            elapsed: Duration::ZERO,
        };
    }

    fn render_board(&self, frame: &mut Frame, area: Rect) {
        if let Phase::Countdown { state } = &self.phase {
            countdown::render(frame, area, state);
            return;
        }
        if let Phase::Result {
            is_correct,
            latency_ms,
            ..
        } = &self.phase
        {
            self.render_result(frame, area, *is_correct, *latency_ms);
            return;
        }
        let (background, headline, text_color, show_hint) = match &self.phase {
            Phase::Waiting { .. } => (WAITING_BG, WAITING_TEXT.to_string(), theme::TEXT, true),
            // 合図は文字間を広げて単独表示し、操作説明を消して見出しだけに注目を集める
            Phase::Signal { .. } => (SIGNAL_BG, signal_headline(), Color::Black, false),
            // フェイントは本物の合図と紛らわしい色(FEINT_BG)だが、文言は「撃つな」で見分けられる
            Phase::Feint { .. } => (FEINT_BG, feint_headline(), Color::Black, false),
            Phase::Countdown { .. } | Phase::Result { .. } => unreachable!("上で処理済み"),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(background))
            .style(Style::default().bg(background));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // フェイントも本物の合図と同じフォントサイズ(画像表示)にする。専用画像(FEINT_PNG)を使う
        let image_renderer = match self.phase {
            Phase::Signal { .. } => Some(&self.signal_renderer),
            Phase::Feint { .. } => Some(&self.feint_renderer),
            _ => None,
        };
        if let Some(renderer) = image_renderer {
            if renderer.render(frame, inner, background) {
                return;
            }
        }

        let mut lines = vec![Line::from(Span::styled(
            headline,
            Style::default().fg(text_color).add_modifier(Modifier::BOLD),
        ))];
        if show_hint {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "合図が出たら Enter / Space / クリック",
                Style::default().fg(text_color),
            )));
        }
        let text_area = theme::vertical_center(inner, lines.len() as u16);
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            text_area,
        );
    }

    /// 押した後の結果表示。成功時は大きな◯の下に反応時間(ms)を表示し、
    /// フライング時は大きな✗を単独で表示する
    fn render_result(
        &self,
        frame: &mut Frame,
        area: Rect,
        is_correct: bool,
        latency_ms: Option<f64>,
    ) {
        let background = result_background(is_correct, self.mark_renderer.uses_image());
        let Some(latency_ms) = latency_ms else {
            self.mark_renderer
                .render(frame, area, is_correct, background);
            return;
        };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(area);
        self.mark_renderer
            .render(frame, rows[0], is_correct, background);
        let footer = Block::default().style(Style::default().bg(background));
        let inner = footer.inner(rows[1]);
        frame.render_widget(footer, rows[1]);
        let line = Line::from(Span::styled(
            format!("{latency_ms:.0}ms"),
            Style::default()
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(
            Paragraph::new(line).alignment(Alignment::Center),
            theme::vertical_center(inner, 1),
        );
    }
}

impl Default for QuickDrawGame {
    fn default() -> Self {
        Self::new()
    }
}

impl Game for QuickDrawGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
            self.press();
        }
    }

    /// 画面のどこをクリックしても反応する(エリアの判定はしない)
    fn handle_mouse(&mut self, mouse: MouseEvent, _area: Rect) {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            self.press();
        }
    }

    fn update(&mut self, dt: Duration) {
        if self.is_finished() {
            return;
        }
        self.feedback.tick(dt);
        match &mut self.phase {
            Phase::Countdown { state } => {
                if let Some(phase) = state.tick(dt) {
                    audio::play_se(phase.se());
                }
                if state.is_finished() {
                    let mut rng = rand::thread_rng();
                    let remaining = random_wait(&mut rng, self.pattern);
                    let feint_at = should_feint(self.tracker.total(), remaining, &mut rng)
                        .then(|| feint_delay(&mut rng, remaining));
                    self.phase = Phase::Waiting { remaining, feint_at };
                }
            }
            Phase::Waiting { remaining, feint_at } => {
                let feint_triggers = matches!(feint_at, Some(t) if t.saturating_sub(dt).is_zero());
                if feint_triggers {
                    self.phase = Phase::Feint {
                        remaining: FEINT_DURATION,
                        resume_waiting: remaining.saturating_sub(dt),
                    };
                } else {
                    let new_feint_at = feint_at.map(|t| t.saturating_sub(dt));
                    let new_remaining = remaining.saturating_sub(dt);
                    self.phase = if new_remaining.is_zero() {
                        // 反応時間は合図が画面に出た時刻から測る
                        Phase::Signal {
                            shown_at: Instant::now(),
                        }
                    } else {
                        Phase::Waiting {
                            remaining: new_remaining,
                            feint_at: new_feint_at,
                        }
                    };
                }
            }
            Phase::Feint {
                remaining,
                resume_waiting,
            } => {
                let remaining = remaining.saturating_sub(dt);
                self.phase = if remaining.is_zero() {
                    Phase::Waiting {
                        remaining: *resume_waiting,
                        feint_at: None,
                    }
                } else {
                    Phase::Feint {
                        remaining,
                        resume_waiting: *resume_waiting,
                    }
                };
            }
            Phase::Signal { .. } => {}
            Phase::Result { elapsed, .. } => {
                *elapsed += dt;
                if *elapsed >= RESULT_HOLD && !self.tracker.is_session_finished() {
                    self.start_round();
                }
            }
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud_area, board_area) = theme::split_hud(area);
        theme::render_hud_with_session_length(
            frame,
            hud_area,
            "ハヤウチ",
            SESSION_DIFFICULTY,
            self.tracker.total(),
            self.tracker.session_length(),
            &self.feedback,
        );
        self.render_board(frame, board_area);
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::countdown::{self as countdown_ui, PHASE_DURATION};
    use crossterm::event::KeyModifiers;
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    const ALL_PATTERNS: [WaitPattern; 2] = [WaitPattern::Normal, WaitPattern::VeryShort];
    const AREA: Rect = Rect::new(0, 0, 60, 20);
    /// 1ラウンドのカウントダウン全体の長さ(3/2/1/GO!!の4フェーズ)
    const COUNTDOWN_TOTAL: Duration = Duration::from_millis(2400);

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn left_click(column: u16, row: u16) -> MouseEvent {
        mouse(MouseEventKind::Down(MouseButton::Left), column, row)
    }

    /// 合図が出てからelapsed経過した状態にする
    fn show_signal_since(game: &mut QuickDrawGame, elapsed: Duration) {
        let shown_at = Instant::now()
            .checked_sub(elapsed)
            .expect("テスト環境のInstantはelapsed分さかのぼれる");
        game.phase = Phase::Signal { shown_at };
    }

    /// ラウンド冒頭のカウントダウンを最後まで進め、合図待ちにする
    fn finish_countdown(game: &mut QuickDrawGame) {
        assert!(is_countdown(game), "カウントダウン中のはず");
        game.update(COUNTDOWN_TOTAL);
        assert!(
            matches!(game.phase, Phase::Waiting { .. }),
            "カウントダウンが終わったら合図待ち"
        );
    }

    fn is_countdown(game: &QuickDrawGame) -> bool {
        matches!(game.phase, Phase::Countdown { .. })
    }

    /// 押した直後の結果表示(◯/✗)を最後まで進め、次のラウンドのカウントダウンにする。
    /// セッションが最終ラウンドで終わっている場合は結果表示のまま(次には進まない)
    fn finish_result(game: &mut QuickDrawGame) {
        assert!(
            matches!(game.phase, Phase::Result { .. }),
            "結果表示中のはず"
        );
        game.update(RESULT_HOLD);
    }

    fn countdown_phase(game: &QuickDrawGame) -> Option<countdown_ui::Phase> {
        match &game.phase {
            Phase::Countdown { state } => state.phase(),
            _ => panic!("カウントダウン中のはず"),
        }
    }

    fn assert_waiting_within_range(game: &QuickDrawGame) {
        let (min, max) = game.pattern.wait_range();
        match game.phase {
            Phase::Waiting { remaining, .. } => assert!(
                (min..=max).contains(&remaining),
                "待機時間{remaining:?}が範囲{min:?}〜{max:?}に入っていない"
            ),
            _ => panic!("待機状態のはず"),
        }
    }

    fn rendered(game: &QuickDrawGame) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(AREA.width, AREA.height)).unwrap();
        terminal.draw(|frame| game.render(frame, AREA)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// 全セルを空白抜きの文字列にする(全角文字の2セル目は空白で埋まるため)
    fn text_of(buffer: &Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace(' ', "")
    }

    /// HUDより下の、合図を表示するパネル内側の中央付近のセル背景
    fn body_center_bg(buffer: &Buffer) -> Color {
        let (_, body) = crate::game::theme::split_hud(AREA);
        buffer[(body.x + body.width / 2, body.y + body.height - 2)].bg
    }

    // --- 待機時間パターン ---

    #[test]
    fn wait_range_matches_spec_for_each_pattern() {
        assert_eq!(WaitPattern::Normal.wait_range(), (ms(800), ms(4000)));
        assert_eq!(WaitPattern::VeryShort.wait_range(), (ms(200), ms(500)));
    }

    #[test]
    fn random_wait_stays_within_range_for_each_pattern() {
        let mut rng = StdRng::seed_from_u64(7);
        for pattern in ALL_PATTERNS {
            let (min, max) = pattern.wait_range();
            let waits: Vec<Duration> = (0..500).map(|_| random_wait(&mut rng, pattern)).collect();
            assert!(
                waits.iter().all(|w| (min..=max).contains(w)),
                "{pattern:?}: 範囲外の待機時間がある"
            );
            // 毎回同じ値ではなくランダムに散らばる(範囲の半分以上に広がる)
            let shortest = waits.iter().min().unwrap();
            let longest = waits.iter().max().unwrap();
            assert!(
                *longest - *shortest > (max - min) / 2,
                "{pattern:?}: 待機時間がばらついていない"
            );
        }
    }

    #[test]
    fn very_short_pattern_appears_about_20_percent() {
        let mut rng = StdRng::seed_from_u64(11);
        let draws = 2000;
        let very_short = (0..draws)
            .filter(|_| random_pattern(&mut rng) == WaitPattern::VeryShort)
            .count();
        // 10問中に体感できる頻度(目安20%)。乱数のゆれを見込んで15〜25%に収まること
        assert!(
            (draws * 15 / 100..=draws * 25 / 100).contains(&very_short),
            "超短時間パターンの出現数: {very_short}/{draws}"
        );
        assert_eq!(VERY_SHORT_RATE, 0.2);
    }

    #[test]
    fn fail_latency_is_the_longest_wait_of_each_pattern() {
        // フライングのペナルティは、そのラウンドのパターンの最大待機時間と同じ値にする
        for pattern in ALL_PATTERNS {
            let (_, max) = pattern.wait_range();
            assert_eq!(pattern.fail_latency_ms(), max.as_millis() as f64);
        }
        assert_eq!(WaitPattern::Normal.fail_latency_ms(), 4000.0);
        assert_eq!(WaitPattern::VeryShort.fail_latency_ms(), 500.0);
    }

    #[test]
    fn session_difficulty_is_advanced() {
        assert_eq!(SESSION_DIFFICULTY, Difficulty::Advanced);
    }

    // --- 開始時のカウントダウンと合図への切り替え ---

    #[test]
    fn new_game_starts_with_countdown_from_three() {
        let game = QuickDrawGame::new();
        assert!(is_countdown(&game), "最初のラウンドはカウントダウンから");
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Three));
        assert!(!game.is_finished());
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn countdown_keeps_going_until_all_phases_elapse() {
        let mut game = QuickDrawGame::new();
        game.update(PHASE_DURATION);
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Two));
        game.update(PHASE_DURATION);
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::One));
        game.update(PHASE_DURATION);
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Go));
        game.update(PHASE_DURATION - ms(1));
        assert!(
            is_countdown(&game),
            "GO!!の表示時間が終わるまではカウントダウン"
        );
        game.update(ms(1));
        assert_waiting_within_range(&game);
        assert_eq!(game.tracker.total(), 0, "カウントダウンだけでは記録しない");
    }

    #[test]
    fn countdown_finishes_into_waiting_within_pattern_range() {
        for _ in 0..50 {
            let mut game = QuickDrawGame::new();
            finish_countdown(&mut game);
            assert_waiting_within_range(&game);
        }
    }

    #[test]
    fn update_switches_to_signal_after_wait_elapses() {
        let mut game = QuickDrawGame::new();
        game.phase = Phase::Waiting {
            remaining: ms(100),
            feint_at: None,
        };
        game.update(ms(60));
        assert!(
            matches!(game.phase, Phase::Waiting { remaining, .. } if remaining == ms(40)),
            "待機時間が経過分だけ減る"
        );
        game.update(ms(60));
        assert!(
            matches!(game.phase, Phase::Signal { .. }),
            "待機時間を過ぎたら合図"
        );
    }

    #[test]
    fn signal_has_no_timeout() {
        // 合図後は押すまで待つ(時間切れで勝手に次のラウンドへ進まない)
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(0));
        game.update(Duration::from_secs(60));
        assert!(matches!(game.phase, Phase::Signal { .. }));
        assert_eq!(game.tracker.total(), 0);
    }

    // --- 反応(合図後) ---

    #[test]
    fn enter_after_signal_records_reaction_time() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(300));
        game.handle_key(key(KeyCode::Enter));
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
        assert!(
            (300.0..400.0).contains(&result.avg_latency_ms),
            "合図からの経過時間が記録される: {}",
            result.avg_latency_ms
        );
    }

    #[test]
    fn space_after_signal_also_counts_as_reaction() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(200));
        game.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(game.result().correct, 1);
    }

    #[test]
    fn reacting_starts_next_round_with_countdown() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(250));
        game.handle_key(key(KeyCode::Enter));
        assert!(
            matches!(
                game.phase,
                Phase::Result {
                    is_correct: true,
                    ..
                }
            ),
            "反応直後はまず結果表示"
        );
        finish_result(&mut game);
        assert!(is_countdown(&game), "次のラウンドもカウントダウンから");
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Three));
        finish_countdown(&mut game);
        assert_waiting_within_range(&game);
    }

    #[test]
    fn reacting_shows_reaction_time_in_feedback() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(250));
        game.handle_key(key(KeyCode::Enter));
        let flash = game.feedback.current().expect("反応直後は結果を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Correct);
        assert!(
            flash.detail.contains("ms"),
            "反応時間を表示: {}",
            flash.detail
        );
    }

    // --- フライング(合図前) ---

    #[test]
    fn key_while_waiting_is_false_start_with_pattern_penalty() {
        for pattern in ALL_PATTERNS {
            let mut game = QuickDrawGame::new();
            finish_countdown(&mut game);
            game.pattern = pattern;
            game.handle_key(key(KeyCode::Enter));
            let result = game.result();
            assert_eq!(result.total, 1, "{pattern:?}");
            assert_eq!(result.correct, 0, "{pattern:?}: フライングは失敗");
            assert_eq!(result.avg_latency_ms, pattern.fail_latency_ms());
        }
    }

    #[test]
    fn key_during_countdown_is_ignored() {
        // カウントダウン中の入力は受け付けない(フライングにもしない)
        for pattern in ALL_PATTERNS {
            let mut game = QuickDrawGame::new();
            game.pattern = pattern;
            game.update(PHASE_DURATION * 3); // GO!!の表示中
            assert!(is_countdown(&game));
            game.handle_key(key(KeyCode::Char(' ')));
            assert!(is_countdown(&game), "{pattern:?}: カウントダウンが続く");
            let result = game.result();
            assert_eq!(result.total, 0, "{pattern:?}: 記録されない");
            assert!(
                game.feedback.current().is_none(),
                "{pattern:?}: フィードバックも出さない"
            );
        }
    }

    #[test]
    fn click_during_countdown_is_ignored() {
        let mut game = QuickDrawGame::new();
        assert!(is_countdown(&game));
        game.handle_mouse(left_click(10, 10), AREA);
        assert!(is_countdown(&game), "カウントダウンが続く");
        let result = game.result();
        assert_eq!(result.total, 0, "記録されない");
    }

    #[test]
    fn click_while_waiting_is_false_start() {
        let mut game = QuickDrawGame::new();
        finish_countdown(&mut game);
        game.pattern = WaitPattern::Normal;
        game.handle_mouse(left_click(10, 10), AREA);
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 0);
        assert_eq!(result.avg_latency_ms, WaitPattern::Normal.fail_latency_ms());
    }

    #[test]
    fn false_start_starts_next_round_with_countdown_and_shows_feedback() {
        let mut game = QuickDrawGame::new();
        finish_countdown(&mut game);
        game.handle_key(key(KeyCode::Char(' ')));
        assert!(
            matches!(
                game.phase,
                Phase::Result {
                    is_correct: false,
                    ..
                }
            ),
            "フライング直後はまず結果表示(✗)"
        );
        let flash = game
            .feedback
            .current()
            .expect("フライング直後は結果を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert!(flash.detail.contains("フライング"), "{}", flash.detail);
        finish_result(&mut game);
        assert!(
            is_countdown(&game),
            "フライング後も次のラウンドのカウントダウンへ進む"
        );
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Three));
        finish_countdown(&mut game);
        assert_waiting_within_range(&game);
    }

    // --- フェイント ---

    #[test]
    fn feint_is_never_scheduled_in_the_first_half() {
        let mut rng = rand::thread_rng();
        for round in 0..FEINT_MIN_ROUND_INDEX {
            for _ in 0..50 {
                assert!(
                    !should_feint(round, WaitPattern::Normal.wait_range().1, &mut rng),
                    "{round}問目(前半)はフェイントを入れない"
                );
            }
        }
    }

    #[test]
    fn feint_can_be_scheduled_in_the_second_half() {
        let mut rng = rand::thread_rng();
        let remaining = WaitPattern::Normal.wait_range().1;
        let scheduled = (0..200).any(|_| should_feint(FEINT_MIN_ROUND_INDEX, remaining, &mut rng));
        assert!(scheduled, "後半のラウンドではフェイントが入ることがある");
    }

    #[test]
    fn feint_is_not_scheduled_when_the_wait_is_too_short_to_fit_it() {
        let mut rng = rand::thread_rng();
        let too_short = FEINT_DURATION * 2 - Duration::from_millis(1);
        for _ in 0..50 {
            assert!(
                !should_feint(FEINT_MIN_ROUND_INDEX, too_short, &mut rng),
                "フェイントを挟む余地が無い待機時間では入れない"
            );
        }
    }

    #[test]
    fn feint_delay_stays_within_the_waiting_window() {
        let mut rng = rand::thread_rng();
        let remaining = ms(2000);
        for _ in 0..100 {
            let delay = feint_delay(&mut rng, remaining);
            assert!(delay <= remaining - FEINT_DURATION, "{delay:?}");
            assert!(delay >= remaining.mul_f64(0.3) - ms(1), "{delay:?}");
        }
    }

    #[test]
    fn pressing_during_feint_is_a_false_start() {
        let mut game = QuickDrawGame::new();
        game.phase = Phase::Feint {
            remaining: ms(100),
            resume_waiting: ms(500),
        };
        game.handle_key(key(KeyCode::Enter));
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 0, "フェイントに引っかかるとフライング扱い");
        assert!(matches!(game.phase, Phase::Result { is_correct: false, .. }));
    }

    #[test]
    fn feint_ends_and_resumes_waiting_then_reaches_signal() {
        let mut game = QuickDrawGame::new();
        game.phase = Phase::Feint {
            remaining: ms(50),
            resume_waiting: ms(80),
        };
        game.update(ms(50));
        assert!(
            matches!(game.phase, Phase::Waiting { remaining, feint_at: None } if remaining == ms(80)),
            "フェイントが終わったら残りの待機時間でWaitingへ戻る"
        );
        game.update(ms(80));
        assert!(
            matches!(game.phase, Phase::Signal { .. }),
            "戻った待機を過ぎたら合図が出る"
        );
    }

    #[test]
    fn feint_background_differs_from_the_real_signal() {
        assert_ne!(FEINT_BG, SIGNAL_BG);
        let mut game = QuickDrawGame::new();
        game.phase = Phase::Feint {
            remaining: ms(100),
            resume_waiting: ms(500),
        };
        assert_eq!(body_center_bg(&rendered(&game)), FEINT_BG);
    }

    #[test]
    fn feint_shows_its_own_text_not_the_real_signal_text() {
        assert_ne!(FEINT_TEXT, SIGNAL_TEXT);
        let mut game = QuickDrawGame::new();
        game.phase = Phase::Feint {
            remaining: ms(100),
            resume_waiting: ms(500),
        };
        let text = text_of(&rendered(&game));
        for c in FEINT_TEXT.chars() {
            assert!(text.contains(c), "{text}");
        }
        assert!(
            !text.contains(SIGNAL_TEXT),
            "フェイントは本物の合図の文字を出さない: {text}"
        );
    }

    #[test]
    fn late_round_can_transition_from_waiting_into_feint() {
        // 後半のラウンドまで進めた上で、フェイントが予定された状態を直接作り、
        // 時間経過でFeintフェーズへ切り替わることを確認する
        let mut game = QuickDrawGame::new();
        for _ in 0..FEINT_MIN_ROUND_INDEX {
            game.tracker.record(true, 0.0);
        }
        game.phase = Phase::Waiting {
            remaining: ms(1000),
            feint_at: Some(ms(50)),
        };
        game.update(ms(60));
        assert!(
            matches!(game.phase, Phase::Feint { .. }),
            "フェイントの予定時刻を過ぎたらFeintへ切り替わる"
        );
    }

    #[test]
    fn false_starts_never_end_the_session_early() {
        // フライングはMISSとして数えるだけで、GAME OVERにはならない。10問全部フライングでも最後まで進む
        let mut game = QuickDrawGame::new();
        for round in 0..ROUNDS_PER_SESSION {
            assert!(!game.is_finished(), "{round}問目の前は終わっていない");
            finish_countdown(&mut game);
            game.handle_key(key(KeyCode::Enter));
            assert_eq!(game.tracker.total(), round + 1);
            assert_eq!(game.result().correct, 0, "フライングでは正答数が増えない");
            if round + 1 < ROUNDS_PER_SESSION {
                finish_result(&mut game);
            }
        }
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(result.total, ROUNDS_PER_SESSION);
        assert_eq!(result.correct, 0);
    }

    // --- 入力の種類 ---

    #[test]
    fn other_keys_are_ignored() {
        let mut game = QuickDrawGame::new();
        let ignored = [
            KeyCode::Left,
            KeyCode::Char('a'),
            KeyCode::Esc,
            KeyCode::Tab,
        ];
        for code in ignored {
            game.handle_key(key(code));
        }
        assert_eq!(
            game.tracker.total(),
            0,
            "カウントダウン中でも対象外のキーはフライングにしない"
        );
        finish_countdown(&mut game);
        for code in ignored {
            game.handle_key(key(code));
        }
        assert_eq!(
            game.tracker.total(),
            0,
            "待機中でも対象外のキーはフライングにしない"
        );
        show_signal_since(&mut game, ms(100));
        game.handle_key(key(KeyCode::Char('x')));
        assert_eq!(game.tracker.total(), 0);
        assert!(matches!(game.phase, Phase::Signal { .. }));
    }

    #[test]
    fn click_anywhere_after_signal_counts_as_reaction() {
        // HUDの上でも、画面の端でも、エリアの外でも反応する
        let positions = [
            (0, 0),
            (AREA.width / 2, AREA.height / 2),
            (AREA.width - 1, AREA.height - 1),
            (AREA.width + 10, AREA.height + 10),
        ];
        for (column, row) in positions {
            let mut game = QuickDrawGame::new();
            show_signal_since(&mut game, ms(200));
            game.handle_mouse(left_click(column, row), AREA);
            assert_eq!(game.result().correct, 1, "({column},{row})のクリック");
        }
    }

    #[test]
    fn non_left_press_mouse_events_are_ignored() {
        let mut game = QuickDrawGame::new();
        for kind in [
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Moved,
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::ScrollDown,
        ] {
            game.handle_mouse(mouse(kind, 5, 5), AREA);
        }
        assert_eq!(game.tracker.total(), 0);
    }

    // --- セッション ---

    #[test]
    fn session_has_ten_rounds() {
        assert_eq!(ROUNDS_PER_SESSION, 10);
    }

    #[test]
    fn session_finishes_after_ten_rounds() {
        let mut game = QuickDrawGame::new();
        for round in 0..ROUNDS_PER_SESSION {
            assert!(!game.is_finished(), "{round}ラウンド目の前は終わっていない");
            finish_countdown(&mut game);
            show_signal_since(&mut game, ms(200));
            game.handle_key(key(KeyCode::Enter));
            if round + 1 < ROUNDS_PER_SESSION {
                finish_result(&mut game);
            }
        }
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.difficulty, SESSION_DIFFICULTY);
        assert_eq!(result.total, 10);
        assert_eq!(result.correct, 10);
    }

    #[test]
    fn mixed_session_averages_reaction_and_penalty() {
        let mut game = QuickDrawGame::new();
        game.pattern = WaitPattern::Normal;
        finish_countdown(&mut game);
        game.handle_key(key(KeyCode::Enter)); // フライング(ペナルティ4000ms)
        show_signal_since(&mut game, ms(300));
        game.handle_key(key(KeyCode::Enter));
        show_signal_since(&mut game, ms(300));
        game.handle_mouse(left_click(1, 1), AREA);
        let result = game.result();
        assert_eq!(result.total, 3);
        assert_eq!(result.correct, 2);
        // (4000 + 300 + 300) / 3 ≒ 1533
        assert!(
            (1533.0..1600.0).contains(&result.avg_latency_ms),
            "{}",
            result.avg_latency_ms
        );
    }

    #[test]
    fn input_after_session_finished_is_ignored() {
        let mut game = QuickDrawGame::new();
        for round in 0..ROUNDS_PER_SESSION {
            finish_countdown(&mut game);
            game.handle_key(key(KeyCode::Enter));
            if round + 1 < ROUNDS_PER_SESSION {
                finish_result(&mut game);
            }
        }
        assert!(game.is_finished());
        game.handle_key(key(KeyCode::Enter));
        game.handle_mouse(left_click(1, 1), AREA);
        game.update(Duration::from_secs(10));
        assert_eq!(game.result().total, ROUNDS_PER_SESSION);
    }

    // --- 結果表示(押した後) ---

    #[test]
    fn pressing_after_signal_shows_result_with_correct_and_latency() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(250));
        game.handle_key(key(KeyCode::Enter));
        match game.phase {
            Phase::Result {
                is_correct: true,
                latency_ms: Some(ms),
                ..
            } => {
                assert!((250.0..350.0).contains(&ms), "反応時間を記録する: {ms}");
            }
            _ => panic!("成功後は結果表示(◯・反応時間あり)のはず"),
        }
    }

    #[test]
    fn pressing_before_signal_shows_result_with_incorrect_and_no_latency() {
        let mut game = QuickDrawGame::new();
        finish_countdown(&mut game);
        game.handle_key(key(KeyCode::Enter));
        assert!(
            matches!(
                game.phase,
                Phase::Result {
                    is_correct: false,
                    latency_ms: None,
                    ..
                }
            ),
            "フライング後は結果表示(✗・反応時間なし)のはず"
        );
    }

    #[test]
    fn result_phase_holds_before_result_hold_elapses() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(0));
        game.handle_key(key(KeyCode::Enter));
        game.update(RESULT_HOLD - ms(1));
        assert!(
            matches!(game.phase, Phase::Result { .. }),
            "表示時間が経つまでは結果表示のまま"
        );
    }

    #[test]
    fn result_phase_advances_to_next_countdown_after_result_hold() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(0));
        game.handle_key(key(KeyCode::Enter));
        game.update(RESULT_HOLD);
        assert!(is_countdown(&game), "表示時間が経つと次のラウンドへ進む");
    }

    #[test]
    fn press_during_result_phase_is_ignored() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(0));
        game.handle_key(key(KeyCode::Enter));
        assert_eq!(game.tracker.total(), 1);
        // 結果表示中に押しても、次の入力としては扱わない(記録が増えない)
        game.handle_key(key(KeyCode::Enter));
        game.handle_mouse(left_click(1, 1), AREA);
        assert_eq!(game.tracker.total(), 1);
        assert!(matches!(game.phase, Phase::Result { .. }));
    }

    #[test]
    fn render_shows_big_correct_mark_and_latency_ms_text() {
        use crate::game::mark_display::CORRECT_MARK;
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(250));
        game.handle_key(key(KeyCode::Enter));
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(text.contains(CORRECT_MARK), "成功時は大きな◯: {text}");
        assert!(text.contains("ms"), "反応時間(ms)を表示する: {text}");
    }

    #[test]
    fn render_shows_big_incorrect_mark_without_latency_text() {
        use crate::game::mark_display::INCORRECT_MARK;
        let mut game = QuickDrawGame::new();
        finish_countdown(&mut game);
        game.handle_key(key(KeyCode::Enter));
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(
            text.contains(INCORRECT_MARK),
            "フライング時は大きな✗: {text}"
        );
        assert!(!text.contains("ms"), "反応時間は表示しない: {text}");
    }

    #[test]
    fn render_does_not_panic_during_result_in_tiny_area() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(0));
        game.handle_key(key(KeyCode::Enter));
        for (width, height) in [(1, 1), (5, 2), (10, 4)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| game.render(frame, Rect::new(0, 0, width, height)))
                .unwrap();
        }
    }

    // --- 描画 ---

    #[test]
    fn countdown_renders_big_countdown_glyph() {
        let game = QuickDrawGame::new();
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(text.contains('█'), "カウントダウンの大きな文字を描く");
        assert!(
            !text.contains(WAITING_TEXT),
            "カウントダウン中は「まだ待て」を出さない"
        );
        assert!(!text.contains(SIGNAL_TEXT));
    }

    #[test]
    fn waiting_renders_gray_background_and_wait_text() {
        let mut game = QuickDrawGame::new();
        finish_countdown(&mut game);
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(text.contains(WAITING_TEXT), "待機中は「まだ待て」");
        assert!(!text.contains(SIGNAL_TEXT));
        assert_eq!(body_center_bg(&buffer), WAITING_BG);
    }

    #[test]
    fn signal_renders_green_background_and_now_text() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(0));
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        for c in SIGNAL_TEXT.chars() {
            assert!(text.contains(c), "合図後は「撃て!」の文字を含む: {c}");
        }
        assert!(!text.contains(WAITING_TEXT));
        assert_eq!(body_center_bg(&buffer), SIGNAL_BG);
    }

    #[test]
    fn signal_headline_spaces_out_each_character() {
        assert_eq!(signal_headline(), "撃　て　!");
    }

    #[test]
    fn signal_text_is_shown_in_black_and_bold() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(0));
        let buffer = rendered(&game);
        let cell = (0..buffer.area().height)
            .flat_map(|y| (0..buffer.area().width).map(move |x| (x, y)))
            .map(|pos| &buffer[pos])
            .find(|c| c.symbol() == "撃")
            .expect("「撃」が描かれること");
        assert_eq!(cell.fg, Color::Black, "合図の文字は黒");
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "合図の文字は太字で強調する"
        );
    }

    #[test]
    fn signal_hides_the_hint_line_to_emphasize_the_headline() {
        let mut game = QuickDrawGame::new();
        show_signal_since(&mut game, ms(0));
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(
            !text.contains("Enter"),
            "合図表示中は見出しだけを目立たせ、操作説明は出さない"
        );
    }

    // --- 合図の画像表示 ---

    fn halfblocks_picker() -> Picker {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        picker
    }

    /// 画像プロトコルを固定したPickerで、合図とフェイントの描画器を作り直す
    fn use_picker(game: &mut QuickDrawGame, protocol: ProtocolType) {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(protocol);
        game.signal_renderer = SignalRenderer::with_picker(Some(picker.clone()), SIGNAL_PNG);
        game.feint_renderer = SignalRenderer::with_picker(Some(picker), FEINT_PNG);
    }

    #[test]
    fn signal_image_asset_is_embedded_and_decodes() {
        let image = load_glyph_image(SIGNAL_PNG).expect("「撃て!」の画像が埋め込まれていること");
        assert_eq!(image.dimensions(), (512, 512), "正方形");
    }

    #[test]
    fn signal_renderer_without_picker_falls_back_to_text() {
        let renderer = SignalRenderer::with_picker(None, SIGNAL_PNG);
        assert!(!renderer.uses_image());
        let mut terminal = Terminal::new(TestBackend::new(20, 10)).unwrap();
        let mut drawn_as_image = false;
        terminal
            .draw(|frame| {
                drawn_as_image = renderer.render(frame, frame.area(), SIGNAL_BG);
            })
            .unwrap();
        assert!(!drawn_as_image, "画像プロトコルが無ければテキストに任せる");
    }

    #[test]
    fn signal_renderer_with_picker_draws_the_image_inside_the_area() {
        let renderer = SignalRenderer::with_picker(Some(halfblocks_picker()), SIGNAL_PNG);
        assert!(renderer.uses_image());
        let area = Rect::new(2, 3, 40, 12);
        let mut terminal = Terminal::new(TestBackend::new(area.right(), area.bottom())).unwrap();
        let mut drawn_as_image = false;
        terminal
            .draw(|frame| {
                drawn_as_image = renderer.render(frame, area, SIGNAL_BG);
            })
            .unwrap();
        assert!(drawn_as_image, "画像プロトコルが使えれば画像で描く");
        let drawn = renderer.cached_area().expect("描画範囲が記録されること");
        assert_eq!(drawn.intersection(area), drawn, "areaの内側に描く");
    }

    #[test]
    fn game_renders_the_signal_as_an_image_when_available() {
        let mut game = QuickDrawGame::new();
        use_picker(&mut game, ProtocolType::Halfblocks);
        show_signal_since(&mut game, ms(0));
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(
            !text.contains(SIGNAL_TEXT),
            "画像で描く時は文字間を広げたテキスト見出しを出さない: {text}"
        );
    }

    #[test]
    fn feint_image_asset_is_embedded_and_decodes() {
        let image = load_glyph_image(FEINT_PNG).expect("「撃つな」の画像が埋め込まれていること");
        assert_eq!(image.dimensions(), (512, 512), "正方形");
    }

    #[test]
    fn game_renders_the_feint_as_an_image_with_the_same_size_as_the_signal() {
        let mut game = QuickDrawGame::new();
        use_picker(&mut game, ProtocolType::Halfblocks);
        game.phase = Phase::Feint {
            remaining: ms(100),
            resume_waiting: ms(500),
        };
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(
            !text.contains(FEINT_TEXT),
            "画像で描く時は文字間を広げたテキスト見出しを出さない: {text}"
        );
        assert!(
            game.feint_renderer.uses_image(),
            "フェイントも画像プロトコルが使えれば画像で描く"
        );
        assert_eq!(
            load_glyph_image(FEINT_PNG).unwrap().dimensions(),
            load_glyph_image(SIGNAL_PNG).unwrap().dimensions(),
            "フェイントと本物の合図は同じ画像サイズ(=同じフォントサイズ相当)で描く"
        );
    }

    #[test]
    fn waiting_still_shows_the_hint_line() {
        let mut game = QuickDrawGame::new();
        finish_countdown(&mut game);
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(text.contains("Enter"), "待機中は従来通り操作説明を出す");
    }

    #[test]
    fn render_switches_when_wait_elapses() {
        let mut game = QuickDrawGame::new();
        game.phase = Phase::Waiting {
            remaining: ms(50),
            feint_at: None,
        };
        assert_eq!(body_center_bg(&rendered(&game)), WAITING_BG);
        game.update(ms(50));
        let buffer = rendered(&game);
        assert_eq!(body_center_bg(&buffer), SIGNAL_BG);
        let text = text_of(&buffer);
        for c in SIGNAL_TEXT.chars() {
            assert!(text.contains(c));
        }
    }

    #[test]
    fn render_shows_hud_with_game_name_and_round_count() {
        let game = QuickDrawGame::new();
        let text = text_of(&rendered(&game));
        assert!(text.contains("ハヤウチ"), "HUDのタイトルはハヤウチ: {text}");
        assert!(!text.contains("反射神経"));
        assert!(text.contains("Q1/10"), "10問制の進捗: {text}");
    }

    #[test]
    fn game_id_stays_quick_draw() {
        // 名称をハヤウチに変えても、統計・履歴の互換のため内部識別子は変えない
        assert_eq!(GAME_ID, "quick_draw");
        assert_eq!(QuickDrawGame::new().result().game_id, "quick_draw");
    }

    #[test]
    fn render_does_not_panic_in_tiny_area() {
        let mut game = QuickDrawGame::new();
        for _ in 0..3 {
            for (width, height) in [(1, 1), (5, 2), (10, 4)] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| game.render(frame, Rect::new(0, 0, width, height)))
                    .unwrap();
            }
            // カウントダウン→待機→合図の各状態で描く
            if is_countdown(&game) {
                finish_countdown(&mut game);
            } else {
                show_signal_since(&mut game, ms(0));
            }
        }
    }
}
