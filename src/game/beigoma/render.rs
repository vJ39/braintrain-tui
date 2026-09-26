//! べーの2視点(軽トラ視点・盤面視点)の描画。
//!
//! 画像アセット(assets/image/beigoma/配下)は埋め込まれていれば使い、無ければテキストで描く。
//! - 盤面: 画像プロトコル(sixel/kitty/iTerm2)が使え、盤の画像(board.png)が読める時だけ画像で描く。
//!   盤(障害物・ゴール)は固定なので、盤の画像は描画エリアが変わった時だけ1回合成・エンコードする。
//!   ベーゴマは盤の画像から「前回の位置と今回の位置」を包む小さな範囲だけを切り出して重ねたパッチにし、
//!   ベーゴマのマスが変わった時だけパッチを作り直す(count_mania/circle_image.rsと同じ考え方)。
//!   パッチはベーゴマ(スロット)ごとに持ち、他のベーゴマが自分のパッチの範囲に出入りした時以外は作り直さない。
//!   それ以外はマスごとに記号・色分けしたテキストで描く
//! - ベーゴマが複数(ROUND2)の時、同じマスにいるものはマスを横に分けて左右に並べて描き、重ねない
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

use super::board::{Board, Cell, Tilt, BOARD_HEIGHT, BOARD_WIDTH, TILT_MAX};
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
/// 転がっている間の回転の見た目(順に切り替える)。
/// 点字の2×4の点の外周8つのうち1つを欠けさせ、欠けた点を時計回りに1つずつ進める(8等分の回転)。
/// どのコマも1セル幅で、欠けた点の位置だけが変わる
pub const TOP_SPIN_GLYPHS: [&str; 8] = ["⣾", "⣷", "⣯", "⣟", "⡿", "⢿", "⣻", "⣽"];
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

/// 平坦なマスの市松の2トーン。FLAT_BGを白・黒へこの割合だけ寄せる
const CHECKER_MIX: f64 = 0.08;
/// 凹凸の陰影の明・暗。BUMP_BG/HOLLOW_BGを白・黒へこの割合だけ寄せる
const SHADE_MIX: f64 = 0.15;
/// 陰影の帯の境界。d ≤ −SHADE_BANDで左上の帯、d ≥ SHADE_BANDで右下の帯、その間が中央の帯
const SHADE_BAND: f64 = 0.2;
/// 混色で明るく・暗くする時の寄せ先
const WHITE: Color = Color::Rgb(255, 255, 255);
const BLACK: Color = Color::Rgb(0, 0, 0);

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

/// 星の演出のコマ(だんだん暗く・小さくなり、最後は消える)
pub const STAR_ANIM_GLYPHS: [&str; 8] = ["★", "☆", "✦", "✧", "∗", "⋆", "‥", "･"];
/// 星の演出の1コマの表示時間。最後のコマに達するまで(7コマ分)を従来(4コマ×180ms)の540ms並みにする
pub const STAR_ANIM_FRAME: Duration = Duration::from_millis(77);

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

/// 傾き最大(TILT_MAX)の時の盤の回転角(rad)。前後・左右とも同じ
const TILT_ANGLE_MAX: f64 = std::f64::consts::PI / 12.0; // 15°
/// カメラの高さ(マス単位)。盤の中心の真上
const CAMERA_HEIGHT: f64 = 20.0;

/// 盤の中心(マス座標)。変換は盤の中心を原点にして行う
const HALF_WIDTH: f64 = BOARD_WIDTH as f64 / 2.0;
const HALF_HEIGHT: f64 = BOARD_HEIGHT as f64 / 2.0;

/// 傾きに応じた、盤のマス座標と端末のセル座標の間の変換(1フレームに1回作る)。
/// 盤を3D空間でroll→pitchの順に回し、盤の中心の真上のカメラから透視投影する。
/// 傾けた方向の遠い側(低くなった側)が縮んだ台形になり、近い側の辺は平らな時と同じ大きさに保つ
struct BoardProjection {
    /// 盤の左上のセル位置と1マスのセル数(BoardAreaから)
    origin: (f64, f64),
    cell: (f64, f64),
    /// 正規化済みの回転係数と透視の係数
    a: f64,
    b: f64,
    c: f64,
    p: f64,
    q: f64,
}

impl BoardProjection {
    fn new(layout: &BoardArea, tilt: &Tilt) -> Self {
        let theta_p = tilt.pitch() / TILT_MAX * TILT_ANGLE_MAX;
        let theta_r = tilt.roll() / TILT_MAX * TILT_ANGLE_MAX;
        let (sin_p, cos_p) = theta_p.sin_cos();
        let (sin_r, cos_r) = theta_r.sin_cos();
        // w = 1 + p·u + q·v(wが大きいほどカメラから遠く、小さく見える)
        let p = sin_r * cos_p / CAMERA_HEIGHT;
        let q = -sin_p / CAMERA_HEIGHT;
        // 盤の4隅で最もカメラに近い(wが最小の)角に合わせて全体を拡大し、近い側の辺を平らな時と同じ大きさにする
        let w_min = [
            (-HALF_WIDTH, -HALF_HEIGHT),
            (HALF_WIDTH, -HALF_HEIGHT),
            (-HALF_WIDTH, HALF_HEIGHT),
            (HALF_WIDTH, HALF_HEIGHT),
        ]
        .into_iter()
        .map(|(u, v)| 1.0 + p * u + q * v)
        .fold(f64::INFINITY, f64::min);
        let s = 1.0 / w_min;
        Self {
            origin: (f64::from(layout.rect.x), f64::from(layout.rect.y)),
            cell: (f64::from(layout.cell_width), f64::from(layout.cell_height)),
            a: cos_r / s,
            b: sin_r * sin_p / s,
            c: cos_p / s,
            p,
            q,
        }
    }

    /// マス座標(連続値)→セル座標(連続値)。盤外の座標もそのまま延長して変換する
    fn project(&self, pos: (f64, f64)) -> (f64, f64) {
        let u = pos.0 - HALF_WIDTH;
        let v = pos.1 - HALF_HEIGHT;
        let w = 1.0 + self.p * u + self.q * v;
        let sx = self.a * u / w;
        let sy = (self.b * u + self.c * v) / w;
        (
            self.origin.0 + (sx + HALF_WIDTH) * self.cell.0,
            self.origin.1 + (sy + HALF_HEIGHT) * self.cell.1,
        )
    }

    /// セル座標(連続値)→マス座標(連続値)。地平線の向こう側はNone
    fn unproject(&self, screen: (f64, f64)) -> Option<(f64, f64)> {
        let sx = (screen.0 - self.origin.0) / self.cell.0 - HALF_WIDTH;
        let sy = (screen.1 - self.origin.1) / self.cell.1 - HALF_HEIGHT;
        let den = 1.0 - self.p * sx / self.a - self.q * (sy - self.b * sx / self.a) / self.c;
        if den <= 0.0 {
            return None;
        }
        let w = 1.0 / den;
        let u = sx * w / self.a;
        let v = (sy - self.b * sx / self.a) * w / self.c;
        Some((u + HALF_WIDTH, v + HALF_HEIGHT))
    }
}

/// 中心座標(セル、連続値)から記号を置くセルを決める。記号が2セル幅で表示されても隣のマスに
/// 食い込まないよう、中心のすぐ左のセルに置く(傾き0では従来の記号の位置と同じになる)
fn glyph_cell(center: (f64, f64)) -> (i32, i32) {
    ((center.0 - 0.5).floor() as i32, center.1.floor() as i32)
}

/// cellがclipの中なら記号を置く(範囲外は何もしない)。背景色はそのまま残す
fn put_glyph_at(buffer: &mut Buffer, cell: (i32, i32), clip: Rect, glyph: &str, style: Style) {
    let (Ok(x), Ok(y)) = (u16::try_from(cell.0), u16::try_from(cell.1)) else {
        return;
    };
    let position = Position::new(x, y);
    if clip.contains(position) {
        buffer[position].set_symbol(glyph).set_style(style);
    }
}

/// 直前に作った盤の画像(障害物・ゴールまで重ねたもの)
struct BaseCache {
    area: BoardArea,
    composed: RgbaImage,
    protocol: StatefulProtocol,
}

/// 同じ場所に描くベーゴマを横に並べるための区画。countは同じ場所にいる数、indexは左から何番目か
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Lane {
    index: usize,
    count: usize,
}

/// 各ベーゴマの区画。keys(topsと同じ並びの描く場所)が同じベーゴマどうしを、位置のxが小さい順
/// (同じならスライスの順)に左から並べる。1個だけなら区画は1つ(従来どおりの描き方)
fn lanes_by<K: PartialEq>(tops: &[TopView], keys: &[K]) -> Vec<Lane> {
    let is_left_of = |a: usize, b: usize| {
        tops[a]
            .pos
            .0
            .total_cmp(&tops[b].pos.0)
            .then(a.cmp(&b))
            .is_lt()
    };
    (0..tops.len())
        .map(|i| {
            let same = (0..tops.len()).filter(|&j| keys[j] == keys[i]);
            Lane {
                index: same.clone().filter(|&j| is_left_of(j, i)).count(),
                count: same.count(),
            }
        })
        .collect()
}

/// 画像表示で1個のベーゴマを描く場所と見た目(ベーゴマのマスのセル範囲, 飛び上がっているか, 区画)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TopMark {
    rect: Rect,
    airborne: bool,
    lane: Lane,
}

/// 直前に作ったベーゴマのパッチ。keyはこのスロットのベーゴマの描き方、drawnはパッチに描き込んだ
/// 全ベーゴマ(範囲に入っている他のベーゴマも含む。後から描いたパッチが他のベーゴマを消さないように)
struct PatchCache {
    rect: Rect,
    key: TopMark,
    drawn: Vec<TopMark>,
    protocol: StatefulProtocol,
}

/// marksのうち、rectに掛かるもの
fn marks_within(marks: &[TopMark], rect: Rect) -> Vec<TopMark> {
    marks
        .iter()
        .copied()
        .filter(|mark| mark.rect.intersects(rect))
        .collect()
}

/// 盤面視点の描画器
pub struct BoardRenderer {
    picker: Option<Picker>,
    board_image: Option<RgbaImage>,
    top_image: Option<RgbaImage>,
    base: RefCell<Option<BaseCache>>,
    /// ベーゴマ(スロット)ごとのパッチ。要素数は描くベーゴマの数に合わせる
    patch: RefCell<Vec<Option<PatchCache>>>,
    /// テスト用: 盤の画像・パッチ(スロットごと)を作り直した回数
    #[cfg(test)]
    base_encodes: std::cell::Cell<usize>,
    #[cfg(test)]
    patch_encodes: RefCell<Vec<usize>>,
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
            patch: RefCell::new(Vec::new()),
            #[cfg(test)]
            base_encodes: std::cell::Cell::new(0),
            #[cfg(test)]
            patch_encodes: RefCell::new(Vec::new()),
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

    /// 全スロットのパッチを作り直した回数の合計
    #[cfg(test)]
    pub fn patch_encode_count(&self) -> usize {
        self.patch_encodes.borrow().iter().sum()
    }

    /// slot番目のベーゴマのパッチを作り直した回数
    #[cfg(test)]
    pub fn slot_patch_encode_count(&self, slot: usize) -> usize {
        self.patch_encodes.borrow().get(slot).copied().unwrap_or(0)
    }

    /// area(盤面パネルの内側)に盤とベーゴマ(tops。ROUND1は1個、ROUND2は2個)を描く。
    /// 傾き(tilt)による疑似3D変形はテキスト表示のみで、画像表示(render_image)には適用しない
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        board: &Board,
        tops: &[TopView],
        tilt: &Tilt,
    ) {
        let area = area.intersection(frame.area());
        let Some(layout) = board_area(area) else {
            return;
        };
        // 星の演出中は画像のパッチ更新に乗せず、テキストで星を描く
        let starring = tops.iter().any(|top| top.star_frame.is_some());
        if !starring && self.render_image(frame, area, layout, board, tops) {
            return;
        }
        render_board_text(frame, area, layout, board, tops, tilt);
    }

    /// 画像で描く。画像を使えない・盤全体がエリアに収まらない時はfalse(テキストで描く)
    fn render_image(
        &self,
        frame: &mut Frame,
        area: Rect,
        layout: BoardArea,
        board: &Board,
        tops: &[TopView],
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
            self.patch.borrow_mut().clear();
            #[cfg(test)]
            self.base_encodes.set(self.base_encodes.get() + 1);
        }
        let Some(cached) = base.as_mut() else {
            return false;
        };
        frame.render_stateful_widget(StatefulImage::default(), layout.rect, &mut cached.protocol);
        // パッチは盤の画像より後に描き、盤の上に重ねる
        self.render_patches(frame, picker, cached, tops);
        true
    }

    /// テスト用: slot番目のパッチを作り直した回数を数える(テスト以外では何もしない)
    fn note_patch_encode(&self, _slot: usize) {
        #[cfg(test)]
        {
            let mut counts = self.patch_encodes.borrow_mut();
            if counts.len() <= _slot {
                counts.resize(_slot + 1, 0);
            }
            counts[_slot] += 1;
        }
    }

    /// ベーゴマのパッチをスロットごとに盤の上に描く。各スロットは自分のマス(と飛び上がりの見た目・区画)が
    /// 変わった時、または他のベーゴマが自分のパッチの範囲に出入りした時だけ作り直す。
    /// 作り直す時は前回のマスも含めて切り出し、前の位置のベーゴマを端末に残さない
    fn render_patches(
        &self,
        frame: &mut Frame,
        picker: &Picker,
        base: &BaseCache,
        tops: &[TopView],
    ) {
        let rects: Vec<Rect> = tops.iter().map(|top| base.area.top_rect(top.pos)).collect();
        let lanes = lanes_by(tops, &rects);
        let marks: Vec<TopMark> = tops
            .iter()
            .zip(rects.iter().zip(lanes))
            .map(|(top, (&rect, lane))| TopMark {
                rect,
                airborne: top.airborne,
                lane,
            })
            .collect();
        let mut patches = self.patch.borrow_mut();
        // ベーゴマの数が変わった(ROUND1→ROUND2)時はスロットの数を合わせる
        patches.resize_with(marks.len(), || None);
        for (slot, (patch, &mark)) in patches.iter_mut().zip(&marks).enumerate() {
            let rect = match patch.as_ref() {
                Some(cached) if cached.key == mark => cached.rect,
                Some(cached) => cached.key.rect.union(mark.rect),
                None => mark.rect,
            };
            let drawn = marks_within(&marks, rect);
            let fresh = matches!(patch.as_ref(), Some(cached) if cached.key == mark && cached.drawn == drawn);
            if fresh {
                continue;
            }
            let image = patch_image(
                base,
                picker.font_size(),
                rect,
                &drawn,
                self.top_image.as_ref(),
            );
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(image));
            *patch = Some(PatchCache {
                rect,
                key: mark,
                drawn,
                protocol,
            });
            self.note_patch_encode(slot);
        }
        for cached in patches.iter_mut().flatten() {
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

/// 盤の合成画像からrectを切り出し、marksの各ベーゴマを重ねたパッチ。
/// 同じマスに複数いるベーゴマは、マスを横に区画の数で等分した自分の区画に描く
fn patch_image(
    base: &BaseCache,
    font_size: (u16, u16),
    rect: Rect,
    marks: &[TopMark],
    top_image: Option<&RgbaImage>,
) -> RgbaImage {
    let origin = (base.area.rect.x, base.area.rect.y);
    let (px, py, pw, ph) = pixel_rect(rect, origin, font_size);
    let mut patch = imageops::crop_imm(&base.composed, px, py, pw.max(1), ph.max(1)).to_image();
    let (fw, fh) = cell_pixels(font_size);
    for mark in marks {
        // パッチの左上からのピクセル位置(パッチの外にはみ出す分は描かれない)
        let tx = (f64::from(mark.rect.x) - f64::from(rect.x)) * f64::from(fw);
        let ty = (f64::from(mark.rect.y) - f64::from(rect.y)) * f64::from(fh);
        let tw = f64::from(mark.rect.width) * f64::from(fw);
        let th = f64::from(mark.rect.height) * f64::from(fh);
        let lane_width = tw / mark.lane.count.max(1) as f64;
        // 飛び上がっている間は小さく描き、浮いているように見せる。
        // 通常時はマスいっぱいに近い大きさにして豆粒にならないようにする
        let scale = if mark.airborne { 0.7 } else { 0.95 };
        let size = (lane_width.min(th) * scale).max(1.0) as u32;
        let center = (
            tx + lane_width * (mark.lane.index as f64 + 0.5),
            ty + th / 2.0,
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

/// 凹凸のマスの中の陰影の帯。光源は左上
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShadeBand {
    TopLeft,
    Middle,
    BottomRight,
}

/// マス内の位置frac = (fu, fv)から陰影の帯を決める。d = fu + fv − 1 で、d ≤ −SHADE_BANDなら左上、
/// d ≥ SHADE_BANDなら右下、その間が中央。d == ∓SHADE_BANDちょうどが外側の帯に入るよう、
/// 1を引かずに fu + fv と 1 ∓ SHADE_BAND を比べる(1を引くと丸め誤差で境界がずれる)
fn shade_band(frac: (f64, f64)) -> ShadeBand {
    let sum = frac.0 + frac.1;
    if sum <= 1.0 - SHADE_BAND {
        ShadeBand::TopLeft
    } else if sum >= 1.0 + SHADE_BAND {
        ShadeBand::BottomRight
    } else {
        ShadeBand::Middle
    }
}

/// colorをtargetへ割合tだけ寄せた色(各チャンネルを四捨五入)。Color::Rgb以外はそのまま
fn mix(color: Color, target: Color, t: f64) -> Color {
    let (Color::Rgb(r, g, b), Color::Rgb(tr, tg, tb)) = (color, target) else {
        return color;
    };
    let channel = |c: u8, tc: u8| {
        let c = f64::from(c);
        (c + (f64::from(tc) - c) * t).round().clamp(0.0, 255.0) as u8
    };
    Color::Rgb(channel(r, tr), channel(g, tg), channel(b, tb))
}

/// テキスト表示のマスの記号。平坦なマスは空白
fn cell_glyph(cell: Cell) -> &'static str {
    match cell {
        Cell::Flat => " ",
        Cell::Bump => BUMP_GLYPH,
        Cell::Hollow => HOLLOW_GLYPH,
        Cell::Goal => GOAL_GLYPH,
    }
}

/// マスmass = (mx, my)のマス内位置fracにあるセルのスタイル。
/// 背景色は市松(平坦)・陰影(凸凹)・一様(ゴール)、記号の色と太字はマスの種類で決まる
fn cell_style(cell: Cell, mass: (usize, usize), frac: (f64, f64)) -> Style {
    match cell {
        Cell::Flat => {
            // 市松: mx + myが偶数なら明るいトーン、奇数なら暗いトーン
            let target = if (mass.0 + mass.1).is_multiple_of(2) {
                WHITE
            } else {
                BLACK
            };
            Style::default().bg(mix(FLAT_BG, target, CHECKER_MIX))
        }
        Cell::Bump => {
            // 盛り上がった面の左上に光が当たり、右下に影が落ちる
            let bg = match shade_band(frac) {
                ShadeBand::TopLeft => mix(BUMP_BG, WHITE, SHADE_MIX),
                ShadeBand::Middle => BUMP_BG,
                ShadeBand::BottomRight => mix(BUMP_BG, BLACK, SHADE_MIX),
            };
            Style::default()
                .fg(BUMP_FG)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        }
        Cell::Hollow => {
            // くぼみの左上の内壁が影になり、右下の内壁に光が当たる
            let bg = match shade_band(frac) {
                ShadeBand::TopLeft => mix(HOLLOW_BG, BLACK, SHADE_MIX),
                ShadeBand::Middle => HOLLOW_BG,
                ShadeBand::BottomRight => mix(HOLLOW_BG, WHITE, SHADE_MIX),
            };
            Style::default().fg(HOLLOW_FG).bg(bg)
        }
        Cell::Goal => Style::default()
            .fg(GOAL_FG)
            .bg(GOAL_BG)
            .add_modifier(Modifier::BOLD),
    }
}

/// テキスト表示の盤面。マスごとに背景色で塗り(平坦なマスは市松、凹凸は左上から光を当てた陰影)、
/// 凸・凹・ゴール・ベーゴマは記号で示す(凸は明るい色の▲、凹は暗い色の▽)。傾き(tilt)に応じて盤を疑似3Dで台形に変形して描く。
/// ベーゴマが同じ場所に複数いる時は、右隣のセルへ1つずつずらして並べる
fn render_board_text(
    frame: &mut Frame,
    area: Rect,
    layout: BoardArea,
    board: &Board,
    tops: &[TopView],
    tilt: &Tilt,
) {
    let projection = BoardProjection::new(&layout, tilt);
    let buffer = frame.buffer_mut();
    // 背景: セルの中心を逆変換し、盤のマスに当たるセルをそのマスの色で塗る
    // (逆変換で塗るので、変形しても隙間・重なりが出ない。盤の外のセルは触らない)
    let (width, height) = (BOARD_WIDTH as f64, BOARD_HEIGHT as f64);
    for position in area.positions() {
        let center = (f64::from(position.x) + 0.5, f64::from(position.y) + 0.5);
        let Some((u, v)) = projection.unproject(center) else {
            continue;
        };
        if !((0.0..width).contains(&u) && (0.0..height).contains(&v)) {
            continue;
        }
        // 属するマスとマス内位置で色を決める(平坦は市松、凹凸は陰影の帯)
        let mass = (u as usize, v as usize);
        let frac = (u - mass.0 as f64, v - mass.1 as f64);
        let style = cell_style(board.cell(mass.0, mass.1), mass, frac);
        buffer[position].set_symbol(" ").set_style(style);
    }
    // 凸・凹・ゴールの記号: マスの中心を順変換したセルに、記号だけ置く(背景色は塗ったまま)
    for y in 0..BOARD_HEIGHT {
        for x in 0..BOARD_WIDTH {
            let cell = board.cell(x, y);
            if cell == Cell::Flat {
                continue;
            }
            let glyph = cell_glyph(cell);
            let center = projection.project((x as f64 + 0.5, y as f64 + 0.5));
            put_glyph_at(buffer, glyph_cell(center), area, glyph, Style::default());
        }
    }
    // 背景色はマスのものを残し、記号と文字色だけ変える
    let style = Style::default().fg(TOP_FG).add_modifier(Modifier::BOLD);
    let panel = layout.panel;
    let (left, right) = (i32::from(panel.left()), i32::from(panel.right()) - 1);
    // 位置を含むマスの中心を順変換する。場外に出た時は盤の外側の位置になり、panelの範囲に収める
    let cells: Vec<(i32, i32)> = tops
        .iter()
        .map(|top| {
            let center = projection.project((top.pos.0.floor() + 0.5, top.pos.1.floor() + 0.5));
            let (x, y) = glyph_cell(center);
            (
                x.clamp(left, right),
                y.clamp(i32::from(panel.top()), i32::from(panel.bottom()) - 1),
            )
        })
        .collect();
    // 同じセルになったベーゴマは、左から順に右隣のセルへずらす(panelの右端を越えないよう左へ寄せる)
    let lanes = lanes_by(tops, &cells);
    for ((top, &(x, y)), lane) in tops.iter().zip(&cells).zip(lanes) {
        let first = x.min(right - (lane.count as i32 - 1)).max(left);
        let cell = ((first + lane.index as i32).min(right), y);
        put_glyph_at(buffer, cell, area, top_glyph(top), style);
    }
}

/// ベーゴマの記号。星の演出中はそのコマ、飛び上がっている間は専用の記号、それ以外は回転のコマ
fn top_glyph(top: &TopView) -> &'static str {
    if let Some(frame) = top.star_frame {
        STAR_ANIM_GLYPHS[frame.min(STAR_ANIM_GLYPHS.len() - 1)]
    } else if top.airborne {
        TOP_AIRBORNE_GLYPH
    } else {
        TOP_SPIN_GLYPHS[top.spin_frame % TOP_SPIN_GLYPHS.len()]
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
    /// ゲーム開始からの経過時間。揺れのアニメーションの位相に使う
    pub elapsed: Duration,
}

impl TruckViewInfo {
    /// 女の子を踏ん張り時の絵にするか(Gがかかっている間)
    pub fn bracing(&self) -> bool {
        self.g.magnitude() >= BRACE_G
    }
}

/// 前方のイベントを強調表示する距離(m)。これより近いと目立たせる
const URGENT_DISTANCE: f64 = 15.0;

/// 前方のイベントが強調表示するほど近いか
fn upcoming_is_urgent(upcoming: Option<Upcoming>) -> bool {
    upcoming.is_some_and(|u| u.distance <= URGENT_DISTANCE)
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

/// 揺れの周期。Gの大きさによらず一定にし、振幅だけをGで変える
const SHAKE_PERIOD_MS: f64 = 90.0;
/// Gの大きさ1あたりの揺れの振幅(セル)。#135の巡航中の微振動(CRUISE_VIBRATION_G)程度の
/// Gでも、ごく僅かに動いているのが分かる強さにする
const SHAKE_AMPLITUDE_PER_G: f64 = 8.0;

/// Gの大きさとelapsed(経過時間)から、女の子の絵の描画位置のずれ(x, y。セル単位の小数)を求める。
/// 前後・左右で異なる位相にして、単調な往復に見えないようにする
fn shake_offset(g: GForce, elapsed: Duration) -> (f64, f64) {
    let t = elapsed.as_secs_f64() * 1000.0 / SHAKE_PERIOD_MS * std::f64::consts::TAU;
    let amplitude = g.magnitude() * SHAKE_AMPLITUDE_PER_G;
    (t.sin() * amplitude, (t * 1.3).cos() * amplitude * 0.6)
}

/// rectをdx,dyだけずらす(boundsの範囲内でクランプし、はみ出さないようにする)
fn offset_within(rect: Rect, dx: i32, dy: i32, bounds: Rect) -> Rect {
    let min_x = i32::from(bounds.x);
    let min_y = i32::from(bounds.y);
    let max_x = min_x + i32::from(bounds.width) - i32::from(rect.width);
    let max_y = min_y + i32::from(bounds.height) - i32::from(rect.height);
    let x = (i32::from(rect.x) + dx).clamp(min_x, max_x.max(min_x));
    let y = (i32::from(rect.y) + dy).clamp(min_y, max_y.max(min_y));
    Rect::new(x as u16, y as u16, rect.width, rect.height)
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

        let mut header_style = Style::default()
            .fg(upcoming_color(info.upcoming))
            .add_modifier(Modifier::BOLD);
        if upcoming_is_urgent(info.upcoming) {
            // 近づいてきたら前景色と背景色を反転させ、目立たせる
            header_style = header_style.add_modifier(Modifier::REVERSED);
        }
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                upcoming_text(info.upcoming),
                header_style,
            )))
            .alignment(Alignment::Center),
            header,
        );

        let bracing = info.bracing();
        let (shake_x, shake_y) = shake_offset(info.g, info.elapsed);
        let shaken_picture = offset_within(
            picture,
            shake_x.round() as i32,
            shake_y.round() as i32,
            area,
        );
        self.render_picture(frame, shaken_picture, bracing);

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
    use super::super::board::{TiltKey, TILT_STEP};
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
        draw_board_with_tilt(renderer, board, top, area, &Tilt::default())
    }

    /// ベーゴマ1個を描く(ROUND1と同じく要素数1のスライスで渡す)
    fn draw_board_with_tilt(
        renderer: &BoardRenderer,
        board: &Board,
        top: &TopView,
        area: Rect,
        tilt: &Tilt,
    ) -> Buffer {
        draw_tops_with_tilt(renderer, board, std::slice::from_ref(top), area, tilt)
    }

    fn draw_tops(renderer: &BoardRenderer, board: &Board, tops: &[TopView], area: Rect) -> Buffer {
        draw_tops_with_tilt(renderer, board, tops, area, &Tilt::default())
    }

    fn draw_tops_with_tilt(
        renderer: &BoardRenderer,
        board: &Board,
        tops: &[TopView],
        area: Rect,
        tilt: &Tilt,
    ) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(area.right(), area.bottom())).unwrap();
        terminal
            .draw(|frame| renderer.render(frame, area, board, tops, tilt))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn spinning_at(pos: (f64, f64), spin_frame: usize) -> TopView {
        TopView {
            spin_frame,
            ..top_at(pos)
        }
    }

    /// symbolが描かれたセルの位置の一覧
    fn positions_of(buffer: &Buffer, symbol: &str) -> Vec<(u16, u16)> {
        buffer
            .content()
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.symbol() == symbol)
            .map(|(i, _)| buffer.pos_of(i))
            .collect()
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
            elapsed: Duration::ZERO,
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

    // --- 傾きによる疑似3D変形(BoardProjection) ---

    /// テスト用: press()を繰り返して指定レベルまで傾ける
    fn tilted(pitch_presses: i32, roll_presses: i32) -> Tilt {
        let mut tilt = Tilt::new();
        for _ in 0..pitch_presses.unsigned_abs() {
            tilt.press(if pitch_presses > 0 {
                TiltKey::Forward
            } else {
                TiltKey::Back
            });
        }
        for _ in 0..roll_presses.unsigned_abs() {
            tilt.press(if roll_presses > 0 {
                TiltKey::Right
            } else {
                TiltKey::Left
            });
        }
        tilt
    }

    /// テスト用: 各軸を最大(±TILT_MAX)まで傾ける。符号は+1で+側、-1で−側、0で傾けない
    fn max_tilt(pitch_sign: i32, roll_sign: i32) -> Tilt {
        let presses = (TILT_MAX / TILT_STEP).ceil() as i32 + 1;
        let tilt = tilted(pitch_sign * presses, roll_sign * presses);
        assert_eq!(tilt.pitch(), f64::from(pitch_sign) * TILT_MAX);
        assert_eq!(tilt.roll(), f64::from(roll_sign) * TILT_MAX);
        tilt
    }

    /// テスト用: 変換の性質を確かめる傾き(各軸最大・組み合わせ最大・途中の値)
    fn tilts_to_check() -> Vec<Tilt> {
        vec![
            max_tilt(1, 0),
            max_tilt(-1, 0),
            max_tilt(0, 1),
            max_tilt(0, -1),
            max_tilt(1, 1),
            max_tilt(1, -1),
            max_tilt(-1, 1),
            max_tilt(-1, -1),
            tilted(2, -3),
        ]
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    /// 盤の描画範囲の中心(セル座標)
    fn board_center(layout: &BoardArea) -> (f64, f64) {
        (
            f64::from(layout.rect.x) + f64::from(layout.rect.width) / 2.0,
            f64::from(layout.rect.y) + f64::from(layout.rect.height) / 2.0,
        )
    }

    /// セル座標を、盤の描画範囲の中心からのずれ(マス単位)にする
    fn masses_from_center(layout: &BoardArea, screen: (f64, f64)) -> (f64, f64) {
        let (cx, cy) = board_center(layout);
        (
            (screen.0 - cx) / f64::from(layout.cell_width),
            (screen.1 - cy) / f64::from(layout.cell_height),
        )
    }

    /// 盤の行row(マス座標のy)の、投影後の横幅(セル)
    fn projected_width(projection: &BoardProjection, row: f64) -> f64 {
        projection.project((BOARD_WIDTH as f64, row)).0 - projection.project((0.0, row)).0
    }

    /// 盤の列column(マス座標のx)の、投影後の高さ(セル)
    fn projected_height(projection: &BoardProjection, column: f64) -> f64 {
        projection.project((column, BOARD_HEIGHT as f64)).1 - projection.project((column, 0.0)).1
    }

    #[test]
    fn flat_projection_matches_cell_rects_and_round_trips() {
        let layouts = [
            board_area(Rect::new(0, 0, 40, 12)).unwrap(),
            board_area(Rect::new(3, 2, 90, 26)).unwrap(),
            // 1マスの大きさが任意(縦横比も任意)でも同じ
            BoardArea {
                rect: Rect::new(5, 7, 60, 84),
                cell_width: 3,
                cell_height: 7,
                panel: Rect::new(0, 0, 250, 120),
            },
        ];
        for layout in layouts {
            let projection = BoardProjection::new(&layout, &Tilt::default());
            for y in 0..=BOARD_HEIGHT {
                for x in 0..=BOARD_WIDTH {
                    let rect = layout.cell_rect(x, y);
                    assert_eq!(
                        projection.project((x as f64, y as f64)),
                        (f64::from(rect.x), f64::from(rect.y)),
                        "傾き0ではマスの角がcell_rectの位置に一致する: ({x}, {y}) {layout:?}"
                    );
                }
            }
            for pos in [
                (0.0, 0.0),
                (3.5, 2.25),
                (19.75, 11.5),
                (10.0, 6.0),
                (7.125, 9.875),
            ] {
                assert_eq!(
                    projection.unproject(projection.project(pos)),
                    Some(pos),
                    "傾き0の往復は元に戻る: {pos:?}"
                );
            }
        }
    }

    #[test]
    fn flat_glyph_cell_matches_the_previous_glyph_position() {
        let mut widths = Vec::new();
        for area in [
            Rect::new(0, 0, 40, 12),
            Rect::new(1, 1, 90, 26),
            Rect::new(0, 0, 200, 60),
        ] {
            let layout = board_area(area).unwrap();
            widths.push(layout.cell_width);
            let projection = BoardProjection::new(&layout, &Tilt::default());
            for y in 0..BOARD_HEIGHT {
                for x in 0..BOARD_WIDTH {
                    let rect = layout.cell_rect(x, y);
                    let previous = (
                        i32::from(rect.x + (rect.width - 2) / 2),
                        i32::from(rect.y + rect.height / 2),
                    );
                    let center = projection.project((x as f64 + 0.5, y as f64 + 0.5));
                    assert_eq!(glyph_cell(center), previous, "({x}, {y}) {layout:?}");
                }
            }
        }
        assert_eq!(
            widths,
            vec![2, 4, 10],
            "1マスの横幅2・4・10の3通りを確かめる"
        );
    }

    #[test]
    fn the_board_center_stays_put_at_any_tilt() {
        let layout = board_area(Rect::new(2, 1, 90, 26)).unwrap();
        for tilt in tilts_to_check() {
            let projection = BoardProjection::new(&layout, &tilt);
            let center = projection.project((BOARD_WIDTH as f64 / 2.0, BOARD_HEIGHT as f64 / 2.0));
            assert!(
                close(center, board_center(&layout)),
                "盤の中心は動かない: {center:?} {tilt:?}"
            );
        }
    }

    #[test]
    fn pitching_narrows_the_far_edge_and_keeps_the_near_edge() {
        let layout = board_area(Rect::new(0, 0, 40, 12)).unwrap();
        let flat = BoardProjection::new(&layout, &Tilt::default());
        let (top_row, bottom_row) = (0.0, BOARD_HEIGHT as f64);
        let (cx, _) = board_center(&layout);

        let forward = BoardProjection::new(&layout, &max_tilt(1, 0));
        let (top, bottom) = (
            projected_width(&forward, top_row),
            projected_width(&forward, bottom_row),
        );
        assert!(top < bottom, "前傾は奥(上の行)が狭い: {top} < {bottom}");
        assert!(
            (bottom - projected_width(&flat, bottom_row)).abs() < 1e-9,
            "近い側(下の行)の横幅は平らな時と同じ"
        );
        for (bx, by) in [(0.0, 0.0), (3.0, 5.0), (7.5, 12.0), (1.25, 9.5)] {
            let left = forward.project((bx, by));
            let right = forward.project((BOARD_WIDTH as f64 - bx, by));
            assert!(
                (left.0 + right.0 - 2.0 * cx).abs() < 1e-9 && (left.1 - right.1).abs() < 1e-9,
                "pitchだけなら左右対称: {left:?} {right:?}"
            );
        }

        let back = BoardProjection::new(&layout, &max_tilt(-1, 0));
        let (top, bottom) = (
            projected_width(&back, top_row),
            projected_width(&back, bottom_row),
        );
        assert!(bottom < top, "後傾は手前(下の行)が狭い: {bottom} < {top}");
        assert!((top - projected_width(&flat, top_row)).abs() < 1e-9);
    }

    #[test]
    fn rolling_shortens_the_far_edge_and_keeps_the_near_edge() {
        let layout = board_area(Rect::new(0, 0, 40, 12)).unwrap();
        let flat = BoardProjection::new(&layout, &Tilt::default());
        let (left_column, right_column) = (0.0, BOARD_WIDTH as f64);
        let (_, cy) = board_center(&layout);

        let right_tilt = BoardProjection::new(&layout, &max_tilt(0, 1));
        let (left, right) = (
            projected_height(&right_tilt, left_column),
            projected_height(&right_tilt, right_column),
        );
        assert!(right < left, "右傾は右の縁が低い: {right} < {left}");
        assert!(
            (left - projected_height(&flat, left_column)).abs() < 1e-9,
            "近い側(左の縁)の高さは平らな時と同じ"
        );
        for (bx, by) in [(0.0, 0.0), (3.0, 5.0), (20.0, 1.5), (12.5, 2.25)] {
            let upper = right_tilt.project((bx, by));
            let lower = right_tilt.project((bx, BOARD_HEIGHT as f64 - by));
            assert!(
                (upper.1 + lower.1 - 2.0 * cy).abs() < 1e-9 && (upper.0 - lower.0).abs() < 1e-9,
                "rollだけなら上下対称: {upper:?} {lower:?}"
            );
        }

        let left_tilt = BoardProjection::new(&layout, &max_tilt(0, -1));
        let (left, right) = (
            projected_height(&left_tilt, left_column),
            projected_height(&left_tilt, right_column),
        );
        assert!(left < right, "左傾は左の縁が低い: {left} < {right}");
        assert!((right - projected_height(&flat, right_column)).abs() < 1e-9);
    }

    #[test]
    fn the_far_edge_shrinks_visibly_but_not_too_much_at_max_tilt() {
        let layout = board_area(Rect::new(0, 0, 40, 12)).unwrap();
        let pitch = BoardProjection::new(&layout, &max_tilt(1, 0));
        let pitch_ratio =
            projected_width(&pitch, 0.0) / projected_width(&pitch, BOARD_HEIGHT as f64);
        assert!(
            (0.80..=0.90).contains(&pitch_ratio),
            "pitch最大の遠い辺/近い辺: {pitch_ratio}"
        );
        let roll = BoardProjection::new(&layout, &max_tilt(0, 1));
        let roll_ratio = projected_height(&roll, BOARD_WIDTH as f64) / projected_height(&roll, 0.0);
        assert!(
            (0.70..=0.85).contains(&roll_ratio),
            "roll最大の遠い辺/近い辺: {roll_ratio}"
        );
    }

    #[test]
    fn combined_tilt_makes_a_convex_quad_with_the_lowest_corner_nearest_the_center() {
        let layout = board_area(Rect::new(0, 0, 40, 12)).unwrap();
        let projection = BoardProjection::new(&layout, &max_tilt(1, 1));
        let (w, h) = (BOARD_WIDTH as f64, BOARD_HEIGHT as f64);
        // 左上・右上・右下・左下の順(一周)
        let corners: Vec<(f64, f64)> = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
            .into_iter()
            .map(|corner| projection.project(corner))
            .collect();
        // 一周する間の曲がる向き(外積の符号)がすべて同じなら、四角形は凸で自己交差しない
        let turns: Vec<f64> = (0..4)
            .map(|i| {
                let (a, b, c) = (corners[i], corners[(i + 1) % 4], corners[(i + 2) % 4]);
                (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0)
            })
            .collect();
        assert!(
            turns.iter().all(|&t| t > 0.0) || turns.iter().all(|&t| t < 0.0),
            "凸な四角形: {turns:?}"
        );
        let distance = |screen: (f64, f64)| {
            let (dx, dy) = masses_from_center(&layout, screen);
            dx.hypot(dy)
        };
        let top_right = distance(corners[1]);
        for (i, &corner) in corners.iter().enumerate().filter(|&(i, _)| i != 1) {
            assert!(
                top_right < distance(corner),
                "右上(最も低い角)が中心に最も近い: {top_right} < {} (角{i})",
                distance(corner)
            );
        }
        // 1本の横の走査線上では、右へ行くほど盤のxが増える
        let (_, cy) = board_center(&layout);
        let mut previous = f64::NEG_INFINITY;
        let mut x = f64::from(layout.rect.x);
        while x <= f64::from(layout.rect.right()) {
            let (u, _) = projection.unproject((x, cy + 0.3)).unwrap();
            assert!(u > previous, "走査線上でxが単調に増える: {u} > {previous}");
            previous = u;
            x += 0.5;
        }
    }

    #[test]
    fn projection_round_trips_at_max_tilt() {
        let layout = board_area(Rect::new(1, 2, 90, 26)).unwrap();
        for tilt in tilts_to_check() {
            let projection = BoardProjection::new(&layout, &tilt);
            for pos in [
                (0.0, 0.0),
                (20.0, 12.0),
                (0.5, 0.5),
                (19.5, 0.5),
                (10.0, 6.0),
                (3.3, 8.7),
                (19.9, 11.9),
            ] {
                let back = projection.unproject(projection.project(pos));
                assert!(
                    back.is_some_and(|back| close(back, pos)),
                    "往復で元に戻る: {pos:?} -> {back:?} {tilt:?}"
                );
            }
        }
    }

    #[test]
    fn points_beyond_the_horizon_unproject_to_none() {
        let layout = board_area(Rect::new(0, 0, 40, 12)).unwrap();
        let projection = BoardProjection::new(&layout, &max_tilt(1, 0));
        let (cx, cy) = board_center(&layout);
        // 盤の中心から真上へmassesマス離れたセル座標
        let above = |masses: f64| (cx, cy - masses * f64::from(layout.cell_height));
        assert_eq!(projection.unproject(above(100.0)), None, "地平線の向こう側");
        assert!(projection.unproject(above(60.0)).is_some(), "地平線の手前");
    }

    #[test]
    fn the_camera_is_above_every_corner_at_max_tilt() {
        let reach = (BOARD_WIDTH / 2 + BOARD_HEIGHT / 2) as f64;
        assert!(
            reach * TILT_ANGLE_MAX.sin() < CAMERA_HEIGHT,
            "盤のどの角も地平線の手前にある"
        );
    }

    // --- 傾けた時のテキスト表示の盤面 ---

    /// 盤のマスの背景色か(市松2色・凸3色・凹3色・ゴールの9色)
    fn is_board_bg(color: Color) -> bool {
        board_bg_colors().contains(&color)
    }

    /// 盤のマスの背景色で塗られたセル
    fn painted_positions(buffer: &Buffer, area: Rect) -> Vec<Position> {
        area.positions()
            .filter(|&position| is_board_bg(buffer[position].bg))
            .collect()
    }

    #[test]
    fn pitching_paints_fewer_cells_and_never_outside_the_board_rect() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 60, 20);
        let layout = board_area(area).unwrap();
        assert!(layout.rect != area, "盤の周りに余白がある");
        let renderer = BoardRenderer::from_parts(None, None, None);
        let top = top_at(board.start_position());
        let flat = painted_positions(
            &draw_board_with_tilt(&renderer, &board, &top, area, &Tilt::default()),
            area,
        )
        .len();
        let pitched = painted_positions(
            &draw_board_with_tilt(&renderer, &board, &top, area, &max_tilt(1, 0)),
            area,
        )
        .len();
        assert!(
            pitched < flat,
            "傾けると塗られる範囲が縮む: {pitched} < {flat}"
        );
        assert!(
            pitched as f64 >= flat as f64 * 0.7,
            "縮みすぎない: {pitched} >= {flat} × 0.7"
        );
        for tilt in tilts_to_check() {
            let buffer = draw_board_with_tilt(&renderer, &board, &top, area, &tilt);
            for position in painted_positions(&buffer, area) {
                assert!(
                    layout.rect.contains(position),
                    "盤の描画範囲の外は塗らない: {position:?} {tilt:?}"
                );
            }
        }
    }

    #[test]
    fn tilted_text_board_is_a_trapezoid_on_a_large_area() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 200, 60);
        let layout = board_area(area).unwrap();
        assert_eq!((layout.cell_width, layout.cell_height), (10, 5));
        let renderer = BoardRenderer::from_parts(None, None, None);
        let top = top_at(board.start_position());

        let pitched = draw_board_with_tilt(&renderer, &board, &top, area, &max_tilt(1, 0));
        let painted = painted_positions(&pitched, area);
        let rows: Vec<usize> = (area.top()..area.bottom())
            .map(|y| painted.iter().filter(|p| p.y == y).count())
            .filter(|&count| count > 0)
            .collect();
        let (first_row, last_row) = (rows[0], rows[rows.len() - 1]);
        assert!(
            first_row < last_row,
            "pitch最大で最上段の行は最下段より狭い: {first_row} < {last_row}"
        );

        let rolled = draw_board_with_tilt(&renderer, &board, &top, area, &max_tilt(0, 1));
        let painted = painted_positions(&rolled, area);
        let columns: Vec<usize> = (area.left()..area.right())
            .map(|x| painted.iter().filter(|p| p.x == x).count())
            .filter(|&count| count > 0)
            .collect();
        let (first_column, last_column) = (columns[0], columns[columns.len() - 1]);
        assert!(
            last_column < first_column,
            "roll最大で最も右の列は最も左の列より低い: {last_column} < {first_column}"
        );
    }

    #[test]
    fn glyphs_stay_inside_their_own_cells_when_tilted() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 200, 60);
        let renderer = BoardRenderer::from_parts(None, None, None);
        let top = top_at(board.start_position());
        let count_of = |cell: Cell| {
            (0..BOARD_HEIGHT)
                .flat_map(|y| (0..BOARD_WIDTH).map(move |x| (x, y)))
                .filter(|&(x, y)| board.cell(x, y) == cell)
                .count()
        };
        for tilt in [
            max_tilt(1, 1),
            max_tilt(1, -1),
            max_tilt(-1, 1),
            max_tilt(-1, -1),
        ] {
            let buffer = draw_board_with_tilt(&renderer, &board, &top, area, &tilt);
            for (glyph, bgs, cell) in [
                (BUMP_GLYPH, BUMP_COLORS.to_vec(), Cell::Bump),
                (HOLLOW_GLYPH, HOLLOW_COLORS.to_vec(), Cell::Hollow),
                (GOAL_GLYPH, vec![GOAL_BG], Cell::Goal),
            ] {
                let positions: Vec<Position> = area
                    .positions()
                    .filter(|&position| buffer[position].symbol() == glyph)
                    .collect();
                assert_eq!(
                    positions.len(),
                    count_of(cell),
                    "{glyph}は全部描く {tilt:?}"
                );
                for position in positions {
                    assert!(
                        bgs.contains(&buffer[position].bg),
                        "{glyph}は自分のマスの中に置く: {position:?} {:?} {tilt:?}",
                        buffer[position].bg
                    );
                }
            }
        }
    }

    #[test]
    fn tilted_top_is_drawn_at_the_projected_center_of_its_cell() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 90, 26);
        let layout = board_area(area).unwrap();
        let renderer = BoardRenderer::from_parts(None, None, None);
        for tilt in tilts_to_check() {
            let projection = BoardProjection::new(&layout, &tilt);
            for pos in [(5.5, 3.5), (12.3, 8.9), (0.2, 11.7), (19.8, 0.1)] {
                let buffer = draw_board_with_tilt(&renderer, &board, &top_at(pos), area, &tilt);
                let (x, y) =
                    glyph_cell(projection.project((pos.0.floor() + 0.5, pos.1.floor() + 0.5)));
                assert_eq!(
                    buffer[(x as u16, y as u16)].symbol(),
                    TOP_SPIN_GLYPHS[0],
                    "ベーゴマは位置を含むマスの中心を順変換したセルに描く: {pos:?} {tilt:?}"
                );
            }
        }
    }

    #[test]
    fn tilted_top_far_off_the_board_stays_inside_the_panel() {
        let board = Board::standard();
        // 端末の左上に余白を取り、パネルの外にはみ出したら分かるようにする
        let area = Rect::new(5, 3, 60, 20);
        let renderer = BoardRenderer::from_parts(None, None, None);
        let mut tilts = tilts_to_check();
        tilts.push(Tilt::default());
        for tilt in tilts {
            for pos in [
                (-1000.0, -1000.0),
                (1000.0, 1000.0),
                (-1000.0, 1000.0),
                (1000.0, -1000.0),
                (-1.0, 5.0),
                (21.0, 13.0),
            ] {
                let buffer = draw_board_with_tilt(&renderer, &board, &top_at(pos), area, &tilt);
                let tops: Vec<(u16, u16)> = buffer
                    .content()
                    .iter()
                    .enumerate()
                    .filter(|(_, cell)| cell.symbol() == TOP_SPIN_GLYPHS[0])
                    .map(|(i, _)| buffer.pos_of(i))
                    .collect();
                assert_eq!(tops.len(), 1, "ベーゴマを1つ描く: {pos:?} {tilt:?}");
                let (x, y) = tops[0];
                assert!(
                    area.contains(Position::new(x, y)),
                    "パネルの中に収める: ({x}, {y}) {pos:?} {tilt:?}"
                );
            }
        }
    }

    #[test]
    fn image_board_ignores_the_tilt() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 40, 12);
        let top = top_at(board.start_position());
        // 画像のキャッシュの影響を受けないよう、描画器は別々に作る
        let flat_renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        let tilted_renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        let flat = draw_board_with_tilt(&flat_renderer, &board, &top, area, &Tilt::default());
        let tilted_board =
            draw_board_with_tilt(&tilted_renderer, &board, &top, area, &max_tilt(1, 1));
        assert_eq!(flat, tilted_board, "画像表示は傾きで描画内容が変わらない");
    }

    #[test]
    fn tilting_the_board_shifts_where_the_top_is_drawn_in_text_mode() {
        let board = Board::standard();
        let top = top_at((5.5, 3.5));
        let area = Rect::new(0, 0, 60, 20);
        let renderer = BoardRenderer::from_parts(None, None, None);
        let flat = draw_board_with_tilt(&renderer, &board, &top, area, &Tilt::default());
        let tilted_board = draw_board_with_tilt(&renderer, &board, &top, area, &tilted(0, 4));
        assert_ne!(
            flat, tilted_board,
            "傾けると盤面の描画内容(セルの位置)が変わる"
        );
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
        let mass = (top.pos.0.floor() as usize, top.pos.1.floor() as usize);
        assert_eq!(board.cell(mass.0, mass.1), Cell::Flat);
        assert_eq!(
            buffer[(top_rect.x + 1, top_rect.y)].bg,
            checker_of(mass),
            "平坦なマスはマスの偶奇に対応する市松のトーン"
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
        assert!(
            HOLLOW_COLORS.contains(&hollow_bg),
            "凹の記号は凹の色の上: {hollow_bg:?}"
        );
        assert!(
            BUMP_COLORS.contains(&bump_bg),
            "凸の記号は凸の色の上: {bump_bg:?}"
        );
        let hollow_right = buffer[(hollow.x + 1, hollow.y)].bg;
        assert!(
            HOLLOW_COLORS.contains(&hollow_right),
            "マス全体を凹の色で塗る: {hollow_right:?}"
        );
        for flat in CHECKER_COLORS {
            for hollow in HOLLOW_COLORS {
                assert!(luma(hollow) < luma(flat), "凹は平坦より暗く沈んだ色");
            }
            for bump in BUMP_COLORS {
                assert!(luma(bump) > luma(flat), "凸は平坦より明るく盛り上がった色");
            }
        }
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

    /// 点字の2×4の点の外周を時計回りに並べたビット(左上から右へ、右列を下へ、下段を左へ、左列を上へ)
    const BRAILLE_RING_CLOCKWISE: [u32; 8] = [0x01, 0x08, 0x10, 0x20, 0x80, 0x40, 0x04, 0x02];

    #[test]
    fn spin_glyphs_rotate_clockwise_one_step_at_a_time() {
        // 回転を滑らかに見せるため8コマにし、欠けた点が外周を1つずつ時計回りに進む(等間隔の回転)
        assert_eq!(TOP_SPIN_GLYPHS.len(), BRAILLE_RING_CLOCKWISE.len());
        for (frame, glyph) in TOP_SPIN_GLYPHS.iter().enumerate() {
            let mut chars = glyph.chars();
            let c = chars.next().unwrap();
            assert!(chars.next().is_none(), "1文字: {glyph}");
            let bits = u32::from(c)
                .checked_sub(0x2800)
                .filter(|b| *b <= 0xFF)
                .unwrap_or_else(|| panic!("点字の記号: {glyph}"));
            assert_eq!(
                0xFF ^ bits,
                BRAILLE_RING_CLOCKWISE[frame],
                "frame={frame}では外周の{frame}番目の点だけが欠ける: {glyph}"
            );
        }
    }

    #[test]
    fn animation_glyphs_are_one_cell_wide() {
        // 幅が変わると隣のマスがずれて見えるため、どのコマも1セル幅にする
        for glyph in TOP_SPIN_GLYPHS.iter().chain(STAR_ANIM_GLYPHS.iter()) {
            assert_eq!(Span::raw(*glyph).width(), 1, "1セル幅: {glyph}");
        }
    }

    #[test]
    fn star_glyphs_fade_out_over_more_frames() {
        // 星の演出を滑らかにするため8コマにする。大きな星から始まり、小さな点で終わる
        assert_eq!(STAR_ANIM_GLYPHS.len(), 8);
        assert_eq!(STAR_ANIM_GLYPHS[0], "★");
        assert_eq!(STAR_ANIM_GLYPHS[STAR_ANIM_GLYPHS.len() - 1], "･");
        for (i, a) in STAR_ANIM_GLYPHS.iter().enumerate() {
            for b in &STAR_ANIM_GLYPHS[i + 1..] {
                assert_ne!(a, b, "同じ記号を繰り返さない");
            }
        }
    }

    #[test]
    fn star_animation_takes_as_long_as_before_to_fade_out() {
        // コマ数を増やしても、最後のコマに達するまでの長さは従来(4コマ×180ms → 540ms)並みにする
        let to_last = STAR_ANIM_FRAME * (STAR_ANIM_GLYPHS.len() as u32 - 1);
        assert!(
            (Duration::from_millis(530)..=Duration::from_millis(550)).contains(&to_last),
            "最後のコマまで{to_last:?}"
        );
    }

    #[test]
    fn board_render_does_not_panic_in_tiny_areas() {
        let board = Board::standard();
        let renderer = BoardRenderer::new();
        let image_renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        // 傾き0・各軸最大・組み合わせ最大の3通り
        let tilts = [
            Tilt::default(),
            max_tilt(1, 0),
            max_tilt(-1, 0),
            max_tilt(0, 1),
            max_tilt(0, -1),
            max_tilt(1, 1),
            max_tilt(-1, -1),
        ];
        for tilt in &tilts {
            for (w, h) in [(1, 1), (3, 2), (10, 4), (39, 11)] {
                let area = Rect::new(0, 0, w, h);
                for pos in [(19.9, 11.9), (0.0, 0.0), (-1000.0, -1000.0)] {
                    draw_board_with_tilt(&renderer, &board, &top_at(pos), area, tilt);
                    draw_board_with_tilt(&image_renderer, &board, &top_at(pos), area, tilt);
                }
            }
        }
    }

    // --- 盤の格子(平坦なマスの市松)と凹凸の陰影 ---

    /// 設計書の参考値の色(基準色を白・黒へ寄せた結果)
    const CHECKER_LIGHT: Color = Color::Rgb(158, 117, 76);
    const CHECKER_DARK: Color = Color::Rgb(138, 97, 55);
    const BUMP_LIGHT: Color = Color::Rgb(213, 174, 128);
    const BUMP_DARK: Color = Color::Rgb(174, 136, 89);
    const HOLLOW_DARK: Color = Color::Rgb(51, 32, 15);
    const HOLLOW_LIGHT: Color = Color::Rgb(89, 71, 54);
    const CHECKER_COLORS: [Color; 2] = [CHECKER_LIGHT, CHECKER_DARK];
    /// 凸の3色(左上の帯・中央の帯・右下の帯の順)
    const BUMP_COLORS: [Color; 3] = [BUMP_LIGHT, BUMP_BG, BUMP_DARK];
    /// 凹の3色(左上の帯・中央の帯・右下の帯の順)
    const HOLLOW_COLORS: [Color; 3] = [HOLLOW_DARK, HOLLOW_BG, HOLLOW_LIGHT];

    /// 盤の背景色の9色
    fn board_bg_colors() -> Vec<Color> {
        CHECKER_COLORS
            .into_iter()
            .chain(BUMP_COLORS)
            .chain(HOLLOW_COLORS)
            .chain([GOAL_BG])
            .collect()
    }

    /// 平坦なマスmassの市松のトーン(mx + myが偶数なら明るい方)
    fn checker_of(mass: (usize, usize)) -> Color {
        if (mass.0 + mass.1).is_multiple_of(2) {
            CHECKER_LIGHT
        } else {
            CHECKER_DARK
        }
    }

    const WHITE_RGB: Color = Color::Rgb(255, 255, 255);
    const BLACK_RGB: Color = Color::Rgb(0, 0, 0);

    /// 各帯に当たるマス内位置(1マス=2×1セルの左のセル・マスの中心・右のセル)
    const TOP_LEFT_FRAC: (f64, f64) = (0.25, 0.5);
    const MIDDLE_FRAC: (f64, f64) = (0.5, 0.5);
    const BOTTOM_RIGHT_FRAC: (f64, f64) = (0.75, 0.5);
    const BAND_FRACS: [(f64, f64); 3] = [TOP_LEFT_FRAC, MIDDLE_FRAC, BOTTOM_RIGHT_FRAC];

    fn bg_of(cell: Cell, mass: (usize, usize), frac: (f64, f64)) -> Color {
        cell_style(cell, mass, frac)
            .bg
            .unwrap_or_else(|| panic!("背景色がある: {cell:?} {mass:?} {frac:?}"))
    }

    #[test]
    fn mix_moves_each_channel_toward_the_target_and_rounds() {
        let base = FLAT_BG;
        assert_eq!(mix(base, WHITE_RGB, 0.0), base);
        assert_eq!(mix(base, BLACK_RGB, 0.0), base);
        assert_eq!(mix(base, WHITE_RGB, 1.0), WHITE_RGB);
        assert_eq!(mix(base, BLACK_RGB, 1.0), BLACK_RGB);
        assert_eq!(
            mix(Color::Rgb(150, 105, 60), WHITE_RGB, 0.08),
            CHECKER_LIGHT
        );
        assert_eq!(mix(Color::Rgb(150, 105, 60), BLACK_RGB, 0.08), CHECKER_DARK);
    }

    #[test]
    fn mix_leaves_non_rgb_colors_unchanged() {
        for t in [0.0, 0.08, 0.5, 1.0] {
            assert_eq!(mix(Color::Yellow, WHITE_RGB, t), Color::Yellow);
            assert_eq!(mix(Color::Reset, BLACK_RGB, t), Color::Reset);
        }
    }

    #[test]
    fn shade_band_splits_a_mass_into_three_bands_lit_from_the_top_left() {
        for (frac, band) in [
            ((0.25, 0.5), ShadeBand::TopLeft),
            ((0.75, 0.5), ShadeBand::BottomRight),
            ((0.5, 0.5), ShadeBand::Middle),
            ((0.125, 0.25), ShadeBand::TopLeft),
            ((0.375, 0.75), ShadeBand::Middle),
            ((0.875, 0.75), ShadeBand::BottomRight),
        ] {
            assert_eq!(shade_band(frac), band, "{frac:?}");
        }
    }

    #[test]
    fn shade_band_boundaries_belong_to_the_outer_bands() {
        // d = fu + fv − 1 がちょうど −SHADE_BAND・+SHADE_BAND になる位置
        let top_left_edge = (1.0 - SHADE_BAND) / 2.0;
        let bottom_right_edge = (1.0 + SHADE_BAND) / 2.0;
        assert_eq!(
            shade_band((top_left_edge, top_left_edge)),
            ShadeBand::TopLeft
        );
        assert_eq!(
            shade_band((bottom_right_edge, bottom_right_edge)),
            ShadeBand::BottomRight
        );
        // 境界のすぐ内側は中央の帯
        assert_eq!(
            shade_band((top_left_edge + 0.01, top_left_edge)),
            ShadeBand::Middle
        );
        assert_eq!(
            shade_band((bottom_right_edge - 0.01, bottom_right_edge)),
            ShadeBand::Middle
        );
    }

    #[test]
    fn shade_band_is_symmetric_about_the_center_of_the_mass() {
        // 1/32刻みの格子点(浮動小数で正確に表せる)で、マスの中心について点対称な位置の帯を比べる
        for i in 0..16 {
            for j in 0..16 {
                let frac = (f64::from(2 * i + 1) / 32.0, f64::from(2 * j + 1) / 32.0);
                let mirrored = (1.0 - frac.0, 1.0 - frac.1);
                let expected = match shade_band(frac) {
                    ShadeBand::TopLeft => ShadeBand::BottomRight,
                    ShadeBand::Middle => ShadeBand::Middle,
                    ShadeBand::BottomRight => ShadeBand::TopLeft,
                };
                assert_eq!(shade_band(mirrored), expected, "{frac:?} ↔ {mirrored:?}");
            }
        }
    }

    #[test]
    fn flat_masses_alternate_between_two_checker_tones() {
        let fracs = [
            TOP_LEFT_FRAC,
            MIDDLE_FRAC,
            BOTTOM_RIGHT_FRAC,
            (0.0, 0.0),
            (0.99, 0.99),
        ];
        for mass in [(0, 0), (1, 1), (2, 4), (19, 11)] {
            for frac in fracs {
                assert_eq!(
                    bg_of(Cell::Flat, mass, frac),
                    CHECKER_LIGHT,
                    "{mass:?} {frac:?}"
                );
            }
        }
        for mass in [(1, 0), (0, 1), (3, 4), (19, 10)] {
            for frac in fracs {
                assert_eq!(
                    bg_of(Cell::Flat, mass, frac),
                    CHECKER_DARK,
                    "{mass:?} {frac:?}"
                );
            }
        }
        assert!(luma(CHECKER_DARK) < luma(FLAT_BG));
        assert!(luma(FLAT_BG) < luma(CHECKER_LIGHT));
    }

    #[test]
    fn bumps_are_lit_on_the_top_left_and_shaded_on_the_bottom_right() {
        let mass = (11, 1);
        let [light, middle, dark] = BAND_FRACS.map(|frac| bg_of(Cell::Bump, mass, frac));
        assert_eq!([light, middle, dark], BUMP_COLORS);
        assert!(luma(light) > luma(middle), "左上は中央より明るい");
        assert!(luma(middle) > luma(dark), "右下は中央より暗い");
        for bg in [light, middle, dark] {
            assert!(
                luma(bg) > luma(CHECKER_LIGHT),
                "凸は平坦の明るいトーンより明るい"
            );
        }
        for frac in BAND_FRACS {
            let style = cell_style(Cell::Bump, mass, frac);
            assert_eq!(style.fg, Some(BUMP_FG));
            assert!(style.add_modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn hollows_are_shaded_on_the_top_left_and_lit_on_the_bottom_right() {
        let mass = (16, 2);
        let [dark, middle, light] = BAND_FRACS.map(|frac| bg_of(Cell::Hollow, mass, frac));
        assert_eq!([dark, middle, light], HOLLOW_COLORS);
        assert!(luma(dark) < luma(middle), "左上は中央より暗い");
        assert!(luma(middle) < luma(light), "右下は中央より明るい");
        for bg in [dark, middle, light] {
            assert!(
                luma(bg) < luma(CHECKER_DARK),
                "凹は平坦の暗いトーンより暗い"
            );
        }
        for frac in BAND_FRACS {
            assert_eq!(cell_style(Cell::Hollow, mass, frac).fg, Some(HOLLOW_FG));
        }
    }

    #[test]
    fn goal_is_a_single_color_regardless_of_position() {
        for mass in [(17, 1), (0, 0), (3, 4)] {
            for frac in BAND_FRACS.into_iter().chain([(0.0, 0.0), (0.99, 0.99)]) {
                let style = cell_style(Cell::Goal, mass, frac);
                assert_eq!(style.bg, Some(GOAL_BG), "{mass:?} {frac:?}");
                assert_eq!(style.fg, Some(GOAL_FG));
                assert!(style.add_modifier.contains(Modifier::BOLD));
            }
        }
    }

    #[test]
    fn hollow_flat_and_bump_colors_are_ordered_and_nine_in_total() {
        let mut colors: Vec<Color> = Vec::new();
        for cell in [Cell::Flat, Cell::Bump, Cell::Hollow, Cell::Goal] {
            for mass in [(0, 0), (1, 0), (4, 7)] {
                for i in 0..16 {
                    for j in 0..16 {
                        let frac = (f64::from(2 * i + 1) / 32.0, f64::from(2 * j + 1) / 32.0);
                        let bg = bg_of(cell, mass, frac);
                        if !colors.contains(&bg) {
                            colors.push(bg);
                        }
                    }
                }
            }
        }
        assert_eq!(colors.len(), 9, "背景色は9色: {colors:?}");
        for color in board_bg_colors() {
            assert!(colors.contains(&color), "{color:?}を使う");
        }
        let max = |set: &[Color]| set.iter().map(|&c| luma(c)).fold(f64::MIN, f64::max);
        let min = |set: &[Color]| set.iter().map(|&c| luma(c)).fold(f64::MAX, f64::min);
        assert!(max(&HOLLOW_COLORS) < min(&CHECKER_COLORS), "凹 < 平坦");
        assert!(max(&CHECKER_COLORS) < min(&BUMP_COLORS), "平坦 < 凸");
    }

    /// テキスト表示の盤を描く(ベーゴマは投入位置)
    fn draw_text_board(area: Rect, tilt: &Tilt) -> (Board, BoardArea, Buffer) {
        let board = Board::standard();
        let renderer = BoardRenderer::from_parts(None, None, None);
        let top = top_at(board.start_position());
        let buffer = draw_board_with_tilt(&renderer, &board, &top, area, tilt);
        (board, board_area(area).unwrap(), buffer)
    }

    #[test]
    fn two_by_one_masses_show_the_checker_and_two_shading_steps() {
        let (board, layout, buffer) = draw_text_board(Rect::new(0, 0, 40, 12), &Tilt::default());
        assert_eq!((layout.cell_width, layout.cell_height), (2, 1));
        let bg_at = |mass: (usize, usize), dx: u16| {
            let rect = layout.cell_rect(mass.0, mass.1);
            buffer[(rect.x + dx, rect.y)].bg
        };
        // 平坦: マスの中は同じトーン、右隣・真下のマスとはトーンが違う
        for mass in [(0, 0), (1, 0), (0, 1)] {
            assert_eq!(board.cell(mass.0, mass.1), Cell::Flat);
        }
        assert_eq!(bg_at((0, 0), 0), bg_at((0, 0), 1));
        assert_eq!(bg_at((0, 0), 0), CHECKER_LIGHT);
        assert_ne!(
            bg_at((0, 0), 0),
            bg_at((1, 0), 0),
            "右隣のマスとトーンが違う"
        );
        assert_ne!(
            bg_at((0, 0), 0),
            bg_at((0, 1), 0),
            "真下のマスとトーンが違う"
        );
        // 凸: 左が明・右が暗
        let bump = first_cell(&board, Cell::Bump);
        assert_eq!((bg_at(bump, 0), bg_at(bump, 1)), (BUMP_LIGHT, BUMP_DARK));
        // 凹: 左が暗・右が明
        let hollow = first_cell(&board, Cell::Hollow);
        assert_eq!(
            (bg_at(hollow, 0), bg_at(hollow, 1)),
            (HOLLOW_DARK, HOLLOW_LIGHT)
        );
        // ゴール: 2セルとも一様
        let goal = board.goal();
        assert_eq!((bg_at(goal, 0), bg_at(goal, 1)), (GOAL_BG, GOAL_BG));
    }

    #[test]
    fn four_by_two_masses_show_three_shading_steps() {
        let (board, layout, buffer) = draw_text_board(Rect::new(0, 0, 90, 26), &Tilt::default());
        assert_eq!((layout.cell_width, layout.cell_height), (4, 2));
        for (cell, glyph, [top_left, middle, bottom_right]) in [
            (Cell::Bump, BUMP_GLYPH, BUMP_COLORS),
            (Cell::Hollow, HOLLOW_GLYPH, HOLLOW_COLORS),
        ] {
            let (mx, my) = first_cell(&board, cell);
            let rect = layout.cell_rect(mx, my);
            assert_eq!(
                buffer[(rect.x, rect.y)].bg,
                top_left,
                "{cell:?}の左上のセル"
            );
            assert_eq!(
                buffer[(rect.x + 3, rect.y + 1)].bg,
                bottom_right,
                "{cell:?}の右下のセル"
            );
            let glyph_position = (rect.x + 1, rect.y + 1);
            assert_eq!(buffer[glyph_position].symbol(), glyph);
            assert_eq!(
                buffer[glyph_position].bg, middle,
                "{cell:?}の記号のセルは中央の帯"
            );
        }
    }

    #[test]
    fn large_masses_have_as_many_lit_cells_as_shaded_cells() {
        let (board, layout, buffer) = draw_text_board(Rect::new(0, 0, 200, 60), &Tilt::default());
        assert_eq!((layout.cell_width, layout.cell_height), (10, 5));
        for (cell, light, dark) in [
            (Cell::Bump, BUMP_LIGHT, BUMP_DARK),
            (Cell::Hollow, HOLLOW_LIGHT, HOLLOW_DARK),
        ] {
            let (mx, my) = first_cell(&board, cell);
            let rect = layout.cell_rect(mx, my);
            let count = |color: Color| rect.positions().filter(|&p| buffer[p].bg == color).count();
            let (lit, shaded) = (count(light), count(dark));
            assert!(lit > 0, "{cell:?}の明のセルがある");
            assert_eq!(lit, shaded, "{cell:?}の明と暗のセル数は等しい");
        }
    }

    #[test]
    fn tilted_boards_use_only_the_board_colors() {
        for area in [Rect::new(0, 0, 90, 26), Rect::new(0, 0, 200, 60)] {
            for tilt in tilts_to_check() {
                let (_, layout, buffer) = draw_text_board(area, &tilt);
                for position in layout.rect.intersection(area).positions() {
                    let bg = buffer[position].bg;
                    assert!(
                        bg == Color::Reset || is_board_bg(bg),
                        "塗ったセルは9色のどれか: {position:?} {bg:?} {tilt:?}"
                    );
                }
                for (glyph, colors) in [
                    (BUMP_GLYPH, BUMP_COLORS.to_vec()),
                    (HOLLOW_GLYPH, HOLLOW_COLORS.to_vec()),
                    (GOAL_GLYPH, vec![GOAL_BG]),
                ] {
                    for position in area.positions().filter(|&p| buffer[p].symbol() == glyph) {
                        assert!(
                            colors.contains(&buffer[position].bg),
                            "{glyph}の背景は自分の種類の色: {position:?} {tilt:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_checker_follows_the_projection_when_pitched() {
        let area = Rect::new(0, 0, 90, 26);
        // 行yで、隣り合う平坦なセルの市松のトーンが切り替わるx
        let switches = |buffer: &Buffer, y: u16| -> Vec<u16> {
            (area.left() + 1..area.right())
                .filter(|&x| {
                    let (left, right) = (buffer[(x - 1, y)].bg, buffer[(x, y)].bg);
                    CHECKER_COLORS.contains(&left)
                        && CHECKER_COLORS.contains(&right)
                        && left != right
                })
                .collect()
        };
        // 市松のセルがある最上段・最下段の行の切り替わり位置
        let edge_rows = |buffer: &Buffer| {
            let rows: Vec<u16> = (area.top()..area.bottom())
                .filter(|&y| {
                    (area.left()..area.right()).any(|x| CHECKER_COLORS.contains(&buffer[(x, y)].bg))
                })
                .collect();
            (
                switches(buffer, rows[0]),
                switches(buffer, rows[rows.len() - 1]),
            )
        };
        let (_, _, flat) = draw_text_board(area, &Tilt::default());
        let (flat_top, flat_bottom) = edge_rows(&flat);
        assert!(!flat_top.is_empty());
        assert_eq!(
            flat_top, flat_bottom,
            "傾き0では最上段と最下段で切り替わる位置が同じ"
        );
        let (_, _, pitched) = draw_text_board(area, &max_tilt(1, 0));
        let (top, bottom) = edge_rows(&pitched);
        assert!(!top.is_empty() && !bottom.is_empty());
        assert_ne!(top, bottom, "pitch最大では投影に従って市松が変形する");
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
        let patch = renderer.patch.borrow()[0].as_ref().map(|p| p.rect).unwrap();
        // 前の位置のベーゴマを消すため、前回と今回のマスを両方含む(盤全体ではない)
        assert_eq!(
            patch,
            layout.cell_rect(1, 10).union(layout.cell_rect(2, 10))
        );
    }

    // --- 複数のベーゴマ(ROUND2) ---

    #[test]
    fn a_single_top_is_drawn_once() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 40, 12);
        let renderer = BoardRenderer::from_parts(None, None, None);
        let buffer = draw_board(&renderer, &board, &top_at(board.start_position()), area);
        assert_eq!(count_symbol(&buffer, TOP_SPIN_GLYPHS[0]), 1);
    }

    #[test]
    fn two_tops_in_the_same_cell_are_drawn_side_by_side() {
        let board = Board::standard();
        let renderer = BoardRenderer::from_parts(None, None, None);
        let (sx, sy) = board.start_position();
        // 左のベーゴマは0コマ目、右のベーゴマは1コマ目の記号にして見分ける
        let left = spinning_at((sx - 0.15, sy), 0);
        let right = spinning_at((sx + 0.15, sy), 1);
        for area in [Rect::new(0, 0, 40, 12), Rect::new(0, 0, 90, 26)] {
            let layout = board_area(area).unwrap();
            let mass = layout.top_rect((sx, sy));
            // スライスの並び順によらず、位置が左の方を左に描く
            for tops in [[left, right], [right, left]] {
                let buffer = draw_tops(&renderer, &board, &tops, area);
                let l = positions_of(&buffer, TOP_SPIN_GLYPHS[0]);
                let r = positions_of(&buffer, TOP_SPIN_GLYPHS[1]);
                assert_eq!((l.len(), r.len()), (1, 1), "2個とも描く: {area:?}");
                assert_eq!(l[0].1, r[0].1, "同じ行");
                assert!(l[0].0 < r[0].0, "左右にずらして重ねない: {l:?} {r:?}");
                for (x, y) in [l[0], r[0]] {
                    assert!(
                        mass.contains(Position::new(x, y)),
                        "どちらも自分のマスの中: ({x}, {y}) {mass:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn two_tops_in_different_cells_are_drawn_at_their_own_cells() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 40, 12);
        let layout = board_area(area).unwrap();
        let renderer = BoardRenderer::from_parts(None, None, None);
        let a = spinning_at((5.5, 3.5), 0);
        let b = spinning_at((12.3, 8.9), 1);
        let buffer = draw_tops(&renderer, &board, &[a, b], area);
        for top in [a, b] {
            let rect = layout.top_rect(top.pos);
            assert_eq!(
                buffer[(rect.x, rect.y)].symbol(),
                TOP_SPIN_GLYPHS[top.spin_frame],
                "位置を含むマスに描く: {:?}",
                top.pos
            );
        }
    }

    #[test]
    fn two_tops_far_off_the_board_stay_inside_the_panel_and_apart() {
        let board = Board::standard();
        let area = Rect::new(5, 3, 60, 20);
        let renderer = BoardRenderer::from_parts(None, None, None);
        for far in [(-1000.0, -1000.0), (1000.0, 1000.0), (1000.0, -1000.0)] {
            let tops = [spinning_at(far, 0), spinning_at((far.0 + 1.0, far.1), 1)];
            let buffer = draw_tops(&renderer, &board, &tops, area);
            let a = positions_of(&buffer, TOP_SPIN_GLYPHS[0]);
            let b = positions_of(&buffer, TOP_SPIN_GLYPHS[1]);
            assert_eq!((a.len(), b.len()), (1, 1), "{far:?}");
            assert_ne!(a[0], b[0], "パネルの端に寄せても重ねない: {far:?}");
            for (x, y) in [a[0], b[0]] {
                assert!(area.contains(Position::new(x, y)), "{far:?}");
            }
        }
    }

    #[test]
    fn a_star_on_any_top_switches_the_image_board_to_text() {
        let board = Board::standard();
        let area = Rect::new(0, 0, 40, 12);
        let renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        let star = TopView {
            star_frame: Some(0),
            ..top_at((5.5, 5.5))
        };
        let buffer = draw_tops(&renderer, &board, &[top_at((1.5, 10.5)), star], area);
        assert_eq!(count_symbol(&buffer, STAR_ANIM_GLYPHS[0]), 1);
        assert_eq!(count_symbol(&buffer, TOP_SPIN_GLYPHS[0]), 1);
    }

    #[test]
    fn each_top_has_its_own_patch_on_the_image_board() {
        let board = Board::standard();
        let renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        let area = Rect::new(0, 0, 40, 12);
        let (a, b) = (top_at((1.5, 10.5)), top_at((15.5, 3.5)));
        draw_tops(&renderer, &board, &[a, b], area);
        assert_eq!(
            (
                renderer.slot_patch_encode_count(0),
                renderer.slot_patch_encode_count(1)
            ),
            (1, 1)
        );
        // 片方だけがマスを移ったら、そのスロットのパッチだけを作り直す
        let a2 = top_at((2.5, 10.5));
        draw_tops(&renderer, &board, &[a2, b], area);
        draw_tops(&renderer, &board, &[a2, b], area);
        assert_eq!(
            (
                renderer.slot_patch_encode_count(0),
                renderer.slot_patch_encode_count(1)
            ),
            (2, 1)
        );
        let b2 = top_at((16.5, 3.5));
        draw_tops(&renderer, &board, &[a2, b2], area);
        assert_eq!(
            (
                renderer.slot_patch_encode_count(0),
                renderer.slot_patch_encode_count(1)
            ),
            (2, 2)
        );
        assert_eq!(renderer.patch_encode_count(), 4, "合計");
        assert_eq!(renderer.base_encode_count(), 1, "盤は作り直さない");
    }

    #[test]
    fn a_patch_is_rebuilt_when_another_top_enters_or_leaves_it() {
        // 他のベーゴマが自分のパッチの範囲に出入りした時だけは、自分のパッチにも描き直す
        // (そうしないと、後から描いたパッチがもう片方のベーゴマを消してしまう)
        let board = Board::standard();
        let renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        let area = Rect::new(0, 0, 40, 12);
        let a = top_at((5.5, 5.5));
        let counts = || {
            (
                renderer.slot_patch_encode_count(0),
                renderer.slot_patch_encode_count(1),
            )
        };
        draw_tops(&renderer, &board, &[a, top_at((8.5, 5.5))], area);
        draw_tops(&renderer, &board, &[a, top_at((6.5, 5.5))], area);
        assert_eq!(counts(), (1, 2), "範囲の外で動いても作り直さない");
        draw_tops(&renderer, &board, &[a, top_at((5.5, 5.5))], area);
        assert_eq!(counts(), (2, 3), "同じマスに入ってきたら両方描き直す");
        draw_tops(&renderer, &board, &[a, top_at((6.5, 5.5))], area);
        assert_eq!(counts(), (3, 4), "出ていったら描き直す");
    }

    #[test]
    fn the_image_board_handles_the_top_count_changing_from_one_to_two() {
        let board = Board::standard();
        let renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        let area = Rect::new(0, 0, 40, 12);
        let (sx, sy) = board.start_position();
        draw_board(&renderer, &board, &top_at((5.5, 5.5)), area);
        assert_eq!(renderer.patch.borrow().len(), 1);
        let pair = [top_at((sx - 0.15, sy)), top_at((sx + 0.15, sy))];
        draw_tops(&renderer, &board, &pair, area);
        let patches = renderer.patch.borrow();
        assert_eq!(patches.len(), 2, "スロットごとにパッチを持つ");
        assert!(patches.iter().all(Option::is_some), "両方とも描く");
    }

    #[test]
    fn the_image_patch_draws_two_tops_in_the_same_cell_apart() {
        let board = Board::standard();
        let picker = test_picker();
        let area = Rect::new(0, 0, 40, 12);
        let layout = board_area(area).unwrap();
        let composed = compose_base(&plain_board_image(), &board, layout, picker.font_size());
        let base = BaseCache {
            area: layout,
            protocol: picker.new_resize_protocol(DynamicImage::ImageRgba8(composed.clone())),
            composed,
        };
        // 平坦なマス(5,5)に2個。1マスは2×1セル=20×20ピクセル
        let rect = layout.cell_rect(5, 5);
        let marks: Vec<TopMark> = (0..2)
            .map(|index| TopMark {
                rect,
                airborne: false,
                lane: Lane { index, count: 2 },
            })
            .collect();
        let patch = patch_image(&base, picker.font_size(), rect, &marks, None);
        assert_eq!(patch.dimensions(), (20, 20));
        assert_eq!(*patch.get_pixel(5, 10), TOP_FACE_PIXEL, "左の区画に描く");
        assert_eq!(*patch.get_pixel(15, 10), TOP_FACE_PIXEL, "右の区画に描く");
        let between = *patch.get_pixel(10, 10);
        assert!(
            between != TOP_FACE_PIXEL && between != TOP_EDGE_PIXEL,
            "2個の間は盤のまま(重ならない): {between:?}"
        );
        // 1個だけならマスの中央に描く(従来どおり)
        let sole = [TopMark {
            rect,
            airborne: false,
            lane: Lane { index: 0, count: 1 },
        }];
        let patch = patch_image(&base, picker.font_size(), rect, &sole, None);
        assert_eq!(*patch.get_pixel(10, 10), TOP_FACE_PIXEL);
    }

    #[test]
    fn two_tops_do_not_panic_in_tiny_areas() {
        let board = Board::standard();
        let renderer = BoardRenderer::new();
        let image_renderer = BoardRenderer::with_images(test_picker(), plain_board_image(), None);
        for tilt in [Tilt::default(), max_tilt(1, 1), max_tilt(-1, -1)] {
            for (w, h) in [(1, 1), (3, 2), (10, 4), (39, 11)] {
                let area = Rect::new(0, 0, w, h);
                for pair in [
                    [top_at((1.35, 10.5)), top_at((1.65, 10.5))],
                    [top_at((19.9, 11.9)), top_at((-1000.0, -1000.0))],
                ] {
                    draw_tops_with_tilt(&renderer, &board, &pair, area, &tilt);
                    draw_tops_with_tilt(&image_renderer, &board, &pair, area, &tilt);
                }
            }
        }
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

    // --- 速度感・接近の可視化(#130) ---

    #[test]
    fn shake_offset_is_zero_with_no_g() {
        assert_eq!(
            shake_offset(GForce::default(), Duration::from_millis(123)),
            (0.0, 0.0)
        );
    }

    #[test]
    fn shake_offset_grows_with_the_size_of_g() {
        let elapsed = Duration::from_millis(30);
        let small = GForce {
            longitudinal: 0.05,
            lateral: 0.0,
        };
        let big = GForce {
            longitudinal: 0.5,
            lateral: 0.0,
        };
        let (small_x, small_y) = shake_offset(small, elapsed);
        let (big_x, big_y) = shake_offset(big, elapsed);
        assert!(big_x.abs() > small_x.abs());
        assert!(big_y.abs() > small_y.abs());
    }

    #[test]
    fn shake_offset_changes_over_time() {
        let g = GForce {
            longitudinal: 0.3,
            lateral: 0.0,
        };
        let first = shake_offset(g, Duration::from_millis(0));
        let later = shake_offset(g, Duration::from_millis(40));
        assert_ne!(
            first, later,
            "経過時間が変われば揺れも変わる(固定値ではない)"
        );
    }

    #[test]
    fn offset_within_clamps_to_the_bounds() {
        let bounds = Rect::new(0, 0, 20, 10);
        let rect = Rect::new(5, 5, 4, 3);
        // 範囲を大きく超えるオフセットでもboundsの外に出ない
        let moved = offset_within(rect, -1000, -1000, bounds);
        assert!(bounds.contains(Position::new(moved.x, moved.y)));
        let moved = offset_within(rect, 1000, 1000, bounds);
        assert!(bounds.contains(Position::new(
            moved.x + moved.width - 1,
            moved.y + moved.height - 1
        )));
    }

    #[test]
    fn upcoming_is_urgent_only_within_the_urgent_distance() {
        let at = |distance: f64| {
            upcoming_is_urgent(Some(Upcoming {
                kind: UpcomingKind::Bump,
                distance,
            }))
        };
        assert!(!at(URGENT_DISTANCE + 0.1));
        assert!(at(URGENT_DISTANCE));
        assert!(at(0.0));
        assert!(!upcoming_is_urgent(None));
    }

    #[test]
    fn truck_view_reverses_the_header_style_when_urgent() {
        let renderer = TruckViewRenderer::new();
        let area = Rect::new(0, 0, 30, 14);
        let near = Upcoming {
            kind: UpcomingKind::Bump,
            distance: URGENT_DISTANCE,
        };
        let far = Upcoming {
            kind: UpcomingKind::Bump,
            distance: URGENT_DISTANCE + 1.0,
        };
        let mut terminal = Terminal::new(TestBackend::new(area.right(), area.bottom())).unwrap();
        terminal
            .draw(|frame| renderer.render(frame, area, &info(GForce::default(), Some(near))))
            .unwrap();
        let near_reversed = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.modifier.contains(Modifier::REVERSED));
        terminal
            .draw(|frame| renderer.render(frame, area, &info(GForce::default(), Some(far))))
            .unwrap();
        let far_reversed = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.modifier.contains(Modifier::REVERSED));
        assert!(near_reversed, "近い前方イベントは反転表示で強調する");
        assert!(!far_reversed, "遠い前方イベントは反転表示にしない");
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
