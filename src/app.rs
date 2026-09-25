use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::audio::{self, BgmCategory, SeKind};
use crate::game::memory::MemoryGame;
use crate::game::mental_calc::MentalCalcGame;
use crate::game::mirror_match::MirrorMatchGame;
use crate::game::pattern_fill::PatternFillGame;
use crate::game::puzzle_connect::PuzzleConnectGame;
use crate::game::reaction::ReactionGame;
use crate::game::rhythm::{RhythmGame, SONGS};
use crate::game::row_index;
use crate::game::sequence::SequenceGame;
use crate::game::shape_rotate::ShapeRotateGame;
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult};
use crate::stats::store;

const MENU_ITEMS: [&str; 11] = [
    "図形回転判定",
    "鏡像判定",
    "反応速度(Stroop)",
    "暗算スピード",
    "パターン補完",
    "記憶(位置と色)",
    "数列予測",
    "組み合わせパズル",
    "リズム(DDR風)",
    "ジュークボックス",
    "履歴",
];

/// リズムゲームだけは難易度選択の前に曲選択を挟む
const RHYTHM_ITEM_INDEX: usize = 8;
const JUKEBOX_ITEM_INDEX: usize = MENU_ITEMS.len() - 2;
const HISTORY_ITEM_INDEX: usize = MENU_ITEMS.len() - 1;

pub enum Screen {
    Menu,
    /// リズムゲームの曲選択(選択中の曲 = SONGSのインデックス)
    SelectSong(usize),
    /// (メニュー項目, リズムゲームの場合は選んだ曲)
    SelectDifficulty(usize, Option<usize>),
    Playing(Box<dyn Game>),
    Result(GameResult),
    History,
    Jukebox(ListState),
}

pub struct App {
    screen: Screen,
    menu_state: ListState,
    should_quit: bool,
    /// 直近のrender()で描画したフルスクリーンのエリア。マウス座標からのヒット
    /// テストに使う(render()より前にhandle_mouseが呼ばれることは無い前提)
    last_area: Rect,
    /// 現在再生中のBGMトラック名(ジュークボックス画面のハイライト表示に使う)
    current_bgm: Option<String>,
}

impl App {
    pub fn new() -> Self {
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));
        let current_bgm = audio::random_bgm_track(BgmCategory::Menu);
        if let Some(name) = &current_bgm {
            audio::play_bgm_track(name);
        }
        Self {
            screen: Screen::Menu,
            menu_state,
            should_quit: false,
            last_area: Rect::default(),
            current_bgm,
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
            Screen::SelectSong(selected) => {
                let selected = *selected;
                self.handle_song_key(key, selected);
            }
            Screen::SelectDifficulty(item, song) => {
                let (item, song) = (*item, *song);
                self.handle_difficulty_key(key, item, song);
            }
            Screen::Playing(game) => {
                game.handle_key(key);
                if game.is_finished() {
                    self.screen = Screen::Result(game.result());
                }
            }
            Screen::Result(_) | Screen::History => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                    self.return_to_menu();
                }
            }
            Screen::Jukebox(_) => self.handle_jukebox_key(key),
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
                    self.select_menu_item(index);
                }
            }
            Screen::SelectSong(_) => {
                if let Some(song) = song_at_row(area, mouse.row) {
                    self.select_song(song);
                }
            }
            Screen::SelectDifficulty(item, song) => {
                let (item, song) = (*item, *song);
                if let Some(difficulty) = difficulty_at_row(area, mouse.row) {
                    self.start_playing(item, difficulty, song);
                }
            }
            Screen::Playing(game) => {
                game.handle_mouse(mouse, area);
                if game.is_finished() {
                    self.screen = Screen::Result(game.result());
                }
            }
            Screen::Result(_) | Screen::History => {
                self.return_to_menu();
            }
            Screen::Jukebox(_) => {}
        }
    }

    /// Menu画面での項目決定(キー/クリック共通)。ゲーム/ジュークボックス/履歴へ振り分ける
    fn select_menu_item(&mut self, selected: usize) {
        audio::play_se(SeKind::Transition);
        if selected == HISTORY_ITEM_INDEX {
            self.screen = Screen::History;
        } else if selected == JUKEBOX_ITEM_INDEX {
            self.screen = Screen::Jukebox(self.jukebox_list_state());
        } else if selected == RHYTHM_ITEM_INDEX {
            self.screen = Screen::SelectSong(0);
        } else {
            self.screen = Screen::SelectDifficulty(selected, None);
        }
    }

    /// 曲選択画面での曲決定(キー/クリック共通)。その曲の難易度選択へ進む
    fn select_song(&mut self, song: usize) {
        audio::play_se(SeKind::Transition);
        self.screen = Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(song));
    }

    /// 難易度決定後、指定ゲームを開始しPlaying用BGMに切り替える(キー/クリック共通)
    fn start_playing(&mut self, item: usize, difficulty: Difficulty, song: Option<usize>) {
        audio::play_se(SeKind::Transition);
        if item == RHYTHM_ITEM_INDEX {
            self.start_rhythm(song.unwrap_or(0), difficulty);
            return;
        }
        if let Some(name) = audio::random_bgm_track(BgmCategory::Playing) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        self.screen = Screen::Playing(new_game(item, difficulty));
    }

    /// リズムゲームを開始する。譜面生成を先に済ませ、選んだ曲のBGM再生を始めた直後に
    /// ゲーム内時計を合わせることで、曲と譜面(実測ビート時刻)の時間基準を揃える
    fn start_rhythm(&mut self, song: usize, difficulty: Difficulty) {
        let mut game = RhythmGame::new(difficulty, song);
        let track_name = game.song().track_name;
        audio::play_bgm_track(track_name);
        game.restart_clock();
        self.current_bgm = Some(track_name.to_string());
        self.screen = Screen::Playing(Box::new(game));
    }

    /// Menu画面に戻り、Menu用BGMに切り替える(キー/クリック共通)
    fn return_to_menu(&mut self) {
        audio::play_se(SeKind::Transition);
        if let Some(name) = audio::random_bgm_track(BgmCategory::Menu) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        self.screen = Screen::Menu;
    }

    /// 現在再生中の曲を選択済みにしたジュークボックス画面用ListStateを作る
    fn jukebox_list_state(&self) -> ListState {
        let tracks = audio::bgm_track_names();
        let mut state = ListState::default();
        let index = self
            .current_bgm
            .as_ref()
            .and_then(|name| tracks.iter().position(|t| t == name))
            .unwrap_or(0);
        state.select(Some(index));
        state
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
            KeyCode::Enter => self.select_menu_item(selected),
            _ => {}
        }
    }

    fn handle_jukebox_key(&mut self, key: KeyEvent) {
        let tracks = audio::bgm_track_names();
        let len = tracks.len().max(1);
        let selected = if let Screen::Jukebox(state) = &self.screen {
            state.selected().unwrap_or(0)
        } else {
            return;
        };

        match key.code {
            KeyCode::Up => {
                if let Screen::Jukebox(state) = &mut self.screen {
                    state.select(Some((selected + len - 1) % len));
                }
            }
            KeyCode::Down => {
                if let Screen::Jukebox(state) = &mut self.screen {
                    state.select(Some((selected + 1) % len));
                }
            }
            KeyCode::Enter => {
                if let Some(name) = tracks.get(selected).cloned() {
                    audio::play_bgm_track(&name);
                    self.current_bgm = Some(name);
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                audio::stop_bgm();
                self.current_bgm = None;
            }
            KeyCode::Esc => {
                self.screen = Screen::Menu;
            }
            _ => {}
        }
    }

    fn handle_song_key(&mut self, key: KeyEvent, selected: usize) {
        let len = SONGS.len().max(1);
        match key.code {
            KeyCode::Up => self.screen = Screen::SelectSong((selected + len - 1) % len),
            KeyCode::Down => self.screen = Screen::SelectSong((selected + 1) % len),
            KeyCode::Enter => self.select_song(selected),
            // 数字キーで直接選ぶ(1始まり)。曲数を超える番号は無視する
            KeyCode::Char(c) => {
                if let Some(index) = c.to_digit(10).and_then(|d| (d as usize).checked_sub(1)) {
                    if index < SONGS.len() {
                        self.select_song(index);
                    }
                }
            }
            KeyCode::Esc => self.screen = Screen::Menu,
            _ => {}
        }
    }

    fn handle_difficulty_key(&mut self, key: KeyEvent, item: usize, song: Option<usize>) {
        let difficulty = match key.code {
            KeyCode::Char('1') => Some(Difficulty::Beginner),
            KeyCode::Char('2') => Some(Difficulty::Intermediate),
            KeyCode::Char('3') => Some(Difficulty::Advanced),
            KeyCode::Esc => {
                // リズムゲームは曲選択に戻る。他のゲームはメニューに戻る
                self.screen = match song {
                    Some(song) => Screen::SelectSong(song),
                    None => Screen::Menu,
                };
                return;
            }
            _ => None,
        };
        if let Some(difficulty) = difficulty {
            self.start_playing(item, difficulty, song);
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
        let current_bgm = self.current_bgm.clone();
        match &mut self.screen {
            Screen::Menu => render_menu(frame, area, &mut self.menu_state),
            Screen::SelectSong(selected) => render_song_select(frame, area, *selected),
            Screen::SelectDifficulty(item, song) => {
                let title = match song.and_then(|s| SONGS.get(s)) {
                    Some(song) => format!("{} / {}", MENU_ITEMS[*item], song.display_name),
                    None => MENU_ITEMS[*item].to_string(),
                };
                render_difficulty_select(frame, area, &title)
            }
            Screen::Playing(game) => game.render(frame, area),
            Screen::Result(result) => render_result(frame, area, result),
            Screen::History => render_history(frame, area),
            Screen::Jukebox(state) => render_jukebox(frame, area, state, current_bgm.as_deref()),
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
        RHYTHM_ITEM_INDEX => unreachable!("rhythm is started via start_rhythm with a song"),
        _ => unreachable!("history is handled without creating a game"),
    }
}

/// SelectSong画面で、曲の行が内部エリアの何行目から始まるか。
/// render_song_selectとsong_at_rowで一致させること
const SONG_ROWS_OFFSET: u16 = 2;

fn render_song_select(frame: &mut Frame, area: Rect, selected: usize) {
    // 行の並びはsong_at_rowと一致させる(1行目=見出し、2行目=空行、3行目以降=曲)
    let mut text = vec![
        Line::from(vec![
            Span::styled("♪ ", Style::default().fg(theme::ACCENT)),
            Span::styled(MENU_ITEMS[RHYTHM_ITEM_INDEX], theme::title_style()),
            Span::styled("  曲を選択", Style::default().fg(theme::TEXT)),
        ]),
        Line::from(""),
    ];
    for (i, song) in SONGS.iter().enumerate() {
        let label = format!("{}: {}", i + 1, song.display_name);
        let line = if i == selected {
            Line::from(Span::styled(format!(" ▶ {label} "), theme::selected_style()))
        } else {
            Line::from(Span::styled(
                format!("   {label} "),
                Style::default().fg(theme::TEXT),
            ))
        };
        text.push(line);
    }
    let block = theme::panel(" ◆ BRAIN TRAIN ◆ ").title_bottom(
        theme::hints_line(&[
            ("↑↓", "選択"),
            ("Enter / 数字", "決定"),
            ("Esc", "戻る"),
        ])
        .centered(),
    );
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(block);
    frame.render_widget(paragraph, area);
}

/// SelectSong画面でのクリック行(area基準、Block枠含む)から曲インデックスを求める
fn song_at_row(area: Rect, mouse_row: u16) -> Option<usize> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let relative = mouse_row
        .checked_sub(inner.y)?
        .checked_sub(SONG_ROWS_OFFSET)? as usize;
    (relative < SONGS.len()).then_some(relative)
}

fn render_menu(frame: &mut Frame, area: Rect, state: &mut ListState) {
    // 各項目の一言説明。配列長をMENU_ITEMS.len()にして、項目の追加漏れをコンパイル時に検出する
    const DESCRIPTIONS: [&str; MENU_ITEMS.len()] = [
        "回転させた図形が元と同じかを見分ける",
        "鏡に映した図形かどうかを見分ける",
        "文字の色と意味が一致するかを即答する",
        "4択から計算の答えを素早く選ぶ",
        "3x3の規則から空欄に入る図形を選ぶ",
        "光ったパネルの順番を覚えて再現する",
        "数列の法則を見抜いて次の数を当てる",
        "完成形から2つ目のピースを当てる",
        "矢印キーで曲に合わせてステップする",
        "BGMを選んで聴く",
        "ゲームごとの反応時間の推移を見る",
    ];

    let block = theme::panel(Line::from(" ◆ BRAIN TRAIN ◆ ").centered())
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
                ("↑↓", "選択"),
                ("Enter / クリック", "決定"),
                ("q", "終了"),
            ])
            .centered(),
        );

    // menu_item_at_rowは枠の内側を項目数で等分(row_index)して判定するので、
    // 1項目の高さもその等分に合わせ、見た目の位置とクリック位置をそろえる
    let inner = block.inner(area);
    let item_height = (inner.height / MENU_ITEMS.len() as u16).max(1) as usize;
    let items: Vec<ListItem> = MENU_ITEMS
        .iter()
        .zip(DESCRIPTIONS)
        .enumerate()
        .map(|(i, (name, description))| {
            let number = Span::styled(format!("{:02}  ", i + 1), Style::default().fg(theme::ACCENT));
            let name = Span::styled(
                *name,
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            );
            let description_style = Style::default().fg(theme::MUTED);
            let content: Vec<Line> = if item_height >= 2 {
                vec![
                    Line::from(vec![number, name]),
                    Line::from(Span::styled(format!("    {description}"), description_style)),
                ]
            } else {
                vec![Line::from(vec![
                    number,
                    name,
                    Span::styled(format!("   {description}"), description_style),
                ])]
            };
            // 帯の中で縦中央に置き、残りは空行で埋めて帯の高さちょうどにする
            let top = (item_height - content.len()) / 2;
            let mut lines = vec![Line::from(""); top];
            lines.extend(content);
            lines.resize(item_height, Line::from(""));
            ListItem::new(lines)
        })
        .collect();

    // 全項目が収まる大きさならスクロールさせない(以前の小さい画面での位置が残らないように)
    if inner.height as usize >= item_height * MENU_ITEMS.len() {
        *state.offset_mut() = 0;
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected_style())
        .highlight_symbol(" ▶ ");
    frame.render_stateful_widget(list, area, state);
}

/// SelectDifficulty画面で、難易度の行(初級/中級/上級)が内部エリアの何行目から
/// 始まるか。render_difficulty_selectとdifficulty_at_rowで一致させること
const DIFFICULTY_ROWS_OFFSET: u16 = 2;

fn render_difficulty_select(frame: &mut Frame, area: Rect, game_name: &str) {
    // 行の並びはdifficulty_at_rowと一致させる(1行目=見出し、2行目=空行、3〜5行目=初級/中級/上級)
    let mut text = vec![
        Line::from(vec![
            Span::styled("◆ ", Style::default().fg(theme::ACCENT)),
            Span::styled(game_name.to_string(), theme::title_style()),
            Span::styled("  難易度を選択", Style::default().fg(theme::TEXT)),
        ]),
        Line::from(""),
    ];
    let options = [
        ("1", Difficulty::Beginner, "まずは肩ならし"),
        ("2", Difficulty::Intermediate, "ほどよい手ごたえ"),
        ("3", Difficulty::Advanced, "腕に自信がある人向け"),
    ];
    // 説明文は全角文字だけなので、最長に合わせて全角空白で埋めると中央寄せでも行頭がそろう
    let note_width = options.iter().map(|(_, _, n)| n.chars().count()).max().unwrap_or(0);
    for (key, difficulty, note) in options {
        let (label, color) = theme::difficulty_label(difficulty);
        let note = format!("{note}{}", "　".repeat(note_width - note.chars().count()));
        text.push(Line::from(vec![
            Span::styled(
                format!(" {key} "),
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                label,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("   {note}"), Style::default().fg(theme::MUTED)),
        ]));
    }
    text.push(Line::from(""));
    text.push(Line::from(Span::styled(
        "(Escで戻る)",
        Style::default().fg(theme::MUTED),
    )));
    let block = theme::panel(" ◆ BRAIN TRAIN ◆ ").title_bottom(
        theme::hints_line(&[("1〜3 / クリック", "開始"), ("Esc", "戻る")]).centered(),
    );
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(block);
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
    // 保存に失敗しても画面表示は続ける。エラー内容は結果の描画後に下端へ重ねて表示する
    let save_error = store::append_result(result).err();

    // game_idからメニュー上の表示名を引く(見つからなければgame_idをそのまま出す)
    let game_names = [
        (crate::game::shape_rotate::GAME_ID, 0),
        (crate::game::mirror_match::GAME_ID, 1),
        (crate::game::reaction::GAME_ID, 2),
        (crate::game::mental_calc::GAME_ID, 3),
        (crate::game::pattern_fill::GAME_ID, 4),
        (crate::game::memory::GAME_ID, 5),
        (crate::game::sequence::GAME_ID, 6),
        (crate::game::puzzle_connect::GAME_ID, 7),
        (crate::game::rhythm::GAME_ID, RHYTHM_ITEM_INDEX),
    ];
    let game_name = game_names
        .iter()
        .find(|(id, _)| *id == result.game_id)
        .map_or(result.game_id.as_str(), |(_, index)| MENU_ITEMS[*index]);
    let (difficulty_text, difficulty_color) = theme::difficulty_label(result.difficulty);
    let (rank, rank_color) = theme::rank_for(result.correct, result.total);
    let percent = if result.total == 0 {
        0
    } else {
        result.correct * 100 / result.total
    };

    let outer = theme::panel(Line::from(" ◆ RESULT ◆ ").centered()).title_bottom(
        theme::hints_line(&[("Enter / Esc / クリック", "メニューに戻る")]).centered(),
    );
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let label_style = Style::default().fg(theme::MUTED);
    let value_style = Style::default()
        .fg(theme::ACCENT_STRONG)
        .add_modifier(Modifier::BOLD);
    let text = vec![
        Line::from(vec![
            Span::styled(game_name.to_string(), theme::title_style()),
            Span::styled("   ", label_style),
            Span::styled(
                difficulty_text,
                Style::default()
                    .fg(difficulty_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("RANK  ", label_style),
            Span::styled(
                format!("  {rank}  "),
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(rank_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("正解  ", label_style),
            Span::styled(
                format!("{} / {}", result.correct, result.total),
                value_style,
            ),
            Span::styled(format!("  ({percent}%)"), Style::default().fg(theme::TEXT)),
        ]),
        Line::from(Span::styled(
            theme::progress_bar(result.correct, result.total, 20),
            Style::default().fg(rank_color),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("平均反応時間  ", label_style),
            Span::styled(format!("{:.0}", result.avg_latency_ms), value_style),
            Span::styled(" ms", Style::default().fg(theme::TEXT)),
        ]),
    ];
    let content_height = text.len() as u16;
    let card = centered_rect(inner, inner.width.min(48), (content_height + 2).min(inner.height));
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(theme::sub_panel());
    frame.render_widget(paragraph, card);

    if let Some(e) = save_error {
        let error_area = Rect::new(
            inner.x,
            inner.y + inner.height.saturating_sub(1),
            inner.width,
            inner.height.min(1),
        );
        let paragraph = Paragraph::new(format!("履歴の保存に失敗: {e}"))
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme::INCORRECT));
        frame.render_widget(paragraph, error_area);
    }
}

fn render_jukebox(frame: &mut Frame, area: Rect, state: &mut ListState, playing: Option<&str>) {
    // 「再生中の曲」「曲リスト」「操作説明」の3段
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);

    let now_playing = match playing {
        Some(name) => Line::from(vec![
            Span::styled("♪ 再生中  ", Style::default().fg(theme::ACCENT)),
            Span::styled(name.to_string(), theme::title_style()),
        ]),
        None => Line::from(Span::styled("■ 停止中", Style::default().fg(theme::MUTED))),
    };
    frame.render_widget(
        Paragraph::new(now_playing)
            .alignment(Alignment::Center)
            .block(theme::panel(" ◆ ジュークボックス ◆ ")),
        rows[0],
    );

    let tracks = audio::bgm_track_names();
    let items: Vec<ListItem> = if tracks.is_empty() {
        vec![ListItem::new(Span::styled(
            "(曲がありません)",
            Style::default().fg(theme::MUTED),
        ))]
    } else {
        tracks
            .iter()
            .map(|name| {
                if Some(name.as_str()) == playing {
                    ListItem::new(Span::styled(
                        format!("♪ {name} (再生中)"),
                        Style::default()
                            .fg(theme::ACCENT_STRONG)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    ListItem::new(Span::styled(
                        format!("  {name}"),
                        Style::default().fg(theme::TEXT),
                    ))
                }
            })
            .collect()
    };
    let list = List::new(items)
        .block(theme::panel(" 曲リスト "))
        .highlight_style(theme::selected_style())
        .highlight_symbol(" ▶ ");
    frame.render_stateful_widget(list, rows[1], state);

    theme::render_hint_footer(
        frame,
        rows[2],
        &[("↑↓", "選択"), ("Enter", "再生"), ("S", "停止"), ("Esc", "戻る")],
    );
}

fn render_history(frame: &mut Frame, area: Rect) {
    let block = theme::panel(" ◆ 履歴: ゲームごとの平均反応時間の推移 ◆ ").title_bottom(
        theme::hints_line(&[("Enter / Esc / クリック", "メニューに戻る")]).centered(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    crate::stats::history_view::render(frame, inner);
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
    fn new_game_handles_every_game_menu_item() {
        for item in 0..JUKEBOX_ITEM_INDEX {
            // リズムは曲選択が必要なので start_rhythm で別経路で作る
            if item == RHYTHM_ITEM_INDEX {
                continue;
            }
            let _game = new_game(item, Difficulty::Beginner);
        }
    }

    #[test]
    fn jukebox_and_history_are_the_last_two_menu_items() {
        assert_eq!(JUKEBOX_ITEM_INDEX, MENU_ITEMS.len() - 2);
        assert_eq!(HISTORY_ITEM_INDEX, MENU_ITEMS.len() - 1);
        assert_eq!(MENU_ITEMS[JUKEBOX_ITEM_INDEX], "ジュークボックス");
        assert_eq!(MENU_ITEMS[HISTORY_ITEM_INDEX], "履歴");
    }

    fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect::new(x, y, width, height)
    }

    #[test]
    fn menu_item_at_row_maps_each_row_to_its_index() {
        // 上下の枠線(2行)+項目数ぶんの高さを持つエリア
        let height = MENU_ITEMS.len() as u16 + 2;
        let area = rect(0, 0, 20, height);
        for (offset, expected) in (0..MENU_ITEMS.len()).enumerate() {
            let row = 1 + offset as u16;
            assert_eq!(menu_item_at_row(area, row), Some(expected));
        }
    }

    #[test]
    fn menu_item_at_row_on_border_is_none() {
        let height = MENU_ITEMS.len() as u16 + 2;
        let area = rect(0, 0, 20, height);
        assert_eq!(menu_item_at_row(area, 0), None, "上端の枠線上はNone");
        assert_eq!(
            menu_item_at_row(area, height - 1),
            None,
            "下端の枠線上はNone"
        );
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

    #[test]
    fn selecting_game_menu_item_enters_difficulty_screen() {
        let mut app = App::new();
        app.select_menu_item(0);
        assert!(matches!(app.screen, Screen::SelectDifficulty(0, None)));
    }

    // --- リズムゲーム: Menu → 曲選択 → 難易度選択 → プレイ ---

    #[test]
    fn rhythm_item_index_points_at_rhythm_menu_item() {
        assert_eq!(MENU_ITEMS[RHYTHM_ITEM_INDEX], "リズム(DDR風)");
    }

    #[test]
    fn selecting_rhythm_menu_item_enters_song_select_instead_of_difficulty() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        assert!(matches!(app.screen, Screen::SelectSong(0)));
    }

    #[test]
    fn song_select_up_down_wraps_around() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.handle_key(KeyEvent::from(KeyCode::Up));
        assert!(matches!(app.screen, Screen::SelectSong(i) if i == SONGS.len() - 1));
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert!(matches!(app.screen, Screen::SelectSong(0)));
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert!(matches!(app.screen, Screen::SelectSong(1)));
    }

    #[test]
    fn song_select_enter_goes_to_difficulty_with_selected_song() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(1);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(
            app.screen,
            Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1))
        ));
    }

    #[test]
    fn song_select_number_key_picks_song_directly() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.handle_key(KeyEvent::from(KeyCode::Char('2')));
        assert!(matches!(
            app.screen,
            Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1))
        ));
    }

    #[test]
    fn song_select_number_key_out_of_range_is_ignored() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.handle_key(KeyEvent::from(KeyCode::Char('9')));
        assert!(matches!(app.screen, Screen::SelectSong(0)));
    }

    #[test]
    fn song_select_esc_returns_to_menu() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(1);
        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::Menu));
    }

    #[test]
    fn rhythm_difficulty_esc_returns_to_song_select() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1));
        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::SelectSong(1)));
    }

    #[test]
    fn other_game_difficulty_esc_still_returns_to_menu() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::Menu));
    }

    /// 画面を描画し、全セルを1つの文字列にして返す(描画内容の確認用)
    fn rendered_text(app: &mut App) -> String {
        let backend = ratatui::backend::TestBackend::new(80, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn full_rhythm_flow_starts_selected_song_with_its_bgm() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Down));
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        app.handle_key(KeyEvent::from(KeyCode::Char('3')));
        let Screen::Playing(game) = &app.screen else {
            panic!("Playing画面のはず");
        };
        let result = game.result();
        assert_eq!(result.game_id, crate::game::rhythm::GAME_ID);
        assert_eq!(result.difficulty, Difficulty::Advanced);
        // 選んだ曲のBGMが流れ、プレイ画面にも曲名が出る
        assert_eq!(app.current_bgm.as_deref(), Some(SONGS[1].track_name));
        assert!(rendered_text(&mut app).contains(SONGS[1].display_name));
    }

    #[test]
    fn song_select_screen_lists_every_song() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        let text = rendered_text(&mut app);
        for song in SONGS {
            assert!(
                text.contains(song.display_name),
                "{}が表示されること",
                song.display_name
            );
        }
    }

    #[test]
    fn rhythm_difficulty_screen_shows_selected_song_name() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1));
        assert!(rendered_text(&mut app).contains(SONGS[1].display_name));
    }

    #[test]
    fn song_at_row_maps_each_song_row() {
        let area = rect(0, 0, 40, 12);
        let inner_top = Block::default().borders(Borders::ALL).inner(area).y;
        for i in 0..SONGS.len() {
            assert_eq!(
                song_at_row(area, inner_top + SONG_ROWS_OFFSET + i as u16),
                Some(i)
            );
        }
        assert_eq!(song_at_row(area, inner_top), None);
        assert_eq!(
            song_at_row(area, inner_top + SONG_ROWS_OFFSET + SONGS.len() as u16),
            None
        );
    }

    #[test]
    fn clicking_song_row_goes_to_difficulty_with_that_song() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.last_area = rect(0, 0, 40, 12);
        let inner_top = Block::default()
            .borders(Borders::ALL)
            .inner(app.last_area)
            .y;
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: inner_top + SONG_ROWS_OFFSET + 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert!(matches!(
            app.screen,
            Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1))
        ));
    }

    #[test]
    fn jukebox_up_down_wraps_around_track_list() {
        let mut app = App::new();
        app.screen = Screen::Jukebox(app.jukebox_list_state());
        let tracks = audio::bgm_track_names();
        let len = tracks.len();

        // 先頭で上キーを押すと末尾に巡回する
        if let Screen::Jukebox(state) = &mut app.screen {
            state.select(Some(0));
        }
        app.handle_jukebox_key(KeyEvent::from(KeyCode::Up));
        let Screen::Jukebox(state) = &app.screen else {
            panic!("Jukebox画面のはず");
        };
        assert_eq!(state.selected(), Some(len - 1));
    }

    #[test]
    fn jukebox_enter_sets_current_bgm_to_selected_track() {
        let mut app = App::new();
        app.screen = Screen::Jukebox(app.jukebox_list_state());
        let tracks = audio::bgm_track_names();
        if let Screen::Jukebox(state) = &mut app.screen {
            state.select(Some(0));
        }
        app.handle_jukebox_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(app.current_bgm.as_deref(), Some(tracks[0].as_str()));
    }

    #[test]
    fn jukebox_stop_key_clears_current_bgm() {
        let mut app = App::new();
        app.screen = Screen::Jukebox(app.jukebox_list_state());
        app.handle_jukebox_key(KeyEvent::from(KeyCode::Char('s')));
        assert_eq!(app.current_bgm, None);
    }

    #[test]
    fn jukebox_esc_returns_to_menu() {
        let mut app = App::new();
        app.screen = Screen::Jukebox(app.jukebox_list_state());
        app.handle_jukebox_key(KeyEvent::from(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::Menu));
    }

    // --- 描画位置とクリック判定の一致 ---

    /// 画面を描画し、各行を空白抜きの文字列にして返す
    /// (全角文字の2セル目は空白で埋まるため、空白を除いて比較する)
    fn rendered_rows_without_spaces(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .replace(' ', "")
            })
            .collect()
    }

    #[test]
    fn menu_items_are_drawn_on_the_rows_that_click_to_them() {
        // 画面の高さによって1項目の高さが変わっても、項目名が見えている行をクリックすればその項目になること
        for height in [24u16, 30, 45] {
            let mut app = App::new();
            let rows = rendered_rows_without_spaces(&mut app, 80, height);
            let area = rect(0, 0, 80, height);
            for (i, name) in MENU_ITEMS.iter().enumerate() {
                let row = rows
                    .iter()
                    .position(|r| r.contains(name))
                    .unwrap_or_else(|| panic!("height={height}: {name}が描かれていること"));
                assert_eq!(
                    menu_item_at_row(area, row as u16),
                    Some(i),
                    "height={height}: {name}の行"
                );
            }
        }
    }

    #[test]
    fn difficulty_rows_are_drawn_where_difficulty_at_row_expects() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        let rows = rendered_rows_without_spaces(&mut app, 80, 24);
        let area = rect(0, 0, 80, 24);
        for (label, expected) in [
            ("初級", Difficulty::Beginner),
            ("中級", Difficulty::Intermediate),
            ("上級", Difficulty::Advanced),
        ] {
            let row = rows
                .iter()
                .position(|r| r.contains(label))
                .unwrap_or_else(|| panic!("{label}が描かれていること"));
            assert_eq!(difficulty_at_row(area, row as u16), Some(expected), "{label}の行");
        }
    }

    #[test]
    fn song_rows_are_drawn_where_song_at_row_expects() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        let rows = rendered_rows_without_spaces(&mut app, 80, 24);
        let area = rect(0, 0, 80, 24);
        for (i, song) in SONGS.iter().enumerate() {
            let name = song.display_name.replace(' ', "");
            let row = rows
                .iter()
                .position(|r| r.contains(&name))
                .unwrap_or_else(|| panic!("{}が描かれていること", song.display_name));
            assert_eq!(song_at_row(area, row as u16), Some(i));
        }
    }
}
