use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/image/"]
struct ImageAssets;

const TITLE_IMAGE_PATH: &str = "title.jpeg";

/// タイトル画面の描画方式。端末が画像プロトコルに対応していない/検出に
/// 失敗した場合はFallback(TUI描画のみ)になる
pub enum SplashRenderer {
    Image(Box<StatefulProtocol>),
    Fallback,
}

impl SplashRenderer {
    pub fn new() -> Self {
        match Self::try_load_image() {
            Some(protocol) => SplashRenderer::Image(Box::new(protocol)),
            None => SplashRenderer::Fallback,
        }
    }

    fn try_load_image() -> Option<StatefulProtocol> {
        let picker = Picker::from_query_stdio().ok()?;
        let file = ImageAssets::get(TITLE_IMAGE_PATH)?;
        let dyn_img = image::load_from_memory(&file.data).ok()?;
        Some(picker.new_resize_protocol(dyn_img))
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        match self {
            SplashRenderer::Image(protocol) => {
                let image_widget = StatefulImage::default().resize(Resize::Crop(None));
                frame.render_stateful_widget(image_widget, area, protocol.as_mut());
            }
            SplashRenderer::Fallback => render_fallback(frame, area),
        }
    }
}

fn render_fallback(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "B R A I N T R A I N",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("A TERMINAL-BASED COGNITIVE TRAINER"),
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
    fn render_fallback_does_not_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_fallback(frame, frame.area()))
            .unwrap();
    }

    #[test]
    fn splash_renderer_render_does_not_panic_regardless_of_variant() {
        // 実行環境によりImage/Fallbackどちらになるかは変わるが、
        // どちらであってもrenderがパニックしないことを確認する
        let mut renderer = SplashRenderer::new();
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
