use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::audio;
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};

/// 曲ごとの実測ビート時刻データ(src/game/rhythm/beats.rs)
mod beats;

pub const GAME_ID: &str = "rhythm";

/// ノーツが画面下端に出現してから判定ラインに到達するまでの所要時間。
/// BPM/譜面とは独立した「スクロール速度」の概念(仕様5)。
const SCROLL_TRAVEL_TIME: Duration = Duration::from_millis(1500);

/// 直近の判定(PERFECT/GREAT/GOOD/MISS)を画面に表示し続ける時間
const JUDGEMENT_DISPLAY_HOLD: Duration = Duration::from_millis(600);

/// 結果に記録する難易度。TTRは難易度を選ばず常に上級の譜面・判定でプレイするので固定値にする
/// (これまでの上級の記録と同じ扱いになるよう上級にする)
pub const SESSION_DIFFICULTY: Difficulty = Difficulty::Advanced;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Up,
    Down,
    Left,
    Right,
}

impl Lane {
    /// レーン並び(画面左から ←↓↑→、実機のPump It Up系プレイ画面に準拠)。
    /// 表示位置と入力キーの対応はこの順序に一致させる。
    fn all() -> [Lane; 4] {
        [Lane::Left, Lane::Down, Lane::Up, Lane::Right]
    }

    /// 判定ライン上に出す矢印
    fn arrow(self) -> &'static str {
        match self {
            Lane::Left => "←",
            Lane::Up => "↑",
            Lane::Down => "↓",
            Lane::Right => "→",
        }
    }

    /// レーンの色。左右と上下で色を分け、どのレーンのノーツかを見分けやすくする
    fn color(self) -> Color {
        match self {
            Lane::Left | Lane::Right => Color::LightMagenta,
            Lane::Up | Lane::Down => Color::LightCyan,
        }
    }
}

/// タイミング判定のランク。宣言順が「良い順」で、Ord導出により
/// 同時押しノーツの最終判定(=最も悪いレーンの判定を採用)を`max`で求められる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Judgement {
    Perfect,
    Great,
    Good,
    Miss,
}

fn judgement_label(judgement: Judgement) -> &'static str {
    match judgement {
        Judgement::Perfect => "PERFECT",
        Judgement::Great => "GREAT",
        Judgement::Good => "GOOD",
        Judgement::Miss => "MISS",
    }
}

fn judgement_color(judgement: Judgement) -> Color {
    match judgement {
        Judgement::Perfect => Color::Cyan,
        Judgement::Great => Color::Green,
        Judgement::Good => Color::Yellow,
        Judgement::Miss => Color::Red,
    }
}

/// 判定ウィンドウ(判定ラインとの時間差の許容幅)
#[derive(Debug, Clone, Copy)]
struct JudgeWindows {
    perfect: Duration,
    great: Duration,
    good: Duration,
}

/// TTRの判定ウィンドウ(旧上級の値)
const JUDGE_WINDOWS: JudgeWindows = JudgeWindows {
    perfect: Duration::from_millis(35),
    great: Duration::from_millis(70),
    good: Duration::from_millis(120),
};

// ---------------------------------------------------------------------------
// 楽曲と譜面生成
//
// 譜面はBPMからの理論値ではなく、曲ごとに実測したビート時刻(beats.rs)の上に配置する。
// 曲の中での忙しさの違いは「セクション密度(Low/Mid/High/Extreme)」で表現する。
// 難易度は選ばず、常に上級相当の配置にする。
// 譜面は乱数を使わず決定的に生成する(同じ曲なら毎回同じ譜面で、練習して覚えられる)。
// ---------------------------------------------------------------------------

/// 曲の大まかな盛り上がり(RMSエネルギー解析による区分)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionDensity {
    Low,
    Mid,
    High,
    /// 最高難度区間。休符なしで毎拍同時押し+8分音符を配置する
    Extreme,
}

/// リズムゲームで遊べる1曲分の定義
pub struct RhythmSong {
    /// audio::play_bgm_trackに渡す名前(拡張子・ディレクトリなし)
    pub track_name: &'static str,
    /// 曲選択画面に表示する名前
    pub display_name: &'static str,
    /// 曲全体の長さ(ミリ秒、音源の実測値)。全ノーツ判定後もこの時間までは曲を流し続ける
    duration_ms: u32,
    /// 実測ビート時刻(ミリ秒、曲頭からの経過時間、昇順)
    beat_times_ms: &'static [u32],
    /// (この区間が始まるビートインデックス, 密度) を昇順で並べたもの。
    /// 先頭要素のビートインデックスは0であること
    sections: &'static [(usize, SectionDensity)],
}

/// 実測ビート時刻の配列から、指定ミリ秒「以降」で最初のビートのインデックスを返す。
/// 全ビートより後ならビート数(=範囲外)を返す。セクション境界をコンパイル時に
/// ビートインデックスへ変換するため const fn にしている
const fn beat_index_at_or_after_ms(beat_times_ms: &[u32], ms: u32) -> usize {
    let mut i = 0;
    while i < beat_times_ms.len() && beat_times_ms[i] < ms {
        i += 1;
    }
    i
}

/// 秒指定版の beat_index_at_or_after_ms(セクション区分表が秒単位のため)
const fn beat_index_at_or_after_secs(beat_times_ms: &[u32], secs: u32) -> usize {
    beat_index_at_or_after_ms(beat_times_ms, secs * 1000)
}

const TOP_OF_THE_LEADERBOARD_SECTIONS: &[(usize, SectionDensity)] = {
    use SectionDensity::{High, Low, Mid};
    const B: &[u32] = beats::TOP_OF_THE_LEADERBOARD_BEATS_MS;
    &[
        (beat_index_at_or_after_secs(B, 0), Low),
        (beat_index_at_or_after_secs(B, 15), Mid),
        (beat_index_at_or_after_secs(B, 50), Low),
        (beat_index_at_or_after_secs(B, 60), Mid),
        (beat_index_at_or_after_secs(B, 90), High),
        (beat_index_at_or_after_secs(B, 115), Mid),
        (beat_index_at_or_after_secs(B, 140), Low),
        (beat_index_at_or_after_secs(B, 150), Mid),
        (beat_index_at_or_after_secs(B, 170), Low),
    ]
};

const REDLINE_RESPONSE_TIME_SECTIONS: &[(usize, SectionDensity)] = {
    use SectionDensity::{High, Low, Mid};
    const B: &[u32] = beats::REDLINE_RESPONSE_TIME_BEATS_MS;
    &[
        (beat_index_at_or_after_secs(B, 0), Low),
        (beat_index_at_or_after_secs(B, 20), Mid),
        (beat_index_at_or_after_secs(B, 30), High),
        (beat_index_at_or_after_secs(B, 40), Mid),
        (beat_index_at_or_after_secs(B, 45), Low),
        (beat_index_at_or_after_secs(B, 50), Mid),
        (beat_index_at_or_after_secs(B, 65), Low),
        (beat_index_at_or_after_secs(B, 75), Mid),
        (beat_index_at_or_after_secs(B, 95), Low),
        (beat_index_at_or_after_secs(B, 100), Mid),
        (beat_index_at_or_after_secs(B, 130), Low),
        (beat_index_at_or_after_secs(B, 145), Mid),
        (beat_index_at_or_after_secs(B, 155), High),
        (beat_index_at_or_after_secs(B, 165), Mid),
        (beat_index_at_or_after_secs(B, 175), Low),
    ]
};

/// 50-100秒・130-170秒の最高密度区間をExtremeにし、既存2曲より明確に難しくする
const APEX_MOVEMENT_SECTIONS: &[(usize, SectionDensity)] = {
    use SectionDensity::{Extreme, Low, Mid};
    const B: &[u32] = beats::APEX_MOVEMENT_BEATS_MS;
    &[
        (beat_index_at_or_after_secs(B, 0), Low),
        (beat_index_at_or_after_secs(B, 30), Mid),
        (beat_index_at_or_after_secs(B, 40), Low),
        (beat_index_at_or_after_secs(B, 50), Extreme),
        (beat_index_at_or_after_secs(B, 100), Low),
        (beat_index_at_or_after_secs(B, 120), Mid),
        (beat_index_at_or_after_secs(B, 130), Extreme),
        (beat_index_at_or_after_secs(B, 170), Low),
    ]
};

/// 90-115秒・150-170秒の最高潮をExtreme、その間の145-150秒の再構築をHighにする
const OVERCLOCKED_TEMPO_SECTIONS: &[(usize, SectionDensity)] = {
    use SectionDensity::{Extreme, High, Low, Mid};
    const B: &[u32] = beats::OVERCLOCKED_TEMPO_BEATS_MS;
    &[
        (beat_index_at_or_after_secs(B, 0), Low),
        (beat_index_at_or_after_secs(B, 10), Mid),
        (beat_index_at_or_after_secs(B, 50), Low),
        (beat_index_at_or_after_secs(B, 60), Mid),
        (beat_index_at_or_after_secs(B, 90), Extreme),
        (beat_index_at_or_after_secs(B, 115), Mid),
        (beat_index_at_or_after_secs(B, 140), Low),
        (beat_index_at_or_after_secs(B, 145), High),
        (beat_index_at_or_after_secs(B, 150), Extreme),
        (beat_index_at_or_after_secs(B, 170), Low),
    ]
};

/// 曲選択画面に並べる曲一覧(表示順)。曲を増やすときはbeats.rsに実測ビート時刻を足し、ここに追加する
pub const SONGS: &[RhythmSong] = &[
    RhythmSong {
        track_name: "Top_of_the_Leaderboard",
        display_name: "Top of the Leaderboard (BPM 150)",
        duration_ms: 178_808,
        beat_times_ms: beats::TOP_OF_THE_LEADERBOARD_BEATS_MS,
        sections: TOP_OF_THE_LEADERBOARD_SECTIONS,
    },
    RhythmSong {
        track_name: "Redline_Response_Time",
        display_name: "Redline Response Time (BPM 180)",
        duration_ms: 179_435,
        beat_times_ms: beats::REDLINE_RESPONSE_TIME_BEATS_MS,
        sections: REDLINE_RESPONSE_TIME_SECTIONS,
    },
    RhythmSong {
        track_name: "Apex_Movement",
        display_name: "Apex Movement (BPM 152)",
        duration_ms: 182_204,
        beat_times_ms: beats::APEX_MOVEMENT_BEATS_MS,
        sections: APEX_MOVEMENT_SECTIONS,
    },
    RhythmSong {
        track_name: "Overclocked_Tempo",
        display_name: "Overclocked Tempo (BPM 152)",
        duration_ms: 177_476,
        beat_times_ms: beats::OVERCLOCKED_TEMPO_BEATS_MS,
        sections: OVERCLOCKED_TEMPO_SECTIONS,
    },
];

/// 指定ビートインデックスが属するセクションの密度を返す
fn density_at(sections: &[(usize, SectionDensity)], beat_index: usize) -> SectionDensity {
    // 後ろから探すことで、開始インデックスが同じ(=ビートを含まない)区間は後の区間が優先される
    sections
        .iter()
        .rev()
        .find(|(start, _)| *start <= beat_index)
        .map(|(_, density)| *density)
        .unwrap_or(SectionDensity::Low)
}

/// 1ビートごとのノーツ配置の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepPattern {
    /// 休符(何も配置しない)
    Skip,
    /// このビート位置に単押し
    Single,
    /// このビート位置+次のビートとの中間点(8分音符)にも単押し
    SingleAndEighth,
    /// このビート位置に同時押し(2レーン)
    Jump,
    /// このビート位置に同時押し+次のビートとの中間点に単押し
    JumpAndEighth,
}

/// セクション密度・ビートインデックスから、そのビートの配置を決める純粋関数
///
/// 方針: 同時押しは控えめにし、片足(単押し)の8分音符連打を主体にする
/// - Lowは休符を残し、単押しのみ(「3拍踏んで1拍休む」)
/// - Midは8分音符を混ぜる(2拍に1回)
/// - Highは8分音符の連続。8拍フレーズの頭だけ同時押しで区切りを付ける
/// - Extremeは最高難度。休符なしで全ビート8分音符の単押し連打。4拍目だけ同時押しで畳みかける
/// - beat_indexは曲全体での通し番号(4拍・8拍周期の基準に使う)
fn step_pattern_for(density: SectionDensity, beat_index: usize) -> StepPattern {
    use SectionDensity::{Extreme, High, Low, Mid};
    use StepPattern::{Jump, JumpAndEighth, Single, SingleAndEighth, Skip};
    match density {
        Low => {
            if beat_index % 4 == 3 {
                Skip
            } else {
                Single
            }
        }
        Mid => {
            if beat_index.is_multiple_of(2) {
                SingleAndEighth
            } else {
                Single
            }
        }
        // 8拍フレーズの頭だけ同時押しで区切りを付け、残りは8分音符の単押し連打にする
        High => match beat_index % 8 {
            0 => Jump,
            _ => SingleAndEighth,
        },
        // 休符を作らず毎拍8分音符の単押し連打。4拍目だけ同時押しで畳みかける
        Extreme => {
            if beat_index % 4 == 3 {
                JumpAndEighth
            } else {
                SingleAndEighth
            }
        }
    }
}

/// 2つのビート時刻の中間点(8分音符の位置)。実測ビートは等間隔ではないため線形補間で求める
fn eighth_hit_at(beat_ms: u32, next_beat_ms: u32) -> Duration {
    // (a + b) / 2 ms = (a + b) * 500 µs として丸めずに求める
    Duration::from_micros((beat_ms as u64 + next_beat_ms as u64) * 500)
}

/// 同時押しで使うレーンの組(左右・上下)
const JUMP_LANES: [[Lane; 2]; 2] = [[Lane::Left, Lane::Right], [Lane::Up, Lane::Down]];

/// 擬似ランダムの系列を単押しと同時押しで分けるための種
const SINGLE_SEED: u64 = 0x5349_4e47_4c45_0001;
const JUMP_SEED: u64 = 0x4a55_4d50_4c41_0002;

/// 区切りの番号から一意に決まる擬似乱数(SplitMix64の混合関数)。
/// 同じ入力なら常に同じ値を返すので、譜面は毎回同じになる
fn mix64(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// 0..lenから擬似ランダムに1つ選ぶ。prevがあればそれ以外の中から選ぶ(同じものを連続させない)
fn pick_excluding(seed: u64, index: u64, len: usize, prev: Option<usize>) -> usize {
    let r = mix64(seed, index);
    match prev {
        None => (r % len as u64) as usize,
        Some(prev) => {
            // prevを除いたlen-1個の中で選び、prev以上なら1つずらしてprevを飛ばす
            let pick = (r % (len as u64 - 1)) as usize;
            if pick >= prev {
                pick + 1
            } else {
                pick
            }
        }
    }
}

/// 単押し・同時押しのレーンを払い出す
struct LaneCycler {
    /// 直前に払い出した単押しレーン(同じレーンの連打を避けるため)
    last: Option<Lane>,
    /// これまでの単押しの回数(単押しレーンを選ぶ擬似乱数の区切りの番号)
    single_count: usize,
    /// これまでの同時押しの回数(同時押しの組を選ぶ区切りの番号)
    jump_count: usize,
}

impl LaneCycler {
    fn new() -> Self {
        Self {
            last: None,
            single_count: 0,
            jump_count: 0,
        }
    }

    /// 次の単押しレーン。4レーンから擬似ランダムに1つ選ぶ(直前と同じレーンは選ばない)
    fn next_single(&mut self) -> Lane {
        let lanes = Lane::all();
        let prev = self
            .last
            .and_then(|last| lanes.iter().position(|&lane| lane == last));
        let pick = pick_excluding(SINGLE_SEED, self.single_count as u64, lanes.len(), prev);
        self.single_count += 1;
        let lane = lanes[pick];
        self.last = Some(lane);
        lane
    }

    /// 次の同時押しレーン。同時押し1回ごとに、左右・上下のどちらかを擬似ランダムに選ぶ。
    /// 2種しかないので直前と違う組に限ると交互の固定巡回になってしまう。そのため排他はせず、同じ組が偶然続くこともある
    fn next_jump(&mut self) -> Vec<Lane> {
        let pick = (mix64(JUMP_SEED, self.jump_count as u64) % JUMP_LANES.len() as u64) as usize;
        self.jump_count += 1;
        // 同時押しの後の単押しは、どのレーンから始めても連打にはならない
        self.last = None;
        JUMP_LANES[pick].to_vec()
    }
}

/// 曲の実測ビートから譜面を作る純粋関数(レーン選択は区切り番号から決まる擬似ランダムなので毎回同じ譜面になる)
fn generate_chart(song: &RhythmSong) -> Vec<Note> {
    let beats = song.beat_times_ms;
    let mut cycler = LaneCycler::new();
    let mut notes = Vec::new();
    for (i, &beat_ms) in beats.iter().enumerate() {
        let hit_at = Duration::from_millis(beat_ms as u64);
        // ノーツは画面下端からSCROLL_TRAVEL_TIMEかけて判定ラインに届くので、
        // それより前のビートに置くと開始直後に判定ライン付近へいきなり湧いてしまう
        if hit_at < SCROLL_TRAVEL_TIME {
            continue;
        }
        let pattern = step_pattern_for(density_at(song.sections, i), i);
        let eighth_at = beats.get(i + 1).map(|&next| eighth_hit_at(beat_ms, next));
        match pattern {
            StepPattern::Skip => {}
            StepPattern::Single => notes.push(Note::new(vec![cycler.next_single()], hit_at)),
            StepPattern::Jump => notes.push(Note::new(cycler.next_jump(), hit_at)),
            StepPattern::SingleAndEighth | StepPattern::JumpAndEighth => {
                let lanes = if pattern == StepPattern::SingleAndEighth {
                    vec![cycler.next_single()]
                } else {
                    cycler.next_jump()
                };
                notes.push(Note::new(lanes, hit_at));
                // 最後のビートには次のビートが無いので8分音符は付けない
                if let Some(eighth_at) = eighth_at {
                    notes.push(Note::new(vec![cycler.next_single()], eighth_at));
                }
            }
        }
    }
    notes
}

/// 1小節のビート数。全曲4/4拍子を前提にする
const BEATS_PER_MEASURE: usize = 4;

/// 小節の頭(ビートインデックスが4の倍数)のビート時刻を返す
fn measure_line_times_ms(beat_times_ms: &[u32]) -> impl Iterator<Item = u32> + '_ {
    beat_times_ms.iter().step_by(BEATS_PER_MEASURE).copied()
}

/// 時刻hit_atに判定ラインへ届くもの(ノーツ・小節ライン)を、時刻nowにトラックの何行目に描くか。
/// 0行目が判定ライン直下、track_height-1行目が画面下端。まだ出現していない/通り過ぎたものはNone
fn track_row(hit_at: Duration, now: Duration, track_height: usize) -> Option<usize> {
    let remaining_secs = hit_at.as_secs_f64() - now.as_secs_f64();
    let progress = 1.0 - remaining_secs / SCROLL_TRAVEL_TIME.as_secs_f64();
    if !(0.0..=1.05).contains(&progress) {
        return None;
    }
    let row = ((1.0 - progress) * (track_height - 1) as f64).round() as usize;
    Some(row.min(track_height - 1))
}

/// タイミング差から判定ランクを求める(仕様7)。良好ウィンドウ(good)を超えたらNone。
fn best_judgement_for_diff(diff: Duration, windows: JudgeWindows) -> Option<Judgement> {
    if diff <= windows.perfect {
        Some(Judgement::Perfect)
    } else if diff <= windows.great {
        Some(Judgement::Great)
    } else if diff <= windows.good {
        Some(Judgement::Good)
    } else {
        None
    }
}

/// 1つのノーツ。同時押しを表現できるよう複数レーンを持てる設計にする(仕様10)。
#[derive(Debug, Clone)]
struct Note {
    lanes: Vec<Lane>,
    /// 判定ラインに到達すべき予定時刻(ゲーム開始からの経過時間基準)
    hit_at: Duration,
    /// 既に正しく踏んだレーンとその判定・タイミング差
    pressed: Vec<(Lane, Judgement, Duration)>,
    /// ノーツ全体の最終判定。Some になったら以後は判定対象から外れる
    judgement: Option<Judgement>,
}

impl Note {
    fn new(lanes: Vec<Lane>, hit_at: Duration) -> Self {
        Self {
            lanes,
            hit_at,
            pressed: Vec::new(),
            judgement: None,
        }
    }

    fn is_judged(&self) -> bool {
        self.judgement.is_some()
    }
}

/// 1回のキー入力に対する結果
struct PressOutcome {
    /// このキー入力自体の精度(表示用)
    press_judgement: Judgement,
    /// この入力でノーツ全体が確定した場合の(最終判定, その判定を決めたレーンのタイミング差)
    note_final: Option<(Judgement, Duration)>,
}

/// 指定レーンへの入力を、未判定ノーツ群に対して判定する純粋関数。
/// 判定ウィンドウ内に対象レーン・未踏の候補が無ければNone(=空打ち)。
/// 複数候補がある場合は最も近いノーツを優先する。
fn process_press(
    notes: &mut [Note],
    lane: Lane,
    pressed_at: Duration,
    windows: JudgeWindows,
) -> Option<PressOutcome> {
    let found = notes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            !n.is_judged()
                && n.lanes.contains(&lane)
                && !n.pressed.iter().any(|(l, _, _)| *l == lane)
        })
        .filter_map(|(i, n)| {
            let diff = n.hit_at.abs_diff(pressed_at);
            best_judgement_for_diff(diff, windows).map(|j| (i, diff, j))
        })
        .min_by_key(|(_, diff, _)| *diff)?;

    let (idx, diff, press_judgement) = found;
    let note = &mut notes[idx];
    note.pressed.push((lane, press_judgement, diff));

    let note_final = if note.pressed.len() == note.lanes.len() {
        // 同時押しは、揃った複数レーンのうち最も悪い判定をノーツ全体の判定として採用する
        let worst = note
            .pressed
            .iter()
            .copied()
            .max_by_key(|(_, j, _)| *j)
            .unwrap();
        note.judgement = Some(worst.1);
        Some((worst.1, worst.2))
    } else {
        None
    };

    Some(PressOutcome {
        press_judgement,
        note_final,
    })
}

pub struct RhythmGame {
    /// SONGSのインデックス(プレイ中の曲)
    song_index: usize,
    tracker: ScoreTracker,
    notes: Vec<Note>,
    /// ゲーム開始時刻。入力判定・描画位置とも、ここからのelapsed()を単一の時間基準として使う(仕様6)。
    started_at: Instant,
    combo: u32,
    max_combo: u32,
    last_judgement: Option<Judgement>,
    last_judgement_at: Duration,
    /// 最初の1小節(先頭4ビート)のカウント音のうち、次に鳴らすビートのインデックス。
    /// 4になったら鳴らし終えている
    next_count_in_beat: usize,
}

impl RhythmGame {
    /// 指定した曲(SONGSのインデックス)の譜面でゲームを作る(常に上級相当の譜面・判定)。
    /// 範囲外のインデックスは先頭の曲として扱う。
    /// 曲との同期のため、呼び出し側はBGM再生の直後に restart_clock を呼ぶこと
    pub fn new(song_index: usize) -> Self {
        let song_index = if song_index < SONGS.len() {
            song_index
        } else {
            0
        };
        Self {
            song_index,
            tracker: ScoreTracker::new(),
            notes: generate_chart(&SONGS[song_index]),
            started_at: Instant::now(),
            combo: 0,
            max_combo: 0,
            last_judgement: None,
            last_judgement_at: Duration::ZERO,
            next_count_in_beat: 0,
        }
    }

    pub fn song(&self) -> &'static RhythmSong {
        &SONGS[self.song_index]
    }

    /// ゲーム内時計(started_at)を現在時刻に合わせ直す。
    /// 譜面生成(new)とBGM再生開始の間の時間差を消すため、BGM再生を始めた直後に呼ぶ
    pub fn restart_clock(&mut self) {
        self.started_at = Instant::now();
    }

    /// テスト用: `started_at` を過去にずらすことで経過時間を擬似的に進める
    #[cfg(test)]
    fn set_elapsed_for_test(&mut self, elapsed: Duration) {
        self.started_at = Instant::now() - elapsed;
    }

    fn set_last_judgement(&mut self, judgement: Judgement, now: Duration) {
        self.last_judgement = Some(judgement);
        self.last_judgement_at = now;
    }

    /// 時刻nowの時点で画面に出すべき直近の判定(表示時間を過ぎていればNone)
    fn visible_judgement(&self, now: Duration) -> Option<Judgement> {
        if now.saturating_sub(self.last_judgement_at) <= JUDGEMENT_DISPLAY_HOLD {
            self.last_judgement
        } else {
            None
        }
    }

    /// 上段: 左=コンボ、中央=直近の判定、右=譜面の進み具合。枠の上辺に曲名と難易度(常に上級)
    fn render_header(&self, frame: &mut Frame, area: Rect, judgement: Option<Judgement>) {
        let (difficulty_text, difficulty_color) = theme::difficulty_label(SESSION_DIFFICULTY);
        let block = theme::panel(format!(" ♪ {} ", self.song().display_name)).title(
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
                Constraint::Length(22),
                Constraint::Fill(1),
                Constraint::Length(24),
            ])
            .split(inner);

        // コンボが続いているほど目立つ色にする
        let combo_color = if self.combo >= 10 {
            theme::HIGHLIGHT
        } else if self.combo > 0 {
            theme::ACCENT_STRONG
        } else {
            theme::MUTED
        };
        let combo = Line::from(vec![
            Span::styled(" COMBO ", Style::default().fg(theme::MUTED)),
            Span::styled(
                self.combo.to_string(),
                Style::default()
                    .fg(combo_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  MAX {}", self.max_combo),
                Style::default().fg(theme::MUTED),
            ),
        ]);
        frame.render_widget(Paragraph::new(combo), cols[0]);

        if let Some(j) = judgement {
            let label = Line::from(Span::styled(
                format!("★ {} ★", judgement_label(j)),
                Style::default()
                    .fg(judgement_color(j))
                    .add_modifier(Modifier::BOLD),
            ));
            frame.render_widget(
                Paragraph::new(label).alignment(Alignment::Center),
                cols[1],
            );
        }

        let judged_count = self.notes.iter().filter(|n| n.is_judged()).count() as u32;
        let total = self.notes.len() as u32;
        let progress = Line::from(vec![
            Span::styled(
                theme::progress_bar(judged_count, total, 10),
                Style::default().fg(theme::ACCENT),
            ),
            Span::styled(
                format!(" {judged_count}/{total} "),
                Style::default().fg(theme::TEXT),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(progress).alignment(Alignment::Right),
            cols[2],
        );
    }

    /// 最初の1小節の間、次のカウント音のビート時刻を過ぎていればtick音を鳴らして次のビートへ進める。
    /// ノーツ判定と同じstarted_at基準の時刻nowを使うので、曲頭から流れるBGMのビートと揃う
    fn play_count_in(&mut self, now: Duration) {
        let beats = self.song().beat_times_ms;
        let count_in_beats = &beats[..BEATS_PER_MEASURE.min(beats.len())];
        let Some(&beat_ms) = count_in_beats.get(self.next_count_in_beat) else {
            return;
        };
        if now >= Duration::from_millis(beat_ms as u64) {
            audio::play_tick();
            self.next_count_in_beat += 1;
        }
    }

    /// ノーツ1つが確定した際に、コンボ・スコアへ反映する
    fn apply_note_result(&mut self, judgement: Judgement, latency_ms: f64) {
        let is_correct = judgement != Judgement::Miss;
        if is_correct {
            self.combo += 1;
            self.max_combo = self.max_combo.max(self.combo);
        } else {
            self.combo = 0;
        }
        self.tracker.record(is_correct, latency_ms);
    }
}

impl Game for RhythmGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.is_finished() {
            return;
        }
        let lane = match key.code {
            KeyCode::Left => Lane::Left,
            KeyCode::Down => Lane::Down,
            KeyCode::Up => Lane::Up,
            KeyCode::Right => Lane::Right,
            _ => return,
        };

        let now = self.started_at.elapsed();
        match process_press(&mut self.notes, lane, now, JUDGE_WINDOWS) {
            Some(outcome) => {
                self.set_last_judgement(outcome.press_judgement, now);
                if let Some((final_judgement, diff)) = outcome.note_final {
                    self.apply_note_result(final_judgement, diff.as_millis() as f64);
                }
            }
            None => {
                // 空打ち(仕様8): 未処理ノーツを消費しない。コンボのみ切る
                self.combo = 0;
            }
        }
    }

    fn update(&mut self, _dt: Duration) {
        if self.is_finished() {
            return;
        }
        let now = self.started_at.elapsed();
        self.play_count_in(now);
        let good_window = JUDGE_WINDOWS.good;
        // 良好ウィンドウを過ぎても未判定のノーツはMissとして確定させる(仕様9)
        let newly_missed: Vec<usize> = self
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.is_judged() && now > n.hit_at + good_window)
            .map(|(i, _)| i)
            .collect();
        for idx in newly_missed {
            self.notes[idx].judgement = Some(Judgement::Miss);
            self.set_last_judgement(Judgement::Miss, now);
            self.apply_note_result(Judgement::Miss, good_window.as_millis() as f64);
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let now = self.started_at.elapsed();
        let lanes = Lane::all();
        let judgement = self.visible_judgement(now);

        // 「曲情報・コンボ」「譜面トラック」「操作説明」の3段
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(theme::HUD_HEIGHT),
                Constraint::Min(6),
                Constraint::Length(3),
            ])
            .split(area);
        let (header_area, track_area, footer_area) = (rows[0], rows[1], rows[2]);

        self.render_header(frame, header_area, judgement);

        // 判定が出ている間はトラックの枠をその判定の色に光らせる
        let track_border = judgement.map_or(theme::ACCENT, judgement_color);
        let track_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(track_border));
        let track_inner = track_block.inner(track_area);
        frame.render_widget(track_block, track_area);

        // 1レーン分の表示幅(文字数)。全行でこの幅を守ることでレーンが縦にそろう
        const LANE_CELL: &str = "       ";
        let lane_cell = |symbol: &str| format!("   {symbol}   ");

        let receptors = Line::from(
            lanes
                .iter()
                .map(|lane| {
                    Span::styled(
                        lane_cell(lane.arrow()),
                        Style::default()
                            .fg(lane.color())
                            .add_modifier(Modifier::BOLD),
                    )
                })
                .collect::<Vec<_>>(),
        );
        let judge_line = Line::from(Span::styled(
            "━".repeat(LANE_CELL.len() * lanes.len()),
            Style::default()
                .fg(theme::ACCENT_STRONG)
                .add_modifier(Modifier::BOLD),
        ));

        // TUIの縦セル数を可能な限り使い、位置計算自体は連続量(progress)で行う(仕様15)
        // トラック内部から「矢印」「判定ライン」の2行を除いた分がノーツの流れる高さ
        let reserved_rows: u16 = 2;
        let track_height = track_inner.height.saturating_sub(reserved_rows).max(4) as usize;

        let mut grid = vec![[' '; 4]; track_height];
        for note in &self.notes {
            if note.is_judged() {
                continue;
            }
            let Some(row) = track_row(note.hit_at, now, track_height) else {
                continue;
            };
            for lane in &note.lanes {
                if let Some(lane_idx) = lanes.iter().position(|l| l == lane) {
                    grid[row][lane_idx] = '●';
                }
            }
        }

        // 小節の頭のビート時刻もノーツと同じ行位置計算で、小節ラインを敷く行を求める
        let mut measure_rows = vec![false; track_height];
        for beat_ms in measure_line_times_ms(self.song().beat_times_ms) {
            let hit_at = Duration::from_millis(beat_ms as u64);
            if let Some(row) = track_row(hit_at, now, track_height) {
                measure_rows[row] = true;
            }
        }

        let mut lines = vec![receptors, judge_line];
        for (row_idx, row) in grid.iter().enumerate() {
            let is_measure_row = measure_rows[row_idx];
            let spans: Vec<Span> = row
                .iter()
                .zip(lanes.iter())
                .map(|(&c, lane)| {
                    if c == ' ' && is_measure_row {
                        // 小節ラインの行は、ノーツの無いマスを罫線でつないで4レーンを貫く横線にする
                        Span::styled(
                            "─".repeat(LANE_CELL.len()),
                            Style::default().fg(theme::MUTED),
                        )
                    } else if c == ' ' {
                        // ノーツの無いマスはレーンの目印として薄い点を置く
                        Span::styled(lane_cell("·"), Style::default().fg(theme::MUTED))
                    } else {
                        let mut style = Style::default().fg(lane.color());
                        // 判定ライン直前のノーツは太字にして踏むタイミングを目立たせる
                        if row_idx <= 1 {
                            style = style.add_modifier(Modifier::BOLD);
                        }
                        Span::styled(lane_cell(&c.to_string()), style)
                    }
                })
                .collect();
            lines.push(Line::from(spans));
        }
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            track_inner,
        );

        theme::render_hint_footer(
            frame,
            footer_area,
            &[("← ↑ ↓ →", "判定ラインで踏む"), ("q", "終了")],
        );
    }

    /// 全ノーツが判定済みで、かつ曲を最後まで再生し終えたら終了。
    /// 最後のノーツを踏んだ直後に曲を打ち切ってリザルトへ進まないよう、曲の長さも条件にする
    fn is_finished(&self) -> bool {
        let song_end = Duration::from_millis(self.song().duration_ms as u64);
        self.notes.iter().all(|n| n.is_judged()) && self.started_at.elapsed() >= song_end
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, SESSION_DIFFICULTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(lanes: &[Lane], hit_at_ms: u64) -> Note {
        Note::new(lanes.to_vec(), Duration::from_millis(hit_at_ms))
    }

    // --- 表示ラベル/色 ---

    #[test]
    fn judgement_label_and_color_are_defined_for_every_rank() {
        for j in [
            Judgement::Perfect,
            Judgement::Great,
            Judgement::Good,
            Judgement::Miss,
        ] {
            assert!(!judgement_label(j).is_empty());
            // 各ランクで別々の色が割り当てられていることを確認する
        }
        let colors = [
            judgement_color(Judgement::Perfect),
            judgement_color(Judgement::Great),
            judgement_color(Judgement::Good),
            judgement_color(Judgement::Miss),
        ];
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(colors[i], colors[j], "判定ランクごとに異なる色にすること");
            }
        }
    }

    // --- 直近判定の表示時間(visible_judgement) ---

    #[test]
    fn visible_judgement_is_none_before_any_judgement() {
        let game = RhythmGame::new(0);
        assert_eq!(game.visible_judgement(Duration::ZERO), None);
    }

    #[test]
    fn visible_judgement_holds_for_display_time_then_disappears() {
        let mut game = RhythmGame::new(0);
        let at = Duration::from_secs(5);
        game.set_last_judgement(Judgement::Great, at);
        assert_eq!(game.visible_judgement(at), Some(Judgement::Great));
        assert_eq!(
            game.visible_judgement(at + JUDGEMENT_DISPLAY_HOLD),
            Some(Judgement::Great),
            "表示時間ちょうどまでは表示する"
        );
        assert_eq!(
            game.visible_judgement(at + JUDGEMENT_DISPLAY_HOLD + Duration::from_millis(1)),
            None
        );
    }

    #[test]
    fn lane_colors_distinguish_horizontal_and_vertical_lanes() {
        assert_eq!(Lane::Left.color(), Lane::Right.color());
        assert_eq!(Lane::Up.color(), Lane::Down.color());
        assert_ne!(Lane::Left.color(), Lane::Up.color());
        // 矢印は4レーンで全部異なる
        let arrows: Vec<&str> = Lane::all().iter().map(|l| l.arrow()).collect();
        assert_eq!(arrows, vec!["←", "↓", "↑", "→"]);
    }

    // --- 判定ランク(best_judgement_for_diff) ---

    #[test]
    fn judge_windows_are_the_former_advanced_values() {
        // 難易度選択が無くなり、常に旧上級の判定幅(perfect35/great70/good120)で判定する
        assert_eq!(JUDGE_WINDOWS.perfect, Duration::from_millis(35));
        assert_eq!(JUDGE_WINDOWS.great, Duration::from_millis(70));
        assert_eq!(JUDGE_WINDOWS.good, Duration::from_millis(120));
    }

    #[test]
    fn best_judgement_boundaries() {
        let w = JUDGE_WINDOWS;
        assert_eq!(
            best_judgement_for_diff(Duration::from_millis(35), w),
            Some(Judgement::Perfect)
        );
        assert_eq!(
            best_judgement_for_diff(Duration::from_millis(36), w),
            Some(Judgement::Great)
        );
        assert_eq!(
            best_judgement_for_diff(Duration::from_millis(70), w),
            Some(Judgement::Great)
        );
        assert_eq!(
            best_judgement_for_diff(Duration::from_millis(71), w),
            Some(Judgement::Good)
        );
        assert_eq!(
            best_judgement_for_diff(Duration::from_millis(120), w),
            Some(Judgement::Good)
        );
        assert_eq!(best_judgement_for_diff(Duration::from_millis(121), w), None);
    }

    // --- 秒/ミリ秒 → ビートインデックス変換 ---

    const SAMPLE_BEATS: &[u32] = &[500, 1000, 1500, 2000, 2500];

    #[test]
    fn beat_index_at_or_after_ms_returns_first_beat_not_before_given_time() {
        assert_eq!(beat_index_at_or_after_ms(SAMPLE_BEATS, 0), 0);
        assert_eq!(beat_index_at_or_after_ms(SAMPLE_BEATS, 499), 0);
        // ちょうどビート時刻と一致する場合はそのビート自身(以降に含める)
        assert_eq!(beat_index_at_or_after_ms(SAMPLE_BEATS, 500), 0);
        assert_eq!(beat_index_at_or_after_ms(SAMPLE_BEATS, 501), 1);
        assert_eq!(beat_index_at_or_after_ms(SAMPLE_BEATS, 1999), 3);
        assert_eq!(beat_index_at_or_after_ms(SAMPLE_BEATS, 2000), 3);
        assert_eq!(beat_index_at_or_after_ms(SAMPLE_BEATS, 2500), 4);
    }

    #[test]
    fn beat_index_at_or_after_ms_past_last_beat_is_len() {
        assert_eq!(
            beat_index_at_or_after_ms(SAMPLE_BEATS, 2501),
            SAMPLE_BEATS.len()
        );
        assert_eq!(beat_index_at_or_after_ms(&[], 0), 0);
    }

    #[test]
    fn beat_index_at_or_after_secs_converts_seconds_to_ms() {
        assert_eq!(beat_index_at_or_after_secs(SAMPLE_BEATS, 0), 0);
        assert_eq!(beat_index_at_or_after_secs(SAMPLE_BEATS, 1), 1);
        assert_eq!(beat_index_at_or_after_secs(SAMPLE_BEATS, 2), 3);
        assert_eq!(
            beat_index_at_or_after_secs(SAMPLE_BEATS, 3),
            SAMPLE_BEATS.len()
        );
    }

    #[test]
    fn leaderboard_high_section_starts_at_first_beat_after_90s() {
        let song = SONGS
            .iter()
            .find(|s| s.track_name == "Top_of_the_Leaderboard")
            .unwrap();
        let (start, density) = song.sections[4];
        assert_eq!(density, SectionDensity::High);
        assert!(song.beat_times_ms[start] >= 90_000);
        assert!(song.beat_times_ms[start - 1] < 90_000);
        // 170秒以降のLow区間は最後のビート(約169.7秒)より後なので、ビートを1つも含まない
        assert_eq!(song.sections.last().unwrap().0, song.beat_times_ms.len());
    }

    #[test]
    fn redline_first_beat_is_after_intro_and_first_section_starts_at_zero() {
        // Redline_Response_Timeは約5.7秒のイントロの後に最初のビートが来る
        let redline = SONGS
            .iter()
            .find(|s| s.track_name == "Redline_Response_Time")
            .unwrap();
        assert!(redline.beat_times_ms[0] > 5_000);
        // 0秒〜20秒のLow区間はビート0から始まる
        assert_eq!(redline.sections[0], (0, SectionDensity::Low));
        // 20秒のMid区間は20000ms以降の最初のビートから
        let mid_start = redline.sections[1].0;
        assert!(redline.beat_times_ms[mid_start] >= 20_000);
        assert!(redline.beat_times_ms[mid_start - 1] < 20_000);
        assert_eq!(redline.sections[1].1, SectionDensity::Mid);
    }

    // --- 曲データの健全性 ---

    #[test]
    fn songs_have_expected_beat_counts_and_sorted_beats() {
        let counts: Vec<(&str, usize)> = SONGS
            .iter()
            .map(|s| (s.track_name, s.beat_times_ms.len()))
            .collect();
        assert_eq!(
            counts,
            vec![
                ("Top_of_the_Leaderboard", 426),
                ("Redline_Response_Time", 504),
                ("Apex_Movement", 439),
                ("Overclocked_Tempo", 433)
            ]
        );
        for song in SONGS {
            for pair in song.beat_times_ms.windows(2) {
                assert!(
                    pair[0] < pair[1],
                    "{}: ビート時刻は狭義単調増加",
                    song.track_name
                );
            }
        }
    }

    #[test]
    fn songs_sections_start_at_zero_and_are_sorted() {
        for song in SONGS {
            assert_eq!(
                song.sections[0].0, 0,
                "{}: 先頭区間は0から",
                song.track_name
            );
            for pair in song.sections.windows(2) {
                assert!(pair[0].0 <= pair[1].0, "{}: 区間は昇順", song.track_name);
            }
        }
    }

    #[test]
    fn songs_track_names_exist_in_rhythm_bgm_assets() {
        let tracks = crate::audio::bgm_tracks_in(crate::audio::BgmCategory::Rhythm);
        for song in SONGS {
            assert!(
                tracks.iter().any(|t| t == song.track_name),
                "{}がassets/audio/bgm/rhythm/にあること: {tracks:?}",
                song.track_name
            );
            assert!(!song.display_name.is_empty());
        }
    }

    /// 曲の全ビートのうち、指定した密度の区間に属するビートがあるか
    fn song_uses_density(song: &RhythmSong, d: SectionDensity) -> bool {
        (0..song.beat_times_ms.len()).any(|i| density_at(song.sections, i) == d)
    }

    fn song_by_track(track_name: &str) -> &'static RhythmSong {
        SONGS.iter().find(|s| s.track_name == track_name).unwrap()
    }

    #[test]
    fn songs_are_listed_in_order_with_overclocked_tempo_last() {
        let names: Vec<&str> = SONGS.iter().map(|s| s.track_name).collect();
        assert_eq!(
            names,
            vec![
                "Top_of_the_Leaderboard",
                "Redline_Response_Time",
                "Apex_Movement",
                "Overclocked_Tempo"
            ]
        );
        assert_eq!(
            song_by_track("Apex_Movement").display_name,
            "Apex Movement (BPM 152)"
        );
        assert_eq!(
            song_by_track("Overclocked_Tempo").display_name,
            "Overclocked Tempo (BPM 152)"
        );
    }

    #[test]
    fn overclocked_tempo_uses_every_density() {
        let song = song_by_track("Overclocked_Tempo");
        for d in [
            SectionDensity::Low,
            SectionDensity::Mid,
            SectionDensity::High,
            SectionDensity::Extreme,
        ] {
            assert!(
                song_uses_density(song, d),
                "Overclocked_Tempo: {d:?}区間を含むこと"
            );
        }
    }

    #[test]
    fn overclocked_tempo_sections_follow_rms_analysis() {
        use SectionDensity::{Extreme, High, Low, Mid};
        let song = song_by_track("Overclocked_Tempo");
        // (区間開始秒, 密度)。RMSエネルギー解析の区分そのもの
        let expected = [
            (0, Low),
            (10, Mid),
            (50, Low),
            (60, Mid),
            (90, Extreme),
            (115, Mid),
            (140, Low),
            (145, High),
            (150, Extreme),
            (170, Low),
        ];
        assert_eq!(song.sections.len(), expected.len());
        assert_eq!(song.sections[0], (0, Low));
        for (&(start, density), &(secs, expected_density)) in song.sections.iter().zip(&expected) {
            assert_eq!(density, expected_density, "{secs}秒の区間");
            let ms = secs * 1000;
            // 区間はその秒数「以降」で最初のビートから始まる
            assert!(song.beat_times_ms[start] >= ms, "{secs}秒の区間");
            if start > 0 {
                assert!(song.beat_times_ms[start - 1] < ms, "{secs}秒の区間");
            }
        }
        // 170秒以降の終息区間は最後のビート(約172.9秒)より前に始まるので、ビートを含む
        let (outro_start, _) = *song.sections.last().unwrap();
        assert!(outro_start < song.beat_times_ms.len());
        assert_eq!(density_at(song.sections, song.beat_times_ms.len() - 1), Low);
        let at = |ms: u32| {
            density_at(
                song.sections,
                beat_index_at_or_after_ms(song.beat_times_ms, ms),
            )
        };
        assert_eq!(at(5_000), Low);
        assert_eq!(at(30_000), Mid);
        assert_eq!(at(55_000), Low);
        assert_eq!(at(100_000), Extreme);
        assert_eq!(at(120_000), Mid);
        assert_eq!(at(142_000), Low);
        assert_eq!(at(147_000), High);
        assert_eq!(at(160_000), Extreme);
        assert_eq!(at(171_000), Low);
    }

    #[test]
    fn existing_songs_use_low_mid_high_and_never_extreme() {
        for track in ["Top_of_the_Leaderboard", "Redline_Response_Time"] {
            let song = song_by_track(track);
            for d in [
                SectionDensity::Low,
                SectionDensity::Mid,
                SectionDensity::High,
            ] {
                assert!(song_uses_density(song, d), "{track}: {d:?}区間を含むこと");
            }
            assert!(
                !song_uses_density(song, SectionDensity::Extreme),
                "{track}: 既存曲はExtremeを使わない"
            );
        }
    }

    #[test]
    fn apex_movement_uses_low_mid_and_extreme() {
        let song = song_by_track("Apex_Movement");
        for d in [
            SectionDensity::Low,
            SectionDensity::Mid,
            SectionDensity::Extreme,
        ] {
            assert!(
                song_uses_density(song, d),
                "Apex_Movement: {d:?}区間を含むこと"
            );
        }
    }

    #[test]
    fn apex_movement_sections_follow_rms_analysis() {
        use SectionDensity::{Extreme, Low, Mid};
        let song = song_by_track("Apex_Movement");
        // (区間開始秒, 密度)。RMSエネルギー解析の区分そのもの
        let expected = [
            (0, Low),
            (30, Mid),
            (40, Low),
            (50, Extreme),
            (100, Low),
            (120, Mid),
            (130, Extreme),
            (170, Low),
        ];
        assert_eq!(song.sections.len(), expected.len());
        assert_eq!(song.sections[0], (0, Low));
        for (&(start, density), &(secs, expected_density)) in song.sections.iter().zip(&expected) {
            assert_eq!(density, expected_density, "{secs}秒の区間");
            let ms = secs * 1000;
            // 区間はその秒数「以降」で最初のビートから始まる
            assert!(song.beat_times_ms[start] >= ms, "{secs}秒の区間");
            if start > 0 {
                assert!(song.beat_times_ms[start - 1] < ms, "{secs}秒の区間");
            }
        }
        // 170秒以降のアウトロは最後のビート(約174.9秒)より前に始まるので、ビートを含む
        let (outro_start, _) = *song.sections.last().unwrap();
        assert!(outro_start < song.beat_times_ms.len());
        assert_eq!(density_at(song.sections, song.beat_times_ms.len() - 1), Low);
        // 50-100秒・130-170秒の最高密度区間の中はExtreme
        let at = |ms: u32| {
            density_at(
                song.sections,
                beat_index_at_or_after_ms(song.beat_times_ms, ms),
            )
        };
        assert_eq!(at(50_000), Extreme);
        assert_eq!(at(99_000), Extreme);
        assert_eq!(at(100_000), Low);
        assert_eq!(at(130_000), Extreme);
        assert_eq!(at(169_500), Extreme);
        assert_eq!(at(170_000), Low);
    }

    // --- セクション密度の決定(density_at) ---

    const SAMPLE_SECTIONS: &[(usize, SectionDensity)] = &[
        (0, SectionDensity::Low),
        (4, SectionDensity::Mid),
        (8, SectionDensity::High),
        (12, SectionDensity::Low),
    ];

    #[test]
    fn density_at_returns_section_containing_beat() {
        assert_eq!(density_at(SAMPLE_SECTIONS, 0), SectionDensity::Low);
        assert_eq!(density_at(SAMPLE_SECTIONS, 3), SectionDensity::Low);
        assert_eq!(density_at(SAMPLE_SECTIONS, 5), SectionDensity::Mid);
        assert_eq!(density_at(SAMPLE_SECTIONS, 9), SectionDensity::High);
        // 最終区間は終端まで続く
        assert_eq!(density_at(SAMPLE_SECTIONS, 1000), SectionDensity::Low);
    }

    #[test]
    fn density_at_boundary_beat_belongs_to_new_section() {
        assert_eq!(density_at(SAMPLE_SECTIONS, 4), SectionDensity::Mid);
        assert_eq!(density_at(SAMPLE_SECTIONS, 8), SectionDensity::High);
        assert_eq!(density_at(SAMPLE_SECTIONS, 7), SectionDensity::Mid);
        assert_eq!(density_at(SAMPLE_SECTIONS, 12), SectionDensity::Low);
    }

    #[test]
    fn density_at_empty_section_is_overridden_by_later_one_with_same_start() {
        // ビートを1つも含まない区間(開始インデックスが次区間と同じ)は無視され、後の区間が採用される
        let sections = &[
            (0, SectionDensity::Low),
            (4, SectionDensity::High),
            (4, SectionDensity::Mid),
        ];
        assert_eq!(density_at(sections, 4), SectionDensity::Mid);
        assert_eq!(density_at(sections, 3), SectionDensity::Low);
    }

    #[test]
    fn density_at_with_no_sections_defaults_to_low() {
        assert_eq!(density_at(&[], 10), SectionDensity::Low);
    }

    // --- 密度ごとの配置(step_pattern_for) ---

    const ALL_DENSITIES: [SectionDensity; 4] = [
        SectionDensity::Low,
        SectionDensity::Mid,
        SectionDensity::High,
        SectionDensity::Extreme,
    ];

    /// 1ビートの配置で踏むキーの回数(同時押しは2回、8分音符付きは+1、8分音符も同時押しなら+2)
    fn presses(p: StepPattern) -> usize {
        match p {
            StepPattern::Skip => 0,
            StepPattern::Single => 1,
            StepPattern::SingleAndEighth => 2,
            StepPattern::Jump => 2,
            StepPattern::JumpAndEighth => 3,
        }
    }

    /// 8拍分の同時押しの回数
    fn jumps_per_8_beats(density: SectionDensity) -> usize {
        (0..8)
            .filter(|&i| {
                matches!(
                    step_pattern_for(density, i),
                    StepPattern::Jump | StepPattern::JumpAndEighth
                )
            })
            .count()
    }

    /// 8拍分(フレーズ2小節分)の合計打鍵数
    fn presses_per_8_beats(density: SectionDensity) -> usize {
        (0..8).map(|i| presses(step_pattern_for(density, i))).sum()
    }

    #[test]
    fn step_pattern_jumps_once_per_eight_beats_in_high() {
        // 同時押しは8拍フレーズの頭だけ。残りは片足(単押し)の8分音符連打にする
        for i in 0..16 {
            let p = step_pattern_for(SectionDensity::High, i);
            if i % 8 == 0 {
                assert_eq!(p, StepPattern::Jump, "beat{i}");
            } else {
                assert_eq!(p, StepPattern::SingleAndEighth, "beat{i}");
            }
        }
        // 同時押しはHigh/Extremeだけ
        for density in [SectionDensity::Low, SectionDensity::Mid] {
            for i in 0..16 {
                let p = step_pattern_for(density, i);
                assert!(!matches!(p, StepPattern::Jump | StepPattern::JumpAndEighth));
            }
        }
    }

    #[test]
    fn step_pattern_extreme_fills_every_beat_with_eighth_singles_and_jumps_on_the_fourth() {
        // 休符なし・片足の8分音符連打で埋め尽くす。4拍目だけ同時押しで畳みかける
        for i in 0..32 {
            let p = step_pattern_for(SectionDensity::Extreme, i);
            if i % 4 == 3 {
                assert_eq!(p, StepPattern::JumpAndEighth, "beat{i}");
            } else {
                assert_eq!(p, StepPattern::SingleAndEighth, "beat{i}");
            }
        }
    }

    #[test]
    fn step_pattern_extreme_is_clearly_denser_than_high() {
        let high = presses_per_8_beats(SectionDensity::High);
        let extreme = presses_per_8_beats(SectionDensity::Extreme);
        // 打鍵数はHigh以上(片足連打主体なので大きな倍率は求めない)
        assert!(extreme >= high, "High {high} / Extreme {extreme}");
        let high_jumps = jumps_per_8_beats(SectionDensity::High);
        let extreme_jumps = jumps_per_8_beats(SectionDensity::Extreme);
        // 同時押しの頻度ははっきりExtremeの方が多い(片足連打主体でも密度の違いは付ける)
        assert!(
            extreme_jumps > high_jumps,
            "同時押し High {high_jumps} / Extreme {extreme_jumps}"
        );
    }

    #[test]
    fn step_pattern_jump_only_appears_in_high_and_extreme() {
        for density in [SectionDensity::Low, SectionDensity::Mid] {
            for i in 0..32 {
                assert!(
                    !matches!(
                        step_pattern_for(density, i),
                        StepPattern::Jump | StepPattern::JumpAndEighth
                    ),
                    "{density:?} beat{i}"
                );
            }
        }
    }

    #[test]
    fn step_pattern_mid_mixes_eighth_notes() {
        let has_eighth = (0..8)
            .any(|i| step_pattern_for(SectionDensity::Mid, i) == StepPattern::SingleAndEighth);
        assert!(has_eighth, "Midは8分音符を含む");
    }

    #[test]
    fn step_pattern_low_sections_keep_rests() {
        // Lowは「3拍踏んで1拍休む」
        let rests = (0..8)
            .filter(|&i| step_pattern_for(SectionDensity::Low, i) == StepPattern::Skip)
            .count();
        assert_eq!(rests, 2, "Lowは8拍中2拍が休符");
        for i in 0..8 {
            let expected = if i % 4 == 3 {
                StepPattern::Skip
            } else {
                StepPattern::Single
            };
            // Lowでは8分音符・同時押しを使わない
            assert_eq!(step_pattern_for(SectionDensity::Low, i), expected, "beat{i}");
        }
    }

    #[test]
    fn step_pattern_gets_busier_with_density() {
        let counts: Vec<usize> = ALL_DENSITIES
            .iter()
            .map(|&d| presses_per_8_beats(d))
            .collect();
        let (l, m, h, e) = (counts[0], counts[1], counts[2], counts[3]);
        assert!(l < m && m <= h && h < e, "{l} < {m} <= {h} < {e}");
    }

    // --- 8分音符の時刻 ---

    #[test]
    fn eighth_hit_at_is_midpoint_of_two_beats() {
        assert_eq!(eighth_hit_at(400, 800), Duration::from_millis(600));
        // 奇数和は0.5ms単位まで正確に
        assert_eq!(eighth_hit_at(93, 488), Duration::from_micros(290_500));
        // 不等間隔の実測ビートでも線形補間の中点
        assert_eq!(eighth_hit_at(1000, 1350), Duration::from_millis(1175));
    }

    // --- 譜面生成(generate_chart) ---

    /// テスト用の小さな曲。ビートは2秒目から400ms間隔で16個
    const TEST_BEATS: &[u32] = &[
        2000, 2400, 2800, 3200, 3600, 4000, 4400, 4800, 5200, 5600, 6000, 6400, 6800, 7200, 7600,
        8000,
    ];
    const TEST_SONG: RhythmSong = RhythmSong {
        track_name: "test",
        display_name: "test",
        duration_ms: 9000,
        beat_times_ms: TEST_BEATS,
        sections: &[(0, SectionDensity::High)],
    };

    fn chart_press_count(notes: &[Note]) -> usize {
        notes.iter().map(|n| n.lanes.len()).sum()
    }

    /// TEST_SONGと同じビートで、全区間を指定の密度にした曲
    const fn test_song_with(sections: &'static [(usize, SectionDensity)]) -> RhythmSong {
        RhythmSong {
            track_name: "test",
            display_name: "test",
            duration_ms: 9000,
            beat_times_ms: TEST_BEATS,
            sections,
        }
    }
    const TEST_SONG_LOW: RhythmSong = test_song_with(&[(0, SectionDensity::Low)]);
    const TEST_SONG_MID: RhythmSong = test_song_with(&[(0, SectionDensity::Mid)]);

    #[test]
    fn generate_chart_places_single_notes_exactly_on_measured_beats() {
        // Lowは「3拍踏んで1拍休む」単押し → ノーツ時刻=休符以外の実測ビート時刻そのもの
        let notes = generate_chart(&TEST_SONG_LOW);
        let times: Vec<Duration> = notes.iter().map(|n| n.hit_at).collect();
        let expected: Vec<Duration> = TEST_BEATS
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 4 != 3)
            .map(|(_, &ms)| Duration::from_millis(ms as u64))
            .collect();
        assert_eq!(times, expected);
        assert!(notes.iter().all(|n| n.lanes.len() == 1));
    }

    #[test]
    fn generate_chart_eighth_notes_sit_between_beats_and_not_after_last_beat() {
        // Midは偶数拍に8分音符が付く。最後のビートには次のビートが無いので8分音符を付けない
        let notes = generate_chart(&TEST_SONG_MID);
        assert_eq!(notes.len(), TEST_BEATS.len() + TEST_BEATS.len() / 2);
        assert_eq!(notes[0].hit_at, Duration::from_millis(2000));
        assert_eq!(notes[1].hit_at, Duration::from_millis(2200));
        assert_eq!(notes[2].hit_at, Duration::from_millis(2400));
        assert_eq!(notes[3].hit_at, Duration::from_millis(2800));
        assert_eq!(notes[4].hit_at, Duration::from_millis(3000));
        let last_beat = Duration::from_millis(*TEST_BEATS.last().unwrap() as u64);
        assert!(notes.iter().all(|n| n.hit_at <= last_beat));
        // Extremeは全ビートに8分音符が付くが、最後のビートだけは付かない
        let extreme = generate_chart(&TEST_SONG_EXTREME);
        assert_eq!(extreme.len(), TEST_BEATS.len() * 2 - 1);
        assert!(extreme.iter().all(|n| n.hit_at <= last_beat));
    }

    #[test]
    fn generate_chart_jumps_use_two_distinct_lanes() {
        let notes = generate_chart(&TEST_SONG);
        let jumps: Vec<&Note> = notes.iter().filter(|n| n.lanes.len() > 1).collect();
        assert!(!jumps.is_empty());
        for j in jumps {
            assert_eq!(j.lanes.len(), 2);
            assert_ne!(j.lanes[0], j.lanes[1]);
        }
    }

    // --- レーン選択の擬似ランダム化(LaneCycler) ---

    /// 列が周期pで繰り返しているか(固定順の巡回になっていないかの判定に使う)
    fn is_periodic<T: PartialEq>(seq: &[T], p: usize) -> bool {
        seq.len() > p && (p..seq.len()).all(|i| seq[i] == seq[i - p])
    }

    /// next_singleだけをcount回呼んだときの単押しレーンの並び
    fn single_sequence(count: usize) -> Vec<Lane> {
        let mut cycler = LaneCycler::new();
        (0..count).map(|_| cycler.next_single()).collect()
    }

    fn jump_sequence(count: usize) -> Vec<Vec<Lane>> {
        let mut cycler = LaneCycler::new();
        (0..count).map(|_| cycler.next_jump()).collect()
    }

    /// 単押しの並びに現れる「直前→次」のレーンの組(重複なし)
    fn single_transitions(seq: &[Lane]) -> Vec<(Lane, Lane)> {
        let mut found: Vec<(Lane, Lane)> = Vec::new();
        for pair in seq.windows(2) {
            let t = (pair[0], pair[1]);
            if !found.contains(&t) {
                found.push(t);
            }
        }
        found
    }

    #[test]
    fn single_selection_uses_every_lane() {
        let seq = single_sequence(64);
        for lane in Lane::all() {
            assert!(seq.contains(&lane), "{lane:?}が一度も選ばれない: {seq:?}");
        }
    }

    #[test]
    fn single_selection_never_repeats_previous_lane() {
        let seq = single_sequence(500);
        for (i, pair) in seq.windows(2).enumerate() {
            assert_ne!(pair[0], pair[1], "{i}回目と{}回目が同じレーン", i + 1);
        }
    }

    #[test]
    fn single_selection_is_not_a_fixed_rotation() {
        // 決まった順の巡回(周期2〜4の繰り返し)になっていない
        let seq = single_sequence(64);
        for p in 2..=4 {
            assert!(!is_periodic(&seq, p), "周期{p}で繰り返している: {seq:?}");
        }
    }

    #[test]
    fn single_selection_produces_every_lane_transition() {
        // 固定フレーズ(歩く/左右交互/上下往復)では出ない ←→↑ のような組も含め、
        // 異なる2レーンの「直前→次」12通りが全て現れる
        let transitions = single_transitions(&single_sequence(200));
        for from in Lane::all() {
            for to in Lane::all() {
                if from != to {
                    assert!(
                        transitions.contains(&(from, to)),
                        "{from:?}→{to:?}が一度も現れない"
                    );
                }
            }
        }
    }

    #[test]
    fn single_selection_is_roughly_uniform_across_lanes() {
        // 4レーンから偏りなく選ぶ(各レーンの出現割合が15%〜35%の範囲)
        let seq = single_sequence(400);
        for lane in Lane::all() {
            let count = seq.iter().filter(|&&l| l == lane).count();
            assert!(
                (60..=140).contains(&count),
                "{lane:?}の出現回数{count}/400が偏っている"
            );
        }
    }

    #[test]
    fn single_selection_is_deterministic() {
        assert_eq!(single_sequence(200), single_sequence(200));
    }

    #[test]
    fn jump_selection_is_independent_of_single_selection() {
        // 単押しを間に挟んでも、同時押しの組の並びは同時押しだけを呼んだときと変わらない
        let mut cycler = LaneCycler::new();
        let interleaved: Vec<Vec<Lane>> = (0..64)
            .map(|i| {
                for _ in 0..(i % 3) {
                    cycler.next_single();
                }
                cycler.next_jump()
            })
            .collect();
        assert_eq!(interleaved, jump_sequence(64));
    }

    #[test]
    fn real_song_charts_single_lanes_use_every_transition() {
        // 全曲の譜面で、連続する単押しどうしの「直前→次」12通りが全て現れる
        // (同時押しを挟んだ単押しどうしは連続とみなさない)
        for song in SONGS {
            let notes = generate_chart(song);
            let mut transitions: Vec<(Lane, Lane)> = Vec::new();
            for pair in notes.windows(2) {
                if pair[0].lanes.len() == 1 && pair[1].lanes.len() == 1 {
                    let t = (pair[0].lanes[0], pair[1].lanes[0]);
                    if !transitions.contains(&t) {
                        transitions.push(t);
                    }
                }
            }
            assert_eq!(transitions.len(), 12, "{}", song.track_name);
        }
    }

    #[test]
    fn jump_selection_uses_a_jump_lane_pair() {
        // 同時押しは常にJUMP_LANES(左右・上下)のどちらかで、2つの異なるレーンを踏む
        for lanes in jump_sequence(200) {
            assert!(
                JUMP_LANES
                    .iter()
                    .any(|pair| pair.as_slice() == lanes.as_slice()),
                "{lanes:?}"
            );
            assert_ne!(lanes[0], lanes[1]);
        }
    }

    #[test]
    fn jump_selection_is_not_a_fixed_alternation() {
        let seq = jump_sequence(64);
        // 左右・上下の両方が使われる
        for pair in JUMP_LANES {
            assert!(
                seq.iter().any(|lanes| lanes.as_slice() == pair.as_slice()),
                "{pair:?}が一度も選ばれない"
            );
        }
        // 0,1,0,1...の固定の交互や、ずっと同じ組にならない
        for p in 1..=2 {
            assert!(!is_periodic(&seq, p), "周期{p}で繰り返している: {seq:?}");
        }
        assert_eq!(seq, jump_sequence(64), "同時押しの選択も決定論的");
    }

    #[test]
    fn real_song_charts_jump_pairs_are_not_a_fixed_alternation() {
        // 全曲の譜面で、同時押しの組が交互の固定巡回になっていない
        for song in SONGS {
            let jumps: Vec<Vec<Lane>> = generate_chart(song)
                .into_iter()
                .filter(|n| n.lanes.len() == 2)
                .map(|n| n.lanes)
                .collect();
            // 片足連打主体になり同時押し数は減ったが、周期性チェックには十分な数がある
            assert!(jumps.len() >= 5, "{}", song.track_name);
            for p in 1..=2 {
                assert!(
                    !is_periodic(&jumps, p),
                    "{}: 周期{p}の固定巡回",
                    song.track_name
                );
            }
        }
    }

    /// TEST_SONGと同じビートで、全区間をExtremeにした曲
    const TEST_SONG_EXTREME: RhythmSong = test_song_with(&[(0, SectionDensity::Extreme)]);

    fn chart_signature(notes: &[Note]) -> Vec<(Duration, Vec<Lane>)> {
        notes.iter().map(|n| (n.hit_at, n.lanes.clone())).collect()
    }

    #[test]
    fn generate_chart_extreme_fills_every_beat_with_eighth_singles_and_jumps_on_the_fourth() {
        let notes = generate_chart(&TEST_SONG_EXTREME);
        // 全ビート+その8分音符(最後のビートは8分音符なし)
        assert_eq!(notes.len(), TEST_BEATS.len() * 2 - 1);
        for (i, &beat_ms) in TEST_BEATS.iter().enumerate() {
            let on_beat = &notes[i * 2];
            assert_eq!(on_beat.hit_at, Duration::from_millis(beat_ms as u64));
            let expected_on_beat_lanes = if i % 4 == 3 { 2 } else { 1 };
            assert_eq!(on_beat.lanes.len(), expected_on_beat_lanes, "beat{i}");
            if expected_on_beat_lanes == 2 {
                assert_ne!(on_beat.lanes[0], on_beat.lanes[1], "beat{i}");
            }
            if let Some(next_ms) = TEST_BEATS.get(i + 1) {
                let eighth = &notes[i * 2 + 1];
                assert_eq!(eighth.hit_at, eighth_hit_at(beat_ms, *next_ms));
                assert_eq!(eighth.lanes.len(), 1, "beat{i}の8分音符は単押し");
            }
        }
    }

    #[test]
    fn generate_chart_extreme_is_clearly_busier_than_high() {
        // 片足連打主体になったため、打鍵数の差は小さい。同時押しの頻度で密度の違いを付けている
        // (jumps_per_8_beatsのテストを参照)ので、ここでは単純に上回ることだけ確認する
        let high = chart_press_count(&generate_chart(&TEST_SONG));
        let extreme = chart_press_count(&generate_chart(&TEST_SONG_EXTREME));
        assert!(extreme > high, "High {high} / Extreme {extreme}");
    }

    #[test]
    fn apex_movement_has_more_total_presses_than_existing_songs() {
        // 片足連打主体になり、Extreme区間のピークの忙しさは他曲のHigh区間と近くなったが、
        // 総打鍵数(曲全体の忙しさ)ではApexが引き続き上回る
        let apex = generate_chart(song_by_track("Apex_Movement"));
        let apex_total = chart_press_count(&apex);
        for track in ["Top_of_the_Leaderboard", "Redline_Response_Time"] {
            let other = generate_chart(song_by_track(track));
            let other_total = chart_press_count(&other);
            assert!(
                apex_total > other_total,
                "{track}: Apex {apex_total} vs {other_total}"
            );
        }
    }

    #[test]
    fn existing_song_charts_are_unchanged_from_the_former_advanced_charts() {
        // 同時押しを減らし片足連打を増やした後の譜面が、意図せず変わっていないことを確認する
        // (ノーツ数, 打鍵数)
        for (track, expected) in [
            ("Top_of_the_Leaderboard", (593, 601)),
            ("Redline_Response_Time", (668, 675)),
            ("Apex_Movement", (645, 701)),
        ] {
            let notes = generate_chart(song_by_track(track));
            assert_eq!((notes.len(), chart_press_count(&notes)), expected, "{track}");
        }
    }

    #[test]
    fn overclocked_tempo_chart_has_jumps() {
        // 片足連打主体になったが、最高潮(Extreme)区間の4拍目には引き続き同時押しが出る
        let notes = generate_chart(song_by_track("Overclocked_Tempo"));
        assert!(notes.len() > 100, "{}", notes.len());
        let jumps = notes.iter().filter(|n| n.lanes.len() == 2).count();
        assert!(jumps > 20, "同時押し{jumps}個");
    }

    #[test]
    fn generate_chart_skips_beats_before_scroll_travel_time() {
        // 開始直後に判定ライン付近へノーツが湧かないよう、スクロール所要時間より前のビートには置かない
        let song = RhythmSong {
            track_name: "test",
            display_name: "test",
            duration_ms: 3000,
            beat_times_ms: &[100, 500, 900, 1300, 1700, 2100],
            sections: &[(0, SectionDensity::Low)],
        };
        let notes = generate_chart(&song);
        assert_eq!(
            notes.iter().map(|n| n.hit_at).collect::<Vec<_>>(),
            vec![Duration::from_millis(1700), Duration::from_millis(2100)]
        );
    }

    #[test]
    fn generate_chart_is_deterministic() {
        for song in SONGS {
            let a = generate_chart(song);
            let b = generate_chart(song);
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                assert_eq!((x.hit_at, &x.lanes), (y.hit_at, &y.lanes));
            }
        }
    }

    #[test]
    fn generate_chart_real_songs_are_sorted_within_song_and_after_lead_in() {
        for song in SONGS {
            let last_beat = Duration::from_millis(*song.beat_times_ms.last().unwrap() as u64);
            let notes = generate_chart(song);
            assert!(!notes.is_empty());
            assert!(notes[0].hit_at >= SCROLL_TRAVEL_TIME);
            assert!(notes.last().unwrap().hit_at <= last_beat);
            for pair in notes.windows(2) {
                assert!(
                    pair[0].hit_at < pair[1].hit_at,
                    "{}: 時刻は狭義増加",
                    song.track_name
                );
            }
        }
    }

    #[test]
    fn generate_chart_real_songs_are_full_chorus_charts() {
        for song in SONGS {
            let notes = generate_chart(song);
            println!(
                "{}: (ノーツ数, 打鍵数) = ({}, {})",
                song.track_name,
                notes.len(),
                chart_press_count(&notes)
            );
            // フルコーラスの長い譜面(旧仕様の最大36ノーツより大幅に多い)
            assert!(notes.len() > 100, "{}: {}", song.track_name, notes.len());
        }
    }

    #[test]
    fn generate_chart_uses_every_lane_and_no_single_note_jacks() {
        for song in SONGS {
            let notes = generate_chart(song);
            for lane in Lane::all() {
                assert!(
                    notes.iter().any(|n| n.lanes.contains(&lane)),
                    "{}: {lane:?}を使うこと",
                    song.track_name
                );
            }
            // 単押しが同じレーンに連続しない(擬似ランダムでも直前と同じレーンは選ばない)
            for pair in notes.windows(2) {
                if pair[0].lanes.len() == 1 && pair[1].lanes.len() == 1 {
                    assert_ne!(pair[0].lanes, pair[1].lanes, "{}", song.track_name);
                }
            }
        }
    }

    #[test]
    fn every_song_chart_has_jumps() {
        for song in SONGS {
            assert!(
                generate_chart(song).iter().any(|n| n.lanes.len() == 2),
                "{}",
                song.track_name
            );
        }
    }

    // --- RhythmGame::new(曲インデックス) ---

    #[test]
    fn new_uses_selected_song_chart() {
        for (i, song) in SONGS.iter().enumerate() {
            let game = RhythmGame::new(i);
            assert_eq!(game.song().track_name, song.track_name);
            assert_eq!(
                chart_signature(&game.notes),
                chart_signature(&generate_chart(song))
            );
        }
    }

    #[test]
    fn new_with_out_of_range_song_index_falls_back_to_first_song() {
        let game = RhythmGame::new(SONGS.len() + 3);
        assert_eq!(game.song().track_name, SONGS[0].track_name);
    }

    #[test]
    fn restart_clock_resets_elapsed_time() {
        let mut game = RhythmGame::new(0);
        game.set_elapsed_for_test(Duration::from_secs(10));
        game.restart_clock();
        assert!(game.started_at.elapsed() < Duration::from_secs(1));
    }

    // --- 入力判定(process_press) ---

    #[test]
    fn process_press_hits_single_lane_note_within_window() {
        let mut notes = vec![note(&[Lane::Up], 1000)];
        let w = JUDGE_WINDOWS;
        let outcome = process_press(&mut notes, Lane::Up, Duration::from_millis(1010), w).unwrap();
        assert_eq!(outcome.press_judgement, Judgement::Perfect);
        assert_eq!(
            outcome.note_final,
            Some((Judgement::Perfect, Duration::from_millis(10)))
        );
        assert!(notes[0].is_judged());
    }

    #[test]
    fn process_press_air_press_returns_none_and_does_not_mutate() {
        let mut notes = vec![note(&[Lane::Up], 1000)];
        let w = JUDGE_WINDOWS;
        // 判定ウィンドウ外のレーンなしの入力(空打ち)
        let outcome = process_press(&mut notes, Lane::Left, Duration::from_millis(1000), w);
        assert!(outcome.is_none());
        assert!(!notes[0].is_judged());
        assert!(notes[0].pressed.is_empty());
    }

    #[test]
    fn process_press_rejects_outside_good_window() {
        let mut notes = vec![note(&[Lane::Up], 1000)];
        let w = JUDGE_WINDOWS;
        let outcome = process_press(&mut notes, Lane::Up, Duration::from_millis(1121), w);
        assert!(outcome.is_none());
    }

    #[test]
    fn process_press_picks_nearest_note_when_multiple_in_window() {
        let mut notes = vec![note(&[Lane::Left], 1000), note(&[Lane::Left], 1080)];
        let w = JUDGE_WINDOWS;
        let outcome =
            process_press(&mut notes, Lane::Left, Duration::from_millis(1050), w).unwrap();
        // 1050msに近いのは2つ目(差30ms)
        assert!(notes[1].is_judged());
        assert!(!notes[0].is_judged());
        assert_eq!(outcome.press_judgement, Judgement::Perfect);
    }

    #[test]
    fn process_press_simultaneous_note_requires_all_lanes() {
        let mut notes = vec![note(&[Lane::Left, Lane::Right], 1000)];
        let w = JUDGE_WINDOWS;
        // 片方だけ踏んだ時点ではまだ確定しない
        let first = process_press(&mut notes, Lane::Left, Duration::from_millis(1000), w).unwrap();
        assert!(first.note_final.is_none());
        assert!(!notes[0].is_judged());
        // 同じレーンへの重複入力は候補にならない(空打ち扱い)
        let dup = process_press(&mut notes, Lane::Left, Duration::from_millis(1005), w);
        assert!(dup.is_none());
        // 残りのレーンを踏むと確定する
        let second =
            process_press(&mut notes, Lane::Right, Duration::from_millis(1010), w).unwrap();
        assert!(second.note_final.is_some());
        assert!(notes[0].is_judged());
    }

    #[test]
    fn process_press_simultaneous_note_takes_worst_judgement() {
        let mut notes = vec![note(&[Lane::Left, Lane::Right], 1000)];
        let w = JUDGE_WINDOWS;
        process_press(&mut notes, Lane::Left, Duration::from_millis(1000), w); // Perfect
        let outcome =
            process_press(&mut notes, Lane::Right, Duration::from_millis(1100), w).unwrap();
        // 差100ms = Good、より良いPerfectがあってもノーツ全体はGoodになる
        let (final_judgement, _) = outcome.note_final.unwrap();
        assert_eq!(final_judgement, Judgement::Good);
    }

    // --- RhythmGame(ゲームループ統合) ---

    #[test]
    fn update_marks_miss_after_good_window_passes_without_input() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Down], 500)];
        game.combo = 3;
        // goodウィンドウは120ms
        game.set_elapsed_for_test(Duration::from_millis(621));
        game.update(Duration::from_millis(0));
        assert!(game.notes[0].is_judged());
        assert_eq!(game.notes[0].judgement, Some(Judgement::Miss));
        assert_eq!(game.tracker.total(), 1);
        assert_eq!(game.combo, 0);
        assert_eq!(game.last_judgement, Some(Judgement::Miss));
    }

    #[test]
    fn update_does_not_mark_miss_within_window() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Down], 500)];
        game.set_elapsed_for_test(Duration::from_millis(619));
        game.update(Duration::from_millis(0));
        assert!(!game.notes[0].is_judged());
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn handle_key_records_hit_and_builds_combo() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Right], 1000)];
        game.set_elapsed_for_test(Duration::from_millis(1000));
        game.handle_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(game.tracker.total(), 1);
        assert_eq!(game.combo, 1);
        assert_eq!(game.max_combo, 1);
        assert!(game.notes[0].is_judged());
    }

    #[test]
    fn handle_key_air_press_does_not_consume_note_or_progress() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Right], 1000)];
        game.combo = 5;
        game.set_elapsed_for_test(Duration::from_millis(1000));
        // ノーツが無いレーン(Left)への空打ち
        game.handle_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(
            game.tracker.total(),
            0,
            "空打ちは判定済みノーツ数を増やさない"
        );
        assert!(!game.notes[0].is_judged());
        assert_eq!(game.combo, 0, "空打ちでコンボが切れる");
    }

    #[test]
    fn miss_resets_combo() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Up], 500), note(&[Lane::Down], 1500)];
        game.combo = 4;
        game.max_combo = 4;
        game.set_elapsed_for_test(Duration::from_millis(621));
        game.update(Duration::from_millis(0));
        assert_eq!(game.combo, 0);
        assert_eq!(game.max_combo, 4, "最大コンボは保持される");
    }

    /// 指定した曲の長さ(duration_ms)をDurationで返す
    fn song_duration(song_index: usize) -> Duration {
        Duration::from_millis(SONGS[song_index].duration_ms as u64)
    }

    #[test]
    fn is_finished_true_only_after_all_notes_judged() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Up], 500), note(&[Lane::Down], 900)];
        // 曲は再生し終えている前提で、ノーツ側の条件だけを確かめる
        game.set_elapsed_for_test(song_duration(0));
        assert!(!game.is_finished());
        game.notes[0].judgement = Some(Judgement::Perfect);
        assert!(!game.is_finished());
        game.notes[1].judgement = Some(Judgement::Miss);
        assert!(game.is_finished());
    }

    #[test]
    fn is_finished_false_while_song_still_playing_after_all_notes_judged() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.notes[0].judgement = Some(Judgement::Perfect);
        // 最後のノーツ直後
        game.set_elapsed_for_test(Duration::from_millis(600));
        assert!(!game.is_finished(), "曲の再生中はリザルトに進まない");
        // 曲の終わりの直前
        game.set_elapsed_for_test(song_duration(0) - Duration::from_millis(1));
        assert!(
            !game.is_finished(),
            "曲の長さに達するまではリザルトに進まない"
        );
    }

    #[test]
    fn is_finished_true_once_song_duration_reached_after_all_notes_judged() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.notes[0].judgement = Some(Judgement::Perfect);
        game.set_elapsed_for_test(song_duration(0));
        assert!(game.is_finished());
    }

    #[test]
    fn is_finished_false_with_unjudged_note_even_after_song_duration() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Up], 500), note(&[Lane::Down], 900)];
        game.notes[0].judgement = Some(Judgement::Perfect);
        game.set_elapsed_for_test(song_duration(0) + Duration::from_secs(5));
        assert!(
            !game.is_finished(),
            "曲の長さを過ぎても未判定のノーツが残っていれば終わらない"
        );
    }

    #[test]
    fn is_finished_uses_duration_of_selected_song() {
        // 一番長い曲(Apex_Movement)を選び、それより短い曲の長さだけ経過させる
        let longest = SONGS
            .iter()
            .enumerate()
            .max_by_key(|(_, s)| s.duration_ms)
            .map(|(i, _)| i)
            .unwrap();
        let shortest = SONGS
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| s.duration_ms)
            .map(|(i, _)| i)
            .unwrap();
        assert_ne!(longest, shortest);
        let mut game = RhythmGame::new(longest);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.notes[0].judgement = Some(Judgement::Perfect);
        game.set_elapsed_for_test(song_duration(shortest));
        assert!(!game.is_finished(), "プレイ中の曲の長さで判定する");
        game.set_elapsed_for_test(song_duration(longest));
        assert!(game.is_finished());
    }

    #[test]
    fn songs_have_measured_duration_longer_than_last_beat() {
        let expected = [
            ("Top_of_the_Leaderboard", 178_808),
            ("Redline_Response_Time", 179_435),
            ("Apex_Movement", 182_204),
            ("Overclocked_Tempo", 177_476),
        ];
        assert_eq!(SONGS.len(), expected.len());
        for (track_name, duration_ms) in expected {
            let song = song_by_track(track_name);
            assert_eq!(
                song.duration_ms, duration_ms,
                "{track_name}: 実測の曲の長さ"
            );
            assert!(
                song.duration_ms > *song.beat_times_ms.last().unwrap(),
                "{track_name}: 曲の長さは最後のビートより後"
            );
        }
    }

    #[test]
    fn keys_and_updates_after_all_notes_judged_do_not_change_score_while_song_plays() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.set_elapsed_for_test(Duration::from_millis(500));
        game.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(game.tracker.total(), 1);
        // 全ノーツ判定済みだが曲はまだ再生中。入力・時間経過ともスコアを変えない
        game.set_elapsed_for_test(Duration::from_secs(60));
        game.handle_key(KeyEvent::from(KeyCode::Up));
        game.update(Duration::from_millis(16));
        assert_eq!(game.tracker.total(), 1);
        assert!(!game.is_finished());
    }

    #[test]
    fn handle_key_does_nothing_once_finished() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.notes[0].judgement = Some(Judgement::Perfect);
        game.set_elapsed_for_test(song_duration(0));
        assert!(game.is_finished());
        game.handle_key(KeyEvent::from(KeyCode::Up));
        // 既に終了しているので何も記録されない
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn result_uses_score_tracker() {
        let mut game = RhythmGame::new(0);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.set_elapsed_for_test(Duration::from_millis(500));
        game.handle_key(KeyEvent::from(KeyCode::Up));
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.correct, 1);
        assert_eq!(result.total, 1);
        // 難易度は選ばないので、常に上級として記録する
        assert_eq!(result.difficulty, Difficulty::Advanced);
        assert_eq!(result.difficulty, SESSION_DIFFICULTY);
    }

    // --- 小節ライン ---

    #[test]
    fn measure_line_times_ms_returns_every_fourth_beat() {
        assert_eq!(
            measure_line_times_ms(TEST_BEATS).collect::<Vec<_>>(),
            vec![2000, 3600, 5200, 6800]
        );
        // ビート数が4の倍数でなくても、存在するビートだけを返す(境界外を含まない)
        assert_eq!(
            measure_line_times_ms(SAMPLE_BEATS).collect::<Vec<_>>(),
            vec![500, 2500]
        );
        assert_eq!(
            measure_line_times_ms(&[100, 200, 300]).collect::<Vec<_>>(),
            vec![100]
        );
        assert_eq!(measure_line_times_ms(&[]).count(), 0);
    }

    #[test]
    fn measure_line_times_of_real_songs_are_beats_at_multiples_of_four() {
        for song in SONGS {
            let times: Vec<u32> = measure_line_times_ms(song.beat_times_ms).collect();
            assert_eq!(times.len(), song.beat_times_ms.len().div_ceil(4));
            for (i, &ms) in times.iter().enumerate() {
                assert_eq!(ms, song.beat_times_ms[i * 4], "{}", song.track_name);
            }
        }
    }

    #[test]
    fn track_row_follows_scroll_progress() {
        let hit_at = Duration::from_millis(2000);
        let at = |now_ms: u64| track_row(hit_at, Duration::from_millis(now_ms), 11);
        // 判定時刻ちょうどは判定ライン直下の行(0行目)
        assert_eq!(at(2000), Some(0));
        // スクロール所要時間前に画面下端(最終行)に出現する
        assert_eq!(at(500), Some(10));
        // 中間時点は中央の行
        assert_eq!(at(1250), Some(5));
        // 出現前は表示しない
        assert_eq!(at(499), None);
        // 判定時刻を少し過ぎても(progress 1.05まで)0行目に残し、それ以降は表示しない
        assert_eq!(at(2050), Some(0));
        assert_eq!(at(2100), None);
    }

    /// 描画結果のうち、譜面トラック内側(太枠の縦線を含む行)だけを文字列で返す
    fn rendered_track_lines(game: &RhythmGame) -> Vec<String> {
        let backend = ratatui::backend::TestBackend::new(80, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .filter(|line| line.contains('┃'))
            .collect()
    }

    #[test]
    fn render_draws_measure_line_across_all_lanes_on_empty_row() {
        let mut game = RhythmGame::new(0);
        game.notes.clear();
        // 開始直後は最初の小節の頭(1ビート目)が判定ライン付近に見えている
        game.set_elapsed_for_test(Duration::ZERO);
        let lines = rendered_track_lines(&game);
        let full_line = "─".repeat(7 * 4);
        assert!(
            lines.iter().any(|l| l.contains(&full_line)),
            "4レーン全体を貫く小節ラインが描かれること: {lines:#?}"
        );
        // 小節ライン以外のノーツ無しの行は従来通り薄い点
        assert!(
            lines.iter().any(|l| l.contains('·') && !l.contains('─')),
            "{lines:#?}"
        );
    }

    #[test]
    fn render_prefers_note_over_measure_line_on_same_row() {
        let mut game = RhythmGame::new(0);
        let measure_ms = measure_line_times_ms(game.song().beat_times_ms)
            .nth(1)
            .unwrap() as u64;
        // 2小節目の頭と同じ時刻に↓レーンのノーツを置き、スクロールの中間まで進める
        game.notes = vec![note(&[Lane::Down], measure_ms)];
        game.set_elapsed_for_test(Duration::from_millis(measure_ms) - SCROLL_TRAVEL_TIME / 2);
        let lines = rendered_track_lines(&game);
        let note_line = lines
            .iter()
            .find(|l| l.contains('●'))
            .expect("ノーツが描かれること");
        // 並びは ←↓↑→。↓のマスだけノーツで、残り3レーンは小節ライン
        let expected = format!("{}   ●   {}", "─".repeat(7), "─".repeat(14));
        assert!(note_line.contains(&expected), "{note_line}");
        assert_eq!(note_line.matches('●').count(), 1);
        assert!(!note_line.contains('·'), "{note_line}");
    }

    // --- 最初の1小節のカウント音 ---

    #[test]
    fn count_in_starts_with_no_beat_played() {
        let game = RhythmGame::new(0);
        assert_eq!(game.next_count_in_beat, 0);
    }

    #[test]
    fn count_in_advances_on_each_of_first_four_beats_only() {
        let mut game = RhythmGame::new(1);
        let beats = game.song().beat_times_ms;
        let ms = |i: usize| Duration::from_millis(beats[i] as u64);
        // 1ビート目の直前はまだ鳴らさない
        game.set_elapsed_for_test(ms(0) - Duration::from_millis(1));
        game.update(Duration::ZERO);
        assert_eq!(game.next_count_in_beat, 0);
        for i in 0..4 {
            game.set_elapsed_for_test(ms(i));
            game.update(Duration::ZERO);
            assert_eq!(game.next_count_in_beat, i + 1, "{}ビート目", i + 1);
            // 次のビート時刻の直前では進まない
            game.set_elapsed_for_test(ms(i + 1) - Duration::from_millis(1));
            game.update(Duration::ZERO);
            assert_eq!(game.next_count_in_beat, i + 1, "{}ビート目の後", i + 1);
        }
        // 5ビート目以降では鳴らさない
        for i in 4..8 {
            game.set_elapsed_for_test(ms(i));
            game.update(Duration::ZERO);
            assert_eq!(game.next_count_in_beat, 4, "{}ビート目", i + 1);
        }
    }

    #[test]
    fn header_shows_song_name_and_advanced_label() {
        let game = RhythmGame::new(SONGS.len() - 1);
        let backend = ratatui::backend::TestBackend::new(80, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace(' ', "");
        assert!(text.contains(&SONGS[SONGS.len() - 1].display_name.replace(' ', "")));
        assert!(text.contains("上級"), "常に上級であることを表示する");
    }
}
