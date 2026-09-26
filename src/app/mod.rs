//! アプリ全体の状態(Screen / App)と、各画面へのディスパッチ(handle_key / handle_mouse /
//! update / render)。各画面の描画・入力処理・画面遷移は画面ごとのモジュールが持つ

mod confirm_quit;
mod game_start;
mod history;
mod jukebox;
mod menu;
mod menu_items;
mod result;
mod screen_layout;
mod select_difficulty;
mod select_song;
mod splash_screens;
#[cfg(test)]
mod test_support;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use ratatui::Frame;

use crate::audio::{self, BgmCategory};
use crate::game::{Difficulty, Game, GameResult};
use crate::ui::background::BackgroundRenderer;
use crate::ui::beigoma_splash_anim::AnimatedSplash;
use crate::ui::countdown::CountdownState;
use crate::ui::menu_icons::MenuIcons;
use crate::ui::result_sprite::ResultSprite;
use crate::ui::splash::SplashRenderer;
use crate::ui::typewriter::{self, Typewriter};

use menu::{MenuGridState, MENU_CHAR_INTERVAL};
use menu_items::MENU_ICON_PATHS;
use screen_layout::render_background;

pub enum Screen {
    /// 起動直後のタイトル画面。Enterを押すとMenuへ進む
    Splash,
    Menu,
    /// 「べー」開始前に挟む専用スプラッシュ画面(コマ送りアニメーション)。
    /// Enter/クリックでBeigomaCharacterSplashへ進む
    BeigomaSplash,
    /// BeigomaSplashの後、べー本編開始前に挟むキャラクター静止画スプラッシュ画面。
    /// Enter/クリックでべーが始まる
    BeigomaCharacterSplash,
    /// リズムゲームの曲選択(選択中の曲 = SONGSのインデックス)。
    /// TTRスプラッシュ画像を背景に、その上へ曲リストのパネルを重ねて描く
    SelectSong(usize),
    /// (メニュー項目, リズムゲームの場合は選んだ曲)
    SelectDifficulty(usize, Option<usize>),
    /// ゲーム開始前のカウントダウン(リズムゲーム以外)。終わるとitemのゲームを
    /// difficultyで作ってPlayingへ進む。ゲームは生成時に回答時間の計測を始めるため、
    /// カウントダウンが終わってから作る
    Countdown {
        item: usize,
        difficulty: Difficulty,
        state: CountdownState,
    },
    Playing(Box<dyn Game>),
    /// (結果, 履歴への保存に失敗していればそのエラー文字列)。
    /// 保存はここへ遷移する時に1回だけ行い、描画のたびに保存し直さない
    Result(GameResult, Option<String>),
    History,
    Jukebox(ListState),
    /// メニュー画面で[q]を押した時の終了確認ダイアログ。メニューの上に重ねて描く
    ConfirmQuit,
}

pub struct App {
    screen: Screen,
    /// メニュー画面のカードグリッドの選択位置・スクロール位置
    menu_state: MenuGridState,
    should_quit: bool,
    /// 直近のrender()で描画したフルスクリーンのエリア。マウス座標からのヒット
    /// テストに使う(render()より前にhandle_mouseが呼ばれることは無い前提)
    last_area: Rect,
    /// 現在再生中のBGMトラック名(ジュークボックス画面のハイライト表示に使う)
    current_bgm: Option<String>,
    splash_renderer: SplashRenderer,
    /// 曲選択画面(Screen::SelectSong)の背景に描くTTRスプラッシュ画像用
    ttr_splash_renderer: SplashRenderer,
    /// 「べー」開始前のスプラッシュ画面(Screen::BeigomaSplash)用のコマ送りアニメーション
    beigoma_splash_renderer: AnimatedSplash,
    /// 「べー」本編開始前のキャラクター静止画スプラッシュ画面(Screen::BeigomaCharacterSplash)用
    beigoma_character_splash_renderer: SplashRenderer,
    /// メニューへ戻った直後にtrueになる。main.rsがtake_pending_scrollback_clear()で
    /// 検知して端末のスクロールバッファをクリアする(画像プロトコルの残留対策)
    pending_scrollback_clear: bool,
    /// メニュー画面(各項目の名前・説明文)のタイプライター表示。enter_menuでリセットする
    menu_typewriter: Typewriter,
    /// リザルト画面のタイプライター表示。show_resultでリセットする
    result_typewriter: Typewriter,
    /// メニュー画面の各カードのアイコン画像。起動時に1回だけ読み込み、以後は使い回す
    menu_icons: MenuIcons,
    /// TTR(曲選択)・プレイ中以外の画面に共通で敷く背景画像。起動時に1回だけ読み込み、以後は使い回す
    background: BackgroundRenderer,
    /// リザルト画面のキャラクターのコマ送りアニメーション。起動時に1回だけ全コマを読み込み、
    /// 経過時間はshow_resultでリセットする
    result_sprite: ResultSprite,
}

impl App {
    pub fn new() -> Self {
        let current_bgm = audio::random_bgm_track(BgmCategory::Menu);
        if let Some(name) = &current_bgm {
            audio::play_bgm_track(name);
        }
        Self {
            screen: Screen::Splash,
            menu_state: MenuGridState::default(),
            should_quit: false,
            last_area: Rect::default(),
            current_bgm,
            splash_renderer: SplashRenderer::new(
                crate::ui::splash::TITLE_IMAGE_PATH,
                crate::ui::splash::TITLE_FALLBACK,
            ),
            ttr_splash_renderer: SplashRenderer::new(
                crate::ui::splash::TTR_SPLASH_IMAGE_PATH,
                crate::ui::splash::TTR_FALLBACK,
            ),
            beigoma_splash_renderer: AnimatedSplash::new(crate::ui::splash::BEIGOMA_FALLBACK),
            beigoma_character_splash_renderer: SplashRenderer::new(
                crate::ui::splash::BEIGOMA_CHARACTER_SPLASH_IMAGE_PATH,
                crate::ui::splash::BEIGOMA_CHARACTER_FALLBACK,
            ),
            pending_scrollback_clear: false,
            // 画面に入る時(enter_menu/show_result)にリセットするので、それまでは表示済みにしておく
            menu_typewriter: Typewriter::completed(MENU_CHAR_INTERVAL),
            result_typewriter: Typewriter::completed(typewriter::CHAR_INTERVAL),
            menu_icons: MenuIcons::new(&MENU_ICON_PATHS),
            background: BackgroundRenderer::new(),
            result_sprite: ResultSprite::new(),
        }
    }

    /// 表示中の画面のタイプライター表示を全文字表示済みにする。キー入力・クリックの
    /// 処理の最初に呼ぶだけで、入力自体は消費しない(その後の通常の操作もそのまま効く)
    fn skip_typewriter(&mut self) {
        match self.screen {
            Screen::Menu => self.menu_typewriter.skip(),
            Screen::Result(..) => self.result_typewriter.skip(),
            _ => {}
        }
    }

    /// メニューへ戻った直後に一度だけtrueを返す(呼ぶとフラグは消費されfalseに戻る)。
    /// main.rsが端末のスクロールバッファをクリアするタイミングの判定に使う
    pub fn take_pending_scrollback_clear(&mut self) -> bool {
        std::mem::take(&mut self.pending_scrollback_clear)
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.skip_typewriter();
        // [q]はメニューでは終了確認を開き、それ以外の画面ではメニューへ戻る。
        // カウントダウン中も特例として受け付ける(カウントダウンを中断する)
        if key.code == KeyCode::Char('q') {
            match self.screen {
                Screen::Menu => self.screen = Screen::ConfirmQuit,
                // 終了確認中の[q]は無視する(y/Enter・n/Escでのみ閉じる)
                Screen::ConfirmQuit => {}
                _ => self.quit_to_menu(),
            }
            return;
        }

        match &mut self.screen {
            Screen::Splash => {
                if matches!(key.code, KeyCode::Enter) {
                    self.leave_splash();
                }
            }
            Screen::BeigomaSplash => {
                if matches!(key.code, KeyCode::Enter) {
                    self.leave_beigoma_splash();
                }
            }
            Screen::BeigomaCharacterSplash => {
                if matches!(key.code, KeyCode::Enter) {
                    self.leave_beigoma_character_splash();
                }
            }
            Screen::Menu => self.handle_menu_key(key),
            Screen::SelectSong(selected) => {
                let selected = *selected;
                self.handle_song_key(key, selected);
            }
            Screen::SelectDifficulty(item, song) => {
                let (item, song) = (*item, *song);
                self.handle_difficulty_key(key, item, song);
            }
            // カウントダウン中はスキップ不可なので入力を無視する
            Screen::Countdown { .. } => {}
            Screen::Playing(game) => {
                game.handle_key(key);
                if game.is_finished() {
                    let result = game.result();
                    self.enter_result(result);
                }
            }
            Screen::Result(..) | Screen::History => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                    self.return_to_menu();
                }
            }
            Screen::Jukebox(_) => self.handle_jukebox_key(key),
            Screen::ConfirmQuit => self.handle_confirm_quit_key(key),
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        self.skip_typewriter();
        // 背景を敷く画面のクリック判定は、各画面のモジュールが描画(render)と同じく
        // 余白を除いた中央の範囲(screen_rect)を基準に行う
        let area = self.last_area;
        match &mut self.screen {
            Screen::Splash => {
                self.leave_splash();
            }
            Screen::BeigomaSplash => {
                self.leave_beigoma_splash();
            }
            Screen::BeigomaCharacterSplash => {
                self.leave_beigoma_character_splash();
            }
            Screen::Menu => self.handle_menu_mouse(mouse),
            Screen::SelectSong(_) => self.handle_song_mouse(mouse),
            Screen::SelectDifficulty(item, _) => {
                let item = *item;
                self.handle_difficulty_mouse(mouse, item);
            }
            Screen::Countdown { .. } => {}
            Screen::Playing(game) => {
                game.handle_mouse(mouse, area);
                if game.is_finished() {
                    let result = game.result();
                    self.enter_result(result);
                }
            }
            Screen::Result(..) | Screen::History => {
                self.return_to_menu();
            }
            // 終了確認中はクリックで背後のメニュー項目を選ばないよう無視する
            Screen::Jukebox(_) | Screen::ConfirmQuit => {}
        }
    }

    pub fn update(&mut self, dt: Duration) {
        match &mut self.screen {
            Screen::Playing(game) => {
                game.update(dt);
                // 時間経過だけで終了する場合がある(DDRの曲が最後まで流れ切った時等)ので、
                // キー/クリック入力を待たずここでも終了を確認する
                if game.is_finished() {
                    let result = game.result();
                    self.enter_result(result);
                }
            }
            Screen::Countdown { .. } => self.tick_countdown(dt),
            Screen::Menu => self.menu_typewriter.tick(dt),
            Screen::Result(..) => {
                self.result_typewriter.tick(dt);
                self.result_sprite.tick(dt);
            }
            Screen::BeigomaSplash => self.beigoma_splash_renderer.tick(dt),
            _ => {}
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.last_area = area;
        let current_bgm = self.current_bgm.clone();
        // TTR(曲選択)・プレイ中以外の画面は、共通の背景を画面全体に敷き、
        // その上の四辺に余白を残した中央の範囲(screen)に各画面を描く
        let background = &mut self.background;
        match &mut self.screen {
            Screen::Splash => {
                let screen = render_background(frame, background, area);
                splash_screens::render_still(frame, screen, &mut self.splash_renderer);
            }
            Screen::BeigomaSplash => {
                let screen = render_background(frame, background, area);
                splash_screens::render_beigoma(frame, screen, &mut self.beigoma_splash_renderer);
            }
            Screen::BeigomaCharacterSplash => {
                let screen = render_background(frame, background, area);
                splash_screens::render_still(
                    frame,
                    screen,
                    &mut self.beigoma_character_splash_renderer,
                );
            }
            Screen::Menu => {
                let screen = render_background(frame, background, area);
                menu::render(
                    frame,
                    screen,
                    &mut self.menu_state,
                    &mut self.menu_typewriter,
                    &mut self.menu_icons,
                );
            }
            Screen::SelectSong(selected) => {
                select_song::render(frame, area, *selected, &mut self.ttr_splash_renderer)
            }
            Screen::SelectDifficulty(item, song) => {
                let screen = render_background(frame, background, area);
                select_difficulty::render(frame, screen, *item, *song)
            }
            Screen::Countdown { state, .. } => {
                let screen = render_background(frame, background, area);
                crate::ui::countdown::render(frame, screen, state)
            }
            Screen::Playing(game) => game.render(frame, area),
            Screen::Result(result, save_error) => {
                let screen = render_background(frame, background, area);
                result::render(
                    frame,
                    screen,
                    result,
                    save_error.as_deref(),
                    &mut self.result_typewriter,
                    &mut self.result_sprite,
                )
            }
            Screen::History => {
                let screen = render_background(frame, background, area);
                history::render(frame, screen)
            }
            Screen::Jukebox(state) => {
                let screen = render_background(frame, background, area);
                jukebox::render(frame, screen, state, current_bgm.as_deref())
            }
            Screen::ConfirmQuit => {
                let screen = render_background(frame, background, area);
                menu::render(
                    frame,
                    screen,
                    &mut self.menu_state,
                    &mut self.menu_typewriter,
                    &mut self.menu_icons,
                );
                confirm_quit::render(frame, screen);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::typewriter::CHAR_INTERVAL;

    use super::test_support::*;
    use super::*;

    // --- [q]キーの終了フロー(メニュー以外→メニューへ戻る / メニュー→終了確認) ---

    fn is_menu_bgm(name: Option<&str>) -> bool {
        name.is_some_and(|n| {
            audio::bgm_tracks_in(BgmCategory::Menu)
                .iter()
                .any(|t| t == n)
        })
    }

    #[test]
    fn q_on_every_non_menu_screen_returns_to_menu_without_quitting() {
        for (name, mut app) in apps_on_every_non_menu_screen() {
            press(&mut app, KeyCode::Char('q'));
            assert!(matches!(app.screen, Screen::Menu), "{name}: メニューへ戻る");
            assert!(!app.should_quit(), "{name}: 即終了しない");
        }
    }

    #[test]
    fn q_on_every_non_menu_screen_switches_to_menu_bgm() {
        for (name, mut app) in apps_on_every_non_menu_screen() {
            press(&mut app, KeyCode::Char('q'));
            assert!(
                is_menu_bgm(app.current_bgm.as_deref()),
                "{name}: メニュー用BGMになる(実際: {:?})",
                app.current_bgm
            );
        }
    }

    #[test]
    fn q_while_playing_switches_bgm_from_playing_to_menu() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        press(&mut app, KeyCode::Char('1'));
        finish_countdown(&mut app);
        assert!(
            !is_menu_bgm(app.current_bgm.as_deref()),
            "プレイ中はプレイ用BGM"
        );
        press(&mut app, KeyCode::Char('q'));
        assert!(is_menu_bgm(app.current_bgm.as_deref()));
    }

    #[test]
    fn q_on_splash_keeps_the_menu_bgm_already_playing() {
        // タイトル画面では既にメニュー用BGMが流れているので、曲を途中で差し替えない
        let mut app = App::new();
        let before = app.current_bgm.clone();
        assert!(is_menu_bgm(before.as_deref()));
        press(&mut app, KeyCode::Char('q'));
        assert_eq!(app.current_bgm, before);
    }

    #[test]
    fn q_on_jukebox_after_stopping_bgm_starts_menu_bgm() {
        let mut app = App::new();
        app.screen = Screen::Jukebox(app.jukebox_list_state());
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.current_bgm, None);
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.screen, Screen::Menu));
        assert!(is_menu_bgm(app.current_bgm.as_deref()));
    }

    // --- タイプライター表示(リザルト・メニュー) ---

    #[test]
    fn typewriters_advance_only_on_their_own_screen() {
        let mut app = app_showing_result();
        app.screen = Screen::History;
        app.update(CHAR_INTERVAL * 5);
        assert_eq!(
            app.result_typewriter.visible_chars(),
            0,
            "リザルト以外では進まない"
        );

        let mut app = app_entering_menu();
        app.screen = Screen::History;
        app.update(MENU_CHAR_INTERVAL * 5);
        assert_eq!(
            app.menu_typewriter.visible_chars(),
            0,
            "メニュー以外では進まない"
        );

        // 終了確認ダイアログの背後のメニューも、ダイアログ中は進めない
        let mut app = app_entering_menu();
        app.screen = Screen::ConfirmQuit;
        app.update(LONG_ENOUGH);
        assert_eq!(app.menu_typewriter.visible_chars(), 0);
    }
}
