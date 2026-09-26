//! 2つ以上の画面モジュールのテストが使う共有ヘルパー(描画結果の取り出し・入力イベント・
//! 各画面にしたAppの用意など)

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui_image::picker::{Picker, ProtocolType};

use crate::game::{Difficulty, GameResult};

use super::menu_items::{new_game, MEMORY_ITEM_INDEX, REACTION_ITEM_INDEX, RHYTHM_ITEM_INDEX};
use super::{App, Screen};

// --- 描画 ---

/// 画面を描画し、全セルを1つの文字列にして返す(描画内容の確認用)
pub(super) fn rendered_text(app: &mut App) -> String {
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

/// 空白を除いた描画テキスト(80x30)
pub(super) fn rendered_compact(app: &mut App) -> String {
    rendered_text(app).replace(' ', "")
}

/// 画面を描画し、各行を空白抜きの文字列にして返す
/// (全角文字の2セル目は空白で埋まるため、空白を除いて比較する)
pub(super) fn rendered_rows_without_spaces(app: &mut App, width: u16, height: u16) -> Vec<String> {
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

/// 描画結果(TestBackendに実際に出力されたセル)を行ごとの文字の並びで返す。
/// 全角文字の2セル目は出力されないので、空白を詰めずセル単位で見る
pub(super) fn rendered_cells(app: &mut App, width: u16, height: u16) -> Vec<Vec<String>> {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect()
        })
        .collect()
}

/// 描画結果から、textが描かれている位置(先頭の文字のセル)を探す。
/// 全角文字の2セル目・空白は除いて行ごとに文字を連結し、その中からtextを探す
pub(super) fn drawn_position_of(
    app: &mut App,
    text: &str,
    width: u16,
    height: u16,
) -> Option<(u16, u16)> {
    let rows = rendered_cells(app, width, height);
    rows.iter().enumerate().find_map(|(y, row)| {
        let chars: Vec<(usize, &str)> = row
            .iter()
            .enumerate()
            .filter(|(_, s)| s.as_str() != " " && !s.is_empty())
            .map(|(x, s)| (x, s.as_str()))
            .collect();
        let joined: String = chars.iter().map(|(_, s)| *s).collect();
        let byte = joined.find(text)?;
        let mut offset = 0;
        chars.iter().find_map(|(x, s)| {
            let hit = (offset == byte).then_some((*x as u16, y as u16));
            offset += s.len();
            hit
        })
    })
}

// --- 入力 ---

pub(super) fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
    Rect::new(x, y, width, height)
}

pub(super) fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::from(code));
}

pub(super) fn left_click(row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 5,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}

pub(super) fn left_click_at(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}

// --- Appの用意 ---

/// メニュー画面(タイプライター表示済み)のAppを作る
pub(super) fn app_on_menu() -> App {
    let mut app = App::new();
    app.screen = Screen::Menu;
    app
}

/// タイトル画面からEnterでメニューへ入ったAppを作る(enter_menuを経由する)
pub(super) fn app_entering_menu() -> App {
    let mut app = App::new();
    app.last_area = rect(0, 0, 80, 30);
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.screen, Screen::Menu));
    app
}

/// メニューで[q]を押して終了確認ダイアログを開いたAppを作る
pub(super) fn app_on_confirm_quit() -> App {
    let mut app = App::new();
    app.screen = Screen::Menu;
    press(&mut app, KeyCode::Char('q'));
    assert!(matches!(app.screen, Screen::ConfirmQuit));
    app
}

/// リザルト画面へ入ったAppを作る(履歴ファイルへの保存は行わない)
pub(super) fn app_showing_result() -> App {
    let mut app = App::new();
    app.show_result(sample_result(), None);
    assert!(matches!(app.screen, Screen::Result(..)));
    app
}

/// メニュー以外の各画面にしたAppを(画面の説明, App)で返す
pub(super) fn apps_on_every_non_menu_screen() -> Vec<(&'static str, App)> {
    let mut apps = Vec::new();

    apps.push(("Splash", App::new()));

    let mut app = App::new();
    app.screen = Screen::SelectSong(1);
    apps.push(("SelectSong", app));

    let mut app = App::new();
    app.screen = Screen::SelectDifficulty(0, None);
    apps.push(("SelectDifficulty", app));

    let mut app = App::new();
    app.screen = Screen::SelectDifficulty(0, None);
    press(&mut app, KeyCode::Char('1'));
    assert!(matches!(app.screen, Screen::Countdown { .. }));
    apps.push(("Countdown", app));

    let mut app = App::new();
    app.screen = Screen::SelectDifficulty(0, None);
    press(&mut app, KeyCode::Char('1'));
    finish_countdown(&mut app);
    apps.push(("Playing", app));

    let mut app = App::new();
    app.screen = Screen::SelectSong(0);
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.screen, Screen::Playing(_)));
    apps.push(("Playing(リズム)", app));

    let mut app = App::new();
    app.screen = Screen::Result(new_game(0, Difficulty::Beginner).result(), None);
    apps.push(("Result", app));

    let mut app = App::new();
    app.screen = Screen::History;
    apps.push(("History", app));

    let mut app = App::new();
    app.select_menu_item(RHYTHM_ITEM_INDEX);
    assert!(matches!(app.screen, Screen::SelectSong(0)));
    apps.push(("SelectSong(TTR選択直後)", app));

    let mut app = App::new();
    app.screen = Screen::Jukebox(app.jukebox_list_state());
    apps.push(("Jukebox", app));

    apps
}

pub(super) fn sample_result() -> GameResult {
    new_game(0, Difficulty::Beginner).result()
}

/// 難易度選択を挟まない固定進行のゲーム(メニュー項目, GAME_ID, 記録する難易度)
pub(super) fn fixed_progression_games() -> [(usize, &'static str, Difficulty); 2] {
    [
        (
            MEMORY_ITEM_INDEX,
            crate::game::memory::GAME_ID,
            crate::game::memory::SESSION_DIFFICULTY,
        ),
        (
            REACTION_ITEM_INDEX,
            crate::game::reaction::GAME_ID,
            crate::game::reaction::SESSION_DIFFICULTY,
        ),
    ]
}

// --- カウントダウン ---

/// カウントダウン全体の長さ(3/2/1/GO!!の4フェーズ)
pub(super) const COUNTDOWN_TOTAL: Duration = Duration::from_millis(2400);

/// カウントダウンを最後まで進めてPlaying画面にする
pub(super) fn finish_countdown(app: &mut App) {
    assert!(
        matches!(app.screen, Screen::Countdown { .. }),
        "カウントダウン画面のはず"
    );
    app.update(COUNTDOWN_TOTAL);
    assert!(
        matches!(app.screen, Screen::Playing(_)),
        "Playing画面のはず"
    );
}

// --- 定数 ---

/// タイプライターを最後まで流し切るのに十分な時間
pub(super) const LONG_ENOUGH: Duration = Duration::from_secs(60);

/// 画像がまだ生成されていないアイコン(読めない間はカードのアイコン部分が空白になる)
pub(super) const PENDING_MENU_ICONS: [&str; 1] = ["menu_icons/look_away.png"];

// --- 画像 ---

/// 端末に問い合わせないpicker(1セル10x20px)
pub(super) fn test_picker(protocol: ProtocolType) -> Picker {
    let mut picker = Picker::from_fontsize((10, 20));
    picker.set_protocol_type(protocol);
    picker
}
