//! 暗算スピードの問題式・選択肢を大きく表示するための、文字列→画像の変換と描画器。
//!
//! 問題式や答えは毎回変わる可変長の文字列なので、事前生成の画像は使えない。
//! 実行時にシステムのTrueTypeフォントをfontdueでラスタライズして画像にする。
//! フォントはライセンスの都合でバイナリに埋め込まず、macOSのシステムフォントを読み込む。
//! 読み込めない環境では画像表示を諦め、呼び出し側がテキスト表示に切り替える。

use std::cell::RefCell;
use std::sync::OnceLock;

use fontdue::{Font, FontSettings};
use image::imageops::{self, FilterType};
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;

/// 出題文字に使うフォント(macOS標準のヒラギノ角ゴシック W6)
const FONT_PATH: &str = "/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc";

/// ラスタライズ時の文字サイズ(px)。表示時は描画範囲に合わせて縮小するので、
/// 大きな端末で拡大してもぼやけにくいよう大きめにしておく
const RASTER_PX: f32 = 160.0;

/// 文字列の左右に空ける余白(文字サイズに対する比率)。画像の端に文字が接しないようにする
const TEXT_PADDING_RATIO: f32 = 0.25;

/// フォントの読み込み関数。テストでフォントが無い環境を再現できるよう差し替え可能にする
type FontLoader = fn() -> Option<&'static Font>;

/// システムフォントを読み込む。重い処理(数MBの読み込みと解析)なので、プロセス内で1回だけ行う
pub fn system_font() -> Option<&'static Font> {
    static FONT: OnceLock<Option<Font>> = OnceLock::new();
    FONT.get_or_init(|| {
        let data = std::fs::read(FONT_PATH).ok()?;
        Font::from_bytes(data, FontSettings::default()).ok()
    })
    .as_ref()
}

/// 文字列を1行の画像にする。黒い文字(RGB=0,0,0、アルファ=ラスタライズ結果の濃さ)を
/// 透明な背景に描く。各文字はフォントの共通のベースラインにそろえ、送り幅で横に並べる。
/// 画像の高さはフォントのascent〜descentで決まり、文字列の中身によらず一定
pub fn rasterize_text(font: &Font, text: &str, px: f32) -> RgbaImage {
    // 行の高さ情報が無いフォントでも描けるよう、一般的な比率で代用する
    let (ascent, descent) = font
        .horizontal_line_metrics(px)
        .map(|m| (m.ascent, m.descent))
        .unwrap_or((px * 0.88, -px * 0.12));
    // 画像の上端からベースラインまでの距離(px)
    let baseline = ascent.ceil() as i32;
    let height = (ascent - descent).ceil().max(1.0) as u32;
    let padding = (px * TEXT_PADDING_RATIO).round() as i32;

    // 各文字をラスタライズし、送り幅を積算して左端の位置を決める
    let mut pen_x = 0.0_f32;
    let mut placed = Vec::new();
    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, px);
        let left = padding + pen_x.round() as i32 + metrics.xmin;
        // ymin=ベースラインから見たビットマップ下端の高さ(上向きが正)。上端はその分だけ上
        let top = baseline - (metrics.ymin + metrics.height as i32);
        placed.push((left, top, metrics.width, bitmap));
        pen_x += metrics.advance_width;
    }
    let width = (2 * padding + pen_x.ceil() as i32).max(1) as u32;

    let mut image = RgbaImage::new(width, height);
    for (left, top, glyph_width, bitmap) in placed {
        if glyph_width == 0 {
            continue;
        }
        for (i, &alpha) in bitmap.iter().enumerate() {
            if alpha == 0 {
                continue;
            }
            let x = left + (i % glyph_width) as i32;
            let y = top + (i / glyph_width) as i32;
            if x < 0 || y < 0 || x as u32 >= width || y as u32 >= height {
                continue;
            }
            // 隣の文字と重なる所は濃い方を残す
            let pixel = image.get_pixel_mut(x as u32, y as u32);
            pixel.0 = [0, 0, 0, pixel.0[3].max(alpha)];
        }
    }
    image
}

/// cols x rows セル分の画像を背景色(不透明)で塗り、文字の画像を縦横比を保ったまま
/// 収まる最大の大きさで中央に重ねる。透明な部分が残らないので、端末ごとの透明の扱いの違い
/// (sixelでは白になる等)に左右されない
fn compose_text_image(
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
    // 文字の輪郭のアルファが254になることがあるため、255にそろえる
    for pixel in canvas.pixels_mut() {
        pixel.0[3] = 255;
    }
    canvas
}

/// 最大 max_cols x max_rows セルの範囲に、画像の縦横比(ピクセル換算)を保ったまま収まる
/// 最大のセル数(幅, 高さ)。セル数は切り捨てる。font_sizeは1セルのピクセル数(幅, 高さ)
pub fn fit_cells(
    max_cols: u16,
    max_rows: u16,
    font_size: (u16, u16),
    glyph_size: (u32, u32),
) -> (u16, u16) {
    let (glyph_width, glyph_height) = (u64::from(glyph_size.0), u64::from(glyph_size.1));
    if glyph_width == 0 || glyph_height == 0 {
        return (0, 0);
    }
    let cell_width = u64::from(font_size.0.max(1));
    let cell_height = u64::from(font_size.1.max(1));
    let max_width = u64::from(max_cols) * cell_width;
    let max_height = u64::from(max_rows) * cell_height;
    // 幅で決まるか高さで決まるかを、縦横比の比較(掛け算)で判定する
    let (width, height) = if max_width * glyph_height <= max_height * glyph_width {
        (max_width, max_width * glyph_height / glyph_width)
    } else {
        (max_height * glyph_width / glyph_height, max_height)
    };
    // width/heightは範囲の幅・高さ(ピクセル)以下なので、セル数も範囲に収まる
    ((width / cell_width) as u16, (height / cell_height) as u16)
}

/// 直前に端末向けに作った画像。文字列・背景色・描画範囲が同じなら再エンコードを省く
struct EncodedCache {
    text: String,
    bg: [u8; 3],
    area: Rect,
    protocol: StatefulProtocol,
}

/// 文字列を画像で大きく描く描画器。表示する文字列1か所につき1つ持つ
/// (ラスタライズ結果とエンコード結果を1件ずつキャッシュするため)
pub struct TextImageRenderer {
    picker: Option<Picker>,
    load_font: FontLoader,
    /// 直前にラスタライズした(文字列, 画像)。描画範囲の計算に画像の寸法が要るので、
    /// 同じ文字列の間は毎フレームラスタライズし直さないよう持っておく
    glyph: RefCell<Option<(String, RgbaImage)>>,
    cache: RefCell<Option<EncodedCache>>,
    /// テスト用: ラスタライズした回数
    #[cfg(test)]
    raster_count: std::cell::Cell<usize>,
    /// テスト用: 画像を新規エンコードした回数
    #[cfg(test)]
    encode_count: std::cell::Cell<usize>,
}

impl TextImageRenderer {
    pub fn new(picker: Option<Picker>) -> Self {
        Self::with_font_loader(picker, system_font)
    }

    fn with_font_loader(picker: Option<Picker>, load_font: FontLoader) -> Self {
        Self {
            picker,
            load_font,
            glyph: RefCell::new(None),
            cache: RefCell::new(None),
            #[cfg(test)]
            raster_count: std::cell::Cell::new(0),
            #[cfg(test)]
            encode_count: std::cell::Cell::new(0),
        }
    }

    /// テスト用: フォントが読み込めない環境を再現する
    #[cfg(test)]
    pub(super) fn without_font(picker: Option<Picker>) -> Self {
        Self::with_font_loader(picker, || None)
    }

    #[cfg(test)]
    pub(super) fn encode_count(&self) -> usize {
        self.encode_count.get()
    }

    #[cfg(test)]
    fn raster_count(&self) -> usize {
        self.raster_count.get()
    }

    /// 直近に画像を描いた範囲(テストで描画位置を確かめる用)
    #[cfg(test)]
    pub(super) fn last_area(&self) -> Option<Rect> {
        self.cache.borrow().as_ref().map(|cached| cached.area)
    }

    /// 画像プロトコルが使える端末か(false=テキスト表示)
    #[cfg(test)]
    pub(super) fn uses_image(&self) -> bool {
        self.picker.is_some()
    }

    /// textの画像を最大 max_cols x max_rows セルに収めた時のセル数(幅, 高さ)。
    /// 画像プロトコルが使えない/フォントが読めない/収まらない場合はNone
    pub fn fit(&self, text: &str, max_cols: u16, max_rows: u16) -> Option<(u16, u16)> {
        let picker = self.picker.as_ref()?;
        let glyph_size = self.with_glyph(text, |glyph| glyph.dimensions())?;
        let (cols, rows) = fit_cells(max_cols, max_rows, picker.font_size(), glyph_size);
        (cols > 0 && rows > 0).then_some((cols, rows))
    }

    /// textの画像をareaにぴったり描く(areaはfitで求めたセル数にする)。
    /// 描けなかった場合はfalse(呼び出し側がテキスト表示に切り替える)
    pub fn draw(&self, frame: &mut Frame, area: Rect, text: &str, bg: [u8; 3]) -> bool {
        let Some(picker) = &self.picker else {
            return false;
        };
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return false;
        }
        let mut cache = self.cache.borrow_mut();
        let needs_regen = !matches!(
            cache.as_ref(),
            Some(cached) if cached.text == text && cached.bg == bg && cached.area == area
        );
        if needs_regen {
            let Some(composed) = self.with_glyph(text, |glyph| {
                compose_text_image(glyph, area.width, area.height, picker.font_size(), bg)
            }) else {
                return false;
            };
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(composed));
            *cache = Some(EncodedCache {
                text: text.to_string(),
                bg,
                area,
                protocol,
            });
            #[cfg(test)]
            self.encode_count.set(self.encode_count.get() + 1);
        }
        let Some(cached) = cache.as_mut() else {
            return false;
        };
        frame.render_stateful_widget(StatefulImage::default(), area, &mut cached.protocol);
        true
    }

    /// textをラスタライズした画像をfに渡す。同じ文字列の間は前回の画像を使い回す。
    /// フォントが読み込めない場合はNone
    fn with_glyph<T>(&self, text: &str, f: impl FnOnce(&RgbaImage) -> T) -> Option<T> {
        let mut glyph = self.glyph.borrow_mut();
        if !matches!(glyph.as_ref(), Some((cached, _)) if cached == text) {
            let font = (self.load_font)()?;
            *glyph = Some((text.to_string(), rasterize_text(font, text, RASTER_PX)));
            #[cfg(test)]
            self.raster_count.set(self.raster_count.get() + 1);
        }
        glyph.as_ref().map(|(_, image)| f(image))
    }
}

/// 端末の画像プロトコルを調べる。sixel/kitty/iTerm2のどれかが使える時だけSome。
/// テストでは端末に問い合わせず、常にテキスト表示にする(実行環境で結果が変わらないように)
pub fn detect_picker() -> Option<Picker> {
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
    use ratatui::Terminal;

    /// 暗算スピードで表示しうる全ての文字(数字・演算子・記号・半角スペース)
    const CALC_SYMBOLS: &str = "0123456789 +-×÷=?";

    /// テスト用のフォント。読み込めない環境(CI等)ではNoneを返し、呼び出し側はテストを飛ばす
    fn test_font() -> Option<&'static Font> {
        let font = system_font();
        if font.is_none() {
            eprintln!("{FONT_PATH} を読み込めないため、フォントを使うテストを飛ばす");
        }
        font
    }

    /// 文字が描かれた(アルファ>0の)ピクセル数
    fn ink_count(image: &RgbaImage) -> usize {
        image.pixels().filter(|p| p.0[3] > 0).count()
    }

    /// 文字が描かれた一番下の行
    fn lowest_ink_row(image: &RgbaImage) -> Option<u32> {
        (0..image.height())
            .rev()
            .find(|&y| (0..image.width()).any(|x| image.get_pixel(x, y).0[3] > 128))
    }

    /// 文字が描かれた一番上の行
    fn highest_ink_row(image: &RgbaImage) -> Option<u32> {
        (0..image.height()).find(|&y| (0..image.width()).any(|x| image.get_pixel(x, y).0[3] > 128))
    }

    fn sixel_picker() -> Picker {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        picker
    }

    #[test]
    fn every_calc_symbol_is_rasterized_into_a_non_empty_image() {
        let Some(font) = test_font() else { return };
        let image = rasterize_text(font, CALC_SYMBOLS, 40.0);
        assert!(image.width() > 0 && image.height() > 0);
        assert!(ink_count(&image) > 0);
        // 空白以外の各文字は、1文字だけでも文字が描かれる(フォントに無い字で空になっていない)
        for ch in CALC_SYMBOLS.chars().filter(|c| *c != ' ') {
            let single = rasterize_text(font, &ch.to_string(), 40.0);
            assert!(ink_count(&single) > 20, "{ch:?} が描かれること");
        }
    }

    #[test]
    fn rasterized_text_is_black_ink_on_transparent_background() {
        let Some(font) = test_font() else { return };
        let image = rasterize_text(font, "12 + 8 = ?", 40.0);
        assert!(
            image.pixels().all(|p| p.0[..3] == [0, 0, 0]),
            "色は全て黒(濃さはアルファで表す)"
        );
        let (w, h) = image.dimensions();
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert_eq!(image.get_pixel(x, y).0[3], 0, "隅({x},{y})は透明");
        }
    }

    #[test]
    fn rasterizing_the_same_text_gives_the_same_image() {
        let Some(font) = test_font() else { return };
        let a = rasterize_text(font, "81 ÷ 9 = ?", 40.0);
        let b = rasterize_text(font, "81 ÷ 9 = ?", 40.0);
        assert_eq!(a, b, "同じ文字列なら同じ画像(キャッシュの前提)");
        let c = rasterize_text(font, "81 × 9 = ?", 40.0);
        assert_ne!(a, c, "文字列が違えば画像も違う");
    }

    #[test]
    fn longer_text_is_wider_but_the_height_is_the_same() {
        let Some(font) = test_font() else { return };
        let one = rasterize_text(font, "7", 40.0);
        let three = rasterize_text(font, "123", 40.0);
        assert!(
            three.width() > one.width() * 2,
            "文字数が増えると横に伸びる"
        );
        assert_eq!(one.height(), three.height(), "高さは文字列によらず一定");
    }

    #[test]
    fn digits_sit_on_a_common_baseline() {
        let Some(font) = test_font() else { return };
        let bottoms: Vec<u32> = "0123456789"
            .chars()
            .map(|ch| lowest_ink_row(&rasterize_text(font, &ch.to_string(), 40.0)).unwrap())
            .collect();
        let (min, max) = (bottoms.iter().min().unwrap(), bottoms.iter().max().unwrap());
        assert!(max - min <= 1, "数字の下端がそろう: {bottoms:?}");
        // 横棒の「-」は数字の下端より上に描かれる(ベースラインからの高さが反映される)
        let minus = rasterize_text(font, "-", 40.0);
        assert!(lowest_ink_row(&minus).unwrap() + 5 < *min);
        assert!(highest_ink_row(&minus).unwrap() > 5);
    }

    #[test]
    fn characters_are_laid_out_left_to_right_without_overlap() {
        let Some(font) = test_font() else { return };
        // 「1」と「1 1」: 2つ目の「1」は1つ目より右に、重ならずに描かれる
        let image = rasterize_text(font, "1 1", 40.0);
        let inked_columns: Vec<u32> = (0..image.width())
            .filter(|&x| (0..image.height()).any(|y| image.get_pixel(x, y).0[3] > 128))
            .collect();
        let gaps = inked_columns.windows(2).filter(|w| w[1] > w[0] + 1).count();
        assert_eq!(gaps, 1, "2つの文字の間に隙間が1つある");
    }

    #[test]
    fn composed_text_image_is_opaque_background_with_black_ink() {
        let Some(font) = test_font() else { return };
        let glyph = rasterize_text(font, "12 + 8 = ?", RASTER_PX);
        let bg = [200, 230, 240];
        let image = compose_text_image(&glyph, 30, 4, (10, 20), bg);
        assert_eq!(image.dimensions(), (300, 80));
        assert!(
            image.pixels().all(|p| p.0[3] == 255),
            "透明な部分は残らない(背景色で埋まる)"
        );
        assert_eq!(image.get_pixel(0, 0).0, [200, 230, 240, 255], "隅は背景色");
        let black = image
            .pixels()
            .filter(|p| p.0[..3].iter().all(|&c| c <= 50))
            .count();
        assert!(black > 500, "文字の黒が描かれる({black})");
    }

    #[test]
    fn composing_an_empty_area_does_not_panic() {
        let glyph = RgbaImage::new(10, 10);
        assert_eq!(
            compose_text_image(&glyph, 0, 3, (10, 20), [1, 2, 3]).dimensions(),
            (0, 60)
        );
        let empty = RgbaImage::new(0, 0);
        let image = compose_text_image(&empty, 2, 1, (10, 20), [1, 2, 3]);
        assert!(image.pixels().all(|p| p.0 == [1, 2, 3, 255]));
    }

    #[test]
    fn fit_cells_keeps_aspect_ratio_within_the_bounds() {
        // 横長の画像(4:1)を横に余裕のある範囲へ: 高さいっぱい
        assert_eq!(fit_cells(100, 5, (10, 20), (400, 100)), (40, 5));
        // 横に余裕が無い範囲へ: 幅いっぱい、高さは切り捨て
        assert_eq!(fit_cells(20, 5, (10, 20), (400, 100)), (20, 2));
        // 範囲からはみ出さない
        for (cols, rows) in [(1, 1), (3, 7), (80, 2), (13, 11)] {
            let (w, h) = fit_cells(cols, rows, (8, 17), (321, 123));
            assert!(w <= cols && h <= rows);
        }
    }

    #[test]
    fn fit_cells_of_an_empty_image_is_zero() {
        assert_eq!(fit_cells(40, 10, (10, 20), (0, 100)), (0, 0));
        assert_eq!(fit_cells(40, 10, (10, 20), (100, 0)), (0, 0));
        assert_eq!(fit_cells(0, 10, (10, 20), (100, 100)), (0, 0));
    }

    #[test]
    fn renderer_without_picker_falls_back_to_text() {
        let renderer = TextImageRenderer::new(None);
        assert!(!renderer.uses_image());
        assert_eq!(renderer.fit("12", 40, 5), None);
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        terminal
            .draw(|frame| {
                assert!(!renderer.draw(frame, Rect::new(0, 0, 10, 3), "12", [255, 255, 255]));
            })
            .unwrap();
    }

    #[test]
    fn renderer_without_font_falls_back_to_text() {
        let renderer = TextImageRenderer::without_font(Some(sixel_picker()));
        assert!(renderer.uses_image());
        assert_eq!(
            renderer.fit("12", 40, 5),
            None,
            "フォントが無ければ画像にしない"
        );
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        terminal
            .draw(|frame| {
                assert!(!renderer.draw(frame, Rect::new(0, 0, 10, 3), "12", [255, 255, 255]));
            })
            .unwrap();
        assert_eq!(renderer.encode_count(), 0);
    }

    fn draw_text(
        terminal: &mut Terminal<TestBackend>,
        renderer: &TextImageRenderer,
        text: &str,
        cols: u16,
    ) {
        terminal
            .draw(|frame| {
                let (w, h) = renderer.fit(text, cols, 4).expect("画像で描けること");
                assert!(renderer.draw(frame, Rect::new(0, 0, w, h), text, [255, 255, 255]));
            })
            .unwrap();
    }

    #[test]
    fn image_path_reuses_encoding_while_content_is_unchanged() {
        if test_font().is_none() {
            return;
        }
        let renderer = TextImageRenderer::new(Some(sixel_picker()));
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        for _ in 0..5 {
            draw_text(&mut terminal, &renderer, "12 + 8 = ?", 40);
        }
        assert_eq!(
            renderer.raster_count(),
            1,
            "同じ文字列の間はラスタライズし直さない"
        );
        assert_eq!(
            renderer.encode_count(),
            1,
            "同じ文字列の間は再エンコードしない"
        );
    }

    #[test]
    fn image_path_reencodes_when_text_or_area_changes() {
        if test_font().is_none() {
            return;
        }
        let renderer = TextImageRenderer::new(Some(sixel_picker()));
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        draw_text(&mut terminal, &renderer, "12", 40);
        draw_text(&mut terminal, &renderer, "15", 40);
        draw_text(&mut terminal, &renderer, "15", 40);
        assert_eq!(renderer.raster_count(), 2);
        assert_eq!(renderer.encode_count(), 2, "文字列が変わったら作り直す");
        // 描画範囲が変わった(端末のリサイズ等)ら、同じ文字列でも作り直す
        draw_text(&mut terminal, &renderer, "15", 10);
        assert_eq!(renderer.raster_count(), 2);
        assert_eq!(renderer.encode_count(), 3);
    }

    #[test]
    fn draw_into_an_empty_area_returns_false() {
        if test_font().is_none() {
            return;
        }
        let renderer = TextImageRenderer::new(Some(sixel_picker()));
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        terminal
            .draw(|frame| {
                assert!(!renderer.draw(frame, Rect::new(0, 0, 0, 3), "12", [255, 255, 255]));
            })
            .unwrap();
    }
}
