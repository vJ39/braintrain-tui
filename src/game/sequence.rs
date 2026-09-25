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

pub const GAME_ID: &str = "sequence";

const CHOICE_COUNT: usize = 4;

/// 描画エリアを「数列表示」「選択肢」「フッター」に分割する。
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

/// 出題する数列の生成規則
#[derive(Debug, Clone, Copy)]
enum Pattern {
    /// 等差数列
    Arithmetic { first: i64, diff: i64 },
    /// 等比数列
    Geometric { first: i64, ratio: i64 },
    /// 階差数列(隣接する差自体が等差数列=二階差分が一定)
    SecondDifference { first: i64, diff0: i64, delta: i64 },
    /// フィボナッチ風数列(直前2項の和が次項)
    Fibonacci { a: i64, b: i64 },
}

/// パターンに従い先頭からcount項を生成する(最後の1項が出題の正解になる)
fn terms_for(pattern: Pattern, count: usize) -> Vec<i64> {
    let mut terms = Vec::with_capacity(count);
    match pattern {
        Pattern::Arithmetic { first, diff } => {
            for i in 0..count {
                terms.push(first + diff * i as i64);
            }
        }
        Pattern::Geometric { first, ratio } => {
            let mut v = first;
            for _ in 0..count {
                terms.push(v);
                v *= ratio;
            }
        }
        Pattern::SecondDifference { first, diff0, delta } => {
            let mut v = first;
            let mut d = diff0;
            for _ in 0..count {
                terms.push(v);
                v += d;
                d += delta;
            }
        }
        Pattern::Fibonacci { a, b } => {
            let mut x = a;
            let mut y = b;
            for _ in 0..count {
                terms.push(x);
                let next = x + y;
                x = y;
                y = next;
            }
        }
    }
    terms
}

/// 法則を誤認した場合に出てきそうな紛らわしい値を、多い順に候補として返す
fn near_miss_candidates(pattern: Pattern, sequence: &[i64]) -> Vec<i64> {
    let len = sequence.len();
    let last = sequence[len - 1];
    let prev = sequence[len - 2];
    let last_step = last - prev;
    match pattern {
        Pattern::Arithmetic { diff, .. } => {
            vec![last, last + diff + 1, last + diff - 1]
        }
        Pattern::Geometric { ratio, .. } => {
            vec![last, last + last_step, last * ratio + ratio]
        }
        Pattern::SecondDifference { .. } => {
            vec![last, last + last_step, last + last_step * 2]
        }
        Pattern::Fibonacci { .. } => {
            vec![last, last + prev / 2, prev + last * 2]
        }
    }
}

struct Question {
    sequence: Vec<i64>,
    choices: [i64; CHOICE_COUNT],
    correct_index: usize,
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let len: usize = if rng.gen_bool(0.5) { 4 } else { 5 };

    let pattern = match difficulty {
        Difficulty::Beginner => Pattern::Arithmetic {
            first: rng.gen_range(1..=20),
            diff: rng.gen_range(1..=9),
        },
        Difficulty::Intermediate => {
            if rng.gen_bool(0.5) {
                Pattern::Arithmetic {
                    first: rng.gen_range(1..=50),
                    diff: rng.gen_range(10..=30),
                }
            } else {
                Pattern::Geometric {
                    first: rng.gen_range(1..=5),
                    ratio: rng.gen_range(2..=3),
                }
            }
        }
        Difficulty::Advanced => {
            if rng.gen_bool(0.5) {
                Pattern::SecondDifference {
                    first: rng.gen_range(1..=10),
                    diff0: rng.gen_range(1..=5),
                    delta: rng.gen_range(1..=5),
                }
            } else {
                Pattern::Fibonacci {
                    a: rng.gen_range(1..=10),
                    b: rng.gen_range(1..=10),
                }
            }
        }
    };

    let terms = terms_for(pattern, len + 1);
    let sequence = terms[..len].to_vec();
    let answer = terms[len];

    let mut choices = vec![answer];
    for candidate in near_miss_candidates(pattern, &sequence) {
        if choices.len() >= CHOICE_COUNT {
            break;
        }
        if candidate != answer && !choices.contains(&candidate) {
            choices.push(candidate);
        }
    }
    while choices.len() < CHOICE_COUNT {
        let offset = rng.gen_range(-10..=10);
        let candidate = answer + offset;
        if offset != 0 && !choices.contains(&candidate) {
            choices.push(candidate);
        }
    }
    choices.shuffle(rng);
    let correct_index = choices.iter().position(|&c| c == answer).unwrap();

    Question {
        sequence,
        choices: choices.try_into().unwrap(),
        correct_index,
    }
}

pub struct SequenceGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
    /// 直前の回答の正誤表示(描画専用)
    feedback: AnswerFeedback,
}

impl SequenceGame {
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
        self.feedback.record(is_correct, format!("こたえ: {answer}"));
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

impl Game for SequenceGame {
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
        let (sequence_area, choices_area, footer_area) = split_areas(area);
        theme::render_hud(
            frame,
            hud_area,
            "数列予測",
            self.difficulty,
            self.tracker.total(),
            &self.feedback,
        );

        // 数列は項ごとに強調し、最後の「?」を注目色にする
        let term_style = Style::default()
            .fg(theme::ACCENT_STRONG)
            .add_modifier(Modifier::BOLD);
        let separator_style = Style::default().fg(theme::MUTED);
        let mut spans = Vec::new();
        for n in &self.current.sequence {
            spans.push(Span::styled(n.to_string(), term_style));
            spans.push(Span::styled("  →  ", separator_style));
        }
        spans.push(Span::styled(
            "?",
            Style::default()
                .fg(theme::HIGHLIGHT)
                .add_modifier(Modifier::BOLD),
        ));
        let sequence_paragraph = Paragraph::new(Line::from(spans))
            .alignment(Alignment::Center)
            .block(theme::focus_panel(" 次に来る数字は？ ", self.feedback.current()));
        frame.render_widget(sequence_paragraph, sequence_area);

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

    /// sequenceの後ろにanswerを足したものが等差数列になっているか
    fn is_arithmetic(full: &[i64]) -> bool {
        let diff = full[1] - full[0];
        full.windows(2).all(|w| w[1] - w[0] == diff)
    }

    /// sequenceの後ろにanswerを足したものが等比数列になっているか
    fn is_geometric(full: &[i64]) -> bool {
        let ratio = full[1] / full[0];
        full.windows(2).all(|w| w[0] != 0 && w[1] == w[0] * ratio)
    }

    /// 隣接する差自体が等差数列(二階差分が一定)になっているか
    fn is_second_difference_constant(full: &[i64]) -> bool {
        let diffs: Vec<i64> = full.windows(2).map(|w| w[1] - w[0]).collect();
        is_arithmetic(&diffs)
    }

    /// 直前2項の和が次項になっているか(フィボナッチ風)
    fn is_fibonacci_like(full: &[i64]) -> bool {
        full.windows(3).all(|w| w[2] == w[0] + w[1])
    }

    fn full_with_answer(q: &Question) -> Vec<i64> {
        let mut full = q.sequence.clone();
        full.push(q.choices[q.correct_index]);
        full
    }

    fn assert_choices_valid(q: &Question) {
        let mut sorted = q.choices.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), CHOICE_COUNT, "選択肢に重複がある");
        assert!(q.correct_index < CHOICE_COUNT);
    }

    #[test]
    fn beginner_is_arithmetic_sequence() {
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..200 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            assert!(q.sequence.len() == 4 || q.sequence.len() == 5);
            assert!(is_arithmetic(&full_with_answer(&q)));
            assert_choices_valid(&q);
        }
    }

    #[test]
    fn intermediate_is_arithmetic_or_geometric() {
        let mut rng = StdRng::seed_from_u64(2);
        let mut saw_arithmetic = false;
        let mut saw_geometric = false;
        for _ in 0..200 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            let full = full_with_answer(&q);
            let arithmetic = is_arithmetic(&full);
            let geometric = is_geometric(&full);
            assert!(
                arithmetic || geometric,
                "等差でも等比でもない数列が生成された: {:?}",
                full
            );
            saw_arithmetic |= arithmetic;
            saw_geometric |= geometric;
            assert_choices_valid(&q);
        }
        assert!(saw_arithmetic, "等差数列が一度も出題されなかった");
        assert!(saw_geometric, "等比数列が一度も出題されなかった");
    }

    #[test]
    fn advanced_is_second_difference_or_fibonacci() {
        let mut rng = StdRng::seed_from_u64(3);
        let mut saw_second_difference = false;
        let mut saw_fibonacci = false;
        for _ in 0..200 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            let full = full_with_answer(&q);
            let second_difference = is_second_difference_constant(&full);
            let fibonacci = is_fibonacci_like(&full);
            assert!(
                second_difference || fibonacci,
                "階差数列でもフィボナッチ風でもない数列が生成された: {:?}",
                full
            );
            saw_second_difference |= second_difference;
            saw_fibonacci |= fibonacci;
            assert_choices_valid(&q);
        }
        assert!(saw_second_difference, "階差数列が一度も出題されなかった");
        assert!(saw_fibonacci, "フィボナッチ風数列が一度も出題されなかった");
    }

    #[test]
    fn sequence_length_is_four_or_five() {
        let mut rng = StdRng::seed_from_u64(4);
        let mut saw_four = false;
        let mut saw_five = false;
        for _ in 0..50 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            match q.sequence.len() {
                4 => saw_four = true,
                5 => saw_five = true,
                other => panic!("想定外の数列長: {other}"),
            }
        }
        assert!(saw_four);
        assert!(saw_five);
    }

    #[test]
    fn advance_question_records_correct_answer() {
        let mut game = SequenceGame::new(Difficulty::Beginner);
        let answer = game.current.correct_index;
        game.advance_question(answer);
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn advance_question_records_incorrect_answer() {
        let mut game = SequenceGame::new(Difficulty::Beginner);
        let wrong = (game.current.correct_index + 1) % CHOICE_COUNT;
        game.advance_question(wrong);
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
        let mut game = SequenceGame::new(Difficulty::Beginner);
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
        let mut game = SequenceGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 15);
        let (sequence_area, _, _) = split_areas(area);
        game.handle_mouse(left_click(sequence_area.x, sequence_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn answering_shows_feedback_with_the_correct_answer() {
        let mut game = SequenceGame::new(Difficulty::Beginner);
        let answer = game.current.choices[game.current.correct_index];
        game.advance_question(game.current.correct_index);
        let flash = game.feedback.current().expect("回答直後は正誤を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Correct);
        assert_eq!(flash.detail, format!("こたえ: {answer}"));
        assert_eq!(game.feedback.streak(), 1);
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
    }

    /// 描画結果の各行を文字列にして返す
    fn rendered_rows(game: &SequenceGame, area: Rect) -> Vec<String> {
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
        let area = Rect::new(0, 0, 60, 30);
        let mut game = SequenceGame::new(Difficulty::Beginner);
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
