//! リザルト画面に表示するキャラクターのコマ送りアニメーション(assets/image/result_sprite/)。
//!
//! 画像プロトコル(sixel/kitty/iTerm2)に対応した端末では、起動時に4コマ全てを1回だけ読み込んで
//! コマごとに`StatefulProtocol`を保持し、以後は同じものを使い回す(描画エリアが変わった時だけ
//! ratatui-image側で作り直される)。非対応の端末・読み込めない場合は何も描かない。

use std::time::Duration;

use image::imageops::FilterType;
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

use super::splash;

/// アニメーションの各コマの画像(assets/image/からの相対パス)。この順にループする
pub const FRAME_PATHS: [&str; 4] = [
    "result_sprite/frame1.png",
    "result_sprite/frame2.png",
    "result_sprite/frame3.png",
    "result_sprite/frame4.png",
];

/// 1コマあたりの表示時間
pub const FRAME_DURATION: Duration = Duration::from_millis(350);

/// キャラクターとテキストカードの間に空けるセル数(重なり・密着を避ける)
const GAP_TO_CARD: u16 = 2;
/// 外枠の内側の左端から空けるセル数
const PADDING_LEFT: u16 = 1;
/// キャラクターを表示する最小の幅・高さ(セル数)。これより狭い余白には描かない
/// (小さすぎると絵が潰れてテキストの横で窮屈に見えるため。80桁の端末では表示しない)
pub const MIN_WIDTH: u16 = 10;
pub const MIN_HEIGHT: u16 = 6;

/// 経過時間から表示するコマ番号(0〜3)を求める。4コマ目の次は1コマ目に戻る(ループ)
pub fn frame_index(elapsed: Duration) -> usize {
    let frames_elapsed = elapsed.as_nanos() / FRAME_DURATION.as_nanos();
    (frames_elapsed % FRAME_PATHS.len() as u128) as usize
}

/// 外枠の内側(inner)のうち、テキストカード(card)の左側でキャラクターを置ける範囲。
/// カードとの間・外枠との間に隙間を空け、最小の幅・高さに満たなければNone(表示しない)。
/// 縦はカードと同じ高さ・同じ位置にする(innerいっぱいにすると、画面が縦に大きい時に
/// キャラクターがカードよりかなり大きく描かれてしまうため)
pub fn sprite_area(inner: Rect, card: Rect) -> Option<Rect> {
    let left = inner.x.saturating_add(PADDING_LEFT);
    let right = card.x.saturating_sub(GAP_TO_CARD).min(inner.right());
    let width = right.saturating_sub(left);
    if width < MIN_WIDTH || card.height < MIN_HEIGHT {
        return None;
    }
    Some(Rect::new(left, card.y, width, card.height))
}

/// 画像プロトコル(sixel/kitty/iTerm2)が使える端末ならそのpicker。端末への問い合わせは
/// スプラッシュ画面と共用する(プロセス内で1回だけ)。ハーフブロック描画は画像プロトコル
/// 非対応として扱い、キャラクターは描かない。テストでは端末に問い合わせない
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

/// リザルト画面のキャラクターのアニメーション(読み込み済みの全コマと経過時間)
pub struct ResultSprite {
    /// 端末の1セルの大きさ(px)。セル数と画像の縦横比の換算に使う
    font_size: (u16, u16),
    /// 元画像の大きさ(px)。縦横比を保った描画範囲の計算に使う
    image_size: (u32, u32),
    /// 読み込み済みの全コマ。1枚でも読めなければ空(何も描かない)
    frames: Vec<StatefulProtocol>,
    /// リザルト画面に入ってからの経過時間
    elapsed: Duration,
}

impl ResultSprite {
    /// 端末の画像プロトコルを検出して全コマを読み込む
    pub fn new() -> Self {
        Self::with_picker(detect_picker())
    }

    /// pickerを指定して読み込む。Noneなら画像を読まず、何も描かない
    pub fn with_picker(picker: Option<Picker>) -> Self {
        let mut sprite = Self {
            font_size: (1, 1),
            image_size: (1, 1),
            frames: Vec::new(),
            elapsed: Duration::ZERO,
        };
        let Some(picker) = picker else {
            return sprite;
        };
        let images: Option<Vec<_>> = FRAME_PATHS
            .iter()
            .map(|path| splash::load_embedded_image(path))
            .collect();
        // 1コマでも欠けるとアニメーションにならないので、全コマ読めた時だけ描く
        let Some(images) = images else {
            return sprite;
        };
        if let Some(first) = images.first() {
            sprite.image_size = (first.width(), first.height());
        }
        sprite.font_size = picker.font_size();
        sprite.frames = images
            .into_iter()
            .map(|image| picker.new_resize_protocol(image))
            .collect();
        sprite
    }

    /// キャラクターを描けない(何も描かない)か
    #[cfg(test)]
    pub fn is_fallback(&self) -> bool {
        self.frames.is_empty()
    }

    /// リザルト画面に入ってからの経過時間
    #[cfg(test)]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// 経過時間を0に戻す(リザルト画面に入る時に呼ぶ)
    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }

    /// 経過時間をdtだけ進める
    pub fn tick(&mut self, dt: Duration) {
        self.elapsed = self.elapsed.saturating_add(dt);
    }

    /// 現在表示するコマ番号(0〜3)
    pub fn current_frame(&self) -> usize {
        frame_index(self.elapsed)
    }

    /// キャラクターを縦横比を保ってareaに収まる最大の大きさで、areaの中央に置いた範囲。
    /// 描けない時・areaが空の時はNone
    pub fn sprite_rect(&self, area: Rect) -> Option<Rect> {
        if self.frames.is_empty() || area.width == 0 || area.height == 0 {
            return None;
        }
        let (font_w, font_h) = (
            f64::from(self.font_size.0.max(1)),
            f64::from(self.font_size.1.max(1)),
        );
        let aspect = f64::from(self.image_size.0.max(1)) / f64::from(self.image_size.1.max(1));
        // まず高さいっぱいで幅を求め、はみ出すなら幅いっぱいにして高さを縮める
        let full_height_width = (f64::from(area.height) * font_h * aspect / font_w).round() as u16;
        let (width, height) = if full_height_width <= area.width {
            (full_height_width.max(1), area.height)
        } else {
            let height = (f64::from(area.width) * font_w / aspect / font_h).round() as u16;
            (area.width, height.clamp(1, area.height))
        };
        Some(Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        ))
    }

    /// 外枠の内側(inner)のうちテキストカード(card)の左側の余白に、現在のコマを描く。
    /// 余白が狭い・画像を描けない時は何もしない
    pub fn render(&mut self, frame: &mut Frame, inner: Rect, card: Rect) {
        let Some(rect) = sprite_area(inner, card).and_then(|area| self.sprite_rect(area)) else {
            return;
        };
        let index = self.current_frame();
        let Some(protocol) = self.frames.get_mut(index) else {
            return;
        };
        let widget = StatefulImage::default().resize(Resize::Fit(Some(FilterType::Triangle)));
        frame.render_stateful_widget(widget, rect, protocol);
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

    /// spriteをwidth x heightの画面に描いたバッファ
    fn rendered_buffer(
        sprite: &mut ResultSprite,
        width: u16,
        height: u16,
        inner: Rect,
        card: Rect,
    ) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let completed = terminal
            .draw(|frame| sprite.render(frame, inner, card))
            .unwrap();
        completed.buffer.clone()
    }

    #[test]
    fn every_frame_is_embedded_and_decodes() {
        for path in FRAME_PATHS {
            let image = splash::load_embedded_image(path)
                .unwrap_or_else(|| panic!("{path}が埋め込まれていてデコードできること"));
            assert_eq!((image.width(), image.height()), (750, 940), "{path}");
        }
    }

    #[test]
    fn frame_duration_is_between_300_and_400_ms() {
        assert!(FRAME_DURATION >= Duration::from_millis(300));
        assert!(FRAME_DURATION <= Duration::from_millis(400));
    }

    #[test]
    fn frame_index_advances_one_frame_per_duration_and_loops() {
        let d = FRAME_DURATION;
        assert_eq!(frame_index(Duration::ZERO), 0);
        assert_eq!(
            frame_index(d - Duration::from_millis(1)),
            0,
            "1コマ分未満は0"
        );
        assert_eq!(frame_index(d), 1, "1コマ分経過で1");
        assert_eq!(frame_index(d * 2), 2);
        assert_eq!(frame_index(d * 3), 3);
        assert_eq!(frame_index(d * 4 - Duration::from_millis(1)), 3);
        assert_eq!(frame_index(d * 4), 0, "4コマ分経過で0に戻る(ループ)");
        assert_eq!(frame_index(d * 5), 1);
        assert_eq!(
            frame_index(d * 4001 + d / 2),
            1,
            "長時間経過してもループし続ける"
        );
    }

    #[test]
    fn tick_advances_and_reset_goes_back_to_the_first_frame() {
        let mut sprite = ResultSprite::with_picker(None);
        assert_eq!(sprite.elapsed(), Duration::ZERO);
        assert_eq!(sprite.current_frame(), 0);
        sprite.tick(FRAME_DURATION);
        assert_eq!(sprite.elapsed(), FRAME_DURATION);
        assert_eq!(sprite.current_frame(), 1);
        sprite.tick(FRAME_DURATION * 2);
        assert_eq!(sprite.current_frame(), 3);
        sprite.reset();
        assert_eq!(sprite.elapsed(), Duration::ZERO);
        assert_eq!(sprite.current_frame(), 0);
    }

    #[test]
    fn new_does_not_use_an_image_protocol_in_tests() {
        assert!(ResultSprite::new().is_fallback());
    }

    #[test]
    fn with_picker_loads_every_frame() {
        let sprite = ResultSprite::with_picker(Some(test_picker(ProtocolType::Halfblocks)));
        assert!(!sprite.is_fallback());
    }

    #[test]
    fn sprite_area_is_left_of_the_card_with_gaps() {
        let inner = Rect::new(5, 3, 110, 34);
        let card = Rect::new(36, 12, 48, 16);
        let area = sprite_area(inner, card).expect("十分な余白があれば表示する");
        assert_eq!(area.x, inner.x + PADDING_LEFT, "外枠の内側から少し空ける");
        assert_eq!(area.right(), card.x - GAP_TO_CARD, "カードとの間を空ける");
        assert_eq!(
            (area.y, area.height),
            (card.y, card.height),
            "縦はカードと同じ高さ・同じ位置(カードよりキャラクターが大きくならないように)"
        );
        assert!(area.right() <= card.x, "カードと重ならない");
    }

    #[test]
    fn sprite_area_height_does_not_exceed_the_card_even_on_a_tall_screen() {
        // 画面が縦に大きくても、キャラクターの表示範囲はカードの高さを超えない
        let inner = Rect::new(2, 1, 150, 60);
        let card = Rect::new(60, 20, 48, 16);
        let area = sprite_area(inner, card).expect("十分な余白があれば表示する");
        assert_eq!(area.height, card.height);
    }

    #[test]
    fn sprite_area_is_none_when_the_margin_is_too_narrow_or_short() {
        // 左の余白 = 1 + MIN_WIDTH + 2 ちょうどなら表示し、1セル足りなければ表示しない
        let inner = Rect::new(0, 0, 80, 20);
        let just = PADDING_LEFT + MIN_WIDTH + GAP_TO_CARD;
        assert!(sprite_area(inner, Rect::new(just, 5, 48, 10)).is_some());
        assert_eq!(sprite_area(inner, Rect::new(just - 1, 5, 48, 10)), None);
        assert_eq!(
            sprite_area(inner, Rect::new(0, 5, 48, 10)),
            None,
            "余白なし"
        );
        // 高さが足りない
        let short = Rect::new(0, 0, 80, MIN_HEIGHT - 1);
        assert_eq!(
            sprite_area(short, Rect::new(30, 0, 48, MIN_HEIGHT - 1)),
            None
        );
        // カードがinnerより左にはみ出していてもパニックしない
        assert_eq!(
            sprite_area(Rect::new(10, 0, 80, 20), Rect::new(3, 0, 48, 10)),
            None
        );
    }

    #[test]
    fn sprite_rect_keeps_the_aspect_ratio_and_fits_the_area() {
        let sprite = ResultSprite::with_picker(Some(test_picker(ProtocolType::Halfblocks)));
        for area in [
            Rect::new(2, 1, 30, 34),
            Rect::new(0, 0, 60, 10),
            Rect::new(1, 1, 9, 24),
        ] {
            let rect = sprite.sprite_rect(area).unwrap();
            assert_eq!(
                rect.intersection(area),
                rect,
                "{area:?}: areaの内側に収まる"
            );
            assert!(
                rect.width == area.width || rect.height == area.height,
                "{area:?}: 幅か高さのどちらかはいっぱい"
            );
            // 1セル10x20pxでの縦横比が元画像(750:940)とほぼ同じ(セル単位の丸めの誤差まで)
            let ratio = (rect.width as f64 * 10.0) / (rect.height as f64 * 20.0);
            let expected = 750.0 / 940.0;
            assert!((ratio - expected).abs() < 0.1, "{area:?}: 縦横比 {ratio}");
            let (left, right) = (rect.x - area.x, area.right() - rect.right());
            let (top, bottom) = (rect.y - area.y, area.bottom() - rect.bottom());
            assert!(
                left.abs_diff(right) <= 1 && top.abs_diff(bottom) <= 1,
                "{area:?}: 中央寄せ"
            );
        }
        assert_eq!(sprite.sprite_rect(Rect::new(0, 0, 0, 5)), None);
        assert_eq!(sprite.sprite_rect(Rect::new(0, 0, 5, 0)), None);
    }

    #[test]
    fn fallback_has_no_sprite_rect_and_draws_nothing() {
        let mut sprite = ResultSprite::with_picker(None);
        assert_eq!(sprite.sprite_rect(Rect::new(0, 0, 30, 20)), None);
        let inner = Rect::new(0, 0, 120, 40);
        let card = Rect::new(36, 12, 48, 16);
        let buffer = rendered_buffer(&mut sprite, 120, 40, inner, card);
        assert_eq!(buffer, Buffer::empty(buffer.area), "何も描かない");
    }

    #[test]
    fn with_picker_draws_only_left_of_the_card() {
        let mut sprite = ResultSprite::with_picker(Some(test_picker(ProtocolType::Halfblocks)));
        let inner = Rect::new(0, 0, 120, 40);
        let card = Rect::new(36, 12, 48, 16);
        let buffer = rendered_buffer(&mut sprite, 120, 40, inner, card);
        let default = ratatui::buffer::Cell::default();
        let drawn: Vec<_> = inner
            .positions()
            .filter(|p| buffer[*p] != default)
            .collect();
        assert!(!drawn.is_empty(), "キャラクターが描かれる");
        assert!(
            drawn.iter().all(|p| p.x + GAP_TO_CARD <= card.x),
            "カードの左側(隙間を残した範囲)だけに描く"
        );
    }

    #[test]
    fn narrow_margin_draws_nothing() {
        let mut sprite = ResultSprite::with_picker(Some(test_picker(ProtocolType::Halfblocks)));
        let inner = Rect::new(0, 0, 60, 20);
        let card = Rect::new(6, 5, 48, 10);
        let buffer = rendered_buffer(&mut sprite, 60, 20, inner, card);
        assert_eq!(buffer, Buffer::empty(buffer.area), "余白が狭ければ描かない");
    }

    #[test]
    fn each_frame_is_drawn_as_a_different_image() {
        // コマが進むと描かれる内容が変わる(同じ画像を描き続けていない)
        let mut sprite = ResultSprite::with_picker(Some(test_picker(ProtocolType::Halfblocks)));
        let inner = Rect::new(0, 0, 120, 40);
        let card = Rect::new(36, 12, 48, 16);
        let first = rendered_buffer(&mut sprite, 120, 40, inner, card);
        sprite.tick(FRAME_DURATION);
        let second = rendered_buffer(&mut sprite, 120, 40, inner, card);
        assert_ne!(first, second);
    }

    #[test]
    fn rendering_into_tiny_or_empty_areas_does_not_panic() {
        let mut sprite = ResultSprite::with_picker(Some(test_picker(ProtocolType::Iterm2)));
        let backend = TestBackend::new(20, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                sprite.render(frame, Rect::new(0, 0, 0, 0), Rect::new(0, 0, 0, 0));
                sprite.render(frame, Rect::new(0, 0, 20, 10), Rect::new(0, 0, 20, 10));
                sprite.render(frame, Rect::new(0, 0, 20, 10), Rect::new(12, 2, 5, 5));
                sprite.render(frame, Rect::new(0, 0, 20, 3), Rect::new(12, 0, 5, 3));
            })
            .unwrap();
    }
}
