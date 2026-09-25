use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "memory";

/// パネル番号(1〜4)ごとの色。1=左上, 2=右上, 3=左下, 4=右下
const PANEL_COLORS: [(usize, Color); 4] = [
    (1, Color::Red),
    (2, Color::Blue),
    (3, Color::Green),
    (4, Color::Yellow),
];

/// 提示フェーズで1パネルを光らせる間隔
const SHOW_INTERVAL: Duration = Duration::from_millis(600);

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

/// 提示フェーズ・入力フェーズの内部状態
enum Phase {
    /// シーケンスを順番に見せている最中。`shown` は見せ終えたパネル数
    Showing {
        shown: usize,
        elapsed_in_step: Duration,
    },
    /// プレイヤーが数字キーで再現している最中。`entered` は入力済みの数
    Input { entered: usize },
}

pub struct MemoryGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    sequence: Vec<usize>,
    phase: Phase,
    /// 直近で光っている/入力されたパネル。描画用
    active_panel: Option<usize>,
    input_started_at: Instant,
}

impl MemoryGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        let sequence = generate_sequence(&mut rng, difficulty);
        let active_panel = sequence.first().copied();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            sequence,
            phase: Phase::Showing {
                shown: 0,
                elapsed_in_step: Duration::ZERO,
            },
            active_panel,
            input_started_at: Instant::now(),
        }
    }

    fn next_sequence(&mut self) {
        let mut rng = rand::thread_rng();
        self.sequence = generate_sequence(&mut rng, self.difficulty);
        self.active_panel = self.sequence.first().copied();
        self.phase = Phase::Showing {
            shown: 0,
            elapsed_in_step: Duration::ZERO,
        };
    }

    fn finish_question(&mut self, is_correct: bool) {
        let latency_ms = self.input_started_at.elapsed().as_millis() as f64;
        self.tracker.record(is_correct, latency_ms);
        audio::play_se(if is_correct {
            SeKind::Correct
        } else {
            SeKind::Incorrect
        });
        if !self.tracker.is_session_finished() {
            self.next_sequence();
        }
    }
}

impl Game for MemoryGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        let Phase::Input { entered } = &mut self.phase else {
            return;
        };
        if let KeyCode::Char(c @ '1'..='4') = key.code {
            let pressed = c.to_digit(10).unwrap() as usize;
            self.active_panel = Some(pressed);
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

    fn update(&mut self, dt: Duration) {
        if self.tracker.is_session_finished() {
            return;
        }
        if let Phase::Showing {
            shown,
            elapsed_in_step,
        } = &mut self.phase
        {
            *elapsed_in_step += dt;
            if *elapsed_in_step >= SHOW_INTERVAL {
                *elapsed_in_step = Duration::ZERO;
                *shown += 1;
                if *shown >= self.sequence.len() {
                    self.active_panel = None;
                    self.phase = Phase::Input { entered: 0 };
                    self.input_started_at = Instant::now();
                } else {
                    self.active_panel = Some(self.sequence[*shown]);
                }
            }
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(7), Constraint::Length(3)])
            .split(area);

        let grid_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);
        let left_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(grid_cols[0]);
        let right_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(grid_cols[1]);
        let panel_areas = [
            (1usize, left_rows[0]),
            (2, right_rows[0]),
            (3, left_rows[1]),
            (4, right_rows[1]),
        ];

        for (panel, panel_area) in panel_areas {
            let is_active = self.active_panel == Some(panel);
            let color = panel_color(panel);
            let style = if is_active {
                Style::default()
                    .bg(color)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!("{panel}"))
                .style(style);
            frame.render_widget(Paragraph::new("").block(block), panel_area);
        }

        let status = match &self.phase {
            Phase::Showing { .. } => "提示中… 順番を覚えてください".to_string(),
            Phase::Input { entered } => format!(
                "入力中: 数字キー1〜4で再現(残り{}手)",
                self.sequence.len() - entered
            ),
        };
        let progress = format!(
            "{} / {}問",
            self.tracker.total(),
            crate::game::QUESTIONS_PER_SESSION
        );
        let footer = Paragraph::new(Line::from(format!("{status}   {progress}")))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, rows[1]);
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
            game.update(SHOW_INTERVAL);
        }
        assert!(matches!(game.phase, Phase::Input { entered: 0 }));
    }

    #[test]
    fn showing_phase_does_not_advance_before_interval_elapses() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        game.update(SHOW_INTERVAL / 2);
        assert!(matches!(game.phase, Phase::Showing { shown: 0, .. }));
    }

    #[test]
    fn correct_full_sequence_input_records_correct_answer() {
        let mut game = MemoryGame::new(Difficulty::Beginner);
        let len = game.sequence.len();
        for _ in 0..len {
            game.update(SHOW_INTERVAL);
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
            game.update(SHOW_INTERVAL);
        }
        // 最初の1手だけ間違った番号を入力すると、残りの手を待たずに即打ち切られる
        let wrong_first = if game.sequence[0] == 1 { 2 } else { 1 };
        game.handle_key(KeyEvent::from(KeyCode::Char(
            std::char::from_digit(wrong_first as u32, 10).unwrap(),
        )));
        assert_eq!(game.tracker.total(), 1);
        // 不正解を記録した後、次のシーケンスへ進んでいるはず(セッションが終わっていない前提)
        assert!(matches!(game.phase, Phase::Showing { .. }));
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
                game.update(SHOW_INTERVAL);
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
        }
        assert!(game.is_finished());
        assert_eq!(game.tracker.total(), crate::game::QUESTIONS_PER_SESSION);
    }
}
