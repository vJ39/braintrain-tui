## 概要

カウントマニアに3点の変更を行う。

1. 難易度選択(初級/中級/上級)を廃止し、他の固定3ラウンドゲーム(カラーストック等)と同様に、ROUND1=初級・ROUND2=中級・ROUND3=上級と自動的に難易度が上がる固定進行にする
2. 上級(ROUND3)相当の配置が画面中央に寄りすぎている問題を修正し、特大の円をもっと大きく・画面を広く使うようにする
3. 正解クリック時の波紋(Ripple)エフェクトを、クリックした円のサイズに応じて大きく広がるようにする

## 1. 難易度選択廃止・固定3ラウンド進行

### 現状

`CountManiaGame::new(difficulty: Difficulty)`は1つの難易度を受け取り、`difficulty`・`params`をフィールドに固定して持ち、3ラウンドとも同じ難易度でプレイする。`app.rs`の`select_menu_item`で`COUNT_MANIA_ITEM_INDEX`は`Screen::SelectDifficulty`を経由して難易度を選ばせている。

### 変更内容

- `CountManiaGame`から`difficulty`・`params`の固定フィールドを削除し、代わりに「現在のラウンド(`tracker.total()`。0始まり)に対応する難易度・パラメータ」を都度求める形にする
- 新しい定数(例: `ROUND_DIFFICULTIES: [Difficulty; ROUNDS_PER_SESSION as usize] = [Difficulty::Beginner, Difficulty::Intermediate, Difficulty::Advanced]`)を追加し、ラウンドが変わるたびにこの並びに従ってパラメータを切り替える
- ラウンド開始時(`new_round`を呼ぶ箇所)は、そのラウンドに対応する難易度の`params(difficulty)`を渡す
- 画面のヘッダー表示(現在「ROUND x/3」を出している箇所)に、そのラウンドの難易度ラベル(初級/中級/上級。`theme::difficulty_label`が使えるはず)も併記する
- `result()`で記録する難易度は、他の固定3ラウンドゲーム(カラーストック)の`SESSION_DIFFICULTY`と同じ考え方で、代表値として`Difficulty::Advanced`を使う(`pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;`のような定数を追加してよい)
- `CountManiaGame::new()`を引数なしに変更する

### app.rs側の変更

- `select_menu_item`の`COUNT_MANIA_ITEM_INDEX`分岐を、既存の`COLOR_STACK_ITEM_INDEX`と同じパターン(難易度選択画面を経由せず、`start_playing(COUNT_MANIA_ITEM_INDEX, crate::game::count_mania::SESSION_DIFFICULTY)`を直接呼ぶ)に変更する
- `new_game`関数の`COUNT_MANIA_ITEM_INDEX => Box::new(CountManiaGame::new(difficulty))`を`Box::new(CountManiaGame::new())`(引数なし)に変更する
- 難易度選択画面(`Screen::SelectDifficulty`)を経由しなくなることに伴うテストの更新

## 2. 上級(ROUND3)の配置改善

### 現状の問題

`Difficulty::Advanced`の`dense: true`設定により、円は画面中央の狭い領域(`DENSE_REGION_RATIO = 0.6`、画面の60%程度)に密集配置される。さらに円の数(`max_number: 20`)に対して密集領域の面積が不足すると、面積カバー率の判定(`DENSE_MAX_COVERAGE`)によりサイズ段階(tier)が1段小さい組に落ちてしまう。結果として「特大」のはずの円が実際にはあまり大きくならず、かつ配置範囲も画面中央に限られて窮屈に見える。

### 変更内容

- ROUND3(上級)相当の配置を、画面をもっと広く使う形に調整する。具体的な調整方法(以下のいずれか、または組み合わせ)は実装時に見た目のバランスで決めてよいが、「特大の円が既存よりはっきり大きく見え、配置が画面の広い範囲を使う」ことを達成すること
  - `DENSE_REGION_RATIO`を大きくする(密集領域を画面によりに広く取る)
  - `dense`フラグの扱いを見直す(常に密集させるのではなく、上級でも配置可能な範囲を広げる)
  - `SIZE_TIERS`の特大(Huge)の値をさらに大きくする
  - 円の数(`max_number`)に対して密集判定の面積上限(`LOOSE_MAX_COVERAGE`/`DENSE_MAX_COVERAGE`)を調整し、tierが不必要に落ちないようにする
- 初級(ROUND1、`dense: false`)との対比で「上級の方が画面を広く使っている」と分かる見た目にすること

## 3. 波紋のサイズを円のサイズに連動させる

### 現状

`Ripple::new(column, row)`は中心座標のみを持ち、波紋の最大半径は定数`RIPPLE_MAX_RADIUS_CELLS`(全ての波紋で共通)で決まる。`click_correct(&mut self, column: u16, row: u16)`はクリックした円のサイズ情報を波紋に渡していない。

### 変更内容

- `Ripple`にクリックした円のサイズ(`layout::CircleSize`、または最大半径の倍率)を持たせ、`Ripple::new(column, row, size)`のようにシグネチャを変更する
- サイズ段階(特大/大/中/小)ごとに波紋の最大半径を変える。具体的な倍率は実装時に見た目のバランスで決めてよいが、「特大の円をクリックした時の波紋は、小さい円をクリックした時よりはっきり大きく広がる」ことが分かる差をつけること
- `click_correct`内で、クリックされた数字(`self.round.next`、インクリメント前)に対応する`RoundCircle`のサイズを`self.round.circles`から求め、`Ripple::new`に渡す
- `circle_image.rs`側の波紋の描画・パッチ範囲計算(`ripple_patch_rect`・`add_ripple`)も、波紋ごとの最大半径を参照するように変更する(既存の`RIPPLE_MAX_RADIUS_CELLS`を波紋から取得したサイズ依存の値に置き換える)

## 対象ファイル

- `src/game/count_mania/mod.rs`
- `src/game/count_mania/layout.rs`
- `src/game/count_mania/ripple.rs`
- `src/game/count_mania/circle_image.rs`
- `src/app.rs`

## テスト観点

- ラウンド1/2/3でそれぞれ初級/中級/上級相当のパラメータ(最大数字・ライフ・失敗時レイテンシ等)が使われること
- ラウンドが進むごとに正しく次の難易度のパラメータに切り替わること
- ヘッダーに現在のラウンドの難易度ラベルが表示されること
- `result()`の難易度が`SESSION_DIFFICULTY`(Advanced)で記録されること
- `select_menu_item(COUNT_MANIA_ITEM_INDEX)`が難易度選択画面を経由せず直接プレイを開始すること
- 上級(ROUND3)の配置が、初級(ROUND1)より明らかに広い範囲(画面に対する割合)を使うこと。特大円のサイズが既存より大きくなっていること
- 特大の円をクリックした時の波紋の最大半径が、小さい円をクリックした時より大きいこと
- 既存のカウントマニア関連テスト(GAME OVER・誤クリック・タイムアウト・波紋の基本動作等)が壊れていないこと
