use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::canvas::shapes::{base_shapes, Shape};
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "pattern_fill";

const CHOICE_COUNT: usize = 4;
const GRID_SIZE: usize = 9;

/// グリッド1マス分の内容(どの図形を何度回転させて置くか)
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cell {
    shape_index: usize,
    angle_deg: f64,
}

struct Question {
    /// 3x3グリッド(row-major)。blank_indexの位置も規則通りの値を保持しておき、
    /// 「本来ここに入るべき図形」の判定に使う(描画時のみblank_indexを隠す)
    cells: [Cell; GRID_SIZE],
    blank_index: usize,
    choices: [Cell; CHOICE_COUNT],
    correct_index: usize,
}

/// Beginner: 全9マス同じ図形の繰り返し
fn generate_beginner(rng: &mut impl Rng, shapes: &[Shape]) -> ([Cell; GRID_SIZE], Cell) {
    let shape_index = rng.gen_range(0..shapes.len());
    let cell = Cell {
        shape_index,
        angle_deg: 0.0,
    };
    ([cell; GRID_SIZE], cell)
}

/// Intermediate: 行ごとに図形が変わる(各行は同じ図形、3行で3種)
fn generate_intermediate(
    rng: &mut impl Rng,
    shapes: &[Shape],
    blank_index: usize,
) -> ([Cell; GRID_SIZE], Cell) {
    let mut row_indices: Vec<usize> = Vec::with_capacity(3);
    while row_indices.len() < 3 {
        let idx = rng.gen_range(0..shapes.len());
        if !row_indices.contains(&idx) {
            row_indices.push(idx);
        }
    }
    let mut cells = [Cell {
        shape_index: 0,
        angle_deg: 0.0,
    }; GRID_SIZE];
    for row in 0..3 {
        for col in 0..3 {
            cells[row * 3 + col] = Cell {
                shape_index: row_indices[row],
                angle_deg: 0.0,
            };
        }
    }
    (cells, cells[blank_index])
}

/// Advanced: 各セルごとに一定角度ずつ回転していく(0度→step度→2*step度…)
fn generate_advanced(rng: &mut impl Rng, shapes: &[Shape]) -> ([Cell; GRID_SIZE], Cell) {
    // 8ステップ分(i=0..=8)を360度未満に収め、かつ全セルの角度が重複しないステップのみ採用
    const STEP_POOL_DEG: [f64; 3] = [30.0, 40.0, 50.0];
    let step_deg = STEP_POOL_DEG[rng.gen_range(0..STEP_POOL_DEG.len())];
    let shape_index = rng.gen_range(0..shapes.len());
    let mut cells = [Cell {
        shape_index: 0,
        angle_deg: 0.0,
    }; GRID_SIZE];
    for (i, cell) in cells.iter_mut().enumerate() {
        *cell = Cell {
            shape_index,
            angle_deg: normalize_angle(i as f64 * step_deg),
        };
    }
    let blank = cells[0]; // placeholder, replaced by caller with correct index
    (cells, blank)
}

fn normalize_angle(angle_deg: f64) -> f64 {
    let mut a = angle_deg % 360.0;
    if a < 0.0 {
        a += 360.0;
    }
    a
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let shapes = base_shapes();
    let blank_index = rng.gen_range(0..GRID_SIZE);

    let (cells, correct_cell) = match difficulty {
        Difficulty::Beginner => generate_beginner(rng, &shapes),
        Difficulty::Intermediate => generate_intermediate(rng, &shapes, blank_index),
        Difficulty::Advanced => {
            let (cells, _) = generate_advanced(rng, &shapes);
            (cells, cells[blank_index])
        }
    };

    // (Cell, is_correct) のペアで作ってからシャッフルし、最後にcorrect_indexを特定する
    let mut tagged: Vec<(Cell, bool)> = vec![(correct_cell, true)];
    match difficulty {
        Difficulty::Beginner => {
            let mut used = vec![correct_cell.shape_index];
            while tagged.len() < CHOICE_COUNT {
                let idx = rng.gen_range(0..shapes.len());
                if !used.contains(&idx) {
                    used.push(idx);
                    tagged.push((
                        Cell {
                            shape_index: idx,
                            angle_deg: 0.0,
                        },
                        false,
                    ));
                }
            }
        }
        Difficulty::Intermediate => {
            // グリッドに使われている行の図形(正解含む)を集め、正解以外の行の図形2つ+
            // グリッドに登場しない図形1つを不正解選択肢にする
            let mut row_shape_indices: Vec<usize> = (0..3).map(|row| cells[row * 3].shape_index).collect();
            row_shape_indices.dedup();
            for &idx in &row_shape_indices {
                if idx != correct_cell.shape_index && tagged.len() < CHOICE_COUNT - 1 + 1 {
                    tagged.push((
                        Cell {
                            shape_index: idx,
                            angle_deg: 0.0,
                        },
                        false,
                    ));
                }
            }
            let mut used: Vec<usize> = row_shape_indices.clone();
            while tagged.len() < CHOICE_COUNT {
                let idx = rng.gen_range(0..shapes.len());
                if !used.contains(&idx) {
                    used.push(idx);
                    tagged.push((
                        Cell {
                            shape_index: idx,
                            angle_deg: 0.0,
                        },
                        false,
                    ));
                }
            }
        }
        Difficulty::Advanced => {
            let mut candidate_angles: Vec<f64> = cells
                .iter()
                .filter(|c| (c.angle_deg - correct_cell.angle_deg).abs() > 1e-9)
                .map(|c| c.angle_deg)
                .collect();
            candidate_angles.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
            candidate_angles.shuffle(rng);
            for angle in candidate_angles.into_iter().take(CHOICE_COUNT - 1) {
                tagged.push((
                    Cell {
                        shape_index: correct_cell.shape_index,
                        angle_deg: angle,
                    },
                    false,
                ));
            }
            // グリッド内に十分な数の異なる角度が無い場合のフォールバック
            while tagged.len() < CHOICE_COUNT {
                let extra = normalize_angle(correct_cell.angle_deg + rng.gen_range(10..350) as f64);
                if !tagged.iter().any(|(c, _)| (c.angle_deg - extra).abs() < 1e-9) {
                    tagged.push((
                        Cell {
                            shape_index: correct_cell.shape_index,
                            angle_deg: extra,
                        },
                        false,
                    ));
                }
            }
        }
    }

    tagged.shuffle(rng);
    let correct_index = tagged.iter().position(|(_, is_correct)| *is_correct).unwrap();
    let choices: [Cell; CHOICE_COUNT] = tagged
        .into_iter()
        .map(|(c, _)| c)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();

    Question {
        cells,
        blank_index,
        choices,
        correct_index,
    }
}

pub struct PatternFillGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
}

impl PatternFillGame {
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

impl Game for PatternFillGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        if let KeyCode::Char(c @ '1'..='4') = key.code {
            let index = c.to_digit(10).unwrap() as usize - 1;
            self.advance_question(index);
        }
    }

    fn update(&mut self, _dt: Duration) {}

    fn render(&self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(11),
                Constraint::Length(7),
                Constraint::Length(3),
            ])
            .split(area);

        draw_grid(frame, rows[0], &self.current);

        let choice_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(25); 4])
            .split(rows[1]);
        let shapes = base_shapes();
        for (i, col_area) in choice_cols.iter().enumerate() {
            let cell = self.current.choices[i];
            let shape = shapes[cell.shape_index].rotated(cell.angle_deg.to_radians());
            draw_choice(frame, *col_area, i + 1, &shape);
        }

        let progress = format!(
            "{} / {}問",
            self.tracker.total(),
            crate::game::QUESTIONS_PER_SESSION
        );
        let footer = Paragraph::new(format!("？に当てはまる図形を数字キー1〜4で回答   {progress}"))
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

fn draw_grid(frame: &mut Frame, area: Rect, question: &Question) {
    let shapes = base_shapes();
    let blank_index = question.blank_index;
    let cells = question.cells;

    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("この規則に当てはまる図形は？"),
        )
        .x_bounds([-3.2, 3.2])
        .y_bounds([-3.2, 3.2])
        .paint(move |ctx| {
            for (i, cell) in cells.iter().enumerate() {
                let row = i / 3;
                let col = i % 3;
                let cx = (col as f64 - 1.0) * 2.2;
                let cy = (1.0 - row as f64) * 2.2;

                if i == blank_index {
                    // 空欄マスは枠と「?」のみ表示する
                    let half = 0.9;
                    let corners = [
                        (cx - half, cy - half),
                        (cx + half, cy - half),
                        (cx + half, cy + half),
                        (cx - half, cy + half),
                    ];
                    for k in 0..4 {
                        let (x1, y1) = corners[k];
                        let (x2, y2) = corners[(k + 1) % 4];
                        ctx.draw(&CanvasLine {
                            x1,
                            y1,
                            x2,
                            y2,
                            color: Color::DarkGray,
                        });
                    }
                    ctx.print(cx - 0.2, cy, "?");
                } else {
                    let shape = shapes[cell.shape_index].rotated(cell.angle_deg.to_radians());
                    for (p1, p2) in shape.to_lines() {
                        ctx.draw(&CanvasLine {
                            x1: p1.0 * 0.8 + cx,
                            y1: p1.1 * 0.8 + cy,
                            x2: p2.0 * 0.8 + cx,
                            y2: p2.1 * 0.8 + cy,
                            color: Color::Green,
                        });
                    }
                }
            }
        });
    frame.render_widget(canvas, area);
}

fn draw_choice(frame: &mut Frame, area: Rect, number: usize, shape: &Shape) {
    let lines = shape.to_lines();
    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(number.to_string()),
        )
        .x_bounds([-1.0, 1.0])
        .y_bounds([-1.0, 1.0])
        .paint(move |ctx| {
            for (p1, p2) in &lines {
                ctx.draw(&CanvasLine {
                    x1: p1.0,
                    y1: p1.1,
                    x2: p2.0,
                    y2: p2.1,
                    color: Color::Cyan,
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
    fn choices_always_have_exactly_one_correct_and_are_in_range() {
        let mut rng = StdRng::seed_from_u64(10);
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            for _ in 0..50 {
                let q = generate_question(&mut rng, difficulty);
                assert!(q.correct_index < CHOICE_COUNT);
                assert!(q.blank_index < GRID_SIZE);
            }
        }
    }

    #[test]
    fn beginner_grid_is_a_single_repeated_shape() {
        let mut rng = StdRng::seed_from_u64(11);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            let expected = q.cells[0].shape_index;
            assert!(q.cells.iter().all(|c| c.shape_index == expected));
            assert!(q.cells.iter().all(|c| c.angle_deg == 0.0));
            let correct = q.choices[q.correct_index];
            assert_eq!(correct.shape_index, expected);
        }
    }

    #[test]
    fn beginner_choices_have_no_duplicate_shapes() {
        let mut rng = StdRng::seed_from_u64(12);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            let mut indices: Vec<usize> = q.choices.iter().map(|c| c.shape_index).collect();
            indices.sort();
            indices.dedup();
            assert_eq!(indices.len(), CHOICE_COUNT, "選択肢の図形が重複している");
        }
    }

    #[test]
    fn intermediate_rows_share_the_same_shape() {
        let mut rng = StdRng::seed_from_u64(13);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            for row in 0..3 {
                let expected = q.cells[row * 3].shape_index;
                for col in 0..3 {
                    assert_eq!(q.cells[row * 3 + col].shape_index, expected);
                }
            }
            let blank_row = q.blank_index / 3;
            let expected_shape = q.cells[blank_row * 3].shape_index;
            let correct = q.choices[q.correct_index];
            assert_eq!(correct.shape_index, expected_shape);
        }
    }

    #[test]
    fn intermediate_uses_three_distinct_row_shapes() {
        let mut rng = StdRng::seed_from_u64(14);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            let mut row_shapes: Vec<usize> = (0..3).map(|row| q.cells[row * 3].shape_index).collect();
            row_shapes.sort();
            row_shapes.dedup();
            assert_eq!(row_shapes.len(), 3, "行の図形が重複している");
        }
    }

    #[test]
    fn advanced_cells_rotate_by_a_constant_step() {
        let mut rng = StdRng::seed_from_u64(15);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            let shape_index = q.cells[0].shape_index;
            assert!(q.cells.iter().all(|c| c.shape_index == shape_index));

            let step = {
                // i=1のセルの角度を基準ステップとみなす(全セル同一ステップである前提)
                let mut d = q.cells[1].angle_deg - q.cells[0].angle_deg;
                if d < 0.0 {
                    d += 360.0;
                }
                d
            };
            for i in 0..GRID_SIZE {
                let expected = normalize_angle(i as f64 * step);
                let actual = q.cells[i].angle_deg;
                assert!(
                    (expected - actual).abs() < 1e-6,
                    "セル{i}の角度が規則から外れている: expected={expected}, actual={actual}"
                );
            }

            let correct = q.choices[q.correct_index];
            assert_eq!(correct.shape_index, shape_index);
            assert!((correct.angle_deg - q.cells[q.blank_index].angle_deg).abs() < 1e-6);
        }
    }

    #[test]
    fn advanced_choices_have_distinct_angles() {
        let mut rng = StdRng::seed_from_u64(16);
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            let mut angles: Vec<i64> = q
                .choices
                .iter()
                .map(|c| (c.angle_deg.round()) as i64)
                .collect();
            angles.sort();
            angles.dedup();
            assert_eq!(angles.len(), CHOICE_COUNT, "選択肢の回転角が重複している");
        }
    }

    #[test]
    fn normalize_angle_wraps_into_0_to_360_range() {
        assert_eq!(normalize_angle(0.0), 0.0);
        assert!((normalize_angle(370.0) - 10.0).abs() < 1e-9);
        assert!((normalize_angle(-30.0) - 330.0).abs() < 1e-9);
        assert!((normalize_angle(720.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn advance_question_records_correct_answer() {
        let mut game = PatternFillGame::new(Difficulty::Beginner);
        let answer = game.current.correct_index;
        game.advance_question(answer);
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn advance_question_records_incorrect_answer() {
        let mut game = PatternFillGame::new(Difficulty::Beginner);
        let wrong = (game.current.correct_index + 1) % CHOICE_COUNT;
        game.advance_question(wrong);
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn session_finishes_after_configured_question_count() {
        let mut game = PatternFillGame::new(Difficulty::Beginner);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            let answer = game.current.correct_index;
            game.advance_question(answer);
        }
        assert!(game.is_finished());
    }
}
