## 概要

イロピッタン(reaction.rs)の難易度選択(初級/中級/上級)を廃止し、記憶ゲーム(#114)と同じ考え方で「3問(初級相当)→4問(中級相当)→3問(上級相当)」の固定10問構成にする。

## 現状

`ReactionGame::new(difficulty: Difficulty)`が難易度を1つ受け取り、`difficulty`をフィールドに固定して持つ。`color_pool(difficulty)`(2/4/9色)・`pick_notation(rng, difficulty)`(上級のみ漢字/カタカナ混在)・`time_limit(difficulty)`(上級のみ3秒制限)が、その固定難易度を毎回参照する。`SESSION_LENGTH = 20`(全問共通)。`app.rs`の`select_menu_item`でメニュー項目(`MENU_ITEMS[2]` = "イロピッタン")は`Screen::SelectDifficulty`を経由する。

## 変更内容

- `ReactionGame`から`difficulty`の固定フィールドを削除し、`ReactionGame::new()`(引数なし)に変更する
- 何問目か(0始まりのインデックス、`tracker.total()`)に応じてその問題の難易度を返す関数(例: `fn question_difficulty(question_index: u32) -> Difficulty`)を追加する
  - 0〜2問目(1〜3問目): `Difficulty::Beginner`
  - 3〜6問目(4〜7問目): `Difficulty::Intermediate`
  - 7〜9問目(8〜10問目): `Difficulty::Advanced`
- `generate_question`・`next_question`・`time_limit`の呼び出し箇所は、固定の`self.difficulty`ではなく`question_difficulty(self.tracker.total())`で求めた値を使う
- `SESSION_LENGTH`を`20`から`10`に変更する
- HUDの難易度表示・`result()`の難易度記録用に、`pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;`を追加する(他の固定進行ゲームと同じ考え方)
- `app.rs`に`const REACTION_ITEM_INDEX: usize = 2;`を追加し、`select_menu_item`に`REACTION_ITEM_INDEX`分岐(難易度選択画面を経由せず`start_playing(REACTION_ITEM_INDEX, crate::game::reaction::SESSION_DIFFICULTY)`を直接呼ぶ)を追加する
- `new_game`関数の`2 => Box::new(ReactionGame::new(difficulty))`を`Box::new(ReactionGame::new())`に変更する
- 難易度選択画面の対象から外すフィルタに`REACTION_ITEM_INDEX`も追加する
- 関連テストの更新(既存の`ALL_DIFFICULTIES`を使ったテストは、難易度ごとの挙動を確かめるテストとして残しつつ、`question_difficulty`経由の呼び出しに合わせて調整する)

## 対象ファイル

- `src/game/reaction.rs`
- `src/app.rs`

## テスト観点

- 1〜3問目は`Beginner`(2色)、4〜7問目は`Intermediate`(4色)、8〜10問目は`Advanced`(9色・漢字/カタカナ混在・3秒制限)のパラメータが使われること
- 問題が進むごとに正しく次の難易度のパラメータへ切り替わること
- セッションが10問で終了すること。HUDが`Q_/10`を表示すること
- `select_menu_item(REACTION_ITEM_INDEX)`が難易度選択画面を経由せず直接プレイを開始すること
- `result()`の難易度が`SESSION_DIFFICULTY`(Advanced)で記録されること
- 既存の色プール・表記・時間制限・正誤表示・画像プロトコル関連のテストが壊れていないこと
