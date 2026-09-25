//! カラーストック: 積まれた色ブロックを、色ボタンで「一番下」から消していくタイムアタック。
//!
//! 1セッション=3ラウンドで、ラウンドごとに盤面の形とルールが決まっている(難易度選択は無い)。
//! - ROUND1: 4列。各列は専用の色で、1段に1列だけブロックがある(同時押しは無い)。
//!   盤面の最下段にある色を押した時だけ最下段が消え、それ以外の色はミスでその列の一番上に1個追加される
//! - ROUND2: 1列。最下段と同じ色を押せば消え、違えばパレットからランダムな色が一番上に積まれる
//! - ROUND3: ROUND2と同じ1列のルールで、1段の高さを半分にして初期のブロック数を倍にする
//!
//! 盤面を空にしたらラウンドクリア。制限時間を超えたラウンドは失敗として記録する。

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

/// 結果に記録する難易度。カラーストックは難易度を選ばないので固定値にする
/// (ROUND2/3が現行の中級相当のパラメータを使うため中級とする)
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Intermediate;

/// 1セッションのラウンド数
pub const ROUNDS_PER_SESSION: u32 = 3;

/// ラウンド終了から次のラウンド開始までの間隔。この間は入力を受け付けない
pub const ROUND_INTERVAL: Duration = Duration::from_millis(1200);

/// 色ボタンを並べる領域の高さ(枠込み)
const BUTTONS_HEIGHT: u16 = 3;

/// マスの幅の上限。広い画面でもブロックが横に伸びすぎないようにする
const MAX_CELL_WIDTH: u16 = 16;

/// 1段の高さの上限(ROUND1/2)。低い積み上げでも縦に伸びすぎないようにする
const MAX_ROW_HEIGHT: u16 = 2;

/// ROUND1の列数。列iの専用色はStackColor::ALL[i]
const LANE_COUNT: usize = StackColor::ALL.len();

/// ROUND2(1列)の初期の高さ。ROUND3はこの2倍にする
const SINGLE_COLUMN_HEIGHT: usize = 12;

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

/// ラウンドの盤面の形と、色ボタンを押した時のルール
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackRule {
    /// 4列で列ごとに専用色。押した色の列の一番下のブロックを消し、無ければその列の一番上に追加する
    Lanes,
    /// 1列。最下段と一致すれば消し、不一致ならパレットからランダムな色を一番上に積む
    SingleColumn,
}

/// ラウンドごとのパラメータ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoundParams {
    pub rule: StackRule,
    /// 初期の段数(1段に1個なので初期のブロック数と同じ)
    pub height: usize,
    /// 使う色の数(StackColor::ALLの先頭から)。ボタンの数でもある
    pub color_count: usize,
    /// 1ラウンドの制限時間
    pub time_limit: Duration,
    /// 1段の表示上の高さの上限
    pub max_row_height: u16,
    /// HUD・ラウンド間の案内に出す、このラウンドの盤面の説明
    pub label: &'static str,
}

/// round_index番目(0始まり)のラウンドのパラメータ。最終ラウンドより先は最終ラウンドのまま
pub fn round_params(round_index: u32) -> RoundParams {
    match round_index {
        0 => RoundParams {
            rule: StackRule::Lanes,
            height: 12,
            color_count: LANE_COUNT,
            time_limit: Duration::from_secs(90),
            max_row_height: MAX_ROW_HEIGHT,
            label: "4列",
        },
        // 現行の中級相当
        1 => RoundParams {
            rule: StackRule::SingleColumn,
            height: SINGLE_COLUMN_HEIGHT,
            color_count: 4,
            time_limit: Duration::from_secs(90),
            max_row_height: MAX_ROW_HEIGHT,
            label: "1列",
        },
        // ROUND2を薄く・倍量にしたもの。制限時間は現行の上級相当
        _ => RoundParams {
            rule: StackRule::SingleColumn,
            height: SINGLE_COLUMN_HEIGHT * 2,
            color_count: 4,
            time_limit: Duration::from_secs(120),
            max_row_height: MAX_ROW_HEIGHT / 2,
            label: "1列 倍量",
        },
    }
}

impl RoundParams {
    /// このラウンドで使う色(ボタンの並び順)
    fn colors(&self) -> &'static [StackColor] {
        &StackColor::ALL[..self.color_count]
    }

    /// 盤面の列数
    fn columns(&self) -> usize {
        match self.rule {
            StackRule::Lanes => self.color_count,
            StackRule::SingleColumn => 1,
        }
    }
}

/// 盤面。rows[0]が最下段で、各段は列数ぶんのセル(Noneは空白)。
/// どのルールでも「ブロックが1個も無い段」は残さない(消えた段は上の段が詰めて下りてくる)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Board {
    columns: usize,
    rows: Vec<Vec<Option<StackColor>>>,
}

impl Board {
    /// 下から順にcolorsを積んだ1列の盤面
    fn single_column(colors: impl IntoIterator<Item = StackColor>) -> Self {
        Self {
            columns: 1,
            rows: colors.into_iter().map(|c| vec![Some(c)]).collect(),
        }
    }

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

    /// 全部消えたか
    fn is_cleared(&self) -> bool {
        self.rows.is_empty()
    }

    /// 最下段にあるブロックの色(左の列から探す)
    fn bottom_color(&self) -> Option<StackColor> {
        self.rows.first()?.iter().flatten().next().copied()
    }
}

/// column列目にだけその列の専用色を置いた、4列盤面の1段
fn lane_row(column: usize) -> Vec<Option<StackColor>> {
    (0..LANE_COUNT)
        .map(|c| (c == column).then_some(StackColor::ALL[c]))
        .collect()
}

/// ラウンド開始時の盤面を作る。
/// 先頭の色数(列数)ぶんに各色(各列)を1個ずつ置き、残りをランダムにしてから全体をシャッフルするので、
/// 使う色(列)はどれも最低1個は含まれる
fn new_board(rng: &mut impl Rng, params: &RoundParams) -> Board {
    let choices = params.color_count;
    let mut picks: Vec<usize> = (0..params.height)
        .map(|i| {
            if i < choices {
                i
            } else {
                rng.gen_range(0..choices)
            }
        })
        .collect();
    picks.shuffle(rng);
    match params.rule {
        StackRule::Lanes => Board::lanes(picks),
        StackRule::SingleColumn => {
            Board::single_column(picks.into_iter().map(|i| StackColor::ALL[i]))
        }
    }
}

/// 色ボタンを押した結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressOutcome {
    /// ブロックが1個消えた
    Removed,
    /// 消せるブロックが無く、一番上に1個追加された(ミス)
    Added,
}

/// 4列盤面でcolumn列目のボタンを押した時の処理。
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

/// 1列盤面でcolorを押した時の処理。最下段がcolorなら消して詰め、違えばpaletteからランダムに
/// 選んだ色を一番上に積む(ミスの度に同じ色が積み上がり続けるのを避けるため)
fn press_single(
    rng: &mut impl Rng,
    board: &mut Board,
    color: StackColor,
    palette: &[StackColor],
) -> PressOutcome {
    if board.bottom_color() == Some(color) {
        // 最下段を取り除くと、残りのブロックは1段ずつ下にずれる(重力)
        board.rows.remove(0);
        PressOutcome::Removed
    } else {
        board
            .rows
            .push(vec![Some(palette[rng.gen_range(0..palette.len())])]);
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
/// 縦は最下段を盤面の下端にそろえる。1列なら盤面の横中央に1列だけ置く。
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

/// 1ラウンドの状態
struct Round {
    board: Board,
    /// ラウンド開始からの経過時間
    elapsed: Duration,
}

fn new_round(params: &RoundParams) -> Round {
    Round {
        board: new_board(&mut rand::thread_rng(), params),
        elapsed: Duration::ZERO,
    }
}

pub struct ColorStackGame {
    /// いまのラウンド(0始まり)。ラウンド間の待ち時間中は終えたラウンドのまま
    round_index: u32,
    params: RoundParams,
    round: Round,
    /// ラウンド間の待ち時間の残り。Noneならプレイ中
    interval: Option<Duration>,
    tracker: ScoreTracker,
    feedback: AnswerFeedback,
}

impl ColorStackGame {
    pub fn new() -> Self {
        let params = round_params(0);
        Self {
            round_index: 0,
            params,
            round: new_round(&params),
            interval: None,
            tracker: ScoreTracker::new(),
            feedback: AnswerFeedback::new(),
        }
    }

    /// round_index番目(0始まり)のラウンドを、そのラウンドのパラメータで始める
    fn start_round(&mut self, round_index: u32) {
        self.round_index = round_index;
        self.params = round_params(round_index);
        self.round = new_round(&self.params);
        self.interval = None;
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
        let outcome = match self.params.rule {
            // 列iのボタン=列iの専用色なので、ボタン番号がそのまま列になる
            StackRule::Lanes => press_lane(&mut self.round.board, index),
            StackRule::SingleColumn => press_single(
                &mut rand::thread_rng(),
                &mut self.round.board,
                color,
                self.params.colors(),
            ),
        };
        if outcome == PressOutcome::Added {
            // ミス: 一番上に1個積まれた(ラウンドは続き、スコアには記録しない)
            audio::play_se(SeKind::Incorrect);
            self.feedback.record(false, "+1段");
            return;
        }
        audio::play_se(SeKind::Correct);
        if self.round.board.is_cleared() {
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
        self.start_round(self.round_index + 1);
    }

    /// いま何ラウンド目か(1始まり。待ち時間中・全ラウンド終了後は終えたラウンドのまま)
    fn current_round_number(&self) -> u32 {
        (self.round_index + 1).min(ROUNDS_PER_SESSION)
    }

    fn render_hud(&self, frame: &mut Frame, area: Rect) {
        let block = theme::panel(" ◆ カラーストック ")
            .border_style(Style::default().fg(theme::flash_border_color(self.feedback.current())))
            .title(
                Line::from(Span::styled(
                    format!(" {} ", self.params.label),
                    Style::default()
                        .fg(theme::ACCENT)
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
                format!("{}個", self.round.board.block_count()),
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
        let grid = grid_geometry(
            board,
            self.params.columns(),
            self.params.height,
            self.params.max_row_height,
        );
        // 数字キーを重ねた列(列ごとに一番下のブロックにだけ出す)
        let mut labeled = vec![false; self.round.board.columns];
        // 入りきらない上の段は描かない
        for (level, row) in self
            .round
            .board
            .rows
            .iter()
            .enumerate()
            .take(grid.visible_rows)
        {
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
        // 盤面を消し切っていればクリア、そうでなければ時間切れ
        let (headline, color) = if self.round.board.is_cleared() {
            ("CLEAR!", theme::CORRECT)
        } else {
            ("TIME UP", theme::INCORRECT)
        };
        let next_index = self.round_index + 1;
        let next_round = (next_index + 1).min(ROUNDS_PER_SESSION);
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
            Line::from(Span::styled(
                round_params(next_index).label,
                Style::default().fg(theme::ACCENT),
            )),
        ];
        let area = theme::vertical_center(board, lines.len() as u16);
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
    }

    /// 盤面パネルの見出し
    fn panel_title(&self) -> &'static str {
        match self.params.rule {
            StackRule::Lanes => " 各列の一番下の色を押して全部消す ",
            StackRule::SingleColumn => " 一番下の色を押して全部消す ",
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

        let block = theme::focus_panel(self.panel_title(), self.feedback.current());
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
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
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

    /// ラウンドの番号(0始まり)
    const ROUND1: u32 = 0;
    const ROUND2: u32 = 1;
    const ROUND3: u32 = 2;
    const ALL_ROUNDS: [u32; 3] = [ROUND1, ROUND2, ROUND3];

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

    /// 指定ラウンドから始めたゲーム
    fn game_at(round_index: u32) -> ColorStackGame {
        let mut game = ColorStackGame::new();
        game.start_round(round_index);
        game
    }

    /// 確認しやすい1列の並び(下から: 赤,青,青,黄,緑)
    fn sample_column() -> Vec<StackColor> {
        vec![Red, Blue, Blue, Yellow, Green]
    }

    /// 確認しやすい1列の盤面(下から: 赤,青,青,黄,緑)
    fn sample_board() -> Board {
        Board::single_column(sample_column())
    }

    /// 1列の盤面の色を下から並べる
    fn column_colors(board: &Board) -> Vec<StackColor> {
        assert_eq!(board.columns, 1, "1列の盤面であること");
        board.rows.iter().map(|row| row[0].unwrap()).collect()
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

    /// pressed色を押した結果(before→after)が、1列ルールの仕様通りかを検証する。
    /// 追加される色はランダムなので、具体的な色までは断定せずパレット内かどうかだけ見る
    fn assert_press_outcome(before: &Board, after: &Board, pressed: StackColor) {
        let before = column_colors(before);
        let after = column_colors(after);
        if before.first() == Some(&pressed) {
            assert_eq!(after, &before[1..], "一致: 最下段が消えて詰まる");
        } else {
            assert_eq!(
                &after[..before.len()],
                before.as_slice(),
                "不一致: 最下段より下は変わらない"
            );
            assert_eq!(after.len(), before.len() + 1, "不一致: 1個増える");
            assert!(
                StackColor::ALL.contains(after.last().unwrap()),
                "追加される色はパレット内であること"
            );
        }
    }

    /// 最下段のブロックの色をキーで押し続けて、ラウンドをクリアする。
    /// 4列でも各列の色=ボタンの色なので、同じ方法で消し切れる
    fn solve_round(game: &mut ColorStackGame) {
        while let Some(color) = game.round.board.bottom_color() {
            press_key(game, key_for(color));
        }
    }

    /// 今のラウンドをクリアし、待ち時間を終えて次のラウンドに進める
    fn clear_and_advance(game: &mut ColorStackGame) {
        solve_round(game);
        game.update(ROUND_INTERVAL);
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

    /// あるラウンドの盤面のマス目
    fn grid_for(round_index: u32, board: Rect) -> GridGeometry {
        let p = round_params(round_index);
        grid_geometry(board, p.columns(), p.height, p.max_row_height)
    }

    // --- ラウンドごとのパラメータ ---

    #[test]
    fn round_params_match_spec() {
        let r1 = round_params(ROUND1);
        assert_eq!(r1.rule, StackRule::Lanes);
        assert_eq!(r1.columns(), 4, "ROUND1は4列");
        assert_eq!(r1.color_count, 4);

        let r2 = round_params(ROUND2);
        assert_eq!(r2.rule, StackRule::SingleColumn);
        assert_eq!(r2.columns(), 1, "ROUND2は1列");
        // ROUND2は現行の中級相当
        assert_eq!(r2.height, 12);
        assert_eq!(r2.color_count, 4);
        assert_eq!(r2.time_limit, Duration::from_secs(90));
        assert_eq!(r2.max_row_height, MAX_ROW_HEIGHT);

        let r3 = round_params(ROUND3);
        assert_eq!(r3.rule, StackRule::SingleColumn);
        assert_eq!(r3.columns(), 1, "ROUND3は1列");
        assert_eq!(r3.height, r2.height * 2, "ROUND3の初期の高さはROUND2の2倍");
        assert_eq!(
            r3.max_row_height * 2,
            r2.max_row_height,
            "ROUND3の1段の高さはROUND2の半分"
        );
        assert_eq!(r3.color_count, 4);
        assert_eq!(r3.time_limit, Duration::from_secs(120));
    }

    #[test]
    fn round_params_beyond_the_last_round_stay_at_the_last_round() {
        assert_eq!(round_params(ROUNDS_PER_SESSION), round_params(ROUND3));
        assert_eq!(round_params(100), round_params(ROUND3));
    }

    #[test]
    fn each_round_has_a_distinct_label() {
        let labels: HashSet<&str> = ALL_ROUNDS.iter().map(|&r| round_params(r).label).collect();
        assert_eq!(labels.len(), ALL_ROUNDS.len());
    }

    #[test]
    fn colors_have_distinct_names_and_terminal_colors() {
        let names: HashSet<&str> = StackColor::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), StackColor::ALL.len());
        assert_eq!(block_bg_colors().len(), StackColor::ALL.len());
    }

    // --- ROUND1: 4列の初期盤面 ---

    #[test]
    fn lane_board_has_exactly_one_block_per_row() {
        let p = round_params(ROUND1);
        for seed in 0..50 {
            let board = new_board(&mut StdRng::seed_from_u64(seed), &p);
            assert_eq!(
                board.rows.len(),
                p.height,
                "seed={seed}: 初期の高さぶんの段"
            );
            assert_eq!(
                board.block_count(),
                p.height,
                "1段に1個なので段数=ブロック数"
            );
            assert_lane_invariants(&board);
        }
    }

    #[test]
    fn lane_board_uses_every_column_at_least_once() {
        let p = round_params(ROUND1);
        for seed in 0..300 {
            let board = new_board(&mut StdRng::seed_from_u64(seed), &p);
            let used: HashSet<usize> = lane_of_each_row(&board).into_iter().collect();
            assert_eq!(used.len(), LANE_COUNT, "seed={seed}: 全列が最低1回");
        }
    }

    #[test]
    fn lane_board_is_random() {
        let p = round_params(ROUND1);
        let boards: HashSet<Board> = (0..5)
            .map(|seed| new_board(&mut StdRng::seed_from_u64(seed), &p))
            .collect();
        assert!(boards.len() > 1, "シードが違えば盤面も変わる");
    }

    // --- ROUND1: 4列の消去・追加ルール ---

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
            assert!(board.is_cleared());
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
        let p = round_params(ROUND1);
        let mut rng = StdRng::seed_from_u64(7);
        let mut board = new_board(&mut rng, &p);
        for _ in 0..500 {
            let column = rng.gen_range(0..LANE_COUNT);
            let before = board.block_count();
            let bottom_lane = lane_of_each_row(&board).first().copied();
            let outcome = press_lane(&mut board, column);
            assert_eq!(
                outcome == PressOutcome::Removed,
                bottom_lane == Some(column),
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
    fn lane_board_is_cleared_when_every_lane_is_empty() {
        let mut board = Board::lanes([0, 1, 2, 3]);
        for column in [0, 1, 2] {
            assert_eq!(press_lane(&mut board, column), PressOutcome::Removed);
            assert!(!board.is_cleared());
        }
        assert_eq!(press_lane(&mut board, 3), PressOutcome::Removed);
        assert!(board.is_cleared());
        assert_eq!(board.block_count(), 0);
    }

    #[test]
    fn pressing_lanes_from_the_top_down_does_not_clear_the_board() {
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
        assert!(!board.is_cleared());
        assert_eq!(lane_of_each_row(&board), vec![1, 2, 3, 3, 2, 1]);
        assert_lane_invariants(&board);
    }

    // --- ROUND2/3: 1列の初期盤面 ---

    #[test]
    fn single_column_board_is_filled_to_height_with_allowed_colors() {
        for round in [ROUND2, ROUND3] {
            let p = round_params(round);
            let allowed: HashSet<StackColor> =
                StackColor::ALL[..p.color_count].iter().copied().collect();
            for seed in 0..20 {
                let board = new_board(&mut StdRng::seed_from_u64(seed), &p);
                assert_eq!(board.columns, 1, "round={round}: 1列");
                let colors = column_colors(&board);
                assert_eq!(colors.len(), p.height, "round={round}: 初期の高さぶん");
                assert!(colors.iter().all(|c| allowed.contains(c)), "使える色だけ");
            }
        }
    }

    #[test]
    fn single_column_board_uses_every_color_at_least_once() {
        for round in [ROUND2, ROUND3] {
            let p = round_params(round);
            for seed in 0..300 {
                let board = new_board(&mut StdRng::seed_from_u64(seed), &p);
                let used: HashSet<StackColor> = column_colors(&board).into_iter().collect();
                assert_eq!(used.len(), p.color_count, "round={round} seed={seed}");
            }
        }
    }

    #[test]
    fn round3_starts_with_twice_as_many_blocks_as_round2() {
        let r2 = game_at(ROUND2);
        let r3 = game_at(ROUND3);
        assert_eq!(
            r3.round.board.block_count(),
            r2.round.board.block_count() * 2
        );
        assert_eq!(r2.round.board.block_count(), round_params(ROUND2).height);
    }

    // --- ROUND2/3: 1列の消去・追加ルール(現行仕様) ---

    #[test]
    fn pressing_the_bottom_color_removes_it_and_drops_the_rest() {
        let mut rng = StdRng::seed_from_u64(1);
        let mut board = sample_board();
        assert_eq!(
            press_single(&mut rng, &mut board, Red, &StackColor::ALL),
            PressOutcome::Removed
        );
        assert_eq!(
            column_colors(&board),
            vec![Blue, Blue, Yellow, Green],
            "最下段の赤が消え、上のブロックが1段下に詰まる"
        );
        assert_eq!(
            press_single(&mut rng, &mut board, Blue, &StackColor::ALL),
            PressOutcome::Removed
        );
        assert_eq!(
            column_colors(&board),
            vec![Blue, Yellow, Green],
            "同じ色が続けば続けて消せる"
        );
    }

    #[test]
    fn pressing_a_different_color_adds_a_random_palette_color_on_top() {
        let mut rng = StdRng::seed_from_u64(2);
        for color in [Blue, Yellow, Green] {
            let mut board = sample_board();
            assert_eq!(
                press_single(&mut rng, &mut board, color, &StackColor::ALL),
                PressOutcome::Added
            );
            assert_press_outcome(&sample_board(), &board, color);
        }
    }

    #[test]
    fn added_color_is_not_always_the_pressed_color() {
        // 十分な試行回数で、追加される色が押した色以外にもなることを統計的に確認する
        let mut rng = StdRng::seed_from_u64(3);
        let mut seen_other_than_pressed = false;
        for _ in 0..100 {
            let mut board = sample_board();
            press_single(&mut rng, &mut board, Blue, &StackColor::ALL);
            if *column_colors(&board).last().unwrap() != Blue {
                seen_other_than_pressed = true;
                break;
            }
        }
        assert!(
            seen_other_than_pressed,
            "100回試行して押した色以外が一度も追加されないのは統計的に考えにくい"
        );
    }

    #[test]
    fn repeated_misses_keep_growing_the_column() {
        let mut rng = StdRng::seed_from_u64(4);
        let mut board = sample_board();
        for i in 1..=500 {
            assert_eq!(
                press_single(&mut rng, &mut board, Green, &StackColor::ALL),
                PressOutcome::Added
            );
            assert_eq!(board.block_count(), sample_column().len() + i);
        }
        let colors = column_colors(&board);
        assert_eq!(colors[0], Red, "最下段は変わらない");
        assert!(colors[5..].iter().all(|c| StackColor::ALL.contains(c)));
    }

    #[test]
    fn single_column_cleared_follows_the_board() {
        let mut rng = StdRng::seed_from_u64(5);
        let mut board = Board::single_column(vec![Red]);
        assert!(!board.is_cleared());
        press_single(&mut rng, &mut board, Red, &StackColor::ALL);
        assert!(board.is_cleared());
    }

    // --- セッションの進行 ---

    #[test]
    fn new_game_starts_at_round1_with_a_full_lane_board_and_no_records() {
        let game = ColorStackGame::new();
        assert_eq!(game.round_index, ROUND1);
        assert_eq!(game.params, round_params(ROUND1));
        assert_eq!(game.round.board.columns, LANE_COUNT, "ROUND1は4列");
        assert_eq!(game.round.board.block_count(), round_params(ROUND1).height);
        assert_lane_invariants(&game.round.board);
        assert!(game.round.elapsed.is_zero());
        assert!(game.interval.is_none());
        assert_eq!(game.result().total, 0);
        assert!(!game.is_finished());
    }

    #[test]
    fn session_goes_round1_then_round2_then_round3() {
        let mut game = ColorStackGame::new();
        assert_eq!(game.round.board.columns, 4);

        clear_and_advance(&mut game);
        assert_eq!(game.round_index, ROUND2);
        assert_eq!(game.params, round_params(ROUND2));
        assert_eq!(game.round.board.columns, 1, "ROUND2は1列");
        assert_eq!(game.round.board.block_count(), round_params(ROUND2).height);

        clear_and_advance(&mut game);
        assert_eq!(game.round_index, ROUND3);
        assert_eq!(game.params, round_params(ROUND3));
        assert_eq!(game.round.board.columns, 1, "ROUND3は1列");
        assert_eq!(
            game.round.board.block_count(),
            round_params(ROUND2).height * 2
        );
        assert!(!game.is_finished());

        solve_round(&mut game);
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.difficulty, SESSION_DIFFICULTY);
        assert_eq!(result.total, ROUNDS_PER_SESSION);
        assert_eq!(result.correct, ROUNDS_PER_SESSION);
    }

    #[test]
    fn session_finishes_after_three_rounds_with_mixed_outcomes() {
        let mut game = ColorStackGame::new();
        // ROUND1: クリア / ROUND2: 時間切れ / ROUND3: クリア
        clear_and_advance(&mut game);
        assert!(!game.is_finished());
        game.update(round_params(ROUND2).time_limit);
        game.update(ROUND_INTERVAL);
        assert!(!game.is_finished());
        assert_eq!(game.round_index, ROUND3, "時間切れでも次のラウンドへ進む");
        solve_round(&mut game);
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(result.total, ROUNDS_PER_SESSION);
        assert_eq!(result.correct, 2);
    }

    #[test]
    fn next_round_starts_with_a_fresh_full_board_after_interval() {
        let mut game = ColorStackGame::new();
        // ミスで伸ばしてから時間切れにしても、次のラウンドは初期の高さから
        game.round.board = Board::lanes([0]);
        for _ in 0..20 {
            press_key(&mut game, key_for(Green));
        }
        game.update(round_params(ROUND1).time_limit);
        assert!(game.interval.is_some());
        game.update(ROUND_INTERVAL);
        assert!(game.interval.is_none());
        assert!(game.round.elapsed.is_zero(), "経過時間も0から");
        assert_eq!(game.round.board.block_count(), round_params(ROUND2).height);
    }

    #[test]
    fn inputs_are_ignored_and_time_stops_during_interval() {
        let mut game = ColorStackGame::new();
        solve_round(&mut game);
        let board_before = game.round.board.clone();
        game.round.board = Board::lanes([0, 1]);
        press_key(&mut game, key_for(Red));
        press_key(&mut game, key_for(Yellow));
        assert_eq!(
            game.round.board,
            Board::lanes([0, 1]),
            "待ち時間中のキーは無視"
        );
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
    fn inputs_and_time_after_session_finished_are_ignored() {
        let mut game = ColorStackGame::new();
        for _ in 0..ROUNDS_PER_SESSION {
            clear_and_advance(&mut game);
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

    // --- ROUND1のキー入力 ---

    #[test]
    fn round1_number_key_of_the_bottom_lane_removes_the_bottom_row() {
        for column in 0..LANE_COUNT {
            let mut game = game_at(ROUND1);
            // 最下段を列columnにし、その上に残りの列を1段ずつ積む
            let others: Vec<usize> = (0..LANE_COUNT).filter(|&c| c != column).collect();
            game.round.board = Board::lanes(std::iter::once(column).chain(others.iter().copied()));
            press_key(&mut game, key_for(StackColor::ALL[column]));
            assert_eq!(
                lane_of_each_row(&game.round.board),
                others,
                "列{column}: 最下段が消え、残りが詰まる"
            );
            assert_eq!(
                game.feedback.current().map(|f| f.verdict),
                None,
                "列{column}: 消えた時はミス表示しない"
            );
            assert_eq!(game.result().total, 0);
        }
    }

    #[test]
    fn round1_number_key_of_a_lane_not_at_the_bottom_is_a_miss() {
        for column in 1..LANE_COUNT {
            let mut game = game_at(ROUND1);
            game.round.board = Board::lanes([0, 1, 2, 3]);
            press_key(&mut game, key_for(StackColor::ALL[column]));
            assert_eq!(
                lane_of_each_row(&game.round.board),
                vec![0, 1, 2, 3, column],
                "列{column}: 何も消えず、押した列が一番上に1個増える"
            );
            assert_eq!(
                game.feedback.current().map(|f| f.verdict),
                Some(crate::game::feedback::Verdict::Incorrect),
                "列{column}: 順番が来ていない列はミス"
            );
            assert_eq!(game.result().total, 0);
        }
    }

    #[test]
    fn round1_full_play_pressing_the_bottom_color_every_time_clears_without_misses() {
        for _ in 0..20 {
            let mut game = game_at(ROUND1);
            let height = game.round.board.block_count();
            let mut presses = 0;
            while let Some(color) = game.round.board.bottom_color() {
                press_key(&mut game, key_for(color));
                presses += 1;
                assert!(presses <= height, "最下段だけ押せばミスは起きない");
            }
            assert_eq!(presses, height, "1回押すごとに1段消える");
            let result = game.result();
            assert_eq!(result.total, 1);
            assert_eq!(result.correct, 1, "ROUND1クリアが記録される");
            assert!(game.interval.is_some());
        }
    }

    #[test]
    fn round1_full_play_pressing_from_the_top_down_does_not_clear() {
        let mut game = game_at(ROUND1);
        // 下から: 黄, 赤, 緑, 青
        game.round.board = Board::lanes([2, 0, 3, 1]);
        // 上の段から順に(逆順に)押す: 青・緑・赤はミスで一番上に増え、最後の黄だけ最下段と一致して消える
        for color in [Blue, Green, Red, Yellow] {
            press_key(&mut game, key_for(color));
        }
        assert!(
            !game.round.board.is_cleared(),
            "逆順に押しても盤面は空にならない"
        );
        assert_eq!(lane_of_each_row(&game.round.board), vec![0, 3, 1, 1, 3, 0]);
        assert_eq!(game.result().total, 0, "クリアは記録されない");
        assert!(game.interval.is_none(), "ラウンドは続く");
        assert_lane_invariants(&game.round.board);
    }

    #[test]
    fn round1_wrong_color_press_during_full_play_is_recorded_as_a_miss() {
        let mut game = game_at(ROUND1);
        game.round.board = Board::lanes([2, 0, 3]);
        // 最下段は黄。赤を押すとミス(赤の段が一番上に増える)
        press_key(&mut game, key_for(Red));
        assert_eq!(lane_of_each_row(&game.round.board), vec![2, 0, 3, 0]);
        assert_eq!(
            game.feedback.current().map(|f| f.verdict),
            Some(crate::game::feedback::Verdict::Incorrect)
        );
        // 以後は最下段の色を押し続ければクリアできる
        for color in [Yellow, Red, Green, Red] {
            press_key(&mut game, key_for(color));
        }
        assert!(game.round.board.is_cleared());
        assert_eq!(game.result().correct, 1);
    }

    #[test]
    fn round1_miss_on_an_empty_lane_adds_a_block_to_that_lane_only() {
        let mut game = game_at(ROUND1);
        game.round.board = Board::lanes([0, 1]);
        press_key(&mut game, key_for(Yellow));
        assert_eq!(
            lane_of_each_row(&game.round.board),
            vec![0, 1, 2],
            "黄の列の最上段に追加"
        );
        assert_lane_invariants(&game.round.board);
        assert_eq!(
            game.feedback.current().map(|f| f.verdict),
            Some(crate::game::feedback::Verdict::Incorrect),
            "ミスは不正解として表示する"
        );
        assert_eq!(game.result().total, 0, "ミスはスコアに記録しない");
        assert!(game.interval.is_none(), "ミスしてもラウンドは続く");
    }

    #[test]
    fn round1_clearing_every_lane_records_success() {
        let mut game = game_at(ROUND1);
        game.update(Duration::from_millis(1500));
        game.round.board = Board::lanes([2, 0, 3, 1]);
        for color in [Yellow, Red, Green] {
            press_key(&mut game, key_for(color));
            assert_eq!(game.result().total, 0, "まだ残っている");
        }
        press_key(&mut game, key_for(Blue));
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
        assert!((result.avg_latency_ms - 1500.0).abs() < 1e-9);
        assert!(game.interval.is_some(), "クリア後は待ち時間に入る");
    }

    #[test]
    fn round1_clearing_after_misses_still_records_success() {
        let mut game = game_at(ROUND1);
        game.round.board = Board::lanes([0]);
        press_key(&mut game, key_for(Green));
        assert_eq!(game.round.board.block_count(), 2, "ミスで1個増える");
        solve_round(&mut game);
        assert_eq!(game.result().correct, 1);
    }

    // --- ROUND2/3のキー入力 ---

    #[test]
    fn single_column_number_keys_press_the_matching_color() {
        for round in [ROUND2, ROUND3] {
            for (key, color) in ['1', '2', '3', '4'].into_iter().zip(StackColor::ALL) {
                let mut game = game_at(round);
                game.round.board = sample_board();
                let before = game.round.board.clone();
                press_key(&mut game, key);
                assert_press_outcome(&before, &game.round.board, color);
            }
        }
    }

    #[test]
    fn single_column_miss_adds_a_block_and_shows_incorrect_feedback_but_records_nothing() {
        let mut game = game_at(ROUND2);
        game.round.board = sample_board();
        let before = game.round.board.clone();
        press_key(&mut game, key_for(Yellow));
        assert_press_outcome(&before, &game.round.board, Yellow);
        assert_eq!(
            game.feedback.current().map(|f| f.verdict),
            Some(crate::game::feedback::Verdict::Incorrect)
        );
        assert_eq!(game.result().total, 0);
        assert!(game.interval.is_none());
    }

    #[test]
    fn single_column_clearing_is_recorded_only_once_at_the_moment_the_board_empties() {
        let mut game = game_at(ROUND2);
        game.round.board = Board::single_column(vec![Red, Blue]);
        press_key(&mut game, key_for(Red));
        assert_eq!(game.result().total, 0, "まだ青が残っている");
        press_key(&mut game, key_for(Blue));
        assert_eq!(game.result().total, 1);
        press_key(&mut game, key_for(Blue));
        assert_eq!(game.result().total, 1, "待ち時間中の入力で二重に記録しない");
        assert!(
            game.round.board.is_cleared(),
            "待ち時間中はミスの追加も起きない"
        );
    }

    #[test]
    fn keys_beyond_color_count_and_other_keys_are_ignored() {
        for round in ALL_ROUNDS {
            let mut game = game_at(round);
            let before = game.round.board.clone();
            for code in [
                KeyCode::Char('5'),
                KeyCode::Char('0'),
                KeyCode::Char('a'),
                KeyCode::Enter,
                KeyCode::Left,
            ] {
                game.handle_key(KeyEvent::from(code));
            }
            assert_eq!(game.round.board, before, "round={round}");
            assert!(game.feedback.current().is_none());
        }
    }

    // --- 制限時間 ---

    #[test]
    fn exceeding_time_limit_records_failure_with_limit_latency() {
        for round in ALL_ROUNDS {
            let mut game = game_at(round);
            let limit = round_params(round).time_limit;
            game.update(limit - Duration::from_millis(1));
            assert_eq!(game.result().total, 0, "制限時間ちょうど手前ではまだ続く");
            game.update(Duration::from_millis(1));
            let result = game.result();
            assert_eq!(result.total, 1, "round={round}: 制限時間でラウンド終了");
            assert_eq!(result.correct, 0);
            assert!((result.avg_latency_ms - limit.as_secs_f64() * 1000.0).abs() < 1e-9);
            assert!(game.interval.is_some());
        }
    }

    #[test]
    fn overshooting_time_limit_in_one_tick_still_records_the_limit() {
        let mut game = ColorStackGame::new();
        let limit = round_params(ROUND1).time_limit;
        game.update(limit + Duration::from_secs(15));
        let result = game.result();
        assert_eq!(result.total, 1);
        assert!((result.avg_latency_ms - limit.as_secs_f64() * 1000.0).abs() < 1e-9);
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
        for round in ALL_ROUNDS {
            for (column, &color) in StackColor::ALL.iter().enumerate() {
                let mut game = game_at(round);
                // ROUND1は、押す色の列を最下段にし、その上に残りの列を1段ずつ積む
                let others: Vec<usize> = (0..LANE_COUNT).filter(|&c| c != column).collect();
                if round == ROUND1 {
                    game.round.board =
                        Board::lanes(std::iter::once(column).chain(others.iter().copied()));
                } else {
                    game.round.board = sample_board();
                }
                let before = game.round.board.clone();
                let buffer = render_buffer(&game, AREA.width, AREA.height);
                let (x, y) = button_label_position(&buffer, color);
                game.handle_mouse(left_click(x, y), AREA);
                if round == ROUND1 {
                    assert_eq!(
                        lane_of_each_row(&game.round.board),
                        others,
                        "{color:?}の列(最下段)のブロックが消える"
                    );
                } else {
                    assert_press_outcome(&before, &game.round.board, color);
                }
            }
        }
    }

    #[test]
    fn clicking_edges_of_each_button_band_presses_that_color() {
        let area = buttons_area(AREA);
        let bands = crate::game::theme::column_bands(area, 4);
        for (band, color) in bands.iter().zip(StackColor::ALL) {
            for x in [band.x, band.right() - 1] {
                let mut game = game_at(ROUND2);
                game.round.board = sample_board();
                let before = game.round.board.clone();
                game.handle_mouse(left_click(x, band.y), AREA);
                assert_press_outcome(&before, &game.round.board, color);
            }
        }
    }

    #[test]
    fn clicks_outside_buttons_or_non_left_clicks_do_nothing() {
        for round in ALL_ROUNDS {
            let mut game = game_at(round);
            let before = game.round.board.clone();
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
            assert_eq!(game.round.board, before, "round={round}");
        }
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
    fn single_column_grid_is_one_centered_column_bottom_aligned_and_stacked() {
        for round in [ROUND2, ROUND3] {
            let height = round_params(round).height;
            let board = board_area(AREA);
            let grid = grid_for(round, board);
            assert!(
                grid.visible_rows >= height,
                "round={round}: 標準の画面なら初期の高さは全段描ける"
            );
            let bottom = grid.cell_rect(0, 0).expect("最下段は描ける");
            assert_eq!(bottom.bottom(), board.bottom(), "最下段は下端にそろう");
            let left_space = bottom.x - board.x;
            let right_space = board.right() - bottom.right();
            assert!(
                left_space.abs_diff(right_space) <= 1,
                "round={round}: 横中央"
            );
            for level in 0..grid.visible_rows {
                let rect = grid.cell_rect(0, level).expect("描ける段");
                assert!(rect.width > 0 && rect.height > 0);
                assert_eq!(rect.x, bottom.x, "全段が同じ1列に並ぶ");
                assert_eq!(rect.width, bottom.width);
                assert!(rect.y >= board.y && rect.bottom() <= board.bottom());
                if level > 0 {
                    let below = grid.cell_rect(0, level - 1).unwrap();
                    assert_eq!(rect.bottom(), below.y, "上の段は下の段のすぐ上");
                }
            }
            assert!(grid.cell_rect(0, grid.visible_rows).is_none());
            assert!(grid.cell_rect(1, 0).is_none(), "1列なので2列目は無い");
        }
    }

    #[test]
    fn lane_grid_has_four_side_by_side_columns_above_their_buttons() {
        let board = board_area(AREA);
        let grid = grid_for(ROUND1, board);
        let bands = crate::game::theme::column_bands(buttons_area(AREA), 4);
        assert!(grid.visible_rows >= round_params(ROUND1).height);
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
    fn grid_on_short_board_draws_only_bottom_rows_that_fit() {
        let board = Rect::new(2, 3, 40, 5);
        let grid = grid_geometry(board, 1, 16, MAX_ROW_HEIGHT);
        assert_eq!(grid.visible_rows, 5);
        assert!(grid.cell_rect(0, 4).is_some());
        assert!(
            grid.cell_rect(0, 5).is_none(),
            "入りきらない上の段は描かない"
        );
        assert_eq!(grid.cell_rect(0, 0).unwrap().bottom(), board.bottom());
        // 盤面が空・列数より狭くても破綻しない
        let empty = grid_geometry(Rect::new(0, 0, 0, 0), 1, 16, MAX_ROW_HEIGHT);
        assert!(empty.cell_rect(0, 0).is_none());
        let narrow = grid_geometry(Rect::new(0, 0, 3, 10), LANE_COUNT, 12, MAX_ROW_HEIGHT);
        assert!(narrow.cell_rect(0, 0).is_none());
    }

    #[test]
    fn row_height_is_fixed_by_initial_height_even_when_the_column_grows() {
        // 1段の高さは初期の高さで決め、ミスで伸びても変えない(伸びたぶんは上に積む)
        let board = board_area(AREA);
        let grid = grid_for(ROUND2, board);
        assert_eq!(grid.row_height, 2);
        assert_eq!(grid.visible_rows, (board.height / 2) as usize);
    }

    #[test]
    fn round3_rows_are_half_as_tall_as_round2_rows() {
        for area in [AREA, Rect::new(0, 0, 120, 80), Rect::new(0, 0, 200, 60)] {
            let board = board_area(area);
            let r2 = grid_for(ROUND2, board);
            let r3 = grid_for(ROUND3, board);
            assert_eq!(r2.row_height, 2, "{area:?}: ROUND2は1段2行");
            assert_eq!(r3.row_height, 1, "{area:?}: ROUND3は1段1行(半分)");
            assert!(
                r3.visible_rows >= round_params(ROUND3).height,
                "倍量でも全段描ける"
            );
        }
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

    #[test]
    fn single_column_board_is_drawn_as_a_single_column() {
        for round in [ROUND2, ROUND3] {
            let mut game = game_at(round);
            game.round.board = sample_board();
            let buffer = render_buffer(&game, AREA.width, AREA.height);
            let column = grid_for(round, board_area(AREA)).cell_rect(0, 0).unwrap();
            let painted = painted_block_cells(&buffer, board_area(AREA));
            assert!(!painted.is_empty());
            assert!(
                painted
                    .iter()
                    .all(|&(x, _)| x >= column.x && x < column.right()),
                "ブロックは1列ぶんの幅にだけ塗られる"
            );
        }
    }

    #[test]
    fn board_cells_are_painted_with_block_colors_in_every_round() {
        for round in ALL_ROUNDS {
            let game = game_at(round);
            let buffer = render_buffer(&game, AREA.width, AREA.height);
            let grid = grid_for(round, board_area(AREA));
            let colors = block_bg_colors();
            for column in 0..game.params.columns() {
                for level in 0..grid.visible_rows {
                    let rect = grid.cell_rect(column, level).unwrap();
                    let cell = game.round.board.rows.get(level).and_then(|row| row[column]);
                    for y in rect.y..rect.bottom() {
                        for x in rect.x..rect.right() {
                            let bg = buffer[(x, y)].bg;
                            match cell {
                                Some(color) => {
                                    assert_eq!(
                                        bg,
                                        color.color(),
                                        "round={round} 列{column} {level}段目"
                                    )
                                }
                                None => assert!(
                                    !colors.contains(&format!("{bg:?}")),
                                    "round={round} 列{column} {level}段目は空"
                                ),
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn lane_board_paints_blocks_only_inside_the_lanes() {
        let mut game = game_at(ROUND1);
        game.round.board = Board::lanes([0, 1, 2, 3, 3, 0]);
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_for(ROUND1, board_area(AREA));
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
        let mut game = game_at(ROUND1);
        // 下から: 列2, 列0, 列2, 列1(列3は空)
        game.round.board = Board::lanes([2, 0, 2, 1]);
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_for(ROUND1, board_area(AREA));
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
        // ROUND2は1段2行で描ける。同じ色が縦に続いても1個ずつ見分けられるよう、各ブロックの下の行に線を引く
        let mut game = game_at(ROUND2);
        game.round.board = Board::single_column(vec![Red, Red, Red]);
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_for(ROUND2, board_area(AREA));
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
    fn board_drawing_follows_removal_and_addition() {
        let mut game = game_at(ROUND2);
        game.round.board = sample_board();
        press_key(&mut game, key_for(Red));
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_for(ROUND2, board_area(AREA));
        // 赤が消えて青が最下段に下りてくる。5段目(level 4)は空く
        let cell = grid.cell_rect(0, 0).unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].bg, Blue.color());
        let cell = grid.cell_rect(0, 4).unwrap();
        assert!(!block_bg_colors().contains(&format!("{:?}", buffer[(cell.x, cell.y)].bg)));

        // ミスでパレット内のいずれかの色が一番上(level 4)に追加される
        press_key(&mut game, key_for(Red));
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let cell = grid.cell_rect(0, 4).unwrap();
        assert!(block_bg_colors().contains(&format!("{:?}", buffer[(cell.x, cell.y)].bg)));
        let cell = grid.cell_rect(0, 0).unwrap();
        assert_eq!(
            buffer[(cell.x, cell.y)].bg,
            Blue.color(),
            "最下段は変わらない"
        );
    }

    #[test]
    fn round3_draws_all_initial_blocks_on_a_standard_screen() {
        let game = game_at(ROUND3);
        let buffer = render_buffer(&game, AREA.width, AREA.height);
        let grid = grid_for(ROUND3, board_area(AREA));
        for level in 0..round_params(ROUND3).height {
            let cell = grid.cell_rect(0, level).unwrap();
            assert!(
                block_bg_colors().contains(&format!("{:?}", buffer[(cell.x, cell.y)].bg)),
                "level{level}も描かれる"
            );
        }
    }

    #[test]
    fn column_overflowing_the_board_draws_bottom_rows_only_without_panic() {
        for round in ALL_ROUNDS {
            let mut game = game_at(round);
            if round == ROUND1 {
                // 4列は同じ列を押し続けると追加と消去を繰り返すので、伸びた盤面を直接作る
                // (最下段は赤の列、その上に緑の列が300段)
                game.round.board =
                    Board::lanes(std::iter::once(0).chain(std::iter::repeat_n(3, 300)));
            } else {
                game.round.board = sample_board();
                for _ in 0..300 {
                    press_key(&mut game, key_for(Green));
                }
            }
            let expected = if round == ROUND1 {
                301
            } else {
                sample_column().len() + 300
            };
            assert_eq!(game.round.board.block_count(), expected);
            assert!(game.interval.is_none(), "伸びてもラウンドは続く");

            let buffer = render_buffer(&game, AREA.width, AREA.height);
            let board = board_area(AREA);
            let grid = grid_for(round, board);
            assert!(
                grid.visible_rows < game.round.board.rows.len(),
                "盤面に収まらない高さ"
            );
            let cell = grid.cell_rect(0, 0).unwrap();
            assert_eq!(buffer[(cell.x, cell.y)].bg, Red.color(), "最下段は元のまま");
            // はみ出た段は盤面より上(HUD・盤面の枠)に描かない
            let above_board = Rect::new(0, 0, AREA.width, board.y);
            assert!(
                painted_block_cells(&buffer, above_board).is_empty(),
                "round={round}"
            );

            // 狭い画面でも描画がパニックしない
            for (w, h) in [(1, 1), (3, 8), (20, 6), (30, 10), (100, 12), (200, 60)] {
                rendered_text(&game, w, h);
            }
            // 伸びた列でも、最下段から押していけばクリアできる
            solve_round(&mut game);
            assert_eq!(game.result().correct, 1);
        }
    }

    // --- HUD・その他の描画 ---

    #[test]
    fn hud_shows_round_elapsed_time_and_remaining_blocks() {
        let mut game = game_at(ROUND2);
        game.round.board = sample_board();
        game.update(Duration::from_millis(12_300));
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("ROUND2/3"));
        assert!(text.contains("12.3"), "経過時間");
        assert!(text.contains("90"), "制限時間");
        assert!(text.contains("残り5個"));

        press_key(&mut game, key_for(Red));
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("残り4個"), "消すと減る");

        press_key(&mut game, key_for(Red));
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("残り5個"), "ミスで追加されると増える");
    }

    #[test]
    fn hud_shows_the_round_label_instead_of_difficulty() {
        for round in ALL_ROUNDS {
            let game = game_at(round);
            let text = rendered_text(&game, AREA.width, AREA.height);
            let label = round_params(round).label.replace(' ', "");
            assert!(text.contains(&label), "round={round}: {label}を表示");
            assert!(text.contains(&format!("ROUND{}/3", round + 1)));
            for difficulty in ["初級", "中級", "上級"] {
                assert!(!text.contains(difficulty), "難易度は表示しない");
            }
        }
    }

    #[test]
    fn hud_counts_remaining_blocks_on_the_lane_board() {
        let mut game = game_at(ROUND1);
        game.round.board = Board::lanes([0, 1, 2]);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("残り3個"), "空白セルは数えない");
    }

    #[test]
    fn interval_message_tells_clear_or_time_up_and_the_next_round() {
        let mut game = ColorStackGame::new();
        solve_round(&mut game);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("CLEAR!"));
        assert!(text.contains("NEXTROUND2/3"));
        assert!(text.contains(&round_params(ROUND2).label.replace(' ', "")));

        game.update(ROUND_INTERVAL);
        game.update(round_params(ROUND2).time_limit);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("TIMEUP"));
        assert!(text.contains("NEXTROUND3/3"));
        assert!(text.contains(&round_params(ROUND3).label.replace(' ', "")));
    }

    #[test]
    fn render_does_not_panic_in_tiny_areas() {
        for round in ALL_ROUNDS {
            let mut game = game_at(round);
            for (w, h) in [(1, 1), (3, 8), (20, 6), (30, 10), (100, 12)] {
                rendered_text(&game, w, h);
            }
            solve_round(&mut game);
            rendered_text(&game, 20, 6);
        }
    }
}
