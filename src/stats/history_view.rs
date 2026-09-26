use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::game::color_stack::GAME_ID as COLOR_STACK_ID;
use crate::game::count_mania::GAME_ID as COUNT_MANIA_ID;
use crate::game::memory::GAME_ID as MEMORY_ID;
use crate::game::mental_calc::GAME_ID as MENTAL_CALC_ID;
use crate::game::mirror_match::GAME_ID as MIRROR_MATCH_ID;
use crate::game::pattern_fill::GAME_ID as PATTERN_FILL_ID;
use crate::game::puzzle_connect::GAME_ID as PUZZLE_CONNECT_ID;
use crate::game::quick_draw::GAME_ID as QUICK_DRAW_ID;
use crate::game::reaction::GAME_ID as REACTION_ID;
use crate::game::rhythm::GAME_ID as RHYTHM_ID;
use crate::game::sequence::GAME_ID as SEQUENCE_ID;
use crate::game::shape_rotate::GAME_ID as SHAPE_ROTATE_ID;
use crate::game::GameResult;
use crate::stats::store;

const GAME_IDS: [&str; 12] = [
    SHAPE_ROTATE_ID,
    MIRROR_MATCH_ID,
    REACTION_ID,
    MENTAL_CALC_ID,
    PATTERN_FILL_ID,
    MEMORY_ID,
    SEQUENCE_ID,
    PUZZLE_CONNECT_ID,
    COUNT_MANIA_ID,
    COLOR_STACK_ID,
    RHYTHM_ID,
    QUICK_DRAW_ID,
];

/// 履歴グラフを並べるグリッドの列数・行数。GAME_IDSが全部入る大きさにする
const GRID_COLUMNS: u16 = 4;
const GRID_ROWS: u16 = (GAME_IDS.len() as u16).div_ceil(GRID_COLUMNS);

pub fn render(frame: &mut Frame, area: Rect) {
    let results = store::load_all().unwrap_or_default();
    if results.is_empty() {
        let paragraph = Paragraph::new("記録がありません")
            .block(Block::default().borders(Borders::ALL).title("履歴"));
        frame.render_widget(paragraph, area);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Fill(1); GRID_ROWS as usize])
        .split(area);

    let mut cells = Vec::with_capacity((GRID_ROWS * GRID_COLUMNS) as usize);
    for row in rows.iter() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Fill(1); GRID_COLUMNS as usize])
            .split(*row);
        cells.extend(cols.iter().copied());
    }

    for (game_id, cell) in GAME_IDS.iter().zip(cells.iter()) {
        render_game_history(frame, *cell, game_id, &results);
    }
}

/// game_idに一致する記録だけを抽出する(render_game_historyの計算部分をテスト可能に切り出したもの)
fn latencies_for(game_id: &str, results: &[GameResult]) -> Vec<f64> {
    results
        .iter()
        .filter(|r| r.game_id == game_id)
        .map(|r| r.avg_latency_ms)
        .collect()
}

fn render_game_history(frame: &mut Frame, area: Rect, game_id: &str, results: &[GameResult]) {
    let latencies = latencies_for(game_id, results);

    if latencies.is_empty() {
        let paragraph =
            Paragraph::new("記録なし").block(Block::default().borders(Borders::ALL).title(game_id));
        frame.render_widget(paragraph, area);
        return;
    }

    let max_latency = latencies.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    let points: Vec<(f64, f64)> = latencies
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f64, *v))
        .collect();

    let canvas = Canvas::default()
        .block(Block::default().borders(Borders::ALL).title(game_id))
        .x_bounds([0.0, (points.len().max(2) - 1) as f64])
        .y_bounds([0.0, max_latency])
        .paint(move |ctx| {
            for pair in points.windows(2) {
                let (x1, y1) = pair[0];
                let (x2, y2) = pair[1];
                ctx.draw(&CanvasLine {
                    x1,
                    y1,
                    x2,
                    y2,
                    color: Color::Cyan,
                });
            }
        });
    frame.render_widget(canvas, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Difficulty;
    use chrono::Utc;

    fn result(game_id: &str, avg_latency_ms: f64) -> GameResult {
        GameResult {
            game_id: game_id.to_string(),
            difficulty: Difficulty::Beginner,
            correct: 8,
            total: 10,
            avg_latency_ms,
            played_at: Utc::now(),
            forced_game_over: false,
        }
    }

    #[test]
    fn game_ids_has_no_duplicates() {
        let mut sorted = GAME_IDS.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), GAME_IDS.len(), "GAME_IDSに重複がある");
    }

    #[test]
    fn game_ids_covers_all_twelve_games() {
        assert_eq!(GAME_IDS.len(), 12);
        assert!(GAME_IDS.contains(&QUICK_DRAW_ID), "反射神経も履歴に出す");
        assert!(
            GAME_IDS.contains(&COUNT_MANIA_ID),
            "カウントメニアも履歴に出す"
        );
        assert!(
            GAME_IDS.contains(&COLOR_STACK_ID),
            "シタケシも履歴に出す"
        );
    }

    #[test]
    fn grid_has_a_cell_for_every_game() {
        assert!((GRID_COLUMNS * GRID_ROWS) as usize >= GAME_IDS.len());
    }

    #[test]
    fn latencies_for_filters_by_game_id_and_preserves_order() {
        let results = vec![
            result("shape_rotate", 100.0),
            result("mirror_match", 200.0),
            result("shape_rotate", 150.0),
        ];
        let latencies = latencies_for("shape_rotate", &results);
        assert_eq!(latencies, vec![100.0, 150.0]);
    }

    #[test]
    fn latencies_for_unknown_game_id_is_empty() {
        let results = vec![result("shape_rotate", 100.0)];
        assert!(latencies_for("no_such_game", &results).is_empty());
    }
}
