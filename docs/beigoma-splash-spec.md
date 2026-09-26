## 概要

「べー」選択時に専用スプラッシュ画面(画像: べーロゴ+荷台のキャラ)を挟む。TTR(#89)と同じ`SplashRenderer`の仕組みを流用する。

## 変更内容

- `ui/splash.rs`に`BEIGOMA_SPLASH_IMAGE_PATH`(`"beigoma_splash.jpeg"`)・`BEIGOMA_FALLBACK`を追加する
- `assets/image/beigoma_splash.jpeg`に画像を配置する
- `app.rs`の`Screen`に`BeigomaSplash`を追加する
- `App`に`beigoma_splash_renderer: SplashRenderer`フィールドを追加し、`new()`で初期化する
- `select_menu_item(BEIGOMA_ITEM_INDEX)`は、これまで直接`start_beigoma()`を呼んでいたのをやめ、Transition SEを鳴らして`Screen::BeigomaSplash`へ遷移する
- `Screen::BeigomaSplash`でEnter(`handle_key`)・クリック(`handle_mouse`)すると、Confirm SEを鳴らして`start_beigoma()`を呼ぶ(Screen::Splashの`leave_splash`と同じパターン)
- `render`に`Screen::BeigomaSplash`の描画(Screen::Splashと同じ、パネル枠+中央の画像)を追加する

## 対象ファイル

- `src/ui/splash.rs`
- `src/app.rs`

## テスト観点

- `select_menu_item(BEIGOMA_ITEM_INDEX)`後は`Screen::BeigomaSplash`になり、まだPlaying画面にならないこと
- `Screen::BeigomaSplash`でEnterを押すとROUND1のカウントダウンからPlaying画面が始まること(既存の`assert_beigoma_round1_countdown_in_game`と同じ確認)
- 画像アセットが埋め込まれ、デコードできること
