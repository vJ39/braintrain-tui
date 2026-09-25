use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use rand::Rng;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};

pub const GAME_ID: &str = "rhythm";

/// 描画上、判定ラインより上に表示するノーツの行数
const VISIBLE_ROWS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Up,
    Down,
    Left,
    Right,
}

impl Lane {
    /// DDR標準のレーン並び(画面左から ←↓↑→)
    fn all() -> [Lane; 4] {
        [Lane::Left, Lane::Down, Lane::Up, Lane::Right]
    }
}

/// 難易度ごとのノーツ間隔(判定ラインへの到達間隔)
fn note_interval(difficulty: Difficulty) -> Duration {
    match difficulty {
        Difficulty::Beginner => Duration::from_millis(1000),
        Difficulty::Intermediate => Duration::from_millis(600),
        Difficulty::Advanced => Duration::from_millis(430),
    }
}

/// 難易度ごとの判定ウィンドウ(±この時間内なら正解)
fn judge_window(difficulty: Difficulty) -> Duration {
    match difficulty {
        Difficulty::Beginner => Duration::from_millis(200),
        Difficulty::Intermediate => Duration::from_millis(150),
        Difficulty::Advanced => Duration::from_millis(100),
    }
}

#[derive(Debug, Clone, Copy)]
struct Note {
    lane: Lane,
    /// 判定ラインに到達すべき予定時刻(ゲーム内経過時間基準)
    hit_at: Duration,
    /// 既に判定済み(正解/不正解/自動失敗のいずれか)か
    judged: bool,
}

/// 2つのDurationの差の絶対値
fn duration_diff(a: Duration, b: Duration) -> Duration {
    a.abs_diff(b)
}

/// ノーツの出現(判定ライン到達予定)時刻とレーンのリストをQUESTIONS_PER_SESSION分生成する純粋関数
fn generate_notes(rng: &mut impl Rng, difficulty: Difficulty) -> Vec<Note> {
    let interval = note_interval(difficulty);
    let lanes = Lane::all();
    (1..=crate::game::QUESTIONS_PER_SESSION)
        .map(|i| Note {
            lane: lanes[rng.gen_range(0..lanes.len())],
            hit_at: interval * i,
            judged: false,
        })
        .collect()
}

/// 指定レーン・指定時刻の入力に対して、判定ウィンドウ内で最も近い未判定ノーツのインデックスを返す
fn judge(notes: &[Note], lane: Lane, pressed_at: Duration, window: Duration) -> Option<usize> {
    notes
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.judged && n.lane == lane)
        .filter(|(_, n)| duration_diff(n.hit_at, pressed_at) <= window)
        .min_by_key(|(_, n)| duration_diff(n.hit_at, pressed_at))
        .map(|(i, _)| i)
}

pub struct RhythmGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    notes: Vec<Note>,
    /// ゲーム内経過時間(累積)
    elapsed: Duration,
}

impl RhythmGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            notes: generate_notes(&mut rng, difficulty),
            elapsed: Duration::ZERO,
        }
    }

    fn record_and_play(&mut self, is_correct: bool, latency_ms: f64) {
        self.tracker.record(is_correct, latency_ms);
        audio::play_se(if is_correct {
            SeKind::Correct
        } else {
            SeKind::Incorrect
        });
    }
}

impl Game for RhythmGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        let lane = match key.code {
            KeyCode::Up => Lane::Up,
            KeyCode::Down => Lane::Down,
            KeyCode::Left => Lane::Left,
            KeyCode::Right => Lane::Right,
            _ => return,
        };

        let window = judge_window(self.difficulty);
        match judge(&self.notes, lane, self.elapsed, window) {
            Some(idx) => {
                let latency_ms = duration_diff(self.notes[idx].hit_at, self.elapsed).as_millis() as f64;
                self.notes[idx].judged = true;
                self.record_and_play(true, latency_ms);
            }
            None => {
                // 判定ウィンドウ内に該当レーンのノーツが無い = お手つきとして不正解
                self.record_and_play(false, window.as_millis() as f64);
            }
        }
    }

    fn update(&mut self, dt: Duration) {
        if self.tracker.is_session_finished() {
            return;
        }
        self.elapsed += dt;
        let window = judge_window(self.difficulty);
        for i in 0..self.notes.len() {
            if self.tracker.is_session_finished() {
                break;
            }
            let note = self.notes[i];
            if !note.judged && self.elapsed > note.hit_at + window {
                self.notes[i].judged = true;
                self.record_and_play(false, window.as_millis() as f64 * 2.0);
            }
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let interval = note_interval(self.difficulty);
        let lanes = Lane::all();

        // grid[0]が判定ライン直前(最も近い)、添字が大きいほどまだ遠い(画面下方)ノーツ
        let mut grid = [[' '; 4]; VISIBLE_ROWS];
        for note in &self.notes {
            if note.judged {
                continue;
            }
            let lane_idx = lanes.iter().position(|&l| l == note.lane).unwrap();
            let remaining = note.hit_at.saturating_sub(self.elapsed);
            let interval_ms = interval.as_millis().max(1);
            let steps_ahead = (remaining.as_millis() / interval_ms) as usize;
            if steps_ahead < VISIBLE_ROWS {
                grid[steps_ahead][lane_idx] = '●';
            }
        }

        // DDR同様、判定ラインを上部に固定し、ノーツは画面下方から出現して
        // 判定ラインに向かって上昇してくるように見せる
        let mut lines = vec![
            Line::from(Span::raw("   ←      ↓      ↑      →   ")),
            Line::from(Span::styled(
                "=============================",
                Style::default().add_modifier(Modifier::BOLD),
            )),
        ];
        for row in grid.iter() {
            let text: String = row.iter().map(|c| format!("   {c}   ")).collect();
            lines.push(Line::from(Span::raw(text)));
        }

        let progress = format!(
            "{} / {}問",
            self.tracker.total(),
            crate::game::QUESTIONS_PER_SESSION
        );
        lines.push(Line::from(Span::raw(format!(
            "矢印キーでノーツが判定ラインに来た瞬間に押そう   {progress}"
        ))));

        let paragraph = Paragraph::new(lines)
            .alignment(ratatui::layout::Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("リズム/タイミング合わせ"),
            );
        frame.render_widget(paragraph, area);
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
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

    #[test]
    fn generate_notes_has_expected_count_and_spacing() {
        let mut rng = StdRng::seed_from_u64(1);
        let notes = generate_notes(&mut rng, Difficulty::Intermediate);
        assert_eq!(notes.len(), crate::game::QUESTIONS_PER_SESSION as usize);
        let interval = note_interval(Difficulty::Intermediate);
        for (i, note) in notes.iter().enumerate() {
            assert_eq!(note.hit_at, interval * (i as u32 + 1));
            assert!(!note.judged);
        }
    }

    #[test]
    fn judge_accepts_within_window_boundary() {
        let notes = vec![Note {
            lane: Lane::Up,
            hit_at: Duration::from_millis(1000),
            judged: false,
        }];
        let window = Duration::from_millis(150);
        // ちょうど境界(±window)は正解扱い
        let early = judge(&notes, Lane::Up, Duration::from_millis(850), window);
        assert_eq!(early, Some(0));
        let late = judge(&notes, Lane::Up, Duration::from_millis(1150), window);
        assert_eq!(late, Some(0));
    }

    #[test]
    fn judge_rejects_just_outside_window() {
        let notes = vec![Note {
            lane: Lane::Up,
            hit_at: Duration::from_millis(1000),
            judged: false,
        }];
        let window = Duration::from_millis(150);
        let result = judge(&notes, Lane::Up, Duration::from_millis(1151), window);
        assert_eq!(result, None);
    }

    #[test]
    fn judge_ignores_different_lane() {
        let notes = vec![Note {
            lane: Lane::Up,
            hit_at: Duration::from_millis(1000),
            judged: false,
        }];
        let window = Duration::from_millis(150);
        // 時刻はぴったりでもレーンが違えば判定されない
        let result = judge(&notes, Lane::Down, Duration::from_millis(1000), window);
        assert_eq!(result, None);
    }

    #[test]
    fn judge_ignores_already_judged_note() {
        let notes = vec![Note {
            lane: Lane::Up,
            hit_at: Duration::from_millis(1000),
            judged: true,
        }];
        let window = Duration::from_millis(150);
        let result = judge(&notes, Lane::Up, Duration::from_millis(1000), window);
        assert_eq!(result, None);
    }

    #[test]
    fn judge_picks_nearest_note_when_multiple_in_window() {
        let notes = vec![
            Note {
                lane: Lane::Left,
                hit_at: Duration::from_millis(1000),
                judged: false,
            },
            Note {
                lane: Lane::Left,
                hit_at: Duration::from_millis(1080),
                judged: false,
            },
        ];
        let window = Duration::from_millis(150);
        let result = judge(&notes, Lane::Left, Duration::from_millis(1050), window);
        // 1050msに最も近いのは2つ目(1080ms, 差30ms) 1つ目は差50ms
        assert_eq!(result, Some(1));
    }

    #[test]
    fn handle_key_records_correct_within_window() {
        let mut game = RhythmGame::new(Difficulty::Beginner);
        game.notes = vec![Note {
            lane: Lane::Right,
            hit_at: Duration::from_millis(1000),
            judged: false,
        }];
        game.elapsed = Duration::from_millis(1000);
        game.handle_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(game.tracker.total(), 1);
        assert!(game.notes[0].judged);
    }

    #[test]
    fn handle_key_records_incorrect_when_no_note_in_lane() {
        let mut game = RhythmGame::new(Difficulty::Beginner);
        game.notes = vec![Note {
            lane: Lane::Right,
            hit_at: Duration::from_millis(1000),
            judged: false,
        }];
        game.elapsed = Duration::from_millis(1000);
        // ノーツが無いレーン(Left)を押す = お手つき不正解
        game.handle_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(game.tracker.total(), 1);
        // 元のノーツは未判定のまま残る
        assert!(!game.notes[0].judged);
    }

    #[test]
    fn update_auto_fails_note_after_window_passes_without_input() {
        let mut game = RhythmGame::new(Difficulty::Beginner);
        game.notes = vec![Note {
            lane: Lane::Down,
            hit_at: Duration::from_millis(500),
            judged: false,
        }];
        // 判定ウィンドウ(±200ms)を過ぎるまで進める
        game.update(Duration::from_millis(701));
        assert_eq!(game.tracker.total(), 1);
        assert!(game.notes[0].judged);
    }

    #[test]
    fn update_does_not_auto_fail_within_window() {
        let mut game = RhythmGame::new(Difficulty::Beginner);
        game.notes = vec![Note {
            lane: Lane::Down,
            hit_at: Duration::from_millis(500),
            judged: false,
        }];
        game.update(Duration::from_millis(699));
        assert_eq!(game.tracker.total(), 0);
        assert!(!game.notes[0].judged);
    }

    #[test]
    fn is_finished_true_after_ten_records() {
        let mut game = RhythmGame::new(Difficulty::Beginner);
        for note in game.notes.iter_mut() {
            note.judged = true;
        }
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            game.record_and_play(true, 0.0);
        }
        assert!(game.is_finished());
    }
}
