use std::cell::RefCell;

use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::Block;
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;

use crate::canvas::shapes::Shape;

/// 1本の線分(始点, 終点)
type LineSegment = ((f64, f64), (f64, f64));
/// 1図形分の(線分リスト, 色)
type ColoredLines = (Vec<LineSegment>, Color);

/// ratatui::style::ColorをRGBのピクセル値に変換する。
/// 名前付き色(ANSI 16色)は標準的なパレット値の近似で決め打ちする。
/// Color::Rgbはそのまま使う。Reset/Indexed等の不明な色は白にする。
fn color_to_rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Black => [0, 0, 0],
        Color::Red => [205, 0, 0],
        Color::Green => [0, 205, 0],
        Color::Yellow => [205, 205, 0],
        Color::Blue => [0, 0, 238],
        Color::Magenta => [205, 0, 205],
        Color::Cyan => [0, 205, 205],
        Color::Gray => [229, 229, 229],
        Color::DarkGray => [127, 127, 127],
        Color::LightRed => [255, 0, 0],
        Color::LightGreen => [0, 255, 0],
        Color::LightYellow => [255, 255, 0],
        Color::LightBlue => [92, 92, 255],
        Color::LightMagenta => [255, 0, 255],
        Color::LightCyan => [0, 255, 255],
        Color::White => [255, 255, 255],
        _ => [255, 255, 255],
    }
}

/// 複数図形分の(線分リスト, 色)を、px_w x px_hのRGBA画像に重ねてラスタライズする。
/// 背景は透過、線は各図形の色で不透明に描く。boundsは全図形共通の(x_bounds, y_bounds)。
fn rasterize_many(
    shapes: &[ColoredLines],
    bounds: ([f64; 2], [f64; 2]),
    px_w: u32,
    px_h: u32,
) -> DynamicImage {
    let mut img = RgbaImage::new(px_w, px_h);
    let ([x_min, x_max], [y_min, y_max]) = bounds;
    let to_px = |x: f64, y: f64| -> (i64, i64) {
        let nx = (x - x_min) / (x_max - x_min);
        // 画像は上端がy最大になるようy軸を反転する(Canvas widgetのy_boundsと向きを揃える)
        let ny = 1.0 - (y - y_min) / (y_max - y_min);
        (
            (nx * (px_w.saturating_sub(1)) as f64).round() as i64,
            (ny * (px_h.saturating_sub(1)) as f64).round() as i64,
        )
    };

    for (lines, color) in shapes {
        let [r, g, b] = color_to_rgb(*color);
        let pixel = Rgba([r, g, b, 255]);
        for &(p1, p2) in lines {
            let (x1, y1) = to_px(p1.0, p1.1);
            let (x2, y2) = to_px(p2.0, p2.1);
            draw_line(&mut img, x1, y1, x2, y2, pixel);
        }
    }
    DynamicImage::ImageRgba8(img)
}

#[cfg(test)]
fn rasterize(
    lines: &[LineSegment],
    bounds: ([f64; 2], [f64; 2]),
    color: Color,
    px_w: u32,
    px_h: u32,
) -> DynamicImage {
    rasterize_many(&[(lines.to_vec(), color)], bounds, px_w, px_h)
}

/// Bresenhamの直線描画アルゴリズム。太さ1pxで画像範囲外の座標は無視する。
fn draw_line(img: &mut RgbaImage, x1: i64, y1: i64, x2: i64, y2: i64, pixel: Rgba<u8>) {
    let (mut x, mut y) = (x1, y1);
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    let sx = if x2 >= x1 { 1 } else { -1 };
    let sy = if y2 >= y1 { 1 } else { -1 };
    let mut err = dx - dy;

    loop {
        if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
            img.put_pixel(x as u32, y as u32, pixel);
        }
        if x == x2 && y == y2 {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
    }
}

/// 直前に描画した内容とキーが変わっていなければ再ラスタライズ・再エンコードを
/// 省略するためのキャッシュキー。画像プロトコルの新規生成は数msかかるため
/// (Sixelで1フレームあたり十数ms)、内容が同じ間は再利用する。
#[derive(Clone, PartialEq)]
struct CacheKey {
    shapes: Vec<ColoredLines>,
    bounds: ([f64; 2], [f64; 2]),
    area: Rect,
}

struct Cached {
    key: CacheKey,
    protocol: StatefulProtocol,
}

/// 図形(線分リスト)をsixel/kitty対応端末では画像として、非対応端末では
/// 既存のCanvas widget(braille)で描画する。`render(&self, ...)`という不変参照の
/// シグネチャのまま、内容が変わらない間はエンコード済み画像をRefCellで再利用する。
pub struct ShapeCanvas {
    picker: Option<Picker>,
    cache: RefCell<Option<Cached>>,
    /// テスト用: 画像を新規生成(ラスタライズ・エンコード)した回数
    #[cfg(test)]
    encode_count: std::cell::Cell<usize>,
}

impl ShapeCanvas {
    pub fn new() -> Self {
        let picker = Picker::from_query_stdio().ok().filter(|p| {
            matches!(
                p.protocol_type(),
                ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2
            )
        });
        Self::with_picker(picker)
    }

    fn with_picker(picker: Option<Picker>) -> Self {
        Self {
            picker,
            cache: RefCell::new(None),
            #[cfg(test)]
            encode_count: std::cell::Cell::new(0),
        }
    }

    /// テスト用: 非TTYでも画像経路を通すため、sixel固定のPickerで作る
    #[cfg(test)]
    pub(crate) fn with_sixel_for_test() -> Self {
        let mut picker = Picker::from_fontsize((8, 16));
        picker.set_protocol_type(ProtocolType::Sixel);
        Self::with_picker(Some(picker))
    }

    #[cfg(test)]
    fn encode_count(&self) -> usize {
        self.encode_count.get()
    }

    /// 1色1図形を描く(shape_rotate/mirror_matchのような単純な用途向け)
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        block: Block,
        shape: &Shape,
        bounds: ([f64; 2], [f64; 2]),
        color: Color,
    ) {
        self.render_many(frame, area, block, &[(shape, color)], bounds);
    }

    /// 複数の図形をそれぞれの色で同じ座標系に重ねて描く
    /// (puzzle_connectのお手本のような、複数図形を1枚にまとめる用途向け)
    pub fn render_many(
        &self,
        frame: &mut Frame,
        area: Rect,
        block: Block,
        shapes: &[(&Shape, Color)],
        bounds: ([f64; 2], [f64; 2]),
    ) {
        let Some(picker) = &self.picker else {
            render_canvas_fallback(frame, area, block, shapes, bounds);
            return;
        };

        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let key = CacheKey {
            shapes: shapes
                .iter()
                .map(|(shape, color)| (shape.to_lines(), *color))
                .collect(),
            bounds,
            area: inner,
        };

        let mut cache = self.cache.borrow_mut();
        let needs_regen = !matches!(&*cache, Some(cached) if cached.key == key);
        if needs_regen {
            let px_w = inner.width as u32 * picker.font_size().0 as u32;
            let px_h = inner.height as u32 * picker.font_size().1 as u32;
            let image = rasterize_many(&key.shapes, key.bounds, px_w, px_h);
            let protocol = picker.new_resize_protocol(image);
            *cache = Some(Cached { key, protocol });
            #[cfg(test)]
            self.encode_count.set(self.encode_count.get() + 1);
        }

        if let Some(cached) = cache.as_mut() {
            let widget = StatefulImage::default();
            frame.render_stateful_widget(widget, inner, &mut cached.protocol);
        }
    }
}

fn render_canvas_fallback(
    frame: &mut Frame,
    area: Rect,
    block: Block,
    shapes: &[(&Shape, Color)],
    bounds: ([f64; 2], [f64; 2]),
) {
    let lines: Vec<ColoredLines> = shapes
        .iter()
        .map(|(shape, color)| (shape.to_lines(), *color))
        .collect();
    let ([x_min, x_max], [y_min, y_max]) = bounds;
    let canvas = Canvas::default()
        .block(block)
        .x_bounds([x_min, x_max])
        .y_bounds([y_min, y_max])
        .paint(move |ctx| {
            for (shape_lines, color) in &lines {
                for (p1, p2) in shape_lines {
                    ctx.draw(&CanvasLine {
                        x1: p1.0,
                        y1: p1.1,
                        x2: p2.0,
                        y2: p2.1,
                        color: *color,
                    });
                }
            }
        });
    frame.render_widget(canvas, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_to_rgb_passes_through_explicit_rgb() {
        assert_eq!(color_to_rgb(Color::Rgb(10, 20, 30)), [10, 20, 30]);
    }

    #[test]
    fn color_to_rgb_maps_named_colors_to_distinct_values() {
        let red = color_to_rgb(Color::LightRed);
        let green = color_to_rgb(Color::LightGreen);
        assert_ne!(red, green);
        assert_eq!(red, [255, 0, 0]);
        assert_eq!(green, [0, 255, 0]);
    }

    #[test]
    fn rasterize_draws_opaque_pixels_along_a_horizontal_line() {
        let lines = [((-1.0, 0.0), (1.0, 0.0))];
        let image = rasterize(&lines, ([-1.0, 1.0], [-1.0, 1.0]), Color::White, 20, 20);
        let rgba = image.to_rgba8();
        // 中央の行(y=0相当)には不透明なピクセルがあるはず
        let mid_row_has_opaque = (0..20).any(|x| rgba.get_pixel(x, 10).0[3] > 0);
        assert!(mid_row_has_opaque, "中央付近に線が描かれていること");
    }

    #[test]
    fn rasterize_leaves_far_corners_transparent() {
        let lines = [((-1.0, 0.0), (1.0, 0.0))];
        let image = rasterize(&lines, ([-1.0, 1.0], [-1.0, 1.0]), Color::White, 20, 20);
        let rgba = image.to_rgba8();
        // 水平線から離れた四隅は透過のままのはず
        assert_eq!(rgba.get_pixel(0, 0).0[3], 0);
        assert_eq!(rgba.get_pixel(19, 19).0[3], 0);
    }

    #[test]
    fn rasterize_uses_the_requested_color() {
        let lines = [((-1.0, 0.0), (1.0, 0.0))];
        let image = rasterize(&lines, ([-1.0, 1.0], [-1.0, 1.0]), Color::Rgb(9, 8, 7), 10, 10);
        let rgba = image.to_rgba8();
        let opaque = (0..10)
            .flat_map(|x| (0..10).map(move |y| (x, y)))
            .find_map(|(x, y)| {
                let p = rgba.get_pixel(x, y);
                (p.0[3] > 0).then_some(p.0)
            })
            .expect("線が1px以上描かれていること");
        assert_eq!(opaque, [9, 8, 7, 255]);
    }

    #[test]
    fn shape_canvas_render_does_not_panic_regardless_of_picker_availability() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        // テスト環境(非TTY)ではPicker検出は必ず失敗し、Fallback経路のみ通る。
        // Image経路は実端末での目視確認に依存するため、ここではpanicしないことのみ確認する。
        let canvas = ShapeCanvas::new();
        let shape = Shape::new(vec![(-0.5, -0.5), (0.5, -0.5), (0.0, 0.5)]);
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                canvas.render(
                    frame,
                    area,
                    Block::default(),
                    &shape,
                    ([-1.0, 1.0], [-1.0, 1.0]),
                    Color::White,
                );
            })
            .unwrap();
    }

    #[test]
    fn rasterize_many_draws_both_shapes_in_their_own_color() {
        let a_lines = vec![((-1.0, 0.0), (0.0, 0.0))];
        let b_lines = vec![((0.0, 1.0), (0.0, -1.0))];
        let shapes = vec![
            (a_lines, Color::Rgb(255, 0, 0)),
            (b_lines, Color::Rgb(0, 255, 0)),
        ];
        let image = rasterize_many(&shapes, ([-1.0, 1.0], [-1.0, 1.0]), 20, 20);
        let rgba = image.to_rgba8();
        let colors: std::collections::HashSet<[u8; 4]> = (0..20)
            .flat_map(|x| (0..20).map(move |y| (x, y)))
            .filter_map(|(x, y)| {
                let p = rgba.get_pixel(x, y).0;
                (p[3] > 0).then_some(p)
            })
            .collect();
        assert!(colors.contains(&[255, 0, 0, 255]), "1つ目の図形の色が描かれていること");
        assert!(colors.contains(&[0, 255, 0, 255]), "2つ目の図形の色が描かれていること");
    }

    fn draw_shape(
        terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
        canvas: &ShapeCanvas,
        shape: &Shape,
    ) {
        terminal
            .draw(|frame| {
                let area = frame.area();
                canvas.render(
                    frame,
                    area,
                    Block::default(),
                    shape,
                    ([-1.0, 1.0], [-1.0, 1.0]),
                    Color::White,
                );
            })
            .unwrap();
    }

    #[test]
    fn image_path_reuses_encoding_while_content_is_unchanged() {
        // 同じ図形を何フレーム描いても、画像の新規生成は最初の1回だけ
        let canvas = ShapeCanvas::with_sixel_for_test();
        let shape = Shape::new(vec![(-0.5, -0.5), (0.5, -0.5), (0.0, 0.5)]);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 10)).unwrap();
        for _ in 0..5 {
            draw_shape(&mut terminal, &canvas, &shape);
        }
        assert_eq!(canvas.encode_count(), 1);
    }

    #[test]
    fn image_path_reencodes_when_shape_changes() {
        let canvas = ShapeCanvas::with_sixel_for_test();
        let a = Shape::new(vec![(-0.5, -0.5), (0.5, -0.5), (0.0, 0.5)]);
        let b = Shape::new(vec![(-0.3, -0.3), (0.3, -0.3), (0.0, 0.3)]);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 10)).unwrap();
        draw_shape(&mut terminal, &canvas, &a);
        draw_shape(&mut terminal, &canvas, &b);
        draw_shape(&mut terminal, &canvas, &b);
        assert_eq!(canvas.encode_count(), 2);
    }

    #[test]
    fn shape_canvas_render_many_does_not_panic_regardless_of_picker_availability() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let canvas = ShapeCanvas::new();
        let shape_a = Shape::new(vec![(-0.5, -0.5), (0.5, -0.5), (0.0, 0.5)]);
        let shape_b = Shape::new(vec![(-0.3, -0.3), (0.3, -0.3), (0.0, 0.3)]);
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                canvas.render_many(
                    frame,
                    area,
                    Block::default(),
                    &[(&shape_a, Color::White), (&shape_b, Color::LightGreen)],
                    ([-1.0, 1.0], [-1.0, 1.0]),
                );
            })
            .unwrap();
    }
}
