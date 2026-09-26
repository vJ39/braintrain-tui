## 概要

「べー」スプラッシュ画面(#136)の静止画を、カット済みイントロ動画(8.7秒、`beigoma_intro_cut.mp4`)のコマ送りアニメーションに差し替える。ratatui-imageは静止画プロトコルしか扱えないため、動画をフレーム画像列に分解し、`ResultSprite`(#104)と同じ「全コマ事前読み込み+経過時間からコマ番号を計算」の方式で実装する。

## 変更内容

- 動画から`FRAME_DURATION`間隔(350ms、`ResultSprite`と同じ)でフレームを抽出し、`assets/image/beigoma_splash/frame_NNN.jpg`として埋め込む(8.7秒÷0.35秒 ≒ 25枚。JPEGで容量を抑える)
- `ui::beigoma_splash_anim`モジュールを新設し、`AnimatedSplash`構造体を実装する
  - 起動時に全フレームを読み込み`Vec<StatefulProtocol>`を保持(画像プロトコル非対応・読み込み失敗時は空 = フォールバック)
  - `frame_index(elapsed) -> usize`: `ResultSprite::frame_index`と同じ計算式で、最後のフレームまで行ったら最初に戻る(ループ再生。Enterで開始するまで見続けられるように)
  - `centered_image_rect`(`splash.rs`)と同じ配置計算をフレームの画像サイズで行う
  - フォールバック(画像プロトコル非対応)は`splash::BEIGOMA_FALLBACK`のテキスト表示のまま
- `App`の`beigoma_splash_renderer: SplashRenderer`を`AnimatedSplash`に置き換え、`Screen::BeigomaSplash`表示中は`update(dt)`で経過時間を進める(`ResultSprite`のように`Screen`に応じて進めるのを絞る)
- `beigoma_splash_renderer`の経過時間は`Screen::BeigomaSplash`に入るたびに0へリセットする
- 実機で画像が画面全体でなく小さく表示された不具合(#157)の修正: `font_size`を生成時に1回だけ保存せず、`SplashRenderer`と同様に`render`のたびに`picker.font_size()`を取得する。`AnimatedSplash`は`Picker`自体を保持し、`font_size`専用のフィールドは持たない

## 対象ファイル

- `assets/image/beigoma_splash/`(新規、静止画の`beigoma_splash.jpeg`は削除)
- `src/ui/beigoma_splash_anim.rs`(新規)
- `src/app.rs`

## テスト観点

- `frame_index`が経過時間に応じて0からフレーム数-1まで進み、最後の次は0に戻る(ループ)こと
- 画像プロトコル非対応・フレーム読み込み失敗時はフォールバック(テキスト)表示になること
- `Screen::BeigomaSplash`表示中のみ経過時間が進み、他の画面では進まないこと(`ResultSprite`と同様)
- `Screen::BeigomaSplash`に入るたびに経過時間が0に戻ること
- 全フレームがembedされ、デコードできること
