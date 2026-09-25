use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::Rng;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::canvas::shapes::{base_shapes, Shape};
use crate::game::{column_index, contains, Difficulty, Game, GameResult, ScoreTracker};

/// このゲームの描画エリアを「図形表示」と「選択肢フッター」に分割する
/// (renderとhandle_mouseで同じ分割を使うため関数化)
fn split_areas(area: Rect) -> (Rect, Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);
    (rows[0], rows[1])
}

pub const GAME_ID: &str = "shape_rotate";

struct Question {
    original: Shape,
    transformed: Shape,
    is_same: bool,
}

fn angle_pool(difficulty: Difficulty) -> Vec<f64> {
    match difficulty {
        Difficulty::Beginner => vec![0.0, 90.0, 180.0, 270.0],
        Difficulty::Intermediate => (0..24).map(|i| i as f64 * 15.0).collect(),
        Difficulty::Advanced => (0..360).map(|i| i as f64).collect(),
    }
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let shapes = base_shapes();
    let original_index = rng.gen_range(0..shapes.len());
    let original = shapes[original_index].clone();
    let is_same = rng.gen_bool(0.5);

    let angles = angle_pool(difficulty);
    let angle_deg = angles[rng.gen_range(0..angles.len())];
    let angle_rad = angle_deg.to_radians();

    let transformed = if is_same {
        original.rotated(angle_rad)
    } else if difficulty == Difficulty::Beginner || rng.gen_bool(0.5) {
        original.rotated(angle_rad).mirrored_x()
    } else {
        let mut other_index = rng.gen_range(0..shapes.len());
        while other_index == original_index {
            other_index = rng.gen_range(0..shapes.len());
        }
        shapes[other_index].rotated(angle_rad)
    };

    Question {
        original,
        transformed,
        is_same,
    }
}

pub struct ShapeRotateGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
}

impl ShapeRotateGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            current: generate_question(&mut rng, difficulty),
            question_started_at: Instant::now(),
        }
    }

    fn advance_question(&mut self, answered_same: bool) {
        let is_correct = answered_same == self.current.is_same;
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

impl Game for ShapeRotateGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        match key.code {
            KeyCode::Left => self.advance_question(true),
            KeyCode::Right => self.advance_question(false),
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
            self.advance_question(col == 0);
        }
    }

    fn update(&mut self, _dt: Duration) {}

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (shapes_area, footer_area) = split_areas(area);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(shapes_area);

        draw_shape(frame, cols[0], "元の図形", &self.current.original);
        draw_shape(frame, cols[1], "比較図形", &self.current.transformed);

        let progress = format!(
            "{} / {}問",
            self.tracker.total(),
            crate::game::QUESTIONS_PER_SESSION
        );
        let line = Line::from(vec![Span::raw(format!(
            "← 同じ    違う →   {progress}"
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

fn draw_shape(frame: &mut Frame, area: Rect, title: &str, shape: &Shape) {
    let lines = shape.to_lines();
    let canvas = Canvas::default()
        .block(Block::default().borders(Borders::ALL).title(title.to_string()))
        .x_bounds([-1.0, 1.0])
        .y_bounds([-1.0, 1.0])
        .paint(move |ctx| {
            for (p1, p2) in &lines {
                ctx.draw(&CanvasLine {
                    x1: p1.0,
                    y1: p1.1,
                    x2: p2.0,
                    y2: p2.1,
                    color: Color::Green,
                });
            }
        });
    frame.render_widget(canvas, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    /// 多角形の符号付き面積(shoelace公式)。回転では符号が保たれ、鏡像では反転する。
    fn signed_area(shape: &Shape) -> f64 {
        let n = shape.points.len();
        let mut sum = 0.0;
        for i in 0..n {
            let (x1, y1) = shape.points[i];
            let (x2, y2) = shape.points[(i + 1) % n];
            sum += x1 * y2 - x2 * y1;
        }
        sum / 2.0
    }

    #[test]
    fn same_question_preserves_orientation_of_original() {
        let mut rng = StdRng::seed_from_u64(1);
        let mut saw_same = false;
        for _ in 0..50 {
            let question = generate_question(&mut rng, Difficulty::Intermediate);
            if question.is_same {
                saw_same = true;
                let orig_sign = signed_area(&question.original).signum();
                let trans_sign = signed_area(&question.transformed).signum();
                assert_eq!(orig_sign, trans_sign);
            }
        }
        assert!(saw_same, "50回中一度もis_same=trueが出なかった");
    }

    #[test]
    fn beginner_incorrect_question_is_always_mirrored() {
        let mut rng = StdRng::seed_from_u64(2);
        let mut saw_incorrect = false;
        for _ in 0..50 {
            let question = generate_question(&mut rng, Difficulty::Beginner);
            if !question.is_same {
                saw_incorrect = true;
                let orig_sign = signed_area(&question.original).signum();
                let trans_sign = signed_area(&question.transformed).signum();
                assert_eq!(orig_sign, -trans_sign, "初級の不正解は必ず鏡像であるべき");
            }
        }
        assert!(saw_incorrect, "50回中一度も不正解パターンが出なかった");
    }

    #[test]
    fn advance_question_records_correct_answer() {
        let mut game = ShapeRotateGame::new(Difficulty::Beginner);
        let answer = game.current.is_same;
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
    fn clicking_left_half_of_footer_answers_same() {
        let mut game = ShapeRotateGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 10);
        let (_, footer_area) = split_areas(area);
        game.current.is_same = true;
        game.handle_mouse(left_click(footer_area.x, footer_area.y), area);
        // is_same=trueのとき左クリック(同じ)を押したので正解のはず
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_right_half_of_footer_answers_different() {
        let mut game = ShapeRotateGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 10);
        let (_, footer_area) = split_areas(area);
        game.current.is_same = false;
        let right_column = footer_area.x + footer_area.width - 1;
        game.handle_mouse(left_click(right_column, footer_area.y), area);
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_outside_footer_area_does_nothing() {
        let mut game = ShapeRotateGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 10);
        let (shapes_area, _) = split_areas(area);
        game.handle_mouse(left_click(shapes_area.x, shapes_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn session_finishes_after_configured_question_count() {
        let mut game = ShapeRotateGame::new(Difficulty::Beginner);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            let answer = game.current.is_same;
            game.advance_question(answer);
        }
        assert!(game.is_finished());
    }
}
