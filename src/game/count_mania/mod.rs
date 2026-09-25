//! カウントマニア: ランダムに並んだ1〜Nの数字付き円を、1から順にクリックしていくマウス専用ゲーム。
//!
//! 1セッション=3ラウンド。ラウンドごとに「全部押せたか(クリア)/ライフが尽きたか(失敗)」を
//! ScoreTrackerに1件として記録する。キー入力は受け付けない。

mod circle_image;
mod layout;
mod ripple;

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
use ripple::Ripple;

pub const GAME_ID: &str = "count_mania";

/// 1セッションのラウンド数
pub const ROUNDS_PER_SESSION: u32 = 3;

/// ラウンド終了から次のラウンド開始までの間隔。この間はクリックを受け付けない
/// (前のラウンドの最後のクリックが次の盤面に当たらないようにするため)
pub const ROUND_INTERVAL: Duration = Duration::from_millis(1200);

/// 次に押すべき数字がこの時間を超えても押されないと、盤面の背景を赤く明滅させて焦らせる
pub const PRESSURE_THRESHOLD: Duration = Duration::from_secs(10);

/// 背景の明滅1回(暗い→明るい→暗い)の周期
pub const PRESSURE_PERIOD: Duration = Duration::from_millis(800);

/// 明滅する背景の赤の強さの範囲(暗い時, 明るい時)。
/// 明るい時でも、円の色・数字が背景に埋もれない暗さに抑える
const PRESSURE_RED_RANGE: (f64, f64) = (40.0, 150.0);

/// 背景の緑・青の強さ(赤に対する割合)。純粋な赤より少しだけ温かみを持たせる
const PRESSURE_GREEN_BLUE_RATIO: f64 = 0.12;

/// 次に押すべき数字になってからの経過時間に対する盤面の背景色。
/// 閾値以内はNone(通常の背景のまま)。閾値を超えたら、超えた分の時間で赤の明るさを
/// 周期的に変え、パトランプのように明滅させる。超えた直後は暗い側から始める
fn pressure_background(time_since_target: Duration) -> Option<Color> {
    let over = time_since_target.checked_sub(PRESSURE_THRESHOLD)?;
    if over.is_zero() {
        return None;
    }
    let phase = over.as_secs_f64() / PRESSURE_PERIOD.as_secs_f64() * std::f64::consts::TAU;
    // 0(暗い)〜1(明るい)を行き来する
    let level = (1.0 - phase.cos()) / 2.0;
    let (low, high) = PRESSURE_RED_RANGE;
    let red = low + (high - low) * level;
    let other = (red * PRESSURE_GREEN_BLUE_RATIO).round() as u8;
    Some(Color::Rgb(red.round() as u8, other, other))
}

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
    use CircleSize::{Huge, Large, Medium, Small};
    match difficulty {
        Difficulty::Beginner => DifficultyParams {
            max_number: 10,
            size_levels: &[Huge, Large, Medium],
            lives: 3,
            dense: false,
            fail_latency_ms: 30_000.0,
        },
        Difficulty::Intermediate => DifficultyParams {
            max_number: 14,
            size_levels: &[Huge, Large, Medium, Small],
            lives: 2,
            dense: false,
            fail_latency_ms: 45_000.0,
        },
        Difficulty::Advanced => DifficultyParams {
            max_number: 20,
            size_levels: &[Huge, Large, Medium, Small],
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
    /// いまの数字が次に押すべき数字になってからの経過時間(プレッシャー背景に使う)
    time_since_target: Duration,
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
        time_since_target: Duration::ZERO,
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
    /// 正解クリックの位置から広がる波紋(見た目だけの演出)。Noneなら表示していない
    ripple: Option<Ripple>,
    layout: RefCell<Option<LayoutCache>>,
    renderer: CircleRenderer,
    /// ライフが尽きた(GAME OVER)か。trueになったら残りラウンドを待たずセッションを終える
    game_over: bool,
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
            ripple: None,
            layout: RefCell::new(None),
            renderer: CircleRenderer::new(),
            game_over: false,
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

    /// 正解の円をクリックした。(column, row)はクリックしたセルで、そこから波紋を広げる
    fn click_correct(&mut self, column: u16, row: u16) {
        audio::play_se(SeKind::Correct);
        // 前の波紋が残っていても、新しい波紋に置き換える
        self.ripple = Some(Ripple::new(column, row));
        self.round.next += 1;
        // 次の数字に進んだので、焦らせる背景は最初からやり直す
        self.round.time_since_target = Duration::ZERO;
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
            // GAME OVERで焦らせる背景を止める
            self.round.time_since_target = Duration::ZERO;
            self.tracker.record(false, self.params.fail_latency_ms);
            self.feedback.record(false, "ライフが尽きた");
            // GAME OVERは残りラウンドを待たずセッションを即終了する(次のラウンドへの
            // 待ち時間には入らない。フィードバック表示が消えたらis_finished()がtrueになる)
            self.game_over = true;
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
    ///
    /// ここに来るのは最後の数字まで押し切ってクリアした時のみ(ライフ切れはGAME OVERとして
    /// セッションを即終了するため、この待ち時間には入らない)
    fn render_interval_message(&self, frame: &mut Frame, board: Rect) {
        let next_round = (self.tracker.total() + 1).min(ROUNDS_PER_SESSION);
        let lines = vec![
            Line::from(Span::styled(
                "CLEAR!",
                Style::default()
                    .fg(theme::CORRECT)
                    .add_modifier(Modifier::BOLD),
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
        // 待ち時間中・セッション終了後・GAME OVER後のクリックは受け付けない
        if self.is_finished() || self.interval.is_some() || self.game_over {
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
            self.click_correct(mouse.column, mouse.row);
        } else {
            self.click_wrong();
        }
    }

    fn update(&mut self, dt: Duration) {
        self.feedback.tick(dt);
        // 波紋はラウンド間の待ち時間中も時間を進め、持続時間を過ぎたら消す
        self.ripple = self.ripple.and_then(|ripple| ripple.advanced(dt));
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
            self.round.time_since_target += dt;
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let (hud, body) = theme::split_hud(area);
        self.render_hud(frame, hud);

        let mut block = theme::focus_panel(" 1から順にクリック ", self.feedback.current());
        // 次の数字がなかなか押されない時は、パネルの背景を赤く明滅させて焦らせる
        if let Some(bg) = pressure_background(self.round.time_since_target) {
            block = block.style(Style::default().bg(bg));
        }
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
        self.renderer
            .render_board(frame, board, &circles, self.ripple.as_ref());
    }

    fn is_finished(&self) -> bool {
        if self.game_over {
            // GAME OVERの正誤フィードバック("ライフが尽きた")が消えたらセッション終了
            return self.feedback.current().is_none();
        }
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
        assert_eq!(beginner.size_levels, &[Huge, Large, Medium]);
        assert_eq!(beginner.lives, 3);
        assert!(!beginner.dense);

        let intermediate = params(Difficulty::Intermediate);
        assert_eq!(intermediate.max_number, 14);
        assert_eq!(intermediate.size_levels, &[Huge, Large, Medium, Small]);
        assert_eq!(intermediate.lives, 2);
        assert!(!intermediate.dense);

        let advanced = params(Difficulty::Advanced);
        assert_eq!(advanced.max_number, 20);
        assert_eq!(advanced.size_levels, &[Huge, Large, Medium, Small]);
        assert_eq!(advanced.lives, 2);
        assert!(advanced.dense, "上級は密集配置");
    }

    #[test]
    fn every_difficulty_uses_huge_circles() {
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            assert!(
                params(difficulty).size_levels.contains(&CircleSize::Huge),
                "{difficulty:?}: 特大の円を使う"
            );
        }
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

    #[test]
    fn game_over_ends_the_session_without_waiting_for_remaining_rounds() {
        // GAME OVER(ライフ0)は、3ラウンド構成の途中でも次のラウンドへ進まず、
        // フィードバック("ライフが尽きた")が消えたらセッション全体が終了する
        let mut game = CountManiaGame::new(Difficulty::Intermediate);
        click_circle(&mut game, 1);
        for _ in 0..2 {
            let wrong = wrong_number(&game);
            click_circle(&mut game, wrong);
        }
        assert_eq!(game.tracker.total(), 1, "3ラウンドのうち1回しか記録されない");
        assert!(!game.is_finished(), "フィードバック表示中はまだ終了しない");
        assert!(game.interval.is_none(), "次のラウンドへの待ち時間には入らない");

        game.update(crate::game::feedback::FEEDBACK_HOLD);
        assert!(game.is_finished(), "フィードバックが消えたらセッション終了");
        assert_eq!(
            game.tracker.total(),
            1,
            "次のラウンドは始まらないので記録は1回のまま"
        );
    }

    #[test]
    fn clicks_after_game_over_are_ignored() {
        let mut game = CountManiaGame::new(Difficulty::Intermediate);
        click_circle(&mut game, 1);
        for _ in 0..2 {
            let wrong = wrong_number(&game);
            click_circle(&mut game, wrong);
        }
        let next_before = game.round.next;
        click_circle(&mut game, next_before);
        assert_eq!(game.tracker.total(), 1, "GAME OVER後のクリックは無視される");
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
    fn interval_message_tells_clear_and_hides_circles() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        clear_round(&mut game);
        let text = rendered_text(&game, AREA.width, AREA.height);
        assert!(text.contains("CLEAR!"));
        assert!(text.contains("NEXTROUND2/3"));
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

    // --- 波紋 ---

    /// 円1・円2が重ならない配置にし、円1の内側のセル(ボード左上からの相対位置)を返す
    fn set_separate_layout(game: &CountManiaGame) -> (u16, u16) {
        set_layout(game, &[(1, 2, 2, 10, 5), (2, 40, 2, 10, 5)]);
        (6, 4)
    }

    #[test]
    fn correct_click_starts_ripple_at_clicked_cell() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let (x, y) = set_separate_layout(&game);
        assert!(game.ripple.is_none(), "始めは波紋なし");
        click_board(&mut game, x, y);
        assert_eq!(game.round.next, 2);
        let board = board_area(AREA);
        let ripple = game.ripple.expect("正解クリックで波紋が始まる");
        assert_eq!(
            ripple.center(),
            (board.x + x, board.y + y),
            "クリックした位置が中心"
        );
        assert_eq!(ripple.progress(), 0.0);
    }

    #[test]
    fn wrong_or_empty_click_does_not_start_ripple() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        set_separate_layout(&game);
        // 円2(次に押すべきでない円)をクリック
        click_board(&mut game, 44, 4);
        assert_eq!(game.round.lives, 2, "不正解");
        assert!(game.ripple.is_none(), "不正解では波紋を出さない");
        // 円の無い所をクリック
        click_board(&mut game, 25, 4);
        assert!(game.ripple.is_none(), "空白のクリックでも波紋を出さない");
    }

    #[test]
    fn update_advances_ripple_and_removes_it_after_duration() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let (x, y) = set_separate_layout(&game);
        click_board(&mut game, x, y);
        game.update(ripple::RIPPLE_DURATION / 2);
        let ripple = game.ripple.expect("持続時間内は残る");
        assert!((ripple.progress() - 0.5).abs() < 1e-9, "経過時間が進む");
        game.update(ripple::RIPPLE_DURATION / 2);
        assert!(game.ripple.is_none(), "持続時間を過ぎたら消える");
    }

    #[test]
    fn new_correct_click_replaces_previous_ripple() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let (x, y) = set_separate_layout(&game);
        click_board(&mut game, x, y);
        game.update(ripple::RIPPLE_DURATION / 2);
        // 円2をクリック(正解)
        click_board(&mut game, 44, 4);
        let board = board_area(AREA);
        let ripple = game.ripple.unwrap();
        assert_eq!(
            ripple.center(),
            (board.x + 44, board.y + 4),
            "新しい位置に置き換わる"
        );
        assert_eq!(ripple.progress(), 0.0, "最初からやり直す");
    }

    #[test]
    fn ripple_expires_during_round_interval_without_affecting_score() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        clear_round(&mut game);
        assert!(game.ripple.is_some(), "最後の正解クリックでも波紋は始まる");
        assert!(game.interval.is_some());
        game.update(ripple::RIPPLE_DURATION);
        assert!(game.ripple.is_none(), "待ち時間中も波紋の時間は進む");
        let result = game.result();
        assert_eq!((result.total, result.correct), (1, 1), "記録は波紋と無関係");
    }

    #[test]
    fn fallback_render_ignores_ripple() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let (x, y) = set_separate_layout(&game);
        click_board(&mut game, x, y);
        assert!(game.ripple.is_some());
        let with_ripple = rendered_text(&game, AREA.width, AREA.height);
        game.ripple = None;
        assert_eq!(rendered_text(&game, AREA.width, AREA.height), with_ripple);
    }

    #[test]
    fn image_mode_render_with_ripple_does_not_panic() {
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut game = CountManiaGame::new(Difficulty::Advanced);
        let mut picker = Picker::from_fontsize((4, 8));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        game.renderer = CircleRenderer::with_picker(picker);
        click_circle(&mut game, 1);
        assert!(game.ripple.is_some());
        for _ in 0..4 {
            rendered_text(&game, AREA.width, AREA.height);
            game.update(ripple::RIPPLE_DURATION / 3);
        }
        assert!(game.ripple.is_none());
        rendered_text(&game, AREA.width, AREA.height);
        click_circle(&mut game, 2);
        rendered_text(&game, 20, 6);
        rendered_text(&game, 1, 1);
    }

    #[test]
    fn ripple_animation_does_not_reencode_whole_board_every_tick() {
        // 実際のゲームループと同じく、tickごとにupdateして描く
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut game = CountManiaGame::new(Difficulty::Advanced);
        let mut picker = Picker::from_fontsize((4, 8));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        game.renderer = CircleRenderer::with_picker(picker);
        rendered_text(&game, AREA.width, AREA.height);
        click_circle(&mut game, 1);
        rendered_text(&game, AREA.width, AREA.height);
        let boards = game.renderer.board_encode_count();
        let patches = game.renderer.ripple_encode_count();
        let mut ticks = 0;
        while game.ripple.is_some() {
            game.update(crate::TICK_RATE);
            rendered_text(&game, AREA.width, AREA.height);
            ticks += 1;
        }
        assert_eq!(
            game.renderer.board_encode_count(),
            boards,
            "波紋のアニメーション中は盤面全体を作り直さない"
        );
        let redrawn = game.renderer.ripple_encode_count() - patches;
        let frames = (ripple::RIPPLE_DURATION.as_millis()
            / ripple::RIPPLE_FRAME_INTERVAL.as_millis()) as usize;
        // コマの切り替わり(最初のコマは描画済み) + 消えた後のリング無しの描き直し1回
        assert!(
            redrawn <= frames,
            "パッチの作り直しはコマの数まで: {redrawn}"
        );
        assert!(redrawn < ticks, "毎tickは作り直さない: {redrawn}/{ticks}");
    }

    // --- プレッシャー背景 ---

    /// 画面を描画し、バッファを返す
    fn rendered_buffer(game: &CountManiaGame) -> ratatui::buffer::Buffer {
        let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| game.render(frame, frame.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// 盤面の円の無いセルの背景色
    fn board_background(game: &CountManiaGame) -> Color {
        let (column, row) = empty_cell(game);
        rendered_buffer(game)[(column, row)].bg
    }

    /// 閾値をsecs秒超えた時点の背景色
    fn background_after_threshold(secs: f64) -> Color {
        pressure_background(PRESSURE_THRESHOLD + Duration::from_secs_f64(secs))
            .expect("閾値を超えたら背景色が付く")
    }

    fn rgb(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("RGBの色であること: {other:?}"),
        }
    }

    #[test]
    fn new_round_starts_with_zero_time_since_target() {
        let mut rng = StdRng::seed_from_u64(1);
        let round = new_round(&mut rng, &params(Difficulty::Beginner));
        assert!(round.time_since_target.is_zero());
        let game = CountManiaGame::new(Difficulty::Beginner);
        assert!(game.round.time_since_target.is_zero());
    }

    #[test]
    fn update_advances_time_since_target_only_while_playing() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        game.update(Duration::from_millis(1500));
        game.update(Duration::from_millis(500));
        assert_eq!(game.round.time_since_target, Duration::from_secs(2));

        clear_round(&mut game);
        assert!(game.interval.is_some());
        game.update(ROUND_INTERVAL / 2);
        assert!(
            game.round.time_since_target.is_zero(),
            "ラウンド間の待ち時間中は進まない"
        );
        game.update(ROUND_INTERVAL / 2);
        assert!(game.interval.is_none());
        assert!(
            game.round.time_since_target.is_zero(),
            "新しいラウンドは0から"
        );
    }

    #[test]
    fn time_since_target_does_not_advance_after_session_finished() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        for _ in 0..ROUNDS_PER_SESSION {
            clear_round(&mut game);
            game.update(ROUND_INTERVAL);
        }
        assert!(game.is_finished());
        game.update(PRESSURE_THRESHOLD * 2);
        assert!(game.round.time_since_target.is_zero());
    }

    #[test]
    fn correct_click_resets_time_since_target() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        game.update(Duration::from_secs(12));
        click_circle(&mut game, 1);
        assert_eq!(game.round.next, 2);
        assert!(game.round.time_since_target.is_zero());
        assert_eq!(
            game.round.elapsed,
            Duration::from_secs(12),
            "ラウンドの経過時間はリセットしない"
        );
    }

    #[test]
    fn wrong_click_keeps_time_since_target_while_lives_remain() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        game.update(Duration::from_secs(12));
        let wrong = wrong_number(&game);
        click_circle(&mut game, wrong);
        assert_eq!(game.round.lives, 2);
        assert_eq!(game.round.time_since_target, Duration::from_secs(12));
    }

    #[test]
    fn game_over_resets_time_since_target() {
        let mut game = CountManiaGame::new(Difficulty::Intermediate);
        game.update(Duration::from_secs(12));
        for _ in 0..2 {
            let wrong = wrong_number(&game);
            click_circle(&mut game, wrong);
        }
        assert_eq!(game.round.lives, 0);
        assert!(game.round.time_since_target.is_zero());
    }

    #[test]
    fn pressure_background_is_none_up_to_threshold() {
        assert_eq!(pressure_background(Duration::ZERO), None);
        assert_eq!(pressure_background(Duration::from_millis(9_999)), None);
        assert_eq!(pressure_background(PRESSURE_THRESHOLD), None);
    }

    #[test]
    fn pressure_background_is_reddish_after_threshold() {
        for step in 1..=40 {
            let (r, g, b) = rgb(background_after_threshold(f64::from(step) * 0.05));
            assert!(r > g && r > b, "赤系であること: {r},{g},{b}");
            assert!(r >= 30, "背景が分かる程度の赤であること: {r}");
            assert!(
                r <= 180,
                "円・数字が読める程度に暗いこと(明るすぎない): {r}"
            );
        }
    }

    #[test]
    fn pressure_background_brightness_pulses_periodically() {
        let dark = rgb(background_after_threshold(0.001)).0;
        let bright = rgb(background_after_threshold(PRESSURE_PERIOD.as_secs_f64() / 2.0)).0;
        assert!(
            bright > dark + 50,
            "時間経過で明るさが変わる: {dark} -> {bright}"
        );
        let later = rgb(background_after_threshold(
            PRESSURE_PERIOD.as_secs_f64() * 3.0 + 0.001,
        ))
        .0;
        assert!(
            later.abs_diff(dark) <= 2,
            "周期ごとに同じ明るさに戻る: {dark} / {later}"
        );
    }

    #[test]
    fn board_background_stays_normal_within_threshold() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        let normal = board_background(&game);
        assert_eq!(normal, Color::Reset);
        game.update(Duration::from_millis(9_900));
        assert_eq!(board_background(&game), normal, "10秒以内は変えない");
    }

    #[test]
    fn board_background_flashes_red_after_threshold_and_resets_on_correct_click() {
        let mut game = CountManiaGame::new(Difficulty::Beginner);
        game.update(PRESSURE_THRESHOLD + PRESSURE_PERIOD / 2);
        let (r, g, b) = rgb(board_background(&game));
        assert!(r > g && r > b, "盤面の背景が赤系になる: {r},{g},{b}");
        // 枠の上もパネルの背景として同じ色になる
        let buffer = rendered_buffer(&game);
        let (_, body) = theme::split_hud(AREA);
        assert_eq!(buffer[(body.x, body.y)].bg, Color::Rgb(r, g, b));

        game.update(PRESSURE_PERIOD / 4);
        assert_ne!(
            rgb(board_background(&game)),
            (r, g, b),
            "時間とともに色が変わる"
        );

        click_circle(&mut game, 1);
        assert_eq!(
            board_background(&game),
            Color::Reset,
            "正解クリックで元に戻る"
        );
    }

    #[test]
    fn board_background_resets_on_game_over() {
        let mut game = CountManiaGame::new(Difficulty::Intermediate);
        game.update(PRESSURE_THRESHOLD * 2);
        for _ in 0..2 {
            let wrong = wrong_number(&game);
            click_circle(&mut game, wrong);
        }
        assert!(game.game_over, "ライフ切れでGAME OVERになる");
        let buffer = rendered_buffer(&game);
        let (_, body) = theme::split_hud(AREA);
        assert_eq!(
            buffer[(body.x, body.y)].bg,
            Color::Reset,
            "GAME OVERで元に戻る"
        );
    }
}
