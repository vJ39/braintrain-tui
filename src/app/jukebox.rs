//! ジュークボックス画面(Screen::Jukebox)。BGMの曲リストから選んで再生・停止する

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::theme;

use super::{App, Screen};

impl App {
    /// メニューからジュークボックス画面へ入る。現在再生中の曲を選択済みにして開く
    pub(super) fn enter_jukebox(&mut self) {
        self.screen = Screen::Jukebox(self.jukebox_list_state());
    }

    /// 現在再生中の曲を選択済みにしたジュークボックス画面用ListStateを作る
    pub(super) fn jukebox_list_state(&self) -> ListState {
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

    pub(super) fn handle_jukebox_key(&mut self, key: KeyEvent) {
        let tracks = audio::bgm_track_names();
        let len = tracks.len().max(1);
        let selected = if let Screen::Jukebox(state) = &self.screen {
            state.selected().unwrap_or(0)
        } else {
            return;
        };

        match key.code {
            KeyCode::Up => {
                audio::play_se(SeKind::CursorMove);
                if let Screen::Jukebox(state) = &mut self.screen {
                    state.select(Some((selected + len - 1) % len));
                }
            }
            KeyCode::Down => {
                audio::play_se(SeKind::CursorMove);
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
                self.enter_menu();
            }
            _ => {}
        }
    }
}

pub(super) fn render(frame: &mut Frame, area: Rect, state: &mut ListState, playing: Option<&str>) {
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
        &[
            ("↑↓", "選択"),
            ("Enter", "再生"),
            ("S", "停止"),
            ("Esc", "戻る"),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
