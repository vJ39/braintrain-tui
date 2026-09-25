//! 画面の見た目(配色・パネル・HUD)の共通部品。
//!
//! 全画面で「シアン/ブルー基調」の配色をそろえるための定数と、よく使う部品(枠・HUD・操作説明)を置く。
//! ゲームの判定・採点には関与しない。

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::game::feedback::{AnswerFeedback, Flash, Verdict};
use crate::game::{Difficulty, QUESTIONS_PER_SESSION};

// ---------------------------------------------------------------------------
// 配色
// ---------------------------------------------------------------------------

/// 枠線など、画面の基調色
pub const ACCENT: Color = Color::Cyan;
/// タイトル・現在の問題など、いちばん目立たせたい情報
pub const ACCENT_STRONG: Color = Color::LightCyan;
/// フッター等の補助パネルの枠
pub const ACCENT_SUB: Color = Color::Blue;
/// 操作説明など、控えめに見せる文字
pub const MUTED: Color = Color::DarkGray;
/// 通常の本文
pub const TEXT: Color = Color::White;
/// 正解
pub const CORRECT: Color = Color::LightGreen;
/// 不正解
pub const INCORRECT: Color = Color::LightRed;
/// 空欄「?」など、問題の注目点
pub const HIGHLIGHT: Color = Color::Yellow;

/// HUD(画面上部の進捗・スコア表示)の高さ
pub const HUD_HEIGHT: u16 = 3;

pub fn title_style() -> Style {
    Style::default()
        .fg(ACCENT_STRONG)
        .add_modifier(Modifier::BOLD)
}

/// 選択中の項目(メニュー・曲選択など)の強調表示
pub fn selected_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

// ---------------------------------------------------------------------------
// パネル(枠)。どれもBorders::ALLの1セル枠なので、inner()の大きさは素のBlockと変わらない
// (クリック判定側が`Block::default().borders(Borders::ALL).inner(..)`で計算していても一致する)
// ---------------------------------------------------------------------------

/// 標準パネル: 角丸・シアン枠
pub fn panel<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(title.into().style(title_style()))
}

/// 補助パネル(フッター・選択肢ボタン等): 角丸・ブルー枠
pub fn sub_panel<'a>() -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT_SUB))
}

/// 問題を出すメインパネル: 太枠。正誤表示中は枠を正誤の色に光らせる
pub fn focus_panel<'a>(title: impl Into<Line<'a>>, flash: Option<&Flash>) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(flash_border_color(flash)))
        .title(title.into().style(title_style()))
}

/// 正誤表示中はその色、表示していなければ基調色
pub fn flash_border_color(flash: Option<&Flash>) -> Color {
    flash.map_or(ACCENT, |f| verdict_color(f.verdict))
}

pub fn verdict_color(verdict: Verdict) -> Color {
    match verdict {
        Verdict::Correct => CORRECT,
        Verdict::Incorrect => INCORRECT,
    }
}

pub fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Correct => "○ せいかい！",
        Verdict::Incorrect => "× ざんねん…",
    }
}

/// 正誤表示1行分(「○ せいかい！  こたえ: 12」)
pub fn flash_line(flash: &Flash) -> Line<'static> {
    let color = verdict_color(flash.verdict);
    let mut spans = vec![Span::styled(
        verdict_label(flash.verdict),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )];
    if !flash.detail.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(flash.detail.clone(), Style::default().fg(color)));
    }
    Line::from(spans)
}

/// 難易度の表示名と色
pub fn difficulty_label(difficulty: Difficulty) -> (&'static str, Color) {
    match difficulty {
        Difficulty::Beginner => ("初級 ★☆☆", Color::LightGreen),
        Difficulty::Intermediate => ("中級 ★★☆", Color::Yellow),
        Difficulty::Advanced => ("上級 ★★★", Color::LightRed),
    }
}

/// 進捗バー(「▰▰▰▱▱▱」)。doneがtotalを超えても満タンで止める。totalが0なら空のバー
pub fn progress_bar(done: u32, total: u32, width: usize) -> String {
    let filled = if total == 0 {
        0
    } else {
        ((done.min(total) as usize) * width) / total as usize
    };
    format!("{}{}", "▰".repeat(filled), "▱".repeat(width - filled))
}

/// 正答率からランク(S/A/B/C)と色を決める。1問も記録が無ければ「-」
pub fn rank_for(correct: u32, total: u32) -> (&'static str, Color) {
    if total == 0 {
        return ("-", MUTED);
    }
    let rate = correct as f64 / total as f64;
    if rate >= 0.9 {
        ("S", ACCENT_STRONG)
    } else if rate >= 0.7 {
        ("A", CORRECT)
    } else if rate >= 0.5 {
        ("B", HIGHLIGHT)
    } else {
        ("C", INCORRECT)
    }
}

/// 操作説明1つ分(「 ← 」をキー風に強調 + 説明文)
pub fn key_hint(key: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {label}"), Style::default().fg(TEXT)),
    ]
}

/// 番号付き選択肢1行分(「 1  42」)。番号はキー風、内容は太字
pub fn choice_line(number: usize, text: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {number} "),
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            text,
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
    ])
}

/// 縦に並んだ番号付き選択肢を、row_bands(=クリック判定のrow_index)の帯ごとに縦中央で描く。
/// 文字列の長さが違っても番号の位置がそろうよう、最長の行幅の列を横中央に置いて左寄せで描く
pub fn render_choice_rows(frame: &mut Frame, area: Rect, texts: &[String]) {
    let lines: Vec<Line> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| choice_line(i + 1, text.clone()))
        .collect();
    let column_width = lines
        .iter()
        .map(|l| l.width() as u16)
        .max()
        .unwrap_or(0)
        .min(area.width);
    let column_x = area.x + (area.width - column_width) / 2;
    for (band, line) in row_bands(area, lines.len() as u16).into_iter().zip(lines) {
        let row = vertical_center(band, 1);
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(column_x, row.y, column_width, row.height),
        );
    }
}

/// 操作説明を並べた1行。枠の辺にタイトルとして置いても罫線と接しないよう両端に空白を入れる
pub fn hints_line(hints: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("    "));
        }
        spans.extend(key_hint(key, label));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// 操作説明だけを表示するフッター
pub fn render_hint_footer(frame: &mut Frame, area: Rect, hints: &[(&str, &str)]) {
    let paragraph = Paragraph::new(hints_line(hints))
        .alignment(Alignment::Center)
        .block(sub_panel());
    frame.render_widget(paragraph, area);
}

/// エリアを横にN等分した各選択肢をボタン風に描く。分割はcolumn_bandsで、
/// クリック判定のcolumn_indexと同じ境界になる
pub fn render_choice_buttons(frame: &mut Frame, area: Rect, hints: &[(&str, &str)]) {
    for (band, (key, label)) in column_bands(area, hints.len() as u16)
        .into_iter()
        .zip(hints)
    {
        let paragraph = Paragraph::new(Line::from(key_hint(key, label)))
            .alignment(Alignment::Center)
            .block(sub_panel());
        frame.render_widget(paragraph, band);
    }
}

// ---------------------------------------------------------------------------
// HUD
// ---------------------------------------------------------------------------

/// エリアを「HUD」と「残り」に分ける
pub fn split_hud(area: Rect) -> (Rect, Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(HUD_HEIGHT), Constraint::Min(0)])
        .split(area);
    (rows[0], rows[1])
}

/// 10問セッション型ゲームのHUD。左=問題番号と進捗バー、中央=直近の正誤、右=正解数・連続正解
pub fn render_hud(
    frame: &mut Frame,
    area: Rect,
    game_name: &str,
    difficulty: Difficulty,
    answered: u32,
    feedback: &AnswerFeedback,
) {
    let (difficulty_text, difficulty_color) = difficulty_label(difficulty);
    let block = panel(format!(" ◆ {game_name} "))
        .border_style(Style::default().fg(flash_border_color(feedback.current())))
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
            Constraint::Length(22),
            Constraint::Fill(1),
            Constraint::Length(20),
        ])
        .split(inner);

    // いま解いている問題の番号(全問解き終えたら最終問のまま)
    let current_question = (answered + 1).min(QUESTIONS_PER_SESSION);
    let progress = Line::from(vec![
        Span::styled(
            format!(" Q{current_question:>2}/{QUESTIONS_PER_SESSION} "),
            title_style(),
        ),
        Span::styled(
            progress_bar(answered, QUESTIONS_PER_SESSION, QUESTIONS_PER_SESSION as usize),
            Style::default().fg(ACCENT),
        ),
    ]);
    frame.render_widget(Paragraph::new(progress), cols[0]);

    if let Some(flash) = feedback.current() {
        frame.render_widget(
            Paragraph::new(flash_line(flash)).alignment(Alignment::Center),
            cols[1],
        );
    }

    let score = Line::from(vec![
        Span::styled("正解 ", Style::default().fg(MUTED)),
        Span::styled(
            feedback.correct().to_string(),
            Style::default().fg(CORRECT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  連続 ", Style::default().fg(MUTED)),
        Span::styled(
            feedback.streak().to_string(),
            Style::default()
                .fg(ACCENT_STRONG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]);
    frame.render_widget(Paragraph::new(score).alignment(Alignment::Right), cols[2]);
}

// ---------------------------------------------------------------------------
// 選択肢の表示領域。クリック判定(row_index/column_index)と同じ分割にして、
// 見た目の位置とクリックして反応する位置を一致させる
// ---------------------------------------------------------------------------

/// エリアを縦にcount個の帯へ分ける。row_indexと同じく各帯はheight/count行で、
/// 割り切れない余りは最後の帯に含める。高さが足りない場合は1行ずつ(はみ出す分は高さ0)
pub fn row_bands(area: Rect, count: u16) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let band_height = area.height / count;
    (0..count)
        .map(|i| {
            if band_height == 0 {
                let height = u16::from(i < area.height);
                return Rect::new(area.x, area.y + i.min(area.height), area.width, height);
            }
            let y = area.y + i * band_height;
            let height = if i == count - 1 {
                area.height - i * band_height
            } else {
                band_height
            };
            Rect::new(area.x, y, area.width, height)
        })
        .collect()
}

/// エリアを横にcount個の帯へ分ける(column_indexと同じ分割。余りは最後の帯)
pub fn column_bands(area: Rect, count: u16) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let band_width = area.width / count;
    (0..count)
        .map(|i| {
            if band_width == 0 {
                let width = u16::from(i < area.width);
                return Rect::new(area.x + i.min(area.width), area.y, width, area.height);
            }
            let x = area.x + i * band_width;
            let width = if i == count - 1 {
                area.width - i * band_width
            } else {
                band_width
            };
            Rect::new(x, area.y, width, area.height)
        })
        .collect()
}

/// area内でcontent_height行の内容を縦中央に置くための領域
pub fn vertical_center(area: Rect, content_height: u16) -> Rect {
    let height = content_height.min(area.height);
    let top = (area.height - height) / 2;
    Rect::new(area.x, area.y + top, area.width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{column_index, row_index};

    #[test]
    fn progress_bar_fills_proportionally() {
        assert_eq!(progress_bar(0, 10, 10), "▱▱▱▱▱▱▱▱▱▱");
        assert_eq!(progress_bar(3, 10, 10), "▰▰▰▱▱▱▱▱▱▱");
        assert_eq!(progress_bar(10, 10, 10), "▰▰▰▰▰▰▰▰▰▰");
        // 幅と総数が違っても比率で埋まる
        assert_eq!(progress_bar(1, 2, 4), "▰▰▱▱");
    }

    #[test]
    fn progress_bar_clamps_overflow_and_handles_zero_total() {
        assert_eq!(progress_bar(15, 10, 5), "▰▰▰▰▰");
        assert_eq!(progress_bar(3, 0, 5), "▱▱▱▱▱");
        assert_eq!(progress_bar(0, 10, 0), "");
    }

    #[test]
    fn rank_for_thresholds() {
        assert_eq!(rank_for(10, 10).0, "S");
        assert_eq!(rank_for(9, 10).0, "S");
        assert_eq!(rank_for(8, 10).0, "A");
        assert_eq!(rank_for(7, 10).0, "A");
        assert_eq!(rank_for(6, 10).0, "B");
        assert_eq!(rank_for(5, 10).0, "B");
        assert_eq!(rank_for(4, 10).0, "C");
        assert_eq!(rank_for(0, 10).0, "C");
        assert_eq!(rank_for(0, 0).0, "-");
    }

    #[test]
    fn verdict_colors_differ_for_correct_and_incorrect() {
        assert_ne!(
            verdict_color(Verdict::Correct),
            verdict_color(Verdict::Incorrect)
        );
    }

    #[test]
    fn flash_border_color_is_accent_without_flash() {
        assert_eq!(flash_border_color(None), ACCENT);
        let flash = Flash {
            verdict: Verdict::Incorrect,
            detail: String::new(),
        };
        assert_eq!(flash_border_color(Some(&flash)), INCORRECT);
    }

    #[test]
    fn row_bands_match_row_index_for_every_row() {
        // 見た目の帯とクリック判定(row_index)が全行で一致すること
        for height in 4..=23 {
            let area = Rect::new(3, 7, 20, height);
            let bands = row_bands(area, 4);
            assert_eq!(bands.len(), 4);
            for (i, band) in bands.iter().enumerate() {
                for row in band.y..band.y + band.height {
                    assert_eq!(
                        row_index(area, row, 4),
                        Some(i),
                        "height={height} row={row}"
                    );
                }
            }
            // 帯を合わせるとエリアをちょうど覆う
            let total: u16 = bands.iter().map(|b| b.height).sum();
            assert_eq!(total, height);
            assert_eq!(bands[0].y, area.y);
        }
    }

    #[test]
    fn row_bands_when_area_is_too_short_uses_one_row_each() {
        let area = Rect::new(0, 10, 20, 2);
        let bands = row_bands(area, 4);
        assert_eq!(bands.len(), 4);
        assert_eq!((bands[0].y, bands[0].height), (10, 1));
        assert_eq!((bands[1].y, bands[1].height), (11, 1));
        // エリアからはみ出す帯は高さ0で、エリア外には描かない
        assert_eq!(bands[2].height, 0);
        assert_eq!(bands[3].height, 0);
        assert!(row_bands(area, 0).is_empty());
    }

    #[test]
    fn column_bands_match_column_index_for_every_column() {
        for width in 2..=41 {
            for count in [2u16, 4] {
                if width < count {
                    continue;
                }
                let area = Rect::new(5, 0, width, 3);
                let bands = column_bands(area, count);
                assert_eq!(bands.len(), count as usize);
                for (i, band) in bands.iter().enumerate() {
                    for col in band.x..band.x + band.width {
                        assert_eq!(
                            column_index(area, col, count),
                            Some(i),
                            "width={width} count={count} col={col}"
                        );
                    }
                }
                let total: u16 = bands.iter().map(|b| b.width).sum();
                assert_eq!(total, width);
            }
        }
    }

    #[test]
    fn vertical_center_places_content_in_middle() {
        let area = Rect::new(0, 10, 20, 9);
        assert_eq!(vertical_center(area, 3), Rect::new(0, 13, 20, 3));
        // 内容がエリアより高い場合はエリア全体
        assert_eq!(vertical_center(area, 20), area);
    }
}
