## 概要

DDR(リズムゲーム)を除く全ゲームで、難易度選択後・ゲーム開始前に「3」「2」「1」「GO!!」のカウントダウン演出を挟む。

## 実装方式

各ゲームの実装(`src/game/*.rs`)には手を入れない。`app.rs`の画面遷移層に、`Screen::Playing`へ入る前段階として共通の`Countdown`状態を挟む。

- カウントダウンの状態管理(残りフェーズ・経過時間)は新規モジュール`src/ui/countdown.rs`に切り出す
- `Screen`enumに`Countdown { game: Box<dyn Game>, state: CountdownState }`のようなバリアントを追加し、`update`でカウントダウンを進め、終わったら`Screen::Playing(game)`に切り替える
- DDR(`crate::game::rhythm::GAME_ID`)を開始する場合はCountdownを挟まず、これまで通り直接`Screen::Playing`にする

## 表示

- 全画面中央に大きな文字で「3」→「2」→「1」→「GO!!」の順に切り替える
- 各フェーズ0.6秒(合計2.4秒)

## 音

フェーズが切り替わるたびに既存SEを鳴らす。
- 「3」「2」「1」: `SeKind::Transition`
- 「GO!!」: `SeKind::Confirm`

## 入力

カウントダウン中はキー入力・マウス入力を無視する(スキップ不可)。
