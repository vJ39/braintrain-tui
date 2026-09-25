//! カラーストック: 1列に積まれた色ブロックを、色ボタンで「一番下」から消していくタイムアタック。
//!
//! 押した色が最下段と一致すれば最下段のブロックが消え、上のブロックが1段下に詰まる。
//! 一致しなければ(ミス)、押した色のブロックが列の一番上に1個追加される。
//! 列を空にしたらラウンドクリア。1セッション=3ラウンドで、制限時間を超えたラウンドは失敗として記録する。

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::theme;
use crate::game::{column_index, contains, Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "color_stack";

/// 1セッションのラウンド数
pub const ROUNDS_PER_SESSION: u32 = 3;

/// ラウンド終了から次のラウンド開始までの間隔。この間は入力を受け付けない
pub const ROUND_INTERVAL: Duration = Duration::from_millis(1200);

/// 色ボタンを並べる領域の高さ(枠込み)
const BUTTONS_HEIGHT: u16 = 3;

/// マスの幅の上限。広い画面でもブロックが横に伸びすぎないようにする
const MAX_CELL_WIDTH: u16 = 16;

/// 1段の高さの上限。低い積み上げでも縦に伸びすぎないようにする
const MAX_ROW_HEIGHT: u16 = 2;

/// ブロックの色。並び順がボタンの並び・数字キー(1始まり)に対応する
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StackColor {
    Red,
    Blue,
    Yellow,
    Green,
}

impl StackColor {
    pub const ALL: [StackColor; 4] = [
        StackColor::Red,
        StackColor::Blue,
        StackColor::Yellow,
        StackColor::Green,
    ];

    pub fn name(self) -> &'static str {
        match self {
            StackColor::Red => "赤",
            StackColor::Blue => "青",
            StackColor::Yellow => "黄",
            StackColor::Green => "緑",
        }
    }

    pub fn color(self) -> Color {
        match self {
            StackColor::Red => Color::Red,
            StackColor::Blue => Color::Blue,
            StackColor::Yellow => Color::Yellow,
            StackColor::Green => Color::Green,
        }
    }
}

/// 難易度ごとのパラメータ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DifficultyParams {
    /// 列の初期の高さ(ブロック数)
    pub height: usize,
    /// 使う色の数(StackColor::ALLの先頭から)
    pub color_count: usize,
    /// 1ラウンドの制限時間
    pub time_limit: Duration,
}

pub fn params(difficulty: Difficulty) -> DifficultyParams {
    match difficulty {
        Difficulty::Beginner => DifficultyParams {
            height: 8,
            color_count: 3,
            time_limit: Duration::from_secs(60),
        },
        Difficulty::Intermediate => DifficultyParams {
            height: 12,
            color_count: 4,
            time_limit: Duration::from_secs(90),
        },
        Difficulty::Advanced => DifficultyParams {
            height: 16,
            color_count: 4,
            time_limit: Duration::from_secs(120),
        },
    }
}

impl DifficultyParams {
    /// このラウンドで使う色(ボタンの並び順)
    fn colors(&self) -> &'static [StackColor] {
        &StackColor::ALL[..self.color_count]
    }
}

/// 盤面。1列に積まれたブロックで、index 0が最下段
type Board = Vec<StackColor>;

/// 初期の高さまでブロックを積んだ1列の盤面を作る。
/// 先頭の色数ぶんのセルに各色を1個ずつ置き、残りをランダムにしてから全体をシャッフルするので、
/// 使う色はどれも最低1個は含まれる
fn new_board(rng: &mut impl Rng, params: &DifficultyParams) -> Board {
    let colors = params.colors();
    let mut cells: Board = (0..params.height)
        .map(|i| match colors.get(i) {
            Some(&color) => color,
            None => colors[rng.gen_range(0..colors.len())],
        })
        .collect();
    cells.shuffle(rng);
    cells
}

/// 色ボタンを押した結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressOutcome {
    /// 最下段と一致して消えた
    Removed,
    /// 一致せず、押した色が一番上に追加された(ミス)
    Added,
}

/// colorを押した時の処理。最下段がcolorなら消して詰め、違えば一番上にcolorを1個積む
fn press_color(board: &mut Board, color: StackColor) -> PressOutcome {
    if board.first() == Some(&color) {
        // 最下段を取り除くと、残りのブロックは1段ずつ下にずれる(重力)
        board.remove(0);
        PressOutcome::Removed
    } else {
        board.push(color);
        PressOutcome::Added
    }
}

/// 列が空か
fn is_cleared(board: &Board) -> bool {
    board.is_empty()
}

/// 盤面を描くマス目の位置。renderと(テストでの)位置確認で共有する
#[derive(Debug, Clone, Copy, PartialEq)]
struct GridGeometry {
    /// マスの左端
    origin_x: u16,
    /// 最下段のマスの下端(この行は含まない)
    bottom: u16,
    /// マスの幅
    cell_width: u16,
    /// マスの高さ
    row_height: u16,
    /// 描ける段数(下から数えて)
    visible_rows: usize,
}

impl GridGeometry {
    /// 下からlevel段目(0=最下段)のマス。描けない段ならNone
    fn cell_rect(&self, level: usize) -> Option<Rect> {
        if level >= self.visible_rows || self.cell_width == 0 {
            return None;
        }
        let y = self.bottom - (level as u16 + 1) * self.row_height;
        Some(Rect::new(
            self.origin_x,
            y,
            self.cell_width,
            self.row_height,
        ))
    }
}

/// board(盤面パネルの内側)に1列のマス目を置く。
/// 横は中央寄せ、縦は最下段を盤面の下端にそろえる。
/// 1段の高さは初期の高さ(base_height)が収まるように決め、ミスで列が伸びても変えない。
/// 描ける段数は盤面に入るぶんだけで、入りきらない上の段は描かない
fn grid_geometry(board: Rect, base_height: usize) -> GridGeometry {
    let cell_width = board.width.min(MAX_CELL_WIDTH);
    let base_height = u16::try_from(base_height.max(1)).unwrap_or(u16::MAX);
    let row_height = (board.height / base_height).clamp(1, MAX_ROW_HEIGHT);
    let visible_rows = if cell_width == 0 {
        0
    } else {
        (board.height / row_height) as usize
    };
    GridGeometry {
        origin_x: board.x + (board.width - cell_width) / 2,
        bottom: board.bottom(),
        cell_width,
        row_height,
        visible_rows,
    }
}

/// ゲームの描画エリアを「HUD」「盤面パネル」「色ボタン」に分ける
fn split_areas(area: Rect) -> (Rect, Rect, Rect) {
    let (hud, body) = theme::split_hud(area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(BUTTONS_HEIGHT)])
        .split(body);
    (hud, rows[0], rows[1])
}

/// ゲームの描画エリアのうち、盤面パネルの内側
fn board_area(area: Rect) -> Rect {
    let (_, panel, _) = split_areas(area);
    Block::default().borders(Borders::ALL).inner(panel)
}

/// ゲームの描画エリアのうち、色ボタンを並べる領域
fn buttons_area(area: Rect) -> Rect {
    split_areas(area).2
}

/// 1ラウンドの状態
struct Round {
    board: Board,
    /// ラウンド開始からの経過時間
    elapsed: Duration,
}

fn new_round(params: &DifficultyParams) -> Round {
    Round {
        board: new_board(&mut rand::thread_rng(), params),
        elapsed: Duration::ZERO,
    }
}

pub struct ColorStackGame {
    difficulty: Difficulty,
    params: DifficultyParams,
    round: Round,
    /// ラウンド間の待ち時間の残り。Noneならプレイ中
    interval: Option<Duration>,
    tracker: ScoreTracker,
    feedback: AnswerFeedback,
}

impl ColorStackGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let params = params(difficulty);
        Self {
            difficulty,
            params,
            round: new_round(&params),
            interval: None,
            tracker: ScoreTracker::new(),
            feedback: AnswerFeedback::new(),
        }
    }

    /// プレイ中(待ち時間中でもセッション終了後でもない)か
    fn is_playing(&self) -> bool {
        !self.is_finished() && self.interval.is_none()
    }

    /// index番目(0始まり)の色ボタンを押す。使わない色の番号やプレイ中以外は無視する
    fn press_button(&mut self, index: usize) {
        if !self.is_playing() {
            return;
        }
        let Some(&color) = self.params.colors().get(index) else {
            return;
        };
        if press_color(&mut self.round.board, color) == PressOutcome::Added {
            // ミス: 押した色が一番上に積まれた(ラウンドは続き、スコアには記録しない)
            audio::play_se(SeKind::Incorrect);
            self.feedback.record(false, "+1段");
            return;
        }
        audio::play_se(SeKind::Correct);
        if is_cleared(&self.round.board) {
            let seconds = self.round.elapsed.as_secs_f64();
            self.tracker.record(true, seconds * 1000.0);
            self.feedback.record(true, format!("CLEAR {seconds:.1}秒"));
            self.finish_round();
        }
    }

    /// 制限時間を超えたラウンドを失敗として終える
    fn time_up(&mut self) {
        audio::play_se(SeKind::Incorrect);
        self.tracker
            .record(false, self.params.time_limit.as_secs_f64() * 1000.0);
        self.feedback.record(false, "時間切れ");
        self.finish_round();
    }

    /// ラウンドを終える。最終ラウンドでなければ次のラウンドまでの待ち時間に入る
    fn finish_round(&mut self) {
        if !self.is_finished() {
            self.interval = Some(ROUND_INTERVAL);
        }
    }

    fn start_next_round(&mut self) {
        self.round = new_round(&self.params);
        self.interval = None;
    }

    /// いま何ラウンド目か(1始まり。全ラウンド終了後は最終ラウンドのまま)
    fn current_round_number(&self) -> u32 {
        (self.tracker.total() + u32::from(self.is_playing())).clamp(1, ROUNDS_PER_SESSION)
    }

    fn render_hud(&self, frame: &mut Frame, area: Rect) {
        let (difficulty_text, difficulty_color) = theme::difficulty_label(self.difficulty);
        let block = theme::panel(" ◆ カラーストック ")
            .border_style(Style::default().fg(theme::flash_border_color(self.feedback.current())))
            .title(
                Line::from(Span::styled(
                    format!(" {difficulty_text} "),
                    Style::default()
                        .fg(difficulty_color)
                        .add_modifier(Modifier::BOLD),
                ))
                .right_aligned(),
            );
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(20),
                Constraint::Fill(1),
                Constraint::Length(16),
            ])
            .split(inner);

        let progress = Line::from(vec![
            Span::styled(
                format!(
                    " ROUND {}/{ROUNDS_PER_SESSION} ",
                    self.current_round_number()
                ),
                theme::title_style(),
            ),
            Span::styled(
                theme::progress_bar(
                    self.tracker.total(),
                    ROUNDS_PER_SESSION,
                    ROUNDS_PER_SESSION as usize,
                ),
                Style::default().fg(theme::ACCENT),
            ),
        ]);
        frame.render_widget(Paragraph::new(progress), cols[0]);

        // 中央: 正誤表示中はそれを、プレイ中は経過時間と制限時間を出す
        let center = match self.feedback.current() {
            Some(flash) => theme::flash_line(flash),
            None if self.is_playing() => Line::from(vec![
                Span::styled("経過 ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    format!("{:.1}秒", self.round.elapsed.as_secs_f64()),
                    Style::default()
                        .fg(theme::HIGHLIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" / {}秒", self.params.time_limit.as_secs()),
                    Style::default().fg(theme::MUTED),
                ),
            ]),
            None => Line::from(""),
        };
        frame.render_widget(Paragraph::new(center).alignment(Alignment::Center), cols[1]);

        let remaining = Line::from(vec![
            Span::styled("残り ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!("{}個", self.round.board.len()),
                Style::default()
                    .fg(theme::ACCENT_STRONG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]);
        frame.render_widget(
            Paragraph::new(remaining).alignment(Alignment::Right),
            cols[2],
        );
    }

    /// 盤面のブロックを描く。最下段のブロックには対応する数字キーを重ねて、押す色を分かりやすくする
    fn render_board(&self, frame: &mut Frame, board: Rect) {
        let grid = grid_geometry(board, self.params.height);
        for (level, color) in self.round.board.iter().enumerate() {
            let Some(rect) = grid.cell_rect(level) else {
                // 入りきらない上の段は描かない
                break;
            };
            let style = Style::default().bg(color.color());
            frame.render_widget(Block::default().style(style), rect);
            // 2行以上で描ける時は下の行に線を引き、同じ色が縦に続いても1個ずつ見分けられるようにする
            if rect.height >= 2 {
                let separator =
                    Paragraph::new("▁".repeat(rect.width as usize)).style(style.fg(Color::Black));
                frame.render_widget(
                    separator,
                    Rect::new(rect.x, rect.bottom() - 1, rect.width, 1),
                );
            }
            if level == 0 {
                let key = self.button_number(*color);
                let label = Paragraph::new(Span::styled(
                    key.to_string(),
                    Style::default()
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                ))
                .alignment(Alignment::Center)
                .style(style);
                frame.render_widget(label, theme::vertical_center(rect, 1));
            }
        }
    }

    /// colorのボタン番号(数字キー。1始まり)
    fn button_number(&self, color: StackColor) -> usize {
        self.params
            .colors()
            .iter()
            .position(|&c| c == color)
            .map_or(0, |i| i + 1)
    }

    /// 色ボタンを横に等分して描く。分割はクリック判定のcolumn_indexと同じcolumn_bands
    fn render_buttons(&self, frame: &mut Frame, area: Rect) {
        let colors = self.params.colors();
        for (index, (band, color)) in theme::column_bands(area, colors.len() as u16)
            .into_iter()
            .zip(colors)
            .enumerate()
        {
            let line = Line::from(vec![
                Span::styled(
                    format!(" {} ", index + 1),
                    Style::default()
                        .fg(Color::Black)
                        .bg(color.color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {}", color.name()),
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            let paragraph = Paragraph::new(line)
                .alignment(Alignment::Center)
                .block(theme::sub_panel().border_style(Style::default().fg(color.color())));
            frame.render_widget(paragraph, band);
        }
    }

    /// ラウンド間の待ち時間中に盤面中央へ出す案内
    fn render_interval_message(&self, frame: &mut Frame, board: Rect) {
        // 列を消し切っていればクリア、そうでなければ時間切れ
        let (headline, color) = if is_cleared(&self.round.board) {
            ("CLEAR!", theme::CORRECT)
        } else {
            ("TIME UP", theme::INCORRECT)
        };
        let next_round = (self.tracker.total() + 1).min(ROUNDS_PER_SESSION);
        let lines = vec![
            Line::from(Span::styled(
                headline,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("NEXT ROUND {next_round}/{ROUNDS_PER_SESSION}"),
                theme::title_style(),
            )),
        ];
        let area = theme::vertical_center(board, lines.len() as u16);
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
    }
}

impl Game for ColorStackGame {
    /// 数字キー(1〜色数)で色ボタンを押す
    fn handle_key(&mut self, key: KeyEvent) {
        let KeyCode::Char(c) = key.code else {
            return;
        };
        if let Some(index) = c.to_digit(10).and_then(|d| (d as usize).checked_sub(1)) {
            self.press_button(index);
        }
    }

    /// 画面下部の色ボタンのクリックで色を押す。盤面のクリックは何もしない
    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let buttons = buttons_area(area);
        if !contains(buttons, mouse.column, mouse.row) {
            return;
        }
        let count = self.params.colors().len() as u16;
        if let Some(index) = column_index(buttons, mouse.column, count) {
            self.press_button(index);
        }
    }

    fn update(&mut self, dt: Duration) {
        self.feedback.tick(dt);
        if let Some(remaining) = self.interval {
            let remaining = remaining.saturating_sub(dt);
            if remaining.is_zero() {
                self.start_next_round();
            } else {
                self.interval = Some(remaining);
            }
            return;
        }
        if self.is_finished() {
            return;
        }
        let limit = self.params.time_limit;
        self.round.elapsed = (self.round.elapsed + dt).min(limit);
        if self.round.elapsed >= limit {
            self.time_up();
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud, panel, buttons) = split_areas(area);
        self.render_hud(frame, hud);

        let block = theme::focus_panel(" 一番下の色を押して全部消す ", self.feedback.current());
        frame.render_widget(block, panel);
        // focus_panelは1セル枠なので、内側はboard_area(テストでの位置確認と共有)と一致する
        let board = board_area(area);
        if !board.is_empty() {
            if self.interval.is_some() {
                self.render_interval_message(frame, board);
            } else {
                self.render_board(frame, board);
            }
        }
        self.render_buttons(frame, buttons);
    }

    fn is_finished(&self) -> bool {
        self.tracker.total() >= ROUNDS_PER_SESSION
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use std::collections::HashSet;
    use StackColor::{Blue, Green, Red, Yellow};

    const AREA: Rect = Rect::new(0, 0, 100, 36);

    const ALL_DIFFICULTIES: [Difficulty; 3] = [
        Difficulty::Beginner,
        Difficulty::Intermediate,
        Difficulty::Advanced,
    ];

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn press_key(game: &mut ColorStackGame, c: char) {
        game.handle_key(KeyEvent::from(KeyCode::Char(c)));
    }

    /// 色colorに対応する数字キー
    fn key_for(color: StackColor) -> char {
        let index = StackColor::ALL.iter().position(|&c| c == color).unwrap();
        char::from_digit(index as u32 + 1, 10).unwrap()
    }

    /// 確認しやすい1列の盤面(下から: 赤,青,青,黄,緑)
    fn sample_board() -> Board {
        vec![Red, Blue, Blue, Yellow, Green]
    }

    /// 最下段の色をキーで押し続けて、ラウンドをクリアする
    fn solve_round(game: &mut ColorStackGame) {
        while let Some(&color) = game.round.board.first() {
            press_key(game, key_for(color));
        }
    }

    /// 画面を描画してバッファを返す
    fn render_buffer(game: &ColorStackGame, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// 画面を描画し、全セルを空白抜きの1つの文字列にして返す
    /// (全角文字の2セル目は空白で埋まるため、空白を除いて比較する)
    fn rendered_text(game: &ColorStackGame, width: u16, height: u16) -> String {
        render_buffer(game, width, height)
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace(' ', "")
    }

    /// ブロックの色(背景色)の集合。盤面のセルがブロックで塗られているかの判定に使う
    fn block_bg_colors() -> HashSet<String> {
        StackColor::ALL
            .iter()
            .map(|c| format!("{:?}", c.color()))
            .collect()
    }

    // --- 難易度パラメータ ---

    #[test]
    fn params_match_spec_for_each_difficulty() {
        let beginner = params(Difficulty::Beginner);
        assert_eq!(beginner.height, 8);
        assert_eq!(beginner.color_count, 3);
        assert_eq!(beginner.time_limit, Duration::from_secs(60));

        let intermediate = params(Difficulty::Intermediate);
        assert_eq!(intermediate.height, 12);
        assert_eq!(intermediate.color_count, 4);
        assert_eq!(intermediate.time_limit, Duration::from_secs(90));

        let advanced = params(Difficulty::Advanced);
        assert_eq!(advanced.height, 16);
        assert_eq!(advanced.color_count, 4);
        assert_eq!(advanced.time_limit, Duration::from_secs(120));
    }

    #[test]
    fn colors_have_distinct_names_and_terminal_colors() {
        let names: HashSet<&str> = StackColor::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), StackColor::ALL.len());
        assert_eq!(block_bg_colors().len(), StackColor::ALL.len());
    }

    // --- 初期盤面 ---

    #[test]
    fn new_board_is_a_single_column_filled_to_height_with_allowed_colors() {
        for difficulty in ALL_DIFFICULTIES {
            let p = params(difficulty);
            let allowed: HashSet<StackColor> =
                StackColor::ALL[..p.color_count].iter().copied().collect();
            for seed in 0..20 {
                let board = new_board(&mut StdRng::seed_from_u64(seed), &p);
                assert_eq!(board.len(), p.height, "{difficulty:?}: 1列に初期の高さぶん");
                assert!(
                    board.iter().all(|c| allowed.contains(c)),
                    "{difficulty:?}: 使える色だけ"
                );
            }
        }
    }

    #[test]
    fn new_board_uses_every_color_at_least_once() {
        for difficulty in ALL_DIFFICULTIES {
            let p = params(difficulty);
            for seed in 0..300 {
                let board = new_board(&mut StdRng::seed_from_u64(seed), &p);
                let used: HashSet<StackColor> = board.iter().copied().collect();
                assert_eq!(
                    used.len(),
                    p.color_count,
                    "{difficulty:?} seed={seed}: 全色が最低1個"
                );
            }
        }
    }

    #[test]
    fn new_board_is_random() {
        let p = params(Difficulty::Advanced);
        let boards: HashSet<Board> = (0..5)
            .map(|seed| new_board(&mut StdRng::seed_from_u64(seed), &p))
            .collect();
        assert!(boards.len() > 1, "シードが違えば盤面も変わる");
    }

    // --- 消去・追加ルール ---

    #[test]
    fn pressing_the_bottom_color_removes_it_and_drops_the_rest() {
        let mut board = sample_board();
        assert_eq!(press_color(&mut board, Red), PressOutcome::Removed);
        assert_eq!(
            board,
            vec![Blue, Blue, Yellow, Green],
            "最下段の赤が消え、上のブロックが1段下に詰まる"
        );
        assert_eq!(press_color(&mut board, Blue), PressOutcome::Removed);
        assert_eq!(
            board,
            vec![Blue, Yellow, Green],
            "同じ色が続けば続けて消せる"
        );
    }

    #[test]
    fn pressing_a_different_color_adds_that_color_on_top() {
        for color in [Blue, Yellow, Green] {
            let mut board = sample_board();
            assert_eq!(press_color(&mut board, color), PressOutcome::Added);
            let mut expected = sample_board();
            expected.push(color);
            assert_eq!(
                board, expected,
                "{color:?}: 最下段は変わらず、一番上に押した色が1個増える"
            );
        }
    }

    #[test]
    fn repeated_misses_keep_growing_the_column() {
        let mut board = sample_board();
        for i in 1..=500 {
            assert_eq!(press_color(&mut board, Green), PressOutcome::Added);
            assert_eq!(board.len(), sample_board().len() + i);
        }
        assert_eq!(board[0], Red, "最下段は変わらない");
        assert!(board[5..].iter().all(|&c| c == Green));
    }

    #[test]
    fn cleared_follows_the_board() {
        let mut board = vec![Red];
        assert!(!is_cleared(&board));
        press_color(&mut board, Red);
        assert!(is_cleared(&board));
    }

    // --- キー入力 ---

    #[test]
    fn new_game_starts_with_a_full_single_column_and_no_records() {
        for difficulty in ALL_DIFFICULTIES {
            let game = ColorStackGame::new(difficulty);
            assert_eq!(game.round.board.len(), params(difficulty).height);
            assert!(game.round.elapsed.is_zero());
            assert!(game.interval.is_none());
            assert_eq!(game.result().total, 0);
            assert!(!game.is_finished());
        }
    }

    #[test]
    fn number_keys_press_the_matching_color() {
        for (key, color) in ['1', '2', '3', '4'].into_iter().zip(StackColor::ALL) {
            let mut game = ColorStackGame::new(Difficulty::Intermediate);
            game.round.board = sample_board();
            let mut expected = sample_board();
            press_color(&mut expected, color);
            press_key(&mut game, key);
            assert_eq!(game.round.board, expected, "キー{key}は{color:?}");
        }
    }

    #[test]
    fn a_miss_adds_a_block_and_shows_incorrect_feedback_but_records_nothing() {
        let mut game = ColorStackGame::new(Difficulty::Intermediate);
        game.round.board = sample_board();
        press_key(&mut game, key_for(Yellow));
        let mut expected = sample_board();
        expected.push(Yellow);
        assert_eq!(game.round.board, expected);
        assert_eq!(
            game.feedback.current().map(|f| f.verdict),
            Some(crate::game::feedback::Verdict::Incorrect),
            "ミスは不正解として表示する"
        );
        assert_eq!(game.result().total, 0, "ミスはスコアに記録しない");
        assert!(game.interval.is_none(), "ミスしてもラウンドは続く");
    }

    #[test]
    fn keys_beyond_color_count_and_other_keys_are_ignored() {
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        // 初級は3色なので4キーは効かない(緑が追加されることも無い)
        game.round.board = sample_board();
        press_key(&mut game, '4');
        for code in [
            KeyCode::Char('5'),
            KeyCode::Char('0'),
            KeyCode::Char('a'),
            KeyCode::Enter,
            KeyCode::Left,
        ] {
            game.handle_key(KeyEvent::from(code));
        }
        assert_eq!(game.round.board, sample_board());
        assert!(game.feedback.current().is_none());
    }

    // --- クリア・失敗 ---

    #[test]
    fn clearing_the_board_records_success_with_elapsed_time() {
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        game.update(Duration::from_millis(1500));
        solve_round(&mut game);
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
        assert!((result.avg_latency_ms - 1500.0).abs() < 1e-9);
        assert!(game.interval.is_some(), "クリア後は待ち時間に入る");
    }

    #[test]
    fn clearing_after_misses_still_records_success() {
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        game.round.board = vec![Red];
        press_key(&mut game, key_for(Blue));
        assert_eq!(game.round.board, vec![Red, Blue]);
        press_key(&mut game, key_for(Red));
        assert_eq!(game.result().total, 0, "追加された青がまだ残っている");
        press_key(&mut game, key_for(Blue));
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clearing_is_recorded_only_once_at_the_moment_the_board_empties() {
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        game.round.board = vec![Red, Blue];
        press_key(&mut game, key_for(Red));
        assert_eq!(game.result().total, 0, "まだ青が残っている");
        press_key(&mut game, key_for(Blue));
        assert_eq!(game.result().total, 1);
        press_key(&mut game, key_for(Blue));
        assert_eq!(game.result().total, 1, "待ち時間中の入力で二重に記録しない");
        assert!(
            game.round.board.is_empty(),
            "待ち時間中はミスの追加も起きない"
        );
    }

    #[test]
    fn exceeding_time_limit_records_failure_with_limit_latency() {
        for difficulty in ALL_DIFFICULTIES {
            let mut game = ColorStackGame::new(difficulty);
            let limit = params(difficulty).time_limit;
            game.update(limit - Duration::from_millis(1));
            assert_eq!(game.result().total, 0, "制限時間ちょうど手前ではまだ続く");
            game.update(Duration::from_millis(1));
            let result = game.result();
            assert_eq!(result.total, 1, "{difficulty:?}: 制限時間でラウンド終了");
            assert_eq!(result.correct, 0);
            assert!((result.avg_latency_ms - limit.as_secs_f64() * 1000.0).abs() < 1e-9);
            assert!(game.interval.is_some());
        }
    }

    #[test]
    fn overshooting_time_limit_in_one_tick_still_records_the_limit() {
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        game.update(Duration::from_secs(75));
        let result = game.result();
        assert_eq!(result.total, 1);
        assert!((result.avg_latency_ms - 60_000.0).abs() < 1e-9);
    }

    // --- ラウンド進行 ---

    #[test]
    fn next_round_starts_with_a_fresh_full_board_after_interval() {
        let mut game = ColorStackGame::new(Difficulty::Intermediate);
        // ミスで伸ばしてから時間切れにしても、次のラウンドは初期の高さに戻る
        game.round.board = sample_board();
        for _ in 0..20 {
            press_key(&mut game, key_for(Green));
        }
        game.update(params(Difficulty::Intermediate).time_limit);
        assert!(game.interval.is_some());
        game.update(ROUND_INTERVAL);
        assert!(game.interval.is_none());
        assert!(game.round.elapsed.is_zero(), "経過時間も0から");
        assert_eq!(game.round.board.len(), 12);
    }

    #[test]
    fn inputs_are_ignored_and_time_stops_during_interval() {
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        solve_round(&mut game);
        let board_before = game.round.board.clone();
        game.round.board = sample_board();
        press_key(&mut game, key_for(Red));
        press_key(&mut game, key_for(Blue));
        assert_eq!(game.round.board, sample_board(), "待ち時間中のキーは無視");
        game.round.board = board_before;

        game.update(ROUND_INTERVAL / 2);
        game.update(ROUND_INTERVAL / 2);
        game.update(Duration::from_millis(700));
        solve_round(&mut game);
        let result = game.result();
        // 2ラウンド目の記録は、ラウンド開始後に進めた700msだけ(1ラウンド目は0ms)
        assert_eq!(result.total, 2);
        assert!((result.avg_latency_ms - 350.0).abs() < 1e-9);
    }

    #[test]
    fn session_finishes_after_three_rounds() {
        let mut game = ColorStackGame::new(Difficulty::Advanced);
        // 1ラウンド目: クリア / 2ラウンド目: 時間切れ / 3ラウンド目: クリア
        solve_round(&mut game);
        game.update(ROUND_INTERVAL);
        assert!(!game.is_finished());
        game.update(params(Difficulty::Advanced).time_limit);
        game.update(ROUND_INTERVAL);
        assert!(!game.is_finished());
        solve_round(&mut game);
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.difficulty, Difficulty::Advanced);
        assert_eq!(result.total, ROUNDS_PER_SESSION);
        assert_eq!(result.correct, 2);
    }

    #[test]
    fn inputs_and_time_after_session_finished_are_ignored() {
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        for _ in 0..ROUNDS_PER_SESSION {
            solve_round(&mut game);
            game.update(ROUND_INTERVAL);
        }
        assert!(game.is_finished());
        game.round.board = sample_board();
        press_key(&mut game, key_for(Red));
        press_key(&mut game, key_for(Blue));
        game.update(Duration::from_secs(600));
        let button = buttons_area(AREA);
        game.handle_mouse(left_click(button.x + 1, button.y + 1), AREA);
        assert_eq!(game.round.board, sample_board());
        assert_eq!(game.result().total, ROUNDS_PER_SESSION);
    }

    // --- マウス: 色ボタン ---

    /// 描画したバッファから、ボタン領域内で色名が描かれているセル座標を探す
    fn button_label_position(buffer: &ratatui::buffer::Buffer, color: StackColor) -> (u16, u16) {
        let area = buttons_area(AREA);
        (area.y..area.bottom())
            .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
            .find(|&(x, y)| buffer[(x, y)].symbol() == color.name())
            .unwrap_or_else(|| panic!("{}ボタンが描かれていること", color.name()))
    }

    #[test]
    fn buttons_are_drawn_where_clicking_presses_their_color() {
        for difficulty in ALL_DIFFICULTIES {
            let count = params(difficulty).color_count;
            for &color in &StackColor::ALL[..count] {
                let mut game = ColorStackGame::new(difficulty);
                game.round.board = sample_board();
                let buffer = render_buffer(&game, AREA.width, AREA.height);
                let (x, y) = button_label_position(&buffer, color);
                game.handle_mouse(left_click(x, y), AREA);
                let mut expected = sample_board();
                press_color(&mut expected, color);
                assert_eq!(
                    game.round.board, expected,
                    "{difficulty:?}: {color:?}ボタン"
                );
            }
        }
    }

    #[test]
    fn beginner_shows_only_three_buttons() {
        let game = ColorStackGame::new(Difficulty::Beginner);
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let area = buttons_area(AREA);
        let has_green = (area.y..area.bottom())
            .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
            .any(|(x, y)| buffer[(x, y)].symbol() == Green.name());
        assert!(!has_green, "初級は3色なので緑ボタンは無い");
    }

    #[test]
    fn clicking_edges_of_each_button_band_presses_that_color() {
        let area = buttons_area(AREA);
        let bands = crate::game::theme::column_bands(area, 4);
        for (band, color) in bands.iter().zip(StackColor::ALL) {
            for x in [band.x, band.right() - 1] {
                let mut game = ColorStackGame::new(Difficulty::Intermediate);
                game.round.board = sample_board();
                game.handle_mouse(left_click(x, band.y), AREA);
                let mut expected = sample_board();
                press_color(&mut expected, color);
                assert_eq!(game.round.board, expected, "x={x}: {color:?}");
            }
        }
    }

    #[test]
    fn clicks_outside_buttons_or_non_left_clicks_do_nothing() {
        let mut game = ColorStackGame::new(Difficulty::Intermediate);
        game.round.board = sample_board();
        // HUD・盤面のクリック
        game.handle_mouse(left_click(1, 1), AREA);
        let board = board_area(AREA);
        for y in board.y..board.bottom() {
            game.handle_mouse(left_click(board.x + board.width / 2, y), AREA);
        }
        // ボタン上の右クリック
        let button = buttons_area(AREA);
        game.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: button.x + 1,
                row: button.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            AREA,
        );
        assert_eq!(game.round.board, sample_board());
    }

    // --- 盤面の描画 ---

    #[test]
    fn layout_areas_do_not_overlap_and_stay_inside() {
        let board = board_area(AREA);
        let buttons = buttons_area(AREA);
        assert!(board.height > 0 && buttons.height > 0);
        assert!(board.bottom() <= buttons.y, "盤面の下にボタンが並ぶ");
        assert!(buttons.bottom() <= AREA.bottom());
        assert!(board.y >= crate::game::theme::HUD_HEIGHT, "盤面はHUDの下");
    }

    #[test]
    fn grid_is_one_centered_column_bottom_aligned_and_stacked() {
        for difficulty in ALL_DIFFICULTIES {
            let height = params(difficulty).height;
            let board = board_area(AREA);
            let grid = grid_geometry(board, height);
            assert!(
                grid.visible_rows >= height,
                "{difficulty:?}: 標準の画面なら初期の高さは全段描ける"
            );
            let bottom = grid.cell_rect(0).expect("最下段は描ける");
            assert_eq!(bottom.bottom(), board.bottom(), "最下段は下端にそろう");
            // 横は盤面の中央
            let left_space = bottom.x - board.x;
            let right_space = board.right() - bottom.right();
            assert!(
                left_space.abs_diff(right_space) <= 1,
                "{difficulty:?}: 横中央"
            );
            for level in 0..grid.visible_rows {
                let rect = grid.cell_rect(level).expect("描ける段");
                assert!(rect.width > 0 && rect.height > 0);
                assert_eq!(rect.x, bottom.x, "全段が同じ1列に並ぶ");
                assert_eq!(rect.width, bottom.width);
                assert!(rect.x >= board.x && rect.right() <= board.right());
                assert!(rect.y >= board.y && rect.bottom() <= board.bottom());
                if level > 0 {
                    let below = grid.cell_rect(level - 1).unwrap();
                    assert_eq!(rect.bottom(), below.y, "上の段は下の段のすぐ上");
                }
            }
            assert!(
                grid.cell_rect(grid.visible_rows).is_none(),
                "盤面より上の段は描かない"
            );
        }
    }

    #[test]
    fn grid_on_short_board_draws_only_bottom_rows_that_fit() {
        let board = Rect::new(2, 3, 40, 5);
        let grid = grid_geometry(board, 16);
        assert_eq!(grid.visible_rows, 5);
        assert!(grid.cell_rect(4).is_some());
        assert!(grid.cell_rect(5).is_none(), "入りきらない上の段は描かない");
        assert_eq!(grid.cell_rect(0).unwrap().bottom(), board.bottom());
        // 盤面が空でも破綻しない
        let empty = grid_geometry(Rect::new(0, 0, 0, 0), 16);
        assert!(empty.cell_rect(0).is_none());
    }

    #[test]
    fn row_height_is_fixed_by_initial_height_even_when_the_column_grows() {
        // 1段の高さは初期の高さで決め、ミスで伸びても変えない(伸びたぶんは上に積む)
        let board = board_area(AREA);
        let grid = grid_geometry(board, params(Difficulty::Beginner).height);
        assert_eq!(grid.row_height, 2);
        assert_eq!(grid.visible_rows, (board.height / 2) as usize);
    }

    /// area内でブロックの色で塗られたセルの座標
    fn painted_block_cells(buffer: &ratatui::buffer::Buffer, area: Rect) -> Vec<(u16, u16)> {
        let colors = block_bg_colors();
        (area.y..area.bottom())
            .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| colors.contains(&format!("{:?}", buffer[(x, y)].bg)))
            .collect()
    }

    #[test]
    fn board_is_drawn_as_a_single_column() {
        let mut game = ColorStackGame::new(Difficulty::Advanced);
        game.round.board = sample_board();
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_geometry(board_area(AREA), params(Difficulty::Advanced).height);
        let column = grid.cell_rect(0).unwrap();
        let painted = painted_block_cells(&buffer, board_area(AREA));
        assert!(!painted.is_empty());
        assert!(
            painted
                .iter()
                .all(|&(x, _)| x >= column.x && x < column.right()),
            "ブロックは1列ぶんの幅にだけ塗られる"
        );
    }

    #[test]
    fn board_cells_are_painted_with_block_colors() {
        let mut game = ColorStackGame::new(Difficulty::Advanced);
        game.round.board = sample_board();
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_geometry(board_area(AREA), params(Difficulty::Advanced).height);
        let colors = block_bg_colors();
        for level in 0..grid.visible_rows {
            let rect = grid.cell_rect(level).unwrap();
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    let bg = buffer[(x, y)].bg;
                    match game.round.board.get(level) {
                        Some(color) => assert_eq!(bg, color.color(), "{level}段目は{color:?}"),
                        None => assert!(!colors.contains(&format!("{bg:?}")), "{level}段目は空"),
                    }
                }
            }
        }
    }

    #[test]
    fn two_row_blocks_have_a_separator_line_on_their_bottom_row() {
        // 初級は1段2行で描ける。同じ色が縦に続いても1個ずつ見分けられるよう、各ブロックの下の行に線を引く
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        game.round.board = vec![Red, Red, Red];
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_geometry(board_area(AREA), params(Difficulty::Beginner).height);
        assert_eq!(grid.row_height, 2);
        for level in 0..3 {
            let rect = grid.cell_rect(level).unwrap();
            let bottom_row = rect.bottom() - 1;
            for x in rect.x..rect.right() {
                let cell = &buffer[(x, bottom_row)];
                assert_eq!(cell.symbol(), "▁", "{level}段目の下の行");
                assert_eq!(cell.bg, Red.color(), "線の行も背景はブロックの色");
            }
        }
        // 最下段のブロックには数字キーを重ねる(上の行)
        let rect = grid.cell_rect(0).unwrap();
        let top_row: String = (rect.x..rect.right())
            .map(|x| buffer[(x, rect.y)].symbol().to_string())
            .collect();
        assert!(top_row.contains('1'), "赤は1キー");
    }

    #[test]
    fn board_drawing_follows_removal_and_addition() {
        let mut game = ColorStackGame::new(Difficulty::Intermediate);
        game.round.board = sample_board();
        press_key(&mut game, key_for(Red));
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_geometry(board_area(AREA), 12);
        // 赤が消えて青が最下段に下りてくる。5段目(level 4)は空く
        let cell = grid.cell_rect(0).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Blue.color());
        let cell = grid.cell_rect(4).unwrap();
        assert!(!block_bg_colors().contains(&format!("{:?}", buffer[(cell.x, cell.y)].bg)));

        // ミスで赤が一番上(level 4)に追加される
        press_key(&mut game, key_for(Red));
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let cell = grid.cell_rect(4).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Red.color());
        let cell = grid.cell_rect(0).unwrap();
        assert_eq!(
            buffer[(cell.x, cell.y)].bg,
            Blue.color(),
            "最下段は変わらない"
        );
    }

    #[test]
    fn column_taller_than_the_initial_height_is_drawn_while_it_fits() {
        // 初級(初期8段)でミスして10段にしても、標準の画面なら10段とも描く
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        game.round.board = vec![Red; 8];
        press_key(&mut game, key_for(Blue));
        press_key(&mut game, key_for(Yellow));
        assert_eq!(game.round.board.len(), 10);
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_geometry(board_area(AREA), 8);
        let cell = grid.cell_rect(8).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Blue.color());
        let cell = grid.cell_rect(9).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Yellow.color());
    }

    #[test]
    fn column_overflowing_the_board_draws_bottom_rows_only_without_panic() {
        let mut game = ColorStackGame::new(Difficulty::Advanced);
        game.round.board = sample_board();
        for _ in 0..300 {
            press_key(&mut game, key_for(Green));
        }
        assert_eq!(game.round.board.len(), sample_board().len() + 300);
        assert!(game.interval.is_none(), "伸びてもラウンドは続く");

        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let board = board_area(AREA);
        let grid = grid_geometry(board, params(Difficulty::Advanced).height);
        assert!(
            grid.visible_rows < game.round.board.len(),
            "盤面に収まらない高さ"
        );
        // 最下段は元のまま、見える一番上まで塗られている
        let cell = grid.cell_rect(0).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Red.color());
        let top = grid.cell_rect(grid.visible_rows - 1).unwrap();
        assert_eq!(buffer[(top.x, top.y)].bg, Green.color());
        // はみ出た段は盤面より上(HUD・盤面の枠)に描かない
        let above_board = Rect::new(0, 0, AREA.width, board.y);
        assert!(
            painted_block_cells(&buffer, above_board).is_empty(),
            "はみ出た段は描かない"
        );

        // 狭い画面でも描画がパニックしない
        for (w, h) in [(1, 1), (20, 6), (30, 10), (100, 12), (200, 60)] {
            rendered_text(&game, w, h);
        }
        // 伸びた列でも、最下段から押していけばクリアできる
        solve_round(&mut game);
        assert_eq!(game.result().correct, 1);
    }

    // --- HUD・その他の描画 ---

    #[test]
    fn hud_shows_round_elapsed_time_and_remaining_blocks() {
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        game.round.board = sample_board();
        game.update(Duration::from_millis(12_300));
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("ROUND1/3"));
        assert!(text.contains("12.3"), "経過時間");
        assert!(text.contains("60"), "制限時間");
        assert!(text.contains("残り5個"));

        press_key(&mut game, key_for(Red));
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("残り4個"), "消すと減る");

        press_key(&mut game, key_for(Red));
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("残り5個"), "ミスで追加されると増える");
    }

    #[test]
    fn interval_message_tells_clear_or_time_up() {
        let mut game = ColorStackGame::new(Difficulty::Beginner);
        solve_round(&mut game);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("CLEAR!"));
        assert!(text.contains("NEXTROUND2/3"));

        game.update(ROUND_INTERVAL);
        game.update(params(Difficulty::Beginner).time_limit);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("TIMEUP"));
        assert!(text.contains("NEXTROUND3/3"));
    }

    #[test]
    fn render_does_not_panic_in_tiny_areas() {
        let mut game = ColorStackGame::new(Difficulty::Advanced);
        for (w, h) in [(1, 1), (20, 6), (30, 10), (100, 12)] {
            rendered_text(&game, w, h);
        }
        solve_round(&mut game);
        rendered_text(&game, 20, 6);
    }
}
