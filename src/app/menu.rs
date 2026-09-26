//! メニュー画面(Screen::Menu)。カードグリッドの配置・描画・クリック判定・タイプライター表示・
//! キー操作と、メニュー画面へ入る遷移(enter_menu / return_to_menu / quit_to_menu)

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, BgmCategory, SeKind};
use crate::game::theme;
use crate::ui::menu_icons::MenuIcons;
use crate::ui::typewriter::{self, Typewriter};

use super::menu_items::{HISTORY_ITEM_INDEX, JUKEBOX_ITEM_INDEX, MENU_DESCRIPTIONS, MENU_ITEMS};
use super::screen_layout::screen_rect;
use super::{App, Screen};

/// メニュー画面のタイプライター表示の1文字あたりの間隔。メニューは全項目で400文字以上あり、
/// 標準の間隔(typewriter::CHAR_INTERVAL)では流し切るのに10秒以上かかるため短くする
pub(super) const MENU_CHAR_INTERVAL: Duration = Duration::from_millis(10);

impl App {
    /// メニュー画面へ遷移する。既存の`self.screen = Screen::Menu`は全てこれに統一し、
    /// メニューに戻るたびに端末側でのスクロールバッファのクリアを要求する。
    /// 項目の名前・説明文はここから改めてタイプライターで流す
    pub(super) fn enter_menu(&mut self) {
        self.screen = Screen::Menu;
        self.pending_scrollback_clear = true;
        let total = typewriter::char_count(&menu_item_lines(screen_rect(self.last_area)));
        self.menu_typewriter = Typewriter::with_interval(total, MENU_CHAR_INTERVAL);
    }

    /// メニュー以外の画面で[q]を押した時にメニューへ戻る。既にメニュー用BGMが
    /// 流れていれば(タイトル画面など)曲を差し替えず、そうでなければ切り替える
    pub(super) fn quit_to_menu(&mut self) {
        let menu_tracks = audio::bgm_tracks_in(BgmCategory::Menu);
        let playing_menu_bgm = self
            .current_bgm
            .as_ref()
            .is_some_and(|name| menu_tracks.contains(name));
        if playing_menu_bgm {
            audio::play_se(SeKind::Transition);
            self.enter_menu();
        } else {
            self.return_to_menu();
        }
    }

    /// Menu画面での項目決定(キー/クリック共通)。ゲーム/ジュークボックス/履歴へ振り分ける
    pub(super) fn select_menu_item(&mut self, selected: usize) {
        if selected == HISTORY_ITEM_INDEX {
            audio::play_se(SeKind::Transition);
            self.screen = Screen::History;
        } else if selected == JUKEBOX_ITEM_INDEX {
            audio::play_se(SeKind::Transition);
            self.enter_jukebox();
        } else {
            self.start_game_item(selected);
        }
    }

    /// Menu画面に戻り、Menu用BGMに切り替える(キー/クリック共通)
    pub(super) fn return_to_menu(&mut self) {
        audio::play_se(SeKind::Transition);
        if let Some(name) = audio::random_bgm_track(BgmCategory::Menu) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        self.enter_menu();
    }

    /// Menu画面のキー操作。上下左右はカードグリッド内の移動(列数は直近の描画サイズで決まる)
    pub(super) fn handle_menu_key(&mut self, key: KeyEvent) {
        let selected = self.menu_state.selected();
        let direction = match key.code {
            KeyCode::Up => GridMove::Up,
            KeyCode::Down => GridMove::Down,
            KeyCode::Left => GridMove::Left,
            KeyCode::Right => GridMove::Right,
            KeyCode::Enter => return self.select_menu_item(selected),
            _ => return,
        };
        let columns = menu_grid(screen_rect(self.last_area)).columns;
        self.menu_state
            .select(grid_move(selected, direction, columns, MENU_ITEMS.len()));
    }

    /// Menu画面でのクリック。描画(render)と同じく余白を除いた中央の範囲(screen_rect)を基準に、
    /// クリックしたカードを選択して決定する
    pub(super) fn handle_menu_mouse(&mut self, mouse: MouseEvent) {
        let screen = screen_rect(self.last_area);
        let offset = self.menu_state.row_offset;
        if let Some(index) = menu_card_at(screen, offset, mouse.column, mouse.row) {
            self.menu_state.select(index);
            self.select_menu_item(index);
        }
    }
}

/// メニュー画面の外枠(見出し・操作説明つき)。タイプライター表示の対象外で、最初から出る
fn menu_block() -> Block<'static> {
    theme::panel(Line::from(" ◆ BRAIN TRAIN ◆ ").centered())
        .border_type(ratatui::widgets::BorderType::Double)
        .title(
            Line::from(Span::styled(
                " 脳トレ ゲーム集 ",
                Style::default().fg(theme::ACCENT),
            ))
            .right_aligned(),
        )
        .title_bottom(
            theme::hints_line(&[
                ("↑↓←→", "選択"),
                ("Enter / クリック", "決定"),
                ("q", "終了"),
            ])
            .centered(),
        )
}

/// メニューのカード1枚の最小幅(枠込み)。説明文が1行に全角12文字ほど入り、
/// アイコン(縦横比約557:397)がMENU_ICON_ROWSの高さで横に収まる幅。列数はこの幅で画面幅を割って決める
const MENU_CARD_MIN_WIDTH: u16 = 28;
/// カード1枚のアイコン部分の行数(画像を描けない時も同じ行数の空白にする)
const MENU_ICON_ROWS: u16 = 5;
/// カードの枠の内側の、テキストの左右の余白(各1セル)
const MENU_CARD_PADDING: u16 = 1;

/// メニュー画面のカードグリッドの状態(選択中のカード・スクロール位置)
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct MenuGridState {
    /// 選択中のカード(MENU_ITEMSのインデックス)
    selected: usize,
    /// 画面の一番上に出しているカードの行。renderで選択中の行が画面内に入るよう合わせる
    row_offset: usize,
}

impl MenuGridState {
    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    pub(super) fn select(&mut self, index: usize) {
        self.selected = index.min(MENU_ITEMS.len() - 1);
    }
}

/// 画面の大きさから決まるカードグリッドの配置。描画(render)・クリック判定(menu_card_at)・
/// タイプライターの文字列(menu_item_lines)・キー操作の列数は全てこれを使ってそろえる
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MenuGrid {
    /// メニューの外枠の内側
    inner: Rect,
    /// 割り切れずに余った幅を左右に分け、グリッドを中央に置くための左の余白
    x_offset: u16,
    columns: usize,
    rows: usize,
    card_width: u16,
    card_height: u16,
    /// 説明文の1行の最大表示幅(セル数)
    text_width: usize,
    /// 説明文の行数(全カード共通。一番長い説明文に合わせる)
    desc_lines: usize,
    /// 画面に収まるカードの行数(最低1行)
    visible_rows: usize,
}

impl MenuGrid {
    /// index番目のカードの範囲(スクロール位置row_offset込み)。画面外(スクロールで隠れた行・
    /// 範囲外のインデックス)ならNone。外枠の内側に収まらない分は切り詰める
    fn card_rect(&self, index: usize, row_offset: usize) -> Option<Rect> {
        if index >= MENU_ITEMS.len() {
            return None;
        }
        let (row, column) = (index / self.columns, index % self.columns);
        if row < row_offset || row >= row_offset + self.visible_rows {
            return None;
        }
        let rect = Rect::new(
            self.inner.x + self.x_offset + column as u16 * self.card_width,
            self.inner.y + (row - row_offset) as u16 * self.card_height,
            self.card_width,
            self.card_height,
        )
        .intersection(self.inner);
        (!rect.is_empty()).then_some(rect)
    }

    /// カード1枚ぶんのテキストの行数(ゲーム名1行+説明文)
    fn lines_per_card(&self) -> usize {
        1 + self.desc_lines
    }

    /// 選択中のカードの行が画面内に入るスクロール位置。currentから動かさなくて済むなら動かさず、
    /// 全行が収まる大きさなら0にする(以前の小さい画面での位置が残らないように)
    fn scroll_offset(&self, selected: usize, current: usize) -> usize {
        let row = selected / self.columns;
        let offset = current.min(self.rows - self.visible_rows);
        if row < offset {
            row
        } else if row >= offset + self.visible_rows {
            row + 1 - self.visible_rows
        } else {
            offset
        }
    }
}

fn menu_grid(area: Rect) -> MenuGrid {
    let inner = menu_block().inner(area);
    let len = MENU_ITEMS.len();
    let columns = usize::from(inner.width / MENU_CARD_MIN_WIDTH).clamp(1, len);
    let rows = len.div_ceil(columns);
    let card_width = inner.width / columns as u16;
    let x_offset = (inner.width - card_width * columns as u16) / 2;
    // 枠(左右各1セル)と余白を除いた幅。極端に狭くても全角1文字は入る幅にする
    let text_width = usize::from(card_width.saturating_sub(2 + 2 * MENU_CARD_PADDING)).max(2);
    let desc_lines = MENU_DESCRIPTIONS
        .iter()
        .map(|description| wrap_by_width(description, text_width).len())
        .max()
        .unwrap_or(1)
        .max(1);
    // 枠(上下各1行)+アイコン+ゲーム名+説明文
    let card_height = 2 + MENU_ICON_ROWS + 1 + desc_lines as u16;
    let visible_rows = usize::from(inner.height / card_height).clamp(1, rows);
    MenuGrid {
        inner,
        x_offset,
        columns,
        rows,
        card_width,
        card_height,
        text_width,
        desc_lines,
        visible_rows,
    }
}

/// textを表示幅widthごとに折り返す(1文字ずつ詰める。文字の追加・削除はしない)
fn wrap_by_width(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    for c in text.chars() {
        let char_width = Span::raw(c.to_string()).width();
        if line_width + char_width > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        line.push(c);
        line_width += char_width;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// メニュー画面での上下左右キーの移動方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GridMove {
    Up,
    Down,
    Left,
    Right,
}

/// columns列に並べたlen枚のカードで、selectedからdirectionへ1つ動いた先のインデックス。
/// 左右は同じ行の中で、上下は同じ列の中で端から反対の端へ折り返す(最終行が欠けていても同様)
fn grid_move(selected: usize, direction: GridMove, columns: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let columns = columns.max(1);
    let selected = selected.min(len - 1);
    let (row, column) = (selected / columns, selected % columns);
    match direction {
        GridMove::Left | GridMove::Right => {
            let row_start = row * columns;
            let row_len = columns.min(len - row_start);
            let step = if direction == GridMove::Right {
                1
            } else {
                row_len - 1
            };
            row_start + (column + step) % row_len
        }
        GridMove::Up | GridMove::Down => {
            let column_len = (len - column).div_ceil(columns);
            let step = if direction == GridMove::Down {
                1
            } else {
                column_len - 1
            };
            (row + step) % column_len * columns + column
        }
    }
}

/// Menu画面でのクリック位置(画面全体のarea基準)から、そこに描いているカードのインデックスを求める。
/// row_offsetは直近の描画でのスクロール位置。カードの外(外枠・カードの間の余白)はNone
fn menu_card_at(area: Rect, row_offset: usize, column: u16, row: u16) -> Option<usize> {
    let grid = menu_grid(area);
    let position = ratatui::layout::Position::new(column, row);
    (0..MENU_ITEMS.len()).find(|&i| {
        grid.card_rect(i, row_offset)
            .is_some_and(|card| card.contains(position))
    })
}

/// メニュー全カードのテキスト(ゲーム名+説明文)をカードの並び順(左上から右下)に並べて返す。
/// 1カードちょうどlines_per_card行ずつで、説明文はカード幅で折り返し、足りない行は空行で埋める。
/// タイプライター表示はこの並びの先頭から1文字ずつ流す
fn menu_item_lines(area: Rect) -> Vec<Line<'static>> {
    let grid = menu_grid(area);
    let name_style = Style::default()
        .fg(theme::TEXT)
        .add_modifier(Modifier::BOLD);
    let description_style = Style::default().fg(theme::MUTED);
    MENU_ITEMS
        .iter()
        .zip(MENU_DESCRIPTIONS)
        .flat_map(|(name, description)| {
            let mut lines = vec![Line::from(Span::styled(*name, name_style))];
            lines.extend(
                wrap_by_width(description, grid.text_width)
                    .into_iter()
                    .map(|line| Line::from(Span::styled(line, description_style))),
            );
            lines.resize(grid.lines_per_card(), Line::from(""));
            lines
        })
        .collect()
}

pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut MenuGridState,
    typing: &mut Typewriter,
    icons: &mut MenuIcons,
) {
    frame.render_widget(menu_block(), area);
    let grid = menu_grid(area);
    state.row_offset = grid.scroll_offset(state.selected, state.row_offset);

    // 画面サイズによって説明文の折り返し(=空行の数)が変わるので、描画する内容に全文字数を合わせる
    let lines = menu_item_lines(area);
    typing.set_total_chars(typewriter::char_count(&lines));
    let lines = typewriter::truncate_lines(&lines, typing.visible_chars());
    for (index, card_lines) in lines.chunks(grid.lines_per_card()).enumerate() {
        if let Some(card) = grid.card_rect(index, state.row_offset) {
            let selected = index == state.selected;
            render_menu_card(frame, card, index, selected, card_lines, icons);
        }
    }
}

/// カード1枚(枠・アイコン・ゲーム名・説明文)を描く。選択中のカードは二重罫線の明るい枠にし、
/// ゲーム名も強調色にする。アイコン画像は位置が変わらない限り使い回されるので、
/// 選択が動いても描き替わるのは枠と文字だけ
fn render_menu_card(
    frame: &mut Frame,
    card: Rect,
    index: usize,
    selected: bool,
    lines: &[Line<'static>],
    icons: &mut MenuIcons,
) {
    let block = if selected {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(
                Style::default()
                    .fg(theme::ACCENT_STRONG)
                    .add_modifier(Modifier::BOLD),
            )
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::ACCENT_SUB))
    };
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let icon_rows = MENU_ICON_ROWS.min(inner.height);
    // 画面からはみ出して欠けたカードには画像を描かない(切れた画像を端末へ送らない)
    if icon_rows == MENU_ICON_ROWS {
        icons.render(
            frame,
            index,
            Rect::new(inner.x, inner.y, inner.width, icon_rows),
        );
    }

    let text_area = Rect::new(
        inner.x + MENU_CARD_PADDING.min(inner.width),
        inner.y + icon_rows,
        inner.width.saturating_sub(2 * MENU_CARD_PADDING),
        inner.height - icon_rows,
    );
    let mut lines = lines.to_vec();
    if selected {
        if let Some(name) = lines.first_mut() {
            for span in &mut name.spans {
                span.style = theme::selected_style();
            }
        }
    }
    // 左寄せにして、タイプライターで文字が増えても行頭の位置が動かないようにする
    frame.render_widget(Paragraph::new(lines), text_area);
}

#[cfg(test)]
mod tests {
    use crossterm::event::MouseEventKind;

    use super::super::menu_items::MENU_ICON_PATHS;
    use super::super::test_support::*;
    use super::*;

    // --- メニューのカードグリッド: データ・レイアウト ---

    #[test]
    fn menu_grid_columns_follow_the_screen_width() {
        let columns = |width: u16| menu_grid(rect(0, 0, width, 30)).columns;
        for width in [0u16, 1, 5, 10, 20] {
            assert_eq!(columns(width), 1, "width={width}: 狭い画面は1列");
        }
        assert!(columns(80) >= 2, "80桁なら複数列");
        assert!(columns(160) > columns(80), "広い画面ほど列が増える");
        let mut previous = 1;
        for width in 0..400u16 {
            let now = columns(width);
            assert!(now >= previous, "width={width}: 幅が増えて列が減らない");
            assert!(now <= MENU_ITEMS.len(), "列数は項目数を超えない");
            previous = now;
        }
    }

    #[test]
    fn menu_grid_rows_is_items_divided_by_columns_rounded_up() {
        for width in [20u16, 80, 120, 200, 400] {
            let grid = menu_grid(rect(0, 0, width, 30));
            assert_eq!(
                grid.rows,
                MENU_ITEMS.len().div_ceil(grid.columns),
                "width={width}"
            );
        }
    }

    #[test]
    fn menu_cards_fit_inside_the_menu_frame_without_overlapping() {
        for (width, height) in [(80u16, 24u16), (80, 30), (120, 40), (200, 60), (30, 12)] {
            let area = rect(0, 0, width, height);
            let grid = menu_grid(area);
            let inner = menu_block().inner(area);
            let cards: Vec<Rect> = (0..MENU_ITEMS.len())
                .filter_map(|i| grid.card_rect(i, 0))
                .collect();
            assert!(!cards.is_empty(), "{width}x{height}: 少なくとも1枚は見える");
            for (i, a) in cards.iter().enumerate() {
                assert_eq!(
                    a.intersection(inner),
                    *a,
                    "{width}x{height}: 枠の内側に収まる"
                );
                for b in &cards[i + 1..] {
                    assert!(
                        !a.intersects(*b),
                        "{width}x{height}: カード同士が重ならない"
                    );
                }
            }
        }
    }

    #[test]
    fn grid_move_left_right_wraps_within_the_row() {
        // 最終行が欠ける場合を確かめるため、メニューの項目数に依存させず15項目で試す
        // (15項目・4列 → 最終行は12,13,14の3枚)
        let len = 15;
        assert_eq!(grid_move(0, GridMove::Right, 4, len), 1);
        assert_eq!(
            grid_move(3, GridMove::Right, 4, len),
            0,
            "行末から行頭へ折り返す"
        );
        assert_eq!(
            grid_move(0, GridMove::Left, 4, len),
            3,
            "行頭から行末へ折り返す"
        );
        assert_eq!(grid_move(5, GridMove::Left, 4, len), 4);
        assert_eq!(
            grid_move(14, GridMove::Right, 4, len),
            12,
            "欠けた最終行の中で折り返す"
        );
        assert_eq!(grid_move(12, GridMove::Left, 4, len), 14);
    }

    #[test]
    fn grid_move_up_down_wraps_within_the_column() {
        // 最終行が欠ける場合を確かめるため、メニューの項目数に依存させず15項目で試す
        let len = 15;
        assert_eq!(grid_move(0, GridMove::Down, 4, len), 4);
        assert_eq!(
            grid_move(12, GridMove::Down, 4, len),
            0,
            "列の下端から上端へ折り返す"
        );
        assert_eq!(
            grid_move(0, GridMove::Up, 4, len),
            12,
            "列の上端から下端へ折り返す"
        );
        // 3列目(0始まり)は3,7,11の3枚だけ(最終行に無い)
        assert_eq!(grid_move(11, GridMove::Down, 4, len), 3);
        assert_eq!(grid_move(3, GridMove::Up, 4, len), 11);
    }

    #[test]
    fn grid_move_with_one_column_behaves_like_the_old_list() {
        let len = MENU_ITEMS.len();
        assert_eq!(grid_move(0, GridMove::Down, 1, len), 1);
        assert_eq!(
            grid_move(len - 1, GridMove::Down, 1, len),
            0,
            "末尾から先頭へ"
        );
        assert_eq!(
            grid_move(0, GridMove::Up, 1, len),
            len - 1,
            "先頭から末尾へ"
        );
        assert_eq!(
            grid_move(5, GridMove::Left, 1, len),
            5,
            "1列なら左右は動かない"
        );
        assert_eq!(grid_move(5, GridMove::Right, 1, len), 5);
    }

    #[test]
    fn arrow_keys_move_the_selection_across_the_grid() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app.last_area = rect(0, 0, 200, 60);
        let columns = menu_grid(screen_rect(app.last_area)).columns;
        assert!(columns >= 3);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.menu_state.selected(), 1);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.menu_state.selected(), 1 + columns);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.menu_state.selected(), columns);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.menu_state.selected(), 0);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.menu_state.selected(), columns - 1, "行内で折り返す");
    }

    #[test]
    fn menu_card_at_maps_every_cell_of_a_card_to_its_index() {
        for (width, height) in [(80u16, 30u16), (200, 60)] {
            let area = rect(0, 0, width, height);
            let grid = menu_grid(area);
            for i in 0..MENU_ITEMS.len() {
                let Some(card) = grid.card_rect(i, 0) else {
                    continue;
                };
                for p in card.positions() {
                    assert_eq!(menu_card_at(area, 0, p.x, p.y), Some(i), "{width}x{height}");
                }
            }
        }
    }

    #[test]
    fn menu_card_at_outside_the_cards_is_none() {
        let area = rect(0, 0, 80, 30);
        assert_eq!(menu_card_at(area, 0, 0, 0), None, "外枠の上はNone");
        assert_eq!(menu_card_at(area, 0, 79, 29), None, "外枠の右下はNone");
        assert_eq!(menu_card_at(area, 0, 200, 5), None, "画面の外はNone");
    }

    #[test]
    fn menu_card_at_follows_the_scroll_offset() {
        let area = rect(0, 0, 80, 24);
        let grid = menu_grid(area);
        let top = grid.card_rect(0, 0).unwrap();
        // 1行ぶんスクロールすると、先頭の位置には2行目の先頭のカードが来る
        assert_eq!(menu_card_at(area, 1, top.x, top.y), Some(grid.columns));
        assert_eq!(
            grid.card_rect(0, 1),
            None,
            "スクロールで上に隠れたカードは描かない"
        );
    }

    #[test]
    fn selecting_jukebox_menu_item_enters_jukebox_screen() {
        let mut app = App::new();
        app.select_menu_item(JUKEBOX_ITEM_INDEX);
        assert!(matches!(app.screen, Screen::Jukebox(_)));
    }

    #[test]
    fn selecting_history_menu_item_enters_history_screen() {
        let mut app = App::new();
        app.select_menu_item(HISTORY_ITEM_INDEX);
        assert!(matches!(app.screen, Screen::History));
    }

    // --- メニューへ戻った時のスクロールバッファのクリア ---

    #[test]
    fn entering_menu_requests_a_scrollback_clear_exactly_once() {
        let mut app = App::new();
        assert!(
            !app.take_pending_scrollback_clear(),
            "起動直後(Splash)ではまだ要求しない"
        );
        app.handle_key(KeyEvent::from(KeyCode::Enter)); // Splash -> Menu
        assert!(matches!(app.screen, Screen::Menu));
        assert!(
            app.take_pending_scrollback_clear(),
            "メニューに入ったらクリアを要求する"
        );
        assert!(
            !app.take_pending_scrollback_clear(),
            "取り出したら消費されて次はfalse"
        );
    }

    #[test]
    fn returning_to_menu_from_every_non_menu_screen_requests_a_scrollback_clear() {
        for (name, mut app) in apps_on_every_non_menu_screen() {
            press(&mut app, KeyCode::Char('q'));
            assert!(matches!(app.screen, Screen::Menu), "{name}: メニューへ戻る");
            assert!(
                app.take_pending_scrollback_clear(),
                "{name}: メニューへ戻ったらクリアを要求する"
            );
        }
    }

    // --- 描画位置とクリック判定の一致 ---

    #[test]
    fn menu_cards_are_drawn_where_clicks_select_them() {
        // 画面の大きさによって列数・カードの大きさが変わっても、項目名が見えている位置を
        // クリックすればその項目になること
        for (width, height) in [(80u16, 24u16), (80, 30), (120, 40), (200, 60)] {
            let mut drawn = 0;
            for (i, name) in MENU_ITEMS.iter().enumerate() {
                let mut app = app_on_menu();
                let Some((x, y)) = drawn_position_of(&mut app, name, width, height) else {
                    continue;
                };
                drawn += 1;
                let area = screen_rect(rect(0, 0, width, height));
                assert_eq!(
                    menu_card_at(area, app.menu_state.row_offset, x, y),
                    Some(i),
                    "{width}x{height}: {name}の位置"
                );
            }
            assert!(drawn >= 2, "{width}x{height}: 複数のカードが見えている");
        }
    }

    #[test]
    fn all_cards_are_drawn_on_a_large_screen() {
        let mut app = app_on_menu();
        for name in MENU_ITEMS {
            assert!(
                drawn_position_of(&mut app, name, 200, 60).is_some(),
                "{name}が描かれていること"
            );
        }
    }

    #[test]
    fn cards_are_laid_out_left_to_right_then_top_to_bottom() {
        let mut app = app_on_menu();
        let positions: Vec<(u16, u16)> = MENU_ITEMS
            .iter()
            .map(|name| drawn_position_of(&mut app, name, 200, 60).unwrap())
            .collect();
        let columns = menu_grid(screen_rect(rect(0, 0, 200, 60))).columns;
        assert_eq!(positions[0].1, positions[1].1, "1枚目と2枚目は同じ行");
        assert!(positions[0].0 < positions[1].0, "2枚目は1枚目の右");
        assert!(
            positions[columns].1 > positions[0].1,
            "列数ぶん進むと次の行"
        );
    }

    #[test]
    fn moving_to_a_card_below_the_screen_scrolls_it_into_view() {
        let (width, height) = (80u16, 24u16);
        let mut app = app_on_menu();
        rendered_cells(&mut app, width, height);
        let grid = menu_grid(screen_rect(rect(0, 0, width, height)));
        assert!(
            grid.visible_rows < grid.rows,
            "この大きさでは全行は収まらない"
        );
        // 同じ列を最下行まで下がる
        let last = (grid.rows - 1) * grid.columns;
        while app.menu_state.selected() != last {
            press(&mut app, KeyCode::Down);
            let name = MENU_ITEMS[app.menu_state.selected()];
            assert!(
                drawn_position_of(&mut app, name, width, height).is_some(),
                "選択中の{name}が常に画面内に描かれる"
            );
        }
        assert!(
            drawn_position_of(&mut app, MENU_ITEMS[0], width, height).is_none(),
            "先頭の行は上にスクロールして隠れる"
        );
        // 最下行で下を押すと列の先頭へ折り返し、先頭の行が見える位置へ戻る
        press(&mut app, KeyCode::Down);
        assert_eq!(app.menu_state.selected(), 0);
        assert!(drawn_position_of(&mut app, MENU_ITEMS[0], width, height).is_some());
    }

    #[test]
    fn clicking_a_scrolled_card_selects_it() {
        let (width, height) = (80u16, 24u16);
        let mut app = app_on_menu();
        rendered_cells(&mut app, width, height);
        let columns = menu_grid(screen_rect(rect(0, 0, width, height))).columns;
        // 履歴のカードが見えるまで下へ移動する
        while app.menu_state.selected() / columns != HISTORY_ITEM_INDEX / columns {
            press(&mut app, KeyCode::Down);
        }
        let (x, y) = drawn_position_of(&mut app, MENU_ITEMS[HISTORY_ITEM_INDEX], width, height)
            .expect("履歴のカードがスクロールして見えていること");
        app.handle_mouse(left_click_at(x, y));
        assert!(matches!(app.screen, Screen::History));
    }

    #[test]
    fn menu_scroll_resets_when_every_card_fits() {
        let mut app = app_on_menu();
        rendered_cells(&mut app, 80, 24);
        app.menu_state.select(HISTORY_ITEM_INDEX);
        rendered_cells(&mut app, 80, 24);
        assert!(
            app.menu_state.row_offset > 0,
            "小さい画面ではスクロールしている"
        );
        rendered_cells(&mut app, 200, 60);
        assert_eq!(
            app.menu_state.row_offset, 0,
            "全部収まる大きさならスクロールしない"
        );
        assert!(drawn_position_of(&mut app, MENU_ITEMS[0], 200, 60).is_some());
    }

    /// 描画結果から、左上の角がcornerの枠のうちnameを囲んでいるものの左上の位置を探す
    fn frame_corner_around(app: &mut App, name: &str, corner: &str) -> Option<(u16, u16)> {
        let (nx, ny) = drawn_position_of(app, name, 200, 60)?;
        let rows = rendered_cells(app, 200, 60);
        (0..ny as usize).rev().find_map(|y| {
            (0..=nx as usize)
                .rev()
                .find(|&x| rows[y][x] == corner)
                .map(|x| (x as u16, y as u16))
        })
    }

    #[test]
    fn only_the_selected_card_has_a_double_border() {
        let mut app = app_on_menu();
        let rows = rendered_cells(&mut app, 200, 60);
        let doubles: Vec<(usize, usize)> = rows
            .iter()
            .enumerate()
            .flat_map(|(y, r)| {
                r.iter()
                    .enumerate()
                    .filter(|(_, c)| *c == "╔")
                    .map(move |(x, _)| (x, y))
            })
            .collect();
        assert_eq!(doubles.len(), 2, "外枠と選択中のカードだけが二重罫線");
        let selected = frame_corner_around(&mut app, MENU_ITEMS[0], "╔");
        assert!(
            selected.is_some_and(|p| p != (0, 0)),
            "選択中の1枚目を二重罫線が囲む"
        );
        assert!(
            frame_corner_around(&mut app, MENU_ITEMS[1], "╭").is_some(),
            "選択していないカードは角丸の枠"
        );

        // 選択を右へ動かすと、二重罫線も右のカードへ移る
        press(&mut app, KeyCode::Right);
        let moved = frame_corner_around(&mut app, MENU_ITEMS[1], "╔").unwrap();
        assert!(moved.0 > selected.unwrap().0);
        assert!(frame_corner_around(&mut app, MENU_ITEMS[0], "╭").is_some());
    }

    #[test]
    fn fallback_cards_show_the_frame_and_text_with_a_blank_icon_area() {
        let mut app = app_on_menu();
        assert!(
            app.menu_icons.is_fallback(),
            "テストでは画像プロトコルを使わない"
        );
        let (x0, y0) = frame_corner_around(&mut app, MENU_ITEMS[0], "╔").expect("カード枠がある");
        let (_, name_y) = drawn_position_of(&mut app, MENU_ITEMS[0], 200, 60).unwrap();
        let rows = rendered_cells(&mut app, 200, 60);
        let right = (x0 as usize + 1..rows[y0 as usize].len())
            .find(|&x| rows[y0 as usize][x] == "╗")
            .unwrap();
        assert_eq!(
            name_y - y0 - 1,
            MENU_ICON_ROWS,
            "枠とゲーム名の間にアイコン分の行がある"
        );
        for (y, row) in rows
            .iter()
            .enumerate()
            .take(name_y as usize)
            .skip(y0 as usize + 1)
        {
            for (x, cell) in row.iter().enumerate().take(right).skip(x0 as usize + 1) {
                assert_eq!(cell, " ", "アイコン部分({x},{y})は空白");
            }
        }
        let card_text: String = (name_y as usize..rows.len())
            .take_while(|&y| rows[y][x0 as usize] != "╚")
            .map(|y| rows[y][x0 as usize + 1..right].concat().replace(' ', ""))
            .collect();
        assert!(card_text.contains(MENU_ITEMS[0]));
        assert!(
            card_text.contains(MENU_DESCRIPTIONS[0]),
            "説明文がゲーム名の下に出る"
        );
    }

    /// 端末に問い合わせないハーフブロック描画のpickerでアイコンを読み込んだApp
    fn app_on_menu_with_icons() -> App {
        let mut picker = ratatui_image::picker::Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Halfblocks);
        let mut app = app_on_menu();
        app.menu_icons = MenuIcons::with_picker(&MENU_ICON_PATHS, Some(picker));
        app
    }

    /// カードのアイコン部分(枠の内側でゲーム名より上)のセル
    fn icon_cells(app: &mut App, index: usize) -> Vec<String> {
        let grid = menu_grid(screen_rect(rect(0, 0, 200, 60)));
        let card = grid.card_rect(index, app.menu_state.row_offset).unwrap();
        let rows = rendered_cells(app, 200, 60);
        (card.y + 1..card.y + 1 + MENU_ICON_ROWS)
            .flat_map(|y| (card.x + 1..card.right() - 1).map(move |x| (x, y)))
            .map(|(x, y)| rows[y as usize][x as usize].clone())
            .collect()
    }

    #[test]
    fn icons_are_drawn_above_the_card_text_when_images_are_available() {
        let mut app = app_on_menu_with_icons();
        assert!(!app.menu_icons.is_fallback());
        for (i, name) in MENU_ITEMS.iter().enumerate() {
            if PENDING_MENU_ICONS.contains(&MENU_ICON_PATHS[i]) {
                continue;
            }
            assert!(
                icon_cells(&mut app, i).iter().any(|c| c != " "),
                "{name}: アイコンが描かれる"
            );
        }
        // アイコンがあってもゲーム名はクリックで選べる
        let (x, y) = drawn_position_of(&mut app, MENU_ITEMS[HISTORY_ITEM_INDEX], 200, 60).unwrap();
        app.handle_mouse(left_click_at(x, y));
        assert!(matches!(app.screen, Screen::History));
    }

    #[test]
    fn moving_the_selection_does_not_move_or_redraw_the_icons() {
        // アイコン画像は使い回し、選択移動では枠だけが描き替わる(カードの位置が動かない)
        let mut app = app_on_menu_with_icons();
        let before: Vec<Vec<String>> = (0..MENU_ITEMS.len())
            .map(|i| icon_cells(&mut app, i))
            .collect();
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Down);
        let after: Vec<Vec<String>> = (0..MENU_ITEMS.len())
            .map(|i| icon_cells(&mut app, i))
            .collect();
        assert_eq!(before, after);
    }

    // --- タイプライター表示 ---

    #[test]
    fn menu_starts_empty_and_types_items_from_the_top() {
        let mut app = app_entering_menu();
        assert_eq!(app.menu_typewriter.visible_chars(), 0);
        assert!(!app.menu_typewriter.is_finished());
        let before = rendered_compact(&mut app);
        assert!(before.contains("BRAINTRAIN"), "枠と見出しは最初から出る");
        assert!(!before.contains(MENU_ITEMS[0]), "項目はまだ出ない");

        // 1枚目のゲーム名「図形回転判定」の先頭2文字
        app.update(MENU_CHAR_INTERVAL * 2);
        let mid = rendered_compact(&mut app);
        assert!(mid.contains("図形"), "1枚目の名前が途中まで出る");
        assert!(!mid.contains(MENU_ITEMS[0]));
        assert!(!mid.contains(MENU_ITEMS[1]), "2枚目はまだ出ない");

        app.update(LONG_ENOUGH);
        assert!(app.menu_typewriter.is_finished());
        for i in 0..MENU_ITEMS.len() {
            let text = card_text(&mut app, i);
            assert!(text.contains(MENU_ITEMS[i]), "{}が出ること", MENU_ITEMS[i]);
            assert!(
                text.contains(MENU_DESCRIPTIONS[i]),
                "{}が出ること",
                MENU_DESCRIPTIONS[i]
            );
        }
    }

    /// 200x60で描画した時の、index番目のカードの中身(枠を除き空白を詰めた文字列)
    fn card_text(app: &mut App, index: usize) -> String {
        let rows = rendered_cells(app, 200, 60);
        let card = menu_grid(screen_rect(rect(0, 0, 200, 60)))
            .card_rect(index, app.menu_state.row_offset)
            .expect("カードが見えていること");
        (card.y + 1..card.bottom() - 1)
            .map(|y| rows[y as usize][card.x as usize + 1..card.right() as usize - 1].concat())
            .collect::<String>()
            .replace(' ', "")
    }

    #[test]
    fn menu_items_appear_in_card_order_from_top_left_to_bottom_right() {
        let mut app = app_entering_menu();
        let mut shown = 0;
        while !app.menu_typewriter.is_finished() {
            app.update(MENU_CHAR_INTERVAL * 5);
            let text = rendered_rows_without_spaces(&mut app, 200, 60).concat();
            let now = MENU_ITEMS
                .iter()
                .take_while(|name| text.contains(*name))
                .count();
            assert!(
                MENU_ITEMS[now..].iter().all(|name| !text.contains(*name)),
                "上の項目より先に下の項目が出ないこと"
            );
            assert!(now >= shown, "一度出た項目は消えない");
            shown = now;
        }
        assert_eq!(shown, MENU_ITEMS.len());
    }

    #[test]
    fn menu_typing_total_follows_the_screen_size() {
        for (width, height) in [(80u16, 24u16), (80, 30), (120, 45), (200, 60), (20, 10)] {
            let mut app = App::new();
            app.last_area = rect(0, 0, width, height);
            press(&mut app, KeyCode::Enter);
            let expected =
                typewriter::char_count(&menu_item_lines(screen_rect(rect(0, 0, width, height))));
            assert_eq!(
                app.menu_typewriter.total_chars(),
                expected,
                "{width}x{height}"
            );
            // 描画した画面サイズの内容に合わせて全文字数が更新される
            rendered_rows_without_spaces(&mut app, 80, 30);
            let resized = typewriter::char_count(&menu_item_lines(screen_rect(rect(0, 0, 80, 30))));
            assert_eq!(
                app.menu_typewriter.total_chars(),
                resized,
                "{width}x{height}→80x30"
            );
        }
    }

    #[test]
    fn menu_item_lines_hold_every_name_and_description_in_card_order() {
        // 説明文はカード幅で折り返すが、文字を足したり削ったりはしない
        for width in [20u16, 80, 200] {
            let area = rect(0, 0, width, 30);
            let grid = menu_grid(area);
            let lines = menu_item_lines(area);
            assert_eq!(
                lines.len(),
                MENU_ITEMS.len() * grid.lines_per_card(),
                "width={width}"
            );
            for (i, card) in lines.chunks(grid.lines_per_card()).enumerate() {
                let text: String = card.iter().map(|l| l.to_string()).collect();
                assert_eq!(
                    text,
                    format!("{}{}", MENU_ITEMS[i], MENU_DESCRIPTIONS[i]),
                    "width={width}"
                );
            }
        }
    }

    #[test]
    fn typed_menu_cards_are_where_clicks_select_them() {
        // 途中まで流れている間も、見えている項目名の位置をクリックするとその項目になる
        for (width, height) in [(80u16, 24u16), (200, 60)] {
            let mut app = App::new();
            app.last_area = rect(0, 0, width, height);
            press(&mut app, KeyCode::Enter);
            app.update(MENU_CHAR_INTERVAL * 120);
            let area = screen_rect(rect(0, 0, width, height));
            let visible: Vec<(usize, (u16, u16))> = (0..MENU_ITEMS.len())
                .filter_map(|i| {
                    drawn_position_of(&mut app, MENU_ITEMS[i], width, height).map(|p| (i, p))
                })
                .collect();
            assert!(
                !visible.is_empty(),
                "{width}x{height}: 見えている項目がある"
            );
            assert!(
                !app.menu_typewriter.is_finished(),
                "{width}x{height}: まだ途中"
            );
            for (i, (x, y)) in visible {
                assert_eq!(menu_card_at(area, app.menu_state.row_offset, x, y), Some(i));
            }
        }
    }

    #[test]
    fn key_while_menu_is_typing_finishes_it_and_still_moves_the_selection() {
        let mut app = app_entering_menu();
        press(&mut app, KeyCode::Down);
        assert!(
            app.menu_typewriter.is_finished(),
            "キー入力で全文字表示済みになる"
        );
        assert_eq!(
            app.menu_state.selected(),
            menu_grid(screen_rect(app.last_area)).columns,
            "上下キーの選択移動もそのまま効く(1つ下の行へ)"
        );
        let text = rendered_compact(&mut app);
        assert!(text.contains(MENU_ITEMS[0]) && text.contains(MENU_ITEMS[1]));
    }

    #[test]
    fn menu_card_text_does_not_shift_while_typing() {
        // カードの文字は、1文字ずつ増えていく間も書き出しの位置が動かないこと
        let mut app = app_entering_menu();
        app.update(MENU_CHAR_INTERVAL);
        let early = drawn_position_of(&mut app, "図", 80, 30).expect("1文字目が出ている");
        app.update(LONG_ENOUGH);
        let done = drawn_position_of(&mut app, MENU_ITEMS[0], 80, 30).unwrap();
        assert_eq!(early, done);
    }

    #[test]
    fn right_key_while_menu_is_typing_finishes_it_and_moves_right() {
        let mut app = app_entering_menu();
        press(&mut app, KeyCode::Right);
        assert!(app.menu_typewriter.is_finished());
        assert_eq!(app.menu_state.selected(), 1);
    }

    #[test]
    fn enter_while_menu_is_typing_still_selects_the_item() {
        let mut app = app_entering_menu();
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.screen, Screen::SelectDifficulty(0, None)));
        assert!(app.menu_typewriter.is_finished());
    }

    #[test]
    fn click_while_menu_is_typing_finishes_it_and_still_selects_the_item() {
        let mut app = app_entering_menu();
        app.last_area = rect(0, 0, 200, 60);
        let (x, y) = app
            .last_area
            .positions()
            .map(|p| (p.x, p.y))
            .find(|&(x, y)| {
                menu_card_at(screen_rect(app.last_area), 0, x, y) == Some(HISTORY_ITEM_INDEX)
            })
            .unwrap();
        app.handle_mouse(left_click_at(x, y));
        assert!(matches!(app.screen, Screen::History));
        assert!(app.menu_typewriter.is_finished());
    }

    #[test]
    fn mouse_move_does_not_skip_the_menu_typing() {
        let mut app = app_entering_menu();
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 5,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert!(!app.menu_typewriter.is_finished());
    }

    #[test]
    fn q_while_menu_is_typing_shows_the_full_menu_behind_the_dialog() {
        let mut app = app_entering_menu();
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.screen, Screen::ConfirmQuit));
        assert!(app.menu_typewriter.is_finished());
        let text = rendered_rows_without_spaces(&mut app, 80, 30).concat();
        assert!(text.contains(MENU_ITEMS[0]));
    }

    #[test]
    fn returning_to_menu_restarts_the_menu_typing() {
        let mut app = app_entering_menu();
        app.update(LONG_ENOUGH);
        app.select_menu_item(HISTORY_ITEM_INDEX);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.screen, Screen::Menu));
        assert_eq!(app.menu_typewriter.visible_chars(), 0);
    }

    #[test]
    fn menu_char_interval_types_the_whole_menu_in_a_few_seconds() {
        // 項目数が多いメニューは標準の間隔だと流し切るのに時間がかかりすぎるため、専用の間隔を使う
        let total = typewriter::char_count(&menu_item_lines(screen_rect(rect(0, 0, 80, 30))));
        let duration = MENU_CHAR_INTERVAL * total as u32;
        assert!(
            duration <= Duration::from_secs(6),
            "メニュー全体が{duration:?}で流れ切る(全{total}文字)"
        );
    }

    // --- 背景の余白を除いた範囲(screen_rect)基準の配置 ---

    #[test]
    fn clicking_the_menu_margin_selects_nothing() {
        let (width, height) = (200u16, 60u16);
        let mut app = app_on_menu();
        rendered_cells(&mut app, width, height);
        let screen = screen_rect(rect(0, 0, width, height));
        for (x, y) in [
            (0, 0),
            (screen.x - 1, 10),
            (10, screen.y - 1),
            (width - 1, height - 1),
        ] {
            app.handle_mouse(left_click_at(x, y));
            assert!(
                matches!(app.screen, Screen::Menu),
                "余白({x},{y})のクリックでは選ばない"
            );
        }
    }

    #[test]
    fn menu_clicks_hit_the_cards_drawn_inside_screen_rect() {
        // 余白の分だけ内側にずれたカードの位置をクリックすると、その項目になる
        let (width, height) = (200u16, 60u16);
        let grid = menu_grid(screen_rect(rect(0, 0, width, height)));
        let card = grid.card_rect(HISTORY_ITEM_INDEX, 0).unwrap();
        let mut app = app_on_menu();
        rendered_cells(&mut app, width, height);
        app.handle_mouse(left_click_at(card.x + 1, card.y + 1));
        assert!(matches!(app.screen, Screen::History));
    }

    #[test]
    fn menu_keys_and_typing_use_the_grid_inside_screen_rect() {
        // キー操作の列数・タイプライターの全文字数も、描画と同じscreen_rect基準にそろえる
        let area = rect(0, 0, 200, 60);
        assert_ne!(
            menu_grid(area).columns,
            menu_grid(screen_rect(area)).columns,
            "この大きさでは余白の有無で列数が変わる(テストの前提)"
        );
        let mut app = App::new();
        app.last_area = area;
        press(&mut app, KeyCode::Enter); // Splash -> Menu
        assert_eq!(
            app.menu_typewriter.total_chars(),
            typewriter::char_count(&menu_item_lines(screen_rect(area)))
        );
        press(&mut app, KeyCode::Down);
        assert_eq!(
            app.menu_state.selected(),
            menu_grid(screen_rect(area)).columns
        );
    }
}
