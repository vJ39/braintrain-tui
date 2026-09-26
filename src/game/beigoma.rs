//! べー(beigoma): 軽トラの荷台で女の子が持つベーゴマ盤を傾け、ベーゴマを唯一のゴールへ導く。
//! 仕様は docs/beigoma-game-spec.md
//!
//! - 盤の傾きは前後・左右の2軸で、矢印キーを押すたびに一定量動く連打式(board.rs)
//! - 軽トラは自動で走り、段差・信号・障害物回避のたびに運動方程式から求めたGが盤にかかる(truck.rs)
//! - 障害物を踏んだ瞬間のGが大きいと弾かれ、さらに大きいと吹っ飛んで即GAME OVER
//! - 盤の縁に壁は無く、盤から落ちても(場外)即GAME OVER
//! - 制限時間60秒。ゴールで成功、吹っ飛び・場外・時間切れで失敗。難易度選択は無い
//! - 開始前の「3.2.1.GO!!」はapp.rsのカウントダウンで行い、終わってからゲームを作る(=ベーゴマを投入する)

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

use board::{Board, Landing, StepEvent, Tilt, TiltKey, Top};
use render::{BoardRenderer, TopView, TruckViewInfo, TruckViewRenderer};
use truck::Truck;

pub const GAME_ID: &str = "beigoma";

/// 結果に記録する難易度。べーは難易度を選ばないので固定値にする
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Intermediate;

/// 制限時間
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
const SPIN_FRAME_INTERVAL: Duration = Duration::from_millis(120);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Playing,
    /// 終わった。shownは終了表示を出してからの時間
    Ended {
        outcome: Outcome,
        shown: Duration,
    },
}

pub struct BeigomaGame {
    board: Board,
    top: Top,
    tilt: Tilt,
    truck: Truck,
    /// ベーゴマを投入してからの経過時間
    elapsed: Duration,
    status: Status,
    tracker: ScoreTracker,
    /// 一言の表示(文言, 表示してからの時間)
    message: Option<(&'static str, Duration)>,
    board_renderer: BoardRenderer,
    truck_view: TruckViewRenderer,
}

impl BeigomaGame {
    /// ベーゴマを盤の投入位置に置き、軽トラが決まったコースを走り始める
    pub fn new() -> Self {
        Self::with_truck(Truck::new())
    }

    /// 軽トラを指定して作る(テストで特定のGの場面を作るため)
    fn with_truck(truck: Truck) -> Self {
        let board = Board::standard();
        let top = Top::new(board.start_position());
        Self {
            board,
            top,
            tilt: Tilt::new(),
            truck,
            elapsed: Duration::ZERO,
            status: Status::Playing,
            tracker: ScoreTracker::with_session_length(1),
            message: None,
            board_renderer: BoardRenderer::new(),
            truck_view: TruckViewRenderer::new(),
        }
    }

    /// 終わり方(まだ終わっていなければNone)
    pub fn outcome(&self) -> Option<Outcome> {
        match self.status {
            Status::Playing => None,
            Status::Ended { outcome, .. } => Some(outcome),
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
        if self.status == Status::Playing && self.elapsed >= TIME_LIMIT {
            self.finish(Outcome::TimeUp);
        }
    }

    /// 盤上の出来事を反映する(弾かれたらSE、吹っ飛んだ・盤から落ちたら即GAME OVER、ゴールなら成功)
    fn on_step_event(&mut self, event: StepEvent) {
        if self.status != Status::Playing {
            return;
        }
        match event {
            StepEvent::Hopped { .. } => self.message = Some(("ガタッ!", Duration::ZERO)),
            StepEvent::Landed(Landing::Light) => self.message = Some(("セーフ", Duration::ZERO)),
            StepEvent::Landed(Landing::Bounce) => {
                audio::play_se(SeKind::Incorrect);
                self.message = Some(("ピューン!", Duration::ZERO));
            }
            StepEvent::Landed(Landing::Flown) | StepEvent::FellOff => self.finish(Outcome::Flown),
            StepEvent::Goal => self.finish(Outcome::Cleared { time: self.elapsed }),
        }
    }

    /// ゲームを終える。結果は1回だけ記録する(成功はクリアタイム、失敗は制限時間を反応時間として記録)
    fn finish(&mut self, outcome: Outcome) {
        if self.status != Status::Playing {
            return;
        }
        let (success, latency) = match outcome {
            Outcome::Cleared { time } => (true, time),
            Outcome::Flown | Outcome::TimeUp => (false, TIME_LIMIT),
        };
        self.tracker.record(success, latency.as_millis() as f64);
        audio::play_se(if success {
            SeKind::Correct
        } else {
            SeKind::Incorrect
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

    /// 上段: 残り時間・一言(終わったら結果)・傾き
    fn render_hud(&self, frame: &mut Frame, area: Rect) {
        let block = theme::panel(" ◆ べー ").border_style(Style::default().fg(self.status_color()));
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
    /// 矢印キーで盤を傾ける(押すたびに一定量。終わった後は受け付けない)
    fn handle_key(&mut self, key: KeyEvent) {
        if self.status != Status::Playing {
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
        if let Status::Ended { outcome, shown } = self.status {
            self.status = Status::Ended {
                outcome,
                shown: shown.saturating_add(dt),
            };
            return;
        }
        if let Some((text, shown)) = self.message {
            let shown = shown.saturating_add(dt);
            self.message = (shown < MESSAGE_HOLD).then_some((text, shown));
        }
        // 大きなdtは小さなステップに分けて進める(途中で終わったらそこで止める)
        let mut remaining = dt;
        while !remaining.is_zero() && self.status == Status::Playing {
            let step = remaining.min(MAX_STEP);
            self.step(step);
            remaining -= step;
        }
    }

    /// 上段にHUD、下段の左に軽トラ視点・右に盤面視点を並べる
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
        };
        self.board_renderer
            .render(frame, board_inner, &self.board, &top);
    }

    /// 終わってから終了表示(END_HOLD)を出し終えたらセッション終了
    fn is_finished(&self) -> bool {
        matches!(self.status, Status::Ended { shown, .. } if shown >= END_HOLD)
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
    }
}

#[cfg(test)]
mod tests {
    use super::board::{Cell, BOARD_HEIGHT, BOARD_WIDTH, HIGH_G_THRESHOLD, TILT_STEP};
    use super::truck::RoadEvent;
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const AREA: Rect = Rect::new(0, 0, 100, 24);
    const STEP: Duration = Duration::from_millis(10);

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    /// イベントの起きない軽トラ(ずっと巡航・Gなし)で始める
    fn calm_game() -> BeigomaGame {
        BeigomaGame::with_truck(Truck::with_course(Vec::new(), 1000.0))
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

    // --- 開始(カウントダウン後の投入) ---

    #[test]
    fn new_game_puts_the_top_on_the_start_and_is_playing() {
        let game = BeigomaGame::new();
        assert_eq!(
            game.top.pos,
            game.board.start_position(),
            "投入位置に置かれる"
        );
        assert_eq!(game.top.vel, (0.0, 0.0));
        assert!(!game.top.is_airborne());
        assert_eq!(game.elapsed, Duration::ZERO, "制限時間はここから数える");
        assert_eq!(game.outcome(), None);
        assert!(!game.is_finished());
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.difficulty, SESSION_DIFFICULTY);
        assert_eq!(result.total, 0);
    }

    #[test]
    fn the_top_starts_spinning_after_it_is_placed() {
        let mut game = calm_game();
        let first = game.spin_frame();
        game.update(SPIN_FRAME_INTERVAL);
        assert_ne!(game.spin_frame(), first, "投入されると回転を始める");
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
        assert!(game.message.is_some(), "ピューンと弾かれた一言を出す");
        game.on_step_event(StepEvent::Landed(Landing::Light));
        assert_eq!(game.outcome(), None);
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
        // 終わった後は時間切れにも吹っ飛びにもならない
        game.update(TIME_LIMIT);
        game.on_step_event(StepEvent::Landed(Landing::Flown));
        assert!(matches!(game.outcome(), Some(Outcome::Cleared { .. })));
        assert_eq!(game.result().total, 1);
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
