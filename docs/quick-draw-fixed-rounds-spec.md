## 概要

反射神経(quick_draw)に5点の変更を行う。

1. 難易度選択(初級/中級/上級)を廃止する。他の固定ラウンド制ゲーム(カラーストック・カウントマニア)と同様に難易度選択画面を経由せず直接プレイを開始する
2. 1セッションの問題数を3問から10問に増やし、待機時間のパターンに「めっちゃ短い」ものも用意する
3. フライング時の扱いは、現状のコードが既に満たしている仕様(GAME OVERにはならず、MISSとして数え正答数はインクリメントしない)を維持し、テストで明示的に固定する
4. 1問ごとに「3.2.1.GO!!」のカウントダウン演出を挟む
5. ゲーム名を「反射神経」から「ハヤウチ」に変更する

## 1. 難易度選択廃止

### 現状

`QuickDrawGame::new(difficulty: Difficulty)`が難易度を受け取り、`wait_range(difficulty)`・`fail_latency_ms(difficulty)`がそれぞれ難易度ごとの値を返す。`app.rs`の`select_menu_item`で`QUICK_DRAW_ITEM_INDEX`は`Screen::SelectDifficulty`を経由する。

### 変更内容

- `QuickDrawGame`から`difficulty`フィールドを削除し、`QuickDrawGame::new()`(引数なし)に変更する
- `wait_range()`・`fail_latency_ms()`は引数なしの関数(または定数)にする。値は「2. 問題数10問化」の待機時間パターンに統合する
- `pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;`を追加し、`result()`で記録する難易度・HUD表示に使う代表値とする(カラーストック・カウントマニアと同じ考え方)
- `app.rs`の`select_menu_item`に`QUICK_DRAW_ITEM_INDEX`分岐を追加し、`COLOR_STACK_ITEM_INDEX`と同じパターン(`start_playing(QUICK_DRAW_ITEM_INDEX, crate::game::quick_draw::SESSION_DIFFICULTY)`を直接呼ぶ)にする
- `new_game`関数の`QUICK_DRAW_ITEM_INDEX => Box::new(QuickDrawGame::new(difficulty))`を`Box::new(QuickDrawGame::new())`に変更する
- 難易度選択画面の対象から外すヘルパー(`difficulty_select_game_items()`等、既存の`COLOR_STACK_ITEM_INDEX`/`COUNT_MANIA_ITEM_INDEX`を除外しているfilter)に`QUICK_DRAW_ITEM_INDEX`も追加する
- 関連テストの更新(難易度選択画面を経由しなくなることに伴う既存テストの修正)

## 2. 問題数10問化・待機時間パターンの追加

### 現状

`ROUNDS_PER_SESSION = 3`。待機時間は難易度ごとの範囲(初級1000-2500ms/中級1000-4000ms/上級800-5000ms)から一様ランダムに選ぶ。

### 変更内容

- `ROUNDS_PER_SESSION`を`3`から`10`に変更する
- 難易度の代わりに、待機時間のパターンを2種類用意する
  - 通常パターン: `800ms 〜 4000ms`(現状の中級相当をベースにする)
  - 超短時間パターン: `200ms 〜 500ms`(めっちゃ短い。合図がほぼ待たずに来る)
- 各ラウンド開始時にランダムでどちらのパターンを使うか選ぶ(超短時間パターンの出現率は実装時に見た目のバランスで決めてよいが、10問中に体感できる頻度で登場すること。目安は20%程度)
- `fail_latency_ms()`は、そのラウンドで選ばれたパターンの最大待機時間をフライング時のペナルティ値として使う(パターンごとに変わる。現状の「難易度の最大待機時間と同じにする」設計をパターン単位に置き換える)
- HUD表示の問題数を`Q1/10`のように10問中の表示にする(`ScoreTracker::with_session_length`の値を変更するだけで対応できるはず)

## 3. フライング時の扱い(現状維持・テストで固定)

### 現状の確認

`press()`内でフライング時は`self.tracker.record(false, fail_latency_ms(...))`を呼ぶのみで、GAME OVER相当の処理は無い。`is_finished()`は`tracker.is_session_finished()`(全問消化したか)のみで判定しており、フライングでセッションが終了することはない。つまり、既存コードは「フライング=MISS(correctを増やさず、GAME OVERにもならない)」という仕様を既に満たしている。

### 変更内容

- 実装上の変更は無し。10問化・待機時間パターン変更後も、フライング=MISS・GAME OVERにならないという既存の挙動が壊れていないことを回帰テストで確認する

## 4. 1問ごとのカウントダウン演出

### 現状

セッション全体の開始前に1回だけ、`app.rs`側(`Screen::Countdown`)で`ui::countdown::CountdownState`を使ったカウントダウンを挟んでいる。`QuickDrawGame`内部には各ラウンド(合図待ち)ごとのカウントダウンは無く、`press()`で回答した直後すぐに次の待機(`Phase::Waiting`)に入る。

### 変更内容

- `QuickDrawGame`内部の`Phase`に、既存の`ui::countdown::CountdownState`をそのまま使うカウントダウン状態(例: `Phase::Countdown { state: CountdownState }`)を追加する
- 最初のラウンド開始時(`new()`)、および`press()`後の次ラウンド開始時は、いきなり`Phase::Waiting`にせず、まず`Phase::Countdown`から始める
- `update()`で`Phase::Countdown`中は`CountdownState::tick`を呼び、カウントダウンが終わったら`Phase::Waiting`(通常の合図待ち、ランダム待機)へ進む
- カウントダウン中の入力(Enter/Space/クリック)は、合図がまだ来ていない点で待機中と同じなので、フライング扱いにする
- `render()`で`Phase::Countdown`中は`ui::countdown::render`を使って「3→2→1→GO!!」を表示する

## 5. 名称変更: 反射神経 → ハヤウチ

### 変更内容

- `app.rs`の`MENU_ITEMS`・`MENU_DESCRIPTIONS`内の「反射神経」を「ハヤウチ」に変更する
- `quick_draw.rs`の`render()`でHUDタイトルに使っている「反射神経」の文字列リテラルを「ハヤウチ」に変更する
- `GAME_ID`(`"quick_draw"`)・モジュール名・関数名等の内部識別子は変更しない(#50のイロピッタン改名と同じ方針。統計・履歴の互換性を保つ)
- 既存テストで「反射神経」という文字列を使っているアサーションを「ハヤウチ」に更新する

## 対象ファイル

- `src/game/quick_draw.rs`
- `src/app.rs`

## テスト観点

- `QuickDrawGame::new()`が難易度なしで生成できること
- 待機時間が通常パターン・超短時間パターンのいずれかの範囲に収まること、超短時間パターンが十分な頻度で出現すること
- フライング時のペナルティ値が、そのラウンドのパターンの最大待機時間と一致すること
- セッションが10問で終了すること、HUDが`Q_/10`を表示すること
- フライングしてもセッションが終了せず、次のラウンドの待機に進むこと。正答数(correct)はインクリメントされないこと
- `result()`の難易度が`SESSION_DIFFICULTY`(Advanced)で記録されること
- `select_menu_item(QUICK_DRAW_ITEM_INDEX)`が難易度選択画面を経由せず直接プレイを開始すること
- 各ラウンド開始時にカウントダウンフェーズから始まること。カウントダウン中の入力はフライング扱いになること。カウントダウン終了後、通常の待機フェーズに進むこと
- メニューの項目名・HUDのタイトルが「ハヤウチ」になっていること。`GAME_ID`は`"quick_draw"`のまま変わらないこと
- 既存のフライング・反応時間記録・描画関連のテストが壊れていないこと
