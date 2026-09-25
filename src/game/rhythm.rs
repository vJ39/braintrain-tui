use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

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

/// 難易度ごとの判定ウィンドウ(判定ラインとの時間差の許容幅)
#[derive(Debug, Clone, Copy)]
struct JudgeWindows {
    perfect: Duration,
    great: Duration,
    good: Duration,
}

fn judge_windows(difficulty: Difficulty) -> JudgeWindows {
    match difficulty {
        Difficulty::Beginner => JudgeWindows {
            perfect: Duration::from_millis(60),
            great: Duration::from_millis(120),
            good: Duration::from_millis(200),
        },
        Difficulty::Intermediate => JudgeWindows {
            perfect: Duration::from_millis(45),
            great: Duration::from_millis(90),
            good: Duration::from_millis(150),
        },
        Difficulty::Advanced => JudgeWindows {
            perfect: Duration::from_millis(35),
            great: Duration::from_millis(70),
            good: Duration::from_millis(120),
        },
    }
}

// ---------------------------------------------------------------------------
// 楽曲と譜面生成
//
// 譜面はBPMからの理論値ではなく、曲ごとに実測したビート時刻(beats.rs)の上に配置する。
// 難易度による違いはBPMではなく「セクション密度(Low/Mid/High/Extreme)への反応」で表現し、
// 同じ曲・同じビートグリッドの上で難易度ごとに異なるノーツ配置をする。
// 譜面は乱数を使わず決定的に生成する(同じ曲・同じ難易度なら毎回同じ譜面で、練習して覚えられる)。
// ---------------------------------------------------------------------------

/// 曲の大まかな盛り上がり(RMSエネルギー解析による区分)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionDensity {
    Low,
    Mid,
    High,
    /// Advanced専用の最高難度区間。Beginner/IntermediateではHighと同じ配置になる
    Extreme,
}

/// リズムゲームで遊べる1曲分の定義
pub struct RhythmSong {
    /// audio::play_bgm_trackに渡す名前(拡張子・ディレクトリなし)
    pub track_name: &'static str,
    /// 曲選択画面に表示する名前
    pub display_name: &'static str,
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

/// 50-100秒・130-170秒の最高密度区間をExtremeにし、上級譜面を既存2曲より明確に難しくする
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

/// 曲選択画面に並べる曲一覧(表示順)。曲を増やすときはbeats.rsに実測ビート時刻を足し、ここに追加する
pub const SONGS: &[RhythmSong] = &[
    RhythmSong {
        track_name: "Top_of_the_Leaderboard",
        display_name: "Top of the Leaderboard (BPM 150)",
        beat_times_ms: beats::TOP_OF_THE_LEADERBOARD_BEATS_MS,
        sections: TOP_OF_THE_LEADERBOARD_SECTIONS,
    },
    RhythmSong {
        track_name: "Redline_Response_Time",
        display_name: "Redline Response Time (BPM 180)",
        beat_times_ms: beats::REDLINE_RESPONSE_TIME_BEATS_MS,
        sections: REDLINE_RESPONSE_TIME_SECTIONS,
    },
    RhythmSong {
        track_name: "Apex_Movement",
        display_name: "Apex Movement (BPM 152)",
        beat_times_ms: beats::APEX_MOVEMENT_BEATS_MS,
        sections: APEX_MOVEMENT_SECTIONS,
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
    /// このビート位置に同時押し+次のビートとの中間点にも同時押し(Advanced×Extreme専用)
    JumpAndEighthJump,
}

/// セクション密度・難易度・ビートインデックスから、そのビートの配置を決める純粋関数
///
/// 方針:
/// - Lowはどの難易度でも休符を多く残し、単押しのみ(Beginner/Intermediateは2拍に1回、
///   Advancedは「3拍踏んで1拍休む」で間引きを減らす)
/// - Beginnerは4分音符の単押しだけ。Intermediateは盛り上がり(High)で8分音符が入る
/// - AdvancedはMidでも8分音符を混ぜ(2拍に1回)、Highは8分音符の連続+4拍ごとの同時押し
/// - Extremeは Advanced 専用の最高難度。休符なしで全ビート同時押し+8分音符、
///   4拍目は8分音符の位置も同時押しにする。Beginner/IntermediateではHighと同じ配置
/// - beat_indexは曲全体での通し番号(4拍・8拍周期の基準に使う)
fn step_pattern_for(
    density: SectionDensity,
    difficulty: Difficulty,
    beat_index: usize,
) -> StepPattern {
    use SectionDensity::{Extreme, High, Low, Mid};
    use StepPattern::{Jump, JumpAndEighth, JumpAndEighthJump, Single, SingleAndEighth, Skip};
    let even_beat = beat_index.is_multiple_of(2);
    match (difficulty, density) {
        (Difficulty::Beginner, Low) | (Difficulty::Intermediate, Low) => {
            if even_beat {
                Single
            } else {
                Skip
            }
        }
        // Beginner/IntermediateにはExtremeの概念が無いのでHighと同じ扱い
        (Difficulty::Beginner, Mid | High | Extreme) | (Difficulty::Intermediate, Mid) => Single,
        (Difficulty::Intermediate, High | Extreme) => SingleAndEighth,
        (Difficulty::Advanced, Low) => {
            if beat_index % 4 == 3 {
                Skip
            } else {
                Single
            }
        }
        (Difficulty::Advanced, Mid) => {
            if even_beat {
                SingleAndEighth
            } else {
                Single
            }
        }
        // 8拍フレーズの頭は同時押しだけで区切りを付け、4拍目は同時押し+8分音符で畳みかける
        (Difficulty::Advanced, High) => match beat_index % 8 {
            0 => Jump,
            4 => JumpAndEighth,
            _ => SingleAndEighth,
        },
        // 休符を作らず毎拍同時押し+8分音符。4拍目は8分音符も同時押しにして畳みかける
        (Difficulty::Advanced, Extreme) => {
            if beat_index % 4 == 3 {
                JumpAndEighthJump
            } else {
                JumpAndEighth
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

/// 曲の実測ビートと難易度から譜面を作る純粋関数(レーン選択は区切り番号から決まる擬似ランダムなので毎回同じ譜面になる)
fn generate_chart(song: &RhythmSong, difficulty: Difficulty) -> Vec<Note> {
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
        let pattern = step_pattern_for(density_at(song.sections, i), difficulty, i);
        let eighth_at = beats.get(i + 1).map(|&next| eighth_hit_at(beat_ms, next));
        match pattern {
            StepPattern::Skip => {}
            StepPattern::Single => notes.push(Note::new(vec![cycler.next_single()], hit_at)),
            StepPattern::Jump => notes.push(Note::new(cycler.next_jump(), hit_at)),
            StepPattern::SingleAndEighth
            | StepPattern::JumpAndEighth
            | StepPattern::JumpAndEighthJump => {
                let lanes = if pattern == StepPattern::SingleAndEighth {
                    vec![cycler.next_single()]
                } else {
                    cycler.next_jump()
                };
                notes.push(Note::new(lanes, hit_at));
                // 最後のビートには次のビートが無いので8分音符は付けない
                if let Some(eighth_at) = eighth_at {
                    let eighth_lanes = if pattern == StepPattern::JumpAndEighthJump {
                        cycler.next_jump()
                    } else {
                        vec![cycler.next_single()]
                    };
                    notes.push(Note::new(eighth_lanes, eighth_at));
                }
            }
        }
    }
    notes
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
    difficulty: Difficulty,
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
}

impl RhythmGame {
    /// 指定した曲(SONGSのインデックス)・難易度の譜面でゲームを作る。
    /// 範囲外のインデックスは先頭の曲として扱う。
    /// 曲との同期のため、呼び出し側はBGM再生の直後に restart_clock を呼ぶこと
    pub fn new(difficulty: Difficulty, song_index: usize) -> Self {
        let song_index = if song_index < SONGS.len() {
            song_index
        } else {
            0
        };
        Self {
            difficulty,
            song_index,
            tracker: ScoreTracker::new(),
            notes: generate_chart(&SONGS[song_index], difficulty),
            started_at: Instant::now(),
            combo: 0,
            max_combo: 0,
            last_judgement: None,
            last_judgement_at: Duration::ZERO,
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

    /// 上段: 左=コンボ、中央=直近の判定、右=譜面の進み具合。枠の上辺に曲名と難易度
    fn render_header(&self, frame: &mut Frame, area: Rect, judgement: Option<Judgement>) {
        let (difficulty_text, difficulty_color) = theme::difficulty_label(self.difficulty);
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
        let windows = judge_windows(self.difficulty);
        match process_press(&mut self.notes, lane, now, windows) {
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
        let good_window = judge_windows(self.difficulty).good;
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
            let remaining_secs = note.hit_at.as_secs_f64() - now.as_secs_f64();
            let progress = 1.0 - remaining_secs / SCROLL_TRAVEL_TIME.as_secs_f64();
            if !(0.0..=1.05).contains(&progress) {
                continue;
            }
            let row = ((1.0 - progress) * (track_height - 1) as f64).round() as usize;
            let row = row.min(track_height - 1);
            for lane in &note.lanes {
                if let Some(lane_idx) = lanes.iter().position(|l| l == lane) {
                    grid[row][lane_idx] = '●';
                }
            }
        }

        let mut lines = vec![receptors, judge_line];
        for (row_idx, row) in grid.iter().enumerate() {
            let spans: Vec<Span> = row
                .iter()
                .zip(lanes.iter())
                .map(|(&c, lane)| {
                    if c == ' ' {
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

    fn is_finished(&self) -> bool {
        self.notes.iter().all(|n| n.is_judged())
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
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
        let game = RhythmGame::new(Difficulty::Beginner, 0);
        assert_eq!(game.visible_judgement(Duration::ZERO), None);
    }

    #[test]
    fn visible_judgement_holds_for_display_time_then_disappears() {
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
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
    fn best_judgement_boundaries_for_intermediate() {
        let w = judge_windows(Difficulty::Intermediate);
        assert_eq!(
            best_judgement_for_diff(Duration::from_millis(45), w),
            Some(Judgement::Perfect)
        );
        assert_eq!(
            best_judgement_for_diff(Duration::from_millis(46), w),
            Some(Judgement::Great)
        );
        assert_eq!(
            best_judgement_for_diff(Duration::from_millis(90), w),
            Some(Judgement::Great)
        );
        assert_eq!(
            best_judgement_for_diff(Duration::from_millis(150), w),
            Some(Judgement::Good)
        );
        assert_eq!(best_judgement_for_diff(Duration::from_millis(151), w), None);
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
                ("Apex_Movement", 439)
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
    fn songs_are_listed_in_order_with_apex_movement_last() {
        let names: Vec<&str> = SONGS.iter().map(|s| s.track_name).collect();
        assert_eq!(
            names,
            vec![
                "Top_of_the_Leaderboard",
                "Redline_Response_Time",
                "Apex_Movement"
            ]
        );
        assert_eq!(
            song_by_track("Apex_Movement").display_name,
            "Apex Movement (BPM 152)"
        );
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

    // --- 難易度×密度ごとの配置(step_pattern_for) ---

    const ALL_DIFFICULTIES: [Difficulty; 3] = [
        Difficulty::Beginner,
        Difficulty::Intermediate,
        Difficulty::Advanced,
    ];
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
            StepPattern::JumpAndEighthJump => 4,
        }
    }

    /// 8拍分の同時押しの回数(8分音符位置の同時押しも数える)
    fn jumps_per_8_beats(density: SectionDensity, difficulty: Difficulty) -> usize {
        (0..8)
            .map(|i| match step_pattern_for(density, difficulty, i) {
                StepPattern::Jump | StepPattern::JumpAndEighth => 1,
                StepPattern::JumpAndEighthJump => 2,
                _ => 0,
            })
            .sum()
    }

    /// 8拍分(フレーズ2小節分)の合計打鍵数
    fn presses_per_8_beats(density: SectionDensity, difficulty: Difficulty) -> usize {
        (0..8)
            .map(|i| presses(step_pattern_for(density, difficulty, i)))
            .sum()
    }

    #[test]
    fn step_pattern_beginner_never_uses_eighth_or_jump() {
        for density in ALL_DENSITIES {
            for i in 0..16 {
                let p = step_pattern_for(density, Difficulty::Beginner, i);
                assert!(
                    matches!(p, StepPattern::Skip | StepPattern::Single),
                    "Beginnerは4分音符の単押しのみ: {density:?} beat{i} => {p:?}"
                );
            }
        }
        // Lowは2拍に1回、Mid/Highは毎拍
        assert_eq!(
            step_pattern_for(SectionDensity::Low, Difficulty::Beginner, 0),
            StepPattern::Single
        );
        assert_eq!(
            step_pattern_for(SectionDensity::Low, Difficulty::Beginner, 1),
            StepPattern::Skip
        );
        for i in 0..8 {
            assert_eq!(
                step_pattern_for(SectionDensity::Mid, Difficulty::Beginner, i),
                StepPattern::Single
            );
            assert_eq!(
                step_pattern_for(SectionDensity::High, Difficulty::Beginner, i),
                StepPattern::Single
            );
        }
    }

    #[test]
    fn step_pattern_intermediate_uses_eighth_only_in_high_and_no_jump() {
        assert_eq!(
            step_pattern_for(SectionDensity::Low, Difficulty::Intermediate, 0),
            StepPattern::Single
        );
        assert_eq!(
            step_pattern_for(SectionDensity::Low, Difficulty::Intermediate, 1),
            StepPattern::Skip
        );
        for i in 0..8 {
            assert_eq!(
                step_pattern_for(SectionDensity::Mid, Difficulty::Intermediate, i),
                StepPattern::Single
            );
            assert_eq!(
                step_pattern_for(SectionDensity::High, Difficulty::Intermediate, i),
                StepPattern::SingleAndEighth
            );
        }
        for density in ALL_DENSITIES {
            for i in 0..16 {
                let p = step_pattern_for(density, Difficulty::Intermediate, i);
                assert!(!matches!(
                    p,
                    StepPattern::Jump | StepPattern::JumpAndEighth | StepPattern::JumpAndEighthJump
                ));
            }
        }
    }

    #[test]
    fn step_pattern_advanced_jumps_every_four_beats_in_high() {
        for i in 0..16 {
            let p = step_pattern_for(SectionDensity::High, Difficulty::Advanced, i);
            if i % 4 == 0 {
                assert!(
                    matches!(p, StepPattern::Jump | StepPattern::JumpAndEighth),
                    "beat{i} => {p:?}"
                );
            } else {
                assert_eq!(p, StepPattern::SingleAndEighth, "beat{i}");
            }
        }
        // 同時押しはHigh/Extremeだけ
        for density in [SectionDensity::Low, SectionDensity::Mid] {
            for i in 0..16 {
                let p = step_pattern_for(density, Difficulty::Advanced, i);
                assert!(!matches!(
                    p,
                    StepPattern::Jump | StepPattern::JumpAndEighth | StepPattern::JumpAndEighthJump
                ));
            }
        }
    }

    #[test]
    fn step_pattern_advanced_extreme_jumps_on_every_beat_with_eighth_and_no_rests() {
        for i in 0..32 {
            let p = step_pattern_for(SectionDensity::Extreme, Difficulty::Advanced, i);
            // 休符なし・全ビートで同時押し・全ビートに8分音符
            assert!(
                matches!(
                    p,
                    StepPattern::JumpAndEighth | StepPattern::JumpAndEighthJump
                ),
                "beat{i} => {p:?}"
            );
            // 4拍目は8分音符の位置も同時押しにして畳みかける
            if i % 4 == 3 {
                assert_eq!(p, StepPattern::JumpAndEighthJump, "beat{i}");
            }
        }
    }

    #[test]
    fn step_pattern_advanced_extreme_is_clearly_denser_than_high() {
        let high = presses_per_8_beats(SectionDensity::High, Difficulty::Advanced);
        let extreme = presses_per_8_beats(SectionDensity::Extreme, Difficulty::Advanced);
        // 打鍵数で1.5倍以上(「既存Highよりはっきり難しい」)
        assert!(extreme * 2 >= high * 3, "High {high} / Extreme {extreme}");
        let high_jumps = jumps_per_8_beats(SectionDensity::High, Difficulty::Advanced);
        let extreme_jumps = jumps_per_8_beats(SectionDensity::Extreme, Difficulty::Advanced);
        assert!(
            extreme_jumps >= high_jumps * 4,
            "同時押し High {high_jumps} / Extreme {extreme_jumps}"
        );
    }

    #[test]
    fn step_pattern_extreme_falls_back_to_high_for_beginner_and_intermediate() {
        // Beginner/IntermediateにはExtremeの概念が無いので、Highと同じ配置にする
        for difficulty in [Difficulty::Beginner, Difficulty::Intermediate] {
            for i in 0..32 {
                assert_eq!(
                    step_pattern_for(SectionDensity::Extreme, difficulty, i),
                    step_pattern_for(SectionDensity::High, difficulty, i),
                    "{difficulty:?} beat{i}"
                );
            }
        }
    }

    #[test]
    fn step_pattern_double_jump_only_appears_in_advanced_extreme() {
        for difficulty in ALL_DIFFICULTIES {
            for density in ALL_DENSITIES {
                if (difficulty, density) == (Difficulty::Advanced, SectionDensity::Extreme) {
                    continue;
                }
                for i in 0..32 {
                    assert_ne!(
                        step_pattern_for(density, difficulty, i),
                        StepPattern::JumpAndEighthJump,
                        "{difficulty:?} {density:?} beat{i}"
                    );
                }
            }
        }
    }

    #[test]
    fn step_pattern_advanced_mid_mixes_eighth_notes() {
        let has_eighth = (0..8).any(|i| {
            step_pattern_for(SectionDensity::Mid, Difficulty::Advanced, i)
                == StepPattern::SingleAndEighth
        });
        assert!(has_eighth, "AdvancedのMidは8分音符を含む");
    }

    #[test]
    fn step_pattern_low_sections_keep_rests_in_every_difficulty() {
        for difficulty in ALL_DIFFICULTIES {
            let rests = (0..8)
                .filter(|&i| {
                    step_pattern_for(SectionDensity::Low, difficulty, i) == StepPattern::Skip
                })
                .count();
            assert!(rests >= 2, "{difficulty:?}のLowは8拍中2拍以上休符: {rests}");
            // Lowでは8分音符・同時押しを使わない
            for i in 0..8 {
                let p = step_pattern_for(SectionDensity::Low, difficulty, i);
                assert!(matches!(p, StepPattern::Skip | StepPattern::Single));
            }
        }
    }

    #[test]
    fn step_pattern_gets_busier_with_difficulty_and_density() {
        for density in ALL_DENSITIES {
            let b = presses_per_8_beats(density, Difficulty::Beginner);
            let i = presses_per_8_beats(density, Difficulty::Intermediate);
            let a = presses_per_8_beats(density, Difficulty::Advanced);
            assert!(b <= i && i <= a, "{density:?}: {b} <= {i} <= {a}");
        }
        // 難易度全体としては明確に忙しくなる(どこかで真に増える)
        let total = |d| {
            ALL_DENSITIES
                .iter()
                .map(|&den| presses_per_8_beats(den, d))
                .sum::<usize>()
        };
        assert!(total(Difficulty::Beginner) < total(Difficulty::Intermediate));
        assert!(total(Difficulty::Intermediate) < total(Difficulty::Advanced));
        // 同じ難易度ならLow < Mid <= High <= Extreme
        for difficulty in ALL_DIFFICULTIES {
            let l = presses_per_8_beats(SectionDensity::Low, difficulty);
            let m = presses_per_8_beats(SectionDensity::Mid, difficulty);
            let h = presses_per_8_beats(SectionDensity::High, difficulty);
            let e = presses_per_8_beats(SectionDensity::Extreme, difficulty);
            assert!(
                l < m && m <= h && h <= e,
                "{difficulty:?}: {l} < {m} <= {h} <= {e}"
            );
        }
        // ExtremeがHighより真に忙しくなるのはAdvancedだけ
        assert!(
            presses_per_8_beats(SectionDensity::High, Difficulty::Advanced)
                < presses_per_8_beats(SectionDensity::Extreme, Difficulty::Advanced)
        );
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
        beat_times_ms: TEST_BEATS,
        sections: &[(0, SectionDensity::High)],
    };

    fn chart_press_count(notes: &[Note]) -> usize {
        notes.iter().map(|n| n.lanes.len()).sum()
    }

    #[test]
    fn generate_chart_places_single_notes_exactly_on_measured_beats() {
        // Beginner×Highは毎拍単押し → ノーツ時刻=実測ビート時刻そのもの
        let notes = generate_chart(&TEST_SONG, Difficulty::Beginner);
        let times: Vec<Duration> = notes.iter().map(|n| n.hit_at).collect();
        let expected: Vec<Duration> = TEST_BEATS
            .iter()
            .map(|&ms| Duration::from_millis(ms as u64))
            .collect();
        assert_eq!(times, expected);
        assert!(notes.iter().all(|n| n.lanes.len() == 1));
    }

    #[test]
    fn generate_chart_eighth_notes_sit_between_beats_and_not_after_last_beat() {
        // Intermediate×Highは毎拍+8分音符。最後のビートには次のビートが無いので8分音符を付けない
        let notes = generate_chart(&TEST_SONG, Difficulty::Intermediate);
        assert_eq!(notes.len(), TEST_BEATS.len() * 2 - 1);
        assert_eq!(notes[0].hit_at, Duration::from_millis(2000));
        assert_eq!(notes[1].hit_at, Duration::from_millis(2200));
        assert_eq!(notes[2].hit_at, Duration::from_millis(2400));
        let last_beat = Duration::from_millis(*TEST_BEATS.last().unwrap() as u64);
        assert!(notes.iter().all(|n| n.hit_at <= last_beat));
    }

    #[test]
    fn generate_chart_jumps_use_two_distinct_lanes() {
        let notes = generate_chart(&TEST_SONG, Difficulty::Advanced);
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
        // 全3曲の中級譜面(全て単押し)で、単押しの「直前→次」12通りが全て現れる
        for song in SONGS {
            let lanes: Vec<Lane> = generate_chart(song, Difficulty::Intermediate)
                .into_iter()
                .map(|n| {
                    assert_eq!(n.lanes.len(), 1, "{}", song.track_name);
                    n.lanes[0]
                })
                .collect();
            assert_eq!(single_transitions(&lanes).len(), 12, "{}", song.track_name);
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
        // 全3曲の上級譜面で、同時押しの組が交互の固定巡回になっていない
        for song in SONGS {
            let jumps: Vec<Vec<Lane>> = generate_chart(song, Difficulty::Advanced)
                .into_iter()
                .filter(|n| n.lanes.len() == 2)
                .map(|n| n.lanes)
                .collect();
            assert!(jumps.len() > 10, "{}", song.track_name);
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
    const TEST_SONG_EXTREME: RhythmSong = RhythmSong {
        track_name: "test",
        display_name: "test",
        beat_times_ms: TEST_BEATS,
        sections: &[(0, SectionDensity::Extreme)],
    };

    fn chart_signature(notes: &[Note]) -> Vec<(Duration, Vec<Lane>)> {
        notes.iter().map(|n| (n.hit_at, n.lanes.clone())).collect()
    }

    #[test]
    fn generate_chart_extreme_matches_high_for_beginner_and_intermediate() {
        for difficulty in [Difficulty::Beginner, Difficulty::Intermediate] {
            let extreme = generate_chart(&TEST_SONG_EXTREME, difficulty);
            let high = generate_chart(&TEST_SONG, difficulty);
            assert!(!extreme.is_empty());
            assert_eq!(
                chart_signature(&extreme),
                chart_signature(&high),
                "{difficulty:?}"
            );
        }
    }

    #[test]
    fn generate_chart_advanced_extreme_jumps_on_every_beat_and_double_jumps_on_fourth() {
        let notes = generate_chart(&TEST_SONG_EXTREME, Difficulty::Advanced);
        // 全ビート+その8分音符(最後のビートは8分音符なし)
        assert_eq!(notes.len(), TEST_BEATS.len() * 2 - 1);
        for (i, &beat_ms) in TEST_BEATS.iter().enumerate() {
            let on_beat = &notes[i * 2];
            assert_eq!(on_beat.hit_at, Duration::from_millis(beat_ms as u64));
            assert_eq!(on_beat.lanes.len(), 2, "beat{i}は同時押し");
            assert_ne!(on_beat.lanes[0], on_beat.lanes[1]);
            if let Some(next_ms) = TEST_BEATS.get(i + 1) {
                let eighth = &notes[i * 2 + 1];
                assert_eq!(eighth.hit_at, eighth_hit_at(beat_ms, *next_ms));
                let expected_lanes = if i % 4 == 3 { 2 } else { 1 };
                assert_eq!(eighth.lanes.len(), expected_lanes, "beat{i}の8分音符");
                if expected_lanes == 2 {
                    assert_ne!(eighth.lanes[0], eighth.lanes[1], "beat{i}");
                }
            }
        }
    }

    #[test]
    fn generate_chart_advanced_extreme_is_clearly_busier_than_high() {
        let high = chart_press_count(&generate_chart(&TEST_SONG, Difficulty::Advanced));
        let extreme = chart_press_count(&generate_chart(&TEST_SONG_EXTREME, Difficulty::Advanced));
        assert!(extreme * 2 >= high * 3, "High {high} / Extreme {extreme}");
    }

    /// 譜面中の任意の10秒間に踏むキー数の最大値(ピークの忙しさ)
    fn peak_presses_in_10s(notes: &[Note]) -> usize {
        const WINDOW: Duration = Duration::from_secs(10);
        let mut best = 0;
        let mut end = 0;
        let mut sum = 0;
        for start in 0..notes.len() {
            while end < notes.len() && notes[end].hit_at < notes[start].hit_at + WINDOW {
                sum += notes[end].lanes.len();
                end += 1;
            }
            best = best.max(sum);
            sum -= notes[start].lanes.len();
        }
        best
    }

    #[test]
    fn apex_movement_advanced_is_clearly_harder_than_existing_songs() {
        let apex = generate_chart(song_by_track("Apex_Movement"), Difficulty::Advanced);
        let apex_peak = peak_presses_in_10s(&apex);
        let apex_total = chart_press_count(&apex);
        for track in ["Top_of_the_Leaderboard", "Redline_Response_Time"] {
            let other = generate_chart(song_by_track(track), Difficulty::Advanced);
            let other_peak = peak_presses_in_10s(&other);
            let other_total = chart_press_count(&other);
            println!(
                "Advanced 10秒ピーク打鍵数 Apex {apex_peak} / {track} {other_peak}, 総打鍵数 Apex {apex_total} / {track} {other_total}"
            );
            // ピークの忙しさで1.2倍超、総打鍵数でも上回る
            assert!(
                apex_peak * 5 > other_peak * 6,
                "{track}: Apex {apex_peak} vs {other_peak}"
            );
            assert!(
                apex_total > other_total,
                "{track}: Apex {apex_total} vs {other_total}"
            );
        }
    }

    #[test]
    fn apex_movement_charts_generate_for_every_difficulty() {
        // Beginner/IntermediateでExtreme区間に入ってもクラッシュせず、High相当の譜面になる
        let song = song_by_track("Apex_Movement");
        for difficulty in ALL_DIFFICULTIES {
            let notes = generate_chart(song, difficulty);
            assert!(notes.len() > 100, "{difficulty:?}: {}", notes.len());
        }
        for difficulty in [Difficulty::Beginner, Difficulty::Intermediate] {
            assert!(generate_chart(song, difficulty)
                .iter()
                .all(|n| n.lanes.len() == 1));
        }
    }

    #[test]
    fn generate_chart_skips_beats_before_scroll_travel_time() {
        // 開始直後に判定ライン付近へノーツが湧かないよう、スクロール所要時間より前のビートには置かない
        let song = RhythmSong {
            track_name: "test",
            display_name: "test",
            beat_times_ms: &[100, 500, 900, 1300, 1700, 2100],
            sections: &[(0, SectionDensity::High)],
        };
        let notes = generate_chart(&song, Difficulty::Beginner);
        assert_eq!(
            notes.iter().map(|n| n.hit_at).collect::<Vec<_>>(),
            vec![Duration::from_millis(1700), Duration::from_millis(2100)]
        );
    }

    #[test]
    fn generate_chart_is_deterministic() {
        for song in SONGS {
            for difficulty in ALL_DIFFICULTIES {
                let a = generate_chart(song, difficulty);
                let b = generate_chart(song, difficulty);
                assert_eq!(a.len(), b.len());
                for (x, y) in a.iter().zip(&b) {
                    assert_eq!((x.hit_at, &x.lanes), (y.hit_at, &y.lanes));
                }
            }
        }
    }

    #[test]
    fn generate_chart_real_songs_are_sorted_within_song_and_after_lead_in() {
        for song in SONGS {
            let last_beat = Duration::from_millis(*song.beat_times_ms.last().unwrap() as u64);
            for difficulty in ALL_DIFFICULTIES {
                let notes = generate_chart(song, difficulty);
                assert!(!notes.is_empty());
                assert!(notes[0].hit_at >= SCROLL_TRAVEL_TIME);
                assert!(notes.last().unwrap().hit_at <= last_beat);
                for pair in notes.windows(2) {
                    assert!(
                        pair[0].hit_at < pair[1].hit_at,
                        "{} {difficulty:?}: 時刻は狭義増加",
                        song.track_name
                    );
                }
            }
        }
    }

    #[test]
    fn generate_chart_note_count_grows_with_difficulty() {
        for song in SONGS {
            let counts: Vec<(usize, usize)> = ALL_DIFFICULTIES
                .iter()
                .map(|&d| {
                    let notes = generate_chart(song, d);
                    (notes.len(), chart_press_count(&notes))
                })
                .collect();
            println!("{}: (ノーツ数, 打鍵数) = {counts:?}", song.track_name);
            assert!(
                counts[0].0 < counts[1].0 && counts[1].0 < counts[2].0,
                "{}: {counts:?}",
                song.track_name
            );
            assert!(
                counts[0].1 < counts[1].1 && counts[1].1 < counts[2].1,
                "{}: {counts:?}",
                song.track_name
            );
            // フルコーラスの長い譜面(旧仕様の最大36ノーツより大幅に多い)
            assert!(counts[0].0 > 100, "{}: {counts:?}", song.track_name);
        }
    }

    #[test]
    fn generate_chart_uses_every_lane_and_no_single_note_jacks() {
        for song in SONGS {
            for difficulty in ALL_DIFFICULTIES {
                let notes = generate_chart(song, difficulty);
                for lane in Lane::all() {
                    assert!(
                        notes.iter().any(|n| n.lanes.contains(&lane)),
                        "{} {difficulty:?}: {lane:?}を使うこと",
                        song.track_name
                    );
                }
                // 単押しが同じレーンに連続しない(擬似ランダムでも直前と同じレーンは選ばない)
                for pair in notes.windows(2) {
                    if pair[0].lanes.len() == 1 && pair[1].lanes.len() == 1 {
                        assert_ne!(
                            pair[0].lanes, pair[1].lanes,
                            "{} {difficulty:?}",
                            song.track_name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn beginner_charts_have_no_jumps_and_advanced_charts_do() {
        for song in SONGS {
            assert!(generate_chart(song, Difficulty::Beginner)
                .iter()
                .all(|n| n.lanes.len() == 1));
            assert!(generate_chart(song, Difficulty::Intermediate)
                .iter()
                .all(|n| n.lanes.len() == 1));
            assert!(generate_chart(song, Difficulty::Advanced)
                .iter()
                .any(|n| n.lanes.len() == 2));
        }
    }

    // --- RhythmGame::new(曲インデックス) ---

    #[test]
    fn new_uses_selected_song_chart() {
        for (i, song) in SONGS.iter().enumerate() {
            let game = RhythmGame::new(Difficulty::Intermediate, i);
            assert_eq!(game.song().track_name, song.track_name);
            assert_eq!(
                game.notes.len(),
                generate_chart(song, Difficulty::Intermediate).len()
            );
        }
    }

    #[test]
    fn new_with_out_of_range_song_index_falls_back_to_first_song() {
        let game = RhythmGame::new(Difficulty::Beginner, SONGS.len() + 3);
        assert_eq!(game.song().track_name, SONGS[0].track_name);
    }

    #[test]
    fn restart_clock_resets_elapsed_time() {
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
        game.set_elapsed_for_test(Duration::from_secs(10));
        game.restart_clock();
        assert!(game.started_at.elapsed() < Duration::from_secs(1));
    }

    // --- 入力判定(process_press) ---

    #[test]
    fn process_press_hits_single_lane_note_within_window() {
        let mut notes = vec![note(&[Lane::Up], 1000)];
        let w = judge_windows(Difficulty::Intermediate);
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
        let w = judge_windows(Difficulty::Intermediate);
        // 判定ウィンドウ外のレーンなしの入力(空打ち)
        let outcome = process_press(&mut notes, Lane::Left, Duration::from_millis(1000), w);
        assert!(outcome.is_none());
        assert!(!notes[0].is_judged());
        assert!(notes[0].pressed.is_empty());
    }

    #[test]
    fn process_press_rejects_outside_good_window() {
        let mut notes = vec![note(&[Lane::Up], 1000)];
        let w = judge_windows(Difficulty::Intermediate);
        let outcome = process_press(&mut notes, Lane::Up, Duration::from_millis(1151), w);
        assert!(outcome.is_none());
    }

    #[test]
    fn process_press_picks_nearest_note_when_multiple_in_window() {
        let mut notes = vec![note(&[Lane::Left], 1000), note(&[Lane::Left], 1080)];
        let w = judge_windows(Difficulty::Beginner);
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
        let w = judge_windows(Difficulty::Advanced);
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
        let w = judge_windows(Difficulty::Advanced); // perfect45/great70/good120
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
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
        game.notes = vec![note(&[Lane::Down], 500)];
        game.combo = 3;
        // Beginnerのgoodウィンドウは200ms
        game.set_elapsed_for_test(Duration::from_millis(701));
        game.update(Duration::from_millis(0));
        assert!(game.notes[0].is_judged());
        assert_eq!(game.notes[0].judgement, Some(Judgement::Miss));
        assert_eq!(game.tracker.total(), 1);
        assert_eq!(game.combo, 0);
        assert_eq!(game.last_judgement, Some(Judgement::Miss));
    }

    #[test]
    fn update_does_not_mark_miss_within_window() {
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
        game.notes = vec![note(&[Lane::Down], 500)];
        game.set_elapsed_for_test(Duration::from_millis(699));
        game.update(Duration::from_millis(0));
        assert!(!game.notes[0].is_judged());
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn handle_key_records_hit_and_builds_combo() {
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
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
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
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
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
        game.notes = vec![note(&[Lane::Up], 500), note(&[Lane::Down], 1500)];
        game.combo = 4;
        game.max_combo = 4;
        game.set_elapsed_for_test(Duration::from_millis(701));
        game.update(Duration::from_millis(0));
        assert_eq!(game.combo, 0);
        assert_eq!(game.max_combo, 4, "最大コンボは保持される");
    }

    #[test]
    fn is_finished_true_only_after_all_notes_judged() {
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
        game.notes = vec![note(&[Lane::Up], 500), note(&[Lane::Down], 900)];
        assert!(!game.is_finished());
        game.notes[0].judgement = Some(Judgement::Perfect);
        assert!(!game.is_finished());
        game.notes[1].judgement = Some(Judgement::Miss);
        assert!(game.is_finished());
    }

    #[test]
    fn handle_key_does_nothing_once_finished() {
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.notes[0].judgement = Some(Judgement::Perfect);
        game.set_elapsed_for_test(Duration::from_millis(500));
        game.handle_key(KeyEvent::from(KeyCode::Up));
        // 既に終了しているので何も記録されない
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn result_uses_score_tracker() {
        let mut game = RhythmGame::new(Difficulty::Beginner, 0);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.set_elapsed_for_test(Duration::from_millis(500));
        game.handle_key(KeyEvent::from(KeyCode::Up));
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.correct, 1);
        assert_eq!(result.total, 1);
    }
}
