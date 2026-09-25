use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::theme;
use crate::game::{contains, Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "memory";

/// 描画エリアを「2x2パネル」「フッター」に分割する。
/// 上端のHUD(theme::split_hud)を除いた残りを分ける。renderはHUDを同じsplit_hudで切り出す
fn split_areas(area: Rect) -> (Rect, Rect) {
    let (_, body) = theme::split_hud(area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(7), Constraint::Length(3)])
        .split(body);
    (rows[0], rows[1])
}

/// パネル番号(1〜4)ごとの描画エリアを求める(renderとhandle_mouseで共有)
fn panel_areas(grid_area: Rect) -> [(usize, Rect); 4] {
    let grid_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(grid_area);
    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(grid_cols[0]);
    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(grid_cols[1]);
    [
        (1usize, left_rows[0]),
        (2, right_rows[0]),
        (3, left_rows[1]),
        (4, right_rows[1]),
    ]
}

/// パネル番号(1〜4)ごとの色。1=左上, 2=右上, 3=左下, 4=右下
const PANEL_COLORS: [(usize, Color); 4] = [
    (1, Color::Red),
    (2, Color::Blue),
    (3, Color::Green),
    (4, Color::Yellow),
];

/// 提示フェーズで1パネルを光らせている時間
const SHOW_ON_DURATION: Duration = Duration::from_millis(400);
/// 提示フェーズで次のパネルへ移る前に消灯しておく時間
/// (同じパネルが連続するとき、一度消えないと1回の点灯か連続点灯か区別がつかないため)
const SHOW_OFF_DURATION: Duration = Duration::from_millis(200);
/// 正誤確定後、次のシーケンスに移るまで結果を表示しておく時間
const RESULT_INTERVAL: Duration = Duration::from_millis(1200);

fn panel_color(panel: usize) -> Color {
    PANEL_COLORS
        .iter()
        .find(|&&(p, _)| p == panel)
        .map(|&(_, c)| c)
        .unwrap_or(Color::White)
}

/// 難易度ごとの手順数
fn sequence_len(difficulty: Difficulty) -> usize {
    match difficulty {
        Difficulty::Beginner => 3,
        Difficulty::Intermediate => 5,
        Difficulty::Advanced => 7,
    }
}

/// ランダムなパネル番号(1〜4)の並びを生成する
fn generate_sequence(rng: &mut impl Rng, difficulty: Difficulty) -> Vec<usize> {
    let len = sequence_len(difficulty);
    (0..len).map(|_| rng.gen_range(1..=4)).collect()
}

/// 提示フェーズ内で、いま点灯中か消灯中か
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShowSubPhase {
    On,
    Off,
}

/// 提示フェーズ・入力フェーズの内部状態
enum Phase {
    /// シーケンスを順番に見せている最中。`shown` は見せ終えたパネル数
    Showing {
        shown: usize,
        sub_phase: ShowSubPhase,
        elapsed_in_step: Duration,
    },
    /// プレイヤーが数字キーで再現している最中。`entered` は入力済みの数
    Input { entered: usize },
    /// 正誤確定後、次のシーケンスへ移るまでの間(結果表示)
    Interval { is_correct: bool, elapsed: Duration },
}

pub struct MemoryGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    sequence: Vec<usize>,
    phase: Phase,
    /// 直近で光っている/入力されたパネル。描画用
    active_panel: Option<usize>,
    input_started_at: Instant,
    /// 正解数・連続正解のHUD表示用(描画専用)
    feedback: AnswerFeedback,
}

impl MemoryGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        let sequence = generate_sequence(&mut rng, difficulty);
        let active_panel = sequence.first().copied();
        audio::play_se(SeKind::Transition);
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            sequence,
            phase: Phase::Showing {
                shown: 0,
                sub_phase: ShowSubPhase::On,
                elapsed_in_step: Duration::ZERO,
            },
            active_panel,
            input_started_at: Instant::now(),
            feedback: AnswerFeedback::new(),
        }
    }

    fn next_sequence(&mut self) {
        let mut rng = rand::thread_rng();
        self.sequence = generate_sequence(&mut rng, self.difficulty);
        self.active_panel = self.sequence.first().copied();
        audio::play_se(SeKind::Transition);
        self.phase = Phase::Showing {
            shown: 0,
            sub_phase: ShowSubPhase::On,
            elapsed_in_step: Duration::ZERO,
        };
    }

    fn finish_question(&mut self, is_correct: bool) {
        let latency_ms = self.input_started_at.elapsed().as_millis() as f64;
        self.tracker.record(is_correct, latency_ms);
        self.feedback.record(is_correct, "");
        audio::play_se(if is_correct {
            SeKind::Correct
        } else {
            SeKind::Incorrect
        });
        self.active_panel = None;
        if !self.tracker.is_session_finished() {
            self.phase = Phase::Interval {
                is_correct,
                elapsed: Duration::ZERO,
            };
        }
    }

    /// 入力フェーズ中にパネル(1〜4)が押された時の共通処理(キー/クリック共通)
    fn press_panel(&mut self, pressed: usize) {
        let Phase::Input { entered } = &mut self.phase else {
            return;
        };
        self.active_panel = Some(pressed);
        audio::play_se(SeKind::Transition);
        let expected = self.sequence[*entered];
        if pressed != expected {
            self.finish_question(false);
            return;
        }
        *entered += 1;
        if *entered >= self.sequence.len() {
            self.finish_question(true);
        }
    }
}

impl Game for MemoryGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        if let KeyCode::Char(c @ '1'..='4') = key.code {
            let pressed = c.to_digit(10).unwrap() as usize;
            self.press_panel(pressed);
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        if self.tracker.is_session_finished() {
            return;
        }
        if !matches!(self.phase, Phase::Input { .. }) {
            return;
        }
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let (grid_area, _) = split_areas(area);
        for (panel, panel_area) in panel_areas(grid_area) {
            if contains(panel_area, mouse.column, mouse.row) {
                self.press_panel(panel);
                return;
            }
        }
    }

    fn update(&mut self, dt: Duration) {
        if self.tracker.is_session_finished() {
            return;
        }
        self.feedback.tick(dt);
        match &mut self.phase {
            Phase::Showing {
                shown,
                sub_phase,
                elapsed_in_step,
            } => {
                *elapsed_in_step += dt;
                match sub_phase {
                    ShowSubPhase::On => {
                        if *elapsed_in_step >= SHOW_ON_DURATION {
                            *elapsed_in_step = Duration::ZERO;
                            *sub_phase = ShowSubPhase::Off;
                            // 同じパネルが連続するとき区別がつくよう、必ず一度消灯する
                            self.active_panel = None;
                        }
                    }
                    ShowSubPhase::Off => {
                        if *elapsed_in_step >= SHOW_OFF_DURATION {
                            *elapsed_in_step = Duration::ZERO;
                            *shown += 1;
                            if *shown >= self.sequence.len() {
                                self.active_panel = None;
                                self.phase = Phase::Input { entered: 0 };
                                self.input_started_at = Instant::now();
                            } else {
                                self.active_panel = Some(self.sequence[*shown]);
                                audio::play_se(SeKind::Transition);
                                *sub_phase = ShowSubPhase::On;
                            }
                        }
                    }
                }
            }
            Phase::Interval { elapsed, .. } => {
                *elapsed += dt;
                if *elapsed >= RESULT_INTERVAL {
                    self.next_sequence();
                }
            }
            Phase::Input { .. } => {}
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud_area, _) = theme::split_hud(area);
        let (grid_area, footer_area) = split_areas(area);
        theme::render_hud(
            frame,
            hud_area,
            "記憶(位置と色)",
            self.difficulty,
            self.tracker.total(),
            &self.feedback,
        );

        let is_showing = matches!(self.phase, Phase::Showing { .. });
        for (panel, panel_area) in panel_areas(grid_area) {
            let is_active = self.active_panel == Some(panel);
            let color = panel_color(panel);
            // 点灯中=太枠+塗りつぶし。提示中の消灯パネルは暗くして点灯パネルを際立たせる
            let (border_type, border_color, style) = if is_active {
                (
                    BorderType::Thick,
                    color,
                    Style::default()
                        .bg(color)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )
            } else if is_showing {
                (BorderType::Rounded, theme::MUTED, Style::default().fg(theme::MUTED))
            } else {
                (BorderType::Rounded, color, Style::default().fg(color))
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(border_color))
                .style(style);
            let inner = block.inner(panel_area);
            frame.render_widget(block, panel_area);
            let number = Paragraph::new(Line::from(Span::styled(
                format!(" {panel} "),
                Style::default().add_modifier(Modifier::BOLD),
            )))
            .alignment(Alignment::Center);
            frame.render_widget(number, theme::vertical_center(inner, 1));
        }

        let (status, status_color) = match &self.phase {
            Phase::Showing { shown, .. } => (
                format!(
                    "提示中… 順番を覚えてください  ({}/{})",
                    (*shown + 1).min(self.sequence.len()),
                    self.sequence.len()
                ),
                theme::ACCENT_STRONG,
            ),
            Phase::Input { entered } => (
                format!(
                    "入力中: 数字キー1〜4で再現  残り{}手  {}",
                    self.sequence.len() - entered,
                    theme::progress_bar(
                        *entered as u32,
                        self.sequence.len() as u32,
                        self.sequence.len()
                    )
                ),
                theme::HIGHLIGHT,
            ),
            Phase::Interval { is_correct, .. } => {
                if *is_correct {
                    ("せいかい！   つぎいくよ…".to_string(), theme::CORRECT)
                } else {
                    ("ざんねん…   つぎいくよ…".to_string(), theme::INCORRECT)
                }
            }
        };
        let footer = Paragraph::new(Line::from(Span::styled(
            status,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center)
        .block(theme::sub_panel().border_style(Style::default().fg(status_color)));
        frame.render_widget(footer, footer_area);
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
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    /// 提示フェーズの1ステップ(点灯→消灯)を進める
    fn advance_one_step(game: &mut MemoryGame) {
        game.update(SHOW_ON_DURATION);
        game.update(SHOW_OFF_DURATION);
    }

    #[test]
    fn generated_sequence_has_expected_length_per_difficulty() {
        let mut rng = StdRng::seed_from_u64(1);
        assert_eq!(generate_sequence(&mut rng, Difficulty::Beginner).len(), 3);
        assert_eq!(
            generate_sequence(&mut rng, Difficulty::Intermediate).len(),
            5
        );
        assert_eq!(generate_sequence(&mut rng, Difficulty::Advanced).len(), 7);
    }

    #[test]
    fn generated_sequence_only_contains_valid_panel_numbers() {
        let mut rng = StdRng::seed_from_u64(2);
        for _ in 0..50 {
            let seq = generate_sequence(&mut rng, Difficulty::Advanced);
            for &p in &seq {
                assert!((1..=4).contains(&p));
            }
        }
    }

    #[test]
    fn showing_phase_advances_to_input_after_all_steps_shown() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        let len = game.sequence.len();
        for _ in 0..len {
            advance_one_step(&mut game);
        }
        assert!(matches!(game.phase, Phase::Input { entered: 0 }));
    }

    #[test]
    fn showing_phase_does_not_advance_before_interval_elapses() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        game.update(SHOW_ON_DURATION / 2);
        assert!(matches!(game.phase, Phase::Showing { shown: 0, .. }));
    }

    #[test]
    fn consecutive_same_panel_blinks_off_between_repeats() {
        // 同じパネルが連続するシーケンスでも、1回の点灯なのか連続点灯なのか
        // 区別できるよう、次の点灯前に必ず一度消灯を経由することを確認する
        let mut game = MemoryGame::new(Difficulty::Beginner);
        game.sequence = vec![1, 1, 3];
        game.active_panel = Some(1);
        game.phase = Phase::Showing {
            shown: 0,
            sub_phase: ShowSubPhase::On,
            elapsed_in_step: Duration::ZERO,
        };

        // 点灯継続中は消灯しない
        game.update(SHOW_ON_DURATION - Duration::from_millis(1));
        assert_eq!(game.active_panel, Some(1));

        // 点灯時間が経過すると、次のパネルが同じ1番でも一度消灯する
        game.update(Duration::from_millis(1));
        assert_eq!(game.active_panel, None);
        assert!(matches!(
            game.phase,
            Phase::Showing {
                sub_phase: ShowSubPhase::Off,
                ..
            }
        ));

        // 消灯時間が経過すると、同じ1番が再点灯する
        game.update(SHOW_OFF_DURATION);
        assert_eq!(game.active_panel, Some(1));
        assert!(matches!(
            game.phase,
            Phase::Showing {
                shown: 1,
                sub_phase: ShowSubPhase::On,
                ..
            }
        ));
    }

    #[test]
    fn correct_full_sequence_input_records_correct_answer() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        let len = game.sequence.len();
        for _ in 0..len {
            advance_one_step(&mut game);
        }
        let sequence = game.sequence.clone();
        for &panel in &sequence {
            let key = KeyEvent::from(KeyCode::Char(
                std::char::from_digit(panel as u32, 10).unwrap(),
            ));
            game.handle_key(key);
        }
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn wrong_key_during_input_immediately_finalizes_as_incorrect() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        let len = game.sequence.len();
        for _ in 0..len {
            advance_one_step(&mut game);
        }
        // 最初の1手だけ間違った番号を入力すると、残りの手を待たずに即打ち切られる
        let wrong_first = if game.sequence[0] == 1 { 2 } else { 1 };
        game.handle_key(KeyEvent::from(KeyCode::Char(
            std::char::from_digit(wrong_first as u32, 10).unwrap(),
        )));
        assert_eq!(game.tracker.total(), 1);
        // 不正解を記録した直後はインターバル表示に入り、まだ次のシーケンスは始まらない
        assert!(matches!(
            game.phase,
            Phase::Interval {
                is_correct: false,
                ..
            }
        ));
    }

    #[test]
    fn interval_advances_to_next_sequence_after_result_interval() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        let len = game.sequence.len();
        for _ in 0..len {
            advance_one_step(&mut game);
        }
        let sequence = game.sequence.clone();
        for &panel in &sequence {
            game.handle_key(KeyEvent::from(KeyCode::Char(
                std::char::from_digit(panel as u32, 10).unwrap(),
            )));
        }
        assert!(matches!(
            game.phase,
            Phase::Interval {
                is_correct: true,
                ..
            }
        ));

        // インターバル時間が経過するまでは次のシーケンスへ進まない
        game.update(RESULT_INTERVAL - Duration::from_millis(1));
        assert!(matches!(game.phase, Phase::Interval { .. }));

        // インターバル時間が経過すると次のシーケンス(提示フェーズ)が始まる
        game.update(Duration::from_millis(1));
        assert!(matches!(
            game.phase,
            Phase::Showing {
                shown: 0,
                sub_phase: ShowSubPhase::On,
                ..
            }
        ));
    }

    #[test]
    fn finishing_a_sequence_updates_hud_feedback() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        let len = game.sequence.len();
        for _ in 0..len {
            advance_one_step(&mut game);
        }
        let sequence = game.sequence.clone();
        for &panel in &sequence {
            game.handle_key(KeyEvent::from(KeyCode::Char(
                std::char::from_digit(panel as u32, 10).unwrap(),
            )));
        }
        let flash = game.feedback.current().expect("正誤確定直後は表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Correct);
        assert_eq!(game.feedback.correct(), 1);
        assert_eq!(game.feedback.streak(), 1);
        // 表示時間が過ぎると正誤表示は消えるが、正解数は残る
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
        assert_eq!(game.feedback.correct(), 1);
    }

    #[test]
    fn key_input_is_ignored_during_showing_phase() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        game.handle_key(KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn session_finishes_after_configured_question_count() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            let len = game.sequence.len();
            for _ in 0..len {
                advance_one_step(&mut game);
            }
            let sequence = game.sequence.clone();
            for &panel in &sequence {
                if game.tracker.is_session_finished() {
                    break;
                }
                let key = KeyEvent::from(KeyCode::Char(
                    std::char::from_digit(panel as u32, 10).unwrap(),
                ));
                game.handle_key(key);
            }
            // 正誤確定後のインターバルを経過させて次のシーケンスへ進める
            if !game.tracker.is_session_finished() {
                game.update(RESULT_INTERVAL);
            }
        }
        assert!(game.is_finished());
        assert_eq!(game.tracker.total(), crate::game::QUESTIONS_PER_SESSION);
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
    fn clicking_correct_panel_sequence_via_mouse_records_correct_answer() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        let len = game.sequence.len();
        for _ in 0..len {
            advance_one_step(&mut game);
        }
        let area = Rect::new(0, 0, 40, 12);
        let (grid_area, _) = split_areas(area);
        let areas = panel_areas(grid_area);
        let sequence = game.sequence.clone();
        for &panel in &sequence {
            let (_, panel_area) = areas.iter().find(|(p, _)| *p == panel).unwrap();
            game.handle_mouse(left_click(panel_area.x, panel_area.y), area);
        }
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_panel_during_showing_phase_is_ignored() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 12);
        let (grid_area, _) = split_areas(area);
        let (_, panel_area) = panel_areas(grid_area)[0];
        game.handle_mouse(left_click(panel_area.x, panel_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }
}
