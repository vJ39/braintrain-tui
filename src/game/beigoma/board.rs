//! べーの盤面: 固定配置の盤(障害物・ゴール)、連打式の2軸の傾き、ベーゴマの転がり。
//!
//! 座標は盤のマス単位(左上が(0,0)、xは右、yは下)。画面の上が軽トラの前方。
//! - 傾き: pitch(前後軸、+が前傾)・roll(左右軸、+が右傾)。-TILT_MAX〜+TILT_MAX
//! - 軽トラのG(軽トラの加速度の向き)は、ベーゴマには慣性として逆向きにかかる
//!   (ブレーキ=後方向のGで、ベーゴマは前(画面の上)へ押される)
//! - 平坦な場所の摩擦はGによらず一定。障害物を踏むと一度飛び上がり、着地の瞬間の摩擦が
//!   踏んだ時のGに応じて増える。その大きさで 軽い着地/弾かれる/吹っ飛ぶ の3段階になる

use std::time::Duration;

use super::truck::GForce;

/// 盤の大きさ(マス)
pub const BOARD_WIDTH: usize = 20;
pub const BOARD_HEIGHT: usize = 12;

/// 盤の固定配置。'.'=平坦、'#'=障害物(凹凸)、'S'=投入位置、'G'=ゴール(唯一)。
/// 毎回同じ配置なので、繰り返しプレイしてコースを覚えられる
const LAYOUT: [&str; BOARD_HEIGHT] = [
    "....................",
    "...........#.....G..",
    "....#...........#...",
    "..........#.........",
    ".......#.......#....",
    "..#.........#.......",
    ".........#.......#..",
    "....#..........#....",
    "...........#........",
    ".......#.......#....",
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
/// 弾かれた時の速さ(マス/秒)と、弾かれた瞬間に位置が飛ぶ量(マス)
pub const BOUNCE_SPEED: f64 = 10.0;
pub const BOUNCE_KICK: f64 = 1.0;
/// 盤の縁に当たった時の跳ね返りの係数
pub const WALL_RESTITUTION: f64 = 0.5;
/// ベーゴマの速さの上限(マス/秒)。1ステップでマスを飛び越さないようにする
pub const MAX_SPEED: f64 = 15.0;

/// 盤の1マス
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Flat,
    Bump,
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

    /// マス(x, y)。盤の外は平坦として扱う(縁で止まるので実際には来ない)
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

/// 平坦な場所(ゴールも含む)の摩擦。Gの大小によらず一定
pub fn surface_friction(cell: Cell, g: f64) -> f64 {
    match cell {
        Cell::Flat | Cell::Goal => ROLLING_FRICTION,
        // 障害物(凹凸)の上ではGがかかるほど引っかかる
        Cell::Bump => landing_friction(g),
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
    /// 障害物を踏んで飛び上がっている。contact_gは踏んだ瞬間のG
    Airborne { remaining: Duration, contact_g: f64 },
}

/// 1ステップの間に起きた出来事
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepEvent {
    /// 障害物を踏んで飛び上がった
    Hopped { contact_g: f64 },
    /// 着地した
    Landed(Landing),
    /// ゴールに入った
    Goal,
}

/// 盤の上のベーゴマ
#[derive(Debug, Clone)]
pub struct Top {
    pub pos: (f64, f64),
    pub vel: (f64, f64),
    pub state: TopState,
    /// 障害物のマスの上にいるか。障害物へ入った瞬間だけ飛び上がるために使う
    on_bump: bool,
}

impl Top {
    /// posに止まった状態で置く
    pub fn new(pos: (f64, f64)) -> Self {
        Self {
            pos,
            vel: (0.0, 0.0),
            state: TopState::Rolling,
            on_bump: false,
        }
    }

    pub fn is_airborne(&self) -> bool {
        matches!(self.state, TopState::Airborne { .. })
    }

    /// dtだけ物理を進める。傾きとGを加速度として加え、摩擦で減速し、縁で跳ね返る
    pub fn step(
        &mut self,
        board: &Board,
        dt: Duration,
        tilt: &Tilt,
        g: GForce,
    ) -> Option<StepEvent> {
        let secs = dt.as_secs_f64();
        if let TopState::Airborne {
            remaining,
            contact_g,
        } = self.state
        {
            return self.fly(board, dt, remaining, contact_g);
        }
        // 傾きの向きへ転がり、Gは慣性として軽トラの加速度と逆向きにかかる
        // (画面の上が前方なので、前傾(pitch +)・後方向のG(longitudinal -)はyを減らす向き)
        let ax = tilt.roll() * TILT_ACCEL_PER_LEVEL - g.lateral * G_ACCEL_PER_G;
        let ay = -tilt.pitch() * TILT_ACCEL_PER_LEVEL + g.longitudinal * G_ACCEL_PER_G;
        self.vel.0 += ax * secs;
        self.vel.1 += ay * secs;
        let friction = surface_friction(board.cell_at(self.pos), g.magnitude());
        let damping = (1.0 - friction * secs).max(0.0);
        self.vel.0 *= damping;
        self.vel.1 *= damping;
        self.cap_speed();
        self.advance(secs);
        match board.cell_at(self.pos) {
            Cell::Bump if !self.on_bump => {
                // 障害物を踏んだ瞬間: 一度飛び上がり、このときのGで着地の摩擦が決まる
                self.on_bump = true;
                let contact_g = g.magnitude();
                self.state = TopState::Airborne {
                    remaining: HOP_DURATION,
                    contact_g,
                };
                Some(StepEvent::Hopped { contact_g })
            }
            Cell::Bump => None,
            Cell::Goal => {
                self.on_bump = false;
                Some(StepEvent::Goal)
            }
            Cell::Flat => {
                self.on_bump = false;
                None
            }
        }
    }

    /// 飛び上がっている間: 盤に触れていないので傾き・G・摩擦は効かず、そのままの速度で進む。
    /// 着地したら、踏んだ時のGから決まる摩擦で結果を判定する
    fn fly(
        &mut self,
        board: &Board,
        dt: Duration,
        remaining: Duration,
        contact_g: f64,
    ) -> Option<StepEvent> {
        self.advance(dt.as_secs_f64());
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
        self.on_bump = board.cell_at(self.pos) == Cell::Bump;
        Some(StepEvent::Landed(landing))
    }

    /// 弾かれる: 来た方向へ勢いよく弾き返し、位置も一気に戻す
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
        self.pos = (
            self.pos.0.clamp(MIN_POS, BOARD_WIDTH as f64 - MIN_POS),
            self.pos.1.clamp(MIN_POS, BOARD_HEIGHT as f64 - MIN_POS),
        );
    }

    fn cap_speed(&mut self) {
        let speed = self.vel.0.hypot(self.vel.1);
        if speed > MAX_SPEED {
            let scale = MAX_SPEED / speed;
            self.vel = (self.vel.0 * scale, self.vel.1 * scale);
        }
    }

    /// 速度のぶん進め、盤の縁に当たったら跳ね返る
    fn advance(&mut self, secs: f64) {
        self.pos.0 += self.vel.0 * secs;
        self.pos.1 += self.vel.1 * secs;
        let (x, vx) = reflect(self.pos.0, self.vel.0, BOARD_WIDTH as f64);
        let (y, vy) = reflect(self.pos.1, self.vel.1, BOARD_HEIGHT as f64);
        self.pos = (x, y);
        self.vel = (vx, vy);
    }
}

/// ベーゴマの中心が盤の縁からこれ以上近づかない距離(マス)。ベーゴマの半径
const MIN_POS: f64 = 0.5;

/// 1軸ぶんの縁での跳ね返り。0〜sizeの盤の中にベーゴマの中心(半径MIN_POS)を収める
fn reflect(pos: f64, vel: f64, size: f64) -> (f64, f64) {
    if pos < MIN_POS {
        (MIN_POS, vel.abs() * WALL_RESTITUTION)
    } else if pos > size - MIN_POS {
        (size - MIN_POS, -vel.abs() * WALL_RESTITUTION)
    } else {
        (pos, vel)
    }
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
        for g in [0.0, 0.3, 0.8, 1.5, 3.0] {
            assert_eq!(surface_friction(Cell::Flat, g), ROLLING_FRICTION, "g={g}");
            assert_eq!(surface_friction(Cell::Goal, g), ROLLING_FRICTION, "g={g}");
        }
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

    #[test]
    fn top_stays_inside_the_board_and_bounces_off_the_rim() {
        let board = Board::standard();
        let mut top = Top::new((1.0, 0.5));
        top.vel = (-10.0, 0.0);
        for _ in 0..20 {
            top.step(&board, STEP, &Tilt::new(), NO_G);
            assert!(top.pos.0 >= 0.0 && top.pos.0 <= BOARD_WIDTH as f64);
        }
        assert!(top.vel.0 > 0.0, "縁で跳ね返る");
        // 強く傾け続けても盤から出ない
        let mut tilt = Tilt::new();
        for _ in 0..10 {
            tilt.press(TiltKey::Forward);
            tilt.press(TiltKey::Left);
        }
        let mut top = Top::new((0.5, 0.5));
        for _ in 0..500 {
            top.step(&board, STEP, &tilt, NO_G);
            assert!(top.pos.0 >= 0.0 && top.pos.0 < BOARD_WIDTH as f64);
            assert!(top.pos.1 >= 0.0 && top.pos.1 < BOARD_HEIGHT as f64);
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
        let board = Board::standard();
        let before = top_just_left_of_bump(&board).pos;
        let (top, landing) = land_after_contact(lateral(-0.5), NO_G);
        assert_eq!(landing, Landing::Bounce);
        assert!(!top.is_airborne());
        assert!(approx(speed(&top), BOUNCE_SPEED), "勢いよく弾かれる");
        assert!(top.vel.0 < 0.0, "来た方向へ弾き返される");
        assert!(top.pos.0 < before.0, "位置も急に戻される");
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
}
