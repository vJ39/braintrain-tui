//! べー(beigoma): 軽トラの荷台で女の子が持つベーゴマ盤を傾け、ベーゴマを唯一のゴールへ導く。
//! 仕様は docs/beigoma-game-spec.md
//!
//! - 盤の傾きは前後・左右の2軸で、矢印キーを押すたびに一定量動く連打式(board.rs)
//! - 軽トラは自動で走り、段差・信号・障害物回避のたびに運動方程式から求めたGが盤にかかる(truck.rs)
//! - 凸を踏んだ瞬間のGが大きいと弾かれ、さらに大きいと吹っ飛んで即GAME OVER
//! - 凹に正面から入るとハマり(強く傾けると抜け出せる)、斜めに入ると側面をこすって弾かれる・吹っ飛ぶ
//! - 盤の縁に壁は無く、盤から落ちても(場外)即GAME OVER
//! - 吹っ飛び・場外のGAME OVERでは、ゴールで待機中のもの以外の全ベーゴマが盤の外へ加速しながら吹っ飛んでいき
//!   (速く回りながら左右に振れる)、盤の外へ出たところから星の演出(キラーン)になる。
//!   音は「ふいっ」(SeKind::Star)の少し後にブブー(SeKind::Incorrect)を鳴らす
//! - 1セッションは2ROUND(ROUND1=やさしい、ROUND2=むずかしい)。盤の配置は共通で、
//!   ROUND1のゴールは投入位置から最も遠い位置に固定、ROUND2のゴールはランダム
//! - ROUND2はベーゴマ2個を共通の傾きで同時に操作する。どちらか1個でも吹っ飛び・場外になったら即GAME OVER、
//!   ゴールに入った方はその場で待機し(settled)、全部がゴールに揃ったらクリア
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

/// ベーゴマの回転の見た目が1コマ進む間隔。8コマで1周280ms(従来の4コマ×70msと同じ速さ)
const SPIN_FRAME_INTERVAL: Duration = Duration::from_millis(35);

/// 吹っ飛んで飛んでいる間の回転の見た目が1コマ進む間隔。転がっている時の2倍以上の速さで回す
const FLY_SPIN_FRAME_INTERVAL: Duration = Duration::from_millis(15);

/// 吹っ飛んで飛んでいる間、描く位置を進む向きの左右へ振る幅(マス)と、1往復の周期
const FLY_WOBBLE: f64 = 0.8;
const FLY_WOBBLE_PERIOD: Duration = Duration::from_millis(120);

/// 吹っ飛び・場外のGAME OVERで「ふいっ」(SeKind::Star、約110ms)を鳴らしてから、
/// ブブー(SeKind::Incorrect)を鳴らすまでの間。2つの音が重なって濁らないようにずらす
const OFF_BOARD_BUZZ_DELAY: Duration = Duration::from_millis(150);

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

/// 吹っ飛び・場外のGAME OVERで、盤の外へ飛んでいく途中の経過
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Flight {
    /// 飛び始めてからの時間
    elapsed: Duration,
    /// 盤の外へ出た時のelapsed(まだ盤の上ならNone)。星の演出はここから始める
    off_board_at: Option<Duration>,
}

impl Flight {
    /// 星の演出のコマ(盤の外へ出るまではNone)。最後のコマで止める
    fn star_frame(&self) -> Option<usize> {
        self.off_board_at.map(|at| {
            let frame = (self.elapsed.saturating_sub(at).as_millis()
                / render::STAR_ANIM_FRAME.as_millis()) as usize;
            frame.min(render::STAR_ANIM_GLYPHS.len() - 1)
        })
    }

    /// 飛んでいる間の回転の見た目のコマ(転がっている時より速く回す)
    fn spin_frame(&self) -> usize {
        (self.elapsed.as_nanos() / FLY_SPIN_FRAME_INTERVAL.as_nanos()) as usize
            % render::TOP_SPIN_GLYPHS.len()
    }

    /// 描く位置の、進む向き(dir、単位ベクトル)の左右への振れ(マス)。飛び始めは振れていない
    fn wobble(&self, dir: (f64, f64)) -> (f64, f64) {
        let phase = self.elapsed.as_secs_f64() / FLY_WOBBLE_PERIOD.as_secs_f64();
        let across = FLY_WOBBLE * (std::f64::consts::TAU * phase).sin();
        (-dir.1 * across, dir.0 * across)
    }
}

/// 1個のベーゴマの進行状態
struct TopSlot {
    top: Top,
    /// 個別にゴールへ達し、以後の物理更新を止めているか(他のベーゴマが揃うのを待つ)
    settled: bool,
    /// 吹っ飛び・場外のGAME OVERで盤の外へ飛んでいる途中(それ以外はNone)
    flight: Option<Flight>,
}

impl TopSlot {
    /// boardのposに置く(置いた位置で触れている凹凸には既に触れている扱い)
    fn new(board: &Board, pos: (f64, f64)) -> Self {
        Self {
            top: Top::new(board, pos),
            settled: false,
            flight: None,
        }
    }

    /// 盤の外へ向けて飛ばし始める。既に盤の外にいれば、すぐに星の演出を始める
    fn launch(&mut self) {
        self.top.launch();
        self.flight = Some(Flight {
            elapsed: Duration::ZERO,
            off_board_at: (!Board::contains(self.top.pos)).then_some(Duration::ZERO),
        });
    }

    /// 飛んでいる途中ならdtだけ飛ばす。盤の外へ出た時刻を覚える
    fn fly(&mut self, dt: Duration) {
        let Some(flight) = self.flight.as_mut() else {
            return;
        };
        self.top.fly_away(dt);
        flight.elapsed += dt;
        if flight.off_board_at.is_none() && !Board::contains(self.top.pos) {
            flight.off_board_at = Some(flight.elapsed);
        }
    }

    /// 描く位置。飛んでいる途中は進む向きの左右へ振る
    fn view_pos(&self) -> (f64, f64) {
        let pos = self.top.pos;
        let Some(flight) = self.flight else {
            return pos;
        };
        let (vx, vy) = self.top.vel;
        let speed = vx.hypot(vy);
        if speed < 1e-9 {
            return pos;
        }
        let (dx, dy) = flight.wobble((vx / speed, vy / speed));
        (pos.0 + dx, pos.1 + dy)
    }
}

/// 盤上の一言。重大度は低い順に セーフ < ガタッ < ズボッ/ぬけた! < ぴよーん!!ああっ!!
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reaction {
    Safe,
    Hop,
    Sank,
    Escaped,
    Bounce,
}

impl Reaction {
    /// 盤上の出来事に対応する一言。GAME OVER・ゴールは一言を出さないのでNone
    fn of(event: StepEvent) -> Option<Self> {
        match event {
            StepEvent::Hopped { .. } => Some(Self::Hop),
            StepEvent::Landed(Landing::Light) | StepEvent::Grazed(Landing::Light) => {
                Some(Self::Safe)
            }
            // 凹凸の側面をこすって弾かれた時も、凸で弾かれた時と同じ演出
            StepEvent::Landed(Landing::Bounce) | StepEvent::Grazed(Landing::Bounce) => {
                Some(Self::Bounce)
            }
            StepEvent::Sank => Some(Self::Sank),
            StepEvent::Escaped => Some(Self::Escaped),
            StepEvent::Landed(Landing::Flown)
            | StepEvent::Grazed(Landing::Flown)
            | StepEvent::FellOff
            | StepEvent::Goal => None,
        }
    }

    fn severity(self) -> u8 {
        match self {
            Self::Safe => 0,
            Self::Hop => 1,
            Self::Sank | Self::Escaped => 2,
            Self::Bounce => 3,
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::Safe => "セーフ",
            Self::Hop => "ガタッ!",
            Self::Sank => "ズボッ",
            Self::Escaped => "ぬけた!",
            Self::Bounce => "ぴよーん!! ああっ!!",
        }
    }

    /// この一言と一緒に鳴らすSE(セーフ・ぬけた!はSEなし)
    fn se(self) -> Option<SeKind> {
        match self {
            Self::Safe | Self::Escaped => None,
            Self::Hop => Some(SeKind::BeigomaBump),
            Self::Sank => Some(SeKind::BeigomaSink),
            Self::Bounce => Some(SeKind::Incorrect),
        }
    }
}

/// 同じステップの出来事のうち、最も重大度の高い一言(同じ重大度なら先に来た方)
fn strongest_reaction(events: &[StepEvent]) -> Option<Reaction> {
    events.iter().filter_map(|&event| Reaction::of(event)).fold(
        None,
        |best: Option<Reaction>, reaction| match best {
            Some(best) if best.severity() >= reaction.severity() => Some(best),
            _ => Some(reaction),
        },
    )
}

/// 吹っ飛び・場外(GAME OVER相当)の出来事か
fn is_game_over_event(event: StepEvent) -> bool {
    matches!(
        event,
        StepEvent::Landed(Landing::Flown) | StepEvent::Grazed(Landing::Flown) | StepEvent::FellOff
    )
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
    /// このROUNDのベーゴマ(要素数はparams.top_count。ROUND1=1、ROUND2=2)
    tops: Vec<TopSlot>,
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
    /// テスト用: 鳴らしたSEの記録(音は端末で確かめられないため)
    #[cfg(test)]
    se_log: Vec<SeKind>,
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
        let tops = Self::new_slots(&board, params.top_count);
        let mut game = Self {
            round_index: 0,
            params,
            board,
            tops,
            tilt: Tilt::new(),
            truck,
            elapsed: Duration::ZERO,
            status: Status::Playing,
            tracker: ScoreTracker::with_session_length(ROUNDS_PER_SESSION),
            message: None,
            board_renderer: BoardRenderer::new(),
            truck_view: TruckViewRenderer::new(),
            #[cfg(test)]
            se_log: Vec::new(),
        };
        game.start_round(0);
        game
    }

    /// SEを鳴らす(テストでは鳴らしたSEを記録する)
    fn play_se(&mut self, se: SeKind) {
        #[cfg(test)]
        self.se_log.push(se);
        audio::play_se(se);
    }

    /// 盤の投入位置に並べたcount個のベーゴマ
    fn new_slots(board: &Board, count: usize) -> Vec<TopSlot> {
        board
            .start_positions(count)
            .into_iter()
            .map(|pos| TopSlot::new(board, pos))
            .collect()
    }

    /// round_index番目のROUNDをカウントダウンから始める。盤を作り直し(ゴールはROUNDごとの選び方で置く)、
    /// ベーゴマは投入位置で待たせる。軽トラのコースはそのまま続く
    fn start_round(&mut self, round_index: u32) {
        self.round_index = round_index;
        self.params = round_params(round_index);
        self.board = Board::generate(&self.params, &mut rand::thread_rng());
        self.tops = Self::new_slots(&self.board, self.params.top_count);
        self.tilt = Tilt::new();
        self.elapsed = Duration::ZERO;
        self.message = None;
        let state = CountdownState::new();
        // 最初のフェーズ「3」の音
        if let Some(phase) = state.phase() {
            self.play_se(phase.se());
        }
        self.status = Status::Countdown { state };
    }

    /// GO!!が終わった: ベーゴマを投入位置に投入し、制限時間を数え始める
    fn drop_top(&mut self) {
        self.tops = Self::new_slots(&self.board, self.params.top_count);
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

    /// 1ステップ(MAX_STEP以下)だけ進める。ゴールで待機中(settled)のベーゴマは物理更新しない
    fn step(&mut self, dt: Duration) {
        self.tilt.update(dt);
        self.truck.update(dt);
        let g = self.truck.current_g();
        let mut events = Vec::new();
        for (i, slot) in self.tops.iter_mut().enumerate().filter(|(_, s)| !s.settled) {
            if let Some(event) = slot.top.step(&self.board, dt, &self.tilt, g) {
                events.push((i, event));
            }
        }
        self.elapsed += dt;
        self.on_step_events(events);
        if self.is_playing() && self.elapsed >= TIME_LIMIT {
            self.finish(Outcome::TimeUp, false);
        }
    }

    /// 1個目のベーゴマの出来事だけを反映する(ROUND1の単一ベーゴマの判定をテストで直接起こすため)
    #[cfg(test)]
    fn on_step_event(&mut self, event: StepEvent) {
        self.on_step_events(vec![(0, event)]);
    }

    /// 1ステップの間に各ベーゴマ(スロット番号, 出来事)で起きたことを反映する。
    /// 1. どれか1個でも吹っ飛び・場外なら、他の状態によらず即GAME OVER
    /// 2. ゴールに入ったスロットは待機(settled)にし、全部揃ったらクリア
    /// 3. それ以外の出来事からは、最も重大度の高い一言を1つだけ出す(弾かれ系ならSEも1回だけ鳴らす)
    fn on_step_events(&mut self, events: Vec<(usize, StepEvent)>) {
        if !self.is_playing() {
            return;
        }
        if events.iter().any(|&(_, event)| is_game_over_event(event)) {
            let fell_off = events.iter().any(|&(_, event)| event == StepEvent::FellOff);
            self.finish(Outcome::Flown, fell_off);
            return;
        }
        for &(i, event) in &events {
            if event == StepEvent::Goal {
                if let Some(slot) = self.tops.get_mut(i) {
                    slot.settled = true;
                }
            }
        }
        if self.tops.iter().all(|slot| slot.settled) {
            self.finish(Outcome::Cleared { time: self.elapsed }, false);
            return;
        }
        let step_events: Vec<StepEvent> = events.iter().map(|&(_, event)| event).collect();
        if let Some(reaction) = strongest_reaction(&step_events) {
            if let Some(se) = reaction.se() {
                self.play_se(se);
            }
            self.message = Some((reaction.text(), Duration::ZERO));
        }
    }

    /// ROUNDを終える。結果はROUNDごとに1回だけ記録する(成功はクリアタイム、失敗は制限時間を反応時間として記録)。
    /// fell_offは吹っ飛び(Outcome::Flown)の原因が場外落下(StepEvent::FellOff)だったか
    fn finish(&mut self, outcome: Outcome, fell_off: bool) {
        if !self.is_playing() {
            return;
        }
        let (success, latency) = match outcome {
            Outcome::Cleared { time } => (true, time),
            Outcome::Flown | Outcome::TimeUp => (false, TIME_LIMIT),
        };
        self.tracker.record(success, latency.as_millis() as f64);
        // 場外落下は専用の落下音、吹っ飛びは「ふいっ」という音
        // (どちらもブブーはOFF_BOARD_BUZZ_DELAY後に鳴らす)、時間切れは従来のブザー音のまま
        self.play_se(match outcome {
            Outcome::Cleared { .. } => SeKind::Correct,
            Outcome::Flown if fell_off => SeKind::BeigomaFalloff,
            Outcome::Flown => SeKind::Star,
            Outcome::TimeUp => SeKind::Incorrect,
        });
        if outcome == Outcome::Flown {
            // 共通の傾きで一蓮托生のため、原因になった1個だけでなく、ゴールで待機中のもの以外を全部吹っ飛ばす
            for slot in self.tops.iter_mut().filter(|slot| !slot.settled) {
                slot.launch();
            }
        }
        self.status = Status::Ended {
            outcome,
            shown: Duration::ZERO,
        };
    }

    /// 終了表示の間に時間がdt進み、表示してからの時間がbeforeからafterになった。
    /// 吹っ飛び・場外なら、ベーゴマを盤の外へ飛ばし続け(小さなステップに分けて、盤の外へ出た時刻を正しく取る)、
    /// OFF_BOARD_BUZZ_DELAYを過ぎた時に1回だけブブーを鳴らす
    fn update_ended(&mut self, outcome: Outcome, before: Duration, after: Duration, dt: Duration) {
        if outcome != Outcome::Flown {
            return;
        }
        let mut remaining = dt;
        while !remaining.is_zero() {
            let step = remaining.min(MAX_STEP);
            for slot in &mut self.tops {
                slot.fly(step);
            }
            remaining -= step;
        }
        if before < OFF_BOARD_BUZZ_DELAY && after >= OFF_BOARD_BUZZ_DELAY {
            self.play_se(SeKind::Incorrect);
        }
    }

    /// 回転の見た目のコマ
    fn spin_frame(&self) -> usize {
        (self.elapsed.as_millis() / SPIN_FRAME_INTERVAL.as_millis()) as usize
            % render::TOP_SPIN_GLYPHS.len()
    }

    /// 盤面に描く各ベーゴマの見た目。吹っ飛び・場外のGAME OVERでは、ゴールで待機中のもの以外の全ベーゴマが
    /// 盤の外へ吹っ飛んでいき(速く回りながら左右に振れる)、盤の外へ出たところから星の演出になる
    fn top_views(&self) -> Vec<TopView> {
        let spin_frame = self.spin_frame();
        self.tops
            .iter()
            .map(|slot| {
                let star_frame = slot.flight.and_then(|flight| flight.star_frame());
                TopView {
                    pos: slot.view_pos(),
                    airborne: slot.top.is_airborne(),
                    spin_frame: slot.flight.map_or(spin_frame, |flight| flight.spin_frame()),
                    star_frame,
                    flying: slot.flight.is_some() && star_frame.is_none(),
                }
            })
            .collect()
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
                let phase = state.tick(dt);
                let finished = state.is_finished();
                if let Some(phase) = phase {
                    self.play_se(phase.se());
                }
                if finished {
                    self.drop_top();
                }
                return;
            }
            Status::Ended { outcome, shown } => {
                let (outcome, before) = (*outcome, *shown);
                *shown = shown.saturating_add(dt);
                let after = *shown;
                self.update_ended(outcome, before, after, dt);
                // 結果の表示を出し終えたら、クリアしていて次のROUNDがあれば進む
                if after >= END_HOLD && !self.is_last_round_ended() {
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
            elapsed: self.elapsed,
        };
        self.truck_view.render(frame, truck_inner, &info);

        let board_block = theme::panel(" 盤面 ")
            .border_style(Style::default().fg(self.status_color()))
            .title_bottom(
                theme::hints_line(&[("↑↓←→", "連打で傾ける"), ("q", "やめる")]).centered(),
            );
        let board_inner = board_block.inner(cols[1]);
        frame.render_widget(board_block, cols[1]);
        self.board_renderer.render(
            frame,
            board_inner,
            &self.board,
            &self.top_views(),
            &self.tilt,
        );
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
        round_params, Cell, TopState, BOARD_HEIGHT, BOARD_WIDTH, FLOWN_KICK, FLY_AWAY_EXIT_LIMIT,
        HIGH_G_THRESHOLD, MAX_SPEED, ROUNDS_PER_SESSION, TILT_STEP, TOP_PAIR_OFFSET_X, TOP_RADIUS,
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
        game.tops[0].top = Top::new(&game.board, (gx as f64 - 0.02, gy as f64 + 0.5));
        game.tops[0].top.vel = (3.0, 0.0);
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
    fn one_physics_step_is_shorter_than_the_top_radius() {
        // 1ステップの移動量が半径より小さいので、凹凸に触れ始める瞬間を飛び越さない
        const { assert!(MAX_SPEED * (MAX_STEP.as_millis() as f64 / 1000.0) < TOP_RADIUS) };
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
            game.tops[0].top.pos,
            game.board.start_position(),
            "ベーゴマは投入位置で待っている"
        );
        assert_eq!(game.tops[0].top.vel, (0.0, 0.0));
        assert!(!game.tops[0].top.is_airborne());
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
            game.tops[0].top.pos,
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
            game.tops[0].top.pos,
            game.board.start_position(),
            "投入位置に置かれる"
        );
        assert_eq!(game.tops[0].top.vel, (0.0, 0.0));
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
        // 回転をもっと激しく見せるため、1周の長さは4コマ×70ms(#129)のまま、コマ数を増やして滑らかにする
        assert_eq!(SPIN_FRAME_INTERVAL, Duration::from_millis(35));
        assert_eq!(
            SPIN_FRAME_INTERVAL * render::TOP_SPIN_GLYPHS.len() as u32,
            Duration::from_millis(280),
            "1周の長さは変えない"
        );
    }

    #[test]
    fn spin_frame_steps_through_every_glyph_and_wraps() {
        let mut game = calm_game();
        let start = game.spin_frame();
        let len = render::TOP_SPIN_GLYPHS.len();
        for step in 1..=len {
            game.update(SPIN_FRAME_INTERVAL);
            assert_eq!(
                game.spin_frame(),
                (start + step) % len,
                "1間隔ごとに1コマずつ進み、1周で元に戻る"
            );
        }
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
        let start = game.tops[0].top.pos;
        game.handle_key(key(KeyCode::Right));
        game.handle_key(key(KeyCode::Right));
        game.update(Duration::from_millis(300));
        assert!(game.tops[0].top.pos.0 > start.0, "右へ傾けると右へ転がる");
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
        assert_eq!(stepped.tops[0].top.pos, once.tops[0].top.pos);
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
        game.tops[0].top = Top::new(&game.board, (0.6, 0.5));
        game.tops[0].top.vel = (-3.0, 0.0);
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
        game.tops[0].top = Top::new(&game.board, (x as f64 + 0.5, y as f64 + TOP_RADIUS + 0.1));
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
        game.tops[0].top = Top::new(
            &game.board,
            (hx as f64 - TOP_RADIUS - 0.02, hy as f64 + 0.5),
        );
        game.tops[0].top.vel = (3.0, 0.0);
        game.update(STEP);
        assert!(matches!(game.tops[0].top.state, TopState::Sunk { .. }));
        assert_eq!(message_text(&game), Some("ズボッ"));
        game.update(Duration::from_secs(2));
        assert_eq!(game.outcome(), None, "ハマっても続く");
        assert_eq!(
            (
                game.tops[0].top.pos.0 as usize,
                game.tops[0].top.pos.1 as usize
            ),
            (hx, hy),
            "傾けなければ凹から出ない"
        );
    }

    // --- 2ROUND制 ---

    /// ROUNDをゴールで終える(全部のベーゴマが同じステップでゴールに入る)
    fn clear_round(game: &mut BeigomaGame) {
        let goals = (0..game.tops.len()).map(|i| (i, StepEvent::Goal)).collect();
        game.on_step_events(goals);
        assert!(matches!(game.outcome(), Some(Outcome::Cleared { .. })));
    }

    fn top_positions(game: &BeigomaGame) -> Vec<(f64, f64)> {
        game.tops.iter().map(|slot| slot.top.pos).collect()
    }

    /// ROUND1をクリアしてROUND2のカウントダウンを終え、ベーゴマ2個を投入した直後。
    /// ゴールは決定的にするためROUND1_GOALに置き直す
    fn calm_round2() -> BeigomaGame {
        let mut game = calm_game();
        clear_round(&mut game);
        game.update(END_HOLD);
        finish_countdown(&mut game);
        assert_eq!(game.round_index, 1);
        game.board = Board::with_goal(round_params(1).layout, ROUND1_GOAL);
        game
    }

    /// i番目のベーゴマを、ゴールの左隣から右へ転がってゴールに入る直前に置く
    fn place_slot_just_before_goal(game: &mut BeigomaGame, i: usize) {
        let (gx, gy) = game.board.goal();
        game.tops[i].top = Top::new(&game.board, (gx as f64 - 0.02, gy as f64 + 0.5));
        game.tops[i].top.vel = (3.0, 0.0);
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
        // ROUND2はベーゴマ2個で、投入位置の左右に並んで待つ
        assert_eq!(top_positions(&game), game.board.start_positions(2));
        for slot in &game.tops {
            assert_eq!(slot.top.vel, (0.0, 0.0));
            assert!(!slot.settled);
        }
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
    fn hud_tells_that_round2_has_two_tops() {
        let mut game = calm_game();
        let text = rendered_text(&game, AREA);
        assert!(!text.contains("ベーゴマ2個"), "ROUND1には出さない: {text}");
        clear_round(&mut game);
        game.update(END_HOLD);
        let text = rendered_text(&game, AREA);
        assert!(text.contains("ベーゴマ2個"), "{text}");
    }

    #[test]
    fn the_session_finishes_after_the_end_display() {
        let mut game = calm_game();
        game.finish(Outcome::Flown, false);
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
        game.finish(Outcome::TimeUp, false);
        assert!(matches!(game.outcome(), Some(Outcome::Cleared { .. })));
        assert_eq!(game.result().total, 1, "ROUNDごとに1回だけ記録する");
    }

    #[test]
    fn input_and_physics_stop_after_the_end() {
        // 時間切れ・クリアの後は転がらない(吹っ飛びGAME OVERだけは盤の外へ飛んでいく演出を続ける)
        for outcome in [
            Outcome::TimeUp,
            Outcome::Cleared {
                time: Duration::from_secs(1),
            },
        ] {
            let mut game = calm_game();
            game.handle_key(key(KeyCode::Right));
            game.finish(outcome, false);
            let (pos, roll) = (game.tops[0].top.pos, game.tilt.roll());
            game.handle_key(key(KeyCode::Right));
            game.update(Duration::from_millis(500));
            assert_eq!(
                game.tops[0].top.pos, pos,
                "{outcome:?}: 終わった後は転がらない"
            );
            assert_eq!(
                game.tilt.roll(),
                roll,
                "{outcome:?}: 終わった後のキーは無視する"
            );
        }
        let mut game = calm_game();
        game.finish(Outcome::Flown, false);
        let roll = game.tilt.roll();
        game.handle_key(key(KeyCode::Right));
        game.update(Duration::from_millis(500));
        assert_eq!(game.tilt.roll(), roll, "吹っ飛んだ後のキーも無視する");
    }

    // --- ROUND2: ベーゴマ2個 ---

    #[test]
    fn round1_drops_a_single_top_at_the_start_position() {
        let game = calm_game();
        assert_eq!(game.tops.len(), 1);
        assert_eq!(top_positions(&game), vec![game.board.start_position()]);
    }

    #[test]
    fn round2_drops_two_tops_on_both_sides_of_the_start_position() {
        let game = calm_round2();
        assert_eq!(game.tops.len(), 2);
        let (sx, sy) = game.board.start_position();
        assert_eq!(
            top_positions(&game),
            vec![(sx - TOP_PAIR_OFFSET_X, sy), (sx + TOP_PAIR_OFFSET_X, sy)]
        );
        assert!(game.tops.iter().all(|slot| !slot.settled));
    }

    #[test]
    fn both_tops_roll_under_the_same_tilt() {
        let mut game = calm_round2();
        let start = top_positions(&game);
        game.handle_key(key(KeyCode::Up));
        game.handle_key(key(KeyCode::Up));
        game.update(Duration::from_millis(300));
        for (i, slot) in game.tops.iter().enumerate() {
            assert!(
                slot.top.pos.1 < start[i].1,
                "slot{i}: 前へ傾けると前へ転がる"
            );
        }
        assert_eq!(game.outcome(), None);
    }

    #[test]
    fn the_two_tops_have_independent_physics() {
        // 片方だけ凸の手前に置くと、その片方だけが飛び上がる
        let mut game = calm_round2();
        let (x, y) = flat_below_a_bump(&game.board);
        game.tops[0].top = Top::new(&game.board, (x as f64 + 0.5, y as f64 + TOP_RADIUS + 0.02));
        game.tops[0].top.vel = (0.0, -3.0);
        game.update(STEP);
        assert!(game.tops[0].top.is_airborne(), "凸を踏んだ方は飛び上がる");
        assert!(!game.tops[1].top.is_airborne(), "もう片方は影響を受けない");
        assert_eq!(game.outcome(), None);
    }

    #[test]
    fn one_top_failing_is_an_immediate_game_over_even_if_the_other_is_safe() {
        for failure in [
            StepEvent::FellOff,
            StepEvent::Landed(Landing::Flown),
            StepEvent::Grazed(Landing::Flown),
        ] {
            for i in 0..2 {
                let mut game = calm_round2();
                game.on_step_events(vec![(i, failure)]);
                assert_eq!(
                    game.outcome(),
                    Some(Outcome::Flown),
                    "slot{i} {failure:?}: 片方だけでも即GAME OVER"
                );
                let result = game.result();
                assert_eq!((result.correct, result.total), (1, 2), "{failure:?}");
            }
        }
    }

    #[test]
    fn one_top_rolling_off_the_rim_ends_round2() {
        let mut game = calm_round2();
        game.tops[1].top = Top::new(&game.board, (0.6, 5.5));
        game.tops[1].top.vel = (-3.0, 0.0);
        for _ in 0..50 {
            game.update(STEP);
            if game.outcome().is_some() {
                break;
            }
        }
        assert_eq!(game.outcome(), Some(Outcome::Flown));
        assert!(
            Board::contains(game.tops[0].top.pos),
            "GAME OVERになった時点では、もう片方は盤の上に残っている"
        );
    }

    #[test]
    fn both_tops_failing_in_the_same_step_finishes_only_once() {
        let mut game = calm_round2();
        game.on_step_events(vec![
            (0, StepEvent::FellOff),
            (1, StepEvent::Landed(Landing::Flown)),
        ]);
        assert_eq!(game.outcome(), Some(Outcome::Flown));
        assert_eq!(game.result().total, 2, "ROUND2の結果は1件だけ記録する");
    }

    #[test]
    fn one_top_at_the_goal_waits_for_the_other() {
        let mut game = calm_round2();
        place_slot_just_before_goal(&mut game, 0);
        game.update(STEP);
        assert!(game.tops[0].settled, "ゴールに入った方は待機する");
        assert!(!game.tops[1].settled);
        assert_eq!(game.outcome(), None, "もう片方が揃うまでは続く");
        assert!(is_playing(&game));
        game.update(Duration::from_secs(1));
        place_slot_just_before_goal(&mut game, 1);
        game.update(STEP);
        let Some(Outcome::Cleared { time }) = game.outcome() else {
            panic!("両方ゴールしたら成功: {:?}", game.outcome());
        };
        assert_eq!(time, game.elapsed, "揃った時点のタイムを記録する");
        assert!(time >= Duration::from_secs(1));
        assert_eq!(game.result().correct, 2);
    }

    #[test]
    fn both_tops_reaching_the_goal_in_the_same_step_clears() {
        let mut game = calm_round2();
        game.on_step_events(vec![(0, StepEvent::Goal), (1, StepEvent::Goal)]);
        assert!(matches!(game.outcome(), Some(Outcome::Cleared { .. })));
    }

    #[test]
    fn a_game_over_beats_a_goal_in_the_same_step() {
        for events in [
            vec![(0, StepEvent::Goal), (1, StepEvent::FellOff)],
            vec![(1, StepEvent::FellOff), (0, StepEvent::Goal)],
            vec![(0, StepEvent::Goal), (1, StepEvent::Landed(Landing::Flown))],
        ] {
            let mut game = calm_round2();
            game.on_step_events(events.clone());
            assert_eq!(game.outcome(), Some(Outcome::Flown), "{events:?}");
        }
        // 先にゴールで待機していても、もう片方が吹っ飛んだらGAME OVER
        let mut game = calm_round2();
        game.on_step_events(vec![(0, StepEvent::Goal)]);
        game.on_step_events(vec![(1, StepEvent::Grazed(Landing::Flown))]);
        assert_eq!(game.outcome(), Some(Outcome::Flown));
    }

    #[test]
    fn a_settled_top_stays_put_while_the_other_keeps_rolling() {
        let mut game = calm_round2();
        place_slot_just_before_goal(&mut game, 0);
        game.update(STEP);
        assert!(game.tops[0].settled);
        let (settled_pos, other_pos) = (game.tops[0].top.pos, game.tops[1].top.pos);
        game.handle_key(key(KeyCode::Up));
        game.handle_key(key(KeyCode::Up));
        game.update(Duration::from_millis(500));
        assert_eq!(game.outcome(), None);
        assert_eq!(game.tops[0].top.pos, settled_pos, "物理更新を止めている");
        assert!(game.tops[0].settled, "settledのまま残る");
        assert_ne!(game.tops[1].top.pos, other_pos, "もう片方は転がる");
    }

    #[test]
    fn a_settled_top_stays_settled_through_the_other_tops_events() {
        let mut game = calm_round2();
        game.on_step_events(vec![(0, StepEvent::Goal)]);
        game.on_step_events(vec![(1, StepEvent::Landed(Landing::Bounce))]);
        game.on_step_events(vec![(1, StepEvent::Sank)]);
        assert!(game.tops[0].settled);
        assert!(!game.tops[1].settled);
        assert_eq!(game.outcome(), None);
    }

    #[test]
    fn a_time_up_ends_round2_even_if_one_top_is_settled() {
        let mut game = calm_round2();
        game.on_step_events(vec![(0, StepEvent::Goal)]);
        game.update(TIME_LIMIT + STEP);
        assert_eq!(game.outcome(), Some(Outcome::TimeUp));
    }

    #[test]
    fn reactions_are_ordered_by_severity() {
        assert!(Reaction::Safe.severity() < Reaction::Hop.severity());
        assert!(Reaction::Hop.severity() < Reaction::Sank.severity());
        assert_eq!(Reaction::Sank.severity(), Reaction::Escaped.severity());
        assert!(Reaction::Escaped.severity() < Reaction::Bounce.severity());
        assert_eq!(Reaction::Safe.text(), "セーフ");
        assert_eq!(Reaction::Hop.text(), "ガタッ!");
        assert_eq!(Reaction::Sank.text(), "ズボッ");
        assert_eq!(Reaction::Escaped.text(), "ぬけた!");
        assert_eq!(Reaction::Bounce.text(), "ぴよーん!! ああっ!!");
    }

    #[test]
    fn each_reaction_plays_its_own_se_except_safe_and_escaped() {
        assert_eq!(Reaction::Safe.se(), None);
        assert_eq!(Reaction::Hop.se(), Some(SeKind::BeigomaBump), "凸に触れた時の音");
        assert_eq!(Reaction::Sank.se(), Some(SeKind::BeigomaSink), "凹にはまった時の音");
        assert_eq!(Reaction::Escaped.se(), None);
        assert_eq!(Reaction::Bounce.se(), Some(SeKind::Incorrect), "弾かれた時の音");
    }

    #[test]
    fn the_strongest_reaction_of_the_step_is_picked() {
        let light = StepEvent::Landed(Landing::Light);
        let bounce = StepEvent::Grazed(Landing::Bounce);
        assert_eq!(strongest_reaction(&[light, bounce]), Some(Reaction::Bounce));
        assert_eq!(strongest_reaction(&[bounce, light]), Some(Reaction::Bounce));
        assert_eq!(
            strongest_reaction(&[StepEvent::Hopped { contact_g: 0.0 }, light]),
            Some(Reaction::Hop)
        );
        assert_eq!(
            strongest_reaction(&[StepEvent::Escaped, StepEvent::Sank]),
            Some(Reaction::Escaped),
            "同じ重大度なら先に来た方"
        );
        assert_eq!(strongest_reaction(&[StepEvent::Goal]), None);
        assert_eq!(strongest_reaction(&[]), None);
    }

    #[test]
    fn a_bounce_and_a_light_landing_in_the_same_step_show_the_bounce() {
        for events in [
            vec![
                (0, StepEvent::Landed(Landing::Light)),
                (1, StepEvent::Landed(Landing::Bounce)),
            ],
            vec![
                (0, StepEvent::Grazed(Landing::Bounce)),
                (1, StepEvent::Landed(Landing::Light)),
            ],
        ] {
            let mut game = calm_round2();
            game.on_step_events(events.clone());
            assert_eq!(game.outcome(), None);
            assert_eq!(
                message_text(&game),
                Some("ぴよーん!! ああっ!!"),
                "{events:?}"
            );
        }
    }

    #[test]
    fn a_goal_of_one_top_still_shows_the_other_tops_message() {
        let mut game = calm_round2();
        game.on_step_events(vec![(0, StepEvent::Goal), (1, StepEvent::Sank)]);
        assert!(game.tops[0].settled);
        assert_eq!(message_text(&game), Some("ズボッ"));
    }

    #[test]
    fn the_star_animation_is_shown_on_every_unsettled_top_after_it_flies_off() {
        let mut game = calm_round2();
        game.on_step_events(vec![(1, StepEvent::FellOff)]);
        let views = game.top_views();
        assert_eq!(views.len(), 2);
        assert!(
            views
                .iter()
                .all(|view| view.flying && view.star_frame.is_none()),
            "盤の上にいる間は星にせず、吹っ飛んでいく: {views:?}"
        );
        game.update(FLY_AWAY_EXIT_LIMIT);
        let views = game.top_views();
        assert!(
            views.iter().all(|view| view.star_frame.is_some()),
            "原因でない方も盤の外へ飛んで星になる: {views:?}"
        );
        // ゴールで待機していた方は通常の見た目のまま、その場に残る
        let mut game = calm_round2();
        game.on_step_events(vec![(0, StepEvent::Goal)]);
        let settled_pos = game.tops[0].top.pos;
        game.on_step_events(vec![(1, StepEvent::FellOff)]);
        game.update(FLY_AWAY_EXIT_LIMIT);
        let views = game.top_views();
        assert_eq!(views[0].star_frame, None);
        assert!(!views[0].flying);
        assert_eq!(views[0].pos, settled_pos);
        assert!(views[1].star_frame.is_some());
    }

    #[test]
    fn top_views_follow_each_slot() {
        let mut game = calm_round2();
        game.tops[1].top.state = TopState::Airborne {
            remaining: Duration::from_millis(100),
            contact_g: 0.0,
        };
        let views = game.top_views();
        assert_eq!(views[0].pos, game.tops[0].top.pos);
        assert_eq!(views[1].pos, game.tops[1].top.pos);
        assert!(!views[0].airborne);
        assert!(views[1].airborne);
        assert!(views
            .iter()
            .all(|view| view.spin_frame == game.spin_frame()));
    }

    /// 描画した盤面の中のベーゴマ記号の数
    fn count_top_glyphs(game: &BeigomaGame) -> usize {
        let glyph = render::TOP_SPIN_GLYPHS[game.spin_frame()];
        rendered_text(game, AREA).matches(glyph).count()
    }

    #[test]
    fn round2_draws_two_tops_and_round1_draws_one() {
        assert_eq!(count_top_glyphs(&calm_game()), 1, "ROUND1は1個");
        assert_eq!(
            count_top_glyphs(&calm_round2()),
            2,
            "ROUND2は同じマスにいても2個とも描く"
        );
    }

    #[test]
    fn round2_renders_without_panicking_in_tiny_areas() {
        let mut game = calm_round2();
        for _ in 0..3 {
            for (w, h) in [(1, 1), (5, 2), (10, 4), (30, 8), (60, 12)] {
                rendered_text(&game, Rect::new(0, 0, w, h));
            }
            game.update(Duration::from_secs(20));
        }
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
            game.finish(outcome, false);
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

    /// i番目のベーゴマの星の演出のコマ
    fn star_frame(game: &BeigomaGame, i: usize) -> Option<usize> {
        game.top_views()[i].star_frame
    }

    /// 1個目のベーゴマを盤の左の縁のすぐ外に、左へ転がり出ていく状態で置く(場外に出た瞬間)
    fn place_just_off_the_left_rim(game: &mut BeigomaGame) {
        game.tops[0].top = Top::new(&game.board, (-0.01, 5.5));
        game.tops[0].top.vel = (-2.0, 0.0);
    }

    #[test]
    fn falling_off_starts_the_star_animation_at_once() {
        // 既に盤の外にいる(場外に落ちた)ベーゴマは、GAME OVER直後から星の1コマ目
        let mut game = calm_game();
        place_just_off_the_left_rim(&mut game);
        game.on_step_event(StepEvent::FellOff);
        assert_eq!(star_frame(&game, 0), Some(0), "GAME OVER直後は星の1コマ目");
    }

    #[test]
    fn star_animation_advances_and_stays_on_the_last_frame() {
        let mut game = calm_game();
        place_just_off_the_left_rim(&mut game);
        game.on_step_event(StepEvent::FellOff);
        game.update(render::STAR_ANIM_FRAME);
        assert_eq!(star_frame(&game, 0), Some(1), "時間が経つとコマが進む");
        game.update(render::STAR_ANIM_FRAME * render::STAR_ANIM_GLYPHS.len() as u32 * 2);
        assert_eq!(
            star_frame(&game, 0),
            Some(render::STAR_ANIM_GLYPHS.len() - 1),
            "最後のコマで止まる(範囲外にならない)"
        );
    }

    #[test]
    fn timeup_and_cleared_do_not_show_the_star_animation() {
        let mut timeup = calm_game();
        timeup.finish(Outcome::TimeUp, false);
        timeup.update(Duration::from_millis(500));
        assert_eq!(star_frame(&timeup, 0), None, "時間切れは星の演出を出さない");
        assert!(!timeup.top_views()[0].flying, "時間切れは吹っ飛ばない");

        let mut cleared = calm_game();
        place_just_before_goal(&mut cleared);
        cleared.update(STEP);
        cleared.update(Duration::from_millis(500));
        assert_eq!(
            star_frame(&cleared, 0),
            None,
            "クリア時は星の演出を出さない"
        );
        assert!(!cleared.top_views()[0].flying, "クリア時は吹っ飛ばない");
    }

    // --- 吹っ飛び・場外のGAME OVERで盤の外へ飛んでいく演出 ---

    /// GAME OVER(吹っ飛び・場外)になる出来事
    const GAME_OVER_EVENTS: [StepEvent; 3] = [
        StepEvent::FellOff,
        StepEvent::Landed(Landing::Flown),
        StepEvent::Grazed(Landing::Flown),
    ];

    /// 盤の中心からの距離
    fn distance_from_center(pos: (f64, f64)) -> f64 {
        (pos.0 - BOARD_WIDTH as f64 / 2.0).hypot(pos.1 - BOARD_HEIGHT as f64 / 2.0)
    }

    #[test]
    fn a_game_over_on_the_board_blasts_the_top_off_the_board() {
        // 盤の上でGAME OVERになっても、その場で小さくなって消えずに盤の外まで吹っ飛ぶ
        for event in GAME_OVER_EVENTS {
            let mut game = calm_game();
            let start = game.tops[0].top.pos;
            assert!(Board::contains(start));
            game.on_step_event(event);
            assert_eq!(game.outcome(), Some(Outcome::Flown), "{event:?}");
            let views = game.top_views();
            assert!(views[0].flying, "{event:?}: 吹っ飛んでいく");
            assert_eq!(
                views[0].star_frame, None,
                "{event:?}: 盤の上ではまだ星にしない"
            );
            game.update(FLY_AWAY_EXIT_LIMIT);
            assert!(
                !Board::contains(game.tops[0].top.pos),
                "{event:?}: 盤の外へ出る: {:?}",
                game.tops[0].top.pos
            );
            assert!(!game.is_finished(), "{event:?}: 演出の途中");
        }
    }

    #[test]
    fn a_hard_hit_during_play_really_moves_the_top_off_the_board() {
        // 実際の物理で吹っ飛んだ時(凸を大きなGで踏んだ): 位置が動かないまま終わらない
        let truck = Truck::with_course(
            vec![RoadEvent::Signal {
                at: 35.0,
                notice_delay: 5.0,
            }],
            1000.0,
        );
        let mut game = BeigomaGame::with_truck(truck);
        finish_countdown(&mut game);
        while game.truck.current_g().magnitude() <= HIGH_G_THRESHOLD {
            game.update(STEP);
        }
        let (x, y) = flat_below_a_bump(&game.board);
        game.tops[0].top = Top::new(&game.board, (x as f64 + 0.5, y as f64 + TOP_RADIUS + 0.1));
        let mut last = game.tops[0].top.pos;
        for _ in 0..100 {
            last = game.tops[0].top.pos;
            game.update(STEP);
            if game.outcome().is_some() {
                break;
            }
        }
        assert_eq!(game.outcome(), Some(Outcome::Flown));
        let at_game_over = game.tops[0].top.pos;
        assert!(
            (at_game_over.0 - last.0).hypot(at_game_over.1 - last.1) > FLOWN_KICK / 2.0,
            "吹っ飛んだ瞬間に弾き飛ばされる: {last:?} → {at_game_over:?}"
        );
        game.update(FLY_AWAY_EXIT_LIMIT);
        assert!(!Board::contains(game.tops[0].top.pos), "盤の外まで飛ぶ");
        assert!(star_frame(&game, 0).is_some(), "盤の外へ出たら星になる");
    }

    #[test]
    fn the_star_starts_when_the_flying_top_leaves_the_board() {
        let mut game = calm_game();
        game.on_step_event(StepEvent::Landed(Landing::Flown));
        let mut t = Duration::ZERO;
        while Board::contains(game.tops[0].top.pos) {
            assert_eq!(star_frame(&game, 0), None, "盤の上にいる間は星にしない");
            game.update(STEP);
            t += STEP;
            assert!(t <= FLY_AWAY_EXIT_LIMIT);
        }
        assert_eq!(
            star_frame(&game, 0),
            Some(0),
            "盤の外へ出た瞬間に星の1コマ目"
        );
        assert!(
            !game.top_views()[0].flying,
            "星になったら飛んでいる見た目は終わり"
        );
    }

    #[test]
    fn flying_tops_keep_getting_farther_until_the_end_display_finishes() {
        for event in GAME_OVER_EVENTS {
            let mut game = calm_round2();
            game.on_step_events(vec![(0, event)]);
            let mut distances: Vec<f64> = game
                .tops
                .iter()
                .map(|s| distance_from_center(s.top.pos))
                .collect();
            let mut shown = Duration::ZERO;
            while shown + STEP < END_HOLD {
                game.update(STEP);
                shown += STEP;
                for (i, slot) in game.tops.iter().enumerate() {
                    let now = distance_from_center(slot.top.pos);
                    assert!(now > distances[i], "{event:?} slot{i}: 遠ざかり続ける");
                    distances[i] = now;
                }
            }
            for (i, d) in distances.iter().enumerate() {
                assert!(*d >= 40.0, "{event:?} slot{i}: 画面の外まで飛ぶ: {d}");
            }
        }
    }

    #[test]
    fn a_flying_top_spins_faster_than_a_rolling_top() {
        const { assert!(FLY_SPIN_FRAME_INTERVAL.as_nanos() * 2 <= SPIN_FRAME_INTERVAL.as_nanos()) };
        let mut game = calm_game();
        game.on_step_event(StepEvent::Landed(Landing::Flown));
        let mut frames = vec![game.top_views()[0].spin_frame];
        for _ in 1..render::TOP_SPIN_GLYPHS.len() {
            game.update(FLY_SPIN_FRAME_INTERVAL);
            frames.push(game.top_views()[0].spin_frame);
        }
        let expected: Vec<usize> = (0..render::TOP_SPIN_GLYPHS.len()).collect();
        assert_eq!(frames, expected, "1間隔ごとに1コマずつ回る");
    }

    #[test]
    fn a_flying_top_zigzags_across_its_path() {
        // 描く位置(TopView.pos)は、実際の軌道(Top.pos)から進む向きの左右へ振れる
        let mut game = calm_game();
        game.tops[0].top.vel = (0.0, -1.0);
        game.on_step_event(StepEvent::Landed(Landing::Flown));
        let (mut left, mut right) = (false, false);
        for _ in 0..30 {
            game.update(STEP);
            let (view, top) = (game.top_views()[0], &game.tops[0].top);
            let dir = {
                let speed = top.vel.0.hypot(top.vel.1);
                (top.vel.0 / speed, top.vel.1 / speed)
            };
            let offset = (view.pos.0 - top.pos.0, view.pos.1 - top.pos.1);
            let along = offset.0 * dir.0 + offset.1 * dir.1;
            let across = offset.0 * -dir.1 + offset.1 * dir.0;
            assert!(along.abs() < 1e-9, "進む向きにはずらさない: {offset:?}");
            assert!(across.abs() <= FLY_WOBBLE + 1e-9, "振れ幅の範囲: {across}");
            left |= across < -FLY_WOBBLE / 2.0;
            right |= across > FLY_WOBBLE / 2.0;
        }
        assert!(left && right, "左右両方へ大きく振れる");
    }

    #[test]
    fn the_star_fades_out_before_the_end_display_finishes_even_from_the_far_corner() {
        // 最も盤の外へ出るのが遅い場合(盤の角から対角の角へ向かって吹っ飛ぶ)でも、
        // 結果の表示が終わるまでに星は最後のコマまで消えていく
        let mut game = calm_game();
        // 遅い速度で向きだけ与え、吹っ飛ぶ速さを最小(FLOWN_SPEED)にする
        game.tops[0].top = Top::new(&game.board, (0.01, 0.01));
        game.tops[0].top.vel = (BOARD_WIDTH as f64 / 100.0, BOARD_HEIGHT as f64 / 100.0);
        game.on_step_event(StepEvent::Grazed(Landing::Flown));
        game.update(END_HOLD - STEP);
        assert!(!game.is_finished());
        assert_eq!(
            star_frame(&game, 0),
            Some(render::STAR_ANIM_GLYPHS.len() - 1),
            "最後のコマまで進んでいる"
        );
    }

    // --- 場外・吹っ飛びのSE ---

    /// 鳴らしたSEの記録を空にする
    fn clear_se_log(game: &mut BeigomaGame) {
        game.se_log.clear();
    }

    fn count_se(game: &BeigomaGame, se: SeKind) -> usize {
        game.se_log.iter().filter(|&&s| s == se).count()
    }

    /// GAME OVERの原因ごとに、まず鳴らす専用の音(場外に落ちた時だけ専用音、それ以外は「ふいっ」)
    fn first_game_over_se(event: StepEvent) -> SeKind {
        if event == StepEvent::FellOff {
            SeKind::BeigomaFalloff
        } else {
            SeKind::Star
        }
    }

    #[test]
    fn game_over_plays_its_first_sound_then_the_buzzer() {
        const {
            assert!(OFF_BOARD_BUZZ_DELAY.as_millis() >= 100);
            assert!(OFF_BOARD_BUZZ_DELAY.as_millis() <= 300);
        };
        for event in GAME_OVER_EVENTS {
            let first = first_game_over_se(event);
            let mut game = calm_game();
            clear_se_log(&mut game);
            game.on_step_event(event);
            assert_eq!(game.se_log, vec![first], "{event:?}: まず専用の落下音/「ふいっ」");
            game.update(OFF_BOARD_BUZZ_DELAY - Duration::from_millis(1));
            assert_eq!(game.se_log, vec![first], "{event:?}: ブブーはまだ");
            game.update(Duration::from_millis(1));
            assert_eq!(
                game.se_log,
                vec![first, SeKind::Incorrect],
                "{event:?}: 少し遅れてブブー"
            );
            game.update(END_HOLD);
            assert_eq!(
                count_se(&game, SeKind::Incorrect),
                1,
                "{event:?}: ブブーは1回だけ"
            );
            assert_eq!(count_se(&game, first), 1, "{event:?}");
        }
    }

    #[test]
    fn falling_off_the_board_plays_the_falloff_sound_not_the_star() {
        let mut game = calm_game();
        clear_se_log(&mut game);
        game.on_step_event(StepEvent::FellOff);
        assert_eq!(
            game.se_log,
            vec![SeKind::BeigomaFalloff],
            "場外に落ちた時は専用の落下音"
        );
        assert_eq!(count_se(&game, SeKind::Star), 0, "「ふいっ」は鳴らさない");
    }

    #[test]
    fn the_buzzer_rings_once_even_if_the_end_display_passes_in_one_update() {
        let mut game = calm_game();
        clear_se_log(&mut game);
        game.on_step_event(StepEvent::FellOff);
        game.update(END_HOLD * 2);
        assert_eq!(game.se_log, vec![SeKind::BeigomaFalloff, SeKind::Incorrect]);
    }

    #[test]
    fn one_top_failing_in_round2_plays_the_buzzer_once() {
        let mut game = calm_round2();
        clear_se_log(&mut game);
        game.on_step_events(vec![
            (0, StepEvent::FellOff),
            (1, StepEvent::Landed(Landing::Flown)),
        ]);
        game.update(END_HOLD);
        assert_eq!(game.se_log, vec![SeKind::BeigomaFalloff, SeKind::Incorrect]);
    }

    #[test]
    fn time_up_and_clear_do_not_add_the_delayed_buzzer() {
        let mut timeup = calm_game();
        clear_se_log(&mut timeup);
        timeup.update(TIME_LIMIT + STEP);
        timeup.update(END_HOLD);
        assert_eq!(
            timeup.se_log,
            vec![SeKind::Incorrect],
            "時間切れはブザー1回のまま"
        );

        let mut cleared = calm_game();
        clear_se_log(&mut cleared);
        clear_round(&mut cleared);
        cleared.update(OFF_BOARD_BUZZ_DELAY * 2);
        assert_eq!(cleared.se_log, vec![SeKind::Correct]);
    }

    #[test]
    fn a_bounce_still_plays_the_buzzer_immediately() {
        let mut game = calm_game();
        clear_se_log(&mut game);
        game.on_step_event(StepEvent::Landed(Landing::Bounce));
        assert_eq!(game.se_log, vec![SeKind::Incorrect]);
        assert_eq!(game.outcome(), None);
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
