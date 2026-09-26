//! 履歴画面(Screen::History)の外枠(見出し・操作説明)。中身は
//! crate::stats::history_view::renderに委譲する

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::game::theme;

pub(super) fn render(frame: &mut Frame, area: Rect) {
    let block = theme::panel(" ◆ 履歴: ゲームごとの平均反応時間の推移 ◆ ").title_bottom(
        theme::hints_line(&[("Enter / Esc / クリック", "メニューに戻る")]).centered(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    crate::stats::history_view::render(frame, inner);
}
