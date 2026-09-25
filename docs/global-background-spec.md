## 概要

`assets/image/background.jpeg`(暗いネイビー基調・グリッド線・発光点・脳モチーフの汎用背景)を、TTR(`Screen::SelectSong`。スプラッシュ画像を背景に使っている)以外の全画面共通の背景として敷く。

## 実装方式(確定): パネル範囲をClearしてから描く

画像プロトコル(Sixel/iTerm2/Kitty)は、画像を描いたセル範囲に`skip`フラグを立てる。その後同じセルにテキストを描いても、`ratatui::widgets::Clear`でそのセルを一度クリアしない限り、変更が端末に送信されずテキストが表示されない(既存の`render_song_select`が`ratatui::widgets::Clear`をパネル範囲に描いてから曲選択パネルを描いているのは、この仕組みの上で動いている)。

このため、各画面で以下の順に描く。

1. 画面全体に背景画像を描く(`Screen::SelectSong`以外)
2. その画面の外枠(`theme::panel`等のBlockが占める範囲)に`ratatui::widgets::Clear`を描く
3. 既存の画面別描画処理(`render_menu`・`render_difficulty_select`等)をそのまま行う

この方式では、背景画像は各画面の外枠(パネル)の「外側」の余白にのみ見える。`Screen::Menu`のようにパネルがほぼ全画面を占める画面では、背景はごくわずかな端の余白にしか見えない。これは方式上の制約として許容する。

## 対象範囲

外枠(`theme::panel`相当のBlock)を持つトップレベルの画面が対象。

- `Screen::Splash`(全画面画像。背景を描いても`title.jpeg`の下に隠れて実質見えない。処理上は含めてよい)
- `Screen::Menu`・`Screen::ConfirmQuit`
- `Screen::SelectDifficulty`
- `Screen::Countdown`
- `Screen::Result`
- `Screen::History`
- `Screen::Jukebox`

`Screen::SelectSong`(TTR)は対象外(既存のスプラッシュ画像を背景にする仕組みをそのまま使う)。

`Screen::Playing(game)`(各ミニゲームのプレイ中画面)は今回のスコープ外とする。各`Game`実装がそれぞれ独自にパネルを描いており、Clear処理を個別に組み込むと実装範囲が大きくなるため、今回は対象にしない。

## 対象ファイル

- `src/app.rs`(`render()`の各画面描画箇所に、背景描画+Clearの処理を追加する)
- 新規: `src/ui/background.rs`(背景画像の読み込み・描画。`src/ui/menu_icons.rs`・`src/ui/splash.rs`の画像プロトコル対応パターンを参考にする)
- `src/ui/mod.rs`(新モジュールの追加)

## 画像描画(新規モジュール `src/ui/background.rs`)

- `assets/image/background.jpeg` を起動時に1回だけ読み込み、`StatefulProtocol`として保持する(`splash::picker()`を使って端末の画像プロトコル対応を検出。非対応・読み込めない場合は何も描かない)
- 描画は画面全体(`frame.area()`)に対して行う。`ratatui_image::Resize::Crop(None)`で画面いっぱいに敷く(`SplashRenderer`と同じ考え方)
- 画像プロトコル非対応の端末では何もしない(フォールバックの代替表示は無し。既存の各画面の黒背景のまま)

## app.rsでの適用

`App::render()`で、対象の各画面ごとに「背景描画→パネル範囲へのClear→既存の描画処理」の順にする。各画面の外枠のRectは既存の`menu_block()`・`theme::panel(...)`等の`Block`の`.inner()`計算と同じ範囲(枠込みの全体)を使う。

`Screen::Menu`・`Screen::ConfirmQuit`は、`render_menu`が呼ばれる前に画面全体(`menu_block()`が占める範囲、通常は`area`全体)へClearを描く。

`Screen::SelectDifficulty`・`Screen::Countdown`・`Screen::Result`・`Screen::History`・`Screen::Jukebox`も同様に、それぞれの外枠のBlockが占める範囲(多くは`area`全体)にClearを描いてから既存の描画関数を呼ぶ。

## Appフィールド

`App`構造体に`background: BackgroundRenderer`(仮称)を追加し、`App::new()`で1回だけ初期化する。既存の`splash_renderer`・`ttr_splash_renderer`・`menu_icons`と同じ並びに追加してよい。

## テスト観点

- `background.jpeg`が`assets/image/`に埋め込まれ、デコードできること
- `Screen::SelectSong`では背景描画が呼ばれない(または描画されても`ttr_splash_renderer`の描画で完全に上書きされる)こと
- 対象範囲に挙げた各画面で背景描画とClearの処理が呼ばれること
- 画像プロトコル非対応(フォールバック)時にパニックしないこと
- 既存の各画面のテスト(パネル位置・クリック判定・テキスト内容等)が壊れていないこと(背景描画・Clearは既存のUI描画の「前」に行うだけで、既存の位置・当たり判定・表示内容には影響しないはず)
