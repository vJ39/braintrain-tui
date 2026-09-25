use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::audio::{self, BgmCategory, SeKind};
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
use crate::game::row_index;
use crate::game::sequence::SequenceGame;
use crate::game::shape_rotate::ShapeRotateGame;
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult};
use crate::stats::store;
use crate::ui::countdown::{self, CountdownState};
use crate::ui::splash::{self, SplashRenderer};
use crate::ui::typewriter::{self, Typewriter};

const MENU_ITEMS: [&str; 14] = [
    "図形回転判定",
    "鏡像判定",
    "イロピッタン",
    "暗算スピード",
    "パターン補完",
    "記憶(位置と色)",
    "数列予測",
    "組み合わせパズル",
    "カウントマニア",
    "カラーストック",
    "TTR",
    "反射神経",
    "ジュークボックス",
    "履歴",
];

/// カウントマニア(マウス専用)
const COUNT_MANIA_ITEM_INDEX: usize = 8;
/// カラーストック
const COLOR_STACK_ITEM_INDEX: usize = 9;
/// リズムゲームだけは難易度選択の前に曲選択を挟む
const RHYTHM_ITEM_INDEX: usize = 10;
/// 反射神経
const QUICK_DRAW_ITEM_INDEX: usize = 11;
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
    "BGMを選んで聴く",
    "ゲームごとの反応時間の推移を見る",
];

/// メニュー画面のタイプライター表示の1文字あたりの間隔。メニューは全項目で400文字以上あり、
/// 標準の間隔(typewriter::CHAR_INTERVAL)では流し切るのに10秒以上かかるため短くする
const MENU_CHAR_INTERVAL: Duration = Duration::from_millis(10);

pub enum Screen {
    /// 起動直後のタイトル画面。Enterを押すとMenuへ進む
    Splash,
    Menu,
    /// メニューでTTR(リズムゲーム)を選んだ直後のスプラッシュ画面。
    /// Enter/クリックで曲選択(SelectSong)へ進む
    RhythmSplash,
    /// リズムゲームの曲選択(選択中の曲 = SONGSのインデックス)
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
    menu_state: ListState,
    should_quit: bool,
    /// 直近のrender()で描画したフルスクリーンのエリア。マウス座標からのヒット
    /// テストに使う(render()より前にhandle_mouseが呼ばれることは無い前提)
    last_area: Rect,
    /// 現在再生中のBGMトラック名(ジュークボックス画面のハイライト表示に使う)
    current_bgm: Option<String>,
    splash_renderer: SplashRenderer,
    /// TTRスプラッシュ画面(Screen::RhythmSplash)用
    ttr_splash_renderer: SplashRenderer,
    /// メニューへ戻った直後にtrueになる。main.rsがtake_pending_scrollback_clear()で
    /// 検知して端末のスクロールバッファをクリアする(画像プロトコルの残留対策)
    pending_scrollback_clear: bool,
    /// メニュー画面(各項目の名前・説明文)のタイプライター表示。enter_menuでリセットする
    menu_typewriter: Typewriter,
    /// リザルト画面のタイプライター表示。show_resultでリセットする
    result_typewriter: Typewriter,
}

impl App {
    pub fn new() -> Self {
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));
        let current_bgm = audio::random_bgm_track(BgmCategory::Menu);
        if let Some(name) = &current_bgm {
            audio::play_bgm_track(name);
        }
        Self {
            screen: Screen::Splash,
            menu_state,
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
        }
    }

    /// メニュー画面へ遷移する。既存の`self.screen = Screen::Menu`は全てこれに統一し、
    /// メニューに戻るたびに端末側でのスクロールバッファのクリアを要求する。
    /// 項目の名前・説明文はここから改めてタイプライターで流す
    fn enter_menu(&mut self) {
        self.screen = Screen::Menu;
        self.pending_scrollback_clear = true;
        let total = typewriter::char_count(&menu_item_lines(self.last_area));
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
            Screen::RhythmSplash => {
                if matches!(key.code, KeyCode::Enter) {
                    self.leave_rhythm_splash();
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
        match &mut self.screen {
            Screen::Splash => {
                self.leave_splash();
            }
            Screen::RhythmSplash => {
                self.leave_rhythm_splash();
            }
            Screen::Menu => {
                if let Some(index) = menu_item_at_row(area, mouse.row) {
                    self.menu_state.select(Some(index));
                    self.select_menu_item(index);
                }
            }
            Screen::SelectSong(_) => {
                if let Some(song) = song_at_row(area, mouse.row) {
                    self.select_song(song);
                }
            }
            Screen::SelectDifficulty(item, song) => {
                let (item, song) = (*item, *song);
                if let Some(difficulty) = difficulty_at_row(area, mouse.row) {
                    self.start_playing(item, difficulty, song);
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

    /// TTRスプラッシュ画面から曲選択へ進む(Enter/クリック共通)
    fn leave_rhythm_splash(&mut self) {
        audio::play_se(SeKind::Confirm);
        self.screen = Screen::SelectSong(0);
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

    /// リザルト画面を表示し、結果の本文をタイプライターで流し始める(履歴への保存はしない)
    fn show_result(&mut self, result: GameResult, save_error: Option<String>) {
        self.result_typewriter = Typewriter::new(typewriter::char_count(&result_lines(&result)));
        self.screen = Screen::Result(result, save_error);
    }

    /// Menu画面での項目決定(キー/クリック共通)。ゲーム/ジュークボックス/履歴へ振り分ける
    fn select_menu_item(&mut self, selected: usize) {
        if selected == COLOR_STACK_ITEM_INDEX {
            // カラーストックは難易度選択を挟まず、すぐにカウントダウンへ進む
            // (カウントダウン最初の「3」の音が画面遷移の音を兼ねる)
            self.start_playing(
                COLOR_STACK_ITEM_INDEX,
                crate::game::color_stack::SESSION_DIFFICULTY,
                None,
            );
            return;
        }
        audio::play_se(SeKind::Transition);
        if selected == HISTORY_ITEM_INDEX {
            self.screen = Screen::History;
        } else if selected == JUKEBOX_ITEM_INDEX {
            self.screen = Screen::Jukebox(self.jukebox_list_state());
        } else if selected == RHYTHM_ITEM_INDEX {
            // 曲選択の前にTTR専用のスプラッシュ画面を挟む。BGMもTTR専用のものに切り替え、
            // 実際に曲を選んでプレイが始まるまで(start_rhythmで曲のBGMに切り替わるまで)流し続ける
            if let Some(name) = audio::random_bgm_track(BgmCategory::RhythmSplash) {
                audio::play_bgm_track(&name);
                self.current_bgm = Some(name);
            }
            self.screen = Screen::RhythmSplash;
        } else {
            self.screen = Screen::SelectDifficulty(selected, None);
        }
    }

    /// 曲選択画面での曲決定(キー/クリック共通)。その曲の難易度選択へ進む
    fn select_song(&mut self, song: usize) {
        audio::play_se(SeKind::Transition);
        self.screen = Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(song));
    }

    /// 難易度決定後、指定ゲームを開始する(キー/クリック共通)。リズムゲームはすぐに
    /// プレイを始め、それ以外はPlaying用BGMに切り替えてカウントダウンを挟む
    fn start_playing(&mut self, item: usize, difficulty: Difficulty, song: Option<usize>) {
        if item == RHYTHM_ITEM_INDEX {
            audio::play_se(SeKind::Transition);
            self.start_rhythm(song.unwrap_or(0), difficulty);
            return;
        }
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

    /// リズムゲームを開始する。譜面生成を先に済ませ、選んだ曲のBGM再生を始めた直後に
    /// ゲーム内時計を合わせることで、曲と譜面(実測ビート時刻)の時間基準を揃える
    fn start_rhythm(&mut self, song: usize, difficulty: Difficulty) {
        let mut game = RhythmGame::new(difficulty, song);
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

    fn handle_menu_key(&mut self, key: KeyEvent) {
        let len = MENU_ITEMS.len();
        let selected = self.menu_state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Up => {
                self.menu_state.select(Some((selected + len - 1) % len));
            }
            KeyCode::Down => {
                self.menu_state.select(Some((selected + 1) % len));
            }
            KeyCode::Enter => self.select_menu_item(selected),
            _ => {}
        }
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
            self.start_playing(item, difficulty, song);
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
            Screen::Result(..) => self.result_typewriter.tick(dt),
            _ => {}
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.last_area = area;
        let current_bgm = self.current_bgm.clone();
        match &mut self.screen {
            Screen::Splash => self.splash_renderer.render(frame, area),
            Screen::RhythmSplash => self.ttr_splash_renderer.render(frame, area),
            Screen::Menu => {
                render_menu(frame, area, &mut self.menu_state, &mut self.menu_typewriter)
            }
            Screen::SelectSong(selected) => render_song_select(frame, area, *selected),
            Screen::SelectDifficulty(item, song) => {
                let title = match song.and_then(|s| SONGS.get(s)) {
                    Some(song) => format!("{} / {}", MENU_ITEMS[*item], song.display_name),
                    None => MENU_ITEMS[*item].to_string(),
                };
                render_difficulty_select(frame, area, &title)
            }
            Screen::Countdown { state, .. } => countdown::render(frame, area, state),
            Screen::Playing(game) => game.render(frame, area),
            Screen::Result(result, save_error) => render_result(
                frame,
                area,
                result,
                save_error.as_deref(),
                &mut self.result_typewriter,
            ),
            Screen::History => render_history(frame, area),
            Screen::Jukebox(state) => render_jukebox(frame, area, state, current_bgm.as_deref()),
            Screen::ConfirmQuit => {
                render_menu(frame, area, &mut self.menu_state, &mut self.menu_typewriter);
                render_confirm_quit(frame, area);
            }
        }
    }
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
        2 => Box::new(ReactionGame::new(difficulty)),
        3 => Box::new(MentalCalcGame::new(difficulty)),
        4 => Box::new(PatternFillGame::new(difficulty)),
        5 => Box::new(MemoryGame::new(difficulty)),
        6 => Box::new(SequenceGame::new(difficulty)),
        7 => Box::new(PuzzleConnectGame::new(difficulty)),
        COUNT_MANIA_ITEM_INDEX => Box::new(CountManiaGame::new(difficulty)),
        // カラーストックは難易度を持たず、ROUND1〜3が固定の内容で進む
        COLOR_STACK_ITEM_INDEX => Box::new(ColorStackGame::new()),
        RHYTHM_ITEM_INDEX => unreachable!("rhythm is started via start_rhythm with a song"),
        QUICK_DRAW_ITEM_INDEX => Box::new(QuickDrawGame::new(difficulty)),
        _ => unreachable!("history is handled without creating a game"),
    }
}

/// SelectSong画面で、曲の行が内部エリアの何行目から始まるか。
/// render_song_selectとsong_at_rowで一致させること
const SONG_ROWS_OFFSET: u16 = 2;

fn render_song_select(frame: &mut Frame, area: Rect, selected: usize) {
    // 行の並びはsong_at_rowと一致させる(1行目=見出し、2行目=空行、3行目以降=曲)
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
    let block = theme::panel(" ◆ BRAIN TRAIN ◆ ").title_bottom(
        theme::hints_line(&[
            ("↑↓", "選択"),
            ("Enter / 数字", "決定"),
            ("Esc", "戻る"),
        ])
        .centered(),
    );
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(block);
    frame.render_widget(paragraph, area);
}

/// SelectSong画面でのクリック行(area基準、Block枠含む)から曲インデックスを求める
fn song_at_row(area: Rect, mouse_row: u16) -> Option<usize> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
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
                ("↑↓", "選択"),
                ("Enter / クリック", "決定"),
                ("q", "終了"),
            ])
            .centered(),
        )
}

/// メニュー1項目あたりの行数。menu_item_at_rowは枠の内側を項目数で等分(row_index)して
/// 判定するので、1項目の高さもその等分に合わせ、見た目の位置とクリック位置をそろえる
fn menu_item_height(area: Rect) -> usize {
    let inner = menu_block().inner(area);
    (inner.height / MENU_ITEMS.len() as u16).max(1) as usize
}

/// メニュー全項目の行を上から順に並べて返す(1項目ちょうどmenu_item_height(area)行ずつ)。
/// タイプライター表示はこの並びの先頭から1文字ずつ流す
fn menu_item_lines(area: Rect) -> Vec<Line<'static>> {
    let item_height = menu_item_height(area);
    MENU_ITEMS
        .iter()
        .zip(MENU_DESCRIPTIONS)
        .enumerate()
        .flat_map(|(i, (name, description))| {
            let number = Span::styled(format!("{:02}  ", i + 1), Style::default().fg(theme::ACCENT));
            let name = Span::styled(
                *name,
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            );
            let description_style = Style::default().fg(theme::MUTED);
            let content: Vec<Line> = if item_height >= 2 {
                vec![
                    Line::from(vec![number, name]),
                    Line::from(Span::styled(format!("    {description}"), description_style)),
                ]
            } else {
                vec![Line::from(vec![
                    number,
                    name,
                    Span::styled(format!("   {description}"), description_style),
                ])]
            };
            // 帯の中で縦中央に置き、残りは空行で埋めて帯の高さちょうどにする
            let top = (item_height - content.len()) / 2;
            let mut lines = vec![Line::from(""); top];
            lines.extend(content);
            lines.resize(item_height, Line::from(""));
            lines
        })
        .collect()
}

fn render_menu(frame: &mut Frame, area: Rect, state: &mut ListState, typing: &mut Typewriter) {
    let block = menu_block();
    let inner = block.inner(area);
    let item_height = menu_item_height(area);

    // 画面サイズによって1項目の行数(=文字数)が変わるので、描画する内容に全文字数を合わせる
    let lines = menu_item_lines(area);
    typing.set_total_chars(typewriter::char_count(&lines));
    let lines = typewriter::truncate_lines(&lines, typing.visible_chars());
    let items: Vec<ListItem> = lines
        .chunks(item_height)
        .map(|item| ListItem::new(item.to_vec()))
        .collect();

    // 全項目が収まる大きさならスクロールさせない(以前の小さい画面での位置が残らないように)
    if inner.height as usize >= item_height * MENU_ITEMS.len() {
        *state.offset_mut() = 0;
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected_style())
        .highlight_symbol(" ▶ ");
    frame.render_stateful_widget(list, area, state);
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

/// Menu画面でのクリック行(area基準、Block枠含む)からメニュー項目インデックスを求める
fn menu_item_at_row(area: Rect, mouse_row: u16) -> Option<usize> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    row_index(inner, mouse_row, MENU_ITEMS.len() as u16)
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

fn render_result(
    frame: &mut Frame,
    area: Rect,
    result: &GameResult,
    save_error: Option<&str>,
    typing: &mut Typewriter,
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
    let card = centered_rect(inner, inner.width.min(48), (content_height + 2).min(inner.height));
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
        let game = new_game(COUNT_MANIA_ITEM_INDEX, Difficulty::Advanced);
        let result = game.result();
        assert_eq!(result.game_id, crate::game::count_mania::GAME_ID);
        assert_eq!(result.difficulty, Difficulty::Advanced);
    }

    #[test]
    fn selecting_count_mania_goes_to_difficulty_and_starts_it() {
        let mut app = App::new();
        app.select_menu_item(COUNT_MANIA_ITEM_INDEX);
        assert!(matches!(
            app.screen,
            Screen::SelectDifficulty(COUNT_MANIA_ITEM_INDEX, None)
        ));
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
        finish_countdown(&mut app);
        let Screen::Playing(game) = &app.screen else {
            panic!("Playing画面のはず");
        };
        assert_eq!(game.result().game_id, crate::game::count_mania::GAME_ID);
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        assert!(rendered_text(&mut app).replace(' ', "").contains("マウス専用"));
    }

    // --- カラーストック ---

    #[test]
    fn color_stack_is_the_last_game_before_rhythm() {
        assert_eq!(MENU_ITEMS[COLOR_STACK_ITEM_INDEX], "カラーストック");
        assert_eq!(COLOR_STACK_ITEM_INDEX + 1, RHYTHM_ITEM_INDEX);
    }

    #[test]
    fn new_game_for_color_stack_item_creates_color_stack() {
        // カラーストックは難易度を持たないので、渡した難易度によらず固定の記録になる
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

    /// カラーストックが始まり、ROUND1(4列)が表示されていることを確かめる
    fn assert_color_stack_round1_is_playing(app: &mut App) {
        finish_countdown(app);
        let Screen::Playing(game) = &app.screen else {
            panic!("Playing画面のはず");
        };
        assert_eq!(game.result().game_id, crate::game::color_stack::GAME_ID);
        assert!(!game.is_finished());
        // 全角文字の2セル目は空白で埋まるため、空白を除いて比較する
        let text = rendered_text(app).replace(' ', "");
        assert!(text.contains("カラーストック"));
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
        app.menu_state.select(Some(COLOR_STACK_ITEM_INDEX));
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
        let height = 30;
        app.last_area = rect(0, 0, 80, height);
        let rows = rendered_rows_without_spaces(&mut app, 80, height);
        let row = rows
            .iter()
            .position(|r| r.contains(MENU_ITEMS[COLOR_STACK_ITEM_INDEX]))
            .expect("カラーストックが描かれていること");
        app.handle_mouse(left_click(row as u16));
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

    #[test]
    fn menu_item_at_row_maps_each_row_to_its_index() {
        // 上下の枠線(2行)+項目数ぶんの高さを持つエリア
        let height = MENU_ITEMS.len() as u16 + 2;
        let area = rect(0, 0, 20, height);
        for (offset, expected) in (0..MENU_ITEMS.len()).enumerate() {
            let row = 1 + offset as u16;
            assert_eq!(menu_item_at_row(area, row), Some(expected));
        }
    }

    #[test]
    fn menu_item_at_row_on_border_is_none() {
        let height = MENU_ITEMS.len() as u16 + 2;
        let area = rect(0, 0, 20, height);
        assert_eq!(menu_item_at_row(area, 0), None, "上端の枠線上はNone");
        assert_eq!(
            menu_item_at_row(area, height - 1),
            None,
            "下端の枠線上はNone"
        );
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

    // --- リズムゲーム: Menu → 曲選択 → 難易度選択 → プレイ ---

    #[test]
    fn rhythm_item_index_points_at_rhythm_menu_item() {
        assert_eq!(MENU_ITEMS[RHYTHM_ITEM_INDEX], "TTR");
    }

    // --- 反射神経 ---

    #[test]
    fn quick_draw_comes_right_after_ttr_and_before_jukebox() {
        assert_eq!(MENU_ITEMS[QUICK_DRAW_ITEM_INDEX], "反射神経");
        assert_eq!(RHYTHM_ITEM_INDEX + 1, QUICK_DRAW_ITEM_INDEX);
        assert_eq!(QUICK_DRAW_ITEM_INDEX + 1, JUKEBOX_ITEM_INDEX);
        // 先頭側の既存インデックスはずれない
        assert_eq!(MENU_ITEMS[COUNT_MANIA_ITEM_INDEX], "カウントマニア");
        assert_eq!(MENU_ITEMS[COLOR_STACK_ITEM_INDEX], "カラーストック");
    }

    #[test]
    fn new_game_for_quick_draw_item_creates_quick_draw() {
        let game = new_game(QUICK_DRAW_ITEM_INDEX, Difficulty::Advanced);
        let result = game.result();
        assert_eq!(result.game_id, crate::game::quick_draw::GAME_ID);
        assert_eq!(result.difficulty, Difficulty::Advanced);
    }

    #[test]
    fn selecting_quick_draw_goes_to_difficulty_then_countdown_then_playing() {
        let mut app = App::new();
        app.select_menu_item(QUICK_DRAW_ITEM_INDEX);
        assert!(matches!(
            app.screen,
            Screen::SelectDifficulty(QUICK_DRAW_ITEM_INDEX, None)
        ));
        app.handle_key(KeyEvent::from(KeyCode::Char('3')));
        assert!(
            matches!(
                app.screen,
                Screen::Countdown {
                    item: QUICK_DRAW_ITEM_INDEX,
                    difficulty: Difficulty::Advanced,
                    ..
                }
            ),
            "反射神経も通常のカウントダウンを経由する"
        );
        finish_countdown(&mut app);
        let Screen::Playing(game) = &app.screen else {
            panic!("Playing画面のはず");
        };
        assert_eq!(game.result().game_id, crate::game::quick_draw::GAME_ID);
        assert!(rendered_text(&mut app).replace(' ', "").contains("まだ待て"));
    }

    #[test]
    fn result_screen_shows_quick_draw_menu_name() {
        let mut app = App::new();
        app.screen = Screen::Result(
            new_game(QUICK_DRAW_ITEM_INDEX, Difficulty::Beginner).result(),
            None,
        );
        assert!(rendered_text(&mut app).replace(' ', "").contains("反射神経"));
    }

    #[test]
    fn selecting_rhythm_menu_item_enters_ttr_splash_first() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        assert!(
            matches!(app.screen, Screen::RhythmSplash),
            "曲選択に直接進まず、TTRスプラッシュ画面を挟む"
        );
    }

    #[test]
    fn selecting_rhythm_menu_item_starts_the_ttr_splash_bgm() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        assert_eq!(app.current_bgm.as_deref(), Some("Overclocked_Tempo"));
    }

    #[test]
    fn ttr_splash_bgm_keeps_playing_through_song_select() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(app.screen, Screen::SelectSong(0)));
        assert_eq!(
            app.current_bgm.as_deref(),
            Some("Overclocked_Tempo"),
            "曲選択画面でもTTR専用BGMのまま"
        );
    }

    #[test]
    fn ttr_splash_bgm_keeps_playing_through_difficulty_select() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        app.select_song(1);
        assert!(matches!(
            app.screen,
            Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1))
        ));
        assert_eq!(app.current_bgm.as_deref(), Some("Overclocked_Tempo"));
    }

    #[test]
    fn starting_the_song_switches_from_ttr_splash_bgm_to_the_song() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        app.select_song(1);
        app.start_playing(RHYTHM_ITEM_INDEX, Difficulty::Beginner, Some(1));
        assert_eq!(app.current_bgm.as_deref(), Some(SONGS[1].track_name));
    }

    #[test]
    fn clicking_rhythm_menu_item_enters_ttr_splash_first() {
        let mut app = App::new();
        app.screen = Screen::Menu;
        let area = rect(0, 0, 80, 30);
        app.last_area = area;
        let row = (0..area.height)
            .find(|&row| menu_item_at_row(area, row) == Some(RHYTHM_ITEM_INDEX))
            .expect("TTR項目の行があるはず");
        app.handle_mouse(left_click(row));
        assert!(matches!(app.screen, Screen::RhythmSplash));
    }

    #[test]
    fn ttr_splash_enter_key_goes_to_song_select() {
        let mut app = App::new();
        app.screen = Screen::RhythmSplash;
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(app.screen, Screen::SelectSong(0)));
    }

    #[test]
    fn ttr_splash_click_goes_to_song_select() {
        let mut app = App::new();
        app.screen = Screen::RhythmSplash;
        app.last_area = rect(0, 0, 40, 12);
        app.handle_mouse(left_click(5));
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
    fn ttr_splash_other_key_stays_on_ttr_splash() {
        let mut app = App::new();
        app.screen = Screen::RhythmSplash;
        app.handle_key(KeyEvent::from(KeyCode::Down));
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
        assert!(matches!(app.screen, Screen::RhythmSplash));
    }

    #[test]
    fn ttr_splash_q_returns_to_menu() {
        let mut app = App::new();
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.screen, Screen::Menu));
        assert!(!app.should_quit());
    }

    #[test]
    fn ttr_splash_renders_without_panicking() {
        let mut app = App::new();
        app.screen = Screen::RhythmSplash;
        rendered_text(&mut app);
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

    #[test]
    fn song_select_enter_goes_to_difficulty_with_selected_song() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(1);
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(
            app.screen,
            Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1))
        ));
    }

    #[test]
    fn song_select_number_key_picks_song_directly() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.handle_key(KeyEvent::from(KeyCode::Char('2')));
        assert!(matches!(
            app.screen,
            Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1))
        ));
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
    fn rhythm_difficulty_esc_returns_to_song_select() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1));
        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::SelectSong(1)));
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
        app.select_menu_item(RHYTHM_ITEM_INDEX);
        app.handle_key(KeyEvent::from(KeyCode::Enter)); // TTRスプラッシュ -> 曲選択
        app.handle_key(KeyEvent::from(KeyCode::Down));
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        app.handle_key(KeyEvent::from(KeyCode::Char('3')));
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
    fn rhythm_difficulty_screen_shows_selected_song_name() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1));
        assert!(rendered_text(&mut app).contains(SONGS[1].display_name));
    }

    #[test]
    fn song_at_row_maps_each_song_row() {
        let area = rect(0, 0, 40, 12);
        let inner_top = Block::default().borders(Borders::ALL).inner(area).y;
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
    fn clicking_song_row_goes_to_difficulty_with_that_song() {
        let mut app = App::new();
        app.screen = Screen::SelectSong(0);
        app.last_area = rect(0, 0, 40, 12);
        let inner_top = Block::default()
            .borders(Borders::ALL)
            .inner(app.last_area)
            .y;
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: inner_top + SONG_ROWS_OFFSET + 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert!(matches!(
            app.screen,
            Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1))
        ));
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

    #[test]
    fn menu_items_are_drawn_on_the_rows_that_click_to_them() {
        // 画面の高さによって1項目の高さが変わっても、項目名が見えている行をクリックすればその項目になること
        for height in [24u16, 30, 45] {
            let mut app = App::new();
            app.screen = Screen::Menu;
            let rows = rendered_rows_without_spaces(&mut app, 80, height);
            let area = rect(0, 0, 80, height);
            for (i, name) in MENU_ITEMS.iter().enumerate() {
                let row = rows
                    .iter()
                    .position(|r| r.contains(name))
                    .unwrap_or_else(|| panic!("height={height}: {name}が描かれていること"));
                assert_eq!(
                    menu_item_at_row(area, row as u16),
                    Some(i),
                    "height={height}: {name}の行"
                );
            }
        }
    }

    #[test]
    fn difficulty_rows_are_drawn_where_difficulty_at_row_expects() {
        let mut app = App::new();
        app.screen = Screen::SelectDifficulty(0, None);
        let rows = rendered_rows_without_spaces(&mut app, 80, 24);
        let area = rect(0, 0, 80, 24);
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

    /// 難易度選択画面を経由するゲームのメニュー項目一覧(DDRとカラーストック以外)
    fn difficulty_select_game_items() -> impl Iterator<Item = usize> {
        non_rhythm_game_items().filter(|&item| item != COLOR_STACK_ITEM_INDEX)
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
        // カラーストックは難易度選択を経由しないので別のテストで確認する
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
            .inner(app.last_area)
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
        app.screen = Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(0));
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
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
        // カラーストックは難易度選択を経由しないので別のテストで確認する
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
        app.screen = Screen::SelectDifficulty(COUNT_MANIA_ITEM_INDEX, None); // マウス専用ゲーム
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
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
        app.screen = Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(1));
        apps.push(("SelectDifficulty(リズム)", app));

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
        app.screen = Screen::SelectDifficulty(RHYTHM_ITEM_INDEX, Some(0));
        press(&mut app, KeyCode::Char('1'));
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
        assert!(matches!(app.screen, Screen::RhythmSplash));
        apps.push(("RhythmSplash", app));

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
        app.menu_state.select(Some(5));
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Down); // ダイアログ中はメニューのカーソルが動かない
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.menu_state.selected(), Some(5));
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
                let rows = rendered_cells(&mut app, width, height);
                let top = rows
                    .iter()
                    .position(|r| r.contains(&"╮".to_string()))
                    .unwrap_or_else(|| panic!("{width}x{height}: 右上の角が描かれていること"));
                let left = rows[top]
                    .iter()
                    .position(|c| c == "╭")
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

        // 「01  」の4文字+「図形」の2文字
        app.update(MENU_CHAR_INTERVAL * 6);
        let mid = rendered_compact(&mut app);
        assert!(mid.contains("01図形"), "1項目目の名前が途中まで出る");
        assert!(!mid.contains(MENU_ITEMS[0]));
        assert!(!mid.contains(MENU_ITEMS[1]), "2項目目はまだ出ない");

        app.update(LONG_ENOUGH);
        assert!(app.menu_typewriter.is_finished());
        let after = rendered_compact(&mut app);
        for (name, description) in MENU_ITEMS.iter().zip(MENU_DESCRIPTIONS) {
            assert!(after.contains(name), "{name}が出ること");
            assert!(after.contains(description), "{description}が出ること");
        }
    }

    #[test]
    fn menu_items_appear_in_order_from_top_to_bottom() {
        let mut app = app_entering_menu();
        let mut shown = 0;
        while !app.menu_typewriter.is_finished() {
            app.update(MENU_CHAR_INTERVAL * 5);
            let text = rendered_compact(&mut app);
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
        for height in [24u16, 30, 45] {
            let mut app = App::new();
            app.last_area = rect(0, 0, 80, height);
            press(&mut app, KeyCode::Enter);
            let expected = typewriter::char_count(&menu_item_lines(rect(0, 0, 80, height)));
            assert_eq!(app.menu_typewriter.total_chars(), expected, "height={height}");
            // 描画した画面サイズの内容に合わせて全文字数が更新される
            rendered_rows_without_spaces(&mut app, 80, 30);
            let resized = typewriter::char_count(&menu_item_lines(rect(0, 0, 80, 30)));
            assert_eq!(app.menu_typewriter.total_chars(), resized, "height={height}→30");
        }
    }

    #[test]
    fn typed_menu_items_are_on_the_rows_that_click_to_them() {
        // 途中まで流れている間も、見えている項目名の行をクリックするとその項目になる
        for height in [24u16, 30] {
            let mut app = App::new();
            app.last_area = rect(0, 0, 80, height);
            press(&mut app, KeyCode::Enter);
            app.update(MENU_CHAR_INTERVAL * 120);
            let rows = rendered_rows_without_spaces(&mut app, 80, height);
            let area = rect(0, 0, 80, height);
            let visible: Vec<usize> = (0..MENU_ITEMS.len())
                .filter(|&i| rows.iter().any(|r| r.contains(MENU_ITEMS[i])))
                .collect();
            assert!(!visible.is_empty() && visible.len() < MENU_ITEMS.len(), "height={height}: 途中まで");
            for i in visible {
                let row = rows.iter().position(|r| r.contains(MENU_ITEMS[i])).unwrap();
                assert_eq!(menu_item_at_row(area, row as u16), Some(i), "height={height}");
            }
        }
    }

    #[test]
    fn key_while_menu_is_typing_finishes_it_and_still_moves_the_selection() {
        let mut app = app_entering_menu();
        press(&mut app, KeyCode::Down);
        assert!(app.menu_typewriter.is_finished(), "キー入力で全文字表示済みになる");
        assert_eq!(app.menu_state.selected(), Some(1), "上下キーの選択移動もそのまま効く");
        assert!(rendered_compact(&mut app).contains(MENU_ITEMS[MENU_ITEMS.len() - 1]));
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
        let row = (0..30)
            .find(|&row| menu_item_at_row(app.last_area, row) == Some(HISTORY_ITEM_INDEX))
            .unwrap();
        app.handle_mouse(left_click(row));
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
        let total = typewriter::char_count(&menu_item_lines(rect(0, 0, 80, 30)));
        let duration = MENU_CHAR_INTERVAL * total as u32;
        assert!(
            duration <= Duration::from_secs(6),
            "メニュー全体が{duration:?}で流れ切る(全{total}文字)"
        );
    }
}
