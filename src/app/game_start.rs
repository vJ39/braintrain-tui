//! メニュー項目からゲームが始まるまでの経路(カウントダウン経由 / 直接開始 / スプラッシュ経由 /
//! 曲選択経由)と、ゲーム開始前のカウントダウン画面(Screen::Countdown)

use std::time::Duration;

use crate::audio::{self, BgmCategory, SeKind};
use crate::game::beigoma::BeigomaGame;
use crate::game::count_mania::CountManiaGame;
use crate::game::quick_draw::QuickDrawGame;
use crate::game::rhythm::RhythmGame;
use crate::game::Difficulty;
use crate::ui::countdown::CountdownState;

use super::menu_items::{
    new_game, BEIGOMA_ITEM_INDEX, COLOR_STACK_ITEM_INDEX, COUNT_MANIA_ITEM_INDEX,
    MEMORY_ITEM_INDEX, QUICK_DRAW_ITEM_INDEX, REACTION_ITEM_INDEX, RHYTHM_ITEM_INDEX,
};
use super::{App, Screen};

impl App {
    /// メニューでゲームの項目を決定した時に、そのゲームの開始経路へ振り分ける
    /// (難易度選択を挟むか・カウントダウンを挟むか・スプラッシュや曲選択を挟むか)
    pub(super) fn start_game_item(&mut self, item: usize) {
        match item {
            // シタケシは難易度選択を挟まず、すぐにカウントダウンへ進む
            // (カウントダウン最初の「3」の音が画面遷移の音を兼ねる)
            COLOR_STACK_ITEM_INDEX => {
                self.start_playing(item, crate::game::color_stack::SESSION_DIFFICULTY)
            }
            // カウントメニアもROUND1〜5で難易度・動きが自動で変わるため、難易度選択を挟まない。
            // ラウンドごとに自前の「3.2.1.GO!!」を持つため、画面遷移側のカウントダウンも挟まない
            // (挟むと演出が2回連続してしまう)
            COUNT_MANIA_ITEM_INDEX => self.start_count_mania(),
            // ハヤウチは難易度選択に加え、ラウンドごとに自前の「3.2.1.GO!!」を持つため、
            // 画面遷移側のカウントダウンも挟まない(挟むと演出が2回連続してしまう)
            QUICK_DRAW_ITEM_INDEX => self.start_quick_draw(),
            // べーは難易度選択の代わりに専用スプラッシュ画面を挟む(TTRと同じ仕組み)。
            // Enter/クリックでstart_beigoma()が呼ばれ、ROUND1のカウントダウンから始まる
            BEIGOMA_ITEM_INDEX => self.enter_beigoma_splash(),
            // 記憶は3問→4問→3問で手数が自動で増えるため、難易度選択を挟まない
            MEMORY_ITEM_INDEX => self.start_playing(item, crate::game::memory::SESSION_DIFFICULTY),
            // イロピッタンも3問→4問→3問で難易度が自動で上がるため、難易度選択を挟まない
            REACTION_ITEM_INDEX => {
                self.start_playing(item, crate::game::reaction::SESSION_DIFFICULTY)
            }
            // TTRは難易度選択の代わりに、TTRスプラッシュ画像を背景にした曲選択を挟む
            RHYTHM_ITEM_INDEX => self.enter_song_select(),
            _ => {
                audio::play_se(SeKind::Transition);
                self.screen = Screen::SelectDifficulty(item, None);
            }
        }
    }

    /// 難易度決定後、指定ゲームを開始する(キー/クリック共通)。
    /// Playing用BGMに切り替えてカウントダウンを挟む(TTRはこの経路を通らない)
    pub(super) fn start_playing(&mut self, item: usize, difficulty: Difficulty) {
        if let Some(name) = audio::random_bgm_track(BgmCategory::Playing) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        let state = CountdownState::new();
        // 最初のフェーズ「3」の音。画面遷移の音(Transition)も兼ねる
        if let Some(phase) = state.phase() {
            audio::play_se(phase.se());
        }
        self.screen = Screen::Countdown {
            item,
            difficulty,
            state,
        };
    }

    /// カウントメニアを開始する。ラウンドごとの「3.2.1.GO!!」を自前で持つため、
    /// 画面遷移側のカウントダウン(start_playing)は経由しない
    fn start_count_mania(&mut self) {
        if let Some(name) = audio::random_bgm_track(BgmCategory::Playing) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        self.screen = Screen::Playing(Box::new(CountManiaGame::new()));
    }

    /// ハヤウチを開始する。ラウンドごとの「3.2.1.GO!!」を自前で持つため、
    /// 画面遷移側のカウントダウン(start_playing)は経由しない
    fn start_quick_draw(&mut self) {
        if let Some(name) = audio::random_bgm_track(BgmCategory::Playing) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        self.screen = Screen::Playing(Box::new(QuickDrawGame::new()));
    }

    /// べーを開始する。ROUNDごとの「3.2.1.GO!!」を自前で持つため、
    /// 画面遷移側のカウントダウン(start_playing)は経由しない
    pub(super) fn start_beigoma(&mut self) {
        if let Some(name) = audio::random_bgm_track(BgmCategory::Playing) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        self.screen = Screen::Playing(Box::new(BeigomaGame::new()));
    }

    /// リズムゲームを開始する(常に上級の譜面・判定)。譜面生成を先に済ませ、選んだ曲のBGM再生を
    /// 始めた直後にゲーム内時計を合わせることで、曲と譜面(実測ビート時刻)の時間基準を揃える
    pub(super) fn start_rhythm(&mut self, song: usize) {
        let mut game = RhythmGame::new(song);
        let track_name = game.song().track_name;
        audio::play_bgm_track(track_name);
        game.restart_clock();
        self.current_bgm = Some(track_name.to_string());
        self.screen = Screen::Playing(Box::new(game));
    }

    /// カウントダウン画面の時間経過。フェーズが変わるたびに音を鳴らし、終わったらitemのゲームを
    /// difficultyで作ってPlayingへ進む
    pub(super) fn tick_countdown(&mut self, dt: Duration) {
        let Screen::Countdown {
            item,
            difficulty,
            state,
        } = &mut self.screen
        else {
            return;
        };
        if let Some(phase) = state.tick(dt) {
            audio::play_se(phase.se());
        }
        if state.is_finished() {
            let (item, difficulty) = (*item, *difficulty);
            self.screen = Screen::Playing(new_game(item, difficulty));
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

    use crate::ui::countdown::{Phase, PHASE_DURATION};

    use super::super::menu_items::{JUKEBOX_ITEM_INDEX, MENU_ITEMS};
    use super::super::test_support::*;
    use super::*;

    // --- カウントメニア ---

    /// カウントメニアが始まり、ゲーム内のROUND1(初級)の「3.2.1.GO!!」から始まっていることを確かめる。
    /// カウントメニアはラウンドごとに自前のカウントダウンを持つため、
    /// 画面遷移側のカウントダウン(Screen::Countdown)は経由しない
    fn assert_count_mania_round1_countdown_in_game(app: &mut App) {
        let Screen::Playing(game) = &app.screen else {
            panic!("外側のカウントダウンを挟まず直接Playing画面になるはず");
        };
        assert_eq!(game.result().game_id, crate::game::count_mania::GAME_ID);
        assert!(!game.is_finished());
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("マウス専用"));
        assert!(text.contains("ROUND1/5"), "ROUND1から始まる");
        assert!(text.contains("初級"), "ROUND1は初級");
        assert!(
            text.contains('█'),
            "ゲーム内のカウントダウンを大きな文字で出す"
        );
        assert!(
            !text.contains("つぎ"),
            "カウントダウン中は次の数字の案内を出さない"
        );
        // ゲーム内のカウントダウン(GO!!まで)が終わるとROUND1のプレイに入る
        app.update(COUNTDOWN_TOTAL);
        assert!(
            matches!(app.screen, Screen::Playing(_)),
            "Playing画面のまま"
        );
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("つぎ"), "GO!!の後はプレイ中: {text}");
    }

    #[test]
    fn selecting_count_mania_skips_difficulty_and_the_outer_countdown() {
        let mut app = App::new();
        app.select_menu_item(COUNT_MANIA_ITEM_INDEX);
        assert_count_mania_round1_countdown_in_game(&mut app);
    }

    #[test]
    fn enter_on_count_mania_in_menu_goes_straight_to_playing() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app.menu_state.select(COUNT_MANIA_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_count_mania_round1_countdown_in_game(&mut app);
    }

    // --- シタケシ ---

    /// シタケシが始まり、ROUND1(4列)が表示されていることを確かめる
    fn assert_color_stack_round1_is_playing(app: &mut App) {
        finish_countdown(app);
        let Screen::Playing(game) = &app.screen else {
            panic!("Playing画面のはず");
        };
        assert_eq!(game.result().game_id, crate::game::color_stack::GAME_ID);
        assert!(!game.is_finished());
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("シタケシ"));
        assert!(text.contains("ROUND1/3"), "ROUND1から始まる");
    }

    #[test]
    fn selecting_color_stack_skips_difficulty_and_starts_round1_after_countdown() {
        let mut app = App::new();
        app.select_menu_item(COLOR_STACK_ITEM_INDEX);
        let Screen::Countdown {
            item,
            difficulty,
            state,
        } = &app.screen
        else {
            panic!("難易度選択を挟まずカウントダウンになるはず");
        };
        assert_eq!(*item, COLOR_STACK_ITEM_INDEX);
        assert_eq!(*difficulty, crate::game::color_stack::SESSION_DIFFICULTY);
        assert_eq!(state.phase(), Some(Phase::Three), "3から始まる");
        assert_color_stack_round1_is_playing(&mut app);
    }

    #[test]
    fn enter_on_color_stack_in_menu_goes_straight_to_countdown() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app.menu_state.select(COLOR_STACK_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(
            app.screen,
            Screen::Countdown {
                item: COLOR_STACK_ITEM_INDEX,
                ..
            }
        ));
        assert_color_stack_round1_is_playing(&mut app);
    }

    #[test]
    fn clicking_color_stack_in_menu_goes_straight_to_countdown() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        let (x, y) = drawn_position_of(&mut app, MENU_ITEMS[COLOR_STACK_ITEM_INDEX], 200, 60)
            .expect("シタケシが描かれていること");
        app.handle_mouse(left_click_at(x, y));
        assert!(matches!(
            app.screen,
            Screen::Countdown {
                item: COLOR_STACK_ITEM_INDEX,
                ..
            }
        ));
        assert_color_stack_round1_is_playing(&mut app);
    }

    // --- 記憶・イロピッタン(3問→4問→3問の固定10問構成) ---

    #[test]
    fn selecting_memory_or_reaction_skips_difficulty_and_starts_after_countdown() {
        for (item, game_id, session_difficulty) in fixed_progression_games() {
            let mut app = App::new();
            app.select_menu_item(item);
            let Screen::Countdown {
                item: counting_item,
                difficulty,
                state,
            } = &app.screen
            else {
                panic!("item={item}: 難易度選択を挟まずカウントダウンになるはず");
            };
            assert_eq!(*counting_item, item);
            assert_eq!(*difficulty, session_difficulty);
            assert_eq!(state.phase(), Some(Phase::Three), "3から始まる");
            finish_countdown(&mut app);
            let Screen::Playing(game) = &app.screen else {
                panic!("Playing画面のはず");
            };
            assert_eq!(game.result().game_id, game_id);
            assert!(!game.is_finished());
        }
    }

    #[test]
    fn enter_on_memory_or_reaction_in_menu_goes_straight_to_countdown() {
        for (item, _, _) in fixed_progression_games() {
            let mut app = App::new();
            app.screen = Screen::Menu;
            app.menu_state.select(item);
            app.handle_key(KeyEvent::from(KeyCode::Enter));
            assert!(
                matches!(app.screen, Screen::Countdown { item: i, .. } if i == item),
                "item={item}"
            );
        }
    }

    #[test]
    fn other_games_still_go_to_difficulty_select() {
        for item in difficulty_select_game_items() {
            let mut app = App::new();
            app.select_menu_item(item);
            assert!(
                matches!(app.screen, Screen::SelectDifficulty(i, None) if i == item),
                "item={item}: 難易度選択へ進む"
            );
        }
    }

    #[test]
    fn selecting_game_menu_item_enters_difficulty_screen() {
        let mut app = App::new();
        app.select_menu_item(0);
        assert!(matches!(app.screen, Screen::SelectDifficulty(0, None)));
    }

    // --- ハヤウチ ---

    /// ハヤウチが始まり、1問目(10問中)が表示されていることを確かめる。
    /// ハヤウチはラウンドごとに自前のカウントダウンを持つため、
    /// 画面遷移側のカウントダウン(Screen::Countdown)は経由しない
    fn assert_quick_draw_round1_is_playing(app: &mut App) {
        let Screen::Playing(game) = &app.screen else {
            panic!("外側のカウントダウンを挟まず直接Playing画面になるはず");
        };
        assert_eq!(game.result().game_id, crate::game::quick_draw::GAME_ID);
        assert!(!game.is_finished());
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("ハヤウチ"));
        assert!(text.contains("Q1/10"), "10問制の1問目から始まる");
    }

    #[test]
    fn selecting_quick_draw_skips_difficulty_and_the_outer_countdown() {
        let mut app = App::new();
        app.select_menu_item(QUICK_DRAW_ITEM_INDEX);
        assert_quick_draw_round1_is_playing(&mut app);
    }

    #[test]
    fn enter_on_quick_draw_in_menu_goes_straight_to_playing() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app.menu_state.select(QUICK_DRAW_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_quick_draw_round1_is_playing(&mut app);
    }

    // --- べー ---

    /// べーが始まり、ゲーム内のROUND1の「3.2.1.GO!!」から始まっていることを確かめる。
    /// べーはROUNDごとに自前のカウントダウンを持つため、
    /// 画面遷移側のカウントダウン(Screen::Countdown)は経由しない
    fn assert_beigoma_round1_countdown_in_game(app: &mut App) {
        let Screen::Playing(game) = &app.screen else {
            panic!("外側のカウントダウンを挟まず直接Playing画面になるはず");
        };
        assert_eq!(game.result().game_id, crate::game::beigoma::GAME_ID);
        assert!(!game.is_finished());
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("べー"), "{text}");
        assert!(text.contains("ROUND1やさしい"), "ROUND1から始まる: {text}");
        assert!(
            text.contains("残り60.0秒"),
            "カウントダウン中はまだ数えない: {text}"
        );
        assert!(
            text.contains("盤面") && text.contains("軽トラ視点"),
            "2視点を出す"
        );
        assert!(
            text.contains("█"),
            "ゲーム内のカウントダウンを大きな文字で出す: {text}"
        );
        // ゲーム内のカウントダウン(GO!!まで)が終わってから制限時間を数え始める
        app.update(COUNTDOWN_TOTAL);
        app.update(Duration::from_secs(1));
        assert!(
            matches!(app.screen, Screen::Playing(_)),
            "Playing画面のまま"
        );
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("残り59.0秒"), "GO!!の後から数える: {text}");
    }

    #[test]
    fn selecting_beigoma_shows_the_splash_screen_first() {
        let mut app = App::new();
        app.select_menu_item(BEIGOMA_ITEM_INDEX);
        assert!(
            matches!(app.screen, Screen::BeigomaSplash),
            "難易度選択の代わりに専用スプラッシュ画面を挟む"
        );
    }

    #[test]
    fn enter_on_beigoma_splash_skips_difficulty_and_the_outer_countdown() {
        let mut app = App::new();
        app.select_menu_item(BEIGOMA_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_beigoma_round1_countdown_in_game(&mut app);
    }

    #[test]
    fn clicking_beigoma_splash_also_starts_the_game() {
        let mut app = App::new();
        app.select_menu_item(BEIGOMA_ITEM_INDEX);
        app.last_area = rect(0, 0, 40, 12);
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert_beigoma_round1_countdown_in_game(&mut app);
    }

    #[test]
    fn enter_on_beigoma_in_menu_goes_straight_to_playing() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app.menu_state.select(BEIGOMA_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(
            matches!(app.screen, Screen::BeigomaSplash),
            "メニューからのEnterはまずスプラッシュ画面を挟む"
        );
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_beigoma_round1_countdown_in_game(&mut app);
    }

    // --- ゲーム開始前のカウントダウン ---

    /// DDR以外のゲームのメニュー項目一覧
    fn non_rhythm_game_items() -> impl Iterator<Item = usize> {
        (0..JUKEBOX_ITEM_INDEX).filter(|&item| item != RHYTHM_ITEM_INDEX)
    }

    /// 難易度選択画面を経由するゲームのメニュー項目一覧
    /// (DDR・シタケシ・カウントメニア・ハヤウチ・べー・記憶・イロピッタン以外)
    fn difficulty_select_game_items() -> impl Iterator<Item = usize> {
        non_rhythm_game_items().filter(|&item| {
            item != COLOR_STACK_ITEM_INDEX
                && item != COUNT_MANIA_ITEM_INDEX
                && item != QUICK_DRAW_ITEM_INDEX
                && item != BEIGOMA_ITEM_INDEX
                && item != MEMORY_ITEM_INDEX
                && item != REACTION_ITEM_INDEX
        })
    }

    #[test]
    fn countdown_total_matches_four_phases() {
        assert_eq!(PHASE_DURATION * 4, COUNTDOWN_TOTAL);
    }

    #[test]
    fn starting_any_non_rhythm_game_goes_through_countdown() {
        // シタケシ・カウントメニア・ハヤウチは難易度選択を経由しないので別のテストで確認する
        for item in difficulty_select_game_items() {
            let mut app = App::new();
            app.screen = Screen::SelectDifficulty(item, None);
            app.handle_key(KeyEvent::from(KeyCode::Char('2')));
            let Screen::Countdown {
                item: counting_item,
                difficulty,
                state,
            } = &app.screen
            else {
                panic!("item={item}: カウントダウン画面のはず");
            };
            assert_eq!(*counting_item, item);
            assert_eq!(*difficulty, Difficulty::Intermediate);
            assert_eq!(
                state.phase(),
                Some(Phase::Three),
                "item={item}: 3から始まる"
            );
        }
    }

    #[test]
    fn countdown_advances_phases_with_update_and_stays_until_total() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
        let phase_of = |app: &App| match &app.screen {
            Screen::Countdown { state, .. } => state.phase(),
            _ => None,
        };
        assert_eq!(phase_of(&app), Some(Phase::Three));
        app.update(PHASE_DURATION);
        assert_eq!(phase_of(&app), Some(Phase::Two));
        app.update(PHASE_DURATION);
        assert_eq!(phase_of(&app), Some(Phase::One));
        app.update(PHASE_DURATION);
        assert_eq!(phase_of(&app), Some(Phase::Go));
        app.update(PHASE_DURATION - Duration::from_millis(1));
        assert!(
            matches!(app.screen, Screen::Countdown { .. }),
            "2.4秒経つまではカウントダウンのまま"
        );
        app.update(Duration::from_millis(1));
        assert!(matches!(app.screen, Screen::Playing(_)));
    }

    #[test]
    fn countdown_finishes_into_the_selected_game_for_every_item() {
        // シタケシ・カウントメニア・ハヤウチは難易度選択を経由しないので別のテストで確認する
        for item in difficulty_select_game_items() {
            let mut app = App::new();
            app.screen = Screen::SelectDifficulty(item, None);
            app.handle_key(KeyEvent::from(KeyCode::Char('3')));
            finish_countdown(&mut app);
            let Screen::Playing(game) = &app.screen else {
                unreachable!();
            };
            let expected = new_game(item, Difficulty::Advanced).result();
            let result = game.result();
            assert_eq!(result.game_id, expected.game_id, "item={item}");
            assert_eq!(result.difficulty, Difficulty::Advanced, "item={item}");
            assert!(!game.is_finished(), "item={item}: 始まったばかり");
        }
    }

    #[test]
    fn keys_during_countdown_are_ignored() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None); // shape_rotate
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
        // ゲームを最後まで回答し切れる回数ぶん押しても、カウントダウン中は何も起きない
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            app.handle_key(KeyEvent::from(KeyCode::Left));
            app.handle_key(KeyEvent::from(KeyCode::Enter));
            app.handle_key(KeyEvent::from(KeyCode::Esc));
        }
        let Screen::Countdown { state, .. } = &app.screen else {
            panic!("キー入力でカウントダウンが飛ばされないこと");
        };
        assert_eq!(
            state.phase(),
            Some(Phase::Three),
            "キー入力でフェーズが進まないこと"
        );

        finish_countdown(&mut app);
        let Screen::Playing(game) = &app.screen else {
            unreachable!();
        };
        assert!(
            !game.is_finished(),
            "カウントダウン中のキーがゲームに届いていないこと"
        );
        assert_eq!(game.result().total, 0, "回答数0のまま");
    }

    #[test]
    fn mouse_clicks_during_countdown_are_ignored() {
        let mut app = App::new();
        // 難易度選択を挟まず画面遷移側のカウントダウンに入るゲーム
        app.select_menu_item(COLOR_STACK_ITEM_INDEX);
        app.last_area = rect(0, 0, 80, 24);
        for row in 0..24 {
            app.handle_mouse(left_click(row));
        }
        let Screen::Countdown { state, .. } = &app.screen else {
            panic!("クリックでカウントダウンが飛ばされないこと");
        };
        assert_eq!(state.phase(), Some(Phase::Three));

        finish_countdown(&mut app);
        let Screen::Playing(game) = &app.screen else {
            unreachable!();
        };
        assert_eq!(game.result().game_id, crate::game::color_stack::GAME_ID);
        assert!(!game.is_finished());
    }

    #[test]
    fn game_is_playable_after_countdown() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None); // shape_rotate
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
        finish_countdown(&mut app);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            app.handle_key(KeyEvent::from(KeyCode::Left));
        }
        let Screen::Result(result, _) = &app.screen else {
            panic!("全問回答したらリザルト画面になるはず");
        };
        assert_eq!(result.total, crate::game::QUESTIONS_PER_SESSION);
    }

    #[test]
    fn countdown_screen_renders_big_text() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
        assert!(rendered_text(&mut app).contains('█'));
    }

    #[test]
    fn q_during_countdown_aborts_it_and_no_game_starts() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        press(&mut app, KeyCode::Char('1'));
        app.update(PHASE_DURATION);
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.screen, Screen::Menu));
        // 残りのカウントダウン時間が経ってもゲームは始まらない
        app.update(COUNTDOWN_TOTAL);
        assert!(
            matches!(app.screen, Screen::Menu),
            "中断後にゲームが始まらないこと"
        );
        assert!(!app.should_quit());
    }
}
