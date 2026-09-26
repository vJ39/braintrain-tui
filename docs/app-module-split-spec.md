## 概要

`src/app.rs`(4347行)を `src/app/` ディレクトリ配下の画面単位のモジュールに分割する。`Screen` の各バリアント(Splash / Menu / BeigomaSplash / SelectSong / SelectDifficulty / Countdown / Playing / Result / History / Jukebox / ConfirmQuit)ごとに、その画面の描画・入力処理・画面遷移・テストを1ファイルにまとめ、1つの画面を変更する時に読む範囲をそのファイルと `mod.rs` のディスパッチだけにする。

振る舞い・公開API(`App::new` / `handle_key` / `handle_mouse` / `update` / `render` / `should_quit` / `take_pending_scrollback_clear`)・テスト名・テスト数(クレート全体で1232本)は現状のまま維持する。`main.rs` の変更はない。

## モジュール構成

```text
src/app/
  mod.rs               Screen / App / new() / 4つのディスパッチ(handle_key, handle_mouse, update, render) / 画面横断の小さな処理
  menu_items.rs        メニュー項目のカタログ(MENU_ITEMS・各項目インデックス・説明文・アイコンパス)と new_game
  screen_layout.rs     画面全体の余白・共通背景・中央配置(screen_rect / render_background / centered_rect)
  splash_screens.rs    Splash(タイトル)・BeigomaSplash
  menu.rs              Menu(カードグリッド・タイプライター・項目決定・メニューへ戻る遷移)
  confirm_quit.rs      ConfirmQuit
  select_song.rs       SelectSong(TTRの曲選択)
  select_difficulty.rs SelectDifficulty
  game_start.rs        Countdown と、各ゲームの開始経路(カウントダウン経由 / 直接開始 / スプラッシュ経由 / 曲選択経由)
  result.rs            Result
  history.rs           History
  jukebox.rs           Jukebox
  test_support.rs      #[cfg(test)] テスト共有ヘルパー
```

`src/ui/` の既存モジュール(`countdown.rs` / `splash.rs`)とは名前を変えてある(`game_start.rs` / `splash_screens.rs`)。`app` 側モジュールは画面(状態と遷移)を、`ui` 側モジュールは部品(描画器・アニメーション)を持つ。

Playing は `Game` トレイトへの委譲(handle_key / handle_mouse / update / render を game に渡し、`is_finished()` なら `enter_result`)だけなので、独立モジュールを持たず `mod.rs` の各ディスパッチ内に書く。

## 可視性の方針

- `App` のフィールドは全て private のまま。子モジュール(`app::menu` 等)と、その配下のテストモジュールは親 `app` の private フィールドにそのままアクセスできる
- 画面モジュールが `impl App` ブロックで定義するメソッドと free 関数のうち、`mod.rs` や他の画面モジュールから呼ぶものは `pub(super)`。それ以外は private
- クレート外・`app` 外への公開は現状の `App` の公開メソッドと `pub enum Screen` のみ(`Screen` の宣言は変えずに `mod.rs` へ置く)
- 各画面モジュールは自分が使う項目だけを明示的に `use` する(`use super::menu_items::{MENU_ITEMS, RHYTHM_ITEM_INDEX};` のように)。glob import は使わない

## 各モジュールの責務と移動する要素

行番号は分割前の `src/app.rs` のもの。

### mod.rs

責務: 状態の定義と、画面へのディスパッチ。各画面の中身はここに置かず、画面モジュールの `pub(super)` 関数を呼ぶ。

| 移動元 | 要素 |
|---|---|
| 112-139 | `pub enum Screen` |
| 141-198 | `pub struct App`、`App::new` |
| 210-218 | `skip_typewriter`(Menu と Result の2画面のタイプライターに触るため画面横断) |
| 220-228 | `take_pending_scrollback_clear`、`should_quit` |
| 230-281 | `handle_key`(ディスパッチのみ。`[q]` の共通ルールを含む) |
| 308-355 | `handle_mouse`(ディスパッチのみ。左クリック以外を弾く判定と `skip_typewriter` を含む) |
| 634-666 | `update`(ディスパッチのみ) |
| 668-758 | `render`(ディスパッチのみ。`last_area` の更新を含む) |

`mod` 宣言:

```rust
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
```

`App::new` は `crate::ui::splash::TITLE_IMAGE_PATH` 等をフルパスで参照する(`mod splash_screens` と `ui::splash` の短縮名が並ばないようにする)。

### menu_items.rs

責務: メニュー項目のカタログ。ゲームを1つ追加する時に触るのはこのファイル(項目・説明・アイコン・`new_game`)と `game_start.rs`(開始経路)の2つ。

| 移動元 | 要素 | 可視性 |
|---|---|---|
| 35-51 | `MENU_ITEMS` | `pub(super)` |
| 53-68 | `REACTION_ITEM_INDEX` / `MEMORY_ITEM_INDEX` / `COUNT_MANIA_ITEM_INDEX` / `COLOR_STACK_ITEM_INDEX` / `RHYTHM_ITEM_INDEX` / `QUICK_DRAW_ITEM_INDEX` / `BEIGOMA_ITEM_INDEX` / `JUKEBOX_ITEM_INDEX` / `HISTORY_ITEM_INDEX` | `pub(super)` |
| 70-87 | `MENU_DESCRIPTIONS` | `pub(super)` |
| 89-106 | `MENU_ICON_PATHS` | `pub(super)` |
| 826-848 | `new_game(item, difficulty) -> Box<dyn Game>` | `pub(super)` |

参照元: `mod.rs`(`new` の `MENU_ICON_PATHS`)、`menu.rs`、`select_song.rs`(`MENU_ITEMS[RHYTHM_ITEM_INDEX]`)、`select_difficulty.rs`(見出しの `MENU_ITEMS[item]`)、`game_start.rs`(`new_game`、各インデックス)、`result.rs`(`result_lines` の game_id → 表示名)。

### screen_layout.rs

責務: 画面全体(`frame.area()`)に対する四辺の余白、共通背景の敷き方、矩形の中央配置。

| 移動元 | 要素 | 可視性 |
|---|---|---|
| 761-764 | `SCREEN_MARGIN_X` / `SCREEN_MARGIN_Y` | private |
| 766-778 | `screen_rect(area) -> Rect` | `pub(super)` |
| 780-792 | `render_background(frame, background, area) -> Rect` | `pub(super)` |
| 1505-1522 | `centered_rect(area, width, height) -> Rect` | `pub(super)` |

参照元: `mod.rs`(`render` の各アーム)、`menu.rs`(`screen_rect(self.last_area)`)、`confirm_quit.rs` / `select_song.rs` / `result.rs`(`centered_rect`)、`select_difficulty.rs`(クリック判定の `screen_rect`)。

### splash_screens.rs

責務: Enter/クリックで次へ進む全画面スプラッシュ2種(タイトル・べー開始前)。

所有する `App` フィールド: `splash_renderer`、`beigoma_splash_renderer`。

| 移動元 | 要素 | 新しい形 |
|---|---|---|
| 357-361 | `leave_splash` | `impl App { pub(super) fn leave_splash(&mut self) }` |
| 363-367 | `leave_beigoma_splash` | `impl App { pub(super) fn leave_beigoma_splash(&mut self) }` |
| 417-424 の BEIGOMA 分岐の本体(SE再生・`beigoma_splash_renderer.reset()`・`Screen::BeigomaSplash`) | `impl App { pub(super) fn enter_beigoma_splash(&mut self) }` |
| 676-683 の Splash アームの本体 | `pub(super) fn render_title(frame, area, renderer: &mut SplashRenderer)` |
| 684-690 の BeigomaSplash アームの本体 | `pub(super) fn render_beigoma(frame, area, renderer: &mut AnimatedSplash)` |

`render_title` / `render_beigoma` は `theme::panel("")` の枠を描き、内側に renderer を描く(現状のアーム本体と同じ)。BeigomaSplash の `update`(`beigoma_splash_renderer.tick(dt)`)は1行なので `mod.rs` の `update` に直接書く。

### menu.rs

責務: メニュー画面(カードグリッドの配置・描画・クリック判定・タイプライター・キー操作)と、メニュー画面へ入る遷移3種。

所有する `App` フィールド: `menu_state`、`menu_typewriter`、`menu_icons`。

| 移動元 | 要素 | 可視性 |
|---|---|---|
| 108-110 | `MENU_CHAR_INTERVAL` | `pub(super)`(`mod.rs` の `new` が使う) |
| 953-959 | `MENU_CARD_MIN_WIDTH` / `MENU_ICON_ROWS` / `MENU_CARD_PADDING` | private |
| 961-978 | `MenuGridState`(`selected()` / `select()`) | 型と2メソッドを `pub(super)`(`mod.rs` の `new` と他モジュールのテストが使う)。フィールドは private |
| 980-1070 | `MenuGrid`、`menu_grid` | private |
| 1072-1090 | `wrap_by_width` | private |
| 1092-1123 | `GridMove`、`grid_move` | private |
| 1125-1132 | `menu_card_at` | private |
| 1134-1157 | `menu_item_lines` | private |
| 932-951 | `menu_block` | private |
| 1159-1233 | `render_menu` → `pub(super) fn render(frame, area, state, typing, icons)`、`render_menu_card` は private | |
| 200-208 | `enter_menu` | `impl App { pub(super) fn enter_menu(&mut self) }` |
| 519-527 | `return_to_menu` | `impl App { pub(super) fn return_to_menu(&mut self) }` |
| 292-306 | `quit_to_menu` | `impl App { pub(super) fn quit_to_menu(&mut self) }` |
| 542-556 | `handle_menu_key` | `impl App { pub(super) fn handle_menu_key(&mut self, key) }` |
| 323-329 の Menu アームの本体 | `impl App { pub(super) fn handle_menu_mouse(&mut self, mouse: MouseEvent) }`(`self.last_area` から `screen_rect` を求め、`menu_card_at` → `select` → `select_menu_item`) |
| 393-451 | `select_menu_item` | `impl App { pub(super) fn select_menu_item(&mut self, selected) }`。分割後の中身は下記 |

`select_menu_item` は次の3分岐だけを持つ。

```rust
pub(super) fn select_menu_item(&mut self, selected: usize) {
    if selected == HISTORY_ITEM_INDEX {
        audio::play_se(SeKind::Transition);
        self.screen = Screen::History;
    } else if selected == JUKEBOX_ITEM_INDEX {
        audio::play_se(SeKind::Transition);
        self.enter_jukebox();
    } else {
        self.start_game_item(selected);
    }
}
```

ゲーム項目ごとの経路判定(難易度選択を挟むか・カウントダウンを挟むか・スプラッシュや曲選択を挟むか)は `game_start.rs` の `start_game_item` が持つ。

### confirm_quit.rs

責務: 終了確認ダイアログ(メニューの上に重ねる)。

| 移動元 | 要素 | 新しい形 |
|---|---|---|
| 794-824 | `render_confirm_quit` | `pub(super) fn render(frame, area)` |
| 283-290 | `handle_confirm_quit_key` | `impl App { pub(super) fn handle_confirm_quit_key(&mut self, key) }` |

ダイアログを開く遷移(`Screen::ConfirmQuit` の代入)は `[q]` の共通ルールの一部なので `mod.rs` の `handle_key` に残す。`render` の ConfirmQuit アームは `menu::render(...)` の後に `confirm_quit::render(frame, screen)` を呼ぶ。

### select_song.rs

責務: TTRの曲選択画面(TTRスプラッシュ画像を背景にした曲リストパネル)。

所有する `App` フィールド: `ttr_splash_renderer`。

| 移動元 | 要素 | 新しい形 |
|---|---|---|
| 850-859 | `SONG_ROWS_OFFSET` / `SONG_SELECT_HINTS` | private |
| 861-908 | `song_panel_block` / `song_select_lines` / `song_panel_rect` | private |
| 701-713 の SelectSong アームの本体 + 910-918 `render_song_select` | `pub(super) fn render(frame, area, selected, ttr_splash: &mut SplashRenderer)`(Fallback時に背景の描画範囲をパネルの上側に限る処理を含む) |
| 920-930 | `song_at_row` | private |
| 440-447 の RHYTHM 分岐の本体(RhythmSplash の BGM 開始・`Screen::SelectSong(0)`) | `impl App { pub(super) fn enter_song_select(&mut self) }`。先頭で `audio::play_se(SeKind::Transition)` を鳴らす(現状は `select_menu_item` の 435 行目で鳴らしている音) |
| 595-612 | `handle_song_key` | `impl App { pub(super) fn handle_song_key(&mut self, key, selected) }` |
| 330-334 の SelectSong アームの本体 | `impl App { pub(super) fn handle_song_mouse(&mut self, mouse) }`(`song_at_row(self.last_area, mouse.row)` → `select_song`) |
| 453-457 | `select_song` | `impl App { pub(super) fn select_song(&mut self, song) }`(`start_rhythm` を呼ぶ) |

### select_difficulty.rs

責務: 難易度選択画面。

| 移動元 | 要素 | 新しい形 |
|---|---|---|
| 1235-1237 | `DIFFICULTY_ROWS_OFFSET` | private |
| 714-721 の SelectDifficulty アームの本体 + 1239-1287 `render_difficulty_select` | `pub(super) fn render(frame, area, item: usize, song: Option<usize>)`(見出し文字列の組み立てをこの中で行う) |
| 1289-1299 | `difficulty_at_row` | private |
| 614-632 | `handle_difficulty_key` | `impl App { pub(super) fn handle_difficulty_key(&mut self, key, item, song) }` |
| 335-340 の SelectDifficulty アームの本体 | `impl App { pub(super) fn handle_difficulty_mouse(&mut self, mouse, item) }`(`screen_rect(self.last_area)` 基準で `difficulty_at_row` → `start_playing`) |

難易度選択画面へ入る遷移は `Screen::SelectDifficulty(item, None)` の代入1行なので、`game_start.rs` の `start_game_item` が直接代入する。

### game_start.rs

責務: メニュー項目からゲームが始まるまでの経路と、カウントダウン画面。ゲームを追加・経路を変える時に読むのはこのファイル。

| 移動元 | 要素 | 新しい形 |
|---|---|---|
| 394-434 と 448-450(`select_menu_item` のゲーム項目ぶんの分岐) | `impl App { pub(super) fn start_game_item(&mut self, item) }` | |
| 459-476 | `start_playing` | `impl App { pub(super) fn start_playing(&mut self, item, difficulty) }` |
| 478-506 | `start_count_mania` / `start_quick_draw` / `start_beigoma` | `impl App { pub(super) fn ... }`(`start_beigoma` は `splash_screens.rs` から、他2つはこのファイル内から呼ばれる) |
| 508-517 | `start_rhythm` | `impl App { pub(super) fn start_rhythm(&mut self, song) }`(`select_song.rs` から呼ばれる) |
| 645-657 の Countdown アームの本体 | `impl App { pub(super) fn tick_countdown(&mut self, dt) }`(`let Screen::Countdown { item, difficulty, state } = &mut self.screen else { return };` で取り出してから現状と同じ処理) |

`start_game_item` の分岐(順序と効果は現状の `select_menu_item` と同じ):

| item | 経路 |
|---|---|
| `COLOR_STACK_ITEM_INDEX` | `start_playing(item, color_stack::SESSION_DIFFICULTY)` |
| `COUNT_MANIA_ITEM_INDEX` | `start_count_mania()` |
| `QUICK_DRAW_ITEM_INDEX` | `start_quick_draw()` |
| `BEIGOMA_ITEM_INDEX` | `enter_beigoma_splash()` |
| `MEMORY_ITEM_INDEX` | `start_playing(item, memory::SESSION_DIFFICULTY)` |
| `REACTION_ITEM_INDEX` | `start_playing(item, reaction::SESSION_DIFFICULTY)` |
| `RHYTHM_ITEM_INDEX` | `enter_song_select()` |
| その他 | `audio::play_se(SeKind::Transition)` の後 `self.screen = Screen::SelectDifficulty(item, None)` |

`SeKind::Transition` が鳴る箇所は現状と同じ(History / Jukebox / 曲選択へ / 難易度選択へ / べースプラッシュへ の5経路。カウントダウン経由と直接開始は鳴らさない)。

Countdown の描画は `mod.rs` の `render` から `crate::ui::countdown::render(frame, screen, state)` を直接呼ぶ。Countdown 中のキー・マウスは `mod.rs` のディスパッチで無視する(現状どおり)。

### result.rs

責務: リザルト画面(本文・タイプライター・キャラクター・保存エラー表示)と、リザルトへ入る遷移。

所有する `App` フィールド: `result_typewriter`、`result_sprite`。

| 移動元 | 要素 | 新しい形 |
|---|---|---|
| 369-383 | `enter_result` | `impl App { pub(super) fn enter_result(&mut self, result) }` |
| 385-391 | `show_result` | `impl App { pub(super) fn show_result(&mut self, result, save_error) }` |
| 1301-1377 | `result_lines` | private |
| 1379-1382 | `result_card_rect` | private |
| 1384-1430 | `render_result` | `pub(super) fn render(frame, area, result, save_error, typing, sprite)` |

Result 画面の Enter/Esc/クリックでメニューへ戻る処理は History と共通の1アーム(`Screen::Result(..) | Screen::History`)として `mod.rs` に残す。`update` の Result アーム(`result_typewriter.tick` と `result_sprite.tick` の2行)も `mod.rs` に直接書く。

### history.rs

責務: 履歴画面の外枠(見出し・操作説明)。中身は `crate::stats::history_view::render` に委譲する。

| 移動元 | 要素 | 新しい形 |
|---|---|---|
| 1496-1503 | `render_history` | `pub(super) fn render(frame, area)` |

### jukebox.rs

責務: ジュークボックス画面。

| 移動元 | 要素 | 新しい形 |
|---|---|---|
| 529-540 | `jukebox_list_state` | `impl App { pub(super) fn jukebox_list_state(&self) -> ListState }`(テストが他モジュールから使う) |
| 438-439 の JUKEBOX 分岐の本体 | `impl App { pub(super) fn enter_jukebox(&mut self) }`(`self.screen = Screen::Jukebox(self.jukebox_list_state())`) |
| 558-593 | `handle_jukebox_key` | `impl App { pub(super) fn handle_jukebox_key(&mut self, key) }` |
| 1432-1494 | `render_jukebox` | `pub(super) fn render(frame, area, state, playing)` |

`current_bgm` は `App` の共有フィールドのまま(ジュークボックスだけでなく、各ゲーム開始・リザルト・メニューへ戻る遷移が更新する)。

## mod.rs のディスパッチ

### handle_key

`skip_typewriter()` → `[q]` の共通ルール(Menu なら `Screen::ConfirmQuit`、ConfirmQuit なら無視、それ以外は `quit_to_menu()`)→ 画面ごとの分岐。

| Screen | 呼び出し |
|---|---|
| Splash | Enter なら `self.leave_splash()` |
| BeigomaSplash | Enter なら `self.leave_beigoma_splash()` |
| Menu | `self.handle_menu_key(key)` |
| SelectSong(selected) | `self.handle_song_key(key, selected)` |
| SelectDifficulty(item, song) | `self.handle_difficulty_key(key, item, song)` |
| Countdown | 無視 |
| Playing(game) | `game.handle_key(key)`、終了していれば `self.enter_result(game.result())` |
| Result / History | Enter か Esc なら `self.return_to_menu()` |
| Jukebox | `self.handle_jukebox_key(key)` |
| ConfirmQuit | `self.handle_confirm_quit_key(key)` |

### handle_mouse

左クリック以外は無視 → `skip_typewriter()` → 画面ごとの分岐。

| Screen | 呼び出し |
|---|---|
| Splash | `self.leave_splash()` |
| BeigomaSplash | `self.leave_beigoma_splash()` |
| Menu | `self.handle_menu_mouse(mouse)` |
| SelectSong | `self.handle_song_mouse(mouse)` |
| SelectDifficulty(item, _) | `self.handle_difficulty_mouse(mouse, item)` |
| Countdown | 無視 |
| Playing(game) | `game.handle_mouse(mouse, self.last_area)`、終了していれば `self.enter_result(game.result())` |
| Result / History | `self.return_to_menu()` |
| Jukebox / ConfirmQuit | 無視 |

### update

| Screen | 処理 |
|---|---|
| Playing(game) | `game.update(dt)`、終了していれば `self.enter_result(game.result())` |
| Countdown | `self.tick_countdown(dt)` |
| Menu | `self.menu_typewriter.tick(dt)` |
| Result | `self.result_typewriter.tick(dt)`、`self.result_sprite.tick(dt)` |
| BeigomaSplash | `self.beigoma_splash_renderer.tick(dt)` |
| その他 | 何もしない |

### render

`self.last_area = frame.area()` の後、画面ごとの分岐。`screen` は `render_background(frame, &mut self.background, area)` の戻り値。

| Screen | 背景 | 呼び出し |
|---|---|---|
| Splash | 共通背景 | `splash_screens::render_title(frame, screen, &mut self.splash_renderer)` |
| BeigomaSplash | 共通背景 | `splash_screens::render_beigoma(frame, screen, &mut self.beigoma_splash_renderer)` |
| Menu | 共通背景 | `menu::render(frame, screen, &mut self.menu_state, &mut self.menu_typewriter, &mut self.menu_icons)` |
| SelectSong(selected) | TTR画像(select_song 側で描く) | `select_song::render(frame, area, *selected, &mut self.ttr_splash_renderer)` |
| SelectDifficulty(item, song) | 共通背景 | `select_difficulty::render(frame, screen, *item, *song)` |
| Countdown { state, .. } | 共通背景 | `crate::ui::countdown::render(frame, screen, state)` |
| Playing(game) | ゲーム側 | `game.render(frame, area)` |
| Result(result, save_error) | 共通背景 | `result::render(frame, screen, result, save_error.as_deref(), &mut self.result_typewriter, &mut self.result_sprite)` |
| History | 共通背景 | `history::render(frame, screen)` |
| Jukebox(state) | 共通背景 | `jukebox::render(frame, screen, state, current_bgm.as_deref())` |
| ConfirmQuit | 共通背景 | `menu::render(...)` の後 `confirm_quit::render(frame, screen)` |

`match &mut self.screen` の中で `self` の別フィールド(`self.menu_state` 等)を `&mut` で借りる書き方は現状と同じ(フィールド単位の借用なので競合しない)。画面モジュールの描画関数はメソッドではなく free 関数で、必要なフィールドを引数で受け取る。

## テストの移動方針

### test_support.rs

`#[cfg(test)] mod test_support;` として `app` 配下に置き、2つ以上の画面モジュールのテストが使うヘルパーを集める。すべて `pub(super)`。各モジュールのテストは `use super::super::test_support::*;`(またはフルパス `crate::app::test_support::*`)で取り込む。

| 分類 | 要素(移動元の行) |
|---|---|
| 描画 | `rendered_text`(2442)、`rendered_compact`(3541)、`rendered_rows_without_spaces`(2716)、`rendered_cells`(3489)、`drawn_position_of`(2733) |
| 入力 | `rect`(1806)、`press`(3236)、`left_click`(3055)、`left_click_at`(2753) |
| Appの用意 | `app_on_menu`(2763)、`app_entering_menu`(3546)、`app_on_confirm_quit`(3384)、`app_showing_result`(3555)、`apps_on_every_non_menu_screen`(3245)、`sample_result`(3536)、`fixed_progression_games`(1710) |
| カウントダウン | `COUNTDOWN_TOTAL`(3025)、`finish_countdown`(3028) |
| 定数 | `LONG_ENOUGH`(3534)、`PENDING_MENU_ICONS`(1838) |
| 画像 | `test_picker`(3889) |

1つのモジュールのテストだけが使うヘルパーはそのモジュールのテスト内に置く(`is_menu_bgm` / `non_rhythm_game_items` / `difficulty_select_game_items` / `halfblocks_background` / `swap_background` / `buffers_with_and_without_background` / `halfblocks_sprite` / `rendered_buffer` / `result_card_for` / `game_over_result` / `card_text` / `icon_cells` / `frame_corner_around` / `app_on_menu_with_icons` / `assert_*`)。

### 各モジュールへの割り当て

各モジュール末尾の `#[cfg(test)] mod tests { use super::*; ... }` へ移す。テスト名は変えない。

| 移動先 | テスト(移動元の行) |
|---|---|
| menu_items.rs | `new_game_handles_every_game_menu_item`(1530)、`count_mania_comes_right_before_color_stack`(1543)、`new_game_for_count_mania_item_creates_count_mania`(1549)、`color_stack_is_the_last_game_before_rhythm`(1618)、`new_game_for_color_stack_item_creates_color_stack`(1624)、`memory_and_reaction_item_indices_match_menu`(1725)、`new_game_for_memory_and_reaction_records_session_difficulty`(1731)、`jukebox_and_history_are_the_last_two_menu_items`(1798)、`menu_icon_paths_match_every_menu_item_in_order`(1812)、`every_menu_icon_is_embedded_and_decodes`(1840)、`rhythm_item_index_points_at_rhythm_menu_item`(2035)、`quick_draw_comes_right_after_ttr_and_before_beigoma`(2042)、`quick_draw_menu_texts_no_longer_use_old_name`(2052)、`new_game_for_quick_draw_item_creates_quick_draw`(2058)、`beigoma_comes_right_after_quick_draw_and_before_jukebox`(2121)、`new_game_for_beigoma_item_creates_beigoma`(2129) |
| screen_layout.rs | `screen_rect_*` 4本(3906-3949)、`apps_on_every_background_screen` と背景の統合テスト6本(`every_background_screen_is_drawn_inside_screen_rect` / `framed_background_screens_put_their_outer_frame_on_screen_rect` / `background_fills_the_margins_and_the_ui_stays_visible` / `image_protocol_background_does_not_hide_the_ui` / `song_select_and_playing_do_not_draw_the_background` / `background_screens_render_without_panicking_at_any_size`、3951-4135) |
| splash_screens.rs | `app_starts_on_splash_screen`(2618)、`splash_enter_key_transitions_to_menu`(2624)、`splash_screen_is_framed_by_a_full_screen_border`(2631)、`splash_other_key_stays_on_splash`(2686)、`splash_click_transitions_to_menu`(2693)、`splash_renders_without_panicking`(2706) |
| menu.rs | カードグリッドのデータ・レイアウト(1853-1980 の `menu_grid_*` / `menu_cards_fit_*` / `grid_move_*` / `arrow_keys_*` / `menu_card_at_*`)、`selecting_jukebox_menu_item_enters_jukebox_screen`(2012)、`selecting_history_menu_item_enters_history_screen`(2019)、スクロールバッファのクリア2本(2655-2684)、描画位置とクリック判定(2769-2983)、メニューのタイプライター(3623-3811 の `menu_*` / `card_text` / `typed_menu_cards_*` / `key_while_menu_*` / `right_key_*` / `enter_while_menu_*` / `click_while_menu_*` / `mouse_move_*` / `q_while_menu_is_typing_*`)、`returning_to_menu_restarts_the_menu_typing`(3843)、`menu_char_interval_types_the_whole_menu_in_a_few_seconds`(3872)、`clicking_the_menu_margin_selects_nothing`(4137)、`menu_clicks_hit_the_cards_drawn_inside_screen_rect`(4149)、`menu_keys_and_typing_use_the_grid_inside_screen_rect`(4328) |
| confirm_quit.rs | `q_on_menu_opens_confirm_quit_without_quitting`(3363)、`q_twice_from_a_non_menu_screen_ends_at_confirm_quit`(3372)、`confirm_quit_*` 8本と `cancelling_confirm_quit_keeps_menu_selection` / `mouse_clicks_on_confirm_quit_are_ignored` / `update_on_confirm_quit_keeps_the_dialog_open`(3392-3527) |
| select_song.rs | リズムのメニュー→曲選択→プレイ(2224-2431 の `selecting_rhythm_*` / `ttr_splash_bgm_*` / `starting_the_song_*` / `clicking_rhythm_*` / `song_select_*` / `ttr_song_select_q_returns_to_menu` / `song_panel_is_centered_*` / `ttr_splash_uses_its_own_renderer_*` / `assert_rhythm_playing_song`)、`full_rhythm_flow_starts_selected_song_with_its_bgm`(2455)、`song_select_screen_lists_every_song`(2512)、`song_at_row_maps_each_song_row`(2526)、`clicking_song_row_starts_playing_that_song`(2546)、`clicking_outside_song_rows_stays_on_song_select`(2559)、`song_rows_are_drawn_where_song_at_row_expects`(3004)、`starting_rhythm_goes_straight_to_playing_without_countdown`(3110) |
| select_difficulty.rs | `difficulty_at_row_maps_beginner_intermediate_advanced`(1982)、`difficulty_at_row_outside_options_is_none`(2000)、`other_game_difficulty_esc_still_returns_to_menu`(2433)、`difficulty_rows_are_drawn_where_difficulty_at_row_expects`(2985)、`clicking_difficulty_also_goes_through_countdown`(3090)、`difficulty_margin_row_selects_nothing`(4161) |
| game_start.rs | ゲームごとの開始経路(カウントメニア 1571-1614、シタケシ 1643-1705、記憶・イロピッタン 1747-1784、`other_games_still_go_to_difficulty_select` 1786、`selecting_game_menu_item_enters_difficulty_screen` 2026、ハヤウチ 2079-2105、べー 2146-2211 のスプラッシュ経由を含む4本と `assert_beigoma_round1_countdown_in_game`)、カウントダウン(`non_rhythm_game_items` / `difficulty_select_game_items` と 3064-3232 の `countdown_*` / `starting_any_non_rhythm_game_goes_through_countdown` / `keys_during_countdown_are_ignored` / `mouse_clicks_during_countdown_are_ignored` / `game_is_playable_after_countdown`)、`q_during_countdown_aborts_it_and_no_game_starts`(3349) |
| result.rs | `result_screen_shows_quick_draw_menu_name`(2107)、`result_screen_shows_beigoma_menu_name`(2213)、`game_over_switches_bgm_to_result_category`(2472)、`rendering_the_result_screen_repeatedly_does_not_re_save_history`(2485)、リザルトのタイプライター(3562-3621、3813-3841)、キャラクター(4176-4326 の `halfblocks_sprite` / `rendered_buffer` / `result_card_for` / `game_over_result` と `result_sprite_*` / `show_result_restarts_*` / `game_over_*` / `non_game_over_*` / `wide_result_*` / `narrow_result_*` / `result_screen_with_*`) |
| jukebox.rs | `jukebox_up_down_wraps_around_track_list`(2570)、`jukebox_enter_sets_current_bgm_to_selected_track`(2588)、`jukebox_stop_key_clears_current_bgm`(2600)、`jukebox_esc_returns_to_menu`(2608) |
| mod.rs | 画面横断の `[q]` フロー(`is_menu_bgm` と 3296-3347 の `q_on_every_non_menu_screen_*` / `q_while_playing_*` / `q_on_splash_*` / `q_on_jukebox_after_stopping_bgm_*`)、`typewriters_advance_only_on_their_own_screen`(3853) |

history.rs はテストを持たない(履歴画面の遷移は menu.rs、背景付き描画は screen_layout.rs のテストが担う)。

テスト内の `use crate::ui::countdown::{Phase, PHASE_DURATION};`(3022)は game_start.rs のテストへ、`use crate::ui::typewriter::{self, CHAR_INTERVAL};`(3531)は result.rs と menu.rs のテストへ、`use ratatui_image::picker::{Picker, ProtocolType};`(3886)は test_support.rs と screen_layout.rs / result.rs のテストへ、それぞれ使う側に置く。

## 実装手順

各ステップの終わりに `cargo test` を全件通し、1ステップ1コミットにする。ステップの途中で `mod.rs` に残っている要素と移動済みの要素が混在するのは構わない(コンパイルとテストが通ればよい)。

1. 分割前の基準を記録する: `cargo test -- --list 2>/dev/null | grep ': test$' | sed 's/.*:://' | sort > /tmp/tests-before.txt`(モジュールパスを除いたテスト名一覧)
2. `git mv src/app.rs src/app/mod.rs`。コード変更なしでビルド・テストが通ることを確認してコミット
3. `menu_items.rs` と `screen_layout.rs` を切り出す(App に依存しない定数・関数だけなので最初に分ける)。`mod.rs` は `use` で取り込む。それぞれのテストも移す
4. `test_support.rs` を作り、共有ヘルパーを移す。`mod.rs` のテストは `use super::test_support::*;` で参照する
5. 画面モジュールを1つずつ切り出す。順序は依存の少ないものから: `history` → `jukebox` → `confirm_quit` → `splash_screens` → `select_difficulty` → `select_song` → `result` → `game_start`(`select_menu_item` の分割を含む) → `menu`。各ステップで、そのモジュールに割り当てたテストも一緒に移す
6. `mod.rs` の残りを整える(不要になった `use` の削除、ディスパッチの各アームが上記の表どおりになっていることの確認)
7. 新規ファイルは `rustfmt src/app/<file>.rs` で個別に整形する(`cargo fmt` はクレート全体に及ぶため使わない)
8. 検証: `cargo test`(1232本パス)、手順1と同じコマンドで `/tmp/tests-after.txt` を作り `diff /tmp/tests-before.txt /tmp/tests-after.txt` が空であること、`cargo build --release` が通ること

## 完了条件

- `src/app.rs` が無く、`src/app/` に上記13ファイルがある
- `main.rs` は無変更で、`App` の公開メソッドのシグネチャが分割前と同じ
- テスト名の一覧(モジュールパスを除く)が分割前と一致し、全件パスする
- `mod.rs` に画面固有の定数・レイアウト計算・描画本体が無く、4つのディスパッチと状態定義だけになっている
- 各画面モジュールの `pub(super)` 項目が本書の「新しい形」「可視性」の欄に書いたものだけである
