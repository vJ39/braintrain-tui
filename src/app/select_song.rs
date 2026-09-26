//! TTR(リズムゲーム)の曲選択画面(Screen::SelectSong)。TTRスプラッシュ画像を背景にした曲リストのパネル

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, BgmCategory, SeKind};
use crate::game::rhythm::SONGS;
use crate::game::theme;
use crate::ui::splash::SplashRenderer;

use super::menu_items::{MENU_ITEMS, RHYTHM_ITEM_INDEX};
use super::screen_layout::centered_rect;
use super::{App, Screen};

impl App {
    /// メニューでTTRを選んだ時に、TTRスプラッシュ画像を背景にした曲選択画面へ進む。BGMもTTR専用のものに
    /// 切り替え、実際に曲を選んでプレイが始まるまで(start_rhythmで曲のBGMに切り替わるまで)流し続ける
    pub(super) fn enter_song_select(&mut self) {
        audio::play_se(SeKind::Transition);
        if let Some(name) = audio::random_bgm_track(BgmCategory::RhythmSplash) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        self.screen = Screen::SelectSong(0);
    }

    /// 曲選択画面での曲決定(キー/クリック共通)。難易度選択を挟まず、その曲のプレイを始める
    pub(super) fn select_song(&mut self, song: usize) {
        audio::play_se(SeKind::TtrSongSelect);
        self.start_rhythm(song);
    }

    pub(super) fn handle_song_key(&mut self, key: KeyEvent, selected: usize) {
        let len = SONGS.len().max(1);
        match key.code {
            KeyCode::Up => {
                audio::play_se(SeKind::CursorMove);
                self.screen = Screen::SelectSong((selected + len - 1) % len);
            }
            KeyCode::Down => {
                audio::play_se(SeKind::CursorMove);
                self.screen = Screen::SelectSong((selected + 1) % len);
            }
            KeyCode::Enter => self.select_song(selected),
            // 数字キーで直接選ぶ(1始まり)。曲数を超える番号は無視する
            KeyCode::Char(c) => {
                if let Some(index) = c.to_digit(10).and_then(|d| (d as usize).checked_sub(1)) {
                    if index < SONGS.len() {
                        self.select_song(index);
                    }
                }
            }
            KeyCode::Esc => self.enter_menu(),
            _ => {}
        }
    }

    /// 曲選択画面でのクリック。曲の行(画面全体のarea基準)ならその曲のプレイを始める
    pub(super) fn handle_song_mouse(&mut self, mouse: MouseEvent) {
        if let Some(song) = song_at_row(self.last_area, mouse.row) {
            self.select_song(song);
        }
    }
}

/// SelectSong画面で、曲の行が内部エリアの何行目から始まるか。
/// renderとsong_at_rowで一致させること
const SONG_ROWS_OFFSET: u16 = 2;

/// 曲選択パネルの操作説明(枠の下辺に出す)
const SONG_SELECT_HINTS: [(&str, &str); 3] =
    [("↑↓", "選択"), ("Enter / 数字", "決定"), ("Esc", "戻る")];

/// 曲選択パネルの外枠(見出し・操作説明つき)
fn song_panel_block() -> Block<'static> {
    theme::panel(" ◆ BRAIN TRAIN ◆ ").title_bottom(theme::hints_line(&SONG_SELECT_HINTS).centered())
}

/// 曲選択パネルの中身(1行目=見出し、2行目=空行、3行目以降=曲)。
/// 行の並びはsong_at_rowと一致させる
fn song_select_lines(selected: usize) -> Vec<Line<'static>> {
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
            Line::from(Span::styled(
                format!(" ▶ {label} "),
                theme::selected_style(),
            ))
        } else {
            Line::from(Span::styled(
                format!("   {label} "),
                Style::default().fg(theme::TEXT),
            ))
        };
        text.push(line);
    }
    text
}

/// 曲選択パネルを置く位置。背景のTTR画像が見えるよう全画面にはせず、中身(曲リスト・
/// 操作説明)が収まる大きさで画面中央に置く。画面が小さければ画面に収まるよう縮める
fn song_panel_rect(area: Rect) -> Rect {
    // 選択中の行は「 ▶ 」で幅が広がるので、各曲を選択した時の幅で測る
    let content_width = (0..SONGS.len())
        .flat_map(song_select_lines)
        .map(|line| line.width() as u16)
        .max()
        .unwrap_or(0);
    let hints_width = theme::hints_line(&SONG_SELECT_HINTS).width() as u16;
    // 左右の枠(各1セル)と、枠の内側の左右の余白(各1セル)
    let width = (content_width.max(hints_width) + 4).min(area.width);
    // 上下の枠(各1セル)
    let height = (SONG_ROWS_OFFSET + SONGS.len() as u16 + 2).min(area.height);
    centered_rect(area, width, height)
}

/// 曲選択画面。TTRスプラッシュ画像を全画面に描いてから、その上に曲リストのパネルを重ねる
pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    selected: usize,
    ttr_splash: &mut SplashRenderer,
) {
    // Fallback表示(画像プロトコル非対応)は文言が画面中央に来るとパネルの裏に
    // 完全に隠れてしまうので、パネルより上の領域だけに表示する
    let bg_area = if ttr_splash.is_fallback() {
        let panel_top = song_panel_rect(area).y;
        Rect::new(area.x, area.y, area.width, panel_top.saturating_sub(area.y))
    } else {
        area
    };
    ttr_splash.render(frame, bg_area);

    let panel = song_panel_rect(area);
    // 背景に描いた画像・文字をパネルの範囲だけ消してから、その上にパネルを描く
    frame.render_widget(ratatui::widgets::Clear, panel);
    let paragraph = Paragraph::new(song_select_lines(selected))
        .alignment(Alignment::Center)
        .block(song_panel_block());
    frame.render_widget(paragraph, panel);
}

/// SelectSong画面でのクリック行(画面全体のarea基準)から曲インデックスを求める。
/// 曲選択パネル(song_panel_rect)の枠の内側の曲の行だけが対象
fn song_at_row(area: Rect, mouse_row: u16) -> Option<usize> {
    let inner = Block::default()
        .borders(Borders::ALL)
        .inner(song_panel_rect(area));
    let relative = mouse_row
        .checked_sub(inner.y)?
        .checked_sub(SONG_ROWS_OFFSET)? as usize;
    (relative < SONGS.len()).then_some(relative)
}

#[cfg(test)]
mod tests {
    use crate::game::Difficulty;

    use super::super::test_support::*;
    use super::*;

    #[test]
    fn selecting_rhythm_menu_item_goes_straight_to_song_select() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        assert!(
            matches!(app.screen, Screen::SelectSong(0)),
            "スプラッシュ画面を挟まず、TTR画像を背景にした曲選択画面へ直接進む"
        );
    }

    #[test]
    fn selecting_rhythm_menu_item_starts_the_ttr_splash_bgm() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        let tracks = audio::bgm_tracks_in(BgmCategory::RhythmSplash);
        assert!(app
            .current_bgm
            .as_ref()
            .is_some_and(|name| tracks.contains(name)));
    }

    #[test]
    fn ttr_splash_bgm_keeps_playing_through_song_select() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        let started_bgm = app.current_bgm.clone();
        assert!(matches!(app.screen, Screen::SelectSong(0)));
        // 曲を選び直している間もTTR専用BGMのまま
        app.handle_key(KeyEvent::from(KeyCode::Down));
        app.handle_key(KeyEvent::from(KeyCode::Up));
        rendered_text(&mut app);
        assert!(matches!(app.screen, Screen::SelectSong(0)));
        assert_eq!(
            app.current_bgm, started_bgm,
            "曲選択画面でもTTR専用BGMのまま"
        );
    }

    #[test]
    fn ttr_splash_bgm_keeps_playing_until_the_song_starts() {
        // 難易度選択を挟まなくなっても、曲を決定するまではTTR専用BGMが途切れない
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        let started_bgm = app.current_bgm.clone();
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert!(matches!(app.screen, Screen::SelectSong(1)));
        assert_eq!(app.current_bgm, started_bgm);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(app.screen, Screen::Playing(_)));
        assert_eq!(app.current_bgm.as_deref(), Some(SONGS[1].track_name));
    }

    #[test]
    fn starting_the_song_switches_from_ttr_splash_bgm_to_the_song() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        app.select_song(1);
        assert!(matches!(app.screen, Screen::Playing(_)));
        assert_eq!(app.current_bgm.as_deref(), Some(SONGS[1].track_name));
    }

    #[test]
    fn clicking_rhythm_menu_item_goes_straight_to_song_select() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        let (x, y) = drawn_position_of(&mut app, MENU_ITEMS[RHYTHM_ITEM_INDEX], 200, 60)
            .expect("TTRのカードが描かれていること");
        app.handle_mouse(left_click_at(x, y));
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
    fn song_select_other_key_stays_on_song_select() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.handle_key(KeyEvent::from(KeyCode::Left));
        app.handle_key(KeyEvent::from(KeyCode::Char('x')));
        assert!(matches!(app.screen, Screen::SelectSong(0)));
    }

    #[test]
    fn ttr_song_select_q_returns_to_menu() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.screen, Screen::Menu));
        assert!(!app.should_quit());
    }

    #[test]
    fn song_select_draws_the_ttr_background_behind_the_song_panel() {
        // 実行環境によりImage/Fallbackどちらになるかは変わる。Fallbackであれば、
        // 曲リストの外側に背景(TTRスプラッシュのフォールバック文言)が見えていること
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        let text = rendered_text(&mut app).replace(' ', "");
        for song in SONGS {
            assert!(text.contains(&song.display_name.replace(' ', "")));
        }
        if let crate::ui::splash::SplashRenderer::Fallback(ttr) = &app.ttr_splash_renderer {
            assert!(
                text.contains(&ttr.subtitle.replace(' ', "")),
                "曲選択パネルの背景にTTRスプラッシュが描かれること"
            );
            assert!(
                !text.contains(&crate::ui::splash::TITLE_FALLBACK.subtitle.replace(' ', "")),
                "タイトル画面の背景は使わない"
            );
        }
    }

    #[test]
    fn song_panel_is_centered_and_smaller_than_the_screen() {
        // 背景の画像が見えるよう、曲選択パネルは全画面ではなく中央に小さく重ねる
        let area = rect(0, 0, 80, 24);
        let panel = song_panel_rect(area);
        assert!(panel.width < area.width && panel.height < area.height);
        assert!(panel.x > 0 && panel.y > 0);
        assert!(panel.right() < area.right() && panel.bottom() < area.bottom());
        // 曲リスト(見出し2行+曲数)と枠が全部収まる高さがある
        assert!(panel.height >= SONG_ROWS_OFFSET + SONGS.len() as u16 + 2);
        // 画面が小さすぎる時は画面からはみ出さない
        let tiny = rect(0, 0, 20, 5);
        let panel = song_panel_rect(tiny);
        assert!(panel.width <= tiny.width && panel.height <= tiny.height);
    }

    #[test]
    fn ttr_splash_uses_its_own_renderer_not_the_title_one() {
        // 実行環境によりImage/Fallbackどちらになるかは変わるが、Fallbackであれば
        // タイトル画面とは別の(TTR用の)文言が出ること
        let app = App::new();
        if let (
            crate::ui::splash::SplashRenderer::Fallback(title),
            crate::ui::splash::SplashRenderer::Fallback(ttr),
        ) = (&app.splash_renderer, &app.ttr_splash_renderer)
        {
            assert_eq!(*title, crate::ui::splash::TITLE_FALLBACK);
            assert_eq!(*ttr, crate::ui::splash::TTR_FALLBACK);
        }
    }

    /// TTRのプレイ画面になっていて、指定した曲が上級で始まっていることを確かめる
    fn assert_rhythm_playing_song(app: &App, song: usize) {
        let Screen::Playing(game) = &app.screen else {
            panic!("難易度選択を挟まずPlaying画面になるはず");
        };
        let result = game.result();
        assert_eq!(result.game_id, crate::game::rhythm::GAME_ID);
        assert_eq!(result.difficulty, Difficulty::Advanced, "常に上級");
        assert_eq!(app.current_bgm.as_deref(), Some(SONGS[song].track_name));
    }

    #[test]
    fn song_select_enter_starts_playing_selected_song() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(1);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_rhythm_playing_song(&app, 1);
    }

    #[test]
    fn song_select_number_key_starts_playing_song_directly() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.handle_key(KeyEvent::from(KeyCode::Char('2')));
        assert_rhythm_playing_song(&app, 1);
    }

    #[test]
    fn song_select_can_start_the_last_song_overclocked_tempo() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        let last = SONGS.len() - 1;
        app.handle_key(KeyEvent::from(KeyCode::Up));
        assert!(matches!(app.screen, Screen::SelectSong(i) if i == last));
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_rhythm_playing_song(&app, last);
        assert_eq!(SONGS[last].track_name, "Overclocked_Tempo");
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
    fn full_rhythm_flow_starts_selected_song_with_its_bgm() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX); // TTR -> (TTR画像を背景にした)曲選択
        app.handle_key(KeyEvent::from(KeyCode::Down));
        app.handle_key(KeyEvent::from(KeyCode::Enter)); // 難易度選択を挟まずプレイ開始
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
    fn song_at_row_maps_each_song_row() {
        let area = rect(0, 0, 80, 24);
        let inner_top = Block::default()
            .borders(Borders::ALL)
            .inner(song_panel_rect(area))
            .y;
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
    fn clicking_song_row_starts_playing_that_song() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.last_area = rect(0, 0, 80, 24);
        let inner_top = Block::default()
            .borders(Borders::ALL)
            .inner(song_panel_rect(app.last_area))
            .y;
        app.handle_mouse(left_click(inner_top + SONG_ROWS_OFFSET + 1));
        assert_rhythm_playing_song(&app, 1);
    }

    #[test]
    fn clicking_outside_song_rows_stays_on_song_select() {
        // 背景(TTR画像)部分のクリックでは曲を決定しない
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.last_area = rect(0, 0, 80, 24);
        app.handle_mouse(left_click(0));
        app.handle_mouse(left_click(23));
        assert!(matches!(app.screen, Screen::SelectSong(0)));
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

    #[test]
    fn starting_rhythm_goes_straight_to_playing_without_countdown() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        let Screen::Playing(game) = &app.screen else {
            panic!("DDRはカウントダウンを挟まずPlaying画面になるはず");
        };
        assert_eq!(game.result().game_id, crate::game::rhythm::GAME_ID);
    }
}
