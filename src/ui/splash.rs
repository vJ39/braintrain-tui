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

fn picker() -> Option<&'static Picker> {
    PICKER
        .get_or_init(|| Picker::from_query_stdio().ok())
        .as_ref()
}

/// スプラッシュ画面(画像1枚の全画面表示)の描画方式。端末が画像プロトコルに
/// 対応していない/検出に失敗した/画像を読めない場合はFallback(テキスト描画)になる
pub enum SplashRenderer {
    Image(Box<StatefulProtocol>),
    Fallback(FallbackText),
}

impl SplashRenderer {
    /// image_pathはassets/image/からの相対パス。fallbackは画像を出せない時の表示
    pub fn new(image_path: &str, fallback: FallbackText) -> Self {
        match Self::try_load_image(image_path) {
            Some(protocol) => SplashRenderer::Image(Box::new(protocol)),
            None => SplashRenderer::Fallback(fallback),
        }
    }

    fn try_load_image(image_path: &str) -> Option<StatefulProtocol> {
        // 画像が無ければ端末への問い合わせ自体を行わない
        let file = ImageAssets::get(image_path)?;
        let picker = picker()?;
        let dyn_img = image::load_from_memory(&file.data).ok()?;
        Some(picker.new_resize_protocol(dyn_img))
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        match self {
            SplashRenderer::Image(protocol) => {
                let image_widget = StatefulImage::default().resize(Resize::Crop(None));
                frame.render_stateful_widget(image_widget, area, protocol.as_mut());
            }
            SplashRenderer::Fallback(fallback) => render_fallback(frame, area, *fallback),
        }
    }
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
    ];
    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, area);
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

    #[test]
    fn missing_image_path_falls_back_to_text() {
        let renderer = SplashRenderer::new("does-not-exist.jpeg", TTR_FALLBACK);
        match renderer {
            SplashRenderer::Fallback(fallback) => assert_eq!(fallback, TTR_FALLBACK),
            SplashRenderer::Image(_) => panic!("存在しない画像ではFallbackになるはず"),
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
