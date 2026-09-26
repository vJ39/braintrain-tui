//! べーの2視点(軽トラ視点・盤面視点)の描画。
//!
//! 画像アセット(assets/image/beigoma/配下)は埋め込まれていれば使い、無ければテキストで描く。
//! - 盤面: 画像プロトコル(sixel/kitty/iTerm2)が使え、盤の画像(board.png)が読める時だけ画像で描く。
//!   盤(障害物・ゴール)は固定なので、盤の画像は描画エリアが変わった時だけ1回合成・エンコードする。
//!   ベーゴマは盤の画像から「前回の位置と今回の位置」を包む小さな範囲だけを切り出して重ねたパッチにし、
//!   ベーゴマのマスが変わった時だけパッチを作り直す(count_mania/circle_image.rsと同じ考え方)。
//!   それ以外はマスごとに記号・色分けしたテキストで描く
//! - 軽トラ視点: 女の子キャラの静止画2枚(通常時・踏ん張り時)を、Gがかかっている間だけ切り替える。
//!   画像が無ければ同じ2通りのテキストの絵で描く

use std::cell::RefCell;
use std::time::Duration;

use image::imageops::{self, FilterType};
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

use super::board::{Board, Cell, BOARD_HEIGHT, BOARD_WIDTH};
use super::truck::{GForce, Side, SignalLight, Upcoming, UpcomingKind};
use crate::game::theme;
use crate::ui::splash;

/// 画像アセット(assets/image/からの相対パス)。無ければテキストで描く
pub const TRUCK_NORMAL_IMAGE: &str = "beigoma/truck_normal.png";
pub const TRUCK_BRACE_IMAGE: &str = "beigoma/truck_brace.png";
pub const BOARD_IMAGE: &str = "beigoma/board.png";
pub const TOP_IMAGE: &str = "beigoma/top.png";

/// このG以上の間は、軽トラ視点の女の子を踏ん張り時の絵にする
pub const BRACE_G: f64 = 0.15;

/// 盤1マスの大きさ(セル)の倍率の上限。広い画面でも盤が大きくなりすぎないようにする。
/// 3だと広い画面でもすぐ上限に張り付き、ベーゴマが豆粒のように小さく見えていたため引き上げた
const MAX_CELL_SCALE: u16 = 5;

/// テキスト表示の記号。記号は各マスの左端のセルに置き、残りは空白にする
/// (端末によって記号が2セル幅で表示されても、隣のマスに食い込まないように)
pub const BUMP_GLYPH: &str = "▲";
pub const HOLLOW_GLYPH: &str = "▽";
pub const GOAL_GLYPH: &str = "◎";
/// 転がっている間の回転の見た目(順に切り替える)
pub const TOP_SPIN_GLYPHS: [&str; 4] = ["◐", "◓", "◑", "◒"];
/// 飛び上がっている間の見た目
pub const TOP_AIRBORNE_GLYPH: &str = "○";

const FLAT_BG: Color = Color::Rgb(150, 105, 60);
/// 凸は平坦より明るく(盛り上がり)、凹は暗く(穴)する
const BUMP_BG: Color = Color::Rgb(205, 160, 105);
const BUMP_FG: Color = Color::Rgb(95, 55, 20);
const HOLLOW_BG: Color = Color::Rgb(60, 38, 18);
const HOLLOW_FG: Color = Color::Rgb(120, 90, 60);
const GOAL_BG: Color = Color::Rgb(40, 40, 40);
const GOAL_FG: Color = Color::Yellow;
const TOP_FG: Color = Color::Rgb(220, 240, 255);

/// 盤面に描くベーゴマの状態
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TopView {
    pub pos: (f64, f64),
    pub airborne: bool,
    /// 回転の見た目のコマ(テキスト表示のみ。画像では再エンコードを避けるため使わない)
    pub spin_frame: usize,
    /// 場外・吹っ飛びGAME OVERの星の演出のコマ(Noneなら通常のベーゴマを描く)
    pub star_frame: Option<usize>,
}

/// 星の演出のコマ(だんだん小さくなり、最後は消える)
pub const STAR_ANIM_GLYPHS: [&str; 4] = ["★", "☆", "✦", "･"];
/// 星の演出の1コマの表示時間
pub const STAR_ANIM_FRAME: Duration = Duration::from_millis(180);

/// 盤を描く範囲とマスの大きさ(セル)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardArea {
    /// 盤全体の範囲(描画エリアより大きい時ははみ出す分を描かない)
    pub rect: Rect,
    pub cell_width: u16,
    pub cell_height: u16,
    /// 呼び出し時に渡された描画エリア全体(盤より外側に余白があり得る)。
    /// 場外に出たベーゴマの位置をこの範囲内に収めるために使う
    panel: Rect,
}

/// area(盤面パネルの内側)に盤を置く範囲。端末の1セルは縦長なので、1マスは横2セル×縦1セルを基本にし、
/// 余裕があれば整数倍に大きくして中央に置く。areaが空ならNone
pub fn board_area(area: Rect) -> Option<BoardArea> {
    if area.is_empty() {
        return None;
    }
    let (width, height) = (BOARD_WIDTH as u16, BOARD_HEIGHT as u16);
    let scale = (area.width / (2 * width))
        .min(area.height / height)
        .clamp(1, MAX_CELL_SCALE);
    let (cell_width, cell_height) = (2 * scale, scale);
    let (board_width, board_height) = (cell_width * width, cell_height * height);
    Some(BoardArea {
        rect: Rect::new(
            area.x + area.width.saturating_sub(board_width) / 2,
            area.y + area.height.saturating_sub(board_height) / 2,
            board_width,
            board_height,
        ),
        cell_width,
        cell_height,
        panel: area,
    })
}

impl BoardArea {
    /// マス(x, y)のセル範囲(描画エリアで切り詰める前)
    pub fn cell_rect(&self, x: usize, y: usize) -> Rect {
        Rect::new(
            self.rect.x + x as u16 * self.cell_width,
            self.rect.y + y as u16 * self.cell_height,
            self.cell_width,
            self.cell_height,
        )
    }

    /// 位置posを含むマスのセル範囲。盤外の位置は、実際にずれた方向・距離に応じて
    /// 盤の外側の位置を返す(panelの範囲を超える分はpanelの端に収める)
    pub fn top_rect(&self, pos: (f64, f64)) -> Rect {
        let cell_w = i32::from(self.cell_width);
        let cell_h = i32::from(self.cell_height);
        let raw_x = i32::from(self.rect.x) + (pos.0.floor() as i32) * cell_w;
        let raw_y = i32::from(self.rect.y) + (pos.1.floor() as i32) * cell_h;
        let min_x = i32::from(self.panel.x);
        let min_y = i32::from(self.panel.y);
        let max_x = min_x + i32::from(self.panel.width) - cell_w;
        let max_y = min_y + i32::from(self.panel.height) - cell_h;
        let x = raw_x.clamp(min_x, max_x.max(min_x));
        let y = raw_y.clamp(min_y, max_y.max(min_y));
        Rect::new(x as u16, y as u16, self.cell_width, self.cell_height)
    }
}

/// 直前に作った盤の画像(障害物・ゴールまで重ねたもの)
struct BaseCache {
    area: BoardArea,
    composed: RgbaImage,
    protocol: StatefulProtocol,
}

/// 直前に作ったベーゴマのパッチ。keyは(ベーゴマのマスのセル範囲, 飛び上がっているか)
struct PatchCache {
    rect: Rect,
    key: (Rect, bool),
    protocol: StatefulProtocol,
}

/// 盤面視点の描画器
pub struct BoardRenderer {
    picker: Option<Picker>,
    board_image: Option<RgbaImage>,
    top_image: Option<RgbaImage>,
    base: RefCell<Option<BaseCache>>,
    patch: RefCell<Option<PatchCache>>,
    /// テスト用: 盤の画像・パッチを作り直した回数
    #[cfg(test)]
    base_encodes: std::cell::Cell<usize>,
    #[cfg(test)]
    patch_encodes: std::cell::Cell<usize>,
}

impl BoardRenderer {
    /// 端末の画像プロトコルと画像アセットを調べて作る
    pub fn new() -> Self {
        let picker = detect_picker();
        // 端末が画像を出せない時は、アセットを読み込むだけ無駄なので読まない
        let load = |path: &str| {
            picker
                .as_ref()
                .and_then(|_| splash::load_embedded_image(path))
                .map(|image| image.to_rgba8())
        };
        let board_image = load(BOARD_IMAGE);
        let top_image = load(TOP_IMAGE);
        Self::from_parts(picker, board_image, top_image)
    }

    fn from_parts(
        picker: Option<Picker>,
        board_image: Option<RgbaImage>,
        top_image: Option<RgbaImage>,
    ) -> Self {
        Self {
            picker,
            board_image,
            top_image,
            base: RefCell::new(None),
            patch: RefCell::new(None),
            #[cfg(test)]
            base_encodes: std::cell::Cell::new(0),
            #[cfg(test)]
            patch_encodes: std::cell::Cell::new(0),
        }
    }

    /// 画像を指定して作る(テストで画像表示の経路を通すため。アセットはまだ無い)
    #[cfg(test)]
    pub fn with_images(
        picker: Picker,
        board_image: RgbaImage,
        top_image: Option<RgbaImage>,
    ) -> Self {
        Self::from_parts(Some(picker), Some(board_image), top_image)
    }

    /// 画像で描くか(false=テキスト表示)。画像プロトコルが使え、盤の画像が読めた時だけ。テストでの確認用
    #[cfg(test)]
    pub fn uses_image(&self) -> bool {
        self.picker.is_some() && self.board_image.is_some()
    }

    #[cfg(test)]
    pub fn base_encode_count(&self) -> usize {
        self.base_encodes.get()
    }

    #[cfg(test)]
    pub fn patch_encode_count(&self) -> usize {
        self.patch_encodes.get()
    }

    /// area(盤面パネルの内側)に盤とベーゴマを描く
    pub fn render(&self, frame: &mut Frame, area: Rect, board: &Board, top: &TopView) {
        let area = area.intersection(frame.area());
        let Some(layout) = board_area(area) else {
            return;
        };
        // 星の演出中は画像のパッチ更新に乗せず、テキストで星を描く
        if top.star_frame.is_none() && self.render_image(frame, area, layout, board, top) {
            return;
        }
        render_board_text(frame, area, layout, board, top);
    }

    /// 画像で描く。画像を使えない・盤全体がエリアに収まらない時はfalse(テキストで描く)
    fn render_image(
        &self,
        frame: &mut Frame,
        area: Rect,
        layout: BoardArea,
        board: &Board,
        top: &TopView,
    ) -> bool {
        let (Some(picker), Some(board_image)) = (&self.picker, &self.board_image) else {
            return false;
        };
        // はみ出した画像は切り詰められて位置がずれるので、収まらない時はテキストにする
        if layout.rect.intersection(area) != layout.rect {
            return false;
        }
        let mut base = self.base.borrow_mut();
        if !matches!(base.as_ref(), Some(cached) if cached.area == layout) {
            let composed = compose_base(board_image, board, layout, picker.font_size());
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(composed.clone()));
            *base = Some(BaseCache {
                area: layout,
                composed,
                protocol,
            });
            // 盤の画像がパッチの範囲も描き直すので、パッチは新しい盤から作り直す
            *self.patch.borrow_mut() = None;
            #[cfg(test)]
            self.base_encodes.set(self.base_encodes.get() + 1);
        }
        let Some(cached) = base.as_mut() else {
            return false;
        };
        frame.render_stateful_widget(StatefulImage::default(), layout.rect, &mut cached.protocol);
        // パッチは盤の画像より後に描き、盤の上に重ねる
        self.render_patch(frame, picker, cached, top);
        true
    }

    /// ベーゴマのパッチを盤の上に描く。ベーゴマのマス(と飛び上がりの見た目)が変わった時だけ作り直す。
    /// 作り直す時は前回のマスも含めて切り出し、前の位置のベーゴマを端末に残さない
    fn render_patch(&self, frame: &mut Frame, picker: &Picker, base: &BaseCache, top: &TopView) {
        let top_rect = base.area.top_rect(top.pos);
        let key = (top_rect, top.airborne);
        let mut patch = self.patch.borrow_mut();
        let same = matches!(patch.as_ref(), Some(cached) if cached.key == key);
        if !same {
            let rect = match patch.as_ref() {
                Some(cached) => cached.key.0.union(top_rect),
                None => top_rect,
            };
            let image = patch_image(
                base,
                picker.font_size(),
                rect,
                top_rect,
                top.airborne,
                self.top_image.as_ref(),
            );
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(image));
            *patch = Some(PatchCache {
                rect,
                key,
                protocol,
            });
            #[cfg(test)]
            self.patch_encodes.set(self.patch_encodes.get() + 1);
        }
        if let Some(cached) = patch.as_mut() {
            // 盤の画像は起点以外の全セルをskip(ratatuiの差分出力の対象外)にしており、そのままでは
            // パッチが端末へ出力されないので、パッチの範囲のskipを先に外す(circle_image.rsと同じ)
            let buffer = frame.buffer_mut();
            for y in cached.rect.top()..cached.rect.bottom() {
                for x in cached.rect.left()..cached.rect.right() {
                    buffer[(x, y)].set_skip(false);
                }
            }
            frame.render_stateful_widget(
                StatefulImage::default(),
                cached.rect,
                &mut cached.protocol,
            );
        }
    }
}

/// 端末の画像プロトコルを調べる。sixel/kitty/iTerm2のどれかが使える時だけSome(問い合わせは
/// スプラッシュ画面と共用)。テストでは端末に問い合わせず、常にテキスト表示にする
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

/// 1セルのピクセル数(幅, 高さ)。0にならないようにする
fn cell_pixels(font_size: (u16, u16)) -> (u32, u32) {
    (u32::from(font_size.0.max(1)), u32::from(font_size.1.max(1)))
}

/// セル範囲rectを、originを左上とするピクセル座標の範囲(x, y, 幅, 高さ)にする
fn pixel_rect(rect: Rect, origin: (u16, u16), font_size: (u16, u16)) -> (u32, u32, u32, u32) {
    let (fw, fh) = cell_pixels(font_size);
    (
        u32::from(rect.x.saturating_sub(origin.0)) * fw,
        u32::from(rect.y.saturating_sub(origin.1)) * fh,
        u32::from(rect.width) * fw,
        u32::from(rect.height) * fh,
    )
}

/// 盤の画像を描画範囲の大きさにし、その上に障害物・ゴールの印を固定配置どおりに重ねる
/// (配置の正はboard.rsのLAYOUTなので、盤の画像に描かれた凹凸の位置には頼らない)
fn compose_base(
    board_image: &RgbaImage,
    board: &Board,
    layout: BoardArea,
    font_size: (u16, u16),
) -> RgbaImage {
    let origin = (layout.rect.x, layout.rect.y);
    let (_, _, width, height) = pixel_rect(layout.rect, origin, font_size);
    let mut canvas = imageops::resize(
        board_image,
        width.max(1),
        height.max(1),
        FilterType::Triangle,
    );
    for y in 0..BOARD_HEIGHT {
        for x in 0..BOARD_WIDTH {
            let (px, py, pw, ph) = pixel_rect(layout.cell_rect(x, y), origin, font_size);
            let center = (
                f64::from(px) + f64::from(pw) / 2.0,
                f64::from(py) + f64::from(ph) / 2.0,
            );
            let radius = f64::from(pw.min(ph)) / 2.0;
            match board.cell(x, y) {
                Cell::Flat => {}
                // 凸: 暗い縁(影)の上に明るい頂上を重ね、盛り上がって見せる
                Cell::Bump => {
                    fill_circle(&mut canvas, center, radius * 0.8, BUMP_EDGE_PIXEL);
                    fill_circle(&mut canvas, center, radius * 0.55, BUMP_TOP_PIXEL);
                }
                // 凹: 暗い穴。中ほどをさらに暗くして沈んで見せる
                Cell::Hollow => {
                    fill_circle(&mut canvas, center, radius * 0.8, HOLLOW_PIXEL);
                    fill_circle(&mut canvas, center, radius * 0.5, HOLLOW_DEEP_PIXEL);
                }
                Cell::Goal => {
                    fill_circle(&mut canvas, center, radius * 0.9, GOAL_RING_PIXEL);
                    fill_circle(&mut canvas, center, radius * 0.6, GOAL_HOLE_PIXEL);
                }
            }
        }
    }
    canvas
}

/// 画像表示の凹凸・ゴール・ベーゴマ(top.pngが無い時)の色
const BUMP_EDGE_PIXEL: Rgba<u8> = Rgba([95, 60, 30, 255]);
const BUMP_TOP_PIXEL: Rgba<u8> = Rgba([215, 170, 115, 255]);
const HOLLOW_PIXEL: Rgba<u8> = Rgba([60, 38, 18, 255]);
const HOLLOW_DEEP_PIXEL: Rgba<u8> = Rgba([35, 22, 10, 255]);
const GOAL_RING_PIXEL: Rgba<u8> = Rgba([255, 215, 0, 255]);
const GOAL_HOLE_PIXEL: Rgba<u8> = Rgba([30, 30, 30, 255]);
const TOP_EDGE_PIXEL: Rgba<u8> = Rgba([90, 100, 115, 255]);
const TOP_FACE_PIXEL: Rgba<u8> = Rgba([200, 210, 225, 255]);

/// 盤の合成画像からrectを切り出し、top_rectにベーゴマを重ねたパッチ
fn patch_image(
    base: &BaseCache,
    font_size: (u16, u16),
    rect: Rect,
    top_rect: Rect,
    airborne: bool,
    top_image: Option<&RgbaImage>,
) -> RgbaImage {
    let origin = (base.area.rect.x, base.area.rect.y);
    let (px, py, pw, ph) = pixel_rect(rect, origin, font_size);
    let mut patch = imageops::crop_imm(&base.composed, px, py, pw.max(1), ph.max(1)).to_image();
    let (tx, ty, tw, th) = pixel_rect(top_rect, (rect.x, rect.y), font_size);
    // 飛び上がっている間は小さく描き、浮いているように見せる。
    // 通常時はマスいっぱいに近い大きさにして豆粒にならないようにする
    let scale = if airborne { 0.7 } else { 0.95 };
    let size = (f64::from(tw.min(th)) * scale).max(1.0) as u32;
    let center = (
        f64::from(tx) + f64::from(tw) / 2.0,
        f64::from(ty) + f64::from(th) / 2.0,
    );
    match top_image {
        Some(image) => {
            let scaled = imageops::resize(image, size, size, FilterType::Triangle);
            let x = (center.0 - f64::from(size) / 2.0) as i64;
            let y = (center.1 - f64::from(size) / 2.0) as i64;
            imageops::overlay(&mut patch, &scaled, x, y);
        }
        None => {
            let radius = f64::from(size) / 2.0;
            fill_circle(&mut patch, center, radius, TOP_EDGE_PIXEL);
            fill_circle(&mut patch, center, radius * 0.7, TOP_FACE_PIXEL);
        }
    }
    patch
}

/// 塗りつぶした円を描く(画像の外にはみ出す部分は描かない)
fn fill_circle(image: &mut RgbaImage, center: (f64, f64), radius: f64, color: Rgba<u8>) {
    if radius <= 0.0 {
        return;
    }
    let (width, height) = image.dimensions();
    let left = (center.0 - radius).floor().max(0.0) as u32;
    let top = (center.1 - radius).floor().max(0.0) as u32;
    let right = ((center.0 + radius).ceil().max(0.0) as u32).min(width);
    let bottom = ((center.1 + radius).ceil().max(0.0) as u32).min(height);
    for y in top..bottom {
        for x in left..right {
            let dx = f64::from(x) + 0.5 - center.0;
            let dy = f64::from(y) + 0.5 - center.1;
            if dx * dx + dy * dy <= radius * radius {
                image.put_pixel(x, y, color);
            }
        }
    }
}

/// テキスト表示の盤面。マスごとに背景色で塗り、凸・凹・ゴール・ベーゴマは記号で示す
/// (凸は明るい色の▲、凹は暗い色の▽)
fn render_board_text(
    frame: &mut Frame,
    area: Rect,
    layout: BoardArea,
    board: &Board,
    top: &TopView,
) {
    let buffer = frame.buffer_mut();
    for y in 0..BOARD_HEIGHT {
        for x in 0..BOARD_WIDTH {
            let (glyph, style) = match board.cell(x, y) {
                Cell::Flat => (" ", Style::default().bg(FLAT_BG)),
                Cell::Bump => (
                    BUMP_GLYPH,
                    Style::default()
                        .fg(BUMP_FG)
                        .bg(BUMP_BG)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::Hollow => (HOLLOW_GLYPH, Style::default().fg(HOLLOW_FG).bg(HOLLOW_BG)),
                Cell::Goal => (
                    GOAL_GLYPH,
                    Style::default()
                        .fg(GOAL_FG)
                        .bg(GOAL_BG)
                        .add_modifier(Modifier::BOLD),
                ),
            };
            let rect = layout.cell_rect(x, y);
            // 幅0の範囲でもpositions()は1点返すので、見えないマスは飛ばす
            let visible = rect.intersection(area);
            if !visible.is_empty() {
                for position in visible.positions() {
                    buffer[position].set_symbol(" ").set_style(style);
                }
            }
            put_glyph(buffer, rect, area, glyph, Style::default());
        }
    }
    let glyph = if let Some(frame) = top.star_frame {
        STAR_ANIM_GLYPHS[frame.min(STAR_ANIM_GLYPHS.len() - 1)]
    } else if top.airborne {
        TOP_AIRBORNE_GLYPH
    } else {
        TOP_SPIN_GLYPHS[top.spin_frame % TOP_SPIN_GLYPHS.len()]
    };
    // 背景色はマスのものを残し、記号と文字色だけ変える
    let style = Style::default().fg(TOP_FG).add_modifier(Modifier::BOLD);
    put_glyph(buffer, layout.top_rect(top.pos), area, glyph, style);
}

/// マスの中央付近(横は左寄りの1セル)に記号を置く。記号の右のセルは空白のままにする
fn put_glyph(buffer: &mut Buffer, rect: Rect, clip: Rect, glyph: &str, style: Style) {
    let position = Position::new(
        rect.x + rect.width.saturating_sub(2) / 2,
        rect.y + rect.height / 2,
    );
    if clip.contains(position) && rect.contains(position) {
        buffer[position].set_symbol(glyph).set_style(style);
    }
}

impl Default for BoardRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// 軽トラ視点に出す情報
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TruckViewInfo {
    /// 軽トラの速度(m/s)
    pub speed: f64,
    pub g: GForce,
    pub upcoming: Option<Upcoming>,
}

impl TruckViewInfo {
    /// 女の子を踏ん張り時の絵にするか(Gがかかっている間)
    pub fn bracing(&self) -> bool {
        self.g.magnitude() >= BRACE_G
    }
}

/// 前方のイベントの案内文(例: "信号(黄) あと18m")。何も無ければ"前方: なし"
pub fn upcoming_text(upcoming: Option<Upcoming>) -> String {
    let Some(upcoming) = upcoming else {
        return "前方: なし".to_string();
    };
    let label = match upcoming.kind {
        UpcomingKind::Bump => "段差".to_string(),
        UpcomingKind::Signal(light) => format!("信号({})", signal_label(light)),
        UpcomingKind::Obstacle(Side::Right) => "障害物(右へよける)".to_string(),
        UpcomingKind::Obstacle(Side::Left) => "障害物(左へよける)".to_string(),
    };
    format!("前方: {label} あと{:.0}m", upcoming.distance)
}

fn signal_label(light: SignalLight) -> &'static str {
    match light {
        SignalLight::Green => "青",
        SignalLight::Yellow => "黄",
        SignalLight::Red => "赤",
    }
}

/// 前方の案内文の色。信号はその色、それ以外は目立つ色、何も無ければ控えめな色
fn upcoming_color(upcoming: Option<Upcoming>) -> Color {
    match upcoming.map(|u| u.kind) {
        Some(UpcomingKind::Signal(SignalLight::Green)) => Color::LightGreen,
        Some(UpcomingKind::Signal(SignalLight::Yellow)) => Color::Yellow,
        Some(UpcomingKind::Signal(SignalLight::Red)) => Color::LightRed,
        Some(_) => theme::HIGHLIGHT,
        None => theme::MUTED,
    }
}

/// 画像が無い時の女の子の絵(通常時)
const GIRL_NORMAL: [&str; 5] = [
    " _/\\___/\\_ ",
    " ( ^ _ ^ ) ",
    "--[=====]--",
    "   |   |   ",
    "   /   \\   ",
];
/// 画像が無い時の女の子の絵(踏ん張り時)
const GIRL_BRACE: [&str; 5] = [
    " _/\\___/\\_;",
    " ( >_< )   ",
    "==[=====]==",
    "  /|   |\\  ",
    " _/     \\_ ",
];

/// 軽トラ視点の描画器。女の子の静止画(通常時・踏ん張り時)が両方読めた時だけ画像で描く
pub struct TruckViewRenderer {
    /// (通常時, 踏ん張り時)
    images: Option<RefCell<(StatefulProtocol, StatefulProtocol)>>,
}

impl TruckViewRenderer {
    pub fn new() -> Self {
        let images = detect_picker().and_then(|picker| {
            let normal = splash::load_embedded_image(TRUCK_NORMAL_IMAGE)?;
            let brace = splash::load_embedded_image(TRUCK_BRACE_IMAGE)?;
            Some(RefCell::new((
                picker.new_resize_protocol(normal),
                picker.new_resize_protocol(brace),
            )))
        });
        Self { images }
    }

    /// 画像で描くか(false=テキストの絵)。テストでの確認用
    #[cfg(test)]
    pub fn uses_image(&self) -> bool {
        self.images.is_some()
    }

    /// area(軽トラ視点パネルの内側)に描く。上に前方の予兆、中央に女の子、下に速度とG
    pub fn render(&self, frame: &mut Frame, area: Rect, info: &TruckViewInfo) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        let header_height = 2.min(area.height);
        let footer_height = 3.min(area.height - header_height);
        let header = Rect::new(area.x, area.y, area.width, header_height);
        let footer = Rect::new(
            area.x,
            area.bottom() - footer_height,
            area.width,
            footer_height,
        );
        let picture = Rect::new(
            area.x,
            header.bottom(),
            area.width,
            area.height - header_height - footer_height,
        );

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                upcoming_text(info.upcoming),
                Style::default()
                    .fg(upcoming_color(info.upcoming))
                    .add_modifier(Modifier::BOLD),
            )))
            .alignment(Alignment::Center),
            header,
        );

        let bracing = info.bracing();
        self.render_picture(frame, picture, bracing);

        let brace_line = if bracing {
            Line::from(Span::styled(
                "ふんばり!",
                Style::default()
                    .fg(theme::INCORRECT)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from("")
        };
        let g_color = if bracing {
            theme::HIGHLIGHT
        } else {
            theme::TEXT
        };
        let footer_lines = vec![
            brace_line,
            Line::from(Span::styled(
                format!("速度 {:.0}km/h", info.speed * 3.6),
                Style::default().fg(theme::TEXT),
            )),
            Line::from(Span::styled(
                format!(
                    "G 前後 {:+.2}  左右 {:+.2}",
                    info.g.longitudinal, info.g.lateral
                ),
                Style::default().fg(g_color),
            )),
        ];
        frame.render_widget(
            Paragraph::new(footer_lines).alignment(Alignment::Center),
            footer,
        );
    }

    /// 女の子の絵(通常時・踏ん張り時)。画像が無ければテキストの絵
    fn render_picture(&self, frame: &mut Frame, area: Rect, bracing: bool) {
        if area.is_empty() {
            return;
        }
        if let Some(images) = &self.images {
            let mut images = images.borrow_mut();
            let protocol = if bracing {
                &mut images.1
            } else {
                &mut images.0
            };
            let widget = StatefulImage::default().resize(Resize::Fit(Some(FilterType::Triangle)));
            frame.render_stateful_widget(widget, area, protocol);
            return;
        }
        let (art, color) = if bracing {
            (GIRL_BRACE, theme::HIGHLIGHT)
        } else {
            (GIRL_NORMAL, theme::ACCENT_STRONG)
        };
        let lines: Vec<Line> = art
            .iter()
            .map(|row| Line::from(Span::styled(*row, Style::default().fg(color))))
            .collect();
        let text_area = theme::vertical_center(area, lines.len() as u16);
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            text_area,
        );
    }
}

impl Default for TruckViewRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn test_picker() -> Picker {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        picker
    }

    fn plain_board_image() -> RgbaImage {
        RgbaImage::from_pixel(40, 24, Rgba([150, 105, 60, 255]))
    }

    fn top_at(pos: (f64, f64)) -> TopView {
        TopView {
            pos,
            airborne: false,
            spin_frame: 0,
            star_frame: None,
        }
    }

    fn draw_board(renderer: &BoardRenderer, board: &Board, top: &TopView, area: Rect) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(area.right(), area.bottom())).unwrap();
        terminal
            .draw(|frame| renderer.render(frame, area, board, top))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn draw_truck(renderer: &TruckViewRenderer, info: &TruckViewInfo, area: Rect) -> String {
        let mut terminal = Terminal::new(TestBackend::new(area.right(), area.bottom())).unwrap();
        terminal
            .draw(|frame| renderer.render(frame, area, info))
            .unwrap();
        text_of(terminal.backend().buffer())
    }

    fn text_of(buffer: &Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace(' ', "")
    }

    fn count_symbol(buffer: &Buffer, symbol: &str) -> usize {
        buffer
            .content()
            .iter()
            .filter(|c| c.symbol() == symbol)
            .count()
    }

    fn info(g: GForce, upcoming: Option<Upcoming>) -> TruckViewInfo {
        TruckViewInfo {
            speed: 11.0,
            g,
            upcoming,
        }
    }

    // --- 盤の配置 ---

    #[test]
    fn board_area_uses_two_by_one_cells_and_centers_the_board() {
        let area = Rect::new(0, 0, 50, 14);
        let layout = board_area(area).unwrap();
        assert_eq!((layout.cell_width, layout.cell_height), (2, 1));
        assert_eq!(layout.rect.width, 2 * BOARD_WIDTH as u16);
        assert_eq!(layout.rect.height, BOARD_HEIGHT as u16);
        assert_eq!(layout.rect.x, 5, "横は中央に置く");
        assert_eq!(layout.rect.y, 1, "縦も中央に置く");
        assert_eq!(layout.cell_rect(1, 2), Rect::new(7, 3, 2, 1));
    }

    #[test]
    fn board_area_scales_up_on_a_large_area() {
        let layout = board_area(Rect::new(0, 0, 200, 60)).unwrap();
        assert_eq!(layout.cell_height, MAX_CELL_SCALE);
        assert_eq!(layout.cell_width, 2 * MAX_CELL_SCALE);
        let medium = board_area(Rect::new(0, 0, 90, 26)).unwrap();
        assert_eq!((medium.cell_width, medium.cell_height), (4, 2));
        assert!(board_area(Rect::new(0, 0, 0, 10)).is_none());
    }

    #[test]
    fn top_rect_is_the_cell_containing_the_top() {
        let layout = board_area(Rect::new(0, 0, 40, 12)).unwrap();
        assert_eq!(layout.top_rect((3.7, 2.1)), layout.cell_rect(3, 2));
    }

    #[test]
    fn top_rect_moves_outside_the_board_when_the_top_falls_off() {
        // 盤の周囲に余白があるpanel(盤より大きいarea)で、左に1マス分外れた位置は、
        // 盤の端のマスではなく、その外側の位置になる(場外に出た方向が見た目でも分かるように)
        let layout = board_area(Rect::new(0, 0, 60, 20)).unwrap();
        let inside_top_left = layout.cell_rect(0, 0);
        let off_left = layout.top_rect((-1.0, 0.0));
        assert_eq!(off_left.y, inside_top_left.y, "縦方向はそのまま");
        assert!(
            off_left.x < inside_top_left.x,
            "盤の外(左)に出た分だけ左にずれる: off_left={off_left:?} inside={inside_top_left:?}"
        );
    }

    #[test]
    fn top_rect_does_not_panic_far_outside_the_panel() {
        let layout = board_area(Rect::new(0, 0, 60, 20)).unwrap();
        // panelの範囲を大きく超える座標でもpanicせず、panelの範囲内に収まる
        let rect = layout.top_rect((-1000.0, -1000.0));
        assert!(layout.panel.contains(Position::new(rect.x, rect.y)));
        let rect = layout.top_rect((1000.0, 1000.0));
        assert!(layout.panel.contains(Position::new(rect.x, rect.y)));
    }

    // --- テキスト表示の盤面 ---

    #[test]
    fn renderer_without_picker_uses_text() {
        let renderer = BoardRenderer::new();
        assert!(
            !renderer.uses_image(),
            "テストでは端末に問い合わせず、アセットも無いのでテキスト"
        );
    }

    #[test]
    fn text_board_shows_bumps_goal_and_top() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 40, 12);
        let renderer = BoardRenderer::new();
        let top = top_at(board.start_position());
        let buffer = draw_board(&renderer, &board, &top, area);
        let bumps = (0..BOARD_HEIGHT)
            .flat_map(|y| (0..BOARD_WIDTH).map(move |x| (x, y)))
            .filter(|&(x, y)| board.cell(x, y) == Cell::Bump)
            .count();
        assert_eq!(count_symbol(&buffer, BUMP_GLYPH), bumps, "障害物は全部描く");
        let layout = board_area(area).unwrap();
        let (gx, gy) = board.goal();
        let goal = layout.cell_rect(gx, gy);
        assert_eq!(buffer[(goal.x, goal.y)].symbol(), GOAL_GLYPH);
        let top_rect = layout.top_rect(top.pos);
        assert_eq!(
            buffer[(top_rect.x, top_rect.y)].symbol(),
            TOP_SPIN_GLYPHS[0]
        );
        assert_eq!(
            buffer[(top_rect.x + 1, top_rect.y)].symbol(),
            " ",
            "記号の右隣は空白"
        );
        assert_eq!(
            buffer[(top_rect.x + 1, top_rect.y)].bg,
            FLAT_BG,
            "平坦なマスは木の色"
        );
    }

    /// 色の明るさ(輝度)
    fn luma(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("RGBの色: {color:?}");
        };
        0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)
    }

    fn pixel_luma(pixel: &Rgba<u8>) -> f64 {
        let [r, g, b, _] = pixel.0;
        luma(Color::Rgb(r, g, b))
    }

    /// 盤の中で最初に見つかるcellのマス
    fn first_cell(board: &Board, cell: Cell) -> (usize, usize) {
        (0..BOARD_HEIGHT)
            .flat_map(|y| (0..BOARD_WIDTH).map(move |x| (x, y)))
            .find(|&(x, y)| board.cell(x, y) == cell)
            .unwrap_or_else(|| panic!("{cell:?}のマスがある"))
    }

    #[test]
    fn text_board_distinguishes_hollow_from_bump() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 40, 12);
        let renderer = BoardRenderer::new();
        let top = top_at((0.5, 0.5));
        let buffer = draw_board(&renderer, &board, &top, area);
        let layout = board_area(area).unwrap();
        let hollows = (0..BOARD_HEIGHT)
            .flat_map(|y| (0..BOARD_WIDTH).map(move |x| (x, y)))
            .filter(|&(x, y)| board.cell(x, y) == Cell::Hollow)
            .count();
        assert!(hollows > 0);
        assert_eq!(count_symbol(&buffer, HOLLOW_GLYPH), hollows, "凹は全部描く");
        assert_ne!(HOLLOW_GLYPH, BUMP_GLYPH, "凹と凸は記号が違う");

        let (hx, hy) = first_cell(&board, Cell::Hollow);
        let (bx, by) = first_cell(&board, Cell::Bump);
        let hollow = layout.cell_rect(hx, hy);
        let bump = layout.cell_rect(bx, by);
        assert_eq!(buffer[(hollow.x, hollow.y)].symbol(), HOLLOW_GLYPH);
        assert_eq!(buffer[(bump.x, bump.y)].symbol(), BUMP_GLYPH);
        let (hollow_bg, bump_bg) = (buffer[(hollow.x, hollow.y)].bg, buffer[(bump.x, bump.y)].bg);
        assert_eq!(hollow_bg, HOLLOW_BG);
        assert_eq!(bump_bg, BUMP_BG);
        assert_eq!(
            buffer[(hollow.x + 1, hollow.y)].bg,
            HOLLOW_BG,
            "マス全体を凹の色で塗る"
        );
        assert!(luma(HOLLOW_BG) < luma(FLAT_BG), "凹は平坦より暗く沈んだ色");
        assert!(
            luma(BUMP_BG) > luma(FLAT_BG),
            "凸は平坦より明るく盛り上がった色"
        );
    }

    #[test]
    fn compose_base_paints_hollow_darker_than_bump() {
        let board = Board::standard();
        let font_size = (10, 20);
        let layout = board_area(Rect::new(0, 0, 40, 12)).unwrap();
        let composed = compose_base(&plain_board_image(), &board, layout, font_size);
        let origin = (layout.rect.x, layout.rect.y);
        // マスの中心から横にoffset(ピクセル)ずれた点の明るさ
        let luma_at = |(x, y): (usize, usize), offset: u32| {
            let (px, py, pw, ph) = pixel_rect(layout.cell_rect(x, y), origin, font_size);
            pixel_luma(composed.get_pixel(px + pw / 2 + offset, py + ph / 2))
        };
        let hollow = first_cell(&board, Cell::Hollow);
        let bump = first_cell(&board, Cell::Bump);
        let flat = (0, 0);
        assert_eq!(board.cell(flat.0, flat.1), Cell::Flat);
        assert!(
            luma_at(hollow, 0) < luma_at(flat, 0),
            "凹は暗い穴: {} < {}",
            luma_at(hollow, 0),
            luma_at(flat, 0)
        );
        assert!(
            luma_at(bump, 0) > luma_at(flat, 0),
            "凸の頂上は明るい: {} > {}",
            luma_at(bump, 0),
            luma_at(flat, 0)
        );
        // 凸は縁が暗く(盛り上がりの影)、頂上が明るい。1マスは20×20ピクセル
        assert!(luma_at(bump, 7) < luma_at(bump, 0), "凸の縁は頂上より暗い");
    }

    #[test]
    fn text_top_spins_and_changes_while_airborne() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 40, 12);
        let renderer = BoardRenderer::new();
        let layout = board_area(area).unwrap();
        let pos = board.start_position();
        let rect = layout.top_rect(pos);
        for (frame, glyph) in TOP_SPIN_GLYPHS.iter().enumerate() {
            let top = TopView {
                pos,
                airborne: false,
                spin_frame: frame,
            star_frame: None,
            };
            let buffer = draw_board(&renderer, &board, &top, area);
            assert_eq!(buffer[(rect.x, rect.y)].symbol(), *glyph);
        }
        let top = TopView {
            pos,
            airborne: true,
            spin_frame: 1,
            star_frame: None,
        };
        let buffer = draw_board(&renderer, &board, &top, area);
        assert_eq!(buffer[(rect.x, rect.y)].symbol(), TOP_AIRBORNE_GLYPH);
    }

    #[test]
    fn text_top_shows_the_star_animation_glyph_when_present() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 40, 12);
        let renderer = BoardRenderer::new();
        let layout = board_area(area).unwrap();
        let pos = board.start_position();
        let rect = layout.top_rect(pos);
        for (frame, glyph) in STAR_ANIM_GLYPHS.iter().enumerate() {
            let top = TopView {
                pos,
                airborne: false,
                spin_frame: 0,
                star_frame: Some(frame),
            };
            let buffer = draw_board(&renderer, &board, &top, area);
            assert_eq!(
                buffer[(rect.x, rect.y)].symbol(),
                *glyph,
                "star_frame={frame}では通常の回転記号ではなく星の演出を描く"
            );
        }
    }

    #[test]
    fn board_render_does_not_panic_in_tiny_areas() {
        let board = Board::standard();
        let renderer = BoardRenderer::new();
        let image_renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        for (w, h) in [(1, 1), (3, 2), (10, 4), (39, 11)] {
            let area = Rect::new(0, 0, w, h);
            draw_board(&renderer, &board, &top_at((19.9, 11.9)), area);
            draw_board(&image_renderer, &board, &top_at((19.9, 11.9)), area);
        }
    }

    // --- 画像表示の盤面(盤は1回だけ合成し、ベーゴマのマスが変わった時だけパッチを作り直す) ---

    #[test]
    fn image_board_is_encoded_once_while_nothing_moves() {
        let board = Board::standard();
        let renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        assert!(renderer.uses_image());
        let area = Rect::new(0, 0, 40, 12);
        let top = top_at(board.start_position());
        for _ in 0..3 {
            draw_board(&renderer, &board, &top, area);
        }
        assert_eq!(renderer.base_encode_count(), 1, "盤は1回だけ");
        assert_eq!(
            renderer.patch_encode_count(),
            1,
            "ベーゴマが動かなければパッチも作り直さない"
        );
    }

    #[test]
    fn patch_is_reencoded_only_when_the_top_changes_cells() {
        let board = Board::standard();
        let renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        let area = Rect::new(0, 0, 40, 12);
        draw_board(&renderer, &board, &top_at((1.2, 10.2)), area);
        // 同じマスの中で動いた・回転のコマが変わっただけなら作り直さない
        draw_board(&renderer, &board, &top_at((1.8, 10.7)), area);
        let spun = TopView {
            pos: (1.5, 10.5),
            airborne: false,
            spin_frame: 3,
            star_frame: None,
        };
        draw_board(&renderer, &board, &spun, area);
        assert_eq!(renderer.patch_encode_count(), 1);
        // 隣のマスへ移ったら作り直す。盤全体は作り直さない
        draw_board(&renderer, &board, &top_at((2.5, 10.5)), area);
        assert_eq!(renderer.patch_encode_count(), 2);
        draw_board(&renderer, &board, &top_at((3.5, 9.5)), area);
        assert_eq!(renderer.patch_encode_count(), 3);
        assert_eq!(renderer.base_encode_count(), 1);
        // 飛び上がった見た目に変わった時も作り直す
        let airborne = TopView {
            pos: (3.5, 9.5),
            airborne: true,
            spin_frame: 0,
            star_frame: None,
        };
        draw_board(&renderer, &board, &airborne, area);
        assert_eq!(renderer.patch_encode_count(), 4);
        assert_eq!(renderer.base_encode_count(), 1);
    }

    #[test]
    fn board_is_recomposed_when_the_area_changes() {
        let board = Board::standard();
        let renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        let top = top_at(board.start_position());
        draw_board(&renderer, &board, &top, Rect::new(0, 0, 40, 12));
        draw_board(&renderer, &board, &top, Rect::new(0, 0, 90, 26));
        assert_eq!(renderer.base_encode_count(), 2);
    }

    #[test]
    fn patch_covers_the_previous_and_current_top_cells() {
        let board = Board::standard();
        let renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        let area = Rect::new(0, 0, 40, 12);
        let layout = board_area(area).unwrap();
        draw_board(&renderer, &board, &top_at((1.5, 10.5)), area);
        draw_board(&renderer, &board, &top_at((2.5, 10.5)), area);
        let patch = renderer.patch.borrow().as_ref().map(|p| p.rect).unwrap();
        // 前の位置のベーゴマを消すため、前回と今回のマスを両方含む(盤全体ではない)
        assert_eq!(
            patch,
            layout.cell_rect(1, 10).union(layout.cell_rect(2, 10))
        );
    }

    // --- 軽トラ視点 ---

    #[test]
    fn truck_view_falls_back_to_text_without_images() {
        let renderer = TruckViewRenderer::new();
        assert!(!renderer.uses_image());
        let text = draw_truck(
            &renderer,
            &info(GForce::default(), None),
            Rect::new(0, 0, 30, 14),
        );
        assert!(text.contains("40km/h"), "速度をkm/hで出す: {text}");
        assert!(text.contains("前方:なし"), "{text}");
    }

    #[test]
    fn truck_view_switches_to_the_bracing_picture_under_g() {
        let renderer = TruckViewRenderer::new();
        let area = Rect::new(0, 0, 30, 14);
        let calm = draw_truck(&renderer, &info(GForce::default(), None), area);
        let braking = GForce {
            longitudinal: -0.5,
            lateral: 0.0,
        };
        let bracing = draw_truck(&renderer, &info(braking, None), area);
        assert!(!calm.contains("ふんばり"));
        assert!(bracing.contains("ふんばり"));
        assert_ne!(calm, bracing, "絵が切り替わる");
    }

    #[test]
    fn bracing_starts_at_the_brace_threshold() {
        let g = |value: f64| GForce {
            longitudinal: 0.0,
            lateral: value,
        };
        assert!(!info(g(BRACE_G - 0.01), None).bracing());
        assert!(info(g(BRACE_G), None).bracing());
        assert!(info(g(-0.5), None).bracing());
    }

    #[test]
    fn upcoming_text_names_each_event_with_its_distance() {
        let at = |kind: UpcomingKind| {
            upcoming_text(Some(Upcoming {
                kind,
                distance: 18.4,
            }))
        };
        assert_eq!(
            at(UpcomingKind::Signal(SignalLight::Yellow)),
            "前方: 信号(黄) あと18m"
        );
        assert_eq!(
            at(UpcomingKind::Signal(SignalLight::Red)),
            "前方: 信号(赤) あと18m"
        );
        assert_eq!(
            at(UpcomingKind::Signal(SignalLight::Green)),
            "前方: 信号(青) あと18m"
        );
        assert_eq!(at(UpcomingKind::Bump), "前方: 段差 あと18m");
        assert_eq!(
            at(UpcomingKind::Obstacle(Side::Right)),
            "前方: 障害物(右へよける) あと18m"
        );
        assert_eq!(
            at(UpcomingKind::Obstacle(Side::Left)),
            "前方: 障害物(左へよける) あと18m"
        );
        assert_eq!(upcoming_text(None), "前方: なし");
    }

    #[test]
    fn truck_view_shows_the_upcoming_signal_and_g() {
        let renderer = TruckViewRenderer::new();
        let upcoming = Some(Upcoming {
            kind: UpcomingKind::Signal(SignalLight::Yellow),
            distance: 25.0,
        });
        let g = GForce {
            longitudinal: -0.31,
            lateral: 0.0,
        };
        let text = draw_truck(&renderer, &info(g, upcoming), Rect::new(0, 0, 32, 14));
        assert!(text.contains("信号(黄)"), "{text}");
        assert!(text.contains("-0.31"), "前後のGを出す: {text}");
    }

    #[test]
    fn truck_view_does_not_panic_in_tiny_areas() {
        let renderer = TruckViewRenderer::new();
        for (w, h) in [(1, 1), (4, 2), (12, 5)] {
            draw_truck(
                &renderer,
                &info(GForce::default(), None),
                Rect::new(0, 0, w, h),
            );
        }
    }
}
