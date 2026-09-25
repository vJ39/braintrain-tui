use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::game::memory::GAME_ID as MEMORY_ID;
use crate::game::mental_calc::GAME_ID as MENTAL_CALC_ID;
use crate::game::mirror_match::GAME_ID as MIRROR_MATCH_ID;
use crate::game::pattern_fill::GAME_ID as PATTERN_FILL_ID;
use crate::game::puzzle_connect::GAME_ID as PUZZLE_CONNECT_ID;
use crate::game::reaction::GAME_ID as REACTION_ID;
use crate::game::rhythm::GAME_ID as RHYTHM_ID;
use crate::game::sequence::GAME_ID as SEQUENCE_ID;
use crate::game::shape_rotate::GAME_ID as SHAPE_ROTATE_ID;
use crate::game::GameResult;
use crate::stats::store;

const GAME_IDS: [&str; 9] = [
    SHAPE_ROTATE_ID,
    MIRROR_MATCH_ID,
    REACTION_ID,
    MENTAL_CALC_ID,
    PATTERN_FILL_ID,
    MEMORY_ID,
    SEQUENCE_ID,
    PUZZLE_CONNECT_ID,
    RHYTHM_ID,
];

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
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    let mut cells = Vec::with_capacity(9);
    for row in rows.iter() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(*row);
        cells.extend(cols.iter().copied());
    }

    for (game_id, cell) in GAME_IDS.iter().zip(cells.iter()) {
        render_game_history(frame, *cell, game_id, &results);
    }
}

fn render_game_history(frame: &mut Frame, area: Rect, game_id: &str, results: &[GameResult]) {
    let latencies: Vec<f64> = results
        .iter()
        .filter(|r| r.game_id == game_id)
        .map(|r| r.avg_latency_ms)
        .collect();

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
