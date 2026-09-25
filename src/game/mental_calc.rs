use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::theme;
use crate::game::{contains, row_index, Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "mental_calc";

const CHOICE_COUNT: usize = 4;

/// 描画エリアを「問題文」「選択肢」「フッター」に分割する。
/// 上端のHUD(theme::split_hud)を除いた残りを分ける。renderはHUDを同じsplit_hudで切り出す
fn split_areas(area: Rect) -> (Rect, Rect, Rect) {
    let (_, body) = theme::split_hud(area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(CHOICE_COUNT as u16 + 2),
            Constraint::Length(3),
        ])
        .split(body);
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
    /// 直前の回答の正誤表示(描画専用)
    feedback: AnswerFeedback,
}

impl MentalCalcGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            current: generate_question(&mut rng, difficulty),
            question_started_at: Instant::now(),
            feedback: AnswerFeedback::new(),
        }
    }

    fn advance_question(&mut self, answered_index: usize) {
        let is_correct = answered_index == self.current.correct_index;
        let latency_ms = self.question_started_at.elapsed().as_millis() as f64;
        self.tracker.record(is_correct, latency_ms);
        let answer = self.current.choices[self.current.correct_index];
        self.feedback.record(
            is_correct,
            format!("{} = {answer}", self.current.expression),
        );
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

    fn update(&mut self, dt: Duration) {
        self.feedback.tick(dt);
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud_area, _) = theme::split_hud(area);
        let (expr_area, choices_area, footer_area) = split_areas(area);
        theme::render_hud(
            frame,
            hud_area,
            "暗算スピード",
            self.difficulty,
            self.tracker.total(),
            &self.feedback,
        );

        let expr_line = Line::from(vec![
            Span::styled(
                self.current.expression.clone(),
                Style::default()
                    .fg(theme::ACCENT_STRONG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  =  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "?",
                Style::default()
                    .fg(theme::HIGHLIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        let expr_paragraph = Paragraph::new(expr_line)
            .alignment(Alignment::Center)
            .block(theme::focus_panel(" この式の答えは？ ", self.feedback.current()));
        frame.render_widget(expr_paragraph, expr_area);

        // 選択肢はクリック判定(row_index)と同じ帯に1つずつ描く
        let choices_block = theme::panel(" こたえを選ぶ ");
        let choices_inner = choices_block.inner(choices_area);
        frame.render_widget(choices_block, choices_area);
        let texts: Vec<String> = self.current.choices.iter().map(|v| v.to_string()).collect();
        theme::render_choice_rows(frame, choices_inner, &texts);

        theme::render_hint_footer(frame, footer_area, &[("1〜4", "回答"), ("q", "終了")]);
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

    #[test]
    fn answering_shows_feedback_with_expression_and_answer() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        let expression = game.current.expression.clone();
        let answer = game.current.choices[game.current.correct_index];
        let wrong = (game.current.correct_index + 1) % CHOICE_COUNT;
        game.advance_question(wrong);
        let flash = game.feedback.current().expect("回答直後は正誤を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert_eq!(flash.detail, format!("{expression} = {answer}"));
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
    }

    /// 描画結果の各行を文字列にして返す
    fn rendered_rows(game: &MentalCalcGame, area: Rect) -> Vec<String> {
        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| game.render(frame, area)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn clicking_the_row_where_a_choice_is_drawn_selects_it_even_on_tall_terminal() {
        // 縦に大きい端末では選択肢1つあたりの帯が複数行になる。
        // 画面に選択肢が見えている行をクリックしたら、その選択肢として扱われること
        let area = Rect::new(0, 0, 50, 30);
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        // 帯の位置ずれが最も大きく出る最後の選択肢を正解扱いにする
        game.current.correct_index = CHOICE_COUNT - 1;
        let correct = game.current.correct_index;
        let label = format!(" {}   {}", correct + 1, game.current.choices[correct]);
        let (_, choices_area, _) = split_areas(area);
        let inner = Block::default().borders(Borders::ALL).inner(choices_area);
        let rows = rendered_rows(&game, area);
        let row = (inner.y..inner.y + inner.height)
            .find(|&y| rows[y as usize].contains(&label))
            .expect("正解の選択肢が選択肢エリア内に描かれていること");
        game.handle_mouse(left_click(inner.x, row), area);
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }
}
