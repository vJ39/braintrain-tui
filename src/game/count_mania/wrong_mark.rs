//! カウントメニアの誤クリック時のバツ印。
//!
//! 押すべきでない円をクリックした位置(セル座標)に、赤いバツ印を一定時間表示する。
//! ここではバツ印の状態(中心と経過時間)と、バツ印を描いたRGBA画像の生成を持つ。
//! 画面への合成はcircle_imageが行う(波紋と同じく、盤面の画像の一部を切り出したパッチに重ねる)。
//! 判定・スコアには関与しない、見た目だけの状態。

use std::time::Duration;

use image::{Rgba, RgbaImage};

/// バツ印を表示しておく時間
pub const WRONG_MARK_DURATION: Duration = Duration::from_millis(1500);
/// 中心から腕の先までの長さ(縦・横それぞれ)。1セルの高さ(ピクセル)に対する倍率
pub const WRONG_MARK_HALF_SIZE_CELLS: f64 = 0.9;
/// 線の太さ。1セルの高さ(ピクセル)に対する倍率
pub const WRONG_MARK_THICKNESS_CELLS: f64 = 0.3;
/// バツ印の色(不正解の赤)
pub const WRONG_MARK_COLOR: [u8; 3] = [255, 60, 60];

/// 表示中のバツ印1つ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WrongMark {
    /// 中心(クリックしたセルの座標)
    center: (u16, u16),
    /// 表示を始めてからの経過時間
    elapsed: Duration,
}

impl WrongMark {
    /// クリックしたセル(column, row)にバツ印を出す
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

    /// dtだけ時間を進めたバツ印。表示時間を過ぎたらNone
    pub fn advanced(self, dt: Duration) -> Option<Self> {
        let elapsed = self.elapsed + dt;
        (elapsed < WRONG_MARK_DURATION).then_some(Self { elapsed, ..self })
    }
}

/// width×heightピクセルの透明な画像に、center(ピクセル座標)を交点とするバツ印を描く。
/// 腕は中心から縦横それぞれhalf_sizeまで伸びる2本の対角線で、線からの距離がthickness/2以内の
/// ピクセルだけをcolorで塗る。それ以外は透明のまま
pub fn cross_image(
    width: u32,
    height: u32,
    center: (f64, f64),
    half_size: f64,
    thickness: f64,
    color: [u8; 3],
) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    let half_thickness = thickness.max(0.0) / 2.0;
    if width == 0 || height == 0 || half_size <= 0.0 {
        return image;
    }
    // バツ印の外接矩形の中だけを調べる
    let reach = half_size + half_thickness;
    let clamp_x = |v: f64| v.floor().clamp(0.0, f64::from(width)) as u32;
    let clamp_y = |v: f64| v.floor().clamp(0.0, f64::from(height)) as u32;
    let (x0, x1) = (clamp_x(center.0 - reach), clamp_x(center.0 + reach + 1.0));
    let (y0, y1) = (clamp_y(center.1 - reach), clamp_y(center.1 + reach + 1.0));
    let pixel = Rgba([color[0], color[1], color[2], 255]);
    for y in y0..y1 {
        for x in x0..x1 {
            // ピクセルの中心(x+0.5, y+0.5)で判定する
            let dx = f64::from(x) + 0.5 - center.0;
            let dy = f64::from(y) + 0.5 - center.1;
            if dx.abs() > half_size || dy.abs() > half_size {
                continue;
            }
            // 2本の対角線(y=x, y=-x)までの距離
            let to_diagonal = (dx - dy).abs() / std::f64::consts::SQRT_2;
            let to_anti_diagonal = (dx + dy).abs() / std::f64::consts::SQRT_2;
            if to_diagonal.min(to_anti_diagonal) <= half_thickness {
                image.put_pixel(x, y, pixel);
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- バツ印の状態 ---

    #[test]
    fn new_mark_starts_at_clicked_cell() {
        let mark = WrongMark::new(12, 7);
        assert_eq!(mark.center(), (12, 7));
    }

    #[test]
    fn mark_stays_until_duration_then_disappears() {
        let mark = WrongMark::new(3, 4);
        let almost = mark
            .advanced(WRONG_MARK_DURATION - Duration::from_millis(1))
            .expect("持続時間の直前はまだ残る");
        assert_eq!(almost.center(), (3, 4), "中心は動かない");
        assert!(
            almost.advanced(Duration::from_millis(1)).is_none(),
            "持続時間で消える"
        );
        assert!(mark.advanced(WRONG_MARK_DURATION * 3).is_none());
    }

    #[test]
    fn mark_duration_is_longer_than_ripple_and_a_few_seconds_at_most() {
        // 数秒間の表示。正解の波紋(一瞬)よりは長く残して、どこを押し間違えたか分かるようにする
        assert!(WRONG_MARK_DURATION > super::super::ripple::RIPPLE_DURATION);
        assert!(WRONG_MARK_DURATION <= Duration::from_secs(3));
    }

    #[test]
    fn mark_color_is_reddish() {
        let [r, g, b] = WRONG_MARK_COLOR;
        assert!(r >= 200 && r > g && r > b, "赤系であること: {r},{g},{b}");
    }

    // --- バツ印の画像 ---

    /// 60x60ピクセルの画像の中央(30, 30)に、腕の長さ20・太さ4のバツ印を描く
    fn sample_cross() -> image::RgbaImage {
        cross_image(60, 60, (30.0, 30.0), 20.0, 4.0, [9, 8, 7])
    }

    #[test]
    fn cross_is_opaque_on_both_diagonals() {
        let image = sample_cross();
        assert_eq!(image.dimensions(), (60, 60));
        assert_eq!(
            image.get_pixel(30, 30).0,
            [9, 8, 7, 255],
            "中心(交点)は塗る"
        );
        // 右下がり・右上がりの両方の対角線の上
        for (x, y) in [(20, 20), (40, 40), (40, 19), (19, 40), (13, 13), (46, 46)] {
            assert_eq!(image.get_pixel(x, y).0[3], 255, "対角線の上({x},{y})は塗る");
        }
    }

    #[test]
    fn cross_is_transparent_off_diagonals_and_outside_arms() {
        let image = sample_cross();
        // 上下左右(対角線から離れた所)は透明
        for (x, y) in [(30, 15), (30, 45), (15, 30), (45, 30)] {
            assert_eq!(
                image.get_pixel(x, y).0[3],
                0,
                "対角線から離れた({x},{y})は透明"
            );
        }
        // 腕の先より外(対角線の延長上でも)は透明
        for (x, y) in [(5, 5), (55, 55), (55, 5), (5, 55)] {
            assert_eq!(image.get_pixel(x, y).0[3], 0, "腕の先より外({x},{y})は透明");
        }
    }

    #[test]
    fn cross_partly_outside_image_does_not_panic() {
        // 画面端をクリックした時など、バツ印の一部が画像の外にはみ出してもよい
        let image = cross_image(30, 20, (0.0, 0.0), 15.0, 3.0, [5, 5, 5]);
        assert!(image.pixels().any(|p| p.0[3] > 0));
        let image = cross_image(30, 20, (-100.0, 500.0), 5.0, 3.0, [5, 5, 5]);
        assert!(
            image.pixels().all(|p| p.0[3] == 0),
            "画像の外のバツ印は描かれない"
        );
        let image = cross_image(0, 0, (0.0, 0.0), 5.0, 3.0, [5, 5, 5]);
        assert_eq!(image.dimensions(), (0, 0));
    }
}
