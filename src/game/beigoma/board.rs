//! べーの盤面: ROUNDごとの盤(凹凸・投入位置は固定配置、ゴールは毎回ランダム)、連打式の2軸の傾き、
//! ベーゴマの転がり。
//!
//! 座標は盤のマス単位(左上が(0,0)、xは右、yは下)。画面の上が軽トラの前方。
//! - 1セッションは2ROUND(ROUND1=やさしい、ROUND2=むずかしい)。ROUND2は縁の内側に凹凸の輪があり、
//!   ゴールは投入位置からの直線上に必ず凹凸が挟まる位置にだけ置く(直線移動だけでは届かない)
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

use rand::Rng;

use super::truck::GForce;

/// 盤の大きさ(マス)
pub const BOARD_WIDTH: usize = 20;
pub const BOARD_HEIGHT: usize = 12;

/// 1セッションのROUND数
pub const ROUNDS_PER_SESSION: u32 = 2;

/// ROUND1の配置。'.'=平坦、'#'=凸(でっぱり)、'u'=凹(くぼみ)、'S'=投入位置。
/// ゴールは毎回ランダムに置くので文字を持たない
const LAYOUT_ROUND1: [&str; BOARD_HEIGHT] = [
    "....................",
    "...........#........",
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

/// ROUND2の配置(文字はLAYOUT_ROUND1と同じ)。縁の1マス内側に凸と凹を交互に並べた輪を置き、
/// 上・下・左・右に1か所ずつ2マスの切れ目を作る。投入位置は輪の内側で、内側にも凹凸を数個置く
const LAYOUT_ROUND2: [&str; BOARD_HEIGHT] = [
    "....................",
    ".#u#u#u#u..#u#u#u#u.",
    ".u................#.",
    ".#....u...........u.",
    "............u.....#.",
    "....................",
    ".u......#...........",
    ".#............#...u.",
    ".u..S......u......#.",
    ".#................u.",
    ".u#u#u#u#u#u#u..#u#.",
    "....................",
];

/// ゴールを置いてよいマスの条件
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoalRule {
    /// 投入位置からゴールまでの最短距離(マス。マスの中心どうしの距離)
    pub min_distance: f64,
    /// 投入位置からゴールへの直線上に凹凸が1つ以上あること(直線移動だけでは届かない)
    pub blocked_straight_line: bool,
}

/// ROUNDごとのパラメータ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoundParams {
    /// HUDに出すROUNDの名前
    pub label: &'static str,
    /// 凹凸と投入位置の配置
    pub layout: &'static [&'static str; BOARD_HEIGHT],
    /// ゴールを置いてよいマスの条件
    pub goal_rule: GoalRule,
}

/// round_index番目(0始まり)のROUNDのパラメータ。最後のROUNDより先は最後のROUNDのまま
pub fn round_params(round_index: u32) -> RoundParams {
    match round_index {
        0 => RoundParams {
            label: "ROUND 1 やさしい",
            layout: &LAYOUT_ROUND1,
            goal_rule: GoalRule {
                min_distance: 10.0,
                blocked_straight_line: false,
            },
        },
        _ => RoundParams {
            label: "ROUND 2 むずかしい",
            layout: &LAYOUT_ROUND2,
            goal_rule: GoalRule {
                min_distance: 6.0,
                blocked_straight_line: true,
            },
        },
    }
}

/// 直線上に凹凸があるかを調べる間隔(マス)
const LINE_SAMPLE_STEP: f64 = 0.25;

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

/// 盤。凹凸と投入位置は配置(layout)どおり、ゴールはgoalに1つ置く
#[derive(Debug, Clone)]
pub struct Board {
    /// 凹凸の配置(Flat/Bump/Hollowだけ。ゴールはgoalで持つ)
    cells: Vec<Cell>,
    start: (usize, usize),
    goal: (usize, usize),
}

impl Board {
    /// paramsの配置に、ルールを満たす候補からランダムに選んだゴールを置いた盤
    pub fn generate(params: &RoundParams, rng: &mut impl Rng) -> Self {
        let candidates = Self::goal_candidates(params.layout, &params.goal_rule);
        assert!(
            !candidates.is_empty(),
            "{}: ゴールの候補が無い",
            params.label
        );
        let goal = candidates[rng.gen_range(0..candidates.len())];
        Self::with_goal(params.layout, goal)
    }

    /// 配置layoutにゴールをgoalに置いた盤(テストで決定的にするため)。
    /// goalは盤の上の平坦なマス(投入位置を除く)でなければpanicする
    pub fn with_goal(layout: &[&str; BOARD_HEIGHT], goal: (usize, usize)) -> Self {
        let (cells, start) = parse_layout(layout);
        assert!(
            goal.0 < BOARD_WIDTH && goal.1 < BOARD_HEIGHT,
            "ゴール{goal:?}が盤の外"
        );
        assert!(
            cells[goal.1 * BOARD_WIDTH + goal.0] == Cell::Flat && goal != start,
            "ゴール{goal:?}は平坦なマス(投入位置以外)に置く"
        );
        Self { cells, start, goal }
    }

    /// 配置layoutでルールruleを満たすゴールの候補(左上から行優先の順)。
    /// 候補は平坦なマス(投入位置を除く)のうち、投入位置からmin_distance以上離れていて、
    /// blocked_straight_lineなら投入位置からの直線上に凹凸があるもの
    pub fn goal_candidates(layout: &[&str; BOARD_HEIGHT], rule: &GoalRule) -> Vec<(usize, usize)> {
        let (cells, start) = parse_layout(layout);
        // 直線の判定には凹凸だけ使うので、ゴールは仮に投入位置へ置いておく
        let board = Self {
            cells,
            start,
            goal: start,
        };
        let from = board.start_position();
        (0..BOARD_HEIGHT)
            .flat_map(|y| (0..BOARD_WIDTH).map(move |x| (x, y)))
            .filter(|&cell| {
                if cell == start || board.cells[cell.1 * BOARD_WIDTH + cell.0] != Cell::Flat {
                    return false;
                }
                let to = cell_center(cell);
                (to.0 - from.0).hypot(to.1 - from.1) >= rule.min_distance
                    && (!rule.blocked_straight_line || board.straight_line_is_blocked(from, to))
            })
            .collect()
    }

    /// ゴールのマス
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn goal(&self) -> (usize, usize) {
        self.goal
    }

    /// fromからtoへの線分上(LINE_SAMPLE_STEP刻み、両端を含む)に凹凸があるか
    pub fn straight_line_is_blocked(&self, from: (f64, f64), to: (f64, f64)) -> bool {
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        let samples = (length / LINE_SAMPLE_STEP).ceil() as usize;
        (0..=samples).any(|i| {
            let t = if samples == 0 {
                0.0
            } else {
                i as f64 / samples as f64
            };
            let pos = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            matches!(self.cell_at(pos), Cell::Bump | Cell::Hollow)
        })
    }

    /// render.rsのテスト用: ROUND1の配置で、ゴールを旧来の固定配置の位置(17,1)に置いた盤
    #[cfg(test)]
    pub fn standard() -> Self {
        Self::with_goal(&LAYOUT_ROUND1, (17, 1))
    }

    /// 位置が盤の上にあるか。ベーゴマの中心が縁を越えたら場外(落ちる)
    pub fn contains(pos: (f64, f64)) -> bool {
        (0.0..BOARD_WIDTH as f64).contains(&pos.0) && (0.0..BOARD_HEIGHT as f64).contains(&pos.1)
    }

    /// マス(x, y)。ゴールのマスはCell::Goal。盤の外は平坦として扱う(場外に出た時点でゲームは終わる)
    pub fn cell(&self, x: usize, y: usize) -> Cell {
        if x >= BOARD_WIDTH || y >= BOARD_HEIGHT {
            return Cell::Flat;
        }
        if (x, y) == self.goal {
            return Cell::Goal;
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
        cell_center(self.start)
    }
}

/// 配置の文字列を凹凸のマス(Flat/Bump/Hollow)と投入位置にする
fn parse_layout(layout: &[&str; BOARD_HEIGHT]) -> (Vec<Cell>, (usize, usize)) {
    let mut cells = Vec::with_capacity(BOARD_WIDTH * BOARD_HEIGHT);
    let mut start = None;
    for (y, row) in layout.iter().enumerate() {
        assert_eq!(row.chars().count(), BOARD_WIDTH, "{y}行目の幅");
        for (x, c) in row.chars().enumerate() {
            cells.push(match c {
                '#' => Cell::Bump,
                'u' => Cell::Hollow,
                'S' => {
                    start = Some((x, y));
                    Cell::Flat
                }
                _ => Cell::Flat,
            });
        }
    }
    (cells, start.expect("配置に投入位置'S'がある"))
}

/// マスの中心の座標
fn cell_center(cell: (usize, usize)) -> (f64, f64) {
    (cell.0 as f64 + 0.5, cell.1 as f64 + 0.5)
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
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    const STEP: Duration = Duration::from_millis(10);

    /// 旧来の固定配置でゴールがあった位置。ROUND1の盤をこのゴールで作ると、既存のテストの前提がそのまま使える
    const ROUND1_GOAL: (usize, usize) = (17, 1);

    fn round1_board() -> Board {
        Board::with_goal(&LAYOUT_ROUND1, ROUND1_GOAL)
    }
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
        let board = round1_board();
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

    const LAYOUTS: [&[&str; BOARD_HEIGHT]; 2] = [&LAYOUT_ROUND1, &LAYOUT_ROUND2];

    #[test]
    fn layouts_have_one_start_and_no_fixed_goal() {
        for (i, layout) in LAYOUTS.iter().enumerate() {
            for row in layout.iter() {
                assert_eq!(row.chars().count(), BOARD_WIDTH, "layout{i}");
            }
            let all: String = layout.concat();
            assert_eq!(
                all.matches('G').count(),
                0,
                "layout{i}: ゴールは毎回ランダムなので文字を持たない"
            );
            assert_eq!(all.matches('S').count(), 1, "layout{i}");
            assert!(
                all.chars().all(|c| ".#uS".contains(c)),
                "layout{i}: 使う文字は4種だけ"
            );
        }
        let board = round1_board();
        assert_eq!(board.goal(), ROUND1_GOAL);
        assert_eq!(board.cell(ROUND1_GOAL.0, ROUND1_GOAL.1), Cell::Goal);
        assert_eq!(
            board.cell_at(board.start_position()),
            Cell::Flat,
            "投入位置は平坦"
        );
        assert_eq!(board.start_position(), (1.5, 10.5));
    }

    #[test]
    fn layout_chars_map_to_bump_and_hollow() {
        for layout in LAYOUTS {
            let goal = Board::goal_candidates(
                layout,
                &GoalRule {
                    min_distance: 0.0,
                    blocked_straight_line: false,
                },
            )[0];
            let board = Board::with_goal(layout, goal);
            let (mut bumps, mut hollows) = (0, 0);
            for (y, row) in layout.iter().enumerate() {
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
    }

    #[test]
    fn round1_keeps_the_previous_obstacles() {
        // ROUND1は旧来の固定配置から'G'を外しただけ(凹凸と投入位置はそのまま)
        let board = round1_board();
        for &(x, y) in &[(11, 1), (4, 2), (16, 2), (10, 3), (2, 5), (4, 10)] {
            assert!(
                matches!(board.cell(x, y), Cell::Bump | Cell::Hollow),
                "({x},{y})"
            );
        }
        assert_eq!(board.cell(4, 10), Cell::Bump);
        assert_eq!(board.cell(16, 2), Cell::Hollow);
    }

    // --- ROUNDとゴール ---

    fn start_of(layout: &[&str; BOARD_HEIGHT]) -> (f64, f64) {
        let goal = Board::goal_candidates(
            layout,
            &GoalRule {
                min_distance: 0.0,
                blocked_straight_line: false,
            },
        )[0];
        Board::with_goal(layout, goal).start_position()
    }

    fn center_of(cell: (usize, usize)) -> (f64, f64) {
        (cell.0 as f64 + 0.5, cell.1 as f64 + 0.5)
    }

    fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
        (a.0 - b.0).hypot(a.1 - b.1)
    }

    #[test]
    fn round_params_has_two_rounds_and_clamps_beyond() {
        assert_eq!(ROUNDS_PER_SESSION, 2);
        let round1 = round_params(0);
        let round2 = round_params(1);
        assert_eq!(round1.label, "ROUND 1 やさしい");
        assert_eq!(round2.label, "ROUND 2 むずかしい");
        assert_eq!(round1.layout, &LAYOUT_ROUND1);
        assert_eq!(round2.layout, &LAYOUT_ROUND2);
        assert!(
            !round1.goal_rule.blocked_straight_line,
            "ROUND1は直線で届いてもよい"
        );
        assert!(
            round2.goal_rule.blocked_straight_line,
            "ROUND2は直線では届かない"
        );
        assert_ne!(round1, round2);
        for beyond in [2, 5, 100] {
            assert_eq!(
                round_params(beyond),
                round2,
                "最後のROUNDより先は最後のまま"
            );
        }
    }

    #[test]
    fn goal_candidates_are_never_empty_for_every_round() {
        for round in 0..ROUNDS_PER_SESSION {
            let params = round_params(round);
            let candidates = Board::goal_candidates(params.layout, &params.goal_rule);
            assert!(
                candidates.len() >= 5,
                "ROUND{}: ゴール候補が十分にある: {}",
                round + 1,
                candidates.len()
            );
        }
    }

    #[test]
    fn every_candidate_satisfies_the_rule() {
        for round in 0..ROUNDS_PER_SESSION {
            let params = round_params(round);
            let start = start_of(params.layout);
            for goal in Board::goal_candidates(params.layout, &params.goal_rule) {
                let ch = params.layout[goal.1].as_bytes()[goal.0];
                assert_eq!(
                    ch,
                    b'.',
                    "ROUND{} {goal:?}: 平坦なマス(投入位置でもない)",
                    round + 1
                );
                assert!(
                    distance(start, center_of(goal)) >= params.goal_rule.min_distance,
                    "ROUND{} {goal:?}: 投入位置から離れている",
                    round + 1
                );
                if params.goal_rule.blocked_straight_line {
                    let board = Board::with_goal(params.layout, goal);
                    assert!(
                        board.straight_line_is_blocked(start, center_of(goal)),
                        "ROUND{} {goal:?}: 直線上に凹凸がある",
                        round + 1
                    );
                }
            }
        }
    }

    #[test]
    fn candidates_exclude_cells_that_break_the_rule() {
        // ROUND2: 投入位置の真上(4,2)は平坦で十分に遠いが、直線で届くので候補にしない
        let params = round_params(1);
        let start = start_of(params.layout);
        let clear = (4, 2);
        assert_eq!(params.layout[clear.1].as_bytes()[clear.0], b'.');
        assert!(distance(start, center_of(clear)) >= params.goal_rule.min_distance);
        let candidates = Board::goal_candidates(params.layout, &params.goal_rule);
        assert!(!candidates.contains(&clear), "直線で届くマスは候補にしない");
        // 近すぎるマス・凹凸・投入位置も候補にしない
        let round1 = round_params(0);
        let candidates = Board::goal_candidates(round1.layout, &round1.goal_rule);
        assert!(!candidates.contains(&(2, 10)), "投入位置の隣は近すぎる");
        assert!(!candidates.contains(&(1, 10)), "投入位置");
        assert!(!candidates.contains(&(11, 1)), "凸");
        assert!(!candidates.contains(&(16, 2)), "凹");
        assert!(
            candidates.contains(&ROUND1_GOAL),
            "旧来のゴール位置は候補に入る"
        );
    }

    #[test]
    fn round2_goal_is_never_reachable_in_a_straight_line() {
        let params = round_params(1);
        for seed in 0..100 {
            let board = Board::generate(&params, &mut StdRng::seed_from_u64(seed));
            let goal = board.goal();
            assert!(
                board.straight_line_is_blocked(board.start_position(), center_of(goal)),
                "seed={seed} {goal:?}: 直線上に凹凸がある"
            );
            assert_eq!(board.cell(goal.0, goal.1), Cell::Goal);
        }
    }

    #[test]
    fn round2_has_obstacles_on_the_rim() {
        // 盤の縁から1マス以内(上下左右それぞれ)に凹凸がある
        let board = Board::with_goal(
            &LAYOUT_ROUND2,
            Board::goal_candidates(&LAYOUT_ROUND2, &round_params(1).goal_rule)[0],
        );
        let is_obstacle =
            |x: usize, y: usize| matches!(board.cell(x, y), Cell::Bump | Cell::Hollow);
        let sides: [(&str, Vec<(usize, usize)>); 4] = [
            (
                "上",
                (0..BOARD_WIDTH).flat_map(|x| [(x, 0), (x, 1)]).collect(),
            ),
            (
                "下",
                (0..BOARD_WIDTH)
                    .flat_map(|x| [(x, BOARD_HEIGHT - 1), (x, BOARD_HEIGHT - 2)])
                    .collect(),
            ),
            (
                "左",
                (0..BOARD_HEIGHT).flat_map(|y| [(0, y), (1, y)]).collect(),
            ),
            (
                "右",
                (0..BOARD_HEIGHT)
                    .flat_map(|y| [(BOARD_WIDTH - 1, y), (BOARD_WIDTH - 2, y)])
                    .collect(),
            ),
        ];
        for (name, cells) in sides {
            let count = cells.iter().filter(|&&(x, y)| is_obstacle(x, y)).count();
            assert!(count >= 5, "{name}の縁に凹凸がある: {count}");
        }
        // 投入位置は輪の内側(縁から2マス以上内側)
        let (sx, sy) = board.start_position();
        assert!(
            sx > 2.0 && sx < BOARD_WIDTH as f64 - 2.0 && sy > 2.0 && sy < BOARD_HEIGHT as f64 - 2.0
        );
        // ROUND1より凹凸が多い
        let count = |layout: &[&str; BOARD_HEIGHT]| {
            layout
                .concat()
                .chars()
                .filter(|&c| c == '#' || c == 'u')
                .count()
        };
        assert!(count(&LAYOUT_ROUND2) > count(&LAYOUT_ROUND1) * 2);
    }

    #[test]
    fn goal_is_random_across_seeds() {
        for round in 0..ROUNDS_PER_SESSION {
            let params = round_params(round);
            let goals: std::collections::HashSet<_> = (0..20)
                .map(|seed| Board::generate(&params, &mut StdRng::seed_from_u64(seed)).goal())
                .collect();
            assert!(
                goals.len() >= 2,
                "ROUND{}: ゴールが毎回変わる: {goals:?}",
                round + 1
            );
        }
    }

    #[test]
    fn generate_keeps_the_layout_and_puts_one_goal() {
        for round in 0..ROUNDS_PER_SESSION {
            let params = round_params(round);
            let board = Board::generate(&params, &mut StdRng::seed_from_u64(7));
            let goal = board.goal();
            assert!(Board::goal_candidates(params.layout, &params.goal_rule).contains(&goal));
            let goals = (0..BOARD_HEIGHT)
                .flat_map(|y| (0..BOARD_WIDTH).map(move |x| (x, y)))
                .filter(|&(x, y)| board.cell(x, y) == Cell::Goal)
                .count();
            assert_eq!(goals, 1, "ゴールは唯一");
            assert_eq!(board.start_position(), start_of(params.layout));
        }
    }

    #[test]
    fn with_goal_places_the_goal_where_asked() {
        let board = Board::with_goal(&LAYOUT_ROUND1, (10, 0));
        assert_eq!(board.goal(), (10, 0));
        assert_eq!(board.cell(10, 0), Cell::Goal);
        assert_eq!(
            board.cell(ROUND1_GOAL.0, ROUND1_GOAL.1),
            Cell::Flat,
            "指定した位置以外はゴールにしない"
        );
    }

    #[test]
    #[should_panic]
    fn with_goal_rejects_a_non_candidate() {
        // 凸(11,1)にはゴールを置けない
        Board::with_goal(&LAYOUT_ROUND1, (11, 1));
    }

    #[test]
    #[should_panic]
    fn with_goal_rejects_the_start() {
        Board::with_goal(&LAYOUT_ROUND1, (1, 10));
    }

    #[test]
    #[should_panic]
    fn with_goal_rejects_outside_the_board() {
        Board::with_goal(&LAYOUT_ROUND1, (BOARD_WIDTH, 0));
    }

    #[test]
    fn straight_line_is_blocked_detects_a_bump_on_the_segment() {
        let board = round1_board();
        // 投入位置(1,10)から右へ: (4,10)の凸を通る
        assert!(board.straight_line_is_blocked((1.5, 10.5), (6.5, 10.5)));
        // 凹(2,5)も遮るものとして扱う
        assert!(board.straight_line_is_blocked((0.5, 5.5), (5.5, 5.5)));
        // 向きを逆にしても同じ
        assert!(board.straight_line_is_blocked((6.5, 10.5), (1.5, 10.5)));
        // 斜めの線分でも途中の凸を見つける((11,1)の凸を斜めに横切る)
        assert!(board.straight_line_is_blocked((9.5, 3.5), (13.5, -0.5)));
    }

    #[test]
    fn straight_line_is_blocked_is_false_on_a_clear_row() {
        let board = round1_board();
        // 最上段は全部平坦
        assert!(!board.straight_line_is_blocked((0.5, 0.5), (19.5, 0.5)));
        // 凸の手前で止まる線分は遮られない
        assert!(!board.straight_line_is_blocked((1.5, 10.5), (3.5, 10.5)));
        // ゴールのマスは遮るものではない
        let (gx, gy) = board.goal();
        assert!(!board.straight_line_is_blocked(
            (gx as f64 - 3.5, gy as f64 + 0.5),
            (gx as f64 + 0.5, gy as f64 + 0.5)
        ));
        // 長さ0の線分
        assert!(!board.straight_line_is_blocked((0.5, 0.5), (0.5, 0.5)));
    }

    #[test]
    fn cell_at_uses_the_cell_containing_the_position() {
        let board = round1_board();
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
        let board = round1_board();
        let mut top = Top::new(board.start_position());
        for _ in 0..100 {
            assert_eq!(top.step(&board, STEP, &Tilt::new(), NO_G), None);
        }
        assert_eq!(top.pos, board.start_position());
        assert_eq!(top.vel, (0.0, 0.0));
    }

    #[test]
    fn tilt_accelerates_the_top_in_the_tilted_direction() {
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
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
        let board = round1_board();
        // 着地後もしばらく同じ障害物の上にいても、入った瞬間ではないので飛び上がらない
        top.vel = (0.0, 0.0);
        for _ in 0..20 {
            assert_eq!(top.step(&board, STEP, &Tilt::new(), NO_G), None);
        }
    }

    #[test]
    fn entering_the_goal_cell_reports_goal() {
        let board = round1_board();
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
            goal: (BOARD_WIDTH - 1, 0),
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
