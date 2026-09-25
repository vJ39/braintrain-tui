use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::memory::MemoryGame;
use crate::game::mental_calc::MentalCalcGame;
use crate::game::mirror_match::MirrorMatchGame;
use crate::game::pattern_fill::PatternFillGame;
use crate::game::puzzle_connect::PuzzleConnectGame;
use crate::game::reaction::ReactionGame;
use crate::game::rhythm::RhythmGame;
use crate::game::row_index;
use crate::game::sequence::SequenceGame;
use crate::game::shape_rotate::ShapeRotateGame;
use crate::game::{Difficulty, Game, GameResult};
use crate::stats::store;

const MENU_ITEMS: [&str; 10] = [
    "図形回転判定",
    "鏡像判定",
    "反応速度(Stroop)",
    "暗算スピード",
    "パターン補完",
    "記憶(位置と色)",
    "数列予測",
    "組み合わせパズル",
    "リズム(DDR風)",
    "履歴",
];

const HISTORY_ITEM_INDEX: usize = MENU_ITEMS.len() - 1;

pub enum Screen {
    Menu,
    SelectDifficulty(usize),
    Playing(Box<dyn Game>),
    Result(GameResult),
    History,
}

pub struct App {
    screen: Screen,
    menu_state: ListState,
    should_quit: bool,
    /// 直近のrender()で描画したフルスクリーンのエリア。マウス座標からのヒット
    /// テストに使う(render()より前にhandle_mouseが呼ばれることは無い前提)
    last_area: Rect,
}

impl App {
    pub fn new() -> Self {
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));
        Self {
            screen: Screen::Menu,
            menu_state,
            should_quit: false,
            last_area: Rect::default(),
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('q') {
            self.should_quit = true;
            return;
        }

        match &mut self.screen {
            Screen::Menu => self.handle_menu_key(key),
            Screen::SelectDifficulty(item) => {
                let item = *item;
                self.handle_difficulty_key(key, item);
            }
            Screen::Playing(game) => {
                game.handle_key(key);
                if game.is_finished() {
                    self.screen = Screen::Result(game.result());
                }
            }
            Screen::Result(_) | Screen::History => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                    audio::play_se(SeKind::Transition);
                    self.screen = Screen::Menu;
                }
            }
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let area = self.last_area;
        match &mut self.screen {
            Screen::Menu => {
                if let Some(index) = menu_item_at_row(area, mouse.row) {
                    self.menu_state.select(Some(index));
                    audio::play_se(SeKind::Transition);
                    if index == HISTORY_ITEM_INDEX {
                        self.screen = Screen::History;
                    } else {
                        self.screen = Screen::SelectDifficulty(index);
                    }
                }
            }
            Screen::SelectDifficulty(item) => {
                let item = *item;
                if let Some(difficulty) = difficulty_at_row(area, mouse.row) {
                    audio::play_se(SeKind::Transition);
                    self.screen = Screen::Playing(new_game(item, difficulty));
                }
            }
            Screen::Playing(game) => {
                game.handle_mouse(mouse, area);
                if game.is_finished() {
                    self.screen = Screen::Result(game.result());
                }
            }
            Screen::Result(_) | Screen::History => {
                audio::play_se(SeKind::Transition);
                self.screen = Screen::Menu;
            }
        }
    }

    fn handle_menu_key(&mut self, key: KeyEvent) {
        let len = MENU_ITEMS.len();
        let selected = self.menu_state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Up => {
                self.menu_state.select(Some((selected + len - 1) % len));
            }
            KeyCode::Down => {
                self.menu_state.select(Some((selected + 1) % len));
            }
            KeyCode::Enter => {
                audio::play_se(SeKind::Transition);
                if selected == HISTORY_ITEM_INDEX {
                    self.screen = Screen::History;
                } else {
                    self.screen = Screen::SelectDifficulty(selected);
                }
            }
            _ => {}
        }
    }

    fn handle_difficulty_key(&mut self, key: KeyEvent, item: usize) {
        let difficulty = match key.code {
            KeyCode::Char('1') => Some(Difficulty::Beginner),
            KeyCode::Char('2') => Some(Difficulty::Intermediate),
            KeyCode::Char('3') => Some(Difficulty::Advanced),
            KeyCode::Esc => {
                self.screen = Screen::Menu;
                return;
            }
            _ => None,
        };
        if let Some(difficulty) = difficulty {
            audio::play_se(SeKind::Transition);
            self.screen = Screen::Playing(new_game(item, difficulty));
        }
    }

    pub fn update(&mut self, dt: Duration) {
        if let Screen::Playing(game) = &mut self.screen {
            game.update(dt);
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.last_area = area;
        match &self.screen {
            Screen::Menu => render_menu(frame, area, &mut self.menu_state),
            Screen::SelectDifficulty(item) => render_difficulty_select(frame, area, MENU_ITEMS[*item]),
            Screen::Playing(game) => game.render(frame, area),
            Screen::Result(result) => render_result(frame, area, result),
            Screen::History => render_history(frame, area),
        }
    }
}

fn new_game(item: usize, difficulty: Difficulty) -> Box<dyn Game> {
    match item {
        0 => Box::new(ShapeRotateGame::new(difficulty)),
        1 => Box::new(MirrorMatchGame::new(difficulty)),
        2 => Box::new(ReactionGame::new(difficulty)),
        3 => Box::new(MentalCalcGame::new(difficulty)),
        4 => Box::new(PatternFillGame::new(difficulty)),
        5 => Box::new(MemoryGame::new(difficulty)),
        6 => Box::new(SequenceGame::new(difficulty)),
        7 => Box::new(PuzzleConnectGame::new(difficulty)),
        8 => Box::new(RhythmGame::new(difficulty)),
        _ => unreachable!("history is handled without creating a game"),
    }
}

fn render_menu(frame: &mut Frame, area: Rect, state: &mut ListState) {
    let items: Vec<ListItem> = MENU_ITEMS.iter().map(|s| ListItem::new(*s)).collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("braintrain-tui"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, area, state);
}

/// SelectDifficulty画面で、難易度の行(初級/中級/上級)が内部エリアの何行目から
/// 始まるか。render_difficulty_selectとdifficulty_at_rowで一致させること
const DIFFICULTY_ROWS_OFFSET: u16 = 2;

fn render_difficulty_select(frame: &mut Frame, area: Rect, game_name: &str) {
    let text = vec![
        Line::from(Span::raw(format!("{game_name} - 難易度を選択"))),
        Line::from(""),
        Line::from("1: 初級"),
        Line::from("2: 中級"),
        Line::from("3: 上級"),
        Line::from(""),
        Line::from("(Escで戻る)"),
    ];
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

/// SelectDifficulty画面でのクリック行(area基準、Block枠含む)から難易度を求める
fn difficulty_at_row(area: Rect, mouse_row: u16) -> Option<Difficulty> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let relative = mouse_row.checked_sub(inner.y)?.checked_sub(DIFFICULTY_ROWS_OFFSET)?;
    match relative {
        0 => Some(Difficulty::Beginner),
        1 => Some(Difficulty::Intermediate),
        2 => Some(Difficulty::Advanced),
        _ => None,
    }
}

/// Menu画面でのクリック行(area基準、Block枠含む)からメニュー項目インデックスを求める
fn menu_item_at_row(area: Rect, mouse_row: u16) -> Option<usize> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    row_index(inner, mouse_row, MENU_ITEMS.len() as u16)
}

fn render_result(frame: &mut Frame, area: Rect, result: &GameResult) {
    if let Err(e) = store::append_result(result) {
        // 保存に失敗しても画面表示は続ける。エラー内容だけ表示する
        let error_area = centered_rect(area, 60, 3);
        let paragraph = Paragraph::new(format!("履歴の保存に失敗: {e}"))
            .alignment(Alignment::Center);
        frame.render_widget(paragraph, error_area);
    }

    let text = vec![
        Line::from(format!("正解: {} / {}", result.correct, result.total)),
        Line::from(format!("平均反応時間: {:.0}ms", result.avg_latency_ms)),
        Line::from(""),
        Line::from("Enterでメニューに戻る"),
    ];
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("結果"));
    frame.render_widget(paragraph, area);
}

fn render_history(frame: &mut Frame, area: Rect) {
    crate::stats::history_view::render(frame, area);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(width),
            Constraint::Fill(1),
        ])
        .split(layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MENU_ITEMSにゲームを追加してnew_gameのmatchを更新し忘れると、
    /// この範囲でunreachable!に到達してpanicし検知できる
    #[test]
    fn new_game_handles_every_non_history_menu_item() {
        for item in 0..HISTORY_ITEM_INDEX {
            let _game = new_game(item, Difficulty::Beginner);
        }
    }

    #[test]
    fn history_item_index_is_the_last_menu_item() {
        assert_eq!(HISTORY_ITEM_INDEX, MENU_ITEMS.len() - 1);
        assert_eq!(MENU_ITEMS[HISTORY_ITEM_INDEX], "履歴");
    }

    fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect::new(x, y, width, height)
    }

    #[test]
    fn menu_item_at_row_maps_each_row_to_its_index() {
        // area(0,0,20,12)、Block枠込みなので内部は(1,1,18,10)
        let area = rect(0, 0, 20, 12);
        for (offset, expected) in (0..MENU_ITEMS.len()).enumerate() {
            let row = 1 + offset as u16;
            assert_eq!(menu_item_at_row(area, row), Some(expected));
        }
    }

    #[test]
    fn menu_item_at_row_on_border_is_none() {
        let area = rect(0, 0, 20, 12);
        assert_eq!(menu_item_at_row(area, 0), None, "上端の枠線上はNone");
        assert_eq!(menu_item_at_row(area, 11), None, "下端の枠線上はNone");
    }

    #[test]
    fn difficulty_at_row_maps_beginner_intermediate_advanced() {
        let area = rect(0, 0, 30, 10);
        let inner_top = Block::default().borders(Borders::ALL).inner(area).y;
        assert_eq!(
            difficulty_at_row(area, inner_top + DIFFICULTY_ROWS_OFFSET),
            Some(Difficulty::Beginner)
        );
        assert_eq!(
            difficulty_at_row(area, inner_top + DIFFICULTY_ROWS_OFFSET + 1),
            Some(Difficulty::Intermediate)
        );
        assert_eq!(
            difficulty_at_row(area, inner_top + DIFFICULTY_ROWS_OFFSET + 2),
            Some(Difficulty::Advanced)
        );
    }

    #[test]
    fn difficulty_at_row_outside_options_is_none() {
        let area = rect(0, 0, 30, 10);
        let inner_top = Block::default().borders(Borders::ALL).inner(area).y;
        // タイトル行・空行・「(Escで戻る)」行はどの難易度にも当たらない
        assert_eq!(difficulty_at_row(area, inner_top), None);
        assert_eq!(
            difficulty_at_row(area, inner_top + DIFFICULTY_ROWS_OFFSET + 3),
            None
        );
    }
}
