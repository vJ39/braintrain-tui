//! メニュー画面のカードに描くアイコン画像(assets/image/menu_icons/)の読み込み・描画。
//!
//! 画像プロトコル(sixel/kitty/iTerm2)に対応した端末では、起動時に全アイコンを読み込んで
//! 項目ごとに`StatefulProtocol`を保持し、以後は同じものを使い回す(描画エリアが変わった時だけ
//! ratatui-image側で作り直される)。非対応の端末・読み込めなかった項目は何も描かない
//! (カード側はアイコン分の空白になる)。

use image::imageops::FilterType;
use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

use super::splash;

/// 1項目ぶんの読み込み済みアイコン
struct Icon {
    /// 元画像の大きさ(px)。縦横比を保った描画範囲の計算に使う
    size: (u32, u32),
    protocol: StatefulProtocol,
}

/// メニューの全項目のアイコン。項目のインデックスでアクセスする
pub struct MenuIcons {
    /// 端末の1セルの大きさ(px)。セル数と画像の縦横比の換算に使う
    font_size: (u16, u16),
    icons: Vec<Option<Icon>>,
}

/// assets/image/からの相対パスのアイコン画像を読み込む。無い・デコードできなければNone
pub fn load_icon_image(path: &str) -> Option<DynamicImage> {
    splash::load_embedded_image(path)
}

/// 画像プロトコル(sixel/kitty/iTerm2)が使える端末ならそのpicker。端末への問い合わせは
/// スプラッシュ画面と共用する(プロセス内で1回だけ)。ハーフブロック描画はカードの小さな
/// アイコンでは線画が潰れて判別できないため使わず、フォールバック(空白)にする。
/// テストでは端末に問い合わせない
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

impl MenuIcons {
    /// pathsはassets/image/からの相対パス(項目の並び順)。端末の画像プロトコルを検出して読み込む
    pub fn new(paths: &[&str]) -> Self {
        Self::with_picker(paths, detect_picker())
    }

    /// pickerを指定して読み込む。Noneなら画像を一切読まず、全項目フォールバックになる
    pub fn with_picker(paths: &[&str], picker: Option<Picker>) -> Self {
        let Some(picker) = picker else {
            return Self {
                font_size: (1, 1),
                icons: paths.iter().map(|_| None).collect(),
            };
        };
        let icons = paths
            .iter()
            .map(|path| {
                // 読めない画像があってもその項目だけアイコン無しにする
                let image = load_icon_image(path)?;
                Some(Icon {
                    size: (image.width(), image.height()),
                    protocol: picker.new_resize_protocol(image),
                })
            })
            .collect();
        Self {
            font_size: picker.font_size(),
            icons,
        }
    }

    /// 1枚もアイコンを描けない(全体がフォールバック表示)か
    #[cfg(test)]
    pub fn is_fallback(&self) -> bool {
        self.icons.iter().all(Option::is_none)
    }

    /// index番目の項目のアイコンを描けるか
    #[cfg(test)]
    pub fn has_icon(&self, index: usize) -> bool {
        self.icons.get(index).is_some_and(Option::is_some)
    }

    /// index番目のアイコンを、縦横比を保ってareaに収まる最大の大きさでareaの中央に置いた範囲。
    /// アイコンが無ければNone
    pub fn icon_rect(&self, index: usize, area: Rect) -> Option<Rect> {
        let icon = self.icons.get(index)?.as_ref()?;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let (font_w, font_h) = (
            f64::from(self.font_size.0.max(1)),
            f64::from(self.font_size.1.max(1)),
        );
        let aspect = f64::from(icon.size.0.max(1)) / f64::from(icon.size.1.max(1));
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

    /// index番目のアイコンをareaの中央に描く。アイコンが無ければ何も描かない(空白のまま)。
    /// 同じ範囲に描き続ける間は、画像の変換・エンコードはratatui-image側でやり直されない
    pub fn render(&mut self, frame: &mut Frame, index: usize, area: Rect) {
        let Some(rect) = self.icon_rect(index, area) else {
            return;
        };
        let Some(Some(icon)) = self.icons.get_mut(index) else {
            return;
        };
        // 細い線画が間引きで消えないよう、縮小は補間して行う
        let widget = StatefulImage::default().resize(Resize::Fit(Some(FilterType::Triangle)));
        frame.render_stateful_widget(widget, rect, &mut icon.protocol);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use ratatui_image::picker::ProtocolType;

    const ICON: &str = "menu_icons/shape_rotate.png";
    const MISSING: &str = "menu_icons/does-not-exist.png";

    /// テスト用のpicker(1セル10x20px・端末に問い合わせないハーフブロック描画)
    fn test_picker() -> Picker {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        picker
    }

    /// iconsのindex番目をareaに描き、area内の各セルの文字を返す
    fn render_cells(icons: &mut MenuIcons, index: usize, area: Rect) -> Vec<String> {
        let backend = TestBackend::new(area.right(), area.bottom());
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| icons.render(frame, index, area))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        area.positions()
            .map(|p| buffer[(p.x, p.y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn load_icon_image_reads_an_embedded_icon() {
        let image = load_icon_image(ICON).expect("埋め込まれていてデコードできること");
        assert!(image.width() > 0 && image.height() > 0);
        assert!(load_icon_image(MISSING).is_none());
    }

    #[test]
    fn without_picker_every_item_falls_back() {
        let icons = MenuIcons::with_picker(&[ICON, ICON], None);
        assert!(icons.is_fallback());
        assert!(!icons.has_icon(0));
        assert!(!icons.has_icon(1));
        assert_eq!(icons.icon_rect(0, Rect::new(0, 0, 30, 5)), None);
    }

    #[test]
    fn with_picker_every_readable_icon_is_loaded() {
        let icons = MenuIcons::with_picker(&[ICON, ICON], Some(test_picker()));
        assert!(!icons.is_fallback());
        assert!(icons.has_icon(0));
        assert!(icons.has_icon(1));
    }

    #[test]
    fn unreadable_icon_falls_back_only_for_that_item() {
        let icons = MenuIcons::with_picker(&[ICON, MISSING], Some(test_picker()));
        assert!(
            !icons.is_fallback(),
            "1枚でも読めれば全体はフォールバックにならない"
        );
        assert!(icons.has_icon(0));
        assert!(!icons.has_icon(1), "読めない項目だけアイコン無し");
    }

    #[test]
    fn all_icons_unreadable_is_fallback() {
        let icons = MenuIcons::with_picker(&[MISSING, MISSING], Some(test_picker()));
        assert!(icons.is_fallback());
    }

    #[test]
    fn index_out_of_range_has_no_icon() {
        let mut icons = MenuIcons::with_picker(&[ICON], Some(test_picker()));
        assert!(!icons.has_icon(5));
        assert_eq!(icons.icon_rect(5, Rect::new(0, 0, 30, 5)), None);
        render_cells(&mut icons, 5, Rect::new(0, 0, 30, 5));
    }

    #[test]
    fn icon_rect_keeps_the_aspect_ratio_and_is_centered() {
        let icons = MenuIcons::with_picker(&[ICON], Some(test_picker()));
        let (w, h) = load_icon_image(ICON)
            .map(|i| (i.width() as f64, i.height() as f64))
            .unwrap();
        let area = Rect::new(3, 2, 40, 5);
        let rect = icons.icon_rect(0, area).unwrap();
        assert_eq!(rect.intersection(area), rect, "areaの内側に収まる");
        assert_eq!(
            rect.height, area.height,
            "横に余裕があれば高さいっぱいに描く"
        );
        // 1セル10x20pxでの縦横比が元画像とほぼ同じ(セル単位の丸めの誤差まで)
        let ratio = (rect.width as f64 * 10.0) / (rect.height as f64 * 20.0);
        assert!((ratio - w / h).abs() < 0.15, "縦横比 {ratio} ≒ {}", w / h);
        let left = rect.x - area.x;
        let right = area.right() - rect.right();
        assert!(left.abs_diff(right) <= 1, "左右の余白がほぼ同じ(中央寄せ)");
    }

    #[test]
    fn icon_rect_in_a_narrow_area_is_limited_by_the_width() {
        let icons = MenuIcons::with_picker(&[ICON], Some(test_picker()));
        let area = Rect::new(0, 0, 6, 5);
        let rect = icons.icon_rect(0, area).unwrap();
        assert_eq!(rect.width, area.width);
        assert!(rect.height < area.height, "幅に合わせて高さが縮む");
        assert!(rect.height >= 1);
    }

    #[test]
    fn icon_rect_of_an_empty_area_is_none() {
        let icons = MenuIcons::with_picker(&[ICON], Some(test_picker()));
        assert_eq!(icons.icon_rect(0, Rect::new(0, 0, 0, 5)), None);
        assert_eq!(icons.icon_rect(0, Rect::new(0, 0, 10, 0)), None);
    }

    #[test]
    fn fallback_render_leaves_the_area_blank() {
        let mut icons = MenuIcons::with_picker(&[ICON], None);
        let cells = render_cells(&mut icons, 0, Rect::new(0, 0, 30, 5));
        assert!(cells.iter().all(|c| c == " "), "アイコン部分は空白のまま");
    }

    #[test]
    fn image_render_draws_something_in_the_area() {
        let mut icons = MenuIcons::with_picker(&[ICON], Some(test_picker()));
        let cells = render_cells(&mut icons, 0, Rect::new(0, 0, 30, 5));
        assert!(cells.iter().any(|c| c != " "), "アイコンが描かれる");
    }
}
