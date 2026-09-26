//! カウントメニアの盤面を水槽に見立てて泳ぐ魚(見た目だけの演出。当たり判定・ゲーム進行には関わらない)。
//!
//! ふだんはランダムウォークで泳ぎ、驚かされると(spook)しばらくその場から逃げる。
//! 盤面の端に当たったら跳ね返って向きを変える。座標はセル単位の連続値で、魚の左上を指す。

use std::time::Duration;

use image::{imageops, Rgba, RgbaImage};
use rand::Rng;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::layout::CELL_ASPECT;

/// ランダムウォークの強さ(セル/秒²)
pub const WANDER_ACCEL: f64 = 4.0;
/// ふだんの最高速度(横のセル/秒)
pub const MAX_SPEED: f64 = 2.5;
/// 驚いて逃げ続ける時間(秒)
pub const FLEE_DURATION: f64 = 1.2;
/// 逃げる間の加速度(セル/秒²)
pub const FLEE_ACCEL: f64 = 80.0;
/// 逃げる間の最高速度(横のセル/秒)。ふだんの上限のままでは逃げても速さが変わらないため別に持つ
pub const FLEE_MAX_SPEED: f64 = 12.0;
/// 縦方向の動きの強さ(横に対する割合)。魚は主に横へ泳ぎ、セルは縦長なので縦はゆっくりにする
const VERTICAL_SCALE: f64 = 0.35;
/// 横の速さがこれ以下の時は向きを変えない(ほぼ止まった時に左右へばたつかせないため)
const FACING_THRESHOLD: f64 = 0.05;

/// テキスト表示の魚の大きさ(セル)
pub const FISH_WIDTH: u16 = 3;
pub const FISH_HEIGHT: u16 = 1;
const TEXT_RIGHT: &str = "><>";
const TEXT_LEFT: &str = "<><";

/// 画像表示の魚のドット絵(右向き)。AQUATERMのネオンテトラ成魚を参考にした、細身で尾の割れた形。
/// W=体(白。recolorで魚の色になる)、F=ひれ(半透明の灰色。recolorで体より暗い色になる)、
/// K=目(黒のまま)、.=透明。テキスト表示の魚と同じ3x1セルに収まるよう、横:縦=3:2にしている
const SPRITE: [&str; 8] = [
    "....FF......",
    "...FFFF.....",
    "F.WWWWWWWW..",
    "FFWWWWWWWWW.",
    "FFWWWWWWWKWW",
    "FFWWWWWWWWW.",
    "F.WWWWWWWW..",
    "....FFF.....",
];
pub const SPRITE_WIDTH: u32 = 12;
pub const SPRITE_HEIGHT: u32 = SPRITE.len() as u32;
const SPRITE_BODY: Rgba<u8> = Rgba([255, 255, 255, 255]);
const SPRITE_FIN: Rgba<u8> = Rgba([150, 150, 150, 190]);
const SPRITE_EYE: Rgba<u8> = Rgba([0, 0, 0, 255]);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fish {
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub facing_right: bool,
    /// 逃げ続ける残り時間(秒)。0ならふだんの泳ぎ
    pub flee_timer: f64,
    /// 逃げる向き(長さ1。縦は見た目の距離で測った向き)
    pub flee_dx: f64,
    pub flee_dy: f64,
}

impl Fish {
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            facing_right: true,
            flee_timer: 0.0,
            flee_dx: 0.0,
            flee_dy: 0.0,
        }
    }

    /// bounds内のランダムな位置に、ランダムな向きでゆっくり泳ぎ出す魚を置く
    pub fn random(rng: &mut impl Rng, bounds: Rect) -> Self {
        let (min_x, max_x, min_y, max_y) = position_range(bounds);
        let mut fish = Self::new(rng.gen_range(min_x..=max_x), rng.gen_range(min_y..=max_y));
        fish.facing_right = rng.gen_bool(0.5);
        let speed = rng.gen_range(0.3..=1.0) * MAX_SPEED;
        fish.vx = if fish.facing_right { speed } else { -speed };
        fish
    }

    /// 魚の中心(セル単位)
    pub fn center(&self) -> (f64, f64) {
        (
            self.x + f64::from(FISH_WIDTH) / 2.0,
            self.y + f64::from(FISH_HEIGHT) / 2.0,
        )
    }

    pub fn update(&mut self, dt: Duration, bounds: Rect, rng: &mut impl Rng) {
        let dt = dt.as_secs_f64();
        if dt <= 0.0 || bounds.is_empty() {
            return;
        }
        self.vx += rng.gen_range(-1.0..=1.0) * WANDER_ACCEL * dt;
        self.vy += rng.gen_range(-1.0..=1.0) * WANDER_ACCEL * VERTICAL_SCALE * dt;
        let limit = if self.flee_timer > 0.0 {
            self.vx += self.flee_dx * FLEE_ACCEL * dt;
            self.vy += self.flee_dy * FLEE_ACCEL * VERTICAL_SCALE * dt;
            self.flee_timer = (self.flee_timer - dt).max(0.0);
            FLEE_MAX_SPEED
        } else {
            MAX_SPEED
        };
        self.vx = self.vx.clamp(-limit, limit);
        let vertical_limit = limit * VERTICAL_SCALE;
        self.vy = self.vy.clamp(-vertical_limit, vertical_limit);
        self.x += self.vx * dt;
        self.y += self.vy * dt;
        self.bounce(bounds);
        if self.vx.abs() > FACING_THRESHOLD {
            self.facing_right = self.vx > 0.0;
        }
    }

    /// boundsの端を越えていたら端に戻し、速度(と逃げる向き)を内側へ反転する
    fn bounce(&mut self, bounds: Rect) {
        let (min_x, max_x, min_y, max_y) = position_range(bounds);
        if self.x < min_x {
            self.x = min_x;
            self.vx = self.vx.abs();
            self.flee_dx = self.flee_dx.abs();
        } else if self.x > max_x {
            self.x = max_x;
            self.vx = -self.vx.abs();
            self.flee_dx = -self.flee_dx.abs();
        }
        if self.y < min_y {
            self.y = min_y;
            self.vy = self.vy.abs();
            self.flee_dy = self.flee_dy.abs();
        } else if self.y > max_y {
            self.y = max_y;
            self.vy = -self.vy.abs();
            self.flee_dy = -self.flee_dy.abs();
        }
    }

    /// from(クリック位置)から離れる向きへ、FLEE_DURATIONのあいだ逃げさせる。
    /// 逃げている最中でも、向きは新しいクリック位置から測り直す
    pub fn spook(&mut self, from: (f64, f64)) {
        let (cx, cy) = self.center();
        let dx = cx - from.0;
        let dy = (cy - from.1) * CELL_ASPECT;
        let length = dx.hypot(dy);
        let (dx, dy) = if length > f64::EPSILON {
            (dx / length, dy / length)
        } else if self.facing_right {
            (1.0, 0.0)
        } else {
            (-1.0, 0.0)
        };
        self.flee_dx = dx;
        self.flee_dy = dy;
        self.flee_timer = FLEE_DURATION;
    }
}

/// 魚の左上が取りうる範囲(最小x, 最大x, 最小y, 最大y)。魚より狭い盤面では左上に寄せる
fn position_range(bounds: Rect) -> (f64, f64, f64, f64) {
    let min_x = f64::from(bounds.x);
    let min_y = f64::from(bounds.y);
    let max_x = f64::from(bounds.right().saturating_sub(FISH_WIDTH)).max(min_x);
    let max_y = f64::from(bounds.bottom().saturating_sub(FISH_HEIGHT)).max(min_y);
    (min_x, max_x, min_y, max_y)
}

/// 画像表示の魚のドット絵(色を付ける前の白・灰・黒)。facing_rightがfalseなら左右反転する
pub fn sprite(facing_right: bool) -> RgbaImage {
    let mut image = RgbaImage::new(SPRITE_WIDTH, SPRITE_HEIGHT);
    for (y, row) in SPRITE.iter().enumerate() {
        for (x, dot) in row.chars().enumerate() {
            let color = match dot {
                'W' => SPRITE_BODY,
                'F' => SPRITE_FIN,
                'K' => SPRITE_EYE,
                _ => continue,
            };
            image.put_pixel(x as u32, y as u32, color);
        }
    }
    if facing_right {
        image
    } else {
        imageops::flip_horizontal(&image)
    }
}

/// 画像プロトコル非対応の端末向けの魚("><>"/"<><")。board・バッファの外にはみ出す部分は描かない。
/// 背景色は変えず、文字と文字色だけを置く(焦らせる赤い背景の上でもそのまま泳がせるため)
pub fn render_text(buf: &mut Buffer, board: Rect, fish: &Fish, color: [u8; 3]) {
    let area = board.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    let y = fish.y.round();
    if y < f64::from(area.y) || y >= f64::from(area.bottom()) {
        return;
    }
    let text = if fish.facing_right {
        TEXT_RIGHT
    } else {
        TEXT_LEFT
    };
    let [r, g, b] = color;
    let left = fish.x.round();
    for (offset, ch) in text.chars().enumerate() {
        let x = left + offset as f64;
        if x < f64::from(area.x) || x >= f64::from(area.right()) {
            continue;
        }
        buf[(x as u16, y as u16)]
            .set_char(ch)
            .set_fg(Color::Rgb(r, g, b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    const BOUNDS: Rect = Rect::new(2, 3, 60, 20);
    const TICK: Duration = Duration::from_millis(33);

    fn inside(fish: &Fish, bounds: Rect) -> bool {
        fish.x >= f64::from(bounds.x)
            && fish.x <= f64::from(bounds.right() - FISH_WIDTH)
            && fish.y >= f64::from(bounds.y)
            && fish.y <= f64::from(bounds.bottom() - FISH_HEIGHT)
    }

    fn speed(fish: &Fish) -> f64 {
        fish.vx.hypot(fish.vy)
    }

    #[test]
    fn new_fish_is_at_rest_and_not_fleeing() {
        let fish = Fish::new(10.0, 5.0);
        assert_eq!((fish.x, fish.y), (10.0, 5.0));
        assert_eq!((fish.vx, fish.vy), (0.0, 0.0));
        assert_eq!(fish.flee_timer, 0.0);
    }

    #[test]
    fn random_fish_is_inside_bounds_even_for_tiny_bounds() {
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..100 {
            assert!(inside(&Fish::random(&mut rng, BOUNDS), BOUNDS));
        }
        // 魚より狭い盤面でもpanicせず、左上に寄せて置く
        let tiny = Rect::new(4, 4, 1, 1);
        let fish = Fish::random(&mut rng, tiny);
        assert_eq!((fish.x, fish.y), (4.0, 4.0));
    }

    #[test]
    fn wandering_for_seconds_stays_inside_bounds() {
        for seed in 0..20 {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut fish = Fish::random(&mut rng, BOUNDS);
            for step in 0..600 {
                // 時々驚かせて、速く泳ぐ時も壁を越えないことを確かめる
                if step % 97 == 0 {
                    fish.spook((f64::from(BOUNDS.x), f64::from(BOUNDS.y)));
                }
                fish.update(TICK, BOUNDS, &mut rng);
                assert!(inside(&fish, BOUNDS), "seed={seed} step={step}: {fish:?}");
            }
        }
    }

    #[test]
    fn hitting_right_wall_bounces_and_turns_left() {
        let mut rng = StdRng::seed_from_u64(2);
        let mut fish = Fish::new(f64::from(BOUNDS.right() - FISH_WIDTH) - 0.01, 10.0);
        fish.vx = MAX_SPEED;
        fish.facing_right = true;
        fish.update(TICK, BOUNDS, &mut rng);
        assert!(fish.vx < 0.0, "跳ね返って左へ泳ぐ: {fish:?}");
        assert!(!fish.facing_right);
        assert!(inside(&fish, BOUNDS));
    }

    #[test]
    fn hitting_left_wall_bounces_and_turns_right() {
        let mut rng = StdRng::seed_from_u64(3);
        let mut fish = Fish::new(f64::from(BOUNDS.x) + 0.01, 10.0);
        fish.vx = -MAX_SPEED;
        fish.facing_right = false;
        fish.update(TICK, BOUNDS, &mut rng);
        assert!(fish.vx > 0.0, "跳ね返って右へ泳ぐ: {fish:?}");
        assert!(fish.facing_right);
    }

    #[test]
    fn spook_sets_full_timer_and_unit_direction_away_from_click() {
        let mut fish = Fish::new(20.0, 10.0);
        let click = (15.0, 12.0);
        fish.spook(click);
        assert_eq!(fish.flee_timer, FLEE_DURATION);
        let length = fish.flee_dx.hypot(fish.flee_dy);
        assert!((length - 1.0).abs() < 1e-9, "長さ1: {length}");
        let (cx, cy) = fish.center();
        let away = (cx - click.0) * fish.flee_dx + (cy - click.1) * fish.flee_dy;
        assert!(away > 0.0, "クリック位置の反対側へ逃げる");
    }

    #[test]
    fn spook_at_fish_center_still_gives_unit_direction() {
        let mut fish = Fish::new(20.0, 10.0);
        fish.spook(fish.center());
        let length = fish.flee_dx.hypot(fish.flee_dy);
        assert!((length - 1.0).abs() < 1e-9);
    }

    #[test]
    fn spook_while_fleeing_extends_timer_and_overwrites_direction() {
        let mut rng = StdRng::seed_from_u64(4);
        let mut fish = Fish::new(30.0, 10.0);
        fish.spook((25.0, 10.5));
        assert!(fish.flee_dx > 0.0);
        fish.update(Duration::from_millis(500), BOUNDS, &mut rng);
        assert!(fish.flee_timer < FLEE_DURATION);
        let (cx, cy) = fish.center();
        fish.spook((cx + 5.0, cy));
        assert_eq!(fish.flee_timer, FLEE_DURATION);
        assert!(
            fish.flee_dx < 0.0,
            "新しいクリック位置から離れる向きに変わる"
        );
    }

    #[test]
    fn update_right_after_spook_moves_farther_than_normal() {
        let start = Fish::new(30.0, 10.0);
        let mut calm = start;
        let mut spooked = start;
        spooked.spook((25.0, 10.5));
        // 同じ乱数列で動かし、差が逃走の加速だけから来るようにする
        let dt = Duration::from_millis(200);
        calm.update(dt, BOUNDS, &mut StdRng::seed_from_u64(5));
        spooked.update(dt, BOUNDS, &mut StdRng::seed_from_u64(5));
        let moved = |fish: &Fish| (fish.x - start.x).hypot(fish.y - start.y);
        assert!(
            moved(&spooked) > moved(&calm) * 3.0,
            "逃げる方が大きく動く: spooked={} calm={}",
            moved(&spooked),
            moved(&calm)
        );
        assert!(spooked.flee_timer < FLEE_DURATION, "逃げる残り時間が減る");
    }

    #[test]
    fn after_flee_timer_runs_out_fish_returns_to_normal_speed() {
        let mut rng = StdRng::seed_from_u64(6);
        let mut fish = Fish::new(30.0, 10.0);
        fish.spook((25.0, 10.5));
        let mut peak: f64 = 0.0;
        let mut elapsed = 0.0;
        while elapsed < FLEE_DURATION + 0.1 {
            fish.update(TICK, BOUNDS, &mut rng);
            peak = peak.max(speed(&fish));
            elapsed += TICK.as_secs_f64();
        }
        assert_eq!(fish.flee_timer, 0.0);
        assert!(peak > MAX_SPEED, "逃げている間はふだんより速い: {peak}");
        for _ in 0..30 {
            fish.update(TICK, BOUNDS, &mut rng);
            assert_eq!(fish.flee_timer, 0.0);
            assert!(fish.vx.abs() <= MAX_SPEED + 1e-9, "{fish:?}");
            assert!(fish.vy.abs() <= MAX_SPEED + 1e-9, "{fish:?}");
        }
    }

    #[test]
    fn update_with_empty_bounds_does_not_panic() {
        let mut rng = StdRng::seed_from_u64(7);
        let mut fish = Fish::new(3.0, 3.0);
        fish.update(TICK, Rect::default(), &mut rng);
        fish.update(TICK, Rect::new(3, 3, 1, 1), &mut rng);
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (buf.area.x..buf.area.right())
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn render_text_draws_fish_facing_its_direction() {
        let area = Rect::new(0, 0, 20, 4);
        let mut buf = Buffer::empty(area);
        let mut fish = Fish::new(4.2, 1.4);
        fish.facing_right = true;
        render_text(&mut buf, area, &fish, [10, 20, 30]);
        assert_eq!(&row_text(&buf, 1)[4..7], "><>");
        assert_eq!(buf[(4, 1)].fg, Color::Rgb(10, 20, 30));

        let mut buf = Buffer::empty(area);
        fish.facing_right = false;
        render_text(&mut buf, area, &fish, [10, 20, 30]);
        assert_eq!(&row_text(&buf, 1)[4..7], "<><");
    }

    #[test]
    fn render_text_keeps_background_color() {
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        buf[(2, 0)].set_bg(Color::Rgb(100, 0, 0));
        render_text(&mut buf, area, &Fish::new(2.0, 0.0), [1, 2, 3]);
        assert_eq!(buf[(2, 0)].bg, Color::Rgb(100, 0, 0));
    }

    #[test]
    fn render_text_clips_to_board_and_buffer() {
        let area = Rect::new(0, 0, 6, 2);
        let mut buf = Buffer::empty(area);
        let board = Rect::new(1, 0, 4, 2);
        // 盤面の右端からはみ出す部分は描かない
        render_text(&mut buf, board, &Fish::new(3.0, 0.0), [1, 2, 3]);
        assert_eq!(row_text(&buf, 0), "   >< ");
        // バッファの外にある魚・盤面がバッファより大きくてもpanicしない
        render_text(
            &mut buf,
            Rect::new(0, 0, 50, 50),
            &Fish::new(40.0, 30.0),
            [1, 2, 3],
        );
        render_text(&mut buf, board, &Fish::new(-5.0, 0.0), [1, 2, 3]);
    }

    // --- 画像表示のドット絵 ---

    /// 不透明で、RGBがrgbのピクセルの数
    fn count_rgb(image: &RgbaImage, rgb: [u8; 3]) -> usize {
        image
            .pixels()
            .filter(|p| p.0[3] > 0 && p.0[..3] == rgb)
            .count()
    }

    #[test]
    fn sprite_has_white_body_gray_fins_black_eye_and_clear_background() {
        let image = sprite(true);
        assert_eq!(image.dimensions(), (SPRITE_WIDTH, SPRITE_HEIGHT));
        assert!((6..=8).contains(&SPRITE_HEIGHT), "6〜8行のドット絵");
        assert!(
            count_rgb(&image, [255, 255, 255]) >= 30,
            "体は白(recolorで魚の色になる)"
        );
        assert_eq!(count_rgb(&image, [0, 0, 0]), 1, "目は黒1ドット");
        let fins = image
            .pixels()
            .filter(|p| p.0[3] > 0 && p.0[0] > 50 && p.0[0] < 200)
            .count();
        assert!(fins > 0, "ひれは白と黒の間の灰色(体より暗い色になる)");
        assert!(image.pixels().any(|p| p.0[3] == 0), "周りは透明");
    }

    #[test]
    fn sprite_faces_its_direction() {
        let eye_x = |image: &RgbaImage| {
            image
                .enumerate_pixels()
                .find(|(_, _, p)| p.0[3] > 0 && p.0[..3] == [0, 0, 0])
                .map(|(x, _, _)| x)
                .unwrap()
        };
        assert!(eye_x(&sprite(true)) > SPRITE_WIDTH / 2, "右向きは目が右");
        assert!(eye_x(&sprite(false)) < SPRITE_WIDTH / 2, "左向きは目が左");
        assert_eq!(
            sprite(false),
            image::imageops::flip_horizontal(&sprite(true)),
            "左向きは右向きの左右反転"
        );
    }
}
