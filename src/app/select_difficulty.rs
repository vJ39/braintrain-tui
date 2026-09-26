//! 難易度選択画面(Screen::SelectDifficulty)

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::game::rhythm::SONGS;
use crate::game::theme;
use crate::game::Difficulty;

use super::menu_items::MENU_ITEMS;
use super::screen_layout::screen_rect;
use super::{App, Screen};

impl App {
    pub(super) fn handle_difficulty_key(
        &mut self,
        key: KeyEvent,
        item: usize,
        song: Option<usize>,
    ) {
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
            self.start_playing(item, difficulty);
        }
    }

    /// 難易度選択画面でのクリック。描画と同じくscreen_rect基準で行を判定し、難易度の行ならそのゲームを開始する
    pub(super) fn handle_difficulty_mouse(&mut self, mouse: MouseEvent, item: usize) {
        if let Some(difficulty) = difficulty_at_row(screen_rect(self.last_area), mouse.row) {
            self.start_playing(item, difficulty);
        }
    }
}

/// SelectDifficulty画面で、難易度の行(初級/中級/上級)が内部エリアの何行目から
/// 始まるか。renderとdifficulty_at_rowで一致させること
const DIFFICULTY_ROWS_OFFSET: u16 = 2;

/// 難易度選択画面。見出しはメニュー項目名(リズムゲームの場合は「項目名 / 曲名」)
pub(super) fn render(frame: &mut Frame, area: Rect, item: usize, song: Option<usize>) {
    let game_name = match song.and_then(|s| SONGS.get(s)) {
        Some(song) => format!("{} / {}", MENU_ITEMS[item], song.display_name),
        None => MENU_ITEMS[item].to_string(),
    };
    // 行の並びはdifficulty_at_rowと一致させる(1行目=見出し、2行目=空行、3〜5行目=初級/中級/上級)
    let mut text = vec![
        Line::from(vec![
            Span::styled("◆ ", Style::default().fg(theme::ACCENT)),
            Span::styled(game_name, theme::title_style()),
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
    let note_width = options
        .iter()
        .map(|(_, _, n)| n.chars().count())
        .max()
        .unwrap_or(0);
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
    let relative = mouse_row
        .checked_sub(inner.y)?
        .checked_sub(DIFFICULTY_ROWS_OFFSET)?;
    match relative {
        0 => Some(Difficulty::Beginner),
        1 => Some(Difficulty::Intermediate),
        2 => Some(Difficulty::Advanced),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

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
    fn other_game_difficulty_esc_still_returns_to_menu() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::Menu));
    }

    #[test]
    fn difficulty_rows_are_drawn_where_difficulty_at_row_expects() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        let rows = rendered_rows_without_spaces(&mut app, 80, 24);
        let area = screen_rect(rect(0, 0, 80, 24));
        for (label, expected) in [
            ("初級", Difficulty::Beginner),
            ("中級", Difficulty::Intermediate),
            ("上級", Difficulty::Advanced),
        ] {
            let row = rows
                .iter()
                .position(|r| r.contains(label))
                .unwrap_or_else(|| panic!("{label}が描かれていること"));
            assert_eq!(
                difficulty_at_row(area, row as u16),
                Some(expected),
                "{label}の行"
            );
        }
    }

    #[test]
    fn clicking_difficulty_also_goes_through_countdown() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        app.last_area = rect(0, 0, 80, 24);
        let inner_top = Block::default()
            .borders(Borders::ALL)
            .inner(screen_rect(app.last_area))
            .y;
        app.handle_mouse(left_click(inner_top + DIFFICULTY_ROWS_OFFSET + 2));
        assert!(matches!(
            app.screen,
            Screen::Countdown {
                item: 0,
                difficulty: Difficulty::Advanced,
                ..
            }
        ));
    }

    #[test]
    fn difficulty_margin_row_selects_nothing() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        app.last_area = rect(0, 0, 80, 24);
        // 余白の分だけずれる前の(area基準の)初級の行は、screen_rect基準では見出しの行になる
        let area_based_beginner = Block::default()
            .borders(Borders::ALL)
            .inner(app.last_area)
            .y
            + DIFFICULTY_ROWS_OFFSET;
        app.handle_mouse(left_click(0));
        app.handle_mouse(left_click(area_based_beginner));
        assert!(matches!(app.screen, Screen::SelectDifficulty(0, None)));
    }
}
