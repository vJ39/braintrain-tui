//! Enter/クリックで次へ進む全画面スプラッシュ3種。起動直後のタイトル画面(Screen::Splash)と、
//! 「べー」開始前の専用スプラッシュ画面(Screen::BeigomaSplash、コマ送りアニメーション)、
//! その後べー本編開始前に挟むキャラクター静止画スプラッシュ画面(Screen::BeigomaCharacterSplash)

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::audio::{self, BgmCategory, SeKind};
use crate::game::theme;
use crate::ui::beigoma_splash_anim::AnimatedSplash;
use crate::ui::splash::SplashRenderer;

use super::{App, Screen};

impl App {
    /// タイトル画面(Splash)からメニューへ進む(Enter/クリック共通)
    pub(super) fn leave_splash(&mut self) {
        audio::play_se(SeKind::Confirm);
        self.enter_menu();
    }

    /// べースプラッシュ画面(動画)からキャラクター静止画スプラッシュ画面へ進む(Enter/クリック共通)。
    /// BGMはべー専用のものをそのまま流し続ける
    pub(super) fn leave_beigoma_splash(&mut self) {
        audio::play_se(SeKind::Confirm);
        self.screen = Screen::BeigomaCharacterSplash;
    }

    /// キャラクター静止画スプラッシュ画面からべーを開始する(Enter/クリック共通)
    pub(super) fn leave_beigoma_character_splash(&mut self) {
        audio::play_se(SeKind::Confirm);
        self.start_beigoma();
    }

    /// メニューで「べー」を選んだ時に、べー開始前のスプラッシュ画面へ進む。
    /// コマ送りアニメーションは最初のコマから始める。BGMもべー専用のものに切り替え、
    /// べー本編が始まるまで(start_beigomaでPlaying用BGMに切り替わるまで)流し続ける
    pub(super) fn enter_beigoma_splash(&mut self) {
        audio::play_se(SeKind::Transition);
        if let Some(name) = audio::random_bgm_track(BgmCategory::BeigomaSplash) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        self.beigoma_splash_renderer.reset();
        self.screen = Screen::BeigomaSplash;
    }
}

/// 静止画1枚のスプラッシュ画面(タイトル画面・べーのキャラクター静止画スプラッシュ画面)。
/// メニューと同じ色・角丸の枠を画面いっぱいに描き、内側の中央に画像を配置する
pub(super) fn render_still(frame: &mut Frame, area: Rect, renderer: &mut SplashRenderer) {
    let block = theme::panel("");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    renderer.render(frame, inner);
}

/// べー開始前のスプラッシュ画面。タイトル画面と同じ枠の内側にコマ送りアニメーションを描く
pub(super) fn render_beigoma(frame: &mut Frame, area: Rect, renderer: &mut AnimatedSplash) {
    let block = theme::panel("");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    renderer.render(frame, inner);
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

    use super::super::screen_layout::screen_rect;
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn app_starts_on_splash_screen() {
        let app = App::new();
        assert!(matches!(app.screen, Screen::Splash));
    }

    #[test]
    fn splash_enter_key_transitions_to_menu() {
        let mut app = App::new();
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(app.screen, Screen::Menu));
    }

    #[test]
    fn splash_screen_is_framed_by_a_full_screen_border() {
        // メニューと同じ色(theme::ACCENT)の角丸枠(Rounded)が、背景の余白を残した画面いっぱいに出る
        let mut app = App::new();
        let text = rendered_text(&mut app);
        assert!(
            text.contains('╭') && text.contains('╮') && text.contains('╰') && text.contains('╯'),
            "画面いっぱいの角丸枠が出ること"
        );
        let buffer = {
            let backend = ratatui::backend::TestBackend::new(80, 30);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|frame| app.render(frame)).unwrap();
            terminal.backend().buffer().clone()
        };
        let screen = screen_rect(rect(0, 0, 80, 30));
        assert_eq!(
            buffer[(screen.x, screen.y)].symbol(),
            "╭",
            "枠の左上はscreen_rectの左上"
        );
        assert_eq!(
            buffer[(screen.x, screen.y)].fg,
            theme::ACCENT,
            "枠の色はメニューと同じACCENT"
        );
    }

    #[test]
    fn splash_other_key_stays_on_splash() {
        let mut app = App::new();
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert!(matches!(app.screen, Screen::Splash));
    }

    #[test]
    fn splash_click_transitions_to_menu() {
        let mut app = App::new();
        app.last_area = rect(0, 0, 40, 12);
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert!(matches!(app.screen, Screen::Menu));
    }

    #[test]
    fn splash_renders_without_panicking() {
        let mut app = App::new();
        rendered_text(&mut app);
    }

    // --- べーのキャラクター静止画スプラッシュ(Screen::BeigomaCharacterSplash) ---

    #[test]
    fn character_splash_renderer_uses_its_own_fallback() {
        // 画像を出せる環境ならImage、出せない環境なら専用のフォールバック文言になる
        let app = App::new();
        match &app.beigoma_character_splash_renderer {
            SplashRenderer::Image { size, .. } => assert_eq!(*size, (1024, 1024)),
            SplashRenderer::Fallback(fallback) => {
                assert_eq!(*fallback, crate::ui::splash::BEIGOMA_CHARACTER_FALLBACK)
            }
        }
    }

    #[test]
    fn character_splash_is_framed_like_the_title_screen() {
        let mut app = App::new();
        app.screen = Screen::BeigomaCharacterSplash;
        let buffer = {
            let backend = ratatui::backend::TestBackend::new(80, 30);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|frame| app.render(frame)).unwrap();
            terminal.backend().buffer().clone()
        };
        let screen = screen_rect(rect(0, 0, 80, 30));
        assert_eq!(buffer[(screen.x, screen.y)].symbol(), "╭");
        assert_eq!(buffer[(screen.x, screen.y)].fg, theme::ACCENT);
    }

    #[test]
    fn character_splash_renders_without_panicking_on_tiny_terminals() {
        let mut app = App::new();
        app.screen = Screen::BeigomaCharacterSplash;
        for (w, h) in [(1, 1), (10, 4), (200, 60)] {
            let backend = ratatui::backend::TestBackend::new(w, h);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|frame| app.render(frame)).unwrap();
        }
    }
}
