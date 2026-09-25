//! カウントマニア: ランダムに並んだ1〜Nの数字付き円を、1から順にクリックしていくマウス専用ゲーム。
//!
//! 1セッション=3ラウンド。ラウンドごとに「全部押せたか(クリア)/ライフが尽きたか(失敗)」を
//! ScoreTrackerに1件として記録する。キー入力は受け付けない。

mod circle_image;
mod layout;

use std::cell::RefCell;
use std::time::Duration;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::audio::{self, SeKind};
use crate::game::feedback::AnswerFeedback;
use crate::game::theme;
use crate::game::{Difficulty, Game, GameResult, ScoreTracker};

use circle_image::{BoardCircle, CircleRenderer};
use layout::{back_to_front, hit_test, layout_circles, CircleSize, Placement};

pub const GAME_ID: &str = "count_mania";

/// 1セッションのラウンド数
pub const ROUNDS_PER_SESSION: u32 = 3;

/// ラウンド終了から次のラウンド開始までの間隔。この間はクリックを受け付けない
/// (前のラウンドの最後のクリックが次の盤面に当たらないようにするため)
pub const ROUND_INTERVAL: Duration = Duration::from_millis(1200);

/// 難易度ごとのパラメータ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DifficultyParams {
    /// 最大の数字(1〜Nを並べる)
    pub max_number: u8,
    /// 使う円のサイズ段階
    pub size_levels: &'static [CircleSize],
    /// 1ラウンドのライフ
    pub lives: u8,
    /// 円を密集させて配置するか
    pub dense: bool,
    /// 失敗したラウンドの記録に使う時間(ms)。そのラウンドの想定上限
    pub fail_latency_ms: f64,
}

pub fn params(difficulty: Difficulty) -> DifficultyParams {
    use CircleSize::{Large, Medium, Small};
    match difficulty {
        Difficulty::Beginner => DifficultyParams {
            max_number: 10,
            size_levels: &[Large, Medium],
            lives: 3,
            dense: false,
            fail_latency_ms: 30_000.0,
        },
        Difficulty::Intermediate => DifficultyParams {
            max_number: 14,
            size_levels: &[Large, Medium, Small],
            lives: 2,
            dense: false,
            fail_latency_ms: 45_000.0,
        },
        Difficulty::Advanced => DifficultyParams {
            max_number: 20,
            size_levels: &[Large, Medium, Small],
            lives: 2,
            dense: true,
            fail_latency_ms: 60_000.0,
        },
    }
}

/// ラウンド中の円1つ(番号・サイズ段階・色)。配置(位置)は描画エリアに依存するので別に持つ
#[derive(Debug, Clone, Copy)]
struct RoundCircle {
    number: u8,
    size: CircleSize,
    color: [u8; 3],
}

/// 1ラウンドの状態
struct Round {
    circles: Vec<RoundCircle>,
    /// 次に押すべき数字。これより小さい数字の円は押し終えて消えている
    next: u8,
    lives: u8,
    /// ラウンド開始からの経過時間
    elapsed: Duration,
}

fn new_round(rng: &mut impl Rng, params: &DifficultyParams) -> Round {
    // サイズ段階は順番に割り当ててからシャッフルし、どの段階も最低1つは使われるようにする
    let levels = params.size_levels;
    let mut sizes: Vec<CircleSize> = (0..params.max_number as usize)
        .map(|i| levels[i % levels.len()])
        .collect();
    sizes.shuffle(rng);
    let circles = (1..=params.max_number)
        .zip(sizes)
        .map(|(number, size)| RoundCircle {
            number,
            size,
            color: random_color(rng),
        })
        .collect();
    Round {
        circles,
        next: 1,
        lives: params.lives,
        elapsed: Duration::ZERO,
    }
}

/// 黒い数字が読みやすい、明るく鮮やかな色をランダムに作る(色相だけランダム、彩度・明度は固定幅)
fn random_color(rng: &mut impl Rng) -> [u8; 3] {
    let hue: f64 = rng.gen_range(0.0..360.0);
    let saturation: f64 = rng.gen_range(0.45..0.75);
    let value: f64 = rng.gen_range(0.85..1.0);
    hsv_to_rgb(hue, saturation, value)
}

/// HSV(色相0〜360, 彩度0〜1, 明度0〜1)をRGBに変換する
fn hsv_to_rgb(hue: f64, saturation: f64, value: f64) -> [u8; 3] {
    let chroma = value * saturation;
    let h = (hue.rem_euclid(360.0)) / 60.0;
    let x = chroma * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = value - chroma;
    let to_u8 = |c: f64| ((c + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [to_u8(r), to_u8(g), to_u8(b)]
}

/// 配置のキャッシュ。描画エリアかラウンドが変わった時だけ作り直す
struct LayoutCache {
    board: Rect,
    round_serial: u32,
    placements: Vec<Placement>,
}

pub struct CountManiaGame {
    difficulty: Difficulty,
    params: DifficultyParams,
    tracker: ScoreTracker,
    round: Round,
    /// ラウンドの通し番号(配置キャッシュの作り直し判定に使う)
    round_serial: u32,
    /// ラウンド間の待ち時間の残り。Noneならプレイ中
    interval: Option<Duration>,
    feedback: AnswerFeedback,
    layout: RefCell<Option<LayoutCache>>,
    renderer: CircleRenderer,
}

/// ゲームの描画エリアのうち、円を並べるボード(枠の内側)。renderとhandle_mouseで共有する
fn board_area(area: Rect) -> Rect {
    let (_, body) = theme::split_hud(area);
    Block::default().borders(Borders::ALL).inner(body)
}

impl CountManiaGame {
    pub fn new(difficulty: Difficulty) -> Self {
        let params = params(difficulty);
        let round = new_round(&mut rand::thread_rng(), &params);
        Self {
            difficulty,
            params,
            tracker: ScoreTracker::new(),
            round,
            round_serial: 0,
            interval: None,
            feedback: AnswerFeedback::new(),
            layout: RefCell::new(None),
            renderer: CircleRenderer::new(),
        }
    }

    /// boardに対する円の配置(キャッシュ済みならそれを返す)
    fn placements(&self, board: Rect) -> Vec<Placement> {
        let mut cache = self.layout.borrow_mut();
        if let Some(cached) = cache.as_ref() {
            if cached.board == board && cached.round_serial == self.round_serial {
                return cached.placements.clone();
            }
        }
        let circles: Vec<(u8, CircleSize)> = self
            .round
            .circles
            .iter()
            .map(|c| (c.number, c.size))
            .collect();
        let placements =
            layout_circles(&mut rand::thread_rng(), board, &circles, self.params.dense);
        *cache = Some(LayoutCache {
            board,
            round_serial: self.round_serial,
            placements: placements.clone(),
        });
        placements
    }

    /// 番号numberの円がまだ残っているか
    fn is_visible(&self, number: u8) -> bool {
        number >= self.round.next && number <= self.params.max_number
    }

    /// いま何ラウンド目か(1始まり。全ラウンド終了後は最終ラウンドのまま)
    fn current_round_number(&self) -> u32 {
        (self.tracker.total() + u32::from(!self.is_finished() && self.interval.is_none()))
            .clamp(1, ROUNDS_PER_SESSION)
    }

    fn click_correct(&mut self) {
        audio::play_se(SeKind::Correct);
        self.round.next += 1;
        if self.round.next > self.params.max_number {
            let latency_ms = self.round.elapsed.as_secs_f64() * 1000.0;
            self.tracker.record(true, latency_ms);
            self.feedback.record(
                true,
                format!("CLEAR {:.1}秒", self.round.elapsed.as_secs_f64()),
            );
            self.finish_round();
        }
    }

    fn click_wrong(&mut self) {
        audio::play_se(SeKind::Incorrect);
        self.round.lives = self.round.lives.saturating_sub(1);
        if self.round.lives == 0 {
            self.tracker.record(false, self.params.fail_latency_ms);
            self.feedback.record(false, "ライフが尽きた");
            self.finish_round();
        } else {
            self.feedback.record(false, "ライフ -1");
        }
    }

    /// ラウンドを終える。最終ラウンドでなければ次のラウンドまでの待ち時間に入る
    fn finish_round(&mut self) {
        if !self.is_finished() {
            self.interval = Some(ROUND_INTERVAL);
        }
    }

    fn start_next_round(&mut self) {
        self.round = new_round(&mut rand::thread_rng(), &self.params);
        self.round_serial += 1;
        self.interval = None;
    }

    fn render_hud(&self, frame: &mut Frame, area: Rect) {
        let (difficulty_text, difficulty_color) = theme::difficulty_label(self.difficulty);
        let block = theme::panel(" ◆ カウントマニア ")
            .border_style(Style::default().fg(theme::flash_border_color(self.feedback.current())))
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
                Constraint::Length(20),
                Constraint::Fill(1),
                Constraint::Length(24),
            ])
            .split(inner);

        let progress = Line::from(vec![
            Span::styled(
                format!(
                    " ROUND {}/{ROUNDS_PER_SESSION} ",
                    self.current_round_number()
                ),
                theme::title_style(),
            ),
            Span::styled(
                theme::progress_bar(
                    self.tracker.total(),
                    ROUNDS_PER_SESSION,
                    ROUNDS_PER_SESSION as usize,
                ),
                Style::default().fg(theme::ACCENT),
            ),
        ]);
        frame.render_widget(Paragraph::new(progress), cols[0]);

        // 中央: 正誤表示中はそれを、そうでなければ次に押す数字を出す
        let center = match self.feedback.current() {
            Some(flash) => theme::flash_line(flash),
            None if self.interval.is_none() => Line::from(vec![
                Span::styled("つぎ ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    format!(" {} ", self.round.next),
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme::HIGHLIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" / {}", self.params.max_number),
                    Style::default().fg(theme::MUTED),
                ),
            ]),
            None => Line::from(""),
        };
        frame.render_widget(Paragraph::new(center).alignment(Alignment::Center), cols[1]);

        let lost = self.params.lives.saturating_sub(self.round.lives);
        let lives = Line::from(vec![
            Span::styled("マウス専用  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!(
                    "{}{}",
                    "♥".repeat(self.round.lives as usize),
                    "♡".repeat(lost as usize)
                ),
                Style::default()
                    .fg(theme::INCORRECT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]);
        frame.render_widget(Paragraph::new(lives).alignment(Alignment::Right), cols[2]);
    }

    /// ラウンド間の待ち時間中にボード中央へ出す案内
    fn render_interval_message(&self, frame: &mut Frame, board: Rect) {
        // 最後の数字まで押し切っていればクリア、そうでなければライフ切れ
        let cleared = self.round.next > self.params.max_number;
        let (headline, color) = if cleared {
            ("CLEAR!", theme::CORRECT)
        } else {
            ("MISS...", theme::INCORRECT)
        };
        let next_round = (self.tracker.total() + 1).min(ROUNDS_PER_SESSION);
        let lines = vec![
            Line::from(Span::styled(
                headline,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("NEXT ROUND {next_round}/{ROUNDS_PER_SESSION}"),
                theme::title_style(),
            )),
        ];
        let area = theme::vertical_center(board, lines.len() as u16);
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
    }
}

impl Game for CountManiaGame {
    /// マウス専用のため、キー入力では何もしない
    fn handle_key(&mut self, _key: KeyEvent) {}

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        // 待ち時間中・セッション終了後のクリックは受け付けない
        if self.is_finished() || self.interval.is_some() {
            return;
        }
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let board = board_area(area);
        if !crate::game::contains(board, mouse.column, mouse.row) {
            return;
        }
        let placements = self.placements(board);
        let Some(number) = hit_test(&placements, |n| self.is_visible(n), mouse.column, mouse.row)
        else {
            // 円の無い場所(消えた円の跡も含む)のクリックは何もしない
            return;
        };
        if number == self.round.next {
            self.click_correct();
        } else {
            self.click_wrong();
        }
    }

    fn update(&mut self, dt: Duration) {
        self.feedback.tick(dt);
        if let Some(remaining) = self.interval {
            let remaining = remaining.saturating_sub(dt);
            if remaining.is_zero() {
                self.start_next_round();
            } else {
                self.interval = Some(remaining);
            }
            return;
        }
        if !self.is_finished() {
            self.round.elapsed += dt;
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud, body) = theme::split_hud(area);
        self.render_hud(frame, hud);

        let block = theme::focus_panel(" 1から順にクリック ", self.feedback.current());
        let board = block.inner(body);
        frame.render_widget(block, body);
        if board.is_empty() {
            return;
        }
        if self.interval.is_some() {
            self.render_interval_message(frame, board);
            return;
        }
        // 大きい円から先に描き、小さい円ほど手前に重ねる(当たり判定hit_testの優先順位と同じ)
        let circles: Vec<BoardCircle> = back_to_front(&self.placements(board))
            .iter()
            .filter(|placement| self.is_visible(placement.number))
            .filter_map(|placement| {
                self.round
                    .circles
                    .iter()
                    .find(|c| c.number == placement.number)
                    .map(|circle| BoardCircle {
                        rect: placement.rect,
                        number: circle.number,
                        color: circle.color,
                    })
            })
            .collect();
        self.renderer.render_board(frame, board, &circles);
    }

    fn is_finished(&self) -> bool {
        self.tracker.total() >= ROUNDS_PER_SESSION
    }

    fn result(&self) -> GameResult {
        self.tracker.to_result(GAME_ID, self.difficulty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use std::collections::HashSet;

    const AREA: Rect = Rect::new(0, 0, 100, 36);

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// 番号numberの円の中心座標
    fn center_of(game: &CountManiaGame, number: u8) -> (u16, u16) {
        let placements = game.placements(board_area(AREA));
        let rect = placements
            .iter()
            .find(|p| p.number == number)
            .unwrap_or_else(|| panic!("円{number}が配置されていること"))
            .rect;
        (rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    /// 番号numberの円をクリックする(実際の当たり判定を通す)
    fn click_circle(game: &mut CountManiaGame, number: u8) {
        let (column, row) = center_of(game, number);
        game.handle_mouse(left_click(column, row), AREA);
    }

    /// どの円にも当たらないセルを探す
    fn empty_cell(game: &CountManiaGame) -> (u16, u16) {
        let board = board_area(AREA);
        let placements = game.placements(board);
        (board.y..board.bottom())
            .flat_map(|y| (board.x..board.right()).map(move |x| (x, y)))
            .find(|&(x, y)| hit_test(&placements, |_| true, x, y).is_none())
            .expect("円の無いセルがあること")
    }

    /// 配置を差し替える(乱数に頼らず、重なり具合を決めて確かめるため)。
    /// rectsはボード左上からの相対位置で(番号, x, y, 幅, 高さ)
    fn set_layout(game: &CountManiaGame, rects: &[(u8, u16, u16, u16, u16)]) {
        let board = board_area(AREA);
        let placements = rects
            .iter()
            .map(|&(number, x, y, width, height)| Placement {
                number,
                rect: Rect::new(board.x + x, board.y + y, width, height),
            })
            .collect();
        *game.layout.borrow_mut() = Some(LayoutCache {
            board,
            round_serial: game.round_serial,
            placements,
        });
    }

    /// ボード左上からの相対位置(x, y)を左クリックする
    fn click_board(game: &mut CountManiaGame, x: u16, y: u16) {
        let board = board_area(AREA);
        game.handle_mouse(left_click(board.x + x, board.y + y), AREA);
    }

    /// 大きい円(14x7)の左側に小さい円(6x3)が重なった配置。OVERLAPは両方の円の内側のセル
    const LARGE_AT: (u16, u16, u16, u16) = (10, 5, 14, 7);
    const SMALL_AT: (u16, u16, u16, u16) = (10, 7, 6, 3);
    const OVERLAP: (u16, u16) = (12, 8);

    fn set_overlap_layout(game: &CountManiaGame, large: u8, small: u8) {
        let (lx, ly, lw, lh) = LARGE_AT;
        let (sx, sy, sw, sh) = SMALL_AT;
        // 小さい円を先に渡しても、描画・当たり判定は大きさで決まる
        set_layout(game, &[(small, sx, sy, sw, sh), (large, lx, ly, lw, lh)]);
    }

    fn clear_round(game: &mut CountManiaGame) {
        for number in 1..=game.params.max_number {
            click_circle(game, number);
        }
    }

    /// 次に押すべきでない(まだ残っている)円の番号
    fn wrong_number(game: &CountManiaGame) -> u8 {
        game.round.next + 1
    }

    /// 画面を描画し、全セルを空白抜きの1つの文字列にして返す
    /// (全角文字の2セル目は空白で埋まるため、空白を除いて比較する)
    fn rendered_text(game: &CountManiaGame, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace(' ', "")
    }

    // --- 難易度パラメータ ---

    #[test]
    fn params_match_spec_for_each_difficulty() {
        use CircleSize::*;
        let beginner = params(Difficulty::Beginner);
        assert_eq!(beginner.max_number, 10);
        assert_eq!(beginner.size_levels, &[Large, Medium]);
        assert_eq!(beginner.lives, 3);
        assert!(!beginner.dense);

        let intermediate = params(Difficulty::Intermediate);
        assert_eq!(intermediate.max_number, 14);
        assert_eq!(intermediate.size_levels, &[Large, Medium, Small]);
        assert_eq!(intermediate.lives, 2);
        assert!(!intermediate.dense);

        let advanced = params(Difficulty::Advanced);
        assert_eq!(advanced.max_number, 20);
        assert_eq!(advanced.size_levels, &[Large, Medium, Small]);
        assert_eq!(advanced.lives, 2);
        assert!(advanced.dense, "上級は密集配置");
    }

    #[test]
    fn fail_latency_is_positive_and_grows_with_difficulty() {
        let b = params(Difficulty::Beginner).fail_latency_ms;
        let i = params(Difficulty::Intermediate).fail_latency_ms;
        let a = params(Difficulty::Advanced).fail_latency_ms;
        assert!(b > 0.0 && b <= i && i <= a);
    }

    #[test]
    fn new_round_has_numbers_one_to_n_using_only_allowed_sizes() {
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let p = params(difficulty);
            let mut rng = StdRng::seed_from_u64(1);
            let round = new_round(&mut rng, &p);
            let mut numbers: Vec<u8> = round.circles.iter().map(|c| c.number).collect();
            numbers.sort();
            assert_eq!(numbers, (1..=p.max_number).collect::<Vec<_>>());
            let used: HashSet<CircleSize> = round.circles.iter().map(|c| c.size).collect();
            let allowed: HashSet<CircleSize> = p.size_levels.iter().copied().collect();
            assert_eq!(
                used, allowed,
                "{difficulty:?}: 決められたサイズ段階をすべて使う"
            );
            assert_eq!(round.next, 1);
            assert_eq!(round.lives, p.lives);
        }
    }

    #[test]
    fn hsv_to_rgb_converts_primary_hues() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [255, 0, 0]);
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), [0, 255, 0]);
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), [0, 0, 255]);
        assert_eq!(hsv_to_rgb(0.0, 0.0, 1.0), [255, 255, 255]);
    }

    #[test]
    fn random_colors_are_bright_enough_for_black_digits() {
        let mut rng = StdRng::seed_from_u64(5);
        for _ in 0..200 {
            let [r, g, b] = random_color(&mut rng);
            assert!(
                r.max(g).max(b) >= 200,
                "最も明るいチャンネルが十分明るい: {r},{g},{b}"
            );
        }
    }

    #[test]
    fn new_game_starts_with_full_lives_and_next_number_one() {
        let game = CountManiaGame::new(Difficulty::Intermediate);
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 2);
        assert_eq!(game.round.circles.len(), 14);
        assert_eq!(game.tracker.total(), 0);
        assert!(!game.is_finished());
    }

    // --- 正解・クリア ---

    #[test]
    fn clicking_next_number_removes_it_and_advances() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 2);
        assert!(!game.is_visible(1), "押した円は消える");
        assert!(game.is_visible(2));
        assert_eq!(game.round.lives, 3, "正解ではライフは減らない");
        assert_eq!(game.tracker.total(), 0, "ラウンド途中は記録しない");
    }

    #[test]
    fn clearing_all_numbers_records_success_with_elapsed_time() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        game.update(Duration::from_millis(1500));
        clear_round(&mut game);
        let result = game.result();
        assert_eq!(result.total, 1);
        assert_eq!(result.correct, 1);
        assert!((result.avg_latency_ms - 1500.0).abs() < 1e-9);
    }

    // --- 不正解・失敗 ---

    #[test]
    fn clicking_wrong_number_loses_a_life_and_keeps_next() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let wrong = wrong_number(&game);
        click_circle(&mut game, wrong);
        assert_eq!(game.round.lives, 2);
        assert_eq!(game.round.next, 1, "次に押す数字は変わらない");
        assert!(game.is_visible(wrong), "不正解の円は消えない");
        assert_eq!(game.tracker.total(), 0);
    }

    #[test]
    fn losing_all_lives_ends_round_as_failure_with_limit_latency() {
        let mut game = CountManiaGame::new(Difficulty::Intermediate);
        click_circle(&mut game, 1);
        for _ in 0..2 {
            let wrong = wrong_number(&game);
            click_circle(&mut game, wrong);
        }
        let result = game.result();
        assert_eq!(result.total, 1, "ライフ0で即ラウンド終了");
        assert_eq!(result.correct, 0);
        assert_eq!(
            result.avg_latency_ms,
            params(Difficulty::Intermediate).fail_latency_ms
        );
    }

    // --- ラウンド進行 ---

    #[test]
    fn clicks_are_ignored_during_interval_then_next_round_starts() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        clear_round(&mut game);
        assert!(game.interval.is_some(), "ラウンド終了後は待ち時間に入る");
        let before = game.round.next;
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, before, "待ち時間中のクリックは無視される");
        assert_eq!(game.tracker.total(), 1);

        game.update(ROUND_INTERVAL);
        assert!(game.interval.is_none());
        assert_eq!(game.round.next, 1, "新しいラウンドは1から");
        assert_eq!(game.round.lives, 3, "ライフも元に戻る");
        assert!(game.round.elapsed.is_zero(), "経過時間も0から");
    }

    #[test]
    fn session_finishes_after_three_rounds() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        for round in 0..ROUNDS_PER_SESSION {
            assert!(
                !game.is_finished(),
                "ラウンド{round}開始時点では終わっていない"
            );
            clear_round(&mut game);
            if round + 1 < ROUNDS_PER_SESSION {
                game.update(ROUND_INTERVAL);
            }
        }
        assert!(game.is_finished());
        let result = game.result();
        assert_eq!(result.game_id, GAME_ID);
        assert_eq!(result.difficulty, Difficulty::Beginner);
        assert_eq!(result.total, ROUNDS_PER_SESSION);
        assert_eq!(result.correct, ROUNDS_PER_SESSION);
    }

    #[test]
    fn clicks_after_session_finished_are_ignored() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        for _ in 0..ROUNDS_PER_SESSION {
            clear_round(&mut game);
            game.update(ROUND_INTERVAL);
        }
        assert!(game.is_finished());
        let (column, row) = center_of(&game, 1);
        game.handle_mouse(left_click(column, row), AREA);
        assert_eq!(game.result().total, ROUNDS_PER_SESSION);
    }

    // --- マウス専用 ---

    #[test]
    fn handle_key_never_changes_state() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let keys = [
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Char(' '),
            KeyCode::Char('1'),
            KeyCode::Char('2'),
            KeyCode::Tab,
        ];
        for code in keys {
            game.handle_key(KeyEvent::from(code));
        }
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 3);
        assert_eq!(game.tracker.total(), 0);
        assert!(game.interval.is_none());
        assert!(game.feedback.current().is_none());
    }

    // --- クリック当たり判定 ---

    #[test]
    fn clicking_empty_space_does_nothing() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let (column, row) = empty_cell(&game);
        game.handle_mouse(left_click(column, row), AREA);
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 3);
    }

    #[test]
    fn clicking_outside_board_or_non_left_button_does_nothing() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        // HUD(ボード外)のクリック
        game.handle_mouse(left_click(1, 1), AREA);
        // 1の円を右クリック
        let (column, row) = center_of(&game, 1);
        game.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            AREA,
        );
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 3);
    }

    #[test]
    fn clicking_overlap_hits_smaller_circle_then_circle_below_after_removal() {
        // 小さい円1が大きい円2の上に重なっている
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        set_overlap_layout(&game, 2, 1);
        let (x, y) = OVERLAP;
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 2, "手前の小さい円1に当たる");
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 3, "円1が消えた後は下の円2に当たる");
        assert_eq!(game.round.lives, 3);
    }

    #[test]
    fn clicking_overlap_never_hits_larger_circle_below() {
        // 次に押すべき円1が大きい方で、その上に小さい円2が重なっている。重なった所は円2扱い
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        set_overlap_layout(&game, 1, 2);
        let (x, y) = OVERLAP;
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 1);
        assert_eq!(game.round.lives, 2, "手前の円2を押した扱いでライフが減る");
    }

    #[test]
    fn render_draws_smaller_circle_on_top_of_larger() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        game.round.circles[0].color = [200, 50, 50];
        game.round.circles[1].color = [50, 50, 200];
        set_overlap_layout(&game, 2, 1);
        let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        let board = board_area(AREA);
        let (x, y) = OVERLAP;
        assert_eq!(
            terminal.backend().buffer()[(board.x + x, board.y + y)].fg,
            Color::Rgb(200, 50, 50),
            "重なった所は小さい円1の色"
        );
    }

    #[test]
    fn clicking_removed_circle_does_nothing() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        // 円1の下に他の円が無い配置にする(下に円があればそちらに当たるのが正しい動作のため)
        set_layout(&game, &[(1, 2, 2, 10, 5), (2, 40, 2, 10, 5)]);
        click_circle(&mut game, 1);
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 2);
        assert_eq!(
            game.round.lives, 3,
            "消えた円の場所は空白扱いでライフは減らない"
        );
    }

    #[test]
    fn layout_is_kept_between_clicks_and_regenerated_for_new_round() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let board = board_area(AREA);
        let first = game.placements(board);
        click_circle(&mut game, 1);
        assert_eq!(
            game.placements(board),
            first,
            "ラウンド中は配置が変わらない"
        );
        for number in 2..=game.params.max_number {
            click_circle(&mut game, number);
        }
        game.update(ROUND_INTERVAL);
        // 新しいラウンドでは色・サイズも振り直されるので、配置も作り直される
        assert_eq!(
            game.placements(board).len(),
            game.params.max_number as usize
        );
        assert_eq!(
            game.layout.borrow().as_ref().unwrap().round_serial,
            game.round_serial
        );
    }

    // --- 描画 ---

    #[test]
    fn hud_shows_mouse_only_next_number_and_lives() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("マウス専用"), "マウス専用と明記すること");
        assert!(text.contains("つぎ"));
        assert!(text.contains("♥♥♥"), "ライフ3");

        click_circle(&mut game, 1);
        let wrong = wrong_number(&game);
        click_circle(&mut game, wrong);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("♥♥♡"), "ライフが1減った表示");
    }

    #[test]
    fn board_shows_every_remaining_number_in_fallback_mode() {
        let mut game = CountManiaGame::new(Difficulty::Advanced);
        click_circle(&mut game, 1);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(!text.contains('①'), "押した円は描かれない");
        for number in 2..=20u8 {
            let digit = circle_image::circled_digit(number).unwrap();
            assert!(text.contains(digit), "{digit}が描かれていること");
        }
    }

    #[test]
    fn render_does_not_panic_in_tiny_or_interval_state() {
        let mut game = CountManiaGame::new(Difficulty::Advanced);
        rendered_text(&game, 20, 6);
        rendered_text(&game, 1, 1);
        clear_round(&mut game);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(
            text.contains("ROUND"),
            "待ち時間中は次のラウンドの案内を出す"
        );
    }

    #[test]
    fn interval_message_tells_clear_or_miss_and_hides_circles() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        clear_round(&mut game);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("CLEAR!"));
        assert!(text.contains("NEXTROUND2/3"));

        game.update(ROUND_INTERVAL);
        for _ in 0..3 {
            let wrong = wrong_number(&game);
            click_circle(&mut game, wrong);
        }
        assert!(game.interval.is_some(), "ライフ切れでも待ち時間に入る");
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("MISS..."));
        assert!(!text.contains('②'), "待ち時間中は円を描かない");
    }

    #[test]
    fn elapsed_time_does_not_advance_during_interval() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        clear_round(&mut game);
        game.update(ROUND_INTERVAL / 2);
        assert!(game.interval.is_some());
        game.update(ROUND_INTERVAL / 2);
        game.update(Duration::from_millis(700));
        clear_round(&mut game);
        let result = game.result();
        // 2ラウンド目の記録は、ラウンド開始後に進めた700msだけ
        assert!((result.avg_latency_ms - 350.0).abs() < 1e-9);
    }
}
