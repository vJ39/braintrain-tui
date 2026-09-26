//! べー(beigoma): 軽トラの荷台で女の子が持つベーゴマ盤を傾け、ベーゴマを唯一のゴールへ導く。
//! 仕様は docs/beigoma-game-spec.md
//!
//! - 盤の傾きは前後・左右の2軸で、矢印キーを押すたびに一定量動く連打式(board.rs)
//! - 軽トラは自動で走り、段差・信号・障害物回避のたびに運動方程式から求めたGが盤にかかる(truck.rs)
//! - 凸を踏んだ瞬間のGが大きいと弾かれ、さらに大きいと吹っ飛んで即GAME OVER
//! - 凹に正面から入るとハマり(強く傾けると抜け出せる)、斜めに入ると側面をこすって弾かれる・吹っ飛ぶ
//! - 盤の縁に壁は無く、盤から落ちても(場外)即GAME OVER
//! - 1セッションは2ROUND(ROUND1=やさしい、ROUND2=むずかしい)。ゴールの位置はROUNDごとにランダム
//! - 制限時間はROUNDごとに60秒。ゴールで成功、吹っ飛び・場外・時間切れで失敗。難易度選択は無い
//! - ROUND1をクリアした時だけROUND2へ進む。ROUND1が失敗ならROUND2へ進まずセッション終了
//! - 各ROUNDの開始前に「3.2.1.GO!!」をゲーム内で行い、GO!!が終わったらベーゴマを投入する
//!   (app.rsの画面遷移側のカウントダウンは経由しない)

mod board;
mod render;
mod truck;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};
use crate::ui::countdown::{self, CountdownState};

use board::{
    round_params, Board, Landing, RoundParams, StepEvent, Tilt, TiltKey, Top, ROUNDS_PER_SESSION,
};
use render::{BoardRenderer, TopView, TruckViewInfo, TruckViewRenderer};
use truck::Truck;

pub const GAME_ID: &str = "beigoma";

/// 結果に記録する難易度。べーは難易度を選ばないので固定値にする
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Intermediate;

/// 1ROUNDの制限時間
pub const TIME_LIMIT: Duration = Duration::from_secs(60);

/// 終了(ゴール・GAME OVER・時間切れ)の表示を出し続けてからリザルトへ進むまでの時間
pub const END_HOLD: Duration = Duration::from_millis(1500);

/// 女の子のセリフ(弾かれ・吹っ飛び・場外のGAME OVERで共通)。吹き出しへの表示は#129で行う
#[cfg_attr(not(test), allow(dead_code))]
pub const GASP_LINE: &str = "ああっ!!";

/// 弾かれた・着地した等の一言を出し続ける時間
const MESSAGE_HOLD: Duration = Duration::from_millis(900);

/// 物理を進める1ステップの上限。大きなdtはこの長さに分けて進める(マスの飛び越し防止)
const MAX_STEP: Duration = Duration::from_millis(10);

/// ベーゴマの回転の見た目が1コマ進む間隔
const SPIN_FRAME_INTERVAL: Duration = Duration::from_millis(70);

/// 画面左の軽トラ視点の幅(%)。残りを盤面に使う
const TRUCK_VIEW_PERCENT: u16 = 40;

/// ゲームの終わり方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 制限時間内にゴールした。timeはクリアタイム
    Cleared { time: Duration },
    /// 吹っ飛んだ・盤から落ちた(どちらも同じ演出のGAME OVER)
    Flown,
    /// 制限時間を過ぎた
    TimeUp,
}

/// ROUNDの進み具合
enum Status {
    /// ROUND開始前の「3.2.1.GO!!」。新しい盤は見せておき、ベーゴマは投入位置で待たせる
    Countdown {
        state: CountdownState,
    },
    Playing,
    /// ROUNDが終わった。shownは終了表示を出してからの時間
    Ended {
        outcome: Outcome,
        shown: Duration,
    },
}

pub struct BeigomaGame {
    /// 現在のROUND(0始まり)
    round_index: u32,
    params: RoundParams,
    board: Board,
    top: Top,
    tilt: Tilt,
    truck: Truck,
    /// このROUNDでベーゴマを投入してからの経過時間
    elapsed: Duration,
    status: Status,
    tracker: ScoreTracker,
    /// 一言の表示(文言, 表示してからの時間)
    message: Option<(&'static str, Duration)>,
    board_renderer: BoardRenderer,
    truck_view: TruckViewRenderer,
}

impl BeigomaGame {
    /// ROUND1をカウントダウンから始める。軽トラは決まったコースを走る
    pub fn new() -> Self {
        Self::with_truck(Truck::new())
    }

    /// 軽トラを指定して作る(テストで特定のGの場面を作るため)
    fn with_truck(truck: Truck) -> Self {
        let params = round_params(0);
        let board = Board::generate(&params, &mut rand::thread_rng());
        let top = Top::new(board.start_position());
        let mut game = Self {
            round_index: 0,
            params,
            board,
            top,
            tilt: Tilt::new(),
            truck,
            elapsed: Duration::ZERO,
            status: Status::Playing,
            tracker: ScoreTracker::with_session_length(ROUNDS_PER_SESSION),
            message: None,
            board_renderer: BoardRenderer::new(),
            truck_view: TruckViewRenderer::new(),
        };
        game.start_round(0);
        game
    }

    /// round_index番目のROUNDをカウントダウンから始める。盤を作り直し(ゴールはランダム)、
    /// ベーゴマは投入位置で待たせる。軽トラのコースはそのまま続く
    fn start_round(&mut self, round_index: u32) {
        self.round_index = round_index;
        self.params = round_params(round_index);
        self.board = Board::generate(&self.params, &mut rand::thread_rng());
        self.top = Top::new(self.board.start_position());
        self.tilt = Tilt::new();
        self.elapsed = Duration::ZERO;
        self.message = None;
        let state = CountdownState::new();
        // 最初のフェーズ「3」の音
        if let Some(phase) = state.phase() {
            audio::play_se(phase.se());
        }
        self.status = Status::Countdown { state };
    }

    /// GO!!が終わった: ベーゴマを投入位置に投入し、制限時間を数え始める
    fn drop_top(&mut self) {
        self.top = Top::new(self.board.start_position());
        self.elapsed = Duration::ZERO;
        self.status = Status::Playing;
    }

    fn is_playing(&self) -> bool {
        matches!(self.status, Status::Playing)
    }

    /// このROUNDの終わり方(まだ終わっていなければNone)
    pub fn outcome(&self) -> Option<Outcome> {
        match self.status {
            Status::Countdown { .. } | Status::Playing => None,
            Status::Ended { outcome, .. } => Some(outcome),
        }
    }

    /// セッションがこのROUNDで終わりか。最後のROUNDが終わった時と、
    /// ROUNDがクリア以外(吹っ飛び・場外・時間切れ)で終わった時(次のROUNDへ進まない)
    fn is_last_round_ended(&self) -> bool {
        match self.outcome() {
            Some(Outcome::Cleared { .. }) => self.tracker.is_session_finished(),
            Some(Outcome::Flown | Outcome::TimeUp) => true,
            None => false,
        }
    }

    /// 1ステップ(MAX_STEP以下)だけ進める
    fn step(&mut self, dt: Duration) {
        self.tilt.update(dt);
        self.truck.update(dt);
        let g = self.truck.current_g();
        let event = self.top.step(&self.board, dt, &self.tilt, g);
        self.elapsed += dt;
        if let Some(event) = event {
            self.on_step_event(event);
        }
        if self.is_playing() && self.elapsed >= TIME_LIMIT {
            self.finish(Outcome::TimeUp);
        }
    }

    /// 盤上の出来事を反映する(弾かれたらSE、凹にハマった・抜けたら一言、
    /// 吹っ飛んだ・盤から落ちたら即GAME OVER、ゴールなら成功)
    fn on_step_event(&mut self, event: StepEvent) {
        if !self.is_playing() {
            return;
        }
        match event {
            StepEvent::Hopped { .. } => self.message = Some(("ガタッ!", Duration::ZERO)),
            StepEvent::Landed(Landing::Light) | StepEvent::Grazed(Landing::Light) => {
                self.message = Some(("セーフ", Duration::ZERO));
            }
            // 凹の側面をこすって弾かれた時も、凸で弾かれた時と同じ演出
            StepEvent::Landed(Landing::Bounce) | StepEvent::Grazed(Landing::Bounce) => {
                audio::play_se(SeKind::Incorrect);
                self.message = Some(("ぴよーん!! ああっ!!", Duration::ZERO));
            }
            StepEvent::Sank => self.message = Some(("ズボッ", Duration::ZERO)),
            StepEvent::Escaped => self.message = Some(("ぬけた!", Duration::ZERO)),
            StepEvent::Landed(Landing::Flown)
            | StepEvent::Grazed(Landing::Flown)
            | StepEvent::FellOff => self.finish(Outcome::Flown),
            StepEvent::Goal => self.finish(Outcome::Cleared { time: self.elapsed }),
        }
    }

    /// ROUNDを終える。結果はROUNDごとに1回だけ記録する(成功はクリアタイム、失敗は制限時間を反応時間として記録)
    fn finish(&mut self, outcome: Outcome) {
        if !self.is_playing() {
            return;
        }
        let (success, latency) = match outcome {
            Outcome::Cleared { time } => (true, time),
            Outcome::Flown | Outcome::TimeUp => (false, TIME_LIMIT),
        };
        self.tracker.record(success, latency.as_millis() as f64);
        // 場外・吹っ飛びは専用の「ふいっ」という音、時間切れは従来のブザー音のまま
        audio::play_se(match outcome {
            Outcome::Cleared { .. } => SeKind::Correct,
            Outcome::Flown => SeKind::Star,
            Outcome::TimeUp => SeKind::Incorrect,
        });
        self.status = Status::Ended {
            outcome,
            shown: Duration::ZERO,
        };
    }

    /// 回転の見た目のコマ
    fn spin_frame(&self) -> usize {
        (self.elapsed.as_millis() / SPIN_FRAME_INTERVAL.as_millis()) as usize
            % render::TOP_SPIN_GLYPHS.len()
    }

    /// 場外・吹っ飛びGAME OVERの星の演出のコマ(それ以外はNone)
    fn star_frame(&self) -> Option<usize> {
        let Status::Ended {
            outcome: Outcome::Flown,
            shown,
        } = self.status
        else {
            return None;
        };
        let frame = (shown.as_millis() / render::STAR_ANIM_FRAME.as_millis()) as usize;
        Some(frame.min(render::STAR_ANIM_GLYPHS.len() - 1))
    }

    /// 上段: 残り時間・一言(終わったら結果)・傾き。枠のタイトルにROUNDの名前を出す
    fn render_hud(&self, frame: &mut Frame, area: Rect) {
        let block = theme::panel(format!(" ◆ べー  {} ", self.params.label))
            .border_style(Style::default().fg(self.status_color()));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(16),
                Constraint::Fill(1),
                Constraint::Length(26),
            ])
            .split(inner);

        let remaining = TIME_LIMIT.saturating_sub(self.elapsed).as_secs_f64();
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" 残り {remaining:.1}秒"),
                theme::title_style(),
            ))),
            cols[0],
        );

        let center = match self.outcome() {
            Some(Outcome::Cleared { time }) => Some(format!("GOAL!! {:.1}秒", time.as_secs_f64())),
            Some(Outcome::Flown) => Some("GAME OVER".to_string()),
            Some(Outcome::TimeUp) => Some("TIME UP".to_string()),
            None => self.message.map(|(text, _)| text.to_string()),
        };
        if let Some(text) = center {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default()
                        .fg(self.status_color())
                        .add_modifier(Modifier::BOLD),
                )))
                .alignment(Alignment::Center),
                cols[1],
            );
        }

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    "傾き 前後{:+.1} 左右{:+.1} ",
                    self.tilt.pitch(),
                    self.tilt.roll()
                ),
                Style::default().fg(theme::TEXT),
            )))
            .alignment(Alignment::Right),
            cols[2],
        );
    }

    /// 枠・一言の色。成功は正解色、失敗は不正解色、プレイ中は通常の色(一言は目立つ色)
    fn status_color(&self) -> Color {
        match self.outcome() {
            Some(Outcome::Cleared { .. }) => theme::CORRECT,
            Some(_) => theme::INCORRECT,
            None if self.message.is_some() => theme::HIGHLIGHT,
            None => theme::ACCENT,
        }
    }
}

impl Default for BeigomaGame {
    fn default() -> Self {
        Self::new()
    }
}

impl Game for BeigomaGame {
    /// 矢印キーで盤を傾ける(押すたびに一定量)。プレイ中以外(カウントダウン中・終わった後)は受け付けない
    fn handle_key(&mut self, key: KeyEvent) {
        if !self.is_playing() {
            return;
        }
        let tilt_key = match key.code {
            KeyCode::Up => TiltKey::Forward,
            KeyCode::Down => TiltKey::Back,
            KeyCode::Left => TiltKey::Left,
            KeyCode::Right => TiltKey::Right,
            _ => return,
        };
        self.tilt.press(tilt_key);
    }

    fn update(&mut self, dt: Duration) {
        match &mut self.status {
            Status::Countdown { state } => {
                if let Some(phase) = state.tick(dt) {
                    audio::play_se(phase.se());
                }
                if state.is_finished() {
                    self.drop_top();
                }
                return;
            }
            Status::Ended { shown, .. } => {
                *shown = shown.saturating_add(dt);
                // 結果の表示を出し終えたら、クリアしていて次のROUNDがあれば進む
                if *shown >= END_HOLD && !self.is_last_round_ended() {
                    self.start_round(self.round_index + 1);
                }
                return;
            }
            Status::Playing => {}
        }
        if let Some((text, shown)) = self.message {
            let shown = shown.saturating_add(dt);
            self.message = (shown < MESSAGE_HOLD).then_some((text, shown));
        }
        // 大きなdtは小さなステップに分けて進める(途中で終わったらそこで止める)
        let mut remaining = dt;
        while !remaining.is_zero() && self.is_playing() {
            let step = remaining.min(MAX_STEP);
            self.step(step);
            remaining -= step;
        }
    }

    /// 上段にHUD、下段の左に軽トラ視点・右に盤面視点を並べる。カウントダウン中は盤面の上に重ねる
    fn render(&self, frame: &mut Frame, area: Rect) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        let (hud_area, body) = theme::split_hud(area);
        self.render_hud(frame, hud_area);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(TRUCK_VIEW_PERCENT),
                Constraint::Min(0),
            ])
            .split(body);

        let truck_block = theme::panel(" 軽トラ視点 ");
        let truck_inner = truck_block.inner(cols[0]);
        frame.render_widget(truck_block, cols[0]);
        let info = TruckViewInfo {
            speed: self.truck.speed(),
            g: self.truck.current_g(),
            upcoming: self.truck.upcoming(),
        };
        self.truck_view.render(frame, truck_inner, &info);

        let board_block = theme::panel(" 盤面 ")
            .border_style(Style::default().fg(self.status_color()))
            .title_bottom(
                theme::hints_line(&[("↑↓←→", "連打で傾ける"), ("q", "やめる")]).centered(),
            );
        let board_inner = board_block.inner(cols[1]);
        frame.render_widget(board_block, cols[1]);
        let top = TopView {
            pos: self.top.pos,
            airborne: self.top.is_airborne(),
            spin_frame: self.spin_frame(),
            star_frame: self.star_frame(),
        };
        self.board_renderer
            .render(frame, board_inner, &self.board, &top, &self.tilt);
        if let Status::Countdown { state } = &self.status {
            countdown::render(frame, board_inner, state);
        }
    }

    /// 最後のROUND(またはクリアできなかったROUND)の終了表示(END_HOLD)を出し終えたらセッション終了
    fn is_finished(&self) -> bool {
        matches!(self.status, Status::Ended { shown, .. } if shown >= END_HOLD)
            && self.is_last_round_ended()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
    }
}

#[cfg(test)]
mod tests {
    use super::board::{
        round_params, Cell, TopState, BOARD_HEIGHT, BOARD_WIDTH, HIGH_G_THRESHOLD,
        ROUNDS_PER_SESSION, TILT_STEP,
    };
    use super::truck::RoadEvent;
    use super::*;
    use crate::ui::countdown::{Phase, PHASE_DURATION};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const AREA: Rect = Rect::new(0, 0, 100, 24);
    const STEP: Duration = Duration::from_millis(10);
    /// ROUND開始前の「3.2.1.GO!!」全体の長さ
    const COUNTDOWN_TOTAL: Duration = Duration::from_millis(2400);
    /// ROUND1の盤をテストで決定的にするためのゴール(旧来の固定配置のゴール位置)
    const ROUND1_GOAL: (usize, usize) = (17, 1);
    /// カウントダウンの大きな文字に使う記号
    const BIG_DOT: &str = "█";

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn is_countdown(game: &BeigomaGame) -> bool {
        matches!(game.status, Status::Countdown { .. })
    }

    fn is_playing(game: &BeigomaGame) -> bool {
        matches!(game.status, Status::Playing)
    }

    fn countdown_phase(game: &BeigomaGame) -> Option<Phase> {
        match &game.status {
            Status::Countdown { state } => state.phase(),
            _ => None,
        }
    }

    /// カウントダウンを最後まで進めて、ベーゴマを投入させる
    fn finish_countdown(game: &mut BeigomaGame) {
        assert!(is_countdown(game), "カウントダウン中のはず");
        game.update(COUNTDOWN_TOTAL);
        assert!(is_playing(game), "GO!!の後はプレイ中のはず");
    }

    /// イベントの起きない軽トラ(ずっと巡航・Gなし)で、ROUND1をカウントダウン前の状態で作る。
    /// ゴールは決定的にするため旧来の位置に置き直す
    fn calm_game_before_go() -> BeigomaGame {
        let mut game = BeigomaGame::with_truck(Truck::with_course(Vec::new(), 1000.0));
        game.board = Board::with_goal(round_params(0).layout, ROUND1_GOAL);
        game
    }

    /// calm_game_before_goのカウントダウンを終え、ベーゴマを投入した直後
    fn calm_game() -> BeigomaGame {
        let mut game = calm_game_before_go();
        finish_countdown(&mut game);
        game
    }

    fn rendered_text(game: &BeigomaGame, area: Rect) -> String {
        let mut terminal = Terminal::new(TestBackend::new(area.right(), area.bottom())).unwrap();
        terminal.draw(|frame| game.render(frame, area)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace(' ', "")
    }

    /// ゴールの左隣から、右へ転がってゴールに入る直前に置く
    fn place_just_before_goal(game: &mut BeigomaGame) {
        let (gx, gy) = game.board.goal();
        game.top = Top::new((gx as f64 - 0.02, gy as f64 + 0.5));
        game.top.vel = (3.0, 0.0);
    }

    /// 真下が平坦で真上が障害物のマス(障害物の1マス下)
    fn flat_below_a_bump(board: &Board) -> (usize, usize) {
        (0..BOARD_HEIGHT - 1)
            .flat_map(|y| (0..BOARD_WIDTH).map(move |x| (x, y)))
            .find(|&(x, y)| board.cell(x, y) == Cell::Bump && board.cell(x, y + 1) == Cell::Flat)
            .map(|(x, y)| (x, y + 1))
            .expect("下が平坦な障害物がある")
    }

    // --- 開始(ROUNDごとのカウントダウン→投入) ---

    #[test]
    fn countdown_total_is_four_phases() {
        assert_eq!(PHASE_DURATION * 4, COUNTDOWN_TOTAL);
    }

    #[test]
    fn new_game_starts_round1_with_a_countdown_and_the_top_waiting() {
        let game = BeigomaGame::new();
        assert!(is_countdown(&game), "ROUND1はカウントダウンから始まる");
        assert_eq!(countdown_phase(&game), Some(Phase::Three), "3から始まる");
        assert_eq!(game.round_index, 0);
        assert_eq!(game.params, round_params(0));
        assert_eq!(
            game.board.start_position(),
            Board::with_goal(round_params(0).layout, ROUND1_GOAL).start_position(),
            "ROUND1の盤"
        );
        assert_eq!(
            game.top.pos,
            game.board.start_position(),
            "ベーゴマは投入位置で待っている"
        );
        assert_eq!(game.top.vel, (0.0, 0.0));
        assert!(!game.top.is_airborne());
        assert_eq!(game.elapsed, Duration::ZERO, "制限時間はまだ数えない");
        assert_eq!(game.outcome(), None);
        assert!(!game.is_finished());
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.difficulty, SESSION_DIFFICULTY);
        assert_eq!(result.total, 0);
        // 盤は見えていて、その上にカウントダウンを重ねる
        // (ゴールはランダムでカウントダウンの文字の下に隠れることがあるので、配置が決まった盤で見る)
        let text = rendered_text(&calm_game_before_go(), AREA);
        assert!(text.contains("盤面"), "{text}");
        assert!(
            text.contains(render::BUMP_GLYPH) || text.contains(render::HOLLOW_GLYPH),
            "新しい盤を見せておく: {text}"
        );
        assert!(text.contains(BIG_DOT), "カウントダウンの大きな文字: {text}");
        assert!(text.contains("残り60.0秒"), "{text}");
    }

    #[test]
    fn the_countdown_goes_three_two_one_go() {
        let mut game = calm_game_before_go();
        for phase in [Phase::Two, Phase::One, Phase::Go] {
            game.update(PHASE_DURATION);
            assert_eq!(countdown_phase(&game), Some(phase));
        }
    }

    #[test]
    fn keys_are_ignored_during_the_countdown() {
        let mut game = calm_game_before_go();
        for code in [
            KeyCode::Up,
            KeyCode::Right,
            KeyCode::Right,
            KeyCode::Left,
            KeyCode::Down,
        ] {
            game.handle_key(key(code));
        }
        assert_eq!(game.tilt.pitch(), 0.0, "カウントダウン中の連打で傾かない");
        assert_eq!(game.tilt.roll(), 0.0);
        game.update(COUNTDOWN_TOTAL - STEP);
        assert!(is_countdown(&game));
        assert_eq!(
            game.top.pos,
            game.board.start_position(),
            "カウントダウン中は転がらない"
        );
        assert_eq!(game.elapsed, Duration::ZERO);
    }

    #[test]
    fn go_drops_the_top_and_starts_the_clock() {
        let mut game = calm_game_before_go();
        game.update(COUNTDOWN_TOTAL - Duration::from_millis(1));
        assert!(is_countdown(&game), "GO!!の表示が終わるまでは投入しない");
        assert_eq!(game.elapsed, Duration::ZERO);
        game.update(Duration::from_millis(1));
        assert!(is_playing(&game), "GO!!が終わったら投入する");
        assert_eq!(
            game.top.pos,
            game.board.start_position(),
            "投入位置に置かれる"
        );
        assert_eq!(game.top.vel, (0.0, 0.0));
        assert_eq!(game.elapsed, Duration::ZERO, "制限時間はここから数える");
        game.update(Duration::from_secs(1));
        assert_eq!(game.elapsed, Duration::from_secs(1));
        // 投入後はキーで傾けられる
        game.handle_key(key(KeyCode::Right));
        assert_eq!(game.tilt.roll(), TILT_STEP);
        let text = rendered_text(&game, AREA);
        assert!(!text.contains(BIG_DOT), "カウントダウンは消える: {text}");
    }

    #[test]
    fn the_top_starts_spinning_after_it_is_placed() {
        let mut game = calm_game();
        let first = game.spin_frame();
        game.update(SPIN_FRAME_INTERVAL);
        assert_ne!(game.spin_frame(), first, "投入されると回転を始める");
    }

    #[test]
    fn spin_frame_interval_is_fast_enough_to_look_energetic() {
        // 回転をもっと激しく見せるため、コマ送りの間隔は短くする(#129)
        assert_eq!(SPIN_FRAME_INTERVAL, Duration::from_millis(70));
    }

    // --- 操作 ---

    #[test]
    fn arrow_keys_tilt_the_matching_axis() {
        let mut game = calm_game();
        game.handle_key(key(KeyCode::Up));
        assert_eq!(game.tilt.pitch(), TILT_STEP, "↑で前傾");
        game.handle_key(key(KeyCode::Down));
        game.handle_key(key(KeyCode::Down));
        assert_eq!(game.tilt.pitch(), -TILT_STEP, "↓で後傾");
        game.handle_key(key(KeyCode::Right));
        assert_eq!(game.tilt.roll(), TILT_STEP, "→で右傾");
        game.handle_key(key(KeyCode::Left));
        game.handle_key(key(KeyCode::Left));
        assert_eq!(game.tilt.roll(), -TILT_STEP, "←で左傾");
    }

    #[test]
    fn other_keys_do_not_tilt() {
        let mut game = calm_game();
        for code in [
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Char('a'),
            KeyCode::Esc,
        ] {
            game.handle_key(key(code));
        }
        assert_eq!(game.tilt.pitch(), 0.0);
        assert_eq!(game.tilt.roll(), 0.0);
    }

    #[test]
    fn tilting_rolls_the_top() {
        let mut game = calm_game();
        let start = game.top.pos;
        game.handle_key(key(KeyCode::Right));
        game.handle_key(key(KeyCode::Right));
        game.update(Duration::from_millis(300));
        assert!(game.top.pos.0 > start.0, "右へ傾けると右へ転がる");
    }

    #[test]
    fn a_large_dt_is_split_into_small_steps() {
        let mut stepped = calm_game();
        let mut once = calm_game();
        for game in [&mut stepped, &mut once] {
            game.handle_key(key(KeyCode::Up));
            game.handle_key(key(KeyCode::Right));
        }
        for _ in 0..50 {
            stepped.update(STEP);
        }
        once.update(STEP * 50);
        assert_eq!(stepped.top.pos, once.top.pos);
        assert_eq!(stepped.elapsed, once.elapsed);
    }

    // --- 成功・失敗 ---

    #[test]
    fn reaching_the_goal_within_the_time_limit_is_success() {
        let mut game = calm_game();
        game.update(Duration::from_secs(5));
        place_just_before_goal(&mut game);
        game.update(STEP);
        let Some(Outcome::Cleared { time }) = game.outcome() else {
            panic!("ゴールしたら成功: {:?}", game.outcome());
        };
        assert_eq!(time, game.elapsed, "クリアタイムを記録する");
        assert!(time >= Duration::from_secs(5) && time < TIME_LIMIT);
        let result = game.result();
        assert_eq!((result.correct, result.total), (1, 1));
        assert!((result.avg_latency_ms - time.as_millis() as f64).abs() < 1.0);
    }

    #[test]
    fn not_reaching_the_goal_in_sixty_seconds_is_a_time_up_failure() {
        let mut game = calm_game();
        game.update(TIME_LIMIT - Duration::from_millis(100));
        assert_eq!(game.outcome(), None, "60秒までは続く");
        game.update(Duration::from_millis(200));
        assert_eq!(game.outcome(), Some(Outcome::TimeUp));
        let result = game.result();
        assert_eq!((result.correct, result.total), (0, 1));
        assert_eq!(result.avg_latency_ms, TIME_LIMIT.as_millis() as f64);
    }

    #[test]
    fn flying_off_is_an_immediate_game_over() {
        let mut game = calm_game();
        game.on_step_event(StepEvent::Landed(Landing::Flown));
        assert_eq!(
            game.outcome(),
            Some(Outcome::Flown),
            "吹っ飛んだら即GAME OVER"
        );
        let result = game.result();
        assert_eq!((result.correct, result.total), (0, 1));
    }

    #[test]
    fn falling_off_is_an_immediate_game_over_with_the_gasp() {
        let mut game = calm_game();
        game.on_step_event(StepEvent::FellOff);
        assert_eq!(
            game.outcome(),
            Some(Outcome::Flown),
            "盤から落ちたら吹っ飛びと同じく即GAME OVER"
        );
        let result = game.result();
        assert_eq!((result.correct, result.total), (0, 1));
        assert_eq!(GASP_LINE, "ああっ!!", "GAME OVER共通の女の子のセリフ");
    }

    #[test]
    fn rolling_off_the_rim_during_play_is_a_game_over() {
        // 盤の縁には壁が無いので、左端から左へ転がると落ちてGAME OVERになる
        let mut game = calm_game();
        game.top = Top::new((0.6, 0.5));
        game.top.vel = (-3.0, 0.0);
        game.update(Duration::from_millis(500));
        assert_eq!(game.outcome(), Some(Outcome::Flown));
        assert_eq!(game.result().correct, 0);
    }

    #[test]
    fn hard_braking_throws_the_top_into_a_bump_and_it_flies_off() {
        // 気づくのが大きく遅れた信号: 下限の距離でブレーキを踏み、約3Gの制動Gがかかる
        let truck = Truck::with_course(
            vec![RoadEvent::Signal {
                at: 35.0,
                notice_delay: 5.0,
            }],
            1000.0,
        );
        let mut game = BeigomaGame::with_truck(truck);
        finish_countdown(&mut game);
        let mut t = Duration::ZERO;
        while game.truck.current_g().magnitude() <= HIGH_G_THRESHOLD {
            game.update(STEP);
            t += STEP;
            assert!(t < Duration::from_secs(10), "ブレーキがかからなかった");
        }
        // 障害物のすぐ下に置くと、ブレーキのGで前(上)へ押されて障害物を踏む
        let (x, y) = flat_below_a_bump(&game.board);
        game.top = Top::new((x as f64 + 0.5, y as f64 + 0.1));
        for _ in 0..100 {
            game.update(STEP);
            if game.outcome().is_some() {
                break;
            }
        }
        assert_eq!(game.outcome(), Some(Outcome::Flown));
        assert_eq!(game.result().correct, 0);
    }

    #[test]
    fn bouncing_does_not_end_the_game_and_shows_a_message() {
        let mut game = calm_game();
        game.on_step_event(StepEvent::Landed(Landing::Bounce));
        assert_eq!(game.outcome(), None, "弾かれても続く");
        assert!(game.message.is_some(), "ぴよーんと弾かれた一言を出す");
        game.on_step_event(StepEvent::Landed(Landing::Light));
        assert_eq!(game.outcome(), None);
    }

    fn message_text(game: &BeigomaGame) -> Option<&'static str> {
        game.message.map(|(text, _)| text)
    }

    #[test]
    fn sinking_and_escaping_show_their_messages() {
        let mut game = calm_game();
        game.on_step_event(StepEvent::Sank);
        assert_eq!(game.outcome(), None, "凹にハマっても続く");
        assert_eq!(message_text(&game), Some("ズボッ"));
        game.on_step_event(StepEvent::Escaped);
        assert_eq!(game.outcome(), None);
        assert_eq!(message_text(&game), Some("ぬけた!"));
    }

    #[test]
    fn grazing_a_hollow_bounces_like_a_bump_or_ends_the_game() {
        let mut game = calm_game();
        game.on_step_event(StepEvent::Grazed(Landing::Bounce));
        assert_eq!(game.outcome(), None, "側面をこすって弾かれても続く");
        assert_eq!(
            message_text(&game),
            Some("ぴよーん!! ああっ!!"),
            "凸で弾かれた時と同じ一言"
        );
        game.on_step_event(StepEvent::Grazed(Landing::Light));
        assert_eq!(game.outcome(), None);
        game.on_step_event(StepEvent::Grazed(Landing::Flown));
        assert_eq!(
            game.outcome(),
            Some(Outcome::Flown),
            "側面で吹っ飛んだら即GAME OVER"
        );
        let result = game.result();
        assert_eq!((result.correct, result.total), (0, 1));
    }

    #[test]
    fn rolling_head_on_into_a_hollow_sinks_during_play() {
        // 左隣が平坦な凹へ、右向きにまっすぐ転がり込む
        let mut game = calm_game();
        let (hx, hy) = (1..BOARD_HEIGHT)
            .flat_map(|y| (1..BOARD_WIDTH).map(move |x| (x, y)))
            .find(|&(x, y)| {
                game.board.cell(x, y) == Cell::Hollow && game.board.cell(x - 1, y) == Cell::Flat
            })
            .expect("左隣が平坦な凹がある");
        game.top = Top::new((hx as f64 - 0.02, hy as f64 + 0.5));
        game.top.vel = (3.0, 0.0);
        game.update(STEP);
        assert_eq!(game.top.state, TopState::Sunk);
        assert_eq!(message_text(&game), Some("ズボッ"));
        game.update(Duration::from_secs(2));
        assert_eq!(game.outcome(), None, "ハマっても続く");
        assert_eq!(
            (game.top.pos.0 as usize, game.top.pos.1 as usize),
            (hx, hy),
            "傾けなければ凹から出ない"
        );
    }

    // --- 2ROUND制 ---

    /// ROUND1をゴールで終える
    fn clear_round(game: &mut BeigomaGame) {
        game.on_step_event(StepEvent::Goal);
        assert!(matches!(game.outcome(), Some(Outcome::Cleared { .. })));
    }

    #[test]
    fn round1_end_leads_to_round2_countdown_after_the_hold() {
        let mut game = calm_game();
        game.handle_key(key(KeyCode::Right));
        game.update(Duration::from_secs(3));
        clear_round(&mut game);
        game.update(END_HOLD - STEP);
        assert!(game.outcome().is_some(), "結果の表示をしばらく出す");
        assert_eq!(game.round_index, 0);
        assert!(!game.is_finished(), "ROUND1をクリアしたらROUND2へ進む");
        game.update(STEP);
        assert!(!game.is_finished());
        assert!(is_countdown(&game), "ROUND2もカウントダウンから");
        assert_eq!(countdown_phase(&game), Some(Phase::Three));
        assert_eq!(game.round_index, 1);
        assert_eq!(game.params, round_params(1));
        assert_eq!(game.outcome(), None);
        // 盤はROUND2の配置に作り直し、ベーゴマは投入位置に戻る
        let layout = round_params(1).layout;
        for (y, row) in layout.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                let expected = match c {
                    '#' => Some(Cell::Bump),
                    'u' => Some(Cell::Hollow),
                    _ => None,
                };
                if let Some(cell) = expected {
                    assert_eq!(game.board.cell(x, y), cell, "({x},{y})");
                }
            }
        }
        assert_eq!(game.top.pos, game.board.start_position());
        assert_eq!(game.top.vel, (0.0, 0.0));
        assert_eq!(game.tilt.roll(), 0.0, "傾きもROUNDごとに水平へ戻す");
        assert_eq!(game.elapsed, Duration::ZERO, "制限時間はROUNDごと");
        assert_eq!(game.result().total, 1, "ROUND1の結果は記録済み");
        game.update(COUNTDOWN_TOTAL);
        assert!(is_playing(&game));
        let text = rendered_text(&game, AREA);
        assert!(text.contains("残り60.0秒"), "{text}");
    }

    #[test]
    fn round1_failure_ends_the_session_without_round2() {
        // 吹っ飛び・場外・時間切れのどれでも、ROUND2へ進まずそこでセッション終了
        let failures: [fn(&mut BeigomaGame); 3] = [
            |game| game.on_step_event(StepEvent::Landed(Landing::Flown)),
            |game| game.on_step_event(StepEvent::FellOff),
            |game| game.update(TIME_LIMIT + STEP),
        ];
        for (i, fail) in failures.into_iter().enumerate() {
            let mut game = calm_game();
            fail(&mut game);
            assert!(
                matches!(game.outcome(), Some(Outcome::Flown | Outcome::TimeUp)),
                "case{i}: {:?}",
                game.outcome()
            );
            assert!(
                !game.is_finished(),
                "case{i}: GAME OVERの表示をしばらく出す"
            );
            game.update(END_HOLD);
            assert!(
                game.is_finished(),
                "case{i}: ROUND2へは進まずセッション終了"
            );
            assert_eq!(game.round_index, 0, "case{i}");
            assert!(
                !is_countdown(&game),
                "case{i}: ROUND2のカウントダウンを始めない"
            );
            game.update(Duration::from_secs(5));
            assert!(game.is_finished(), "case{i}");
            assert_eq!(game.round_index, 0, "case{i}");
            let result = game.result();
            assert_eq!(
                (result.correct, result.total),
                (0, 1),
                "case{i}: ROUND1の1件だけ"
            );
        }
    }

    #[test]
    fn session_finishes_after_round2_end_display() {
        for round2_cleared in [true, false] {
            let mut game = calm_game();
            clear_round(&mut game);
            game.update(END_HOLD);
            finish_countdown(&mut game);
            assert_eq!(game.round_index, 1);
            if round2_cleared {
                clear_round(&mut game);
            } else {
                game.on_step_event(StepEvent::FellOff);
            }
            assert!(!game.is_finished(), "ROUND2の結果の表示をしばらく出す");
            game.update(END_HOLD - STEP);
            assert!(!game.is_finished());
            game.update(STEP);
            assert!(game.is_finished(), "ROUND2で終わり");
            assert_eq!(game.round_index + 1, ROUNDS_PER_SESSION);
            game.update(END_HOLD);
            assert!(!is_countdown(&game), "3つ目のROUNDは無い");
            let result = game.result();
            assert_eq!(result.total, 2);
            assert_eq!(result.correct, if round2_cleared { 2 } else { 1 });
        }
    }

    #[test]
    fn each_round_generates_a_goal_from_its_rule() {
        for _ in 0..10 {
            let mut game = BeigomaGame::new();
            let rule = round_params(0).goal_rule;
            assert!(
                Board::goal_candidates(round_params(0).layout, &rule).contains(&game.board.goal())
            );
            game.update(COUNTDOWN_TOTAL);
            clear_round(&mut game);
            game.update(END_HOLD);
            let params = round_params(1);
            let goal = game.board.goal();
            assert!(Board::goal_candidates(params.layout, &params.goal_rule).contains(&goal));
            assert!(game.board.straight_line_is_blocked(
                game.board.start_position(),
                (goal.0 as f64 + 0.5, goal.1 as f64 + 0.5)
            ));
        }
    }

    #[test]
    fn hud_shows_the_round_label() {
        let mut game = calm_game();
        let text = rendered_text(&game, AREA);
        assert!(text.contains("ROUND1やさしい"), "{text}");
        assert!(text.contains("べー"), "{text}");
        clear_round(&mut game);
        game.update(END_HOLD);
        let text = rendered_text(&game, AREA);
        assert!(
            text.contains("ROUND2むずかしい"),
            "ROUND2のカウントダウン中から出す: {text}"
        );
        assert!(!text.contains("ROUND1"), "{text}");
    }

    #[test]
    fn the_session_finishes_after_the_end_display() {
        let mut game = calm_game();
        game.finish(Outcome::Flown);
        assert!(!game.is_finished(), "GAME OVERの表示をしばらく出す");
        game.update(END_HOLD - STEP);
        assert!(!game.is_finished());
        game.update(STEP);
        assert!(game.is_finished());
    }

    #[test]
    fn the_result_is_recorded_only_once() {
        let mut game = calm_game();
        place_just_before_goal(&mut game);
        game.update(STEP);
        assert!(matches!(game.outcome(), Some(Outcome::Cleared { .. })));
        // 終わった後(結果の表示中)は時間切れにも吹っ飛びにもならない
        game.elapsed = TIME_LIMIT;
        game.update(END_HOLD - STEP);
        game.on_step_event(StepEvent::Landed(Landing::Flown));
        game.finish(Outcome::TimeUp);
        assert!(matches!(game.outcome(), Some(Outcome::Cleared { .. })));
        assert_eq!(game.result().total, 1, "ROUNDごとに1回だけ記録する");
    }

    #[test]
    fn input_and_physics_stop_after_the_end() {
        let mut game = calm_game();
        game.handle_key(key(KeyCode::Right));
        game.finish(Outcome::Flown);
        let (pos, roll) = (game.top.pos, game.tilt.roll());
        game.handle_key(key(KeyCode::Right));
        game.update(Duration::from_millis(500));
        assert_eq!(game.top.pos, pos, "終わった後は転がらない");
        assert_eq!(game.tilt.roll(), roll, "終わった後のキーは無視する");
    }

    // --- 描画 ---

    #[test]
    fn render_shows_both_views_and_the_remaining_time() {
        let game = calm_game();
        let text = rendered_text(&game, AREA);
        assert!(text.contains("べー"), "{text}");
        assert!(text.contains("残り60.0秒"), "{text}");
        assert!(text.contains("軽トラ視点"), "{text}");
        assert!(text.contains("盤面"), "{text}");
        assert!(text.contains(render::GOAL_GLYPH));
        assert!(text.contains(render::BUMP_GLYPH), "凸を描く");
        assert!(text.contains(render::HOLLOW_GLYPH), "凹を描く");
        assert!(
            text.contains(render::TOP_SPIN_GLYPHS[0]),
            "投入されたベーゴマを描く"
        );
    }

    #[test]
    fn render_shows_how_the_game_ended() {
        for (outcome, label) in [
            (Outcome::Flown, "GAMEOVER"),
            (Outcome::TimeUp, "TIMEUP"),
            (
                Outcome::Cleared {
                    time: Duration::from_millis(23_400),
                },
                "GOAL!!",
            ),
        ] {
            let mut game = calm_game();
            game.finish(outcome);
            let text = rendered_text(&game, AREA);
            assert!(text.contains(label), "{outcome:?}: {text}");
        }
    }

    #[test]
    fn flying_off_and_falling_off_show_the_same_game_over() {
        let mut flown = calm_game();
        flown.on_step_event(StepEvent::Landed(Landing::Flown));
        let mut fell = calm_game();
        fell.on_step_event(StepEvent::FellOff);
        let flown_text = rendered_text(&flown, AREA);
        let fell_text = rendered_text(&fell, AREA);
        for text in [&flown_text, &fell_text] {
            assert!(text.contains("GAMEOVER"), "{text}");
            assert!(
                !text.contains("吹っ飛んだ"),
                "吹っ飛び専用の文言は出さない: {text}"
            );
        }
        assert_eq!(flown_text, fell_text, "吹っ飛び・場外で同じ表示");
    }

    #[test]
    fn falling_off_starts_the_star_animation() {
        let mut game = calm_game();
        game.on_step_event(StepEvent::FellOff);
        assert_eq!(
            game.star_frame(),
            Some(0),
            "GAME OVER直後は星の1コマ目"
        );
    }

    #[test]
    fn star_animation_advances_and_stays_on_the_last_frame() {
        let mut game = calm_game();
        game.on_step_event(StepEvent::FellOff);
        game.update(render::STAR_ANIM_FRAME);
        assert_eq!(game.star_frame(), Some(1), "時間が経つとコマが進む");
        game.update(render::STAR_ANIM_FRAME * 10);
        assert_eq!(
            game.star_frame(),
            Some(render::STAR_ANIM_GLYPHS.len() - 1),
            "最後のコマで止まる(範囲外にならない)"
        );
    }

    #[test]
    fn timeup_and_cleared_do_not_show_the_star_animation() {
        let mut timeup = calm_game();
        timeup.finish(Outcome::TimeUp);
        assert_eq!(timeup.star_frame(), None, "時間切れは星の演出を出さない");

        let mut cleared = calm_game();
        place_just_before_goal(&mut cleared);
        cleared.update(STEP);
        assert_eq!(cleared.star_frame(), None, "クリア時は星の演出を出さない");
    }

    #[test]
    fn render_does_not_panic_in_tiny_areas() {
        let mut game = BeigomaGame::new();
        for _ in 0..3 {
            for (w, h) in [(1, 1), (5, 2), (10, 4), (30, 8), (60, 12)] {
                rendered_text(&game, Rect::new(0, 0, w, h));
            }
            game.update(Duration::from_secs(20));
        }
    }
}
