## 概要

「べー」の盤面(テキスト表示)は、傾き(pitch/roll)に応じて盤が立体的に傾いて見えるよう変形して描く。対象はテキスト表示のみ(画像表示は対象外)。

変形は「盤を3D空間で回転させ、真上のカメラから透視投影する」1本の計算で求める。傾けた方向の遠い側(低くなった側)が短くなる台形になり、pitchとrollを同時にかけた時も同じ式でそのまま扱える。

## 見た目の決まり

- 傾き0: 従来と同じ矩形グリッド(描画結果はセル単位で完全に一致する)
- pitch +(↑、前傾): 盤の奥(画面の上)が下がって遠ざかり、上の行ほど横幅が狭くなる台形。pitch −はその逆(下の行が狭い)
- roll +(→、右傾): 盤の右側が下がって遠ざかり、右へ行くほど縦の高さが低くなる台形。roll −はその逆(左が低い)
- 同時にかけた時: 両方が組み合わさった四角形(最も低くなる角が最も小さく縮む)
- 盤の中心のマス(10, 6)は傾けても盤の描画範囲の中心から動かない
- 近い側(高くなった側)の辺の長さは平らな時と同じにし、遠い側だけ縮める(盤全体が平らな時の描画範囲からはみ出さない)
- 傾き最大(`TILT_MAX`)の時の縮み: pitchのみで遠い辺の横幅は近い辺の約0.86倍、rollのみで遠い辺の高さは近い辺の約0.77倍(盤が横長なので、rollの方が縁の移動量が大きい)。1マス=横2セル×縦1セルの表示では、pitch最大で上の行が40セル→約34セル、roll最大で遠い側の高さが12行→約9行になる

## 座標変換

### 記号

- `(bx, by)`: 盤のマス座標(連続値。左上が(0, 0)、xは右、yは下。`Top::pos`と同じ)
- `(u, v)`: 盤の中心を原点にしたマス座標。`u = bx − BOARD_WIDTH/2`、`v = by − BOARD_HEIGHT/2`
- `(sx, sy)`: 投影後の座標(マス単位。平らな時は`(u, v)`に一致)
- `(X, Y)`: 端末のセル座標(連続値)。`X = rect.x + (sx + BOARD_WIDTH/2) × cell_width`、`Y = rect.y + (sy + BOARD_HEIGHT/2) × cell_height`(`rect`/`cell_width`/`cell_height`は`BoardArea`のもの)
- `θp = pitch / TILT_MAX × TILT_ANGLE_MAX`、`θr = roll / TILT_MAX × TILT_ANGLE_MAX`
- `D`: カメラの高さ(マス単位、`CAMERA_HEIGHT`)。盤の中心の真上から真下を見る

### 定数

```rust
/// 傾き最大(TILT_MAX)の時の盤の回転角(rad)。前後・左右とも同じ
const TILT_ANGLE_MAX: f64 = std::f64::consts::PI / 12.0; // 15°
/// カメラの高さ(マス単位)。盤の中心の真上
const CAMERA_HEIGHT: f64 = 20.0;
```

制約: `(BOARD_WIDTH/2 + BOARD_HEIGHT/2) × sin(TILT_ANGLE_MAX) < CAMERA_HEIGHT`(盤のどの角も地平線の手前にある)。現在の値では `16 × 0.259 = 4.1 < 20`。

### 3D回転

盤の点`(u, v, 0)`を、先にroll(盤の前後軸まわり)、次にpitch(左右軸まわり)で回す。roll +で右側(u > 0)が下がり、pitch +で奥(v < 0)が下がる。

```
x2 = u·cosθr
y2 = u·sinθr·sinθp + v·cosθp
z2 = v·sinθp − u·sinθr·cosθp      (z > 0 がカメラに近い側)
```

### 透視投影と正規化

真上のカメラから見た座標は`(x2, y2) / w`、`w = 1 − z2 / D`。`w`は`(u, v)`の一次式なので、盤の4隅での最小値`w_min`を取り、`S = 1 / w_min`で全体を割る(近い側の辺を平らな時と同じ大きさに保つ正規化)。

```
w  = 1 + p·u + q·v
sx = a·u / w
sy = (b·u + c·v) / w

a = cosθr / S
b = sinθr·sinθp / S
c = cosθp / S
p = sinθr·cosθp / D
q = −sinθp / D
S = 1 / min(w at (u, v) = (±W/2, ±H/2))     (W = BOARD_WIDTH, H = BOARD_HEIGHT)
```

傾き0では`a = c = 1`、`b = p = q = 0`、`S = 1`となり、`(sx, sy) = (u, v)`に浮動小数の誤差なく一致する。

### 逆変換(セル座標→マス座標)

背景の塗りはこの逆変換で行う。`(sx, sy)`から:

```
den = 1 − p·sx/a − q·(sy − b·sx/a)/c
w   = 1 / den
u   = sx·w / a
v   = (sy − b·sx/a)·w / c
```

`den ≤ 0`は地平線の向こう側なので盤外として扱う(`None`)。

## 描画の手順(`render_board_text`)

1. `BoardProjection::new(&layout, tilt)`で係数を1回だけ求める
2. 背景: `area`(盤面パネルの内側)の全セルについて、セルの中心`(x + 0.5, y + 0.5)`を逆変換し、`0 ≤ u < BOARD_WIDTH`かつ`0 ≤ v < BOARD_HEIGHT`ならそのマス`board.cell(u as usize, v as usize)`の背景色で塗る(記号は空白)。範囲外のセルは触らない。逆変換で塗るので、変形しても隙間・重なりが出ない
3. 凸・凹・ゴールの記号: マスの中心`(mx + 0.5, my + 0.5)`を順変換し、`glyph_cell`のセルに置く(背景色は塗ったまま、記号だけ置く。従来の`put_glyph`と同じ扱い)
4. ベーゴマ: 位置を含むマスの中心`(pos.0.floor() + 0.5, pos.1.floor() + 0.5)`を順変換し、`glyph_cell`のセルを`layout.panel`の範囲に収めてから記号を置く(場外に出た時は従来どおり盤の外側の位置になる)

記号を置くセルの決め方(記号が2セル幅で表示されても隣のマスに食い込まないよう、中心のすぐ左のセルに置く。従来の`put_glyph`と傾き0で同じ位置になる):

```
glyph_cell(cx, cy) = (floor(cx − 0.5), floor(cy))
```

計算量: 1フレームあたり係数計算1回 + パネルのセル数ぶんの逆変換(乗算数回と除算1回) + 記号の数ぶんの順変換。傾き0でも同じ経路を通す(分岐を増やさない)。

## 関数・型

```rust
/// 傾きに応じた、盤のマス座標と端末のセル座標の間の変換(1フレームに1回作る)
struct BoardProjection {
    /// 盤の左上のセル位置と1マスのセル数(BoardAreaから)
    origin: (f64, f64),
    cell: (f64, f64),
    /// 正規化済みの回転係数と透視の係数
    a: f64,
    b: f64,
    c: f64,
    p: f64,
    q: f64,
}

impl BoardProjection {
    fn new(layout: &BoardArea, tilt: &Tilt) -> Self;
    /// マス座標(連続値)→セル座標(連続値)。盤外の座標もそのまま延長して変換する
    fn project(&self, pos: (f64, f64)) -> (f64, f64);
    /// セル座標(連続値)→マス座標(連続値)。地平線の向こう側はNone
    fn unproject(&self, screen: (f64, f64)) -> Option<(f64, f64)>;
}

/// 中心座標(セル、連続値)から記号を置くセルを決める
fn glyph_cell(center: (f64, f64)) -> (i32, i32);

/// cellがclipの中なら記号を置く(範囲外は何もしない)
fn put_glyph_at(buffer: &mut Buffer, cell: (i32, i32), clip: Rect, glyph: &str, style: Style);
```

`tilt_shift`・`TILT_SHIFT_PER_LEVEL`・`BoardArea::offset_rect`・`put_glyph`は上記に置き換えて削除する。`BoardArea::cell_rect`・`top_rect`は画像表示と既存テストで使うので残す。`BoardRenderer::render`のシグネチャと呼び出し元(`beigoma.rs`)は変えない。

## 対象ファイル

- `src/game/beigoma/render.rs`

## テスト観点

先にテストを書き、実装で通す。

### 変換の性質(`BoardProjection`の単体テスト)

- 傾き0: `project`が`cell_rect`の位置と一致する(任意のマス、任意の`cell_width`/`cell_height`)。`unproject(project(p)) == p`
- 傾き0: `glyph_cell(project(マスの中心))`が従来の記号位置`(rect.x + (w−2)/2, rect.y + h/2)`と一致する(`cell_width`が2・4・10の3通り)
- 中心不動: 傾き最大・組み合わせ含む任意の傾きで、`project((10.0, 6.0))`が盤の描画範囲の中心に一致する
- pitch +のみ: 上の行(`v = −6`)の投影後の横幅 < 下の行(`v = +6`)の横幅。下の行の横幅は平らな時と一致(誤差1e-9)。左右対称(`project`のxが中心について反転)。pitch −では上下が入れ替わる
- roll +のみ: 右の縁(`u = +10`)の投影後の高さ < 左の縁の高さ。左の縁の高さは平らな時と一致。上下対称。roll −では左右が入れ替わる
- 縮みの程度: 傾き最大で、遠い辺/近い辺の比がpitchで0.80〜0.90、rollで0.70〜0.85の範囲にある(見えるが破綻しない)
- 組み合わせ(pitch +・roll +とも最大): 4隅を順変換した四角形が凸で自己交差しない。右上の角(両軸で最も低い角)が中心に最も近い。1本の横の走査線上で`unproject`のuが単調に増える
- 往復: 傾き最大の組み合わせで、盤内の複数点について`unproject(project(p))`が`p`と一致(誤差1e-9)
- 地平線: pitch +最大で、盤の中心から真上(画面の上方向)へ100マス離れた点(`sy = −100`。`den ≈ −0.45`)のセル座標で`unproject`が`None`。60マス(`den ≈ 0.13`)では`Some`
- 定数の制約: `16 × sin(TILT_ANGLE_MAX) < CAMERA_HEIGHT`(4隅の`w`が正)

### 描画(`render_board_text`の統合テスト)

- 傾き0の描画バッファが変更前と完全一致する(既存の`text_board_shows_bumps_goal_and_top`・`text_board_distinguishes_hollow_from_bump`・`text_top_spins_and_changes_while_airborne`・`text_top_shows_the_star_animation_glyph_when_present`をそのまま通す)
- 傾けると描画バッファが傾き0と異なる(既存の`tilting_the_board_shifts_where_the_top_is_drawn_in_text_mode`)
- 塗り: pitch最大で、盤の背景色で塗られたセル数が平らな時より少なく、かつ平らな時の0.7倍以上。盤の描画範囲の外(パネルの余白)は塗られない
- 台形: 広いエリア(1マス=10×5セル)でpitch最大にした時、盤の背景色で塗られた最上段の行のセル数 < 最下段の行のセル数。roll最大では最も右の列の塗られた行数 < 最も左の列の行数
- 記号と背景の対応: 広いエリア(1マス=10×5セル)で組み合わせ最大の傾きにした時、全ての凸の記号のセルの背景色が`BUMP_BG`、凹が`HOLLOW_BG`、ゴールが`GOAL_BG`(記号が自分のマスの中に置かれる)
- ベーゴマ: 傾けた時の記号の位置が、位置を含むマスの中心を順変換して`glyph_cell`にかけた位置と一致する。場外の位置(`(-1000, -1000)`等)でもpanicせずパネルの中に収まる
- 小さいエリア(既存の`board_render_does_not_panic_in_tiny_areas`)を、傾き0・各軸最大・組み合わせ最大の3通りで回してpanicしない
- 画像表示の経路は傾きを渡しても描画内容が変わらない(既存テストのまま)
