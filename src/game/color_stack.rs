//! シタケシ: 積まれた色ブロックを、色ボタンで「一番下」から消していくタイムアタック。
//!
//! 1セッション=制限時間60秒で、難易度選択は無い。盤面は4列で、各列は専用の色。
//! 1段に1列だけブロックがあり(同時押しは無い)、盤面の最下段にある色を押した時だけ最下段が消える。
//! 消えたブロックはその列の一番上にすぐ補充されるので、盤面は空にならず時間いっぱい消し続けられる。
//! それ以外の色はミスで、その列の一番上に1個追加される(段が増えるだけで詰みにはしない)。
//!
//! スコアは消せた数と、1個消すごとの間隔(応答時間)。ミスは記録しない。

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

/// 結果に記録する難易度。シタケシは難易度を選ばないので固定値にする
/// (これまでの記録と同じ扱いになるよう中級のままにする)
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Intermediate;

/// セッション全体の制限時間。これを過ぎたら終了する
pub const TIME_LIMIT: Duration = Duration::from_secs(60);

/// 色ボタンを並べる領域の高さ(枠込み)
const BUTTONS_HEIGHT: u16 = 3;

/// マスの幅の上限。広い画面でもブロックが横に伸びすぎないようにする
const MAX_CELL_WIDTH: u16 = 16;

/// 1段の高さの上限。低い積み上げでも縦に伸びすぎないようにする
const MAX_ROW_HEIGHT: u16 = 2;

/// 盤面の列数。列iの専用色はStackColor::ALL[i]
const LANE_COUNT: usize = StackColor::ALL.len();

/// 開始時の段数。正解では補充されて減らないので、ミスが無ければ最後までこの高さのまま
const INITIAL_HEIGHT: usize = 36;

/// 盤面パネルの見出し
const PANEL_TITLE: &str = " 各列の一番下の色を押して消す ";

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

/// 盤面。rows[0]が最下段で、各段は列数ぶんのセル(Noneは空白)。
/// 「ブロックが1個も無い段」は残さない(消えた段は上の段が詰めて下りてくる)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Board {
    columns: usize,
    rows: Vec<Vec<Option<StackColor>>>,
}

impl Board {
    /// 下から順に、各段のlanes[i]列目にだけその列の専用色を置いた4列の盤面
    fn lanes(lanes: impl IntoIterator<Item = usize>) -> Self {
        Self {
            columns: LANE_COUNT,
            rows: lanes.into_iter().map(lane_row).collect(),
        }
    }

    /// ブロックの数(空白セルは数えない)
    fn block_count(&self) -> usize {
        self.rows.iter().flatten().filter(|c| c.is_some()).count()
    }
}

/// column列目にだけその列の専用色を置いた、4列盤面の1段
fn lane_row(column: usize) -> Vec<Option<StackColor>> {
    (0..LANE_COUNT)
        .map(|c| (c == column).then_some(StackColor::ALL[c]))
        .collect()
}

/// セッション開始時の盤面を作る。各段にどの列を置くかを決める。
/// 先頭の列数ぶんに各列を1個ずつ置き、残りをランダムにしてから全体をシャッフルするので、
/// どの列も最低1個は含まれる
fn new_board(rng: &mut impl Rng) -> Board {
    let mut picks: Vec<usize> = (0..INITIAL_HEIGHT)
        .map(|i| {
            if i < LANE_COUNT {
                i
            } else {
                rng.gen_range(0..LANE_COUNT)
            }
        })
        .collect();
    picks.shuffle(rng);
    Board::lanes(picks)
}

/// 色ボタンを押した結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressOutcome {
    /// ブロックが1個消えた
    Removed,
    /// 消せるブロックが無く、一番上に1個追加された(ミス)
    Added,
}

/// column列目のボタンを押した時の処理。
/// 盤面全体の最下段(rows[0])のブロックがcolumn列にある時だけ、その段を消して上の段を詰める。
/// それ以外(押した列の順番がまだ来ていない・盤面が空)はミスとして、
/// その列の専用色を一番上の新しい段に置く(1段1列を保つ)
fn press_lane(board: &mut Board, column: usize) -> PressOutcome {
    let bottom_matches = board
        .rows
        .first()
        .is_some_and(|row| row.get(column).is_some_and(|c| c.is_some()));
    if bottom_matches {
        // 1段1列なので最下段はこの1個だけ。消した段には上の段が1段ずつ下りてくる
        board.rows.remove(0);
        PressOutcome::Removed
    } else {
        board.rows.push(lane_row(column));
        PressOutcome::Added
    }
}

/// 盤面を描くマス目の位置。renderと(テストでの)位置確認で共有する
#[derive(Debug, Clone, Copy, PartialEq)]
struct GridGeometry {
    /// 1列目のマスの左端
    origin_x: u16,
    /// 列の間隔
    column_pitch: u16,
    /// 列数
    columns: usize,
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
    /// column列目(0始まり)・下からlevel段目(0=最下段)のマス。描けない位置ならNone
    fn cell_rect(&self, column: usize, level: usize) -> Option<Rect> {
        if column >= self.columns || level >= self.visible_rows || self.cell_width == 0 {
            return None;
        }
        let x = self.origin_x + column as u16 * self.column_pitch;
        let y = self.bottom - (level as u16 + 1) * self.row_height;
        Some(Rect::new(x, y, self.cell_width, self.row_height))
    }
}

/// board(盤面パネルの内側)にcolumns列のマス目を置く。
/// 盤面を列数で等分した帯(色ボタンの並びと同じ分け方)の中央に各列を置き、
/// 縦は最下段を盤面の下端にそろえる。
/// 1段の高さは初期の高さ(base_height)が収まるように決め(max_row_heightまで)、ミスで伸びても変えない。
/// 描ける段数は盤面に入るぶんだけで、入りきらない上の段は描かない
fn grid_geometry(
    board: Rect,
    columns: usize,
    base_height: usize,
    max_row_height: u16,
) -> GridGeometry {
    let columns_u16 = u16::try_from(columns.max(1)).unwrap_or(u16::MAX);
    let column_pitch = board.width / columns_u16;
    // 複数列の時は隣の列とくっつかないよう、1セルのすき間を空ける
    let gap = u16::from(columns > 1 && column_pitch >= 2);
    let cell_width = (column_pitch - gap).min(MAX_CELL_WIDTH);
    let base_height = u16::try_from(base_height.max(1)).unwrap_or(u16::MAX);
    let row_height = (board.height / base_height).clamp(1, max_row_height.max(1));
    let visible_rows = if cell_width == 0 {
        0
    } else {
        (board.height / row_height) as usize
    };
    GridGeometry {
        origin_x: board.x + (column_pitch - cell_width) / 2,
        column_pitch,
        columns,
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

pub struct ColorStackGame {
    board: Board,
    /// 直前にブロックを消してからの経過時間(応答時間の測定に使う)
    elapsed_since_clear: Duration,
    /// セッション開始からの経過時間。TIME_LIMITを超えたら終了
    elapsed_total: Duration,
    tracker: ScoreTracker,
    feedback: AnswerFeedback,
}

impl ColorStackGame {
    pub fn new() -> Self {
        Self {
            board: new_board(&mut rand::thread_rng()),
            elapsed_since_clear: Duration::ZERO,
            elapsed_total: Duration::ZERO,
            tracker: ScoreTracker::new(),
            feedback: AnswerFeedback::new(),
        }
    }

    /// プレイ中(制限時間内)か
    fn is_playing(&self) -> bool {
        !self.is_finished()
    }

    /// 残り時間(秒・切り上げ)。開始直後を60秒、終了直前を1秒と見せ、0秒のまま押せる瞬間を作らない
    fn remaining_seconds(&self) -> u64 {
        let remaining = TIME_LIMIT.saturating_sub(self.elapsed_total);
        remaining.as_nanos().div_ceil(1_000_000_000) as u64
    }

    /// index番目(0始まり)の色ボタンを押す。使わない色の番号やプレイ中以外は無視する
    fn press_button(&mut self, index: usize) {
        if !self.is_playing() || index >= LANE_COUNT {
            return;
        }
        // 列iのボタン=列iの専用色なので、ボタン番号がそのまま列になる
        if press_lane(&mut self.board, index) == PressOutcome::Added {
            // ミス: 一番上に1個積まれた(ゲームは続き、スコアには記録しない)
            audio::play_se(SeKind::ColorStackMiss);
            self.feedback.record(false, "+1段");
            return;
        }
        audio::play_se(SeKind::ColorStackClear);
        // 盤面を空にしないため、消えた列の一番上にすぐ同じ色を補充する
        self.board.rows.push(lane_row(index));
        self.tracker
            .record(true, self.elapsed_since_clear.as_secs_f64() * 1000.0);
        self.feedback.record(true, "");
        self.elapsed_since_clear = Duration::ZERO;
    }

    fn render_hud(&self, frame: &mut Frame, area: Rect) {
        let block = theme::panel(" ◆ シタケシ ")
            .border_style(Style::default().fg(theme::flash_border_color(self.feedback.current())));
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

        let cleared = Line::from(vec![
            Span::styled(" 消せた ", Style::default().fg(theme::MUTED)),
            Span::styled(format!("{}枚", self.tracker.total()), theme::title_style()),
        ]);
        frame.render_widget(Paragraph::new(cleared), cols[0]);

        // 中央: 正誤表示中はそれを、プレイ中は残り時間を出す
        let center = match self.feedback.current() {
            Some(flash) => theme::flash_line(flash),
            None if self.is_playing() => Line::from(vec![
                Span::styled("残り ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    format!("{}秒", self.remaining_seconds()),
                    Style::default()
                        .fg(theme::HIGHLIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            None => Line::from(""),
        };
        frame.render_widget(Paragraph::new(center).alignment(Alignment::Center), cols[1]);

        // 無限に補充されるので、ミスで増えた分が分かる程度の参考値
        let remaining = Line::from(vec![
            Span::styled("残り ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!("{}個", self.board.block_count()),
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

    /// 盤面のブロックを描く。各列の一番下のブロックには対応する数字キーを重ねて、押す色を分かりやすくする
    fn render_board(&self, frame: &mut Frame, board: Rect) {
        let grid = grid_geometry(board, LANE_COUNT, INITIAL_HEIGHT, MAX_ROW_HEIGHT);
        // 数字キーを重ねた列(列ごとに一番下のブロックにだけ出す)
        let mut labeled = vec![false; self.board.columns];
        // 入りきらない上の段は描かない
        for (level, row) in self.board.rows.iter().enumerate().take(grid.visible_rows) {
            for (column, cell) in row.iter().enumerate() {
                let (Some(color), Some(rect)) = (cell, grid.cell_rect(column, level)) else {
                    continue;
                };
                let style = Style::default().bg(color.color());
                frame.render_widget(Block::default().style(style), rect);
                // 2行以上で描ける時は下の行に線を引き、同じ色が縦に続いても1個ずつ見分けられるようにする
                if rect.height >= 2 {
                    let separator = Paragraph::new("▁".repeat(rect.width as usize))
                        .style(style.fg(Color::Black));
                    frame.render_widget(
                        separator,
                        Rect::new(rect.x, rect.bottom() - 1, rect.width, 1),
                    );
                }
                if !labeled[column] {
                    labeled[column] = true;
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
    }

    /// colorのボタン番号(数字キー。1始まり)
    fn button_number(&self, color: StackColor) -> usize {
        StackColor::ALL
            .iter()
            .position(|&c| c == color)
            .map_or(0, |i| i + 1)
    }

    /// 色ボタンを横に等分して描く。分割はクリック判定のcolumn_indexと同じcolumn_bands
    fn render_buttons(&self, frame: &mut Frame, area: Rect) {
        let colors = &StackColor::ALL;
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
}

impl Default for ColorStackGame {
    fn default() -> Self {
        Self::new()
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
        if let Some(index) = column_index(buttons, mouse.column, LANE_COUNT as u16) {
            self.press_button(index);
        }
    }

    fn update(&mut self, dt: Duration) {
        self.feedback.tick(dt);
        if self.is_finished() {
            return;
        }
        // 1回のtickで大きく越えても、表示・判定上は制限時間で止める
        self.elapsed_total = (self.elapsed_total + dt).min(TIME_LIMIT);
        self.elapsed_since_clear += dt;
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud, panel, buttons) = split_areas(area);
        self.render_hud(frame, hud);

        let block = theme::focus_panel(PANEL_TITLE, self.feedback.current());
        frame.render_widget(block, panel);
        // focus_panelは1セル枠なので、内側はboard_area(テストでの位置確認と共有)と一致する
        let board = board_area(area);
        if !board.is_empty() {
            self.render_board(frame, board);
        }
        self.render_buttons(frame, buttons);
    }

    fn is_finished(&self) -> bool {
        self.elapsed_total >= TIME_LIMIT
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::feedback::{Verdict, FEEDBACK_HOLD};
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use std::collections::HashSet;
    use StackColor::{Blue, Green, Red, Yellow};

    const AREA: Rect = Rect::new(0, 0, 100, 36);

    /// 初期のブロック(INITIAL_HEIGHT段)が全段、1段2行でも収まる縦長の画面
    const TALL_AREA: Rect = Rect::new(0, 0, 100, 160);

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

    /// 色colorの並び順(0始まり)。この番号の列がcolorの専用列
    fn color_index(color: StackColor) -> usize {
        StackColor::ALL.iter().position(|&c| c == color).unwrap()
    }

    /// 色colorに対応する数字キー
    fn key_for(color: StackColor) -> char {
        char::from_digit(color_index(color) as u32 + 1, 10).unwrap()
    }

    /// column列目に対応する数字キー
    fn key_for_lane(column: usize) -> char {
        key_for(StackColor::ALL[column])
    }

    /// 確認しやすい盤面(下から: 赤の列,青の列,青の列,黄の列,緑の列)
    fn sample_board() -> Board {
        Board::lanes([0, 1, 1, 2, 3])
    }

    /// 最下段のブロックがある列。盤面が空ならNone
    fn bottom_lane(board: &Board) -> Option<usize> {
        board.rows.first()?.iter().position(|c| c.is_some())
    }

    /// 最下段をcolumn列にし、その上に残りの列を1段ずつ積んだ盤面と、残りの列
    fn board_with_bottom_lane(column: usize) -> (Board, Vec<usize>) {
        let others: Vec<usize> = (0..LANE_COUNT).filter(|&c| c != column).collect();
        let board = Board::lanes(std::iter::once(column).chain(others.iter().copied()));
        (board, others)
    }

    /// column列を最下段から消した直後の盤面(残りの列が下りてきて、一番上にcolumn列が補充される)
    fn board_after_clearing_bottom_lane(column: usize) -> Board {
        let (_, others) = board_with_bottom_lane(column);
        Board::lanes(others.into_iter().chain(std::iter::once(column)))
    }

    /// 4列の盤面で、各段のブロックがどの列にあるか(下から)。ブロックが1段に1個である前提
    fn lane_of_each_row(board: &Board) -> Vec<usize> {
        board
            .rows
            .iter()
            .map(|row| {
                let lanes: Vec<usize> = (0..row.len()).filter(|&c| row[c].is_some()).collect();
                assert_eq!(lanes.len(), 1, "1段にブロックは1個だけ: {row:?}");
                lanes[0]
            })
            .collect()
    }

    /// 各列にあるブロックの数(列の高さ)
    fn lane_heights(board: &Board) -> Vec<usize> {
        (0..LANE_COUNT)
            .map(|column| {
                board
                    .rows
                    .iter()
                    .filter(|row| row[column].is_some())
                    .count()
            })
            .collect()
    }

    /// 4列の盤面が「各段に1列だけ」「各列は専用色だけ」を満たすか
    fn assert_lane_invariants(board: &Board) {
        assert_eq!(board.columns, LANE_COUNT);
        for row in &board.rows {
            assert_eq!(row.len(), LANE_COUNT, "各段は4列ぶんのセル");
            assert_eq!(
                row.iter().filter(|c| c.is_some()).count(),
                1,
                "各段にブロックは1列だけ: {row:?}"
            );
            for (column, cell) in row.iter().enumerate() {
                if let Some(color) = cell {
                    assert_eq!(*color, StackColor::ALL[column], "列{column}は専用色だけ");
                }
            }
        }
    }

    /// 最下段の列のボタンを押す(必ず1個消える)
    fn press_bottom(game: &mut ColorStackGame) {
        let column = bottom_lane(&game.board).expect("無限生成なので盤面は空にならない");
        press_key(game, key_for_lane(column));
    }

    /// 最下段ではない列のボタンを押す(必ずミスになる)
    fn press_wrong(game: &mut ColorStackGame) {
        let bottom = bottom_lane(&game.board).expect("盤面は空にならない");
        press_key(game, key_for_lane((bottom + 1) % LANE_COUNT));
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

    /// ブロックの色(背景色)の集合
    fn block_bg_colors() -> HashSet<String> {
        StackColor::ALL
            .iter()
            .map(|c| format!("{:?}", c.color()))
            .collect()
    }

    /// 盤面のマス目
    fn grid_for(board: Rect) -> GridGeometry {
        grid_geometry(board, LANE_COUNT, INITIAL_HEIGHT, MAX_ROW_HEIGHT)
    }

    fn assert_latency(result: &GameResult, expected_ms: f64) {
        assert!(
            (result.avg_latency_ms - expected_ms).abs() < 1e-6,
            "avg_latency_ms={} expected={expected_ms}",
            result.avg_latency_ms
        );
    }

    // --- 定数 ---

    #[test]
    fn time_limit_is_60_seconds() {
        assert_eq!(TIME_LIMIT, Duration::from_secs(60));
    }

    #[test]
    fn initial_height_is_36_rows_on_four_lanes() {
        assert_eq!(INITIAL_HEIGHT, 36);
        assert_eq!(LANE_COUNT, 4);
    }

    #[test]
    fn colors_have_distinct_names_and_terminal_colors() {
        let names: HashSet<&str> = StackColor::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), StackColor::ALL.len());
        assert_eq!(block_bg_colors().len(), StackColor::ALL.len());
    }

    // --- 初期盤面 ---

    #[test]
    fn new_board_has_four_lanes_and_exactly_one_block_per_row() {
        for seed in 0..50 {
            let board = new_board(&mut StdRng::seed_from_u64(seed));
            assert_eq!(board.columns, LANE_COUNT, "seed={seed}: 4列");
            assert_eq!(
                board.rows.len(),
                INITIAL_HEIGHT,
                "seed={seed}: 初期の高さぶんの段"
            );
            assert_eq!(
                board.block_count(),
                INITIAL_HEIGHT,
                "1段に1個なので段数=ブロック数"
            );
            assert_lane_invariants(&board);
        }
    }

    #[test]
    fn new_board_uses_every_column_at_least_once() {
        for seed in 0..300 {
            let board = new_board(&mut StdRng::seed_from_u64(seed));
            let used: HashSet<usize> = lane_of_each_row(&board).into_iter().collect();
            assert_eq!(used.len(), LANE_COUNT, "seed={seed}: 全列が最低1回");
        }
    }

    #[test]
    fn new_board_is_random() {
        let boards: HashSet<Board> = (0..5)
            .map(|seed| new_board(&mut StdRng::seed_from_u64(seed)))
            .collect();
        assert!(boards.len() > 1, "シードが違えば盤面も変わる");
    }

    #[test]
    fn new_game_starts_playing_with_a_full_lane_board_and_no_records() {
        let game = ColorStackGame::new();
        assert_eq!(game.board.columns, LANE_COUNT);
        assert_eq!(game.board.block_count(), INITIAL_HEIGHT);
        assert_lane_invariants(&game.board);
        assert!(game.elapsed_total.is_zero());
        assert!(game.elapsed_since_clear.is_zero());
        assert!(game.is_playing());
        assert!(!game.is_finished());
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.difficulty, SESSION_DIFFICULTY);
        assert_eq!(result.total, 0);
    }

    // --- 消去・追加ルール(press_lane) ---

    #[test]
    fn pressing_the_lane_of_the_bottom_row_removes_it_and_drops_the_rest() {
        // 下から: 列0, 列1, 列0
        let mut board = Board::lanes([0, 1, 0]);
        assert_eq!(press_lane(&mut board, 0), PressOutcome::Removed);
        assert_eq!(
            lane_of_each_row(&board),
            vec![1, 0],
            "最下段の列0が消え、上の段が1段ずつ下りてくる"
        );
        assert_eq!(press_lane(&mut board, 1), PressOutcome::Removed);
        assert_eq!(lane_of_each_row(&board), vec![0]);
        assert_lane_invariants(&board);
    }

    #[test]
    fn pressing_a_lane_whose_block_is_not_yet_at_the_bottom_is_a_miss() {
        // 下から: 列2(黄), 列0(赤), 列3(緑)。今一番下にあるのは黄なので、赤・緑は順番がまだ来ていない
        for column in [0, 3] {
            let mut board = Board::lanes([2, 0, 3]);
            assert_eq!(
                press_lane(&mut board, column),
                PressOutcome::Added,
                "列{column}: 最下段に無い列はミス"
            );
            assert_eq!(
                lane_of_each_row(&board),
                vec![2, 0, 3, column],
                "列{column}: 既存の段は変わらず、押した列が一番上に1個増える"
            );
            assert_lane_invariants(&board);
        }
    }

    #[test]
    fn pressing_a_lane_that_is_used_only_above_the_bottom_does_not_remove_it() {
        // 下から: 列0, 列1, 列0。列0の2個目は最下段ではないので、列1より先には消せない
        let mut board = Board::lanes([0, 1, 0]);
        assert_eq!(press_lane(&mut board, 0), PressOutcome::Removed);
        assert_eq!(
            press_lane(&mut board, 0),
            PressOutcome::Added,
            "最下段は列1なので列0はミス"
        );
        assert_eq!(lane_of_each_row(&board), vec![1, 0, 0]);
        assert_lane_invariants(&board);
    }

    #[test]
    fn miss_adds_the_pressed_lanes_own_color_on_top_and_keeps_other_lanes() {
        let before = Board::lanes([2, 0, 3, 1]);
        for column in [0, 1, 3] {
            let mut board = before.clone();
            assert_eq!(press_lane(&mut board, column), PressOutcome::Added);
            assert_eq!(
                &board.rows[..before.rows.len()],
                before.rows.as_slice(),
                "列{column}: 既存の段は1セルも変わらない"
            );
            assert_eq!(
                board.rows.last().unwrap(),
                &lane_row(column),
                "列{column}: 一番上に押した列の専用色だけの段が追加される"
            );
            assert_eq!(
                board.rows.last().unwrap()[column],
                Some(StackColor::ALL[column])
            );
            assert_lane_invariants(&board);
        }
    }

    #[test]
    fn pressing_any_lane_on_an_empty_board_is_a_miss() {
        for column in 0..LANE_COUNT {
            let mut board = Board::lanes([]);
            assert_eq!(
                press_lane(&mut board, column),
                PressOutcome::Added,
                "列{column}: 空の盤面ではミス"
            );
            assert_eq!(lane_of_each_row(&board), vec![column]);
            assert_lane_invariants(&board);
        }
    }

    #[test]
    fn pressing_an_empty_lane_is_a_miss_and_adds_its_own_color_on_top() {
        let mut board = Board::lanes([0, 1]);
        assert_eq!(press_lane(&mut board, 2), PressOutcome::Added);
        assert_eq!(
            lane_of_each_row(&board),
            vec![0, 1, 2],
            "押した列の最上段に1個追加"
        );
        assert_eq!(
            board.rows.last().unwrap()[2],
            Some(StackColor::ALL[2]),
            "追加されるのはその列の専用色"
        );
        assert_lane_invariants(&board);
    }

    #[test]
    fn lane_invariants_hold_after_many_random_presses() {
        let mut rng = StdRng::seed_from_u64(7);
        let mut board = new_board(&mut rng);
        for _ in 0..500 {
            let column = rng.gen_range(0..LANE_COUNT);
            let before = board.block_count();
            let bottom = bottom_lane(&board);
            let outcome = press_lane(&mut board, column);
            assert_eq!(
                outcome == PressOutcome::Removed,
                bottom == Some(column),
                "消えるのは押した列が最下段の列と一致した時だけ"
            );
            match outcome {
                PressOutcome::Removed => assert_eq!(board.block_count(), before - 1),
                PressOutcome::Added => assert_eq!(board.block_count(), before + 1),
            }
            assert_lane_invariants(&board);
        }
    }

    #[test]
    fn pressing_lanes_from_the_top_down_removes_only_the_bottom_row() {
        // 下から: 列0, 列1, 列2, 列3 を上から(逆順に)押すと、最下段の列0以外は全部ミスになる
        let mut board = Board::lanes([0, 1, 2, 3]);
        let outcomes: Vec<PressOutcome> = [3, 2, 1, 0]
            .into_iter()
            .map(|column| press_lane(&mut board, column))
            .collect();
        assert_eq!(
            outcomes,
            vec![
                PressOutcome::Added,
                PressOutcome::Added,
                PressOutcome::Added,
                PressOutcome::Removed
            ]
        );
        assert_eq!(lane_of_each_row(&board), vec![1, 2, 3, 3, 2, 1]);
        assert_lane_invariants(&board);
    }

    // --- 正解: 即座に補充する ---

    #[test]
    fn correct_press_refills_the_cleared_lane_on_top_immediately() {
        for column in 0..LANE_COUNT {
            let mut game = ColorStackGame::new();
            let (board, _) = board_with_bottom_lane(column);
            game.board = board;
            let heights_before = lane_heights(&game.board);
            press_key(&mut game, key_for_lane(column));
            assert_eq!(
                game.board,
                board_after_clearing_bottom_lane(column),
                "列{column}: 最下段が消えて残りが詰まり、一番上に同じ列が補充される"
            );
            assert_eq!(
                lane_heights(&game.board),
                heights_before,
                "列{column}: どの列の高さも変わらない"
            );
            assert_lane_invariants(&game.board);
        }
    }

    #[test]
    fn correct_press_keeps_the_total_height_constant_forever() {
        let mut game = ColorStackGame::new();
        for i in 0..500 {
            press_bottom(&mut game);
            assert_eq!(
                game.board.block_count(),
                INITIAL_HEIGHT,
                "{i}回目: 消しても補充されるので段数は一定"
            );
            assert_eq!(game.board.rows.len(), INITIAL_HEIGHT);
            assert_lane_invariants(&game.board);
        }
        assert!(!game.board.rows.is_empty(), "盤面は空にならない");
        assert!(!game.is_finished(), "消し続けても時間内なら続く");
        assert_eq!(game.result().total, 500);
    }

    #[test]
    fn correct_press_is_recorded_as_correct_with_correct_feedback() {
        let mut game = ColorStackGame::new();
        press_bottom(&mut game);
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
        assert_eq!(
            game.feedback.current().map(|f| f.verdict),
            Some(Verdict::Correct)
        );
    }

    // --- ミス: 1段増えるが詰まない ---

    #[test]
    fn miss_adds_one_row_to_the_pressed_lane_and_is_not_recorded() {
        for column in 1..LANE_COUNT {
            let mut game = ColorStackGame::new();
            game.board = Board::lanes([0, 1, 2, 3]);
            press_key(&mut game, key_for_lane(column));
            assert_eq!(
                lane_of_each_row(&game.board),
                vec![0, 1, 2, 3, column],
                "列{column}: 何も消えず、押した列が一番上に1個増える"
            );
            assert_eq!(
                game.feedback.current().map(|f| f.verdict),
                Some(Verdict::Incorrect),
                "列{column}: 順番が来ていない列はミス"
            );
            assert_eq!(game.result().total, 0, "ミスはスコアに記録しない");
        }
    }

    #[test]
    fn many_misses_grow_the_board_but_never_finish_the_game() {
        let mut game = ColorStackGame::new();
        for i in 1..=300 {
            press_wrong(&mut game);
            assert_eq!(
                game.board.block_count(),
                INITIAL_HEIGHT + i,
                "{i}回目のミスで1段増える"
            );
            assert!(!game.is_finished(), "{i}回目: ミスで詰みにはならない");
            assert!(game.is_playing());
        }
        assert_lane_invariants(&game.board);
        assert_eq!(game.result().total, 0, "ミスは記録しない");
        // 伸びた盤面でも最下段を押せば消せる
        press_bottom(&mut game);
        assert_eq!(game.result().correct, 1);
        assert_eq!(
            game.board.block_count(),
            INITIAL_HEIGHT + 300,
            "補充で高さは保つ"
        );
    }

    #[test]
    fn miss_on_an_empty_lane_adds_a_block_to_that_lane_only() {
        let mut game = ColorStackGame::new();
        game.board = Board::lanes([0, 1]);
        press_key(&mut game, key_for(Yellow));
        assert_eq!(
            lane_of_each_row(&game.board),
            vec![0, 1, 2],
            "黄の列の最上段に追加"
        );
        assert_lane_invariants(&game.board);
        assert_eq!(
            game.feedback.current().map(|f| f.verdict),
            Some(Verdict::Incorrect)
        );
        assert_eq!(game.result().total, 0);
    }

    #[test]
    fn pressing_from_the_top_down_misses_then_clears_and_refills_the_bottom() {
        let mut game = ColorStackGame::new();
        // 下から: 黄, 赤, 緑, 青
        game.board = Board::lanes([2, 0, 3, 1]);
        // 上の段から順に押す: 青・緑・赤はミスで一番上に増え、最後の黄だけ最下段と一致して消え、一番上に補充される
        for color in [Blue, Green, Red, Yellow] {
            press_key(&mut game, key_for(color));
        }
        assert_eq!(lane_of_each_row(&game.board), vec![0, 3, 1, 1, 3, 0, 2]);
        let result = game.result();
        assert_eq!(result.total, 1, "消せた1個だけ記録される");
        assert_eq!(result.correct, 1);
        assert_lane_invariants(&game.board);
    }

    #[test]
    fn keys_beyond_color_count_and_other_keys_are_ignored() {
        let mut game = ColorStackGame::new();
        let before = game.board.clone();
        for code in [
            KeyCode::Char('5'),
            KeyCode::Char('0'),
            KeyCode::Char('a'),
            KeyCode::Enter,
            KeyCode::Left,
        ] {
            game.handle_key(KeyEvent::from(code));
        }
        assert_eq!(game.board, before);
        assert!(game.feedback.current().is_none());
        assert_eq!(game.result().total, 0);
    }

    // --- スコア ---

    #[test]
    fn result_correct_always_equals_total_even_with_misses() {
        let mut game = ColorStackGame::new();
        let mut rng = StdRng::seed_from_u64(3);
        let mut clears = 0;
        for _ in 0..400 {
            game.update(Duration::from_millis(50));
            if rng.gen_bool(0.4) {
                press_wrong(&mut game);
            } else {
                press_bottom(&mut game);
                clears += 1;
            }
        }
        let result = game.result();
        assert_eq!(result.total, clears, "消せた数だけ記録される");
        assert_eq!(result.correct, result.total, "ミスは記録されない");
    }

    #[test]
    fn first_latency_is_measured_from_the_start_of_the_session() {
        let mut game = ColorStackGame::new();
        game.update(Duration::from_millis(1500));
        press_bottom(&mut game);
        assert_latency(&game.result(), 1500.0);
    }

    #[test]
    fn latency_is_the_time_since_the_previous_clear() {
        let mut game = ColorStackGame::new();
        for ms in [1500, 500, 1000] {
            game.update(Duration::from_millis(ms));
            press_bottom(&mut game);
        }
        let result = game.result();
        assert_eq!(result.total, 3);
        assert_latency(&result, 1000.0);
    }

    #[test]
    fn misses_do_not_reset_the_latency_timer() {
        let mut game = ColorStackGame::new();
        game.update(Duration::from_millis(400));
        press_wrong(&mut game);
        game.update(Duration::from_millis(600));
        press_bottom(&mut game);
        assert_latency(&game.result(), 1000.0);
    }

    #[test]
    fn clearing_resets_the_latency_timer() {
        let mut game = ColorStackGame::new();
        game.update(Duration::from_millis(700));
        press_bottom(&mut game);
        assert!(
            game.elapsed_since_clear.is_zero(),
            "消した直後は0から数え直す"
        );
        assert_eq!(
            game.elapsed_total,
            Duration::from_millis(700),
            "全体の経過は続く"
        );
    }

    // --- 制限時間 ---

    #[test]
    fn game_finishes_when_the_time_limit_passes() {
        let mut game = ColorStackGame::new();
        game.update(TIME_LIMIT - Duration::from_millis(1));
        assert!(!game.is_finished(), "制限時間ちょうど手前ではまだ続く");
        assert!(game.is_playing());
        game.update(Duration::from_millis(1));
        assert!(game.is_finished(), "60秒で終了");
        assert!(!game.is_playing());
    }

    #[test]
    fn small_ticks_add_up_to_the_time_limit() {
        let mut game = ColorStackGame::new();
        for i in 1..=600 {
            assert!(!game.is_finished(), "{i}回目の前はまだ続く");
            game.update(Duration::from_millis(100));
        }
        assert!(game.is_finished(), "100ms×600回=60秒で終了");
    }

    #[test]
    fn time_up_records_nothing() {
        let mut game = ColorStackGame::new();
        game.update(Duration::from_millis(800));
        press_bottom(&mut game);
        game.update(TIME_LIMIT + Duration::from_secs(15));
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(result.total, 1, "時間切れ自体は記録しない");
        assert_eq!(result.correct, 1);
        assert_latency(&result, 800.0);
    }

    #[test]
    fn cleared_count_keeps_growing_until_time_up() {
        let mut game = ColorStackGame::new();
        let mut counts = Vec::new();
        loop {
            game.update(Duration::from_millis(250));
            if game.is_finished() {
                break;
            }
            press_bottom(&mut game);
            counts.push(game.result().total);
        }
        // 250msごとに消すと、60秒になる直前(59.75秒)までの239回消せる
        assert_eq!(counts.len(), 239);
        assert!(
            counts.windows(2).all(|w| w[1] == w[0] + 1),
            "消すたびに1ずつ増え続ける"
        );
        let result = game.result();
        assert_eq!(result.total, 239);
        assert_eq!(result.correct, 239);
        assert_latency(&result, 250.0);
        assert_eq!(game.board.block_count(), INITIAL_HEIGHT);
    }

    #[test]
    fn inputs_and_time_after_session_finished_are_ignored() {
        let mut game = ColorStackGame::new();
        game.update(TIME_LIMIT);
        assert!(game.is_finished());
        game.board = sample_board();
        press_key(&mut game, key_for(Red));
        press_key(&mut game, key_for(Blue));
        game.update(Duration::from_secs(600));
        let button = buttons_area(AREA);
        game.handle_mouse(left_click(button.x + 1, button.y + 1), AREA);
        assert_eq!(game.board, sample_board());
        assert_eq!(game.result().total, 0);
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
        for column in 0..LANE_COUNT {
            let mut game = ColorStackGame::new();
            let (board, _) = board_with_bottom_lane(column);
            game.board = board;
            let buffer = render_buffer(&game, AREA.width, AREA.height);
            let (x, y) = button_label_position(&buffer, StackColor::ALL[column]);
            game.handle_mouse(left_click(x, y), AREA);
            assert_eq!(
                game.board,
                board_after_clearing_bottom_lane(column),
                "列{column}(最下段)のブロックが消えて補充される"
            );
        }
    }

    #[test]
    fn clicking_edges_of_each_button_band_presses_that_color() {
        let area = buttons_area(AREA);
        let bands = crate::game::theme::column_bands(area, 4);
        for (column, band) in bands.iter().enumerate() {
            for x in [band.x, band.right() - 1] {
                let mut game = ColorStackGame::new();
                let (board, _) = board_with_bottom_lane(column);
                game.board = board;
                game.handle_mouse(left_click(x, band.y), AREA);
                assert_eq!(
                    game.board,
                    board_after_clearing_bottom_lane(column),
                    "列{column} x={x}"
                );
            }
        }
    }

    #[test]
    fn clicks_outside_buttons_or_non_left_clicks_do_nothing() {
        let mut game = ColorStackGame::new();
        let before = game.board.clone();
        // HUD・盤面のクリック
        game.handle_mouse(left_click(1, 1), AREA);
        let board = board_area(AREA);
        for y in board.y..board.bottom() {
            for x in [board.x + 1, board.x + board.width / 2, board.right() - 2] {
                game.handle_mouse(left_click(x, y), AREA);
            }
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
        assert_eq!(game.board, before);
    }

    // --- 盤面の配置 ---

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
    fn lane_grid_has_four_side_by_side_columns_above_their_buttons() {
        let board = board_area(AREA);
        let grid = grid_for(board);
        let bands = crate::game::theme::column_bands(buttons_area(AREA), 4);
        assert!(
            grid_for(board_area(TALL_AREA)).visible_rows >= INITIAL_HEIGHT,
            "縦長の画面なら初期の高さは全段描ける"
        );
        let mut previous: Option<Rect> = None;
        for (column, band) in bands.iter().enumerate() {
            let cell = grid.cell_rect(column, 0).expect("各列の最下段は描ける");
            assert_eq!(cell.bottom(), board.bottom(), "列{column}: 下端にそろう");
            assert!(cell.x >= board.x && cell.right() <= board.right());
            let center = cell.x + cell.width / 2;
            assert!(
                center >= band.x && center < band.right(),
                "列{column}はそのボタンの真上に来る"
            );
            if let Some(prev) = previous {
                assert!(prev.right() < cell.x, "列どうしは重ならず、すき間がある");
                assert_eq!(prev.width, cell.width, "列の幅はそろう");
            }
            previous = Some(cell);
        }
        assert!(grid.cell_rect(LANE_COUNT, 0).is_none(), "5列目は無い");
    }

    #[test]
    fn grid_rows_are_stacked_from_the_bottom_without_gaps() {
        let board = board_area(TALL_AREA);
        let grid = grid_for(board);
        for column in 0..LANE_COUNT {
            for level in 0..grid.visible_rows {
                let rect = grid.cell_rect(column, level).expect("描ける段");
                assert!(rect.width > 0 && rect.height > 0);
                assert!(rect.y >= board.y && rect.bottom() <= board.bottom());
                if level > 0 {
                    let below = grid.cell_rect(column, level - 1).unwrap();
                    assert_eq!(rect.bottom(), below.y, "上の段は下の段のすぐ上");
                    assert_eq!(rect.x, below.x, "同じ列は縦にそろう");
                }
            }
            assert!(grid.cell_rect(column, grid.visible_rows).is_none());
        }
    }

    #[test]
    fn grid_on_short_board_draws_only_bottom_rows_that_fit() {
        let board = Rect::new(2, 3, 40, 5);
        let grid = grid_geometry(board, LANE_COUNT, 16, MAX_ROW_HEIGHT);
        assert_eq!(grid.visible_rows, 5);
        assert!(grid.cell_rect(0, 4).is_some());
        assert!(
            grid.cell_rect(0, 5).is_none(),
            "入りきらない上の段は描かない"
        );
        assert_eq!(grid.cell_rect(0, 0).unwrap().bottom(), board.bottom());
        // 盤面が空・列数より狭くても破綻しない
        let empty = grid_geometry(Rect::new(0, 0, 0, 0), LANE_COUNT, 16, MAX_ROW_HEIGHT);
        assert!(empty.cell_rect(0, 0).is_none());
        let narrow = grid_geometry(Rect::new(0, 0, 3, 10), LANE_COUNT, 12, MAX_ROW_HEIGHT);
        assert!(narrow.cell_rect(0, 0).is_none());
    }

    #[test]
    fn rows_are_one_line_on_a_standard_screen_and_two_lines_on_a_tall_screen() {
        // 標準の画面では36段は1段2行で入りきらないので1段1行になり、入るぶんだけ描く
        let board = board_area(AREA);
        let grid = grid_for(board);
        assert_eq!(grid.row_height, 1);
        assert_eq!(grid.visible_rows, board.height as usize);
        // 縦長の画面なら1段2行で全段描ける
        let tall = grid_for(board_area(TALL_AREA));
        assert_eq!(tall.row_height, 2);
        assert!(tall.visible_rows >= INITIAL_HEIGHT);
    }

    // --- 盤面の描画 ---

    /// area内でブロックの色で塗られたセルの座標
    fn painted_block_cells(buffer: &ratatui::buffer::Buffer, area: Rect) -> Vec<(u16, u16)> {
        let colors = block_bg_colors();
        (area.y..area.bottom())
            .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| colors.contains(&format!("{:?}", buffer[(x, y)].bg)))
            .collect()
    }

    /// 盤面の全マスが、ブロックがあればその本来の色で、無ければブロック以外の色で塗られていることを確かめる
    fn assert_board_cells_are_painted(game: &ColorStackGame, area: Rect) {
        let buffer = render_buffer(game, area.width, area.height);
        let grid = grid_for(board_area(area));
        let colors = block_bg_colors();
        for column in 0..LANE_COUNT {
            for level in 0..grid.visible_rows {
                let rect = grid.cell_rect(column, level).unwrap();
                let cell = game.board.rows.get(level).and_then(|row| row[column]);
                for y in rect.y..rect.bottom() {
                    for x in rect.x..rect.right() {
                        let bg = buffer[(x, y)].bg;
                        match cell {
                            Some(color) => {
                                assert_eq!(bg, color.color(), "列{column} {level}段目は本来の色")
                            }
                            None => assert!(
                                !colors.contains(&format!("{bg:?}")),
                                "列{column} {level}段目は空"
                            ),
                        }
                    }
                }
            }
        }
    }

    /// 盤面の中に灰色(theme::MUTED)の背景が1マスも無いことを確かめる
    fn assert_no_gray_on_board(game: &ColorStackGame, area: Rect) {
        let buffer = render_buffer(game, area.width, area.height);
        let board = board_area(area);
        for y in board.y..board.bottom() {
            for x in board.x..board.right() {
                assert_ne!(
                    buffer[(x, y)].bg,
                    theme::MUTED,
                    "({x},{y})に灰色のブロックは無い"
                );
            }
        }
    }

    #[test]
    fn board_cells_are_painted_with_their_own_colors() {
        for area in [AREA, TALL_AREA] {
            let mut game = ColorStackGame::new();
            assert_board_cells_are_painted(&game, area);
            // 消して補充した後・ミスで伸びた後も本来の色で描く
            press_bottom(&mut game);
            press_wrong(&mut game);
            assert_board_cells_are_painted(&game, area);
        }
    }

    #[test]
    fn block_colors_are_never_hidden_in_gray() {
        for area in [AREA, TALL_AREA] {
            assert_no_gray_on_board(&ColorStackGame::new(), area);
        }
    }

    #[test]
    fn lane_board_paints_blocks_only_inside_the_lanes() {
        let mut game = ColorStackGame::new();
        game.board = Board::lanes([0, 1, 2, 3, 3, 0]);
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_for(board_area(AREA));
        let painted = painted_block_cells(&buffer, board_area(AREA));
        assert!(!painted.is_empty());
        for (x, _) in painted {
            let lane = (0..LANE_COUNT)
                .find(|&c| {
                    let cell = grid.cell_rect(c, 0).unwrap();
                    x >= cell.x && x < cell.right()
                })
                .unwrap_or_else(|| panic!("x={x}はどの列にも入っていない"));
            assert!(lane < LANE_COUNT);
        }
    }

    #[test]
    fn lowest_block_of_each_lane_shows_its_key() {
        let mut game = ColorStackGame::new();
        // 下から: 列2, 列0, 列2, 列1(列3は空)
        game.board = Board::lanes([2, 0, 2, 1]);
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_for(board_area(AREA));
        let row_text = |column: usize, level: usize| -> String {
            let rect = grid.cell_rect(column, level).unwrap();
            (rect.y..rect.bottom())
                .flat_map(|y| (rect.x..rect.right()).map(move |x| (x, y)))
                .map(|(x, y)| buffer[(x, y)].symbol().to_string())
                .collect()
        };
        assert!(row_text(2, 0).contains('3'), "列2の一番下に3キー");
        assert!(row_text(0, 1).contains('1'), "列0の一番下に1キー");
        assert!(row_text(1, 3).contains('2'), "列1の一番下に2キー");
        assert!(
            !row_text(2, 2).contains('3'),
            "一番下でない列2のブロックには出さない"
        );
    }

    #[test]
    fn two_row_blocks_have_a_separator_line_on_their_bottom_row() {
        // 縦長の画面なら1段2行で描ける。同じ色が縦に続いても1個ずつ見分けられるよう、各ブロックの下の行に線を引く
        let mut game = ColorStackGame::new();
        game.board = Board::lanes([0, 0, 0]);
        let buffer = render_buffer(&game, TALL_AREA.width, TALL_AREA.height);
        let grid = grid_for(board_area(TALL_AREA));
        assert_eq!(grid.row_height, 2);
        for level in 0..3 {
            let rect = grid.cell_rect(0, level).unwrap();
            let bottom_row = rect.bottom() - 1;
            for x in rect.x..rect.right() {
                let cell = &buffer[(x, bottom_row)];
                assert_eq!(cell.symbol(), "▁", "{level}段目の下の行");
                assert_eq!(cell.bg, Red.color(), "線の行も背景はブロックの色");
            }
        }
        // 最下段のブロックには数字キーを重ねる(上の行)
        let rect = grid.cell_rect(0, 0).unwrap();
        let top_row: String = (rect.x..rect.right())
            .map(|x| buffer[(x, rect.y)].symbol().to_string())
            .collect();
        assert!(top_row.contains('1'), "赤は1キー");
    }

    #[test]
    fn board_drawing_follows_removal_refill_and_addition() {
        let mut game = ColorStackGame::new();
        // 下から: 赤,青,青,黄,緑
        game.board = sample_board();
        press_key(&mut game, key_for(Red));
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_for(board_area(AREA));
        // 赤が消えて青が最下段に下りてくる。一番上(level 4)には赤が補充される
        let cell = grid.cell_rect(1, 0).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Blue.color());
        let cell = grid.cell_rect(0, 4).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Red.color(), "補充された赤");
        for column in 0..LANE_COUNT {
            let cell = grid.cell_rect(column, 5).unwrap();
            assert!(
                !block_bg_colors().contains(&format!("{:?}", buffer[(cell.x, cell.y)].bg)),
                "列{column}: 6段目は空のまま"
            );
        }

        // 最下段は青なので赤はミスで、赤の列の一番上(level 5)に追加される
        press_key(&mut game, key_for(Red));
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let cell = grid.cell_rect(0, 5).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Red.color());
        let cell = grid.cell_rect(1, 0).unwrap();
        assert_eq!(
            buffer[(cell.x, cell.y)].bg,
            Blue.color(),
            "最下段は変わらない"
        );
    }

    #[test]
    fn initial_blocks_are_all_drawn_on_a_tall_screen() {
        let game = ColorStackGame::new();
        let buffer = render_buffer(&game, TALL_AREA.width, TALL_AREA.height);
        let grid = grid_for(board_area(TALL_AREA));
        for (level, row) in game.board.rows.iter().enumerate() {
            let column = row.iter().position(|c| c.is_some()).unwrap();
            let cell = grid.cell_rect(column, level).unwrap();
            assert!(
                block_bg_colors().contains(&format!("{:?}", buffer[(cell.x, cell.y)].bg)),
                "level{level}も描かれる"
            );
        }
    }

    #[test]
    fn initial_blocks_on_a_standard_screen_are_drawn_from_the_bottom_as_far_as_they_fit() {
        // 標準の画面では36段は入りきらず、盤面の高さぶんだけ下から描く(上の段は描かない)
        let game = ColorStackGame::new();
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let board = board_area(AREA);
        let grid = grid_for(board);
        assert!(grid.visible_rows < INITIAL_HEIGHT);
        let painted_rows: HashSet<u16> = painted_block_cells(&buffer, board)
            .into_iter()
            .map(|(_, y)| y)
            .collect();
        assert_eq!(
            painted_rows.len(),
            board.height as usize,
            "盤面の全行にブロックが描かれる"
        );
        let above_board = Rect::new(0, 0, AREA.width, board.y);
        assert!(
            painted_block_cells(&buffer, above_board).is_empty(),
            "盤面より上にははみ出さない"
        );
    }

    #[test]
    fn board_overflowing_the_panel_draws_bottom_rows_only_without_panic() {
        let mut game = ColorStackGame::new();
        // 最下段は赤、その上に緑が300段(ミスを重ねて伸びた状態)
        game.board = Board::lanes(std::iter::once(0).chain(std::iter::repeat_n(3, 300)));
        assert_eq!(game.board.block_count(), 301);
        assert!(game.is_playing(), "伸びてもゲームは続く");

        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let board = board_area(AREA);
        let grid = grid_for(board);
        assert!(
            grid.visible_rows < game.board.rows.len(),
            "盤面に収まらない高さ"
        );
        let cell = grid.cell_rect(0, 0).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Red.color(), "最下段は元のまま");
        // はみ出た段は盤面より上(HUD・盤面の枠)に描かない
        let above_board = Rect::new(0, 0, AREA.width, board.y);
        assert!(painted_block_cells(&buffer, above_board).is_empty());

        // 狭い画面でも描画がパニックしない
        for (w, h) in [(1, 1), (3, 8), (20, 6), (30, 10), (100, 12), (200, 60)] {
            rendered_text(&game, w, h);
        }
        // 伸びた盤面でも、最下段を押せば消せる
        press_key(&mut game, key_for(Red));
        assert_eq!(game.result().correct, 1);
    }

    // --- HUD・その他の描画 ---

    #[test]
    fn hud_shows_cleared_count_remaining_seconds_and_remaining_blocks() {
        let mut game = ColorStackGame::new();
        game.board = sample_board();
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("消せた0枚"), "{text}");
        assert!(text.contains("残り60秒"), "開始時は制限時間いっぱい");
        assert!(text.contains("残り5個"));

        game.update(Duration::from_millis(12_300));
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("残り48秒"), "残り47.7秒は切り上げて表示");

        press_key(&mut game, key_for(Red));
        game.update(FEEDBACK_HOLD);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("消せた1枚"), "消すと増える");
        assert!(text.contains("残り5個"), "消しても補充されるので変わらない");
        assert!(text.contains("残り47秒"));

        press_key(&mut game, key_for(Red));
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("残り6個"), "ミスで追加されると増える");
        assert!(text.contains("消せた1枚"), "ミスは数えない");
    }

    #[test]
    fn hud_shows_one_second_left_just_before_time_up() {
        let mut game = ColorStackGame::new();
        game.update(TIME_LIMIT - Duration::from_millis(1));
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("残り1秒"));
    }

    #[test]
    fn hud_shows_feedback_instead_of_remaining_seconds_while_flashing() {
        let mut game = ColorStackGame::new();
        press_wrong(&mut game);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("+1段"), "ミスの表示を優先する");
        assert!(!text.contains("残り60秒"));
        game.update(FEEDBACK_HOLD);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(!text.contains("+1段"), "表示時間を過ぎたら消える");
        assert!(
            text.contains("残り60秒"),
            "残り秒数の表示に戻る(59.1秒は切り上げ)"
        );
    }

    #[test]
    fn hud_does_not_show_rounds_or_difficulty() {
        let game = ColorStackGame::new();
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("シタケシ"));
        assert!(!text.contains("ROUND"), "ラウンド制は無い");
        for difficulty in ["初級", "中級", "上級"] {
            assert!(!text.contains(difficulty), "難易度は表示しない");
        }
    }

    #[test]
    fn hud_counts_remaining_blocks_on_the_lane_board() {
        let mut game = ColorStackGame::new();
        game.board = Board::lanes([0, 1, 2]);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("残り3個"), "空白セルは数えない");
    }

    #[test]
    fn render_does_not_panic_in_tiny_areas() {
        let mut game = ColorStackGame::new();
        for (w, h) in [(1, 1), (3, 8), (20, 6), (30, 10), (100, 12)] {
            rendered_text(&game, w, h);
        }
        game.update(TIME_LIMIT);
        for (w, h) in [(1, 1), (20, 6), (100, 36)] {
            rendered_text(&game, w, h);
        }
    }
}
