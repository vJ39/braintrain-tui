//! ゲーム開始前のカウントダウン演出(「3」→「2」→「1」→「GO!!」)。
//!
//! 状態管理(現在のフェーズ・経過時間)と描画だけを持つ。どのゲームを始めるかや、
//! 終わった後の画面切り替えは呼び出し側(app.rs)が受け持つ。

use std::time::Duration;

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::audio::SeKind;
use crate::game::theme;

/// 1フェーズの表示時間
pub const PHASE_DURATION: Duration = Duration::from_millis(600);

/// カウントダウンの各フェーズ(表示順)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Three,
    Two,
    One,
    Go,
}

/// 表示順に並べたフェーズ。CountdownStateは経過時間からこの添字を求める
const PHASES: [Phase; 4] = [Phase::Three, Phase::Two, Phase::One, Phase::Go];

impl Phase {
    /// 画面に出す文字列
    pub fn label(self) -> &'static str {
        match self {
            Phase::Three => "3",
            Phase::Two => "2",
            Phase::One => "1",
            Phase::Go => "GO!!",
        }
    }

    /// このフェーズに入った時に鳴らすSE
    pub fn se(self) -> SeKind {
        match self {
            Phase::Three | Phase::Two | Phase::One => SeKind::Transition,
            Phase::Go => SeKind::Confirm,
        }
    }
}

/// カウントダウンの進行状態。開始時は「3」で、PHASE_DURATIONごとに次へ進み、
/// 「GO!!」の表示時間が過ぎると終了する
pub struct CountdownState {
    /// 開始からの経過時間
    elapsed: Duration,
}

impl CountdownState {
    pub fn new() -> Self {
        Self {
            elapsed: Duration::ZERO,
        }
    }

    /// 経過時間から求めた現在フェーズの添字(PHASES.len()以上なら終了)
    fn phase_index(&self) -> usize {
        (self.elapsed.as_nanos() / PHASE_DURATION.as_nanos()) as usize
    }

    /// 現在表示中のフェーズ。終了後はNone
    pub fn phase(&self) -> Option<Phase> {
        PHASES.get(self.phase_index()).copied()
    }

    pub fn is_finished(&self) -> bool {
        self.phase_index() >= PHASES.len()
    }

    /// 経過時間を進める。新しいフェーズに入った場合はそのフェーズを返す(SEを鳴らす合図)。
    /// 1回のtickで複数フェーズを飛び越えた場合は最後に入ったフェーズだけを返す。
    /// 終了した時・フェーズが変わらない時はNone
    pub fn tick(&mut self, dt: Duration) -> Option<Phase> {
        if self.is_finished() {
            return None;
        }
        let before = self.phase_index();
        self.elapsed += dt;
        if self.phase_index() == before {
            None
        } else {
            self.phase()
        }
    }
}

/// 大きな文字の1ドットを横に何セルで描くか。端末のセルは縦長なので2セルで正方形に近づける
const DOT_WIDTH: u16 = 2;
/// グリフ1文字の縦・横のドット数
const GLYPH_ROWS: u16 = 5;
const GLYPH_COLS: u16 = 5;
/// 文字と文字の間のドット数
const GLYPH_GAP: u16 = 1;
/// 大きな文字の最大倍率
const MAX_SCALE: u16 = 3;

/// カウントダウンで使う文字だけの5x5ドットフォント('#'がドット)
fn glyph(c: char) -> Option<[&'static str; GLYPH_ROWS as usize]> {
    let rows = match c {
        '3' => ["#####", "    #", "#####", "    #", "#####"],
        '2' => ["#####", "    #", "#####", "#    ", "#####"],
        '1' => ["  #  ", " ##  ", "  #  ", "  #  ", "#####"],
        'G' => ["#####", "#    ", "# ###", "#   #", "#####"],
        'O' => ["#####", "#   #", "#   #", "#   #", "#####"],
        '!' => ["  #  ", "  #  ", "  #  ", "     ", "  #  "],
        _ => return None,
    };
    Some(rows)
}

/// labelを大きな文字の行の並びにする。scaleはドットの倍率(1ドット=縦scale行 x 横scale*DOT_WIDTHセル)。
/// フォントに無い文字を含む場合はNone
fn big_text_lines(label: &str, scale: u16) -> Option<Vec<String>> {
    let glyphs: Vec<_> = label.chars().map(glyph).collect::<Option<_>>()?;
    let cells = (DOT_WIDTH * scale) as usize;
    let dot_on = "█".repeat(cells);
    let dot_off = " ".repeat(cells);
    let gap = " ".repeat(cells * GLYPH_GAP as usize);
    let mut lines = Vec::new();
    for row in 0..GLYPH_ROWS as usize {
        let line = glyphs
            .iter()
            .map(|g| {
                g[row]
                    .chars()
                    .map(|dot| if dot == '#' { dot_on.as_str() } else { dot_off.as_str() })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(&gap);
        for _ in 0..scale {
            lines.push(line.clone());
        }
    }
    Some(lines)
}

/// labelを倍率scaleで描いた時の(幅, 高さ)のセル数
fn big_text_size(label: &str, scale: u16) -> (u16, u16) {
    let chars = label.chars().count() as u16;
    let dots = chars * GLYPH_COLS + chars.saturating_sub(1) * GLYPH_GAP;
    (dots * DOT_WIDTH * scale, GLYPH_ROWS * scale)
}

/// 全画面の中央に現在フェーズの文字を大きく描く。大きな文字が収まらない狭い画面では
/// 通常サイズの文字で描く。終了後は何も描かない
pub fn render(frame: &mut Frame, area: Rect, state: &CountdownState) {
    let Some(phase) = state.phase() else {
        return;
    };
    let label = phase.label();
    let color = match phase {
        Phase::Go => theme::HIGHLIGHT,
        _ => theme::ACCENT_STRONG,
    };
    let style = Style::default().fg(color).add_modifier(Modifier::BOLD);

    // 画面に収まる最大の倍率を選ぶ
    let scale = (1..=MAX_SCALE).rev().find(|&s| {
        let (width, height) = big_text_size(label, s);
        width <= area.width && height <= area.height
    });
    let lines: Vec<Line> = match scale.and_then(|s| big_text_lines(label, s)) {
        Some(big) => big
            .into_iter()
            .map(|l| Line::from(Span::styled(l, style)))
            .collect(),
        None => vec![Line::from(Span::styled(label, style))],
    };

    // 上下の余白を等しくして縦中央に置く(横はAlignment::Centerで中央寄せ)
    let height = (lines.len() as u16).min(area.height);
    let top = area.y + (area.height - height) / 2;
    let text_area = Rect::new(area.x, top, area.width, height);
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), text_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const ALL_PHASES: [Phase; 4] = [Phase::Three, Phase::Two, Phase::One, Phase::Go];

    #[test]
    fn phase_duration_is_600ms_and_total_is_2400ms() {
        assert_eq!(PHASE_DURATION, Duration::from_millis(600));
        assert_eq!(PHASE_DURATION * ALL_PHASES.len() as u32, Duration::from_millis(2400));
    }

    #[test]
    fn phase_labels_are_3_2_1_go() {
        let labels: Vec<&str> = ALL_PHASES.iter().map(|p| p.label()).collect();
        assert_eq!(labels, ["3", "2", "1", "GO!!"]);
    }

    #[test]
    fn countdown_numbers_play_transition_and_go_plays_confirm() {
        assert_eq!(Phase::Three.se(), SeKind::Transition);
        assert_eq!(Phase::Two.se(), SeKind::Transition);
        assert_eq!(Phase::One.se(), SeKind::Transition);
        assert_eq!(Phase::Go.se(), SeKind::Confirm);
    }

    #[test]
    fn new_state_starts_at_three_and_is_not_finished() {
        let state = CountdownState::new();
        assert_eq!(state.phase(), Some(Phase::Three));
        assert!(!state.is_finished());
    }

    #[test]
    fn tick_shorter_than_a_phase_keeps_the_phase() {
        let mut state = CountdownState::new();
        assert_eq!(state.tick(Duration::from_millis(599)), None);
        assert_eq!(state.phase(), Some(Phase::Three));
    }

    #[test]
    fn tick_advances_3_2_1_go_then_finishes() {
        let mut state = CountdownState::new();
        assert_eq!(state.tick(PHASE_DURATION), Some(Phase::Two));
        assert_eq!(state.phase(), Some(Phase::Two));
        assert_eq!(state.tick(PHASE_DURATION), Some(Phase::One));
        assert_eq!(state.phase(), Some(Phase::One));
        assert_eq!(state.tick(PHASE_DURATION), Some(Phase::Go));
        assert_eq!(state.phase(), Some(Phase::Go));
        assert!(!state.is_finished());
        // 終了時は新しいフェーズに入らないのでNone
        assert_eq!(state.tick(PHASE_DURATION), None);
        assert_eq!(state.phase(), None);
        assert!(state.is_finished());
    }

    #[test]
    fn small_ticks_accumulate_across_phase_boundary() {
        // 実際のループは約33msごとにtickする。細かいtickの合計で境界を越えること
        let mut state = CountdownState::new();
        let mut entered = Vec::new();
        let mut total = Duration::ZERO;
        while !state.is_finished() {
            let dt = Duration::from_millis(33);
            total += dt;
            if let Some(phase) = state.tick(dt) {
                entered.push(phase);
            }
            assert!(total <= Duration::from_millis(2400 + 33), "2.4秒前後で終わること");
        }
        assert_eq!(entered, [Phase::Two, Phase::One, Phase::Go]);
        assert!(total >= Duration::from_millis(2400), "2.4秒より前に終わらないこと");
    }

    #[test]
    fn large_tick_can_skip_phases_and_reports_the_latest() {
        let mut state = CountdownState::new();
        assert_eq!(state.tick(Duration::from_millis(1300)), Some(Phase::One));
        assert_eq!(state.phase(), Some(Phase::One));
        // 合計1.7秒ではOneの終わり(1.8秒)に達しないのでOneのまま
        assert_eq!(state.tick(Duration::from_millis(400)), None);
        assert_eq!(state.phase(), Some(Phase::One));
    }

    #[test]
    fn tick_after_finished_stays_finished() {
        let mut state = CountdownState::new();
        state.tick(Duration::from_secs(10));
        assert!(state.is_finished());
        assert_eq!(state.tick(PHASE_DURATION), None);
        assert!(state.is_finished());
    }

    #[test]
    fn big_text_has_glyph_for_every_label_char() {
        for phase in ALL_PHASES {
            assert!(
                big_text_lines(phase.label(), 1).is_some(),
                "{}のグリフがあること",
                phase.label()
            );
        }
    }

    #[test]
    fn big_text_of_one_at_scale_1_matches_glyph() {
        let lines = big_text_lines("1", 1).unwrap();
        // 端末のセルは縦長なので、横方向は2セルで1ドットにしている
        assert_eq!(
            lines,
            [
                "    ██    ",
                "  ████    ",
                "    ██    ",
                "    ██    ",
                "██████████",
            ]
        );
    }

    #[test]
    fn big_text_scale_multiplies_size() {
        let s1 = big_text_lines("GO!!", 1).unwrap();
        let s2 = big_text_lines("GO!!", 2).unwrap();
        assert_eq!(s2.len(), s1.len() * 2);
        let w1 = s1[0].chars().count();
        let w2 = s2[0].chars().count();
        assert_eq!(w2, w1 * 2);
        // どの行も同じ幅
        assert!(s2.iter().all(|l| l.chars().count() == w2));
    }

    #[test]
    fn big_text_unknown_char_is_none() {
        assert!(big_text_lines("X", 1).is_none());
    }

    fn rendered_rows(state: &CountdownState, width: u16, height: u16) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect())
            .collect()
    }

    #[test]
    fn render_draws_big_glyph_centered() {
        let state = CountdownState::new();
        let rows = rendered_rows(&state, 80, 30);
        let drawn: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.contains('█'))
            .map(|(i, _)| i)
            .collect();
        assert!(!drawn.is_empty(), "大きな文字が描かれること");
        // 上下の余白がほぼ同じ(中央寄せ)
        let top = drawn[0];
        let bottom = 30 - 1 - drawn[drawn.len() - 1];
        assert!(top.abs_diff(bottom) <= 1, "top={top} bottom={bottom}");
    }

    #[test]
    fn render_falls_back_to_plain_label_on_tiny_area() {
        let mut state = CountdownState::new();
        state.tick(PHASE_DURATION * 3); // GO!!
        let rows = rendered_rows(&state, 8, 3);
        assert!(rows.iter().any(|r| r.contains("GO!!")), "{rows:?}");
    }

    #[test]
    fn render_every_phase_and_finished_state_does_not_panic() {
        let mut state = CountdownState::new();
        for _ in 0..5 {
            rendered_rows(&state, 80, 24);
            rendered_rows(&state, 1, 1);
            state.tick(PHASE_DURATION);
        }
    }
}
