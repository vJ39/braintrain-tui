pub mod color_stack;
pub mod count_mania;
pub mod feedback;
pub mod memory;
pub mod mental_calc;
pub mod mirror_match;
pub mod pattern_fill;
pub mod puzzle_connect;
pub mod quick_draw;
pub mod reaction;
pub mod rhythm;
pub mod sequence;
pub mod shape_rotate;
pub mod theme;

use std::time::Duration;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyEvent, MouseEvent};
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
    /// マウス入力を受け取り、内部状態を更新する。`area`はこのゲームに割り当てられた
    /// 描画領域で、renderに渡されるものと同じ。既定では何もしない
    fn handle_mouse(&mut self, _mouse: MouseEvent, _area: Rect) {}
    /// tick駆動の更新(タイマー等)。経過時間を渡す
    fn update(&mut self, dt: Duration);
    /// 描画。Frame全体でなく割り当てられたRectのみ使う
    fn render(&self, frame: &mut Frame, area: Rect);
    /// このゲームのセッションが終わったか
    fn is_finished(&self) -> bool;
    /// 終了後にスコアを取り出す
    fn result(&self) -> GameResult;
}

/// クリック座標(column, row)がareaの矩形内にあるかを判定する。
/// column_index/row_indexはそれぞれ1軸しか見ないため、呼び出し側はまずこれで
/// 2次元的にエリア内かどうかを確認してから使うこと
pub fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x + area.width
        && row >= area.y
        && row < area.y + area.height
}

/// エリアを横方向にN列に等分し、クリック座標(column)がどの列(0-indexed)に
/// 属するかを返す。選択肢を横並びに表示するゲーム(2択等)のクリック判定に使う
pub fn column_index(area: Rect, column: u16, column_count: u16) -> Option<usize> {
    if column_count == 0 || area.width == 0 {
        return None;
    }
    if column < area.x || column >= area.x + area.width {
        return None;
    }
    let relative = column - area.x;
    let column_width = area.width / column_count;
    if column_width == 0 {
        return None;
    }
    let index = (relative / column_width) as usize;
    Some(index.min(column_count as usize - 1))
}

/// エリアを縦方向にN行に等分し、クリック座標(row)がどの行(0-indexed)に
/// 属するかを返す。選択肢を縦並びに表示するゲーム(4択等)のクリック判定に使う
pub fn row_index(area: Rect, row: u16, row_count: u16) -> Option<usize> {
    if row_count == 0 || area.height == 0 {
        return None;
    }
    if row < area.y || row >= area.y + area.height {
        return None;
    }
    let relative = row - area.y;
    let row_height = area.height / row_count;
    if row_height == 0 {
        return None;
    }
    let index = (relative / row_height) as usize;
    Some(index.min(row_count as usize - 1))
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
#[derive(Debug)]
pub struct ScoreTracker {
    correct: u32,
    total: u32,
    latencies_ms: Vec<f64>,
    /// このセッションで出題する問題数
    session_length: u32,
}

impl Default for ScoreTracker {
    fn default() -> Self {
        Self::with_session_length(QUESTIONS_PER_SESSION)
    }
}

impl ScoreTracker {
    /// 全ゲーム共通の問題数(QUESTIONS_PER_SESSION)のセッション
    pub fn new() -> Self {
        Self::default()
    }

    /// 問題数を指定したセッション。共通の問題数と違うゲームが使う
    pub fn with_session_length(len: u32) -> Self {
        Self {
            correct: 0,
            total: 0,
            latencies_ms: Vec::new(),
            session_length: len,
        }
    }

    pub fn session_length(&self) -> u32 {
        self.session_length
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
        self.total >= self.session_length
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
    fn tracker_new_keeps_default_session_length() {
        // 既存のnew()/default()は全ゲーム共通の問題数のまま
        assert_eq!(ScoreTracker::new().session_length(), QUESTIONS_PER_SESSION);
        assert_eq!(
            ScoreTracker::default().session_length(),
            QUESTIONS_PER_SESSION
        );
    }

    #[test]
    fn tracker_with_session_length_finishes_after_given_questions() {
        let mut tracker = ScoreTracker::with_session_length(20);
        assert_eq!(tracker.session_length(), 20);
        for _ in 0..19 {
            tracker.record(true, 50.0);
        }
        assert!(!tracker.is_session_finished(), "19問では終わらない");
        tracker.record(false, 50.0);
        assert!(tracker.is_session_finished(), "20問で終わる");
        let result = tracker.to_result("test_game", Difficulty::Beginner);
        assert_eq!(result.total, 20);
        assert_eq!(result.correct, 19);
    }

    #[test]
    fn tracker_with_shorter_session_length_finishes_early() {
        let mut tracker = ScoreTracker::with_session_length(3);
        for _ in 0..2 {
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

    fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect::new(x, y, width, height)
    }

    #[test]
    fn contains_true_inside_area() {
        let area = rect(5, 10, 20, 8);
        assert!(contains(area, 5, 10), "左上端は含む");
        assert!(contains(area, 24, 17), "右下端(width-1,height-1)は含む");
    }

    #[test]
    fn contains_false_outside_area() {
        let area = rect(5, 10, 20, 8);
        assert!(!contains(area, 4, 10), "左端より左");
        assert!(!contains(area, 25, 10), "右端(x+width)ちょうどは含まない");
        assert!(!contains(area, 5, 9), "上端より上");
        assert!(!contains(area, 5, 18), "下端(y+height)ちょうどは含まない");
    }

    #[test]
    fn column_index_splits_area_into_equal_columns() {
        let area = rect(0, 0, 20, 5);
        assert_eq!(column_index(area, 0, 2), Some(0));
        assert_eq!(column_index(area, 9, 2), Some(0));
        assert_eq!(column_index(area, 10, 2), Some(1));
        assert_eq!(column_index(area, 19, 2), Some(1));
    }

    #[test]
    fn column_index_respects_area_offset() {
        let area = rect(100, 0, 20, 5);
        assert_eq!(column_index(area, 99, 2), None, "エリアより左は範囲外");
        assert_eq!(column_index(area, 100, 2), Some(0));
        assert_eq!(column_index(area, 120, 2), None, "エリアより右は範囲外");
    }

    #[test]
    fn column_index_clamps_rounding_remainder_to_last_column() {
        // 幅が列数で割り切れない場合、余りは最後の列に含める
        let area = rect(0, 0, 7, 5);
        assert_eq!(column_index(area, 6, 3), Some(2));
    }

    #[test]
    fn column_index_with_zero_columns_or_width_is_none() {
        assert_eq!(column_index(rect(0, 0, 10, 5), 0, 0), None);
        assert_eq!(column_index(rect(0, 0, 0, 5), 0, 2), None);
    }

    #[test]
    fn row_index_splits_area_into_equal_rows() {
        let area = rect(0, 0, 10, 8);
        assert_eq!(row_index(area, 0, 4), Some(0));
        assert_eq!(row_index(area, 1, 4), Some(0));
        assert_eq!(row_index(area, 2, 4), Some(1));
        assert_eq!(row_index(area, 7, 4), Some(3));
    }

    #[test]
    fn row_index_respects_area_offset() {
        let area = rect(0, 50, 10, 8);
        assert_eq!(row_index(area, 49, 4), None, "エリアより上は範囲外");
        assert_eq!(row_index(area, 50, 4), Some(0));
        assert_eq!(row_index(area, 58, 4), None, "エリアより下は範囲外");
    }

    #[test]
    fn row_index_with_zero_rows_or_height_is_none() {
        assert_eq!(row_index(rect(0, 0, 10, 8), 0, 0), None);
        assert_eq!(row_index(rect(0, 0, 10, 0), 0, 4), None);
    }
}
