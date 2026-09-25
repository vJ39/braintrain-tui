## 概要

`assets/image/background.jpeg`(暗いネイビー基調・グリッド線・発光点・脳モチーフの汎用背景)を、TTR(`Screen::SelectSong`。スプラッシュ画像を背景に使っている)以外の全画面共通の背景として敷く。

## 前回の実装で判明した問題

1回目の実装は「各画面の外枠(パネル)の範囲だけClearしてから描く」方式だったが、対象画面の外枠がほぼ`area`全体(画面いっぱい)を占めているため、Clearで背景が丸ごと消えてしまい、実質どの画面でも背景が見えない結果になった。

## 実装方式(確定): 画面全体に余白を持たせて中央配置する

対象画面の外枠(パネル)自体を、渡された`area`いっぱいに描くのではなく、`area`の四辺に余白を持たせて一回り小さくした中央のRectに描く。この余白の部分に背景画像がそのまま見える。

### 新しいヘルパー関数

`src/app.rs`に、画面全体から余白を引いた中央のRectを返す関数を追加する。

```rust
/// 画面全体(area)から四辺に余白を持たせた中央のRect。この余白に背景画像を見せる。
/// 余白を引くと0以下になるほど小さい画面では、余白なし(area全体)にする
fn screen_rect(area: Rect) -> Rect {
    const MARGIN: u16 = 2; // 具体的な値は実装時に見た目のバランスで決めてよい
    ...
}
```

### 適用方法

`App::render()`で、対象画面を描画する直前に、渡す`area`を`screen_rect(area)`に置き換える。背景画像自体は`area`全体(画面いっぱい)に描く。

```rust
Screen::Menu => {
    self.background.render(frame, area); // 背景は画面全体に
    let screen = screen_rect(area);
    render_menu(frame, screen, &mut self.menu_state, &mut self.menu_typewriter, &mut self.menu_icons);
}
```

対象画面の内部実装(`render_menu`・`menu_grid`・`render_difficulty_select`・`render_result`・`render_history`・`render_jukebox`・`countdown::render`)自体は変更しない。これらの関数はいずれも受け取った`area`いっぱいに外枠を描く実装になっているため、呼び出し側で`area`を`screen_rect(area)`に絞り込むだけで、内部のロジック(パネルの内側の計算・行の折り返し等)を一切変えずに「一回り小さい中央のパネル」にできる。

### マウスクリック判定への適用

`App::handle_mouse`側でも、各画面のクリック判定関数(`menu_card_at`・`difficulty_at_row`・`song_at_row`(対象外)等)に渡す`area`を、描画時と同じ`screen_rect(area)`に置き換える。描画とクリック判定で使う`area`がずれると、見た目の位置とクリック位置が合わなくなるため、必ず両方で同じ`screen_rect(area)`を使うこと。

### 対象範囲

- `Screen::Splash`(既存の`theme::panel`枠。この枠自体も`screen_rect`で一回り小さくする)
- `Screen::Menu`・`Screen::ConfirmQuit`
- `Screen::SelectDifficulty`
- `Screen::Countdown`
- `Screen::Result`
- `Screen::History`
- `Screen::Jukebox`

`Screen::SelectSong`(TTR)は対象外(既存のスプラッシュ画像を背景にする仕組みをそのまま使う)。

`Screen::Playing(game)`(各ミニゲームのプレイ中画面)は今回のスコープ外とする。

## 対象ファイル

- `src/app.rs`(`screen_rect`ヘルパーの追加、`render()`・`handle_mouse`の各画面描画/クリック判定箇所への適用)
- 新規: `src/ui/background.rs`(背景画像の読み込み・描画。`src/ui/menu_icons.rs`・`src/ui/splash.rs`の画像プロトコル対応パターンを参考にする)
- `src/ui/mod.rs`(新モジュールの追加)

## 画像描画(新規モジュール `src/ui/background.rs`)

- `assets/image/background.jpeg` を起動時に1回だけ読み込み、`StatefulProtocol`として保持する(`splash::picker()`を使って端末の画像プロトコル対応を検出。非対応・読み込めない場合は何も描かない)
- 描画は画面全体(`frame.area()`)に対して行う。`ratatui_image::Resize::Crop(None)`で画面いっぱいに敷く(`SplashRenderer`と同じ考え方)
- 画像プロトコル非対応の端末では何もしない(フォールバックの代替表示は無し。既存の各画面の黒背景のまま)

## Appフィールド

`App`構造体に`background: BackgroundRenderer`(仮称)を追加し、`App::new()`で1回だけ初期化する。既存の`splash_renderer`・`ttr_splash_renderer`・`menu_icons`と同じ並びに追加してよい。

## テスト観点

- `background.jpeg`が`assets/image/`に埋め込まれ、デコードできること
- `screen_rect`が、余白を引いた分だけ小さく・画面中央に配置されたRectを返すこと(上下左右の余白がほぼ均等)。画面が小さすぎる場合は余白なし(area全体)になり、幅・高さが0以下にならないこと
- `Screen::SelectSong`では背景描画が呼ばれない(または描画されても`ttr_splash_renderer`の描画で完全に上書きされる)こと
- 対象範囲に挙げた各画面で、外枠が`area`いっぱいではなく`screen_rect(area)`の範囲に描かれること(四隅のセルが背景の描画対象になっていること、または外枠の位置が`area`の端と一致しないこと)
- 各画面のクリック判定(メニューのカード選択・難易度選択・履歴/ジュークボックスの操作等)が、描画位置とずれずに機能すること
- 画像プロトコル非対応(フォールバック)時にパニックしないこと
- 既存の各画面のテスト(パネル位置・クリック判定・テキスト内容等)のうち、`area`いっぱいの外枠を前提にしていたものは、`screen_rect(area)`基準に書き換える
