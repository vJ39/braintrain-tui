use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
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
        self.elapsed_in_question += dt;
        if let Some(limit) = time_limit(self.difficulty) {
            if self.elapsed_in_question >= limit {
                self.advance_question(false);
            }
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (label_area, footer_area) = split_areas(area);
        let vertical_padding = label_area.height.saturating_sub(3) / 2;
        let mut lines: Vec<Line> = (0..vertical_padding).map(|_| Line::from("")).collect();
        let label_style = Style::default()
            .fg(self.current.display_color)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD);
        lines.push(Line::from(Span::styled(self.current.label, label_style)));

        let label_paragraph = Paragraph::new(lines)
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().bg(Color::Black))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Thick)
                    .border_style(Style::default().fg(self.current.display_color))
                    .title("この文字色と文字の意味は一致？"),
            );
        frame.render_widget(label_paragraph, label_area);

        let progress = format!(
            "{} / {}問",
            self.tracker.total(),
            crate::game::QUESTIONS_PER_SESSION
        );
        let line = Line::from(vec![Span::raw(format!(
            "← 一致    不一致 →   {progress}"
        ))]);
        let paragraph = Paragraph::new(line).block(Block::default().borders(Borders::ALL));
        frame.render_widget(paragraph, footer_area);
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
}
