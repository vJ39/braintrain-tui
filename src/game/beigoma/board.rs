//! べーの盤面: 固定配置の盤(障害物・ゴール)、連打式の2軸の傾き、ベーゴマの転がり。
//!
//! 座標は盤のマス単位(左上が(0,0)、xは右、yは下)。画面の上が軽トラの前方。
//! - 傾き: pitch(前後軸、+が前傾)・roll(左右軸、+が右傾)。-TILT_MAX〜+TILT_MAX
//! - 軽トラのG(軽トラの加速度の向き)は、ベーゴマには慣性として逆向きにかかる
//!   (ブレーキ=後方向のGで、ベーゴマは前(画面の上)へ押される)
//! - 平坦な場所の摩擦はGによらず一定。障害物は凸(でっぱり)と凹(くぼみ)の2種類で、
//!   どちらも「マスに入った瞬間」に判定する
//!   - 凸: 踏むと一度飛び上がり、着地の瞬間の摩擦が踏んだ時のGに応じて増える。
//!     その大きさで 軽い着地/弾かれる/吹っ飛ぶ の3段階になる
//!   - 凹: 正面から入るとハマり、強い摩擦で速さを奪われてマスの中に留まる(十分な速さで抜け出せる)。
//!     斜めに入ると側面をこすり、凸の着地と同じ3段階(ただし摩擦が上乗せされる)で弾かれる
//! - 盤の縁に壁は無い。ベーゴマの中心が縁を越えたら盤から落ちる(場外)

use std::time::Duration;

use super::truck::GForce;

/// 盤の大きさ(マス)
pub const BOARD_WIDTH: usize = 20;
pub const BOARD_HEIGHT: usize = 12;

/// 盤の固定配置。'.'=平坦、'#'=凸(でっぱり)、'u'=凹(くぼみ)、'S'=投入位置、'G'=ゴール(唯一)。
/// 毎回同じ配置なので、繰り返しプレイしてコースを覚えられる
const LAYOUT: [&str; BOARD_HEIGHT] = [
    "....................",
    "...........#.....G..",
    "....#...........u...",
    "..........u.........",
    ".......#.......u....",
    "..u.........#.......",
    ".........u.......#..",
    "....u..........#....",
    "...........u........",
    ".......#.......u....",
    ".S..#...............",
    "....................",
];

/// 傾きの最大値(前後・左右それぞれ)
pub const TILT_MAX: f64 = 4.0;
/// キーを1回押すごとに傾きが動く量
pub const TILT_STEP: f64 = 0.7;
/// 最後に押してからこの時間が経つと、傾きが0へ戻り始める(押していない間の自然減衰)
pub const TILT_DECAY_DELAY: Duration = Duration::from_millis(500);
/// 自然減衰の速さ(1秒あたりに0へ近づく量)
pub const TILT_DECAY_PER_SEC: f64 = 1.0;

/// 傾き1あたりのベーゴマの加速度(マス/秒^2)
pub const TILT_ACCEL_PER_LEVEL: f64 = 1.5;
/// G1あたりのベーゴマの加速度(マス/秒^2)
pub const G_ACCEL_PER_G: f64 = 8.0;
/// 平坦な場所の転がり摩擦(1秒あたりの速度の減衰率)。Gによらず一定
pub const ROLLING_FRICTION: f64 = 0.8;
/// 着地の瞬間の摩擦が、踏んだ時のG1あたりに増える量
pub const FRICTION_PER_G: f64 = 2.0;
/// 摩擦増加の閾値の目安にするG(低=弾かれ始める、高=吹っ飛ぶ)
pub const LOW_G_THRESHOLD: f64 = 0.3;
pub const HIGH_G_THRESHOLD: f64 = 0.8;
/// 着地の摩擦の閾値。これ以下なら軽い着地、これを超えると弾かれる
pub const LOW_FRICTION_THRESHOLD: f64 = ROLLING_FRICTION + FRICTION_PER_G * LOW_G_THRESHOLD;
/// 着地の摩擦の閾値。これを超えると吹っ飛ぶ
pub const HIGH_FRICTION_THRESHOLD: f64 = ROLLING_FRICTION + FRICTION_PER_G * HIGH_G_THRESHOLD;
/// 障害物を踏んで飛び上がってから着地するまでの時間
pub const HOP_DURATION: Duration = Duration::from_millis(250);
/// 弾かれた時の速さ(マス/秒)と、弾かれた瞬間に位置が飛ぶ量(マス)。
/// 盤の縁には壁が無いので、盤の中央で弾かれても縁まで届かない強さにする
/// (平坦な場所での減速距離 BOUNCE_SPEED/ROLLING_FRICTION + BOUNCE_KICK ≒ 5.5マス、中央から縦の縁まで6マス)
pub const BOUNCE_SPEED: f64 = 4.0;
pub const BOUNCE_KICK: f64 = 0.5;
/// ベーゴマの速さの上限(マス/秒)。1ステップでマスを飛び越さないようにする
pub const MAX_SPEED: f64 = 15.0;
/// 凹にハマっている間の摩擦(1秒あたりの速度の減衰率)。平坦の0.8に対して大きく、抜け出しにくい
pub const HOLLOW_FRICTION: f64 = 3.0;
/// 凹から抜け出すのに必要な速さ(マス/秒)。終端速度は 加速度/HOLLOW_FRICTION なので、
/// 傾き最大(6マス/s^2)なら約2.0に届いて約0.5秒で抜け、それより弱い傾きだけでは届かない
pub const HOLLOW_EXIT_SPEED: f64 = 1.5;
/// 凹に入る向きと、跨いだ縁の法線のなす角のcos。これ未満(60°より浅い角度)なら側面をこすって弾かれる
pub const HOLLOW_GRAZE_COS: f64 = 0.5;
/// 凹の側面をこすった時に、着地の摩擦へ上乗せする分。G=0でも弾かれ、G>0.45で吹っ飛ぶ
pub const HOLLOW_GRAZE_FRICTION: f64 = 0.7;

/// 盤の1マス
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Flat,
    /// 凸(でっぱり)。踏むと飛び上がり、着地の摩擦が踏んだ時のGで増える
    Bump,
    /// 凹(くぼみ)。正面から入るとハマって抜け出しにくい。斜めに入ると側面をこすって弾かれる
    Hollow,
    Goal,
}

/// 固定配置の盤
#[derive(Debug, Clone)]
pub struct Board {
    cells: Vec<Cell>,
    start: (usize, usize),
}

impl Board {
    /// 固定配置(LAYOUT)の盤
    pub fn standard() -> Self {
        let mut cells = Vec::with_capacity(BOARD_WIDTH * BOARD_HEIGHT);
        let mut start = (0, 0);
        for (y, row) in LAYOUT.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                cells.push(match c {
                    '#' => Cell::Bump,
                    'u' => Cell::Hollow,
                    'G' => Cell::Goal,
                    'S' => {
                        start = (x, y);
                        Cell::Flat
                    }
                    _ => Cell::Flat,
                });
            }
        }
        Self { cells, start }
    }

    /// 位置が盤の上にあるか。ベーゴマの中心が縁を越えたら場外(落ちる)
    pub fn contains(pos: (f64, f64)) -> bool {
        (0.0..BOARD_WIDTH as f64).contains(&pos.0) && (0.0..BOARD_HEIGHT as f64).contains(&pos.1)
    }

    /// マス(x, y)。盤の外は平坦として扱う(場外に出た時点でゲームは終わる)
    pub fn cell(&self, x: usize, y: usize) -> Cell {
        if x >= BOARD_WIDTH || y >= BOARD_HEIGHT {
            return Cell::Flat;
        }
        self.cells[y * BOARD_WIDTH + x]
    }

    /// 位置(マス単位の座標)を含むマス
    pub fn cell_at(&self, pos: (f64, f64)) -> Cell {
        if pos.0 < 0.0 || pos.1 < 0.0 {
            return Cell::Flat;
        }
        self.cell(pos.0 as usize, pos.1 as usize)
    }

    /// ベーゴマの投入位置(マスの中央)
    pub fn start_position(&self) -> (f64, f64) {
        (self.start.0 as f64 + 0.5, self.start.1 as f64 + 0.5)
    }

    /// ゴールのマス(テストでゴールの手前に置くため)
    #[cfg(test)]
    pub fn goal(&self) -> (usize, usize) {
        let index = self
            .cells
            .iter()
            .position(|&cell| cell == Cell::Goal)
            .expect("ゴールがある");
        (index % BOARD_WIDTH, index / BOARD_WIDTH)
    }
}

/// 傾きを動かすキーの向き
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiltKey {
    /// ↑: 前傾(pitch +)
    Forward,
    /// ↓: 後傾(pitch -)
    Back,
    /// ←: 左傾(roll -)
    Left,
    /// →: 右傾(roll +)
    Right,
}

/// 盤の傾き(連打式・2軸)。キーを押すたびにTILT_STEPずつ動き、押していない間は0へ戻る
#[derive(Debug, Clone, Default)]
pub struct Tilt {
    pitch: f64,
    roll: f64,
    /// 各軸を最後に押してからの経過時間
    pitch_idle: Duration,
    roll_idle: Duration,
}

impl Tilt {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pitch(&self) -> f64 {
        self.pitch
    }

    pub fn roll(&self) -> f64 {
        self.roll
    }

    /// キーを1回押した。その軸の傾きをTILT_STEPだけ動かす(±TILT_MAXでクランプ)
    pub fn press(&mut self, key: TiltKey) {
        let (value, idle, delta) = match key {
            TiltKey::Forward => (&mut self.pitch, &mut self.pitch_idle, TILT_STEP),
            TiltKey::Back => (&mut self.pitch, &mut self.pitch_idle, -TILT_STEP),
            TiltKey::Right => (&mut self.roll, &mut self.roll_idle, TILT_STEP),
            TiltKey::Left => (&mut self.roll, &mut self.roll_idle, -TILT_STEP),
        };
        *value = (*value + delta).clamp(-TILT_MAX, TILT_MAX);
        *idle = Duration::ZERO;
    }

    /// 経過時間を進める。最後に押してからTILT_DECAY_DELAYを過ぎた軸は0へ近づく
    pub fn update(&mut self, dt: Duration) {
        decay_axis(&mut self.pitch, &mut self.pitch_idle, dt);
        decay_axis(&mut self.roll, &mut self.roll_idle, dt);
    }
}

/// 1軸の自然減衰。dtのうち「最後に押してからTILT_DECAY_DELAYを過ぎた後」の時間だけ0へ近づける
fn decay_axis(value: &mut f64, idle: &mut Duration, dt: Duration) {
    let before = *idle;
    *idle = idle.saturating_add(dt);
    if *idle <= TILT_DECAY_DELAY {
        return;
    }
    let decaying = (*idle - before.max(TILT_DECAY_DELAY)).as_secs_f64();
    let amount = TILT_DECAY_PER_SEC * decaying;
    *value = if *value > 0.0 {
        (*value - amount).max(0.0)
    } else {
        (*value + amount).min(0.0)
    };
}

/// 足元のマスとベーゴマの状態から決まる摩擦。凹にハマっている間は常にHOLLOW_FRICTION、
/// それ以外の平坦な場所(ゴール・ハマらずに乗った凹も含む)はGの大小によらず一定
pub fn surface_friction(cell: Cell, state: TopState, g: f64) -> f64 {
    if state == TopState::Sunk {
        return HOLLOW_FRICTION;
    }
    match cell {
        Cell::Flat | Cell::Hollow | Cell::Goal => ROLLING_FRICTION,
        // 凸の上ではGがかかるほど引っかかる
        Cell::Bump => landing_friction(g),
    }
}

/// 凹への入り方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HollowContact {
    /// 正面から入った: ハマる
    Sink,
    /// 斜めに入った: 側面をこする
    Graze,
}

/// 凹に入った向きの判定。prevからcurへ跨いだ縁の法線(curへ向かう向き)と速度のなす角で、
/// 正面(Sink)か斜め(Graze)かを決める。角を斜めに跨いだ時は対角の向きを法線とみなす
pub fn hollow_contact(vel: (f64, f64), prev: (usize, usize), cur: (usize, usize)) -> HollowContact {
    let normal = (
        (cur.0 as i64 - prev.0 as i64).signum() as f64,
        (cur.1 as i64 - prev.1 as i64).signum() as f64,
    );
    let normal_len = normal.0.hypot(normal.1);
    let speed = vel.0.hypot(vel.1);
    if normal_len == 0.0 || speed < 1e-9 {
        // 縁を跨いでいない・止まっている(通常は起きない)時は正面扱いにする
        return HollowContact::Sink;
    }
    let cos = (vel.0 * normal.0 + vel.1 * normal.1) / (speed * normal_len);
    hollow_contact_from_cos(cos)
}

/// 入る向きと縁の法線のなす角のcosから入り方を決める。HOLLOW_GRAZE_COSちょうどはハマる
pub fn hollow_contact_from_cos(cos: f64) -> HollowContact {
    if cos >= HOLLOW_GRAZE_COS {
        HollowContact::Sink
    } else {
        HollowContact::Graze
    }
}

/// 障害物から着地する瞬間の摩擦。踏んだ時のGが大きいほど増える
pub fn landing_friction(contact_g: f64) -> f64 {
    ROLLING_FRICTION + FRICTION_PER_G * contact_g
}

/// 着地の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// 摩擦が低い閾値以下: 通常の転がりに近い、軽い着地
    Light,
    /// 低い閾値〜高い閾値: ボード上を勢いよく弾かれる
    Bounce,
    /// 高い閾値を超える: 吹っ飛ぶ(GAME OVER)
    Flown,
}

/// 着地の摩擦から結果を決める
pub fn classify_landing(friction: f64) -> Landing {
    if friction > HIGH_FRICTION_THRESHOLD {
        Landing::Flown
    } else if friction > LOW_FRICTION_THRESHOLD {
        Landing::Bounce
    } else {
        Landing::Light
    }
}

/// ベーゴマの状態
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TopState {
    /// 盤の上を転がっている
    Rolling,
    /// 凸を踏んで飛び上がっている。contact_gは踏んだ瞬間のG
    Airborne { remaining: Duration, contact_g: f64 },
    /// 凹にハマっている。凹のマスの縁で位置を留め、速さがHOLLOW_EXIT_SPEEDに届いたら抜ける
    Sunk,
}

/// 1ステップの間に起きた出来事
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepEvent {
    /// 凸を踏んで飛び上がった
    Hopped { contact_g: f64 },
    /// 着地した
    Landed(Landing),
    /// 凹に正面から入ってハマった
    Sank,
    /// 凹の側面をこすって弾かれた/吹っ飛んだ
    Grazed(Landing),
    /// 凹から抜け出した
    Escaped,
    /// 盤から落ちた(場外)
    FellOff,
    /// ゴールに入った
    Goal,
}

/// 位置を含むマス(盤の範囲に収める)
fn cell_index(pos: (f64, f64)) -> (usize, usize) {
    (
        (pos.0.max(0.0) as usize).min(BOARD_WIDTH - 1),
        (pos.1.max(0.0) as usize).min(BOARD_HEIGHT - 1),
    )
}

/// 盤の上のベーゴマ
#[derive(Debug, Clone)]
pub struct Top {
    pub pos: (f64, f64),
    pub vel: (f64, f64),
    pub state: TopState,
    /// 直前のステップにいたマス。マスに入った瞬間(凸・凹の判定)の検出に使う。
    /// ハマっている間は、ハマっている凹のマス
    prev_cell: (usize, usize),
}

impl Top {
    /// posに止まった状態で置く(置いたマスには既に入っている扱い)
    pub fn new(pos: (f64, f64)) -> Self {
        Self {
            pos,
            vel: (0.0, 0.0),
            state: TopState::Rolling,
            prev_cell: cell_index(pos),
        }
    }

    pub fn is_airborne(&self) -> bool {
        matches!(self.state, TopState::Airborne { .. })
    }

    /// dtだけ物理を進める。傾きとGを加速度として加え、摩擦で減速する。
    /// 盤の縁に壁は無く、中心が縁を越えたら他の判定より先にFellOffを返す
    pub fn step(
        &mut self,
        board: &Board,
        dt: Duration,
        tilt: &Tilt,
        g: GForce,
    ) -> Option<StepEvent> {
        let secs = dt.as_secs_f64();
        match self.state {
            TopState::Airborne {
                remaining,
                contact_g,
            } => self.fly(board, dt, remaining, contact_g),
            TopState::Sunk => self.struggle(secs, tilt, g),
            TopState::Rolling => self.roll(board, secs, tilt, g),
        }
    }

    /// 転がっている間: 進んだ先のマスに入った瞬間だけ、凸・凹の判定をする
    fn roll(&mut self, board: &Board, secs: f64, tilt: &Tilt, g: GForce) -> Option<StepEvent> {
        let friction = surface_friction(board.cell_at(self.pos), self.state, g.magnitude());
        self.drive(secs, tilt, g, friction);
        if !Board::contains(self.pos) {
            return Some(StepEvent::FellOff);
        }
        let prev = self.prev_cell;
        let cur = cell_index(self.pos);
        self.prev_cell = cur;
        let cell = board.cell(cur.0, cur.1);
        if cell == Cell::Goal {
            // ゴールは入った瞬間に限らず、ゴールのマスにいれば入ったとみなす(着地・脱出した先も含む)
            return Some(StepEvent::Goal);
        }
        if cur == prev {
            return None;
        }
        match cell {
            Cell::Bump => {
                // 凸を踏んだ瞬間: 一度飛び上がり、このときのGで着地の摩擦が決まる
                let contact_g = g.magnitude();
                self.state = TopState::Airborne {
                    remaining: HOP_DURATION,
                    contact_g,
                };
                Some(StepEvent::Hopped { contact_g })
            }
            Cell::Hollow => Some(self.enter_hollow(prev, cur, g.magnitude())),
            Cell::Flat | Cell::Goal => None,
        }
    }

    /// 凹のマスに入った瞬間。正面ならハマり、斜めなら側面をこすって凸の着地と同じ3段階で判定する
    fn enter_hollow(&mut self, prev: (usize, usize), cur: (usize, usize), g: f64) -> StepEvent {
        match hollow_contact(self.vel, prev, cur) {
            HollowContact::Sink => {
                self.state = TopState::Sunk;
                StepEvent::Sank
            }
            HollowContact::Graze => {
                let landing = classify_landing(landing_friction(g) + HOLLOW_GRAZE_FRICTION);
                if landing == Landing::Bounce {
                    self.bounce();
                    self.prev_cell = cell_index(self.pos);
                }
                StepEvent::Grazed(landing)
            }
        }
    }

    /// 凹にハマっている間: 摩擦はHOLLOW_FRICTIONで、凹のマスから出る位置まで進んでも
    /// 速さがHOLLOW_EXIT_SPEEDに届かなければ位置だけマスの内側に留める。
    /// 速度は殺さないので、縁へ向かって傾け続けている間は速さが溜まっていく
    fn struggle(&mut self, secs: f64, tilt: &Tilt, g: GForce) -> Option<StepEvent> {
        let friction = surface_friction(Cell::Hollow, self.state, g.magnitude());
        self.drive(secs, tilt, g, friction);
        let hollow = self.prev_cell;
        if Board::contains(self.pos) && cell_index(self.pos) == hollow {
            return None;
        }
        if self.vel.0.hypot(self.vel.1) < HOLLOW_EXIT_SPEED {
            self.pos = clamp_into_cell(self.pos, hollow);
            return None;
        }
        if !Board::contains(self.pos) {
            return Some(StepEvent::FellOff);
        }
        // prev_cellは凹のままにしておき、次のステップで抜けた先のマスに入った判定をする
        self.state = TopState::Rolling;
        Some(StepEvent::Escaped)
    }

    /// 傾きとGで加速し、摩擦で減速して、速度のぶん進める
    fn drive(&mut self, secs: f64, tilt: &Tilt, g: GForce, friction: f64) {
        // 傾きの向きへ転がり、Gは慣性として軽トラの加速度と逆向きにかかる
        // (画面の上が前方なので、前傾(pitch +)・後方向のG(longitudinal -)はyを減らす向き)
        let ax = tilt.roll() * TILT_ACCEL_PER_LEVEL - g.lateral * G_ACCEL_PER_G;
        let ay = -tilt.pitch() * TILT_ACCEL_PER_LEVEL + g.longitudinal * G_ACCEL_PER_G;
        self.vel.0 += ax * secs;
        self.vel.1 += ay * secs;
        let damping = (1.0 - friction * secs).max(0.0);
        self.vel.0 *= damping;
        self.vel.1 *= damping;
        self.cap_speed();
        self.advance(secs);
    }

    /// 飛び上がっている間: 盤に触れていないので傾き・G・摩擦は効かず、そのままの速度で進む。
    /// 着地したら、踏んだ時のGから決まる摩擦で結果を判定する。凹の上に着地したら
    /// (吹っ飛んだ時以外は)そのままハマる。出来事は着地(Landed)を返す。
    /// 飛び上がっている間に中心が縁を越えたら、着地を待たずに落ちる
    fn fly(
        &mut self,
        board: &Board,
        dt: Duration,
        remaining: Duration,
        contact_g: f64,
    ) -> Option<StepEvent> {
        self.advance(dt.as_secs_f64());
        if !Board::contains(self.pos) {
            return Some(StepEvent::FellOff);
        }
        let remaining = remaining.saturating_sub(dt);
        if !remaining.is_zero() {
            self.state = TopState::Airborne {
                remaining,
                contact_g,
            };
            return None;
        }
        self.state = TopState::Rolling;
        let landing = classify_landing(landing_friction(contact_g));
        if landing == Landing::Bounce {
            self.bounce();
        }
        // 着地したマスには既に入っている扱い(同じ凸の上で飛び上がり直さない)
        self.prev_cell = cell_index(self.pos);
        if landing != Landing::Flown
            && Board::contains(self.pos)
            && board.cell(self.prev_cell.0, self.prev_cell.1) == Cell::Hollow
        {
            self.state = TopState::Sunk;
        }
        Some(StepEvent::Landed(landing))
    }

    /// 弾かれる: 来た方向へ勢いよく弾き返し、位置も一気に戻す(盤の外へ出てもよい。
    /// その場合は次のステップでFellOffになる)
    fn bounce(&mut self) {
        let speed = self.vel.0.hypot(self.vel.1);
        let dir = if speed > 1e-9 {
            (-self.vel.0 / speed, -self.vel.1 / speed)
        } else {
            // 止まったまま踏むことは無いが、念のため手前(画面の下)へ弾く
            (0.0, 1.0)
        };
        self.vel = (dir.0 * BOUNCE_SPEED, dir.1 * BOUNCE_SPEED);
        self.pos = (
            self.pos.0 + dir.0 * BOUNCE_KICK,
            self.pos.1 + dir.1 * BOUNCE_KICK,
        );
    }

    fn cap_speed(&mut self) {
        let speed = self.vel.0.hypot(self.vel.1);
        if speed > MAX_SPEED {
            let scale = MAX_SPEED / speed;
            self.vel = (self.vel.0 * scale, self.vel.1 * scale);
        }
    }

    /// 速度のぶん進める(盤の縁で跳ね返らない)
    fn advance(&mut self, secs: f64) {
        self.pos.0 += self.vel.0 * secs;
        self.pos.1 += self.vel.1 * secs;
    }
}

/// マスの内側に収めた位置(右端・下端はわずかに内側にして、次のマスに入らないようにする)
fn clamp_into_cell(pos: (f64, f64), cell: (usize, usize)) -> (f64, f64) {
    const INSET: f64 = 1e-9;
    let (x, y) = (cell.0 as f64, cell.1 as f64);
    (
        pos.0.clamp(x, x + 1.0 - INSET),
        pos.1.clamp(y, y + 1.0 - INSET),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: Duration = Duration::from_millis(10);
    const NO_G: GForce = GForce {
        longitudinal: 0.0,
        lateral: 0.0,
    };

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn speed(top: &Top) -> f64 {
        top.vel.0.hypot(top.vel.1)
    }

    /// 最初の障害物のマス(左上から行優先で探す)
    fn some_bump(board: &Board) -> (usize, usize) {
        (0..BOARD_HEIGHT)
            .flat_map(|y| (0..BOARD_WIDTH).map(move |x| (x, y)))
            .find(|&(x, y)| board.cell(x, y) == Cell::Bump)
            .expect("障害物がある")
    }

    /// 障害物の左隣の平坦なマスの中から、右へ向かって障害物に入る直前に置く
    fn top_just_left_of_bump(board: &Board) -> Top {
        let (bx, by) = (1..BOARD_HEIGHT)
            .flat_map(|y| (1..BOARD_WIDTH).map(move |x| (x, y)))
            .find(|&(x, y)| board.cell(x, y) == Cell::Bump && board.cell(x - 1, y) == Cell::Flat)
            .expect("左隣が平坦な障害物がある");
        let mut top = Top::new((bx as f64 - 0.02, by as f64 + 0.5));
        top.vel = (3.0, 0.0);
        top
    }

    /// 障害物を踏ませ、飛び上がりの間はafter_contactのGにして着地まで進める
    fn land_after_contact(contact: GForce, after_contact: GForce) -> (Top, Landing) {
        let board = Board::standard();
        let mut top = top_just_left_of_bump(&board);
        let tilt = Tilt::new();
        let event = top.step(&board, STEP, &tilt, contact);
        assert!(
            matches!(event, Some(StepEvent::Hopped { contact_g }) if approx(contact_g, contact.magnitude())),
            "障害物に入った瞬間に飛び上がる: {event:?}"
        );
        assert!(top.is_airborne());
        for _ in 0..100 {
            if let Some(StepEvent::Landed(landing)) = top.step(&board, STEP, &tilt, after_contact) {
                return (top, landing);
            }
        }
        panic!("着地しなかった");
    }

    fn lateral(g: f64) -> GForce {
        GForce {
            longitudinal: 0.0,
            lateral: g,
        }
    }

    // --- 盤の配置 ---

    #[test]
    fn layout_is_fixed_with_one_goal_and_one_start() {
        for row in LAYOUT {
            assert_eq!(row.chars().count(), BOARD_WIDTH);
        }
        let all: String = LAYOUT.concat();
        assert_eq!(all.matches('G').count(), 1, "ゴールは唯一");
        assert_eq!(all.matches('S').count(), 1);
        assert!(all.matches('#').count() >= 8, "障害物を複数置く");
        let board = Board::standard();
        let (gx, gy) = board.goal();
        assert_eq!(board.cell(gx, gy), Cell::Goal);
        assert_eq!(
            board.cell_at(board.start_position()),
            Cell::Flat,
            "投入位置は平坦"
        );
        assert_eq!(board.start_position(), (1.5, 10.5));
        // 毎回同じ配置(ランダム生成しない)
        let again = Board::standard();
        assert_eq!(again.goal(), board.goal());
        assert_eq!(again.cells, board.cells);
    }

    #[test]
    fn layout_chars_map_to_bump_and_hollow() {
        let board = Board::standard();
        let (mut bumps, mut hollows) = (0, 0);
        for (y, row) in LAYOUT.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                match c {
                    '#' => {
                        assert_eq!(board.cell(x, y), Cell::Bump, "({x},{y}): '#'は凸");
                        bumps += 1;
                    }
                    'u' => {
                        assert_eq!(board.cell(x, y), Cell::Hollow, "({x},{y}): 'u'は凹");
                        hollows += 1;
                    }
                    _ => assert!(
                        !matches!(board.cell(x, y), Cell::Bump | Cell::Hollow),
                        "({x},{y}): '#'/'u'以外は凹凸にしない"
                    ),
                }
            }
        }
        assert!(bumps >= 4, "凸を複数置く: {bumps}");
        assert!(hollows >= 4, "凹を複数置く: {hollows}");
    }

    #[test]
    fn cell_at_uses_the_cell_containing_the_position() {
        let board = Board::standard();
        let (bx, by) = some_bump(&board);
        assert_eq!(
            board.cell_at((bx as f64 + 0.1, by as f64 + 0.9)),
            Cell::Bump
        );
        assert_eq!(board.cell(99, 99), Cell::Flat, "盤の外は平坦扱い");
    }

    // --- 傾き(連打式・2軸) ---

    #[test]
    fn each_press_moves_the_matching_axis_by_one_step() {
        let mut tilt = Tilt::new();
        tilt.press(TiltKey::Forward);
        assert!(approx(tilt.pitch(), TILT_STEP));
        assert_eq!(tilt.roll(), 0.0, "上下キーは左右軸を動かさない");
        tilt.press(TiltKey::Forward);
        assert!(approx(tilt.pitch(), TILT_STEP * 2.0));
        tilt.press(TiltKey::Right);
        assert!(approx(tilt.roll(), TILT_STEP));
        tilt.press(TiltKey::Left);
        tilt.press(TiltKey::Left);
        assert!(approx(tilt.roll(), -TILT_STEP), "反対のキーで逆方向へ動く");
        tilt.press(TiltKey::Back);
        assert!(approx(tilt.pitch(), TILT_STEP));
    }

    #[test]
    fn tilt_is_clamped_to_plus_minus_max() {
        let mut tilt = Tilt::new();
        for _ in 0..20 {
            tilt.press(TiltKey::Forward);
            tilt.press(TiltKey::Left);
        }
        assert_eq!(tilt.pitch(), TILT_MAX);
        assert_eq!(tilt.roll(), -TILT_MAX);
        for _ in 0..40 {
            tilt.press(TiltKey::Back);
            tilt.press(TiltKey::Right);
        }
        assert_eq!(tilt.pitch(), -TILT_MAX);
        assert_eq!(tilt.roll(), TILT_MAX);
    }

    /// intervalごとに1回押し続けて、傾きが最大に届くまでの時間(秒)
    fn time_to_reach_max(interval: Duration) -> f64 {
        let mut tilt = Tilt::new();
        let mut t = 0.0;
        loop {
            tilt.press(TiltKey::Right);
            if tilt.roll() >= TILT_MAX {
                return t;
            }
            tilt.update(interval);
            t += interval.as_secs_f64();
            assert!(t < 60.0, "{interval:?}間隔では60秒で届かなかった");
        }
    }

    #[test]
    fn faster_tapping_reaches_the_max_sooner() {
        let fast = time_to_reach_max(Duration::from_millis(100));
        let medium = time_to_reach_max(Duration::from_millis(400));
        let slow = time_to_reach_max(Duration::from_millis(800));
        assert!(fast < medium, "{fast} < {medium}");
        assert!(medium < slow, "{medium} < {slow}");
    }

    #[test]
    fn tilt_decays_back_to_zero_while_no_key_is_pressed() {
        let mut tilt = Tilt::new();
        for _ in 0..4 {
            tilt.press(TiltKey::Forward);
            tilt.press(TiltKey::Left);
        }
        let (pitch, roll) = (tilt.pitch(), tilt.roll());
        tilt.update(TILT_DECAY_DELAY / 2);
        assert_eq!(tilt.pitch(), pitch, "押した直後はすぐには戻らない");
        let mut previous = tilt.pitch();
        for _ in 0..10 {
            tilt.update(Duration::from_millis(200));
            assert!(tilt.pitch() <= previous, "徐々に戻る");
            previous = tilt.pitch();
        }
        assert!(
            tilt.pitch() < pitch && tilt.pitch() > 0.0,
            "徐々に(一気にではなく)戻る"
        );
        assert!(
            tilt.roll() > roll && tilt.roll() < 0.0,
            "負の傾きも0へ近づく"
        );
        tilt.update(Duration::from_secs(10));
        assert_eq!(tilt.pitch(), 0.0, "最後は水平で止まる(行き過ぎない)");
        assert_eq!(tilt.roll(), 0.0);
    }

    #[test]
    fn decay_follows_the_rate_after_the_delay() {
        let mut tilt = Tilt::new();
        for _ in 0..5 {
            tilt.press(TiltKey::Forward);
        }
        let start = tilt.pitch();
        tilt.update(TILT_DECAY_DELAY + Duration::from_secs(1));
        assert!(approx(tilt.pitch(), start - TILT_DECAY_PER_SEC));
    }

    #[test]
    fn pressing_one_axis_does_not_hold_the_other_axis() {
        let mut tilt = Tilt::new();
        tilt.press(TiltKey::Right);
        tilt.press(TiltKey::Right);
        let roll = tilt.roll();
        // 前後軸だけ連打し続けても、左右軸は押していないので戻っていく
        for _ in 0..20 {
            tilt.press(TiltKey::Forward);
            tilt.update(Duration::from_millis(100));
        }
        assert!(tilt.roll() < roll);
    }

    // --- 摩擦・着地の判定 ---

    #[test]
    fn flat_friction_does_not_depend_on_g() {
        let rolling = TopState::Rolling;
        for g in [0.0, 0.3, 0.8, 1.5, 3.0] {
            assert_eq!(
                surface_friction(Cell::Flat, rolling, g),
                ROLLING_FRICTION,
                "g={g}"
            );
            assert_eq!(
                surface_friction(Cell::Goal, rolling, g),
                ROLLING_FRICTION,
                "g={g}"
            );
        }
    }

    #[test]
    fn sunk_top_always_uses_the_hollow_friction() {
        for cell in [Cell::Flat, Cell::Bump, Cell::Hollow, Cell::Goal] {
            for g in [0.0, 0.5, 3.0] {
                assert_eq!(
                    surface_friction(cell, TopState::Sunk, g),
                    HOLLOW_FRICTION,
                    "{cell:?} g={g}"
                );
            }
        }
        // 凹の中は平坦よりずっと転がりにくい
        const { assert!(HOLLOW_FRICTION > ROLLING_FRICTION * 2.0) };
    }

    #[test]
    fn landing_friction_grows_with_the_contact_g() {
        assert!(approx(landing_friction(0.0), ROLLING_FRICTION));
        assert!(landing_friction(0.5) > landing_friction(0.2));
        assert!(approx(
            landing_friction(LOW_G_THRESHOLD),
            LOW_FRICTION_THRESHOLD
        ));
        assert!(approx(
            landing_friction(HIGH_G_THRESHOLD),
            HIGH_FRICTION_THRESHOLD
        ));
    }

    #[test]
    fn classify_landing_has_three_levels() {
        assert_eq!(classify_landing(landing_friction(0.0)), Landing::Light);
        assert_eq!(classify_landing(landing_friction(0.2)), Landing::Light);
        assert_eq!(
            classify_landing(LOW_FRICTION_THRESHOLD),
            Landing::Light,
            "低い閾値以下は軽い着地"
        );
        assert_eq!(
            classify_landing(LOW_FRICTION_THRESHOLD + 1e-6),
            Landing::Bounce
        );
        assert_eq!(classify_landing(landing_friction(0.5)), Landing::Bounce);
        assert_eq!(
            classify_landing(HIGH_FRICTION_THRESHOLD),
            Landing::Bounce,
            "高い閾値ちょうどはまだ弾かれる"
        );
        assert_eq!(
            classify_landing(HIGH_FRICTION_THRESHOLD + 1e-6),
            Landing::Flown
        );
        assert_eq!(classify_landing(landing_friction(1.2)), Landing::Flown);
    }

    // --- ベーゴマの物理 ---

    #[test]
    fn top_stays_still_on_a_level_board_without_g() {
        let board = Board::standard();
        let mut top = Top::new(board.start_position());
        for _ in 0..100 {
            assert_eq!(top.step(&board, STEP, &Tilt::new(), NO_G), None);
        }
        assert_eq!(top.pos, board.start_position());
        assert_eq!(top.vel, (0.0, 0.0));
    }

    #[test]
    fn tilt_accelerates_the_top_in_the_tilted_direction() {
        let board = Board::standard();
        let center = (10.5, 0.5);
        let cases = [
            (TiltKey::Right, (1.0, 0.0)),
            (TiltKey::Left, (-1.0, 0.0)),
            (TiltKey::Forward, (0.0, -1.0)),
            (TiltKey::Back, (0.0, 1.0)),
        ];
        for (key, (dx, dy)) in cases {
            let mut tilt = Tilt::new();
            tilt.press(key);
            let mut top = Top::new(if dy < 0.0 { (10.5, 11.5) } else { center });
            let start = top.pos;
            top.step(&board, STEP, &tilt, NO_G);
            // 1ステップ目の速度は 傾き×係数×時間(から摩擦の分だけ減る)
            let expected = TILT_STEP * TILT_ACCEL_PER_LEVEL * STEP.as_secs_f64();
            assert!(top.vel.0 * dx >= 0.0 && top.vel.1 * dy >= 0.0, "{key:?}");
            assert!((speed(&top) - expected).abs() < expected * 0.05, "{key:?}");
            for _ in 0..20 {
                top.step(&board, STEP, &tilt, NO_G);
            }
            assert!((top.pos.0 - start.0) * dx >= 0.0 && (top.pos.1 - start.1) * dy >= 0.0);
            assert!(top.pos != start, "{key:?}: 傾けた方へ転がる");
        }
    }

    #[test]
    fn stronger_tilt_accelerates_more() {
        let board = Board::standard();
        let run = |presses: usize| {
            let mut tilt = Tilt::new();
            for _ in 0..presses {
                tilt.press(TiltKey::Right);
            }
            let mut top = Top::new((2.5, 0.5));
            for _ in 0..30 {
                top.step(&board, STEP, &tilt, NO_G);
            }
            top.vel.0
        };
        assert!(run(4) > run(1));
    }

    #[test]
    fn g_pushes_the_top_opposite_to_the_truck_acceleration() {
        let board = Board::standard();
        let tilt = Tilt::new();
        // ブレーキ(後方向のG)で、ベーゴマは前(画面の上)へ押される
        let mut top = Top::new((10.5, 11.5));
        top.step(
            &board,
            STEP,
            &tilt,
            GForce {
                longitudinal: -0.5,
                lateral: 0.0,
            },
        );
        assert!(top.vel.1 < 0.0);
        assert!(approx(
            -top.vel.1,
            0.5 * G_ACCEL_PER_G
                * STEP.as_secs_f64()
                * (1.0 - ROLLING_FRICTION * STEP.as_secs_f64())
        ));
        // 右へ避ける(右方向のG)と、ベーゴマは左へ押される
        let mut top = Top::new((10.5, 0.5));
        top.step(&board, STEP, &tilt, lateral(0.5));
        assert!(top.vel.0 < 0.0);
    }

    #[test]
    fn rolling_friction_slows_the_top_down() {
        let board = Board::standard();
        let mut top = Top::new((2.5, 0.5));
        top.vel = (4.0, 0.0);
        let mut previous = speed(&top);
        for _ in 0..50 {
            top.step(&board, STEP, &Tilt::new(), NO_G);
            assert!(speed(&top) < previous);
            previous = speed(&top);
        }
    }

    #[test]
    fn flat_ground_never_causes_hops_even_under_huge_g() {
        // 最上段は全部平坦。左右のGで左右に振り回しても、平坦なら特別な影響は無い
        let board = Board::standard();
        let mut top = Top::new((10.5, 0.5));
        for i in 0..300 {
            let g = if (i / 50) % 2 == 0 { 3.0 } else { -3.0 };
            let event = top.step(&board, STEP, &Tilt::new(), lateral(g));
            assert_eq!(event, None, "i={i}");
            assert!(!top.is_airborne());
            assert!((top.pos.1 - 0.5).abs() < 1e-9, "横のGでは縦に動かない");
        }
    }

    // --- 場外(盤の縁に壁は無い) ---

    #[test]
    fn board_contains_only_positions_on_the_board() {
        let (w, h) = (BOARD_WIDTH as f64, BOARD_HEIGHT as f64);
        assert!(Board::contains((0.0, 0.0)), "左上の角は盤の上");
        assert!(
            Board::contains((w - 0.01, h - 0.01)),
            "右下の角の手前も盤の上"
        );
        assert!(Board::contains((10.5, 5.5)));
        assert!(!Board::contains((-0.01, 5.5)), "左の縁を越えた");
        assert!(!Board::contains((w, 5.5)), "右の縁ちょうどは盤の外");
        assert!(!Board::contains((10.5, -0.01)), "上の縁を越えた");
        assert!(!Board::contains((10.5, h)), "下の縁ちょうどは盤の外");
    }

    /// 転がしてFellOffが返るまで進め、その時の位置を返す(FellOff以外の出来事は起きない前提)
    fn roll_until_fell_off(top: &mut Top) -> (f64, f64) {
        let board = Board::standard();
        for i in 0..200 {
            let before = top.pos;
            match top.step(&board, STEP, &Tilt::new(), NO_G) {
                Some(StepEvent::FellOff) => {
                    assert!(Board::contains(before), "直前までは盤の上");
                    return top.pos;
                }
                None => assert!(Board::contains(top.pos), "i={i}: 落ちるまでは盤の上"),
                other => panic!("i={i}: 場外より先に別の出来事: {other:?}"),
            }
        }
        panic!("盤から落ちなかった: {:?}", top.pos);
    }

    #[test]
    fn top_falls_off_when_its_center_crosses_the_rim() {
        // 4辺とも、縁の近くの平坦なマスから外へ向かって転がすと落ちる(跳ね返らない)
        let (w, h) = (BOARD_WIDTH as f64, BOARD_HEIGHT as f64);
        let cases = [
            ((0.6, 5.5), (-3.0, 0.0)),
            ((w - 0.6, 5.5), (3.0, 0.0)),
            ((10.5, 0.6), (0.0, -3.0)),
            ((10.5, h - 0.6), (0.0, 3.0)),
        ];
        for (pos, vel) in cases {
            let mut top = Top::new(pos);
            top.vel = vel;
            let fell = roll_until_fell_off(&mut top);
            assert!(!Board::contains(fell), "{pos:?}: 中心が縁を越えたら場外");
            assert!(
                fell.0 * vel.0.signum() > pos.0 * vel.0.signum()
                    || fell.1 * vel.1.signum() > pos.1 * vel.1.signum(),
                "{pos:?}: 進んでいた向きのまま外へ出る(縁で押し戻されない)"
            );
            assert!(
                top.vel.0 * vel.0 >= 0.0 && top.vel.1 * vel.1 >= 0.0,
                "{pos:?}: 縁で跳ね返らない: {:?}",
                top.vel
            );
        }
    }

    #[test]
    fn strong_tilt_rolls_the_top_off_the_board() {
        // 強く傾け続けると、縁で止まらずに盤から落ちる
        let board = Board::standard();
        let mut tilt = Tilt::new();
        for _ in 0..10 {
            tilt.press(TiltKey::Forward);
        }
        let mut top = Top::new((0.5, 0.5));
        let fell =
            (0..500).any(|_| top.step(&board, STEP, &tilt, NO_G) == Some(StepEvent::FellOff));
        assert!(fell, "{:?}", top.pos);
    }

    #[test]
    fn airborne_top_also_falls_off_the_rim() {
        // 飛び上がっている最中でも、中心が縁を越えたら着地を待たずに落ちる
        let board = Board::standard();
        let mut top = Top::new((0.6, 5.5));
        top.vel = (-5.0, 0.0);
        top.state = TopState::Airborne {
            remaining: HOP_DURATION,
            contact_g: 0.0,
        };
        let mut event = None;
        for _ in 0..100 {
            assert!(top.is_airborne(), "落ちる前に着地してしまった");
            event = top.step(&board, STEP, &Tilt::new(), NO_G);
            if event.is_some() {
                break;
            }
        }
        assert_eq!(event, Some(StepEvent::FellOff));
        assert!(!Board::contains(top.pos));
    }

    /// posで着地して弾かれる状態を作り、1ステップ進めて弾かせる。incomingは弾かれる前の向きの速度
    fn bounce_at(board: &Board, pos: (f64, f64), incoming: (f64, f64)) -> Top {
        let mut top = Top::new(pos);
        top.vel = incoming;
        top.state = TopState::Airborne {
            remaining: STEP,
            contact_g: 0.5,
        };
        assert_eq!(
            top.step(board, STEP, &Tilt::new(), NO_G),
            Some(StepEvent::Landed(Landing::Bounce))
        );
        assert!(approx(speed(&top), BOUNCE_SPEED));
        top
    }

    #[test]
    fn bounce_from_the_center_stays_on_the_board() {
        // 盤の中央付近で弾かれても、縁まで飛ばずに盤の上で止まる。
        // 縁まで一番近いのは縦方向(中央から6マス)。13列目は障害物が無い列なので、途中で飛び上がらない
        let board = Board::standard();
        let cy = BOARD_HEIGHT as f64 / 2.0;
        let cx = BOARD_WIDTH as f64 / 2.0;
        let cases = [
            ((13.5, cy), (0.0, 3.0)),
            ((13.5, cy), (0.0, -3.0)),
            ((cx, 0.5), (3.0, 0.0)),
            ((cx, 0.5), (-3.0, 0.0)),
        ];
        for (pos, incoming) in cases {
            let mut top = bounce_at(&board, pos, incoming);
            for i in 0..1000 {
                let event = top.step(&board, STEP, &Tilt::new(), NO_G);
                assert_ne!(
                    event,
                    Some(StepEvent::FellOff),
                    "{pos:?} {incoming:?} i={i}"
                );
                assert!(
                    Board::contains(top.pos),
                    "{pos:?} {incoming:?}: {:?}",
                    top.pos
                );
            }
            assert!(speed(&top) < 0.01, "{pos:?} {incoming:?}: 盤の上で止まる");
        }
    }

    #[test]
    fn speed_is_capped() {
        let board = Board::standard();
        let mut top = Top::new((10.5, 0.5));
        for _ in 0..500 {
            top.step(&board, STEP, &Tilt::new(), lateral(5.0));
            assert!(speed(&top) <= MAX_SPEED + 1e-9);
        }
    }

    #[test]
    fn low_g_contact_gives_a_light_landing() {
        let (top, landing) = land_after_contact(lateral(-0.2), NO_G);
        assert_eq!(landing, Landing::Light);
        assert!(!top.is_airborne());
        assert!(speed(&top) < BOUNCE_SPEED, "弾かれない");
    }

    #[test]
    fn middle_g_contact_bounces_the_top_away() {
        let (top, landing) = land_after_contact(lateral(-0.5), NO_G);
        assert_eq!(landing, Landing::Bounce);
        assert!(!top.is_airborne());
        assert!(approx(speed(&top), BOUNCE_SPEED), "勢いよく弾かれる");
        assert!(top.vel.0 < 0.0, "来た方向へ弾き返される");
        // 着地の直前の位置から、BOUNCE_KICKだけ来た方向へ一気に戻される
        // (着地のステップで進む分はBOUNCE_KICKより十分小さい)
        let board = Board::standard();
        let mut before_landing = top_just_left_of_bump(&board);
        let tilt = Tilt::new();
        let previous = loop {
            let pos = before_landing.pos;
            if let Some(StepEvent::Landed(_)) =
                before_landing.step(&board, STEP, &tilt, lateral(-0.5))
            {
                break pos;
            }
        };
        assert_eq!(before_landing.pos, top.pos);
        assert!(
            top.pos.0 < previous.0 - BOUNCE_KICK / 2.0,
            "位置も急に戻される: {:?} → {:?}",
            previous,
            top.pos
        );
    }

    #[test]
    fn high_g_contact_flies_the_top_off() {
        let (_, landing) = land_after_contact(lateral(-1.0), NO_G);
        assert_eq!(landing, Landing::Flown);
    }

    #[test]
    fn landing_uses_the_g_at_the_moment_of_contact() {
        // 踏んだ時のGで決まり、飛んでいる間にGが変わっても結果は変わらない
        let (_, landing) = land_after_contact(lateral(-1.0), NO_G);
        assert_eq!(landing, Landing::Flown);
        let (_, landing) = land_after_contact(lateral(-0.1), lateral(-3.0));
        assert_eq!(landing, Landing::Light);
    }

    #[test]
    fn airborne_lasts_for_the_hop_duration() {
        let board = Board::standard();
        let mut top = top_just_left_of_bump(&board);
        let tilt = Tilt::new();
        assert!(matches!(
            top.step(&board, STEP, &tilt, NO_G),
            Some(StepEvent::Hopped { .. })
        ));
        let mut steps = 0;
        while top.is_airborne() {
            top.step(&board, STEP, &tilt, NO_G);
            steps += 1;
            assert!(steps < 100);
        }
        let expected = (HOP_DURATION.as_millis() / STEP.as_millis()) as i32;
        assert!((steps - expected).abs() <= 1, "{steps} vs {expected}");
    }

    #[test]
    fn staying_on_the_same_bump_does_not_hop_again() {
        let (mut top, landing) = land_after_contact(lateral(-0.1), NO_G);
        assert_eq!(landing, Landing::Light);
        let board = Board::standard();
        // 着地後もしばらく同じ障害物の上にいても、入った瞬間ではないので飛び上がらない
        top.vel = (0.0, 0.0);
        for _ in 0..20 {
            assert_eq!(top.step(&board, STEP, &Tilt::new(), NO_G), None);
        }
    }

    #[test]
    fn entering_the_goal_cell_reports_goal() {
        let board = Board::standard();
        let (gx, gy) = board.goal();
        let mut top = Top::new((gx as f64 - 0.02, gy as f64 + 0.5));
        top.vel = (3.0, 0.0);
        assert_eq!(board.cell(gx - 1, gy), Cell::Flat, "ゴールの左隣は平坦");
        assert_eq!(
            top.step(&board, STEP, &Tilt::new(), NO_G),
            Some(StepEvent::Goal)
        );
    }

    // --- 凹(Hollow) ---

    /// 凹の物理のテストで使う凹のマス(盤の中央付近)
    const HOLLOW_AT: (usize, usize) = (10, 6);

    /// 全部平坦な盤に、指定したマスだけ置いた盤(凹のテストを固定配置に依存させないため)
    fn board_with(cells: &[((usize, usize), Cell)]) -> Board {
        let mut board = Board {
            cells: vec![Cell::Flat; BOARD_WIDTH * BOARD_HEIGHT],
            start: (1, 10),
        };
        for &((x, y), cell) in cells {
            board.cells[y * BOARD_WIDTH + x] = cell;
        }
        board
    }

    fn hollow_board() -> Board {
        board_with(&[(HOLLOW_AT, Cell::Hollow)])
    }

    /// posを含むマス
    fn cell_containing(pos: (f64, f64)) -> (usize, usize) {
        (pos.0 as usize, pos.1 as usize)
    }

    /// posで凹にハマって止まっているベーゴマ
    fn sunk_top(pos: (f64, f64)) -> Top {
        let mut top = Top::new(pos);
        top.state = TopState::Sunk;
        top
    }

    /// keyの向きへpresses回押した傾き
    fn tilt_toward(key: TiltKey, presses: usize) -> Tilt {
        let mut tilt = Tilt::new();
        for _ in 0..presses {
            tilt.press(key);
        }
        tilt
    }

    #[test]
    fn entering_a_hollow_head_on_sinks_the_top() {
        // 4辺とも、縁に垂直(正面)に入るとハマり、その後は凹のマスの中に留まる
        let board = hollow_board();
        let (hx, hy) = (HOLLOW_AT.0 as f64, HOLLOW_AT.1 as f64);
        let cases = [
            ((hx - 0.02, hy + 0.5), (3.0, 0.0)),
            ((hx + 1.02, hy + 0.5), (-3.0, 0.0)),
            ((hx + 0.5, hy - 0.02), (0.0, 3.0)),
            ((hx + 0.5, hy + 1.02), (0.0, -3.0)),
        ];
        for (pos, vel) in cases {
            let mut top = Top::new(pos);
            top.vel = vel;
            assert_eq!(
                top.step(&board, STEP, &Tilt::new(), NO_G),
                Some(StepEvent::Sank),
                "{pos:?}: 正面から入るとハマる"
            );
            assert_eq!(top.state, TopState::Sunk);
            for i in 0..300 {
                assert_eq!(
                    top.step(&board, STEP, &Tilt::new(), NO_G),
                    None,
                    "{pos:?} i={i}"
                );
                assert_eq!(
                    cell_containing(top.pos),
                    HOLLOW_AT,
                    "{pos:?} i={i}: 凹の中に留まる"
                );
                assert_eq!(top.state, TopState::Sunk);
            }
        }
    }

    #[test]
    fn entering_a_hollow_at_a_shallow_angle_grazes_and_bounces() {
        // 上の縁(法線は下向き)に、横へ流れながら浅い角度(cos=1/√10≈0.32)で入る
        let board = hollow_board();
        let mut top = Top::new((HOLLOW_AT.0 as f64 + 0.5, HOLLOW_AT.1 as f64 - 0.005));
        let incoming = (3.0, 1.0);
        top.vel = incoming;
        assert_eq!(
            top.step(&board, STEP, &Tilt::new(), NO_G),
            Some(StepEvent::Grazed(Landing::Bounce)),
            "側面をこすって弾かれる(G=0でも弾かれる)"
        );
        assert_eq!(top.state, TopState::Rolling, "ハマらない");
        assert!(approx(speed(&top), BOUNCE_SPEED), "勢いよく弾かれる");
        assert!(
            top.vel.0 < 0.0 && top.vel.1 < 0.0,
            "来た方向へ速度が反転する: {:?}",
            top.vel
        );
        let dot = top.vel.0 * incoming.0 + top.vel.1 * incoming.1;
        assert!(
            approx(dot, -BOUNCE_SPEED * incoming.0.hypot(incoming.1)),
            "真逆の向き: {:?}",
            top.vel
        );
        assert_ne!(cell_containing(top.pos), HOLLOW_AT, "凹の外へ弾き出される");
        // 盤の中央付近で弾かれても場外へは出ない
        for i in 0..1000 {
            assert_eq!(top.step(&board, STEP, &Tilt::new(), NO_G), None, "i={i}");
            assert!(Board::contains(top.pos), "i={i}: {:?}", top.pos);
        }
    }

    #[test]
    fn grazing_under_high_g_flies_the_top_off() {
        let board = hollow_board();
        let graze = |g: f64| {
            let mut top = Top::new((HOLLOW_AT.0 as f64 + 0.5, HOLLOW_AT.1 as f64 - 0.005));
            top.vel = (3.0, 1.0);
            top.step(&board, STEP, &Tilt::new(), lateral(g))
        };
        assert_eq!(graze(0.6), Some(StepEvent::Grazed(Landing::Flown)));
        assert_eq!(
            graze(0.4),
            Some(StepEvent::Grazed(Landing::Bounce)),
            "G=0.45以下なら弾かれるだけ"
        );
    }

    #[test]
    fn hollow_contact_threshold_is_exact() {
        assert_eq!(
            hollow_contact_from_cos(HOLLOW_GRAZE_COS),
            HollowContact::Sink,
            "しきい値ちょうどはハマる"
        );
        assert_eq!(
            hollow_contact_from_cos(HOLLOW_GRAZE_COS - 1e-6),
            HollowContact::Graze
        );
        assert_eq!(hollow_contact_from_cos(1.0), HollowContact::Sink);
        // 左の縁(法線は右向き)・上の縁(法線は下向き)とも、法線から60°の前後で分かれる
        let edge_angle = HOLLOW_GRAZE_COS.acos();
        let (left, right) = ((9, 6), (10, 6));
        let (above, below) = ((10, 5), (10, 6));
        for (delta, expected) in [
            (-0.1_f64.to_radians(), HollowContact::Sink),
            (0.1_f64.to_radians(), HollowContact::Graze),
        ] {
            let angle = edge_angle + delta;
            let from_left = (2.0 * angle.cos(), 2.0 * angle.sin());
            assert_eq!(hollow_contact(from_left, left, right), expected, "{delta}");
            let from_above = (2.0 * angle.sin(), 2.0 * angle.cos());
            assert_eq!(
                hollow_contact(from_above, above, below),
                expected,
                "{delta}"
            );
        }
    }

    #[test]
    fn sunk_top_cannot_leave_below_the_exit_speed() {
        // 最大より弱い傾きでは、60秒傾け続けても凹から出られない
        let board = hollow_board();
        let center = (HOLLOW_AT.0 as f64 + 0.5, HOLLOW_AT.1 as f64 + 0.5);
        for presses in 1..=4 {
            for key in [
                TiltKey::Right,
                TiltKey::Left,
                TiltKey::Forward,
                TiltKey::Back,
            ] {
                let tilt = tilt_toward(key, presses);
                let mut top = sunk_top(center);
                for i in 0..6000 {
                    assert_eq!(
                        top.step(&board, STEP, &tilt, NO_G),
                        None,
                        "{key:?}×{presses} i={i}"
                    );
                    assert_eq!(
                        cell_containing(top.pos),
                        HOLLOW_AT,
                        "{key:?}×{presses} i={i}"
                    );
                }
                assert_eq!(top.state, TopState::Sunk);
                assert!(speed(&top) < HOLLOW_EXIT_SPEED);
            }
        }
    }

    #[test]
    fn sunk_top_escapes_with_full_tilt() {
        let board = hollow_board();
        let center = (HOLLOW_AT.0 as f64 + 0.5, HOLLOW_AT.1 as f64 + 0.5);
        for key in [
            TiltKey::Right,
            TiltKey::Left,
            TiltKey::Forward,
            TiltKey::Back,
        ] {
            let tilt = tilt_toward(key, 20);
            let mut top = sunk_top(center);
            let steps = (0..100)
                .position(|_| top.step(&board, STEP, &tilt, NO_G) == Some(StepEvent::Escaped))
                .unwrap_or_else(|| panic!("{key:?}: 1秒以内に抜け出せなかった: {:?}", top.pos));
            assert!(steps > 10, "{key:?}: 一瞬では抜けない({steps}ステップ)");
            assert_eq!(top.state, TopState::Rolling, "{key:?}: 抜けたら転がる");
            assert_ne!(cell_containing(top.pos), HOLLOW_AT, "{key:?}");
            assert!(speed(&top) >= HOLLOW_EXIT_SPEED);
        }
    }

    #[test]
    fn sunk_top_keeps_building_speed_while_clamped() {
        // 縁の手前から、抜け出せない強さで縁へ向かって傾け続ける
        let board = hollow_board();
        let tilt = tilt_toward(TiltKey::Right, 3);
        let mut top = sunk_top((HOLLOW_AT.0 as f64 + 0.9, HOLLOW_AT.1 as f64 + 0.5));
        let mut previous = speed(&top);
        let mut clamped_steps = 0;
        for i in 0..100 {
            assert_eq!(top.step(&board, STEP, &tilt, NO_G), None, "i={i}");
            assert_eq!(cell_containing(top.pos), HOLLOW_AT, "i={i}: 縁で留まる");
            assert!(
                speed(&top) > previous,
                "i={i}: 留められている間も速さは増え続ける: {} → {}",
                previous,
                speed(&top)
            );
            previous = speed(&top);
            if top.pos.0 > HOLLOW_AT.0 as f64 + 0.999 {
                clamped_steps += 1;
            }
        }
        assert!(clamped_steps > 30, "縁で留められていた: {clamped_steps}");
        assert!(top.vel.0 > 0.8, "速度は殺されていない: {:?}", top.vel);
        assert_eq!(top.state, TopState::Sunk);
    }

    #[test]
    fn sunk_top_escaping_over_the_rim_falls_off() {
        // 縁にある凹から盤の外へ抜け出したら、そのまま落ちる
        let board = board_with(&[((0, 6), Cell::Hollow)]);
        let tilt = tilt_toward(TiltKey::Left, 20);
        let mut top = sunk_top((0.5, 6.5));
        let event = (0..100).find_map(|_| top.step(&board, STEP, &tilt, NO_G));
        assert_eq!(event, Some(StepEvent::FellOff));
        assert!(!Board::contains(top.pos));
    }

    #[test]
    fn landing_on_a_hollow_sinks_after_landing() {
        // 凸を踏んで飛び上がり、隣の凹に着地する
        let (bump, hollow) = ((9, 6), (10, 6));
        let board = board_with(&[(bump, Cell::Bump), (hollow, Cell::Hollow)]);
        let mut top = Top::new((bump.0 as f64 - 0.02, bump.1 as f64 + 0.5));
        top.vel = (5.0, 0.0);
        let tilt = Tilt::new();
        assert!(matches!(
            top.step(&board, STEP, &tilt, NO_G),
            Some(StepEvent::Hopped { .. })
        ));
        let event = (0..100).find_map(|_| top.step(&board, STEP, &tilt, NO_G));
        assert_eq!(
            event,
            Some(StepEvent::Landed(Landing::Light)),
            "着地の出来事を返す"
        );
        assert_eq!(cell_containing(top.pos), hollow, "凹の上に着地した");
        assert_eq!(top.state, TopState::Sunk, "着地したらそのままハマる");
    }

    #[test]
    fn moving_within_a_cell_triggers_nothing() {
        // マスが変わらなければ、凸・凹とも何も起きない(入った瞬間だけ判定する)
        for cell in [Cell::Bump, Cell::Hollow] {
            let board = board_with(&[(HOLLOW_AT, cell)]);
            let mut top = Top::new((HOLLOW_AT.0 as f64 + 0.1, HOLLOW_AT.1 as f64 + 0.5));
            top.vel = (1.0, 0.0);
            for i in 0..50 {
                assert_eq!(
                    top.step(&board, STEP, &Tilt::new(), NO_G),
                    None,
                    "{cell:?} i={i}"
                );
            }
            assert_eq!(top.state, TopState::Rolling);
        }
    }
}
