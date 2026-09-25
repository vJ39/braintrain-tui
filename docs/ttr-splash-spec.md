## 概要

TTR(旧「リズム(DDR風)」)のメニュー項目を選ぶと、既存の曲選択画面(`Screen::SelectSong`)に入る前に、TTR専用のスプラッシュ画面を挟む。

## 画像

`assets/image/ttr_splash.jpeg`(配置済み)を全画面表示する。既存のタイトル画面(`src/ui/splash.rs`の`SplashRenderer`)と同じ仕組み(画像プロトコル対応端末では画像、非対応端末はテキストフォールバック)を使う。

## 実装方針

- `SplashRenderer`を画像パス(と非対応端末用のフォールバックの見出しテキスト)を引数に取れるよう汎用化し、既存のタイトル画面とTTRスプラッシュ画面の両方で使い回す
- `App`の`Screen`enumに新規バリアント(例: `Screen::RhythmSplash`)を追加する
- メニューでTTR項目を選ぶと、直接`Screen::SelectSong`にする現行の遷移を、まず`Screen::RhythmSplash`に変え、そこでEnter/クリックすると`Screen::SelectSong(0)`に進む(既存のSplash→Menuの操作感に合わせる)
- `[q]`を押すとメニューへ戻る(既存の全画面共通の仕組みに合わせる)

## スコープ外

- 曲選択画面(`SelectSong`)・難易度選択・プレイ中画面のデザインは変更しない(#90で別途検討)
