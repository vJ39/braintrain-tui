//! 「べー」スプラッシュ画面のコマ送りアニメーション(assets/image/beigoma_splash/)。
//!
//! ratatui-imageは静止画プロトコルしか扱えないため、動画をフレーム画像列に分解し、
//! result_sprite.rsと同じ「全コマ事前読み込み+経過時間からコマ番号を計算」方式で再生する。
//! Enterで開始するまで見ていられるよう、最後のコマまで行ったら最初に戻る(ループ)。
//! 画像プロトコル非対応・読み込み失敗時はフォールバック(テキスト)表示になる。

use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

use super::splash::{self, FallbackText};

/// アニメーションの各コマの画像(assets/image/からの相対パス)。この順にループする
pub const FRAME_PATHS: [&str; 25] = [
    "beigoma_splash/frame_001.jpg",
    "beigoma_splash/frame_002.jpg",
    "beigoma_splash/frame_003.jpg",
    "beigoma_splash/frame_004.jpg",
    "beigoma_splash/frame_005.jpg",
    "beigoma_splash/frame_006.jpg",
    "beigoma_splash/frame_007.jpg",
    "beigoma_splash/frame_008.jpg",
    "beigoma_splash/frame_009.jpg",
    "beigoma_splash/frame_010.jpg",
    "beigoma_splash/frame_011.jpg",
    "beigoma_splash/frame_012.jpg",
    "beigoma_splash/frame_013.jpg",
    "beigoma_splash/frame_014.jpg",
    "beigoma_splash/frame_015.jpg",
    "beigoma_splash/frame_016.jpg",
    "beigoma_splash/frame_017.jpg",
    "beigoma_splash/frame_018.jpg",
    "beigoma_splash/frame_019.jpg",
    "beigoma_splash/frame_020.jpg",
    "beigoma_splash/frame_021.jpg",
    "beigoma_splash/frame_022.jpg",
    "beigoma_splash/frame_023.jpg",
    "beigoma_splash/frame_024.jpg",
    "beigoma_splash/frame_025.jpg",
];

/// 1コマあたりの表示時間(result_sprite.rsと同じ)
pub const FRAME_DURATION: Duration = Duration::from_millis(350);

/// 経過時間から表示するコマ番号を求める。最後のコマの次は最初に戻る(ループ)
pub fn frame_index(elapsed: Duration) -> usize {
    let frames_elapsed = elapsed.as_nanos() / FRAME_DURATION.as_nanos();
    (frames_elapsed % FRAME_PATHS.len() as u128) as usize
}

/// 「べー」スプラッシュ画面のコマ送りアニメーション
pub struct AnimatedSplash {
    /// 端末の1セルの大きさ(px)
    font_size: (u16, u16),
    /// 元画像の大きさ(px、全コマ共通)
    image_size: (u32, u32),
    /// 読み込み済みの全コマ。1枚でも読めなければ空(フォールバック表示)
    frames: Vec<StatefulProtocol>,
    fallback: FallbackText,
    elapsed: Duration,
}

impl AnimatedSplash {
    pub fn new(fallback: FallbackText) -> Self {
        Self::with_picker(splash::picker().cloned(), fallback)
    }

    fn with_picker(picker: Option<Picker>, fallback: FallbackText) -> Self {
        let mut anim = Self {
            font_size: (1, 1),
            image_size: (1, 1),
            frames: Vec::new(),
            fallback,
            elapsed: Duration::ZERO,
        };
        let Some(picker) = picker else {
            return anim;
        };
        let images: Option<Vec<_>> = FRAME_PATHS
            .iter()
            .map(|path| splash::load_embedded_image(path))
            .collect();
        // 1コマでも欠けるとアニメーションにならないので、全コマ読めた時だけ描く
        let Some(images) = images else {
            return anim;
        };
        if let Some(first) = images.first() {
            anim.image_size = (first.width(), first.height());
        }
        anim.font_size = picker.font_size();
        anim.frames = images
            .into_iter()
            .map(|image| picker.new_resize_protocol(image))
            .collect();
        anim
    }

    /// 画像を描けない(フォールバック表示になる)か
    #[cfg(test)]
    pub fn is_fallback(&self) -> bool {
        self.frames.is_empty()
    }

    /// 経過時間を0に戻す(Screen::BeigomaSplashに入るたびに呼ぶ)
    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }

    /// 経過時間をdtだけ進める
    pub fn tick(&mut self, dt: Duration) {
        self.elapsed = self.elapsed.saturating_add(dt);
    }

    /// 現在表示するコマ番号
    #[cfg(test)]
    pub fn current_frame(&self) -> usize {
        frame_index(self.elapsed)
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        if self.frames.is_empty() {
            splash::render_fallback(frame, area, self.fallback);
            return;
        }
        let index = frame_index(self.elapsed);
        let Some(protocol) = self.frames.get_mut(index) else {
            return;
        };
        let target = splash::centered_image_rect(area, self.image_size, self.font_size);
        let widget = StatefulImage::default().resize(Resize::Fit(None));
        frame.render_stateful_widget(widget, target, protocol);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use ratatui_image::picker::ProtocolType;

    fn test_picker() -> Picker {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        picker
    }

    #[test]
    fn frame_index_advances_and_loops_back_to_the_first_frame() {
        assert_eq!(frame_index(Duration::ZERO), 0);
        assert_eq!(frame_index(FRAME_DURATION), 1);
        assert_eq!(frame_index(FRAME_DURATION * 2), 2);
        let last = FRAME_PATHS.len() as u32 - 1;
        assert_eq!(frame_index(FRAME_DURATION * last), last as usize);
        assert_eq!(
            frame_index(FRAME_DURATION * (last + 1)),
            0,
            "最後のコマの次は最初に戻る(ループ)"
        );
    }

    #[test]
    fn without_picker_is_fallback() {
        let anim = AnimatedSplash::with_picker(None, splash::BEIGOMA_FALLBACK);
        assert!(anim.is_fallback());
    }

    #[test]
    fn with_picker_loads_every_frame() {
        let anim = AnimatedSplash::with_picker(Some(test_picker()), splash::BEIGOMA_FALLBACK);
        assert!(!anim.is_fallback());
    }

    #[test]
    fn tick_advances_the_current_frame() {
        let mut anim = AnimatedSplash::with_picker(Some(test_picker()), splash::BEIGOMA_FALLBACK);
        assert_eq!(anim.current_frame(), 0);
        anim.tick(FRAME_DURATION);
        assert_eq!(anim.current_frame(), 1);
    }

    #[test]
    fn reset_returns_to_the_first_frame() {
        let mut anim = AnimatedSplash::with_picker(Some(test_picker()), splash::BEIGOMA_FALLBACK);
        anim.tick(FRAME_DURATION * 3);
        assert_eq!(anim.current_frame(), 3);
        anim.reset();
        assert_eq!(anim.current_frame(), 0);
    }

    #[test]
    fn render_does_not_panic_with_or_without_a_picker() {
        for picker in [None, Some(test_picker())] {
            let mut anim = AnimatedSplash::with_picker(picker, splash::BEIGOMA_FALLBACK);
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    anim.render(frame, area);
                })
                .unwrap();
        }
    }

    #[test]
    fn all_frames_are_embedded_and_decodable() {
        for path in FRAME_PATHS {
            let image = splash::load_embedded_image(path);
            assert!(image.is_some(), "{path}が埋め込まれ、デコードできること");
        }
    }
}
