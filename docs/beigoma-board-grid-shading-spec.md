## 概要

「べー」の盤面(テキスト表示)に、マス目の格子と凹凸の陰影を加える。対象はテキスト表示のみ(画像表示は対象外)。

- 格子: 平坦なマスを2トーンの市松模様(隣り合うマスで明暗を交互にする)で塗り、マスの境界が見えるようにする。傾けると市松の1マスが投影に従って台形に変形するので、傾きの立体感が強調される。傾き0でも市松は常に見える
- 陰影: 凸・凹のマスを、マス内の位置に応じた3段階の明度(明・中・暗)で塗る。光源は左上に固定し、凸は左上が明るく右下が暗い、凹はその逆にする。色相は変えず、明度だけを変える

どちらも背景塗りの段階でセルごとに決めるので、描画手順は変わらず、記号(▲▽◎・ベーゴマ)の置き方も変わらない。

## 見た目の決まり

- 1マス=横2×縦1セルの表示(通常の端末サイズ)から、横10×縦5セルの表示まで同じ規則で描く。1マスが何セルでも、セルの中心が属するマスとマス内位置だけで色が決まる
- 平坦なマス: マスの座標`(mx, my)`の`mx + my`が偶数なら明るいトーン、奇数なら暗いトーン。マスの中は同じトーンで塗る
- 凸のマス: 左上の帯を明、中央の帯を中(従来の`BUMP_BG`)、右下の帯を暗で塗る(盛り上がった面の左上に光が当たり、右下に影が落ちる)
- 凹のマス: 左上の帯を暗、中央の帯を中(従来の`HOLLOW_BG`)、右下の帯を明で塗る(くぼみの左上の内壁が影になり、右下の内壁に光が当たる)
- ゴールのマス: 従来どおり`GOAL_BG`で一様に塗る(市松・陰影とも付けない)
- 明度の順序: 凹のどの帯 < 平坦のどのトーン < 凸のどの帯。凸は帯によらず平坦より明るく、凹は帯によらず平坦より暗い(種類の区別が陰影で崩れない)
- 記号の色(`BUMP_FG`・`HOLLOW_FG`・`GOAL_FG`・`TOP_FG`)と太字は従来のまま
- 傾けた時: 市松のマスと陰影の帯は、逆変換した`(u, v)`から決めるので投影に従って変形する。陰影の光源は傾きによらず左上に固定
- 1マス=2×1セルの表示では、凸は「左のセルが明・右のセルが暗」、凹は「左が暗・右が明」の2段階に見える(中央の帯はこの表示では現れない)。4×2セル以上の表示で3段階が現れる

## 色

### 記号

- `(mx, my)`: セルの中心を逆変換した`(u, v)`が属するマス。`mx = floor(u)`、`my = floor(v)`
- `(fu, fv)`: マス内の位置。`fu = u − mx`、`fv = v − my`(それぞれ`[0, 1)`)
- `d = fu + fv − 1`: マスの左上の角で−1、右下の角で+1、左下と右上を結ぶ対角線上で0

### 定数

```rust
/// 平坦なマスの市松の2トーン。FLAT_BGを白・黒へこの割合だけ寄せる
const CHECKER_MIX: f64 = 0.08;
/// 凹凸の陰影の明・暗。BUMP_BG/HOLLOW_BGを白・黒へこの割合だけ寄せる
const SHADE_MIX: f64 = 0.15;
/// 陰影の帯の境界。d ≤ −SHADE_BANDで左上の帯、d ≥ SHADE_BANDで右下の帯、その間が中央の帯
const SHADE_BAND: f64 = 0.2;
```

`FLAT_BG`・`BUMP_BG`・`HOLLOW_BG`・`GOAL_BG`と記号の色の定数は従来の値のまま。

### 混色

`mix(color, target, t)`は各チャンネルを`round(c + (t_c − c) × t)`にする(`round`は四捨五入、結果は0〜255)。`Color::Rgb`以外の色はそのまま返す。

- 明るくする: `mix(base, Rgb(255, 255, 255), t)`
- 暗くする: `mix(base, Rgb(0, 0, 0), t)`

### 帯の決め方

```
shade_band(fu, fv):
    d = fu + fv − 1
    d ≤ −SHADE_BAND  →  TopLeft
    d ≥  SHADE_BAND  →  BottomRight
    それ以外          →  Middle
```

1マス=2×1セルでは`fu ∈ {0.25, 0.75}`・`fv = 0.5`なので`d = ∓0.25`となり、左のセルがTopLeft、右のセルがBottomRight。4×2セルでは`fu ∈ {0.125, 0.375, 0.625, 0.875}`・`fv ∈ {0.25, 0.75}`で、上段は左から TopLeft, TopLeft, Middle, Middle、下段は Middle, Middle, BottomRight, BottomRight。10×5セルでは`(i, j) ↔ (9 − i, 4 − j)`で`d`の符号が反転するので、TopLeftとBottomRightのセル数が等しい。

### セルの背景色

| マス | 条件 | 背景色 |
|---|---|---|
| Flat | `(mx + my) % 2 == 0` | `mix(FLAT_BG, 白, CHECKER_MIX)` |
| Flat | `(mx + my) % 2 == 1` | `mix(FLAT_BG, 黒, CHECKER_MIX)` |
| Bump | TopLeft | `mix(BUMP_BG, 白, SHADE_MIX)` |
| Bump | Middle | `BUMP_BG` |
| Bump | BottomRight | `mix(BUMP_BG, 黒, SHADE_MIX)` |
| Hollow | TopLeft | `mix(HOLLOW_BG, 黒, SHADE_MIX)` |
| Hollow | Middle | `HOLLOW_BG` |
| Hollow | BottomRight | `mix(HOLLOW_BG, 白, SHADE_MIX)` |
| Goal | 常に | `GOAL_BG` |

参考値(上の式で求まる色と輝度`0.299R + 0.587G + 0.114B`):

| 色 | RGB | 輝度 |
|---|---|---|
| 凹 暗 | (51, 32, 15) | 36 |
| 凹 中 `HOLLOW_BG` | (60, 38, 18) | 42 |
| 凹 明 | (89, 71, 54) | 75 |
| 平坦 暗 | (138, 97, 55) | 105 |
| 平坦 明 | (158, 117, 76) | 125 |
| 凸 暗 | (174, 136, 89) | 142 |
| 凸 中 `BUMP_BG` | (205, 160, 105) | 167 |
| 凸 明 | (213, 174, 128) | 180 |

記号を置くセル`glyph_cell(マスの中心)`は、2×1セルでは左のセル(TopLeft)、4×2セルでは`(x0 + 1, y0 + 1)`(Middle)、10×5セルでは`(x0 + 4, y0 + 2)`(Middle)なので、凸の▲は明か中、凹の▽は暗か中の背景に載る。

## 描画の手順(`render_board_text`)

`docs/beigoma-tilt-3d-spec.md`の手順のうち、2の背景塗りで色の決め方だけを変える。

1. `BoardProjection::new(&layout, tilt)`で係数を1回だけ求める(変更なし)
2. 背景: `area`の全セルについて、セルの中心を逆変換して`(u, v)`を得る。盤内なら`(mx, my) = (u as usize, v as usize)`・`(fu, fv) = (u − mx, v − my)`とし、`cell_style(board.cell(mx, my), (mx, my), (fu, fv))`のスタイルで塗る(記号は空白)。盤外のセルは触らない
3. 凸・凹・ゴールの記号: `cell_glyph`の記号を、マスの中心を順変換した`glyph_cell`のセルに置く(変更なし)
4. ベーゴマ: 変更なし

`cell_look`は`cell_glyph`と`cell_style`に分ける。画像表示(`compose_base`・`patch_image`)は変えない。

計算量: 1セルあたり、逆変換に加えて小数部の計算・帯の判定・混色(乗算3回)が増えるだけ。

## 関数・型

```rust
/// 凹凸のマスの中の陰影の帯。光源は左上
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShadeBand {
    TopLeft,
    Middle,
    BottomRight,
}

/// マス内の位置frac = (fu, fv)から陰影の帯を決める
fn shade_band(frac: (f64, f64)) -> ShadeBand;

/// colorをtargetへ割合tだけ寄せた色(各チャンネルを四捨五入)。Color::Rgb以外はそのまま
fn mix(color: Color, target: Color, t: f64) -> Color;

/// テキスト表示のマスの記号。平坦なマスは空白
fn cell_glyph(cell: Cell) -> &'static str;

/// マスmass = (mx, my)のマス内位置fracにあるセルのスタイル。
/// 背景色は市松(平坦)・陰影(凸凹)・一様(ゴール)、記号の色と太字はマスの種類で決まる
fn cell_style(cell: Cell, mass: (usize, usize), frac: (f64, f64)) -> Style;
```

## 対象ファイル

- `src/game/beigoma/render.rs`

## テスト観点

先にテストを書き、実装で通す。

### 混色と帯(`mix`・`shade_band`の単体テスト)

- `mix(c, t, 0.0) == c`、`mix(c, t, 1.0) == t`。中間は四捨五入(`mix(Rgb(150, 105, 60), 白, 0.08) == Rgb(158, 117, 76)`、`mix(Rgb(150, 105, 60), 黒, 0.08) == Rgb(138, 97, 55)`)
- `Color::Rgb`以外(`Color::Yellow`等)は`t`によらずそのまま
- `shade_band`: `(0.25, 0.5)`→TopLeft、`(0.75, 0.5)`→BottomRight、`(0.5, 0.5)`→Middle、`(0.125, 0.25)`→TopLeft、`(0.375, 0.75)`→Middle、`(0.875, 0.75)`→BottomRight
- 境界: `d == −SHADE_BAND`ちょうどはTopLeft、`d == SHADE_BAND`ちょうどはBottomRight
- 対称: 任意の`(fu, fv)`で、`(1 − fu, 1 − fv)`の帯はTopLeftとBottomRightが入れ替わり、MiddleはMiddleのまま

### スタイル(`cell_style`の単体テスト)

- Flat: `mx + my`が偶数なら明るいトーン、奇数なら暗いトーン。`frac`によらない。輝度は 暗いトーン < `FLAT_BG` < 明るいトーン
- Bump: TopLeft・Middle・BottomRightの背景色の輝度が 明 > `BUMP_BG` > 暗。3つとも平坦の明るいトーンより明るい。文字色`BUMP_FG`・太字は3つとも同じ
- Hollow: 暗 < `HOLLOW_BG` < 明。3つとも平坦の暗いトーンより暗い。文字色`HOLLOW_FG`は3つとも同じ
- Goal: `frac`・`mass`によらず背景`GOAL_BG`・文字色`GOAL_FG`・太字
- 全体の順序: 凹の3色すべて < 平坦の2色すべて < 凸の3色すべて(輝度)。背景色の種類は合計9色

### 描画(`render_board_text`の統合テスト)

- 傾き0・2×1セル(40×12): 平坦なマスは2セルとも同じトーンで、右隣・真下のマスとはトーンが違う。凸のマスは左のセルが凸の明・右のセルが凸の暗。凹のマスは左が凹の暗・右が凹の明。ゴールは2セルとも`GOAL_BG`
- 傾き0・4×2セル(90×26): 凸のマスの左上のセルが凸の明、右下のセルが凸の暗、記号のセル`(x0 + 1, y0 + 1)`が`BUMP_BG`。凹はその逆(左上が暗・右下が明・記号のセルが`HOLLOW_BG`)
- 傾き0・10×5セル(200×60): 凸のマスの明のセル数と暗のセル数が等しく、どちらも0より多い
- 傾けた時(各軸最大・組み合わせ最大): 盤の描画範囲の中で塗られたセルの背景色はすべて9色のどれか。凸の記号のセルの背景は凸の3色のどれか、凹は凹の3色のどれか、ゴールは`GOAL_BG`
- 傾けた時、市松のトーンの境界が動く: 4×2セル以上でpitch最大にした時、最上段の行でトーンが切り替わる位置が最下段の行と異なる(投影に従って変形している)
- 画像表示の経路は描画内容が変わらない(既存テストのまま)
- 小さいエリア(既存の`board_render_does_not_panic_in_tiny_areas`)がpanicしない

### 既存テストの更新

- `text_board_shows_bumps_goal_and_top`: 記号の右隣のセルの背景を`FLAT_BG`ではなく「そのマスの偶奇に対応する市松のトーン」と比べる
- `text_board_distinguishes_hollow_from_bump`: 記号のセルの背景を`HOLLOW_BG`/`BUMP_BG`との一致ではなく「凹の3色のどれか」「凸の3色のどれか」で確かめる。「マス全体を凹の色で塗る」は右隣のセルも凹の3色のどれかであることで確かめる。輝度の順序(凹 < 平坦 < 凸)の確認は残す
- `glyphs_stay_inside_their_own_cells_when_tilted`: 記号のセルの背景を単色との一致ではなく、種類ごとの色の集合(凸3色・凹3色・ゴール1色)に含まれることで確かめる
- テストの`is_board_bg`: 9色すべてを盤の背景色として扱う
