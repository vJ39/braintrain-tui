## 概要

記憶ゲーム(memory.rs)に2点の変更を行う。

1. 初級/中級/上級の難易度選択を廃止し、3問(初級相当)→4問(中級相当)→3問(上級相当)の固定10問構成にする
2. 正誤確定後の表示(`Phase::Interval`)を、現状の小さめのテキストメッセージ(「せいかい！つぎいくよ…」「ざんねん…つぎいくよ…」)から、イロピッタン(reaction.rs)と同じ「大きな◯/✗」の表示に変更する。イロピッタンで使っている◯✗の画像表示ロジックは、記憶ゲームでも同じ見た目・同じ判定(画像プロトコル対応端末では画像、非対応端末では大きめのテキスト)にするため、共通モジュールへ切り出して両ゲームで使う

## 0. 難易度選択廃止・3/4/3問の固定10問構成

### 現状

`MemoryGame::new(difficulty: Difficulty)`が難易度を受け取り、`sequence_len(difficulty)`(3/5/7手)を全問共通で使う。セッション長は`ScoreTracker::new()`(`QUESTIONS_PER_SESSION` = 10)で、既に10問。`app.rs`の`select_menu_item`でメニュー項目(`MENU_ITEMS[5]` = "記憶(位置と色)")は`Screen::SelectDifficulty`を経由する。

### 変更内容

- `MemoryGame`から`difficulty`フィールドを削除し、`MemoryGame::new()`(引数なし)に変更する
- 何問目か(0始まりのインデックス、`tracker.total()`)に応じて、そのシーケンスの手数を決める関数(例: `fn sequence_len_for_question(question_index: u32) -> usize`)を追加する
  - 0〜2問目(1〜3問目): 3手(初級相当)
  - 3〜6問目(4〜7問目): 5手(中級相当)
  - 7〜9問目(8〜10問目): 7手(上級相当)
- `generate_sequence`・`next_sequence`は、この関数から手数を求めて使うように変更する
- HUDの難易度表示・`result()`の難易度記録用に、`pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;`を追加する(他の固定進行ゲームと同じ考え方)
- `app.rs`に`const MEMORY_ITEM_INDEX: usize = 5;`を追加し、`select_menu_item`に`MEMORY_ITEM_INDEX`分岐(難易度選択画面を経由せず`start_playing(MEMORY_ITEM_INDEX, crate::game::memory::SESSION_DIFFICULTY)`を直接呼ぶ)を追加する
- `new_game`関数の`5 => Box::new(MemoryGame::new(difficulty))`を`Box::new(MemoryGame::new())`に変更する
- 難易度選択画面の対象から外すフィルタ(`COLOR_STACK_ITEM_INDEX`等を除外している箇所)に`MEMORY_ITEM_INDEX`も追加する
- 関連テストの更新

## 1. ◯✗の大表示ロジックの共通化

### 現状

`reaction.rs`の`LabelRenderer`・`label_image_file`・`compose_label_image`・`glyph_area`等が、出題文字(色名の漢字/カタカナ)と正誤マーク(`◯`/`✗`、`maru.png`/`batsu.png`)の両方を同じ仕組みで画像表示している。この仕組みは`reaction.rs`に閉じている。

### 変更内容

- 新規モジュール`src/game/mark_display.rs`を作り、「◯/✗を画面の指定エリアいっぱいに大きく表示する」部分だけを切り出す
  - `maru.png`/`batsu.png`の埋め込み・読み込み
  - 画像プロトコル対応端末向けの画像表示(`glyph_area`・`compose_label_image`相当のロジック)
  - 非対応端末向けのテキストフォールバック表示
  - 公開インターフェースは「正誤(bool)を受け取り、指定エリアに◯/✗を描く」関数・構造体(例: `MarkRenderer`)にする
- `reaction.rs`は出題文字(色名)の画像表示はそのまま自分で持ち、◯/✗の表示部分だけ`mark_display`を呼ぶ形に置き換える。既存のイロピッタン関連テスト(◯✗表示・画像プロトコル対応)が壊れないことを確認する
- `memory.rs`は`mark_display::MarkRenderer`を新しく持ち、`Phase::Interval`中の描画で使う

## 2. 記憶ゲームの正誤表示

### 現状

`Phase::Interval { is_correct, elapsed }`の間、footerに「せいかい！つぎいくよ…」/「ざんねん…つぎいくよ…」というテキストのみを表示する。

### 変更内容

- `Phase::Interval`中は、パネルの2x2グリッドがある中央エリアに`mark_display`で大きな◯(正解)/✗(不正解)を表示する
- footerのテキストは残してよい(補足情報として)。大きな◯✗がメインの表示になる
- 正誤の効果音(`SeKind::Correct`/`SeKind::Incorrect`)・インターバル時間(`RESULT_INTERVAL`)は現状のまま変更しない

## 対象ファイル

- 新規: `src/game/mark_display.rs`
- `src/game/reaction.rs`(◯✗表示部分を`mark_display`利用に置き換え)
- `src/game/memory.rs`
- `src/game/mod.rs`(新規モジュールの宣言)
- `src/app.rs`(難易度選択スキップの配線)

## テスト観点

- `mark_display`単体で、正解=◯・不正解=✗の画像/テキストが指定エリアに描かれること(画像プロトコル対応・非対応の両方)
- `reaction.rs`の既存の◯✗表示関連テストが、共通化後も壊れずに通ること
- `memory.rs`で、`Phase::Interval`中に◯(正解時)/✗(不正解時)が描画されること
- `Phase::Interval`が明けたら◯✗表示が消え、次のシーケンスの提示フェーズに進むこと(既存の`interval_advances_to_next_sequence_after_result_interval`相当の挙動が保たれること)
- 1〜3問目は3手、4〜7問目は5手、8〜10問目は7手のシーケンスになること
- セッションが10問で終了すること
- `select_menu_item(MEMORY_ITEM_INDEX)`が難易度選択画面を経由せず直接プレイを開始すること
- `result()`の難易度が`SESSION_DIFFICULTY`(Advanced)で記録されること
