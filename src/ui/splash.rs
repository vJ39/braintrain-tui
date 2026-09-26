use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};
use rust_embed::RustEmbed;
use std::sync::OnceLock;

#[derive(RustEmbed)]
#[folder = "assets/image/"]
struct ImageAssets;

/// 起動直後のタイトル画面の画像(assets/image/からの相対パス)
pub const TITLE_IMAGE_PATH: &str = "title.jpeg";
/// TTR(リズムゲーム)の曲選択前に挟むスプラッシュ画面の画像
pub const TTR_SPLASH_IMAGE_PATH: &str = "ttr_splash.jpeg";

/// 画像プロトコル非対応端末で画像の代わりに出すテキスト
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FallbackText {
    /// 見出し(太字)
    pub title: &'static str,
    /// 見出しの下に出す一文
    pub subtitle: &'static str,
}

/// タイトル画面のフォールバック表示
pub const TITLE_FALLBACK: FallbackText = FallbackText {
    title: "B R A I N T R A I N",
    subtitle: "A TERMINAL-BASED COGNITIVE TRAINER",
};

/// TTRスプラッシュ画面のフォールバック表示
pub const TTR_FALLBACK: FallbackText = FallbackText {
    title: "T T R",
    subtitle: "TAP TAP REVOLUTION!!",
};

/// 端末の画像プロトコル検出結果。検出は応答待ちのタイムアウトがあり得るため、
/// スプラッシュ画面が複数あっても端末への問い合わせはプロセス内で1回だけにする
static PICKER: OnceLock<Option<Picker>> = OnceLock::new();

/// 端末の画像プロトコル検出結果(プロセス内で1回だけ問い合わせる)。メニューのアイコンでも共用する
pub(crate) fn picker() -> Option<&'static Picker> {
    PICKER
        .get_or_init(|| Picker::from_query_stdio().ok())
        .as_ref()
}

/// assets/image/に埋め込んだ画像を読み込んでデコードする。無い・デコードできなければNone
pub(crate) fn load_embedded_image(image_path: &str) -> Option<image::DynamicImage> {
    let file = ImageAssets::get(image_path)?;
    image::load_from_memory(&file.data).ok()
}

/// スプラッシュ画面(画像1枚の全画面表示)の描画方式。端末が画像プロトコルに
/// 対応していない/検出に失敗した/画像を読めない場合はFallback(テキスト描画)になる
pub enum SplashRenderer {
    Image {
        protocol: Box<StatefulProtocol>,
        /// 元画像の大きさ(px)。アスペクト比を保って中央に配置する計算に使う
        size: (u32, u32),
    },
    Fallback(FallbackText),
}

impl SplashRenderer {
    /// image_pathはassets/image/からの相対パス。fallbackは画像を出せない時の表示
    pub fn new(image_path: &str, fallback: FallbackText) -> Self {
        match Self::try_load_image(image_path) {
            Some((protocol, size)) => SplashRenderer::Image {
                protocol: Box::new(protocol),
                size,
            },
            None => SplashRenderer::Fallback(fallback),
        }
    }

    fn try_load_image(image_path: &str) -> Option<(StatefulProtocol, (u32, u32))> {
        // 画像が無ければ端末への問い合わせ自体を行わない
        let file = ImageAssets::get(image_path)?;
        let picker = picker()?;
        let dyn_img = image::load_from_memory(&file.data).ok()?;
        let size = (dyn_img.width(), dyn_img.height());
        Some((picker.new_resize_protocol(dyn_img), size))
    }

    /// 画像プロトコル非対応環境でテキスト表示(Fallback)になっているか。
    /// 曲選択画面のように背景の上に別のパネルを重ねる場合、Fallbackの文言が
    /// パネルの裏に完全に隠れないよう表示領域を分けるために使う
    pub fn is_fallback(&self) -> bool {
        matches!(self, SplashRenderer::Fallback(_))
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        match self {
            SplashRenderer::Image { protocol, size } => {
                let font_size = picker().map_or((1, 1), Picker::font_size);
                let target = centered_image_rect(area, *size, font_size);
                let image_widget = StatefulImage::default().resize(Resize::Fit(None));
                frame.render_stateful_widget(image_widget, target, protocol.as_mut());
            }
            SplashRenderer::Fallback(fallback) => render_fallback(frame, area, *fallback),
        }
    }
}

/// 画像(size, px)をareaの中央に、縦横比を保って収まる最大の大きさで配置するRect。
/// areaが空なら空のRectを返す
fn centered_image_rect(area: Rect, size: (u32, u32), font_size: (u16, u16)) -> Rect {
    if area.width == 0 || area.height == 0 {
        return Rect::new(area.x, area.y, 0, 0);
    }
    let (font_w, font_h) = (f64::from(font_size.0.max(1)), f64::from(font_size.1.max(1)));
    let aspect = f64::from(size.0.max(1)) / f64::from(size.1.max(1));
    // まず高さいっぱいで幅を求め、はみ出すなら幅いっぱいにして高さを縮める
    let full_height_width = (f64::from(area.height) * font_h * aspect / font_w).round() as u16;
    let (width, height) = if full_height_width <= area.width {
        (full_height_width.max(1), area.height)
    } else {
        let height = (f64::from(area.width) * font_w / aspect / font_h).round() as u16;
        (area.width, height.clamp(1, area.height))
    };
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// 高さがheight(areaに収まる範囲)で、幅はareaのまま上下中央に配置したRect
fn centered_rect_vertically(area: Rect, height: u16) -> Rect {
    let height = height.min(area.height);
    Rect::new(
        area.x,
        area.y + (area.height - height) / 2,
        area.width,
        height,
    )
}

fn render_fallback(frame: &mut Frame, area: Rect, fallback: FallbackText) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            fallback.title,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(fallback.subtitle),
        Line::from(""),
        Line::from("PRESS [ENTER] TO START"),
        Line::from(""),
    ];
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let target = centered_rect_vertically(inner, lines.len() as u16);
    let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(paragraph, target);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn title_image_asset_is_embedded() {
        assert!(
            ImageAssets::get(TITLE_IMAGE_PATH).is_some(),
            "assets/image/title.jpegが埋め込まれていること"
        );
    }

    #[test]
    fn ttr_splash_image_asset_is_embedded() {
        assert!(
            ImageAssets::get(TTR_SPLASH_IMAGE_PATH).is_some(),
            "assets/image/ttr_splash.jpegが埋め込まれていること"
        );
    }

    #[test]
    fn embedded_splash_images_can_be_decoded() {
        for path in [TITLE_IMAGE_PATH, TTR_SPLASH_IMAGE_PATH] {
            let file = ImageAssets::get(path).expect("埋め込まれていること");
            assert!(
                image::load_from_memory(&file.data).is_ok(),
                "{path}: 画像としてデコードできること"
            );
        }
    }

    /// フォールバック表示を描画し、全セルを1つの文字列にして返す
    fn rendered_fallback_text(fallback: FallbackText) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_fallback(frame, frame.area(), fallback))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn render_fallback_does_not_panic() {
        rendered_fallback_text(TITLE_FALLBACK);
        rendered_fallback_text(TTR_FALLBACK);
    }

    #[test]
    fn render_fallback_is_vertically_centered_in_a_tall_area() {
        // 画像プロトコル非対応環境ではフォールバック文言が画面上部に張り付き、
        // 下にある曲選択パネル等との間に大きな空白ができてレイアウトがズレて見えていた。
        // 縦に余裕がある画面では上下の余白がほぼ均等になること
        let backend = TestBackend::new(80, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_fallback(frame, frame.area(), TTR_FALLBACK))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        // 枠(Borders::ALL)自体は常にarea全体を覆うので、判定対象は中身のテキストの行
        let text_rows: Vec<u16> = (0..buffer.area.height)
            .filter(|&y| {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                line.contains("TTR") || line.contains("REVOLUTION") || line.contains("PRESS")
            })
            .collect();
        let top = *text_rows.iter().min().expect("文言が描画されていること");
        let bottom = *text_rows.iter().max().expect("文言が描画されていること");
        let top_margin = top;
        let bottom_margin = buffer.area.height - 1 - bottom;
        assert!(
            top_margin.abs_diff(bottom_margin) <= 1,
            "上下の余白がほぼ均等: top={top_margin} bottom={bottom_margin}"
        );
    }

    #[test]
    fn render_fallback_draws_the_given_title_and_subtitle() {
        let title = rendered_fallback_text(TITLE_FALLBACK);
        assert!(title.contains(TITLE_FALLBACK.title));
        assert!(title.contains(TITLE_FALLBACK.subtitle));

        let ttr = rendered_fallback_text(TTR_FALLBACK);
        assert!(ttr.contains(TTR_FALLBACK.title));
        assert!(ttr.contains(TTR_FALLBACK.subtitle));
        assert!(
            !ttr.contains(TITLE_FALLBACK.title),
            "TTR用のフォールバックにタイトル画面の見出しを出さない"
        );
    }

    #[test]
    fn fallback_texts_of_title_and_ttr_differ() {
        assert_ne!(TITLE_FALLBACK.title, TTR_FALLBACK.title);
    }

    // --- センタリング配置(centered_image_rect) ---

    #[test]
    fn wide_image_is_centered_vertically_with_full_width() {
        // 横長画像(1600x900)を正方形寄りのareaに収める: 幅いっぱいにし、上下に余白ができて中央になる
        let area = Rect::new(0, 0, 100, 100);
        let rect = centered_image_rect(area, (1600, 900), (10, 20));
        assert_eq!(rect.x, 0);
        assert_eq!(rect.width, 100);
        assert!(rect.height < 100, "上下に余白ができる");
        // 上下の余白が均等(中央寄せ)
        let bottom_margin = area.height - rect.y - rect.height;
        assert!(
            rect.y.abs_diff(bottom_margin) <= 1,
            "上下の余白がほぼ均等: top={} bottom={}",
            rect.y,
            bottom_margin
        );
    }

    #[test]
    fn tall_image_is_centered_horizontally_with_full_height() {
        // 縦長画像(900x1600)を横長のareaに収める: 高さいっぱいにし、左右に余白ができて中央になる
        let area = Rect::new(0, 0, 200, 50);
        let rect = centered_image_rect(area, (900, 1600), (10, 20));
        assert_eq!(rect.y, 0);
        assert_eq!(rect.height, 50);
        assert!(rect.width < 200, "左右に余白ができる");
        let right_margin = area.width - rect.x - rect.width;
        assert!(
            rect.x.abs_diff(right_margin) <= 1,
            "左右の余白がほぼ均等: left={} right={}",
            rect.x,
            right_margin
        );
    }

    #[test]
    fn zero_size_area_yields_empty_rect_without_panicking() {
        let rect = centered_image_rect(Rect::new(0, 0, 0, 0), (100, 100), (10, 20));
        assert!(rect.width == 0 || rect.height == 0);
    }

    #[test]
    fn missing_image_path_falls_back_to_text() {
        let renderer = SplashRenderer::new("does-not-exist.jpeg", TTR_FALLBACK);
        match renderer {
            SplashRenderer::Fallback(fallback) => assert_eq!(fallback, TTR_FALLBACK),
            SplashRenderer::Image { .. } => panic!("存在しない画像ではFallbackになるはず"),
        }
    }

    #[test]
    fn splash_renderer_render_does_not_panic_regardless_of_variant() {
        // 実行環境によりImage/Fallbackどちらになるかは変わるが、
        // どちらであってもrenderがパニックしないことを確認する
        for (path, fallback) in [
            (TITLE_IMAGE_PATH, TITLE_FALLBACK),
            (TTR_SPLASH_IMAGE_PATH, TTR_FALLBACK),
        ] {
            let mut renderer = SplashRenderer::new(path, fallback);
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    renderer.render(frame, area);
                })
                .unwrap();
        }
    }
}
