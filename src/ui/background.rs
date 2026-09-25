//! TTR(曲選択)・ゲームのプレイ中以外の全画面に共通で敷く背景画像(assets/image/background.jpeg)。
//!
//! 画像プロトコル(sixel/kitty/iTerm2)に対応した端末では、起動時に1回だけ読み込んで
//! `StatefulProtocol`を保持し、以後は同じものを使い回す(画面の大きさが変わった時だけ
//! ratatui-image側で作り直される)。非対応の端末・読み込めない場合は何も描かない。

use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

use super::splash;

/// 背景画像(assets/image/からの相対パス)
pub const BACKGROUND_IMAGE_PATH: &str = "background.jpeg";

/// 画像プロトコル(sixel/kitty/iTerm2)が使える端末ならそのpicker。端末への問い合わせは
/// スプラッシュ画面と共用する(プロセス内で1回だけ)。ハーフブロック描画は画像プロトコル
/// 非対応として扱い、背景は描かない。テストでは端末に問い合わせない
fn detect_picker() -> Option<Picker> {
    if cfg!(test) {
        return None;
    }
    splash::picker()
        .filter(|picker| {
            matches!(
                picker.protocol_type(),
                ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2
            )
        })
        .cloned()
}

/// 全画面共通の背景画像の描画
pub struct BackgroundRenderer {
    /// 読み込み済みの背景画像。画像を描けない時はNone(何も描かない)
    protocol: Option<Box<StatefulProtocol>>,
}

impl BackgroundRenderer {
    /// 端末の画像プロトコルを検出して背景画像を読み込む
    pub fn new() -> Self {
        Self::with_picker(detect_picker())
    }

    /// pickerを指定して読み込む。Noneなら画像を読まず、何も描かない
    pub fn with_picker(picker: Option<Picker>) -> Self {
        let protocol = picker.and_then(|picker| {
            let image = splash::load_embedded_image(BACKGROUND_IMAGE_PATH)?;
            Some(Box::new(picker.new_resize_protocol(image)))
        });
        Self { protocol }
    }

    /// 背景画像を描けない(何も描かない)か
    #[cfg(test)]
    pub fn is_fallback(&self) -> bool {
        self.protocol.is_none()
    }

    /// 背景画像をareaいっぱいに敷く(はみ出す部分は切り取る)。画像を描けなければ何もしない
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let Some(protocol) = self.protocol.as_mut() else {
            return;
        };
        if area.width == 0 || area.height == 0 {
            return;
        }
        let widget = StatefulImage::default().resize(Resize::Crop(None));
        frame.render_stateful_widget(widget, area, protocol.as_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    /// 端末に問い合わせないpicker(1セル10x20px)
    fn test_picker(protocol: ProtocolType) -> Picker {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(protocol);
        picker
    }

    /// backgroundをwidth x heightの画面全体に描いたバッファ(ratatui側で組み立てた内容)
    fn rendered_buffer(background: &mut BackgroundRenderer, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let completed = terminal
            .draw(|frame| {
                let area = frame.area();
                background.render(frame, area);
            })
            .unwrap();
        completed.buffer.clone()
    }

    #[test]
    fn background_image_is_embedded_and_decodes() {
        let image = splash::load_embedded_image(BACKGROUND_IMAGE_PATH)
            .expect("assets/image/background.jpegが埋め込まれていてデコードできること");
        assert!(image.width() > 0 && image.height() > 0);
    }

    #[test]
    fn new_does_not_use_an_image_protocol_in_tests() {
        // テストでは端末に問い合わせず、背景は描かない
        assert!(BackgroundRenderer::new().is_fallback());
    }

    #[test]
    fn without_picker_nothing_is_drawn() {
        let mut background = BackgroundRenderer::with_picker(None);
        assert!(background.is_fallback());
        let buffer = rendered_buffer(&mut background, 40, 12);
        assert_eq!(
            buffer,
            Buffer::empty(buffer.area),
            "何も描かない(既存の黒背景のまま)"
        );
    }

    #[test]
    fn with_picker_the_image_covers_the_whole_area() {
        let mut background =
            BackgroundRenderer::with_picker(Some(test_picker(ProtocolType::Halfblocks)));
        assert!(!background.is_fallback());
        let buffer = rendered_buffer(&mut background, 40, 12);
        // 四隅まで画像で埋まる(画面いっぱいに敷く)
        for (x, y) in [(0, 0), (39, 0), (0, 11), (39, 11)] {
            assert_ne!(
                buffer[(x, y)],
                ratatui::buffer::Cell::default(),
                "({x},{y})も画像"
            );
        }
    }

    #[test]
    fn image_protocol_puts_the_data_in_the_top_left_cell_and_skips_the_rest() {
        // 画像プロトコルでは左上のセルに画像データが入り、残りのセルは出力対象外(skip)になる。
        // UIを重ねる範囲はApp側でClearしてskipを外す前提(app.rsのテストで確認)
        let mut background =
            BackgroundRenderer::with_picker(Some(test_picker(ProtocolType::Iterm2)));
        let buffer = rendered_buffer(&mut background, 40, 12);
        assert!(
            buffer[(0, 0)].symbol().starts_with('\x1b'),
            "左上に画像データ"
        );
        assert!(buffer[(39, 11)].skip, "画像の範囲のセルはskip");
    }

    #[test]
    fn rendering_into_an_empty_area_does_not_panic() {
        let mut background =
            BackgroundRenderer::with_picker(Some(test_picker(ProtocolType::Halfblocks)));
        let backend = TestBackend::new(10, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                background.render(frame, Rect::new(0, 0, 0, 5));
                background.render(frame, Rect::new(0, 0, 5, 0));
            })
            .unwrap();
    }
}
