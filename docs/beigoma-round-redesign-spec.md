## 概要

「べー」のROUND1・ROUND2を次の内容に変更する。

- ROUND1: ゴールを「投入位置から最も遠い候補」に固定する(現状のランダム選出をやめる)
- ROUND2: 盤面配置をROUND1と同じ(`LAYOUT_ROUND1`)にする(`LAYOUT_ROUND2`は削除)
- ROUND2: ベーゴマを2個同時に投入し、共通のTiltで両方を操作する。片方でも場外・吹っ飛びになった時点で即GAME OVER、両方が同じゴールに乗った時点でクリアとする

## 変更内容

### 1. ROUND1: ゴールの最遠固定

`GoalRule`(候補の条件)と選出方法(候補からどう1つ選ぶか)を分離する。`GoalRule`は現状のまま候補集合を決める役割に留め、新たに選出方法を表す型を`RoundParams`に持たせる。

```rust
/// 候補からゴールを1つ選ぶ方法
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GoalSelection {
    /// 候補からランダムに1つ選ぶ
    Random,
    /// 投入位置から最も遠い候補に固定する(同着なら候補の並び順=左上から行優先で先に出てくる方)
    Farthest,
}

pub struct RoundParams {
    pub label: &'static str,
    pub layout: &'static [&'static str; BOARD_HEIGHT],
    pub goal_rule: GoalRule,
    pub goal_selection: GoalSelection,
    pub top_count: usize,
}
```

`Board::generate`は`goal_selection`で分岐する。

```rust
let candidates = Self::goal_candidates(params.layout, &params.goal_rule);
let goal = match params.goal_selection {
    GoalSelection::Random => candidates[rng.gen_range(0..candidates.len())],
    GoalSelection::Farthest => farthest_candidate(&candidates, from),
};
```

`farthest_candidate`は候補配列と投入位置からユークリッド距離が最大の1点を返す純粋関数(候補は`goal_candidates`の並び順=左上から行優先で並んでいるため、厳密不等号`>`で更新することで同着時は先に出てきた方を残す)。

ROUND1は`goal_rule`を変更せず(`min_distance: 10.0, blocked_straight_line: false`)、`goal_selection: GoalSelection::Farthest`にする。ROUND2は`goal_selection: GoalSelection::Random`のまま(2個同時操作という別の難易度軸があるため、ゴール自体はランダムで維持する)。

`LAYOUT_ROUND1`・`goal_rule(min_distance:10.0, blocked_straight_line:false)`の下で実際に候補を計算すると、最遠の候補は`(19, 0)`(投入位置から距離約20.6、候補124件中で一意に最大)になる。同着は発生しない。

### 2. ROUND2: 盤面配置をLAYOUT_ROUND1に統一

`LAYOUT_ROUND2`(縁の凹凸の輪)を削除し、ROUND2も`layout: &LAYOUT_ROUND1`にする。

`LAYOUT_ROUND1`に対して既存のROUND2の`goal_rule`(`min_distance: 6.0, blocked_straight_line: true`)をそのまま適用しても候補は124件あり(投入位置からの直線上に凹凸がある候補が十分に存在する)、`goal_rule`自体は変更しない。

### 3. ROUND2: ベーゴマ2個の同時操作

**データ構造**

`BeigomaGame`の`top: Top`を、ゴール到達済みかどうかを個別に持てる形へ変える。

```rust
/// 1個のベーゴマの進行状態
struct TopSlot {
    top: Top,
    /// 個別にゴールへ達し、以後の物理更新を止めているか(他のベーゴマが揃うのを待つ)
    settled: bool,
}

pub struct BeigomaGame {
    ...
    tops: Vec<TopSlot>,
    ...
}
```

要素数は固定長ではなく`RoundParams.top_count`(ROUND1=1、ROUND2=2)から作る。ROUND1は要素数1の`Vec`として扱い、ロジックを共通化する(ROUND数ごとの分岐を増やさない)。

**投入位置**

2個は投入位置(`board.start_position()`)を中心に左右へ`TOP_PAIR_OFFSET_X`だけ離した位置に投入する(同じ位置に重ねて出すと、以降もタイル同士が完全に同じ物理・同じ入力を受け続けるため常に重なったまま動き、2個にした意味が無くなる)。

```rust
/// 2個投入時、投入位置から左右にこの分だけ離す(セル単位)。
/// 投入マスの内側(中心±0.5)に収まり、両方とも隣接マスへはみ出さない値
pub const TOP_PAIR_OFFSET_X: f64 = 0.15;

impl Board {
    /// count個のベーゴマの投入位置。1個なら投入位置そのまま、2個なら左右にTOP_PAIR_OFFSET_Xずつ離す
    pub fn start_positions(&self, count: usize) -> Vec<(f64, f64)>
}
```

わずかな初期位置のずれにより、2個は同じマスに入る/出るタイミングがずれ、片方だけ先に凸凹に触れて摩擦が変わることで軌道が分岐していく(以後は完全に独立した物理として扱う。2個の間の当たり判定は無い)。

**GAME OVER・クリア判定**

`step()`は`settled`でない`TopSlot`だけを物理更新し、どのスロットで何が起きたかを`(index, StepEvent)`として集める。

```rust
fn step(&mut self, dt: Duration) {
    self.tilt.update(dt);
    self.truck.update(dt);
    let g = self.truck.current_g();
    let mut events = Vec::new();
    for (i, slot) in self.tops.iter_mut().enumerate().filter(|(_, s)| !s.settled) {
        if let Some(event) = slot.top.step(&self.board, dt, &self.tilt, g) {
            events.push((i, event));
        }
    }
    self.elapsed += dt;
    self.on_step_events(events);
    if self.is_playing() && self.elapsed >= TIME_LIMIT {
        self.finish(Outcome::TimeUp);
    }
}
```

`on_step_events`の判定順序(1ステップ内に複数の出来事が重なった場合も含む):

1. いずれかのイベントが`Landed(Flown)` / `Grazed(Flown)` / `FellOff`(=GAME OVER相当)なら、他のスロットの状態に関わらず即`finish(Outcome::Flown)`で終える
2. GAME OVERが無ければ、`Goal`イベントが来たスロットの`settled`を`true`にする。全スロットが`settled`になった時点で`finish(Outcome::Cleared { time: self.elapsed })`
3. GAME OVERでもクリアでもない残りのイベントからは、盤上の一言(下記)を1つ選んで`self.message`にする

ROUND1(要素数1)ではこの分岐がそのまま従来の単一ベーゴマの判定と同じ結果になる。

**盤上の一言のメッセージ**

同じステップで複数のベーゴマが別々の出来事を起こした場合に備え、一言の重大度を固定の優先順位で決める(低→高: セーフ < ガタッ < ズボッ/ぬけた! < ぴよーん!!ああっ!!)。同じステップの出来事から最も重大度の高いものを1つ選び、その文言だけを`self.message`に出す。SE(`SeKind::Incorrect`)も選ばれた出来事が弾かれ系の時だけ1回鳴らす(2個同時に弾かれても二重に鳴らさない)。

**演出との整合**

GAME OVER(`Outcome::Flown`)時の星の演出(`star_frame`)は、原因になったベーゴマだけでなく、その時点で残っている(settledでない)全ベーゴマに対して表示する(共通のTiltで一蓮托生というROUND2のテーマに合わせる)。個別にゴールへ達して待機中のベーゴマの見た目は通常時と変えない。

### 4. render.rs: 複数ベーゴマの描画

`BoardRenderer::render`はベーゴマ1個ではなく`tops: &[TopView]`を受け取る形に変える。

```rust
pub fn render(&self, frame: &mut Frame, area: Rect, board: &Board, tops: &[TopView], tilt: &Tilt)
```

テキスト表示は`tops`をループして1個ずつ`put_glyph`する(ROUND1は要素数1のスライスを渡すだけで従来通り)。

画像表示のパッチキャッシュは、ベーゴマ1個ぶんの`PatchCache`を前提にしていたのを、スロットごとに独立したキャッシュへ拡張する。

```rust
patch: RefCell<Vec<Option<PatchCache>>>,
```

`tops`の要素数ぶんの`Vec`として扱い、各スロットは自分のマス・飛び上がり状態が変わった時だけ自分のパッチを作り直す(他スロットの変化では作り直さない)。要素数がROUND1→ROUND2で1→2に変わる時は`Vec`を作り直す。

### 5. HUD表示

ROUND2のベーゴマが2個であることは、`RoundParams.label`に含めて表す。

```rust
label: "ROUND 2 むずかしい ベーゴマ2個",
```

既存の`render_hud`(パネルタイトルに`params.label`をそのまま出す仕組み)を変更せずに反映できる。

### 6. 既存仕様との整合

ROUND1が失敗(吹っ飛び・場外・時間切れ)で終わった場合はROUND2へ進まずセッション終了する、という既存の確定仕様は変えない。ROUND1は`top_count: 1`なので、上記の判定ロジックは従来のセッション終了条件と同じ結果になる。

## 対象ファイル

- `src/game/beigoma/board.rs`: `GoalSelection`追加、`RoundParams`に`goal_selection`・`top_count`追加、`farthest_candidate`関数追加、`Board::start_positions`追加、`TOP_PAIR_OFFSET_X`定数追加、`LAYOUT_ROUND2`削除、`round_params`の内容更新、モジュール冒頭コメント更新
- `src/game/beigoma.rs`: `top: Top`→`tops: Vec<TopSlot>`、`start_round`/`drop_top`の投入位置生成、`step`/`on_step_events`のマルチベーゴマ対応、メッセージ優先順位関数、`star_frame`の対象拡張、モジュール冒頭コメント更新
- `src/game/beigoma/render.rs`: `BoardRenderer::render`の引数を`tops: &[TopView]`に変更、`patch`キャッシュの`Vec`化

## テスト観点

**ゴール選出(board.rs)**

- ROUND1: 複数シードで`Board::generate`しても常に同じゴール(`(19, 0)`)になること
- ROUND1: そのゴールが`goal_candidates(round_params(0).layout, &round_params(0).goal_rule)`に含まれること
- `farthest_candidate`の同着時の挙動: 距離が同じ候補が複数ある合成データで、並び順が先の候補が選ばれること
- ROUND2: `goal_candidates(round_params(1).layout, &round_params(1).goal_rule)`が空でないこと(`LAYOUT_ROUND1`に対する`blocked_straight_line`条件の充足)、複数シードでゴールが変わること(ランダムのまま)

**ROUND2のベーゴマ2個(beigoma.rs)**

- ROUND2開始直後: `tops`の要素数が2、2個の初期位置が投入位置を挟んで左右に`TOP_PAIR_OFFSET_X`ずれていること
- 片方だけが場外(`FellOff`)/吹っ飛び(`Landed(Flown)`)になったら、もう片方が無事でも即`Outcome::Flown`になること
- 両方が同じステップで場外・吹っ飛びになっても正しく`Outcome::Flown`になり、二重に`finish`が走らないこと
- 片方だけゴールに到達した時点ではまだ`Playing`のままで、`Outcome`が確定しないこと。その後もう片方もゴールに到達した時点で`Outcome::Cleared`になること
- 両方が同じステップでゴールに到達したら、その場で`Outcome::Cleared`になること
- 同じステップで片方がゴール・もう片方が場外/吹っ飛びになったら、GAME OVER側が優先されて`Outcome::Flown`になること
- ゴールに到達して`settled`になったベーゴマは、以後`update`を重ねても位置が動かないこと(物理更新が止まっていること)
- 同じステップで片方が弾かれ・もう片方が軽い着地だった場合、一言は弾かれ側の文言になり、SEは1回だけ鳴ること
- ROUND1(要素数1)は上記の判定がすべて既存の単一ベーゴマの挙動と一致すること(`round1_failure_ends_the_session_without_round2`等の既存テストが変更なく通ること)

**描画(render.rs)**

- ROUND2の盤面に2個のベーゴマ記号が同時に描かれ、それぞれの位置に対応していること
- ROUND1の盤面には従来通りベーゴマ記号が1個だけ描かれること
- 画像表示: 片方のベーゴマだけがマスを移動した時、そのスロットのパッチだけが再エンコードされ、もう片方のパッチ再エンコード回数は変わらないこと
- 画像表示: ROUND1→ROUND2でベーゴマの個数が1→2に変わっても、パニックせず両方が描画されること
- 小さい描画エリアでROUND2(ベーゴマ2個)を描いてもパニックしないこと

**HUD**

- ROUND2の表示に「ベーゴマ2個」が含まれ、ROUND1には含まれないこと
