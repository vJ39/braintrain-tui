//! パソコン通信(BBS)風に、テキストを1文字ずつ流れるように表示するタイプライター演出。
//!
//! 経過時間から「今何文字まで見せるか」を求める状態(Typewriter)と、装飾付きの
//! テキスト(Line/Span)を文字数で切り詰めるヘルパーを持つ。start_loop_seを呼んだ
//! Typewriterは、文字が流れている間タイプライターSEをループ再生し、流れ終わった
//! (tick/skip等でis_finishedになった)時点で止める。どの画面に使うか、いつリセットして
//! SEを鳴らし始めるかは呼び出し側(app/menu.rs・app/result.rs)が受け持つ。

use std::time::Duration;

use ratatui::text::{Line, Span};

use crate::audio;

/// 1文字あたりの表示間隔(標準)。約33msごとの画面更新で1フレームに1文字前後進む速さ
pub const CHAR_INTERVAL: Duration = Duration::from_millis(30);

/// タイプライター表示の進行状態。表示する全文字数と経過時間から、表示済み文字数を求める
#[derive(Debug, Clone)]
pub struct Typewriter {
    total_chars: usize,
    interval: Duration,
    /// 開始(リセット)からの経過時間
    elapsed: Duration,
    /// skip()等で全文字表示済みにされたか
    skipped: bool,
    /// タイプライターSEのループ再生を受け持っている(鳴らしている)か。start_loop_seでtrueにし、
    /// 流れ終わってSEを止めた時にfalseに戻す
    loop_se: bool,
}

impl Typewriter {
    /// 標準の表示間隔(CHAR_INTERVAL)で、経過時間0から始める
    pub fn new(total_chars: usize) -> Self {
        Self::with_interval(total_chars, CHAR_INTERVAL)
    }

    /// 表示間隔を指定して、経過時間0から始める
    pub fn with_interval(total_chars: usize, interval: Duration) -> Self {
        Self {
            total_chars,
            interval,
            elapsed: Duration::ZERO,
            skipped: false,
            loop_se: false,
        }
    }

    /// 最初から全文字表示済みの状態を作る(画面に入る前の初期値用)
    pub fn completed(interval: Duration) -> Self {
        let mut typewriter = Self::with_interval(0, interval);
        typewriter.skip();
        typewriter
    }

    /// 文字が流れている間、タイプライターSEのループ再生を始める(画面に入って流し始める時に呼ぶ)。
    /// 既に流れ終わっていれば鳴らさない
    pub fn start_loop_se(&mut self, kind: audio::TypewriterSeKind) {
        if self.is_finished() {
            audio::stop_typewriter_loop();
            self.loop_se = false;
        } else {
            audio::play_typewriter_loop(kind);
            self.loop_se = true;
        }
    }

    /// タイプライターSEを鳴らしているか(テストでの確認用)
    #[cfg(test)]
    pub fn plays_loop_se(&self) -> bool {
        self.loop_se
    }

    /// 流れ終わった時点で、鳴らしていたタイプライターSEを止める
    fn stop_loop_se_if_finished(&mut self) {
        if self.loop_se && self.is_finished() {
            audio::stop_typewriter_loop();
            self.loop_se = false;
        }
    }

    /// 経過時間を進める。最後の文字まで表示したらタイプライターSEを止める
    pub fn tick(&mut self, dt: Duration) {
        self.elapsed = self.elapsed.saturating_add(dt);
        self.stop_loop_se_if_finished();
    }

    /// 表示する全文字数(テストでの確認用)
    #[cfg(test)]
    pub fn total_chars(&self) -> usize {
        self.total_chars
    }

    /// 表示する全文字数を更新する(画面サイズが変わって表示内容の文字数が変わった時用)。
    /// 経過時間は保つ。既に全文字表示済みだった場合は、新しい文字数でも表示済みのままにする
    pub fn set_total_chars(&mut self, total_chars: usize) {
        if self.is_finished() {
            self.skipped = true;
        }
        self.total_chars = total_chars;
        // 文字数が減って表示済みの文字数に届いた場合も、流れ終わりとしてSEを止める
        self.stop_loop_se_if_finished();
    }

    /// 経過時間から求めた文字数(全文字数で頭打ちにしない)
    fn elapsed_chars(&self) -> usize {
        if self.interval.is_zero() {
            return usize::MAX;
        }
        let chars = self.elapsed.as_nanos() / self.interval.as_nanos();
        usize::try_from(chars).unwrap_or(usize::MAX)
    }

    /// 現時点で表示する文字数(0〜total_chars)
    pub fn visible_chars(&self) -> usize {
        if self.skipped {
            self.total_chars
        } else {
            self.elapsed_chars().min(self.total_chars)
        }
    }

    /// 全文字を表示し終えたか
    pub fn is_finished(&self) -> bool {
        self.skipped || self.elapsed_chars() >= self.total_chars
    }

    /// すぐに全文字表示済みにする(キー入力・クリックでのスキップ用)。タイプライターSEも止める
    pub fn skip(&mut self) {
        self.skipped = true;
        self.stop_loop_se_if_finished();
    }
}

/// 行の並びに含まれる文字数の合計(タイプライターで流す全文字数)
pub fn char_count(lines: &[Line]) -> usize {
    lines
        .iter()
        .flat_map(|line| &line.spans)
        .map(|span| span.content.chars().count())
        .sum()
}

/// 行の並びを、先頭から数えてmax_chars文字までに切り詰める。行数は保ち
/// (はみ出した行は空になる)、表示する部分の装飾(Span/Lineのスタイル・配置)は保つ
pub fn truncate_lines<'a>(lines: &[Line<'a>], max_chars: usize) -> Vec<Line<'a>> {
    cut_lines(lines, max_chars, false)
}

/// truncate_linesと同じく切り詰めるが、まだ表示しない部分を同じ表示幅の空白で埋める。
/// 中央寄せのテキストで、文字が増えるたびに行全体の位置がずれないようにするため
pub fn truncate_lines_keep_width<'a>(lines: &[Line<'a>], max_chars: usize) -> Vec<Line<'a>> {
    cut_lines(lines, max_chars, true)
}

/// 切り詰めの本体。keep_widthなら、表示しない部分を無装飾の空白で埋めて表示幅を保つ
fn cut_lines<'a>(lines: &[Line<'a>], max_chars: usize, keep_width: bool) -> Vec<Line<'a>> {
    let mut remaining = max_chars;
    lines
        .iter()
        .map(|line| {
            let mut spans = Vec::with_capacity(line.spans.len());
            let mut hidden_width = 0;
            for span in &line.spans {
                let len = span.content.chars().count();
                if remaining >= len {
                    spans.push(span.clone());
                    remaining -= len;
                } else if remaining > 0 {
                    // Spanの途中で切る。表示する側はそのSpanの装飾を引き継ぐ
                    let shown: String = span.content.chars().take(remaining).collect();
                    let hidden: String = span.content.chars().skip(remaining).collect();
                    hidden_width += Span::raw(hidden).width();
                    spans.push(Span::styled(shown, span.style));
                    remaining = 0;
                } else {
                    hidden_width += span.width();
                }
            }
            if keep_width && hidden_width > 0 {
                spans.push(Span::raw(" ".repeat(hidden_width)));
            }
            let mut cut = Line::from(spans).style(line.style);
            cut.alignment = line.alignment;
            cut
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio;
    use ratatui::layout::Alignment;
    use ratatui::style::{Color, Modifier, Style};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    // --- Typewriter ---

    #[test]
    fn nothing_is_visible_at_zero_elapsed() {
        let t = Typewriter::new(10);
        assert_eq!(t.visible_chars(), 0);
        assert!(!t.is_finished());
    }

    #[test]
    fn visible_chars_grow_with_elapsed_time() {
        let mut t = Typewriter::with_interval(10, ms(20));
        t.tick(ms(19));
        assert_eq!(t.visible_chars(), 0, "1文字ぶんの時間が経つまでは0文字");
        t.tick(ms(1));
        assert_eq!(t.visible_chars(), 1);
        t.tick(ms(60));
        assert_eq!(t.visible_chars(), 4);
        assert!(!t.is_finished());
    }

    #[test]
    fn finishes_when_all_chars_are_visible_and_does_not_exceed_total() {
        let mut t = Typewriter::with_interval(5, ms(20));
        t.tick(ms(80));
        assert!(!t.is_finished(), "4文字目まではまだ途中");
        t.tick(ms(20));
        assert_eq!(t.visible_chars(), 5);
        assert!(t.is_finished());
        t.tick(Duration::from_secs(60));
        assert_eq!(t.visible_chars(), 5, "全文字数を超えない");
    }

    #[test]
    fn new_uses_the_standard_interval() {
        let mut t = Typewriter::new(100);
        t.tick(CHAR_INTERVAL * 3);
        assert_eq!(t.visible_chars(), 3);
    }

    #[test]
    fn standard_interval_is_in_a_readable_range() {
        assert!(CHAR_INTERVAL >= ms(20) && CHAR_INTERVAL <= ms(40));
    }

    #[test]
    fn skip_makes_everything_visible_immediately() {
        let mut t = Typewriter::new(50);
        t.tick(CHAR_INTERVAL);
        t.skip();
        assert_eq!(t.visible_chars(), 50);
        assert!(t.is_finished());
    }

    #[test]
    fn empty_text_is_finished_from_the_start() {
        let t = Typewriter::new(0);
        assert!(t.is_finished());
        assert_eq!(t.visible_chars(), 0);
    }

    #[test]
    fn completed_is_finished_even_after_total_is_set() {
        let mut t = Typewriter::completed(CHAR_INTERVAL);
        assert!(t.is_finished());
        t.set_total_chars(30);
        assert!(t.is_finished());
        assert_eq!(t.visible_chars(), 30);
    }

    #[test]
    fn set_total_keeps_elapsed_while_typing() {
        let mut t = Typewriter::with_interval(10, ms(10));
        t.tick(ms(30));
        t.set_total_chars(20);
        assert_eq!(t.total_chars(), 20);
        assert_eq!(t.visible_chars(), 3);
        assert!(!t.is_finished());
    }

    #[test]
    fn set_total_keeps_a_naturally_finished_typewriter_finished() {
        let mut t = Typewriter::with_interval(3, ms(10));
        t.tick(ms(30));
        assert!(t.is_finished());
        t.set_total_chars(10);
        assert!(
            t.is_finished(),
            "表示し終えた後に文字数が増えても、途中から流し直さない"
        );
        assert_eq!(t.visible_chars(), 10);
    }

    #[test]
    fn zero_interval_shows_everything_at_once() {
        let t = Typewriter::with_interval(7, Duration::ZERO);
        assert_eq!(t.visible_chars(), 7);
        assert!(t.is_finished());
    }

    // --- タイプライターSE(文字が流れている間のループ再生) ---

    #[test]
    fn typewriter_without_start_loop_se_never_plays_the_loop() {
        let mut t = Typewriter::with_interval(3, ms(10));
        t.tick(ms(10));
        assert!(!t.plays_loop_se());
        assert!(!audio::is_typewriter_loop_playing());
        t.tick(ms(20));
        assert!(!audio::is_typewriter_loop_playing());
    }

    #[test]
    fn start_loop_se_plays_the_loop_while_typing() {
        let mut t = Typewriter::with_interval(3, ms(10));
        t.start_loop_se(audio::TypewriterSeKind::Menu);
        assert!(t.plays_loop_se());
        assert!(audio::is_typewriter_loop_playing(), "流れ始めたら鳴らす");
        t.tick(ms(20));
        assert!(
            audio::is_typewriter_loop_playing(),
            "流れている間は鳴り続ける"
        );
    }

    #[test]
    fn loop_se_stops_the_moment_typing_finishes() {
        let mut t = Typewriter::with_interval(3, ms(10));
        t.start_loop_se(audio::TypewriterSeKind::Menu);
        t.tick(ms(29));
        assert!(audio::is_typewriter_loop_playing(), "最後の1文字の手前");
        t.tick(ms(1));
        assert!(t.is_finished());
        assert!(!audio::is_typewriter_loop_playing(), "流れ終わったら止める");
        assert!(!t.plays_loop_se());
        t.tick(ms(100));
        assert!(
            !audio::is_typewriter_loop_playing(),
            "止めた後に鳴り直さない"
        );
    }

    #[test]
    fn skip_stops_the_loop_se() {
        let mut t = Typewriter::new(50);
        t.start_loop_se(audio::TypewriterSeKind::Menu);
        t.skip();
        assert!(!audio::is_typewriter_loop_playing());
        assert!(!t.plays_loop_se());
    }

    #[test]
    fn start_loop_se_on_already_finished_text_does_not_play() {
        for mut t in [Typewriter::new(0), Typewriter::completed(CHAR_INTERVAL)] {
            t.start_loop_se(audio::TypewriterSeKind::Menu);
            assert!(!audio::is_typewriter_loop_playing());
            assert!(!t.plays_loop_se());
        }
    }

    #[test]
    fn shrinking_total_to_finished_stops_the_loop_se() {
        // 画面サイズの変化で全文字数が減り、表示済み文字数に届いた時も止める
        let mut t = Typewriter::with_interval(10, ms(10));
        t.start_loop_se(audio::TypewriterSeKind::Menu);
        t.tick(ms(50));
        assert!(audio::is_typewriter_loop_playing());
        t.set_total_chars(5);
        assert!(t.is_finished());
        assert!(!audio::is_typewriter_loop_playing());
    }

    // --- 装飾付きテキストの切り詰め ---

    fn red() -> Style {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    }

    fn sample() -> Vec<Line<'static>> {
        vec![
            Line::from(vec![Span::styled("ab", red()), Span::raw("cd")]),
            Line::from(""),
            Line::from(Span::styled("正解です", Style::default().fg(Color::Green))),
        ]
    }

    fn text_of(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn char_count_sums_every_span_of_every_line() {
        assert_eq!(char_count(&sample()), 8, "ab+cd+(空行)+正解です");
    }

    #[test]
    fn truncate_cuts_to_the_given_number_of_chars_across_lines() {
        let lines = truncate_lines(&sample(), 6);
        assert_eq!(lines.len(), 3, "行数は保つ");
        assert_eq!(text_of(&lines[0]), "abcd");
        assert_eq!(text_of(&lines[1]), "");
        assert_eq!(text_of(&lines[2]), "正解");
        assert_eq!(char_count(&lines), 6);
    }

    #[test]
    fn truncate_in_the_middle_of_a_span_keeps_its_style() {
        let lines = truncate_lines(&sample(), 1);
        assert_eq!(text_of(&lines[0]), "a");
        assert_eq!(
            lines[0].spans[0].style,
            red(),
            "切った後も色・太字が保たれる"
        );
        assert_eq!(text_of(&lines[2]), "", "まだ届いていない行は空");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn truncate_keeps_line_style_and_alignment() {
        let lines = vec![Line::from("abc")
            .style(Style::default().fg(Color::Blue))
            .alignment(Alignment::Center)];
        let cut = truncate_lines(&lines, 2);
        assert_eq!(cut[0].style, Style::default().fg(Color::Blue));
        assert_eq!(cut[0].alignment, Some(Alignment::Center));
    }

    #[test]
    fn truncate_to_zero_and_to_more_than_total() {
        let none = truncate_lines(&sample(), 0);
        assert_eq!(char_count(&none), 0);
        assert_eq!(none.len(), 3);
        let all = truncate_lines(&sample(), 1000);
        assert_eq!(all, sample(), "全文字以上なら元のまま");
    }

    #[test]
    fn keep_width_pads_hidden_part_with_blanks_of_the_same_width() {
        let original = sample();
        let lines = truncate_lines_keep_width(&original, 5);
        for (cut, full) in lines.iter().zip(&original) {
            assert_eq!(cut.width(), full.width(), "表示幅が元の行と同じ");
        }
        assert_eq!(text_of(&lines[0]), "abcd");
        // 全角2文字ぶん(「解です」の3文字=6セル)は空白で埋まる
        assert_eq!(text_of(&lines[2]).trim_end(), "正");
        assert_eq!(lines[2].spans[0].style, Style::default().fg(Color::Green));
        let padding = lines[2].spans.last().unwrap();
        assert_eq!(padding.content.trim(), "", "埋めた部分は空白だけ");
        assert_eq!(
            padding.style,
            Style::default(),
            "埋めた空白には色を付けない"
        );
    }

    #[test]
    fn keep_width_with_everything_visible_is_unchanged() {
        assert_eq!(truncate_lines_keep_width(&sample(), 8), sample());
    }
}
