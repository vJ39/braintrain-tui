//! カウントメニアの数字付き円の描画。
//!
//! 画像(assets/image/count_mania/1.png〜20.png、白い円に黒い数字)の白い部分だけを
//! 指定色に塗り替えて表示する。sixel/kitty/iTerm2の画像プロトコルに対応した端末では画像を、
//! 非対応の端末では丸囲み数字(①②…)を色付きテキストで表示する。
//! 円どうしは重なり合い、渡された並び順に描く(後の円ほど手前)。
//! 画像表示では、盤面を泳ぐ魚も円の後ろに描く(テキスト表示の魚は呼び出し側が文字で描く)。
//! 図形描画用のShapeCanvasとは独立した、このゲーム専用の部品。

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;
use rust_embed::RustEmbed;

use super::fish::{sprite, FISH_HEIGHT, FISH_WIDTH, SPRITE_HEIGHT, SPRITE_WIDTH};
use super::layout::{circle_contains, label_area};
use super::ripple::{ring_image, Ripple, RIPPLE_COLOR, RIPPLE_THICKNESS_CELLS};
use super::wrong_mark::{
    cross_image, WrongMark, WRONG_MARK_COLOR, WRONG_MARK_HALF_SIZE_CELLS,
    WRONG_MARK_THICKNESS_CELLS,
};

#[derive(RustEmbed)]
#[folder = "assets/image/count_mania/"]
struct CircleAssets;

/// RGB各チャンネルがこの値以上なら白(円の塗り)とみなす
const WHITE_THRESHOLD: u8 = 200;
/// RGB各チャンネルがこの値以下なら黒(数字)とみなす
const BLACK_THRESHOLD: u8 = 50;

/// 番号(1〜20)の円画像を読み込む。該当する画像が無ければNone
pub fn load_circle_image(number: u8) -> Option<RgbaImage> {
    let file = CircleAssets::get(&format!("{number}.png"))?;
    let image = image::load_from_memory(&file.data).ok()?;
    Some(image.to_rgba8())
}

/// 白いピクセル(円の塗り)だけをcolorに置き換える。黒いピクセル(数字)と透明部分はそのまま。
/// 白と黒の中間(文字の輪郭のなめらかな部分)は、黒からcolorへの間を明るさに応じて補間する
pub fn recolor(image: &RgbaImage, color: [u8; 3]) -> RgbaImage {
    let mut out = image.clone();
    for pixel in out.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        // 完全に透明なピクセルは見えないので触らない
        if a == 0 {
            continue;
        }
        let rgb = [r, g, b];
        let min = rgb.iter().copied().min().unwrap_or(0);
        let max = rgb.iter().copied().max().unwrap_or(0);
        if min >= WHITE_THRESHOLD {
            pixel.0 = [color[0], color[1], color[2], a];
        } else if max <= BLACK_THRESHOLD {
            // 数字の黒はそのまま
        } else {
            // 中間の明るさ: 黒(0)〜color(1)の間を、しきい値の間での明るさの位置で補間する
            let brightness = rgb.iter().map(|&c| f64::from(c)).sum::<f64>() / 3.0;
            let t = ((brightness - f64::from(BLACK_THRESHOLD))
                / f64::from(WHITE_THRESHOLD - BLACK_THRESHOLD))
            .clamp(0.0, 1.0);
            let mix = |c: u8| (f64::from(c) * t).round() as u8;
            pixel.0 = [mix(color[0]), mix(color[1]), mix(color[2]), a];
        }
    }
    out
}

/// 番号(1〜20)の丸囲み数字(①〜⑳)。範囲外はNone
pub fn circled_digit(number: u8) -> Option<char> {
    if !(1..=20).contains(&number) {
        return None;
    }
    // ①(U+2460)〜⑳(U+2473)は連続している
    char::from_u32(0x2460 + u32::from(number) - 1)
}

/// ボードに描く円1つ(位置・番号・色)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardCircle {
    pub rect: Rect,
    pub number: u8,
    pub color: [u8; 3],
}

/// 盤面を泳ぐ魚1匹(画像表示用)。x, yは魚の左上のセル(テキスト表示と同じく丸めたセル位置)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BoardFish {
    pub x: u16,
    pub y: u16,
    pub facing_right: bool,
    pub color: [u8; 3],
}

impl BoardFish {
    /// 魚が占めるセル範囲(テキスト表示の"><>"と同じ大きさ)
    fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, FISH_WIDTH, FISH_HEIGHT)
    }
}

/// 直前に作った盤面の画像。描画エリア・円の並びが同じなら再エンコードを省く
struct BoardCache {
    board: Rect,
    circles: Vec<BoardCircle>,
    /// 円だけを重ねた合成画像(魚・波紋は含まない)。パッチはここから切り出す
    composed: RgbaImage,
    /// 端末へ送る画像(作った時の位置の魚を円の後ろに描いたもの)
    protocol: StatefulProtocol,
}

/// パッチに描いた演出の組み合わせ。(波紋の(中心, コマ), バツ印の中心)。
/// どちらもNoneは演出が消えた後の、何も重ねていない切り出し
type EffectsKey = (Option<((u16, u16), u32)>, Option<(u16, u16)>);

/// パッチに描いたもの。(演出(パッチが波紋・バツ印の範囲を含む時だけSome), パッチにかかる魚)
type PatchKey = (Option<EffectsKey>, Vec<BoardFish>);

/// パッチの画像の指紋(幅, 高さ, ピクセルのハッシュ)。同じなら端末へ送る画像データも同じになる
type Fingerprint = (u32, u32, u64);

/// パッチにする範囲1つ。effectsは波紋・バツ印の範囲か(falseは動いた魚の範囲)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PatchItem {
    rect: Rect,
    effects: bool,
}

/// 直前のフレームに描いたパッチ(盤面の一部を切り出して魚・波紋のリング・バツ印を重ねた画像)
struct PatchCache {
    /// パッチを置くセル範囲
    rect: Rect,
    /// 描いたもの
    key: PatchKey,
    fingerprint: Fingerprint,
    protocol: StatefulProtocol,
}

/// このフレームに描くパッチ1枚の予定。(範囲, 描くもの, 作り直す画像と指紋(Noneは直前のフレームのものを使い回す))
type PlannedPatch = (PatchItem, PatchKey, Option<(RgbaImage, Fingerprint)>);

/// 同じ起点に同じ画像のパッチを作りそうになった時に、範囲を広げて作り直す回数の上限
const MAX_PATCH_GROWS: usize = 4;

/// 数字付き円の描画器。画像プロトコルが使える端末では、ボード上の全ての円を1枚の画像に
/// 重ね合わせてから表示する(円ごとに別の画像にすると、重なった所で手前の画像が矩形ごと
/// 奥の画像を消したり、押して消えた円の画像が画面に残ったりするため)。
/// 盤面の画像は描画エリアか円の並び(押して消えた円・色・位置)が変わった時だけ作り直す。
///
/// 正解クリックの波紋は、盤面の画像から波紋の届く範囲だけを切り出してリングを重ねた
/// 小さな画像(パッチ)にし、盤面の画像の上に重ねて描く。波紋は毎コマ見た目が変わるが、
/// 盤面全体を作り直すと画像のエンコード・端末への送信が1回あたり数十〜数百msかかり
/// (sixelでは盤面全体で約230ms)、tick(33ms)に間に合わずちらつくため。
/// パッチはコマ(RIPPLE_FRAME_INTERVAL)が変わった時だけ作り直す。
/// 波紋が消えた後は、端末に残った最後のリングを消すため、リングの無い切り出しを1回描いて置いておく。
/// 誤クリックのバツ印も同じパッチに重ねて描く(パッチ同士が重なると、後に描いた方の起点セルが
/// 先の方の画像データを上書きして端末へ送られなくなるため、演出は1枚のパッチにまとめる)。
///
/// 泳ぐ魚は、盤面の画像を作る時にその時の位置で円の後ろに描いておき、その後セルを移った
/// (または向きを変えた)魚だけを、前の位置と今の位置を合わせた範囲のパッチで描き直す
/// (前の位置も描き直すので、端末に古い魚が残らない)。パッチ同士が重なる・左右に隣り合う時は
/// 1枚にまとめる(merge_patch_itemsを参照)
pub struct CircleRenderer {
    picker: Option<Picker>,
    board: RefCell<Option<BoardCache>>,
    /// 波紋・バツ印のパッチの範囲(前の演出の範囲も含む)。演出が消えて何も重ねていない
    /// 切り出しを1回描いた時・盤面を作り直した時に捨てる
    effects_rect: Cell<Option<Rect>>,
    /// 直前のフレームに描いたパッチ
    patches: RefCell<Vec<PatchCache>>,
    /// 端末にいま見えている魚(盤面の画像かパッチに最後に描いた位置・向き)
    shown_fish: RefCell<Vec<BoardFish>>,
    /// パッチの起点セルごとに、最後に置いた画像の指紋。端末側のImageDedupBackendは同じ位置へ
    /// 前回と同じ画像データを送り直さないため、間に別のパッチ・盤面で上書きされた所へ同じ画像を
    /// 置いても端末に出ない。これと同じ画像になるパッチは、範囲を広げて別の画像にする
    sent: RefCell<HashMap<(u16, u16), Fingerprint>>,
    /// テスト用: 盤面の画像を作り直した回数
    #[cfg(test)]
    board_encodes: Cell<usize>,
    /// テスト用: パッチ(波紋・バツ印・魚)を作り直した回数
    #[cfg(test)]
    ripple_encodes: Cell<usize>,
}

impl CircleRenderer {
    pub fn new() -> Self {
        Self::from_picker(detect_picker())
    }

    fn from_picker(picker: Option<Picker>) -> Self {
        Self {
            picker,
            board: RefCell::new(None),
            effects_rect: Cell::new(None),
            patches: RefCell::new(Vec::new()),
            shown_fish: RefCell::new(Vec::new()),
            sent: RefCell::new(HashMap::new()),
            #[cfg(test)]
            board_encodes: Cell::new(0),
            #[cfg(test)]
            ripple_encodes: Cell::new(0),
        }
    }

    /// テスト用: 盤面の画像を作り直した回数
    #[cfg(test)]
    pub fn board_encode_count(&self) -> usize {
        self.board_encodes.get()
    }

    /// テスト用: パッチ(波紋・バツ印・魚)を作り直した回数
    #[cfg(test)]
    pub fn ripple_encode_count(&self) -> usize {
        self.ripple_encodes.get()
    }

    /// テスト用: 直前に描いた、演出(波紋・バツ印)を含むパッチの範囲
    #[cfg(test)]
    pub fn patch_rect(&self) -> Option<Rect> {
        self.patches
            .borrow()
            .iter()
            .find(|patch| patch.key.0.is_some())
            .map(|patch| patch.rect)
    }

    /// テスト用: 直前のフレームに描いたパッチ全部の範囲
    #[cfg(test)]
    pub fn patch_rects(&self) -> Vec<Rect> {
        self.patches
            .borrow()
            .iter()
            .map(|patch| patch.rect)
            .collect()
    }

    /// 画像プロトコルを使うか(false=丸囲み数字のテキスト表示)
    pub fn uses_image(&self) -> bool {
        self.picker.is_some()
    }

    /// 画像プロトコルを指定して作る。テストで画像表示の経路を通すため
    #[cfg(test)]
    pub fn with_picker(picker: Picker) -> Self {
        Self::from_picker(Some(picker))
    }

    /// 魚のいない盤面を描く(render_board_with_fishを参照)
    #[cfg(test)]
    pub fn render_board(
        &self,
        frame: &mut Frame,
        board: Rect,
        circles: &[BoardCircle],
        ripple: Option<&Ripple>,
        mark: Option<&WrongMark>,
    ) {
        self.render_board_with_fish(frame, board, circles, ripple, mark, &[]);
    }

    /// board内にcirclesを並び順に描く。後の円ほど手前に重なる。
    /// rippleがあれば円の手前に波紋を重ねる(画像プロトコルが使える時だけ。テキスト表示では描かない)。
    /// markがあれば円の手前にバツ印を重ねる(テキスト表示では、クリックしたセルに赤い✗を置く)。
    /// fishは円の後ろに描く(画像プロトコルが使える時だけ。テキスト表示の魚は呼び出し側が描く)。
    /// fishは毎フレーム同じ並びで渡す(何番目の魚が動いたかで描き直す範囲を決めるため)
    pub fn render_board_with_fish(
        &self,
        frame: &mut Frame,
        board: Rect,
        circles: &[BoardCircle],
        ripple: Option<&Ripple>,
        mark: Option<&WrongMark>,
        fish: &[BoardFish],
    ) {
        // 描画先がフレームからはみ出さないよう切り詰める
        let board = board.intersection(frame.area());
        if board.is_empty() {
            return;
        }
        if self.render_board_image(frame, board, circles, ripple, mark, fish) {
            return;
        }
        for circle in circles {
            let rect = circle.rect.intersection(frame.area());
            if !rect.is_empty() {
                render_text(frame, rect, circle.number, circle.color);
            }
        }
        if let Some(mark) = mark {
            render_mark_text(frame, board, mark);
        }
    }

    /// 画像プロトコルで描く。画像プロトコルが使えない/画像が読めない場合はfalse
    fn render_board_image(
        &self,
        frame: &mut Frame,
        board: Rect,
        circles: &[BoardCircle],
        ripple: Option<&Ripple>,
        mark: Option<&WrongMark>,
        fish: &[BoardFish],
    ) -> bool {
        let Some(picker) = &self.picker else {
            return false;
        };
        if circles.is_empty() && ripple.is_none() && mark.is_none() && fish.is_empty() {
            return true;
        }
        let mut cache = self.board.borrow_mut();
        let same_board = matches!(
            cache.as_ref(),
            Some(cached) if cached.board == board && cached.circles == circles
        );
        if !same_board {
            let Some(images) = recolored_images(circles) else {
                return false;
            };
            let composed = compose_circles(board, picker.font_size(), circles, &images);
            // 盤面の画像を送り直すと、端末ではそれまでのパッチ(に描いた魚)も上書きされるので、
            // いまの位置の魚も盤面の画像に描いておく
            let scene = compose_scene(&composed, board, picker.font_size(), fish);
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(scene));
            *cache = Some(BoardCache {
                board,
                circles: circles.to_vec(),
                composed,
                protocol,
            });
            // 盤面の画像がパッチの範囲も描き直すので、パッチは新しい盤面から作り直す
            self.effects_rect.set(None);
            self.patches.borrow_mut().clear();
            *self.shown_fish.borrow_mut() = fish.to_vec();
            #[cfg(test)]
            self.board_encodes.set(self.board_encodes.get() + 1);
        }
        let Some(cached) = cache.as_mut() else {
            return false;
        };
        frame.render_stateful_widget(StatefulImage::default(), board, &mut cached.protocol);
        // パッチは盤面の画像より後に描き、盤面の上に重ねる
        self.render_patches(frame, picker, cached, ripple, mark, fish);
        true
    }

    /// 波紋・バツ印のパッチの範囲と、描く演出。演出が消えたフレームには、何も重ねていない切り出しを
    /// 同じ範囲に1回描き(描かずにやめると端末に最後のリング・バツ印が残るため)、範囲を手放す
    /// (持ち続けると、その範囲を泳ぐ魚が動くたびに範囲全体のパッチを作り直すことになるため)。
    /// 範囲が無ければNone
    fn effects_item(
        &self,
        board: Rect,
        font_size: (u16, u16),
        ripple: Option<&Ripple>,
        mark: Option<&WrongMark>,
    ) -> Option<(Rect, EffectsKey)> {
        let previous = self.effects_rect.get();
        let (rect, key) = if ripple.is_some() || mark.is_some() {
            let own = effects_rect(board, font_size, ripple, mark);
            // 盤面が同じまま演出の位置が変わった時は、前のパッチの範囲も含めて描き直し、
            // 前のリング・バツ印を端末に残さない
            let rect = match previous {
                Some(previous) if !previous.is_empty() && !own.is_empty() => previous.union(own),
                Some(previous) if own.is_empty() => previous,
                _ => own,
            };
            let key = (
                ripple.map(|ripple| (ripple.center(), ripple.frame())),
                mark.map(WrongMark::center),
            );
            (rect, key)
        } else {
            let previous = previous?;
            self.effects_rect.set(None);
            return (!previous.is_empty()).then_some((previous, (None, None)));
        };
        if rect.is_empty() {
            return None;
        }
        self.effects_rect.set(Some(rect));
        Some((rect, key))
    }

    /// 演出(波紋・バツ印)と、セルを移った魚の周りのパッチを盤面の上に描く。
    /// 直前のフレームと同じ範囲・同じ中身のパッチは作り直さない
    fn render_patches(
        &self,
        frame: &mut Frame,
        picker: &Picker,
        board: &BoardCache,
        ripple: Option<&Ripple>,
        mark: Option<&WrongMark>,
        fish: &[BoardFish],
    ) {
        let font_size = picker.font_size();
        let area = patch_area(board.board);
        let effects = self.effects_item(board.board, font_size, ripple, mark);
        let mut items: Vec<PatchItem> = effects
            .iter()
            .map(|&(rect, _)| PatchItem {
                rect,
                effects: true,
            })
            .collect();
        items.extend(
            moved_fish_rects(&self.shown_fish.borrow(), fish, area)
                .into_iter()
                .map(|rect| PatchItem {
                    rect,
                    effects: false,
                }),
        );
        let effects_key = effects.map(|(_, key)| key);
        let mut previous = std::mem::take(&mut *self.patches.borrow_mut());
        let mut sent = self.sent.borrow_mut();

        let mut planned: Vec<PlannedPatch>;
        let mut grows = 0;
        loop {
            planned = Vec::new();
            let mut grown = Vec::new();
            for group in merge_patch_items(&items) {
                let key: PatchKey = (
                    if group.effects { effects_key } else { None },
                    fish_in(fish, group.rect),
                );
                if previous
                    .iter()
                    .any(|patch| patch.rect == group.rect && patch.key == key)
                {
                    planned.push((group, key, None));
                    continue;
                }
                let (group_ripple, group_mark) = if group.effects {
                    (ripple, mark)
                } else {
                    (None, None)
                };
                let image = effects_patch(
                    &board.composed,
                    board.board,
                    font_size,
                    group.rect,
                    group_ripple,
                    group_mark,
                    &key.1,
                );
                let print = fingerprint(&image);
                if grows < MAX_PATCH_GROWS
                    && sent.get(&(group.rect.x, group.rect.y)) == Some(&print)
                {
                    if let Some(rect) = grow_within(group.rect, area) {
                        grown.push(PatchItem {
                            rect,
                            effects: false,
                        });
                    }
                }
                planned.push((group, key, Some((image, print))));
            }
            if grown.is_empty() {
                break;
            }
            // 広げた範囲が他のパッチにかかることがあるので、まとめ直す
            items.extend(grown);
            grows += 1;
        }

        let mut current = Vec::with_capacity(planned.len());
        for (group, key, image) in planned {
            let patch = match image {
                Some((image, fingerprint)) => {
                    #[cfg(test)]
                    self.ripple_encodes.set(self.ripple_encodes.get() + 1);
                    PatchCache {
                        rect: group.rect,
                        key,
                        fingerprint,
                        protocol: picker.new_resize_protocol(DynamicImage::ImageRgba8(image)),
                    }
                }
                None => {
                    let Some(index) = previous
                        .iter()
                        .position(|patch| patch.rect == group.rect && patch.key == key)
                    else {
                        continue;
                    };
                    previous.swap_remove(index)
                }
            };
            current.push(patch);
        }
        for patch in &mut current {
            // 盤面の画像は起点以外の全セルをskip(ratatuiの差分出力の対象外)にしており、
            // ratatui-imageはパッチの起点セルに画像データを入れる時にskipを戻さない。
            // そのままではパッチが端末へ一切出力されないので、パッチの範囲のskipを先に外す
            // (起点以外はパッチの描画で再びskipになる)
            let buffer = frame.buffer_mut();
            for y in patch.rect.top()..patch.rect.bottom() {
                for x in patch.rect.left()..patch.rect.right() {
                    buffer[(x, y)].set_skip(false);
                }
            }
            frame.render_stateful_widget(StatefulImage::default(), patch.rect, &mut patch.protocol);
            sent.insert((patch.rect.x, patch.rect.y), patch.fingerprint);
        }
        *self.patches.borrow_mut() = current;
        *self.shown_fish.borrow_mut() = fish.to_vec();
    }
}

/// circlesそれぞれの円画像を読み込み、色を塗り替える。読めない画像があればNone
fn recolored_images(circles: &[BoardCircle]) -> Option<Vec<RgbaImage>> {
    circles
        .iter()
        .map(|c| load_circle_image(c.number).map(|image| recolor(&image, c.color)))
        .collect()
}

/// ボード全体の画像を作る。円を並び順に重ね、rippleがあればその手前にリングを重ねる。
/// font_sizeは1セルのピクセル数(幅, 高さ)。円の画像が読めない場合はNone。
/// 描画では盤面(円だけ)とパッチを別々に作るため、合成結果の確認用にテストでだけ使う
#[cfg(test)]
pub fn build_board_image(
    board: Rect,
    font_size: (u16, u16),
    circles: &[BoardCircle],
    ripple: Option<&Ripple>,
) -> Option<RgbaImage> {
    let images = recolored_images(circles)?;
    let mut image = compose_circles(board, font_size, circles, &images);
    if let Some(ripple) = ripple {
        add_ripple(&mut image, board, font_size, ripple);
    }
    Some(image)
}

/// 円の画像(circlesと同じ並び)を、compose_boardでボード全体の1枚に重ねる
fn compose_circles(
    board: Rect,
    font_size: (u16, u16),
    circles: &[BoardCircle],
    images: &[RgbaImage],
) -> RgbaImage {
    let layers: Vec<(Rect, &RgbaImage)> = circles
        .iter()
        .zip(images)
        .map(|(c, image)| (c.rect, image))
        .collect();
    compose_board(board, font_size, &layers)
}

/// リングの線の太さ(ピクセル)。細くなりすぎて消えないよう1ピクセル以上にする
fn ring_thickness(cell_height: f64) -> f64 {
    (RIPPLE_THICKNESS_CELLS * cell_height).max(1.0)
}

/// 波紋が広がり切るまでにリングが届く範囲(セル)。パッチはこの範囲を切り出す。
/// 広がり切った時の半径は波紋ごと(押した円の大きさごと)に違うので、その波紋の値を使う。
/// ボードの中に収め、ボードの左端の2列は含めない。盤面の画像はボード左上のセルを起点に描かれ、
/// kittyでは各行の左端のセルが起点になる。そこにパッチを重ねると盤面の画像が描かれなくなる。
/// また、ratatuiの差分処理は画像データの入ったセルの直後の1セルを出力しないため、
/// 起点のすぐ右の列にパッチの起点を置くとパッチが端末へ送られない。
/// 範囲が無い(ボードが2列以下等)時は空のRect
fn ripple_patch_rect(board: Rect, font_size: (u16, u16), ripple: &Ripple) -> Rect {
    let cell_height = f64::from(font_size.1.max(1));
    // 中心からリングの外側の端までの距離(ピクセル)。丸め誤差の分として1ピクセル足す
    let reach = ripple.max_radius_cells() * cell_height + ring_thickness(cell_height) / 2.0 + 1.0;
    patch_rect_around(board, font_size, ripple.center(), reach)
}

/// バツ印の線の太さ(ピクセル)。細くなりすぎて消えないよう1ピクセル以上にする
fn mark_thickness(cell_height: f64) -> f64 {
    (WRONG_MARK_THICKNESS_CELLS * cell_height).max(1.0)
}

/// バツ印が届く範囲(セル)。範囲の決め方(ボードの左端2列を含めない等)は波紋と同じ
fn mark_patch_rect(board: Rect, font_size: (u16, u16), mark: &WrongMark) -> Rect {
    let cell_height = f64::from(font_size.1.max(1));
    // 腕の先に線の太さぶん(斜めの線は縦横に太さ/√2ずつはみ出す)と丸め誤差の1ピクセルを足す
    let reach = WRONG_MARK_HALF_SIZE_CELLS * cell_height + mark_thickness(cell_height) + 1.0;
    patch_rect_around(board, font_size, mark.center(), reach)
}

/// いま表示している演出(波紋・バツ印)が届く範囲を合わせたもの。どれも範囲が無ければ空のRect
fn effects_rect(
    board: Rect,
    font_size: (u16, u16),
    ripple: Option<&Ripple>,
    mark: Option<&WrongMark>,
) -> Rect {
    let rects = [
        ripple.map(|ripple| ripple_patch_rect(board, font_size, ripple)),
        mark.map(|mark| mark_patch_rect(board, font_size, mark)),
    ];
    rects
        .into_iter()
        .flatten()
        .filter(|rect| !rect.is_empty())
        .reduce(|a, b| a.union(b))
        .unwrap_or_default()
}

/// セル(column, row)の中央からreachピクセル以内に届く範囲(セル)。ボードの中に収め、
/// ボードの左端の2列は含めない(理由はripple_patch_rectを参照)。範囲が無い時は空のRect
fn patch_rect_around(
    board: Rect,
    font_size: (u16, u16),
    (column, row): (u16, u16),
    reach: f64,
) -> Rect {
    let cell_width = f64::from(font_size.0.max(1));
    let cell_height = f64::from(font_size.1.max(1));
    let center_x = (f64::from(column) + 0.5) * cell_width;
    let center_y = (f64::from(row) + 0.5) * cell_height;
    let left = ((center_x - reach) / cell_width)
        .floor()
        .max(f64::from(board.x) + 2.0);
    let right = ((center_x + reach) / cell_width)
        .ceil()
        .min(f64::from(board.right()));
    let top = ((center_y - reach) / cell_height)
        .floor()
        .max(f64::from(board.y));
    let bottom = ((center_y + reach) / cell_height)
        .ceil()
        .min(f64::from(board.bottom()));
    if right <= left || bottom <= top {
        return Rect::default();
    }
    Rect::new(
        left as u16,
        top as u16,
        (right - left) as u16,
        (bottom - top) as u16,
    )
}

/// 盤面の画像(円だけ)からセル範囲rectを切り出し、fishのうち範囲にかかる魚を円の後ろに、
/// rippleがあればリングを、markがあればバツ印を円の手前に重ねたパッチを作る(バツ印が一番手前)
fn effects_patch(
    board_image: &RgbaImage,
    board: Rect,
    font_size: (u16, u16),
    rect: Rect,
    ripple: Option<&Ripple>,
    mark: Option<&WrongMark>,
    fish: &[BoardFish],
) -> RgbaImage {
    let cell_width = u32::from(font_size.0.max(1));
    let cell_height = u32::from(font_size.1.max(1));
    let circles = imageops::crop_imm(
        board_image,
        u32::from(rect.x.saturating_sub(board.x)) * cell_width,
        u32::from(rect.y.saturating_sub(board.y)) * cell_height,
        u32::from(rect.width) * cell_width,
        u32::from(rect.height) * cell_height,
    )
    .to_image();
    let mut patch = if fish.is_empty() {
        circles
    } else {
        let mut layer = fish_layer(rect, font_size, patch_area(board), fish);
        imageops::overlay(&mut layer, &circles, 0, 0);
        layer
    };
    if let Some(ripple) = ripple {
        add_ripple(&mut patch, rect, font_size, ripple);
    }
    if let Some(mark) = mark {
        add_mark(&mut patch, rect, font_size, mark);
    }
    patch
}

/// パッチ・魚を描ける範囲。盤面から左端の2列を除いたもの(理由はripple_patch_rectを参照)。
/// 魚もここにしか描かない(描き直せない所に描くと、魚が離れた後も消せずに残るため)
fn patch_area(board: Rect) -> Rect {
    let left = board.x.saturating_add(2).min(board.right());
    Rect::new(left, board.y, board.right() - left, board.height)
}

/// 円だけの盤面の画像circles_onlyの後ろに、魚を描いたボード全体の画像
fn compose_scene(
    circles_only: &RgbaImage,
    board: Rect,
    font_size: (u16, u16),
    fish: &[BoardFish],
) -> RgbaImage {
    if fish.is_empty() {
        return circles_only.clone();
    }
    let mut scene = fish_layer(board, font_size, patch_area(board), fish);
    imageops::overlay(&mut scene, circles_only, 0, 0);
    scene
}

/// canvas(セル範囲)と同じ大きさのピクセル画像に、fishを並び順に描く(後の魚ほど手前)。
/// clip(セル範囲)の外には描かない
fn fish_layer(canvas: Rect, font_size: (u16, u16), clip: Rect, fish: &[BoardFish]) -> RgbaImage {
    let cell_width = i64::from(font_size.0.max(1));
    let cell_height = i64::from(font_size.1.max(1));
    let mut layer = RgbaImage::new(
        u32::from(canvas.width) * cell_width as u32,
        u32::from(canvas.height) * cell_height as u32,
    );
    for one in fish {
        let (sprite, (dx, dy)) = fish_sprite_image(font_size, one);
        let x = (i64::from(one.x) - i64::from(canvas.x)) * cell_width + dx;
        let y = (i64::from(one.y) - i64::from(canvas.y)) * cell_height + dy;
        imageops::overlay(&mut layer, &sprite, x, y);
    }
    let clip = clip.intersection(canvas);
    if clip != canvas {
        // clipのピクセル範囲(canvasの左上から)
        let left = i64::from(clip.x) - i64::from(canvas.x);
        let top = i64::from(clip.y) - i64::from(canvas.y);
        let columns = (left * cell_width)..((left + i64::from(clip.width)) * cell_width);
        let rows = (top * cell_height)..((top + i64::from(clip.height)) * cell_height);
        for (x, y, pixel) in layer.enumerate_pixels_mut() {
            if !columns.contains(&i64::from(x)) || !rows.contains(&i64::from(y)) {
                pixel.0 = [0, 0, 0, 0];
            }
        }
    }
    layer
}

/// 魚のドット絵を魚の色に塗り、魚のセル範囲に縦横比を保って収まる大きさにしたものと、
/// セル範囲の左上からの位置(ピクセル)。ドットがつぶれないよう、収まるなら整数倍に拡大する
fn fish_sprite_image(font_size: (u16, u16), fish: &BoardFish) -> (RgbaImage, (i64, i64)) {
    let frame_width = u32::from(FISH_WIDTH) * u32::from(font_size.0.max(1));
    let frame_height = u32::from(FISH_HEIGHT) * u32::from(font_size.1.max(1));
    let scale = f64::min(
        f64::from(frame_width) / f64::from(SPRITE_WIDTH),
        f64::from(frame_height) / f64::from(SPRITE_HEIGHT),
    );
    let scale = if scale >= 1.0 { scale.floor() } else { scale };
    let width = ((f64::from(SPRITE_WIDTH) * scale).round() as u32).clamp(1, frame_width);
    let height = ((f64::from(SPRITE_HEIGHT) * scale).round() as u32).clamp(1, frame_height);
    let image = imageops::resize(
        &recolor(&sprite(fish.facing_right), fish.color),
        width,
        height,
        FilterType::Nearest,
    );
    let offset = (
        i64::from((frame_width - width) / 2),
        i64::from((frame_height - height) / 2),
    );
    (image, offset)
}

/// 前に見えていた魚shownから、いまの魚fishへ位置・向き・色が変わった魚ごとに、
/// 前の位置と今の位置を合わせたセル範囲(areaの中)
fn moved_fish_rects(shown: &[BoardFish], fish: &[BoardFish], area: Rect) -> Vec<Rect> {
    (0..shown.len().max(fish.len()))
        .filter_map(|index| {
            let (old, new) = (shown.get(index), fish.get(index));
            if old == new {
                return None;
            }
            [old, new]
                .into_iter()
                .flatten()
                .map(|one| one.rect().intersection(area))
                .filter(|rect| !rect.is_empty())
                .reduce(|a, b| a.union(b))
        })
        .collect()
}

/// fishのうち、セル範囲rectにかかる魚
fn fish_in(fish: &[BoardFish], rect: Rect) -> Vec<BoardFish> {
    fish.iter()
        .copied()
        .filter(|one| one.rect().intersects(rect))
        .collect()
}

/// パッチにする範囲のうち、重なるもの・左右に隣り合うものを1つにまとめる(まとめた範囲が
/// さらに別の範囲にかかれば、それもまとめる)。演出の範囲を含むものは演出の範囲として扱う。
/// 1フレームに描くパッチ同士が重なると、後に描いた方の起点セルが先の方の画像データを上書きして
/// 端末へ送られなくなる。左右に隣り合うと、左のパッチの画像データのセル(kittyでは各行の左端)の
/// すぐ右に右のパッチの起点が来ることがあり、ratatuiの差分処理がそのセルを出力しない
fn merge_patch_items(items: &[PatchItem]) -> Vec<PatchItem> {
    let mut merged: Vec<PatchItem> = items
        .iter()
        .copied()
        .filter(|item| !item.rect.is_empty())
        .collect();
    loop {
        let pair = (0..merged.len()).find_map(|i| {
            (i + 1..merged.len())
                .find(|&j| patches_touch(merged[i].rect, merged[j].rect))
                .map(|j| (i, j))
        });
        let Some((i, j)) = pair else {
            return merged;
        };
        let other = merged.swap_remove(j);
        merged[i] = PatchItem {
            rect: merged[i].rect.union(other.rect),
            effects: merged[i].effects || other.effects,
        };
    }
}

/// 2つのパッチの範囲が、重なるか左右に隣り合うか
fn patches_touch(a: Rect, b: Rect) -> bool {
    let widened = |rect: Rect| Rect::new(rect.x, rect.y, rect.width.saturating_add(1), rect.height);
    widened(a).intersects(b) || widened(b).intersects(a)
}

/// rectをareaの中で1セル広げる(右・下・上・左の順に、広げられる向きへ)。広げられなければNone
fn grow_within(rect: Rect, area: Rect) -> Option<Rect> {
    if rect.right() < area.right() {
        Some(Rect::new(rect.x, rect.y, rect.width + 1, rect.height))
    } else if rect.bottom() < area.bottom() {
        Some(Rect::new(rect.x, rect.y, rect.width, rect.height + 1))
    } else if rect.y > area.y {
        Some(Rect::new(rect.x, rect.y - 1, rect.width, rect.height + 1))
    } else if rect.x > area.x {
        Some(Rect::new(rect.x - 1, rect.y, rect.width + 1, rect.height))
    } else {
        None
    }
}

/// パッチの画像の指紋
fn fingerprint(image: &RgbaImage) -> Fingerprint {
    let mut hasher = DefaultHasher::new();
    image.as_raw().hash(&mut hasher);
    (image.width(), image.height(), hasher.finish())
}

/// area(セル範囲)を写したimageに、バツ印を重ねる。
/// 腕の長さ・線の太さはセルの高さを単位にし、ピクセル上で正方形のバツ印になるようにする
fn add_mark(image: &mut RgbaImage, area: Rect, font_size: (u16, u16), mark: &WrongMark) {
    let cell_width = f64::from(font_size.0.max(1));
    let cell_height = f64::from(font_size.1.max(1));
    let (column, row) = mark.center();
    // クリックしたセルの中央を交点にする
    let center = (
        (f64::from(column) - f64::from(area.x) + 0.5) * cell_width,
        (f64::from(row) - f64::from(area.y) + 0.5) * cell_height,
    );
    let cross = cross_image(
        u32::from(area.width) * cell_width as u32,
        u32::from(area.height) * cell_height as u32,
        center,
        WRONG_MARK_HALF_SIZE_CELLS * cell_height,
        mark_thickness(cell_height),
        WRONG_MARK_COLOR,
    );
    imageops::overlay(image, &cross, 0, 0);
}

/// area(セル範囲)を写したimageに、波紋のいまのリングを重ねる
fn add_ripple(image: &mut RgbaImage, area: Rect, font_size: (u16, u16), ripple: &Ripple) {
    let ring = ripple_layer(area, font_size, ripple);
    imageops::overlay(image, &ring, 0, 0);
}

/// 波紋のいまの状態を、area(セル範囲)と同じ大きさ(ピクセル)のリング画像にする。
/// 半径・線の太さはセルの高さを単位にし、ピクセル上で真円になるようにする
fn ripple_layer(area: Rect, font_size: (u16, u16), ripple: &Ripple) -> RgbaImage {
    let cell_width = f64::from(font_size.0.max(1));
    let cell_height = f64::from(font_size.1.max(1));
    let (column, row) = ripple.center();
    // クリックしたセルの中央を中心にする
    let center = (
        (f64::from(column) - f64::from(area.x) + 0.5) * cell_width,
        (f64::from(row) - f64::from(area.y) + 0.5) * cell_height,
    );
    ring_image(
        u32::from(area.width) * cell_width as u32,
        u32::from(area.height) * cell_height as u32,
        center,
        ripple.radius_cells() * cell_height,
        ring_thickness(cell_height),
        ripple.opacity(),
        RIPPLE_COLOR,
    )
}

/// board(セル単位)と同じ大きさのピクセル画像に、layers(セル単位の位置, 画像)を並び順に
/// 重ね合わせる。後の画像ほど手前で、透明な部分からは奥の画像が見える。円の無い所は透明。
/// 各画像は縦横比を保ったまま位置の枠に収まる最大の大きさにして、枠の中央に置く。
/// font_sizeは1セルのピクセル数(幅, 高さ)
pub fn compose_board(
    board: Rect,
    font_size: (u16, u16),
    layers: &[(Rect, &RgbaImage)],
) -> RgbaImage {
    let cell_width = u32::from(font_size.0.max(1));
    let cell_height = u32::from(font_size.1.max(1));
    let mut canvas = RgbaImage::new(
        u32::from(board.width) * cell_width,
        u32::from(board.height) * cell_height,
    );
    for &(rect, image) in layers {
        if rect.is_empty() || image.width() == 0 || image.height() == 0 {
            continue;
        }
        let frame_width = u32::from(rect.width) * cell_width;
        let frame_height = u32::from(rect.height) * cell_height;
        let scale = f64::min(
            f64::from(frame_width) / f64::from(image.width()),
            f64::from(frame_height) / f64::from(image.height()),
        );
        let width = ((f64::from(image.width()) * scale).round() as u32).clamp(1, frame_width);
        let height = ((f64::from(image.height()) * scale).round() as u32).clamp(1, frame_height);
        let scaled = imageops::resize(image, width, height, FilterType::Triangle);
        let x = (i64::from(rect.x) - i64::from(board.x)) * i64::from(cell_width)
            + i64::from((frame_width - width) / 2);
        let y = (i64::from(rect.y) - i64::from(board.y)) * i64::from(cell_height)
            + i64::from((frame_height - height) / 2);
        imageops::overlay(&mut canvas, &scaled, x, y);
    }
    canvas
}

/// 端末の画像プロトコルを調べる。sixel/kitty/iTerm2のどれかが使える時だけSome。
/// テストでは端末に問い合わせず、常にテキスト表示にする(実行環境で結果が変わらないように)
fn detect_picker() -> Option<Picker> {
    if cfg!(test) {
        return None;
    }
    Picker::from_query_stdio().ok().filter(|picker| {
        matches!(
            picker.protocol_type(),
            ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2
        )
    })
}

/// 画像プロトコル非対応の端末向け表示。円の範囲(内接楕円)を色付きの網掛けで塗り、
/// 中央に色付きの丸囲み数字を置く。網掛けでクリックできる範囲(大きさ)が分かるようにする
fn render_text(frame: &mut Frame, rect: Rect, number: u8, color: [u8; 3]) {
    let [r, g, b] = color;
    let fg = Color::Rgb(r, g, b);
    let buffer = frame.buffer_mut();
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if circle_contains(rect, x, y) {
                buffer[(x, y)].set_symbol("░").set_fg(fg);
            }
        }
    }
    let Some(digit) = circled_digit(number) else {
        return;
    };
    // 丸囲み数字は端末によって2セル幅で表示されるため、両隣を空白にして網掛けと重ならないようにする。
    // 置く位置は配置側(layout)が手前の円に覆わせないよう守っている範囲と同じにする
    let label = label_area(rect);
    let text = if label.width >= 3 {
        format!(" {digit} ")
    } else {
        digit.to_string()
    };
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(fg).add_modifier(Modifier::BOLD)),
        label,
    );
}

/// 画像プロトコル非対応の端末向けのバツ印。クリックしたセルに赤い✗を置く(ボードの外なら描かない)
fn render_mark_text(frame: &mut Frame, board: Rect, mark: &WrongMark) {
    let (column, row) = mark.center();
    if !board.contains(ratatui::layout::Position::new(column, row)) {
        return;
    }
    let [r, g, b] = WRONG_MARK_COLOR;
    frame.buffer_mut()[(column, row)].set_symbol("✗").set_style(
        Style::default()
            .fg(Color::Rgb(r, g, b))
            .add_modifier(Modifier::BOLD),
    );
}

#[cfg(test)]
mod tests {
    use super::super::layout::CircleSize;
    use super::super::ripple::RIPPLE_DURATION;
    use super::*;
    use image::Rgba;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn all_twenty_circle_images_are_embedded() {
        for number in 1..=20u8 {
            let image = load_circle_image(number)
                .unwrap_or_else(|| panic!("{number}.pngが埋め込まれていること"));
            assert_eq!(image.dimensions(), (128, 128));
        }
        assert!(load_circle_image(0).is_none());
        assert!(load_circle_image(21).is_none());
    }

    #[test]
    fn recolor_replaces_white_keeps_black_and_transparent() {
        let mut image = RgbaImage::new(3, 1);
        image.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        image.put_pixel(1, 0, Rgba([0, 0, 0, 255]));
        image.put_pixel(2, 0, Rgba([0, 0, 0, 0]));
        let out = recolor(&image, [10, 200, 30]);
        assert_eq!(
            out.get_pixel(0, 0).0,
            [10, 200, 30, 255],
            "白は指定色になる"
        );
        assert_eq!(out.get_pixel(1, 0).0, [0, 0, 0, 255], "黒はそのまま");
        assert_eq!(out.get_pixel(2, 0).0[3], 0, "透明はそのまま透明");
    }

    #[test]
    fn recolor_keeps_alpha_of_antialiased_white_edge() {
        let mut image = RgbaImage::new(1, 1);
        image.put_pixel(0, 0, Rgba([250, 250, 250, 100]));
        let out = recolor(&image, [200, 100, 50]);
        assert_eq!(out.get_pixel(0, 0).0, [200, 100, 50, 100]);
    }

    #[test]
    fn recolor_of_real_asset_keeps_black_digit_pixels() {
        let image = load_circle_image(8).unwrap();
        let out = recolor(&image, [0, 128, 255]);
        let count = |img: &RgbaImage, rgb: [u8; 3]| {
            img.pixels()
                .filter(|p| p.0[3] == 255 && p.0[..3] == rgb)
                .count()
        };
        assert!(count(&out, [0, 128, 255]) > 1000, "円の塗りが指定色になる");
        assert_eq!(count(&out, [255, 255, 255]), 0, "白は残らない");
        assert_eq!(
            count(&image, [0, 0, 0]),
            count(&out, [0, 0, 0]),
            "数字の黒は変わらない"
        );
    }

    #[test]
    fn circled_digit_maps_one_to_twenty() {
        assert_eq!(circled_digit(1), Some('①'));
        assert_eq!(circled_digit(10), Some('⑩'));
        assert_eq!(circled_digit(20), Some('⑳'));
        assert_eq!(circled_digit(0), None);
        assert_eq!(circled_digit(21), None);
    }

    /// テキスト表示でボードを描き、バッファを返す
    fn render_fallback(
        width: u16,
        height: u16,
        circles: &[BoardCircle],
    ) -> ratatui::buffer::Buffer {
        let renderer = CircleRenderer::new();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| renderer.render_board(frame, frame.area(), circles, None, None))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba(rgba))
    }

    #[test]
    fn fallback_draws_later_circles_on_top() {
        // 大きい円1の左側に小さい円2が重なっている。重なった所は後に描いた円の色になる
        let large = BoardCircle {
            rect: Rect::new(0, 0, 14, 7),
            number: 1,
            color: [10, 10, 10],
        };
        let small = BoardCircle {
            rect: Rect::new(0, 2, 6, 3),
            number: 2,
            color: [200, 200, 200],
        };
        let buffer = render_fallback(20, 8, &[large, small]);
        assert_eq!(
            buffer[(2, 3)].fg,
            Color::Rgb(200, 200, 200),
            "小さい円が手前"
        );
        assert_eq!(
            buffer[(10, 3)].fg,
            Color::Rgb(10, 10, 10),
            "重ならない所は大きい円"
        );
        let buffer = render_fallback(20, 8, &[small, large]);
        assert_eq!(
            buffer[(2, 3)].fg,
            Color::Rgb(10, 10, 10),
            "描く順番どおりに重なる"
        );
    }

    #[test]
    fn compose_board_stacks_layers_in_order() {
        // ボード20x10セル、1セル10x20ピクセル => 200x200ピクセル
        let board = Rect::new(3, 5, 20, 10);
        let red = solid(4, 4, [255, 0, 0, 255]);
        let blue = solid(4, 4, [0, 0, 255, 255]);
        // 大きい円: 12x6セル=120x120ピクセル(ボード左上から)。小さい円: 4x2セル=40x40ピクセル
        let large = Rect::new(3, 5, 12, 6);
        let small = Rect::new(7, 7, 4, 2);
        let image = compose_board(board, (10, 20), &[(large, &red), (small, &blue)]);
        assert_eq!(image.dimensions(), (200, 200));
        assert_eq!(image.get_pixel(10, 10).0, [255, 0, 0, 255], "大きい円");
        assert_eq!(image.get_pixel(50, 50).0, [0, 0, 255, 255], "後の円が手前");
        assert_eq!(image.get_pixel(150, 150).0[3], 0, "円の無い所は透明");

        let image = compose_board(board, (10, 20), &[(small, &blue), (large, &red)]);
        assert_eq!(
            image.get_pixel(50, 50).0,
            [255, 0, 0, 255],
            "並び順どおりに重なる"
        );
    }

    #[test]
    fn compose_board_keeps_aspect_ratio_and_centers_image() {
        // 4x1セル=40x20ピクセルの枠に正方形の画像を置くと、20x20で横方向の中央に来る
        let board = Rect::new(0, 0, 4, 1);
        let green = solid(8, 8, [0, 255, 0, 255]);
        let image = compose_board(board, (10, 20), &[(board, &green)]);
        assert_eq!(image.dimensions(), (40, 20));
        assert_eq!(image.get_pixel(5, 10).0[3], 0, "左の余白は透明");
        assert_eq!(image.get_pixel(20, 10).0, [0, 255, 0, 255]);
        assert_eq!(image.get_pixel(35, 10).0[3], 0, "右の余白は透明");
    }

    #[test]
    fn compose_board_shows_lower_layer_through_transparent_pixels() {
        // 上の画像の透明な部分(円の外側の角)からは下の円が見える
        let board = Rect::new(0, 0, 2, 1);
        let below = solid(2, 2, [255, 0, 0, 255]);
        let mut above = solid(2, 2, [0, 0, 255, 255]);
        above.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        let image = compose_board(board, (10, 20), &[(board, &below), (board, &above)]);
        assert_eq!(
            image.get_pixel(1, 1).0,
            [255, 0, 0, 255],
            "透明な角から下が見える"
        );
        assert_eq!(image.get_pixel(18, 18).0, [0, 0, 255, 255]);
    }

    // --- 波紋のレイヤー ---

    /// 20x10セル・1セル10x20ピクセル(=200x200ピクセル)のボード
    const RIPPLE_BOARD: Rect = Rect::new(0, 0, 20, 10);
    const RIPPLE_FONT: (u16, u16) = (10, 20);

    /// 持続時間の半分まで進めた、セル(10, 5)中心の波紋
    fn half_way_ripple() -> Ripple {
        Ripple::new(10, 5, CircleSize::Small)
            .advanced(RIPPLE_DURATION / 2)
            .unwrap()
    }

    /// 波紋の中心から右へ半径ぶん進んだピクセル(リングの上)
    fn point_on_ring(ripple: &Ripple) -> (u32, u32) {
        let radius = ripple.radius_cells() * f64::from(RIPPLE_FONT.1);
        // セル(10, 5)の中心 = ピクセル(105, 110)
        ((105.0 + radius).round() as u32, 110)
    }

    #[test]
    fn board_image_includes_ring_layer_while_ripple_is_active() {
        let ripple = half_way_ripple();
        let corner = BoardCircle {
            rect: Rect::new(0, 0, 4, 2),
            number: 1,
            color: [10, 20, 30],
        };
        let image = build_board_image(RIPPLE_BOARD, RIPPLE_FONT, &[corner], Some(&ripple))
            .expect("画像が作れること");
        assert_eq!(image.dimensions(), (200, 200));
        let (x, y) = point_on_ring(&ripple);
        assert!(image.get_pixel(x, y).0[3] > 0, "リングの上は不透明");
        assert_eq!(
            image.get_pixel(105, 110).0[3],
            0,
            "波紋の中心は透明(リング状)"
        );
        assert_eq!(image.get_pixel(195, 5).0[3], 0, "リングから離れた所は透明");
        assert_eq!(image.get_pixel(20, 20).0[3], 255, "円はそのまま描かれる");

        let without = build_board_image(RIPPLE_BOARD, RIPPLE_FONT, &[corner], None).unwrap();
        assert_eq!(
            without.get_pixel(x, y).0[3],
            0,
            "波紋が無ければリングも無い"
        );
    }

    #[test]
    fn ring_is_drawn_in_front_of_circles() {
        // ボード全体を覆う大きな円の上に波紋を重ねると、リングの所だけ色が変わる
        let ripple = half_way_ripple();
        let big = BoardCircle {
            rect: RIPPLE_BOARD,
            number: 2,
            color: [0, 0, 255],
        };
        let with = build_board_image(RIPPLE_BOARD, RIPPLE_FONT, &[big], Some(&ripple)).unwrap();
        let without = build_board_image(RIPPLE_BOARD, RIPPLE_FONT, &[big], None).unwrap();
        let (x, y) = point_on_ring(&ripple);
        assert_eq!(without.get_pixel(x, y).0, [0, 0, 255, 255]);
        assert_ne!(
            with.get_pixel(x, y).0,
            [0, 0, 255, 255],
            "リングが手前に重なる"
        );
        assert_eq!(
            with.get_pixel(20, 100).0,
            without.get_pixel(20, 100).0,
            "リングの無い所は円のまま"
        );
    }

    #[test]
    fn board_image_with_only_ripple_has_ring() {
        let ripple = half_way_ripple();
        let image = build_board_image(RIPPLE_BOARD, RIPPLE_FONT, &[], Some(&ripple)).unwrap();
        let (x, y) = point_on_ring(&ripple);
        assert!(image.get_pixel(x, y).0[3] > 0);
    }

    /// 画像プロトコル(テストではハーフブロック)で描く描画器
    fn image_renderer() -> CircleRenderer {
        let mut picker = Picker::from_fontsize((4, 8));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        CircleRenderer::with_picker(picker)
    }

    #[test]
    fn image_mode_render_with_and_without_ripple_does_not_panic() {
        let renderer = image_renderer();
        assert!(renderer.uses_image());
        let circle = BoardCircle {
            rect: Rect::new(2, 1, 8, 4),
            number: 3,
            color: [200, 100, 50],
        };
        let mut terminal = Terminal::new(TestBackend::new(30, 12)).unwrap();
        let mut ripple = Some(Ripple::new(6, 3, CircleSize::Small));
        // 波紋が広がって消えるまでの各フレームと、消えた後を描く
        for _ in 0..8 {
            terminal
                .draw(|frame| {
                    renderer.render_board(frame, frame.area(), &[circle], ripple.as_ref(), None)
                })
                .unwrap();
            ripple = ripple.and_then(|r| r.advanced(RIPPLE_DURATION / 6));
        }
        assert!(ripple.is_none());
        // 端のセルを中心にした波紋・小さすぎる画面でもパニックしない
        let edge = Ripple::new(29, 11, CircleSize::Small);
        terminal
            .draw(|frame| renderer.render_board(frame, frame.area(), &[circle], Some(&edge), None))
            .unwrap();
        let mut tiny = Terminal::new(TestBackend::new(1, 1)).unwrap();
        tiny.draw(|frame| renderer.render_board(frame, frame.area(), &[circle], Some(&edge), None))
            .unwrap();
    }

    // --- 波紋の部分画像(パッチ) ---

    /// ボード画像のうち、セル範囲rectに当たる部分を切り出す
    fn crop_cells(image: &RgbaImage, board: Rect, font: (u16, u16), rect: Rect) -> RgbaImage {
        let (cw, ch) = (u32::from(font.0), u32::from(font.1));
        imageops::crop_imm(
            image,
            u32::from(rect.x - board.x) * cw,
            u32::from(rect.y - board.y) * ch,
            u32::from(rect.width) * cw,
            u32::from(rect.height) * ch,
        )
        .to_image()
    }

    /// 持続時間の終わる直前(半径が最大)の、セル(10, 5)中心の波紋
    fn widest_ripple() -> Ripple {
        Ripple::new(10, 5, CircleSize::Small)
            .advanced(RIPPLE_DURATION - std::time::Duration::from_millis(1))
            .unwrap()
    }

    #[test]
    fn patch_rect_covers_whole_ring_inside_board() {
        let ripple = widest_ripple();
        let rect = ripple_patch_rect(RIPPLE_BOARD, RIPPLE_FONT, &ripple);
        assert_eq!(
            rect.intersection(RIPPLE_BOARD),
            rect,
            "ボードからはみ出さない"
        );
        assert!(
            rect.width < RIPPLE_BOARD.width && rect.height < RIPPLE_BOARD.height,
            "ボード全体より小さい: {rect:?}"
        );
        // 広がり切ったリングの全ピクセルがパッチの中に入る
        let full = build_board_image(RIPPLE_BOARD, RIPPLE_FONT, &[], Some(&ripple)).unwrap();
        let (cw, ch) = (u32::from(RIPPLE_FONT.0), u32::from(RIPPLE_FONT.1));
        let inside = |x: u32, y: u32| {
            (u32::from(rect.x) * cw..u32::from(rect.right()) * cw).contains(&x)
                && (u32::from(rect.y) * ch..u32::from(rect.bottom()) * ch).contains(&y)
        };
        let ring: Vec<(u32, u32)> = full
            .enumerate_pixels()
            .filter(|(_, _, p)| p.0[3] > 0)
            .map(|(x, y, _)| (x, y))
            .collect();
        assert!(!ring.is_empty());
        for (x, y) in ring {
            assert!(inside(x, y), "リングの({x},{y})がパッチ{rect:?}の外にある");
        }
    }

    /// 持続時間の終わる直前(半径が最大)の、セルcenterを中心とするsizeの円の波紋
    fn widest_ripple_of(center: (u16, u16), size: CircleSize) -> Ripple {
        Ripple::new(center.0, center.1, size)
            .advanced(RIPPLE_DURATION - std::time::Duration::from_millis(1))
            .unwrap()
    }

    #[test]
    fn huge_circle_ripple_patch_is_larger_and_still_covers_whole_ring() {
        // 特大の円の波紋は小さい円の波紋より大きく広がり、パッチもそのぶん広く切り出す
        let board = Rect::new(0, 0, 60, 30);
        let center = (30, 15);
        let small = ripple_patch_rect(
            board,
            RIPPLE_FONT,
            &widest_ripple_of(center, CircleSize::Small),
        );
        let huge_ripple = widest_ripple_of(center, CircleSize::Huge);
        let huge = ripple_patch_rect(board, RIPPLE_FONT, &huge_ripple);
        assert!(
            huge.width > small.width && huge.height > small.height,
            "特大{huge:?} > 小{small:?}"
        );
        let full = build_board_image(board, RIPPLE_FONT, &[], Some(&huge_ripple)).unwrap();
        let (cw, ch) = (u32::from(RIPPLE_FONT.0), u32::from(RIPPLE_FONT.1));
        let ring: Vec<(u32, u32)> = full
            .enumerate_pixels()
            .filter(|(_, _, p)| p.0[3] > 0)
            .map(|(x, y, _)| (x, y))
            .collect();
        assert!(!ring.is_empty());
        for (x, y) in ring {
            assert!(
                (u32::from(huge.x) * cw..u32::from(huge.right()) * cw).contains(&x)
                    && (u32::from(huge.y) * ch..u32::from(huge.bottom()) * ch).contains(&y),
                "リングの({x},{y})がパッチ{huge:?}の外にある"
            );
        }
    }

    #[test]
    fn huge_circle_ripple_ring_is_drawn_farther_than_small_one() {
        // 同じ時点で比べると、特大の円の波紋のリングは中心からより遠くに描かれる
        let board = Rect::new(0, 0, 60, 30);
        let farthest = |size: CircleSize| {
            let ripple = Ripple::new(30, 15, size)
                .advanced(RIPPLE_DURATION / 2)
                .unwrap();
            let image = build_board_image(board, RIPPLE_FONT, &[], Some(&ripple)).unwrap();
            // セル(30, 15)の中心 = ピクセル(305, 310)
            image
                .enumerate_pixels()
                .filter(|(_, _, p)| p.0[3] > 0)
                .map(|(x, y, _)| (f64::from(x) + 0.5 - 305.0).hypot(f64::from(y) + 0.5 - 310.0))
                .fold(0.0, f64::max)
        };
        let (huge, small) = (farthest(CircleSize::Huge), farthest(CircleSize::Small));
        assert!(huge > small * 2.0, "特大{huge} > 小{small}の2倍");
    }

    #[test]
    fn patch_rect_avoids_two_left_columns_of_board() {
        // 盤面画像の起点(左端の列)にパッチを重ねると盤面画像が描かれなくなり、
        // そのすぐ右の列は差分処理が起点の画像データの表示幅ぶん出力を飛ばすため、左端の2列は含めない
        let board = Rect::new(2, 3, 20, 10);
        let corner = Ripple::new(2, 3, CircleSize::Small);
        let rect = ripple_patch_rect(board, RIPPLE_FONT, &corner);
        assert!(!rect.is_empty());
        assert_eq!(rect.x, board.x + 2);
        assert_eq!(rect.y, board.y);
        assert_eq!(rect.intersection(board), rect);
        // 2列以下のボードではパッチを作らない
        let narrow = Rect::new(0, 0, 2, 10);
        assert!(
            ripple_patch_rect(narrow, RIPPLE_FONT, &Ripple::new(0, 5, CircleSize::Small))
                .is_empty()
        );
    }

    #[test]
    fn patch_pixels_match_board_image_with_ring() {
        // パッチ=「円と波紋を重ねたボード画像」の同じ範囲の切り出しと一致する
        let ripple = half_way_ripple();
        let big = BoardCircle {
            rect: RIPPLE_BOARD,
            number: 2,
            color: [0, 0, 255],
        };
        let images = recolored_images(&[big]).unwrap();
        let circles_only = compose_circles(RIPPLE_BOARD, RIPPLE_FONT, &[big], &images);
        let rect = ripple_patch_rect(RIPPLE_BOARD, RIPPLE_FONT, &ripple);
        let patch = effects_patch(
            &circles_only,
            RIPPLE_BOARD,
            RIPPLE_FONT,
            rect,
            Some(&ripple),
            None,
            &[],
        );
        let full = build_board_image(RIPPLE_BOARD, RIPPLE_FONT, &[big], Some(&ripple)).unwrap();
        assert_eq!(patch, crop_cells(&full, RIPPLE_BOARD, RIPPLE_FONT, rect));
        assert_ne!(
            patch,
            crop_cells(&circles_only, RIPPLE_BOARD, RIPPLE_FONT, rect),
            "リングが重なっている"
        );
        // 波紋が消えた後のパッチはリングの無いボードの切り出し
        let clean = effects_patch(
            &circles_only,
            RIPPLE_BOARD,
            RIPPLE_FONT,
            rect,
            None,
            None,
            &[],
        );
        assert_eq!(
            clean,
            crop_cells(&circles_only, RIPPLE_BOARD, RIPPLE_FONT, rect)
        );
    }

    /// 画像表示の描画器で、circlesと波紋を描いたバッファを返す
    fn draw_image_board(
        renderer: &CircleRenderer,
        terminal: &mut Terminal<TestBackend>,
        circles: &[BoardCircle],
        ripple: Option<&Ripple>,
    ) -> ratatui::buffer::Buffer {
        draw_image_board_with_mark(renderer, terminal, circles, ripple, None)
    }

    /// 画像表示の描画器で、circlesと波紋・バツ印を描いたバッファを返す
    fn draw_image_board_with_mark(
        renderer: &CircleRenderer,
        terminal: &mut Terminal<TestBackend>,
        circles: &[BoardCircle],
        ripple: Option<&Ripple>,
        mark: Option<&WrongMark>,
    ) -> ratatui::buffer::Buffer {
        terminal
            .draw(|frame| renderer.render_board(frame, frame.area(), circles, ripple, mark))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    const PATCH_CIRCLES: [BoardCircle; 2] = [
        BoardCircle {
            rect: Rect::new(2, 1, 12, 6),
            number: 3,
            color: [200, 100, 50],
        },
        BoardCircle {
            rect: Rect::new(30, 10, 12, 6),
            number: 4,
            color: [50, 100, 200],
        },
    ];

    #[test]
    fn ripple_animation_reencodes_only_patch_once_per_frame() {
        let renderer = image_renderer();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut ripple = Some(Ripple::new(20, 8, CircleSize::Small));
        let mut frames = std::collections::BTreeSet::new();
        let mut ticks = 0;
        while let Some(r) = ripple {
            frames.insert(r.frame());
            draw_image_board(&renderer, &mut terminal, &PATCH_CIRCLES, ripple.as_ref());
            ripple = r.advanced(crate::TICK_RATE);
            ticks += 1;
        }
        assert_eq!(renderer.board_encode_count(), 1, "盤面全体は最初の1回だけ");
        assert_eq!(
            renderer.ripple_encode_count(),
            frames.len(),
            "パッチはコマが変わった時だけ作り直す"
        );
        assert_eq!(
            frames.len() as u128,
            RIPPLE_DURATION.as_millis() / super::super::ripple::RIPPLE_FRAME_INTERVAL.as_millis()
        );
        assert!(frames.len() < ticks, "毎tickは作り直さない: {ticks}tick");
        // 波紋が消えたら、リングの無いパッチを1回だけ描き直して残りを消す
        for _ in 0..3 {
            draw_image_board(&renderer, &mut terminal, &PATCH_CIRCLES, None);
        }
        assert_eq!(renderer.board_encode_count(), 1);
        assert_eq!(renderer.ripple_encode_count(), frames.len() + 1);
    }

    #[test]
    fn ripple_patch_only_changes_cells_inside_patch() {
        let ripple = Ripple::new(20, 8, CircleSize::Small)
            .advanced(RIPPLE_DURATION / 2)
            .unwrap();
        let rect = ripple_patch_rect(Rect::new(0, 0, 60, 24), (4, 8), &ripple);
        let with = draw_image_board(
            &image_renderer(),
            &mut Terminal::new(TestBackend::new(60, 24)).unwrap(),
            &PATCH_CIRCLES,
            Some(&ripple),
        );
        let without = draw_image_board(
            &image_renderer(),
            &mut Terminal::new(TestBackend::new(60, 24)).unwrap(),
            &PATCH_CIRCLES,
            None,
        );
        let mut changed = 0;
        for y in 0..24 {
            for x in 0..60 {
                if rect.contains(ratatui::layout::Position::new(x, y)) {
                    changed += usize::from(with[(x, y)] != without[(x, y)]);
                } else {
                    assert_eq!(
                        with[(x, y)],
                        without[(x, y)],
                        "パッチの外({x},{y})は変わらない"
                    );
                }
            }
        }
        assert!(changed > 0, "パッチの中にリングが描かれる");
    }

    #[test]
    fn board_change_rebuilds_board_and_patch() {
        let renderer = image_renderer();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let ripple = Ripple::new(20, 8, CircleSize::Small);
        draw_image_board(&renderer, &mut terminal, &PATCH_CIRCLES, Some(&ripple));
        // 円が1つ消えると、同じコマの波紋でも下地が変わるので両方作り直す
        draw_image_board(&renderer, &mut terminal, &PATCH_CIRCLES[..1], Some(&ripple));
        assert_eq!(renderer.board_encode_count(), 2);
        assert_eq!(renderer.ripple_encode_count(), 2);
    }

    // --- 端末へ実際に送られる画像データ(main.rsと同じTerminal+ImageDedupBackend経由) ---

    use crate::image_backend::{ImageDedupBackend, RecordingBackend};

    const PIPE_W: u16 = 60;
    const PIPE_H: u16 = 24;
    const PIPE_FONT: (u16, u16) = (4, 8);
    const PIPE_BOARD: Rect = Rect::new(0, 0, PIPE_W, PIPE_H);

    /// 実際の端末と同じ画像プロトコル(セルにエスケープシーケンスを入れ、残りをskipにする)の描画器
    fn protocol_renderer(protocol: ProtocolType) -> CircleRenderer {
        let mut picker = Picker::from_fontsize(PIPE_FONT);
        picker.set_protocol_type(protocol);
        CircleRenderer::with_picker(picker)
    }

    fn pipeline_terminal() -> Terminal<ImageDedupBackend<RecordingBackend>> {
        Terminal::new(ImageDedupBackend::new(RecordingBackend::new(
            PIPE_W, PIPE_H,
        )))
        .unwrap()
    }

    /// 1フレーム描き、そのフレームで端末へ送られた画像データのセル位置を返す
    fn draw_sent(
        renderer: &CircleRenderer,
        terminal: &mut Terminal<ImageDedupBackend<RecordingBackend>>,
        ripple: Option<&Ripple>,
    ) -> Vec<(u16, u16)> {
        draw_sent_with_mark(renderer, terminal, ripple, None)
    }

    /// 波紋・バツ印を指定して1フレーム描き、端末へ送られた画像データのセル位置を返す
    fn draw_sent_with_mark(
        renderer: &CircleRenderer,
        terminal: &mut Terminal<ImageDedupBackend<RecordingBackend>>,
        ripple: Option<&Ripple>,
        mark: Option<&WrongMark>,
    ) -> Vec<(u16, u16)> {
        terminal
            .draw(|frame| renderer.render_board(frame, frame.area(), &PATCH_CIRCLES, ripple, mark))
            .unwrap();
        terminal.backend().inner().last_payload_positions()
    }

    /// 盤面画像の起点セル(sixel/iTerm2は盤面の左上1セルに画像データを入れる)
    const BOARD_ORIGIN: (u16, u16) = (PIPE_BOARD.x, PIPE_BOARD.y);

    #[test]
    fn patch_origin_cell_is_not_left_skipped_by_board_image() {
        // 原因の固定: 盤面の画像は起点以外の全セルをskip(差分出力の対象外)にする。
        // その内側に重ねたパッチの起点セルがskipのままだと、画像データを入れても端末へ出力されない
        let renderer = protocol_renderer(ProtocolType::Sixel);
        let mut terminal = Terminal::new(TestBackend::new(PIPE_W, PIPE_H)).unwrap();
        let ripple = Ripple::new(20, 8, CircleSize::Small);
        let patch = ripple_patch_rect(PIPE_BOARD, PIPE_FONT, &ripple);
        let completed = terminal
            .draw(|frame| {
                renderer.render_board(frame, frame.area(), &PATCH_CIRCLES, Some(&ripple), None)
            })
            .unwrap();
        let origin = &completed.buffer[(patch.x, patch.y)];
        assert!(
            origin.symbol().starts_with('\x1b'),
            "パッチの画像データが入る"
        );
        assert!(!origin.skip, "パッチの起点セルは出力の対象にする");
        let inner = &completed.buffer[(patch.x + 1, patch.y)];
        assert!(inner.skip, "パッチの起点以外は画像に覆われるのでskipのまま");
    }

    #[test]
    fn ripple_patch_is_sent_to_terminal_on_every_ripple_frame() {
        // 報告された症状の再現: 盤面の画像の内側に重ねたパッチの画像データが端末へ送られず、
        // 波紋が全く表示されない
        for protocol in [
            ProtocolType::Sixel,
            ProtocolType::Iterm2,
            ProtocolType::Kitty,
        ] {
            let renderer = protocol_renderer(protocol);
            let mut terminal = pipeline_terminal();
            let mut ripple = Some(Ripple::new(20, 8, CircleSize::Small));
            let mut last_frame = None;
            while let Some(r) = ripple {
                let patch = ripple_patch_rect(PIPE_BOARD, PIPE_FONT, &r);
                let origin = (patch.x, patch.y);
                let sent = draw_sent(&renderer, &mut terminal, Some(&r));
                if last_frame != Some(r.frame()) {
                    assert!(
                        sent.contains(&origin),
                        "{protocol:?}: コマ{}のパッチ{patch:?}が端末へ送られていない: {sent:?}",
                        r.frame()
                    );
                } else if protocol != ProtocolType::Kitty {
                    // (kittyは画像の転送後に中身の無い配置だけのデータへ変わるため除く)
                    assert!(
                        !sent.contains(&origin),
                        "{protocol:?}: コマが変わらない間はパッチを送り直さない"
                    );
                }
                if protocol != ProtocolType::Kitty && last_frame.is_some() {
                    assert!(
                        !sent.contains(&BOARD_ORIGIN),
                        "{protocol:?}: 盤面が変わらない間は盤面の画像を送り直さない"
                    );
                }
                last_frame = Some(r.frame());
                ripple = r.advanced(crate::TICK_RATE);
            }
        }
    }

    #[test]
    fn only_board_is_sent_without_ripple_and_patch_follows_when_ripple_starts() {
        for protocol in [ProtocolType::Sixel, ProtocolType::Iterm2] {
            let renderer = protocol_renderer(protocol);
            let mut terminal = pipeline_terminal();
            let sent = draw_sent(&renderer, &mut terminal, None);
            assert_eq!(
                sent,
                vec![BOARD_ORIGIN],
                "{protocol:?}: 波紋が無ければ盤面だけ"
            );
            assert!(draw_sent(&renderer, &mut terminal, None).is_empty());
            let ripple = Ripple::new(20, 8, CircleSize::Small);
            let patch = ripple_patch_rect(PIPE_BOARD, PIPE_FONT, &ripple);
            let sent = draw_sent(&renderer, &mut terminal, Some(&ripple));
            assert_eq!(
                sent,
                vec![(patch.x, patch.y)],
                "{protocol:?}: 波紋が始まるとパッチだけを送る"
            );
        }
    }

    #[test]
    fn clean_patch_is_sent_once_after_ripple_ends() {
        // 波紋が消えたら、端末に残った最後のリングを消すため、リングの無いパッチを1回送る
        let renderer = protocol_renderer(ProtocolType::Sixel);
        let mut terminal = pipeline_terminal();
        let ripple = Ripple::new(20, 8, CircleSize::Small);
        let patch = ripple_patch_rect(PIPE_BOARD, PIPE_FONT, &ripple);
        draw_sent(&renderer, &mut terminal, Some(&ripple));
        assert_eq!(
            draw_sent(&renderer, &mut terminal, None),
            vec![(patch.x, patch.y)]
        );
        assert!(draw_sent(&renderer, &mut terminal, None).is_empty());
    }

    #[test]
    fn ripple_at_board_corner_is_sent_to_terminal() {
        // 盤面の左上の隅をクリックした波紋。パッチが盤面画像の起点セルのすぐ右隣から始まると、
        // ratatuiの差分処理が起点セルの画像データの表示幅ぶん後続セルの出力を飛ばすため送られない
        for protocol in [
            ProtocolType::Sixel,
            ProtocolType::Iterm2,
            ProtocolType::Kitty,
        ] {
            for center in [(0, 0), (1, 0), (2, 1), (0, 5)] {
                let renderer = protocol_renderer(protocol);
                let mut terminal = pipeline_terminal();
                draw_sent(&renderer, &mut terminal, None);
                let ripple = Ripple::new(center.0, center.1, CircleSize::Small);
                let patch = ripple_patch_rect(PIPE_BOARD, PIPE_FONT, &ripple);
                assert!(!patch.is_empty());
                let sent = draw_sent(&renderer, &mut terminal, Some(&ripple));
                assert!(
                    sent.contains(&(patch.x, patch.y)),
                    "{protocol:?}: 中心{center:?}のパッチ{patch:?}が送られていない: {sent:?}"
                );
            }
        }
    }

    #[test]
    fn fallback_ignores_ripple() {
        // テキスト表示では波紋を描かない(波紋があっても無くても同じ画面)
        let circle = BoardCircle {
            rect: Rect::new(4, 2, 8, 4),
            number: 5,
            color: [1, 2, 3],
        };
        let draw = |ripple: Option<&Ripple>| {
            let renderer = CircleRenderer::new();
            let mut terminal = Terminal::new(TestBackend::new(30, 10)).unwrap();
            terminal
                .draw(|frame| renderer.render_board(frame, frame.area(), &[circle], ripple, None))
                .unwrap();
            terminal.backend().buffer().clone()
        };
        let ripple = half_way_ripple();
        assert_eq!(draw(Some(&ripple)), draw(None));
    }

    // --- 誤クリックのバツ印 ---

    #[test]
    fn fallback_draws_red_cross_symbol_at_mark_cell() {
        // テキスト表示でも、クリックしたセルに赤い✗を円の手前に重ねて描く
        let circle = BoardCircle {
            rect: Rect::new(4, 2, 8, 4),
            number: 5,
            color: [1, 2, 3],
        };
        let draw = |mark: Option<&WrongMark>| {
            let renderer = CircleRenderer::new();
            let mut terminal = Terminal::new(TestBackend::new(30, 10)).unwrap();
            terminal
                .draw(|frame| renderer.render_board(frame, frame.area(), &[circle], None, mark))
                .unwrap();
            terminal.backend().buffer().clone()
        };
        let mark = WrongMark::new(5, 3);
        let with = draw(Some(&mark));
        let [r, g, b] = WRONG_MARK_COLOR;
        assert_eq!(with[(5, 3)].symbol(), "✗", "クリックしたセルに✗");
        assert_eq!(with[(5, 3)].fg, Color::Rgb(r, g, b), "赤系の色");
        let without = draw(None);
        assert_ne!(without[(5, 3)].symbol(), "✗");
        for y in 0..10 {
            for x in 0..30 {
                if (x, y) != (5, 3) {
                    assert_eq!(
                        with[(x, y)],
                        without[(x, y)],
                        "他のセル({x},{y})は変わらない"
                    );
                }
            }
        }
        // ボードの外のバツ印は描かない(パニックしない)
        let outside = WrongMark::new(100, 100);
        draw(Some(&outside));
    }

    #[test]
    fn mark_patch_has_red_cross_over_board_image() {
        let mark = WrongMark::new(10, 5);
        let big = BoardCircle {
            rect: RIPPLE_BOARD,
            number: 2,
            color: [0, 0, 255],
        };
        let images = recolored_images(&[big]).unwrap();
        let circles_only = compose_circles(RIPPLE_BOARD, RIPPLE_FONT, &[big], &images);
        let rect = mark_patch_rect(RIPPLE_BOARD, RIPPLE_FONT, &mark);
        assert!(!rect.is_empty());
        assert_eq!(
            rect.intersection(RIPPLE_BOARD),
            rect,
            "ボードからはみ出さない"
        );
        assert!(
            rect.contains(ratatui::layout::Position::new(10, 5)),
            "クリックしたセルを含む: {rect:?}"
        );
        let patch = effects_patch(
            &circles_only,
            RIPPLE_BOARD,
            RIPPLE_FONT,
            rect,
            None,
            Some(&mark),
            &[],
        );
        // セル(10, 5)の中心のピクセル = ボード画像の(105, 110)。パッチの中での位置に直す
        let (cw, ch) = (u32::from(RIPPLE_FONT.0), u32::from(RIPPLE_FONT.1));
        let center = (105 - u32::from(rect.x) * cw, 110 - u32::from(rect.y) * ch);
        let [r, g, b] = WRONG_MARK_COLOR;
        assert_eq!(
            patch.get_pixel(center.0, center.1).0,
            [r, g, b, 255],
            "バツ印の交点は赤"
        );
        let clean = crop_cells(&circles_only, RIPPLE_BOARD, RIPPLE_FONT, rect);
        assert_ne!(patch, clean, "バツ印が重なっている");
        // バツ印が無ければ、盤面の切り出しそのまま
        assert_eq!(
            effects_patch(
                &circles_only,
                RIPPLE_BOARD,
                RIPPLE_FONT,
                rect,
                None,
                None,
                &[]
            ),
            clean
        );
    }

    #[test]
    fn mark_patch_covers_whole_cross() {
        let mark = WrongMark::new(10, 5);
        let rect = mark_patch_rect(RIPPLE_BOARD, RIPPLE_FONT, &mark);
        let board_image = RgbaImage::new(200, 200);
        let full = effects_patch(
            &board_image,
            RIPPLE_BOARD,
            RIPPLE_FONT,
            RIPPLE_BOARD,
            None,
            Some(&mark),
            &[],
        );
        let (cw, ch) = (u32::from(RIPPLE_FONT.0), u32::from(RIPPLE_FONT.1));
        let painted: Vec<(u32, u32)> = full
            .enumerate_pixels()
            .filter(|(_, _, p)| p.0[3] > 0)
            .map(|(x, y, _)| (x, y))
            .collect();
        assert!(!painted.is_empty());
        for (x, y) in painted {
            assert!(
                (u32::from(rect.x) * cw..u32::from(rect.right()) * cw).contains(&x)
                    && (u32::from(rect.y) * ch..u32::from(rect.bottom()) * ch).contains(&y),
                "バツ印の({x},{y})がパッチ{rect:?}の外にある"
            );
        }
    }

    #[test]
    fn static_mark_patch_is_encoded_once_and_board_is_kept() {
        let renderer = image_renderer();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut mark = Some(WrongMark::new(20, 8));
        while let Some(m) = mark {
            draw_image_board_with_mark(&renderer, &mut terminal, &PATCH_CIRCLES, None, Some(&m));
            mark = m.advanced(crate::TICK_RATE);
        }
        assert_eq!(renderer.board_encode_count(), 1, "盤面全体は最初の1回だけ");
        assert_eq!(
            renderer.ripple_encode_count(),
            1,
            "バツ印は動かないので、表示中のパッチは1回だけ作る"
        );
        // バツ印が消えたら、バツ印の無いパッチを1回だけ描き直して残りを消す
        for _ in 0..3 {
            draw_image_board_with_mark(&renderer, &mut terminal, &PATCH_CIRCLES, None, None);
        }
        assert_eq!(renderer.board_encode_count(), 1);
        assert_eq!(renderer.ripple_encode_count(), 2);
    }

    #[test]
    fn mark_patch_is_sent_to_terminal_and_cleared_after_mark_ends() {
        for protocol in [ProtocolType::Sixel, ProtocolType::Iterm2] {
            let renderer = protocol_renderer(protocol);
            let mut terminal = pipeline_terminal();
            draw_sent(&renderer, &mut terminal, None);
            let mark = WrongMark::new(20, 8);
            let patch = mark_patch_rect(PIPE_BOARD, PIPE_FONT, &mark);
            assert!(!patch.is_empty());
            assert_eq!(
                draw_sent_with_mark(&renderer, &mut terminal, None, Some(&mark)),
                vec![(patch.x, patch.y)],
                "{protocol:?}: バツ印が出るとパッチだけを送る"
            );
            assert!(
                draw_sent_with_mark(&renderer, &mut terminal, None, Some(&mark)).is_empty(),
                "{protocol:?}: バツ印が変わらない間は送り直さない"
            );
            assert_eq!(
                draw_sent(&renderer, &mut terminal, None),
                vec![(patch.x, patch.y)],
                "{protocol:?}: 消えたらバツ印の無いパッチを1回送る"
            );
            assert!(draw_sent(&renderer, &mut terminal, None).is_empty());
        }
    }

    #[test]
    fn moving_mark_redraws_previous_position_too() {
        // 別の場所を押し間違えた時は、前のバツ印を端末に残さないよう前の範囲も含めて描き直す
        let renderer = image_renderer();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let first = WrongMark::new(20, 8);
        let second = WrongMark::new(40, 15);
        draw_image_board_with_mark(&renderer, &mut terminal, &PATCH_CIRCLES, None, Some(&first));
        let first_rect = renderer.patch_rect().expect("パッチを描いている");
        draw_image_board_with_mark(
            &renderer,
            &mut terminal,
            &PATCH_CIRCLES,
            None,
            Some(&second),
        );
        let rect = renderer.patch_rect().unwrap();
        assert_eq!(rect.union(first_rect), rect, "前のバツ印の範囲も含む");
        assert!(rect.contains(ratatui::layout::Position::new(40, 15)));
    }

    #[test]
    fn fallback_renders_colored_circled_digit_in_rect() {
        // テスト環境(非TTY)では画像プロトコルを検出できず、テキスト表示になる
        let renderer = CircleRenderer::new();
        assert!(!renderer.uses_image());
        let backend = TestBackend::new(30, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let rect = Rect::new(4, 2, 8, 4);
        let circle = BoardCircle {
            rect,
            number: 12,
            color: [1, 2, 3],
        };
        terminal
            .draw(|frame| renderer.render_board(frame, frame.area(), &[circle], None, None))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let cell = (rect.x..rect.right())
            .flat_map(|x| (rect.y..rect.bottom()).map(move |y| (x, y)))
            .map(|pos| &buffer[pos])
            .find(|c| c.symbol() == "⑫")
            .expect("rect内に⑫が描かれること");
        assert_eq!(cell.fg, Color::Rgb(1, 2, 3));
        // rectの外には何も描かない
        let outside: String = (0..30).map(|x| buffer[(x, 0)].symbol()).collect();
        assert_eq!(outside.trim(), "");
    }

    // --- 水槽の魚(画像表示) ---

    use super::super::fish::{FISH_HEIGHT, FISH_WIDTH};
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    const FISH_RED: [u8; 3] = [255, 0, 0];

    /// 左上のセル(x, y)にいる、右向きの赤い魚
    fn fish_at(x: u16, y: u16) -> BoardFish {
        BoardFish {
            x,
            y,
            facing_right: true,
            color: FISH_RED,
        }
    }

    fn fish_cells(fish: &BoardFish) -> Rect {
        Rect::new(fish.x, fish.y, FISH_WIDTH, FISH_HEIGHT)
    }

    /// ボード画像のピクセル(x, y)が、セル範囲rectに入るか
    fn pixel_in_cells(board: Rect, font: (u16, u16), rect: Rect, x: u32, y: u32) -> bool {
        let (cw, ch) = (u32::from(font.0), u32::from(font.1));
        let left = u32::from(rect.x - board.x) * cw;
        let top = u32::from(rect.y - board.y) * ch;
        (left..left + u32::from(rect.width) * cw).contains(&x)
            && (top..top + u32::from(rect.height) * ch).contains(&y)
    }

    /// ボード画像のうち、セル範囲rectにあって不透明な、RGBがrgbのピクセルの数
    fn count_rgb_in_cells(
        image: &RgbaImage,
        board: Rect,
        font: (u16, u16),
        rect: Rect,
        rgb: [u8; 3],
    ) -> usize {
        image
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                p.0[3] > 0 && p.0[..3] == rgb && pixel_in_cells(board, font, rect, *x, *y)
            })
            .count()
    }

    #[test]
    fn scene_draws_fish_in_its_cells_with_its_color() {
        let circles_only = RgbaImage::new(200, 200);
        let fish = fish_at(12, 7);
        let scene = compose_scene(&circles_only, RIPPLE_BOARD, RIPPLE_FONT, &[fish]);
        assert_eq!(scene.dimensions(), (200, 200));
        let cells = fish_cells(&fish);
        assert!(
            count_rgb_in_cells(&scene, RIPPLE_BOARD, RIPPLE_FONT, cells, FISH_RED) > 20,
            "体が魚の色で描かれる"
        );
        assert!(
            count_rgb_in_cells(&scene, RIPPLE_BOARD, RIPPLE_FONT, cells, [0, 0, 0]) > 0,
            "目は黒のまま"
        );
        for (x, y, p) in scene.enumerate_pixels() {
            if !pixel_in_cells(RIPPLE_BOARD, RIPPLE_FONT, cells, x, y) {
                assert_eq!(p.0[3], 0, "魚のセルの外({x},{y})は透明");
            }
        }
        // 左向きの魚は左右反転した絵になる
        let left = BoardFish {
            facing_right: false,
            ..fish
        };
        let flipped = compose_scene(&circles_only, RIPPLE_BOARD, RIPPLE_FONT, &[left]);
        assert_ne!(scene, flipped);
    }

    #[test]
    fn circles_are_drawn_in_front_of_fish() {
        // ボード全体を覆う大きな円。中央の魚は円に隠れ、円の外(ボードの角)にいる魚は見える
        let big = BoardCircle {
            rect: RIPPLE_BOARD,
            number: 2,
            color: [0, 0, 255],
        };
        let images = recolored_images(&[big]).unwrap();
        let circles_only = compose_circles(RIPPLE_BOARD, RIPPLE_FONT, &[big], &images);
        let hidden = fish_at(8, 4);
        let corner = fish_at(17, 0);
        let scene = compose_scene(&circles_only, RIPPLE_BOARD, RIPPLE_FONT, &[hidden, corner]);
        for (x, y, p) in circles_only.enumerate_pixels() {
            if p.0[3] == 255 {
                assert_eq!(
                    scene.get_pixel(x, y),
                    p,
                    "円の不透明な所({x},{y})は円のまま"
                );
            }
        }
        assert_eq!(
            count_rgb_in_cells(
                &scene,
                RIPPLE_BOARD,
                RIPPLE_FONT,
                fish_cells(&hidden),
                FISH_RED
            ),
            0,
            "円の後ろの魚は見えない"
        );
        assert!(
            count_rgb_in_cells(
                &scene,
                RIPPLE_BOARD,
                RIPPLE_FONT,
                fish_cells(&corner),
                FISH_RED
            ) > 0,
            "円の外の魚は見える"
        );
    }

    #[test]
    fn fish_is_not_drawn_in_two_left_columns_of_board() {
        // 左端の2列はパッチで描き直せない(patch_rect_aroundを参照)ので、魚も描かない。
        // 描くと、魚が離れた後も消せずに残る
        let board = Rect::new(2, 3, 20, 10);
        let circles_only = RgbaImage::new(200, 200);
        let scene = compose_scene(&circles_only, board, RIPPLE_FONT, &[fish_at(2, 5)]);
        for (x, y, p) in scene.enumerate_pixels() {
            if x < 2 * u32::from(RIPPLE_FONT.0) {
                assert_eq!(p.0[3], 0, "左端の2列({x},{y})には描かない");
            }
        }
        assert!(
            count_rgb_in_cells(&scene, board, RIPPLE_FONT, Rect::new(4, 5, 1, 1), FISH_RED) > 0,
            "3列目からは描く"
        );
    }

    #[test]
    fn fish_patch_shows_fish_at_new_cells_and_board_at_old_cells() {
        // 魚が(5,4)から(6,4)へ動いた。パッチは前の位置と今の位置を合わせた範囲を、
        // 今の魚で描き直す(前の位置に魚の絵を残さない)
        let circles_only = RgbaImage::new(200, 200);
        let rect = Rect::new(5, 4, 4, 1);
        // 範囲の端に一部だけ入っている別の魚も、はみ出した分を切って描く
        let fish = [fish_at(6, 4), fish_at(8, 4)];
        let patch = effects_patch(
            &circles_only,
            RIPPLE_BOARD,
            RIPPLE_FONT,
            rect,
            None,
            None,
            &fish,
        );
        let scene = compose_scene(&circles_only, RIPPLE_BOARD, RIPPLE_FONT, &fish);
        assert_eq!(patch, crop_cells(&scene, RIPPLE_BOARD, RIPPLE_FONT, rect));
        let old_cell = imageops::crop_imm(&patch, 0, 0, 10, 20).to_image();
        assert!(
            old_cell.pixels().all(|p| p.0[3] == 0),
            "前の位置の左端のセルには何も残らない"
        );
    }

    fn item(x: u16, y: u16, width: u16, height: u16) -> PatchItem {
        PatchItem {
            rect: Rect::new(x, y, width, height),
            effects: false,
        }
    }

    fn merged_rects(items: &[PatchItem]) -> Vec<Rect> {
        let mut rects: Vec<Rect> = merge_patch_items(items)
            .into_iter()
            .map(|item| item.rect)
            .collect();
        rects.sort_by_key(|rect| (rect.y, rect.x));
        rects
    }

    #[test]
    fn merge_keeps_far_patches_apart_and_joins_overlapping_or_touching_ones() {
        assert_eq!(
            merged_rects(&[item(5, 2, 3, 1), item(20, 2, 3, 1)]),
            vec![Rect::new(5, 2, 3, 1), Rect::new(20, 2, 3, 1)],
            "離れたパッチは別々"
        );
        assert_eq!(
            merged_rects(&[item(5, 2, 4, 2), item(7, 3, 4, 1)]),
            vec![Rect::new(5, 2, 6, 2)],
            "重なるパッチは1枚にまとめる"
        );
        // 左右に隣り合うと、左のパッチの画像データのセルの直後に右のパッチの起点が来ることがあり、
        // ratatuiの差分処理がそのセルを出力しない。1枚にまとめる
        assert_eq!(
            merged_rects(&[item(5, 2, 1, 1), item(6, 2, 3, 1)]),
            vec![Rect::new(5, 2, 4, 1)]
        );
        assert_eq!(
            merged_rects(&[item(5, 2, 3, 1), item(5, 3, 3, 1)]),
            vec![Rect::new(5, 2, 3, 1), Rect::new(5, 3, 3, 1)],
            "上下に隣り合うだけなら別々"
        );
        // AとCは重ならないが、AとBをまとめた範囲にCが入るので全部1枚にする
        assert_eq!(
            merged_rects(&[item(0, 0, 4, 1), item(0, 3, 1, 1), item(2, 0, 1, 4)]),
            vec![Rect::new(0, 0, 4, 4)]
        );
        // 演出(波紋・バツ印)の印はまとめた先に引き継ぐ
        let effects = PatchItem {
            rect: Rect::new(5, 2, 3, 1),
            effects: true,
        };
        assert_eq!(
            merge_patch_items(&[effects, item(7, 2, 3, 1)]),
            vec![PatchItem {
                rect: Rect::new(5, 2, 5, 1),
                effects: true,
            }]
        );
    }

    /// 画像表示の描画器で、circles・演出・魚を描いたバッファを返す
    fn draw_scene(
        renderer: &CircleRenderer,
        terminal: &mut Terminal<TestBackend>,
        circles: &[BoardCircle],
        ripple: Option<&Ripple>,
        mark: Option<&WrongMark>,
        fish: &[BoardFish],
    ) {
        terminal
            .draw(|frame| {
                renderer.render_board_with_fish(frame, frame.area(), circles, ripple, mark, fish)
            })
            .unwrap();
    }

    #[test]
    fn moving_fish_reencodes_only_patch_covering_old_and_new_cells() {
        let renderer = image_renderer();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut draw = |fish: BoardFish| {
            draw_scene(
                &renderer,
                &mut terminal,
                &PATCH_CIRCLES,
                None,
                None,
                &[fish],
            )
        };
        draw(fish_at(20, 8));
        assert_eq!(renderer.board_encode_count(), 1);
        assert_eq!(
            renderer.ripple_encode_count(),
            0,
            "最初の位置の魚は盤面の画像に描く"
        );
        assert!(renderer.patch_rects().is_empty());

        draw(fish_at(21, 8));
        assert_eq!(
            renderer.board_encode_count(),
            1,
            "魚が動いても盤面全体は作り直さない"
        );
        assert_eq!(renderer.ripple_encode_count(), 1);
        assert_eq!(
            renderer.patch_rects(),
            vec![Rect::new(20, 8, 4, 1)],
            "前の位置と今の位置を合わせた範囲"
        );

        draw(fish_at(21, 8));
        assert_eq!(
            renderer.ripple_encode_count(),
            1,
            "止まっている間は作り直さない"
        );
        assert!(renderer.patch_rects().is_empty());

        // 向きだけ変わっても描き直す
        draw(BoardFish {
            facing_right: false,
            ..fish_at(21, 8)
        });
        assert_eq!(renderer.ripple_encode_count(), 2);
        assert_eq!(renderer.patch_rects(), vec![Rect::new(21, 8, 3, 1)]);
    }

    #[test]
    fn board_rebuild_draws_fish_into_board_image() {
        let renderer = image_renderer();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        draw_scene(
            &renderer,
            &mut terminal,
            &PATCH_CIRCLES,
            None,
            None,
            &[fish_at(20, 8)],
        );
        draw_scene(
            &renderer,
            &mut terminal,
            &PATCH_CIRCLES,
            None,
            None,
            &[fish_at(21, 8)],
        );
        assert_eq!(renderer.ripple_encode_count(), 1);
        // 円が1つ消えて盤面を作り直す時は、いまの位置の魚も盤面の画像に描くのでパッチは要らない
        draw_scene(
            &renderer,
            &mut terminal,
            &PATCH_CIRCLES[..1],
            None,
            None,
            &[fish_at(21, 8)],
        );
        assert_eq!(renderer.board_encode_count(), 2);
        assert_eq!(renderer.ripple_encode_count(), 1);
        assert!(renderer.patch_rects().is_empty());
        // その後に動いたら、盤面の画像に描いた位置から描き直す
        draw_scene(
            &renderer,
            &mut terminal,
            &PATCH_CIRCLES[..1],
            None,
            None,
            &[fish_at(22, 8)],
        );
        assert_eq!(renderer.patch_rects(), vec![Rect::new(21, 8, 4, 1)]);
    }

    #[test]
    fn fish_near_ripple_is_drawn_in_same_patch() {
        // パッチ同士が重なると後に描いた方が先の方の画像データを上書きするため、1枚にまとめる
        let renderer = image_renderer();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let ripple = Ripple::new(20, 8, CircleSize::Small);
        draw_scene(
            &renderer,
            &mut terminal,
            &PATCH_CIRCLES,
            Some(&ripple),
            None,
            &[fish_at(21, 8)],
        );
        let effects_only = renderer.patch_rect().expect("波紋のパッチ");
        draw_scene(
            &renderer,
            &mut terminal,
            &PATCH_CIRCLES,
            Some(&ripple),
            None,
            &[fish_at(22, 8)],
        );
        let rects = renderer.patch_rects();
        assert_eq!(rects.len(), 1, "波紋と魚は1枚のパッチ: {rects:?}");
        assert_eq!(rects[0].union(effects_only), rects[0]);
        assert_eq!(
            rects[0].union(Rect::new(21, 8, 4, 1)),
            rects[0],
            "魚の前と今の位置も含む"
        );
        assert_eq!(renderer.patch_rect(), Some(rects[0]));
    }

    #[test]
    fn after_effects_end_fish_patch_does_not_grow_to_old_effects_area() {
        // 演出が消えたらリングの無い切り出しを1回だけ描き、その範囲は手放す。
        // 手放さないと、跡を泳ぐ魚が動くたびに演出の範囲全体(特大の円の波紋では盤面の数割)を作り直す
        let renderer = image_renderer();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let ripple = Ripple::new(20, 8, CircleSize::Huge);
        let still = [fish_at(20, 8)];
        draw_scene(
            &renderer,
            &mut terminal,
            &PATCH_CIRCLES,
            Some(&ripple),
            None,
            &still,
        );
        let effects = renderer.patch_rect().expect("波紋のパッチ");
        draw_scene(&renderer, &mut terminal, &PATCH_CIRCLES, None, None, &still);
        assert_eq!(
            renderer.patch_rects(),
            vec![effects],
            "消えた直後はリングの無い切り出しを描く"
        );
        draw_scene(&renderer, &mut terminal, &PATCH_CIRCLES, None, None, &still);
        assert!(renderer.patch_rects().is_empty(), "その後は描かない");
        draw_scene(
            &renderer,
            &mut terminal,
            &PATCH_CIRCLES,
            None,
            None,
            &[fish_at(21, 8)],
        );
        assert_eq!(
            renderer.patch_rects(),
            vec![Rect::new(20, 8, 4, 1)],
            "波紋の跡を泳ぐ魚は魚の周りだけ描き直す"
        );
    }

    #[test]
    fn patches_in_one_frame_never_overlap_or_touch() {
        // 魚3匹が泳ぎ回り、時々波紋・バツ印が出る。どのフレームでもパッチは重ならず、
        // 左右にも隣り合わず、盤面の左端の2列にかからない
        let renderer = image_renderer();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut rng = StdRng::seed_from_u64(11);
        let mut fish: Vec<BoardFish> = (0..3).map(|i| fish_at(5 + i * 15, 3 + i * 6)).collect();
        let mut ripple: Option<Ripple> = None;
        let mut mark: Option<WrongMark> = None;
        for step in 0..300 {
            for f in &mut fish {
                f.x = (i32::from(f.x) + rng.gen_range(-1..=1)).clamp(0, 57) as u16;
                f.y = (i32::from(f.y) + rng.gen_range(-1..=1)).clamp(0, 23) as u16;
                f.facing_right = rng.gen_bool(0.5);
            }
            if step % 40 == 0 {
                ripple = Some(Ripple::new(
                    rng.gen_range(0..60),
                    rng.gen_range(0..24),
                    CircleSize::Small,
                ));
            }
            if step % 55 == 0 {
                mark = Some(WrongMark::new(rng.gen_range(0..60), rng.gen_range(0..24)));
            }
            draw_scene(
                &renderer,
                &mut terminal,
                &PATCH_CIRCLES,
                ripple.as_ref(),
                mark.as_ref(),
                &fish,
            );
            ripple = ripple.and_then(|r| r.advanced(crate::TICK_RATE));
            mark = mark.and_then(|m| m.advanced(crate::TICK_RATE));
            let rects = renderer.patch_rects();
            for (i, a) in rects.iter().enumerate() {
                assert!(a.x >= 2, "step{step}: 左端の2列にかかる {a:?}");
                assert_eq!(a.intersection(Rect::new(0, 0, 60, 24)), *a);
                for b in &rects[i + 1..] {
                    let widened = |r: &Rect| Rect::new(r.x, r.y, r.width + 1, r.height);
                    assert!(
                        !widened(a).intersects(*b) && !widened(b).intersects(*a),
                        "step{step}: パッチ{a:?}と{b:?}が重なる/隣り合う"
                    );
                }
            }
        }
        assert_eq!(renderer.board_encode_count(), 1, "盤面全体は最初の1回だけ");
    }

    #[test]
    fn image_mode_render_with_fish_does_not_panic_on_edges_and_tiny_areas() {
        for (width, height) in [(60, 24), (5, 3), (3, 1), (2, 2), (1, 1)] {
            let renderer = image_renderer();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let circle = BoardCircle {
                rect: Rect::new(0, 0, width, height),
                number: 1,
                color: [10, 20, 30],
            };
            let ripple = Ripple::new(0, 0, CircleSize::Small);
            let mark = WrongMark::new(width - 1, height - 1);
            for fish in [
                fish_at(0, 0),
                fish_at(1, 0),
                fish_at(width - 1, height - 1),
                fish_at(width + 10, height + 10),
                fish_at(u16::MAX - 1, u16::MAX - 1),
            ] {
                draw_scene(
                    &renderer,
                    &mut terminal,
                    &[circle],
                    Some(&ripple),
                    Some(&mark),
                    &[fish],
                );
                draw_scene(&renderer, &mut terminal, &[], None, None, &[fish]);
            }
        }
    }

    // --- 魚のパッチが端末へ実際に送られること ---

    /// 魚を描いて1フレーム描き、端末へ送られた画像データのセル位置を返す
    fn draw_sent_with_fish(
        renderer: &CircleRenderer,
        terminal: &mut Terminal<ImageDedupBackend<RecordingBackend>>,
        ripple: Option<&Ripple>,
        fish: &[BoardFish],
    ) -> Vec<(u16, u16)> {
        terminal
            .draw(|frame| {
                renderer.render_board_with_fish(
                    frame,
                    frame.area(),
                    &PATCH_CIRCLES,
                    ripple,
                    None,
                    fish,
                )
            })
            .unwrap();
        terminal.backend().inner().last_payload_positions()
    }

    /// パッチの画像データ(盤面の左端2列より右にあるもの)が送られたか
    fn patch_was_sent(sent: &[(u16, u16)]) -> bool {
        sent.iter().any(|&(x, _)| x >= PIPE_BOARD.x + 2)
    }

    #[test]
    fn moving_fish_is_sent_to_terminal_every_time_it_moves() {
        // 2回目に(20,8)から(21,8)へ動いた時のパッチは、1回目と同じ起点・同じ中身の画像になる。
        // 端末では間に別の起点のパッチで上書きされているが、ImageDedupBackendは同じ位置へ同じ
        // 画像データを送り直さないため、そのままでは(21,8)の魚が描かれず、古い絵が残る
        let path = [(20, 8), (21, 8), (20, 7), (20, 8), (21, 8), (22, 8)];
        for protocol in [
            ProtocolType::Sixel,
            ProtocolType::Iterm2,
            ProtocolType::Kitty,
        ] {
            let renderer = protocol_renderer(protocol);
            let mut terminal = pipeline_terminal();
            let (x, y) = path[0];
            draw_sent_with_fish(&renderer, &mut terminal, None, &[fish_at(x, y)]);
            for window in path.windows(2) {
                let (x, y) = window[1];
                let sent = draw_sent_with_fish(&renderer, &mut terminal, None, &[fish_at(x, y)]);
                assert!(
                    patch_was_sent(&sent),
                    "{protocol:?}: {:?}から{:?}へ動いたパッチが送られていない: {sent:?}",
                    window[0],
                    window[1]
                );
                if protocol != ProtocolType::Kitty {
                    assert!(
                        !sent.contains(&BOARD_ORIGIN),
                        "{protocol:?}: 魚が動いても盤面の画像は送り直さない"
                    );
                }
            }
            if protocol != ProtocolType::Kitty {
                let (x, y) = path[path.len() - 1];
                assert!(
                    draw_sent_with_fish(&renderer, &mut terminal, None, &[fish_at(x, y)])
                        .is_empty(),
                    "{protocol:?}: 止まっている間は何も送らない"
                );
            }
        }
    }

    #[test]
    fn wandering_fish_are_sent_on_every_move_even_with_ripples() {
        for protocol in [ProtocolType::Sixel, ProtocolType::Iterm2] {
            let renderer = protocol_renderer(protocol);
            let mut terminal = pipeline_terminal();
            let mut rng = StdRng::seed_from_u64(21);
            let mut fish: Vec<BoardFish> =
                (0..3).map(|i| fish_at(10 + i * 12, 4 + i * 5)).collect();
            let mut ripple: Option<Ripple> = None;
            draw_sent_with_fish(&renderer, &mut terminal, None, &fish);
            for step in 0..300 {
                let before = fish.clone();
                for f in &mut fish {
                    // 小さな範囲を行き来させ、同じ位置の組み合わせが何度も出るようにする
                    if rng.gen_bool(0.5) {
                        f.x = (i32::from(f.x) + rng.gen_range(-1..=1)).clamp(8, 44) as u16;
                        f.y = (i32::from(f.y) + rng.gen_range(-1..=1)).clamp(2, 20) as u16;
                    }
                }
                if step % 60 == 0 {
                    ripple = Some(Ripple::new(24, 10, CircleSize::Small));
                }
                let sent = draw_sent_with_fish(&renderer, &mut terminal, ripple.as_ref(), &fish);
                ripple = ripple.and_then(|r| r.advanced(crate::TICK_RATE));
                if fish != before {
                    assert!(
                        patch_was_sent(&sent),
                        "{protocol:?} step{step}: 魚が動いたのにパッチが送られていない {before:?} -> {fish:?}"
                    );
                }
            }
        }
    }
}
