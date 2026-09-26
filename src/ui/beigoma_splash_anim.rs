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
pub const FRAME_PATHS: [&str; 89] = [
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
    "beigoma_splash/frame_026.jpg",
    "beigoma_splash/frame_027.jpg",
    "beigoma_splash/frame_028.jpg",
    "beigoma_splash/frame_029.jpg",
    "beigoma_splash/frame_030.jpg",
    "beigoma_splash/frame_031.jpg",
    "beigoma_splash/frame_032.jpg",
    "beigoma_splash/frame_033.jpg",
    "beigoma_splash/frame_034.jpg",
    "beigoma_splash/frame_035.jpg",
    "beigoma_splash/frame_036.jpg",
    "beigoma_splash/frame_037.jpg",
    "beigoma_splash/frame_038.jpg",
    "beigoma_splash/frame_039.jpg",
    "beigoma_splash/frame_040.jpg",
    "beigoma_splash/frame_041.jpg",
    "beigoma_splash/frame_042.jpg",
    "beigoma_splash/frame_043.jpg",
    "beigoma_splash/frame_044.jpg",
    "beigoma_splash/frame_045.jpg",
    "beigoma_splash/frame_046.jpg",
    "beigoma_splash/frame_047.jpg",
    "beigoma_splash/frame_048.jpg",
    "beigoma_splash/frame_049.jpg",
    "beigoma_splash/frame_050.jpg",
    "beigoma_splash/frame_051.jpg",
    "beigoma_splash/frame_052.jpg",
    "beigoma_splash/frame_053.jpg",
    "beigoma_splash/frame_054.jpg",
    "beigoma_splash/frame_055.jpg",
    "beigoma_splash/frame_056.jpg",
    "beigoma_splash/frame_057.jpg",
    "beigoma_splash/frame_058.jpg",
    "beigoma_splash/frame_059.jpg",
    "beigoma_splash/frame_060.jpg",
    "beigoma_splash/frame_061.jpg",
    "beigoma_splash/frame_062.jpg",
    "beigoma_splash/frame_063.jpg",
    "beigoma_splash/frame_064.jpg",
    "beigoma_splash/frame_065.jpg",
    "beigoma_splash/frame_066.jpg",
    "beigoma_splash/frame_067.jpg",
    "beigoma_splash/frame_068.jpg",
    "beigoma_splash/frame_069.jpg",
    "beigoma_splash/frame_070.jpg",
    "beigoma_splash/frame_071.jpg",
    "beigoma_splash/frame_072.jpg",
    "beigoma_splash/frame_073.jpg",
    "beigoma_splash/frame_074.jpg",
    "beigoma_splash/frame_075.jpg",
    "beigoma_splash/frame_076.jpg",
    "beigoma_splash/frame_077.jpg",
    "beigoma_splash/frame_078.jpg",
    "beigoma_splash/frame_079.jpg",
    "beigoma_splash/frame_080.jpg",
    "beigoma_splash/frame_081.jpg",
    "beigoma_splash/frame_082.jpg",
    "beigoma_splash/frame_083.jpg",
    "beigoma_splash/frame_084.jpg",
    "beigoma_splash/frame_085.jpg",
    "beigoma_splash/frame_086.jpg",
    "beigoma_splash/frame_087.jpg",
    "beigoma_splash/frame_088.jpg",
    "beigoma_splash/frame_089.jpg",
];

/// 1コマあたりの表示時間(10fps。元動画から100ms間隔でフレームを切り出している)
pub const FRAME_DURATION: Duration = Duration::from_millis(100);

/// 経過時間から表示するコマ番号を求める。最後のコマの次は最初に戻る(ループ)
pub fn frame_index(elapsed: Duration) -> usize {
    let frames_elapsed = elapsed.as_nanos() / FRAME_DURATION.as_nanos();
    (frames_elapsed % FRAME_PATHS.len() as u128) as usize
}

/// 「べー」スプラッシュ画面のコマ送りアニメーション
pub struct AnimatedSplash {
    /// 端末の1セルの大きさ(px)は描画のたびに取得する(SplashRendererと同じ。
    /// 生成時に1回だけ保存すると、実機で画像が小さく表示される不具合があった: #157)
    picker: Option<Picker>,
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
            picker: picker.clone(),
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
        let font_size = self.picker.as_ref().map_or((1, 1), Picker::font_size);
        let target = splash::centered_image_rect(area, self.image_size, font_size);
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
    fn frames_are_89_images_from_frame_001_to_frame_089_in_order() {
        assert_eq!(FRAME_PATHS.len(), 89);
        for (i, path) in FRAME_PATHS.iter().enumerate() {
            assert_eq!(
                *path,
                format!("beigoma_splash/frame_{:03}.jpg", i + 1),
                "{i}番目のコマはframe_{:03}.jpg",
                i + 1
            );
        }
    }

    #[test]
    fn frame_duration_is_100ms_for_10fps() {
        assert_eq!(FRAME_DURATION, Duration::from_millis(100));
    }

    #[test]
    fn one_loop_takes_89_frames_of_100ms() {
        // 8.9秒で1周し、最初のコマに戻る
        assert_eq!(frame_index(Duration::from_millis(8_899)), 88);
        assert_eq!(frame_index(Duration::from_millis(8_900)), 0);
        // コマの途中(1コマ目の表示中)はまだ次のコマへ進まない
        assert_eq!(frame_index(Duration::from_millis(99)), 0);
        assert_eq!(frame_index(Duration::from_millis(100)), 1);
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
