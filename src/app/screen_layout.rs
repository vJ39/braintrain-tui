//! 画面全体(frame.area())に対する四辺の余白、共通背景の敷き方、矩形の中央配置

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

use crate::ui::background::BackgroundRenderer;

/// 画面の四辺に残す余白(左右・上下のセル数)。この余白に背景画像が見える。
/// 端末の1セルは縦長(おおむね横:縦=1:2)なので、左右は上下の倍にして見た目の太さをそろえる
const SCREEN_MARGIN_X: u16 = 4;
const SCREEN_MARGIN_Y: u16 = 2;

/// 画面全体(area)から四辺に余白を持たせた中央のRect。この余白に背景画像を見せる。
/// 余白を引くと幅・高さが0以下になるほど小さい画面では、余白なし(area全体)にする
pub(super) fn screen_rect(area: Rect) -> Rect {
    if area.width <= 2 * SCREEN_MARGIN_X || area.height <= 2 * SCREEN_MARGIN_Y {
        return area;
    }
    Rect::new(
        area.x + SCREEN_MARGIN_X,
        area.y + SCREEN_MARGIN_Y,
        area.width - 2 * SCREEN_MARGIN_X,
        area.height - 2 * SCREEN_MARGIN_Y,
    )
}

/// 共通の背景画像を画面全体(area)に敷き、各画面を描く中央の範囲(screen_rect)を返す。
/// 画像プロトコルは画像の範囲の左上以外のセルを「端末へ出力しない」(skip)にするため、
/// 各画面を描く範囲はClearでskipを外してから返す(外さないとその上に描いた枠・文字が
/// 端末に出ない)。余白のセルはskipのまま残り、端末上では背景画像だけが見える。
/// 余白が取れない小さい画面では背景を描かない
pub(super) fn render_background(
    frame: &mut Frame,
    background: &mut BackgroundRenderer,
    area: Rect,
) -> Rect {
    let screen = screen_rect(area);
    if screen != area {
        background.render(frame, area);
        frame.render_widget(ratatui::widgets::Clear, screen);
    }
    screen
}

pub(super) fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
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
    use crossterm::event::KeyCode;
    use ratatui_image::picker::ProtocolType;

    use super::super::menu_items::MENU_ITEMS;
    use super::super::test_support::*;
    use super::super::{App, Screen};
    use super::*;

    /// 背景画像を描けるBackgroundRenderer(ハーフブロック描画)。デバッグビルドでは背景画像の
    /// デコードに時間がかかるため、テスト内では1つを作ってswap_backgroundで使い回す
    fn halfblocks_background() -> BackgroundRenderer {
        BackgroundRenderer::with_picker(Some(test_picker(ProtocolType::Halfblocks)))
    }

    /// appの背景をbackgroundに差し替え、それまでの背景を返す
    fn swap_background(app: &mut App, background: BackgroundRenderer) -> BackgroundRenderer {
        std::mem::replace(&mut app.background, background)
    }

    #[test]
    fn screen_rect_is_smaller_and_centered_with_margins_on_every_side() {
        for (width, height) in [(80u16, 24u16), (80, 30), (120, 40), (200, 60), (40, 12)] {
            let area = rect(0, 0, width, height);
            let screen = screen_rect(area);
            assert!(
                screen.width < area.width && screen.height < area.height,
                "{width}x{height}: 一回り小さい"
            );
            assert_eq!(
                screen.intersection(area),
                screen,
                "{width}x{height}: areaの内側"
            );
            let (left, right) = (screen.x - area.x, area.right() - screen.right());
            let (top, bottom) = (screen.y - area.y, area.bottom() - screen.bottom());
            assert!(left > 0 && top > 0, "{width}x{height}: 四辺に余白がある");
            assert_eq!(left, right, "{width}x{height}: 左右の余白が同じ");
            assert_eq!(top, bottom, "{width}x{height}: 上下の余白が同じ");
        }
    }

    #[test]
    fn screen_rect_follows_the_position_of_the_area() {
        let base = screen_rect(rect(0, 0, 80, 24));
        let moved = screen_rect(rect(3, 5, 80, 24));
        assert_eq!((moved.x, moved.y), (base.x + 3, base.y + 5));
        assert_eq!((moved.width, moved.height), (base.width, base.height));
    }

    #[test]
    fn screen_rect_of_a_too_small_screen_is_the_whole_area() {
        for (width, height) in [
            (0u16, 0u16),
            (1, 1),
            (4, 30),
            (80, 2),
            (8, 4),
            (0, 30),
            (80, 0),
        ] {
            let area = rect(0, 0, width, height);
            assert_eq!(screen_rect(area), area, "{width}x{height}: 余白なし");
        }
    }

    #[test]
    fn screen_rect_never_becomes_empty_for_a_non_empty_area() {
        for width in 0..60u16 {
            for height in 0..30u16 {
                let area = rect(0, 0, width, height);
                let screen = screen_rect(area);
                assert_eq!(screen.intersection(area), screen, "{width}x{height}");
                if !area.is_empty() {
                    assert!(
                        screen.width > 0 && screen.height > 0,
                        "{width}x{height}: 幅・高さが0にならない"
                    );
                }
            }
        }
    }

    /// 背景を敷く各画面(画面の説明, App)。TTR(SelectSong)とプレイ中は含まない
    fn apps_on_every_background_screen() -> Vec<(&'static str, App)> {
        let mut apps = vec![("Splash", App::new()), ("Menu", app_on_menu())];
        apps.push(("ConfirmQuit", app_on_confirm_quit()));

        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        apps.push(("SelectDifficulty", app));

        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        press(&mut app, KeyCode::Char('1'));
        assert!(matches!(app.screen, Screen::Countdown { .. }));
        apps.push(("Countdown", app));

        let mut app = App::new();
        app.screen = Screen::Result(sample_result(), Some("保存エラー".to_string()));
        apps.push(("Result", app));

        let mut app = App::new();
        app.screen = Screen::History;
        apps.push(("History", app));

        let mut app = App::new();
        app.screen = Screen::Jukebox(app.jukebox_list_state());
        apps.push(("Jukebox", app));
        apps
    }

    #[test]
    fn every_background_screen_is_drawn_inside_screen_rect() {
        // 外枠・文字は全てscreen_rectの内側に描かれ、四辺の余白には一切描かない
        for (width, height) in [(80u16, 24u16), (80, 30), (200, 60)] {
            let area = rect(0, 0, width, height);
            let screen = screen_rect(area);
            for (name, mut app) in apps_on_every_background_screen() {
                let rows = rendered_cells(&mut app, width, height);
                for position in area.positions() {
                    if !screen.contains(position) {
                        assert_eq!(
                            rows[position.y as usize][position.x as usize], " ",
                            "{name} {width}x{height}: 余白({},{})には描かない",
                            position.x, position.y
                        );
                    }
                }
                let drawn = screen
                    .positions()
                    .any(|p| rows[p.y as usize][p.x as usize] != " ");
                assert!(drawn, "{name} {width}x{height}: screen_rectの内側に描く");
            }
        }
    }

    #[test]
    fn framed_background_screens_put_their_outer_frame_on_screen_rect() {
        // 外枠のある画面は、枠の左上の角がareaの端ではなくscreen_rectの左上に来る
        let area = rect(0, 0, 80, 30);
        let screen = screen_rect(area);
        for (name, mut app) in apps_on_every_background_screen() {
            if name == "Countdown" {
                continue; // カウントダウンは枠を持たない
            }
            let rows = rendered_cells(&mut app, 80, 30);
            let corner = &rows[screen.y as usize][screen.x as usize];
            assert!(
                ["╭", "╔", "┌"].contains(&corner.as_str()),
                "{name}: screen_rectの左上が外枠の角(実際: {corner:?})"
            );
            assert_eq!(rows[0][0], " ", "{name}: areaの左上には描かない");
        }
    }

    #[test]
    fn background_fills_the_margins_and_the_ui_stays_visible() {
        // 背景画像を描ける場合、余白には背景が見え、screen_rectの内側にはUIが描かれる
        let screen = screen_rect(rect(0, 0, 80, 30));
        let mut background = halfblocks_background();
        for (name, mut app) in apps_on_every_background_screen() {
            let (without, with) = buffers_with_and_without_background(&mut app, &mut background);
            let default = ratatui::buffer::Cell::default();
            for (x, y) in [(0, 0), (79, 0), (0, 29), (79, 29)] {
                assert_ne!(
                    with[(x, y)],
                    default,
                    "{name}: 余白({x},{y})に背景が描かれる"
                );
            }
            // 履歴画面の中身は履歴ファイルの内容で決まり、並行して動く他のテストが
            // 結果を保存すると2回の描画の間に変わりうるので、内側の比較はしない
            if name == "History" {
                continue;
            }
            for p in screen.positions() {
                assert_eq!(
                    with[p], without[p],
                    "{name}: screen_rectの内側({},{})は背景の有無で変わらない",
                    p.x, p.y
                );
            }
        }
    }

    #[test]
    fn image_protocol_background_does_not_hide_the_ui() {
        // 画像プロトコルは画像の範囲のセルをskip(端末へ出力しない)にする。
        // screen_rectの内側はskipを外してから描くので、UIの文字が端末へ出力される。
        // 余白のセルはskipのまま(=端末上では背景画像だけが見える)
        let mut app = app_on_menu();
        app.background = BackgroundRenderer::with_picker(Some(test_picker(ProtocolType::Iterm2)));
        let (width, height) = (80u16, 30u16);
        let area = rect(0, 0, width, height);
        let screen = screen_rect(area);
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let buffer = terminal
            .draw(|frame| app.render(frame))
            .unwrap()
            .buffer
            .clone();

        assert!(
            buffer[(0, 0)].symbol().starts_with('\x1b'),
            "左上のセルに背景画像のデータ"
        );
        for position in area.positions() {
            let cell = &buffer[position];
            if screen.contains(position) {
                assert!(
                    !cell.skip,
                    "screen_rectの内側({},{})は出力される",
                    position.x, position.y
                );
            } else if position != ratatui::layout::Position::new(0, 0) {
                assert!(
                    cell.skip,
                    "余白({},{})は背景画像のみ",
                    position.x, position.y
                );
            }
        }
        // 実際に端末(TestBackend)へ出力された内容にメニューの文字が含まれる
        let emitted: String = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<String>()
            .replace(' ', "");
        assert!(emitted.contains("BRAINTRAIN"), "外枠の見出しが出力される");
        assert!(emitted.contains(MENU_ITEMS[0]), "メニュー項目が出力される");
    }

    /// 背景画像の有無を切り替えて同じ画面(80x30)を描き、(背景なし, 背景あり)のバッファを返す。
    /// backgroundは背景ありの描画に使い、描画後にappから取り戻す
    fn buffers_with_and_without_background(
        app: &mut App,
        background: &mut BackgroundRenderer,
    ) -> (ratatui::buffer::Buffer, ratatui::buffer::Buffer) {
        let draw = |app: &mut App| {
            let backend = ratatui::backend::TestBackend::new(80, 30);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| app.render(frame))
                .unwrap()
                .buffer
                .clone()
        };
        swap_background(app, BackgroundRenderer::with_picker(None));
        let without = draw(app);
        let taken = std::mem::replace(background, BackgroundRenderer::with_picker(None));
        swap_background(app, taken);
        let with = draw(app);
        *background = swap_background(app, BackgroundRenderer::with_picker(None));
        (without, with)
    }

    #[test]
    fn song_select_and_playing_do_not_draw_the_background() {
        // TTR(曲選択)は既存のTTRスプラッシュ画像を背景にし、プレイ中は対象外
        let mut background = halfblocks_background();
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        let (without, with) = buffers_with_and_without_background(&mut app, &mut background);
        assert_eq!(
            without, with,
            "SelectSong: 背景画像の有無で描画が変わらない"
        );

        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        press(&mut app, KeyCode::Char('1'));
        finish_countdown(&mut app);
        let (without, with) = buffers_with_and_without_background(&mut app, &mut background);
        assert_eq!(without, with, "Playing: 背景画像の有無で描画が変わらない");
    }

    #[test]
    fn background_screens_render_without_panicking_at_any_size() {
        let mut background = halfblocks_background();
        for (width, height) in [
            (1u16, 1u16),
            (2, 2),
            (5, 3),
            (9, 5),
            (10, 6),
            (20, 8),
            (80, 24),
        ] {
            for (_, mut app) in apps_on_every_background_screen() {
                swap_background(&mut app, background);
                rendered_cells(&mut app, width, height);
                background = swap_background(&mut app, BackgroundRenderer::with_picker(None));
            }
        }
    }
}
