# べー 改修10件 設計メモ

## 0. 結論(先に要点)

| 観点 | 結論 |
|---|---|
| 矛盾 | 致命的な矛盾は無い。ただし (a) #133 ランダムゴールは既存仕様書の「固定配置で覚えて上達」と正面から反する→仕様書側を改訂、(b) #127 壁削除により現在の弾かれ速度(BOUNCE_SPEED=10)だとほぼ確実に場外へ出るため係数の見直しが必須、(c) 「ああっ!」「ああっ!!」は1定数に統一、の3点は設計判断が要る |
| 重複 | #128(画像化)と#130(横構図の動的表示)は同じ TruckViewRenderer を触る。#130 のテキスト実装を正にして、#128 の画像は「絵の差し替え」に限定する |
| 依存 | board.rs 物理(#127→#132)→ROUND/ゴール生成(#133+#134)→微振動(#135)→演出(#129)→app.rs 導線(#136)→描画(#131→#130→#128) の順で積む |
| 要確認 | 7点(末尾「ユーザー確認事項」) |

---

## 1. 矛盾・重複の精査

### 1-1. #133(ゴールランダム) × #134(ROUND2 は端に凹凸・直線で抜けられない)

両立させる。分担は次の通り。

- **配置(凹凸・投入位置)は ROUND ごとに固定レイアウト**(`LAYOUT_ROUND1` / `LAYOUT_ROUND2`)。ROUND2 が「端っこに凹凸」を持つのはここで表現する
- **ゴールだけ毎回ランダム**。候補マスは「平坦」「投入位置から `min_distance` 以上」で、ROUND2 はさらに「投入位置→ゴールの直線上に凹凸が1つ以上ある」を必須条件にする(`GoalRule.blocked_straight_line`)
- こうすると「直線移動だけでは抜けられない」がゴール位置の制約として検証可能になり、レイアウト固定部分と衝突しない

副作用: `docs/beigoma-game-spec.md` の「固定配置で毎回同じ」(28行目)と board.rs 冒頭コメントが古くなる。「凹凸と投入位置は固定、ゴールは毎回変わる」に改訂する。

### 1-2. #127(壁削除) × #132(凹凸の側面接触)

どちらも `Top::step` の当たり判定を触る。順序は **#127 → #132**。

- #127 は「advance 後に場外なら即 `FellOff`」を最初に置くだけで、以降の判定(凸・凹・ゴール)は全部その後ろに並ぶ。#132 が追加する凹の「縁でクランプして留める」処理は場外判定と干渉しない(凹の中に留まる=盤内)
- 現状 `on_bump: bool` で「入った瞬間」を検出しているが、凹凸2種になるので `prev_cell: (usize, usize)` に一般化し、「マスが変わった瞬間」を1箇所で検出する。#127 の段階でこの一般化を先にやると #132 の差分が小さくなる
- **要注意**: 壁が無くなると `bounce()` の `BOUNCE_SPEED=10` / 摩擦0.8 で減速距離 ≒ 10/0.8 = 12.5マス。盤は20×12なので、弾かれたらほぼ場外=即GAME OVER になる。`BOUNCE_SPEED` 4.0 / `BOUNCE_KICK` 0.5(減速距離約5.5マス、中央から縦の縁まで6マス)を初期値として提案。テストで「盤の中央で弾かれても場外に出ない」を固定する

### 1-3. #127「ああっ!」 × #129「ああっ!!」

1定数 `GASP_LINE = "ああっ!!"` に統一(感嘆符2つ)。意味づけは次の通り。

- 「ああっ!!」= 女の子のセリフ。軽トラ視点の吹き出しに出す(弾かれ・場外・吹っ飛びの全部で同じ)
- 「ぴよーん!!」= 盤面側の効果文字+ベーゴマの弾かれアニメ(#129)
- 「GAME OVER」= HUD 中央のバナー(結果の区別はここで付く)

現在 HUD 中央に出している `"ピューン!"` は廃止。

### 1-4. #135(常時微振動) × #132(凹は抜け出しにくい)

数値で両立させる。

- 微振動は **走った距離から決まる決定的な正弦波で平均0**(乱数を使わない)。これで「振動だけで一方向に流されて凹から出る/場外に落ちる」が起きない
- 凹の中の摩擦 `HOLLOW_FRICTION=3.0`、脱出に必要な速さ `HOLLOW_EXIT_SPEED=1.5`。微振動の最大加速度は 0.08G×8 = 0.64マス/s² → 終端速度 0.21 < 1.5 で抜けられない。傾き最大(6マス/s²)なら終端2.0で約0.5秒後に抜けられる。G イベント(0.3G=2.4マス/s²)が重なると早く抜ける、という設計意図に合う
- 微振動の振幅 < `BRACE_G`(0.15) にして、女の子が常時「ふんばり」にならないこと、凸を踏んだ時の contact_g に微振動が乗っても `Landing::Light` に収まること(0.08 → 摩擦0.96 < 1.4)をテストで固定する

### 1-5. #128 × #130

- #128 の画像は「女の子だけの切り抜き(透過PNG)」である必要がある。#130 の横構図は「テキストの軽トラ+路面+信号」を組み、その荷台の上に女の子の画像矩形を置く方式にするため、軽トラごと描かれた画像だと構図が組めない
- 既存プロンプトは front-three-quarter view。#130 で右横から見る構図にするなら、再生成時は「女の子のみ・右側面寄り」で依頼するのがよい(要確認)

### 1-6. #134 × app.rs のカウントダウン

現状は `Screen::Countdown`(app.rs)→ `BeigomaGame::new()`。2ROUND制で ROUND ごとにカウントダウンが要るので、ハヤウチ(quick_draw.rs)と同じく **ゲーム内で `CountdownState` を回す**方式に変え、app.rs 側の外側カウントダウンは通さない(通すと ROUND1 で2回連続する)。#136 のスプラッシュも app.rs の同じ導線を触るので #134 の直後にやる。

---

## 2. 実装順序と依存

```
#127 壁削除・場外 ──→ #132 凹凸 ──→ #133+#134 ROUND/ゴール生成/ゲーム内カウントダウン ──→ #136 スプラッシュ(app.rs)
                                  └→ #135 微振動(truck.rs)  ──→ #129 演出(セリフ/ぴよーん) ──→ #130 横構図 ──→ #128 画像差替
                                                                 #131 疑似3D(render.rs、#132後ならいつでも)
```

理由: 物理(board.rs)の型変更を先に固めないと、描画・テストが二度手間になる。#135 は #127/#132 の不変条件(場外に出ない・凹から出ない)をテストするので、その後。#130 は #135 の揺れと #129 の吹き出しを使うので最後。

---

## 3. 詳細設計

### 3-1. #127 壁削除・場外GAME OVER

**board.rs**

```rust
// 削除: WALL_RESTITUTION, MIN_POS, fn reflect(), bounce()内のclamp
// 変更
pub const BOUNCE_SPEED: f64 = 4.0;   // 10→4。壁が無くなったので盤内で収まる強さ
pub const BOUNCE_KICK: f64 = 0.5;    // 1.0→0.5

impl Board {
    /// 位置が盤の上にあるか。ベーゴマの中心が縁を越えたら場外(落ちる)
    pub fn contains(pos: (f64, f64)) -> bool
}

pub enum StepEvent {
    Hopped { contact_g: f64 },
    Landed(Landing),
    /// 盤から落ちた(場外)
    FellOff,
    Goal,
}

impl Top {
    /// 速度のぶん進める(縁で跳ね返らない)
    fn advance(&mut self, secs: f64)
}
```

`step()`/`fly()` とも `advance` 直後に `if !Board::contains(self.pos) { return Some(StepEvent::FellOff); }`。飛び上がり中に縁を越えた場合も落ちる。

**beigoma.rs**

```rust
pub enum Outcome {
    Cleared { time: Duration },
    /// 吹っ飛んだ・盤から落ちた(どちらも同じ演出のGAME OVER)
    Flown,
    TimeUp,
}
/// 女の子のセリフ(弾かれ・吹っ飛び・場外で共通)
pub const GASP_LINE: &str = "ああっ!!";
```

`on_step_event`: `FellOff => finish(Flown)`。HUD 中央は `"GAME OVER"` のみ(「吹っ飛んだ!」の文言は削除)。セリフは #129 の `speech` へ。

### 3-2. #132 凹(Hollow)と凸(Bump)

**board.rs**

```rust
pub enum Cell {
    Flat,
    /// 凸(でっぱり)。踏むと飛び上がり、着地の摩擦が踏んだ時のGで増える(現行のBump)
    Bump,
    /// 凹(くぼみ)。正面から入るとハマって抜け出しにくい。斜めに入ると側面をこすって弾かれる
    Hollow,
    Goal,
}
// LAYOUTの文字: '.'=平坦 '#'=凸 'u'=凹 'S'=投入位置 (ゴールは#133でランダムなので文字を持たない)

/// 凹の中の摩擦(1秒あたりの減衰率)。平坦の0.8に対して大きく、ハマる
pub const HOLLOW_FRICTION: f64 = 3.0;
/// 凹から抜け出すのに必要な速さ(マス/秒)。傾き最大(6マス/s^2)なら約0.5秒で届き、微振動だけでは届かない
pub const HOLLOW_EXIT_SPEED: f64 = 1.5;
/// 凹に入る向きと縁の法線のなす角のcos。これ未満(60°より浅い)なら側面をこすって弾かれる
pub const HOLLOW_GRAZE_COS: f64 = 0.5;
/// 側面をこすった時に着地摩擦へ足す分。G=0でもBounce、G>0.45で吹っ飛ぶ
pub const HOLLOW_GRAZE_FRICTION: f64 = 0.7;

pub enum TopState {
    Rolling,
    Airborne { remaining: Duration, contact_g: f64 },
    /// 凹にハマっている。縁で位置を留め、速さがHOLLOW_EXIT_SPEEDに届いたら抜ける
    Sunk,
}

pub enum StepEvent {
    Hopped { contact_g: f64 },
    Landed(Landing),
    /// 凹に正面から入ってハマった
    Sank,
    /// 凹の側面をこすって弾かれた/吹っ飛んだ
    Grazed(Landing),
    /// 凹から抜け出した
    Escaped,
    FellOff,
    Goal,
}

pub struct Top {
    pub pos: (f64, f64),
    pub vel: (f64, f64),
    pub state: TopState,
    /// 直前のステップにいたマス。マスに入った瞬間の検出に使う(旧on_bump)
    prev_cell: (usize, usize),
}

/// 凹に入った向きの判定。prevからcurへ跨いだ辺の法線と速度のなす角で、正面(Sink)か斜め(Graze)か
pub fn hollow_contact(vel: (f64, f64), prev: (usize, usize), cur: (usize, usize)) -> HollowContact
pub enum HollowContact { Sink, Graze }

/// 状態と足元のマスから摩擦を決める(Sunkなら常にHOLLOW_FRICTION)
pub fn surface_friction(cell: Cell, state: TopState, g: f64) -> f64
```

`step()` の流れ(Rolling):
1. 加速(傾き+G)→摩擦→cap→advance
2. 場外なら `FellOff`
3. `cur = cell_of(pos)`。`cur != prev_cell` なら「入った瞬間」:
   - Bump → 従来通り hop(`Hopped`)
   - Hollow → `hollow_contact`。Sink: `state=Sunk`、`Sank`。Graze: `landing = classify_landing(landing_friction(g) + HOLLOW_GRAZE_FRICTION)`、Bounce なら `bounce()`、`Grazed(landing)`(Flown はゲーム側で GAME OVER)
   - Goal → `Goal`
4. `prev_cell = cur`

Sunk のステップ: 加速は同じ・摩擦は `HOLLOW_FRICTION`。advance 後に凹のマスから出る位置なら、速さ ≥ `HOLLOW_EXIT_SPEED` で `state=Rolling`・`Escaped`、未満なら**位置だけ**マスの内側にクランプ(速度は殺さない=壁を登る途中の蓄積を許す)。飛び上がりから凹に着地したら `Landed` を返しつつ `state=Sunk`(イベントは1ステップ1つ、Landed 優先)。

凸(Bump)の物理は現状維持(要望の記述と一致している)。

**render.rs**

```rust
pub const BUMP_GLYPH: &str = "▲";     // 凸
pub const HOLLOW_GLYPH: &str = "▽";   // 凹
const HOLLOW_BG: Color = Color::Rgb(60, 38, 18);   // 凸より暗い(穴)
const HOLLOW_FG: Color = Color::Rgb(120, 90, 60);
const HOLLOW_PIXEL: Rgba<u8> = Rgba([60, 38, 18, 255]);
```

画像合成 `compose_base`: 凹は暗い円(穴)、凸は明るい円+暗い縁(盛り上がり)。

**beigoma.rs** `on_step_event`: `Sank => message "ズボッ"`, `Escaped => message "ぬけた!"`, `Grazed(Bounce) => Bounce と同じ演出`, `Grazed(Flown) => finish(Flown)`。

### 3-3. #133 ゴールランダム + #134 2ROUND制

**board.rs**

```rust
pub const ROUNDS_PER_SESSION: u32 = 2;

pub struct GoalRule {
    /// 投入位置からゴールまでの最短距離(マス)
    pub min_distance: f64,
    /// 投入位置からゴールへの直線上に凹凸が1つ以上あること(直線移動だけでは届かない)
    pub blocked_straight_line: bool,
}

pub struct RoundParams {
    pub label: &'static str,                  // "ROUND 1 やさしい" / "ROUND 2 むずかしい"
    pub layout: &'static [&'static str; BOARD_HEIGHT],
    pub goal_rule: GoalRule,
}

/// round_index(0始まり)のパラメータ。最後のROUND以降は最後のまま(color_stackと同じ)
pub fn round_params(round_index: u32) -> RoundParams

const LAYOUT_ROUND1: [&str; BOARD_HEIGHT]  // 現LAYOUTから'G'を外し、'#'の一部を'u'にする(凹も体験させる)
const LAYOUT_ROUND2: [&str; BOARD_HEIGHT]  // 縁1マス内側に'#'/'u'を交互に並べた輪+内部に数個。投入位置は輪の内側

pub struct Board {
    cells: Vec<Cell>,
    start: (usize, usize),
    goal: (usize, usize),
}

impl Board {
    /// paramsのレイアウトに、ルールを満たす候補からランダムに選んだゴールを置く
    pub fn generate(params: &RoundParams, rng: &mut impl Rng) -> Self
    /// ゴールを指定して作る(テストで決定的にするため)。候補外の位置はpanic
    pub fn with_goal(layout: &[&str; BOARD_HEIGHT], goal: (usize, usize)) -> Self
    /// ルールを満たすゴール候補(空にならないことをテストで保証する)
    pub fn goal_candidates(layout: &[&str; BOARD_HEIGHT], rule: &GoalRule) -> Vec<(usize, usize)>
    pub fn goal(&self) -> (usize, usize)     // #[cfg(test)]を外す
    /// fromからtoへの線分上(0.25マス刻み)に凹凸があるか
    pub fn straight_line_is_blocked(&self, from: (f64, f64), to: (f64, f64)) -> bool
}
```

`Board::standard()` は削除し、既存テストは `Board::with_goal(&LAYOUT_ROUND1, (17,1))` に置き換える(現在の G の位置)。

**beigoma.rs**

```rust
use crate::ui::countdown::{self, CountdownState};

enum Status {
    /// ROUND開始前の「3.2.1.GO!!」。新しい盤は見せておき、ベーゴマはまだ置かない
    Countdown { state: CountdownState },
    Playing,
    /// ROUNDが終わった。shownは終了表示を出してからの時間
    Ended { outcome: Outcome, shown: Duration },
}

pub struct BeigomaGame {
    round_index: u32,
    params: RoundParams,
    board: Board,
    top: Top,        // Countdown中は投入位置に置くが描画しない(TopView省略)
    ...
    tracker: ScoreTracker,   // with_session_length(ROUNDS_PER_SESSION)
}

impl BeigomaGame {
    pub fn new() -> Self                       // start_round(0)
    fn with_truck(truck: Truck) -> Self
    /// round_index番目のROUNDをカウントダウンから始める。盤を作り直し、軽トラのコースはそのまま続く
    fn start_round(&mut self, round_index: u32)
    /// GO!!が終わった: ベーゴマを投入し、制限時間を数え始める
    fn drop_top(&mut self)
    fn current_round_number(&self) -> u32     // 1始まり
}
```

- `update`: Countdown なら `state.tick(dt)` で SE、終了で `drop_top()`。Ended で `shown ≥ END_HOLD` なら次 ROUND があれば `start_round(round_index+1)`。
- `is_finished`: `tracker.is_session_finished() && Ended{shown ≥ END_HOLD}`。
- `handle_key`: Playing 以外は無視(カウントダウン中の連打で傾かない)。
- `render`: 盤面パネルの上に Countdown 中は `countdown::render(frame, board_inner, state)` を重ねる。HUD パネルの title に `params.label` を出す。
- 時間: `elapsed` は ROUND ごとにリセット。制限時間60秒/ROUND。
- app.rs: `select_menu_item(BEIGOMA)` → `start_beigoma()`(`start_quick_draw` と同じ形。BGM Playing、`Screen::Playing(Box::new(BeigomaGame::new()))`)。`new_game` の BEIGOMA は `unreachable!`。#136 で間にスプラッシュが入る。

ROUND1 が GAME OVER の場合、ROUND2 へは進めずセッション終了とする(確定)。`tracker` は ROUND1 の1件のみで `is_session_finished()` になるよう、`finish` 側で `round_index==0 && !success` なら残り ROUND を打ち切る分岐を入れる。

### 3-4. #135 常時の微振動

**truck.rs**

```rust
/// 路面のゆっくりした揺れ(G)。走った距離から決まる正弦波で平均0(乱数でない)
pub const SWAY_G: f64 = 0.08;
/// 揺れの波長(m)。巡航11m/sで前後約0.3Hz・左右約0.2Hz
pub const SWAY_WAVELENGTH_LONG: f64 = 36.0;
pub const SWAY_WAVELENGTH_LAT: f64 = 55.0;
/// 細かいガタつき(前後のみ、主に軽トラ視点の揺れ表示用)
pub const RATTLE_G: f64 = 0.03;
pub const RATTLE_WAVELENGTH: f64 = 1.4;   // 約8Hz

/// 位置position(m)・速度speedでの路面の揺れ。速度に比例し、止まっていれば0
pub fn road_jitter(position: f64, speed: f64) -> GForce

impl Truck {
    /// イベント(ブレーキ・発進・操舵・段差)によるG(旧current_g)
    pub fn event_g(&self) -> GForce
    /// 路面の微振動
    pub fn jitter(&self) -> GForce
    /// 盤にかかっているG = event_g + jitter
    pub fn current_g(&self) -> GForce
    /// 微振動を切る(テストでベーゴマを静止させるため)
    pub fn without_jitter(self) -> Self
    /// ガタつきの山側か(軽トラ視点で車体を1行持ち上げる合図)
    pub fn is_rattling(&self) -> bool
}
```

内部: `jitter_scale: f64`(1.0 / without_jitter で 0.0)。速度係数 `(speed / CRUISE_SPEED).clamp(0.0, 1.0)`。

既存テストへの影響: `cruising_has_no_g`・`truck_stops_waits_on_red_...` の `current_g()==default` は `event_g()` へ。beigoma.rs の `calm_game()` は `.without_jitter()` を付け、微振動ありの専用テストを別に足す。

### 3-5. #129 回転を激しく・「ぴよーん!!」・セリフ

**beigoma.rs**

```rust
/// 回転の見た目の進み方。止まっていても回り、速く転がるほど速く回る
const SPIN_BASE_HZ: f64 = 16.0;            // 旧120ms間隔=8.3Hz の約2倍
const SPIN_HZ_PER_SPEED: f64 = 2.0;        // 速さ1(マス/秒)あたり
/// 弾かれ演出(ぴよーん!!)の長さと1コマ
pub const BOUNCE_FX_DURATION: Duration = Duration::from_millis(700);
const BOUNCE_FX_FRAME: Duration = Duration::from_millis(80);
/// セリフの吹き出しを出し続ける時間
const SPEECH_HOLD: Duration = Duration::from_millis(1200);

pub struct BeigomaGame {
    ...
    /// 回転の位相(コマ数の累積)。spin_frame = phase as usize % 枚数
    spin_phase: f64,
    /// 弾かれ演出の経過時間(演出中のみ)
    bounce_fx: Option<Duration>,
    /// 女の子のセリフ(文言, 出してからの時間)
    speech: Option<(&'static str, Duration)>,
}
```

- `Landed(Bounce)` / `Grazed(Bounce)`: SE Incorrect、`bounce_fx=Some(0)`、`speech=Some((GASP_LINE,0))`。
- `finish(Flown)`: `speech=Some((GASP_LINE,0))`(終了後も speech の時間は進める)。
- Ended バナーが出ている間は bounce_fx を描かない(バナー優先)。

**render.rs**

```rust
pub const TOP_SPIN_GLYPHS: [&str; 4] = ["◐", "◓", "◑", "◒"];   // 据え置き。コマ送りの速さと色の交互で激しさを出す
/// 回転の見た目をコマごとに交互に変える文字色(残像感)
const TOP_SPIN_FG: [Color; 2] = [Color::Rgb(220, 240, 255), Color::Rgb(255, 255, 255)];
/// 弾かれ演出のコマ(つぶれる→伸びる→戻る)
pub const TOP_BOUNCE_GLYPHS: [&str; 4] = ["●", "◉", "○", "◎"];
pub const BOUNCE_LINE: &str = "ぴよーん!!";

pub struct TopView {
    pub pos: (f64, f64),
    pub airborne: bool,
    pub spin_frame: usize,
    /// 弾かれ演出中のコマ(Noneなら通常)
    pub bounce_frame: Option<usize>,
}

pub struct TruckViewInfo {
    pub speed: f64,
    pub g: GForce,
    pub upcoming: Option<Upcoming>,
    /// 女の子のセリフ(吹き出し)
    pub speech: Option<&'static str>,
}
```

- テキスト: bounce 中は `TOP_BOUNCE_GLYPHS[frame]` と、ベーゴマのマスの1行上(0行目なら1行下)に `BOUNCE_LINE` を描く(area でクリップ)。
- 画像: `PatchCache.key` を `(Rect, bool, usize /*spin_frame*/, Option<usize> /*bounce_frame*/)` に拡張。回転は `imageops::rotate90` の回数で表す(4コマ)。パッチは1〜2マス分の小画像なので毎コマ再エンコードして良いと判断。**既存テスト `patch_is_reencoded_only_when_the_top_changes_cells` の「回転のコマが変わっても作り直さない」を「コマが変わったら作り直す、盤全体は作り直さない」に書き換える**。`BOUNCE_LINE` は画像の上のセルの skip を外して文字で描く(パッチと同じ手法)。

### 3-6. #136 べー専用スプラッシュ

**splash.rs**

```rust
pub const BEIGOMA_SPLASH_IMAGE_PATH: &str = "beigoma_splash.jpeg";   // ttr_splash.jpegに倣う
pub const BEIGOMA_FALLBACK: FallbackText = FallbackText {
    title: "べ ー",
    subtitle: "軽トラの荷台でベーゴマをゴールへ",   // 要確認
};
```

**app.rs**

```rust
pub enum Screen {
    ...
    /// べーのスプラッシュ。Enter/クリックでゲーム開始、qでメニュー
    BeigomaSplash,
}
struct App { ..., beigoma_splash_renderer: SplashRenderer }

fn select_menu_item: BEIGOMA → audio::play_se(Transition); self.screen = Screen::BeigomaSplash;
fn start_beigoma(&mut self)   // BGM Playing → Screen::Playing(Box::new(BeigomaGame::new()))
handle_key: BeigomaSplash + Enter → start_beigoma();  handle_mouse: 同様
render: BeigomaSplash → タイトル画面(Screen::Splash)と同じ「背景+panel+内側中央に画像」
```

TTR は曲選択の背景として使っているが、べーは「Enterで開始」なのでタイトル画面側の形に合わせる。既存テスト `selecting_beigoma_skips_difficulty_and_starts_after_countdown` / `enter_on_beigoma_in_menu_goes_straight_to_countdown` は「BeigomaSplash → Enter → Playing で ROUND1 カウントダウン中」に書き換え。

### 3-7. #131 盤の疑似3D(傾きで変形)

**render.rs**

```rust
/// 左右の傾き1あたりの行ごとの横ずれ(セル)。上端と下端で逆向きにずらして平行四辺形にする(roll=4で±2セル)
pub const SHEAR_CELLS_PER_LEVEL: f64 = 0.5;
/// 前後の傾き1あたりの盤全体の上下ずれ(行)。pitch=4で1行
pub const PITCH_SHIFT_ROWS_PER_LEVEL: f64 = 0.25;
/// 前後の傾きによる明暗。高い側を明るく、低い側を暗く(FLAT_BGの倍率)
pub const PITCH_SHADE_PER_LEVEL: f64 = 0.04;
const MAX_SHEAR_CELLS: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TiltView { pub pitch: f64, pub roll: f64 }

pub struct BoardArea {
    pub rect: Rect,
    pub cell_width: u16,
    pub cell_height: u16,
    /// 行ごとの横ずれ(セル)。傾き0なら全部0
    pub row_shift: [i16; BOARD_HEIGHT],
    /// 行ごとの明るさ倍率(1.0=そのまま)
    pub row_shade: [f32; BOARD_HEIGHT],
}

pub fn board_area(area: Rect, tilt: TiltView) -> Option<BoardArea>   // 幅から2*MAX_SHEAR_CELLSを引いてscaleを決める
pub fn row_shift_of(roll: f64, y: usize) -> i16
pub fn row_shade_of(pitch: f64, y: usize) -> f32
impl BoardArea { pub fn cell_rect(&self, x, y) -> Rect  /* row_shift[y]を足す */ }
```

- `BoardRenderer::render(frame, area, board, top: Option<&TopView>, tilt: TiltView)`。
- 画像モード: `BaseCache.area == layout` の比較に `row_shift` が含まれるので、整数の横ずれパターンが変わった時だけ base を再合成する(再合成時は行ごとにピクセルをずらすだけで安価)。減衰中は約0.7秒に1回、連打中は押下ごとに1回の再エンコード。これを避けたい場合はテキストのみ適用(§5-4)。
- 既存の `board_area` テスト(幅50→x=5、90×26→(4,2)、200×60→3倍)は margin 込みでも同じ値になるので据え置き。

### 3-8. #130 軽トラ視点を右横構図に

女の子以外はテキストで組む(画像は女の子の矩形だけ)。純粋関数を先に固めてテストする。

**render.rs**

```rust
/// 軽トラ視点(右横から見た構図)の場面。描画前に純粋に求める
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TruckScene {
    /// 車体を1行持ち上げるか(ガタつきの山側)
    pub lift: bool,
    /// 女の子の姿勢
    pub pose: Pose,
    /// 前方イベントの画面上のx(パネル内側基準)。無ければNone
    pub upcoming_x: Option<u16>,
    /// 路面の破線の位相(走った距離から)
    pub road_phase: usize,
}
pub enum Pose { Normal, LeanForward, LeanBack, Brace }

/// 姿勢の閾値。前後Gがこれ以上で体が持っていかれ、BRACE_G以上で踏ん張り
pub const LEAN_G: f64 = 0.05;
/// 軽トラを置く位置(パネル幅に対する割合)。イベントは右端(VISIBLE_DISTANCE)から近づいてくる
pub const TRUCK_X_RATIO: f64 = 0.2;
pub const ROAD_DASH_PERIOD_M: f64 = 2.0;

pub fn pose_of(g: GForce) -> Pose
pub fn scene_x(distance: f64, width: u16) -> u16          // distance=0で軽トラの位置、VISIBLE_DISTANCEで右端
pub fn truck_scene(info: &TruckViewInfo, width: u16) -> TruckScene

pub struct TruckViewInfo {
    pub speed: f64,
    pub g: GForce,
    pub upcoming: Option<Upcoming>,
    pub speech: Option<&'static str>,
    /// 走った距離(m)。路面の破線を流すのに使う
    pub position: f64,
    pub rattling: bool,
}
```

パネル内側の縦構成: 見出し(前方案内 1行) / 情景(信号の柱「●│」を色付き・障害物「▮」・段差「▲」を `upcoming_x` に) / 荷台+女の子(画像 or テキスト。`lift` で1行上へ) / 軽トラのテキスト絵(3行) / 路面の破線1行(`road_phase` で流す) / 脚注(ふんばり・速度・G、現行どおり)。吹き出しは女の子の右上に `speech` を1行。

画像側: `Pose`×`lift` で矩形が変わるたび再エンコードになるのを避けるため、`RefCell<HashMap<(Pose, bool), StatefulProtocol>>` に矩形ごとに遅延生成して保持(画像は Normal/Brace の2枚。Lean は Normal を x±1 セルずらして置く)。

- 「右横から見た」= 車の右側面が見えるので前は画面右、イベントは右から近づく。
- 操舵(横G)は横構図では見えないので、`Pose` と「よける→/←よける」の文字で補う。

### 3-9. #128 画像化

- 仕組みは実装済み。追加は `TruckViewRenderer::with_images(picker, normal: RgbaImage, brace: RgbaImage)`(`#[cfg(test)]`、BoardRenderer と同じ)で、合成画像を使って画像経路のテストを通す。
- アセット到着時に `beigoma_truck_images_are_embedded` / `..._can_be_decoded` を splash.rs のテストに倣って追加(到着前は書かない=赤のまま放置しない)。
- 生成依頼の条件: 女の子のみ・透過PNG・右側面寄りの構図(§1-5)。

---

## 4. TDD テストケース案(機能ごとの最小セット)

命名は既存に合わせ英語 snake_case、assert メッセージは日本語。

**#127**
- `top_falls_off_when_its_center_crosses_the_rim`: (0.6,5.5) から左へ → 数ステップで `FellOff`。x/y 4辺で
- `airborne_top_also_falls_off_the_rim`: 飛び上がり中に縁越え → `FellOff`
- `bounce_from_the_center_stays_on_the_board`: 盤中央で Bounce → 場外にならず速度が減衰して止まる(BOUNCE_SPEED の上限を固定)
- `falling_off_is_an_immediate_game_over_with_the_gasp`: `FellOff` → `Outcome::Flown`、speech==GASP_LINE
- `flying_off_and_falling_off_show_the_same_game_over`: 描画文字列に "GAMEOVER" と "ああっ!!" が両方
- 既存の `top_stays_inside_the_board_and_bounces_off_the_rim` は削除

**#132**
- `layout_chars_map_to_bump_and_hollow`: '#'→Bump、'u'→Hollow
- `entering_a_hollow_head_on_sinks_the_top`: 法線方向で入る → `Sank`、state==Sunk
- `entering_a_hollow_at_a_shallow_angle_grazes_and_bounces`: cos<0.5 → `Grazed(Bounce)`、速度が反転
- `grazing_under_high_g_flies_the_top_off`: G=0.6 → `Grazed(Flown)`
- `hollow_contact_threshold_is_exact`: cos ちょうど 0.5 は Sink、0.5-1e-6 は Graze
- `sunk_top_cannot_leave_below_the_exit_speed`: 傾き1段では60秒経っても Hollow のマス内
- `sunk_top_escapes_with_full_tilt`: 傾き最大で1秒以内に `Escaped`
- `sunk_top_keeps_building_speed_while_clamped`: クランプ後も速さが単調増加(速度を殺していない)
- `landing_on_a_hollow_sinks_after_landing`: 凸→飛ぶ→凹に着地 → `Landed(Light)` かつ state==Sunk
- `bump_behaviour_is_unchanged`: 既存の3段階テストがそのまま通る
- render: `text_board_distinguishes_hollow_from_bump`(記号・背景色が異なる)、画像 `compose_base_paints_hollow_darker_than_bump`

**#133/#134**
- `goal_candidates_are_never_empty_for_every_round`
- `every_candidate_satisfies_the_rule`: 平坦・距離・(ROUND2)直線が遮られている
- `round2_goal_is_never_reachable_in_a_straight_line`: 100シードで `straight_line_is_blocked(start, goal)`
- `goal_is_random_across_seeds`: seed 0..20 で2種類以上
- `with_goal_rejects_a_non_candidate`(`#[should_panic]`)
- `straight_line_is_blocked_detects_a_bump_on_the_segment` / `..._is_false_on_a_clear_row`
- `round_params_has_two_rounds_and_clamps_beyond`: label/layout、`round_params(5)==round_params(1)`
- `round2_has_obstacles_on_the_rim`: 縁から1マス以内に凹凸が上下左右すべてにある
- `new_game_starts_round1_with_a_countdown_and_no_top`: Status::Countdown、描画に "3"、ベーゴマ記号なし、残り60.0秒
- `keys_are_ignored_during_the_countdown`
- `go_drops_the_top_and_starts_the_clock`: `COUNTDOWN_TOTAL` 経過 → Playing、elapsed 0 から
- `round1_end_leads_to_round2_countdown_after_the_hold`: Cleared → END_HOLD → Countdown、round 2、盤が ROUND2 レイアウト
- `session_finishes_after_round2_end_display`: result.total==2
- `hud_shows_the_round_label`
- app.rs: `selecting_beigoma_goes_to_its_splash` / `enter_on_beigoma_splash_starts_round1_countdown_in_game` / `new_game_never_creates_beigoma`

**#135**
- `road_jitter_is_zero_when_stopped` / `..._is_bounded_by_sway_plus_rattle`
- `road_jitter_averages_to_zero_over_a_wavelength`(1周期を積分して |平均| < 1e-3)
- `jitter_never_reaches_the_brace_threshold`: 0..400m を 0.1m 刻みで `magnitude() < BRACE_G`
- `current_g_is_event_g_plus_jitter`
- `without_jitter_restores_the_calm_truck`(既存の静止テストの前提)
- board: `jitter_alone_moves_the_top_a_little`: 3秒で >0.05マス動く / `jitter_alone_keeps_the_top_near_the_start_for_a_minute`: 60秒で始点から1マス以内(場外に出ない)
- `jitter_alone_cannot_free_a_sunk_top`
- `bump_contact_under_jitter_alone_is_a_light_landing`

**#129**
- `spin_advances_faster_when_the_top_rolls_faster`: 静止1秒のコマ数 < 速さ5で1秒のコマ数
- `bounce_starts_the_fx_and_the_gasp`: bounce_fx Some、speech==GASP_LINE
- `bounce_fx_ends_after_its_duration` / `speech_disappears_after_the_hold`
- `gasp_stays_on_screen_after_game_over`
- render: `text_board_draws_the_bounce_line_above_the_top` / `..._below_when_on_the_top_row` / `end_banner_hides_the_bounce_line`
- image: `patch_is_reencoded_per_spin_frame_but_the_base_is_not`(既存テストの置換)

**#136**
- `beigoma_splash_image_asset_is_embedded`(画像配置で緑)、`missing_image_falls_back_to_beigoma_text`
- app.rs: `q_on_beigoma_splash_returns_to_menu`、`click_on_beigoma_splash_starts_the_game`、`beigoma_splash_uses_its_own_fallback_text`

**#131**
- `row_shift_is_zero_without_roll` / `row_shift_is_antisymmetric_top_and_bottom` / `row_shift_is_bounded_by_max_shear`
- `row_shade_is_brighter_on_the_raised_side`
- `board_area_keeps_the_same_scale_and_center_as_before`(既存値を固定)
- `sheared_cells_never_leave_the_area`(全 tilt 極値で描画してパニックなし・クリップ)
- image: `base_is_recomposed_only_when_the_integer_shear_changes`

**#130**
- `pose_of_thresholds`: 0→Normal、-0.06→LeanForward、+0.06→LeanBack、0.15→Brace
- `scene_x_maps_distance_to_the_road`: 0→軽トラ位置、VISIBLE_DISTANCE→右端、単調
- `signal_is_drawn_in_its_color_at_scene_x`
- `truck_lifts_one_row_while_rattling`(2描画の差分が1行分)
- `road_dashes_scroll_with_position`
- `speech_bubble_appears_next_to_the_girl`
- `truck_view_does_not_panic_in_tiny_areas`(既存を維持)
- image: `with_images_uses_the_picture_and_keeps_the_text_scenery`

---

## 5. 確認済み・残る判断

1. **ROUND1 で GAME OVER になった時ROUND2へ進むか**: 進まずセッション終了(確定)
2. セリフ「ああっ!!」(感嘆符2つ)への統一、`BOUNCE_SPEED`/`BOUNCE_KICK` の初期値、#131の画像モード適用可否、#136のファイル名・フォールバック文言、#135の振幅は実装しながら妥当な値を決め、動きを見て調整する
3. #128 の再生成条件(女の子のみ・透過・右側面寄り)は#128着手時にあらためて相談する

---

## 6. 影響ファイル

| ファイル | 変更内容 |
|---|---|
| `src/game/beigoma/board.rs` | 壁削除・場外・Cell 3種・凹の物理・prev_cell・ROUND/GoalRule/generate・直線遮蔽判定 |
| `src/game/beigoma/truck.rs` | road_jitter・event_g/jitter/current_g 分離・without_jitter・is_rattling |
| `src/game/beigoma.rs` | Status に Countdown・ROUND 進行・speech/bounce_fx/spin_phase・HUD ラベル・GASP_LINE |
| `src/game/beigoma/render.rs` | 凹凸の記号/色/合成・TopView/TruckViewInfo 拡張・パッチ key 拡張・BoardArea の shear/shade・横構図(TruckScene)・with_images |
| `src/app.rs` | Screen::BeigomaSplash・start_beigoma・new_game から除外・beigoma テスト書換 |
| `src/ui/splash.rs` | BEIGOMA_SPLASH_IMAGE_PATH / BEIGOMA_FALLBACK |
| `docs/beigoma-game-spec.md` | 固定配置→ゴール毎回変更、2ROUND、壁なし、凹凸2種を反映(または新規 spec を追加) |
| `assets/image/beigoma_splash.jpeg`、`assets/image/beigoma/truck_{normal,brace}.png` | 配置待ち(現状ディレクトリ自体が無い) |

---

## 推奨実装順序

1. **#127** 壁削除・場外 `FellOff`・`prev_cell` 一般化・BOUNCE 係数見直し(board.rs → beigoma.rs)
2. **#132** 凹凸2種の物理と表示(board.rs → render.rs → beigoma.rs のメッセージ)
3. **#133+#134** RoundParams/GoalRule/`Board::generate`・ゲーム内カウントダウン・2ROUND 進行・app.rs の導線を `start_beigoma` へ(仕様書の固定配置記述も改訂)
4. **#135** 微振動(truck.rs)と #127/#132 との相互作用テスト(場外に出ない・凹から出ない・ふんばり閾値未満)
5. **#129** 回転速度・弾かれ演出・セリフ(`speech` を TruckViewInfo に追加、HUD の「ピューン!」廃止、パッチ key 拡張)
6. **#136** スプラッシュ画面(splash.rs 定数 → app.rs Screen 追加 → 画像配置)
7. **#131** 盤の shear/shade(テキスト先行。画像適用は確認事項4の回答後)
8. **#130** 横構図のテキスト実装(TruckScene の純粋関数 → 描画)
9. **#128** 画像到着後に `with_images` 経路で確認、埋め込みテスト追加
