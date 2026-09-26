//! ヤッホー(look_away): お茶漬け屋のカウンター越しに親父(白い割烹着)と対峙する。
//! 親父が指さして「ヤー!!」と叫んだら、その方向と逆の矢印キーを押して防御する。
//! 「やっほー」と言われたらSpaceで「やっほー」と返す。
//! どちらもRESPONSE_SAFE_WINDOW(400ms)以内に正しく反応すれば正解。
//! 400ms超過・誤入力・無反応(MAX_RESPONSE_WINDOWでタイムアウト確定)は不正解になり、
//! 遅れた分だけ複数個の♥を失う(penalty_for参照)。♥が0になったらGAME OVER。10問を終えたら勝利。
//!
//! 表示名(DISPLAY_NAME)は仮名称。改名はDISPLAY_NAMEを変えるだけで済むよう、
//! メニュー・HUD・テストはすべてこの定数を参照する。GAME_ID・モジュール名は
//! 統計・履歴の互換のため表示名とは独立させている

use std::cell::RefCell;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use image::imageops::FilterType;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::mark_display::MarkRenderer;
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};
use crate::ui::countdown::{self, CountdownState, GoSeOnce};
use crate::ui::splash;

/// メニュー・HUD・リザルトに出す表示名(仮名称)。改名する時はここだけを変える
pub const DISPLAY_NAME: &str = "ヤッホー";

/// 統計・履歴に記録する内部識別子。表示名を変えてもこれは変えない
pub const GAME_ID: &str = "look_away";

/// 結果に記録する難易度・HUDに出す難易度。難易度を選ばないので代表値にする
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;

/// 1セッションの問題数。正誤に関わらずこの数だけ出題する
pub const ROUNDS_PER_SESSION: u32 = 10;

/// セッション開始時の♥の数。0になったらGAME OVER
pub const MAX_LIVES: u32 = 5;

/// 待機(相手が何もしていない)の長さの範囲(最小, 最大)ms
pub const IDLE_WAIT_MS: (u64, u64) = (700, 2500);
/// これ以内に正しい入力ができれば正解(♥は減らない)
pub const RESPONSE_SAFE_WINDOW: Duration = Duration::from_millis(400);
/// RESPONSE_SAFE_WINDOWを超えた経過時間をこの単位で区切り、超過1区分ごとに♥をもう1つ失う
pub const PENALTY_STEP: Duration = Duration::from_millis(80);
/// 入力を受け付ける最大時間。これを過ぎても入力が無ければ自動的に不正解確定(経過時間はこの値として計算する)
pub const MAX_RESPONSE_WINDOW: Duration = Duration::from_millis(800);
/// 正誤の結果(◯/✗)を表示し続ける時間。この間は次の問題へ進まず、入力も受け付けない
pub const RESULT_HOLD: Duration = Duration::from_millis(1000);

/// 待機の後に「やっほー」イベントになる確率
pub const YAHHO_RATE: f64 = 0.25;

/// 相手の決め台詞。指さしと一緒にこれを叫んだら、逆を向く合図
pub const SHOUT_TEXT: &str = "ヤー!!";
/// 相手の呼びかけ。これを言われたらSpaceで返す
pub const YAHHO_CALL_TEXT: &str = "やっほー!";
/// 相手の顔(画像プロトコル非対応環境のフォールバック)
pub const FACE_TEXT: &str = "( ・ω・ )";
/// 左を指す腕
pub const POINT_LEFT_TEXT: &str = "◀━━━ ";
/// 右を指す腕
pub const POINT_RIGHT_TEXT: &str = " ━━━▶";
/// 時間切れの時のフィードバック
pub const TIMEOUT_TEXT: &str = "時間切れ";
/// 待機中に押した時のフィードバック
pub const FALSE_START_TEXT: &str = "フライング";
/// ライフが尽きた時の表示
pub const GAME_OVER_TEXT: &str = "GAME OVER";
/// 全問を終えてライフが残っていた時の表示
pub const CLEAR_TEXT: &str = "CLEAR!";

/// 待機中の背景(暗いグレー)
pub const IDLE_BG: Color = Color::Rgb(48, 48, 48);
/// 「ヤー!!」と叫んでいる時の背景(オレンジ)
pub const SHOUT_BG: Color = Color::Rgb(255, 140, 0);
/// 「やっほー」と言っている時の背景(空色)
pub const YAHHO_BG: Color = Color::Rgb(0, 170, 230);

/// 画像アセット(assets/image/からの相対パス)。無ければテキストで描く
pub const STAGE_NORMAL_IMAGE: &str = "look_away/normal.png";
/// 右を指して叫んでいる絵
pub const STAGE_SHOUT_RIGHT_IMAGE: &str = "look_away/shout_right.png";
/// 左を指して叫んでいる絵(shout_rightを左右反転したもの)
pub const STAGE_SHOUT_LEFT_IMAGE: &str = "look_away/shout_left.png";
/// 「やっほー」と呼びかけている絵
pub const STAGE_YAHHO_IMAGE: &str = "look_away/yahho.png";

/// 相手が指す向き
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// 逆の向き
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// この向きを向くキー
    pub fn key(self) -> KeyCode {
        match self {
            Side::Left => KeyCode::Left,
            Side::Right => KeyCode::Right,
        }
    }

    /// 左右をランダムに選ぶ
    fn random(rng: &mut impl Rng) -> Side {
        if rng.gen_bool(0.5) {
            Side::Left
        } else {
            Side::Right
        }
    }
}

/// 待機の後に起こすイベント(フェイントは廃止し、この2つだけ)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    /// 「やっほー」と言う
    Yahho,
    /// 指さして「ヤー!!」と叫ぶ
    Shout(Side),
}

/// 範囲(最小, 最大)msの中からランダムな長さを選ぶ
fn random_between(rng: &mut impl Rng, (min_ms, max_ms): (u64, u64)) -> Duration {
    Duration::from_millis(rng.gen_range(min_ms..=max_ms))
}

/// 待機の後に起こすイベントを選ぶ
fn choose_event(rng: &mut impl Rng) -> Event {
    if rng.gen_bool(YAHHO_RATE) {
        Event::Yahho
    } else {
        Event::Shout(Side::random(rng))
    }
}

/// ゲームで使うキー(←→Space)。それ以外のキーは無視する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Input {
    Turn(Side),
    Yahho,
}

impl Input {
    fn from_key(code: KeyCode) -> Option<Input> {
        if code == KeyCode::Char(' ') {
            return Some(Input::Yahho);
        }
        [Side::Left, Side::Right]
            .into_iter()
            .find(|side| side.key() == code)
            .map(Input::Turn)
    }
}

/// RESPONSE_SAFE_WINDOWを超えた経過時間から、失う♥の個数を求める。
/// 超過0〜79msで1個、80〜159msで2個、...とPENALTY_STEP刻みで増える
fn penalty_for(elapsed: Duration) -> u32 {
    let overage = elapsed.saturating_sub(RESPONSE_SAFE_WINDOW);
    1 + (overage.as_millis() / PENALTY_STEP.as_millis()) as u32
}

/// 1問の判定結果(正誤・HUDに出す説明・記録する反応時間ms・失う♥の個数)
struct Verdict {
    is_correct: bool,
    detail: String,
    latency_ms: f64,
    /// 失う♥の個数。正解なら常に0
    penalty: u32,
}

impl Verdict {
    fn correct(elapsed: Duration) -> Self {
        let latency_ms = elapsed.as_millis() as f64;
        Self {
            is_correct: true,
            detail: format!("{latency_ms:.0}ms"),
            latency_ms,
            penalty: 0,
        }
    }

    /// 不正解。elapsedはその判定が確定した時点での経過時間(誤入力ならその瞬間、
    /// 無反応ならMAX_RESPONSE_WINDOW)で、これがそのままpenalty個数の計算に使われる
    fn incorrect(detail: &str, elapsed: Duration) -> Self {
        Self {
            is_correct: false,
            detail: detail.to_string(),
            latency_ms: elapsed.as_millis() as f64,
            penalty: penalty_for(elapsed),
        }
    }
}

/// いまの状態で入力inputを受けた時の判定。判定しない状態(カウントダウン・結果表示)ならNone
fn judge(phase: &Phase, input: Input) -> Option<Verdict> {
    match (phase, input) {
        (Phase::Countdown { .. } | Phase::Result { .. }, _) => None,
        (Phase::Idle { .. }, _) => Some(Verdict::incorrect(FALSE_START_TEXT, Duration::ZERO)),
        (Phase::Shout { side, remaining }, Input::Turn(turned)) if turned == side.opposite() => {
            let elapsed = MAX_RESPONSE_WINDOW.saturating_sub(*remaining);
            if elapsed <= RESPONSE_SAFE_WINDOW {
                Some(Verdict::correct(elapsed))
            } else {
                Some(Verdict::incorrect("反応が遅い", elapsed))
            }
        }
        (Phase::Shout { remaining, .. }, Input::Turn(_)) => Some(Verdict::incorrect(
            "逆を向く",
            MAX_RESPONSE_WINDOW.saturating_sub(*remaining),
        )),
        (Phase::Shout { remaining, .. }, Input::Yahho) => Some(Verdict::incorrect(
            "向きで答える",
            MAX_RESPONSE_WINDOW.saturating_sub(*remaining),
        )),
        (Phase::Yahho { remaining }, Input::Yahho) => {
            let elapsed = MAX_RESPONSE_WINDOW.saturating_sub(*remaining);
            if elapsed <= RESPONSE_SAFE_WINDOW {
                Some(Verdict::correct(elapsed))
            } else {
                Some(Verdict::incorrect("反応が遅い", elapsed))
            }
        }
        (Phase::Yahho { remaining }, Input::Turn(_)) => Some(Verdict::incorrect(
            "Spaceで返す",
            MAX_RESPONSE_WINDOW.saturating_sub(*remaining),
        )),
    }
}

/// 問題内の状態
enum Phase {
    /// 問題冒頭の「3.2.1.GO!!」。この間の入力は受け付けない
    Countdown { state: CountdownState },
    /// 相手が何もしていない待機。残りの待機時間
    Idle { remaining: Duration },
    /// 指さして「ヤー!!」と叫んでいる。残りの入力受付時間(MAX_RESPONSE_WINDOWから減っていく)
    Shout { side: Side, remaining: Duration },
    /// 「やっほー」と言っている。残りの入力受付時間
    Yahho { remaining: Duration },
    /// 正誤の結果表示。この表示が終わるまで次の問題へは進まず、入力も受け付けない
    Result { is_correct: bool, elapsed: Duration },
}

pub struct LookAwayGame {
    tracker: ScoreTracker,
    phase: Phase,
    /// 残りの♥
    lives: u32,
    /// 最後の結果表示が終わってセッションを終えたか
    finished: bool,
    /// 直前の問題の結果表示(HUD用の小さい表示)
    feedback: AnswerFeedback,
    /// 結果表示(◯/✗の大表示)の描画器
    mark_renderer: MarkRenderer,
    /// カウントダウンの「GO!!」の音を1問目だけ鳴らすためのゲート
    go_se: GoSeOnce,
    /// 「3.2.1.GO!!」のカウントダウン演出自体を出したことがあるか。1問目だけ出し、
    /// 2問目以降は演出を挟まず直接待機から始める
    shown_countdown_once: bool,
    /// カウンター越しの親父の絵(通常/ヤー左右/やっほー)の描画器
    stage_renderer: StageRenderer,
}

/// 結果表示(◯/✗)のエリアを塗る色。画像表示の時は画像の背景と周りのセルを同じ色で塗れる
/// ようRGBにし、テキスト表示の時は端末の名前付き色にする(ハヤウチと同じ配色)
fn result_background(is_correct: bool, uses_image: bool) -> Color {
    match (is_correct, uses_image) {
        (true, true) => Color::Rgb(40, 190, 70),
        (false, true) => Color::Rgb(230, 50, 50),
        (true, false) => Color::Green,
        (false, false) => Color::Red,
    }
}

/// ライフをハートで表す(残り=♥、失った分=♡)
fn hearts(lives: u32) -> String {
    let lost = MAX_LIVES.saturating_sub(lives);
    "♥".repeat(lives as usize) + &"♡".repeat(lost as usize)
}

/// 相手の顔と、指している腕の行
fn pointing_line(side: Option<Side>) -> String {
    match side {
        None => FACE_TEXT.to_string(),
        Some(Side::Left) => format!("{POINT_LEFT_TEXT}{FACE_TEXT}"),
        Some(Side::Right) => format!("{FACE_TEXT}{POINT_RIGHT_TEXT}"),
    }
}

impl LookAwayGame {
    pub fn new() -> Self {
        let mut game = Self {
            tracker: ScoreTracker::with_session_length(ROUNDS_PER_SESSION),
            phase: Phase::Countdown {
                state: CountdownState::new(),
            },
            lives: MAX_LIVES,
            finished: false,
            feedback: AnswerFeedback::new(),
            mark_renderer: MarkRenderer::new(),
            go_se: GoSeOnce::new(),
            shown_countdown_once: false,
            stage_renderer: StageRenderer::new(),
        };
        game.start_round();
        game
    }

    /// ライフが尽きたか
    pub fn is_game_over(&self) -> bool {
        self.lives == 0
    }

    /// 全問を終えてライフが残っていたか(勝利)。最後の結果表示中からtrueになる
    pub fn is_cleared(&self) -> bool {
        self.tracker.is_session_finished() && !self.is_game_over()
    }

    /// 最後の結果表示が終わったらセッションを終えるか(ライフ切れ・全問終了)
    fn is_last_round(&self) -> bool {
        self.is_game_over() || self.tracker.is_session_finished()
    }

    /// 新しい問題を始める。1問目だけ「3.2.1.GO!!」の
    /// カウントダウンから始め、2問目以降は演出を挟まず直接待機から始める
    fn start_round(&mut self) {
        if self.shown_countdown_once {
            self.phase = Phase::Idle {
                remaining: random_between(&mut rand::thread_rng(), IDLE_WAIT_MS),
            };
            return;
        }
        let state = CountdownState::new();
        // 最初のフェーズ「3」の音
        if let Some(phase) = state.phase() {
            if let Some(se) = self.go_se.se_for(phase) {
                audio::play_se(se);
            }
        }
        self.phase = Phase::Countdown { state };
    }

    /// 待機が終わった時に、次のイベントを始める
    fn begin_event(&mut self, event: Event) {
        self.phase = match event {
            Event::Yahho => {
                audio::play_se(SeKind::LookAwayYahho);
                Phase::Yahho {
                    remaining: MAX_RESPONSE_WINDOW,
                }
            }
            Event::Shout(side) => {
                audio::play_se(SeKind::LookAwayShout);
                Phase::Shout {
                    side,
                    remaining: MAX_RESPONSE_WINDOW,
                }
            }
        };
    }

    /// 1問の正誤を記録して結果表示に入る。不正解ならverdict.penalty個ぶん♥を減らす
    fn finish_question(&mut self, verdict: Verdict) {
        self.tracker.record(verdict.is_correct, verdict.latency_ms);
        self.lives = self.lives.saturating_sub(verdict.penalty);
        self.feedback.record(verdict.is_correct, verdict.detail);
        audio::play_se(if verdict.is_correct {
            SeKind::Correct
        } else {
            SeKind::Incorrect
        });
        self.phase = Phase::Result {
            is_correct: verdict.is_correct,
            elapsed: Duration::ZERO,
        };
    }

    /// 時間経過で状態を進める。制限時間を過ぎた問題は不正解にする
    fn tick_phase(&mut self, dt: Duration) {
        match &mut self.phase {
            Phase::Countdown { state } => {
                if let Some(phase) = state.tick(dt) {
                    if let Some(se) = self.go_se.se_for(phase) {
                        audio::play_se(se);
                    }
                }
                if state.is_finished() {
                    self.shown_countdown_once = true;
                    self.phase = Phase::Idle {
                        remaining: random_between(&mut rand::thread_rng(), IDLE_WAIT_MS),
                    };
                }
            }
            Phase::Idle { remaining } => {
                *remaining = remaining.saturating_sub(dt);
                if remaining.is_zero() {
                    let event = choose_event(&mut rand::thread_rng());
                    self.begin_event(event);
                }
            }
            Phase::Shout { remaining, .. } => {
                *remaining = remaining.saturating_sub(dt);
                if remaining.is_zero() {
                    self.finish_question(Verdict::incorrect(TIMEOUT_TEXT, MAX_RESPONSE_WINDOW));
                }
            }
            Phase::Yahho { remaining } => {
                *remaining = remaining.saturating_sub(dt);
                if remaining.is_zero() {
                    self.finish_question(Verdict::incorrect(TIMEOUT_TEXT, MAX_RESPONSE_WINDOW));
                }
            }
            Phase::Result { elapsed, .. } => {
                *elapsed += dt;
                if *elapsed >= RESULT_HOLD {
                    if self.is_last_round() {
                        self.finished = true;
                    } else {
                        self.start_round();
                    }
                }
            }
        }
    }

    /// 相手(と合図)を描くステージ
    fn render_stage(&self, frame: &mut Frame, area: Rect) {
        match &self.phase {
            Phase::Countdown { state } => countdown::render(frame, area, state),
            Phase::Result { is_correct, .. } => self.render_result(frame, area, *is_correct),
            Phase::Idle { .. } => self.render_scene(
                frame,
                area,
                IDLE_BG,
                theme::TEXT,
                StageKind::Normal,
                vec![pointing_line(None)],
            ),
            Phase::Shout { side, .. } => {
                let kind = if *side == Side::Left {
                    StageKind::ShoutLeft
                } else {
                    StageKind::ShoutRight
                };
                self.render_scene(
                    frame,
                    area,
                    SHOUT_BG,
                    Color::Black,
                    kind,
                    vec![
                        pointing_line(Some(*side)),
                        String::new(),
                        SHOUT_TEXT.to_string(),
                    ],
                )
            }
            Phase::Yahho { .. } => self.render_scene(
                frame,
                area,
                YAHHO_BG,
                Color::Black,
                StageKind::Yahho,
                vec![
                    pointing_line(None),
                    String::new(),
                    YAHHO_CALL_TEXT.to_string(),
                ],
            ),
        }
    }

    /// カウンター越しの親父を描く。画像が揃っていれば画像、無ければテキストの絵
    fn render_scene(
        &self,
        frame: &mut Frame,
        area: Rect,
        background: Color,
        text_color: Color,
        kind: StageKind,
        texts: Vec<String>,
    ) {
        if self.stage_renderer.render(frame, area, kind) {
            return;
        }
        render_character(frame, area, background, text_color, texts);
    }

    /// 結果表示。大きな◯/✗を出し、最後の問題ならGAME OVER/CLEAR!を添える
    fn render_result(&self, frame: &mut Frame, area: Rect, is_correct: bool) {
        let background = result_background(is_correct, self.mark_renderer.uses_image());
        let ending = if self.is_game_over() {
            Some(GAME_OVER_TEXT)
        } else if self.is_cleared() {
            Some(CLEAR_TEXT)
        } else {
            None
        };
        let Some(ending) = ending else {
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
            ending,
            Style::default()
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(
            Paragraph::new(line).alignment(Alignment::Center),
            theme::vertical_center(inner, 1),
        );
    }

    /// ライフと操作説明のフッター
    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let line = Line::from(vec![
            Span::styled("ライフ ", Style::default().fg(theme::MUTED)),
            Span::styled(
                hearts(self.lives),
                Style::default()
                    .fg(theme::INCORRECT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   ← → で「ヤー!!」の逆を向く / Space で「やっほー」を返す",
                Style::default().fg(theme::MUTED),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(line)
                .alignment(Alignment::Center)
                .block(theme::sub_panel()),
            area,
        );
    }
}

/// 相手の顔(と腕・台詞)を背景色backgroundの中央に描く
fn render_character(
    frame: &mut Frame,
    area: Rect,
    background: Color,
    text_color: Color,
    texts: Vec<String>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(background))
        .style(Style::default().bg(background));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines: Vec<Line> = texts
        .into_iter()
        .map(|text| {
            Line::from(Span::styled(
                text,
                Style::default().fg(text_color).add_modifier(Modifier::BOLD),
            ))
        })
        .collect();
    let text_area = theme::vertical_center(inner, lines.len() as u16);
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        text_area,
    );
}

/// 端末の画像プロトコルを調べる。sixel/kitty/iTerm2のどれかが使える時だけSome。
/// テストでは端末に問い合わせず、常にテキスト表示にする
fn detect_picker() -> Option<Picker> {
    if cfg!(test) {
        return None;
    }
    splash::picker()
        .filter(|picker| {
            matches!(
                picker.protocol_type(),
                ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2
            )
        })
        .cloned()
}

/// カウンター越しの親父の絵の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageKind {
    Normal,
    ShoutLeft,
    ShoutRight,
    Yahho,
}

/// 親父の静止画4種(通常/ヤー左/ヤー右/やっほー)
struct StageImages {
    normal: StatefulProtocol,
    shout_left: StatefulProtocol,
    shout_right: StatefulProtocol,
    yahho: StatefulProtocol,
}

/// カウンター越しの親父の描画器。4種の静止画が全て読めた時だけ画像で描く
struct StageRenderer {
    images: Option<RefCell<StageImages>>,
}

impl StageRenderer {
    fn new() -> Self {
        let images = detect_picker().and_then(|picker| {
            let normal = splash::load_embedded_image(STAGE_NORMAL_IMAGE)?;
            let shout_right = splash::load_embedded_image(STAGE_SHOUT_RIGHT_IMAGE)?;
            let shout_left = splash::load_embedded_image(STAGE_SHOUT_LEFT_IMAGE)?;
            let yahho = splash::load_embedded_image(STAGE_YAHHO_IMAGE)?;
            Some(RefCell::new(StageImages {
                normal: picker.new_resize_protocol(normal),
                shout_left: picker.new_resize_protocol(shout_left),
                shout_right: picker.new_resize_protocol(shout_right),
                yahho: picker.new_resize_protocol(yahho),
            }))
        });
        Self { images }
    }

    /// 画像で描くか(false=テキストの絵)。テストでの確認用
    #[cfg(test)]
    fn uses_image(&self) -> bool {
        self.images.is_some()
    }

    /// areaに画像を描いたか(true=描いた、false=画像が無いので呼び出し元がテキストで描く)
    fn render(&self, frame: &mut Frame, area: Rect, kind: StageKind) -> bool {
        let Some(images) = &self.images else {
            return false;
        };
        if area.is_empty() {
            return true;
        }
        let mut images = images.borrow_mut();
        let protocol = match kind {
            StageKind::Normal => &mut images.normal,
            StageKind::ShoutLeft => &mut images.shout_left,
            StageKind::ShoutRight => &mut images.shout_right,
            StageKind::Yahho => &mut images.yahho,
        };
        let widget = StatefulImage::default().resize(Resize::Fit(Some(FilterType::Triangle)));
        frame.render_stateful_widget(widget, area, protocol);
        true
    }
}

impl Default for LookAwayGame {
    fn default() -> Self {
        Self::new()
    }
}

impl Game for LookAwayGame {
    /// ←→Spaceだけを受け付ける。カウントダウン中・結果表示中の入力は無視する
    fn handle_key(&mut self, key: KeyEvent) {
        if self.finished {
            return;
        }
        let Some(input) = Input::from_key(key.code) else {
            return;
        };
        if let Some(verdict) = judge(&self.phase, input) {
            self.finish_question(verdict);
        }
    }

    fn update(&mut self, dt: Duration) {
        if self.finished {
            return;
        }
        self.feedback.tick(dt);
        self.tick_phase(dt);
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud_area, body) = theme::split_hud(area);
        theme::render_hud_with_session_length(
            frame,
            hud_area,
            DISPLAY_NAME,
            SESSION_DIFFICULTY,
            self.tracker.total(),
            self.tracker.session_length(),
            &self.feedback,
        );
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(body);
        self.render_stage(frame, rows[0]);
        self.render_footer(frame, rows[1]);
    }

    fn is_finished(&self) -> bool {
        self.finished
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::mark_display::{CORRECT_MARK, INCORRECT_MARK};
    use crate::ui::countdown::{self as countdown_ui, PHASE_DURATION};
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    const AREA: Rect = Rect::new(0, 0, 60, 20);
    /// 1問のカウントダウン全体の長さ(3/2/1/GO!!の4フェーズ)
    const COUNTDOWN_TOTAL: Duration = Duration::from_millis(2400);
    const GAME_KEYS: [KeyCode; 3] = [KeyCode::Left, KeyCode::Right, KeyCode::Char(' ')];
    const SIDES: [Side; 2] = [Side::Left, Side::Right];

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn press(game: &mut LookAwayGame, code: KeyCode) {
        game.handle_key(KeyEvent::from(code));
    }

    fn is_countdown(game: &LookAwayGame) -> bool {
        matches!(game.phase, Phase::Countdown { .. })
    }

    fn is_result(game: &LookAwayGame, correct: bool) -> bool {
        matches!(game.phase, Phase::Result { is_correct, .. } if is_correct == correct)
    }

    /// 問題冒頭のカウントダウンを最後まで進め、待機にする。フェーズ(PHASE_DURATION)ごとに
    /// 分けて進める(実機のフレームループと同じく、一度に全部進めるとGO!!への遷移自体を
    /// 検出できずSEが鳴らないため)
    fn finish_countdown(game: &mut LookAwayGame) {
        assert!(is_countdown(game), "カウントダウン中のはず");
        for _ in 0..(COUNTDOWN_TOTAL.as_millis() / PHASE_DURATION.as_millis()) {
            game.update(PHASE_DURATION);
        }
        assert!(
            matches!(game.phase, Phase::Idle { .. }),
            "カウントダウンが終わったら待機"
        );
    }

    /// 結果表示を最後まで進める(最終問・GAME OVERならセッションが終わる)
    fn finish_result(game: &mut LookAwayGame) {
        assert!(
            matches!(game.phase, Phase::Result { .. }),
            "結果表示中のはず"
        );
        game.update(RESULT_HOLD);
    }

    fn shout(game: &mut LookAwayGame, side: Side) {
        game.phase = Phase::Shout {
            side,
            remaining: MAX_RESPONSE_WINDOW,
        };
    }

    fn yahho(game: &mut LookAwayGame) {
        game.phase = Phase::Yahho {
            remaining: MAX_RESPONSE_WINDOW,
        };
    }

    /// (1問目ならカウントダウンを終えて)「ヤー!!」を出し、correctに応じて正解/不正解のキーを
    /// 即座に(elapsed≈0で)押す。不正解の場合はpenalty=1になる
    fn answer_round(game: &mut LookAwayGame, correct: bool) {
        if is_countdown(game) {
            finish_countdown(game);
        }
        shout(game, Side::Left);
        let code = if correct {
            KeyCode::Right
        } else {
            KeyCode::Left
        };
        press(game, code);
        assert!(is_result(game, correct));
    }

    fn rendered(game: &LookAwayGame) -> Buffer {
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

    fn compact(text: &str) -> String {
        text.replace(' ', "")
    }

    /// HUDとフッター(操作説明)を除いた、相手を表示するステージ部分だけの文字列
    fn stage_text(buffer: &Buffer) -> String {
        let (_, body) = theme::split_hud(AREA);
        let stage_bottom = body.y + body.height.saturating_sub(3);
        (body.y..stage_bottom)
            .flat_map(|y| (body.x..body.right()).map(move |x| (x, y)))
            .map(|pos| buffer[pos].symbol().to_string())
            .collect::<String>()
            .replace(' ', "")
    }

    /// HUDより下の、相手を表示するパネル内側の左上付近のセル背景
    fn stage_bg(buffer: &Buffer) -> Color {
        let (_, body) = theme::split_hud(AREA);
        buffer[(body.x + 2, body.y + 2)].bg
    }

    // --- 名称・定数 ---

    #[test]
    fn display_name_and_game_id_are_separate_constants() {
        assert_eq!(DISPLAY_NAME, "ヤッホー");
        assert_eq!(GAME_ID, "look_away");
        assert_eq!(LookAwayGame::new().result().game_id, GAME_ID);
        assert_eq!(LookAwayGame::new().result().difficulty, SESSION_DIFFICULTY);
    }

    #[test]
    fn session_has_ten_rounds_and_five_hearts() {
        assert_eq!(ROUNDS_PER_SESSION, 10);
        assert_eq!(MAX_LIVES, 5);
        let game = LookAwayGame::new();
        assert_eq!(game.lives, MAX_LIVES);
        assert_eq!(game.tracker.session_length(), ROUNDS_PER_SESSION);
    }

    #[test]
    fn side_opposite_and_key() {
        assert_eq!(Side::Left.opposite(), Side::Right);
        assert_eq!(Side::Right.opposite(), Side::Left);
        assert_eq!(Side::Left.key(), KeyCode::Left);
        assert_eq!(Side::Right.key(), KeyCode::Right);
    }

    // --- ペナルティ計算 ---

    #[test]
    fn penalty_for_is_one_within_and_just_after_the_safe_window() {
        assert_eq!(penalty_for(Duration::ZERO), 1);
        assert_eq!(penalty_for(RESPONSE_SAFE_WINDOW), 1);
        assert_eq!(penalty_for(RESPONSE_SAFE_WINDOW + ms(79)), 1);
    }

    #[test]
    fn penalty_for_steps_every_80ms_after_the_safe_window() {
        assert_eq!(penalty_for(RESPONSE_SAFE_WINDOW + ms(80)), 2);
        assert_eq!(penalty_for(RESPONSE_SAFE_WINDOW + ms(159)), 2);
        assert_eq!(penalty_for(RESPONSE_SAFE_WINDOW + ms(160)), 3);
        assert_eq!(penalty_for(MAX_RESPONSE_WINDOW), 6, "無反応で確定した時の最大ペナルティ");
    }

    // --- イベントの選び方 ---

    #[test]
    fn random_between_stays_within_range() {
        let mut rng = StdRng::seed_from_u64(3);
        for _ in 0..500 {
            let wait = random_between(&mut rng, IDLE_WAIT_MS);
            assert!((ms(IDLE_WAIT_MS.0)..=ms(IDLE_WAIT_MS.1)).contains(&wait));
        }
    }

    #[test]
    fn choose_event_produces_every_kind_of_event() {
        let mut rng = StdRng::seed_from_u64(5);
        let events: Vec<Event> = (0..1000).map(|_| choose_event(&mut rng)).collect();
        let count = |f: &dyn Fn(&Event) -> bool| events.iter().filter(|e| f(e)).count();
        let yahho = count(&|e| *e == Event::Yahho);
        // 1000回中の目安250回(25%)。乱数のゆれを見込んで幅を持たせる
        assert!((180..=320).contains(&yahho), "やっほーの出現数: {yahho}");
        for side in SIDES {
            assert!(
                count(&|e| *e == Event::Shout(side)) > 0,
                "{side:?}を指して叫ぶ"
            );
        }
    }

    // --- カウントダウン ---

    #[test]
    fn new_game_starts_with_countdown_from_three() {
        let game = LookAwayGame::new();
        assert!(is_countdown(&game));
        match &game.phase {
            Phase::Countdown { state } => {
                assert_eq!(state.phase(), Some(countdown_ui::Phase::Three))
            }
            _ => unreachable!(),
        }
        assert!(!game.is_finished());
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn countdown_finishes_into_idle_within_wait_range() {
        for _ in 0..30 {
            let mut game = LookAwayGame::new();
            game.update(COUNTDOWN_TOTAL - ms(1));
            assert!(is_countdown(&game), "GO!!が終わるまではカウントダウン");
            game.update(ms(1));
            match game.phase {
                Phase::Idle { remaining } => {
                    assert!((ms(IDLE_WAIT_MS.0)..=ms(IDLE_WAIT_MS.1)).contains(&remaining))
                }
                _ => panic!("カウントダウンの後は待機"),
            }
        }
    }

    #[test]
    fn keys_during_countdown_are_ignored() {
        // カウントダウン中の入力は受け付けない(フライングにもしない)
        let mut game = LookAwayGame::new();
        game.update(PHASE_DURATION * 3); // GO!!の表示中
        for code in GAME_KEYS {
            press(&mut game, code);
            assert!(is_countdown(&game), "{code:?}: カウントダウンが続く");
        }
        assert_eq!(game.tracker.total(), 0, "記録されない");
        assert_eq!(game.lives, MAX_LIVES, "ライフも減らない");
        assert!(game.feedback.current().is_none());
    }

    // --- 待機 ---

    #[test]
    fn idle_elapses_into_an_event() {
        let mut game = LookAwayGame::new();
        game.phase = Phase::Idle { remaining: ms(100) };
        game.update(ms(60));
        assert!(matches!(game.phase, Phase::Idle { remaining } if remaining == ms(40)));
        game.update(ms(40));
        assert!(
            matches!(game.phase, Phase::Shout { .. } | Phase::Yahho { .. }),
            "待機が終わったら指さしかやっほー"
        );
    }

    #[test]
    fn pressing_while_idle_is_a_false_start() {
        for code in GAME_KEYS {
            let mut game = LookAwayGame::new();
            finish_countdown(&mut game);
            press(&mut game, code);
            assert!(
                is_result(&game, false),
                "{code:?}: 待機中に押すとフライング"
            );
            assert_eq!(game.tracker.total(), 1);
            assert_eq!(game.lives, MAX_LIVES - 1, "フライングのペナルティは1個");
            let flash = game.feedback.current().unwrap();
            assert!(flash.detail.contains(FALSE_START_TEXT));
        }
    }

    // --- 指さし+「ヤー!!」 ---

    #[test]
    fn pressing_the_opposite_within_the_safe_window_is_correct() {
        for side in SIDES {
            let mut game = LookAwayGame::new();
            finish_countdown(&mut game);
            shout(&mut game, side);
            game.update(ms(300));
            press(&mut game, side.opposite().key());
            assert!(is_result(&game, true), "{side:?}を指したら逆を向くのが正解");
            let result = game.result();
            assert_eq!((result.correct, result.total), (1, 1));
            assert_eq!(result.avg_latency_ms, 300.0, "叫んでからの反応時間を記録");
            assert_eq!(game.lives, MAX_LIVES, "正解では♥が減らない");
        }
    }

    #[test]
    fn pressing_the_opposite_after_the_safe_window_is_incorrect_with_penalty() {
        let mut game = LookAwayGame::new();
        finish_countdown(&mut game);
        shout(&mut game, Side::Left);
        // 400ms(RESPONSE_SAFE_WINDOW)を160ms超えたところで正しい方向を押す→penalty=3
        game.update(RESPONSE_SAFE_WINDOW + ms(160));
        press(&mut game, Side::Right.key());
        assert!(is_result(&game, false), "遅れれば正しい方向でも不正解");
        assert_eq!(game.lives, MAX_LIVES - 3);
    }

    #[test]
    fn pressing_the_same_side_as_the_shout_is_incorrect() {
        for side in SIDES {
            let mut game = LookAwayGame::new();
            finish_countdown(&mut game);
            shout(&mut game, side);
            press(&mut game, side.key());
            assert!(is_result(&game, false), "{side:?}: 指した方を向くと不正解");
            assert_eq!(game.result().correct, 0);
            assert_eq!(game.lives, MAX_LIVES - 1, "即座の誤入力はペナルティ1個");
        }
    }

    #[test]
    fn pressing_space_during_shout_is_incorrect() {
        let mut game = LookAwayGame::new();
        finish_countdown(&mut game);
        shout(&mut game, Side::Left);
        press(&mut game, KeyCode::Char(' '));
        assert!(is_result(&game, false));
        assert_eq!(game.lives, MAX_LIVES - 1);
    }

    #[test]
    fn shout_times_out_when_no_key_is_pressed() {
        let mut game = LookAwayGame::new();
        finish_countdown(&mut game);
        shout(&mut game, Side::Right);
        game.update(MAX_RESPONSE_WINDOW - ms(1));
        assert!(
            matches!(game.phase, Phase::Shout { .. }),
            "受付時間内は待つ"
        );
        game.update(ms(1));
        assert!(is_result(&game, false), "受付時間を過ぎたら不正解");
        assert_eq!(game.tracker.total(), 1);
        assert_eq!(
            game.lives, 0,
            "無反応の最大ペナルティは♥5個を上回るので0になる"
        );
        assert!(game
            .feedback
            .current()
            .unwrap()
            .detail
            .contains(TIMEOUT_TEXT));
    }

    // --- やっほー ---

    #[test]
    fn space_during_yahho_is_correct() {
        let mut game = LookAwayGame::new();
        finish_countdown(&mut game);
        yahho(&mut game);
        game.update(ms(400));
        press(&mut game, KeyCode::Char(' '));
        assert!(is_result(&game, true), "やっほーにはSpaceで返すのが正解");
        let result = game.result();
        assert_eq!(result.correct, 1);
        assert_eq!(result.avg_latency_ms, 400.0);
        assert_eq!(game.lives, MAX_LIVES);
    }

    #[test]
    fn space_after_the_safe_window_is_incorrect_with_penalty() {
        let mut game = LookAwayGame::new();
        finish_countdown(&mut game);
        yahho(&mut game);
        game.update(RESPONSE_SAFE_WINDOW + ms(1));
        press(&mut game, KeyCode::Char(' '));
        assert!(is_result(&game, false), "400msを1msでも過ぎたら不正解");
        assert_eq!(game.lives, MAX_LIVES - 1);
    }

    #[test]
    fn arrow_keys_during_yahho_are_incorrect() {
        for code in [KeyCode::Left, KeyCode::Right] {
            let mut game = LookAwayGame::new();
            finish_countdown(&mut game);
            yahho(&mut game);
            press(&mut game, code);
            assert!(
                is_result(&game, false),
                "{code:?}: やっほーに向きで答えると不正解"
            );
            assert_eq!(game.lives, MAX_LIVES - 1);
        }
    }

    #[test]
    fn yahho_times_out_when_space_is_not_pressed() {
        let mut game = LookAwayGame::new();
        finish_countdown(&mut game);
        yahho(&mut game);
        game.update(MAX_RESPONSE_WINDOW - ms(1));
        assert!(matches!(game.phase, Phase::Yahho { .. }));
        game.update(ms(1));
        assert!(is_result(&game, false));
        assert_eq!(game.lives, 0, "無反応の最大ペナルティは♥5個を上回るので0になる");
        assert!(game
            .feedback
            .current()
            .unwrap()
            .detail
            .contains(TIMEOUT_TEXT));
    }

    // --- 対象外のキー ---

    #[test]
    fn other_keys_are_ignored_in_every_phase() {
        let ignored = [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Char('a'),
        ];
        let mut game = LookAwayGame::new();
        let setups: [fn(&mut LookAwayGame); 3] = [
            |g| g.phase = Phase::Idle { remaining: ms(500) },
            |g| shout(g, Side::Left),
            yahho,
        ];
        for setup in setups {
            setup(&mut game);
            for code in ignored {
                press(&mut game, code);
            }
            assert!(!matches!(game.phase, Phase::Result { .. }));
            assert_eq!(game.tracker.total(), 0);
            assert_eq!(game.lives, MAX_LIVES);
        }
    }

    // --- 結果表示と次の問題 ---

    #[test]
    fn result_holds_then_next_round_starts_without_countdown() {
        let mut game = LookAwayGame::new();
        answer_round(&mut game, true);
        game.update(RESULT_HOLD - ms(1));
        assert!(
            matches!(game.phase, Phase::Result { .. }),
            "表示時間中は結果のまま"
        );
        game.update(ms(1));
        assert!(
            matches!(game.phase, Phase::Idle { .. }),
            "2問目以降はカウントダウンを挟まず直接待機から始まる"
        );
        assert!(!game.is_finished());
    }

    #[test]
    fn countdown_is_shown_on_the_first_round_only() {
        let mut game = LookAwayGame::new();
        assert!(is_countdown(&game), "1問目はカウントダウンから");
        for _ in 0..3 {
            answer_round(&mut game, true);
            game.update(RESULT_HOLD);
            assert!(
                matches!(game.phase, Phase::Idle { .. }),
                "2問目以降はカウントダウンを挟まない"
            );
        }
    }

    #[test]
    fn keys_during_result_are_ignored() {
        let mut game = LookAwayGame::new();
        answer_round(&mut game, false);
        for code in GAME_KEYS {
            press(&mut game, code);
        }
        assert_eq!(game.tracker.total(), 1);
        assert_eq!(game.lives, MAX_LIVES - 1);
    }

    // --- ライフとGAME OVER・勝利 ---

    #[test]
    fn losing_all_lives_is_game_over_after_the_last_mark() {
        let mut game = LookAwayGame::new();
        for round in 0..MAX_LIVES {
            answer_round(&mut game, false);
            assert_eq!(game.lives, MAX_LIVES - round - 1);
            if round + 1 < MAX_LIVES {
                finish_result(&mut game);
            }
        }
        assert_eq!(game.lives, 0);
        assert!(game.is_game_over());
        assert!(!game.is_finished(), "最後の✗を見せ終えるまでは終わらない");
        finish_result(&mut game);
        assert!(
            game.is_finished(),
            "ライフが尽きたら残りの問題を待たずに終わる"
        );
        assert!(!game.is_cleared());
        let result = game.result();
        assert_eq!(result.total, MAX_LIVES, "10問を待たずに終わる");
        assert_eq!(result.correct, 0);
    }

    #[test]
    fn game_over_even_after_some_correct_answers() {
        let mut game = LookAwayGame::new();
        answer_round(&mut game, true);
        finish_result(&mut game);
        for _ in 0..MAX_LIVES {
            answer_round(&mut game, false);
            finish_result(&mut game);
        }
        assert!(game.is_finished());
        assert!(game.is_game_over());
        assert!(!game.is_cleared());
        assert_eq!(game.result().total, MAX_LIVES + 1);
        assert_eq!(game.result().correct, 1);
    }

    #[test]
    fn finishing_all_rounds_with_lives_left_is_a_clear() {
        let mut game = LookAwayGame::new();
        for round in 0..ROUNDS_PER_SESSION {
            assert!(!game.is_finished(), "{round}問目の前は終わっていない");
            assert!(!game.is_cleared(), "全問を終えるまでは勝利にしない");
            // 間違いはライフの数より1つ少なく抑える
            answer_round(&mut game, round >= MAX_LIVES - 1);
            if round + 1 < ROUNDS_PER_SESSION {
                finish_result(&mut game);
            }
        }
        assert!(
            !game.is_finished(),
            "最後の結果表示が終わるまでは終わらない"
        );
        finish_result(&mut game);
        assert!(game.is_finished());
        assert!(game.is_cleared());
        assert!(!game.is_game_over());
        assert_eq!(game.lives, 1);
        let result = game.result();
        assert_eq!(result.total, ROUNDS_PER_SESSION);
        assert_eq!(result.correct, ROUNDS_PER_SESSION - (MAX_LIVES - 1));
    }

    #[test]
    fn losing_the_last_life_on_the_final_round_is_game_over_not_clear() {
        let mut game = LookAwayGame::new();
        for round in 0..ROUNDS_PER_SESSION {
            // 最後の問題で3つ目の間違いをする
            let correct = round < ROUNDS_PER_SESSION - MAX_LIVES;
            answer_round(&mut game, correct);
            finish_result(&mut game);
        }
        assert!(game.is_finished());
        assert!(game.is_game_over());
        assert!(!game.is_cleared());
    }

    #[test]
    fn input_and_updates_after_finish_are_ignored() {
        let mut game = LookAwayGame::new();
        for _ in 0..MAX_LIVES {
            answer_round(&mut game, false);
            finish_result(&mut game);
        }
        assert!(game.is_finished());
        for code in GAME_KEYS {
            press(&mut game, code);
        }
        game.update(Duration::from_secs(10));
        assert!(game.is_finished());
        assert_eq!(game.result().total, MAX_LIVES);
    }

    // --- 描画 ---

    #[test]
    fn hud_shows_display_name_and_round_count() {
        let text = text_of(&rendered(&LookAwayGame::new()));
        assert!(text.contains(&compact(DISPLAY_NAME)), "{text}");
        assert!(text.contains("Q1/10"), "{text}");
    }

    #[test]
    fn countdown_renders_big_glyph() {
        let buffer = rendered(&LookAwayGame::new());
        assert!(text_of(&buffer).contains('█'), "カウントダウンの大きな文字");
        assert!(!stage_text(&buffer).contains(&compact(SHOUT_TEXT)));
    }

    #[test]
    fn lives_are_shown_as_hearts() {
        let mut game = LookAwayGame::new();
        finish_countdown(&mut game);
        let text = text_of(&rendered(&game));
        assert!(text.contains("♥♥♥♥♥"), "♥5個: {text}");
        game.lives = 1;
        let text = text_of(&rendered(&game));
        assert!(text.contains("♥♡♡♡♡"), "♥1個: {text}");
    }

    #[test]
    fn stage_renderer_falls_back_to_text_without_images() {
        // テスト環境ではpickerを検出できないので、常にテキストのフォールバックになる
        let renderer = StageRenderer::new();
        assert!(!renderer.uses_image());
    }

    #[test]
    fn idle_shows_the_face_without_pointing_or_shout() {
        let mut game = LookAwayGame::new();
        finish_countdown(&mut game);
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(text.contains(&compact(FACE_TEXT)), "{text}");
        assert!(!text.contains('◀') && !text.contains('▶'));
        assert!(!stage_text(&buffer).contains(&compact(SHOUT_TEXT)));
        assert_eq!(stage_bg(&buffer), IDLE_BG);
    }

    #[test]
    fn shout_points_and_shows_the_shout_text() {
        let mut game = LookAwayGame::new();
        shout(&mut game, Side::Right);
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(text.contains('▶'), "右を指す: {text}");
        assert!(!text.contains('◀'));
        assert!(
            stage_text(&buffer).contains(&compact(SHOUT_TEXT)),
            "ステージに「ヤー!!」を叫ぶ: {text}"
        );
        assert_eq!(stage_bg(&buffer), SHOUT_BG);
        assert_ne!(SHOUT_BG, IDLE_BG, "叫んだかどうかは背景色でも分かる");
    }

    #[test]
    fn yahho_shows_the_call_text() {
        let mut game = LookAwayGame::new();
        yahho(&mut game);
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(text.contains(&compact(YAHHO_CALL_TEXT)), "{text}");
        assert!(!stage_text(&buffer).contains(&compact(SHOUT_TEXT)));
        assert_eq!(stage_bg(&buffer), YAHHO_BG);
    }

    #[test]
    fn result_shows_big_marks() {
        let mut game = LookAwayGame::new();
        answer_round(&mut game, true);
        assert!(text_of(&rendered(&game)).contains(CORRECT_MARK));
        finish_result(&mut game);
        answer_round(&mut game, false);
        assert!(text_of(&rendered(&game)).contains(INCORRECT_MARK));
    }

    #[test]
    fn final_result_shows_game_over_or_clear() {
        let mut game = LookAwayGame::new();
        for round in 0..MAX_LIVES {
            answer_round(&mut game, false);
            if round + 1 < MAX_LIVES {
                assert!(!text_of(&rendered(&game)).contains(&compact(GAME_OVER_TEXT)));
                finish_result(&mut game);
            }
        }
        assert!(text_of(&rendered(&game)).contains(&compact(GAME_OVER_TEXT)));

        let mut game = LookAwayGame::new();
        for round in 0..ROUNDS_PER_SESSION {
            answer_round(&mut game, true);
            if round + 1 < ROUNDS_PER_SESSION {
                assert!(!text_of(&rendered(&game)).contains(&compact(CLEAR_TEXT)));
                finish_result(&mut game);
            }
        }
        assert!(text_of(&rendered(&game)).contains(&compact(CLEAR_TEXT)));
    }

    #[test]
    fn render_does_not_panic_in_tiny_area() {
        let mut game = LookAwayGame::new();
        let setups: [fn(&mut LookAwayGame); 4] = [
            |_| {},
            |g| g.phase = Phase::Idle { remaining: ms(500) },
            |g| shout(g, Side::Right),
            yahho,
        ];
        for setup in setups {
            setup(&mut game);
            for (width, height) in [(1, 1), (5, 2), (10, 4)] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| game.render(frame, Rect::new(0, 0, width, height)))
                    .unwrap();
            }
        }
    }
}
