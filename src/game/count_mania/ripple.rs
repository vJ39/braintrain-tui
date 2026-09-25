//! カウントマニアの正解クリック時の波紋エフェクト。
//!
//! 正解の円をクリックした位置(セル座標)を中心に、リングが広がりながら薄くなって消える。
//! ここでは波紋の状態(中心と経過時間)と、ある時点のリングを描いたRGBA画像の生成を持つ。
//! 画面への合成はcircle_imageが行う(波紋の周りだけを切り出した小さな画像を、盤面の画像の上に重ねる)。
//! 判定・スコアには関与しない、見た目だけの状態。

use std::time::Duration;

use image::{Rgba, RgbaImage};

/// 波紋が広がり切って消えるまでの時間
pub const RIPPLE_DURATION: Duration = Duration::from_millis(500);
/// 波紋の画像を作り直す間隔(1コマの長さ)。画像プロトコルのエンコード・端末への送信は重いため、
/// 毎tick(33ms)ではなくこの間隔ごとにだけ作り直す
pub const RIPPLE_FRAME_INTERVAL: Duration = Duration::from_millis(50);
/// 広がり切った時の半径。1セルの高さ(ピクセル)に対する倍率
pub const RIPPLE_MAX_RADIUS_CELLS: f64 = 3.0;
/// リングの線の太さ。1セルの高さ(ピクセル)に対する倍率
pub const RIPPLE_THICKNESS_CELLS: f64 = 0.3;
/// リングの色(正解フィードバックの明るい緑に合わせる)
pub const RIPPLE_COLOR: [u8; 3] = [120, 235, 150];

/// 表示中の波紋1つ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ripple {
    /// 中心(クリックしたセルの座標)
    center: (u16, u16),
    /// 開始からの経過時間
    elapsed: Duration,
}

impl Ripple {
    /// クリックしたセル(column, row)を中心に、波紋を始める
    pub fn new(column: u16, row: u16) -> Self {
        Self {
            center: (column, row),
            elapsed: Duration::ZERO,
        }
    }

    /// 中心のセル座標
    pub fn center(&self) -> (u16, u16) {
        self.center
    }

    /// dtだけ時間を進めた波紋。持続時間を過ぎたらNone
    pub fn advanced(self, dt: Duration) -> Option<Self> {
        let elapsed = self.elapsed + dt;
        (elapsed < RIPPLE_DURATION).then_some(Self { elapsed, ..self })
    }

    /// いま何コマ目か(RIPPLE_FRAME_INTERVALごとに1ずつ増える)。
    /// 描画側はコマが変わった時だけ画像を作り直す
    pub fn frame(&self) -> u32 {
        (self.elapsed.as_millis() / RIPPLE_FRAME_INTERVAL.as_millis()) as u32
    }

    /// 進み具合(0.0=開始直後, 1.0=消える時)
    pub fn progress(&self) -> f64 {
        (self.elapsed.as_secs_f64() / RIPPLE_DURATION.as_secs_f64()).clamp(0.0, 1.0)
    }

    /// いまの半径(1セルの高さに対する倍率)。0から最大半径まで、初めは速く後はゆっくり広がる
    pub fn radius_cells(&self) -> f64 {
        let remaining = 1.0 - self.progress();
        RIPPLE_MAX_RADIUS_CELLS * (1.0 - remaining * remaining)
    }

    /// いまの不透明度(1.0から0.0へ減っていく)
    pub fn opacity(&self) -> f64 {
        1.0 - self.progress()
    }
}

/// width×heightピクセルの透明な画像に、center(ピクセル座標)を中心とする半径radiusのリングを描く。
/// 中心からの距離が「radius ± thickness/2」の範囲のピクセルだけをcolor・不透明度opacity
/// (0.0〜1.0)で塗り、それ以外は透明のまま
pub fn ring_image(
    width: u32,
    height: u32,
    center: (f64, f64),
    radius: f64,
    thickness: f64,
    opacity: f64,
    color: [u8; 3],
) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let half = thickness.max(0.0) / 2.0;
    let outer = radius + half;
    if alpha == 0 || width == 0 || height == 0 || outer <= 0.0 {
        return image;
    }
    // リングの外接矩形の中だけを調べる(毎フレーム作り直すため、ボード全体を走査しない)
    let clamp_x = |v: f64| v.floor().clamp(0.0, f64::from(width)) as u32;
    let clamp_y = |v: f64| v.floor().clamp(0.0, f64::from(height)) as u32;
    let (x0, x1) = (clamp_x(center.0 - outer), clamp_x(center.0 + outer + 1.0));
    let (y0, y1) = (clamp_y(center.1 - outer), clamp_y(center.1 + outer + 1.0));
    let pixel = Rgba([color[0], color[1], color[2], alpha]);
    for y in y0..y1 {
        for x in x0..x1 {
            // ピクセルの中心(x+0.5, y+0.5)から波紋の中心までの距離で判定する
            let distance = (f64::from(x) + 0.5 - center.0).hypot(f64::from(y) + 0.5 - center.1);
            if (distance - radius).abs() <= half {
                image.put_pixel(x, y, pixel);
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ピクセル(x, y)の中心から点centerまでの距離
    fn distance(x: u32, y: u32, center: (f64, f64)) -> f64 {
        (f64::from(x) + 0.5 - center.0).hypot(f64::from(y) + 0.5 - center.1)
    }

    // --- リング画像 ---

    #[test]
    fn ring_is_opaque_only_near_radius() {
        let center = (50.0, 40.0);
        let (radius, thickness) = (20.0, 4.0);
        let image = ring_image(100, 80, center, radius, thickness, 1.0, [1, 2, 3]);
        assert_eq!(image.dimensions(), (100, 80));
        let mut ring_pixels = 0;
        for (x, y, pixel) in image.enumerate_pixels() {
            let d = distance(x, y, center);
            if (d - radius).abs() <= thickness / 2.0 - 0.01 {
                assert_eq!(pixel.0, [1, 2, 3, 255], "半径付近({x},{y})は塗る");
                ring_pixels += 1;
            } else if (d - radius).abs() > thickness / 2.0 + 0.01 {
                assert_eq!(pixel.0[3], 0, "半径から離れた({x},{y})は透明");
            }
        }
        assert!(ring_pixels > 100, "リングが描かれていること: {ring_pixels}");
        assert_eq!(
            image.get_pixel(50, 40).0[3],
            0,
            "中心は透明(円盤ではなくリング)"
        );
    }

    #[test]
    fn ring_alpha_follows_opacity() {
        let alpha_on_ring = |opacity: f64| {
            let image = ring_image(60, 60, (30.0, 30.0), 20.0, 4.0, opacity, [9, 9, 9]);
            // 中心から右へ半径分の位置はリングの上
            image.get_pixel(50, 30).0[3]
        };
        assert_eq!(alpha_on_ring(1.0), 255);
        let half = alpha_on_ring(0.5);
        let faint = alpha_on_ring(0.05);
        assert!(
            half < 255 && half > faint,
            "不透明度に応じて薄くなる: {half}, {faint}"
        );
        assert!(faint <= 13);
        assert_eq!(alpha_on_ring(0.0), 0, "不透明度0なら見えない");
    }

    #[test]
    fn ring_partly_outside_image_does_not_panic() {
        // 画面端をクリックした時など、リングの一部が画像の外にはみ出してもよい
        let image = ring_image(30, 20, (0.0, 0.0), 15.0, 3.0, 1.0, [5, 5, 5]);
        assert!(image.pixels().any(|p| p.0[3] > 0));
        let image = ring_image(30, 20, (-100.0, 500.0), 5.0, 3.0, 1.0, [5, 5, 5]);
        assert!(
            image.pixels().all(|p| p.0[3] == 0),
            "画像の外のリングは描かれない"
        );
        let image = ring_image(0, 0, (0.0, 0.0), 5.0, 3.0, 1.0, [5, 5, 5]);
        assert_eq!(image.dimensions(), (0, 0));
    }

    // --- 波紋の状態 ---

    #[test]
    fn new_ripple_starts_at_center_with_zero_radius_and_full_opacity() {
        let ripple = Ripple::new(12, 7);
        assert_eq!(ripple.center(), (12, 7));
        assert_eq!(ripple.progress(), 0.0);
        assert_eq!(ripple.radius_cells(), 0.0);
        assert_eq!(ripple.opacity(), 1.0);
    }

    #[test]
    fn ripple_grows_and_fades_over_time() {
        let mut ripple = Ripple::new(0, 0);
        let step = RIPPLE_DURATION / 5;
        let mut last_radius = ripple.radius_cells();
        let mut last_opacity = ripple.opacity();
        for _ in 0..4 {
            ripple = ripple.advanced(step).expect("持続時間内は残る");
            assert!(ripple.radius_cells() > last_radius, "半径は広がっていく");
            assert!(ripple.opacity() < last_opacity, "不透明度は減っていく");
            assert!(ripple.radius_cells() <= RIPPLE_MAX_RADIUS_CELLS);
            last_radius = ripple.radius_cells();
            last_opacity = ripple.opacity();
        }
        assert_eq!(ripple.center(), (0, 0), "中心は動かない");
    }

    #[test]
    fn ripple_disappears_after_duration() {
        let ripple = Ripple::new(3, 4);
        let almost = ripple
            .advanced(RIPPLE_DURATION - Duration::from_millis(1))
            .expect("持続時間の直前はまだ残る");
        assert!(almost.opacity() > 0.0);
        assert!(
            almost.advanced(Duration::from_millis(1)).is_none(),
            "持続時間で消える"
        );
        assert!(ripple.advanced(RIPPLE_DURATION * 3).is_none());
    }

    // --- 画像を作り直すコマ ---

    #[test]
    fn frame_advances_once_per_frame_interval() {
        let ripple = Ripple::new(0, 0);
        assert_eq!(ripple.frame(), 0);
        let just_before = ripple
            .advanced(RIPPLE_FRAME_INTERVAL - Duration::from_millis(1))
            .unwrap();
        assert_eq!(just_before.frame(), 0, "コマの間隔に届くまでは同じコマ");
        assert_eq!(ripple.advanced(RIPPLE_FRAME_INTERVAL).unwrap().frame(), 1);
        let last = ripple
            .advanced(RIPPLE_DURATION - Duration::from_millis(1))
            .unwrap();
        let frames = (RIPPLE_DURATION.as_millis() / RIPPLE_FRAME_INTERVAL.as_millis()) as u32;
        assert_eq!(last.frame(), frames - 1, "持続時間の中のコマ数");
    }

    #[test]
    fn frame_interval_is_coarser_than_tick() {
        // 毎tick作り直さないよう、コマの間隔はtickより長くする
        assert!(RIPPLE_FRAME_INTERVAL > crate::TICK_RATE);
        assert!(RIPPLE_FRAME_INTERVAL < RIPPLE_DURATION);
    }

    #[test]
    fn ripple_reaches_max_radius_at_end() {
        let almost = Ripple::new(0, 0)
            .advanced(RIPPLE_DURATION - Duration::from_millis(1))
            .unwrap();
        assert!((almost.radius_cells() - RIPPLE_MAX_RADIUS_CELLS).abs() < 0.01);
        assert!(almost.opacity() < 0.01);
    }
}
