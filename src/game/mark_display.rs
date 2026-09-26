//! 正誤の記号(◯/✗)を、指定エリアいっぱいに大きく表示する描画器。
//! イロピッタン・記憶ゲームの正誤表示で共通に使う。
//!
//! 記号は黒・透明背景の画像(maru.png/batsu.png)を背景色の上に重ねて大きく表示する。
//! 正解時の画像はゲームごとに差し替えられる(オイカケは専用のイラスト)。不正解時の✗は全ゲーム共通。
//! sixel/kitty/iTerm2の画像プロトコルに対応した端末では画像を、非対応の端末では
//! 通常サイズの黒い文字をテキストで表示する。
//! 画像を縦横比を保ってセルの矩形に収める処理(glyph_area/compose_glyph_image)は、
//! イロピッタンの出題文字の画像表示でも使う。

use std::cell::RefCell;

use image::imageops::{self, FilterType};
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;

/// 正解の記号(テキスト表示の時の文字)
pub const CORRECT_MARK: &str = "◯";
/// 不正解の記号(テキスト表示の時の文字)
pub const INCORRECT_MARK: &str = "✗";

/// 正誤の記号の画像。黒・透明背景の正方形
const MARU_PNG: &[u8] = include_bytes!("../../assets/image/reaction/maru.png");
const BATSU_PNG: &[u8] = include_bytes!("../../assets/image/reaction/batsu.png");

/// 正誤に対応する記号の文字。正解=◯、不正解=✗
pub fn mark_text(is_correct: bool) -> &'static str {
    if is_correct {
        CORRECT_MARK
    } else {
        INCORRECT_MARK
    }
}

/// 記号の画像(PNG/JPEG等のバイト列)を読み込む。読めない場合はNone
fn load_mark_image(bytes: &[u8]) -> Option<RgbaImage> {
    let image = image::load_from_memory(bytes).ok()?;
    Some(image.to_rgba8())
}

/// inner(セル単位)の中央に置く、画像の縦横比(ピクセル換算)を保ったまま収まる
/// 最大の範囲。セル数は切り捨てるので、画像の縦横比に最も近い整数セルの矩形になる。
/// font_sizeは1セルのピクセル数(幅, 高さ)、glyph_sizeは画像のピクセル数(幅, 高さ)
pub fn glyph_area(inner: Rect, font_size: (u16, u16), glyph_size: (u32, u32)) -> Rect {
    let (glyph_width, glyph_height) = (u64::from(glyph_size.0), u64::from(glyph_size.1));
    if glyph_width == 0 || glyph_height == 0 {
        return Rect::new(inner.x, inner.y, 0, 0);
    }
    let cell_width = u64::from(font_size.0.max(1));
    let cell_height = u64::from(font_size.1.max(1));
    let inner_width = u64::from(inner.width) * cell_width;
    let inner_height = u64::from(inner.height) * cell_height;
    // 幅で決まるか高さで決まるかを、縦横比の比較(掛け算)で判定する
    let (width, height) = if inner_width * glyph_height <= inner_height * glyph_width {
        (inner_width, inner_width * glyph_height / glyph_width)
    } else {
        (inner_height * glyph_width / glyph_height, inner_height)
    };
    // width/heightはinnerの幅・高さ(ピクセル)以下なので、セル数もinnerに収まる
    let cols = (width / cell_width) as u16;
    let rows = (height / cell_height) as u16;
    Rect::new(
        inner.x + (inner.width - cols) / 2,
        inner.y + (inner.height - rows) / 2,
        cols,
        rows,
    )
}

/// cols x rows セル分のピクセル画像を背景色(不透明)で塗り、画像を縦横比を保ったまま
/// 収まる最大の大きさで中央に重ねる。透明な部分が残らないので、端末ごとの透明の扱いの違い
/// (sixelでは白になる等)に左右されない
pub fn compose_glyph_image(
    glyph: &RgbaImage,
    cols: u16,
    rows: u16,
    font_size: (u16, u16),
    bg: [u8; 3],
) -> RgbaImage {
    let width = u32::from(cols) * u32::from(font_size.0.max(1));
    let height = u32::from(rows) * u32::from(font_size.1.max(1));
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([bg[0], bg[1], bg[2], 255]));
    if width == 0 || height == 0 || glyph.width() == 0 || glyph.height() == 0 {
        return canvas;
    }
    let scale = f64::min(
        f64::from(width) / f64::from(glyph.width()),
        f64::from(height) / f64::from(glyph.height()),
    );
    let glyph_width = ((f64::from(glyph.width()) * scale).round() as u32).clamp(1, width);
    let glyph_height = ((f64::from(glyph.height()) * scale).round() as u32).clamp(1, height);
    let scaled = imageops::resize(glyph, glyph_width, glyph_height, FilterType::Triangle);
    imageops::overlay(
        &mut canvas,
        &scaled,
        i64::from((width - glyph_width) / 2),
        i64::from((height - glyph_height) / 2),
    );
    // 不透明な背景に重ねたので結果も不透明のはずだが、合成時の小数の切り捨てで
    // 輪郭のアルファが254になることがあるため、255にそろえる
    for pixel in canvas.pixels_mut() {
        pixel.0[3] = 255;
    }
    canvas
}

/// 画像プロトコル非対応の端末向け表示。通常サイズの黒い記号を上下中央に置く
fn render_mark_text(frame: &mut Frame, area: Rect, is_correct: bool, background: Color) {
    let vertical_padding = area.height.saturating_sub(1) / 2;
    let mut lines: Vec<Line> = (0..vertical_padding).map(|_| Line::from("")).collect();
    let mark_style = Style::default()
        .fg(Color::Black)
        .bg(background)
        .add_modifier(Modifier::BOLD);
    lines.push(Line::from(Span::styled(
        format!("  {}  ", mark_text(is_correct)),
        mark_style,
    )));
    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .style(Style::default().bg(background));
    frame.render_widget(paragraph, area);
}

/// 直前に作った記号の画像。正誤・背景色・描画範囲が同じなら再エンコードを省く
struct MarkCache {
    is_correct: bool,
    bg: [u8; 3],
    area: Rect,
    protocol: StatefulProtocol,
}

/// 正誤の記号の描画器。画像プロトコルが使える端末では記号を画像で大きく表示する
pub struct MarkRenderer {
    picker: Option<Picker>,
    /// 正解時に表示する画像のバイト列。既定はMARU_PNG
    correct_image: &'static [u8],
    /// 直前に読み込んだ記号の画像(正誤, 画像)。描画範囲の計算に画像の寸法が要るので、
    /// 同じ記号の間は毎フレーム画像を読み直さないよう持っておく
    glyph: RefCell<Option<(bool, RgbaImage)>>,
    cache: RefCell<Option<MarkCache>>,
}

impl Default for MarkRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkRenderer {
    /// 端末の画像プロトコルを調べて描画器を作る
    pub fn new() -> Self {
        Self::with_picker(detect_picker())
    }

    /// 画像プロトコルを指定して作る(None=テキスト表示)。
    /// 出題文字の画像と同じPickerを使い回す時や、テストで使う
    pub fn with_picker(picker: Option<Picker>) -> Self {
        Self::with_picker_and_correct_image(picker, MARU_PNG)
    }

    /// 端末の画像プロトコルを調べ、正解時の画像を差し替えた描画器を作る。
    /// correct_imageは画像のバイト列(PNG/JPEG等)。不正解時の✗は差し替えない
    pub fn with_correct_image(correct_image: &'static [u8]) -> Self {
        Self::with_picker_and_correct_image(detect_picker(), correct_image)
    }

    /// 画像プロトコルと正解時の画像を指定して作る(None=テキスト表示)
    pub fn with_picker_and_correct_image(
        picker: Option<Picker>,
        correct_image: &'static [u8],
    ) -> Self {
        Self {
            picker,
            correct_image,
            glyph: RefCell::new(None),
            cache: RefCell::new(None),
        }
    }

    /// 正誤に対応する画像のバイト列。正解=差し替えた画像(既定は◯)、不正解=✗
    fn image_bytes(&self, is_correct: bool) -> &'static [u8] {
        if is_correct {
            self.correct_image
        } else {
            BATSU_PNG
        }
    }

    /// 画像プロトコルを使うか(false=テキスト表示)
    pub fn uses_image(&self) -> bool {
        self.picker.is_some()
    }

    /// areaをbackgroundで塗り、その中央に正誤の記号を大きく描く。
    /// 画像プロトコルが使え、背景色がRGBの時は画像で(画像の背景と周りのセルを同じ色にするため)、
    /// それ以外はテキストで描く
    pub fn render(&self, frame: &mut Frame, area: Rect, is_correct: bool, background: Color) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        frame.render_widget(
            Block::default().style(Style::default().bg(background)),
            area,
        );
        let drawn_as_image = match background {
            Color::Rgb(r, g, b) => self.render_image(frame, area, is_correct, [r, g, b]),
            _ => false,
        };
        if !drawn_as_image {
            render_mark_text(frame, area, is_correct, background);
        }
    }

    /// areaの中央に記号の画像を描く。画像プロトコルが使えない/画像が読めない/
    /// 描く場所が無い場合は何もせずfalse(呼び出し側がテキスト表示に切り替える)
    fn render_image(&self, frame: &mut Frame, area: Rect, is_correct: bool, bg: [u8; 3]) -> bool {
        let Some(picker) = &self.picker else {
            return false;
        };
        // 描画範囲は画像の縦横比で決まるので、先に画像を読み込む
        let mut glyph_cache = self.glyph.borrow_mut();
        if !matches!(glyph_cache.as_ref(), Some((cached, _)) if *cached == is_correct) {
            let Some(image) = load_mark_image(self.image_bytes(is_correct)) else {
                return false;
            };
            *glyph_cache = Some((is_correct, image));
        }
        let Some((_, glyph)) = glyph_cache.as_ref() else {
            return false;
        };
        let drawn = glyph_area(area, picker.font_size(), glyph.dimensions());
        if drawn.is_empty() {
            return false;
        }
        let mut cache = self.cache.borrow_mut();
        let needs_regen = !matches!(
            cache.as_ref(),
            Some(cached)
                if cached.is_correct == is_correct && cached.bg == bg && cached.area == drawn
        );
        if needs_regen {
            let composed =
                compose_glyph_image(glyph, drawn.width, drawn.height, picker.font_size(), bg);
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(composed));
            *cache = Some(MarkCache {
                is_correct,
                bg,
                area: drawn,
                protocol,
            });
        }
        let Some(cached) = cache.as_mut() else {
            return false;
        };
        frame.render_stateful_widget(StatefulImage::default(), drawn, &mut cached.protocol);
        true
    }

    /// 直前に画像で描いた記号(正誤, 背景色, 描画範囲)。テストで描画内容を確かめる用
    #[cfg(test)]
    pub fn cached_mark(&self) -> Option<(bool, [u8; 3], Rect)> {
        self.cache
            .borrow()
            .as_ref()
            .map(|cached| (cached.is_correct, cached.bg, cached.area))
    }

    /// 正誤に対応して使う画像のバイト列。テストで差し替えを確かめる用
    #[cfg(test)]
    pub fn mark_image_bytes(&self, is_correct: bool) -> &'static [u8] {
        self.image_bytes(is_correct)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn halfblocks_picker() -> Picker {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        picker
    }

    /// width x height の画面のareaに正誤の記号を描き、バッファを返す
    fn render_mark(
        renderer: &MarkRenderer,
        (width, height): (u16, u16),
        area: Rect,
        is_correct: bool,
        background: Color,
    ) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| renderer.render(frame, area, is_correct, background))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn area_cells(area: Rect) -> impl Iterator<Item = (u16, u16)> {
        (area.y..area.bottom()).flat_map(move |y| (area.x..area.right()).map(move |x| (x, y)))
    }

    fn area_text(buffer: &Buffer, area: Rect) -> String {
        area_cells(area)
            .map(|pos| buffer[pos].symbol().to_string())
            .collect()
    }

    #[test]
    fn mark_text_is_maru_for_correct_and_batsu_for_incorrect() {
        assert_eq!(mark_text(true), CORRECT_MARK);
        assert_eq!(mark_text(false), INCORRECT_MARK);
        assert_eq!(CORRECT_MARK, "◯");
        assert_eq!(INCORRECT_MARK, "✗");
    }

    #[test]
    fn mark_images_are_embedded_as_squares() {
        for (bytes, label) in [(MARU_PNG, "◯"), (BATSU_PNG, "✗")] {
            let image = load_mark_image(bytes)
                .unwrap_or_else(|| panic!("「{label}」の画像が埋め込まれていること"));
            assert_eq!(image.dimensions(), (512, 512), "正方形");
        }
    }

    #[test]
    fn load_mark_image_returns_none_for_broken_bytes() {
        assert!(load_mark_image(b"not an image").is_none());
        assert!(load_mark_image(&[]).is_none());
    }

    #[test]
    fn default_renderer_uses_maru_for_correct_and_batsu_for_incorrect() {
        // 差し替えていないゲーム(イロピッタン等)は従来通り◯/✗の画像を使う
        for renderer in [
            MarkRenderer::new(),
            MarkRenderer::default(),
            MarkRenderer::with_picker(None),
            MarkRenderer::with_picker(Some(halfblocks_picker())),
        ] {
            // constは使う箇所ごとにアドレスが変わり得るので、中身で比べる
            assert!(renderer.mark_image_bytes(true) == MARU_PNG);
            assert!(renderer.mark_image_bytes(false) == BATSU_PNG);
        }
    }

    #[test]
    fn custom_correct_image_replaces_only_correct_mark() {
        let custom = leaked_png(300, 100, [255, 0, 0]);
        for renderer in [
            MarkRenderer::with_correct_image(custom),
            MarkRenderer::with_picker_and_correct_image(None, custom),
            MarkRenderer::with_picker_and_correct_image(Some(halfblocks_picker()), custom),
        ] {
            assert!(renderer.mark_image_bytes(true) == custom);
            assert!(
                renderer.mark_image_bytes(false) == BATSU_PNG,
                "不正解の画像は全ゲーム共通のまま"
            );
        }
        // テストでは端末に問い合わせず、常にテキスト表示にする
        assert!(!MarkRenderer::with_correct_image(custom).uses_image());
        assert!(
            MarkRenderer::with_picker_and_correct_image(Some(halfblocks_picker()), custom)
                .uses_image()
        );
    }

    /// width x height の単色のPNGを作り、'staticなバイト列にする(テスト専用)
    fn leaked_png(width: u32, height: u32, rgb: [u8; 3]) -> &'static [u8] {
        let image = RgbaImage::from_pixel(width, height, Rgba([rgb[0], rgb[1], rgb[2], 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        Box::leak(bytes.into_boxed_slice())
    }

    /// 描画範囲のピクセル換算の縦横比(幅/高さ)
    fn pixel_aspect(area: Rect, font_size: (u16, u16)) -> f64 {
        f64::from(area.width) * f64::from(font_size.0)
            / (f64::from(area.height) * f64::from(font_size.1))
    }

    #[test]
    fn image_mode_draws_custom_correct_image_and_default_batsu() {
        // 横長(4:1)の真っ赤な画像を正解の画像にすると、正解の時だけその画像で描かれる
        let custom = leaked_png(400, 100, [255, 0, 0]);
        let renderer =
            MarkRenderer::with_picker_and_correct_image(Some(halfblocks_picker()), custom);
        let area = Rect::new(0, 0, 60, 16);
        let background = Color::Rgb(40, 190, 70);

        let buffer = render_mark(&renderer, (60, 16), area, true, background);
        let (cached_correct, _, drawn) = renderer.cached_mark().expect("画像で描かれていること");
        assert!(cached_correct);
        let aspect = pixel_aspect(drawn, (10, 20));
        assert!(
            (3.0..=5.0).contains(&aspect),
            "正解は差し替えた横長の画像: {aspect}"
        );
        let has_red = area_cells(drawn).any(|pos| {
            let cell = &buffer[pos];
            [cell.fg, cell.bg].contains(&Color::Rgb(255, 0, 0))
        });
        assert!(has_red, "差し替えた画像の色で描かれる");

        // 不正解は従来通りの✗(正方形・赤を含まない)
        let buffer = render_mark(&renderer, (60, 16), area, false, background);
        let (cached_correct, _, drawn) = renderer.cached_mark().expect("画像で描かれていること");
        assert!(!cached_correct);
        let aspect = pixel_aspect(drawn, (10, 20));
        assert!(
            (0.8..=1.25).contains(&aspect),
            "不正解は正方形の✗: {aspect}"
        );
        let has_red = area_cells(drawn).any(|pos| {
            let cell = &buffer[pos];
            [cell.fg, cell.bg].contains(&Color::Rgb(255, 0, 0))
        });
        assert!(!has_red, "不正解では差し替えた画像を使わない");
    }

    #[test]
    fn image_mode_default_renderer_draws_square_maru() {
        // 差し替えていない描画器は、正解でも正方形の◯で描く
        let renderer = MarkRenderer::with_picker(Some(halfblocks_picker()));
        let area = Rect::new(0, 0, 60, 16);
        render_mark(&renderer, (60, 16), area, true, Color::Rgb(40, 190, 70));
        let (_, _, drawn) = renderer.cached_mark().expect("画像で描かれていること");
        let aspect = pixel_aspect(drawn, (10, 20));
        assert!((0.8..=1.25).contains(&aspect), "正方形の◯: {aspect}");
    }

    #[test]
    fn opaque_custom_image_is_composed_without_transparency() {
        // 背景込みの不透明な画像(JPEG等)でも合成でき、透明な部分は残らない
        let custom = leaked_png(512, 512, [10, 20, 200]);
        let glyph = load_mark_image(custom).unwrap();
        let image = compose_glyph_image(&glyph, 40, 10, (10, 20), [240, 210, 0]);
        assert_eq!(image.dimensions(), (400, 200));
        assert!(image.pixels().all(|p| p.0[3] == 255));
        assert_eq!(
            image.get_pixel(0, 0).0,
            [240, 210, 0, 255],
            "左右の余白は背景色"
        );
        assert_eq!(
            image.get_pixel(200, 100).0,
            [10, 20, 200, 255],
            "中央は画像の色"
        );
    }

    #[test]
    fn composed_mark_image_has_black_glyph_on_background() {
        for (bytes, is_correct) in [(MARU_PNG, true), (BATSU_PNG, false)] {
            let glyph = load_mark_image(bytes).unwrap();
            let image = compose_glyph_image(&glyph, 20, 10, (10, 20), [240, 210, 0]);
            assert_eq!(image.dimensions(), (200, 200));
            assert!(
                image.pixels().all(|p| p.0[3] == 255),
                "透明な部分は残らない(背景色で埋まる)"
            );
            assert_eq!(image.get_pixel(0, 0).0, [240, 210, 0, 255], "隅は背景色");
            let black = image
                .pixels()
                .filter(|p| p.0[..3].iter().all(|&c| c <= 50))
                .count();
            assert!(black > 1000, "「{}」の黒が描かれる", mark_text(is_correct));
        }
    }

    #[test]
    fn glyph_area_is_centered_square_in_pixels() {
        const SQUARE: (u32, u32) = (512, 512);
        // 40x10セル、1セル10x20ピクセル => 400x200ピクセル。正方形の一辺は200ピクセル=20x10セル
        let inner = Rect::new(2, 3, 40, 10);
        let glyph = glyph_area(inner, (10, 20), SQUARE);
        assert_eq!((glyph.width, glyph.height), (20, 10));
        assert_eq!((glyph.x, glyph.y), (12, 3), "横方向の中央に置く");
        // 縦長のエリアでも収まる
        let tall = Rect::new(0, 0, 10, 30);
        let glyph = glyph_area(tall, (10, 20), SQUARE);
        assert_eq!((glyph.width, glyph.height), (10, 5));
        assert!(glyph.y >= tall.y && glyph.bottom() <= tall.bottom());
        // 空のエリアは空のまま
        assert!(glyph_area(Rect::new(0, 0, 0, 5), (10, 20), SQUARE).is_empty());
    }

    #[test]
    fn glyph_area_keeps_aspect_ratio_of_wide_glyph() {
        const WIDE: (u32, u32) = (1789, 512);
        // 幅で決まる場合: 400x200ピクセル => 幅400、高さ400*512/1789=114ピクセル => 40x5セル
        let inner = Rect::new(2, 3, 40, 10);
        let glyph = glyph_area(inner, (10, 20), WIDE);
        assert_eq!((glyph.width, glyph.height), (40, 5));
        assert_eq!((glyph.x, glyph.y), (2, 5), "縦方向の中央に置く");
        // 高さで決まる場合: 1000x100ピクセル => 高さ100、幅100*1789/512=349ピクセル => 34x5セル
        let flat = Rect::new(0, 0, 100, 5);
        let glyph = glyph_area(flat, (10, 20), WIDE);
        assert_eq!((glyph.width, glyph.height), (34, 5));
        assert_eq!((glyph.x, glyph.y), (33, 0), "横方向の中央に置く");
        // 正方形の画像より横に広い範囲を取る
        let square = glyph_area(flat, (10, 20), (512, 512));
        assert!(glyph.width > square.width);
    }

    #[test]
    fn glyph_area_fits_inside_inner_and_follows_aspect_ratio() {
        let font_sizes = [(10, 20), (8, 16), (7, 15)];
        let glyphs = [(512, 512), (1380, 512), (1789, 512), (512, 1380)];
        for inner in [
            Rect::new(1, 2, 58, 14),
            Rect::new(0, 0, 13, 40),
            Rect::new(5, 5, 200, 3),
        ] {
            for font_size in font_sizes {
                for glyph_size in glyphs {
                    let area = glyph_area(inner, font_size, glyph_size);
                    assert!(area.x >= inner.x && area.y >= inner.y);
                    assert!(area.right() <= inner.right() && area.bottom() <= inner.bottom());
                    // ピクセル換算の縦横比が画像に近い(誤差はセルの切り捨て分だけ)
                    let (cw, ch) = (u32::from(font_size.0), u32::from(font_size.1));
                    let px_w = u32::from(area.width) * cw;
                    let px_h = u32::from(area.height) * ch;
                    let inner_w = u32::from(inner.width) * cw;
                    let inner_h = u32::from(inner.height) * ch;
                    let scale = f64::min(
                        f64::from(inner_w) / f64::from(glyph_size.0),
                        f64::from(inner_h) / f64::from(glyph_size.1),
                    );
                    let ideal_w = f64::from(glyph_size.0) * scale;
                    let ideal_h = f64::from(glyph_size.1) * scale;
                    assert!(f64::from(px_w) <= ideal_w + 1e-6 && ideal_w < f64::from(px_w + cw));
                    assert!(f64::from(px_h) <= ideal_h + 1e-6 && ideal_h < f64::from(px_h + ch));
                }
            }
        }
    }

    #[test]
    fn glyph_area_is_empty_for_empty_glyph() {
        assert!(glyph_area(Rect::new(0, 0, 40, 10), (10, 20), (0, 512)).is_empty());
        assert!(glyph_area(Rect::new(0, 0, 40, 10), (10, 20), (512, 0)).is_empty());
    }

    #[test]
    fn test_environment_uses_text_fallback() {
        // テストでは端末に問い合わせず、常にテキスト表示にする
        assert!(!MarkRenderer::new().uses_image());
        assert!(MarkRenderer::with_picker(Some(halfblocks_picker())).uses_image());
    }

    #[test]
    fn text_fallback_draws_black_mark_filling_area_background() {
        let renderer = MarkRenderer::with_picker(None);
        let area = Rect::new(3, 2, 20, 7);
        for (is_correct, mark, other) in [
            (true, CORRECT_MARK, INCORRECT_MARK),
            (false, INCORRECT_MARK, CORRECT_MARK),
        ] {
            let buffer = render_mark(&renderer, (30, 12), area, is_correct, Color::Blue);
            let text = area_text(&buffer, area);
            assert!(text.contains(mark), "「{mark}」が描かれる: {text:?}");
            assert!(!text.contains(other), "反対の記号は描かない");
            let cell = area_cells(area)
                .map(|pos| &buffer[pos])
                .find(|c| c.symbol() == mark)
                .unwrap();
            assert_eq!(cell.fg, Color::Black, "記号は黒");
            // エリアの四隅まで背景色で塗る
            for pos in [
                (area.x, area.y),
                (area.right() - 1, area.y),
                (area.x, area.bottom() - 1),
                (area.right() - 1, area.bottom() - 1),
            ] {
                assert_eq!(buffer[pos].bg, Color::Blue, "{pos:?}は背景色");
            }
            // エリアの外は塗らない
            assert_eq!(buffer[(0, 0)].bg, Color::Reset);
            assert_eq!(buffer[(29, 11)].bg, Color::Reset);
        }
    }

    #[test]
    fn image_mode_draws_mark_image_centered_on_rgb_background() {
        let renderer = MarkRenderer::with_picker(Some(halfblocks_picker()));
        let area = Rect::new(3, 2, 30, 8);
        for is_correct in [true, false] {
            let background = Color::Rgb(40, 190, 70);
            let buffer = render_mark(&renderer, (40, 12), area, is_correct, background);
            let (cached_correct, bg, drawn) =
                renderer.cached_mark().expect("画像で描かれていること");
            assert_eq!((cached_correct, bg), (is_correct, [40, 190, 70]));
            assert!(!drawn.is_empty());
            assert!(drawn.x >= area.x && drawn.y >= area.y);
            assert!(drawn.right() <= area.right() && drawn.bottom() <= area.bottom());
            // 正方形の画像なので、横長のエリアでは横方向の中央に置き、左右は背景色で塗る
            assert!(drawn.x > area.x, "横方向の中央に置く");
            assert_eq!(buffer[(area.x, area.y)].bg, Color::Rgb(40, 190, 70));
            // 画像で描いた時は記号をテキストで重ねない
            let text = area_text(&buffer, area);
            assert!(!text.contains(CORRECT_MARK) && !text.contains(INCORRECT_MARK));
        }
    }

    #[test]
    fn image_mode_with_named_background_falls_back_to_text() {
        // 画像の背景はRGBで塗る必要があるので、RGBでない背景色の時はテキストで描く
        let renderer = MarkRenderer::with_picker(Some(halfblocks_picker()));
        let area = Rect::new(0, 0, 20, 6);
        let buffer = render_mark(&renderer, (20, 6), area, true, Color::Red);
        assert!(renderer.cached_mark().is_none());
        assert!(area_text(&buffer, area).contains(CORRECT_MARK));
    }

    #[test]
    fn render_does_not_panic_with_image_protocols_or_tiny_areas() {
        for protocol in [
            ProtocolType::Halfblocks,
            ProtocolType::Sixel,
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
        ] {
            let mut picker = Picker::from_fontsize((10, 20));
            picker.set_protocol_type(protocol);
            let renderer = MarkRenderer::with_picker(Some(picker));
            let background = Color::Rgb(230, 50, 50);
            for is_correct in [true, false] {
                for (width, height) in [(60u16, 20u16), (12, 9), (4, 4), (1, 1)] {
                    let area = Rect::new(0, 0, width, height);
                    render_mark(&renderer, (width, height), area, is_correct, background);
                }
                // 画面からはみ出すエリアでもパニックしない
                let overflow = Rect::new(5, 2, 20, 10);
                render_mark(&renderer, (10, 5), overflow, is_correct, background);
                // 空のエリア
                let empty = Rect::new(0, 0, 0, 0);
                render_mark(&renderer, (10, 5), empty, is_correct, background);
            }
        }
        let text = MarkRenderer::with_picker(None);
        render_mark(&text, (1, 1), Rect::new(0, 0, 1, 1), false, Color::Red);
    }
}
