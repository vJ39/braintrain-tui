//! リザルト画面(Screen::Result)。本文のタイプライター表示・キャラクターのアニメーション・
//! 履歴の保存エラー表示と、ゲーム終了後にリザルトへ入る遷移

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::audio::{self, BgmCategory};
use crate::game::{theme, GameResult};
use crate::stats::store;
use crate::ui::result_sprite::ResultSprite;
use crate::ui::typewriter::{self, Typewriter};

use super::menu_items::{
    BEIGOMA_ITEM_INDEX, COLOR_STACK_ITEM_INDEX, COUNT_MANIA_ITEM_INDEX, MENU_ITEMS,
    QUICK_DRAW_ITEM_INDEX, RHYTHM_ITEM_INDEX,
};
use super::screen_layout::centered_rect;
use super::{App, Screen};

impl App {
    /// ゲーム終了後、リザルト画面へ進む(キー/クリック共通)。リザルト用BGMに切り替える。
    /// 履歴への保存はここで1回だけ行う(描画のたびに保存し直さない)
    pub(super) fn enter_result(&mut self, result: GameResult) {
        let category = if result.is_game_over() {
            BgmCategory::ResultFailure
        } else {
            BgmCategory::Result
        };
        if let Some(name) = audio::random_bgm_track(category) {
            audio::play_bgm_track(&name);
            self.current_bgm = Some(name);
        }
        let save_error = store::append_result(&result).err().map(|e| e.to_string());
        self.show_result(result, save_error);
    }

    /// リザルト画面を表示し、結果の本文のタイプライターとキャラクターのアニメーションを
    /// 最初から始める(履歴への保存はしない)
    pub(super) fn show_result(&mut self, result: GameResult, save_error: Option<String>) {
        self.result_typewriter = Typewriter::new(typewriter::char_count(&result_lines(&result)));
        self.result_sprite.reset();
        self.screen = Screen::Result(result, save_error);
    }
}

/// リザルト画面の本文(カード内のテキスト)。タイプライター表示はこの並びの先頭から1文字ずつ流す
fn result_lines(result: &GameResult) -> Vec<Line<'static>> {
    // game_idからメニュー上の表示名を引く(見つからなければgame_idをそのまま出す)
    let game_names = [
        (crate::game::shape_rotate::GAME_ID, 0),
        (crate::game::mirror_match::GAME_ID, 1),
        (crate::game::reaction::GAME_ID, 2),
        (crate::game::mental_calc::GAME_ID, 3),
        (crate::game::pattern_fill::GAME_ID, 4),
        (crate::game::memory::GAME_ID, 5),
        (crate::game::sequence::GAME_ID, 6),
        (crate::game::puzzle_connect::GAME_ID, 7),
        (crate::game::count_mania::GAME_ID, COUNT_MANIA_ITEM_INDEX),
        (crate::game::color_stack::GAME_ID, COLOR_STACK_ITEM_INDEX),
        (crate::game::rhythm::GAME_ID, RHYTHM_ITEM_INDEX),
        (crate::game::quick_draw::GAME_ID, QUICK_DRAW_ITEM_INDEX),
        (crate::game::beigoma::GAME_ID, BEIGOMA_ITEM_INDEX),
    ];
    let game_name = game_names
        .iter()
        .find(|(id, _)| *id == result.game_id)
        .map_or(result.game_id.as_str(), |(_, index)| MENU_ITEMS[*index]);
    let (difficulty_text, difficulty_color) = theme::difficulty_label(result.difficulty);
    let (rank, rank_color) = theme::rank_for(result.correct, result.total);
    let percent = if result.total == 0 {
        0
    } else {
        result.correct * 100 / result.total
    };

    let label_style = Style::default().fg(theme::MUTED);
    let value_style = Style::default()
        .fg(theme::ACCENT_STRONG)
        .add_modifier(Modifier::BOLD);
    vec![
        Line::from(vec![
            Span::styled(game_name.to_string(), theme::title_style()),
            Span::styled("   ", label_style),
            Span::styled(
                difficulty_text,
                Style::default()
                    .fg(difficulty_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("RANK  ", label_style),
            Span::styled(
                format!("  {rank}  "),
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(rank_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("正解  ", label_style),
            Span::styled(
                format!("{} / {}", result.correct, result.total),
                value_style,
            ),
            Span::styled(format!("  ({percent}%)"), Style::default().fg(theme::TEXT)),
        ]),
        Line::from(Span::styled(
            theme::progress_bar(result.correct, result.total, 20),
            Style::default().fg(rank_color),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("平均反応時間  ", label_style),
            Span::styled(format!("{:.0}", result.avg_latency_ms), value_style),
            Span::styled(" ms", Style::default().fg(theme::TEXT)),
        ]),
    ]
}

/// リザルト画面のテキストカードの範囲。外枠の内側(inner)の中央に、幅48以内・本文の行数+枠の高さで置く
fn result_card_rect(inner: Rect, content_height: u16) -> Rect {
    centered_rect(
        inner,
        inner.width.min(48),
        (content_height + 2).min(inner.height),
    )
}

/// リザルト画面(本文のカード・キャラクター・履歴の保存エラー)
pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    result: &GameResult,
    save_error: Option<&str>,
    typing: &mut Typewriter,
    sprite: &mut ResultSprite,
) {
    // 保存(呼び出し側のenter_resultで1回だけ実施済み)に失敗していれば、
    // その内容を結果の描画後に下端へ重ねて表示する

    // 外枠(見出し・操作説明)はタイプライター表示の対象外で、最初から出す
    let outer = theme::panel(Line::from(" ◆ RESULT ◆ ").centered()).title_bottom(
        theme::hints_line(&[("Enter / Esc / クリック", "メニューに戻る")]).centered(),
    );
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let lines = result_lines(result);
    typing.set_total_chars(typewriter::char_count(&lines));
    // 中央寄せなので、まだ出していない部分を空白で埋めて行の位置がずれないようにする
    let text = typewriter::truncate_lines_keep_width(&lines, typing.visible_chars());
    let content_height = text.len() as u16;
    let card = result_card_rect(inner, content_height);
    // キャラクターはカードの左側の余白に描く(余白が狭ければsprite側で省略する)。
    // GAME OVERの時は踊っているように見えてしまうため描かない
    if !result.is_game_over() {
        sprite.render(frame, inner, card);
    }
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(theme::sub_panel());
    frame.render_widget(paragraph, card);

    if let Some(e) = save_error {
        let error_area = Rect::new(
            inner.x,
            inner.y + inner.height.saturating_sub(1),
            inner.width,
            inner.height.min(1),
        );
        let paragraph = Paragraph::new(format!("履歴の保存に失敗: {e}"))
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme::INCORRECT));
        frame.render_widget(paragraph, error_area);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyEvent};
    use ratatui_image::picker::ProtocolType;

    use crate::game::Difficulty;
    use crate::ui::result_sprite;
    use crate::ui::typewriter::CHAR_INTERVAL;

    use super::super::menu_items::new_game;
    use super::super::screen_layout::screen_rect;
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn result_screen_shows_quick_draw_menu_name() {
        let mut app = App::new();
        app.screen = Screen::Result(
            new_game(QUICK_DRAW_ITEM_INDEX, Difficulty::Beginner).result(),
            None,
        );
        let text = rendered_text(&mut app).replace(' ', "");
        assert!(text.contains("ハヤウチ"));
        assert!(!text.contains("反射神経"));
    }

    #[test]
    fn result_screen_shows_beigoma_menu_name() {
        let mut app = App::new();
        app.screen = Screen::Result(
            new_game(BEIGOMA_ITEM_INDEX, Difficulty::Beginner).result(),
            None,
        );
        let text = rendered_text(&mut app).replace(' ', "");
        assert!(text.contains("べー"), "{text}");
    }

    #[test]
    fn game_over_switches_bgm_to_result_category() {
        let mut app = App::new();
        app.select_menu_item(0); // shape_rotate
        app.handle_key(KeyEvent::from(KeyCode::Char('1'))); // Beginnerでプレイ開始
        finish_countdown(&mut app);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            app.handle_key(KeyEvent::from(KeyCode::Left));
        }
        assert!(matches!(app.screen, Screen::Result(..)));
        assert_eq!(app.current_bgm.as_deref(), Some("New_Personal_Best"));
    }

    #[test]
    fn rendering_the_result_screen_repeatedly_does_not_re_save_history() {
        // 履歴への保存はenter_resultで1回だけ行い、render()を繰り返しても再実行しない
        // (以前はrender_resultがappend_resultを呼んでいて、描画のたびに重複保存されていた)
        let mut app = App::new();
        app.select_menu_item(0); // shape_rotate
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
        finish_countdown(&mut app);
        for _ in 0..crate::game::QUESTIONS_PER_SESSION {
            app.handle_key(KeyEvent::from(KeyCode::Left));
        }
        let Screen::Result(_, save_error_after_enter) = &app.screen else {
            panic!("リザルト画面のはず");
        };
        let save_error_after_enter = save_error_after_enter.clone();
        for _ in 0..5 {
            rendered_text(&mut app);
        }
        let Screen::Result(_, save_error_after_render) = &app.screen else {
            panic!("リザルト画面のはず");
        };
        assert_eq!(
            &save_error_after_enter, save_error_after_render,
            "描画を繰り返しても保存処理は再実行されない"
        );
    }

    // --- タイプライター表示 ---

    #[test]
    fn result_screen_starts_empty_and_types_in_over_time() {
        let mut app = app_showing_result();
        assert_eq!(app.result_typewriter.visible_chars(), 0);
        assert!(!app.result_typewriter.is_finished());
        let before = rendered_compact(&mut app);
        assert!(before.contains("RESULT"), "枠と見出しは最初から出る");
        assert!(!before.contains("図形回"), "本文はまだ出ない");

        app.update(CHAR_INTERVAL * 3);
        assert_eq!(app.result_typewriter.visible_chars(), 3);
        let mid = rendered_compact(&mut app);
        assert!(mid.contains("図形回"), "先頭から3文字だけ出る");
        assert!(!mid.contains("図形回転"));
        assert!(!mid.contains("平均反応時間"), "後ろの行はまだ出ない");

        app.update(LONG_ENOUGH);
        assert!(app.result_typewriter.is_finished());
        let after = rendered_compact(&mut app);
        assert!(after.contains(MENU_ITEMS[0]));
        assert!(after.contains("RANK"));
        assert!(after.contains("平均反応時間"));
    }

    #[test]
    fn result_typewriter_total_matches_the_result_text() {
        let app = app_showing_result();
        let Screen::Result(result, _) = &app.screen else {
            unreachable!();
        };
        assert_eq!(
            app.result_typewriter.total_chars(),
            typewriter::char_count(&result_lines(result))
        );
    }

    #[test]
    fn showing_a_result_again_restarts_typing() {
        let mut app = app_showing_result();
        app.update(LONG_ENOUGH);
        assert!(app.result_typewriter.is_finished());
        app.show_result(sample_result(), None);
        assert_eq!(app.result_typewriter.visible_chars(), 0);
    }

    #[test]
    fn centered_result_lines_do_not_shift_while_typing() {
        // 中央寄せの行が、文字が増えるたびに左右へずれないこと(先頭の文字の位置が変わらない)
        let first_col = |app: &mut App| {
            let rows = rendered_cells(app, 80, 30);
            rows.iter()
                .find_map(|r| r.iter().position(|c| c == "図"))
                .expect("ゲーム名の先頭文字が描かれていること")
        };
        let mut app = app_showing_result();
        app.update(CHAR_INTERVAL);
        let early = first_col(&mut app);
        app.update(LONG_ENOUGH);
        assert_eq!(first_col(&mut app), early);
    }

    #[test]
    fn other_key_while_result_is_typing_finishes_it_without_leaving() {
        let mut app = app_showing_result();
        press(&mut app, KeyCode::Char(' '));
        assert!(
            matches!(app.screen, Screen::Result(..)),
            "Enter/Esc以外では戻らない"
        );
        assert!(app.result_typewriter.is_finished());
        assert!(rendered_compact(&mut app).contains("平均反応時間"));
    }

    #[test]
    fn enter_or_esc_while_result_is_typing_still_returns_to_menu() {
        for code in [KeyCode::Enter, KeyCode::Esc] {
            let mut app = app_showing_result();
            press(&mut app, code);
            assert!(
                matches!(app.screen, Screen::Menu),
                "{code:?}でメニューに戻る"
            );
            assert!(
                !app.menu_typewriter.is_finished(),
                "{code:?}: 戻ったメニューは頭からタイプライターで流れる"
            );
        }
    }

    #[test]
    fn click_while_result_is_typing_still_returns_to_menu() {
        let mut app = app_showing_result();
        app.last_area = rect(0, 0, 80, 30);
        app.handle_mouse(left_click(5));
        assert!(matches!(app.screen, Screen::Menu));
    }

    // --- キャラクターのコマ送りアニメーション ---

    /// キャラクターを描けるResultSprite(ハーフブロック描画)
    fn halfblocks_sprite() -> ResultSprite {
        ResultSprite::with_picker(Some(test_picker(ProtocolType::Halfblocks)))
    }

    /// appをwidth x heightで描いたバッファ
    fn rendered_buffer(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| app.render(frame))
            .unwrap()
            .buffer
            .clone()
    }

    /// width x heightの画面に描いたリザルト画面のテキストカードの範囲(renderと同じ計算)
    fn result_card_for(app: &App, width: u16, height: u16) -> Rect {
        let Screen::Result(result, _) = &app.screen else {
            unreachable!();
        };
        let inner = theme::panel("").inner(screen_rect(rect(0, 0, width, height)));
        result_card_rect(inner, result_lines(result).len() as u16)
    }

    #[test]
    fn result_sprite_advances_only_on_the_result_screen() {
        let mut app = app_showing_result();
        assert_eq!(app.result_sprite.elapsed(), Duration::ZERO);
        app.update(result_sprite::FRAME_DURATION);
        assert_eq!(app.result_sprite.elapsed(), result_sprite::FRAME_DURATION);
        assert_eq!(
            app.result_sprite.current_frame(),
            1,
            "リザルト画面ではコマが進む"
        );

        for screen in [
            Screen::History,
            Screen::Menu,
            Screen::ConfirmQuit,
            Screen::Splash,
        ] {
            let mut app = app_showing_result();
            app.screen = screen;
            app.update(result_sprite::FRAME_DURATION * 3);
            assert_eq!(
                app.result_sprite.elapsed(),
                Duration::ZERO,
                "リザルト以外では進まない"
            );
        }
    }

    #[test]
    fn show_result_restarts_the_sprite_animation() {
        let mut app = app_showing_result();
        app.update(result_sprite::FRAME_DURATION * 2);
        assert_eq!(app.result_sprite.current_frame(), 2);
        app.show_result(sample_result(), None);
        assert_eq!(
            app.result_sprite.elapsed(),
            Duration::ZERO,
            "入るたびに最初のコマから"
        );
        assert_eq!(app.result_sprite.current_frame(), 0);
    }

    /// 1問以上あって全問不正解(GAME OVER)なGameResult
    fn game_over_result() -> GameResult {
        let mut result = sample_result();
        result.total = 5;
        result.correct = 0;
        result
    }

    #[test]
    fn game_over_result_does_not_draw_the_sprite() {
        let (width, height) = (120u16, 40u16);
        let mut app = App::new();
        app.show_result(game_over_result(), None);
        app.update(LONG_ENOUGH);
        let without_sprite = rendered_buffer(&mut app, width, height);
        app.result_sprite = halfblocks_sprite();
        let with_sprite_configured = rendered_buffer(&mut app, width, height);
        assert_eq!(
            without_sprite, with_sprite_configured,
            "GAME OVERでは、キャラクターを描けるsprite構成でも描画結果が変わらない(描かれない)"
        );
    }

    #[test]
    fn game_over_switches_bgm_to_result_failure_category() {
        let mut app = App::new();
        app.enter_result(game_over_result());
        assert_eq!(app.current_bgm.as_deref(), Some("Apex_Pursuit"));
    }

    #[test]
    fn non_game_over_result_still_uses_the_regular_result_bgm() {
        let mut app = App::new();
        app.enter_result(sample_result());
        assert_eq!(app.current_bgm.as_deref(), Some("New_Personal_Best"));
    }

    #[test]
    fn wide_result_screen_draws_the_sprite_left_of_the_card_without_touching_the_text() {
        let (width, height) = (120u16, 40u16);
        let mut app = app_showing_result();
        app.update(LONG_ENOUGH); // テキストを全部出しておく
        let without = rendered_buffer(&mut app, width, height);
        app.result_sprite = halfblocks_sprite();
        let with = rendered_buffer(&mut app, width, height);

        let card = result_card_for(&app, width, height);
        let changed: Vec<_> = with
            .area
            .positions()
            .filter(|p| with[*p] != without[*p])
            .collect();
        assert!(!changed.is_empty(), "広い画面ではキャラクターが描かれる");
        let inner = theme::panel("").inner(screen_rect(rect(0, 0, width, height)));
        for p in &changed {
            assert!(p.x < card.x, "({},{}): カードの左側だけに描く", p.x, p.y);
            assert!(inner.contains(*p), "({},{}): 外枠の内側に描く", p.x, p.y);
        }
        for p in card.positions() {
            assert_eq!(
                with[p], without[p],
                "カード({},{})はキャラクターの有無で変わらない",
                p.x, p.y
            );
        }
    }

    #[test]
    fn narrow_result_screen_omits_the_sprite_and_keeps_the_card_intact() {
        for (width, height) in [(80u16, 30u16), (80, 24), (60, 30), (120, 11)] {
            let mut app = app_showing_result();
            app.update(LONG_ENOUGH);
            let without = rendered_buffer(&mut app, width, height);
            app.result_sprite = halfblocks_sprite();
            let with = rendered_buffer(&mut app, width, height);
            assert_eq!(
                with, without,
                "{width}x{height}: 余白が狭ければキャラクターを描かない"
            );
            let text = rendered_compact(&mut app);
            assert!(
                text.contains("平均反応時間"),
                "{width}x{height}: テキストは読める"
            );
        }
    }

    #[test]
    fn result_screen_with_sprite_renders_without_panicking_at_any_size() {
        let mut sprite = halfblocks_sprite();
        for (width, height) in [
            (1u16, 1u16),
            (5, 3),
            (10, 6),
            (20, 8),
            (80, 24),
            (100, 30),
            (200, 60),
        ] {
            for save_error in [None, Some("保存エラー".to_string())] {
                let mut app = app_showing_result();
                app.screen = Screen::Result(sample_result(), save_error);
                std::mem::swap(&mut app.result_sprite, &mut sprite);
                rendered_cells(&mut app, width, height);
                std::mem::swap(&mut app.result_sprite, &mut sprite);
            }
        }
    }

    #[test]
    fn result_screen_with_an_image_protocol_sprite_still_shows_the_text() {
        // 画像プロトコルのキャラクターはカードと重ならないので、カードの文字は端末へ出力される
        let (width, height) = (120u16, 40u16);
        let mut app = app_showing_result();
        app.update(LONG_ENOUGH);
        app.result_sprite = ResultSprite::with_picker(Some(test_picker(ProtocolType::Iterm2)));
        let buffer = rendered_buffer(&mut app, width, height);
        let card = result_card_for(&app, width, height);
        assert!(
            card.positions().all(|p| !buffer[p].skip),
            "カードのセルは出力される"
        );
        assert!(
            buffer
                .area
                .positions()
                .any(|p| buffer[p].symbol().starts_with('\x1b')),
            "キャラクターの画像データが入る"
        );
    }
}
