## 概要

「べー」の盤上の物理(`src/game/beigoma/board.rs`)に次の2つの変更を加える。

1. 凹凸に関わる摩擦係数を上げる。着地の結果(軽い着地/弾かれ/吹っ飛び)を分ける摩擦の閾値は現在の値で固定し、摩擦係数を上げたぶんだけ同じGでも弾かれ・吹っ飛びになりやすくする。平坦な場所の転がり摩擦(`ROLLING_FRICTION`)は変えない
2. 凸(Bump)にも凹(Hollow)と同じ「側面接触」を導入する。正面から踏んだ場合は従来どおり飛び上がって着地の判定をする。速さが一定以上で斜めから当たった場合は飛び上がらず、その場で側面をこすって、速さとGに応じて弾かれるか吹っ飛ぶ。入り方(正面/斜め)の判定は凹専用だった`HollowContact`/`hollow_contact`を凸・凹共通の`EdgeContact`/`edge_contact`にして両方で使う

## 挙動の決まり

### 摩擦

- 着地の摩擦の閾値は摩擦の値で固定する: 軽い着地は`1.4`以下、弾かれは`1.4`より大きく`2.4`以下、吹っ飛びは`2.4`より大きい(現在の計算値と同じ)
- 凸を正面から踏んだ時の着地の摩擦は`0.8 + 3.0 × G`。G相当の境目は 軽い着地/弾かれ が`0.2G`、弾かれ/吹っ飛び が`0.533G`
- 凹の側面をこすった時の摩擦は`0.8 + 3.0 × G + 1.0`。G=0でも弾かれ、`0.2G`を超えると吹っ飛ぶ
- 凹にハマっている間の摩擦は`4.5`。抜け出すのに必要な速さは`1.0`。抜け出せる条件は 加速度 ≥ `4.5 × 1.0 = 4.5`マス/秒² のままなので、傾きだけで抜けられるのは5回押し(傾き3.5)以上で変わらず、傾き最大(6.0マス/秒²)なら約0.6秒で抜ける。ハマっている間の終端速度は傾き最大で約1.3マス/秒(従来約1.9)になり、凹の中での動きが鈍くなる。巡航の揺れ(`CRUISE_VIBRATION_G`=0.06)だけでは終端速度約0.1で抜けられない
- 巡航の揺れだけで凸を踏んだ時の摩擦は`0.8 + 3.0 × 0.06 = 0.98`で軽い着地のまま。凹の側面をこすった時は`1.98`で弾かれ止まり(吹っ飛ばない)

コース上のGごとの結果(`truck.rs`の`COURSE`から求めた値):

| 出来事 | G | 凸を正面から踏む | 凹の側面をこする |
|---|---|---|---|
| 巡航の揺れ | 0.06 | 軽い着地 | 弾かれ |
| 発進 | 0.26 | 弾かれ | 吹っ飛び |
| ブレーキ1(気づき0.9秒) | 0.31 | 弾かれ | 吹っ飛び |
| 段差1(高さ0.03) | 0.33 | 弾かれ | 吹っ飛び |
| 回避1(10m手前) | 0.37 | 弾かれ | 吹っ飛び |
| 段差2(高さ0.08) | 0.88 | 吹っ飛び | 吹っ飛び |
| 回避2(6.5m手前) | 0.88 | 吹っ飛び | 吹っ飛び |
| ブレーキ2(気づき2.2秒) | 1.06 | 吹っ飛び | 吹っ飛び |

### 凸の側面接触

- 凸のマスに入った瞬間に、跨いだ縁の法線と速度のなす角のcosを求める(凹と同じ計算)。cosが`0.5`以上(法線から60°以内)なら正面、`0.5`未満なら斜め
- 斜め、かつ入った瞬間の速さが`3.0`マス/秒以上なら側面接触。飛び上がらず、摩擦`0.8 + 3.0 × G + 0.25 × 速さ`で弾かれ/吹っ飛びを決める。速さ3.0以上なら摩擦は`1.55`以上になるので、側面接触で軽い着地になることはない
  - G=0のとき: 速さ3.0〜6.4で弾かれ、6.4を超えると吹っ飛ぶ(傾き最大の終端速度7.5マス/秒で当たると吹っ飛ぶ。弾かれた直後の速さ4.0で当たると再び弾かれる)
  - G=0.2のとき: 速さ4.0を超えると吹っ飛ぶ
  - G=0.33(段差1)のとき: 側面接触はすべて吹っ飛ぶ
- 斜めでも速さが`3.0`未満なら、正面と同じく飛び上がって着地の判定をする(ゆっくり乗り上げる)
- 正面なら速さによらず飛び上がる(従来どおり)
- 弾かれる時の動きは凹の側面と同じ`bounce()`(来た方向へ`BOUNCE_SPEED`で弾き返し、`BOUNCE_KICK`だけ位置を戻す)。弾かれた先のマスには入った扱いにする
- 出来事は凹と同じ`StepEvent::Grazed(Landing)`で返す。`beigoma.rs`の`on_step_event`は凸・凹を区別しないので変更しない

## 定数

```rust
/// 着地の瞬間の摩擦が、踏んだ時のG1あたりに増える量
pub const FRICTION_PER_G: f64 = 3.0;
/// 着地の摩擦の閾値(摩擦の値で固定)。これ以下なら軽い着地、これを超えると弾かれる
pub const LOW_FRICTION_THRESHOLD: f64 = 1.4;
/// 着地の摩擦の閾値(摩擦の値で固定)。これを超えると吹っ飛ぶ
pub const HIGH_FRICTION_THRESHOLD: f64 = 2.4;
/// 閾値のG相当(凸を正面から踏んだ時)。0.2G / 0.533G
pub const LOW_G_THRESHOLD: f64 = (LOW_FRICTION_THRESHOLD - ROLLING_FRICTION) / FRICTION_PER_G;
pub const HIGH_G_THRESHOLD: f64 = (HIGH_FRICTION_THRESHOLD - ROLLING_FRICTION) / FRICTION_PER_G;
/// 凹にハマっている間の摩擦(1秒あたりの速度の減衰率)
pub const HOLLOW_FRICTION: f64 = 4.5;
/// 凹から抜け出すのに必要な速さ(マス/秒)。抜けられる条件は 加速度 ≥ HOLLOW_FRICTION × HOLLOW_EXIT_SPEED = 4.5マス/秒²
pub const HOLLOW_EXIT_SPEED: f64 = 1.0;
/// 凹凸のマスに入る向きと、跨いだ縁の法線のなす角のcos。これ未満(60°より浅い角度)なら斜め(側面接触)
pub const GRAZE_COS: f64 = 0.5;
/// 凹の側面をこすった時に、着地の摩擦へ上乗せする分。G=0でも弾かれ、G>0.2で吹っ飛ぶ
pub const HOLLOW_GRAZE_FRICTION: f64 = 1.0;
/// 凸に斜めから当たった時に側面接触とみなす最低の速さ(マス/秒)。これ未満なら正面と同じく飛び上がる
pub const BUMP_GRAZE_SPEED: f64 = 3.0;
/// 凸の側面をこすった時に、着地の摩擦へ上乗せする分の速さ1(マス/秒)あたりの量。
/// G=0なら速さ3.0で1.55(弾かれ)、6.4を超えると吹っ飛ぶ
pub const BUMP_GRAZE_FRICTION_PER_SPEED: f64 = 0.25;
```

`HOLLOW_GRAZE_COS`は`GRAZE_COS`に改名する。`ROLLING_FRICTION`(0.8)・`BOUNCE_SPEED`・`BOUNCE_KICK`・`HOP_DURATION`・`MAX_SPEED`は変えない。

定数どうしの関係(テストでconst assertとして固定する):

- `LOW_FRICTION_THRESHOLD < HIGH_FRICTION_THRESHOLD`
- `ROLLING_FRICTION + HOLLOW_GRAZE_FRICTION > LOW_FRICTION_THRESHOLD`(凹の側面接触はG=0でも弾かれる)
- `ROLLING_FRICTION + BUMP_GRAZE_FRICTION_PER_SPEED × BUMP_GRAZE_SPEED > LOW_FRICTION_THRESHOLD`(凸の側面接触は軽い着地にならない)
- `HOLLOW_FRICTION × HOLLOW_EXIT_SPEED < TILT_MAX × TILT_ACCEL_PER_LEVEL`(傾き最大なら凹から抜けられる)
- `HOLLOW_FRICTION × HOLLOW_EXIT_SPEED > 4 × TILT_STEP × TILT_ACCEL_PER_LEVEL`(4回押しまでの傾きだけでは抜けられない)
- `ROLLING_FRICTION + FRICTION_PER_G × CRUISE_VIBRATION_G <= LOW_FRICTION_THRESHOLD`(巡航の揺れだけでは凸で弾かれない)
- `ROLLING_FRICTION + FRICTION_PER_G × CRUISE_VIBRATION_G + HOLLOW_GRAZE_FRICTION <= HIGH_FRICTION_THRESHOLD`(巡航の揺れだけでは凹の側面で吹っ飛ばない)

## 関数・型

```rust
/// 凹凸のマスに入った時の当たり方(凸・凹共通)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeContact {
    /// 正面から入った。凸なら飛び上がり、凹ならハマる
    HeadOn,
    /// 斜めに入った。側面をこする
    Graze,
}

/// 凹凸のマスに入った向きの判定。prevからcurへ跨いだ縁の法線(curへ向かう向き)と速度のなす角で
/// 正面(HeadOn)か斜め(Graze)かを決める。角を斜めに跨いだ時は対角の向きを法線とみなす。
/// 縁を跨いでいない・止まっている時は正面扱い
pub fn edge_contact(vel: (f64, f64), prev: (usize, usize), cur: (usize, usize)) -> EdgeContact;

/// 入る向きと縁の法線のなす角のcosから当たり方を決める。GRAZE_COSちょうどは正面
pub fn edge_contact_from_cos(cos: f64) -> EdgeContact;

/// 凸に入った時に側面接触になるか。斜め、かつ速さがBUMP_GRAZE_SPEED以上(ちょうどを含む)
pub fn is_bump_graze(contact: EdgeContact, speed: f64) -> bool;

/// 凸の側面をこすった時に着地の摩擦へ上乗せする分。速さに比例する
pub fn bump_graze_friction(speed: f64) -> f64;
```

`HollowContact`・`hollow_contact`・`hollow_contact_from_cos`は上記に置き換えて削除する(`Sink`は`HeadOn`になる)。

`Top`の変更:

```rust
impl Top {
    /// 速さ(マス/秒)
    fn speed(&self) -> f64;

    /// 凸のマスに入った瞬間。側面接触なら飛び上がらずその場で判定し、それ以外は飛び上がる
    fn enter_bump(&mut self, prev: (usize, usize), cur: (usize, usize), g: f64) -> StepEvent;

    /// 凹のマスに入った瞬間。正面ならハマり、斜めなら側面をこする
    fn enter_hollow(&mut self, prev: (usize, usize), cur: (usize, usize), g: f64) -> StepEvent;

    /// 側面をこすった(凸・凹共通)。frictionから弾かれ/吹っ飛びを決め、弾かれるならbounce()して
    /// 弾かれた先のマスに入った扱いにする。Grazed(landing)を返す
    fn graze(&mut self, friction: f64) -> StepEvent;
}
```

`roll()`の`Cell::Bump`の分岐は`Some(self.enter_bump(prev, cur, g.magnitude()))`にする。

`enter_bump`の流れ:

1. `speed = self.speed()`、`contact = edge_contact(self.vel, prev, cur)`
2. `is_bump_graze(contact, speed)`なら`self.graze(landing_friction(g) + bump_graze_friction(speed))`を返す
3. それ以外は従来どおり`state = Airborne { remaining: HOP_DURATION, contact_g: g }`にして`Hopped { contact_g: g }`を返す

`enter_hollow`の流れ:

1. `edge_contact(self.vel, prev, cur)`が`HeadOn`なら`state = Sunk`にして`Sank`を返す
2. `Graze`なら`self.graze(landing_friction(g) + HOLLOW_GRAZE_FRICTION)`を返す

`graze`の流れ(現在の`enter_hollow`の`Graze`側の処理をそのまま切り出す):

1. `landing = classify_landing(friction)`
2. `landing == Bounce`なら`self.bounce()`して`self.prev_cell = cell_index(self.pos)`
3. `Grazed(landing)`を返す

`fly()`・`struggle()`・`drive()`・`bounce()`・`surface_friction()`・`landing_friction()`・`classify_landing()`は変えない。

コメントの更新:

- モジュール冒頭のコメント: 凸の説明に「斜めから速く当たると飛び上がらず側面をこすり、速さとGで弾かれ/吹っ飛びになる」を足す
- `StepEvent::Grazed`のコメント: 「凹の側面」を「凹凸の側面」にする
- `beigoma.rs`の`on_step_event`内のコメント「凹の側面をこすって弾かれた時も」を「凹凸の側面を」にする(コードは変えない)

## 対象ファイル

- `src/game/beigoma/board.rs`: 定数の変更・追加、`EdgeContact`/`edge_contact`/`edge_contact_from_cos`/`is_bump_graze`/`bump_graze_friction`、`Top::speed`/`enter_bump`/`graze`、`enter_hollow`の書き換え、`roll()`の`Cell::Bump`分岐、コメント、既存テストの更新と追加
- `src/game/beigoma.rs`: コメント1か所のみ

## テスト観点

先にテストを書き、実装で通す。G・速さのテスト値は閾値の境目(0.2G・0.533G・速さ6.4)を避けて取る(閾値が固定値になったので、`0.8 + 3.0 × 0.2`のような計算値と`1.4`の等価性を当てにしない)。

### 定数と閾値

- `FRICTION_PER_G == 3.0`、`HOLLOW_FRICTION == 4.5`、`HOLLOW_GRAZE_FRICTION == 1.0`、`HOLLOW_EXIT_SPEED == 1.0`、`LOW_FRICTION_THRESHOLD == 1.4`、`HIGH_FRICTION_THRESHOLD == 2.4`
- `LOW_G_THRESHOLD ≈ 0.2`、`HIGH_G_THRESHOLD ≈ 0.5333`(誤差1e-9)。`landing_friction(LOW_G_THRESHOLD) ≈ LOW_FRICTION_THRESHOLD`、`landing_friction(HIGH_G_THRESHOLD) ≈ HIGH_FRICTION_THRESHOLD`(既存の`landing_friction_grows_with_the_contact_g`をそのまま通す)
- 「定数」節の関係7つをconst assertで固定する
- `classify_landing`: `landing_friction(0.1)`は軽い着地、`landing_friction(0.4)`は弾かれ、`landing_friction(1.0)`は吹っ飛び。閾値ちょうどの扱い(`LOW`ちょうどは軽い着地、`HIGH`ちょうどは弾かれ)は既存のまま
- 巡航の揺れ: `classify_landing(landing_friction(CRUISE_VIBRATION_G)) == Light`、`classify_landing(landing_friction(CRUISE_VIBRATION_G) + HOLLOW_GRAZE_FRICTION) == Bounce`

### 既存テストの更新(閾値の変更に伴うもの)

- `classify_landing_has_three_levels`: 軽い着地の例`landing_friction(0.2)`を`landing_friction(0.1)`にする
- `low_g_contact_gives_a_light_landing`: `lateral(-0.2)`を`lateral(-0.1)`にする
- `grazing_under_high_g_flies_the_top_off`: `graze(0.6)`は吹っ飛びのまま。弾かれの例`graze(0.4)`を`graze(0.1)`にし、メッセージを「G=0.2以下なら弾かれるだけ」にする
- `hollow_contact_threshold_is_exact`: `edge_contact_threshold_is_exact`に改名し、`edge_contact_from_cos`/`edge_contact`/`EdgeContact::HeadOn`/`GRAZE_COS`で同じ内容を確かめる
- `sunk_top_keeps_building_speed_while_clamped`: 最後の`top.vel.0 > 0.8`を`top.vel.0 > 0.6`にする(3回押し=加速度3.15、摩擦4.5の終端速度は約0.67)
- `middle_g_contact_bounces_the_top_away`(-0.5G)・`high_g_contact_flies_the_top_off`(-1.0G)・`landing_uses_the_g_at_the_moment_of_contact`(-1.0G/-0.1G)・`staying_on_the_same_bump_does_not_hop_again`(-0.1G)・`bounce_at`(contact_g 0.5)・`entering_a_hollow_at_a_shallow_angle_grazes_and_bounces`(G=0)・`sunk_top_cannot_leave_below_the_exit_speed`・`sunk_top_escapes_with_full_tilt`・`landing_on_a_hollow_sinks_after_landing`は変更なしで通ること

### 凹の摩擦

- ハマった状態で傾き1〜4回押しのどの向きでも60秒間凹の中に留まり、速さが`HOLLOW_EXIT_SPEED`未満のまま(既存)
- 傾き最大でどの向きでも1秒以内に`Escaped`、かつ10ステップより後(既存)
- ハマった状態で傾き最大にした時の速さが、60ステップ後も1.4未満(摩擦4.5の終端速度約1.3を超えない。従来の摩擦3.0では約1.9まで届く)

### 凸の側面接触(全部平坦な盤に凸を(10, 6)へ置いた`board_with`で確かめる)

- 正面: 左隣から`(3.0, 0.0)`でも`(10.0, 0.0)`でも`Hopped`になり、飛び上がる(速さによらず正面は従来どおり)
- 斜め・速い: 上の縁の外側`(10.5, 5.995)`から`(3.0, 1.0)`(速さ約3.16、cos約0.32)で入ると`Grazed(Landing::Bounce)`。飛び上がらず(`is_airborne()`が偽、`state == Rolling`)、速さが`BOUNCE_SPEED`、速度が来た方向の真逆、位置は凸のマスの外。その後1000ステップ場外に出ず、出来事も起きない
- 斜め・遅い: 同じ位置から`(1.5, 0.5)`(速さ約1.58)で入ると`Hopped`になり、飛び上がる(1ステップでは縁に届かないので、最初の出来事が出るまで数ステップ進める)
- 斜め・とても速い: 同じ位置から`(7.0, 2.0)`(速さ約7.28、摩擦約2.62)で入るとG=0でも`Grazed(Landing::Flown)`
- 斜め・Gあり: `(3.0, 1.0)`で`lateral(0.4)`(摩擦約2.79)なら`Grazed(Landing::Flown)`、`lateral(0.1)`(摩擦約1.89)なら`Grazed(Landing::Bounce)`
- 弾かれた後: `Grazed(Bounce)`の直後に同じ凸へ再び入り直さない(次のステップの出来事が`None`)
- 4辺: 上下左右それぞれの縁の外側から、縁に沿う成分の大きい速度(速さ3.0以上)で入ると全部`Grazed(Bounce)`になる
- `is_bump_graze`: `(Graze, 3.0)`は真、`(Graze, 3.0 - 1e-6)`は偽、`(HeadOn, 100.0)`は偽、`(Graze, MAX_SPEED)`は真
- `bump_graze_friction`: `bump_graze_friction(0.0) == 0.0`、`bump_graze_friction(3.0) ≈ 0.75`、速さについて単調増加。`classify_landing(landing_friction(0.0) + bump_graze_friction(BUMP_GRAZE_SPEED)) == Bounce`、`... + bump_graze_friction(6.0)) == Bounce`、`... + bump_graze_friction(7.0)) == Flown`
- `edge_contact`が凸・凹で同じ結果になること: 同じ`vel`・`prev`・`cur`に対して、凸に置いても凹に置いても正面/斜めの分かれ方が一致する(凸は`Hopped`/`Grazed`、凹は`Sank`/`Grazed`で見分ける)。速さは`BUMP_GRAZE_SPEED`以上で揃える
- 既存の`moving_within_a_cell_triggers_nothing`(凸の中で動いても何も起きない)、`airborne_lasts_for_the_hop_duration`、`landing_on_a_hollow_sinks_after_landing`(左隣から`(5.0, 0.0)`で正面)は変更なしで通ること

### ゲーム側(beigoma.rs)

- `Grazed(Bounce)`で弾かれの演出とSE、`Grazed(Flown)`で`Outcome::Flown`になる既存テストは変更なしで通ること(凸・凹を区別しない)
