# シタケシ: ROUND制廃止・60秒タイムアタック化

## 変更概要
- ROUND1〜3の3ラウンド制を廃止し、単一セッション・制限時間60秒のタイムアタックにする
- 盤面構成は既存ROUND1/2と同じ「4列・列ごとに専用色」(BoardRule::Lanes)を採用する。1列(SingleColumn)構成は使わない
- ブロックは無限に生成される。正解して1個消えたら、その列の一番上に同じ専用色のブロックを即座に補充し、常に一定の高さを保つ。ミス(違う色を押す)は既存仕様通りその列の専用色を一番上に追加する(段が増える)
- ミスをしても盤面がスタック(積み上がって詰む)して詰みになる処理は追加しない。ブーイングSEを鳴らすだけ
- リザルトは「消せた数」と「平均応答時間」を表示する(既存のGameResult.correct/avg_latency_msをそのまま使う。項目自体は変えない)

## スコアリング方針
- 正解(ブロックを1個消す)ごとに `tracker.record(true, latency_ms)` を呼ぶ
- ミスは既存仕様通り `tracker.record` を呼ばない(feedbackのSE・表示のみ)。したがって `GameResult.correct == GameResult.total == 消せた枚数`
- `latency_ms` は「直前に1個消してからの経過時間」(セッション開始直後の1個目は、食事開始ならぬボード表示開始からの経過時間)。消すたびに経過時間をリセットする
- `is_finished()` は `elapsed_total >= TIME_LIMIT` で判定する(ラウンド数での判定を廃止)

## 定数変更
- 削除: `ROUNDS_PER_SESSION`, `ROUND_INTERVAL`, `TIME_PER_ROW`, `time_limit_for`
- 追加: `TIME_LIMIT: Duration = Duration::from_secs(60)` (セッション全体の制限時間)
- 追加: `INITIAL_HEIGHT: usize` (開始時の各列の段数。既存の `ROUND1_HEIGHT`(36) と同じ値でよい)
- 維持: `LANE_COUNT`, `MAX_CELL_WIDTH`, `MAX_ROW_HEIGHT`, `BUTTONS_HEIGHT`

## 構造変更
- `BoardRule` enum・`RoundParams.rule`フィールドを削除(4列固定になるため、ルール分岐そのものが不要)
- `RoundParams` → 盤面が単一構成になるため、構造体自体を削除し、`LANE_COUNT`/`INITIAL_HEIGHT`/`MAX_ROW_HEIGHT` を直接参照する形に簡略化する。`round_params()`関数・`label`フィールドも削除
- `press_single`関数・`Board::single_column`関数を削除(SingleColumn構成を使わなくなるため)
- `Round`構造体を廃止し、`board: Board` と `elapsed_since_clear: Duration` を `ColorStackGame` に直接持たせる
- `ColorStackGame`:
  ```rust
  pub struct ColorStackGame {
      board: Board,
      /// 直前にブロックを消してからの経過時間(応答時間の測定に使う)
      elapsed_since_clear: Duration,
      /// セッション開始からの経過時間。TIME_LIMITを超えたら終了
      elapsed_total: Duration,
      tracker: ScoreTracker,
      feedback: AnswerFeedback,
  }
  ```
  `round_index`・`params`・`interval`フィールドは削除

## 主要メソッドの変更方針
- `new()`: 4列・INITIAL_HEIGHT段の盤面を1回だけ生成する
- `is_playing()`: `!self.is_finished()`
- `press_button(index)`:
  - `press_lane(&mut self.board, index)` で既存通り判定
  - ミス(`PressOutcome::Added`): `ColorStackMiss` SE・`feedback.record(false, "+1段")` を鳴らして終わる(既存仕様のまま)
  - 正解(`PressOutcome::Removed`): `ColorStackClear` SE、消した列(`index`)の一番上に専用色を1個補充(`self.board.rows.push(lane_row(index))`)、`elapsed_since_clear` を latency として `tracker.record(true, ...)`、`feedback.record(true, ...)`、`elapsed_since_clear` を0にリセット
- `update(dt)`: `feedback.tick(dt)` → `is_finished()`ならreturn → `elapsed_total += dt`, `elapsed_since_clear += dt`(ラウンド間インターバル分岐は削除)
- `is_finished()`: `self.elapsed_total >= TIME_LIMIT`
- `result()`: 変更なし(`tracker.to_result(GAME_ID, SESSION_DIFFICULTY)`)
- `render_hud`: 「ROUND x/y」表示を「残り{}秒」表示に置き換える。左欄は「消せた {}枚」(`tracker.total()`)、中央は残り秒数(feedback表示中はそちらを優先、既存の分岐パターンを踏襲)、右欄は現状通り「残り {}個」(盤面に今見えているブロック数、無限生成なので参考値程度の意味になるがそのまま残してよい)
- `render_interval_message`: 削除(ラウンドクリア/次ラウンド案内が不要になるため)
- `render_board`・`button_number`・`render_buttons`: `self.params.columns()`/`self.params.height`/`self.params.max_row_height`/`self.params.colors()` の参照を `LANE_COUNT`/`INITIAL_HEIGHT`/`MAX_ROW_HEIGHT`/`StackColor::ALL` の直接参照に置き換える
- `new_board(rng)`: `RoundParams`を受け取らず、`LANE_COUNT`列・`INITIAL_HEIGHT`段の4列盤面を1つ生成するだけに簡略化する

## 影響ファイル
- `src/game/color_stack.rs`: 本体。定数・構造体・メソッド・テストの大部分を書き換える
- `src/app/game_start.rs`: `assert_color_stack_round1_is_playing`内の `assert!(text.contains("ROUND1/3"), "ROUND1から始まる");` はROUND表示が無くなるため削除する(周辺のGAME_ID・「シタケシ」文言チェックは維持)

## テスト方針(TDD)
四谷方式(設計→テスト→実装)で進める。既存テストのうち以下は書き直しが必要:
- ラウンド遷移・ROUND_INTERVAL関連のテスト(`game_at`, `round_index`を参照するもの、`ROUNDS_PER_SESSION`回プレイしてセッション終了を確認するもの等)は「60秒の制限時間で終了する」「時間切れになるまで正解しても消せた数が増え続ける」テストに置き換える
- 新規で以下を確認するテストを追加する:
  - 正解するたびに消した列の高さが変わらない(即座に補充される)こと
  - ミスすると列の高さが1段増えること、ただしそれによってゲームが終了(詰み)しないこと
  - 60秒経過で `is_finished()` が true になること
  - `result().correct == result().total`(ミスは記録されない)であること
  - `result().avg_latency_ms` が消すたびの間隔から妥当に計算されること
- `grid_geometry`関連のテスト(`grid_for`ヘルパー)は`round_params`ではなく新しい定数を直接使うよう書き換える
