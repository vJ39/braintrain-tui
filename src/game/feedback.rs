//! 回答直後の正誤フィードバック(画面表示専用の状態)。
//!
//! スコアの記録はScoreTrackerが持ち、ここで数える正解数・連続正解数はHUD表示のためだけに使う。
//! 採点・判定ロジックには一切関与しない。

use std::time::Duration;

/// 直近の正誤表示を画面に出し続ける時間
pub const FEEDBACK_HOLD: Duration = Duration::from_millis(900);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Correct,
    Incorrect,
}

/// 画面に一時表示する正誤結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flash {
    pub verdict: Verdict,
    /// 補足(直前の問題の正解など)。空文字なら表示しない
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct AnswerFeedback {
    /// 表示中の結果と、表示し始めてからの経過時間
    last: Option<(Flash, Duration)>,
    correct: u32,
    streak: u32,
}

impl AnswerFeedback {
    pub fn new() -> Self {
        Self::default()
    }

    /// 1問の正誤を反映し、表示タイマーを最初からやり直す
    pub fn record(&mut self, is_correct: bool, detail: impl Into<String>) {
        let verdict = if is_correct {
            self.correct += 1;
            self.streak += 1;
            Verdict::Correct
        } else {
            self.streak = 0;
            Verdict::Incorrect
        };
        self.last = Some((
            Flash {
                verdict,
                detail: detail.into(),
            },
            Duration::ZERO,
        ));
    }

    /// 経過時間を進め、表示時間を過ぎた結果を消す
    pub fn tick(&mut self, dt: Duration) {
        if let Some((_, elapsed)) = &mut self.last {
            *elapsed += dt;
            if *elapsed >= FEEDBACK_HOLD {
                self.last = None;
            }
        }
    }

    /// いま表示すべき結果(表示時間内のもののみ)
    pub fn current(&self) -> Option<&Flash> {
        self.last.as_ref().map(|(flash, _)| flash)
    }

    pub fn correct(&self) -> u32 {
        self.correct
    }

    pub fn streak(&self) -> u32 {
        self.streak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_feedback_shows_nothing_and_counts_are_zero() {
        let feedback = AnswerFeedback::new();
        assert_eq!(feedback.current(), None);
        assert_eq!(feedback.correct(), 0);
        assert_eq!(feedback.streak(), 0);
    }

    #[test]
    fn record_correct_shows_correct_flash_and_counts_up() {
        let mut feedback = AnswerFeedback::new();
        feedback.record(true, "こたえ: 12");
        let flash = feedback.current().expect("直後は表示中のはず");
        assert_eq!(flash.verdict, Verdict::Correct);
        assert_eq!(flash.detail, "こたえ: 12");
        assert_eq!(feedback.correct(), 1);
        assert_eq!(feedback.streak(), 1);
    }

    #[test]
    fn record_incorrect_resets_streak_but_keeps_correct_count() {
        let mut feedback = AnswerFeedback::new();
        feedback.record(true, "");
        feedback.record(true, "");
        feedback.record(false, "");
        assert_eq!(feedback.current().unwrap().verdict, Verdict::Incorrect);
        assert_eq!(feedback.correct(), 2);
        assert_eq!(feedback.streak(), 0);
    }

    #[test]
    fn flash_stays_until_hold_time_then_disappears() {
        let mut feedback = AnswerFeedback::new();
        feedback.record(true, "");
        feedback.tick(FEEDBACK_HOLD - Duration::from_millis(1));
        assert!(feedback.current().is_some(), "表示時間内は消えない");
        feedback.tick(Duration::from_millis(1));
        assert!(feedback.current().is_none(), "表示時間を過ぎたら消える");
    }

    #[test]
    fn new_record_restarts_display_timer() {
        let mut feedback = AnswerFeedback::new();
        feedback.record(true, "");
        feedback.tick(FEEDBACK_HOLD - Duration::from_millis(10));
        feedback.record(false, "");
        // 直前の表示時間の残りではなく、新しい結果の表示時間が最初から数えられる
        feedback.tick(Duration::from_millis(100));
        assert_eq!(feedback.current().unwrap().verdict, Verdict::Incorrect);
    }

    #[test]
    fn tick_without_flash_is_noop() {
        let mut feedback = AnswerFeedback::new();
        feedback.tick(Duration::from_secs(10));
        assert!(feedback.current().is_none());
        assert_eq!(feedback.correct(), 0);
    }

    #[test]
    fn counts_survive_after_flash_disappears() {
        let mut feedback = AnswerFeedback::new();
        feedback.record(true, "");
        feedback.tick(FEEDBACK_HOLD);
        assert!(feedback.current().is_none());
        assert_eq!(feedback.correct(), 1);
        assert_eq!(feedback.streak(), 1);
    }
}
