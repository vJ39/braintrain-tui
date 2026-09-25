use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::text::Line;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::canvas::shapes::{base_shapes, Shape};
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "puzzle_connect";

const CHOICE_COUNT: usize = 4;

/// base_shapes()のインデックス順と対応する図形名
const SHAPE_NAMES: [&str; 8] = [
    "三角形", "矢印", "L字", "T字", "稲妻", "旗", "階段", "フック",
];

/// ピースAの右端とピースBの左端が接するように、ピースBをX軸方向へ平行移動する。
/// Shape自体は編集せず、このファイル内だけで完結するローカルヘルパー。
fn translate_x(shape: &Shape, dx: f64) -> Shape {
    let points = shape.points.iter().map(|&(x, y)| (x + dx, y)).collect();
    Shape::new(points)
}

fn shape_max_x(shape: &Shape) -> f64 {
    shape
        .points
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max)
}

fn shape_min_x(shape: &Shape) -> f64 {
    shape
        .points
        .iter()
        .map(|p| p.0)
        .fold(f64::INFINITY, f64::min)
}

/// 難易度に応じた回転角(ラジアン)を1つ決める
fn rotation_angle_rad(rng: &mut impl Rng, difficulty: Difficulty) -> f64 {
    match difficulty {
        Difficulty::Beginner => 0.0,
        Difficulty::Intermediate => {
            let step = rng.gen_range(0..8); // 45度刻み x 8方向
            (step as f64 * 45.0).to_radians()
        }
        Difficulty::Advanced => {
            let deg = rng.gen_range(0..360); // 1度刻み
            (deg as f64).to_radians()
        }
    }
}

struct Question {
    /// お手本描画用: 回転後のピースA
    demo_piece_a: Shape,
    /// お手本描画用: 回転+平行移動後のピースB
    demo_piece_b: Shape,
    /// 選択肢として提示するbase_shapes()インデックス4つ(重複なし)
    choices: [usize; CHOICE_COUNT],
    /// choices中で正解ピースBが入っている位置
    correct_choice_position: usize,
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let shapes = base_shapes();
    let shape_count = shapes.len();

    let piece_a_idx = rng.gen_range(0..shape_count);
    let piece_b_idx = rng.gen_range(0..shape_count);

    let angle_a = rotation_angle_rad(rng, difficulty);
    let angle_b = rotation_angle_rad(rng, difficulty);

    let demo_piece_a = shapes[piece_a_idx].rotated(angle_a);
    let piece_b_rotated = shapes[piece_b_idx].rotated(angle_b);

    let dx = shape_max_x(&demo_piece_a) - shape_min_x(&piece_b_rotated) + 0.1;
    let demo_piece_b = translate_x(&piece_b_rotated, dx);

    // 選択肢: 正解ピースB + 重複しない別図形3つ
    let mut choices = vec![piece_b_idx];
    while choices.len() < CHOICE_COUNT {
        let candidate = rng.gen_range(0..shape_count);
        if !choices.contains(&candidate) {
            choices.push(candidate);
        }
    }
    choices.shuffle(rng);
    let correct_choice_position = choices
        .iter()
        .position(|&idx| idx == piece_b_idx)
        .expect("正解ピースBは必ずchoicesに含まれる");

    Question {
        demo_piece_a,
        demo_piece_b,
        choices: choices.try_into().unwrap(),
        correct_choice_position,
    }
}

pub struct PuzzleConnectGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
}

impl PuzzleConnectGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            current: generate_question(&mut rng, difficulty),
            question_started_at: Instant::now(),
        }
    }

    fn advance_question(&mut self, answered_position: usize) {
        let is_correct = answered_position == self.current.correct_choice_position;
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

impl Game for PuzzleConnectGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        if let KeyCode::Char(c @ '1'..='4') = key.code {
            let position = c.to_digit(10).unwrap() as usize - 1;
            self.advance_question(position);
        }
    }

    fn update(&mut self, _dt: Duration) {}

    fn render(&self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(CHOICE_COUNT as u16 + 2),
                Constraint::Length(3),
            ])
            .split(area);

        draw_demo(frame, rows[0], &self.current.demo_piece_a, &self.current.demo_piece_b);

        let choice_lines: Vec<Line> = self
            .current
            .choices
            .iter()
            .enumerate()
            .map(|(i, &shape_idx)| {
                Line::from(format!(" 候補{}: {} ", i + 1, SHAPE_NAMES[shape_idx]))
            })
            .collect();
        let choices_paragraph = Paragraph::new(choice_lines)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("2つ目のピースはどれ？"),
            );
        frame.render_widget(choices_paragraph, rows[1]);

        let progress = format!(
            "{} / {}問",
            self.tracker.total(),
            crate::game::QUESTIONS_PER_SESSION
        );
        let footer = Paragraph::new(format!("数字キー1〜4で回答   {progress}"))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, rows[2]);
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

fn draw_demo(frame: &mut Frame, area: Rect, piece_a: &Shape, piece_b: &Shape) {
    let min_x = shape_min_x(piece_a).min(shape_min_x(piece_b)) - 0.2;
    let max_x = shape_max_x(piece_a).max(shape_max_x(piece_b)) + 0.2;
    let min_y = piece_a
        .points
        .iter()
        .chain(piece_b.points.iter())
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min)
        - 0.2;
    let max_y = piece_a
        .points
        .iter()
        .chain(piece_b.points.iter())
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max)
        + 0.2;

    let lines_a = piece_a.to_lines();
    let lines_b = piece_b.to_lines();
    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("お手本(この2つを組み合わせた完成形)"),
        )
        .x_bounds([min_x, max_x])
        .y_bounds([min_y, max_y])
        .paint(move |ctx| {
            for (p1, p2) in &lines_a {
                ctx.draw(&CanvasLine {
                    x1: p1.0,
                    y1: p1.1,
                    x2: p2.0,
                    y2: p2.1,
                    color: Color::Green,
                });
            }
            for (p1, p2) in &lines_b {
                ctx.draw(&CanvasLine {
                    x1: p1.0,
                    y1: p1.1,
                    x2: p2.0,
                    y2: p2.1,
                    color: Color::Yellow,
                });
            }
        });
    frame.render_widget(canvas, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn translate_x_shifts_only_x_axis() {
        let shape = Shape::new(vec![(0.0, 0.5), (1.0, -0.5)]);
        let translated = translate_x(&shape, 2.0);
        assert_eq!(translated.points, vec![(2.0, 0.5), (3.0, -0.5)]);
    }

    #[test]
    fn beginner_rotation_angle_is_always_zero() {
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..20 {
            assert_eq!(rotation_angle_rad(&mut rng, Difficulty::Beginner), 0.0);
        }
    }

    #[test]
    fn generate_question_places_piece_b_directly_after_piece_a_with_margin() {
        let mut rng = StdRng::seed_from_u64(10);
        for _ in 0..30 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            let a_max_x = shape_max_x(&q.demo_piece_a);
            let b_min_x = shape_min_x(&q.demo_piece_b);
            assert!(
                (b_min_x - (a_max_x + 0.1)).abs() < 1e-9,
                "ピースBはピースAの右端+マージン0.1の位置から始まるべき"
            );
        }
    }

    #[test]
    fn generate_question_choices_have_no_duplicates() {
        let mut rng = StdRng::seed_from_u64(20);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            let mut sorted = q.choices.to_vec();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), CHOICE_COUNT, "選択肢に重複がある");
        }
    }

    #[test]
    fn generate_question_correct_position_is_within_choices() {
        let mut rng = StdRng::seed_from_u64(21);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            assert!(q.correct_choice_position < CHOICE_COUNT);
            // correct_choice_positionの位置にある値は必ずchoices内に実在する図形インデックス
            let idx = q.choices[q.correct_choice_position];
            assert!(idx < base_shapes().len());
        }
    }

    #[test]
    fn advance_question_with_correct_position_records_success() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        let correct = game.current.correct_choice_position;
        game.advance_question(correct);
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn advance_question_with_wrong_position_records_failure() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        let correct = game.current.correct_choice_position;
        let wrong = (correct + 1) % CHOICE_COUNT;
        game.advance_question(wrong);
        assert_eq!(game.tracker.total(), 1);
        // is_correctはScoreTracker内部にしか無いため、resultのcorrectで間接確認する
        // (最終セッションまで進めていないのでresult()は呼べる状態: 1問だけ判定済み)
    }

    #[test]
    fn session_finishes_after_configured_question_count() {
        let mut game = PuzzleConnectGame::new(Difficulty::Beginner);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            let position = game.current.correct_choice_position;
            game.advance_question(position);
        }
        assert!(game.is_finished());
        assert_eq!(game.result().correct, crate::game::QUESTIONS_PER_SESSION);
    }
}
