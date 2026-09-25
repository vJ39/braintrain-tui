use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::theme;
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
    /// 直前の回答の正誤表示(描画専用)
    feedback: AnswerFeedback,
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
            feedback: AnswerFeedback::new(),
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
        let answer = if self.current.is_match {
            "一致"
        } else {
            "不一致"
        };
        self.feedback.record(is_correct, format!("こたえ: {answer}"));
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
        self.feedback.tick(dt);
        self.elapsed_in_question += dt;
        if let Some(limit) = time_limit(self.difficulty) {
            if self.elapsed_in_question >= limit {
                self.advance_question(false);
            }
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (label_area, footer_area) = split_areas(area);
        // HUDはクリック判定の無いラベルエリアの上端から切り出す(フッターの位置は変えない)
        let (hud_area, label_area) = theme::split_hud(label_area);
        theme::render_hud(
            frame,
            hud_area,
            "イロピッタン",
            self.difficulty,
            self.tracker.total(),
            &self.feedback,
        );

        let vertical_padding = label_area.height.saturating_sub(3) / 2;
        let mut lines: Vec<Line> = (0..vertical_padding).map(|_| Line::from("")).collect();
        let label_style = Style::default()
            .fg(self.current.display_color)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD);
        // 文字の両側を空けて、色の付いた文字が目に入りやすいようにする
        lines.push(Line::from(Span::styled(
            format!("  {}  ", self.current.label),
            label_style,
        )));

        // 枠の色は出題の文字色そのもの(文字色を判断する手がかりの一部なので変えない)
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(self.current.display_color))
            .title(Line::from(" この文字色と文字の意味は一致？ ").style(theme::title_style()));
        if let Some(limit) = time_limit(self.difficulty) {
            block = block.title_bottom(time_limit_line(self.elapsed_in_question, limit).centered());
        }

        let label_paragraph = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(Style::default().bg(Color::Black))
            .block(block);
        frame.render_widget(label_paragraph, label_area);

        // フッターはcolumn_index(2列)と同じ分割の2ボタン
        theme::render_choice_buttons(frame, footer_area, &[("←", "一致"), ("→", "不一致")]);
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

/// 残り時間の割合に応じた色。残り2/3超=通常、1/3超=注意、それ以下=警告
fn time_limit_color(remaining: Duration, limit: Duration) -> Color {
    if remaining <= limit / 3 {
        theme::INCORRECT
    } else if remaining <= limit * 2 / 3 {
        theme::HIGHLIGHT
    } else {
        theme::ACCENT_STRONG
    }
}

/// 制限時間付きの難易度で、残り時間をバーと秒数で示す1行
fn time_limit_line(elapsed: Duration, limit: Duration) -> Line<'static> {
    const BAR_WIDTH: usize = 12;
    let remaining = limit.saturating_sub(elapsed);
    let color = time_limit_color(remaining, limit);
    let bar = theme::progress_bar(
        remaining.as_millis() as u32,
        limit.as_millis() as u32,
        BAR_WIDTH,
    );
    Line::from(vec![
        Span::styled(" 残り ", Style::default().fg(theme::MUTED)),
        Span::styled(bar, Style::default().fg(color)),
        Span::styled(
            format!(" {:.1}秒 ", remaining.as_secs_f64()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn time_limit_color_turns_warning_as_time_runs_out() {
        let limit = Duration::from_secs(3);
        assert_eq!(
            time_limit_color(Duration::from_secs(3), limit),
            theme::ACCENT_STRONG
        );
        assert_eq!(
            time_limit_color(Duration::from_millis(1500), limit),
            theme::HIGHLIGHT
        );
        assert_eq!(
            time_limit_color(Duration::from_millis(500), limit),
            theme::INCORRECT
        );
        assert_eq!(time_limit_color(Duration::ZERO, limit), theme::INCORRECT);
    }

    #[test]
    fn answering_shows_feedback_with_the_correct_answer() {
        let mut game = ReactionGame::new(Difficulty::Beginner);
        game.current.is_match = true;
        // 一致の問題に「不一致」(→)と答える
        game.handle_key(KeyEvent::from(KeyCode::Right));
        let flash = game.feedback.current().expect("回答直後は正誤を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert_eq!(flash.detail, "こたえ: 一致");
    }

    #[test]
    fn feedback_disappears_after_hold_time() {
        let mut game = ReactionGame::new(Difficulty::Beginner);
        let is_match = game.current.is_match;
        game.handle_key(KeyEvent::from(if is_match {
            KeyCode::Left
        } else {
            KeyCode::Right
        }));
        assert!(game.feedback.current().is_some());
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
        // 正解数の表示は消えない
        assert_eq!(game.feedback.correct(), 1);
    }

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
