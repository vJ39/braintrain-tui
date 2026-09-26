use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::theme;
use crate::game::{contains, row_index, Difficulty, Game, GameResult, ScoreTracker};

mod text_image;

use text_image::TextImageRenderer;

pub const GAME_ID: &str = "mental_calc";

const CHOICE_COUNT: usize = 4;

/// フッターの高さ
const FOOTER_HEIGHT: u16 = 3;

/// 問題文エリアの最小の高さ(枠+1行)
const EXPR_MIN_HEIGHT: u16 = 3;

/// 画像表示の時の、問題式の背景色(黒い文字が読みやすい明るい水色)
const EXPR_IMAGE_BG: [u8; 3] = [200, 235, 245];

/// 画像表示の時の、選択肢の背景色(黒い文字が読みやすい明るい灰色)
const CHOICE_IMAGE_BG: [u8; 3] = [235, 235, 235];

/// 選択肢の番号キー(「 1 」+空白2つ)の幅。画像表示の時は番号の右に画像を置く
const CHOICE_KEY_WIDTH: u16 = 5;

/// 描画エリアを「問題文」「選択肢」「フッター」に分割する。
/// 上端のHUD(theme::split_hud)を除いた残りを分ける。renderはHUDを同じsplit_hudで切り出す。
/// 画像表示で問題式を大きく描けるよう、問題文エリアはフッターを除いた高さの1/3にする
/// (選択肢が1行ずつ入る高さは残す)
fn split_areas(area: Rect) -> (Rect, Rect, Rect) {
    let (_, body) = theme::split_hud(area);
    let available = body.height.saturating_sub(FOOTER_HEIGHT);
    let choices_min = CHOICE_COUNT as u16 + 2;
    let expr_height = (available / 3)
        .min(available.saturating_sub(choices_min))
        .max(EXPR_MIN_HEIGHT);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(expr_height),
            Constraint::Min(choices_min),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(body);
    (rows[0], rows[1], rows[2])
}

struct Question {
    expression: String,
    choices: [i64; CHOICE_COUNT],
    correct_index: usize,
}

fn generate_question(rng: &mut impl Rng, difficulty: Difficulty) -> Question {
    let (expression, answer) = match difficulty {
        Difficulty::Beginner => {
            let a = rng.gen_range(1..=9);
            let b = rng.gen_range(1..=9);
            if rng.gen_bool(0.5) {
                (format!("{a} + {b}"), a + b)
            } else {
                let (x, y) = if a >= b { (a, b) } else { (b, a) };
                (format!("{x} - {y}"), x - y)
            }
        }
        Difficulty::Intermediate => {
            let a = rng.gen_range(10..=99);
            let b = rng.gen_range(10..=99);
            if rng.gen_bool(0.5) {
                (format!("{a} + {b}"), a + b)
            } else {
                let (x, y) = if a >= b { (a, b) } else { (b, a) };
                (format!("{x} - {y}"), x - y)
            }
        }
        Difficulty::Advanced => {
            if rng.gen_bool(0.5) {
                let a = rng.gen_range(11..=20);
                let b = rng.gen_range(2..=9);
                (format!("{a} × {b}"), a * b)
            } else {
                let b = rng.gen_range(2..=9);
                let quotient = rng.gen_range(2..=12);
                let a = b * quotient;
                (format!("{a} ÷ {b}"), quotient)
            }
        }
    };

    let mut choices = vec![answer];
    while choices.len() < CHOICE_COUNT {
        let offset = rng.gen_range(-5..=5);
        let candidate = answer + offset;
        if offset != 0 && !choices.contains(&candidate) {
            choices.push(candidate);
        }
    }
    choices.shuffle(rng);
    let correct_index = choices.iter().position(|&c| c == answer).unwrap();

    Question {
        expression,
        choices: choices.try_into().unwrap(),
        correct_index,
    }
}

pub struct MentalCalcGame {
    difficulty: Difficulty,
    tracker: ScoreTracker,
    current: Question,
    question_started_at: Instant,
    /// 直前の回答の正誤表示(描画専用)
    feedback: AnswerFeedback,
    /// 問題式を画像で大きく描く描画器
    expr_renderer: TextImageRenderer,
    /// 選択肢を画像で大きく描く描画器。選択肢ごとにキャッシュを持つため4つに分ける
    choice_renderers: [TextImageRenderer; CHOICE_COUNT],
}

impl MentalCalcGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let mut rng = rand::thread_rng();
        // 端末への問い合わせは1回だけにして、結果を5つの描画器で共有する
        let picker = text_image::detect_picker();
        Self {
            difficulty,
            tracker: ScoreTracker::new(),
            current: generate_question(&mut rng, difficulty),
            question_started_at: Instant::now(),
            feedback: AnswerFeedback::new(),
            expr_renderer: TextImageRenderer::new(picker.clone()),
            choice_renderers: std::array::from_fn(|_| TextImageRenderer::new(picker.clone())),
        }
    }

    /// 問題文として表示する文字列(例: 「12 + 8 = ?」)
    fn expression_text(&self) -> String {
        format!("{} = ?", self.current.expression)
    }

    /// 問題式を画像で大きく描く。描けない場合はfalse(呼び出し側がテキスト表示にする)。
    /// 1行しか無い範囲では通常の文字と大きさが変わらないので画像にしない
    fn render_expression_image(&self, frame: &mut Frame, inner: Rect) -> bool {
        let inner = inner.intersection(frame.area());
        if inner.height < 2 {
            return false;
        }
        let text = self.expression_text();
        let Some((cols, rows)) = self.expr_renderer.fit(&text, inner.width, inner.height) else {
            return false;
        };
        let area = Rect::new(
            inner.x + (inner.width - cols) / 2,
            inner.y + (inner.height - rows) / 2,
            cols,
            rows,
        );
        self.expr_renderer.draw(frame, area, &text, EXPR_IMAGE_BG)
    }

    /// 選択肢を画像で大きく描く。描けない場合は何も描かずfalse(呼び出し側がテキスト表示にする)。
    /// 各選択肢はクリック判定(row_index)と同じ帯の中に、番号キーと画像を横に並べて描く。
    /// 帯の最下行は空けて、上下に並ぶ選択肢の画像がくっつかないようにする
    fn render_choice_images(&self, frame: &mut Frame, inner: Rect) -> bool {
        let inner = inner.intersection(frame.area());
        let bands = theme::row_bands(inner, CHOICE_COUNT as u16);
        if bands.iter().any(|band| band.height < 2) {
            return false;
        }
        // row_bandsは余りを最後の帯に集めるため、そのまま使うと最後の選択肢だけ
        // 文字が大きくなる。全選択肢を同じ大きさにそろえるため、最小の帯の高さに合わせる
        let uniform_height = bands
            .iter()
            .map(|band| band.height)
            .min()
            .unwrap_or(0);
        let max_cols = inner.width.saturating_sub(CHOICE_KEY_WIDTH);
        let texts: Vec<String> = self.current.choices.iter().map(|v| v.to_string()).collect();
        let mut fits = Vec::with_capacity(CHOICE_COUNT);
        for (renderer, text) in self.choice_renderers.iter().zip(&texts) {
            let Some(fit) = renderer.fit(text, max_cols, uniform_height - 1) else {
                return false;
            };
            fits.push(fit);
        }
        // 番号の位置がそろうよう、最も幅の広い画像に合わせた列を横中央に置いて左寄せで描く
        let widest = fits.iter().map(|&(cols, _)| cols).max().unwrap_or(0);
        let column_width = CHOICE_KEY_WIDTH + widest;
        let column_x = inner.x + (inner.width - column_width) / 2;
        for (i, (((renderer, text), band), (cols, rows))) in self
            .choice_renderers
            .iter()
            .zip(&texts)
            .zip(&bands)
            .zip(fits)
            .enumerate()
        {
            let image_area = Rect::new(
                column_x + CHOICE_KEY_WIDTH,
                band.y + (band.height - 1 - rows) / 2,
                cols,
                rows,
            );
            let key_row = image_area.y + (rows - 1) / 2;
            frame.render_widget(
                Paragraph::new(theme::choice_line(i + 1, String::new())),
                Rect::new(column_x, key_row, CHOICE_KEY_WIDTH, 1),
            );
            renderer.draw(frame, image_area, text, CHOICE_IMAGE_BG);
        }
        true
    }

    fn advance_question(&mut self, answered_index: usize) {
        let is_correct = answered_index == self.current.correct_index;
        let latency_ms = self.question_started_at.elapsed().as_millis() as f64;
        self.tracker.record(is_correct, latency_ms);
        let answer = self.current.choices[self.current.correct_index];
        self.feedback.record(
            is_correct,
            format!("{} = {answer}", self.current.expression),
        );
        audio::play_se(if is_correct {
            SeKind::Correct
        } else {
            SeKind::Incorrect
        });
        if !self.tracker.is_session_finished() {
            let mut rng = rand::thread_rng();
            self.current = generate_question(&mut rng, self.difficulty);
            self.question_started_at = Instant::now();
        }
    }
}

impl Game for MentalCalcGame {
    fn handle_key(&mut self, key: KeyEvent) {
        if self.tracker.is_session_finished() {
            return;
        }
        if let KeyCode::Char(c @ '1'..='4') = key.code {
            let index = c.to_digit(10).unwrap() as usize - 1;
            self.advance_question(index);
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        if self.tracker.is_session_finished() {
            return;
        }
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let (_, choices_area, _) = split_areas(area);
        let inner = Block::default().borders(Borders::ALL).inner(choices_area);
        if !contains(inner, mouse.column, mouse.row) {
            return;
        }
        if let Some(index) = row_index(inner, mouse.row, CHOICE_COUNT as u16) {
            self.advance_question(index);
        }
    }

    fn update(&mut self, dt: Duration) {
        self.feedback.tick(dt);
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud_area, _) = theme::split_hud(area);
        let (expr_area, choices_area, footer_area) = split_areas(area);
        theme::render_hud(
            frame,
            hud_area,
            "暗算スピード",
            self.difficulty,
            self.tracker.total(),
            &self.feedback,
        );

        let expr_line = Line::from(vec![
            Span::styled(
                self.current.expression.clone(),
                Style::default()
                    .fg(theme::ACCENT_STRONG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  =  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "?",
                Style::default()
                    .fg(theme::HIGHLIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        let expr_block = theme::focus_panel(" この式の答えは？ ", self.feedback.current());
        let expr_inner = expr_block.inner(expr_area);
        frame.render_widget(expr_block, expr_area);
        if !self.render_expression_image(frame, expr_inner) {
            // 画像で描けない端末では通常の文字で、問題文エリアの縦中央に描く
            let expr_paragraph = Paragraph::new(expr_line).alignment(Alignment::Center);
            frame.render_widget(expr_paragraph, theme::vertical_center(expr_inner, 1));
        }

        // 選択肢はクリック判定(row_index)と同じ帯に1つずつ描く
        let choices_block = theme::panel(" こたえを選ぶ ");
        let choices_inner = choices_block.inner(choices_area);
        frame.render_widget(choices_block, choices_area);
        if !self.render_choice_images(frame, choices_inner) {
            let texts: Vec<String> = self.current.choices.iter().map(|v| v.to_string()).collect();
            theme::render_choice_rows(frame, choices_inner, &texts);
        }

        theme::render_hint_footer(frame, footer_area, &[("1〜4", "回答"), ("q", "終了")]);
    }

    fn is_finished(&self) -> bool {
        self.tracker.is_session_finished()
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use ratatui_image::picker::{Picker, ProtocolType};

    #[test]
    fn generated_question_has_unique_choices() {
        let mut rng = StdRng::seed_from_u64(30);
        for _ in 0..100 {
            let q = generate_question(&mut rng, Difficulty::Intermediate);
            let mut sorted = q.choices.to_vec();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), CHOICE_COUNT, "選択肢に重複がある");
            assert!(q.correct_index < CHOICE_COUNT);
        }
    }

    #[test]
    fn advanced_division_always_divides_evenly() {
        let mut rng = StdRng::seed_from_u64(31);
        for _ in 0..100 {
            let q = generate_question(&mut rng, Difficulty::Advanced);
            if q.expression.contains('÷') {
                let answer = q.choices[q.correct_index];
                let parts: Vec<&str> = q.expression.split(" ÷ ").collect();
                let a: i64 = parts[0].parse().unwrap();
                let b: i64 = parts[1].parse().unwrap();
                assert_eq!(a % b, 0);
                assert_eq!(a / b, answer);
            }
        }
    }

    #[test]
    fn beginner_subtraction_never_goes_negative() {
        let mut rng = StdRng::seed_from_u64(32);
        for _ in 0..100 {
            let q = generate_question(&mut rng, Difficulty::Beginner);
            if q.expression.contains('-') {
                let answer = q.choices[q.correct_index];
                assert!(answer >= 0);
            }
        }
    }

    #[test]
    fn advance_question_records_correct_answer() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        let answer = game.current.correct_index;
        game.advance_question(answer);
        assert_eq!(game.tracker.total(), 1);
    }

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    #[test]
    fn clicking_a_choice_row_selects_that_index() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 15);
        let (_, choices_area, _) = split_areas(area);
        let inner = Block::default().borders(Borders::ALL).inner(choices_area);
        let correct = game.current.correct_index;
        let row = inner.y + correct as u16;
        game.handle_mouse(left_click(inner.x, row), area);
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }

    #[test]
    fn clicking_outside_choices_area_does_nothing() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        let area = Rect::new(0, 0, 40, 15);
        let (expr_area, _, _) = split_areas(area);
        game.handle_mouse(left_click(expr_area.x, expr_area.y), area);
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn answering_shows_feedback_with_expression_and_answer() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        let expression = game.current.expression.clone();
        let answer = game.current.choices[game.current.correct_index];
        let wrong = (game.current.correct_index + 1) % CHOICE_COUNT;
        game.advance_question(wrong);
        let flash = game.feedback.current().expect("回答直後は正誤を表示する");
        assert_eq!(flash.verdict, crate::game::feedback::Verdict::Incorrect);
        assert_eq!(flash.detail, format!("{expression} = {answer}"));
        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.feedback.current().is_none());
    }

    /// 描画結果の各行を文字列にして返す
    fn rendered_rows(game: &MentalCalcGame, area: Rect) -> Vec<String> {
        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| game.render(frame, area)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    /// 画像プロトコルを固定したPickerで、問題式・選択肢の描画器を作り直す
    fn use_picker(game: &mut MentalCalcGame, protocol: ProtocolType) {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(protocol);
        game.expr_renderer = TextImageRenderer::new(Some(picker.clone()));
        game.choice_renderers =
            std::array::from_fn(|_| TextImageRenderer::new(Some(picker.clone())));
    }

    fn fixed_question(game: &mut MentalCalcGame, expression: &str, choices: [i64; CHOICE_COUNT]) {
        game.current = Question {
            expression: expression.to_string(),
            choices,
            correct_index: 0,
        };
    }

    /// 問題文エリアの枠の内側
    fn expr_inner(area: Rect) -> Rect {
        let (expr_area, _, _) = split_areas(area);
        Block::default().borders(Borders::ALL).inner(expr_area)
    }

    /// 選択肢エリアの枠の内側
    fn choices_inner(area: Rect) -> Rect {
        let (_, choices_area, _) = split_areas(area);
        Block::default().borders(Borders::ALL).inner(choices_area)
    }

    fn font_available() -> bool {
        let available = text_image::system_font().is_some();
        if !available {
            eprintln!("システムフォントを読み込めないため、画像表示のテストを飛ばす");
        }
        available
    }

    #[test]
    fn expression_text_shows_the_expression_and_a_question_mark() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        fixed_question(&mut game, "12 + 8", [20, 21, 19, 18]);
        assert_eq!(game.expression_text(), "12 + 8 = ?");
    }

    #[test]
    fn layout_areas_stack_without_overlap_and_expression_grows_on_tall_terminal() {
        for (w, h) in [(40, 15), (50, 30), (80, 24), (120, 50)] {
            let area = Rect::new(0, 0, w, h);
            let (hud, _) = theme::split_hud(area);
            let (expr, choices, footer) = split_areas(area);
            assert_eq!(expr.y, hud.bottom());
            assert_eq!(choices.y, expr.bottom());
            assert_eq!(footer.y, choices.bottom());
            assert!(footer.bottom() <= area.bottom());
            assert!(expr.height >= 3, "問題文の枠と1行は必ず入る");
            assert!(
                choices.height >= CHOICE_COUNT as u16 + 2,
                "選択肢が1行ずつ入る"
            );
        }
        // 縦に大きい端末では問題文エリアも大きくなり、文字を大きく描ける
        let (expr, _, _) = split_areas(Rect::new(0, 0, 80, 40));
        assert!(expr.height >= 8, "問題文エリアの高さ: {}", expr.height);
    }

    #[test]
    fn fallback_shows_expression_as_text_vertically_centered() {
        // テスト環境では画像プロトコルを検出しないのでテキスト表示になる
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        assert!(!game.expr_renderer.uses_image());
        fixed_question(&mut game, "12 + 8", [20, 21, 19, 18]);
        let area = Rect::new(0, 0, 60, 40);
        let rows = rendered_rows(&game, area);
        let inner = expr_inner(area);
        let row = (inner.y..inner.bottom())
            .find(|&y| rows[y as usize].contains("12 + 8"))
            .expect("問題式が問題文エリアに描かれること");
        assert!(rows[row as usize].contains('?'));
        assert_eq!(row, inner.y + (inner.height - 1) / 2, "縦中央に描く");
    }

    #[test]
    fn fallback_is_used_when_the_font_cannot_be_loaded() {
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        game.expr_renderer = TextImageRenderer::without_font(Some(picker.clone()));
        game.choice_renderers =
            std::array::from_fn(|_| TextImageRenderer::without_font(Some(picker.clone())));
        fixed_question(&mut game, "34 - 12", [22, 23, 21, 20]);
        let rows = rendered_rows(&game, Rect::new(0, 0, 60, 40)).join("\n");
        assert!(rows.contains("34 - 12"), "問題式はテキストで描かれる");
        for choice in ["22", "23", "21", "20"] {
            assert!(rows.contains(choice), "選択肢{choice}はテキストで描かれる");
        }
    }

    #[test]
    fn render_does_not_panic_with_or_without_image_protocols() {
        let sizes = [(60, 40), (80, 24), (40, 15), (12, 9), (4, 4), (1, 1)];
        let mut game = MentalCalcGame::new(Difficulty::Advanced);
        fixed_question(&mut game, "108 ÷ 9", [12, 13, 11, 10]);
        for (w, h) in sizes {
            rendered_rows(&game, Rect::new(0, 0, w, h));
        }
        for protocol in [
            ProtocolType::Halfblocks,
            ProtocolType::Sixel,
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
        ] {
            use_picker(&mut game, protocol);
            for (w, h) in sizes {
                rendered_rows(&game, Rect::new(0, 0, w, h));
            }
        }
    }

    #[test]
    fn image_mode_draws_expression_large_inside_the_expression_panel() {
        if !font_available() {
            return;
        }
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        use_picker(&mut game, ProtocolType::Halfblocks);
        fixed_question(&mut game, "12 + 8", [20, 21, 19, 18]);
        let area = Rect::new(0, 0, 80, 40);
        rendered_rows(&game, area);
        let drawn = game
            .expr_renderer
            .last_area()
            .expect("問題式が画像で描かれること");
        let inner = expr_inner(area);
        assert_eq!(drawn.intersection(inner), drawn, "問題文エリアの内側に描く");
        assert!(drawn.height >= 3, "複数行分の大きさで描く({drawn:?})");
        // 横中央に置く
        let left = drawn.x - inner.x;
        let right = inner.right() - drawn.right();
        assert!(left.abs_diff(right) <= 1, "左右の余白 {left}/{right}");
    }

    #[test]
    fn image_mode_draws_each_choice_inside_the_row_that_selects_it() {
        if !font_available() {
            return;
        }
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        use_picker(&mut game, ProtocolType::Halfblocks);
        fixed_question(&mut game, "12 + 8", [20, 21, 7, 108]);
        let area = Rect::new(0, 0, 80, 40);
        let rows = rendered_rows(&game, area);
        let inner = choices_inner(area);
        let mut previous_bottom = inner.y;
        for (i, renderer) in game.choice_renderers.iter().enumerate() {
            let drawn = renderer.last_area().expect("選択肢が画像で描かれること");
            assert_eq!(drawn.intersection(inner), drawn, "選択肢エリアの内側に描く");
            assert!(drawn.height >= 2, "選択肢も複数行分の大きさ({drawn:?})");
            assert!(
                drawn.y >= previous_bottom,
                "選択肢の画像は上から順に重ならない"
            );
            previous_bottom = drawn.bottom();
            // 画像のどの行をクリックしても、その選択肢として扱われる
            for y in drawn.y..drawn.bottom() {
                assert_eq!(row_index(inner, y, CHOICE_COUNT as u16), Some(i));
            }
            // 番号キーは画像の左に、画像の縦中央の行に描く
            let key_row = drawn.y + (drawn.height - 1) / 2;
            let key = format!(" {} ", i + 1);
            let left_of_image: String = rows[key_row as usize]
                .chars()
                .take(drawn.x as usize)
                .collect();
            assert!(
                left_of_image.contains(&key),
                "選択肢{}の番号が画像の左にある",
                i + 1
            );
        }
        // 番号の位置がそろうよう、画像の左端をそろえる
        let lefts: Vec<u16> = game
            .choice_renderers
            .iter()
            .map(|r| r.last_area().unwrap().x)
            .collect();
        assert!(lefts.windows(2).all(|w| w[0] == w[1]), "左端: {lefts:?}");
    }

    #[test]
    fn image_mode_draws_every_choice_at_the_same_size() {
        // 選択肢エリアの高さがCHOICE_COUNTで割り切れない時、row_bandsは余りを最後の帯に
        // 集めるため、対策が無いと最後の選択肢だけ文字が大きく描かれてしまう
        if !font_available() {
            return;
        }
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        use_picker(&mut game, ProtocolType::Halfblocks);
        fixed_question(&mut game, "79 + 72", [150, 147, 146, 151]);
        // 高さがCHOICE_COUNT(4)で割り切れないエリアを選ぶ
        let area = Rect::new(0, 0, 80, 41);
        rendered_rows(&game, area);
        let heights: Vec<u16> = game
            .choice_renderers
            .iter()
            .map(|r| r.last_area().unwrap().height)
            .collect();
        assert!(
            heights.windows(2).all(|w| w[0] == w[1]),
            "全選択肢の描画高さが揃うこと: {heights:?}"
        );
    }

    #[test]
    fn image_mode_reuses_encoding_until_the_question_changes() {
        if !font_available() {
            return;
        }
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        use_picker(&mut game, ProtocolType::Halfblocks);
        fixed_question(&mut game, "3 + 4", [7, 8, 6, 5]);
        let area = Rect::new(0, 0, 80, 40);
        for _ in 0..3 {
            rendered_rows(&game, area);
        }
        assert_eq!(game.expr_renderer.encode_count(), 1);
        for renderer in &game.choice_renderers {
            assert_eq!(renderer.encode_count(), 1);
        }
        // 次の問題に変わったら、変わった所だけ作り直す(3番目の選択肢「6」は同じ)
        fixed_question(&mut game, "9 - 3", [5, 7, 6, 4]);
        rendered_rows(&game, area);
        rendered_rows(&game, area);
        assert_eq!(game.expr_renderer.encode_count(), 2);
        let counts: Vec<usize> = game
            .choice_renderers
            .iter()
            .map(|r| r.encode_count())
            .collect();
        assert_eq!(counts, vec![2, 2, 1, 2]);
    }

    #[test]
    fn clicking_the_row_where_a_choice_is_drawn_selects_it_even_on_tall_terminal() {
        // 縦に大きい端末では選択肢1つあたりの帯が複数行になる。
        // 画面に選択肢が見えている行をクリックしたら、その選択肢として扱われること
        let area = Rect::new(0, 0, 50, 30);
        let mut game = MentalCalcGame::new(Difficulty::Beginner);
        // 帯の位置ずれが最も大きく出る最後の選択肢を正解扱いにする
        game.current.correct_index = CHOICE_COUNT - 1;
        let correct = game.current.correct_index;
        let label = format!(" {}   {}", correct + 1, game.current.choices[correct]);
        let (_, choices_area, _) = split_areas(area);
        let inner = Block::default().borders(Borders::ALL).inner(choices_area);
        let rows = rendered_rows(&game, area);
        let row = (inner.y..inner.y + inner.height)
            .find(|&y| rows[y as usize].contains(&label))
            .expect("正解の選択肢が選択肢エリア内に描かれていること");
        game.handle_mouse(left_click(inner.x, row), area);
        let result = game.tracker.to_result(GAME_ID, game.difficulty);
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
    }
}
