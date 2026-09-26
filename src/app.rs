use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::audio::{self, BgmCategory, SeKind};
use crate::game::beigoma::BeigomaGame;
use crate::game::color_stack::ColorStackGame;
use crate::game::count_mania::CountManiaGame;
use crate::game::memory::MemoryGame;
use crate::game::mental_calc::MentalCalcGame;
use crate::game::mirror_match::MirrorMatchGame;
use crate::game::pattern_fill::PatternFillGame;
use crate::game::puzzle_connect::PuzzleConnectGame;
use crate::game::quick_draw::QuickDrawGame;
use crate::game::reaction::ReactionGame;
use crate::game::rhythm::{RhythmGame, SONGS};
use crate::game::sequence::SequenceGame;
use crate::game::shape_rotate::ShapeRotateGame;
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult};
use crate::stats::store;
use crate::ui::background::BackgroundRenderer;
use crate::ui::countdown::{self, CountdownState};
use crate::ui::menu_icons::MenuIcons;
use crate::ui::result_sprite::ResultSprite;
use crate::ui::splash::{self, SplashRenderer};
use crate::ui::typewriter::{self, Typewriter};

const MENU_ITEMS: [&str; 15] = [
    "図形回転判定",
    "鏡像判定",
    "イロピッタン",
    "暗算スピード",
    "パターン補完",
    "オイカケ",
    "数列予測",
    "組み合わせパズル",
    "カウントマニア",
    "ソコヌキ",
    "TTR",
    "ハヤウチ",
    "べー",
    "ジュークボックス",
    "履歴",
];

/// イロピッタン(3問→4問→3問で難易度が上がる固定10問)
const REACTION_ITEM_INDEX: usize = 2;
/// オイカケ(3問→4問→3問で手数が増える固定10問)
const MEMORY_ITEM_INDEX: usize = 5;
/// カウントマニア(マウス専用)
const COUNT_MANIA_ITEM_INDEX: usize = 8;
/// ソコヌキ
const COLOR_STACK_ITEM_INDEX: usize = 9;
/// リズムゲーム(TTR)は難易度選択の代わりに曲選択を挟む(難易度は常に上級)
const RHYTHM_ITEM_INDEX: usize = 10;
/// ハヤウチ
const QUICK_DRAW_ITEM_INDEX: usize = 11;
/// べー
const BEIGOMA_ITEM_INDEX: usize = 12;
const JUKEBOX_ITEM_INDEX: usize = MENU_ITEMS.len() - 2;
const HISTORY_ITEM_INDEX: usize = MENU_ITEMS.len() - 1;

/// メニュー各項目の一言説明。配列長をMENU_ITEMS.len()にして、項目の追加漏れをコンパイル時に検出する
const MENU_DESCRIPTIONS: [&str; MENU_ITEMS.len()] = [
    "回転させた図形が元と同じかを見分ける",
    "鏡に映した図形かどうかを見分ける",
    "文字の色と意味が一致するかを即答する",
    "4択から計算の答えを素早く選ぶ",
    "3x3の規則から空欄に入る図形を選ぶ",
    "光ったパネルの順番を覚えて再現する",
    "数列の法則を見抜いて次の数を当てる",
    "完成形から2つ目のピースを当てる",
    "数字の円を1から順にクリックする(マウス専用)",
    "色ボタンで各列の一番下のブロックを消して盤面を空にする",
    "矢印キーで曲に合わせてステップする",
    "合図が出たら即座に反応する",
    "軽トラの揺れに耐えてベーゴマをゴールへ運ぶ",
    "BGMを選んで聴く",
    "ゲームごとの反応時間の推移を見る",
];

/// メニュー各項目のカードに描くアイコン画像(assets/image/からの相対パス)。並び順はMENU_ITEMSと同じ
const MENU_ICON_PATHS: [&str; MENU_ITEMS.len()] = [
    "menu_icons/shape_rotate.png",
    "menu_icons/mirror_match.png",
    "menu_icons/reaction.png",
    "menu_icons/mental_calc.png",
    "menu_icons/pattern_fill.png",
    "menu_icons/memory.png",
    "menu_icons/sequence.png",
    "menu_icons/puzzle_connect.png",
    "menu_icons/count_mania.png",
    "menu_icons/color_stack.png",
    "menu_icons/rhythm.png",
    "menu_icons/quick_draw.png",
    "menu_icons/beigoma.png",
    "menu_icons/jukebox.png",
    "menu_icons/history.png",
];

/// メニュー画面のタイプライター表示の1文字あたりの間隔。メニューは全項目で400文字以上あり、
/// 標準の間隔(typewriter::CHAR_INTERVAL)では流し切るのに10秒以上かかるため短くする
const MENU_CHAR_INTERVAL: Duration = Duration::from_millis(10);

pub enum Screen {
    /// 起動直後のタイトル画面。Enterを押すとMenuへ進む
    Splash,
    Menu,
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
            splash_renderer: SplashRenderer::new(splash::TITLE_IMAGE_PATH, splash::TITLE_FALLBACK),
            ttr_splash_renderer: SplashRenderer::new(
                splash::TTR_SPLASH_IMAGE_PATH,
                splash::TTR_FALLBACK,
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

    /// メニュー画面へ遷移する。既存の`self.screen = Screen::Menu`は全てこれに統一し、
    /// メニューに戻るたびに端末側でのスクロールバッファのクリアを要求する。
    /// 項目の名前・説明文はここから改めてタイプライターで流す
    fn enter_menu(&mut self) {
        self.screen = Screen::Menu;
        self.pending_scrollback_clear = true;
        let total = typewriter::char_count(&menu_item_lines(screen_rect(self.last_area)));
        self.menu_typewriter = Typewriter::with_interval(total, MENU_CHAR_INTERVAL);
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

    /// 終了確認ダイアログ: y/Enterで終了、n/Escでメニューに戻る。それ以外は無視する
    fn handle_confirm_quit_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => self.should_quit = true,
            KeyCode::Char('n') | KeyCode::Esc => self.enter_menu(),
            _ => {}
        }
    }

    /// メニュー以外の画面で[q]を押した時にメニューへ戻る。既にメニュー用BGMが
    /// 流れていれば(タイトル画面など)曲を差し替えず、そうでなければ切り替える
    fn quit_to_menu(&mut self) {
        let menu_tracks = audio::bgm_tracks_in(BgmCategory::Menu);
        let playing_menu_bgm = self
            .current_bgm
            .as_ref()
            .is_some_and(|name| menu_tracks.contains(name));
        if playing_menu_bgm {
            audio::play_se(SeKind::Transition);
            self.enter_menu();
        } else {
            self.return_to_menu();
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        self.skip_typewriter();
        let area = self.last_area;
        // 背景を敷く画面は、描画(render)と同じく余白を除いた中央の範囲を基準に判定する
        let screen = screen_rect(area);
        match &mut self.screen {
            Screen::Splash => {
                self.leave_splash();
            }
            Screen::Menu => {
                let offset = self.menu_state.row_offset;
                if let Some(index) = menu_card_at(screen, offset, mouse.column, mouse.row) {
                    self.menu_state.select(index);
                    self.select_menu_item(index);
                }
            }
            Screen::SelectSong(_) => {
                if let Some(song) = song_at_row(area, mouse.row) {
                    self.select_song(song);
                }
            }
            Screen::SelectDifficulty(item, _) => {
                let item = *item;
                if let Some(difficulty) = difficulty_at_row(screen, mouse.row) {
                    self.start_playing(item, difficulty);
                }
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

    /// タイトル画面(Splash)からメニューへ進む(Enter/クリック共通)
    fn leave_splash(&mut self) {
        audio::play_se(SeKind::Confirm);
        self.enter_menu();
    }

    /// ゲーム終了後、リザルト画面へ進む(キー/クリック共通)。リザルト用BGMに切り替える。
    /// 履歴への保存はここで1回だけ行う(描画のたびに保存し直さない)
    fn enter_result(&mut self, result: GameResult) {
        if let Some(name) = audio::random_bgm_track(BgmCategory::Result) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        let save_error = store::append_result(&result).err().map(|e| e.to_string());
        self.show_result(result, save_error);
    }

    /// リザルト画面を表示し、結果の本文のタイプライターとキャラクターのアニメーションを
    /// 最初から始める(履歴への保存はしない)
    fn show_result(&mut self, result: GameResult, save_error: Option<String>) {
        self.result_typewriter = Typewriter::new(typewriter::char_count(&result_lines(&result)));
        self.result_sprite.reset();
        self.screen = Screen::Result(result, save_error);
    }

    /// Menu画面での項目決定(キー/クリック共通)。ゲーム/ジュークボックス/履歴へ振り分ける
    fn select_menu_item(&mut self, selected: usize) {
        if selected == COLOR_STACK_ITEM_INDEX {
            // ソコヌキは難易度選択を挟まず、すぐにカウントダウンへ進む
            // (カウントダウン最初の「3」の音が画面遷移の音を兼ねる)
            self.start_playing(
                COLOR_STACK_ITEM_INDEX,
                crate::game::color_stack::SESSION_DIFFICULTY,
            );
            return;
        }
        if selected == COUNT_MANIA_ITEM_INDEX {
            // カウントマニアもROUND1〜5で難易度・動きが自動で変わるため、難易度選択を挟まない
            self.start_playing(
                COUNT_MANIA_ITEM_INDEX,
                crate::game::count_mania::SESSION_DIFFICULTY,
            );
            return;
        }
        if selected == QUICK_DRAW_ITEM_INDEX {
            // ハヤウチは難易度選択に加え、ラウンドごとに自前の「3.2.1.GO!!」を持つため、
            // 画面遷移側のカウントダウンも挟まない(挟むと演出が2回連続してしまう)
            self.start_quick_draw();
            return;
        }
        if selected == BEIGOMA_ITEM_INDEX {
            // べーも難易度を持たず、決まったコース・制限時間60秒で進むため、難易度選択を挟まない
            self.start_playing(BEIGOMA_ITEM_INDEX, crate::game::beigoma::SESSION_DIFFICULTY);
            return;
        }
        if selected == MEMORY_ITEM_INDEX {
            // 記憶は3問→4問→3問で手数が自動で増えるため、難易度選択を挟まない
            self.start_playing(MEMORY_ITEM_INDEX, crate::game::memory::SESSION_DIFFICULTY);
            return;
        }
        if selected == REACTION_ITEM_INDEX {
            // イロピッタンも3問→4問→3問で難易度が自動で上がるため、難易度選択を挟まない
            self.start_playing(REACTION_ITEM_INDEX, crate::game::reaction::SESSION_DIFFICULTY);
            return;
        }
        audio::play_se(SeKind::Transition);
        if selected == HISTORY_ITEM_INDEX {
            self.screen = Screen::History;
        } else if selected == JUKEBOX_ITEM_INDEX {
            self.screen = Screen::Jukebox(self.jukebox_list_state());
        } else if selected == RHYTHM_ITEM_INDEX {
            // TTRスプラッシュ画像を背景にした曲選択画面へ進む。BGMもTTR専用のものに切り替え、
            // 実際に曲を選んでプレイが始まるまで(start_rhythmで曲のBGMに切り替わるまで)流し続ける
            if let Some(name) = audio::random_bgm_track(BgmCategory::RhythmSplash) {
                audio::play_bgm_track(&name);
                self.current_bgm = Some(name);
            }
            self.screen = Screen::SelectSong(0);
        } else {
            self.screen = Screen::SelectDifficulty(selected, None);
        }
    }

    /// 曲選択画面での曲決定(キー/クリック共通)。難易度選択を挟まず、その曲のプレイを始める
    fn select_song(&mut self, song: usize) {
        audio::play_se(SeKind::Transition);
        self.start_rhythm(song);
    }

    /// 難易度決定後、指定ゲームを開始する(キー/クリック共通)。
    /// Playing用BGMに切り替えてカウントダウンを挟む(TTRはこの経路を通らない)
    fn start_playing(&mut self, item: usize, difficulty: Difficulty) {
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

    /// ハヤウチを開始する。ラウンドごとの「3.2.1.GO!!」を自前で持つため、
    /// 画面遷移側のカウントダウン(start_playing)は経由しない
    fn start_quick_draw(&mut self) {
        if let Some(name) = audio::random_bgm_track(BgmCategory::Playing) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        self.screen = Screen::Playing(Box::new(QuickDrawGame::new()));
    }

    /// リズムゲームを開始する(常に上級の譜面・判定)。譜面生成を先に済ませ、選んだ曲のBGM再生を
    /// 始めた直後にゲーム内時計を合わせることで、曲と譜面(実測ビート時刻)の時間基準を揃える
    fn start_rhythm(&mut self, song: usize) {
        let mut game = RhythmGame::new(song);
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
        self.enter_menu();
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

    /// Menu画面のキー操作。上下左右はカードグリッド内の移動(列数は直近の描画サイズで決まる)
    fn handle_menu_key(&mut self, key: KeyEvent) {
        let selected = self.menu_state.selected();
        let direction = match key.code {
            KeyCode::Up => GridMove::Up,
            KeyCode::Down => GridMove::Down,
            KeyCode::Left => GridMove::Left,
            KeyCode::Right => GridMove::Right,
            KeyCode::Enter => return self.select_menu_item(selected),
            _ => return,
        };
        let columns = menu_grid(screen_rect(self.last_area)).columns;
        self.menu_state
            .select(grid_move(selected, direction, columns, MENU_ITEMS.len()));
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
                self.enter_menu();
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
            KeyCode::Esc => self.enter_menu(),
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
            self.start_playing(item, difficulty);
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
            Screen::Countdown {
                item,
                difficulty,
                state,
            } => {
                if let Some(phase) = state.tick(dt) {
                    audio::play_se(phase.se());
                }
                if state.is_finished() {
                    let (item, difficulty) = (*item, *difficulty);
                    self.screen = Screen::Playing(new_game(item, difficulty));
                }
            }
            Screen::Menu => self.menu_typewriter.tick(dt),
            Screen::Result(..) => {
                self.result_typewriter.tick(dt);
                self.result_sprite.tick(dt);
            }
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
                // メニューと同じ色・角丸の枠を画面いっぱいに描き、内側の中央にロゴを配置する
                let block = theme::panel("");
                let inner = block.inner(screen);
                frame.render_widget(block, screen);
                self.splash_renderer.render(frame, inner);
            }
            Screen::Menu => {
                let screen = render_background(frame, background, area);
                render_menu(
                    frame,
                    screen,
                    &mut self.menu_state,
                    &mut self.menu_typewriter,
                    &mut self.menu_icons,
                );
            }
            Screen::SelectSong(selected) => {
                // TTRスプラッシュ画像を全画面に描いてから、その上に曲リストのパネルを重ねる。
                // Fallback表示(画像プロトコル非対応)は文言が画面中央に来るとパネルの裏に
                // 完全に隠れてしまうので、パネルより上の領域だけに表示する
                let bg_area = if self.ttr_splash_renderer.is_fallback() {
                    let panel_top = song_panel_rect(area).y;
                    Rect::new(area.x, area.y, area.width, panel_top.saturating_sub(area.y))
                } else {
                    area
                };
                self.ttr_splash_renderer.render(frame, bg_area);
                render_song_select(frame, area, *selected)
            }
            Screen::SelectDifficulty(item, song) => {
                let title = match song.and_then(|s| SONGS.get(s)) {
                    Some(song) => format!("{} / {}", MENU_ITEMS[*item], song.display_name),
                    None => MENU_ITEMS[*item].to_string(),
                };
                let screen = render_background(frame, background, area);
                render_difficulty_select(frame, screen, &title)
            }
            Screen::Countdown { state, .. } => {
                let screen = render_background(frame, background, area);
                countdown::render(frame, screen, state)
            }
            Screen::Playing(game) => game.render(frame, area),
            Screen::Result(result, save_error) => {
                let screen = render_background(frame, background, area);
                render_result(
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
                render_history(frame, screen)
            }
            Screen::Jukebox(state) => {
                let screen = render_background(frame, background, area);
                render_jukebox(frame, screen, state, current_bgm.as_deref())
            }
            Screen::ConfirmQuit => {
                let screen = render_background(frame, background, area);
                render_menu(
                    frame,
                    screen,
                    &mut self.menu_state,
                    &mut self.menu_typewriter,
                    &mut self.menu_icons,
                );
                render_confirm_quit(frame, screen);
            }
        }
    }
}

/// 画面の四辺に残す余白(左右・上下のセル数)。この余白に背景画像が見える。
/// 端末の1セルは縦長(おおむね横:縦=1:2)なので、左右は上下の倍にして見た目の太さをそろえる
const SCREEN_MARGIN_X: u16 = 4;
const SCREEN_MARGIN_Y: u16 = 2;

/// 画面全体(area)から四辺に余白を持たせた中央のRect。この余白に背景画像を見せる。
/// 余白を引くと幅・高さが0以下になるほど小さい画面では、余白なし(area全体)にする
fn screen_rect(area: Rect) -> Rect {
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
fn render_background(frame: &mut Frame, background: &mut BackgroundRenderer, area: Rect) -> Rect {
    let screen = screen_rect(area);
    if screen != area {
        background.render(frame, area);
        frame.render_widget(ratatui::widgets::Clear, screen);
    }
    screen
}

/// 終了確認ダイアログ。背景のメニューが見えるよう、中央に小さなパネルを重ねて描く
fn render_confirm_quit(frame: &mut Frame, area: Rect) {
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

fn new_game(item: usize, difficulty: Difficulty) -> Box<dyn Game> {
    match item {
        0 => Box::new(ShapeRotateGame::new(difficulty)),
        1 => Box::new(MirrorMatchGame::new(difficulty)),
        // イロピッタン・記憶は難易度を選ばず、3問→4問→3問で初級→中級→上級相当と進む
        REACTION_ITEM_INDEX => Box::new(ReactionGame::new()),
        3 => Box::new(MentalCalcGame::new(difficulty)),
        4 => Box::new(PatternFillGame::new(difficulty)),
        MEMORY_ITEM_INDEX => Box::new(MemoryGame::new()),
        6 => Box::new(SequenceGame::new(difficulty)),
        7 => Box::new(PuzzleConnectGame::new(difficulty)),
        // カウントマニアは難易度を選ばず、ROUND1=初級・ROUND2=中級・ROUND3=上級と進む
        COUNT_MANIA_ITEM_INDEX => Box::new(CountManiaGame::new()),
        // ソコヌキは難易度を持たず、ROUND1〜3が固定の内容で進む
        COLOR_STACK_ITEM_INDEX => Box::new(ColorStackGame::new()),
        RHYTHM_ITEM_INDEX => unreachable!("rhythm is started via start_rhythm with a song"),
        // ハヤウチは難易度を持たず、10問固定で進む
        QUICK_DRAW_ITEM_INDEX => Box::new(QuickDrawGame::new()),
        // べーは難易度を持たず、決まったコース・制限時間60秒で進む
        BEIGOMA_ITEM_INDEX => Box::new(BeigomaGame::new()),
        _ => unreachable!("history is handled without creating a game"),
    }
}

/// SelectSong画面で、曲の行が内部エリアの何行目から始まるか。
/// render_song_selectとsong_at_rowで一致させること
const SONG_ROWS_OFFSET: u16 = 2;

/// 曲選択パネルの操作説明(枠の下辺に出す)
const SONG_SELECT_HINTS: [(&str, &str); 3] = [
    ("↑↓", "選択"),
    ("Enter / 数字", "決定"),
    ("Esc", "戻る"),
];

/// 曲選択パネルの外枠(見出し・操作説明つき)
fn song_panel_block() -> Block<'static> {
    theme::panel(" ◆ BRAIN TRAIN ◆ ")
        .title_bottom(theme::hints_line(&SONG_SELECT_HINTS).centered())
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
            Line::from(Span::styled(format!(" ▶ {label} "), theme::selected_style()))
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

fn render_song_select(frame: &mut Frame, area: Rect, selected: usize) {
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

/// メニュー画面の外枠(見出し・操作説明つき)。タイプライター表示の対象外で、最初から出る
fn menu_block() -> Block<'static> {
    theme::panel(Line::from(" ◆ BRAIN TRAIN ◆ ").centered())
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
                ("↑↓←→", "選択"),
                ("Enter / クリック", "決定"),
                ("q", "終了"),
            ])
            .centered(),
        )
}

/// メニューのカード1枚の最小幅(枠込み)。説明文が1行に全角12文字ほど入り、
/// アイコン(縦横比約557:397)がMENU_ICON_ROWSの高さで横に収まる幅。列数はこの幅で画面幅を割って決める
const MENU_CARD_MIN_WIDTH: u16 = 28;
/// カード1枚のアイコン部分の行数(画像を描けない時も同じ行数の空白にする)
const MENU_ICON_ROWS: u16 = 5;
/// カードの枠の内側の、テキストの左右の余白(各1セル)
const MENU_CARD_PADDING: u16 = 1;

/// メニュー画面のカードグリッドの状態(選択中のカード・スクロール位置)
#[derive(Debug, Default, Clone, Copy)]
struct MenuGridState {
    /// 選択中のカード(MENU_ITEMSのインデックス)
    selected: usize,
    /// 画面の一番上に出しているカードの行。render_menuで選択中の行が画面内に入るよう合わせる
    row_offset: usize,
}

impl MenuGridState {
    fn selected(&self) -> usize {
        self.selected
    }

    fn select(&mut self, index: usize) {
        self.selected = index.min(MENU_ITEMS.len() - 1);
    }
}

/// 画面の大きさから決まるカードグリッドの配置。描画(render_menu)・クリック判定(menu_card_at)・
/// タイプライターの文字列(menu_item_lines)・キー操作の列数は全てこれを使ってそろえる
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MenuGrid {
    /// メニューの外枠の内側
    inner: Rect,
    /// 割り切れずに余った幅を左右に分け、グリッドを中央に置くための左の余白
    x_offset: u16,
    columns: usize,
    rows: usize,
    card_width: u16,
    card_height: u16,
    /// 説明文の1行の最大表示幅(セル数)
    text_width: usize,
    /// 説明文の行数(全カード共通。一番長い説明文に合わせる)
    desc_lines: usize,
    /// 画面に収まるカードの行数(最低1行)
    visible_rows: usize,
}

impl MenuGrid {
    /// index番目のカードの範囲(スクロール位置row_offset込み)。画面外(スクロールで隠れた行・
    /// 範囲外のインデックス)ならNone。外枠の内側に収まらない分は切り詰める
    fn card_rect(&self, index: usize, row_offset: usize) -> Option<Rect> {
        if index >= MENU_ITEMS.len() {
            return None;
        }
        let (row, column) = (index / self.columns, index % self.columns);
        if row < row_offset || row >= row_offset + self.visible_rows {
            return None;
        }
        let rect = Rect::new(
            self.inner.x + self.x_offset + column as u16 * self.card_width,
            self.inner.y + (row - row_offset) as u16 * self.card_height,
            self.card_width,
            self.card_height,
        )
        .intersection(self.inner);
        (!rect.is_empty()).then_some(rect)
    }

    /// カード1枚ぶんのテキストの行数(ゲーム名1行+説明文)
    fn lines_per_card(&self) -> usize {
        1 + self.desc_lines
    }

    /// 選択中のカードの行が画面内に入るスクロール位置。currentから動かさなくて済むなら動かさず、
    /// 全行が収まる大きさなら0にする(以前の小さい画面での位置が残らないように)
    fn scroll_offset(&self, selected: usize, current: usize) -> usize {
        let row = selected / self.columns;
        let offset = current.min(self.rows - self.visible_rows);
        if row < offset {
            row
        } else if row >= offset + self.visible_rows {
            row + 1 - self.visible_rows
        } else {
            offset
        }
    }
}

fn menu_grid(area: Rect) -> MenuGrid {
    let inner = menu_block().inner(area);
    let len = MENU_ITEMS.len();
    let columns = usize::from(inner.width / MENU_CARD_MIN_WIDTH).clamp(1, len);
    let rows = len.div_ceil(columns);
    let card_width = inner.width / columns as u16;
    let x_offset = (inner.width - card_width * columns as u16) / 2;
    // 枠(左右各1セル)と余白を除いた幅。極端に狭くても全角1文字は入る幅にする
    let text_width = usize::from(card_width.saturating_sub(2 + 2 * MENU_CARD_PADDING)).max(2);
    let desc_lines = MENU_DESCRIPTIONS
        .iter()
        .map(|description| wrap_by_width(description, text_width).len())
        .max()
        .unwrap_or(1)
        .max(1);
    // 枠(上下各1行)+アイコン+ゲーム名+説明文
    let card_height = 2 + MENU_ICON_ROWS + 1 + desc_lines as u16;
    let visible_rows = usize::from(inner.height / card_height).clamp(1, rows);
    MenuGrid {
        inner,
        x_offset,
        columns,
        rows,
        card_width,
        card_height,
        text_width,
        desc_lines,
        visible_rows,
    }
}

/// textを表示幅widthごとに折り返す(1文字ずつ詰める。文字の追加・削除はしない)
fn wrap_by_width(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    for c in text.chars() {
        let char_width = Span::raw(c.to_string()).width();
        if line_width + char_width > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        line.push(c);
        line_width += char_width;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// メニュー画面での上下左右キーの移動方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GridMove {
    Up,
    Down,
    Left,
    Right,
}

/// columns列に並べたlen枚のカードで、selectedからdirectionへ1つ動いた先のインデックス。
/// 左右は同じ行の中で、上下は同じ列の中で端から反対の端へ折り返す(最終行が欠けていても同様)
fn grid_move(selected: usize, direction: GridMove, columns: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let columns = columns.max(1);
    let selected = selected.min(len - 1);
    let (row, column) = (selected / columns, selected % columns);
    match direction {
        GridMove::Left | GridMove::Right => {
            let row_start = row * columns;
            let row_len = columns.min(len - row_start);
            let step = if direction == GridMove::Right { 1 } else { row_len - 1 };
            row_start + (column + step) % row_len
        }
        GridMove::Up | GridMove::Down => {
            let column_len = (len - column).div_ceil(columns);
            let step = if direction == GridMove::Down { 1 } else { column_len - 1 };
            (row + step) % column_len * columns + column
        }
    }
}

/// Menu画面でのクリック位置(画面全体のarea基準)から、そこに描いているカードのインデックスを求める。
/// row_offsetは直近の描画でのスクロール位置。カードの外(外枠・カードの間の余白)はNone
fn menu_card_at(area: Rect, row_offset: usize, column: u16, row: u16) -> Option<usize> {
    let grid = menu_grid(area);
    let position = ratatui::layout::Position::new(column, row);
    (0..MENU_ITEMS.len())
        .find(|&i| grid.card_rect(i, row_offset).is_some_and(|card| card.contains(position)))
}

/// メニュー全カードのテキスト(ゲーム名+説明文)をカードの並び順(左上から右下)に並べて返す。
/// 1カードちょうどlines_per_card行ずつで、説明文はカード幅で折り返し、足りない行は空行で埋める。
/// タイプライター表示はこの並びの先頭から1文字ずつ流す
fn menu_item_lines(area: Rect) -> Vec<Line<'static>> {
    let grid = menu_grid(area);
    let name_style = Style::default()
        .fg(theme::TEXT)
        .add_modifier(Modifier::BOLD);
    let description_style = Style::default().fg(theme::MUTED);
    MENU_ITEMS
        .iter()
        .zip(MENU_DESCRIPTIONS)
        .flat_map(|(name, description)| {
            let mut lines = vec![Line::from(Span::styled(*name, name_style))];
            lines.extend(
                wrap_by_width(description, grid.text_width)
                    .into_iter()
                    .map(|line| Line::from(Span::styled(line, description_style))),
            );
            lines.resize(grid.lines_per_card(), Line::from(""));
            lines
        })
        .collect()
}

fn render_menu(
    frame: &mut Frame,
    area: Rect,
    state: &mut MenuGridState,
    typing: &mut Typewriter,
    icons: &mut MenuIcons,
) {
    frame.render_widget(menu_block(), area);
    let grid = menu_grid(area);
    state.row_offset = grid.scroll_offset(state.selected, state.row_offset);

    // 画面サイズによって説明文の折り返し(=空行の数)が変わるので、描画する内容に全文字数を合わせる
    let lines = menu_item_lines(area);
    typing.set_total_chars(typewriter::char_count(&lines));
    let lines = typewriter::truncate_lines(&lines, typing.visible_chars());
    for (index, card_lines) in lines.chunks(grid.lines_per_card()).enumerate() {
        if let Some(card) = grid.card_rect(index, state.row_offset) {
            let selected = index == state.selected;
            render_menu_card(frame, card, index, selected, card_lines, icons);
        }
    }
}

/// カード1枚(枠・アイコン・ゲーム名・説明文)を描く。選択中のカードは二重罫線の明るい枠にし、
/// ゲーム名も強調色にする。アイコン画像は位置が変わらない限り使い回されるので、
/// 選択が動いても描き替わるのは枠と文字だけ
fn render_menu_card(
    frame: &mut Frame,
    card: Rect,
    index: usize,
    selected: bool,
    lines: &[Line<'static>],
    icons: &mut MenuIcons,
) {
    let block = if selected {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(
                Style::default()
                    .fg(theme::ACCENT_STRONG)
                    .add_modifier(Modifier::BOLD),
            )
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::ACCENT_SUB))
    };
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let icon_rows = MENU_ICON_ROWS.min(inner.height);
    // 画面からはみ出して欠けたカードには画像を描かない(切れた画像を端末へ送らない)
    if icon_rows == MENU_ICON_ROWS {
        icons.render(frame, index, Rect::new(inner.x, inner.y, inner.width, icon_rows));
    }

    let text_area = Rect::new(
        inner.x + MENU_CARD_PADDING.min(inner.width),
        inner.y + icon_rows,
        inner.width.saturating_sub(2 * MENU_CARD_PADDING),
        inner.height - icon_rows,
    );
    let mut lines = lines.to_vec();
    if selected {
        if let Some(name) = lines.first_mut() {
            for span in &mut name.spans {
                span.style = theme::selected_style();
            }
        }
    }
    // 左寄せにして、タイプライターで文字が増えても行頭の位置が動かないようにする
    frame.render_widget(Paragraph::new(lines), text_area);
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

/// リザルト画面の本文(カード内のテキスト)。タイプライター表示はこの並びの先頭から1文字ずつ流す
fn result_lines(result: &GameResult) -> Vec<Line<'static>> {
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
        (crate::game::count_mania::GAME_ID, COUNT_MANIA_ITEM_INDEX),
        (crate::game::color_stack::GAME_ID, COLOR_STACK_ITEM_INDEX),
        (crate::game::rhythm::GAME_ID, RHYTHM_ITEM_INDEX),
        (crate::game::quick_draw::GAME_ID, QUICK_DRAW_ITEM_INDEX),
        (crate::game::beigoma::GAME_ID, BEIGOMA_ITEM_INDEX),
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

    let label_style = Style::default().fg(theme::MUTED);
    let value_style = Style::default()
        .fg(theme::ACCENT_STRONG)
        .add_modifier(Modifier::BOLD);
    vec![
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
    ]
}

/// リザルト画面のテキストカードの範囲。外枠の内側(inner)の中央に、幅48以内・本文の行数+枠の高さで置く
fn result_card_rect(inner: Rect, content_height: u16) -> Rect {
    centered_rect(inner, inner.width.min(48), (content_height + 2).min(inner.height))
}

fn render_result(
    frame: &mut Frame,
    area: Rect,
    result: &GameResult,
    save_error: Option<&str>,
    typing: &mut Typewriter,
    sprite: &mut ResultSprite,
) {
    // 保存(呼び出し側のenter_resultで1回だけ実施済み)に失敗していれば、
    // その内容を結果の描画後に下端へ重ねて表示する

    // 外枠(見出し・操作説明)はタイプライター表示の対象外で、最初から出す
    let outer = theme::panel(Line::from(" ◆ RESULT ◆ ").centered()).title_bottom(
        theme::hints_line(&[("Enter / Esc / クリック", "メニューに戻る")]).centered(),
    );
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let lines = result_lines(result);
    typing.set_total_chars(typewriter::char_count(&lines));
    // 中央寄せなので、まだ出していない部分を空白で埋めて行の位置がずれないようにする
    let text = typewriter::truncate_lines_keep_width(&lines, typing.visible_chars());
    let content_height = text.len() as u16;
    let card = result_card_rect(inner, content_height);
    // キャラクターはカードの左側の余白に描く(余白が狭ければsprite側で省略する)
    sprite.render(frame, inner, card);
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

    // --- カウントマニア ---

    #[test]
    fn count_mania_comes_right_before_color_stack() {
        assert_eq!(MENU_ITEMS[COUNT_MANIA_ITEM_INDEX], "カウントマニア");
        assert_eq!(COUNT_MANIA_ITEM_INDEX + 1, COLOR_STACK_ITEM_INDEX);
    }

    #[test]
    fn new_game_for_count_mania_item_creates_count_mania() {
        // カウントマニアはROUND1〜5で難易度・動きが変わる固定進行なので、渡した難易度によらず
        // 代表値の難易度で記録する
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let game = new_game(COUNT_MANIA_ITEM_INDEX, difficulty);
            let result = game.result();
            assert_eq!(result.game_id, crate::game::count_mania::GAME_ID);
            assert_eq!(
                result.difficulty,
                crate::game::count_mania::SESSION_DIFFICULTY
            );
        }
    }

    /// カウントマニアが始まり、ROUND1(初級)が表示されていることを確かめる
    fn assert_count_mania_round1_is_playing(app: &mut App) {
        finish_countdown(app);
        let Screen::Playing(game) = &app.screen else {
            panic!("Playing画面のはず");
        };
        assert_eq!(game.result().game_id, crate::game::count_mania::GAME_ID);
        assert!(!game.is_finished());
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("マウス専用"));
        assert!(text.contains("ROUND1/5"), "ROUND1から始まる");
        assert!(text.contains("初級"), "ROUND1は初級");
    }

    #[test]
    fn selecting_count_mania_skips_difficulty_and_starts_round1_after_countdown() {
        let mut app = App::new();
        app.select_menu_item(COUNT_MANIA_ITEM_INDEX);
        let Screen::Countdown {
            item,
            difficulty,
            state,
        } = &app.screen
        else {
            panic!("難易度選択を挟まずカウントダウンになるはず");
        };
        assert_eq!(*item, COUNT_MANIA_ITEM_INDEX);
        assert_eq!(*difficulty, crate::game::count_mania::SESSION_DIFFICULTY);
        assert_eq!(state.phase(), Some(Phase::Three), "3から始まる");
        assert_count_mania_round1_is_playing(&mut app);
    }

    #[test]
    fn enter_on_count_mania_in_menu_goes_straight_to_countdown() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app.menu_state.select(COUNT_MANIA_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(
            app.screen,
            Screen::Countdown {
                item: COUNT_MANIA_ITEM_INDEX,
                ..
            }
        ));
        assert_count_mania_round1_is_playing(&mut app);
    }

    // --- ソコヌキ ---

    #[test]
    fn color_stack_is_the_last_game_before_rhythm() {
        assert_eq!(MENU_ITEMS[COLOR_STACK_ITEM_INDEX], "ソコヌキ");
        assert_eq!(COLOR_STACK_ITEM_INDEX + 1, RHYTHM_ITEM_INDEX);
    }

    #[test]
    fn new_game_for_color_stack_item_creates_color_stack() {
        // ソコヌキは難易度を持たないので、渡した難易度によらず固定の記録になる
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let game = new_game(COLOR_STACK_ITEM_INDEX, difficulty);
            let result = game.result();
            assert_eq!(result.game_id, crate::game::color_stack::GAME_ID);
            assert_eq!(
                result.difficulty,
                crate::game::color_stack::SESSION_DIFFICULTY
            );
        }
    }

    /// ソコヌキが始まり、ROUND1(4列)が表示されていることを確かめる
    fn assert_color_stack_round1_is_playing(app: &mut App) {
        finish_countdown(app);
        let Screen::Playing(game) = &app.screen else {
            panic!("Playing画面のはず");
        };
        assert_eq!(game.result().game_id, crate::game::color_stack::GAME_ID);
        assert!(!game.is_finished());
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("ソコヌキ"));
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
            .expect("ソコヌキが描かれていること");
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

    /// 難易度選択を挟まない固定進行のゲーム(メニュー項目, GAME_ID, 記録する難易度)
    fn fixed_progression_games() -> [(usize, &'static str, Difficulty); 2] {
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

    #[test]
    fn memory_and_reaction_item_indices_match_menu() {
        assert_eq!(MENU_ITEMS[MEMORY_ITEM_INDEX], "オイカケ");
        assert_eq!(MENU_ITEMS[REACTION_ITEM_INDEX], "イロピッタン");
    }

    #[test]
    fn new_game_for_memory_and_reaction_records_session_difficulty() {
        // 難易度を選ばないので、渡した難易度によらず代表値の難易度で記録する
        for (item, game_id, session_difficulty) in fixed_progression_games() {
            for difficulty in [
                Difficulty::Beginner,
                Difficulty::Intermediate,
                Difficulty::Advanced,
            ] {
                let result = new_game(item, difficulty).result();
                assert_eq!(result.game_id, game_id);
                assert_eq!(result.difficulty, session_difficulty, "item={item}");
            }
        }
    }

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
    fn jukebox_and_history_are_the_last_two_menu_items() {
        assert_eq!(JUKEBOX_ITEM_INDEX, MENU_ITEMS.len() - 2);
        assert_eq!(HISTORY_ITEM_INDEX, MENU_ITEMS.len() - 1);
        assert_eq!(MENU_ITEMS[JUKEBOX_ITEM_INDEX], "ジュークボックス");
        assert_eq!(MENU_ITEMS[HISTORY_ITEM_INDEX], "履歴");
    }

    fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect::new(x, y, width, height)
    }

    // --- メニューのカードグリッド: データ・レイアウト ---

    #[test]
    fn menu_icon_paths_match_every_menu_item_in_order() {
        let expected = [
            "shape_rotate.png",
            "mirror_match.png",
            "reaction.png",
            "mental_calc.png",
            "pattern_fill.png",
            "memory.png",
            "sequence.png",
            "puzzle_connect.png",
            "count_mania.png",
            "color_stack.png",
            "rhythm.png",
            "quick_draw.png",
            "beigoma.png",
            "jukebox.png",
            "history.png",
        ];
        assert_eq!(MENU_ICON_PATHS.len(), MENU_ITEMS.len());
        for (path, file) in MENU_ICON_PATHS.iter().zip(expected) {
            assert_eq!(*path, format!("menu_icons/{file}"));
        }
    }

    /// 画像がまだ生成されていないアイコン(読めない間はカードのアイコン部分が空白になる)
    const PENDING_MENU_ICONS: [&str; 0] = [];

    #[test]
    fn every_menu_icon_is_embedded_and_decodes() {
        for path in MENU_ICON_PATHS {
            if PENDING_MENU_ICONS.contains(&path) {
                continue;
            }
            assert!(
                crate::ui::menu_icons::load_icon_image(path).is_some(),
                "{path}: 埋め込まれていて画像としてデコードできること"
            );
        }
    }

    #[test]
    fn menu_grid_columns_follow_the_screen_width() {
        let columns = |width: u16| menu_grid(rect(0, 0, width, 30)).columns;
        for width in [0u16, 1, 5, 10, 20] {
            assert_eq!(columns(width), 1, "width={width}: 狭い画面は1列");
        }
        assert!(columns(80) >= 2, "80桁なら複数列");
        assert!(columns(160) > columns(80), "広い画面ほど列が増える");
        let mut previous = 1;
        for width in 0..400u16 {
            let now = columns(width);
            assert!(now >= previous, "width={width}: 幅が増えて列が減らない");
            assert!(now <= MENU_ITEMS.len(), "列数は項目数を超えない");
            previous = now;
        }
    }

    #[test]
    fn menu_grid_rows_is_items_divided_by_columns_rounded_up() {
        for width in [20u16, 80, 120, 200, 400] {
            let grid = menu_grid(rect(0, 0, width, 30));
            assert_eq!(grid.rows, MENU_ITEMS.len().div_ceil(grid.columns), "width={width}");
        }
    }

    #[test]
    fn menu_cards_fit_inside_the_menu_frame_without_overlapping() {
        for (width, height) in [(80u16, 24u16), (80, 30), (120, 40), (200, 60), (30, 12)] {
            let area = rect(0, 0, width, height);
            let grid = menu_grid(area);
            let inner = menu_block().inner(area);
            let cards: Vec<Rect> = (0..MENU_ITEMS.len())
                .filter_map(|i| grid.card_rect(i, 0))
                .collect();
            assert!(!cards.is_empty(), "{width}x{height}: 少なくとも1枚は見える");
            for (i, a) in cards.iter().enumerate() {
                assert_eq!(a.intersection(inner), *a, "{width}x{height}: 枠の内側に収まる");
                for b in &cards[i + 1..] {
                    assert!(!a.intersects(*b), "{width}x{height}: カード同士が重ならない");
                }
            }
        }
    }

    #[test]
    fn grid_move_left_right_wraps_within_the_row() {
        let len = MENU_ITEMS.len(); // 15項目・4列 → 最終行は12,13,14の3枚
        assert_eq!(grid_move(0, GridMove::Right, 4, len), 1);
        assert_eq!(grid_move(3, GridMove::Right, 4, len), 0, "行末から行頭へ折り返す");
        assert_eq!(grid_move(0, GridMove::Left, 4, len), 3, "行頭から行末へ折り返す");
        assert_eq!(grid_move(5, GridMove::Left, 4, len), 4);
        assert_eq!(grid_move(14, GridMove::Right, 4, len), 12, "欠けた最終行の中で折り返す");
        assert_eq!(grid_move(12, GridMove::Left, 4, len), 14);
    }

    #[test]
    fn grid_move_up_down_wraps_within_the_column() {
        let len = MENU_ITEMS.len();
        assert_eq!(grid_move(0, GridMove::Down, 4, len), 4);
        assert_eq!(grid_move(12, GridMove::Down, 4, len), 0, "列の下端から上端へ折り返す");
        assert_eq!(grid_move(0, GridMove::Up, 4, len), 12, "列の上端から下端へ折り返す");
        // 3列目(0始まり)は3,7,11の3枚だけ(最終行に無い)
        assert_eq!(grid_move(11, GridMove::Down, 4, len), 3);
        assert_eq!(grid_move(3, GridMove::Up, 4, len), 11);
    }

    #[test]
    fn grid_move_with_one_column_behaves_like_the_old_list() {
        let len = MENU_ITEMS.len();
        assert_eq!(grid_move(0, GridMove::Down, 1, len), 1);
        assert_eq!(grid_move(len - 1, GridMove::Down, 1, len), 0, "末尾から先頭へ");
        assert_eq!(grid_move(0, GridMove::Up, 1, len), len - 1, "先頭から末尾へ");
        assert_eq!(grid_move(5, GridMove::Left, 1, len), 5, "1列なら左右は動かない");
        assert_eq!(grid_move(5, GridMove::Right, 1, len), 5);
    }

    #[test]
    fn arrow_keys_move_the_selection_across_the_grid() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app.last_area = rect(0, 0, 200, 60);
        let columns = menu_grid(screen_rect(app.last_area)).columns;
        assert!(columns >= 3);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.menu_state.selected(), 1);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.menu_state.selected(), 1 + columns);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.menu_state.selected(), columns);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.menu_state.selected(), 0);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.menu_state.selected(), columns - 1, "行内で折り返す");
    }

    #[test]
    fn menu_card_at_maps_every_cell_of_a_card_to_its_index() {
        for (width, height) in [(80u16, 30u16), (200, 60)] {
            let area = rect(0, 0, width, height);
            let grid = menu_grid(area);
            for i in 0..MENU_ITEMS.len() {
                let Some(card) = grid.card_rect(i, 0) else {
                    continue;
                };
                for p in card.positions() {
                    assert_eq!(menu_card_at(area, 0, p.x, p.y), Some(i), "{width}x{height}");
                }
            }
        }
    }

    #[test]
    fn menu_card_at_outside_the_cards_is_none() {
        let area = rect(0, 0, 80, 30);
        assert_eq!(menu_card_at(area, 0, 0, 0), None, "外枠の上はNone");
        assert_eq!(menu_card_at(area, 0, 79, 29), None, "外枠の右下はNone");
        assert_eq!(menu_card_at(area, 0, 200, 5), None, "画面の外はNone");
    }

    #[test]
    fn menu_card_at_follows_the_scroll_offset() {
        let area = rect(0, 0, 80, 24);
        let grid = menu_grid(area);
        let top = grid.card_rect(0, 0).unwrap();
        // 1行ぶんスクロールすると、先頭の位置には2行目の先頭のカードが来る
        assert_eq!(menu_card_at(area, 1, top.x, top.y), Some(grid.columns));
        assert_eq!(grid.card_rect(0, 1), None, "スクロールで上に隠れたカードは描かない");
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

    // --- リズムゲーム: Menu → (TTR画像を背景にした)曲選択 → プレイ ---

    #[test]
    fn rhythm_item_index_points_at_rhythm_menu_item() {
        assert_eq!(MENU_ITEMS[RHYTHM_ITEM_INDEX], "TTR");
    }

    // --- ハヤウチ ---

    #[test]
    fn quick_draw_comes_right_after_ttr_and_before_beigoma() {
        assert_eq!(MENU_ITEMS[QUICK_DRAW_ITEM_INDEX], "ハヤウチ");
        assert_eq!(RHYTHM_ITEM_INDEX + 1, QUICK_DRAW_ITEM_INDEX);
        assert_eq!(QUICK_DRAW_ITEM_INDEX + 1, BEIGOMA_ITEM_INDEX);
        // 先頭側の既存インデックスはずれない
        assert_eq!(MENU_ITEMS[COUNT_MANIA_ITEM_INDEX], "カウントマニア");
        assert_eq!(MENU_ITEMS[COLOR_STACK_ITEM_INDEX], "ソコヌキ");
    }

    #[test]
    fn quick_draw_menu_texts_no_longer_use_old_name() {
        assert!(!MENU_ITEMS.contains(&"反射神経"));
        assert!(!MENU_DESCRIPTIONS[QUICK_DRAW_ITEM_INDEX].contains("反射神経"));
    }

    #[test]
    fn new_game_for_quick_draw_item_creates_quick_draw() {
        // ハヤウチは難易度を選ばないので、渡した難易度によらず代表値の難易度で記録する
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let game = new_game(QUICK_DRAW_ITEM_INDEX, difficulty);
            let result = game.result();
            assert_eq!(result.game_id, crate::game::quick_draw::GAME_ID);
            assert_eq!(
                result.difficulty,
                crate::game::quick_draw::SESSION_DIFFICULTY
            );
        }
    }

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

    #[test]
    fn result_screen_shows_quick_draw_menu_name() {
        let mut app = App::new();
        app.screen = Screen::Result(
            new_game(QUICK_DRAW_ITEM_INDEX, Difficulty::Beginner).result(),
            None,
        );
        let text = rendered_text(&mut app).replace(' ', "");
        assert!(text.contains("ハヤウチ"));
        assert!(!text.contains("反射神経"));
    }

    // --- べー ---

    #[test]
    fn beigoma_comes_right_after_quick_draw_and_before_jukebox() {
        assert_eq!(MENU_ITEMS[BEIGOMA_ITEM_INDEX], "べー");
        assert_eq!(QUICK_DRAW_ITEM_INDEX + 1, BEIGOMA_ITEM_INDEX);
        assert_eq!(BEIGOMA_ITEM_INDEX + 1, JUKEBOX_ITEM_INDEX);
        assert_eq!(MENU_ICON_PATHS[BEIGOMA_ITEM_INDEX], "menu_icons/beigoma.png");
    }

    #[test]
    fn new_game_for_beigoma_item_creates_beigoma() {
        // べーは難易度を選ばないので、渡した難易度によらず固定の難易度で記録する
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let result = new_game(BEIGOMA_ITEM_INDEX, difficulty).result();
            assert_eq!(result.game_id, crate::game::beigoma::GAME_ID);
            assert_eq!(result.difficulty, crate::game::beigoma::SESSION_DIFFICULTY);
        }
    }

    /// べーが始まり、ベーゴマが盤に投入されて制限時間60秒から数え始めていることを確かめる
    fn assert_beigoma_is_playing(app: &mut App) {
        finish_countdown(app);
        let Screen::Playing(game) = &app.screen else {
            panic!("Playing画面のはず");
        };
        assert_eq!(game.result().game_id, crate::game::beigoma::GAME_ID);
        assert!(!game.is_finished());
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("べー"), "{text}");
        assert!(text.contains("残り60.0秒"), "カウントダウン後から数え始める: {text}");
        assert!(text.contains("盤面") && text.contains("軽トラ視点"), "2視点を出す");
    }

    #[test]
    fn selecting_beigoma_skips_difficulty_and_starts_after_countdown() {
        let mut app = App::new();
        app.select_menu_item(BEIGOMA_ITEM_INDEX);
        let Screen::Countdown {
            item,
            difficulty,
            state,
        } = &app.screen
        else {
            panic!("難易度選択を挟まずカウントダウンになるはず");
        };
        assert_eq!(*item, BEIGOMA_ITEM_INDEX);
        assert_eq!(*difficulty, crate::game::beigoma::SESSION_DIFFICULTY);
        assert_eq!(state.phase(), Some(Phase::Three), "3から始まる");
        // GO!!の表示が終わるまではまだベーゴマを投入しない(ゲームを作らない)
        app.update(COUNTDOWN_TOTAL - Duration::from_millis(1));
        assert!(matches!(app.screen, Screen::Countdown { .. }));
        assert_beigoma_is_playing(&mut app);
    }

    #[test]
    fn enter_on_beigoma_in_menu_goes_straight_to_countdown() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app.menu_state.select(BEIGOMA_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(
            app.screen,
            Screen::Countdown {
                item: BEIGOMA_ITEM_INDEX,
                ..
            }
        ));
        assert_beigoma_is_playing(&mut app);
    }

    #[test]
    fn result_screen_shows_beigoma_menu_name() {
        let mut app = App::new();
        app.screen = Screen::Result(
            new_game(BEIGOMA_ITEM_INDEX, Difficulty::Beginner).result(),
            None,
        );
        let text = rendered_text(&mut app).replace(' ', "");
        assert!(text.contains("べー"), "{text}");
    }

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
    fn game_over_switches_bgm_to_result_category() {
        let mut app = App::new();
        app.select_menu_item(0); // shape_rotate
        app.handle_key(KeyEvent::from(KeyCode::Char('1'))); // Beginnerでプレイ開始
        finish_countdown(&mut app);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            app.handle_key(KeyEvent::from(KeyCode::Left));
        }
        assert!(matches!(app.screen, Screen::Result(..)));
        assert_eq!(app.current_bgm.as_deref(), Some("New_Personal_Best"));
    }

    #[test]
    fn rendering_the_result_screen_repeatedly_does_not_re_save_history() {
        // 履歴への保存はenter_resultで1回だけ行い、render()を繰り返しても再実行しない
        // (以前はrender_resultがappend_resultを呼んでいて、描画のたびに重複保存されていた)
        let mut app = App::new();
        app.select_menu_item(0); // shape_rotate
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
        finish_countdown(&mut app);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            app.handle_key(KeyEvent::from(KeyCode::Left));
        }
        let Screen::Result(_, save_error_after_enter) = &app.screen else {
            panic!("リザルト画面のはず");
        };
        let save_error_after_enter = save_error_after_enter.clone();
        for _ in 0..5 {
            rendered_text(&mut app);
        }
        let Screen::Result(_, save_error_after_render) = &app.screen else {
            panic!("リザルト画面のはず");
        };
        assert_eq!(
            &save_error_after_enter, save_error_after_render,
            "描画を繰り返しても保存処理は再実行されない"
        );
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

    // --- タイトル画面(Splash) ---

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
        assert_eq!(buffer[(screen.x, screen.y)].symbol(), "╭", "枠の左上はscreen_rectの左上");
        assert_eq!(
            buffer[(screen.x, screen.y)].fg,
            theme::ACCENT,
            "枠の色はメニューと同じACCENT"
        );
    }

    #[test]
    fn entering_menu_requests_a_scrollback_clear_exactly_once() {
        let mut app = App::new();
        assert!(
            !app.take_pending_scrollback_clear(),
            "起動直後(Splash)ではまだ要求しない"
        );
        app.handle_key(KeyEvent::from(KeyCode::Enter)); // Splash -> Menu
        assert!(matches!(app.screen, Screen::Menu));
        assert!(
            app.take_pending_scrollback_clear(),
            "メニューに入ったらクリアを要求する"
        );
        assert!(
            !app.take_pending_scrollback_clear(),
            "取り出したら消費されて次はfalse"
        );
    }

    #[test]
    fn returning_to_menu_from_every_non_menu_screen_requests_a_scrollback_clear() {
        for (name, mut app) in apps_on_every_non_menu_screen() {
            press(&mut app, KeyCode::Char('q'));
            assert!(matches!(app.screen, Screen::Menu), "{name}: メニューへ戻る");
            assert!(
                app.take_pending_scrollback_clear(),
                "{name}: メニューへ戻ったらクリアを要求する"
            );
        }
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

    /// 描画結果から、textが描かれている位置(先頭の文字のセル)を探す。
    /// 全角文字の2セル目・空白は除いて行ごとに文字を連結し、その中からtextを探す
    fn drawn_position_of(app: &mut App, text: &str, width: u16, height: u16) -> Option<(u16, u16)> {
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

    fn left_click_at(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    /// メニュー画面(タイプライター表示済み)のAppを作る
    fn app_on_menu() -> App {
        let mut app = App::new();
        app.screen = Screen::Menu;
        app
    }

    #[test]
    fn menu_cards_are_drawn_where_clicks_select_them() {
        // 画面の大きさによって列数・カードの大きさが変わっても、項目名が見えている位置を
        // クリックすればその項目になること
        for (width, height) in [(80u16, 24u16), (80, 30), (120, 40), (200, 60)] {
            let mut drawn = 0;
            for (i, name) in MENU_ITEMS.iter().enumerate() {
                let mut app = app_on_menu();
                let Some((x, y)) = drawn_position_of(&mut app, name, width, height) else {
                    continue;
                };
                drawn += 1;
                let area = screen_rect(rect(0, 0, width, height));
                assert_eq!(
                    menu_card_at(area, app.menu_state.row_offset, x, y),
                    Some(i),
                    "{width}x{height}: {name}の位置"
                );
            }
            assert!(drawn >= 2, "{width}x{height}: 複数のカードが見えている");
        }
    }

    #[test]
    fn all_cards_are_drawn_on_a_large_screen() {
        let mut app = app_on_menu();
        for name in MENU_ITEMS {
            assert!(
                drawn_position_of(&mut app, name, 200, 60).is_some(),
                "{name}が描かれていること"
            );
        }
    }

    #[test]
    fn cards_are_laid_out_left_to_right_then_top_to_bottom() {
        let mut app = app_on_menu();
        let positions: Vec<(u16, u16)> = MENU_ITEMS
            .iter()
            .map(|name| drawn_position_of(&mut app, name, 200, 60).unwrap())
            .collect();
        let columns = menu_grid(screen_rect(rect(0, 0, 200, 60))).columns;
        assert_eq!(positions[0].1, positions[1].1, "1枚目と2枚目は同じ行");
        assert!(positions[0].0 < positions[1].0, "2枚目は1枚目の右");
        assert!(positions[columns].1 > positions[0].1, "列数ぶん進むと次の行");
    }

    #[test]
    fn moving_to_a_card_below_the_screen_scrolls_it_into_view() {
        let (width, height) = (80u16, 24u16);
        let mut app = app_on_menu();
        rendered_cells(&mut app, width, height);
        let grid = menu_grid(screen_rect(rect(0, 0, width, height)));
        assert!(grid.visible_rows < grid.rows, "この大きさでは全行は収まらない");
        // 同じ列を最下行まで下がる
        let last = (grid.rows - 1) * grid.columns;
        while app.menu_state.selected() != last {
            press(&mut app, KeyCode::Down);
            let name = MENU_ITEMS[app.menu_state.selected()];
            assert!(
                drawn_position_of(&mut app, name, width, height).is_some(),
                "選択中の{name}が常に画面内に描かれる"
            );
        }
        assert!(
            drawn_position_of(&mut app, MENU_ITEMS[0], width, height).is_none(),
            "先頭の行は上にスクロールして隠れる"
        );
        // 最下行で下を押すと列の先頭へ折り返し、先頭の行が見える位置へ戻る
        press(&mut app, KeyCode::Down);
        assert_eq!(app.menu_state.selected(), 0);
        assert!(drawn_position_of(&mut app, MENU_ITEMS[0], width, height).is_some());
    }

    #[test]
    fn clicking_a_scrolled_card_selects_it() {
        let (width, height) = (80u16, 24u16);
        let mut app = app_on_menu();
        rendered_cells(&mut app, width, height);
        let columns = menu_grid(screen_rect(rect(0, 0, width, height))).columns;
        // 履歴のカードが見えるまで下へ移動する
        while app.menu_state.selected() / columns != HISTORY_ITEM_INDEX / columns {
            press(&mut app, KeyCode::Down);
        }
        let (x, y) = drawn_position_of(&mut app, MENU_ITEMS[HISTORY_ITEM_INDEX], width, height)
            .expect("履歴のカードがスクロールして見えていること");
        app.handle_mouse(left_click_at(x, y));
        assert!(matches!(app.screen, Screen::History));
    }

    #[test]
    fn menu_scroll_resets_when_every_card_fits() {
        let mut app = app_on_menu();
        rendered_cells(&mut app, 80, 24);
        app.menu_state.select(HISTORY_ITEM_INDEX);
        rendered_cells(&mut app, 80, 24);
        assert!(app.menu_state.row_offset > 0, "小さい画面ではスクロールしている");
        rendered_cells(&mut app, 200, 60);
        assert_eq!(app.menu_state.row_offset, 0, "全部収まる大きさならスクロールしない");
        assert!(drawn_position_of(&mut app, MENU_ITEMS[0], 200, 60).is_some());
    }

    /// 描画結果から、左上の角がcornerの枠のうちnameを囲んでいるものの左上の位置を探す
    fn frame_corner_around(app: &mut App, name: &str, corner: &str) -> Option<(u16, u16)> {
        let (nx, ny) = drawn_position_of(app, name, 200, 60)?;
        let rows = rendered_cells(app, 200, 60);
        (0..ny as usize).rev().find_map(|y| {
            (0..=nx as usize)
                .rev()
                .find(|&x| rows[y][x] == corner)
                .map(|x| (x as u16, y as u16))
        })
    }

    #[test]
    fn only_the_selected_card_has_a_double_border() {
        let mut app = app_on_menu();
        let rows = rendered_cells(&mut app, 200, 60);
        let doubles: Vec<(usize, usize)> = rows
            .iter()
            .enumerate()
            .flat_map(|(y, r)| r.iter().enumerate().filter(|(_, c)| *c == "╔").map(move |(x, _)| (x, y)))
            .collect();
        assert_eq!(doubles.len(), 2, "外枠と選択中のカードだけが二重罫線");
        let selected = frame_corner_around(&mut app, MENU_ITEMS[0], "╔");
        assert!(selected.is_some_and(|p| p != (0, 0)), "選択中の1枚目を二重罫線が囲む");
        assert!(
            frame_corner_around(&mut app, MENU_ITEMS[1], "╭").is_some(),
            "選択していないカードは角丸の枠"
        );

        // 選択を右へ動かすと、二重罫線も右のカードへ移る
        press(&mut app, KeyCode::Right);
        let moved = frame_corner_around(&mut app, MENU_ITEMS[1], "╔").unwrap();
        assert!(moved.0 > selected.unwrap().0);
        assert!(frame_corner_around(&mut app, MENU_ITEMS[0], "╭").is_some());
    }

    #[test]
    fn fallback_cards_show_the_frame_and_text_with_a_blank_icon_area() {
        let mut app = app_on_menu();
        assert!(app.menu_icons.is_fallback(), "テストでは画像プロトコルを使わない");
        let (x0, y0) = frame_corner_around(&mut app, MENU_ITEMS[0], "╔").expect("カード枠がある");
        let (_, name_y) = drawn_position_of(&mut app, MENU_ITEMS[0], 200, 60).unwrap();
        let rows = rendered_cells(&mut app, 200, 60);
        let right = (x0 as usize + 1..rows[y0 as usize].len())
            .find(|&x| rows[y0 as usize][x] == "╗")
            .unwrap();
        assert_eq!(
            name_y - y0 - 1,
            MENU_ICON_ROWS,
            "枠とゲーム名の間にアイコン分の行がある"
        );
        for (y, row) in rows.iter().enumerate().take(name_y as usize).skip(y0 as usize + 1) {
            for (x, cell) in row.iter().enumerate().take(right).skip(x0 as usize + 1) {
                assert_eq!(cell, " ", "アイコン部分({x},{y})は空白");
            }
        }
        let card_text: String = (name_y as usize..rows.len())
            .take_while(|&y| rows[y][x0 as usize] != "╚")
            .map(|y| rows[y][x0 as usize + 1..right].concat().replace(' ', ""))
            .collect();
        assert!(card_text.contains(MENU_ITEMS[0]));
        assert!(card_text.contains(MENU_DESCRIPTIONS[0]), "説明文がゲーム名の下に出る");
    }

    /// 端末に問い合わせないハーフブロック描画のpickerでアイコンを読み込んだApp
    fn app_on_menu_with_icons() -> App {
        let mut picker = ratatui_image::picker::Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Halfblocks);
        let mut app = app_on_menu();
        app.menu_icons = MenuIcons::with_picker(&MENU_ICON_PATHS, Some(picker));
        app
    }

    /// カードのアイコン部分(枠の内側でゲーム名より上)のセル
    fn icon_cells(app: &mut App, index: usize) -> Vec<String> {
        let grid = menu_grid(screen_rect(rect(0, 0, 200, 60)));
        let card = grid.card_rect(index, app.menu_state.row_offset).unwrap();
        let rows = rendered_cells(app, 200, 60);
        (card.y + 1..card.y + 1 + MENU_ICON_ROWS)
            .flat_map(|y| (card.x + 1..card.right() - 1).map(move |x| (x, y)))
            .map(|(x, y)| rows[y as usize][x as usize].clone())
            .collect()
    }

    #[test]
    fn icons_are_drawn_above_the_card_text_when_images_are_available() {
        let mut app = app_on_menu_with_icons();
        assert!(!app.menu_icons.is_fallback());
        for (i, name) in MENU_ITEMS.iter().enumerate() {
            if PENDING_MENU_ICONS.contains(&MENU_ICON_PATHS[i]) {
                continue;
            }
            assert!(
                icon_cells(&mut app, i).iter().any(|c| c != " "),
                "{name}: アイコンが描かれる"
            );
        }
        // アイコンがあってもゲーム名はクリックで選べる
        let (x, y) = drawn_position_of(&mut app, MENU_ITEMS[HISTORY_ITEM_INDEX], 200, 60).unwrap();
        app.handle_mouse(left_click_at(x, y));
        assert!(matches!(app.screen, Screen::History));
    }

    #[test]
    fn moving_the_selection_does_not_move_or_redraw_the_icons() {
        // アイコン画像は使い回し、選択移動では枠だけが描き替わる(カードの位置が動かない)
        let mut app = app_on_menu_with_icons();
        let before: Vec<Vec<String>> = (0..MENU_ITEMS.len()).map(|i| icon_cells(&mut app, i)).collect();
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Down);
        let after: Vec<Vec<String>> = (0..MENU_ITEMS.len()).map(|i| icon_cells(&mut app, i)).collect();
        assert_eq!(before, after);
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

    // --- ゲーム開始前のカウントダウン ---

    use crate::ui::countdown::{Phase, PHASE_DURATION};

    /// カウントダウン全体の長さ(3/2/1/GO!!の4フェーズ)
    const COUNTDOWN_TOTAL: Duration = Duration::from_millis(2400);

    /// カウントダウンを最後まで進めてPlaying画面にする
    fn finish_countdown(app: &mut App) {
        assert!(
            matches!(app.screen, Screen::Countdown { .. }),
            "カウントダウン画面のはず"
        );
        app.update(COUNTDOWN_TOTAL);
        assert!(matches!(app.screen, Screen::Playing(_)), "Playing画面のはず");
    }

    /// DDR以外のゲームのメニュー項目一覧
    fn non_rhythm_game_items() -> impl Iterator<Item = usize> {
        (0..JUKEBOX_ITEM_INDEX).filter(|&item| item != RHYTHM_ITEM_INDEX)
    }

    /// 難易度選択画面を経由するゲームのメニュー項目一覧
    /// (DDR・ソコヌキ・カウントマニア・ハヤウチ・べー・記憶・イロピッタン以外)
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

    fn left_click(row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    #[test]
    fn countdown_total_matches_four_phases() {
        assert_eq!(PHASE_DURATION * 4, COUNTDOWN_TOTAL);
    }

    #[test]
    fn starting_any_non_rhythm_game_goes_through_countdown() {
        // ソコヌキ・カウントマニア・ハヤウチは難易度選択を経由しないので別のテストで確認する
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
            assert_eq!(state.phase(), Some(Phase::Three), "item={item}: 3から始まる");
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
    fn starting_rhythm_goes_straight_to_playing_without_countdown() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        let Screen::Playing(game) = &app.screen else {
            panic!("DDRはカウントダウンを挟まずPlaying画面になるはず");
        };
        assert_eq!(game.result().game_id, crate::game::rhythm::GAME_ID);
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
        // ソコヌキ・カウントマニア・ハヤウチは難易度選択を経由しないので別のテストで確認する
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
        assert_eq!(state.phase(), Some(Phase::Three), "キー入力でフェーズが進まないこと");

        finish_countdown(&mut app);
        let Screen::Playing(game) = &app.screen else {
            unreachable!();
        };
        assert!(!game.is_finished(), "カウントダウン中のキーがゲームに届いていないこと");
        assert_eq!(game.result().total, 0, "回答数0のまま");
    }

    #[test]
    fn mouse_clicks_during_countdown_are_ignored() {
        let mut app = App::new();
        // マウス専用ゲーム(難易度選択を挟まずカウントダウンに入る)
        app.select_menu_item(COUNT_MANIA_ITEM_INDEX);
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
        assert_eq!(game.result().game_id, crate::game::count_mania::GAME_ID);
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

    // --- [q]キーの終了フロー(メニュー以外→メニューへ戻る / メニュー→終了確認) ---

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::from(code));
    }

    fn is_menu_bgm(name: Option<&str>) -> bool {
        name.is_some_and(|n| audio::bgm_tracks_in(BgmCategory::Menu).iter().any(|t| t == n))
    }

    /// メニュー以外の各画面にしたAppを(画面の説明, App)で返す
    fn apps_on_every_non_menu_screen() -> Vec<(&'static str, App)> {
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
        assert!(!is_menu_bgm(app.current_bgm.as_deref()), "プレイ中はプレイ用BGM");
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
        assert!(matches!(app.screen, Screen::Menu), "中断後にゲームが始まらないこと");
        assert!(!app.should_quit());
    }

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

    /// メニューで[q]を押して終了確認ダイアログを開いたAppを作る
    fn app_on_confirm_quit() -> App {
        let mut app = App::new();
        app.screen = Screen::Menu;
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.screen, Screen::ConfirmQuit));
        app
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
            assert!(matches!(app.screen, Screen::Menu), "{code:?}でメニューに戻る");
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
            assert!(matches!(app.screen, Screen::ConfirmQuit), "{code:?}では閉じない");
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
        assert!(matches!(app.screen, Screen::ConfirmQuit), "背後のメニュー項目が選ばれないこと");
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
        assert!(text.contains(MENU_ITEMS[0]), "背景にメニュー項目が見えること");
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

    /// 描画結果(TestBackendに実際に出力されたセル)を行ごとの文字の並びで返す。
    /// 全角文字の2セル目は出力されないので、空白を詰めずセル単位で見る
    fn rendered_cells(app: &mut App, width: u16, height: u16) -> Vec<Vec<String>> {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect())
            .collect()
    }

    #[test]
    fn confirm_quit_dialog_border_is_not_hidden_by_wide_menu_text() {
        // 背景メニューの全角文字がダイアログの左端をまたいでいても、枠線が欠けないこと
        for width in 60u16..=100 {
            for height in [20u16, 24, 30] {
                let mut app = app_on_confirm_quit();
                // 背景のメニューのカードも角丸の枠なので、見出し「終了確認」の左にある角から辿る
                let (title_x, top) = drawn_position_of(&mut app, "終了確認", width, height)
                    .unwrap_or_else(|| panic!("{width}x{height}: ダイアログの見出しが描かれていること"));
                let (title_x, top) = (title_x as usize, top as usize);
                let rows = rendered_cells(&mut app, width, height);
                let left = (0..title_x)
                    .rev()
                    .find(|&x| rows[top][x] == "╭")
                    .unwrap_or_else(|| panic!("{width}x{height}: 左上の角が描かれていること"));
                let bottom = (top + 1..rows.len())
                    .find(|&y| rows[y][left] != "│")
                    .unwrap_or_else(|| panic!("{width}x{height}: 下端があること"));
                assert_eq!(rows[bottom][left], "╰", "{width}x{height}: 左辺が途切れないこと");
            }
        }
    }

    #[test]
    fn update_on_confirm_quit_keeps_the_dialog_open() {
        let mut app = app_on_confirm_quit();
        app.update(Duration::from_secs(10));
        assert!(matches!(app.screen, Screen::ConfirmQuit));
    }

    // --- タイプライター表示(リザルト・メニュー) ---

    use crate::ui::typewriter::{self, CHAR_INTERVAL};

    /// タイプライターを最後まで流し切るのに十分な時間
    const LONG_ENOUGH: Duration = Duration::from_secs(60);

    fn sample_result() -> GameResult {
        new_game(0, Difficulty::Beginner).result()
    }

    /// 空白を除いた描画テキスト(80x30)
    fn rendered_compact(app: &mut App) -> String {
        rendered_text(app).replace(' ', "")
    }

    /// タイトル画面からEnterでメニューへ入ったAppを作る(enter_menuを経由する)
    fn app_entering_menu() -> App {
        let mut app = App::new();
        app.last_area = rect(0, 0, 80, 30);
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.screen, Screen::Menu));
        app
    }

    /// リザルト画面へ入ったAppを作る(履歴ファイルへの保存は行わない)
    fn app_showing_result() -> App {
        let mut app = App::new();
        app.show_result(sample_result(), None);
        assert!(matches!(app.screen, Screen::Result(..)));
        app
    }

    #[test]
    fn result_screen_starts_empty_and_types_in_over_time() {
        let mut app = app_showing_result();
        assert_eq!(app.result_typewriter.visible_chars(), 0);
        assert!(!app.result_typewriter.is_finished());
        let before = rendered_compact(&mut app);
        assert!(before.contains("RESULT"), "枠と見出しは最初から出る");
        assert!(!before.contains("図形回"), "本文はまだ出ない");

        app.update(CHAR_INTERVAL * 3);
        assert_eq!(app.result_typewriter.visible_chars(), 3);
        let mid = rendered_compact(&mut app);
        assert!(mid.contains("図形回"), "先頭から3文字だけ出る");
        assert!(!mid.contains("図形回転"));
        assert!(!mid.contains("平均反応時間"), "後ろの行はまだ出ない");

        app.update(LONG_ENOUGH);
        assert!(app.result_typewriter.is_finished());
        let after = rendered_compact(&mut app);
        assert!(after.contains(MENU_ITEMS[0]));
        assert!(after.contains("RANK"));
        assert!(after.contains("平均反応時間"));
    }

    #[test]
    fn result_typewriter_total_matches_the_result_text() {
        let app = app_showing_result();
        let Screen::Result(result, _) = &app.screen else {
            unreachable!();
        };
        assert_eq!(
            app.result_typewriter.total_chars(),
            typewriter::char_count(&result_lines(result))
        );
    }

    #[test]
    fn showing_a_result_again_restarts_typing() {
        let mut app = app_showing_result();
        app.update(LONG_ENOUGH);
        assert!(app.result_typewriter.is_finished());
        app.show_result(sample_result(), None);
        assert_eq!(app.result_typewriter.visible_chars(), 0);
    }

    #[test]
    fn centered_result_lines_do_not_shift_while_typing() {
        // 中央寄せの行が、文字が増えるたびに左右へずれないこと(先頭の文字の位置が変わらない)
        let first_col = |app: &mut App| {
            let rows = rendered_cells(app, 80, 30);
            rows.iter()
                .find_map(|r| r.iter().position(|c| c == "図"))
                .expect("ゲーム名の先頭文字が描かれていること")
        };
        let mut app = app_showing_result();
        app.update(CHAR_INTERVAL);
        let early = first_col(&mut app);
        app.update(LONG_ENOUGH);
        assert_eq!(first_col(&mut app), early);
    }

    #[test]
    fn menu_starts_empty_and_types_items_from_the_top() {
        let mut app = app_entering_menu();
        assert_eq!(app.menu_typewriter.visible_chars(), 0);
        assert!(!app.menu_typewriter.is_finished());
        let before = rendered_compact(&mut app);
        assert!(before.contains("BRAINTRAIN"), "枠と見出しは最初から出る");
        assert!(!before.contains(MENU_ITEMS[0]), "項目はまだ出ない");

        // 1枚目のゲーム名「図形回転判定」の先頭2文字
        app.update(MENU_CHAR_INTERVAL * 2);
        let mid = rendered_compact(&mut app);
        assert!(mid.contains("図形"), "1枚目の名前が途中まで出る");
        assert!(!mid.contains(MENU_ITEMS[0]));
        assert!(!mid.contains(MENU_ITEMS[1]), "2枚目はまだ出ない");

        app.update(LONG_ENOUGH);
        assert!(app.menu_typewriter.is_finished());
        for i in 0..MENU_ITEMS.len() {
            let text = card_text(&mut app, i);
            assert!(text.contains(MENU_ITEMS[i]), "{}が出ること", MENU_ITEMS[i]);
            assert!(text.contains(MENU_DESCRIPTIONS[i]), "{}が出ること", MENU_DESCRIPTIONS[i]);
        }
    }

    /// 200x60で描画した時の、index番目のカードの中身(枠を除き空白を詰めた文字列)
    fn card_text(app: &mut App, index: usize) -> String {
        let rows = rendered_cells(app, 200, 60);
        let card = menu_grid(screen_rect(rect(0, 0, 200, 60)))
            .card_rect(index, app.menu_state.row_offset)
            .expect("カードが見えていること");
        (card.y + 1..card.bottom() - 1)
            .map(|y| rows[y as usize][card.x as usize + 1..card.right() as usize - 1].concat())
            .collect::<String>()
            .replace(' ', "")
    }

    #[test]
    fn menu_items_appear_in_card_order_from_top_left_to_bottom_right() {
        let mut app = app_entering_menu();
        let mut shown = 0;
        while !app.menu_typewriter.is_finished() {
            app.update(MENU_CHAR_INTERVAL * 5);
            let text = rendered_rows_without_spaces(&mut app, 200, 60).concat();
            let now = MENU_ITEMS.iter().take_while(|name| text.contains(*name)).count();
            assert!(
                MENU_ITEMS[now..].iter().all(|name| !text.contains(*name)),
                "上の項目より先に下の項目が出ないこと"
            );
            assert!(now >= shown, "一度出た項目は消えない");
            shown = now;
        }
        assert_eq!(shown, MENU_ITEMS.len());
    }

    #[test]
    fn menu_typing_total_follows_the_screen_size() {
        for (width, height) in [(80u16, 24u16), (80, 30), (120, 45), (200, 60), (20, 10)] {
            let mut app = App::new();
            app.last_area = rect(0, 0, width, height);
            press(&mut app, KeyCode::Enter);
            let expected =
                typewriter::char_count(&menu_item_lines(screen_rect(rect(0, 0, width, height))));
            assert_eq!(app.menu_typewriter.total_chars(), expected, "{width}x{height}");
            // 描画した画面サイズの内容に合わせて全文字数が更新される
            rendered_rows_without_spaces(&mut app, 80, 30);
            let resized = typewriter::char_count(&menu_item_lines(screen_rect(rect(0, 0, 80, 30))));
            assert_eq!(app.menu_typewriter.total_chars(), resized, "{width}x{height}→80x30");
        }
    }

    #[test]
    fn menu_item_lines_hold_every_name_and_description_in_card_order() {
        // 説明文はカード幅で折り返すが、文字を足したり削ったりはしない
        for width in [20u16, 80, 200] {
            let area = rect(0, 0, width, 30);
            let grid = menu_grid(area);
            let lines = menu_item_lines(area);
            assert_eq!(lines.len(), MENU_ITEMS.len() * grid.lines_per_card(), "width={width}");
            for (i, card) in lines.chunks(grid.lines_per_card()).enumerate() {
                let text: String = card.iter().map(|l| l.to_string()).collect();
                assert_eq!(
                    text,
                    format!("{}{}", MENU_ITEMS[i], MENU_DESCRIPTIONS[i]),
                    "width={width}"
                );
            }
        }
    }

    #[test]
    fn typed_menu_cards_are_where_clicks_select_them() {
        // 途中まで流れている間も、見えている項目名の位置をクリックするとその項目になる
        for (width, height) in [(80u16, 24u16), (200, 60)] {
            let mut app = App::new();
            app.last_area = rect(0, 0, width, height);
            press(&mut app, KeyCode::Enter);
            app.update(MENU_CHAR_INTERVAL * 120);
            let area = screen_rect(rect(0, 0, width, height));
            let visible: Vec<(usize, (u16, u16))> = (0..MENU_ITEMS.len())
                .filter_map(|i| drawn_position_of(&mut app, MENU_ITEMS[i], width, height).map(|p| (i, p)))
                .collect();
            assert!(!visible.is_empty(), "{width}x{height}: 見えている項目がある");
            assert!(!app.menu_typewriter.is_finished(), "{width}x{height}: まだ途中");
            for (i, (x, y)) in visible {
                assert_eq!(menu_card_at(area, app.menu_state.row_offset, x, y), Some(i));
            }
        }
    }

    #[test]
    fn key_while_menu_is_typing_finishes_it_and_still_moves_the_selection() {
        let mut app = app_entering_menu();
        press(&mut app, KeyCode::Down);
        assert!(app.menu_typewriter.is_finished(), "キー入力で全文字表示済みになる");
        assert_eq!(
            app.menu_state.selected(),
            menu_grid(screen_rect(app.last_area)).columns,
            "上下キーの選択移動もそのまま効く(1つ下の行へ)"
        );
        let text = rendered_compact(&mut app);
        assert!(text.contains(MENU_ITEMS[0]) && text.contains(MENU_ITEMS[1]));
    }

    #[test]
    fn menu_card_text_does_not_shift_while_typing() {
        // カードの文字は、1文字ずつ増えていく間も書き出しの位置が動かないこと
        let mut app = app_entering_menu();
        app.update(MENU_CHAR_INTERVAL);
        let early = drawn_position_of(&mut app, "図", 80, 30).expect("1文字目が出ている");
        app.update(LONG_ENOUGH);
        let done = drawn_position_of(&mut app, MENU_ITEMS[0], 80, 30).unwrap();
        assert_eq!(early, done);
    }

    #[test]
    fn right_key_while_menu_is_typing_finishes_it_and_moves_right() {
        let mut app = app_entering_menu();
        press(&mut app, KeyCode::Right);
        assert!(app.menu_typewriter.is_finished());
        assert_eq!(app.menu_state.selected(), 1);
    }

    #[test]
    fn enter_while_menu_is_typing_still_selects_the_item() {
        let mut app = app_entering_menu();
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.screen, Screen::SelectDifficulty(0, None)));
        assert!(app.menu_typewriter.is_finished());
    }

    #[test]
    fn click_while_menu_is_typing_finishes_it_and_still_selects_the_item() {
        let mut app = app_entering_menu();
        app.last_area = rect(0, 0, 200, 60);
        let (x, y) = app
            .last_area
            .positions()
            .map(|p| (p.x, p.y))
            .find(|&(x, y)| {
                menu_card_at(screen_rect(app.last_area), 0, x, y) == Some(HISTORY_ITEM_INDEX)
            })
            .unwrap();
        app.handle_mouse(left_click_at(x, y));
        assert!(matches!(app.screen, Screen::History));
        assert!(app.menu_typewriter.is_finished());
    }

    #[test]
    fn mouse_move_does_not_skip_the_menu_typing() {
        let mut app = app_entering_menu();
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 5,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert!(!app.menu_typewriter.is_finished());
    }

    #[test]
    fn q_while_menu_is_typing_shows_the_full_menu_behind_the_dialog() {
        let mut app = app_entering_menu();
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.screen, Screen::ConfirmQuit));
        assert!(app.menu_typewriter.is_finished());
        let text = rendered_rows_without_spaces(&mut app, 80, 30).concat();
        assert!(text.contains(MENU_ITEMS[0]));
    }

    #[test]
    fn other_key_while_result_is_typing_finishes_it_without_leaving() {
        let mut app = app_showing_result();
        press(&mut app, KeyCode::Char(' '));
        assert!(matches!(app.screen, Screen::Result(..)), "Enter/Esc以外では戻らない");
        assert!(app.result_typewriter.is_finished());
        assert!(rendered_compact(&mut app).contains("平均反応時間"));
    }

    #[test]
    fn enter_or_esc_while_result_is_typing_still_returns_to_menu() {
        for code in [KeyCode::Enter, KeyCode::Esc] {
            let mut app = app_showing_result();
            press(&mut app, code);
            assert!(matches!(app.screen, Screen::Menu), "{code:?}でメニューに戻る");
            assert!(
                !app.menu_typewriter.is_finished(),
                "{code:?}: 戻ったメニューは頭からタイプライターで流れる"
            );
        }
    }

    #[test]
    fn click_while_result_is_typing_still_returns_to_menu() {
        let mut app = app_showing_result();
        app.last_area = rect(0, 0, 80, 30);
        app.handle_mouse(left_click(5));
        assert!(matches!(app.screen, Screen::Menu));
    }

    #[test]
    fn returning_to_menu_restarts_the_menu_typing() {
        let mut app = app_entering_menu();
        app.update(LONG_ENOUGH);
        app.select_menu_item(HISTORY_ITEM_INDEX);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.screen, Screen::Menu));
        assert_eq!(app.menu_typewriter.visible_chars(), 0);
    }

    #[test]
    fn typewriters_advance_only_on_their_own_screen() {
        let mut app = app_showing_result();
        app.screen = Screen::History;
        app.update(CHAR_INTERVAL * 5);
        assert_eq!(app.result_typewriter.visible_chars(), 0, "リザルト以外では進まない");

        let mut app = app_entering_menu();
        app.screen = Screen::History;
        app.update(MENU_CHAR_INTERVAL * 5);
        assert_eq!(app.menu_typewriter.visible_chars(), 0, "メニュー以外では進まない");

        // 終了確認ダイアログの背後のメニューも、ダイアログ中は進めない
        let mut app = app_entering_menu();
        app.screen = Screen::ConfirmQuit;
        app.update(LONG_ENOUGH);
        assert_eq!(app.menu_typewriter.visible_chars(), 0);
    }

    #[test]
    fn menu_char_interval_types_the_whole_menu_in_a_few_seconds() {
        // 項目数が多いメニューは標準の間隔だと流し切るのに時間がかかりすぎるため、専用の間隔を使う
        let total = typewriter::char_count(&menu_item_lines(screen_rect(rect(0, 0, 80, 30))));
        let duration = MENU_CHAR_INTERVAL * total as u32;
        assert!(
            duration <= Duration::from_secs(6),
            "メニュー全体が{duration:?}で流れ切る(全{total}文字)"
        );
    }

    // --- 全画面共通の背景画像(画面全体に余白を持たせて中央配置) ---

    use crate::ui::background::BackgroundRenderer;
    use ratatui_image::picker::{Picker, ProtocolType};

    /// 端末に問い合わせないpicker(1セル10x20px)
    fn test_picker(protocol: ProtocolType) -> Picker {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(protocol);
        picker
    }

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
            assert!(screen.width < area.width && screen.height < area.height, "{width}x{height}: 一回り小さい");
            assert_eq!(screen.intersection(area), screen, "{width}x{height}: areaの内側");
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
        for (width, height) in [(0u16, 0u16), (1, 1), (4, 30), (80, 2), (8, 4), (0, 30), (80, 0)] {
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
                    assert!(screen.width > 0 && screen.height > 0, "{width}x{height}: 幅・高さが0にならない");
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
                            rows[position.y as usize][position.x as usize],
                            " ",
                            "{name} {width}x{height}: 余白({},{})には描かない",
                            position.x,
                            position.y
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
                assert_ne!(with[(x, y)], default, "{name}: 余白({x},{y})に背景が描かれる");
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
        let buffer = terminal.draw(|frame| app.render(frame)).unwrap().buffer.clone();

        assert!(buffer[(0, 0)].symbol().starts_with('\x1b'), "左上のセルに背景画像のデータ");
        for position in area.positions() {
            let cell = &buffer[position];
            if screen.contains(position) {
                assert!(!cell.skip, "screen_rectの内側({},{})は出力される", position.x, position.y);
            } else if position != ratatui::layout::Position::new(0, 0) {
                assert!(cell.skip, "余白({},{})は背景画像のみ", position.x, position.y);
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
            terminal.draw(|frame| app.render(frame)).unwrap().buffer.clone()
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
        assert_eq!(without, with, "SelectSong: 背景画像の有無で描画が変わらない");

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
        for (width, height) in [(1u16, 1u16), (2, 2), (5, 3), (9, 5), (10, 6), (20, 8), (80, 24)] {
            for (_, mut app) in apps_on_every_background_screen() {
                swap_background(&mut app, background);
                rendered_cells(&mut app, width, height);
                background = swap_background(&mut app, BackgroundRenderer::with_picker(None));
            }
        }
    }

    #[test]
    fn clicking_the_menu_margin_selects_nothing() {
        let (width, height) = (200u16, 60u16);
        let mut app = app_on_menu();
        rendered_cells(&mut app, width, height);
        let screen = screen_rect(rect(0, 0, width, height));
        for (x, y) in [(0, 0), (screen.x - 1, 10), (10, screen.y - 1), (width - 1, height - 1)] {
            app.handle_mouse(left_click_at(x, y));
            assert!(matches!(app.screen, Screen::Menu), "余白({x},{y})のクリックでは選ばない");
        }
    }

    #[test]
    fn menu_clicks_hit_the_cards_drawn_inside_screen_rect() {
        // 余白の分だけ内側にずれたカードの位置をクリックすると、その項目になる
        let (width, height) = (200u16, 60u16);
        let grid = menu_grid(screen_rect(rect(0, 0, width, height)));
        let card = grid.card_rect(HISTORY_ITEM_INDEX, 0).unwrap();
        let mut app = app_on_menu();
        rendered_cells(&mut app, width, height);
        app.handle_mouse(left_click_at(card.x + 1, card.y + 1));
        assert!(matches!(app.screen, Screen::History));
    }

    #[test]
    fn difficulty_margin_row_selects_nothing() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        app.last_area = rect(0, 0, 80, 24);
        // 余白の分だけずれる前の(area基準の)初級の行は、screen_rect基準では見出しの行になる
        let area_based_beginner = Block::default().borders(Borders::ALL).inner(app.last_area).y
            + DIFFICULTY_ROWS_OFFSET;
        app.handle_mouse(left_click(0));
        app.handle_mouse(left_click(area_based_beginner));
        assert!(matches!(app.screen, Screen::SelectDifficulty(0, None)));
    }

    // --- リザルト画面のキャラクターのコマ送りアニメーション ---

    use crate::ui::result_sprite::{self, ResultSprite};

    /// キャラクターを描けるResultSprite(ハーフブロック描画)
    fn halfblocks_sprite() -> ResultSprite {
        ResultSprite::with_picker(Some(test_picker(ProtocolType::Halfblocks)))
    }

    /// appをwidth x heightで描いたバッファ
    fn rendered_buffer(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap().buffer.clone()
    }

    /// width x heightの画面に描いたリザルト画面のテキストカードの範囲(render_resultと同じ計算)
    fn result_card_for(app: &App, width: u16, height: u16) -> Rect {
        let Screen::Result(result, _) = &app.screen else {
            unreachable!();
        };
        let inner = theme::panel("").inner(screen_rect(rect(0, 0, width, height)));
        result_card_rect(inner, result_lines(result).len() as u16)
    }

    #[test]
    fn result_sprite_advances_only_on_the_result_screen() {
        let mut app = app_showing_result();
        assert_eq!(app.result_sprite.elapsed(), Duration::ZERO);
        app.update(result_sprite::FRAME_DURATION);
        assert_eq!(app.result_sprite.elapsed(), result_sprite::FRAME_DURATION);
        assert_eq!(app.result_sprite.current_frame(), 1, "リザルト画面ではコマが進む");

        for screen in [Screen::History, Screen::Menu, Screen::ConfirmQuit, Screen::Splash] {
            let mut app = app_showing_result();
            app.screen = screen;
            app.update(result_sprite::FRAME_DURATION * 3);
            assert_eq!(app.result_sprite.elapsed(), Duration::ZERO, "リザルト以外では進まない");
        }
    }

    #[test]
    fn show_result_restarts_the_sprite_animation() {
        let mut app = app_showing_result();
        app.update(result_sprite::FRAME_DURATION * 2);
        assert_eq!(app.result_sprite.current_frame(), 2);
        app.show_result(sample_result(), None);
        assert_eq!(app.result_sprite.elapsed(), Duration::ZERO, "入るたびに最初のコマから");
        assert_eq!(app.result_sprite.current_frame(), 0);
    }

    #[test]
    fn wide_result_screen_draws_the_sprite_left_of_the_card_without_touching_the_text() {
        let (width, height) = (120u16, 40u16);
        let mut app = app_showing_result();
        app.update(LONG_ENOUGH); // テキストを全部出しておく
        let without = rendered_buffer(&mut app, width, height);
        app.result_sprite = halfblocks_sprite();
        let with = rendered_buffer(&mut app, width, height);

        let card = result_card_for(&app, width, height);
        let changed: Vec<_> = with.area.positions().filter(|p| with[*p] != without[*p]).collect();
        assert!(!changed.is_empty(), "広い画面ではキャラクターが描かれる");
        let inner = theme::panel("").inner(screen_rect(rect(0, 0, width, height)));
        for p in &changed {
            assert!(p.x < card.x, "({},{}): カードの左側だけに描く", p.x, p.y);
            assert!(inner.contains(*p), "({},{}): 外枠の内側に描く", p.x, p.y);
        }
        for p in card.positions() {
            assert_eq!(with[p], without[p], "カード({},{})はキャラクターの有無で変わらない", p.x, p.y);
        }
    }

    #[test]
    fn narrow_result_screen_omits_the_sprite_and_keeps_the_card_intact() {
        for (width, height) in [(80u16, 30u16), (80, 24), (60, 30), (120, 11)] {
            let mut app = app_showing_result();
            app.update(LONG_ENOUGH);
            let without = rendered_buffer(&mut app, width, height);
            app.result_sprite = halfblocks_sprite();
            let with = rendered_buffer(&mut app, width, height);
            assert_eq!(with, without, "{width}x{height}: 余白が狭ければキャラクターを描かない");
            let text = rendered_compact(&mut app);
            assert!(text.contains("平均反応時間"), "{width}x{height}: テキストは読める");
        }
    }

    #[test]
    fn result_screen_with_sprite_renders_without_panicking_at_any_size() {
        let mut sprite = halfblocks_sprite();
        for (width, height) in [(1u16, 1u16), (5, 3), (10, 6), (20, 8), (80, 24), (100, 30), (200, 60)] {
            for save_error in [None, Some("保存エラー".to_string())] {
                let mut app = app_showing_result();
                app.screen = Screen::Result(sample_result(), save_error);
                std::mem::swap(&mut app.result_sprite, &mut sprite);
                rendered_cells(&mut app, width, height);
                std::mem::swap(&mut app.result_sprite, &mut sprite);
            }
        }
    }

    #[test]
    fn result_screen_with_an_image_protocol_sprite_still_shows_the_text() {
        // 画像プロトコルのキャラクターはカードと重ならないので、カードの文字は端末へ出力される
        let (width, height) = (120u16, 40u16);
        let mut app = app_showing_result();
        app.update(LONG_ENOUGH);
        app.result_sprite = ResultSprite::with_picker(Some(test_picker(ProtocolType::Iterm2)));
        let buffer = rendered_buffer(&mut app, width, height);
        let card = result_card_for(&app, width, height);
        assert!(card.positions().all(|p| !buffer[p].skip), "カードのセルは出力される");
        assert!(
            buffer.area.positions().any(|p| buffer[p].symbol().starts_with('\x1b')),
            "キャラクターの画像データが入る"
        );
    }

    #[test]
    fn menu_keys_and_typing_use_the_grid_inside_screen_rect() {
        // キー操作の列数・タイプライターの全文字数も、描画と同じscreen_rect基準にそろえる
        let area = rect(0, 0, 200, 60);
        assert_ne!(
            menu_grid(area).columns,
            menu_grid(screen_rect(area)).columns,
            "この大きさでは余白の有無で列数が変わる(テストの前提)"
        );
        let mut app = App::new();
        app.last_area = area;
        press(&mut app, KeyCode::Enter); // Splash -> Menu
        assert_eq!(
            app.menu_typewriter.total_chars(),
            typewriter::char_count(&menu_item_lines(screen_rect(area)))
        );
        press(&mut app, KeyCode::Down);
        assert_eq!(app.menu_state.selected(), menu_grid(screen_rect(area)).columns);
    }
}
