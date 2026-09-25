//! カウントマニアの数字付き円の描画。
//!
//! 画像(assets/image/count_mania/1.png〜20.png、白い円に黒い数字)の白い部分だけを
//! 指定色に塗り替えて表示する。sixel/kitty/iTerm2の画像プロトコルに対応した端末では画像を、
//! 非対応の端末では丸囲み数字(①②…)を色付きテキストで表示する。
//! 図形描画用のShapeCanvasとは独立した、このゲーム専用の部品。

use std::cell::RefCell;
use std::collections::HashMap;

use image::{DynamicImage, RgbaImage};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;
use rust_embed::RustEmbed;

use super::layout::circle_contains;

#[derive(RustEmbed)]
#[folder = "assets/image/count_mania/"]
struct CircleAssets;

/// RGB各チャンネルがこの値以上なら白(円の塗り)とみなす
const WHITE_THRESHOLD: u8 = 200;
/// RGB各チャンネルがこの値以下なら黒(数字)とみなす
const BLACK_THRESHOLD: u8 = 50;

/// 番号(1〜20)の円画像を読み込む。該当する画像が無ければNone
pub fn load_circle_image(number: u8) -> Option<RgbaImage> {
    let file = CircleAssets::get(&format!("{number}.png"))?;
    let image = image::load_from_memory(&file.data).ok()?;
    Some(image.to_rgba8())
}

/// 白いピクセル(円の塗り)だけをcolorに置き換える。黒いピクセル(数字)と透明部分はそのまま。
/// 白と黒の中間(文字の輪郭のなめらかな部分)は、黒からcolorへの間を明るさに応じて補間する
pub fn recolor(image: &RgbaImage, color: [u8; 3]) -> RgbaImage {
    let mut out = image.clone();
    for pixel in out.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        // 完全に透明なピクセルは見えないので触らない
        if a == 0 {
            continue;
        }
        let rgb = [r, g, b];
        let min = rgb.iter().copied().min().unwrap_or(0);
        let max = rgb.iter().copied().max().unwrap_or(0);
        if min >= WHITE_THRESHOLD {
            pixel.0 = [color[0], color[1], color[2], a];
        } else if max <= BLACK_THRESHOLD {
            // 数字の黒はそのまま
        } else {
            // 中間の明るさ: 黒(0)〜color(1)の間を、しきい値の間での明るさの位置で補間する
            let brightness = rgb.iter().map(|&c| f64::from(c)).sum::<f64>() / 3.0;
            let t = ((brightness - f64::from(BLACK_THRESHOLD))
                / f64::from(WHITE_THRESHOLD - BLACK_THRESHOLD))
            .clamp(0.0, 1.0);
            let mix = |c: u8| (f64::from(c) * t).round() as u8;
            pixel.0 = [mix(color[0]), mix(color[1]), mix(color[2]), a];
        }
    }
    out
}

/// 番号(1〜20)の丸囲み数字(①〜⑳)。範囲外はNone
pub fn circled_digit(number: u8) -> Option<char> {
    if !(1..=20).contains(&number) {
        return None;
    }
    // ①(U+2460)〜⑳(U+2473)は連続している
    char::from_u32(0x2460 + u32::from(number) - 1)
}

/// 直前に作った画像と同じ内容なら再エンコードを省くためのキー
#[derive(Clone, Copy, PartialEq, Eq)]
struct CacheKey {
    color: [u8; 3],
    width: u16,
    height: u16,
}

struct Cached {
    key: CacheKey,
    protocol: StatefulProtocol,
}

/// 数字付き円の描画器。画像プロトコルが使える端末では円ごとにエンコード済み画像を
/// キャッシュし、色やサイズが変わった時だけ作り直す
pub struct CircleRenderer {
    picker: Option<Picker>,
    cache: RefCell<HashMap<u8, Cached>>,
}

impl CircleRenderer {
    pub fn new() -> Self {
        Self {
            picker: detect_picker(),
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// 画像プロトコルを使うか(false=丸囲み数字のテキスト表示)。テストでの確認用
    #[cfg(test)]
    pub fn uses_image(&self) -> bool {
        self.picker.is_some()
    }

    /// 番号numberの円をrectにcolorで描く
    pub fn render(&self, frame: &mut Frame, rect: Rect, number: u8, color: [u8; 3]) {
        // 描画先がフレームからはみ出さないよう切り詰める
        let rect = rect.intersection(frame.area());
        if rect.is_empty() {
            return;
        }
        if !self.render_image(frame, rect, number, color) {
            render_text(frame, rect, number, color);
        }
    }

    /// 画像プロトコルで描く。画像プロトコルが使えない/画像が読めない場合はfalse
    fn render_image(&self, frame: &mut Frame, rect: Rect, number: u8, color: [u8; 3]) -> bool {
        let Some(picker) = &self.picker else {
            return false;
        };
        let key = CacheKey {
            color,
            width: rect.width,
            height: rect.height,
        };
        let mut cache = self.cache.borrow_mut();
        let needs_regen = !matches!(cache.get(&number), Some(cached) if cached.key == key);
        if needs_regen {
            let Some(image) = load_circle_image(number) else {
                return false;
            };
            let recolored = DynamicImage::ImageRgba8(recolor(&image, color));
            let protocol = picker.new_resize_protocol(recolored);
            cache.insert(number, Cached { key, protocol });
        }
        let Some(cached) = cache.get_mut(&number) else {
            return false;
        };
        frame.render_stateful_widget(StatefulImage::default(), rect, &mut cached.protocol);
        true
    }
}

/// 端末の画像プロトコルを調べる。sixel/kitty/iTerm2のどれかが使える時だけSome。
/// テストでは端末に問い合わせず、常にテキスト表示にする(実行環境で結果が変わらないように)
fn detect_picker() -> Option<Picker> {
    if cfg!(test) {
        return None;
    }
    Picker::from_query_stdio().ok().filter(|picker| {
        matches!(
            picker.protocol_type(),
            ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2
        )
    })
}

/// 画像プロトコル非対応の端末向け表示。円の範囲(内接楕円)を色付きの網掛けで塗り、
/// 中央に色付きの丸囲み数字を置く。網掛けでクリックできる範囲(大きさ)が分かるようにする
fn render_text(frame: &mut Frame, rect: Rect, number: u8, color: [u8; 3]) {
    let [r, g, b] = color;
    let fg = Color::Rgb(r, g, b);
    let buffer = frame.buffer_mut();
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if circle_contains(rect, x, y) {
                buffer[(x, y)].set_symbol("░").set_fg(fg);
            }
        }
    }
    let Some(digit) = circled_digit(number) else {
        return;
    };
    // 丸囲み数字は端末によって2セル幅で表示されるため、両隣を空白にして網掛けと重ならないようにする
    let label_width = rect.width.min(3);
    let label = Rect::new(
        rect.x + (rect.width - label_width) / 2,
        rect.y + rect.height / 2,
        label_width,
        1,
    );
    let text = if label_width >= 3 {
        format!(" {digit} ")
    } else {
        digit.to_string()
    };
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(fg).add_modifier(Modifier::BOLD)),
        label,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn all_twenty_circle_images_are_embedded() {
        for number in 1..=20u8 {
            let image = load_circle_image(number)
                .unwrap_or_else(|| panic!("{number}.pngが埋め込まれていること"));
            assert_eq!(image.dimensions(), (128, 128));
        }
        assert!(load_circle_image(0).is_none());
        assert!(load_circle_image(21).is_none());
    }

    #[test]
    fn recolor_replaces_white_keeps_black_and_transparent() {
        let mut image = RgbaImage::new(3, 1);
        image.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        image.put_pixel(1, 0, Rgba([0, 0, 0, 255]));
        image.put_pixel(2, 0, Rgba([0, 0, 0, 0]));
        let out = recolor(&image, [10, 200, 30]);
        assert_eq!(
            out.get_pixel(0, 0).0,
            [10, 200, 30, 255],
            "白は指定色になる"
        );
        assert_eq!(out.get_pixel(1, 0).0, [0, 0, 0, 255], "黒はそのまま");
        assert_eq!(out.get_pixel(2, 0).0[3], 0, "透明はそのまま透明");
    }

    #[test]
    fn recolor_keeps_alpha_of_antialiased_white_edge() {
        let mut image = RgbaImage::new(1, 1);
        image.put_pixel(0, 0, Rgba([250, 250, 250, 100]));
        let out = recolor(&image, [200, 100, 50]);
        assert_eq!(out.get_pixel(0, 0).0, [200, 100, 50, 100]);
    }

    #[test]
    fn recolor_of_real_asset_keeps_black_digit_pixels() {
        let image = load_circle_image(8).unwrap();
        let out = recolor(&image, [0, 128, 255]);
        let count = |img: &RgbaImage, rgb: [u8; 3]| {
            img.pixels()
                .filter(|p| p.0[3] == 255 && p.0[..3] == rgb)
                .count()
        };
        assert!(count(&out, [0, 128, 255]) > 1000, "円の塗りが指定色になる");
        assert_eq!(count(&out, [255, 255, 255]), 0, "白は残らない");
        assert_eq!(
            count(&image, [0, 0, 0]),
            count(&out, [0, 0, 0]),
            "数字の黒は変わらない"
        );
    }

    #[test]
    fn circled_digit_maps_one_to_twenty() {
        assert_eq!(circled_digit(1), Some('①'));
        assert_eq!(circled_digit(10), Some('⑩'));
        assert_eq!(circled_digit(20), Some('⑳'));
        assert_eq!(circled_digit(0), None);
        assert_eq!(circled_digit(21), None);
    }

    #[test]
    fn fallback_renders_colored_circled_digit_in_rect() {
        // テスト環境(非TTY)では画像プロトコルを検出できず、テキスト表示になる
        let renderer = CircleRenderer::new();
        assert!(!renderer.uses_image());
        let backend = TestBackend::new(30, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let rect = Rect::new(4, 2, 8, 4);
        terminal
            .draw(|frame| renderer.render(frame, rect, 12, [1, 2, 3]))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let cell = (rect.x..rect.right())
            .flat_map(|x| (rect.y..rect.bottom()).map(move |y| (x, y)))
            .map(|pos| &buffer[pos])
            .find(|c| c.symbol() == "⑫")
            .expect("rect内に⑫が描かれること");
        assert_eq!(cell.fg, Color::Rgb(1, 2, 3));
        // rectの外には何も描かない
        let outside: String = (0..30).map(|x| buffer[(x, 0)].symbol()).collect();
        assert_eq!(outside.trim(), "");
    }
}
