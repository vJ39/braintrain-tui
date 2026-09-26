## 概要

「べー」のベーゴマに半径`TOP_RADIUS = 0.5`マス(直径1マス)を持たせ、見た目と当たり判定の両方をこの半径で決める。

1. 見た目(テキスト表示): ベーゴマを「位置を含むマスの左のセルに置いた1セルの記号」ではなく、実際の位置`pos`を中心とする半径`TOP_RADIUS`の円盤として塗り、その上に回転の記号を置く。円盤はマス座標で定義し、背景と同じ逆変換で塗るので、1マスが2×1セルでも10×5セルでも「マスと同じ大きさ」に見え、傾けると盤と一緒に変形する。盤の大きさ(`BOARD_WIDTH`=20・`BOARD_HEIGHT`=12)、1マスの基本比率(横2×縦1セル)、`board_area`の計算、`MAX_CELL_SCALE`=5は変えない。画像表示(`render_image`・`patch_image`)は変えない
2. 当たり判定(`board.rs`): 凹凸への接触を「中心が凹凸のマスに入った瞬間」から「円盤が凹凸のマスの四角形に重なった瞬間」に変える。接触の法線は、跨いだ縁ではなく「中心から四角形の最も近い点への向き」で求め、正面/斜めの判定(`edge_contact_from_cos`・`GRAZE_COS`)、凸の飛び上がり/側面接触、凹のハマり/側面接触、着地の3段階はそのまま使う。ゴール・場外・足元の摩擦は従来どおり中心の位置で判定する

## 見た目の決まり(テキスト表示)

- 円盤の半径`r`: 転がっている間・ハマっている間は`TOP_RADIUS`(0.5)、飛び上がっている間は`TOP_RADIUS × AIRBORNE_DISC_SCALE`(0.35)
- 円盤に含めるセル: セルの4隅のセル座標を逆変換したマス座標の外接矩形と`pos`の距離(矩形の中なら0)が`r`未満のセル。そのセルは記号を空白にして背景色を`TOP_BG`にする(凹凸・ゴールの記号は円盤の下に隠れる)
- 記号: `pos`を順変換したセル座標に`glyph_cell`をかけたセルへ、回転の記号(`TOP_SPIN_GLYPHS`)または飛び上がりの記号(`TOP_AIRBORNE_GLYPH`)を`TOP_FG`・太字で置く。このセルは必ず円盤の中にある。複数のベーゴマが同じセルになった時は従来どおり右隣へずらす(`lanes_by`)
- 星の演出中(`star_frame`が`Some`): 円盤は描かず、星の記号だけを`STAR_FG`で描く(従来の描き方)
- 中心が盤の上(`Board::contains(pos)`)の間だけ円盤を描く。円盤が盤の縁からはみ出す分は、パネル(`area`)の中であれば盤の外側のセルにも塗る。中心が盤の外の時は記号だけ(従来どおりパネルの範囲に収めて描く)
- 1マス=2×1セル: 円盤は横2〜3セル×縦1〜2セル。マスの中央にいる時はそのマスの2セルちょうど。行をまたぐ位置では上下2行になり、消えることはない
- 1マス=4×2セル: 横4〜5セル×縦2〜3セル。角のセルは含まれず、丸みが出る
- 1マス=10×5セル: 横10〜11セル×縦5〜6セルの円
- 傾けた時: 円盤の範囲は逆変換したマス座標で決めるので、盤と同じ台形の変形に従う
- 画像表示: 変更なし(`top_rect`・`patch_image`・0.95/0.7の倍率はそのまま)

## 当たり判定の決まり

### 触れている

- 中心`pos`からマス`(cx, cy)`の四角形`[cx, cx+1] × [cy, cy+1]`までの距離を`d`とする(四角形の中なら0)。`d < TOP_RADIUS`なら触れている。`d == TOP_RADIUS`ちょうどは触れていない(隣の行・列の中心線の上を真っ直ぐ進む間は触れない)
- 触れている凹凸の集合は`Top`が持つ(`touching`)。転がっている間、毎ステップ`Board::contacts(pos)`で今のステップの集合を求め、前のステップの集合に無かったマスがあれば「入った」とする
- 同じステップで2つ以上に新しく触れた時は、距離が最小のもの(同じなら左上から行優先で先のもの)だけ判定し、残りは触れている扱いにする(その接触が続く間は判定しない)
- 集合を持ち直す時(その位置で触れている凹凸を全部「触れている」にする): 置いた時(`Top::new`)、着地した時、弾かれた直後、凹から抜けた時
- 1ステップの移動量は最大`MAX_SPEED × 0.01秒 = 0.15`マスで`TOP_RADIUS`より小さいので、触れ始めを飛び越さない
- 弾かれる時の`BOUNCE_KICK`(0.5)は`TOP_RADIUS`以上なので、弾かれた直後は同じ凹凸に触れていない

### 法線と正面/斜め

- 法線`n`: `pos`から見た、四角形の最も近い点への単位ベクトル。縁に触れた時は軸に平行(左の縁なら(1, 0)、上の縁なら(0, 1))、角に触れた時は角への向き。中心が四角形の中(`d == 0`)なら零ベクトル
- `cos = (v · n) / |v|`。`GRAZE_COS`(0.5)以上なら正面、未満なら斜め(`edge_contact_from_cos`は変えない)。`n`が零・速度が零なら正面
- 凹凸の隣の行・列を横に通る時は、縁ではなく先に角に触れる。中心と縁の延長線の距離`h`が`TOP_RADIUS × sin 60° ≈ 0.433`より大きければ斜め(角の法線と速度のなす角が60°を超える)、`0.433`以下なら正面。`h`が`TOP_RADIUS`以上なら触れない

### 凸

- 正面、または斜めでも速さが`BUMP_GRAZE_SPEED`未満: 飛び上がる(従来どおり)。飛んでいる間は接触を判定しない
- 斜めで速い: 側面接触(従来どおり)

### 凹

- 正面: ハマる。中心を凹のマスの最も近い縁まで動かし(`clamp_into_cell`。動く量は最大`TOP_RADIUS`)、`Sunk { cell }`にする。以後は従来の`struggle`(凹のマスの中に留め、速さが`HOLLOW_EXIT_SPEED`に届いたら抜ける)
- 斜め: 側面接触(従来どおり)
- 抜けた時: その位置で触れている凹凸を全部「触れている」にする(抜けた直後に同じ凹へ入り直さない)

### 着地

- 着地の3段階(軽い着地/弾かれ/吹っ飛び)は従来どおり。弾かれるなら先に`bounce()`する
- 吹っ飛び以外で、その位置で触れている凹があれば、最も近い凹にハマる(中心をその凹の縁まで動かす)。中心が凹のマスの中にある場合(従来の条件)も含む
- それ以外は、その位置で触れている凹凸を全部「触れている」にする(同じ凸の上で飛び上がり直さない)

### 変えないもの

- ゴール: 中心の位置がゴールのマスにあれば入ったとみなす
- 場外: 中心が縁を越えたら落ちる(円盤の縁がはみ出していても落ちない)
- 足元の摩擦: `surface_friction(board.cell_at(pos), ...)`(中心のマス)
- ゴール候補の条件(`GoalRule`・`straight_line_is_blocked`・`LINE_SAMPLE_STEP`)、`TOP_PAIR_OFFSET_X`、ROUNDのパラメータ
- 摩擦・閾値・速さの定数(`docs/beigoma-friction-bump-graze-spec.md`のまま)

## 定数

`src/game/beigoma/board.rs`:

```rust
/// ベーゴマの半径(マス)。直径が1マス。見た目の円盤と凹凸への接触判定の両方に使う
pub const TOP_RADIUS: f64 = 0.5;
```

定数どうしの関係(テストでconst assertとして固定する):

- `TOP_RADIUS <= BOUNCE_KICK`(弾かれた直後は同じ凹凸に触れていない)
- `MAX_SPEED × MAX_STEP(0.01秒) < TOP_RADIUS`(1ステップで触れ始めを飛び越さない。`MAX_STEP`は`beigoma.rs`にあるので`beigoma.rs`側のテストで固定する)
- `TOP_RADIUS × 2.0 <= 1.0`(直径がマスを超えない)

`src/game/beigoma/render.rs`:

```rust
/// テキスト表示のベーゴマの円盤の色(画像表示のTOP_FACE_PIXELと同じ)
const TOP_BG: Color = Color::Rgb(200, 210, 225);
/// 円盤の上に置く回転・飛び上がりの記号の色
const TOP_FG: Color = Color::Rgb(60, 70, 90);
/// 星の演出の記号の色(円盤は描かない)
const STAR_FG: Color = Color::Rgb(220, 240, 255);
/// 飛び上がっている間の円盤の半径の倍率
const AIRBORNE_DISC_SCALE: f64 = 0.7;
```

`TOP_FG`は値を変える(従来の`Rgb(220, 240, 255)`は`STAR_FG`に移す)。`TOP_BG`の輝度は約209で、盤の背景9色のどれ(最大は凸の明180)よりも明るい。

## 関数・型

### `src/game/beigoma/board.rs`

```rust
/// ベーゴマが触れている凹凸のマス1つ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    pub cell: (usize, usize),
    /// 中心からマスの四角形までの距離(中心が四角形の中なら0)
    pub distance: f64,
}

impl Board {
    /// 中心posのベーゴマ(半径TOP_RADIUS)が触れている凹凸(Bump/Hollow)のマス。
    /// 距離が近い順(同じなら左上から行優先)。調べるのはpos ± TOP_RADIUSを含む2×2マスだけ
    pub fn contacts(&self, pos: (f64, f64)) -> Vec<Contact>;
}

/// posからマスcellの四角形までの距離。四角形の中なら0
pub fn cell_distance(pos: (f64, f64), cell: (usize, usize)) -> f64;

/// 接触の法線: posから見た、マスcellの四角形の最も近い点への単位ベクトル。中(距離0)なら(0, 0)
pub fn contact_normal(pos: (f64, f64), cell: (usize, usize)) -> (f64, f64);

/// 凹凸に触れた向きの判定。速度と法線のなす角のcosで正面(HeadOn)か斜め(Graze)かを決める。
/// 法線が零・止まっている時は正面扱い
pub fn edge_contact(vel: (f64, f64), normal: (f64, f64)) -> EdgeContact;

/// ベーゴマの状態
pub enum TopState {
    Rolling,
    Airborne { remaining: Duration, contact_g: f64 },
    /// 凹cellにハマっている。凹のマスの縁で位置を留め、速さがHOLLOW_EXIT_SPEEDに届いたら抜ける
    Sunk { cell: (usize, usize) },
}

pub struct Top {
    pub pos: (f64, f64),
    pub vel: (f64, f64),
    pub state: TopState,
    /// 今触れている凹凸のマス。新しく触れたマスだけ判定する
    touching: Vec<(usize, usize)>,
}

impl Top {
    /// posに止まった状態で置く。置いた位置で触れている凹凸には既に触れている扱い
    pub fn new(board: &Board, pos: (f64, f64)) -> Self;

    /// 転がっている間: 新しく触れた凹凸があれば、最も近い1つを判定する
    fn roll(&mut self, board: &Board, secs: f64, tilt: &Tilt, g: GForce) -> Option<StepEvent>;

    /// 凸に触れた瞬間。側面接触なら飛び上がらずその場で判定し、それ以外は飛び上がる
    fn enter_bump(&mut self, board: &Board, normal: (f64, f64), g: f64) -> StepEvent;

    /// 凹cellに触れた瞬間。正面ならハマり、斜めなら側面をこする
    fn enter_hollow(&mut self, board: &Board, cell: (usize, usize), normal: (f64, f64), g: f64) -> StepEvent;

    /// 側面をこすった(凸・凹共通)。弾かれるならbounce()して、弾かれた先で触れている凹凸を持ち直す
    fn graze(&mut self, board: &Board, friction: f64) -> StepEvent;

    /// 凹にハマっている間(従来のstruggle)。抜けた時に触れている凹凸を持ち直す
    fn struggle(&mut self, board: &Board, secs: f64, tilt: &Tilt, g: GForce) -> Option<StepEvent>;

    /// 飛び上がっている間(従来のfly)。着地で触れている凹があればハマり、無ければ触れている凹凸を持ち直す
    fn fly(&mut self, board: &Board, dt: Duration, remaining: Duration, contact_g: f64) -> Option<StepEvent>;

    /// 今の位置で触れている凹凸を全部「触れている」にする
    fn settle_contacts(&mut self, board: &Board);

    /// 凹cellにハマる。中心をマスの縁まで動かし(clamp_into_cell)、Sunk { cell }にする
    fn sink_into(&mut self, cell: (usize, usize));
}
```

`cell_index`と`Top::prev_cell`は削除する。`clamp_into_cell`・`edge_contact_from_cos`・`is_bump_graze`・`bump_graze_friction`・`landing_friction`・`classify_landing`・`surface_friction`・`drive`・`bounce`・`cap_speed`・`advance`は変えない(`surface_friction`の`state == TopState::Sunk`は`matches!(state, TopState::Sunk { .. })`にする)。

`Board::contacts`の手順:

1. 調べるマスの範囲を`x ∈ [floor(pos.0 − TOP_RADIUS), floor(pos.0 + TOP_RADIUS)]`、`y`も同様に取り、盤の範囲(`0..BOARD_WIDTH`・`0..BOARD_HEIGHT`)で切り詰める(常に最大2×2マス)
2. `cells`(ゴールは含まない)が`Bump`または`Hollow`のマスについて`cell_distance`を求め、`TOP_RADIUS`未満のものを`Contact`にする
3. 距離の昇順(`total_cmp`)、同じなら`(y, x)`の昇順に並べる

`cell_distance`と`contact_normal`: 最も近い点`q = (pos.0.clamp(cx, cx + 1), pos.1.clamp(cy, cy + 1))`。距離は`|pos − q|`、法線は`(q − pos) / |q − pos|`(距離0なら`(0, 0)`)。

`roll`の流れ:

1. 足元の摩擦を`surface_friction(board.cell_at(pos), state, g)`で求め、`drive`する(従来どおり)
2. `!Board::contains(pos)`なら`FellOff`
3. `board.cell_at(pos) == Goal`なら`Goal`
4. `contacts = board.contacts(pos)`。`contacts`の先頭から`touching`に無い最初のものを`entered`とする。`touching`を`contacts`のマスに置き換える
5. `entered`が無ければ`None`。あれば`normal = contact_normal(pos, entered.cell)`を求め、`board.cell(entered.cell)`が`Bump`なら`enter_bump(board, normal, g)`、`Hollow`なら`enter_hollow(board, entered.cell, normal, g)`

`enter_bump`: `contact = edge_contact(vel, normal)`。`is_bump_graze(contact, speed)`なら`graze(board, landing_friction(g) + bump_graze_friction(speed))`、それ以外は`Airborne { remaining: HOP_DURATION, contact_g: g }`にして`Hopped { contact_g: g }`(従来どおり)。

`enter_hollow`: `edge_contact(vel, normal)`が`HeadOn`なら`sink_into(cell)`して`Sank`。`Graze`なら`graze(board, landing_friction(g) + HOLLOW_GRAZE_FRICTION)`。

`graze`: `landing = classify_landing(friction)`。`Bounce`なら`bounce()`して`settle_contacts(board)`。`Grazed(landing)`を返す。

`struggle`: `hollow`は`state`の`cell`から取る。従来どおり凹のマスの中に留め、抜ける条件を満たしたら`Rolling`にして`settle_contacts(board)`し`Escaped`(場外は先に`FellOff`)。

`fly`: 従来どおり進めて着地まで待つ。着地したら`Rolling`にし、`landing`が`Bounce`なら`bounce()`。`landing != Flown`かつ`Board::contains(pos)`のとき、`board.contacts(pos)`のうち`Hollow`で最初のもの(最も近い凹)があれば`sink_into`、無ければ`settle_contacts(board)`。`Landed(landing)`を返す。

`Top::new`: `touching`を`board.contacts(pos)`のマスで初期化する。

モジュール冒頭のコメント: 「どちらも『マスに入った瞬間』に判定する」を「どちらもベーゴマ(半径`TOP_RADIUS`)の円盤がマスに触れた瞬間に判定する」にし、法線の説明を「中心からマスの最も近い点への向き」にする。

### `src/game/beigoma/render.rs`

```rust
/// ベーゴマの円盤の半径(マス)。飛び上がっている間は小さくする
fn disc_radius(top: &TopView) -> f64;

/// posを中心とする半径radiusの円盤を塗る。セルの4隅を逆変換した外接矩形(マス座標)とposの距離が
/// radius未満のセルの記号を空白にし、背景色をTOP_BGにする。調べるセルは、pos ± radiusの正方形の
/// 4隅を順変換した外接矩形を上下左右1セルずつ広げてareaで切り詰めた範囲
fn paint_disc(buffer: &mut Buffer, projection: &BoardProjection, area: Rect, pos: (f64, f64), radius: f64);

/// セル(x, y)の4隅を逆変換したマス座標の外接矩形((min_u, min_v), (max_u, max_v))。
/// 4隅のどれかが地平線の向こう側ならNone
fn cell_mass_bounds(projection: &BoardProjection, x: u16, y: u16) -> Option<((f64, f64), (f64, f64))>;

/// 点posから矩形((min_u, min_v), (max_u, max_v))までの距離。中なら0
fn rect_distance(pos: (f64, f64), bounds: ((f64, f64), (f64, f64))) -> f64;
```

`render_board_text`の手順(`docs/beigoma-board-grid-shading-spec.md`の手順に4を足し、5を変える):

1. `BoardProjection::new(&layout, tilt)`(変更なし)
2. 背景: 市松・陰影で塗る(変更なし)
3. 凸・凹・ゴールの記号(変更なし)
4. 円盤: `tops`のうち`star_frame`が`None`かつ`Board::contains(pos)`のものについて、`paint_disc(buffer, &projection, area, pos, disc_radius(top))`。スライスの順に塗る(重なった範囲は後のものが上。色は同じ)
5. ベーゴマの記号: `projection.project(top.pos)`に`glyph_cell`をかけたセルを`layout.panel`の範囲に収め、同じセルになったものは従来どおり右隣へずらして置く。記号は`top_glyph`(変更なし)、スタイルは星の演出中なら`STAR_FG`・太字、それ以外は`TOP_FG`・太字

`board_area`・`BoardArea`・`BoardProjection`・`glyph_cell`・`put_glyph_at`・`lanes_by`・`cell_style`・`compose_base`・`patch_image`は変えない。

### `src/game/beigoma.rs`

- `TopSlot::new(board: &Board, pos: (f64, f64))`にし、`Top::new(board, pos)`を呼ぶ。呼び出し元(ROUND開始時の投入)で`&self.board`を渡す
- `Reaction::of`・`on_step_event`は変えない(出来事の種類は変わらない)

## 対象ファイル

- `src/game/beigoma/board.rs`: `TOP_RADIUS`、`Contact`、`Board::contacts`、`cell_distance`、`contact_normal`、`edge_contact`の引数変更、`TopState::Sunk { cell }`、`Top`の`touching`と`new`/`roll`/`enter_bump`/`enter_hollow`/`graze`/`struggle`/`fly`/`settle_contacts`/`sink_into`、`cell_index`と`prev_cell`の削除、コメント、既存テストの更新と追加
- `src/game/beigoma/render.rs`: `TOP_BG`/`TOP_FG`/`STAR_FG`/`AIRBORNE_DISC_SCALE`、`disc_radius`/`paint_disc`/`cell_mass_bounds`/`rect_distance`、`render_board_text`の手順4・5、既存テストの更新と追加
- `src/game/beigoma.rs`: `TopSlot::new`に`&Board`を渡す、`MAX_STEP`とのconst assert、既存テストの更新

## テスト観点

先にテストを書き、実装で通す。凹凸に触れる位置の境目(`d == 0.5`)を避けた値で確かめる。

### 定数

- `TOP_RADIUS == 0.5`
- 「定数」節の関係3つをconst assertで固定する

### 距離・法線(`cell_distance`・`contact_normal`の単体テスト)

- マス(10, 6)に対して: `(9.6, 6.5)`は距離0.4・法線`(1, 0)`、`(10.5, 5.7)`は距離0.3・法線`(0, 1)`、`(11.4, 6.5)`は法線`(−1, 0)`、`(10.5, 7.4)`は法線`(0, −1)`
- 角: `(9.7, 5.6)`は距離0.5・法線`(0.6, 0.8)`(誤差1e-9)
- 中: `(10.5, 6.5)`・`(10.0, 6.0)`・`(10.999, 6.999)`は距離0・法線`(0, 0)`
- 法線の長さは常に1か0

### 触れている凹凸(`Board::contacts`の単体テスト。全部平坦な盤に凹凸を置く`board_with`で確かめる)

- 凹(10, 6)だけの盤: `(10.5, 6.5)`は`[(10, 6) 距離0]`、`(10.5, 5.6)`は`[(10, 6) 距離0.4]`、`(10.5, 5.5)`は空(ちょうど`TOP_RADIUS`は触れない)、`(10.5, 5.4)`は空、`(9.7, 5.7)`は`[(10, 6) 距離約0.424]`、`(9.6, 5.6)`は空(角までの距離約0.566)
- 凸(9, 6)と凸(11, 6)の盤: `(10.5, 6.5)`は空(両方ちょうど0.5)、`(10.4, 6.5)`は`[(9, 6)]`だけ
- 近い順: 凸(10, 5)と凹(9, 6)の盤で`(10.2, 6.3)`は`[(9, 6) 距離0.2, (10, 5) 距離0.3]`(行優先なら(10, 5)が先だが、距離が近い方を先にする)
- 同じ距離: 凸(10, 5)と凸(9, 6)の盤で`(10.2, 6.2)`は`[(10, 5) 距離0.2, (9, 6) 距離0.2]`(左上から行優先)
- ゴールのマス・平坦なマスは含まない。盤の縁(`(0.2, 6.5)`で凹(0, 6))でもpanicしない。盤の外の位置(`(−0.3, 6.5)`)でも、触れていれば含む
- ROUND1の盤で、投入位置`(1.5, 10.5)`とROUND1のゴール`(19, 0)`の中心は空(置いた瞬間・ゴールで触れているものが無い)

### 正面/斜め(`edge_contact`の単体テスト。既存`edge_contact_threshold_is_exact`の書き換え)

- 法線`(1, 0)`に対して速度の角が60°の前後0.1°で正面/斜めが分かれる(既存と同じ角度の取り方)。法線`(0, 1)`でも同じ
- 法線`(0, 0)`は速度によらず正面、速度`(0, 0)`は法線によらず正面
- 角の法線`(0.6, 0.8)`に対して速度`(1, 1)`は正面(cos≈0.99)、速度`(1, −0.5)`は斜め(cos≈0.18)

### 転がりの接触(`Top`の物理テスト)

- 触れる位置: 凸(10, 6)の左隣の平坦なマスに`(9.0, 6.5)`で置き、速度`(3, 0)`で進めると、中心が`9.5`を越えた最初のステップで`Hopped`になる(それより前は`None`)。中心のマスは(9, 6)のまま(凸のマスに入る前に触れる)
- 隣の行の中心線: 凸(10, 6)の盤で`(9.0, 5.5)`から速度`(3, 0)`で100ステップ進めても何も起きない(角までの距離が0.5を下回らない)
- 隣の行を縁に寄って通る: `(9.0, 5.55)`(縁から0.45)から速度`(3, 0)`で進めると、角`(10, 6)`まで0.5未満になったステップで`Grazed(Bounce)`(法線は角への向きで縦成分が約0.9、cos≈0.44で斜め、速さ3.0で側面接触)。中心のマスは(9, 5)のまま
- 同じ位置からゆっくり: `(9.0, 5.55)`から速度`(1.5, 0)`なら`Hopped`(斜めでも速さが`BUMP_GRAZE_SPEED`未満。触れるまでに約70ステップかかるので`first_event`で待つ)
- 角に正面から: `(9.0, 5.7)`(縁から0.3)から速度`(3, 0)`で進めると`Hopped`(法線は約`(0.8, 0.6)`でcos≈0.8、正面)
- 置いた位置で触れている: `Top::new(board, (9.6, 6.5))`(凸(10, 6)に触れている)から速度`(3, 0)`で進めても、凸のマスの中を通り過ぎるまで`Hopped`にならない。中を抜けて距離が0.5以上離れてから戻る(速度を`(−3, 0)`にする)と再び`Hopped`
- 2つに同時に触れた: 凸(10, 5)と凸(9, 6)の盤で`(10.6, 6.6)`から速度`(−3, −3)`で進めると、最初の出来事は`Hopped`が1回だけで、その時点で`touching`に両方入っている(2回目の`Hopped`は出ない)
- 凹に正面から: 凹(10, 6)の盤で`(9.0, 6.5)`から速度`(3, 0)`。中心が`9.5`を越えたステップで`Sank`、その直後の`pos.0`は`10.0`(縁まで動く)、`state == Sunk { cell: (10, 6) }`、`cell_at(pos) == Hollow`。上下左右の4辺とも同様(それぞれ縁の外側0.02ではなく`TOP_RADIUS + 0.02`から始める)
- 凹に斜めから: 凹(10, 6)の盤で`(10.5, 5.495)`から速度`(3, 1)`(cos≈0.32)。最初のステップで`Grazed(Bounce)`、`state == Rolling`、位置は縁から0.5以上離れる
- 凹の角に正面から: 凹(10, 6)の盤で`(9.6, 5.6)`から速度`(3, 3)`。角まで0.5未満になったステップ(2ステップ目)で`Sank`、`pos`は凹の左上の角`(10.0, 6.0)`(`clamp_into_cell`の結果)
- ハマった後: 既存の`sunk_top_cannot_leave_below_the_exit_speed`・`sunk_top_escapes_with_full_tilt`・`sunk_top_keeps_building_speed_while_clamped`・`sunk_top_stays_slow_even_with_full_tilt`・`sunk_top_escaping_over_the_rim_falls_off`は、`sunk_top`を`state = Sunk { cell: HOLLOW_AT }`で作るだけで変更なしで通ること
- 抜けた直後: 凹(10, 6)から傾き最大で右へ抜けた直後(`Escaped`のステップ)の`touching`は`[(10, 6)]`で、次のステップで`Sank`にならない。抜けてから0.5以上離れて戻ると再び`Sank`
- 着地で触れている凹: 凸(9, 6)と凹(10, 6)の盤で`(8.48, 6.5)`から速度`(5, 0)`。`Hopped`の後、着地で`Landed(Light)`かつ`state == Sunk { cell: (10, 6) }`、`cell_at(pos) == Hollow`(飛んでいる間に1.25進み、着地時に凹に触れている)
- 着地で触れている凸: 既存`staying_on_the_same_bump_does_not_hop_again`(着地後に同じ凸で飛び上がり直さない)を変更なしで通す
- 弾かれた直後: `Grazed(Bounce)`・`Landed(Bounce)`の直後の`touching`が空(`BOUNCE_KICK`で0.5以上離れる)
- 場外・ゴール: 既存の`top_falls_off_when_its_center_crosses_the_rim`(4辺)・`airborne_top_also_falls_off_the_rim`・`entering_the_goal_cell_reports_goal`・`bounce_from_the_center_stays_on_the_board`(隣の行・列の中心線上で凹凸に触れないことに依存する)・`flat_ground_never_causes_hops_even_under_huge_g`(最上段を横に振っても(11, 1)には距離0.5で触れない)・`speed_is_capped`を変更なしで通す
- 摩擦・傾き・Gの既存テスト(`tilt_accelerates_the_top_in_the_tilted_direction`等)を変更なしで通す

### 既存テストの更新(`board.rs`)

- `Top::new(pos)`を`Top::new(&board, pos)`にする(テスト内の全呼び出し)
- `top_just_left_of_bump`: 置く位置を`bx − 0.02`から`bx − TOP_RADIUS − 0.02`にする(1ステップ目で触れて`Hopped`になる)
- `above_bump`: `by − 0.005`から`by − TOP_RADIUS − 0.005`にする
- `entering_a_hollow_head_on_sinks_the_top`: 4辺の開始位置を縁の外側`TOP_RADIUS + 0.02`にする。`Sank`の後に`cell_containing(pos) == HOLLOW_AT`を確かめる部分は残す
- `entering_a_hollow_at_a_shallow_angle_grazes_and_bounces`・`grazing_under_high_g_flies_the_top_off`・`grazing_a_bump_*`・`edge_contact_splits_bumps_and_hollows_the_same_way`: 開始位置を`above_bump`/`TOP_RADIUS`ぶん外側にする。`edge_contact_splits_bumps_and_hollows_the_same_way`の`edge_contact(vel, prev, BUMP_AT)`は`edge_contact(vel, contact_normal(pos, BUMP_AT))`にする
- `a_pair_of_tops_moves_independently`: 後ろのベーゴマを`2 × TOP_PAIR_OFFSET_X`ではなく`TOP_RADIUS`だけ後ろに置く(前だけが触れる)
- `landing_on_a_hollow_sinks_after_landing`: 開始位置を`bump.0 − TOP_RADIUS − 0.02`にする
- `moving_within_a_cell_triggers_nothing`: 変更なし(置いた時に触れている扱いになるので、中で動いても何も起きない)
- `a_pair_of_tops_is_dropped_side_by_side_inside_the_start_cell`: `cell_index`の代わりに`cell_containing`で確かめる
- `edge_contact_threshold_is_exact`: 「正面/斜め」節のとおり法線を直接渡す形にする
- `sunk_top`: `state = TopState::Sunk { cell: cell_containing(pos) }`
- `bounce_at`: 変更なし(着地で触れている凹凸が無い位置)

### 描画(`render.rs`)

#### 単体(`rect_distance`・`cell_mass_bounds`・`disc_radius`)

- `rect_distance`: 中は0、左は横の距離、角は斜めの距離(`cell_distance`と同じ関係)
- `cell_mass_bounds`: 傾き0・2×1セルで、セル`(rect.x + 20, rect.y + 6)`は`((10.0, 6.0), (10.5, 7.0))`(誤差1e-9)。4×2セルではセル`(rect.x + 40, rect.y + 12)`が`((10.0, 6.0), (10.25, 6.5))`。傾き最大でも`min < max`
- `disc_radius`: 転がり中0.5、飛び上がり中0.35

#### 円盤(傾き0)

- 2×1セル(40×12): ベーゴマ`(10.5, 6.5)`で`TOP_BG`のセルが`cell_rect(10, 6)`の2セルちょうど。`(10.3, 6.5)`では`x ∈ {19, 20, 21}`の3セル。`(10.5, 6.0)`では行5と行6の各2セル(4セル)。どの位置でも`TOP_BG`のセルが1つ以上ある(`x`を`10.0`から`11.0`まで0.05刻み、`y`を`6.0`から`7.0`まで0.05刻みで回す)
- 4×2セル(90×26): `(10.5, 6.5)`で`TOP_BG`が`cell_rect(10, 6)`の8セルちょうど。`(10.4, 6.4)`では横5セル(`cell_rect(10, 6)`の4セルと左隣1セル)×縦3行(`cell_rect(10, 6)`の2行と上の1行)の範囲のうち、上の行の左右の端2セル(角までの距離約0.57・約0.53)を除く13セル
- 10×5セル(200×60): `(10.5, 6.5)`で`TOP_BG`のセルが40〜50個。`(10.55, 6.55)`では`cell_rect(10, 6)`の左上のセルは`TOP_BG`でない(角までの距離約0.57)
- 飛び上がり中: 10×5セルで`(10.5, 6.5)`の`TOP_BG`のセル数が転がり中より少ない(横8セル以下)。2×1セル・4×2セルでマスの中央にいる時は転がり中と同じセル数
- 記号: 回転の記号のセルは`glyph_cell(project(pos))`で、そのセルの背景は`TOP_BG`(円盤の中)。記号の色は`TOP_FG`
- 凹凸の記号が隠れる: 凸(10, 6)の左隣`(9.6, 6.5)`にベーゴマを置くと、凸の記号のセル`cell_rect(10, 6)`の左のセルは`TOP_BG`で記号は回転の記号か空白(▲は描かれない)。ベーゴマを離すと▲が戻る
- 星の演出中: `TOP_BG`のセルが無く、星の記号のセルの色が`STAR_FG`(既存`text_top_shows_the_star_animation_glyph_when_present`に追加)
- 場外: `(−1.0, 5.0)`では`TOP_BG`のセルが無く記号だけ(既存`tilted_top_far_off_the_board_stays_inside_the_panel`の位置)。`(−0.3, 5.5)`(中心は盤の外)でも同じ。`(0.2, 5.5)`(中心は盤の上、円盤が左へはみ出す)では盤の描画範囲`layout.rect`の左の外側にも`TOP_BG`のセルがある(パネルに余白があるエリア`Rect::new(5, 3, 60, 20)`で確かめる)
- 2個: `two_tops_in_the_same_cell_are_drawn_side_by_side`(記号の位置)は変更なしで通す。`(1.35, 10.5)`と`(1.65, 10.5)`の円盤は合わせて1つの連続した範囲になる(間に`TOP_BG`でないセルが無い)

#### 円盤(傾けた時)

- 各軸最大・組み合わせ最大で、`TOP_BG`のセルの中心を逆変換した点は全部`pos`から1.25以内(0.5 + セルの対角線の半分。2×1セルで遠い側が縮んでも約0.73)にあり、逆変換した点が`pos`から距離0.25以内のセルは全部`TOP_BG`
- 記号のセルの背景は`TOP_BG`(既存`tilted_top_is_drawn_at_the_projected_center_of_its_cell`は`tilted_top_is_drawn_at_its_projected_position`に改名し、期待値を`glyph_cell(projection.project(pos))`にする)
- 既存`tilting_the_board_shifts_where_the_top_is_drawn_in_text_mode`は変更なしで通す

#### 既存テストの更新(`render.rs`)

- `text_board_shows_bumps_goal_and_top`: 記号の右隣のセルの背景を`checker_of(mass)`ではなく`TOP_BG`と比べる(記号は空白のまま)。市松の確認は、ベーゴマの円盤の外のセル(例: `cell_rect(1, 10)`の右隣のマス`cell_rect(3, 10)`)で行う
- `tilted_boards_use_only_the_board_colors`: `TOP_BG`のセルを除いて確かめる
- `pitching_paints_fewer_cells_and_never_outside_the_board_rect`・`tilted_text_board_is_a_trapezoid_on_a_large_area`: ベーゴマを`(−1000.0, −1000.0)`に置く(円盤で塗られたセルが盤の背景色の数を変えないように)
- `glyphs_stay_inside_their_own_cells_when_tilted`・`text_board_distinguishes_hollow_from_bump`・`two_by_one_masses_show_the_checker_and_two_shading_steps`・`four_by_two_masses_show_three_shading_steps`・`large_masses_have_as_many_lit_cells_as_shaded_cells`: ベーゴマが凹凸のマスに触れない位置(投入位置`(1.5, 10.5)`)のまま変更なし
- `board_render_does_not_panic_in_tiny_areas`・`two_tops_do_not_panic_in_tiny_areas`: 位置に`(10.5, 6.0)`・`(0.2, 0.2)`・`(19.8, 11.8)`を足す(行またぎ・縁のはみ出し)
- 画像表示の既存テスト(`image_board_*`・`patch_*`・`each_top_has_its_own_patch_on_the_image_board`等)は変更なしで通す

### ゲーム側(`beigoma.rs`)

- `MAX_SPEED × MAX_STEP < TOP_RADIUS`のconst assert
- `Top::new(pos)`を`Top::new(&game.board, pos)`にする(テスト内の全呼び出し)。凹凸の縁の外側に置くテストは、縁からの距離を`TOP_RADIUS`ぶん足す(`flat_below_a_bump`の下に置く2件は`y + 0.1`→`y + TOP_RADIUS + 0.1`・`y + 0.02`→`y + TOP_RADIUS + 0.02`、凹の左隣に置く1件は`hx − 0.02`→`hx − TOP_RADIUS − 0.02`)。縁から離れた位置に置く場外のテスト(`(0.6, 0.5)`・`(0.6, 5.5)`)は位置を変えない
- `TopState::Sunk`との等価比較は`matches!(state, TopState::Sunk { .. })`にする
- 出来事(`Sank`・`Escaped`・`Hopped`・`Grazed`・`Landed`)と演出・SE・一言の既存テストは変更なしで通す
