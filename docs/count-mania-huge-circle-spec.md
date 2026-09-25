## 概要

カウントマニアの円のサイズ段階(`CircleSize`: `Large`/`Medium`/`Small`)に、既存の`Large`よりさらに大きい段階`Huge`を追加する。

## 対象ファイル

- `src/game/count_mania/layout.rs`: `CircleSize`列挙体、`SIZE_TIERS`、`size_dims`
- `src/game/count_mania/mod.rs`: 各難易度の`DifficultyParams::size_levels`

## 実装方針

### CircleSize

`CircleSize`に`Huge`を追加する(`Huge, Large, Medium, Small`の4段階)。

### SIZE_TIERS

`SIZE_TIERS`の各組(tier)は現在`[Large高さ, Medium高さ, Small高さ]`の3要素配列。これを`[Huge高さ, Large高さ, Medium高さ, Small高さ]`の4要素配列に拡張する。既存の3段階の高さは変えず、それぞれの組の先頭にLargeより一回り大きい高さを追加する(大 > 中 > 小の関係を保ったまま、更に大きい値を足す形)。

具体的な数値の決め方は実装時に見た目のバランスで決めてよいが、「Hugeの高さ > その組のLargeの高さ」を必ず満たすこと。

### size_dims

`size_dims`のmatch式に`CircleSize::Huge => heights[0]`を追加し、既存のLarge/Medium/Smallのインデックスをそれぞれ1つ後ろにずらす。

### 各難易度への組み込み

`params()`の`size_levels`に`Huge`を追加する。

- Beginner: `[Huge, Large, Medium]`
- Intermediate: `[Huge, Large, Medium, Small]`
- Advanced: `[Huge, Large, Medium, Small]`

Hugeを含めることで、円のサイズ段階数が難易度によって3〜4種類になる。既存の「サイズ段階は順番に割り当ててからシャッフルし、どの段階も最低1つは使われるようにする」(`new_round`)というロジックはそのまま使える(`levels`配列にHugeが増えるだけ)。

## 既存ロジックへの影響確認

- `layout_circles`の面積カバー率判定(`LOOSE_MAX_COVERAGE`/`DENSE_MAX_COVERAGE`)・組を1段落とすフォールバックは、Hugeを含む組でも同じロジックで動く前提。Hugeを含む組がプレイエリアに収まらない場合、既存の「1段小さい組に落とす」フォールバックで対応する
- 既存テストで`SIZE_TIERS`や`CircleSize`の段階数を決め打ちしている箇所があれば、4段階に合わせて更新する

## テスト観点

- `size_dims`が`Huge`に対して`Large`より大きい高さ・幅を返すこと(全tierで)
- 各難易度の`size_levels`に`Huge`が含まれること
- 既存の「サイズ段階は最低1つ使われる」系のテストが4段階でも成り立つこと
- 既存のレイアウト関連テスト(重なり・収まり判定等)が壊れていないこと
