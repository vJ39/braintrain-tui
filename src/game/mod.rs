pub mod mental_calc;
pub mod mirror_match;
pub mod reaction;
pub mod shape_rotate;

use std::time::Duration;

use chrono::{DateTime, Utc};
use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;
use serde::{Deserialize, Serialize};

pub const QUESTIONS_PER_SESSION: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Difficulty {
    Beginner,
    Intermediate,
    Advanced,
}

pub trait Game {
    /// キー入力を受け取り、内部状態を更新する
    fn handle_key(&mut self, key: KeyEvent);
    /// tick駆動の更新(タイマー等)。経過時間を渡す
    fn update(&mut self, dt: Duration);
    /// 描画。Frame全体でなく割り当てられたRectのみ使う
    fn render(&self, frame: &mut Frame, area: Rect);
    /// このゲームのセッションが終わったか
    fn is_finished(&self) -> bool;
    /// 終了後にスコアを取り出す
    fn result(&self) -> GameResult;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameResult {
    pub game_id: String,
    pub difficulty: Difficulty,
    pub correct: u32,
    pub total: u32,
    pub avg_latency_ms: f64,
    pub played_at: DateTime<Utc>,
}

/// 正誤とレイテンシの記録を積み上げ、GameResultにまとめる共通ヘルパー
#[derive(Debug, Default)]
pub struct ScoreTracker {
    correct: u32,
    total: u32,
    latencies_ms: Vec<f64>,
}

impl ScoreTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, is_correct: bool, latency_ms: f64) {
        self.total += 1;
        if is_correct {
            self.correct += 1;
        }
        self.latencies_ms.push(latency_ms);
    }

    pub fn total(&self) -> u32 {
        self.total
    }

    pub fn is_session_finished(&self) -> bool {
        self.total >= QUESTIONS_PER_SESSION
    }

    pub fn to_result(&self, game_id: &'static str, difficulty: Difficulty) -> GameResult {
        let avg_latency_ms = if self.latencies_ms.is_empty() {
            0.0
        } else {
            self.latencies_ms.iter().sum::<f64>() / self.latencies_ms.len() as f64
        };
        GameResult {
            game_id: game_id.to_string(),
            difficulty,
            correct: self.correct,
            total: self.total,
            avg_latency_ms,
            played_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_computes_average_latency() {
        let mut tracker = ScoreTracker::new();
        tracker.record(true, 100.0);
        tracker.record(false, 200.0);
        let result = tracker.to_result("test_game", Difficulty::Beginner);
        assert_eq!(result.correct, 1);
        assert_eq!(result.total, 2);
        assert!((result.avg_latency_ms - 150.0).abs() < 1e-9);
    }

    #[test]
    fn tracker_is_finished_after_configured_questions() {
        let mut tracker = ScoreTracker::new();
        for _ in 0..QUESTIONS_PER_SESSION - 1 {
            tracker.record(true, 50.0);
        }
        assert!(!tracker.is_session_finished());
        tracker.record(true, 50.0);
        assert!(tracker.is_session_finished());
    }

    #[test]
    fn tracker_with_no_records_has_zero_avg_latency() {
        let tracker = ScoreTracker::new();
        let result = tracker.to_result("test_game", Difficulty::Beginner);
        assert_eq!(result.total, 0);
        assert_eq!(result.avg_latency_ms, 0.0);
    }
}
