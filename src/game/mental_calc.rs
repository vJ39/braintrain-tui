use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::{contains, row_index, Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "mental_calc";

const CHOICE_COUNT: usize = 4;

/// 描画エリアを「問題文」「選択肢」「フッター」に分割する
fn split_areas(area: Rect) -> (Rect, Rect, Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(CHOICE_COUNT as u16 + 2),
            Constraint::Length(3),
        ])
        .split(area);
    (rows[0], rows[1], rows[2])
}

struct Question {
    expression: String,
    choices: [i64; CHOICE_COUNT],
    correct_index: usize,
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let (expression, answer) = match difficulty {
        Difficulty::Beginner => {
            let a = rng.gen_range(1..=9);
            let b = rng.gen_range(1..=9);
            if rng.gen_bool(0.5) {
                (format!("{a} + {b}"), a + b)
            } else {
                let (x, y) = if a >= b { (a, b) } else { (b, a) };
                (format!("{x} - {y}"), x - y)
            }
        }
        Difficulty::Intermediate => {
            let a = rng.gen_range(10..=99);
            let b = rng.gen_range(10..=99);
            if rng.gen_bool(0.5) {
                (format!("{a} + {b}"), a + b)
            } else {
                let (x, y) = if a >= b { (a, b) } else { (b, a) };
                (format!("{x} - {y}"), x - y)
            }
        }
        Difficulty::Advanced => {
            if rng.gen_bool(0.5) {
                let a = rng.gen_range(11..=20);
                let b = rng.gen_range(2..=9);
                (format!("{a} × {b}"), a * b)
            } else {
                let b = rng.gen_range(2..=9);
                let quotient = rng.gen_range(2..=12);
                let a = b * quotient;
                (format!("{a} ÷ {b}"), quotient)
            }
        }
    };

    let mut choices = vec![answer];
    while choices.len() < CHOICE_COUNT {
        let offset = rng.gen_range(-5..=5);
        let candidate = answer + offset;
        if offset != 0 && !choices.contains(&candidate) {
            choices.push(candidate);
        }
    }
    choices.shuffle(rng);
    let correct_index = choices.iter().position(|&c| c == answer).unwrap();

    Question {
        expression,
        choices: choices.try_into().unwrap(),
        correct_index,
    }
}

pub struct MentalCalcGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
}

impl MentalCalcGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            current: generate_question(&mut rng, difficulty),
            question_started_at: Instant::now(),
        }
    }

    fn advance_question(&mut self, answered_index: usize) {
        let is_correct = answered_index == self.current.correct_index;
        let latency_ms = self.question_started_at.elapsed().as_millis() as f64;
        self.tracker.record(is_correct, latency_ms);
        audio::play_se(if is_correct {
            SeKind::Correct
        } else {
            SeKind::Incorrect
        });
        if !self.tracker.is_session_finished() {
            let mut rng = rand::thread_rng();
            self.current = generate_question(&mut rng, self.difficulty);
            self.question_started_at = Instant::now();
        }
    }
}

impl Game for MentalCalcGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        if let KeyCode::Char(c @ '1'..='4') = key.code {
            let index = c.to_digit(10).unwrap() as usize - 1;
            self.advance_question(index);
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        if self.tracker.is_session_finished() {
            return;
        }
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let (_, choices_area, _) = split_areas(area);
        let inner = Block::default().borders(Borders::ALL).inner(choices_area);
        if !contains(inner, mouse.column, mouse.row) {
            return;
        }
        if let Some(index) = row_index(inner, mouse.row, CHOICE_COUNT as u16) {
            self.advance_question(index);
        }
    }

    fn update(&mut self, _dt: Duration) {}

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (expr_area, choices_area, footer_area) = split_areas(area);

        let expr_paragraph = Paragraph::new(Line::from(self.current.expression.clone()))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("この式の答えは？"));
        frame.render_widget(expr_paragraph, expr_area);

        let choice_lines: Vec<Line> = self
            .current
            .choices
            .iter()
            .enumerate()
            .map(|(i, value)| Line::from(format!(" {}: {value} ", i + 1)))
            .collect();
        let choices_paragraph = Paragraph::new(choice_lines)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(choices_paragraph, choices_area);

        let progress = format!(
            "{} / {}問",
            self.tracker.total(),
            crate::game::QUESTIONS_PER_SESSION
        );
        let footer = Paragraph::new(format!("数字キー1〜4で回答   {progress}"))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
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

    #[test]
    fn generated_question_has_unique_choices() {
        let mut rng = StdRng::seed_from_u64(30);
        for _ in 0..100 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            let mut sorted = q.choices.to_vec();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), CHOICE_COUNT, "選択肢に重複がある");
            assert!(q.correct_index < CHOICE_COUNT);
        }
    }

    #[test]
    fn advanced_division_always_divides_evenly() {
        let mut rng = StdRng::seed_from_u64(31);
        for _ in 0..100 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            if q.expression.contains('÷') {
                let answer = q.choices[q.correct_index];
                let parts: Vec<&str> = q.expression.split(" ÷ ").collect();
                let a: i64 = parts[0].parse().unwrap();
                let b: i64 = parts[1].parse().unwrap();
                assert_eq!(a % b, 0);
                assert_eq!(a / b, answer);
            }
        }
    }

    #[test]
    fn beginner_subtraction_never_goes_negative() {
        let mut rng = StdRng::seed_from_u64(32);
        for _ in 0..100 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            if q.expression.contains('-') {
                let answer = q.choices[q.correct_index];
                assert!(answer >= 0);
            }
        }
    }

    #[test]
    fn advance_question_records_correct_answer() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        let answer = game.current.correct_index;
        game.advance_question(answer);
        assert_eq!(game.tracker.total(), 1);
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
    fn clicking_a_choice_row_selects_that_index() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 15);
        let (_, choices_area, _) = split_areas(area);
        let inner = Block::default().borders(Borders::ALL).inner(choices_area);
        let correct = game.current.correct_index;
        let row = inner.y + correct as u16;
        game.handle_mouse(left_click(inner.x, row), area);
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_outside_choices_area_does_nothing() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 15);
        let (expr_area, _, _) = split_areas(area);
        game.handle_mouse(left_click(expr_area.x, expr_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }
}
