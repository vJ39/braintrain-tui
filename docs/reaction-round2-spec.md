## 概要

イロピッタンの追加改修3点。

## 1. 問題数を10→20問に

`ScoreTracker::is_session_finished`は現在、全ゲーム共通の`QUESTIONS_PER_SESSION`(10)で判定している。これを変えず、イロピッタン専用のセッション長を持てるように`ScoreTracker`へ新規コンストラクタ`ScoreTracker::with_session_length(len: u32)`を追加する(既存の`ScoreTracker::new()`は`QUESTIONS_PER_SESSION`のまま、他ゲームは無変更)。イロピッタンは`ReactionGame::new`で`ScoreTracker::with_session_length(20)`を使う。

## 2. 正解/不正解を出題文字と同じフォントで画像表示

現在、正誤フィードバック(`AnswerFeedback`/`Flash`、`src/game/feedback.rs`)は全ゲーム共通のテキスト表示。この共通コンポーネント自体は変更しない(他ゲームに影響するため)。

イロピッタン側(`reaction.rs`)で、`Flash`があるとき、出題文字の画像表示の代わりに「◯」(正解)または「✗」(不正解)の画像を、出題時と同じ仕組み(黒文字・透明背景PNG→背景色に合成→画像プロトコルで表示、非対応端末はテキストフォールバック)で表示する。フォントは既存の出題文字画像と同じ(ヒラギノ角ゴシックW6)。

- 新規画像2枚: `assets/image/reaction/maru.png`(◯)・`assets/image/reaction/batsu.png`(✗)
- 表示条件: `self.feedback.current()`が`Some`の間だけ、出題文字の代わりにこの画像を出す。表示が終わったら通常の出題文字表示に戻る
- 背景色は現在の出題(display_color)のままにする(正誤表示中も背景色は変えない)

## 3. 正解/不正解SEをイロピッタン専用の新しい音に

イロピッタン専用の新規SE種別を追加する(既存の`SeKind::Correct`/`Incorrect`は他ゲームで使われているため変更しない)。

- 正解: 「ピンポン」のような、上昇する2音のチャイム風の短い音
- 不正解: 「ブブー」のような、低めのブザー風の短い音

音源はPillow等と同様にPythonスクリプトで機械的に生成する(sine波+envelope)。生成後、`src/audio/mod.rs`の`SeKind`に新規バリアントを追加し、イロピッタンの正誤判定時にこれを再生するよう`reaction.rs`を変更する。

## スコープ外

- 他ゲームのSE・問題数・フィードバック表示は変更しない
