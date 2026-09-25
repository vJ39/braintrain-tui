use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use rand::Rng;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::game::{Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "rhythm";

/// ノーツが画面下端に出現してから判定ラインに到達するまでの所要時間。
/// BPM/譜面とは独立した「スクロール速度」の概念(仕様5)。
const SCROLL_TRAVEL_TIME: Duration = Duration::from_millis(1500);

/// 直近の判定(PERFECT/GREAT/GOOD/MISS)を画面に表示し続ける時間
const JUDGEMENT_DISPLAY_HOLD: Duration = Duration::from_millis(600);

/// 譜面の先頭に入れる助走(拍数)。SCROLL_TRAVEL_TIMEより確実に長くなる値にし、
/// 開始直後にノーツがいきなり判定ライン付近に湧かないようにする。
const LEAD_IN_BEATS: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Up,
    Down,
    Left,
    Right,
}

impl Lane {
    /// DDR標準のレーン並び(画面左から ←↓↑→)。表示位置と入力キーの対応はこの順序に一致させる。
    fn all() -> [Lane; 4] {
        [Lane::Left, Lane::Down, Lane::Up, Lane::Right]
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

fn bpm_for(difficulty: Difficulty) -> f64 {
    match difficulty {
        Difficulty::Beginner => 90.0,
        Difficulty::Intermediate => 120.0,
        Difficulty::Advanced => 150.0,
    }
}

/// 譜面全体のノーツ数(QUESTIONS_PER_SESSIONには縛られない。仕様14)
fn target_note_count(difficulty: Difficulty) -> usize {
    match difficulty {
        Difficulty::Beginner => 20,
        Difficulty::Intermediate => 28,
        Difficulty::Advanced => 36,
    }
}

/// 「気持ちよく踏める」フレーズの最小単位。完全ランダムな単発ノーツ生成を避け、
/// 4分/8分音符・左右交互・上下往復・同時押しといった認識可能なパターンを組み合わせる(仕様4)。
#[derive(Debug, PartialEq)]
struct Phrase {
    /// (フレーズ先頭からの相対拍, そのタイミングで踏むレーン群)
    notes: &'static [(f64, &'static [Lane])],
    /// このフレーズが専有する拍数(次フレーズはこの分だけ後ろにずれる)
    span_beats: f64,
}

/// 左右交互(4分音符)
const PHRASE_ALT_LR: Phrase = Phrase {
    notes: &[
        (0.0, &[Lane::Left] as &[Lane]),
        (1.0, &[Lane::Right]),
        (2.0, &[Lane::Left]),
        (3.0, &[Lane::Right]),
    ],
    span_beats: 4.0,
};

/// ←↓↑→ と歩くように踏む基本パターン(4分音符)
const PHRASE_WALK: Phrase = Phrase {
    notes: &[
        (0.0, &[Lane::Left] as &[Lane]),
        (1.0, &[Lane::Down]),
        (2.0, &[Lane::Up]),
        (3.0, &[Lane::Right]),
    ],
    span_beats: 4.0,
};

/// ↑↓の往復(4分音符)
const PHRASE_UD_REPEAT: Phrase = Phrase {
    notes: &[
        (0.0, &[Lane::Up] as &[Lane]),
        (1.0, &[Lane::Down]),
        (2.0, &[Lane::Up]),
        (3.0, &[Lane::Down]),
    ],
    span_beats: 4.0,
};

/// 8分音符を含む細かいフレーズ
const PHRASE_EIGHTH: Phrase = Phrase {
    notes: &[
        (0.0, &[Lane::Left] as &[Lane]),
        (0.5, &[Lane::Down]),
        (1.0, &[Lane::Up]),
        (1.5, &[Lane::Right]),
    ],
    span_beats: 2.0,
};

/// 同時押し(左右)を含むフレーズ
const PHRASE_JUMP: Phrase = Phrase {
    notes: &[
        (0.0, &[Lane::Left, Lane::Right] as &[Lane]),
        (1.0, &[Lane::Down] as &[Lane]),
        (2.0, &[Lane::Up] as &[Lane]),
        (3.0, &[Lane::Left, Lane::Right]),
    ],
    span_beats: 4.0,
};

/// 難易度ごとに使ってよいフレーズの集合。Beginnerは4分音符・単押しのみ、
/// Intermediateで8分音符・上下往復が混ざり、Advancedで同時押しも登場する。
fn phrases_for_difficulty(difficulty: Difficulty) -> &'static [Phrase] {
    match difficulty {
        Difficulty::Beginner => &[PHRASE_ALT_LR, PHRASE_WALK],
        Difficulty::Intermediate => {
            &[PHRASE_ALT_LR, PHRASE_WALK, PHRASE_UD_REPEAT, PHRASE_EIGHTH]
        }
        Difficulty::Advanced => &[PHRASE_WALK, PHRASE_UD_REPEAT, PHRASE_EIGHTH, PHRASE_JUMP],
    }
}

/// フレーズをランダムに選んで連結し、(拍位置, レーン群)のリスト(=譜面の骨格)を作る純粋関数。
/// フレーズ内部のパターンは固定なので、リズムとして認識できる構造は保たれる。
fn build_beat_pattern(rng: &mut impl Rng, difficulty: Difficulty) -> Vec<(f64, Vec<Lane>)> {
    let phrases = phrases_for_difficulty(difficulty);
    let target = target_note_count(difficulty);
    let mut beat_offset = LEAD_IN_BEATS;
    let mut out = Vec::with_capacity(target);
    while out.len() < target {
        let phrase = &phrases[rng.gen_range(0..phrases.len())];
        for &(rel_beat, lanes) in phrase.notes {
            if out.len() >= target {
                break;
            }
            out.push((beat_offset + rel_beat, lanes.to_vec()));
        }
        beat_offset += phrase.span_beats;
    }
    out
}

/// beat位置(仕様3: `hit_time = beat * 60 / BPM`)を実時間のノーツ到達予定時刻に変換する純粋関数
fn beats_to_notes(pattern: &[(f64, Vec<Lane>)], bpm: f64) -> Vec<Note> {
    pattern
        .iter()
        .map(|(beat, lanes)| Note::new(lanes.clone(), Duration::from_secs_f64(beat * 60.0 / bpm)))
        .collect()
}

fn generate_chart(rng: &mut impl Rng, difficulty: Difficulty) -> Vec<Note> {
    let pattern = build_beat_pattern(rng, difficulty);
    beats_to_notes(&pattern, bpm_for(difficulty))
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
            !n.is_judged() && n.lanes.contains(&lane) && !n.pressed.iter().any(|(l, _, _)| *l == lane)
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
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            notes: generate_chart(&mut rng, difficulty),
            started_at: Instant::now(),
            combo: 0,
            max_combo: 0,
            last_judgement: None,
            last_judgement_at: Duration::ZERO,
        }
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

        let header = Line::from(Span::styled(
            "   ←      ↓      ↑      →   ",
            Style::default().add_modifier(Modifier::BOLD),
        ));
        let judge_line = Line::from(Span::styled(
            "=============================",
            Style::default().add_modifier(Modifier::BOLD),
        ));

        // TUIの縦セル数を可能な限り使い、位置計算自体は連続量(progress)で行う(仕様15)
        let reserved_rows: u16 = 5;
        let track_height = area.height.saturating_sub(reserved_rows).max(4) as usize;

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

        let mut lines = vec![header, judge_line];
        for row in grid.iter() {
            let text: String = row.iter().map(|c| format!("   {c}   ")).collect();
            lines.push(Line::from(Span::raw(text)));
        }

        let judged_count = self.notes.iter().filter(|n| n.is_judged()).count();
        let judgement_text = if now.saturating_sub(self.last_judgement_at) <= JUDGEMENT_DISPLAY_HOLD {
            self.last_judgement.map(judgement_label).unwrap_or("")
        } else {
            ""
        };

        lines.push(Line::from(Span::raw(format!(
            "COMBO {}  (MAX {})   {judgement_text}",
            self.combo, self.max_combo
        ))));
        lines.push(Line::from(Span::raw(format!(
            "判定ラインに矢印キーを合わせよう   {judged_count} / {}ノーツ",
            self.notes.len()
        ))));

        let paragraph = Paragraph::new(lines)
            .alignment(ratatui::layout::Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("リズム/タイミング合わせ(DDR風)"),
            );
        frame.render_widget(paragraph, area);
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
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn note(lanes: &[Lane], hit_at_ms: u64) -> Note {
        Note::new(lanes.to_vec(), Duration::from_millis(hit_at_ms))
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

    // --- 譜面生成(build_beat_pattern / beats_to_notes) ---

    #[test]
    fn build_beat_pattern_reaches_target_note_count() {
        let mut rng = StdRng::seed_from_u64(1);
        for &difficulty in &[
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let pattern = build_beat_pattern(&mut rng, difficulty);
            assert_eq!(pattern.len(), target_note_count(difficulty));
        }
    }

    #[test]
    fn build_beat_pattern_beats_are_non_decreasing_and_after_lead_in() {
        let mut rng = StdRng::seed_from_u64(7);
        let pattern = build_beat_pattern(&mut rng, Difficulty::Intermediate);
        assert!(pattern[0].0 >= LEAD_IN_BEATS);
        for pair in pattern.windows(2) {
            assert!(pair[1].0 >= pair[0].0);
        }
    }

    #[test]
    fn beginner_phrases_are_quarter_notes_only_single_lane() {
        // Beginnerは「4分音符中心の簡単な譜面」「左右交互」のみで、8分音符や同時押しを含まない
        assert_eq!(
            phrases_for_difficulty(Difficulty::Beginner),
            &[PHRASE_ALT_LR, PHRASE_WALK]
        );
        let mut rng = StdRng::seed_from_u64(3);
        let pattern = build_beat_pattern(&mut rng, Difficulty::Beginner);
        for (beat, lanes) in &pattern {
            assert_eq!(lanes.len(), 1, "beginnerは同時押しを含まない");
            assert_eq!(beat.fract(), 0.0, "beginnerは4分音符のみ");
        }
    }

    #[test]
    fn advanced_phrases_include_eighth_notes_and_simultaneous_press() {
        assert!(phrases_for_difficulty(Difficulty::Advanced).contains(&PHRASE_EIGHTH));
        assert!(phrases_for_difficulty(Difficulty::Advanced).contains(&PHRASE_JUMP));
        // PHRASE_JUMP自体が同時押しを含むことを確認
        assert!(PHRASE_JUMP.notes.iter().any(|(_, lanes)| lanes.len() == 2));
        // PHRASE_EIGHTH自体が8分音符(拍の端数)を含むことを確認
        assert!(PHRASE_EIGHTH.notes.iter().any(|(beat, _)| beat.fract() != 0.0));
    }

    #[test]
    fn beats_to_notes_uses_bpm_to_compute_hit_at() {
        let pattern = vec![(1.0, vec![Lane::Up]), (1.5, vec![Lane::Down])];
        // BPM120 => 1beat = 500ms
        let notes = beats_to_notes(&pattern, 120.0);
        assert_eq!(notes[0].hit_at, Duration::from_millis(500));
        assert_eq!(notes[1].hit_at, Duration::from_millis(750));
    }

    // --- 入力判定(process_press) ---

    #[test]
    fn process_press_hits_single_lane_note_within_window() {
        let mut notes = vec![note(&[Lane::Up], 1000)];
        let w = judge_windows(Difficulty::Intermediate);
        let outcome = process_press(&mut notes, Lane::Up, Duration::from_millis(1010), w).unwrap();
        assert_eq!(outcome.press_judgement, Judgement::Perfect);
        assert_eq!(outcome.note_final, Some((Judgement::Perfect, Duration::from_millis(10))));
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
        let outcome = process_press(&mut notes, Lane::Left, Duration::from_millis(1050), w).unwrap();
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
        let second = process_press(&mut notes, Lane::Right, Duration::from_millis(1010), w).unwrap();
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
        let mut game = RhythmGame::new(Difficulty::Beginner);
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
        let mut game = RhythmGame::new(Difficulty::Beginner);
        game.notes = vec![note(&[Lane::Down], 500)];
        game.set_elapsed_for_test(Duration::from_millis(699));
        game.update(Duration::from_millis(0));
        assert!(!game.notes[0].is_judged());
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn handle_key_records_hit_and_builds_combo() {
        let mut game = RhythmGame::new(Difficulty::Beginner);
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
        let mut game = RhythmGame::new(Difficulty::Beginner);
        game.notes = vec![note(&[Lane::Right], 1000)];
        game.combo = 5;
        game.set_elapsed_for_test(Duration::from_millis(1000));
        // ノーツが無いレーン(Left)への空打ち
        game.handle_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(game.tracker.total(), 0, "空打ちは判定済みノーツ数を増やさない");
        assert!(!game.notes[0].is_judged());
        assert_eq!(game.combo, 0, "空打ちでコンボが切れる");
    }

    #[test]
    fn miss_resets_combo() {
        let mut game = RhythmGame::new(Difficulty::Beginner);
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
        let mut game = RhythmGame::new(Difficulty::Beginner);
        game.notes = vec![note(&[Lane::Up], 500), note(&[Lane::Down], 900)];
        assert!(!game.is_finished());
        game.notes[0].judgement = Some(Judgement::Perfect);
        assert!(!game.is_finished());
        game.notes[1].judgement = Some(Judgement::Miss);
        assert!(game.is_finished());
    }

    #[test]
    fn handle_key_does_nothing_once_finished() {
        let mut game = RhythmGame::new(Difficulty::Beginner);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.notes[0].judgement = Some(Judgement::Perfect);
        game.set_elapsed_for_test(Duration::from_millis(500));
        game.handle_key(KeyEvent::from(KeyCode::Up));
        // 既に終了しているので何も記録されない
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn result_uses_score_tracker() {
        let mut game = RhythmGame::new(Difficulty::Beginner);
        game.notes = vec![note(&[Lane::Up], 500)];
        game.set_elapsed_for_test(Duration::from_millis(500));
        game.handle_key(KeyEvent::from(KeyCode::Up));
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.correct, 1);
        assert_eq!(result.total, 1);
    }
}
