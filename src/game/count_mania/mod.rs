//! カウントメニア: ランダムに並んだ1〜Nの数字付き円を、1から順にクリックしていくマウス専用ゲーム。
//!
//! 1セッション=5ラウンド。難易度は選ばせず、ROUND1=初級・ROUND2=中級・ROUND3〜5=上級と上がっていく。
//! ROUND4・ROUND5は正解クリックのたびに近くの円が離れる方向へ散らばる(ROUND5はより広く・遠くへ)。
//! ラウンドごとに「全部押せたか(クリア)/ライフが尽きたか(失敗)」を
//! ScoreTrackerに1件として記録する。キー入力は受け付けない。
//! 各ラウンドの冒頭には「3.2.1.GO!!」を自前で挟む(ROUND2以降はCLEAR!の待ち時間の後)。

mod circle_image;
mod fish;
mod layout;
mod ripple;
mod wrong_mark;

use std::cell::{Cell, RefCell};
use std::time::Duration;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};
use crate::ui::countdown::{self, CountdownState, GoSeOnce};

use circle_image::{BoardCircle, BoardFish, CircleRenderer};
use fish::Fish;
use layout::{
    back_to_front, clamp_position, hit_test, layout_circles, numbers_stay_readable, scatter_offset,
    CircleSize, Placement, CELL_ASPECT,
};
use ripple::Ripple;
use wrong_mark::WrongMark;

pub const GAME_ID: &str = "count_mania";

/// 1セッションのラウンド数
pub const ROUNDS_PER_SESSION: u32 = 5;

/// 各ラウンドの難易度。難易度は選ばせず、ROUND1=初級・ROUND2=中級・ROUND3=上級と上がっていく。
/// ROUND4・ROUND5も上級のパラメータのまま、円が動く(ROUND_MOTIONS)
pub const ROUND_DIFFICULTIES: [Difficulty; ROUNDS_PER_SESSION as usize] = [
    Difficulty::Beginner,
    Difficulty::Intermediate,
    Difficulty::Advanced,
    Difficulty::Advanced,
    Difficulty::Advanced,
];

/// ラウンド中の円の動き方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    /// 動かない(配置したまま)
    Still,
    /// 正解クリックのたびに、クリック位置の近くの円がクリック位置から離れる方向へ動く
    Scatter,
}

/// 各ラウンドの円の動き方。ROUND4・ROUND5で拡散する(強さはCountManiaGame::round_scatter)
pub const ROUND_MOTIONS: [MotionKind; ROUNDS_PER_SESSION as usize] = [
    MotionKind::Still,
    MotionKind::Still,
    MotionKind::Still,
    MotionKind::Scatter,
    MotionKind::Scatter,
];

/// 円を散らす強さ
#[derive(Debug, Clone, Copy, PartialEq)]
struct ScatterStrength {
    /// クリック位置からこの見た目の距離(横のセル数)以内に中心がある円を散らす
    radius: f64,
    /// 散らす円を動かす見た目の距離(横のセル数。縦はこの半分の行数)
    distance: f64,
}

/// ROUND4の散らす強さ
const SCATTER_NORMAL: ScatterStrength = ScatterStrength {
    radius: 24.0,
    distance: 12.0,
};

/// ROUND5の散らす強さ。ROUND4より広い範囲の円を、1.5倍の距離まで弾き飛ばす
const SCATTER_STRONG: ScatterStrength = ScatterStrength {
    radius: 32.0,
    distance: 18.0,
};

/// 散らす円の動かし方の候補(離れる向きから回す角度[度], 距離の倍率)。先頭から試し、
/// 動かした先で残っている円の数字が隠れない最初の候補を使う(配置の時と同じく、重なりは許すが
/// 数字は隠さない。盤面の端に寄せられた同じ大きさの円がぴったり重なると、奥の円を押せなくなるため)。
/// どれも隠れるなら動かさない(元の位置はどの数字も隠していない)
const SCATTER_CANDIDATES: [(f64, f64); 8] = [
    (0.0, 1.0),
    (25.0, 1.0),
    (-25.0, 1.0),
    (0.0, 0.75),
    (25.0, 0.75),
    (-25.0, 0.75),
    (0.0, 0.5),
    (0.0, 0.25),
];

/// 盤面(水槽)を泳ぐ魚の数
const FISH_COUNT: usize = 3;

/// 魚の色(ネオンテトラの青・赤と、差し色の黄緑)。FISH_COUNT匹に順に割り当てる
const FISH_COLORS: [[u8; 3]; FISH_COUNT] = [[70, 190, 255], [255, 90, 90], [170, 235, 120]];

/// 円の無い所をクリックした時、この見た目の距離(横のセル数)以内にいる魚が驚いて逃げる
const SPOOK_RADIUS: f64 = 14.0;

/// 結果の記録に使う難易度。ラウンドごとに難易度が変わるため、最後のラウンドの上級を代表値にする
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;

/// ラウンド終了から次のラウンド開始までの間隔。この間はクリックを受け付けない
/// (前のラウンドの最後のクリックが次の盤面に当たらないようにするため)
pub const ROUND_INTERVAL: Duration = Duration::from_millis(1200);

/// 次に押すべき数字がこの時間を超えても押されないと、盤面の背景を赤く明滅させて焦らせる
pub const PRESSURE_THRESHOLD: Duration = Duration::from_secs(5);

/// 次に押すべき数字がこの時間押されないと、時間切れでGAME OVERにする
pub const TIMEOUT_LIMIT: Duration = Duration::from_secs(12);

/// 時間切れのこの時間前から、押すべき数字の円を点滅させて警告する
pub const TIMEOUT_WARNING: Duration = Duration::from_secs(3);

/// 警告中の円の点滅1回(強調→元の色)の周期。画像表示では点滅のたびに盤面の画像を作り直すため、
/// 作り直しが続きすぎない程度にゆっくりにする
pub const TIMEOUT_BLINK_PERIOD: Duration = Duration::from_millis(600);

/// 点滅で強調する時の円の色(白い円に黒い数字。どの円の色とも、赤く明滅する背景とも違う色にする)
const TIMEOUT_BLINK_COLOR: [u8; 3] = [255, 255, 255];

/// 次に押すべき数字になってからの経過時間に対して、その円をいま強調色で描くか。
/// 時間切れのTIMEOUT_WARNING前から、周期の前半は強調・後半は元の色にして点滅させる
fn target_blink_on(time_since_target: Duration) -> bool {
    let Some(over) = time_since_target.checked_sub(TIMEOUT_LIMIT - TIMEOUT_WARNING) else {
        return false;
    };
    let period = TIMEOUT_BLINK_PERIOD.as_millis().max(1);
    over.as_millis() % period < period / 2
}

/// 背景の明滅1回(暗い→明るい→暗い)の周期
pub const PRESSURE_PERIOD: Duration = Duration::from_millis(800);

/// 明滅する背景の赤の強さの範囲(暗い時, 明るい時)。
/// 明るい時でも、円の色・数字が背景に埋もれない暗さに抑える
const PRESSURE_RED_RANGE: (f64, f64) = (40.0, 150.0);

/// 背景の緑・青の強さ(赤に対する割合)。純粋な赤より少しだけ温かみを持たせる
const PRESSURE_GREEN_BLUE_RATIO: f64 = 0.12;

/// 次に押すべき数字になってからの経過時間に対する盤面の背景色。
/// 閾値以内はNone(通常の背景のまま)。閾値を超えたら、超えた分の時間で赤の明るさを
/// 周期的に変え、パトランプのように明滅させる。超えた直後は暗い側から始める
fn pressure_background(time_since_target: Duration) -> Option<Color> {
    let over = time_since_target.checked_sub(PRESSURE_THRESHOLD)?;
    if over.is_zero() {
        return None;
    }
    let phase = over.as_secs_f64() / PRESSURE_PERIOD.as_secs_f64() * std::f64::consts::TAU;
    // 0(暗い)〜1(明るい)を行き来する
    let level = (1.0 - phase.cos()) / 2.0;
    let (low, high) = PRESSURE_RED_RANGE;
    let red = low + (high - low) * level;
    let other = (red * PRESSURE_GREEN_BLUE_RATIO).round() as u8;
    Some(Color::Rgb(red.round() as u8, other, other))
}

/// 難易度ごとのパラメータ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DifficultyParams {
    /// 最大の数字(1〜Nを並べる)
    pub max_number: u8,
    /// 使う円のサイズ段階
    pub size_levels: &'static [CircleSize],
    /// 1ラウンドのライフ
    pub lives: u8,
    /// 円を密集させて配置するか
    pub dense: bool,
    /// 失敗したラウンドの記録に使う時間(ms)。そのラウンドの想定上限
    pub fail_latency_ms: f64,
}

pub fn params(difficulty: Difficulty) -> DifficultyParams {
    use CircleSize::{Huge, Large, Medium, Small};
    match difficulty {
        Difficulty::Beginner => DifficultyParams {
            max_number: 10,
            size_levels: &[Huge, Large, Medium],
            lives: 3,
            dense: false,
            fail_latency_ms: 30_000.0,
        },
        Difficulty::Intermediate => DifficultyParams {
            max_number: 14,
            size_levels: &[Huge, Large, Medium, Small],
            lives: 2,
            dense: false,
            fail_latency_ms: 45_000.0,
        },
        Difficulty::Advanced => DifficultyParams {
            max_number: 20,
            size_levels: &[Huge, Large, Medium, Small],
            lives: 2,
            dense: true,
            fail_latency_ms: 60_000.0,
        },
    }
}

/// ラウンド中の円1つ(番号・サイズ段階・色)。配置(位置)は描画エリアに依存するので別に持つ
#[derive(Debug, Clone, Copy)]
struct RoundCircle {
    number: u8,
    size: CircleSize,
    color: [u8; 3],
}

/// 1ラウンドの状態
struct Round {
    circles: Vec<RoundCircle>,
    /// 次に押すべき数字。これより小さい数字の円は押し終えて消えている
    next: u8,
    lives: u8,
    /// ラウンド開始からの経過時間
    elapsed: Duration,
    /// いまの数字が次に押すべき数字になってからの経過時間(プレッシャー背景に使う)
    time_since_target: Duration,
    /// 円が動くラウンド(ROUND4・ROUND5)の状態。動かないラウンドはNone
    motion: Option<Motion>,
}

/// 動くラウンドの円1つの現在位置。大きさは配置した時のまま変えない
#[derive(Debug, Clone, Copy, PartialEq)]
struct MovingCircle {
    number: u8,
    /// 左上のセル座標(盤面の絶対座標)。浮動小数点で持ち、描く時に丸める
    x: f64,
    y: f64,
    width: u16,
    height: u16,
}

impl MovingCircle {
    /// 描く位置(左上を丸めたセル)
    fn rect(&self) -> Rect {
        Rect::new(
            self.x.round() as u16,
            self.y.round() as u16,
            self.width,
            self.height,
        )
    }

    fn center(&self) -> (f64, f64) {
        (
            self.x + f64::from(self.width) / 2.0,
            self.y + f64::from(self.height) / 2.0,
        )
    }
}

/// 円が動くラウンド(ROUND4・ROUND5)専用の状態。円の現在位置は、最初の配置
/// (LayoutCache::base)から取り込み、以後はここで更新する
struct Motion {
    /// 現在位置を取り込んだ盤面。Noneならまだ配置から取り込んでいない
    board: Option<Rect>,
    /// 円ごとの現在位置。取り込んだ配置と同じ並び(描画順・当たり判定の優先順位を保つため)
    circles: Vec<MovingCircle>,
    /// 位置を更新した回数(位置の世代)。配置のキャッシュはこれが変わった時だけ置き直す
    generation: u32,
}

impl Motion {
    /// kindの動き方の状態を作る。動かないラウンドはNone
    fn new(kind: MotionKind) -> Option<Self> {
        match kind {
            MotionKind::Still => None,
            MotionKind::Scatter => Some(Self {
                board: None,
                circles: Vec::new(),
                generation: 0,
            }),
        }
    }

    /// boardの配置baseから位置を取り込んだ状態か(盤面・円の並び・大きさが同じか)
    fn follows(&self, board: Rect, base: &[Placement]) -> bool {
        self.board == Some(board)
            && self.circles.len() == base.len()
            && self.circles.iter().zip(base).all(|(circle, placement)| {
                circle.number == placement.number
                    && circle.width == placement.rect.width
                    && circle.height == placement.rect.height
            })
    }

    /// boardの配置baseの位置を、現在位置として取り込み直す
    fn adopt(&mut self, board: Rect, base: &[Placement]) {
        self.board = Some(board);
        self.circles = base
            .iter()
            .map(|placement| MovingCircle {
                number: placement.number,
                x: f64::from(placement.rect.x),
                y: f64::from(placement.rect.y),
                width: placement.rect.width,
                height: placement.rect.height,
            })
            .collect();
        self.generation += 1;
    }

    /// 配置baseを今の位置に置き直したもの。まだ取り込んでいない盤面・配置ならbaseのまま
    fn placements(&self, board: Rect, base: &[Placement]) -> Vec<Placement> {
        if !self.follows(board, base) {
            return base.to_vec();
        }
        self.circles
            .iter()
            .map(|circle| Placement {
                number: circle.number,
                rect: circle.rect(),
            })
            .collect()
    }

    /// ROUND4・ROUND5: クリック位置clickからstrength.radius以内にある、残っている円
    /// (is_remainingがtrueのもの)を、クリック位置から離れる方向へstrength.distanceだけ動かし、
    /// 盤面の中に収める。動かした先で残っている円の数字が隠れる時は、SCATTER_CANDIDATESの順に
    /// 向き・距離を変えて試す
    fn scatter(
        &mut self,
        rng: &mut impl Rng,
        click: (f64, f64),
        strength: ScatterStrength,
        is_remaining: impl Fn(u8) -> bool,
    ) {
        let Some(board) = self.board else {
            return;
        };
        // 円の中心がちょうどクリック位置にあって向きが決まらない時に使う向き
        let angle = rng.gen_range(0.0..std::f64::consts::TAU);
        let fallback = (angle.cos(), angle.sin());
        // 数字が隠れるかの判定に使う、今の位置の配置(動かすたびに更新する)
        let mut current: Vec<Placement> = self
            .circles
            .iter()
            .map(|circle| Placement {
                number: circle.number,
                rect: circle.rect(),
            })
            .collect();
        for (index, circle) in self.circles.iter_mut().enumerate() {
            if !is_remaining(circle.number) {
                continue;
            }
            let Some((dx, dy)) = scatter_offset(
                click,
                circle.center(),
                strength.radius,
                strength.distance,
                fallback,
            ) else {
                continue;
            };
            let destination = SCATTER_CANDIDATES.iter().find_map(|&(degrees, scale)| {
                let (ox, oy) = rotate_offset((dx, dy), degrees, scale);
                let (x, y) = clamp_position(
                    board,
                    circle.width,
                    circle.height,
                    circle.x + ox,
                    circle.y + oy,
                );
                let moved = MovingCircle { x, y, ..*circle };
                numbers_stay_readable(&current, index, moved.rect(), &is_remaining).then_some(moved)
            });
            if let Some(moved) = destination {
                *circle = moved;
                current[index].rect = moved.rect();
            }
        }
        self.generation += 1;
    }
}

/// 移動量(横のセル数, 縦の行数)を、見た目の向きでdegrees度回してscale倍したもの
fn rotate_offset((dx, dy): (f64, f64), degrees: f64, scale: f64) -> (f64, f64) {
    let (sin, cos) = degrees.to_radians().sin_cos();
    // 見た目の距離(縦は行数×CELL_ASPECT)にしてから回し、行数に戻す
    let (vx, vy) = (dx, dy * CELL_ASPECT);
    (
        (vx * cos - vy * sin) * scale,
        (vx * sin + vy * cos) * scale / CELL_ASPECT,
    )
}

fn new_round(rng: &mut impl Rng, params: &DifficultyParams) -> Round {
    // サイズ段階は順番に割り当ててからシャッフルし、どの段階も最低1つは使われるようにする
    let levels = params.size_levels;
    let mut sizes: Vec<CircleSize> = (0..params.max_number as usize)
        .map(|i| levels[i % levels.len()])
        .collect();
    sizes.shuffle(rng);
    let circles = (1..=params.max_number)
        .zip(sizes)
        .map(|(number, size)| RoundCircle {
            number,
            size,
            color: random_color(rng),
        })
        .collect();
    Round {
        circles,
        next: 1,
        lives: params.lives,
        elapsed: Duration::ZERO,
        time_since_target: Duration::ZERO,
        motion: None,
    }
}

/// 黒い数字が読みやすい、明るく鮮やかな色をランダムに作る(色相だけランダム、彩度・明度は固定幅)
fn random_color(rng: &mut impl Rng) -> [u8; 3] {
    let hue: f64 = rng.gen_range(0.0..360.0);
    let saturation: f64 = rng.gen_range(0.45..0.75);
    let value: f64 = rng.gen_range(0.85..1.0);
    hsv_to_rgb(hue, saturation, value)
}

/// HSV(色相0〜360, 彩度0〜1, 明度0〜1)をRGBに変換する
fn hsv_to_rgb(hue: f64, saturation: f64, value: f64) -> [u8; 3] {
    let chroma = value * saturation;
    let h = (hue.rem_euclid(360.0)) / 60.0;
    let x = chroma * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = value - chroma;
    let to_u8 = |c: f64| ((c + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [to_u8(r), to_u8(g), to_u8(b)]
}

/// 配置のキャッシュ。描画エリアかラウンドが変わった時だけ最初の配置(base)を作り直す。
/// 動くラウンド(ROUND4・ROUND5)で位置の世代(generation)が変わった時は、baseは作り直さず
/// 今の位置に置き直す(動かないラウンドは世代が0のまま変わらないので、そのまま使い回す)
struct LayoutCache {
    board: Rect,
    round_serial: u32,
    /// placementsを作った時の位置の世代
    generation: u32,
    /// layout_circlesで決めた最初の配置
    base: Vec<Placement>,
    /// 今の位置の配置(動かないラウンドはbaseと同じ)
    placements: Vec<Placement>,
}

pub struct CountManiaGame {
    tracker: ScoreTracker,
    round: Round,
    /// ラウンドの通し番号(0始まり)。いまのラウンドの難易度と、配置キャッシュの作り直し判定に使う。
    /// 次のラウンドが始まった時だけ進むので、ラウンドの記録直後(待ち時間中・GAME OVER後)も
    /// 記録したラウンドを指したままになる
    round_serial: u32,
    /// ラウンド間の待ち時間の残り。Noneならプレイ中(またはラウンド冒頭のカウントダウン中)
    interval: Option<Duration>,
    /// ラウンド冒頭の「3.2.1.GO!!」(ハヤウチ・べーと同じく、ラウンドごとに自前で持つ)。
    /// Someの間は円・魚を描かず、クリックも受け付けず、ラウンドの時間も数えない。
    /// ROUND1はゲーム開始時、ROUND2以降は待ち時間(CLEAR!)が終わって次の盤面を作った時に始まる
    countdown: Option<CountdownState>,
    feedback: AnswerFeedback,
    /// 正解クリックの位置から広がる波紋(見た目だけの演出)。Noneなら表示していない
    ripple: Option<Ripple>,
    /// 誤クリックの位置に出すバツ印(見た目だけの演出)。Noneなら表示していない
    wrong_mark: Option<WrongMark>,
    layout: RefCell<Option<LayoutCache>>,
    renderer: CircleRenderer,
    /// ライフが尽きた・時間切れ(GAME OVER)か。trueになったら残りラウンドを待たずセッションを終える
    game_over: bool,
    /// 盤面を水槽に見立てて泳ぐ魚(見た目だけの演出)。盤面の大きさが分かるまでは空
    fish: Vec<Fish>,
    /// 魚を放した盤面。盤面の大きさが変わったら放し直す
    fish_board: Option<Rect>,
    /// 直前に描いた盤面。update(dt)は描画エリアを受け取らないので、魚を泳がせる範囲をここから取る
    last_board: Cell<Option<Rect>>,
    /// カウントダウンの「GO!!」の音をROUND1だけ鳴らすためのゲート
    go_se: GoSeOnce,
}

/// ゲームの描画エリアのうち、円を並べるボード(枠の内側)。renderとhandle_mouseで共有する
fn board_area(area: Rect) -> Rect {
    let (_, body) = theme::split_hud(area);
    Block::default().borders(Borders::ALL).inner(body)
}

impl CountManiaGame {
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        let mut round = new_round(&mut rng, &params(ROUND_DIFFICULTIES[0]));
        round.motion = Motion::new(ROUND_MOTIONS[0]);
        let mut game = Self {
            tracker: ScoreTracker::new(),
            round,
            round_serial: 0,
            interval: None,
            countdown: None,
            feedback: AnswerFeedback::new(),
            ripple: None,
            wrong_mark: None,
            layout: RefCell::new(None),
            renderer: CircleRenderer::new(),
            game_over: false,
            fish: Vec::new(),
            fish_board: None,
            last_board: Cell::new(None),
            go_se: GoSeOnce::new(),
        };
        // ROUND1もカウントダウンから始める
        game.start_countdown();
        game
    }

    /// ラウンド冒頭のカウントダウンを始める
    fn start_countdown(&mut self) {
        let state = CountdownState::new();
        // 最初のフェーズ「3」の音
        if let Some(phase) = state.phase() {
            if let Some(se) = self.go_se.se_for(phase) {
                audio::play_se(se);
            }
        }
        self.countdown = Some(state);
    }

    /// boardに魚を放す。すでにboardに放してあれば何もしない
    fn place_fish(&mut self, board: Rect) {
        if self.fish_board == Some(board) {
            return;
        }
        let mut rng = rand::thread_rng();
        self.fish = (0..FISH_COUNT)
            .map(|_| Fish::random(&mut rng, board))
            .collect();
        self.fish_board = Some(board);
    }

    /// 直前に描いた盤面の中で魚を泳がせる。まだ描いていなければ何もしない
    fn swim_fish(&mut self, dt: Duration) {
        let Some(board) = self.last_board.get() else {
            return;
        };
        self.place_fish(board);
        let mut rng = rand::thread_rng();
        for fish in &mut self.fish {
            fish.update(dt, board, &mut rng);
        }
    }

    /// 円の無い所(column, row)がクリックされた。近くの魚をクリック位置から逃げさせる
    fn spook_fish(&mut self, board: Rect, column: u16, row: u16) {
        self.place_fish(board);
        let click = (f64::from(column) + 0.5, f64::from(row) + 0.5);
        for fish in &mut self.fish {
            let (x, y) = fish.center();
            if (x - click.0).hypot((y - click.1) * CELL_ASPECT) <= SPOOK_RADIUS {
                fish.spook(click);
            }
        }
    }

    /// 画像表示で円の描画器に渡す魚(テキスト表示と同じく丸めたセル位置)。
    /// テキスト表示の時・まだboardに魚を放していない時は空
    fn board_fish(&self, board: Rect) -> Vec<BoardFish> {
        if !self.renderer.uses_image() || self.fish_board != Some(board) {
            return Vec::new();
        }
        self.fish
            .iter()
            .zip(FISH_COLORS.iter().cycle())
            .map(|(fish, &color)| BoardFish {
                x: fish.x.round().max(0.0) as u16,
                y: fish.y.round().max(0.0) as u16,
                facing_right: fish.facing_right,
                color,
            })
            .collect()
    }

    /// 魚を文字で描く(画像プロトコル非対応の端末のみ)。円より先に描き、円の後ろを泳ぐようにする
    fn render_fish(&self, frame: &mut Frame, board: Rect) {
        // 画像表示では盤面全体が1枚の画像で覆われ、その下に描いた文字は見えないため、
        // 円の描画器が盤面の画像・パッチに魚を描く(board_fish)
        if self.renderer.uses_image() || self.fish_board != Some(board) {
            return;
        }
        let buffer = frame.buffer_mut();
        for (fish, color) in self.fish.iter().zip(FISH_COLORS.iter().cycle()) {
            fish::render_text(buffer, board, fish, *color);
        }
    }

    /// いまのラウンドの難易度(ROUND_DIFFICULTIESの並びに従う)
    fn round_difficulty(&self) -> Difficulty {
        let last = ROUND_DIFFICULTIES.len() - 1;
        ROUND_DIFFICULTIES[(self.round_serial as usize).min(last)]
    }

    /// いまのラウンドの難易度のパラメータ
    fn round_params(&self) -> DifficultyParams {
        params(self.round_difficulty())
    }

    /// いまのラウンドの円の動き方(ROUND_MOTIONSの並びに従う)
    fn round_motion(&self) -> MotionKind {
        let last = ROUND_MOTIONS.len() - 1;
        ROUND_MOTIONS[(self.round_serial as usize).min(last)]
    }

    /// 円の位置の世代。動かないラウンドは常に0
    fn position_generation(&self) -> u32 {
        self.round
            .motion
            .as_ref()
            .map_or(0, |motion| motion.generation)
    }

    /// boardに対する円の配置(キャッシュ済みならそれを返す)。
    /// 動くラウンドでは、円の今の位置を反映した配置を返す
    fn placements(&self, board: Rect) -> Vec<Placement> {
        let generation = self.position_generation();
        let mut cache = self.layout.borrow_mut();
        if let Some(cached) = cache.as_mut() {
            if cached.board == board && cached.round_serial == self.round_serial {
                if cached.generation != generation {
                    // 位置が更新された(ROUND4・ROUND5のみ)。最初の配置は作り直さず、今の位置に置き直す
                    cached.placements = self.moved_placements(board, &cached.base);
                    cached.generation = generation;
                }
                return cached.placements.clone();
            }
        }
        let circles: Vec<(u8, CircleSize)> = self
            .round
            .circles
            .iter()
            .map(|c| (c.number, c.size))
            .collect();
        let base = layout_circles(
            &mut rand::thread_rng(),
            board,
            &circles,
            self.round_params().dense,
        );
        let placements = self.moved_placements(board, &base);
        *cache = Some(LayoutCache {
            board,
            round_serial: self.round_serial,
            generation,
            base,
            placements: placements.clone(),
        });
        placements
    }

    /// 最初の配置baseを、円の今の位置に置き直したもの(動かないラウンドはbaseのまま)
    fn moved_placements(&self, board: Rect, base: &[Placement]) -> Vec<Placement> {
        match &self.round.motion {
            Some(motion) => motion.placements(board, base),
            None => base.to_vec(),
        }
    }

    /// 動くラウンドで、円の今の位置を配置のキャッシュに合わせる。まだ取り込んでいない
    /// (ラウンド最初の配置、盤面の大きさが変わって配置し直した)時は、その配置から取り込む。
    /// このラウンドの配置がまだ無い(描画もクリックもしていない)・動かないラウンドならfalse
    fn sync_motion(&mut self) -> bool {
        let round_serial = self.round_serial;
        let cache = self.layout.borrow();
        let Some(cached) = cache.as_ref().filter(|c| c.round_serial == round_serial) else {
            return false;
        };
        let Some(motion) = self.round.motion.as_mut() else {
            return false;
        };
        if !motion.follows(cached.board, &cached.base) {
            motion.adopt(cached.board, &cached.base);
        }
        true
    }

    /// いまのラウンドで円を散らす強さ。ROUND5(0始まりで4番目以降)はROUND4より強く散らす
    fn round_scatter(&self) -> ScatterStrength {
        if self.round_serial >= 4 {
            SCATTER_STRONG
        } else {
            SCATTER_NORMAL
        }
    }

    /// ROUND4・ROUND5: 正解クリックの位置(column, row)の近くにある、まだ残っている円を散らす。
    /// 押した円(いまの次に押すべき数字)自身は動かさない
    fn scatter_from(&mut self, column: u16, row: u16) {
        if self.round_motion() != MotionKind::Scatter || !self.sync_motion() {
            return;
        }
        let clicked = self.round.next;
        let max = self.round_params().max_number;
        let strength = self.round_scatter();
        let Some(motion) = self.round.motion.as_mut() else {
            return;
        };
        // クリックしたセルの中心から測る
        let click = (f64::from(column) + 0.5, f64::from(row) + 0.5);
        motion.scatter(&mut rand::thread_rng(), click, strength, |n| {
            n > clicked && n <= max
        });
    }

    /// 番号numberの円がまだ残っているか
    fn is_visible(&self, number: u8) -> bool {
        number >= self.round.next && number <= self.round_params().max_number
    }

    /// いま何ラウンド目か(1始まり。全ラウンド終了後は最終ラウンドのまま)
    fn current_round_number(&self) -> u32 {
        (self.tracker.total() + u32::from(!self.is_finished() && self.interval.is_none()))
            .clamp(1, ROUNDS_PER_SESSION)
    }

    /// 正解の円をクリックした。(column, row)はクリックしたセルで、そこから波紋を広げる
    fn click_correct(&mut self, column: u16, row: u16) {
        audio::play_se(SeKind::Correct);
        // 押した円(次に押すべき数字の円)の大きさに応じて、波紋の広がる大きさを変える
        let size = self
            .round
            .circles
            .iter()
            .find(|c| c.number == self.round.next)
            .map_or(CircleSize::Medium, |c| c.size);
        // 前の波紋が残っていても、新しい波紋に置き換える
        self.ripple = Some(Ripple::new(column, row, size));
        // 押し間違いの印は、正解に進んだら役目を終えるので消す(画像表示では波紋と同じパッチに
        // 重ねて描くため、離れた位置のバツ印が残ると波紋のパッチが大きくなり作り直しが重くなる)
        self.wrong_mark = None;
        // ROUND4はクリックした位置の近くの円を散らす(押した円はこの後で消える)
        self.scatter_from(column, row);
        self.round.next += 1;
        // 次の数字に進んだので、焦らせる背景は最初からやり直す
        self.round.time_since_target = Duration::ZERO;
        if self.round.next > self.round_params().max_number {
            let latency_ms = self.round.elapsed.as_secs_f64() * 1000.0;
            self.tracker.record(true, latency_ms);
            self.feedback.record(
                true,
                format!("CLEAR {:.1}秒", self.round.elapsed.as_secs_f64()),
            );
            self.finish_round();
        }
    }

    /// 押すべきでない円をクリックした。(column, row)はクリックしたセルで、そこにバツ印を出す
    fn click_wrong(&mut self, column: u16, row: u16) {
        audio::play_se(SeKind::Incorrect);
        // 前のバツ印が残っていても、新しい位置のバツ印に置き換える
        self.wrong_mark = Some(WrongMark::new(column, row));
        self.round.lives = self.round.lives.saturating_sub(1);
        if self.round.lives == 0 {
            // GAME OVERで焦らせる背景を止める
            self.round.time_since_target = Duration::ZERO;
            self.tracker
                .record(false, self.round_params().fail_latency_ms);
            self.feedback.record(false, "ライフが尽きた");
            // GAME OVERは残りラウンドを待たずセッションを即終了する(次のラウンドへの
            // 待ち時間には入らない。フィードバック表示が消えたらis_finished()がtrueになる)
            self.game_over = true;
        } else {
            self.feedback.record(false, "ライフ -1");
        }
    }

    /// 次に押すべき数字がTIMEOUT_LIMITのあいだ押されなかった。ライフ切れと同じくGAME OVERにする
    fn time_out(&mut self) {
        audio::play_se(SeKind::Incorrect);
        // GAME OVERで焦らせる背景・円の点滅を止める
        self.round.time_since_target = Duration::ZERO;
        self.tracker
            .record(false, self.round_params().fail_latency_ms);
        self.feedback.record(false, "時間切れ");
        self.game_over = true;
    }

    /// ラウンドを終える。最終ラウンドでなければ次のラウンドまでの待ち時間に入る
    fn finish_round(&mut self) {
        if !self.is_finished() {
            self.interval = Some(ROUND_INTERVAL);
        }
    }

    /// 次のラウンドを始める。ラウンドの番号を先に進め、そのラウンドの難易度で盤面を作り、
    /// カウントダウンに入る(GO!!が終わるまでは盤面を見せず、クリックも受け付けない)
    fn start_next_round(&mut self) {
        self.round_serial += 1;
        let mut rng = rand::thread_rng();
        self.round = new_round(&mut rng, &self.round_params());
        // ROUND4・ROUND5だけ円を動かす状態を持つ
        self.round.motion = Motion::new(self.round_motion());
        self.interval = None;
        self.start_countdown();
    }

    fn render_hud(&self, frame: &mut Frame, area: Rect) {
        let (difficulty_text, difficulty_color) = theme::difficulty_label(self.round_difficulty());
        let block = theme::panel(" ◆ カウントメニア ")
            .border_style(Style::default().fg(theme::flash_border_color(self.feedback.current())))
            .title(
                Line::from(Span::styled(
                    format!(" {difficulty_text} "),
                    Style::default()
                        .fg(difficulty_color)
                        .add_modifier(Modifier::BOLD),
                ))
                .right_aligned(),
            );
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(20),
                Constraint::Fill(1),
                Constraint::Length(24),
            ])
            .split(inner);

        let progress = Line::from(vec![
            Span::styled(
                format!(
                    " ROUND {}/{ROUNDS_PER_SESSION} ",
                    self.current_round_number()
                ),
                theme::title_style(),
            ),
            Span::styled(
                theme::progress_bar(
                    self.tracker.total(),
                    ROUNDS_PER_SESSION,
                    ROUNDS_PER_SESSION as usize,
                ),
                Style::default().fg(theme::ACCENT),
            ),
        ]);
        frame.render_widget(Paragraph::new(progress), cols[0]);

        // 中央: 正誤表示中はそれを、そうでなければ次に押す数字を出す
        // (待ち時間中・カウントダウン中は盤面に円が無いので出さない)
        let center = match self.feedback.current() {
            Some(flash) => theme::flash_line(flash),
            None if self.interval.is_none() && self.countdown.is_none() => Line::from(vec![
                Span::styled("つぎ ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    format!(" {} ", self.round.next),
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme::HIGHLIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" / {}", self.round_params().max_number),
                    Style::default().fg(theme::MUTED),
                ),
            ]),
            None => Line::from(""),
        };
        frame.render_widget(Paragraph::new(center).alignment(Alignment::Center), cols[1]);

        let lost = self.round_params().lives.saturating_sub(self.round.lives);
        let lives = Line::from(vec![
            Span::styled("マウス専用  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!(
                    "{}{}",
                    "♥".repeat(self.round.lives as usize),
                    "♡".repeat(lost as usize)
                ),
                Style::default()
                    .fg(theme::INCORRECT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]);
        frame.render_widget(Paragraph::new(lives).alignment(Alignment::Right), cols[2]);
    }

    /// ラウンド間の待ち時間中にボード中央へ出す案内
    ///
    /// ここに来るのは最後の数字まで押し切ってクリアした時のみ(ライフ切れはGAME OVERとして
    /// セッションを即終了するため、この待ち時間には入らない)
    fn render_interval_message(&self, frame: &mut Frame, board: Rect) {
        let next_round = (self.tracker.total() + 1).min(ROUNDS_PER_SESSION);
        let lines = vec![
            Line::from(Span::styled(
                "CLEAR!",
                Style::default()
                    .fg(theme::CORRECT)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("NEXT ROUND {next_round}/{ROUNDS_PER_SESSION}"),
                theme::title_style(),
            )),
        ];
        let area = theme::vertical_center(board, lines.len() as u16);
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
    }
}

impl Default for CountManiaGame {
    fn default() -> Self {
        Self::new()
    }
}

impl Game for CountManiaGame {
    /// マウス専用のため、キー入力では何もしない
    fn handle_key(&mut self, _key: KeyEvent) {}

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        // 待ち時間中・カウントダウン中・セッション終了後・GAME OVER後のクリックは受け付けない
        if self.is_finished()
            || self.interval.is_some()
            || self.countdown.is_some()
            || self.game_over
        {
            return;
        }
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let board = board_area(area);
        if !crate::game::contains(board, mouse.column, mouse.row) {
            return;
        }
        let placements = self.placements(board);
        let Some(number) = hit_test(&placements, |n| self.is_visible(n), mouse.column, mouse.row)
        else {
            // 円の無い場所(消えた円の跡も含む)のクリックはゲームを進めず、近くの魚を驚かすだけ
            self.spook_fish(board, mouse.column, mouse.row);
            return;
        };
        if number == self.round.next {
            self.click_correct(mouse.column, mouse.row);
        } else {
            self.click_wrong(mouse.column, mouse.row);
        }
    }

    fn update(&mut self, dt: Duration) {
        self.feedback.tick(dt);
        // 魚はゲームの進行と関係なく、待ち時間中・GAME OVER後も泳ぎ続ける
        self.swim_fish(dt);
        // 波紋はラウンド間の待ち時間中も時間を進め、持続時間を過ぎたら消す
        self.ripple = self.ripple.and_then(|ripple| ripple.advanced(dt));
        // バツ印も同じく時間で消す
        self.wrong_mark = self.wrong_mark.and_then(|mark| mark.advanced(dt));
        // カウントダウン中はラウンドの時間を数えない。GO!!が終わったらプレイに入る
        if let Some(state) = self.countdown.as_mut() {
            if let Some(phase) = state.tick(dt) {
                if let Some(se) = self.go_se.se_for(phase) {
                    audio::play_se(se);
                }
            }
            if state.is_finished() {
                self.countdown = None;
            }
            return;
        }
        if let Some(remaining) = self.interval {
            let remaining = remaining.saturating_sub(dt);
            if remaining.is_zero() {
                self.start_next_round();
            } else {
                self.interval = Some(remaining);
            }
            return;
        }
        if !self.is_finished() {
            self.round.elapsed += dt;
            self.round.time_since_target += dt;
            // GAME OVERのフィードバック表示中は、もう時間切れにしない(記録は1回だけ)
            if !self.game_over && self.round.time_since_target >= TIMEOUT_LIMIT {
                self.time_out();
            }
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud, body) = theme::split_hud(area);
        self.render_hud(frame, hud);

        let mut block = theme::focus_panel(" 1から順にクリック ", self.feedback.current());
        // 次の数字がなかなか押されない時は、パネルの背景を赤く明滅させて焦らせる
        if let Some(bg) = pressure_background(self.round.time_since_target) {
            block = block.style(Style::default().bg(bg));
        }
        let board = block.inner(body);
        frame.render_widget(block, body);
        if board.is_empty() {
            return;
        }
        self.last_board.set(Some(board));
        // カウントダウン中は円・魚を描かず、盤面に「3.2.1.GO!!」だけを大きく出す
        if let Some(state) = &self.countdown {
            countdown::render(frame, board, state);
            return;
        }
        self.render_fish(frame, board);
        if self.interval.is_some() {
            self.render_interval_message(frame, board);
            return;
        }
        // 時間切れが近い時は、押すべき数字の円を強調色と元の色で点滅させる
        let blink = target_blink_on(self.round.time_since_target);
        // 大きい円から先に描き、小さい円ほど手前に重ねる(当たり判定hit_testの優先順位と同じ)
        let circles: Vec<BoardCircle> = back_to_front(&self.placements(board))
            .iter()
            .filter(|placement| self.is_visible(placement.number))
            .filter_map(|placement| {
                self.round
                    .circles
                    .iter()
                    .find(|c| c.number == placement.number)
                    .map(|circle| BoardCircle {
                        rect: placement.rect,
                        number: circle.number,
                        color: if blink && circle.number == self.round.next {
                            TIMEOUT_BLINK_COLOR
                        } else {
                            circle.color
                        },
                    })
            })
            .collect();
        self.renderer.render_board_with_fish(
            frame,
            board,
            &circles,
            self.ripple.as_ref(),
            self.wrong_mark.as_ref(),
            &self.board_fish(board),
        );
    }

    fn is_finished(&self) -> bool {
        if self.game_over {
            // GAME OVERの正誤フィードバック("ライフが尽きた"/"時間切れ")が消えたらセッション終了
            return self.feedback.current().is_none();
        }
        self.tracker.total() >= ROUNDS_PER_SESSION
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::countdown::{self as countdown_ui, PHASE_DURATION};
    use crossterm::event::{KeyCode, KeyModifiers};
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use std::collections::HashSet;

    const AREA: Rect = Rect::new(0, 0, 100, 36);

    /// ラウンド冒頭のカウントダウン全体の長さ(3/2/1/GO!!の4フェーズ)
    const COUNTDOWN_TOTAL: Duration = Duration::from_millis(2400);

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn is_countdown(game: &CountManiaGame) -> bool {
        game.countdown.is_some()
    }

    fn countdown_phase(game: &CountManiaGame) -> Option<countdown_ui::Phase> {
        game.countdown
            .as_ref()
            .expect("カウントダウン中のはず")
            .phase()
    }

    /// ラウンド冒頭のカウントダウンを最後まで進め、プレイ中にする。フェーズ(PHASE_DURATION)
    /// ごとに分けて進める(実機のフレームループと同じく、一度に全部進めるとGO!!への遷移自体を
    /// 検出できずSEが鳴らないため)
    fn finish_countdown(game: &mut CountManiaGame) {
        assert!(is_countdown(game), "カウントダウン中のはず");
        for _ in 0..(COUNTDOWN_TOTAL.as_millis() / PHASE_DURATION.as_millis()) {
            game.update(PHASE_DURATION);
        }
        assert!(!is_countdown(game), "カウントダウンが終わったらプレイ中");
    }

    /// ROUND1冒頭のカウントダウンを終えて、すぐクリックできる状態のゲーム
    fn playing_game() -> CountManiaGame {
        let mut game = CountManiaGame::new();
        finish_countdown(&mut game);
        game
    }

    /// ラウンド間の待ち時間を終え、次のラウンドがあればその冒頭のカウントダウンも終える
    fn pass_interval(game: &mut CountManiaGame) {
        game.update(ROUND_INTERVAL);
        if !game.is_finished() {
            finish_countdown(game);
        }
    }

    /// 番号numberの円の中心座標
    fn center_of(game: &CountManiaGame, number: u8) -> (u16, u16) {
        let placements = game.placements(board_area(AREA));
        let rect = placements
            .iter()
            .find(|p| p.number == number)
            .unwrap_or_else(|| panic!("円{number}が配置されていること"))
            .rect;
        (rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    /// 番号numberの円をクリックする(実際の当たり判定を通す)
    fn click_circle(game: &mut CountManiaGame, number: u8) {
        let (column, row) = clickable_cell(game, number);
        game.handle_mouse(left_click(column, row), AREA);
    }

    /// 番号numberの円に当たるセル。まず中心を試し、当たらなければ円の矩形の中を探す。
    /// 円が散らばるラウンド(ROUND4・ROUND5)では、動いた手前の円が中心に重なることがあるため。
    /// どこも当たらない(消えた円など)なら中心を返す
    fn clickable_cell(game: &CountManiaGame, number: u8) -> (u16, u16) {
        let center = center_of(game, number);
        let placements = game.placements(board_area(AREA));
        let rect = placements
            .iter()
            .find(|p| p.number == number)
            .expect("配置されていること")
            .rect;
        let hits = |&(x, y): &(u16, u16)| {
            hit_test(&placements, |n| game.is_visible(n), x, y) == Some(number)
        };
        std::iter::once(center)
            .chain(
                (rect.y..rect.bottom()).flat_map(|y| (rect.x..rect.right()).map(move |x| (x, y))),
            )
            .find(hits)
            .unwrap_or(center)
    }

    /// どの円にも当たらないセルを探す
    fn empty_cell(game: &CountManiaGame) -> (u16, u16) {
        let board = board_area(AREA);
        let placements = game.placements(board);
        (board.y..board.bottom())
            .flat_map(|y| (board.x..board.right()).map(move |x| (x, y)))
            .find(|&(x, y)| hit_test(&placements, |_| true, x, y).is_none())
            .expect("円の無いセルがあること")
    }

    /// 配置を差し替える(乱数に頼らず、重なり具合を決めて確かめるため)。
    /// rectsはボード左上からの相対位置で(番号, x, y, 幅, 高さ)
    fn set_layout(game: &CountManiaGame, rects: &[(u8, u16, u16, u16, u16)]) {
        let board = board_area(AREA);
        let placements = rects
            .iter()
            .map(|&(number, x, y, width, height)| Placement {
                number,
                rect: Rect::new(board.x + x, board.y + y, width, height),
            })
            .collect::<Vec<_>>();
        *game.layout.borrow_mut() = Some(LayoutCache {
            board,
            round_serial: game.round_serial,
            generation: game.position_generation(),
            base: placements.clone(),
            placements,
        });
    }

    /// ボード左上からの相対位置(x, y)を左クリックする
    fn click_board(game: &mut CountManiaGame, x: u16, y: u16) {
        let board = board_area(AREA);
        game.handle_mouse(left_click(board.x + x, board.y + y), AREA);
    }

    /// 大きい円(14x7)の左側に小さい円(6x3)が重なった配置。OVERLAPは両方の円の内側のセル
    const LARGE_AT: (u16, u16, u16, u16) = (10, 5, 14, 7);
    const SMALL_AT: (u16, u16, u16, u16) = (10, 7, 6, 3);
    const OVERLAP: (u16, u16) = (12, 8);

    fn set_overlap_layout(game: &CountManiaGame, large: u8, small: u8) {
        let (lx, ly, lw, lh) = LARGE_AT;
        let (sx, sy, sw, sh) = SMALL_AT;
        // 小さい円を先に渡しても、描画・当たり判定は大きさで決まる
        set_layout(game, &[(small, sx, sy, sw, sh), (large, lx, ly, lw, lh)]);
    }

    fn clear_round(game: &mut CountManiaGame) {
        for number in 1..=game.round_params().max_number {
            click_circle(game, number);
        }
    }

    /// 0始まりでindex番目のラウンドまで、前のラウンドを全部クリアして進める
    fn advance_to_round(game: &mut CountManiaGame, index: u32) {
        for _ in 0..index {
            clear_round(game);
            pass_interval(game);
        }
        assert_eq!(
            game.round_serial,
            index,
            "ROUND{}が始まっていること",
            index + 1
        );
        assert!(game.interval.is_none());
    }

    /// いまのラウンドのライフを全部失うまで押し間違える
    fn lose_all_lives(game: &mut CountManiaGame) {
        for _ in 0..game.round.lives {
            let wrong = wrong_number(game);
            click_circle(game, wrong);
        }
    }

    /// 次に押すべきでない(まだ残っている)円の番号
    fn wrong_number(game: &CountManiaGame) -> u8 {
        game.round.next + 1
    }

    /// 画面を描画し、全セルを空白抜きの1つの文字列にして返す
    /// (全角文字の2セル目は空白で埋まるため、空白を除いて比較する)
    fn rendered_text(game: &CountManiaGame, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace(' ', "")
    }

    // --- 難易度パラメータ ---

    #[test]
    fn params_match_spec_for_each_difficulty() {
        use CircleSize::*;
        let beginner = params(Difficulty::Beginner);
        assert_eq!(beginner.max_number, 10);
        assert_eq!(beginner.size_levels, &[Huge, Large, Medium]);
        assert_eq!(beginner.lives, 3);
        assert!(!beginner.dense);

        let intermediate = params(Difficulty::Intermediate);
        assert_eq!(intermediate.max_number, 14);
        assert_eq!(intermediate.size_levels, &[Huge, Large, Medium, Small]);
        assert_eq!(intermediate.lives, 2);
        assert!(!intermediate.dense);

        let advanced = params(Difficulty::Advanced);
        assert_eq!(advanced.max_number, 20);
        assert_eq!(advanced.size_levels, &[Huge, Large, Medium, Small]);
        assert_eq!(advanced.lives, 2);
        assert!(advanced.dense, "上級は密集配置");
    }

    #[test]
    fn every_difficulty_uses_huge_circles() {
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            assert!(
                params(difficulty).size_levels.contains(&CircleSize::Huge),
                "{difficulty:?}: 特大の円を使う"
            );
        }
    }

    #[test]
    fn fail_latency_is_positive_and_grows_with_difficulty() {
        let b = params(Difficulty::Beginner).fail_latency_ms;
        let i = params(Difficulty::Intermediate).fail_latency_ms;
        let a = params(Difficulty::Advanced).fail_latency_ms;
        assert!(b > 0.0 && b <= i && i <= a);
    }

    #[test]
    fn new_round_has_numbers_one_to_n_using_only_allowed_sizes() {
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let p = params(difficulty);
            let mut rng = StdRng::seed_from_u64(1);
            let round = new_round(&mut rng, &p);
            let mut numbers: Vec<u8> = round.circles.iter().map(|c| c.number).collect();
            numbers.sort();
            assert_eq!(numbers, (1..=p.max_number).collect::<Vec<_>>());
            let used: HashSet<CircleSize> = round.circles.iter().map(|c| c.size).collect();
            let allowed: HashSet<CircleSize> = p.size_levels.iter().copied().collect();
            assert_eq!(
                used, allowed,
                "{difficulty:?}: 決められたサイズ段階をすべて使う"
            );
            assert_eq!(round.next, 1);
            assert_eq!(round.lives, p.lives);
        }
    }

    #[test]
    fn hsv_to_rgb_converts_primary_hues() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [255, 0, 0]);
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), [0, 255, 0]);
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), [0, 0, 255]);
        assert_eq!(hsv_to_rgb(0.0, 0.0, 1.0), [255, 255, 255]);
    }

    #[test]
    fn random_colors_are_bright_enough_for_black_digits() {
        let mut rng = StdRng::seed_from_u64(5);
        for _ in 0..200 {
            let [r, g, b] = random_color(&mut rng);
            assert!(
                r.max(g).max(b) >= 200,
                "最も明るいチャンネルが十分明るい: {r},{g},{b}"
            );
        }
    }

    #[test]
    fn new_game_starts_with_full_lives_and_next_number_one() {
        // ROUND1は初級
        let game = playing_game();
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 3);
        assert_eq!(game.round.circles.len(), 10);
        assert_eq!(game.tracker.total(), 0);
        assert!(!game.is_finished());
    }

    // --- 正解・クリア ---

    #[test]
    fn clicking_next_number_removes_it_and_advances() {
        let mut game = playing_game();
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 2);
        assert!(!game.is_visible(1), "押した円は消える");
        assert!(game.is_visible(2));
        assert_eq!(game.round.lives, 3, "正解ではライフは減らない");
        assert_eq!(game.tracker.total(), 0, "ラウンド途中は記録しない");
    }

    #[test]
    fn clearing_all_numbers_records_success_with_elapsed_time() {
        let mut game = playing_game();
        game.update(Duration::from_millis(1500));
        clear_round(&mut game);
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
        assert!((result.avg_latency_ms - 1500.0).abs() < 1e-9);
    }

    // --- 不正解・失敗 ---

    #[test]
    fn clicking_wrong_number_loses_a_life_and_keeps_next() {
        let mut game = playing_game();
        let wrong = wrong_number(&game);
        click_circle(&mut game, wrong);
        assert_eq!(game.round.lives, 2);
        assert_eq!(game.round.next, 1, "次に押す数字は変わらない");
        assert!(game.is_visible(wrong), "不正解の円は消えない");
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn losing_all_lives_ends_round_as_failure_with_limit_latency() {
        let mut game = playing_game();
        click_circle(&mut game, 1);
        lose_all_lives(&mut game);
        let result = game.result();
        assert_eq!(result.total, 1, "ライフ0で即ラウンド終了");
        assert_eq!(result.correct, 0);
        assert_eq!(
            result.avg_latency_ms,
            params(Difficulty::Beginner).fail_latency_ms,
            "ROUND1(初級)の想定上限で記録する"
        );
    }

    #[test]
    fn game_over_ends_the_session_without_waiting_for_remaining_rounds() {
        // GAME OVER(ライフ0)は、5ラウンド構成の途中でも次のラウンドへ進まず、
        // フィードバック("ライフが尽きた")が消えたらセッション全体が終了する
        let mut game = playing_game();
        click_circle(&mut game, 1);
        lose_all_lives(&mut game);
        assert_eq!(
            game.tracker.total(),
            1,
            "5ラウンドのうち1回しか記録されない"
        );
        assert!(!game.is_finished(), "フィードバック表示中はまだ終了しない");
        assert!(
            game.interval.is_none(),
            "次のラウンドへの待ち時間には入らない"
        );

        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.is_finished(), "フィードバックが消えたらセッション終了");
        assert_eq!(
            game.tracker.total(),
            1,
            "次のラウンドは始まらないので記録は1回のまま"
        );
    }

    #[test]
    fn clicks_after_game_over_are_ignored() {
        let mut game = playing_game();
        click_circle(&mut game, 1);
        lose_all_lives(&mut game);
        let next_before = game.round.next;
        click_circle(&mut game, next_before);
        assert_eq!(game.tracker.total(), 1, "GAME OVER後のクリックは無視される");
    }

    // --- ラウンド進行 ---

    #[test]
    fn clicks_are_ignored_during_interval_then_next_round_starts() {
        let mut game = playing_game();
        clear_round(&mut game);
        assert!(game.interval.is_some(), "ラウンド終了後は待ち時間に入る");
        let before = game.round.next;
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, before, "待ち時間中のクリックは無視される");
        assert_eq!(game.tracker.total(), 1);

        pass_interval(&mut game);
        assert!(game.interval.is_none());
        assert_eq!(game.round.next, 1, "新しいラウンドは1から");
        assert_eq!(
            game.round.lives,
            params(Difficulty::Intermediate).lives,
            "ライフもROUND2(中級)の数で満タンに戻る"
        );
        assert!(game.round.elapsed.is_zero(), "経過時間も0から");
    }

    #[test]
    fn session_finishes_after_five_rounds() {
        let mut game = playing_game();
        for round in 0..ROUNDS_PER_SESSION {
            assert!(
                !game.is_finished(),
                "ラウンド{round}開始時点では終わっていない"
            );
            clear_round(&mut game);
            if round + 1 < ROUNDS_PER_SESSION {
                pass_interval(&mut game);
            }
        }
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(
            result.difficulty, SESSION_DIFFICULTY,
            "記録の難易度は代表値の上級"
        );
        assert_eq!(SESSION_DIFFICULTY, Difficulty::Advanced);
        assert_eq!(result.total, ROUNDS_PER_SESSION);
        assert_eq!(result.correct, ROUNDS_PER_SESSION);
    }

    #[test]
    fn clicks_after_session_finished_are_ignored() {
        let mut game = playing_game();
        for _ in 0..ROUNDS_PER_SESSION {
            clear_round(&mut game);
            pass_interval(&mut game);
        }
        assert!(game.is_finished());
        let (column, row) = center_of(&game, 1);
        game.handle_mouse(left_click(column, row), AREA);
        assert_eq!(game.result().total, ROUNDS_PER_SESSION);
    }

    // --- ラウンド冒頭のカウントダウン(3.2.1.GO!!) ---

    #[test]
    fn countdown_total_matches_four_phases() {
        assert_eq!(PHASE_DURATION * 4, COUNTDOWN_TOTAL);
    }

    #[test]
    fn new_game_starts_round1_with_countdown_from_three() {
        let game = CountManiaGame::new();
        assert!(is_countdown(&game), "ROUND1はカウントダウンから");
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Three));
        assert_eq!(
            game.round_serial, 0,
            "カウントダウン中もROUND1の盤面を用意している"
        );
        assert!(game.interval.is_none());
        assert!(!game.is_finished());
    }

    #[test]
    fn countdown_advances_3_2_1_go_and_ends_after_total() {
        let mut game = CountManiaGame::new();
        game.update(PHASE_DURATION);
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Two));
        game.update(PHASE_DURATION);
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::One));
        game.update(PHASE_DURATION);
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Go));
        game.update(PHASE_DURATION - Duration::from_millis(1));
        assert!(
            is_countdown(&game),
            "GO!!の表示時間が終わるまではカウントダウン中"
        );
        game.update(Duration::from_millis(1));
        assert!(!is_countdown(&game), "GO!!が終わったらプレイ中");
    }

    #[test]
    fn clicks_during_countdown_are_ignored() {
        let mut game = CountManiaGame::new();
        // 正解の円・押し間違いの円・円の無い所をどれもクリックする
        click_circle(&mut game, 1);
        let wrong = wrong_number(&game);
        click_circle(&mut game, wrong);
        let (column, row) = empty_cell(&game);
        game.handle_mouse(left_click(column, row), AREA);
        assert!(
            is_countdown(&game),
            "クリックでカウントダウンが飛ばされない"
        );
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Three));
        assert_eq!(game.round.next, 1, "円は消えない");
        assert_eq!(game.round.lives, params(Difficulty::Beginner).lives);
        assert_eq!(game.tracker.total(), 0);
        assert!(game.ripple.is_none(), "波紋も出ない");
        assert!(game.wrong_mark.is_none(), "バツ印も出ない");
        assert!(game.feedback.current().is_none());
    }

    #[test]
    fn countdown_finishes_into_playable_round1() {
        let mut game = CountManiaGame::new();
        finish_countdown(&mut game);
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 2, "カウントダウン後はクリックを受け付ける");
    }

    #[test]
    fn round_time_does_not_advance_during_countdown() {
        let mut game = CountManiaGame::new();
        game.update(COUNTDOWN_TOTAL - Duration::from_millis(1));
        assert!(
            game.round.elapsed.is_zero(),
            "カウントダウン中はラウンドの時間を数えない"
        );
        assert!(game.round.time_since_target.is_zero());
        assert!(pressure_background(game.round.time_since_target).is_none());
    }

    #[test]
    fn countdown_does_not_count_toward_time_out() {
        // カウントダウン中に大きく時間が進んでも、時間切れにはならない
        let mut game = CountManiaGame::new();
        game.update(TIMEOUT_LIMIT);
        assert!(!game.game_over);
        assert!(!is_countdown(&game));
        assert!(game.round.time_since_target.is_zero());
    }

    #[test]
    fn recorded_round_time_excludes_countdown() {
        let mut game = CountManiaGame::new();
        game.update(COUNTDOWN_TOTAL);
        game.update(Duration::from_millis(1500));
        clear_round(&mut game);
        let result = game.result();
        assert!(
            (result.avg_latency_ms - 1500.0).abs() < 1e-9,
            "記録はGO!!の後から: {}",
            result.avg_latency_ms
        );
    }

    #[test]
    fn next_round_countdown_starts_after_clear_interval() {
        let mut game = playing_game();
        clear_round(&mut game);
        assert!(game.interval.is_some());
        assert!(
            !is_countdown(&game),
            "CLEAR!の待ち時間中はまだカウントダウンしない"
        );
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("CLEAR!"), "CLEAR!の表示は今まで通り");

        game.update(ROUND_INTERVAL);
        assert!(game.interval.is_none());
        assert!(is_countdown(&game), "CLEAR!の後に3.2.1.GO!!が入る");
        assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Three));
        assert_eq!(game.round_serial, 1, "ROUND2の盤面はカウントダウン前に作る");
        assert_eq!(game.round_difficulty(), Difficulty::Intermediate);

        click_circle(&mut game, 1);
        assert_eq!(
            game.round.next, 1,
            "ROUND2のカウントダウン中もクリックは無視"
        );
        finish_countdown(&mut game);
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 2);
    }

    #[test]
    fn every_round_from_1_to_5_starts_with_countdown() {
        let mut game = CountManiaGame::new();
        for round in 0..ROUNDS_PER_SESSION {
            assert!(
                is_countdown(&game),
                "ROUND{}はカウントダウンから",
                round + 1
            );
            assert_eq!(countdown_phase(&game), Some(countdown_ui::Phase::Three));
            assert_eq!(game.round_serial, round);
            let text = rendered_text(&game, AREA.width, AREA.height);
            assert!(
                text.contains(&format!("ROUND{}/{ROUNDS_PER_SESSION}", round + 1)),
                "カウントダウン中もHUDは始まるラウンドの番号"
            );
            finish_countdown(&mut game);
            clear_round(&mut game);
            game.update(ROUND_INTERVAL);
        }
        assert!(game.is_finished());
        assert!(
            !is_countdown(&game),
            "最終ラウンドの後にはカウントダウンしない"
        );
    }

    #[test]
    fn game_over_does_not_start_countdown() {
        let mut game = playing_game();
        lose_all_lives(&mut game);
        assert!(game.game_over);
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.is_finished());
        assert!(!is_countdown(&game));
    }

    #[test]
    fn countdown_renders_big_glyph_and_hides_circles() {
        let game = CountManiaGame::new();
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains('█'), "カウントダウンの大きな文字を描く");
        for number in 1..=params(Difficulty::Beginner).max_number {
            let digit = circle_image::circled_digit(number).unwrap();
            assert!(
                !text.contains(digit),
                "カウントダウン中は円{digit}を描かない"
            );
        }
        assert!(
            !text.contains("つぎ"),
            "カウントダウン中は次の数字の案内を出さない"
        );
        assert!(text.contains("ROUND1/5"));
        assert!(text.contains("マウス専用"));
    }

    #[test]
    fn countdown_hides_fish() {
        let mut game = CountManiaGame::new();
        let (column, row) = empty_cell(&game);
        put_all_fish_at(&mut game, column, row);
        for fish in &mut game.fish {
            fish.facing_right = true;
        }
        let buffer = rendered_buffer(&game);
        let fish_colors: Vec<Color> = FISH_COLORS
            .iter()
            .map(|&[r, g, b]| Color::Rgb(r, g, b))
            .collect();
        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| !fish_colors.contains(&cell.fg)),
            "カウントダウン中は魚を描かない"
        );

        // カウントダウンが終わったら魚が見える
        finish_countdown(&mut game);
        put_all_fish_at(&mut game, column, row);
        for fish in &mut game.fish {
            fish.facing_right = true;
        }
        let buffer = rendered_buffer(&game);
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| fish_colors.contains(&cell.fg)),
            "プレイ中は魚を描く"
        );
    }

    #[test]
    fn image_mode_countdown_does_not_build_board_image() {
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut game = CountManiaGame::new();
        let mut picker = Picker::from_fontsize((4, 8));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        game.renderer = CircleRenderer::with_picker(picker);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains('█'));
        assert_eq!(
            game.renderer.board_encode_count(),
            0,
            "カウントダウン中は盤面の画像を作らない"
        );
        finish_countdown(&mut game);
        rendered_text(&game, AREA.width, AREA.height);
        assert!(
            game.renderer.board_encode_count() > 0,
            "プレイ中は盤面を描く"
        );
    }

    #[test]
    fn render_during_countdown_does_not_panic_on_tiny_areas() {
        let mut game = CountManiaGame::new();
        for (width, height) in [(1, 1), (5, 2), (20, 6), (AREA.width, AREA.height)] {
            rendered_text(&game, width, height);
            game.update(PHASE_DURATION);
        }
    }

    // --- マウス専用 ---

    #[test]
    fn handle_key_never_changes_state() {
        let mut game = playing_game();
        let keys = [
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Char(' '),
            KeyCode::Char('1'),
            KeyCode::Char('2'),
            KeyCode::Tab,
        ];
        for code in keys {
            game.handle_key(KeyEvent::from(code));
        }
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 3);
        assert_eq!(game.tracker.total(), 0);
        assert!(game.interval.is_none());
        assert!(game.feedback.current().is_none());
    }

    // --- クリック当たり判定 ---

    #[test]
    fn clicking_empty_space_does_nothing() {
        let mut game = playing_game();
        let (column, row) = empty_cell(&game);
        game.handle_mouse(left_click(column, row), AREA);
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 3);
    }

    #[test]
    fn clicking_outside_board_or_non_left_button_does_nothing() {
        let mut game = playing_game();
        // HUD(ボード外)のクリック
        game.handle_mouse(left_click(1, 1), AREA);
        // 1の円を右クリック
        let (column, row) = center_of(&game, 1);
        game.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            AREA,
        );
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 3);
    }

    #[test]
    fn clicking_overlap_hits_smaller_circle_then_circle_below_after_removal() {
        // 小さい円1が大きい円2の上に重なっている
        let mut game = playing_game();
        set_overlap_layout(&game, 2, 1);
        let (x, y) = OVERLAP;
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 2, "手前の小さい円1に当たる");
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 3, "円1が消えた後は下の円2に当たる");
        assert_eq!(game.round.lives, 3);
    }

    #[test]
    fn clicking_overlap_never_hits_larger_circle_below() {
        // 次に押すべき円1が大きい方で、その上に小さい円2が重なっている。重なった所は円2扱い
        let mut game = playing_game();
        set_overlap_layout(&game, 1, 2);
        let (x, y) = OVERLAP;
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 2, "手前の円2を押した扱いでライフが減る");
    }

    #[test]
    fn render_draws_smaller_circle_on_top_of_larger() {
        let mut game = playing_game();
        game.round.circles[0].color = [200, 50, 50];
        game.round.circles[1].color = [50, 50, 200];
        set_overlap_layout(&game, 2, 1);
        let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        let board = board_area(AREA);
        let (x, y) = OVERLAP;
        assert_eq!(
            terminal.backend().buffer()[(board.x + x, board.y + y)].fg,
            Color::Rgb(200, 50, 50),
            "重なった所は小さい円1の色"
        );
    }

    #[test]
    fn clicking_removed_circle_does_nothing() {
        let mut game = playing_game();
        // 円1の下に他の円が無い配置にする(下に円があればそちらに当たるのが正しい動作のため)
        set_layout(&game, &[(1, 2, 2, 10, 5), (2, 40, 2, 10, 5)]);
        click_circle(&mut game, 1);
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 2);
        assert_eq!(
            game.round.lives, 3,
            "消えた円の場所は空白扱いでライフは減らない"
        );
    }

    #[test]
    fn layout_is_kept_between_clicks_and_regenerated_for_new_round() {
        let mut game = playing_game();
        let board = board_area(AREA);
        let first = game.placements(board);
        click_circle(&mut game, 1);
        assert_eq!(
            game.placements(board),
            first,
            "ラウンド中は配置が変わらない"
        );
        for number in 2..=game.round_params().max_number {
            click_circle(&mut game, number);
        }
        pass_interval(&mut game);
        // 新しいラウンドでは色・サイズも振り直されるので、配置も作り直される
        assert_eq!(
            game.placements(board).len(),
            game.round_params().max_number as usize
        );
        assert_eq!(
            game.layout.borrow().as_ref().unwrap().round_serial,
            game.round_serial
        );
    }

    // --- 描画 ---

    #[test]
    fn hud_shows_mouse_only_next_number_and_lives() {
        let mut game = playing_game();
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("マウス専用"), "マウス専用と明記すること");
        assert!(text.contains("つぎ"));
        assert!(text.contains("♥♥♥"), "ライフ3");

        click_circle(&mut game, 1);
        let wrong = wrong_number(&game);
        click_circle(&mut game, wrong);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("♥♥♡"), "ライフが1減った表示");
    }

    #[test]
    fn board_shows_every_remaining_number_in_fallback_mode() {
        // ROUND3(上級)の20個の円
        let mut game = playing_game();
        advance_to_round(&mut game, 2);
        click_circle(&mut game, 1);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(!text.contains('①'), "押した円は描かれない");
        for number in 2..=20u8 {
            let digit = circle_image::circled_digit(number).unwrap();
            assert!(text.contains(digit), "{digit}が描かれていること");
        }
    }

    #[test]
    fn render_does_not_panic_in_tiny_or_interval_state() {
        let mut game = playing_game();
        advance_to_round(&mut game, 1);
        rendered_text(&game, 20, 6);
        rendered_text(&game, 1, 1);
        clear_round(&mut game);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(
            text.contains("ROUND"),
            "待ち時間中は次のラウンドの案内を出す"
        );
    }

    #[test]
    fn interval_message_tells_clear_and_hides_circles() {
        let mut game = playing_game();
        clear_round(&mut game);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("CLEAR!"));
        assert!(text.contains("NEXTROUND2/5"));
        assert!(!text.contains('②'), "待ち時間中は円を描かない");
    }

    #[test]
    fn elapsed_time_does_not_advance_during_interval() {
        let mut game = playing_game();
        clear_round(&mut game);
        game.update(ROUND_INTERVAL / 2);
        assert!(game.interval.is_some());
        game.update(ROUND_INTERVAL / 2);
        finish_countdown(&mut game);
        game.update(Duration::from_millis(700));
        clear_round(&mut game);
        let result = game.result();
        // 2ラウンド目の記録は、ラウンド開始後に進めた700msだけ
        assert!((result.avg_latency_ms - 350.0).abs() < 1e-9);
    }

    // --- 波紋 ---

    /// 円1・円2が重ならない配置にし、円1の内側のセル(ボード左上からの相対位置)を返す
    fn set_separate_layout(game: &CountManiaGame) -> (u16, u16) {
        set_layout(game, &[(1, 2, 2, 10, 5), (2, 40, 2, 10, 5)]);
        (6, 4)
    }

    #[test]
    fn correct_click_starts_ripple_at_clicked_cell() {
        let mut game = playing_game();
        let (x, y) = set_separate_layout(&game);
        assert!(game.ripple.is_none(), "始めは波紋なし");
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 2);
        let board = board_area(AREA);
        let ripple = game.ripple.expect("正解クリックで波紋が始まる");
        assert_eq!(
            ripple.center(),
            (board.x + x, board.y + y),
            "クリックした位置が中心"
        );
        assert_eq!(ripple.progress(), 0.0);
    }

    #[test]
    fn wrong_or_empty_click_does_not_start_ripple() {
        let mut game = playing_game();
        set_separate_layout(&game);
        // 円2(次に押すべきでない円)をクリック
        click_board(&mut game, 44, 4);
        assert_eq!(game.round.lives, 2, "不正解");
        assert!(game.ripple.is_none(), "不正解では波紋を出さない");
        // 円の無い所をクリック
        click_board(&mut game, 25, 4);
        assert!(game.ripple.is_none(), "空白のクリックでも波紋を出さない");
    }

    #[test]
    fn update_advances_ripple_and_removes_it_after_duration() {
        let mut game = playing_game();
        let (x, y) = set_separate_layout(&game);
        click_board(&mut game, x, y);
        game.update(ripple::RIPPLE_DURATION / 2);
        let ripple = game.ripple.expect("持続時間内は残る");
        assert!((ripple.progress() - 0.5).abs() < 1e-9, "経過時間が進む");
        game.update(ripple::RIPPLE_DURATION / 2);
        assert!(game.ripple.is_none(), "持続時間を過ぎたら消える");
    }

    #[test]
    fn new_correct_click_replaces_previous_ripple() {
        let mut game = playing_game();
        let (x, y) = set_separate_layout(&game);
        click_board(&mut game, x, y);
        game.update(ripple::RIPPLE_DURATION / 2);
        // 円2をクリック(正解)
        click_board(&mut game, 44, 4);
        let board = board_area(AREA);
        let ripple = game.ripple.unwrap();
        assert_eq!(
            ripple.center(),
            (board.x + 44, board.y + 4),
            "新しい位置に置き換わる"
        );
        assert_eq!(ripple.progress(), 0.0, "最初からやり直す");
    }

    #[test]
    fn ripple_expires_during_round_interval_without_affecting_score() {
        let mut game = playing_game();
        clear_round(&mut game);
        assert!(game.ripple.is_some(), "最後の正解クリックでも波紋は始まる");
        assert!(game.interval.is_some());
        game.update(ripple::RIPPLE_DURATION);
        assert!(game.ripple.is_none(), "待ち時間中も波紋の時間は進む");
        let result = game.result();
        assert_eq!((result.total, result.correct), (1, 1), "記録は波紋と無関係");
    }

    #[test]
    fn fallback_render_ignores_ripple() {
        let mut game = playing_game();
        let (x, y) = set_separate_layout(&game);
        click_board(&mut game, x, y);
        assert!(game.ripple.is_some());
        let with_ripple = rendered_text(&game, AREA.width, AREA.height);
        game.ripple = None;
        assert_eq!(rendered_text(&game, AREA.width, AREA.height), with_ripple);
    }

    #[test]
    fn image_mode_render_with_ripple_does_not_panic() {
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut game = playing_game();
        advance_to_round(&mut game, 2);
        let mut picker = Picker::from_fontsize((4, 8));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        game.renderer = CircleRenderer::with_picker(picker);
        click_circle(&mut game, 1);
        assert!(game.ripple.is_some());
        for _ in 0..4 {
            rendered_text(&game, AREA.width, AREA.height);
            game.update(ripple::RIPPLE_DURATION / 3);
        }
        assert!(game.ripple.is_none());
        rendered_text(&game, AREA.width, AREA.height);
        click_circle(&mut game, 2);
        rendered_text(&game, 20, 6);
        rendered_text(&game, 1, 1);
    }

    #[test]
    fn ripple_animation_does_not_reencode_whole_board_every_tick() {
        // 実際のゲームループと同じく、tickごとにupdateして描く
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut game = playing_game();
        advance_to_round(&mut game, 2);
        let mut picker = Picker::from_fontsize((4, 8));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        game.renderer = CircleRenderer::with_picker(picker);
        // 泳ぐ魚のパッチを数えないよう、魚のいない水槽にする
        game.fish_board = Some(board_area(AREA));
        game.fish.clear();
        rendered_text(&game, AREA.width, AREA.height);
        click_circle(&mut game, 1);
        rendered_text(&game, AREA.width, AREA.height);
        let boards = game.renderer.board_encode_count();
        let patches = game.renderer.ripple_encode_count();
        let mut ticks = 0;
        while game.ripple.is_some() {
            game.update(crate::TICK_RATE);
            rendered_text(&game, AREA.width, AREA.height);
            ticks += 1;
        }
        assert_eq!(
            game.renderer.board_encode_count(),
            boards,
            "波紋のアニメーション中は盤面全体を作り直さない"
        );
        let redrawn = game.renderer.ripple_encode_count() - patches;
        let frames = (ripple::RIPPLE_DURATION.as_millis()
            / ripple::RIPPLE_FRAME_INTERVAL.as_millis()) as usize;
        // コマの切り替わり(最初のコマは描画済み) + 消えた後のリング無しの描き直し1回
        assert!(
            redrawn <= frames,
            "パッチの作り直しはコマの数まで: {redrawn}"
        );
        assert!(redrawn < ticks, "毎tickは作り直さない: {redrawn}/{ticks}");
    }

    // --- プレッシャー背景 ---

    /// 画面を描画し、バッファを返す
    fn rendered_buffer(game: &CountManiaGame) -> ratatui::buffer::Buffer {
        let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// 盤面の円の無いセルの背景色
    fn board_background(game: &CountManiaGame) -> Color {
        let (column, row) = empty_cell(game);
        rendered_buffer(game)[(column, row)].bg
    }

    /// 閾値をsecs秒超えた時点の背景色
    fn background_after_threshold(secs: f64) -> Color {
        pressure_background(PRESSURE_THRESHOLD + Duration::from_secs_f64(secs))
            .expect("閾値を超えたら背景色が付く")
    }

    fn rgb(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("RGBの色であること: {other:?}"),
        }
    }

    #[test]
    fn new_round_starts_with_zero_time_since_target() {
        let mut rng = StdRng::seed_from_u64(1);
        let round = new_round(&mut rng, &params(Difficulty::Beginner));
        assert!(round.time_since_target.is_zero());
        let game = playing_game();
        assert!(game.round.time_since_target.is_zero());
    }

    #[test]
    fn update_advances_time_since_target_only_while_playing() {
        let mut game = playing_game();
        game.update(Duration::from_millis(1500));
        game.update(Duration::from_millis(500));
        assert_eq!(game.round.time_since_target, Duration::from_secs(2));

        clear_round(&mut game);
        assert!(game.interval.is_some());
        game.update(ROUND_INTERVAL / 2);
        assert!(
            game.round.time_since_target.is_zero(),
            "ラウンド間の待ち時間中は進まない"
        );
        game.update(ROUND_INTERVAL / 2);
        assert!(game.interval.is_none());
        assert!(
            game.round.time_since_target.is_zero(),
            "新しいラウンドは0から"
        );
    }

    #[test]
    fn time_since_target_does_not_advance_after_session_finished() {
        let mut game = playing_game();
        for _ in 0..ROUNDS_PER_SESSION {
            clear_round(&mut game);
            pass_interval(&mut game);
        }
        assert!(game.is_finished());
        game.update(PRESSURE_THRESHOLD * 2);
        assert!(game.round.time_since_target.is_zero());
    }

    #[test]
    fn correct_click_resets_time_since_target() {
        let mut game = playing_game();
        game.update(Duration::from_secs(8));
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 2);
        assert!(game.round.time_since_target.is_zero());
        assert_eq!(
            game.round.elapsed,
            Duration::from_secs(8),
            "ラウンドの経過時間はリセットしない"
        );
    }

    #[test]
    fn wrong_click_keeps_time_since_target_while_lives_remain() {
        let mut game = playing_game();
        game.update(Duration::from_secs(8));
        let wrong = wrong_number(&game);
        click_circle(&mut game, wrong);
        assert_eq!(game.round.lives, 2);
        assert_eq!(game.round.time_since_target, Duration::from_secs(8));
    }

    #[test]
    fn game_over_resets_time_since_target() {
        let mut game = playing_game();
        game.update(Duration::from_secs(8));
        lose_all_lives(&mut game);
        assert_eq!(game.round.lives, 0);
        assert!(game.round.time_since_target.is_zero());
    }

    #[test]
    fn pressure_background_is_none_up_to_threshold() {
        assert_eq!(pressure_background(Duration::ZERO), None);
        assert_eq!(pressure_background(Duration::from_millis(4_999)), None);
        assert_eq!(pressure_background(PRESSURE_THRESHOLD), None);
    }

    #[test]
    fn pressure_background_is_reddish_after_threshold() {
        for step in 1..=40 {
            let (r, g, b) = rgb(background_after_threshold(f64::from(step) * 0.05));
            assert!(r > g && r > b, "赤系であること: {r},{g},{b}");
            assert!(r >= 30, "背景が分かる程度の赤であること: {r}");
            assert!(
                r <= 180,
                "円・数字が読める程度に暗いこと(明るすぎない): {r}"
            );
        }
    }

    #[test]
    fn pressure_background_brightness_pulses_periodically() {
        let dark = rgb(background_after_threshold(0.001)).0;
        let bright = rgb(background_after_threshold(
            PRESSURE_PERIOD.as_secs_f64() / 2.0,
        ))
        .0;
        assert!(
            bright > dark + 50,
            "時間経過で明るさが変わる: {dark} -> {bright}"
        );
        let later = rgb(background_after_threshold(
            PRESSURE_PERIOD.as_secs_f64() * 3.0 + 0.001,
        ))
        .0;
        assert!(
            later.abs_diff(dark) <= 2,
            "周期ごとに同じ明るさに戻る: {dark} / {later}"
        );
    }

    #[test]
    fn board_background_stays_normal_within_threshold() {
        let mut game = playing_game();
        let normal = board_background(&game);
        assert_eq!(normal, Color::Reset);
        game.update(Duration::from_millis(4_900));
        assert_eq!(board_background(&game), normal, "5秒以内は変えない");
    }

    #[test]
    fn board_background_flashes_red_after_threshold_and_resets_on_correct_click() {
        let mut game = playing_game();
        game.update(PRESSURE_THRESHOLD + PRESSURE_PERIOD / 2);
        let (r, g, b) = rgb(board_background(&game));
        assert!(r > g && r > b, "盤面の背景が赤系になる: {r},{g},{b}");
        // 枠の上もパネルの背景として同じ色になる
        let buffer = rendered_buffer(&game);
        let (_, body) = theme::split_hud(AREA);
        assert_eq!(buffer[(body.x, body.y)].bg, Color::Rgb(r, g, b));

        game.update(PRESSURE_PERIOD / 4);
        assert_ne!(
            rgb(board_background(&game)),
            (r, g, b),
            "時間とともに色が変わる"
        );

        click_circle(&mut game, 1);
        assert_eq!(
            board_background(&game),
            Color::Reset,
            "正解クリックで元に戻る"
        );
    }

    #[test]
    fn board_background_resets_on_game_over() {
        let mut game = playing_game();
        game.update(PRESSURE_THRESHOLD * 2);
        lose_all_lives(&mut game);
        assert!(game.game_over, "ライフ切れでGAME OVERになる");
        let buffer = rendered_buffer(&game);
        let (_, body) = theme::split_hud(AREA);
        assert_eq!(
            buffer[(body.x, body.y)].bg,
            Color::Reset,
            "GAME OVERで元に戻る"
        );
    }

    // --- 閾値 ---

    #[test]
    fn pressure_and_timeout_thresholds_match_spec() {
        assert_eq!(PRESSURE_THRESHOLD, Duration::from_secs(5));
        assert_eq!(TIMEOUT_LIMIT, Duration::from_secs(12));
        // 点滅は時間切れの2〜3秒前から
        assert!(TIMEOUT_WARNING >= Duration::from_secs(2));
        assert!(TIMEOUT_WARNING <= Duration::from_secs(3));
        assert!(
            PRESSURE_THRESHOLD < TIMEOUT_LIMIT - TIMEOUT_WARNING,
            "背景の明滅が先に始まり、その後で円が点滅する"
        );
    }

    // --- 誤クリックのバツ印 ---

    #[test]
    fn wrong_click_shows_mark_at_clicked_cell() {
        let mut game = playing_game();
        set_separate_layout(&game);
        assert!(game.wrong_mark.is_none(), "始めはバツ印なし");
        // 円2(次に押すべきでない円)をクリック
        click_board(&mut game, 44, 4);
        assert_eq!(game.round.lives, 2, "不正解");
        let board = board_area(AREA);
        let mark = game.wrong_mark.expect("誤クリックでバツ印が出る");
        assert_eq!(
            mark.center(),
            (board.x + 44, board.y + 4),
            "クリックした位置"
        );
    }

    #[test]
    fn wrong_mark_disappears_after_duration() {
        let mut game = playing_game();
        set_separate_layout(&game);
        click_board(&mut game, 44, 4);
        game.update(wrong_mark::WRONG_MARK_DURATION - Duration::from_millis(1));
        assert!(game.wrong_mark.is_some(), "持続時間内は残る");
        game.update(Duration::from_millis(1));
        assert!(game.wrong_mark.is_none(), "持続時間を過ぎたら消える");
    }

    #[test]
    fn empty_or_correct_click_does_not_show_mark() {
        let mut game = playing_game();
        let (x, y) = set_separate_layout(&game);
        click_board(&mut game, 25, 4);
        assert!(
            game.wrong_mark.is_none(),
            "円の無い所のクリックでは出さない"
        );
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 2);
        assert!(game.wrong_mark.is_none(), "正解では出さない");
    }

    #[test]
    fn new_wrong_click_moves_mark_and_restarts_it() {
        let mut game = playing_game();
        set_layout(
            &game,
            &[(1, 2, 2, 10, 5), (2, 40, 2, 10, 5), (3, 60, 10, 10, 5)],
        );
        click_board(&mut game, 44, 4);
        game.update(wrong_mark::WRONG_MARK_DURATION / 2);
        click_board(&mut game, 64, 12);
        assert_eq!(game.round.lives, 1);
        let board = board_area(AREA);
        assert_eq!(
            game.wrong_mark.unwrap().center(),
            (board.x + 64, board.y + 12),
            "新しい位置に置き換わる"
        );
        game.update(wrong_mark::WRONG_MARK_DURATION / 2);
        assert!(game.wrong_mark.is_some(), "持続時間は最初からやり直す");
    }

    #[test]
    fn correct_click_after_wrong_click_keeps_ripple_and_feedback_unaffected() {
        let mut game = playing_game();
        let (x, y) = set_separate_layout(&game);
        click_board(&mut game, 44, 4);
        assert!(game.wrong_mark.is_some());
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 2);
        let board = board_area(AREA);
        let ripple = game.ripple.expect("正解の波紋はいつも通り出る");
        assert_eq!(ripple.center(), (board.x + x, board.y + y));
        assert_eq!(ripple.progress(), 0.0);
        // 途中の正解はフィードバックを記録しない(既存の挙動)ので、直前の不正解の表示がそのまま続く
        let flash = game.feedback.current().expect("直前の不正解の表示");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert_eq!(flash.detail, "ライフ -1");
        // 波紋(画像表示では盤面の切り出しに重ねる)とぶつからないよう、正解に進んだらバツ印は消す
        assert!(game.wrong_mark.is_none());
    }

    #[test]
    fn game_over_click_shows_mark_with_life_feedback() {
        let mut game = playing_game();
        click_circle(&mut game, 1);
        lose_all_lives(&mut game);
        assert!(game.game_over);
        assert!(
            game.wrong_mark.is_some(),
            "GAME OVERのクリックにもバツ印を出す"
        );
        let flash = game.feedback.current().unwrap();
        assert_eq!(flash.detail, "ライフが尽きた", "ライフ切れの文言はそのまま");
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.is_finished(), "バツ印が残っていてもセッションは終わる");
    }

    #[test]
    fn fallback_render_draws_red_cross_at_wrong_click() {
        let mut game = playing_game();
        set_separate_layout(&game);
        let board = board_area(AREA);
        let cell = (board.x + 42, board.y + 3);
        game.handle_mouse(left_click(cell.0, cell.1), AREA);
        assert!(game.wrong_mark.is_some());
        let buffer = rendered_buffer(&game);
        let [r, g, b] = wrong_mark::WRONG_MARK_COLOR;
        assert_eq!(buffer[cell].symbol(), "✗");
        assert_eq!(buffer[cell].fg, Color::Rgb(r, g, b));

        game.update(wrong_mark::WRONG_MARK_DURATION);
        assert_ne!(
            rendered_buffer(&game)[cell].symbol(),
            "✗",
            "消えたら描かない"
        );
    }

    // --- 時間切れ ---

    #[test]
    fn time_out_is_game_over_with_time_out_message() {
        let mut game = playing_game();
        click_circle(&mut game, 1);
        game.update(TIMEOUT_LIMIT - Duration::from_millis(1));
        assert!(!game.game_over, "上限の直前はまだGAME OVERではない");
        game.update(Duration::from_millis(1));
        assert!(game.game_over, "上限に達したらGAME OVER");
        let flash = game.feedback.current().expect("時間切れのフィードバック");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert!(
            flash.detail.contains("時間切れ"),
            "時間切れの文言: {}",
            flash.detail
        );
        assert_ne!(flash.detail, "ライフが尽きた");
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("時間切れ"), "HUDに時間切れと出る");

        let result = game.result();
        assert_eq!((result.total, result.correct), (1, 0), "失敗として1回記録");
        assert_eq!(
            result.avg_latency_ms,
            params(Difficulty::Beginner).fail_latency_ms
        );
        assert_eq!(
            game.round.lives,
            params(Difficulty::Beginner).lives,
            "ライフは減らさない"
        );
    }

    #[test]
    fn time_out_accumulates_over_many_ticks() {
        let mut game = playing_game();
        let mut elapsed = Duration::ZERO;
        while !game.game_over {
            game.update(crate::TICK_RATE);
            elapsed += crate::TICK_RATE;
            assert!(
                elapsed <= TIMEOUT_LIMIT + crate::TICK_RATE,
                "上限で必ず終わる"
            );
        }
        assert!(elapsed >= TIMEOUT_LIMIT);
    }

    #[test]
    fn time_out_ends_session_after_feedback_like_life_game_over() {
        let mut game = playing_game();
        game.update(TIMEOUT_LIMIT);
        assert!(game.game_over);
        assert!(!game.is_finished(), "フィードバック表示中はまだ終了しない");
        assert!(
            game.interval.is_none(),
            "次のラウンドへの待ち時間には入らない"
        );
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.is_finished(), "フィードバックが消えたらセッション終了");
        assert_eq!(game.tracker.total(), 1, "記録は1回だけ");
    }

    #[test]
    fn time_out_is_recorded_only_once() {
        let mut game = playing_game();
        game.update(TIMEOUT_LIMIT);
        game.update(TIMEOUT_LIMIT);
        game.update(TIMEOUT_LIMIT);
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn clicks_after_time_out_are_ignored() {
        let mut game = playing_game();
        game.update(TIMEOUT_LIMIT);
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 1, "GAME OVER後のクリックは無視される");
        assert_eq!(game.tracker.total(), 1);
    }

    #[test]
    fn time_out_resets_pressure_background() {
        let mut game = playing_game();
        game.update(TIMEOUT_LIMIT);
        assert!(game.round.time_since_target.is_zero());
        let buffer = rendered_buffer(&game);
        let (_, body) = theme::split_hud(AREA);
        assert_eq!(buffer[(body.x, body.y)].bg, Color::Reset, "背景は元に戻る");
    }

    #[test]
    fn correct_click_resets_time_out_countdown() {
        let mut game = playing_game();
        game.update(TIMEOUT_LIMIT - Duration::from_secs(1));
        click_circle(&mut game, 1);
        game.update(TIMEOUT_LIMIT - Duration::from_secs(1));
        assert!(!game.game_over, "正解で時間切れまでの時間は最初から");
        click_circle(&mut game, 2);
        assert_eq!(game.round.next, 3);
    }

    #[test]
    fn wrong_click_does_not_reset_time_out_countdown() {
        let mut game = playing_game();
        game.update(TIMEOUT_LIMIT - Duration::from_secs(1));
        let wrong = wrong_number(&game);
        click_circle(&mut game, wrong);
        assert_eq!(game.round.lives, 2);
        game.update(Duration::from_secs(1));
        assert!(game.game_over, "押し間違えても時間切れまでの時間は続く");
    }

    #[test]
    fn time_out_does_not_happen_during_round_interval() {
        let mut game = playing_game();
        clear_round(&mut game);
        assert!(game.interval.is_some());
        game.update(TIMEOUT_LIMIT);
        assert!(!game.game_over, "待ち時間中は時間切れにならない");
        assert!(game.interval.is_none(), "次のラウンドが始まる");
        assert!(game.round.time_since_target.is_zero());
    }

    // --- 時間切れ前の点滅 ---

    #[test]
    fn target_does_not_blink_before_warning() {
        assert!(!target_blink_on(Duration::ZERO));
        assert!(!target_blink_on(PRESSURE_THRESHOLD));
        assert!(!target_blink_on(
            TIMEOUT_LIMIT - TIMEOUT_WARNING - Duration::from_millis(1)
        ));
    }

    #[test]
    fn target_blinks_periodically_during_warning() {
        let start = TIMEOUT_LIMIT - TIMEOUT_WARNING;
        assert!(target_blink_on(start), "警告が始まったらすぐ強調する");
        assert!(
            !target_blink_on(start + TIMEOUT_BLINK_PERIOD / 2),
            "半周期後は元の見た目に戻る"
        );
        assert!(
            target_blink_on(start + TIMEOUT_BLINK_PERIOD),
            "1周期で強調に戻る"
        );
        // 警告中に強調・非強調が何度も入れ替わる
        let mut toggles = 0;
        let mut last = target_blink_on(start);
        let mut t = start;
        while t < TIMEOUT_LIMIT {
            let now = target_blink_on(t);
            toggles += usize::from(now != last);
            last = now;
            t += crate::TICK_RATE;
        }
        assert!(toggles >= 4, "警告中に何度も点滅する: {toggles}");
    }

    /// テキスト表示で、番号numberの丸囲み数字が描かれたセルの文字色
    fn digit_color(game: &CountManiaGame, number: u8) -> Color {
        let digit = circle_image::circled_digit(number).unwrap().to_string();
        let buffer = rendered_buffer(game);
        buffer
            .content()
            .iter()
            .find(|c| c.symbol() == digit)
            .unwrap_or_else(|| panic!("{digit}が描かれていること"))
            .fg
    }

    #[test]
    fn target_circle_blinks_before_time_out_and_other_circles_do_not() {
        let mut game = playing_game();
        set_separate_layout(&game);
        let [r, g, b] = game.round.circles[0].color;
        let own = Color::Rgb(r, g, b);
        let [r2, g2, b2] = game.round.circles[1].color;
        let other = Color::Rgb(r2, g2, b2);
        let [hr, hg, hb] = TIMEOUT_BLINK_COLOR;
        let highlight = Color::Rgb(hr, hg, hb);
        assert_ne!(own, highlight);

        game.update(TIMEOUT_LIMIT - TIMEOUT_WARNING - Duration::from_millis(100));
        assert_eq!(digit_color(&game, 1), own, "警告前は元の色");

        game.update(Duration::from_millis(100));
        assert_eq!(
            digit_color(&game, 1),
            highlight,
            "警告中は押すべき円が点滅する"
        );
        assert_eq!(digit_color(&game, 2), other, "他の円は点滅しない");

        game.update(TIMEOUT_BLINK_PERIOD / 2);
        assert_eq!(digit_color(&game, 1), own, "点滅なので元の色にも戻る");

        // 正解で押すべき数字が進むと、点滅は解除される
        game.update(TIMEOUT_BLINK_PERIOD / 2);
        assert_eq!(digit_color(&game, 1), highlight);
        click_circle(&mut game, 1);
        assert_eq!(digit_color(&game, 2), other, "正解で点滅は解除される");
    }

    // --- 固定5ラウンド(初級→中級→上級→上級→上級) ---

    #[test]
    fn round_difficulties_rise_from_beginner_to_advanced() {
        assert_eq!(ROUNDS_PER_SESSION, 5);
        assert_eq!(
            ROUND_DIFFICULTIES,
            [
                Difficulty::Beginner,
                Difficulty::Intermediate,
                Difficulty::Advanced,
                Difficulty::Advanced,
                Difficulty::Advanced
            ]
        );
        assert_eq!(ROUND_DIFFICULTIES.len(), ROUNDS_PER_SESSION as usize);
    }

    #[test]
    fn each_round_uses_params_of_its_difficulty() {
        let mut game = playing_game();
        let board = board_area(AREA);
        for (index, difficulty) in ROUND_DIFFICULTIES.into_iter().enumerate() {
            let expected = params(difficulty);
            assert_eq!(game.round_difficulty(), difficulty, "ROUND{}", index + 1);
            assert_eq!(game.round_params(), expected, "ROUND{}", index + 1);
            assert_eq!(game.round.circles.len(), expected.max_number as usize);
            assert_eq!(game.round.lives, expected.lives);
            let used: HashSet<CircleSize> = game.round.circles.iter().map(|c| c.size).collect();
            let allowed: HashSet<CircleSize> = expected.size_levels.iter().copied().collect();
            assert_eq!(used, allowed, "ROUND{}のサイズ段階", index + 1);
            assert_eq!(game.placements(board).len(), expected.max_number as usize);
            clear_round(&mut game);
            if index + 1 < ROUND_DIFFICULTIES.len() {
                pass_interval(&mut game);
            }
        }
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(
            (result.total, result.correct),
            (ROUNDS_PER_SESSION, ROUNDS_PER_SESSION)
        );
    }

    #[test]
    fn difficulty_switches_only_when_next_round_starts() {
        let mut game = playing_game();
        clear_round(&mut game);
        assert!(game.interval.is_some());
        assert_eq!(
            game.round_difficulty(),
            Difficulty::Beginner,
            "待ち時間中はクリアしたラウンドの難易度のまま"
        );
        game.update(ROUND_INTERVAL / 2);
        assert_eq!(game.round_difficulty(), Difficulty::Beginner);
        game.update(ROUND_INTERVAL / 2);
        assert!(game.interval.is_none());
        assert_eq!(game.round_difficulty(), Difficulty::Intermediate);
        assert_eq!(game.round.circles.len(), 14);
    }

    #[test]
    fn game_over_in_round2_records_intermediate_fail_latency() {
        let mut game = playing_game();
        // ROUND1は経過時間0でクリア(記録0ms)
        advance_to_round(&mut game, 1);
        lose_all_lives(&mut game);
        assert!(game.game_over);
        assert_eq!(
            game.round_difficulty(),
            Difficulty::Intermediate,
            "GAME OVER後も失敗したラウンドの難易度のまま"
        );
        let result = game.result();
        assert_eq!((result.total, result.correct), (2, 1));
        let expected = params(Difficulty::Intermediate).fail_latency_ms / 2.0;
        assert!((result.avg_latency_ms - expected).abs() < 1e-9);
    }

    #[test]
    fn round3_uses_advanced_dense_layout_and_two_lives() {
        let mut game = playing_game();
        advance_to_round(&mut game, 2);
        assert_eq!(game.round_difficulty(), Difficulty::Advanced);
        assert!(game.round_params().dense);
        assert_eq!(game.round.lives, 2);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("♥♥"));
        assert!(!text.contains("♥♥♥"), "上級のライフは2");
    }

    #[test]
    fn hud_shows_difficulty_label_of_current_round() {
        let mut game = playing_game();
        let labels: Vec<String> = ROUND_DIFFICULTIES
            .iter()
            .map(|&d| theme::difficulty_label(d).0.replace(' ', ""))
            .collect();
        for (index, label) in labels.iter().enumerate() {
            if index > 0 {
                clear_round(&mut game);
                // 待ち時間中は、クリアしたラウンドの番号・難易度のまま
                let text = rendered_text(&game, AREA.width, AREA.height);
                assert!(text.contains(&format!("ROUND{index}/{ROUNDS_PER_SESSION}")));
                assert!(text.contains(labels[index - 1].as_str()));
                pass_interval(&mut game);
            }
            let text = rendered_text(&game, AREA.width, AREA.height);
            assert!(
                text.contains(&format!("ROUND{}/{ROUNDS_PER_SESSION}", index + 1)),
                "ROUND{}の表示",
                index + 1
            );
            assert!(text.contains(label.as_str()), "ROUND{}: {label}", index + 1);
            // ROUND3〜5はどれも上級なので、ラベルの文字が違うものだけ出ないことを確かめる
            for other in &labels {
                if other != label {
                    assert!(
                        !text.contains(other.as_str()),
                        "ROUND{}に{other}を出さない",
                        index + 1
                    );
                }
            }
        }
    }

    #[test]
    fn result_difficulty_is_session_difficulty_from_the_start() {
        let game = playing_game();
        assert_eq!(game.result().difficulty, SESSION_DIFFICULTY);
    }

    // --- 円のサイズに応じた波紋 ---

    #[test]
    fn ripple_size_follows_clicked_circle_size() {
        let mut game = playing_game();
        let (x, y) = set_separate_layout(&game);
        game.round.circles[0].size = CircleSize::Huge;
        game.round.circles[1].size = CircleSize::Small;
        click_board(&mut game, x, y);
        let huge = game.ripple.expect("正解で波紋が出る");
        assert_eq!(
            huge.max_radius_cells(),
            ripple::max_radius_cells_for(CircleSize::Huge)
        );
        click_board(&mut game, 44, 4);
        assert_eq!(game.round.next, 3);
        let small = game.ripple.expect("正解で波紋が出る");
        assert_eq!(
            small.max_radius_cells(),
            ripple::max_radius_cells_for(CircleSize::Small)
        );
        assert!(
            huge.max_radius_cells() > small.max_radius_cells() * 2.0,
            "特大の円の波紋は小さい円よりはっきり大きく広がる"
        );
    }

    // --- ROUND4・ROUND5: 動く円 ---

    /// 前のラウンドを遊ばずに、0始まりでindex番目のラウンドを直接始める
    /// (動くラウンドを、前のラウンドの配置の乱数に左右されずに確かめるため)。
    /// ラウンド冒頭のカウントダウンも終えて、すぐクリックできる状態にする
    fn start_round(game: &mut CountManiaGame, index: u32) {
        assert!(index >= 1, "ROUND1はnew()で始まる");
        game.round_serial = index - 1;
        game.start_next_round();
        assert_eq!(game.round_serial, index);
        finish_countdown(game);
    }

    /// 番号numberの円の、盤面左上からの相対位置の矩形
    fn rect_of(game: &CountManiaGame, number: u8) -> Rect {
        let board = board_area(AREA);
        let rect = game
            .placements(board)
            .iter()
            .find(|p| p.number == number)
            .unwrap_or_else(|| panic!("円{number}が配置されていること"))
            .rect;
        Rect::new(rect.x - board.x, rect.y - board.y, rect.width, rect.height)
    }

    fn assert_all_inside_board(game: &CountManiaGame, board: Rect) {
        for p in game.placements(board) {
            assert_eq!(
                board.intersection(p.rect),
                p.rect,
                "円{}が盤面の中にあること",
                p.number
            );
        }
    }

    #[test]
    fn round4_and_round5_use_advanced_params() {
        let mut game = playing_game();
        for index in [3, 4] {
            start_round(&mut game, index);
            let advanced = params(Difficulty::Advanced);
            assert_eq!(game.round_difficulty(), Difficulty::Advanced);
            assert_eq!(game.round_params(), advanced, "ROUND{}", index + 1);
            assert_eq!(game.round.circles.len(), advanced.max_number as usize);
            assert_eq!(game.round.lives, advanced.lives);
        }
    }

    #[test]
    fn round4_and_round5_scatter_and_other_rounds_stay_still() {
        assert_eq!(
            ROUND_MOTIONS,
            [
                MotionKind::Still,
                MotionKind::Still,
                MotionKind::Still,
                MotionKind::Scatter,
                MotionKind::Scatter
            ]
        );
        let mut game = playing_game();
        assert!(game.round.motion.is_none(), "ROUND1は円が動かない");
        for index in 1..ROUNDS_PER_SESSION {
            start_round(&mut game, index);
            let scatters = matches!(index, 3 | 4);
            let expected = if scatters {
                MotionKind::Scatter
            } else {
                MotionKind::Still
            };
            assert_eq!(game.round_motion(), expected, "ROUND{}", index + 1);
            assert_eq!(
                game.round.motion.is_some(),
                scatters,
                "ROUND{}: 散らすラウンドだけ円の位置を持つ",
                index + 1
            );
        }
    }

    #[test]
    fn round5_scatters_stronger_than_round4() {
        let mut game = playing_game();
        start_round(&mut game, 3);
        let round4 = game.round_scatter();
        assert_eq!(round4, SCATTER_NORMAL, "ROUND4は今までの強さのまま");
        assert_eq!(
            (round4.radius, round4.distance),
            (24.0, 12.0),
            "ROUND4の値は変えない"
        );
        start_round(&mut game, 4);
        let round5 = game.round_scatter();
        assert_eq!(round5, SCATTER_STRONG);
        assert!(
            round5.distance >= round4.distance * 1.5 && round5.distance <= round4.distance * 2.0,
            "ROUND5は1.5〜2倍の距離を弾き飛ばす: {round5:?}"
        );
        assert!(
            round5.radius > round4.radius,
            "ROUND5は広い範囲の円を弾き飛ばす: {round5:?}"
        );
    }

    /// 0始まりでindex番目のラウンドを始め、set_scatter_layoutの配置で円1を押した後の
    /// 円2(真右)の位置
    fn pushed_right_circle_after_first_click(index: u32) -> Rect {
        let mut game = playing_game();
        start_round(&mut game, index);
        let (x, y) = set_scatter_layout(&game);
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 2, "円1は正解");
        rect_of(&game, 2)
    }

    #[test]
    fn round5_pushes_nearby_circle_farther_than_round4() {
        let round4 = pushed_right_circle_after_first_click(3);
        let round5 = pushed_right_circle_after_first_click(4);
        assert_eq!(round4.x, 54 + 12, "ROUND4は今まで通り12セル");
        assert!(
            round5.x > round4.x,
            "ROUND5の方が遠くへ弾き飛ばす: ROUND4={round4:?} ROUND5={round5:?}"
        );
        assert_eq!(round5.y, 10, "真横の円は縦には動かない");
    }

    #[test]
    fn round5_scatters_circles_beyond_round4_radius() {
        // 円1の中心(45.5,12.5)から円2の中心(75,12.5)までは見た目の距離29.5。
        // ROUND4の範囲(24)の外で、ROUND5の範囲の内側
        let layout = [(1, 40, 10, 10, 5), (2, 70, 10, 10, 5)];
        let mut game = playing_game();
        start_round(&mut game, 3);
        set_layout(&game, &layout);
        click_board(&mut game, 45, 12);
        assert_eq!(
            rect_of(&game, 2),
            Rect::new(70, 10, 10, 5),
            "ROUND4では範囲外なので動かない"
        );
        start_round(&mut game, 4);
        set_layout(&game, &layout);
        click_board(&mut game, 45, 12);
        assert!(
            rect_of(&game, 2).x > 70,
            "ROUND5では範囲内なので離れる: {:?}",
            rect_of(&game, 2)
        );
    }

    #[test]
    fn rounds_1_to_3_keep_positions_over_time_and_clicks() {
        let board = board_area(AREA);
        let mut game = playing_game();
        for index in 0..3 {
            if index > 0 {
                start_round(&mut game, index);
            }
            let first = game.placements(board);
            for _ in 0..30 {
                game.update(crate::TICK_RATE);
            }
            click_circle(&mut game, 1);
            game.update(Duration::from_secs(1));
            assert_eq!(game.round.next, 2);
            assert_eq!(
                game.placements(board),
                first,
                "ROUND{}は円が動かない",
                index + 1
            );
            assert_eq!(game.position_generation(), 0, "位置の世代も進まない");
            assert_eq!(game.layout.borrow().as_ref().unwrap().generation, 0);
        }
    }

    /// ROUND4の拡散を確かめる配置(盤面左上からの相対位置)。円1の中心をクリックすると、
    /// 近くの円2(真右)・円3(左下)は離れる向きへ動き、遠くの円4は動かない。戻り値は円1の中心
    fn set_scatter_layout(game: &CountManiaGame) -> (u16, u16) {
        set_layout(
            game,
            &[
                (1, 40, 10, 10, 5),
                (2, 54, 10, 10, 5),
                (3, 30, 14, 10, 5),
                (4, 2, 25, 10, 5),
            ],
        );
        (45, 12)
    }

    #[test]
    fn round4_correct_click_pushes_nearby_circles_away_and_keeps_far_ones() {
        let mut game = playing_game();
        start_round(&mut game, 3);
        let (x, y) = set_scatter_layout(&game);
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 2, "円1は正解");

        let right = rect_of(&game, 2);
        assert!(right.x >= 54 + 8, "真右の円2は右へ離れる: {right:?}");
        assert_eq!(right.y, 10, "真横の円は縦には動かない");
        let lower_left = rect_of(&game, 3);
        assert!(
            lower_left.x < 30 && lower_left.y > 14,
            "左下の円3は左下へ離れる: {lower_left:?}"
        );
        assert_eq!(
            rect_of(&game, 4),
            Rect::new(2, 25, 10, 5),
            "遠くの円4は動かない"
        );
        assert_eq!(
            rect_of(&game, 1),
            Rect::new(40, 10, 10, 5),
            "押した円自身は動かさない"
        );
        assert_eq!((right.width, right.height), (10, 5), "大きさは変わらない");
    }

    #[test]
    fn round4_and_round5_scatter_keeps_circles_inside_board() {
        for index in [3, 4] {
            let mut game = playing_game();
            start_round(&mut game, index);
            // 盤面は98x31なので、幅10・高さ5の円の左上が取れる範囲は x=0〜88, y=0〜26
            set_layout(
                &game,
                &[(1, 70, 12, 10, 5), (2, 84, 12, 10, 5), (3, 72, 3, 10, 5)],
            );
            click_board(&mut game, 75, 14);
            assert_eq!(game.round.next, 2);
            assert_eq!(rect_of(&game, 2).x, 88, "ROUND{}: 右端で止まる", index + 1);
            assert_eq!(rect_of(&game, 3).y, 0, "ROUND{}: 上端で止まる", index + 1);
            assert_all_inside_board(&game, board_area(AREA));
        }
    }

    #[test]
    fn round4_wrong_or_empty_click_does_not_move_circles() {
        let mut game = playing_game();
        start_round(&mut game, 3);
        set_scatter_layout(&game);
        let board = board_area(AREA);
        let before = game.placements(board);
        // 円2(次に押すべきでない円)をクリック
        click_board(&mut game, 59, 12);
        assert_eq!(game.round.lives, 1, "不正解");
        assert!(game.wrong_mark.is_some(), "動くラウンドでもバツ印を出す");
        // 円の無い所をクリック
        click_board(&mut game, 20, 2);
        assert_eq!(game.placements(board), before, "正解以外では円は動かない");
    }

    #[test]
    fn round4_and_round5_circles_stay_still_between_clicks() {
        for index in [3, 4] {
            let mut game = playing_game();
            start_round(&mut game, index);
            let board = board_area(AREA);
            let first = game.placements(board);
            for _ in 0..30 {
                game.update(crate::TICK_RATE);
            }
            assert_eq!(
                game.placements(board),
                first,
                "ROUND{}: クリックするまでは動かない",
                index + 1
            );
            let (x, y) = set_scatter_layout(&game);
            click_board(&mut game, x, y);
            let after = game.placements(board);
            let generation = game.position_generation();
            for _ in 0..30 {
                game.update(crate::TICK_RATE);
            }
            assert_eq!(
                game.placements(board),
                after,
                "ROUND{}: 放っておいても動き続けない",
                index + 1
            );
            assert_eq!(
                game.position_generation(),
                generation,
                "位置の世代も進まない"
            );
        }
    }

    #[test]
    fn round5_image_mode_does_not_reencode_board_while_idle() {
        // ROUND5も放っておく間は盤面を作り直さない(円が動き続けて盤面全体がちらつかない)
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut game = playing_game();
        start_round(&mut game, 4);
        let mut picker = Picker::from_fontsize((4, 8));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        game.renderer = CircleRenderer::with_picker(picker);
        rendered_text(&game, AREA.width, AREA.height);
        let boards = game.renderer.board_encode_count();
        for _ in 0..30 {
            // 時間切れ前の点滅(これも盤面を作り直す)が混ざらないようにする
            game.round.time_since_target = Duration::ZERO;
            game.update(crate::TICK_RATE);
            rendered_text(&game, AREA.width, AREA.height);
        }
        assert_eq!(
            game.renderer.board_encode_count(),
            boards,
            "クリックしていない間は盤面を作り直さない"
        );
    }

    #[test]
    fn round4_next_click_is_judged_at_moved_position() {
        let mut game = playing_game();
        start_round(&mut game, 3);
        let (x, y) = set_scatter_layout(&game);
        click_board(&mut game, x, y);
        // 円2が元あった場所のクリックは空振り(円1も消えている)
        click_board(&mut game, 59, 12);
        assert_eq!(game.round.next, 2);
        assert_eq!(game.round.lives, 2, "空白扱いでライフは減らない");
        // 動いた先の円2をクリックすれば正解
        let moved = rect_of(&game, 2);
        click_board(
            &mut game,
            moved.x + moved.width / 2,
            moved.y + moved.height / 2,
        );
        assert_eq!(game.round.next, 3);
        let board = board_area(AREA);
        let ripple = game.ripple.expect("動いた円でも正解の波紋が出る");
        assert_eq!(
            ripple.center(),
            (
                board.x + moved.x + moved.width / 2,
                board.y + moved.y + moved.height / 2
            )
        );
    }

    #[test]
    fn round5_resized_board_restarts_positions_from_new_layout() {
        let mut game = playing_game();
        start_round(&mut game, 4);
        let (x, y) = set_scatter_layout(&game);
        click_board(&mut game, x, y);
        assert_eq!(
            game.round.motion.as_ref().unwrap().board,
            Some(board_area(AREA))
        );
        let bigger = board_area(Rect::new(0, 0, 160, 50));
        let relaid = game.placements(bigger);
        assert_all_inside_board(&game, bigger);
        // 大きくした盤面の配置から、次の正解クリックで散らし直す
        let target = relaid.iter().find(|p| p.number == 2).unwrap().rect;
        let (cx, cy) = (target.x + target.width / 2, target.y + target.height / 2);
        game.handle_mouse(left_click(cx, cy), Rect::new(0, 0, 160, 50));
        assert_eq!(game.round.next, 3, "大きくした盤面の配置で判定する");
        assert_eq!(
            game.round.motion.as_ref().unwrap().board,
            Some(bigger),
            "新しい盤面の配置から動かし直す"
        );
        assert_all_inside_board(&game, bigger);
    }

    #[test]
    fn full_session_of_five_rounds_can_be_cleared_with_moving_rounds() {
        let mut game = playing_game();
        for round in 0..ROUNDS_PER_SESSION {
            // ROUND4・ROUND5は円が散らばっても、今の位置をクリックすれば進める
            clear_round(&mut game);
            if round + 1 < ROUNDS_PER_SESSION {
                pass_interval(&mut game);
            }
        }
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(
            (result.total, result.correct),
            (ROUNDS_PER_SESSION, ROUNDS_PER_SESSION)
        );
    }

    #[test]
    fn round4_and_round5_scatter_never_hides_any_remaining_number() {
        // 散らした円が盤面の端に寄せられて同じ大きさの円とぴったり重なる等で、
        // 残っている円の数字が隠れる(=押せなくなる)ことが無いこと。
        // どの時点でも、残っている全ての円の中心をクリックすればその円に当たる
        let board = board_area(AREA);
        for index in [3, 4] {
            for trial in 0..40 {
                let mut game = playing_game();
                start_round(&mut game, index);
                for number in 1..=game.round_params().max_number {
                    let placements = game.placements(board);
                    for p in placements.iter().filter(|p| game.is_visible(p.number)) {
                        let (x, y) = (p.rect.x + p.rect.width / 2, p.rect.y + p.rect.height / 2);
                        assert_eq!(
                            hit_test(&placements, |n| game.is_visible(n), x, y),
                            Some(p.number),
                            "ROUND{} trial={trial} 円{number}を押す前: 円{}の中心が他の円に隠れている",
                            index + 1,
                            p.number
                        );
                    }
                    let (x, y) = center_of(&game, number);
                    game.handle_mouse(left_click(x, y), AREA);
                    assert_eq!(
                        game.round.next,
                        number + 1,
                        "ROUND{} trial={trial}",
                        index + 1
                    );
                }
                assert_eq!(game.tracker.total(), 1);
                assert_eq!(game.round.lives, params(Difficulty::Advanced).lives);
            }
        }
    }

    /// 0始まりでindex番目のラウンドを何度か通しで押し、散らす範囲にあった円が1回の正解クリックで
    /// 動いた見た目の距離の平均と、そのラウンドの散らす強さ
    fn average_scatter_movement(index: u32) -> (f64, ScatterStrength) {
        let board = board_area(AREA);
        let (mut moved, mut total) = (0.0, 0usize);
        let mut strength = SCATTER_NORMAL;
        for _ in 0..20 {
            let mut game = playing_game();
            start_round(&mut game, index);
            strength = game.round_scatter();
            for number in 1..game.round_params().max_number {
                let before = game.placements(board);
                let (x, y) = center_of(&game, number);
                game.handle_mouse(left_click(x, y), AREA);
                let after = game.placements(board);
                let click = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                for (b, a) in before.iter().zip(&after) {
                    let center = (
                        f64::from(b.rect.x) + f64::from(b.rect.width) / 2.0,
                        f64::from(b.rect.y) + f64::from(b.rect.height) / 2.0,
                    );
                    let nearby =
                        scatter_offset(click, center, strength.radius, 1.0, (1.0, 0.0)).is_some();
                    if b.number > number && nearby {
                        let dx = f64::from(a.rect.x) - f64::from(b.rect.x);
                        let dy = (f64::from(a.rect.y) - f64::from(b.rect.y)) * CELL_ASPECT;
                        moved += dx.hypot(dy);
                        total += 1;
                    }
                }
            }
        }
        (moved / total as f64, strength)
    }

    #[test]
    fn round4_and_round5_scatter_still_moves_circles_on_average() {
        // 数字を隠さない位置に限っても、クリックした付近の円ははっきり散らばる
        let (round4, strength4) = average_scatter_movement(3);
        let (round5, strength5) = average_scatter_movement(4);
        assert!(
            round4 >= strength4.distance * 0.5,
            "ROUND4の近くの円の平均の移動距離{round4:.1}"
        );
        assert!(
            round5 >= strength5.distance * 0.5,
            "ROUND5の近くの円の平均の移動距離{round5:.1}"
        );
        assert!(
            round5 > round4,
            "ROUND5の方が平均しても遠くへ散らばる: ROUND4={round4:.1} ROUND5={round5:.1}"
        );
    }

    #[test]
    fn round4_scatter_avoids_hiding_number_of_crowded_neighbor() {
        let mut game = playing_game();
        start_round(&mut game, 3);
        // 円2をそのまま右へ押すと、同じ大きさで手前(並びの後ろ)の円3にぴったり重なって隠れる
        set_layout(
            &game,
            &[(1, 40, 10, 10, 5), (2, 54, 10, 10, 5), (3, 66, 10, 10, 5)],
        );
        click_board(&mut game, 45, 12);
        assert_eq!(game.round.next, 2);
        let board = board_area(AREA);
        let placements = game.placements(board);
        for number in [2, 3] {
            let (x, y) = center_of(&game, number);
            assert_eq!(
                hit_test(&placements, |n| game.is_visible(n), x, y),
                Some(number),
                "円{number}の数字が隠れていないこと: {placements:?}"
            );
        }
    }

    // --- 水槽の魚 ---

    use fish::{FISH_WIDTH, FLEE_DURATION};

    const FISH_TICK: Duration = Duration::from_millis(33);

    /// 魚を盤面に放し、全部を左上(x, y)に止めて置く
    fn put_all_fish_at(game: &mut CountManiaGame, x: u16, y: u16) {
        game.place_fish(board_area(AREA));
        for fish in &mut game.fish {
            *fish = Fish::new(f64::from(x), f64::from(y));
        }
    }

    fn fish_inside(fish: &Fish, board: Rect) -> bool {
        fish.x >= f64::from(board.x)
            && fish.x <= f64::from(board.right() - FISH_WIDTH)
            && fish.y >= f64::from(board.y)
            && fish.y < f64::from(board.bottom())
    }

    #[test]
    fn fish_are_released_into_board_after_render_and_update() {
        let mut game = playing_game();
        rendered_text(&game, AREA.width, AREA.height);
        game.update(FISH_TICK);
        let board = board_area(AREA);
        assert_eq!(game.fish.len(), FISH_COUNT);
        assert!((2..=3).contains(&FISH_COUNT));
        for _ in 0..300 {
            game.update(FISH_TICK);
            assert!(game.fish.iter().all(|fish| fish_inside(fish, board)));
        }
    }

    #[test]
    fn fish_keep_swimming_during_round_interval_and_after_game_over() {
        let mut game = playing_game();
        rendered_text(&game, AREA.width, AREA.height);
        game.update(FISH_TICK);
        clear_round(&mut game);
        assert!(game.interval.is_some());
        let before = game.fish.clone();
        for _ in 0..10 {
            game.update(FISH_TICK);
        }
        assert_ne!(game.fish, before, "待ち時間中も泳ぐ");

        let mut game = playing_game();
        rendered_text(&game, AREA.width, AREA.height);
        game.update(FISH_TICK);
        lose_all_lives(&mut game);
        assert!(game.game_over);
        let before = game.fish.clone();
        for _ in 0..10 {
            game.update(FISH_TICK);
        }
        assert_ne!(game.fish, before, "GAME OVER後も泳ぐ");
    }

    #[test]
    fn empty_click_spooks_only_nearby_fish() {
        let mut game = playing_game();
        let (column, row) = empty_cell(&game);
        put_all_fish_at(&mut game, column, row);
        let board = board_area(AREA);
        game.fish[0] = Fish::new(
            f64::from(board.right() - FISH_WIDTH),
            f64::from(board.bottom() - 1),
        );
        let (fx, fy) = game.fish[0].center();
        let distance = (fx - f64::from(column)).hypot((fy - f64::from(row)) * CELL_ASPECT);
        assert!(
            distance > SPOOK_RADIUS,
            "遠い魚は驚かない範囲にいる: {distance}"
        );
        let lives = game.round.lives;

        game.handle_mouse(left_click(column, row), AREA);

        assert_eq!(game.fish[0].flee_timer, 0.0, "遠い魚は驚かない");
        for fish in &game.fish[1..] {
            assert_eq!(fish.flee_timer, FLEE_DURATION, "近くの魚は驚く");
        }
        assert_eq!(game.round.next, 1, "ゲームは進まない");
        assert_eq!(game.round.lives, lives, "ライフも減らない");
    }

    #[test]
    fn circle_clicks_do_not_spook_fish() {
        let mut game = playing_game();
        let (column, row) = clickable_cell(&game, 1);
        put_all_fish_at(&mut game, column, row);
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 2);
        assert!(
            game.fish.iter().all(|fish| fish.flee_timer == 0.0),
            "正解では驚かない"
        );

        let wrong = wrong_number(&game);
        let (column, row) = clickable_cell(&game, wrong);
        put_all_fish_at(&mut game, column, row);
        let lives = game.round.lives;
        click_circle(&mut game, wrong);
        assert_eq!(game.round.lives, lives - 1);
        assert!(
            game.fish.iter().all(|fish| fish.flee_timer == 0.0),
            "不正解でも驚かない"
        );
    }

    #[test]
    fn text_mode_draws_fish_behind_circles() {
        let mut game = playing_game();
        let (column, row) = empty_cell(&game);
        put_all_fish_at(&mut game, column, row);
        for fish in &mut game.fish {
            fish.facing_right = true;
        }
        assert_eq!(rendered_buffer(&game)[(column, row)].symbol(), ">");

        // 円の中心に置いた魚は、手前の円に隠れる
        let (cx, cy) = center_of(&game, 1);
        put_all_fish_at(&mut game, cx, cy);
        let symbol = rendered_buffer(&game)[(cx, cy)].symbol().to_string();
        assert!(symbol != ">" && symbol != "<", "円が魚より手前: {symbol}");
    }

    #[test]
    fn text_mode_render_with_fish_does_not_panic_on_tiny_areas() {
        let mut game = playing_game();
        rendered_text(&game, AREA.width, AREA.height);
        game.update(FISH_TICK);
        for (width, height) in [(20, 6), (5, 5), (1, 1), (AREA.width, AREA.height)] {
            rendered_text(&game, width, height);
            game.update(FISH_TICK);
        }
        clear_round(&mut game);
        rendered_text(&game, AREA.width, AREA.height);
    }

    #[test]
    fn image_mode_render_with_fish_does_not_panic() {
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut game = playing_game();
        let mut picker = Picker::from_fontsize((4, 8));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        game.renderer = CircleRenderer::with_picker(picker);
        rendered_text(&game, AREA.width, AREA.height);
        game.update(FISH_TICK);
        let (column, row) = empty_cell(&game);
        game.handle_mouse(left_click(column, row), AREA);
        for (width, height) in [(AREA.width, AREA.height), (20, 6), (1, 1)] {
            rendered_text(&game, width, height);
            game.update(FISH_TICK);
        }
        rendered_text(&game, AREA.width, AREA.height);
    }

    #[test]
    fn image_mode_draws_moving_fish_as_patches_without_rebuilding_board() {
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut game = playing_game();
        let mut picker = Picker::from_fontsize((4, 8));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        game.renderer = CircleRenderer::with_picker(picker);
        let (column, row) = empty_cell(&game);
        put_all_fish_at(&mut game, column, row);
        rendered_text(&game, AREA.width, AREA.height);
        let boards = game.renderer.board_encode_count();
        assert_eq!(
            game.renderer.ripple_encode_count(),
            0,
            "最初は盤面の画像に魚も描く"
        );

        // 魚が別のセルへ動くと、盤面全体は作り直さず、魚の周りのパッチだけを作る
        game.fish[0].x += 2.0;
        rendered_text(&game, AREA.width, AREA.height);
        assert_eq!(game.renderer.board_encode_count(), boards);
        assert!(game.renderer.ripple_encode_count() > 0);
        assert!(!game.renderer.patch_rects().is_empty());
    }

    #[test]
    fn text_mode_does_not_pass_fish_to_image_patches() {
        // テキスト表示では魚を文字で描き、画像のパッチは作らない
        let mut game = playing_game();
        let (column, row) = empty_cell(&game);
        put_all_fish_at(&mut game, column, row);
        rendered_text(&game, AREA.width, AREA.height);
        game.fish[0].x += 2.0;
        rendered_text(&game, AREA.width, AREA.height);
        assert_eq!(game.renderer.ripple_encode_count(), 0);
        assert!(game.renderer.patch_rects().is_empty());
    }
}
