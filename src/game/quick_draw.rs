//! 反射神経(quick_draw): いつ来るかわからない合図を待ち、合図が出た瞬間に反応する。
//! 仕様は docs/quick-draw-spec.md

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::Rng;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "quick_draw";

/// 1セッションのラウンド数
pub const ROUNDS_PER_SESSION: u32 = 3;

/// 待機中の背景(暗いグレー)
pub const WAITING_BG: Color = Color::Rgb(48, 48, 48);
/// 合図の背景(明るい緑)
pub const SIGNAL_BG: Color = Color::Rgb(0, 230, 64);
/// 待機中の表示
pub const WAITING_TEXT: &str = "まだ待て";
/// 合図の表示
pub const SIGNAL_TEXT: &str = "今だ!";

/// 難易度ごとの待機時間の範囲(最小, 最大)
pub fn wait_range(difficulty: Difficulty) -> (Duration, Duration) {
    let (min_ms, max_ms) = match difficulty {
        Difficulty::Beginner => (1000, 2500),
        Difficulty::Intermediate => (1000, 4000),
        Difficulty::Advanced => (800, 5000),
    };
    (Duration::from_millis(min_ms), Duration::from_millis(max_ms))
}

/// フライング時に反応時間の代わりに記録する固定ペナルティ値(ms)。
/// その難易度の最大待機時間と同じ値にし、どんなに遅い反応よりも悪い記録になるようにする
pub fn fail_latency_ms(difficulty: Difficulty) -> f64 {
    let (_, max) = wait_range(difficulty);
    max.as_millis() as f64
}

/// 待機時間を難易度の範囲内からランダムに決める
fn random_wait(rng: &mut impl Rng, difficulty: Difficulty) -> Duration {
    let (min, max) = wait_range(difficulty);
    Duration::from_millis(rng.gen_range(min.as_millis() as u64..=max.as_millis() as u64))
}

fn new_waiting(difficulty: Difficulty) -> Phase {
    Phase::Waiting {
        remaining: random_wait(&mut rand::thread_rng(), difficulty),
    }
}

/// ラウンド内の状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// 合図待ち。残りの待機時間
    Waiting { remaining: Duration },
    /// 合図が出ている。合図が出た時刻
    Signal { shown_at: Instant },
}

pub struct QuickDrawGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    phase: Phase,
    /// 直前のラウンドの結果表示(描画専用)
    feedback: AnswerFeedback,
}

impl QuickDrawGame {
    pub fn new(difficulty: Difficulty) -> Self {
        Self {
            difficulty,
            tracker: ScoreTracker::with_session_length(ROUNDS_PER_SESSION),
            phase: new_waiting(difficulty),
            feedback: AnswerFeedback::new(),
        }
    }

    /// キー/クリックで押された時の処理。合図前ならフライング、合図後なら反応時間を記録する
    fn press(&mut self) {
        if self.is_finished() {
            return;
        }
        match self.phase {
            Phase::Waiting { .. } => {
                self.tracker.record(false, fail_latency_ms(self.difficulty));
                self.feedback.record(false, "フライング");
                audio::play_se(SeKind::Incorrect);
            }
            Phase::Signal { shown_at } => {
                let latency_ms = shown_at.elapsed().as_millis() as f64;
                self.tracker.record(true, latency_ms);
                self.feedback.record(true, format!("{latency_ms:.0}ms"));
                audio::play_se(SeKind::Correct);
            }
        }
        // 最終ラウンドの後も待機状態にしておく(is_finishedで入力と時間経過は止まる)
        self.phase = new_waiting(self.difficulty);
    }

    fn render_board(&self, frame: &mut Frame, area: Rect) {
        let (background, headline, text_color) = match self.phase {
            Phase::Waiting { .. } => (WAITING_BG, WAITING_TEXT, theme::TEXT),
            Phase::Signal { .. } => (SIGNAL_BG, SIGNAL_TEXT, Color::Black),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(background))
            .style(Style::default().bg(background));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines = vec![
            Line::from(Span::styled(
                headline,
                Style::default().fg(text_color).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "合図が出たら Enter / Space / クリック",
                Style::default().fg(text_color),
            )),
        ];
        let text_area = theme::vertical_center(inner, lines.len() as u16);
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            text_area,
        );
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
        if let Phase::Waiting { remaining } = self.phase {
            let remaining = remaining.saturating_sub(dt);
            self.phase = if remaining.is_zero() {
                // 反応時間は合図が画面に出た時刻から測る
                Phase::Signal {
                    shown_at: Instant::now(),
                }
            } else {
                Phase::Waiting { remaining }
            };
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud_area, board_area) = theme::split_hud(area);
        theme::render_hud_with_session_length(
            frame,
            hud_area,
            "反射神経",
            self.difficulty,
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
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    const ALL_DIFFICULTIES: [Difficulty; 3] = [
        Difficulty::Beginner,
        Difficulty::Intermediate,
        Difficulty::Advanced,
    ];
    const AREA: Rect = Rect::new(0, 0, 60, 20);

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

    fn assert_waiting_within_range(game: &QuickDrawGame) {
        let (min, max) = wait_range(game.difficulty);
        match game.phase {
            Phase::Waiting { remaining } => assert!(
                (min..=max).contains(&remaining),
                "待機時間{remaining:?}が範囲{min:?}〜{max:?}に入っていない"
            ),
            Phase::Signal { .. } => panic!("待機状態のはず"),
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

    // --- 難易度別パラメータ ---

    #[test]
    fn wait_range_matches_spec_for_each_difficulty() {
        assert_eq!(wait_range(Difficulty::Beginner), (ms(1000), ms(2500)));
        assert_eq!(wait_range(Difficulty::Intermediate), (ms(1000), ms(4000)));
        assert_eq!(wait_range(Difficulty::Advanced), (ms(800), ms(5000)));
    }

    #[test]
    fn random_wait_stays_within_range_for_each_difficulty() {
        let mut rng = StdRng::seed_from_u64(7);
        for difficulty in ALL_DIFFICULTIES {
            let (min, max) = wait_range(difficulty);
            let waits: Vec<Duration> = (0..500)
                .map(|_| random_wait(&mut rng, difficulty))
                .collect();
            assert!(
                waits.iter().all(|w| (min..=max).contains(w)),
                "{difficulty:?}: 範囲外の待機時間がある"
            );
            // 毎回同じ値ではなくランダムに散らばる
            let shortest = waits.iter().min().unwrap();
            let longest = waits.iter().max().unwrap();
            assert!(
                *longest - *shortest > ms(500),
                "{difficulty:?}: 待機時間がばらついていない"
            );
        }
    }

    #[test]
    fn fail_latency_is_the_longest_wait_of_each_difficulty() {
        // フライングのペナルティは、その難易度の最大待機時間と同じ値にする
        for difficulty in ALL_DIFFICULTIES {
            let (_, max) = wait_range(difficulty);
            assert_eq!(fail_latency_ms(difficulty), max.as_millis() as f64);
        }
        assert_eq!(fail_latency_ms(Difficulty::Beginner), 2500.0);
        assert_eq!(fail_latency_ms(Difficulty::Intermediate), 4000.0);
        assert_eq!(fail_latency_ms(Difficulty::Advanced), 5000.0);
    }

    // --- 開始時と合図への切り替え ---

    #[test]
    fn new_game_starts_waiting_within_difficulty_range() {
        for difficulty in ALL_DIFFICULTIES {
            let game = QuickDrawGame::new(difficulty);
            assert_waiting_within_range(&game);
            assert!(!game.is_finished());
            assert_eq!(game.tracker.total(), 0);
        }
    }

    #[test]
    fn update_switches_to_signal_after_wait_elapses() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        game.phase = Phase::Waiting { remaining: ms(100) };
        game.update(ms(60));
        assert_eq!(game.phase, Phase::Waiting { remaining: ms(40) });
        game.update(ms(60));
        assert!(
            matches!(game.phase, Phase::Signal { .. }),
            "待機時間を過ぎたら合図"
        );
    }

    #[test]
    fn signal_has_no_timeout() {
        // 合図後は押すまで待つ(時間切れで勝手に次のラウンドへ進まない)
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        show_signal_since(&mut game, ms(0));
        game.update(Duration::from_secs(60));
        assert!(matches!(game.phase, Phase::Signal { .. }));
        assert_eq!(game.tracker.total(), 0);
    }

    // --- 反応(合図後) ---

    #[test]
    fn enter_after_signal_records_reaction_time() {
        let mut game = QuickDrawGame::new(Difficulty::Intermediate);
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
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        show_signal_since(&mut game, ms(200));
        game.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(game.result().correct, 1);
    }

    #[test]
    fn reacting_starts_next_round_waiting() {
        let mut game = QuickDrawGame::new(Difficulty::Advanced);
        show_signal_since(&mut game, ms(250));
        game.handle_key(key(KeyCode::Enter));
        assert_waiting_within_range(&game);
    }

    #[test]
    fn reacting_shows_reaction_time_in_feedback() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
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
    fn key_before_signal_is_false_start_with_fixed_penalty() {
        for difficulty in ALL_DIFFICULTIES {
            let mut game = QuickDrawGame::new(difficulty);
            game.handle_key(key(KeyCode::Enter));
            let result = game.result();
            assert_eq!(result.total, 1, "{difficulty:?}");
            assert_eq!(result.correct, 0, "{difficulty:?}: フライングは失敗");
            assert_eq!(result.avg_latency_ms, fail_latency_ms(difficulty));
        }
    }

    #[test]
    fn click_before_signal_is_false_start() {
        let mut game = QuickDrawGame::new(Difficulty::Intermediate);
        game.handle_mouse(left_click(10, 10), AREA);
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 0);
        assert_eq!(
            result.avg_latency_ms,
            fail_latency_ms(Difficulty::Intermediate)
        );
    }

    #[test]
    fn false_start_starts_next_round_and_shows_feedback() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        game.handle_key(key(KeyCode::Char(' ')));
        assert_waiting_within_range(&game);
        let flash = game
            .feedback
            .current()
            .expect("フライング直後は結果を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert!(flash.detail.contains("フライング"), "{}", flash.detail);
    }

    // --- 入力の種類 ---

    #[test]
    fn other_keys_are_ignored() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        for code in [
            KeyCode::Left,
            KeyCode::Char('a'),
            KeyCode::Esc,
            KeyCode::Tab,
        ] {
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
            let mut game = QuickDrawGame::new(Difficulty::Beginner);
            show_signal_since(&mut game, ms(200));
            game.handle_mouse(left_click(column, row), AREA);
            assert_eq!(game.result().correct, 1, "({column},{row})のクリック");
        }
    }

    #[test]
    fn non_left_press_mouse_events_are_ignored() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
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
    fn session_finishes_after_three_rounds() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        for round in 0..ROUNDS_PER_SESSION {
            assert!(!game.is_finished(), "{round}ラウンド目の前は終わっていない");
            show_signal_since(&mut game, ms(200));
            game.handle_key(key(KeyCode::Enter));
        }
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.difficulty, Difficulty::Beginner);
        assert_eq!(result.total, 3);
        assert_eq!(result.correct, 3);
    }

    #[test]
    fn mixed_session_averages_reaction_and_penalty() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        game.handle_key(key(KeyCode::Enter)); // フライング
        show_signal_since(&mut game, ms(300));
        game.handle_key(key(KeyCode::Enter));
        show_signal_since(&mut game, ms(300));
        game.handle_mouse(left_click(1, 1), AREA);
        let result = game.result();
        assert_eq!(result.total, 3);
        assert_eq!(result.correct, 2);
        // (2500 + 300 + 300) / 3 ≒ 1033
        assert!(
            (1033.0..1100.0).contains(&result.avg_latency_ms),
            "{}",
            result.avg_latency_ms
        );
    }

    #[test]
    fn input_after_session_finished_is_ignored() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        for _ in 0..ROUNDS_PER_SESSION {
            game.handle_key(key(KeyCode::Enter));
        }
        assert!(game.is_finished());
        game.handle_key(key(KeyCode::Enter));
        game.handle_mouse(left_click(1, 1), AREA);
        game.update(Duration::from_secs(10));
        assert_eq!(game.result().total, ROUNDS_PER_SESSION);
    }

    // --- 描画 ---

    #[test]
    fn waiting_renders_gray_background_and_wait_text() {
        let game = QuickDrawGame::new(Difficulty::Beginner);
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(text.contains(WAITING_TEXT), "待機中は「まだ待て」");
        assert!(!text.contains(SIGNAL_TEXT));
        assert_eq!(body_center_bg(&buffer), WAITING_BG);
    }

    #[test]
    fn signal_renders_green_background_and_now_text() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        show_signal_since(&mut game, ms(0));
        let buffer = rendered(&game);
        let text = text_of(&buffer);
        assert!(text.contains(SIGNAL_TEXT), "合図後は「今だ!」");
        assert!(!text.contains(WAITING_TEXT));
        assert_eq!(body_center_bg(&buffer), SIGNAL_BG);
    }

    #[test]
    fn render_switches_when_wait_elapses() {
        let mut game = QuickDrawGame::new(Difficulty::Beginner);
        game.phase = Phase::Waiting { remaining: ms(50) };
        assert_eq!(body_center_bg(&rendered(&game)), WAITING_BG);
        game.update(ms(50));
        let buffer = rendered(&game);
        assert_eq!(body_center_bg(&buffer), SIGNAL_BG);
        assert!(text_of(&buffer).contains(SIGNAL_TEXT));
    }

    #[test]
    fn render_shows_hud_with_game_name_and_round_count() {
        let game = QuickDrawGame::new(Difficulty::Beginner);
        let text = text_of(&rendered(&game));
        assert!(text.contains("反射神経"));
        assert!(text.contains("Q1/3"), "3ラウンド制の進捗: {text}");
    }

    #[test]
    fn render_does_not_panic_in_tiny_area() {
        let game = QuickDrawGame::new(Difficulty::Beginner);
        for (width, height) in [(1, 1), (5, 2), (10, 4)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| game.render(frame, Rect::new(0, 0, width, height)))
                .unwrap();
        }
    }
}
