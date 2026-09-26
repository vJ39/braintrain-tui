//! 終了確認ダイアログ(Screen::ConfirmQuit)。メニューの上に重ねて描く

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::game::theme;

use super::screen_layout::centered_rect;
use super::App;

impl App {
    /// 終了確認ダイアログ: y/Enterで終了、n/Escでメニューに戻る。それ以外は無視する
    pub(super) fn handle_confirm_quit_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => self.should_quit = true,
            KeyCode::Char('n') | KeyCode::Esc => self.enter_menu(),
            _ => {}
        }
    }
}

/// 終了確認ダイアログ。背景のメニューが見えるよう、中央に小さなパネルを重ねて描く
pub(super) fn render(frame: &mut Frame, area: Rect) {
    let hints = theme::hints_line(&[("y / Enter", "終了"), ("n / Esc", "キャンセル")]);
    let text = vec![
        Line::from(""),
        Line::from(Span::styled("終了しますか？", theme::title_style())),
        Line::from(""),
        hints,
    ];
    // 枠(上下左右1セル)ぶんを足した大きさ。画面が小さければ画面に収まるよう縮める
    let content_width = text.iter().map(|l| l.width() as u16).max().unwrap_or(0);
    let width = (content_width + 4).min(area.width);
    let height = (text.len() as u16 + 2).min(area.height);
    let dialog = centered_rect(area, width, height);
    // 背景の全角文字がダイアログの左端をまたいでいると、端末出力時にその2セル目
    // (=ダイアログの左枠)が飛ばされて枠が欠ける。またいでいる文字は空白に置き換える
    if dialog.x > area.x {
        let buffer = frame.buffer_mut();
        for y in dialog.top()..dialog.bottom() {
            let cell = &mut buffer[(dialog.x - 1, y)];
            if Span::raw(cell.symbol()).width() > 1 {
                cell.set_symbol(" ");
            }
        }
    }
    frame.render_widget(ratatui::widgets::Clear, dialog);
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(theme::panel(Line::from(" 終了確認 ").centered()));
    frame.render_widget(paragraph, dialog);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::menu_items::MENU_ITEMS;
    use super::super::test_support::*;
    use super::super::Screen;
    use super::*;

    #[test]
    fn q_on_menu_opens_confirm_quit_without_quitting() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.screen, Screen::ConfirmQuit));
        assert!(!app.should_quit(), "確認ダイアログではまだ終了しない");
    }

    #[test]
    fn q_twice_from_a_non_menu_screen_ends_at_confirm_quit() {
        // メニュー以外 → [q]でメニュー → [q]で終了確認、の2段階になる
        let mut app = App::new();
        app.screen = Screen::History;
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.screen, Screen::ConfirmQuit));
        assert!(!app.should_quit());
    }

    #[test]
    fn confirm_quit_y_or_enter_quits() {
        for code in [KeyCode::Char('y'), KeyCode::Enter] {
            let mut app = app_on_confirm_quit();
            press(&mut app, code);
            assert!(app.should_quit(), "{code:?}で終了する");
        }
    }

    #[test]
    fn confirm_quit_n_or_esc_returns_to_menu_without_quitting() {
        for code in [KeyCode::Char('n'), KeyCode::Esc] {
            let mut app = app_on_confirm_quit();
            press(&mut app, code);
            assert!(
                matches!(app.screen, Screen::Menu),
                "{code:?}でメニューに戻る"
            );
            assert!(!app.should_quit(), "{code:?}では終了しない");
        }
    }

    #[test]
    fn confirm_quit_ignores_other_keys() {
        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Char('q'),
            KeyCode::Char('x'),
            KeyCode::Char('1'),
        ] {
            let mut app = app_on_confirm_quit();
            press(&mut app, code);
            assert!(
                matches!(app.screen, Screen::ConfirmQuit),
                "{code:?}では閉じない"
            );
            assert!(!app.should_quit(), "{code:?}では終了しない");
        }
    }

    #[test]
    fn cancelling_confirm_quit_keeps_menu_selection() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app.menu_state.select(5);
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Down); // ダイアログ中はメニューのカーソルが動かない
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.menu_state.selected(), 5);
    }

    #[test]
    fn confirm_quit_does_not_change_bgm() {
        let mut app = app_on_confirm_quit();
        let before = app.current_bgm.clone();
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.current_bgm, before);
    }

    #[test]
    fn mouse_clicks_on_confirm_quit_are_ignored() {
        let mut app = app_on_confirm_quit();
        app.last_area = rect(0, 0, 80, 30);
        for row in 0..30 {
            app.handle_mouse(left_click(row));
        }
        assert!(
            matches!(app.screen, Screen::ConfirmQuit),
            "背後のメニュー項目が選ばれないこと"
        );
        assert!(!app.should_quit());
    }

    #[test]
    fn confirm_quit_is_drawn_over_the_menu() {
        let mut app = app_on_confirm_quit();
        let rows = rendered_rows_without_spaces(&mut app, 80, 30);
        let text = rows.concat();
        assert!(text.contains("終了しますか"), "確認メッセージが出ること");
        // ダイアログの外側には背景としてメニューが見えている
        assert!(text.contains("BRAINTRAIN"), "背景にメニューが見えること");
        assert!(
            text.contains(MENU_ITEMS[0]),
            "背景にメニュー項目が見えること"
        );
    }

    #[test]
    fn confirm_quit_renders_without_panicking_at_any_size() {
        for (width, height) in [
            (1u16, 1u16),
            (2, 2),
            (5, 3),
            (10, 5),
            (20, 8),
            (40, 12),
            (80, 24),
            (120, 40),
            (250, 80),
        ] {
            let mut app = app_on_confirm_quit();
            rendered_rows_without_spaces(&mut app, width, height);
        }
    }

    #[test]
    fn confirm_quit_dialog_border_is_not_hidden_by_wide_menu_text() {
        // 背景メニューの全角文字がダイアログの左端をまたいでいても、枠線が欠けないこと
        for width in 60u16..=100 {
            for height in [20u16, 24, 30] {
                let mut app = app_on_confirm_quit();
                // 背景のメニューのカードも角丸の枠なので、見出し「終了確認」の左にある角から辿る
                let (title_x, top) = drawn_position_of(&mut app, "終了確認", width, height)
                    .unwrap_or_else(|| {
                        panic!("{width}x{height}: ダイアログの見出しが描かれていること")
                    });
                let (title_x, top) = (title_x as usize, top as usize);
                let rows = rendered_cells(&mut app, width, height);
                let left = (0..title_x)
                    .rev()
                    .find(|&x| rows[top][x] == "╭")
                    .unwrap_or_else(|| panic!("{width}x{height}: 左上の角が描かれていること"));
                let bottom = (top + 1..rows.len())
                    .find(|&y| rows[y][left] != "│")
                    .unwrap_or_else(|| panic!("{width}x{height}: 下端があること"));
                assert_eq!(
                    rows[bottom][left], "╰",
                    "{width}x{height}: 左辺が途切れないこと"
                );
            }
        }
    }

    #[test]
    fn update_on_confirm_quit_keeps_the_dialog_open() {
        let mut app = app_on_confirm_quit();
        app.update(Duration::from_secs(10));
        assert!(matches!(app.screen, Screen::ConfirmQuit));
    }
}
