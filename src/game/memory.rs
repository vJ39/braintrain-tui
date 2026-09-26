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
use crate::game::mark_display::MarkRenderer;
use crate::game::theme;
use crate::game::{contains, Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "memory";

/// 正解時に大きく表示するオイカケ専用の画像(背景込みの不透明なイラスト)
const CORRECT_IMAGE_JPG: &[u8] = include_bytes!("../../assets/image/memory/correct.jpg");

/// 結果・HUDに出す難易度。問題が進むと手数が増えるため、最後の区間の上級を代表値にする
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;

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
/// 結果表示(◯/✗)の後、次の問題が始まる前に何も表示せず置く間
const BLANK_INTERVAL: Duration = Duration::from_millis(1000);

fn panel_color(panel: usize) -> Color {
    PANEL_COLORS
        .iter()
        .find(|&&(p, _)| p == panel)
        .map(|&(_, c)| c)
        .unwrap_or(Color::White)
}

/// 何問目(0始まり)のシーケンスの手数。
/// 1〜3問目=3手(初級相当)、4〜7問目=5手(中級相当)、8〜10問目=7手(上級相当)
fn sequence_len_for_question(question_index: u32) -> usize {
    match question_index {
        0..=2 => 3,
        3..=6 => 5,
        _ => 7,
    }
}

/// 何問目(0始まり)の、ランダムなパネル番号(1〜4)の並びを生成する
fn generate_sequence(rng: &mut impl Rng, question_index: u32) -> Vec<usize> {
    let len = sequence_len_for_question(question_index);
    (0..len).map(|_| rng.gen_range(1..=4)).collect()
}

/// 正誤の記号を出す時にパネルのエリアを塗る色。画像表示の時は画像の背景と周りのセルを
/// 同じ色で塗れるようRGBにし、テキスト表示の時は端末の名前付き色にする。
/// 記号は黒なので、黒が読みやすい明るさの緑(正解)/赤(不正解)にする
fn mark_background(is_correct: bool, uses_image: bool) -> Color {
    match (is_correct, uses_image) {
        (true, true) => Color::Rgb(0, 0, 0),
        (false, true) => Color::Rgb(230, 50, 50),
        (true, false) => Color::Green,
        (false, false) => Color::Red,
    }
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
    /// 結果表示の後、次の問題が始まる前の何も表示しない間
    Blank { elapsed: Duration },
}

pub struct MemoryGame {
    tracker: ScoreTracker,
    sequence: Vec<usize>,
    phase: Phase,
    /// 直近で光っている/入力されたパネル。描画用
    active_panel: Option<usize>,
    input_started_at: Instant,
    /// 正解数・連続正解のHUD表示用(描画専用)
    feedback: AnswerFeedback,
    /// 正誤確定後の大きな◯/✗の描画器(描画専用)
    mark_renderer: MarkRenderer,
}

impl Default for MemoryGame {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryGame {
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        let sequence = generate_sequence(&mut rng, 0);
        let active_panel = sequence.first().copied();
        audio::play_se(SeKind::Transition);
        Self {
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
            mark_renderer: MarkRenderer::with_correct_image(CORRECT_IMAGE_JPG),
        }
    }

    fn next_sequence(&mut self) {
        let mut rng = rand::thread_rng();
        // 記録済みの問題数が、次に出す問題の0始まりの番号になる
        self.sequence = generate_sequence(&mut rng, self.tracker.total());
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

    /// 2x2のパネルを描く(提示中・入力中)
    fn render_panels(&self, frame: &mut Frame, grid_area: Rect) {
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
                (
                    BorderType::Rounded,
                    theme::MUTED,
                    Style::default().fg(theme::MUTED),
                )
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
                    self.phase = Phase::Blank {
                        elapsed: Duration::ZERO,
                    };
                }
            }
            Phase::Blank { elapsed } => {
                *elapsed += dt;
                if *elapsed >= BLANK_INTERVAL {
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
            "オイカケ",
            SESSION_DIFFICULTY,
            self.tracker.total(),
            &self.feedback,
        );

        match self.phase {
            Phase::Interval { is_correct, .. } => {
                // 正誤確定後はパネルの代わりに、グリッドのエリアいっぱいに大きな◯/✗を出す
                let background = mark_background(is_correct, self.mark_renderer.uses_image());
                self.mark_renderer
                    .render(frame, grid_area, is_correct, background);
            }
            Phase::Blank { .. } => {
                // 次の問題が始まる前の何もない間。パネル・記号は出さず枠だけにする
                frame.render_widget(Block::default().borders(Borders::ALL), grid_area);
            }
            _ => self.render_panels(frame, grid_area),
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
            Phase::Blank { .. } => (String::new(), theme::MUTED),
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
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
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

    /// 何問目(0始まり)ごとの手数。3問(初級相当)→4問(中級相当)→3問(上級相当)
    const EXPECTED_LENGTHS: [usize; 10] = [3, 3, 3, 5, 5, 5, 5, 7, 7, 7];

    /// 提示フェーズを最後まで進め、現在のシーケンスを入力する。
    /// correct=falseなら最初の1手を間違えて即不正解にする
    fn play_current_sequence(game: &mut MemoryGame, correct: bool) {
        let len = game.sequence.len();
        for _ in 0..len {
            advance_one_step(game);
        }
        assert!(matches!(game.phase, Phase::Input { entered: 0 }));
        if !correct {
            let wrong_first = if game.sequence[0] == 1 { 2 } else { 1 };
            game.handle_key(KeyEvent::from(KeyCode::Char(
                std::char::from_digit(wrong_first as u32, 10).unwrap(),
            )));
            return;
        }
        let sequence = game.sequence.clone();
        for &panel in &sequence {
            game.handle_key(KeyEvent::from(KeyCode::Char(
                std::char::from_digit(panel as u32, 10).unwrap(),
            )));
        }
    }

    #[test]
    fn sequence_len_follows_three_four_three_questions() {
        for (index, expected) in EXPECTED_LENGTHS.iter().enumerate() {
            assert_eq!(
                sequence_len_for_question(index as u32),
                *expected,
                "{}問目",
                index + 1
            );
        }
    }

    #[test]
    fn generated_sequence_has_expected_length_per_question() {
        let mut rng = StdRng::seed_from_u64(1);
        for (index, expected) in EXPECTED_LENGTHS.iter().enumerate() {
            assert_eq!(
                generate_sequence(&mut rng, index as u32).len(),
                *expected,
                "{}問目",
                index + 1
            );
        }
    }

    #[test]
    fn generated_sequence_only_contains_valid_panel_numbers() {
        let mut rng = StdRng::seed_from_u64(2);
        for _ in 0..50 {
            let seq = generate_sequence(&mut rng, 9);
            for &p in &seq {
                assert!((1..=4).contains(&p));
            }
        }
    }

    #[test]
    fn session_sequences_follow_fixed_progression() {
        // 正解・不正解に関わらず、何問目かだけで手数が決まる
        let mut game = MemoryGame::new();
        for (index, expected) in EXPECTED_LENGTHS.iter().enumerate() {
            assert_eq!(game.sequence.len(), *expected, "{}問目の手数", index + 1);
            play_current_sequence(&mut game, index % 2 == 0);
            assert_eq!(game.tracker.total(), index as u32 + 1);
            if !game.is_finished() {
                game.update(RESULT_INTERVAL);
                game.update(BLANK_INTERVAL);
            }
        }
        assert!(game.is_finished(), "10問で終わる");
        assert_eq!(game.result().total, 10);
    }

    #[test]
    fn result_records_session_difficulty() {
        assert_eq!(SESSION_DIFFICULTY, Difficulty::Advanced);
        let mut game = MemoryGame::new();
        assert_eq!(game.result().difficulty, SESSION_DIFFICULTY);
        play_current_sequence(&mut game, true);
        assert_eq!(game.result().difficulty, SESSION_DIFFICULTY);
    }

    #[test]
    fn showing_phase_advances_to_input_after_all_steps_shown() {
        let mut game = MemoryGame::new();
        let len = game.sequence.len();
        for _ in 0..len {
            advance_one_step(&mut game);
        }
        assert!(matches!(game.phase, Phase::Input { entered: 0 }));
    }

    #[test]
    fn showing_phase_does_not_advance_before_interval_elapses() {
        let mut game = MemoryGame::new();
        game.update(SHOW_ON_DURATION / 2);
        assert!(matches!(game.phase, Phase::Showing { shown: 0, .. }));
    }

    #[test]
    fn consecutive_same_panel_blinks_off_between_repeats() {
        // 同じパネルが連続するシーケンスでも、1回の点灯なのか連続点灯なのか
        // 区別できるよう、次の点灯前に必ず一度消灯を経由することを確認する
        let mut game = MemoryGame::new();
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
        let mut game = MemoryGame::new();
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
        let mut game = MemoryGame::new();
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
    fn interval_advances_to_blank_after_result_interval() {
        let mut game = MemoryGame::new();
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

        // インターバル時間が経過するまでは次のフェーズへ進まない
        game.update(RESULT_INTERVAL - Duration::from_millis(1));
        assert!(matches!(game.phase, Phase::Interval { .. }));

        // インターバル時間が経過すると、何も表示しない間(Blank)に移る
        // (次の問題がすぐ始まらないよう、◯✗表示の後にもう1秒間を置く)
        game.update(Duration::from_millis(1));
        assert!(matches!(game.phase, Phase::Blank { .. }));
    }

    #[test]
    fn blank_advances_to_next_sequence_after_blank_interval() {
        let mut game = MemoryGame::new();
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
        game.update(RESULT_INTERVAL);
        assert!(matches!(game.phase, Phase::Blank { .. }));

        // 空白の時間が経過するまでは次のシーケンスへ進まない
        game.update(BLANK_INTERVAL - Duration::from_millis(1));
        assert!(matches!(game.phase, Phase::Blank { .. }));

        // 空白の時間が経過すると次のシーケンス(提示フェーズ)が始まる
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
        let mut game = MemoryGame::new();
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
        let mut game = MemoryGame::new();
        game.handle_key(KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn session_finishes_after_configured_question_count() {
        let mut game = MemoryGame::new();
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
            // 正誤確定後のインターバル・空白時間を経過させて次のシーケンスへ進める
            if !game.tracker.is_session_finished() {
                game.update(RESULT_INTERVAL);
                game.update(BLANK_INTERVAL);
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
        let mut game = MemoryGame::new();
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
        let result = game.tracker.to_result(GAME_ID, SESSION_DIFFICULTY);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_panel_during_showing_phase_is_ignored() {
        let mut game = MemoryGame::new();
        let area = Rect::new(0, 0, 40, 12);
        let (grid_area, _) = split_areas(area);
        let (_, panel_area) = panel_areas(grid_area)[0];
        game.handle_mouse(left_click(panel_area.x, panel_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }

    // ---- 正誤確定後の大きな◯/✗表示 ----

    use crate::game::mark_display::{MarkRenderer, CORRECT_MARK, INCORRECT_MARK};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;
    use ratatui_image::picker::{Picker, ProtocolType};

    /// ゲームを描き、バッファとパネルの2x2グリッドのエリアを返す
    fn render_game(game: &MemoryGame, width: u16, height: u16) -> (Buffer, Rect) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        let (grid_area, _) = split_areas(Rect::new(0, 0, width, height));
        (terminal.backend().buffer().clone(), grid_area)
    }

    fn area_text(buffer: &Buffer, area: Rect) -> String {
        (area.y..area.bottom())
            .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
            .map(|pos| buffer[pos].symbol().to_string())
            .collect()
    }

    fn halfblocks_renderer() -> MarkRenderer {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        MarkRenderer::with_picker_and_correct_image(Some(picker), CORRECT_IMAGE_JPG)
    }

    #[test]
    fn correct_image_is_embedded_and_decodable() {
        let image = image::load_from_memory(CORRECT_IMAGE_JPG)
            .expect("オイカケの正解画像が埋め込まれ、読み込めること");
        assert_eq!((image.width(), image.height()), (512, 512));
    }

    #[test]
    fn game_uses_own_correct_image_and_common_incorrect_image() {
        let game = MemoryGame::new();
        assert!(
            game.mark_renderer.mark_image_bytes(true) == CORRECT_IMAGE_JPG,
            "正解時はオイカケ専用の画像"
        );
        let common = MarkRenderer::with_picker(None);
        assert!(
            game.mark_renderer.mark_image_bytes(false) == common.mark_image_bytes(false),
            "不正解時は全ゲーム共通の✗"
        );
        assert!(
            game.mark_renderer.mark_image_bytes(true) != common.mark_image_bytes(true),
            "正解時は共通の◯を使わない"
        );
    }

    #[test]
    fn correct_interval_is_drawn_with_own_image_colors() {
        // 共通の◯は黒と背景色(緑)を混ぜた色だけで描かれ、どの色も緑の成分が赤・青より大きい。
        // オイカケの画像はイラストなので、赤や青が緑より強い色が一定数含まれる
        let mut game = MemoryGame::new();
        game.mark_renderer = halfblocks_renderer();
        play_current_sequence(&mut game, true);
        let (buffer, _) = render_game(&game, 60, 20);
        let (cached_correct, _, drawn) = game
            .mark_renderer
            .cached_mark()
            .expect("画像で描かれていること");
        assert!(cached_correct);
        let colors: Vec<Color> = (drawn.y..drawn.bottom())
            .flat_map(|y| (drawn.x..drawn.right()).map(move |x| (x, y)))
            .flat_map(|pos| [buffer[pos].fg, buffer[pos].bg])
            .collect();
        let not_greenish = colors
            .iter()
            .filter(|color| match color {
                Color::Rgb(r, g, b) => u16::from(*r).max(u16::from(*b)) > u16::from(*g) + 30,
                _ => false,
            })
            .count();
        assert!(
            not_greenish * 10 > colors.len(),
            "イラストの色で描かれる: {not_greenish}/{}",
            colors.len()
        );
    }

    #[test]
    fn interval_shows_big_mark_in_grid_area_instead_of_panels() {
        for (correct, mark, other) in [
            (true, CORRECT_MARK, INCORRECT_MARK),
            (false, INCORRECT_MARK, CORRECT_MARK),
        ] {
            let mut game = MemoryGame::new();
            play_current_sequence(&mut game, correct);
            assert!(matches!(
                game.phase,
                Phase::Interval { is_correct, .. } if is_correct == correct
            ));
            let (buffer, grid_area) = render_game(&game, 40, 16);
            let text = area_text(&buffer, grid_area);
            assert!(text.contains(mark), "「{mark}」が描かれる: {text:?}");
            assert!(!text.contains(other), "反対の記号は描かない");
            // パネルの番号は隠れ、◯/✗がメインの表示になる
            for number in ['1', '2', '3', '4'] {
                assert!(!text.contains(number), "パネル{number}は描かない");
            }
            let cell = (grid_area.y..grid_area.bottom())
                .flat_map(|y| (grid_area.x..grid_area.right()).map(move |x| (x, y)))
                .map(|pos| &buffer[pos])
                .find(|c| c.symbol() == mark)
                .unwrap();
            assert_eq!(cell.fg, Color::Black, "記号は黒");
            // グリッドのエリア全体を正誤の色で塗る
            let expected_bg = mark_background(correct, false);
            assert_eq!(buffer[(grid_area.x, grid_area.y)].bg, expected_bg);
            assert_eq!(
                buffer[(grid_area.right() - 1, grid_area.bottom() - 1)].bg,
                expected_bg
            );
        }
    }

    #[test]
    fn mark_backgrounds_differ_between_correct_and_incorrect() {
        assert_ne!(mark_background(true, false), mark_background(false, false));
        assert_ne!(mark_background(true, true), mark_background(false, true));
        // 画像表示の時は画像の背景と同じ色で塗れるようRGBにする
        assert!(matches!(mark_background(true, true), Color::Rgb(..)));
        assert!(matches!(mark_background(false, true), Color::Rgb(..)));
    }

    #[test]
    fn interval_footer_keeps_supplementary_text() {
        let mut game = MemoryGame::new();
        play_current_sequence(&mut game, true);
        let (buffer, _) = render_game(&game, 60, 16);
        let text: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(
            text.replace(' ', "").contains("せいかい"),
            "footerに補足を残す"
        );
    }

    #[test]
    fn mark_disappears_after_interval_and_panels_return() {
        let mut game = MemoryGame::new();
        play_current_sequence(&mut game, true);
        game.update(RESULT_INTERVAL);
        // ◯✗表示が終わった直後は、次の問題が始まる前の何もない間(Blank)。
        // マークもパネルもまだ出さない
        assert!(matches!(game.phase, Phase::Blank { .. }));
        let (buffer, grid_area) = render_game(&game, 40, 16);
        let text = area_text(&buffer, grid_area);
        assert!(!text.contains(CORRECT_MARK) && !text.contains(INCORRECT_MARK));
        for number in ['1', '2', '3', '4'] {
            assert!(!text.contains(number), "空白の間はパネル{number}も出さない");
        }

        // 空白の時間が経つと、次の問題の提示(パネル)に戻る
        game.update(BLANK_INTERVAL);
        assert!(matches!(game.phase, Phase::Showing { shown: 0, .. }));
        let (buffer, grid_area) = render_game(&game, 40, 16);
        let text = area_text(&buffer, grid_area);
        assert!(!text.contains(CORRECT_MARK) && !text.contains(INCORRECT_MARK));
        for number in ['1', '2', '3', '4'] {
            assert!(text.contains(number), "パネル{number}が戻る");
        }
    }

    #[test]
    fn no_mark_is_shown_outside_interval() {
        let mut game = MemoryGame::new();
        let (buffer, grid_area) = render_game(&game, 40, 16);
        let text = area_text(&buffer, grid_area);
        assert!(
            !text.contains(CORRECT_MARK) && !text.contains(INCORRECT_MARK),
            "提示中"
        );
        let len = game.sequence.len();
        for _ in 0..len {
            advance_one_step(&mut game);
        }
        let (buffer, grid_area) = render_game(&game, 40, 16);
        let text = area_text(&buffer, grid_area);
        assert!(
            !text.contains(CORRECT_MARK) && !text.contains(INCORRECT_MARK),
            "入力中"
        );
    }

    #[test]
    fn interval_mark_is_drawn_as_image_with_image_protocol() {
        for correct in [true, false] {
            let mut game = MemoryGame::new();
            game.mark_renderer = halfblocks_renderer();
            play_current_sequence(&mut game, correct);
            let (buffer, grid_area) = render_game(&game, 60, 20);
            let (cached_correct, bg, drawn) = game
                .mark_renderer
                .cached_mark()
                .expect("画像で描かれていること");
            assert_eq!(cached_correct, correct);
            let expected_bg = mark_background(correct, true);
            assert_eq!(Color::Rgb(bg[0], bg[1], bg[2]), expected_bg);
            assert!(drawn.x >= grid_area.x && drawn.y >= grid_area.y);
            assert!(drawn.right() <= grid_area.right() && drawn.bottom() <= grid_area.bottom());
            assert_eq!(buffer[(grid_area.x, grid_area.y)].bg, expected_bg);
        }
    }

    #[test]
    fn interval_render_does_not_panic_on_small_screens_or_image_protocols() {
        for protocol in [
            ProtocolType::Halfblocks,
            ProtocolType::Sixel,
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
        ] {
            for correct in [true, false] {
                let mut picker = Picker::from_fontsize((10, 20));
                picker.set_protocol_type(protocol);
                let mut game = MemoryGame::new();
                game.mark_renderer =
                    MarkRenderer::with_picker_and_correct_image(Some(picker), CORRECT_IMAGE_JPG);
                play_current_sequence(&mut game, correct);
                for (width, height) in [(60u16, 20u16), (12, 9), (4, 4), (1, 1)] {
                    render_game(&game, width, height);
                }
            }
        }
        let mut game = MemoryGame::new();
        play_current_sequence(&mut game, false);
        render_game(&game, 1, 1);
    }
}
