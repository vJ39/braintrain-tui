//! カウントマニアの数字付き円の描画。
//!
//! 画像(assets/image/count_mania/1.png〜20.png、白い円に黒い数字)の白い部分だけを
//! 指定色に塗り替えて表示する。sixel/kitty/iTerm2の画像プロトコルに対応した端末では画像を、
//! 非対応の端末では丸囲み数字(①②…)を色付きテキストで表示する。
//! 円どうしは重なり合い、渡された並び順に描く(後の円ほど手前)。
//! 図形描画用のShapeCanvasとは独立した、このゲーム専用の部品。

use std::cell::RefCell;

use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;
use rust_embed::RustEmbed;

use super::layout::{circle_contains, label_area};

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

/// ボードに描く円1つ(位置・番号・色)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardCircle {
    pub rect: Rect,
    pub number: u8,
    pub color: [u8; 3],
}

/// 直前に作ったボード画像。描画エリアと円の並びが同じなら再エンコードを省く
struct BoardCache {
    board: Rect,
    circles: Vec<BoardCircle>,
    protocol: StatefulProtocol,
}

/// 数字付き円の描画器。画像プロトコルが使える端末では、ボード上の全ての円を1枚の画像に
/// 重ね合わせてから表示する(円ごとに別の画像にすると、重なった所で手前の画像が矩形ごと
/// 奥の画像を消したり、押して消えた円の画像が画面に残ったりするため)。
/// 描画エリアか円の並び(押して消えた円・色・位置)が変わった時だけ作り直す
pub struct CircleRenderer {
    picker: Option<Picker>,
    cache: RefCell<Option<BoardCache>>,
}

impl CircleRenderer {
    pub fn new() -> Self {
        Self {
            picker: detect_picker(),
            cache: RefCell::new(None),
        }
    }

    /// 画像プロトコルを使うか(false=丸囲み数字のテキスト表示)。テストでの確認用
    #[cfg(test)]
    pub fn uses_image(&self) -> bool {
        self.picker.is_some()
    }

    /// board内にcirclesを並び順に描く。後の円ほど手前に重なる
    pub fn render_board(&self, frame: &mut Frame, board: Rect, circles: &[BoardCircle]) {
        // 描画先がフレームからはみ出さないよう切り詰める
        let board = board.intersection(frame.area());
        if board.is_empty() {
            return;
        }
        if self.render_board_image(frame, board, circles) {
            return;
        }
        for circle in circles {
            let rect = circle.rect.intersection(frame.area());
            if !rect.is_empty() {
                render_text(frame, rect, circle.number, circle.color);
            }
        }
    }

    /// 画像プロトコルで描く。画像プロトコルが使えない/画像が読めない場合はfalse
    fn render_board_image(&self, frame: &mut Frame, board: Rect, circles: &[BoardCircle]) -> bool {
        let Some(picker) = &self.picker else {
            return false;
        };
        if circles.is_empty() {
            return true;
        }
        let mut cache = self.cache.borrow_mut();
        let needs_regen = !matches!(
            cache.as_ref(),
            Some(cached) if cached.board == board && cached.circles == circles
        );
        if needs_regen {
            let Some(images) = circles
                .iter()
                .map(|c| load_circle_image(c.number).map(|image| recolor(&image, c.color)))
                .collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            let layers: Vec<(Rect, &RgbaImage)> = circles
                .iter()
                .zip(&images)
                .map(|(c, image)| (c.rect, image))
                .collect();
            let composed = compose_board(board, picker.font_size(), &layers);
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(composed));
            *cache = Some(BoardCache {
                board,
                circles: circles.to_vec(),
                protocol,
            });
        }
        let Some(cached) = cache.as_mut() else {
            return false;
        };
        frame.render_stateful_widget(StatefulImage::default(), board, &mut cached.protocol);
        true
    }
}

/// board(セル単位)と同じ大きさのピクセル画像に、layers(セル単位の位置, 画像)を並び順に
/// 重ね合わせる。後の画像ほど手前で、透明な部分からは奥の画像が見える。円の無い所は透明。
/// 各画像は縦横比を保ったまま位置の枠に収まる最大の大きさにして、枠の中央に置く。
/// font_sizeは1セルのピクセル数(幅, 高さ)
pub fn compose_board(
    board: Rect,
    font_size: (u16, u16),
    layers: &[(Rect, &RgbaImage)],
) -> RgbaImage {
    let cell_width = u32::from(font_size.0.max(1));
    let cell_height = u32::from(font_size.1.max(1));
    let mut canvas = RgbaImage::new(
        u32::from(board.width) * cell_width,
        u32::from(board.height) * cell_height,
    );
    for &(rect, image) in layers {
        if rect.is_empty() || image.width() == 0 || image.height() == 0 {
            continue;
        }
        let frame_width = u32::from(rect.width) * cell_width;
        let frame_height = u32::from(rect.height) * cell_height;
        let scale = f64::min(
            f64::from(frame_width) / f64::from(image.width()),
            f64::from(frame_height) / f64::from(image.height()),
        );
        let width = ((f64::from(image.width()) * scale).round() as u32).clamp(1, frame_width);
        let height = ((f64::from(image.height()) * scale).round() as u32).clamp(1, frame_height);
        let scaled = imageops::resize(image, width, height, FilterType::Triangle);
        let x = (i64::from(rect.x) - i64::from(board.x)) * i64::from(cell_width)
            + i64::from((frame_width - width) / 2);
        let y = (i64::from(rect.y) - i64::from(board.y)) * i64::from(cell_height)
            + i64::from((frame_height - height) / 2);
        imageops::overlay(&mut canvas, &scaled, x, y);
    }
    canvas
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
    // 丸囲み数字は端末によって2セル幅で表示されるため、両隣を空白にして網掛けと重ならないようにする。
    // 置く位置は配置側(layout)が手前の円に覆わせないよう守っている範囲と同じにする
    let label = label_area(rect);
    let text = if label.width >= 3 {
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

    /// テキスト表示でボードを描き、バッファを返す
    fn render_fallback(
        width: u16,
        height: u16,
        circles: &[BoardCircle],
    ) -> ratatui::buffer::Buffer {
        let renderer = CircleRenderer::new();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| renderer.render_board(frame, frame.area(), circles))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba(rgba))
    }

    #[test]
    fn fallback_draws_later_circles_on_top() {
        // 大きい円1の左側に小さい円2が重なっている。重なった所は後に描いた円の色になる
        let large = BoardCircle {
            rect: Rect::new(0, 0, 14, 7),
            number: 1,
            color: [10, 10, 10],
        };
        let small = BoardCircle {
            rect: Rect::new(0, 2, 6, 3),
            number: 2,
            color: [200, 200, 200],
        };
        let buffer = render_fallback(20, 8, &[large, small]);
        assert_eq!(
            buffer[(2, 3)].fg,
            Color::Rgb(200, 200, 200),
            "小さい円が手前"
        );
        assert_eq!(
            buffer[(10, 3)].fg,
            Color::Rgb(10, 10, 10),
            "重ならない所は大きい円"
        );
        let buffer = render_fallback(20, 8, &[small, large]);
        assert_eq!(
            buffer[(2, 3)].fg,
            Color::Rgb(10, 10, 10),
            "描く順番どおりに重なる"
        );
    }

    #[test]
    fn compose_board_stacks_layers_in_order() {
        // ボード20x10セル、1セル10x20ピクセル => 200x200ピクセル
        let board = Rect::new(3, 5, 20, 10);
        let red = solid(4, 4, [255, 0, 0, 255]);
        let blue = solid(4, 4, [0, 0, 255, 255]);
        // 大きい円: 12x6セル=120x120ピクセル(ボード左上から)。小さい円: 4x2セル=40x40ピクセル
        let large = Rect::new(3, 5, 12, 6);
        let small = Rect::new(7, 7, 4, 2);
        let image = compose_board(board, (10, 20), &[(large, &red), (small, &blue)]);
        assert_eq!(image.dimensions(), (200, 200));
        assert_eq!(image.get_pixel(10, 10).0, [255, 0, 0, 255], "大きい円");
        assert_eq!(image.get_pixel(50, 50).0, [0, 0, 255, 255], "後の円が手前");
        assert_eq!(image.get_pixel(150, 150).0[3], 0, "円の無い所は透明");

        let image = compose_board(board, (10, 20), &[(small, &blue), (large, &red)]);
        assert_eq!(
            image.get_pixel(50, 50).0,
            [255, 0, 0, 255],
            "並び順どおりに重なる"
        );
    }

    #[test]
    fn compose_board_keeps_aspect_ratio_and_centers_image() {
        // 4x1セル=40x20ピクセルの枠に正方形の画像を置くと、20x20で横方向の中央に来る
        let board = Rect::new(0, 0, 4, 1);
        let green = solid(8, 8, [0, 255, 0, 255]);
        let image = compose_board(board, (10, 20), &[(board, &green)]);
        assert_eq!(image.dimensions(), (40, 20));
        assert_eq!(image.get_pixel(5, 10).0[3], 0, "左の余白は透明");
        assert_eq!(image.get_pixel(20, 10).0, [0, 255, 0, 255]);
        assert_eq!(image.get_pixel(35, 10).0[3], 0, "右の余白は透明");
    }

    #[test]
    fn compose_board_shows_lower_layer_through_transparent_pixels() {
        // 上の画像の透明な部分(円の外側の角)からは下の円が見える
        let board = Rect::new(0, 0, 2, 1);
        let below = solid(2, 2, [255, 0, 0, 255]);
        let mut above = solid(2, 2, [0, 0, 255, 255]);
        above.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        let image = compose_board(board, (10, 20), &[(board, &below), (board, &above)]);
        assert_eq!(
            image.get_pixel(1, 1).0,
            [255, 0, 0, 255],
            "透明な角から下が見える"
        );
        assert_eq!(image.get_pixel(18, 18).0, [0, 0, 255, 255]);
    }

    #[test]
    fn fallback_renders_colored_circled_digit_in_rect() {
        // テスト環境(非TTY)では画像プロトコルを検出できず、テキスト表示になる
        let renderer = CircleRenderer::new();
        assert!(!renderer.uses_image());
        let backend = TestBackend::new(30, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let rect = Rect::new(4, 2, 8, 4);
        let circle = BoardCircle {
            rect,
            number: 12,
            color: [1, 2, 3],
        };
        terminal
            .draw(|frame| renderer.render_board(frame, frame.area(), &[circle]))
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
