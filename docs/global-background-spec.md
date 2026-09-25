## 概要

`assets/image/background.jpeg`(暗いネイビー基調・グリッド線・発光点・脳モチーフの汎用背景)を、TTR(`Screen::SelectSong`。スプラッシュ画像を背景に使っている)以外の全画面共通の背景として敷く。

## 対象ファイル

- `src/app.rs`(`render()`の先頭で背景を描画する)
- 新規: `src/ui/background.rs`(背景画像の読み込み・描画。`src/ui/menu_icons.rs`・`src/ui/splash.rs`の画像プロトコル対応パターンを参考にする)
- `src/ui/mod.rs`(新モジュールの追加)

## 画像描画(新規モジュール `src/ui/background.rs`)

- `assets/image/background.jpeg` を起動時に1回だけ読み込み、`StatefulProtocol`として保持する(`splash::picker()`を使って端末の画像プロトコル対応を検出。非対応・読み込めない場合は何も描かない)
- 描画は画面全体(`frame.area()`)に対して行う。`ratatui_image::Resize::Crop(None)`で画面いっぱいに敷く(`SplashRenderer`と同じ考え方)
- 画像プロトコル非対応の端末では何もしない(フォールバックの代替表示は無し。既存の各画面の黒背景のまま)

## app.rsでの適用

`App::render()`の`match &mut self.screen { ... }`の直前に、`Screen::SelectSong`以外の全バリアントで背景画像を描画する処理を追加する。

```
match &self.screen {
    Screen::SelectSong(_) => {} // TTRは専用のスプラッシュ画像を背景にするため対象外
    _ => self.background.render(frame, area),
}
```

背景を描いた「後」に、既存の各画面の描画(`render_menu`・`render_song_select`・`game.render`等)をそのまま行う。各画面のパネル(`theme::panel`等)は背景色(bg)を指定していないため、パネルの枠内の空白部分には背景画像がそのまま透けて見える(`Screen::SelectSong`で`ttr_splash_renderer`→`render_song_select`の順に描いている既存パターンと同じ)。

`Screen::Playing(game)`(各ミニゲーム画面)も対象に含める。ただしカウントマニアのプレッシャー背景・イロピッタンの全面色背景のように、ゲーム側が明示的に背景色を敷いている画面では、その上からゲーム側の描画で上書きされるため、共通背景は見えなくなる。これは意図した動作(ゲーム側の演出を壊さない)としてそのままでよい。

## Appフィールド

`App`構造体に`background: BackgroundRenderer`(仮称)を追加し、`App::new()`で1回だけ初期化する。既存の`splash_renderer`・`ttr_splash_renderer`・`menu_icons`と同じ並びに追加してよい。

## テスト観点

- `background.jpeg`が`assets/image/`に埋め込まれ、デコードできること
- `Screen::SelectSong`では背景描画が呼ばれない(または描画されても`ttr_splash_renderer`の描画で完全に上書きされる)こと
- それ以外の画面(Menu・SelectDifficulty・Countdown・Playing・Result・History・Jukebox・ConfirmQuit・Splash)で背景描画の処理が呼ばれること
- 画像プロトコル非対応(フォールバック)時にパニックしないこと
- 既存の各画面のテスト(パネル位置・クリック判定・テキスト内容等)が壊れていないこと(背景は「奥」に描くだけで、既存のUIの位置・当たり判定には影響しないはず)
