//! カウントマニアの円の配置とクリック当たり判定。
//!
//! 円の大きさは「端末上で占めるセル数」で表す。端末のセルは縦長(横:縦 ≒ 1:2)なので、
//! 幅を高さの2倍にすると見た目がほぼ正円になる。配置はプレイエリアをセルのグリッドとして扱い、
//! 円どうしが重なり合うようにランダムに置く。大きい円ほど奥、小さい円ほど手前に描く。
//! どの円も数字が読めてクリックできるよう、手前の円が奥の円の数字の範囲(protected_area)を
//! 覆う位置には置かない。

use std::cmp::Reverse;

use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::Rect;

/// 円のサイズ段階
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CircleSize {
    /// 大よりさらに一回り大きい円
    Huge,
    Large,
    Medium,
    Small,
}

/// 1つの円の配置(セル単位の矩形)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub number: u8,
    pub rect: Rect,
}

/// サイズ段階ごとの高さ(セル数)の候補。大きい組から順に試し、プレイエリアに収まらなければ
/// 1段小さい組に落とす。各組は[特大, 大, 中, 小]の高さで、どの組でも 特大 > 大 > 中 > 小 を保ち、
/// サイズ段階の違いが見えるようにする。幅は高さの2倍にする。
/// 特大は大との差がはっきり見えるよう、大きい端末で使う組ほど大きく離している
const SIZE_TIERS: [[u16; 4]; 5] = [
    [12, 7, 5, 3],
    [10, 6, 4, 3],
    [9, 5, 4, 2],
    [6, 4, 3, 2],
    [4, 3, 2, 1],
];

/// 1つの組・配置領域で配置をやり直す回数(ランダムな置き方の運で失敗することがあるため)
const ATTEMPTS_PER_TIER: usize = 3;

/// 円1つを置く時に、全ての位置を調べる前にランダムな位置を試す回数
const RANDOM_POSITION_TRIES: usize = 64;

/// 円の面積の合計が配置領域の面積に占めてよい割合の上限。重なりを許すので1を超えてもよい。
/// 超える場合は円を1段小さい組に落とす(重なりすぎて数字の範囲を避けて置けなくなるのを防ぐ)
const LOOSE_MAX_COVERAGE: f64 = 0.8;
const DENSE_MAX_COVERAGE: f64 = 1.4;

/// 密集配置のとき、円を寄せるプレイエリア中央の領域の割合(幅・高さそれぞれ)。
/// 狭くしすぎると画面の中央だけに固まって窮屈に見えるため、画面の大部分を使う広さにする。
/// 密集配置は円の数が多く、面積の上限(DENSE_MAX_COVERAGE)も高いので、この広さでも重なりは強くなる
const DENSE_REGION_RATIO: f64 = 0.9;

/// 数字の範囲(手前の円に覆わせない範囲)の、円の幅・高さに対する割合。
/// 円の画像の2桁の数字は幅約42%・高さ約30%を占めるので、少し余裕を持たせている
const DIGIT_WIDTH_RATIO: f64 = 0.46;
const DIGIT_HEIGHT_RATIO: f64 = 0.4;

/// 指定の組(tier)でのサイズ段階ごとのセル数(幅, 高さ)
pub fn size_dims(tier: usize, size: CircleSize) -> (u16, u16) {
    let heights = SIZE_TIERS[tier.min(SIZE_TIERS.len() - 1)];
    let height = match size {
        CircleSize::Huge => heights[0],
        CircleSize::Large => heights[1],
        CircleSize::Medium => heights[2],
        CircleSize::Small => heights[3],
    };
    (height * 2, height)
}

/// 円(番号, サイズ段階)をarea内にランダムに配置する。円どうしの重なりは許すが、
/// 手前の円が奥の円の数字の範囲を覆うことはない。
/// 戻り値は描画順(奥→手前。面積の大きい円が先)に並ぶ。
/// denseがtrueならプレイエリア中央の狭い領域に寄せ、重なりを強くする。
/// 円の面積の合計が領域に対して大きすぎる場合は、円を小さい組に落として収める
pub fn layout_circles(
    rng: &mut impl Rng,
    area: Rect,
    circles: &[(u8, CircleSize)],
    dense: bool,
) -> Vec<Placement> {
    if circles.is_empty() || area.is_empty() {
        return Vec::new();
    }
    let max_coverage = if dense {
        DENSE_MAX_COVERAGE
    } else {
        LOOSE_MAX_COVERAGE
    };
    // 密集配置はまず中央の領域に置き、収まらなければ同じ大きさのままエリア全体に広げる
    // (円を小さくするより、広げて置く方を優先する)
    let mut regions = Vec::with_capacity(2);
    if dense {
        regions.push(central_region(area, DENSE_REGION_RATIO));
    }
    regions.push(area);

    for tier in 0..SIZE_TIERS.len() {
        let sized = sized_circles(tier, circles);
        for region in &regions {
            if !fits_by_area(region, &sized, max_coverage) {
                continue;
            }
            for _ in 0..ATTEMPTS_PER_TIER {
                let placements = place_circles(rng, *region, &sized, false);
                if placements.len() == sized.len() {
                    return placements;
                }
            }
        }
    }

    // どの組でも全部は収まらない(端末が極端に小さい)場合は、最小の組で置けるだけ置く。
    // 置けなかった円はクリックできないが、端末を広げれば描画エリアが変わって配置し直される
    let sized = sized_circles(SIZE_TIERS.len() - 1, circles);
    (0..ATTEMPTS_PER_TIER)
        .map(|_| place_circles(rng, area, &sized, true))
        .max_by_key(|placements| placements.len())
        .unwrap_or_default()
}

/// areaの中央に、幅・高さをそれぞれratio倍した領域
fn central_region(area: Rect, ratio: f64) -> Rect {
    let width = ((area.width as f64 * ratio).round() as u16).clamp(1, area.width);
    let height = ((area.height as f64 * ratio).round() as u16).clamp(1, area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// 各円に、指定の組でのセル数(幅, 高さ)を割り当てる
fn sized_circles(tier: usize, circles: &[(u8, CircleSize)]) -> Vec<(u8, u16, u16)> {
    circles
        .iter()
        .map(|&(number, size)| {
            let (width, height) = size_dims(tier, size);
            (number, width, height)
        })
        .collect()
}

/// どの円も領域に入る大きさで、円の面積の合計が領域の面積のmax_coverage倍以下か
fn fits_by_area(region: &Rect, sized: &[(u8, u16, u16)], max_coverage: f64) -> bool {
    let every_circle_fits = sized
        .iter()
        .all(|&(_, w, h)| w <= region.width && h <= region.height);
    if !every_circle_fits {
        return false;
    }
    let occupied: u64 = sized
        .iter()
        .map(|&(_, w, h)| u64::from(w) * u64::from(h))
        .sum();
    occupied as f64 <= f64::from(region.area()) * max_coverage
}

/// 大きい円から順に(=奥から手前へ)置く。新しく置く円は、それまでに置いた円の数字の範囲を
/// 覆わない位置の中からランダムに選ぶ。置けない円があった場合、skip_unplaceableがfalseなら
/// そこで打ち切り、trueならその円を飛ばして残りを置き続ける。戻り値は置けた円(描画順)
fn place_circles(
    rng: &mut impl Rng,
    region: Rect,
    sized: &[(u8, u16, u16)],
    skip_unplaceable: bool,
) -> Vec<Placement> {
    let mut order: Vec<&(u8, u16, u16)> = sized.iter().collect();
    // 同じ大きさの円どうしの順番(=重なった時の前後)はランダムにする
    order.shuffle(rng);
    order.sort_by_key(|&&(_, w, h)| Reverse(u32::from(w) * u32::from(h)));

    let mut placed: Vec<Placement> = Vec::with_capacity(sized.len());
    for &&(number, width, height) in &order {
        match choose_position(rng, region, width, height, &placed) {
            Some(rect) => placed.push(Placement { number, rect }),
            None if skip_unplaceable => continue,
            None => break,
        }
    }
    placed
}

/// 幅width・高さheightの円を、region内で置いた円(placed)の数字の範囲を覆わない位置に
/// ランダムに置く。まずランダムな位置を何度か試し(条件を満たす位置の中から一様に選ぶのと同じ)、
/// 見つからなければ全ての位置を調べて選ぶ。置ける位置が無ければNone
fn choose_position(
    rng: &mut impl Rng,
    region: Rect,
    width: u16,
    height: u16,
    placed: &[Placement],
) -> Option<Rect> {
    if width > region.width || height > region.height {
        return None;
    }
    let valid = |rect: Rect| placed.iter().all(|below| !hides_number(rect, below.rect));
    let (max_x, max_y) = (region.right() - width, region.bottom() - height);
    for _ in 0..RANDOM_POSITION_TRIES {
        let rect = Rect::new(
            rng.gen_range(region.x..=max_x),
            rng.gen_range(region.y..=max_y),
            width,
            height,
        );
        if valid(rect) {
            return Some(rect);
        }
    }
    let candidates: Vec<Rect> = (region.y..=max_y)
        .flat_map(|y| (region.x..=max_x).map(move |x| Rect::new(x, y, width, height)))
        .filter(|&rect| valid(rect))
        .collect();
    candidates.choose(rng).copied()
}

/// 手前に置く円aboveが、奥の円belowの数字の範囲のセルを1つでも覆うか
fn hides_number(above: Rect, below: Rect) -> bool {
    let protected = protected_area(below);
    if !above.intersects(protected) {
        return false;
    }
    (protected.y..protected.bottom())
        .flat_map(|y| (protected.x..protected.right()).map(move |x| (x, y)))
        .any(|(x, y)| circle_contains(above, x, y))
}

/// 丸囲み数字(テキスト表示)を置くセル。円の中央の行の、中央最大3セル
pub fn label_area(rect: Rect) -> Rect {
    let width = rect.width.min(3);
    Rect::new(
        rect.x + (rect.width - width) / 2,
        rect.y + rect.height / 2,
        width,
        rect.height.min(1),
    )
}

/// 円の数字の範囲。手前の円にここを覆わせないことで、どの円も数字が読め、
/// 中心をクリックすればその円に当たる。丸囲み数字のセル(label_area)と、
/// 画像表示での数字の範囲(円の中央、幅・高さの一定割合)を合わせた範囲
pub fn protected_area(rect: Rect) -> Rect {
    let label = label_area(rect);
    let (x, width) = central_span(rect.x, rect.width, DIGIT_WIDTH_RATIO);
    let (y, height) = central_span(rect.y, rect.height, DIGIT_HEIGHT_RATIO);
    let digits = Rect::new(x, y, width, height);
    if digits.is_empty() {
        label
    } else {
        label.union(digits)
    }
}

/// start から len セル並んだ区間のうち、中央の長さ len*ratio の範囲にセルの中心が入るものの
/// (先頭のセル, 個数)。該当するセルが無ければ個数0
fn central_span(start: u16, len: u16, ratio: f64) -> (u16, u16) {
    let mid = f64::from(len) / 2.0;
    let half = f64::from(len) * ratio / 2.0;
    // セルiの中心は i+0.5。mid-half <= i+0.5 <= mid+half を満たすiの範囲
    let first = (mid - half - 0.5).ceil().max(0.0);
    let last = (mid + half - 0.5).floor().min(f64::from(len) - 1.0);
    if last < first {
        return (start, 0);
    }
    (start + first as u16, (last - first) as u16 + 1)
}

/// 描画順(奥→手前)に並べ替えた配置。面積の大きい円が奥で、同じ大きさどうしは元の並び順
/// (後ろほど手前)を保つ。hit_testの優先順位とそろえている
pub fn back_to_front(placements: &[Placement]) -> Vec<Placement> {
    let mut ordered = placements.to_vec();
    ordered.sort_by_key(|p| Reverse(p.rect.area()));
    ordered
}

/// (column, row)のセルが、rectに内接する楕円(=円の見た目)の内側にあるか。
/// セルの中心点が楕円の内側(境界を含む)にあれば内側とみなす
pub fn circle_contains(rect: Rect, column: u16, row: u16) -> bool {
    if rect.is_empty() || !crate::game::contains(rect, column, row) {
        return false;
    }
    let radius_x = rect.width as f64 / 2.0;
    let radius_y = rect.height as f64 / 2.0;
    let dx = (column as f64 + 0.5 - rect.x as f64 - radius_x) / radius_x;
    let dy = (row as f64 + 0.5 - rect.y as f64 - radius_y) / radius_y;
    dx * dx + dy * dy <= 1.0
}

/// クリック座標にある円の番号を返す。is_visibleがfalseの円(消えた円)は対象外。
/// 複数の円が重なっている場合は、見た目で手前にある円を返す。
/// つまり最も面積の小さい円で、同じ大きさどうしなら並びの後ろ(後に描かれる方)を優先する
pub fn hit_test(
    placements: &[Placement],
    is_visible: impl Fn(u8) -> bool,
    column: u16,
    row: u16,
) -> Option<u8> {
    placements
        .iter()
        .enumerate()
        .filter(|(_, p)| is_visible(p.number) && circle_contains(p.rect, column, row))
        .min_by_key(|&(index, p)| (p.rect.area(), Reverse(index)))
        .map(|(_, p)| p.number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn circles(n: u8, levels: &[CircleSize]) -> Vec<(u8, CircleSize)> {
        (1..=n)
            .map(|i| (i, levels[(i as usize - 1) % levels.len()]))
            .collect()
    }

    const ALL: [CircleSize; 4] = [
        CircleSize::Huge,
        CircleSize::Large,
        CircleSize::Medium,
        CircleSize::Small,
    ];
    const HUGE_LARGE_MEDIUM: [CircleSize; 3] =
        [CircleSize::Huge, CircleSize::Large, CircleSize::Medium];

    /// 難易度ごとの(円の数, サイズ段階, 密集配置)の組み合わせ
    const CONFIGS: [(u8, &[CircleSize], bool); 3] = [
        (10, &HUGE_LARGE_MEDIUM, false),
        (14, &ALL, false),
        (20, &ALL, true),
    ];

    /// 円の中心セル(テストや当たり判定の確認でクリックする位置)
    fn center(rect: Rect) -> (u16, u16) {
        (rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    fn cells(rect: Rect) -> impl Iterator<Item = (u16, u16)> {
        (rect.y..rect.bottom()).flat_map(move |y| (rect.x..rect.right()).map(move |x| (x, y)))
    }

    /// どの円の数字も読めて、中心をクリックすればその円に当たること。
    /// 押し終えた円が消えていく途中(1..k-1が消えた状態)でも同じことを確かめる
    fn assert_numbers_readable(placements: &[Placement]) {
        // 描画順で後ろ(手前)の円が、前(奥)の円の数字の範囲を覆っていないこと
        let ordered = back_to_front(placements);
        for (i, below) in ordered.iter().enumerate() {
            for above in &ordered[i + 1..] {
                for (x, y) in cells(protected_area(below.rect)) {
                    assert!(
                        !circle_contains(above.rect, x, y),
                        "円{}({:?})の数字の範囲({x},{y})が手前の円{}({:?})に隠れている",
                        below.number,
                        below.rect,
                        above.number,
                        above.rect
                    );
                }
            }
        }
        let max = placements.iter().map(|p| p.number).max().unwrap_or(0);
        for removed_below in 1..=max {
            let visible = |n: u8| n >= removed_below;
            for p in placements.iter().filter(|p| visible(p.number)) {
                let (x, y) = center(p.rect);
                assert_eq!(
                    hit_test(placements, visible, x, y),
                    Some(p.number),
                    "{removed_below}未満が消えた状態で円{}の中心をクリックするとその円に当たる",
                    p.number
                );
            }
        }
    }

    fn assert_inside(area: Rect, placements: &[Placement]) {
        for p in placements {
            assert_eq!(
                area.intersection(p.rect),
                p.rect,
                "円{}がエリア内に収まること",
                p.number
            );
        }
    }

    /// 2つの円の矩形が重なっている面積の合計
    fn total_overlap(placements: &[Placement]) -> u64 {
        let mut total = 0u64;
        for (i, a) in placements.iter().enumerate() {
            for b in &placements[i + 1..] {
                total += u64::from(a.rect.intersection(b.rect).area());
            }
        }
        total
    }

    fn any_overlap(placements: &[Placement]) -> bool {
        placements.iter().enumerate().any(|(i, a)| {
            placements[i + 1..]
                .iter()
                .any(|b| a.rect.intersects(b.rect))
        })
    }

    fn placement(number: u8, x: u16, y: u16, width: u16, height: u16) -> Placement {
        Placement {
            number,
            rect: Rect::new(x, y, width, height),
        }
    }

    // --- サイズ段階 ---

    #[test]
    fn size_dims_keep_huge_large_medium_small_order_in_every_tier() {
        for tier in 0..SIZE_TIERS.len() {
            let (hw, hh) = size_dims(tier, CircleSize::Huge);
            let (lw, lh) = size_dims(tier, CircleSize::Large);
            let (mw, mh) = size_dims(tier, CircleSize::Medium);
            let (sw, sh) = size_dims(tier, CircleSize::Small);
            assert!(hh > lh && lh > mh && mh > sh, "tier={tier}: 特大>大>中>小");
            assert!(sh >= 1);
            // 幅は高さの2倍(セルの縦横比を考えてほぼ正円に見せる)
            assert_eq!((hw, lw, mw, sw), (hh * 2, lh * 2, mh * 2, sh * 2));
        }
    }

    #[test]
    fn size_dims_huge_is_larger_than_large_in_width_and_height_in_every_tier() {
        for tier in 0..SIZE_TIERS.len() {
            let (hw, hh) = size_dims(tier, CircleSize::Huge);
            let (lw, lh) = size_dims(tier, CircleSize::Large);
            assert!(hw > lw, "tier={tier}: 特大の幅{hw} > 大の幅{lw}");
            assert!(hh > lh, "tier={tier}: 特大の高さ{hh} > 大の高さ{lh}");
        }
    }

    #[test]
    fn size_dims_keep_existing_large_medium_small_heights() {
        // 特大を足しても、既存の大・中・小の高さは変えない
        let existing: [[u16; 3]; 5] = [[7, 5, 3], [6, 4, 3], [5, 4, 2], [4, 3, 2], [3, 2, 1]];
        for (tier, [l, m, s]) in existing.into_iter().enumerate() {
            assert_eq!(size_dims(tier, CircleSize::Large).1, l, "tier={tier}");
            assert_eq!(size_dims(tier, CircleSize::Medium).1, m, "tier={tier}");
            assert_eq!(size_dims(tier, CircleSize::Small).1, s, "tier={tier}");
        }
    }

    // --- 配置 ---

    #[test]
    fn layout_places_every_circle_inside_area_with_numbers_readable() {
        let areas = [
            Rect::new(3, 5, 78, 19),
            Rect::new(0, 0, 160, 50),
            Rect::new(0, 0, 40, 10),
        ];
        for area in areas {
            for (n, levels, dense) in CONFIGS {
                for trial in 0..20u64 {
                    let mut rng = StdRng::seed_from_u64(u64::from(n) * 1000 + trial);
                    let placements = layout_circles(&mut rng, area, &circles(n, levels), dense);
                    assert_eq!(placements.len(), n as usize, "全ての円が配置されること");
                    let mut numbers: Vec<u8> = placements.iter().map(|p| p.number).collect();
                    numbers.sort();
                    assert_eq!(numbers, (1..=n).collect::<Vec<_>>(), "各番号が1つずつ");
                    assert_inside(area, &placements);
                    assert_numbers_readable(&placements);
                }
            }
        }
    }

    #[test]
    fn layout_lets_circles_overlap() {
        // 元のゲームと同じく円どうしが重なり合う配置になる
        let area = Rect::new(3, 5, 78, 19);
        for (n, levels, dense) in CONFIGS {
            let overlapping = (0..20u64)
                .filter(|&seed| {
                    let mut rng = StdRng::seed_from_u64(seed);
                    any_overlap(&layout_circles(&mut rng, area, &circles(n, levels), dense))
                })
                .count();
            assert!(
                overlapping >= 10,
                "n={n}: 20回中{overlapping}回しか重ならない"
            );
        }
    }

    #[test]
    fn layout_returns_circles_back_to_front() {
        // 戻り値は描画順(大きい円が先=奥、小さい円が後=手前)
        let area = Rect::new(0, 0, 120, 40);
        for seed in 0..10 {
            let mut rng = StdRng::seed_from_u64(seed);
            let placements = layout_circles(&mut rng, area, &circles(20, &ALL), true);
            let areas: Vec<u32> = placements.iter().map(|p| p.rect.area()).collect();
            assert!(
                areas.windows(2).all(|w| w[0] >= w[1]),
                "面積が大きい順: {areas:?}"
            );
        }
    }

    #[test]
    fn layout_keeps_size_order_between_levels() {
        // 同じ配置の中では、特大の円は大より、大の円は中より、中の円は小より大きい
        let area = Rect::new(0, 0, 120, 40);
        let mut rng = StdRng::seed_from_u64(7);
        let input = circles(14, &ALL);
        let placements = layout_circles(&mut rng, area, &input, false);
        let height_of = |size: CircleSize| {
            let number = input.iter().find(|(_, s)| *s == size).unwrap().0;
            placements
                .iter()
                .find(|p| p.number == number)
                .unwrap()
                .rect
                .height
        };
        let (h, l, m, s) = (
            height_of(CircleSize::Huge),
            height_of(CircleSize::Large),
            height_of(CircleSize::Medium),
            height_of(CircleSize::Small),
        );
        assert!(h > l && l > m && m > s, "特大{h} > 大{l} > 中{m} > 小{s}");
    }

    #[test]
    fn layout_shrinks_circles_to_fit_small_area() {
        // 小さな端末でも全ての円が収まり、数字も読める
        let area = Rect::new(0, 0, 40, 10);
        let mut rng = StdRng::seed_from_u64(3);
        let placements = layout_circles(&mut rng, area, &circles(20, &ALL), true);
        assert_eq!(placements.len(), 20);
        assert_inside(area, &placements);
        assert_numbers_readable(&placements);
    }

    #[test]
    fn dense_layout_packs_circles_closer_than_loose_layout() {
        // 密集配置は円が占める範囲(外接矩形)が狭くなる
        let area = Rect::new(0, 0, 160, 50);
        let input = circles(20, &ALL);
        let spread = |dense: bool| {
            let mut total = 0u64;
            for seed in 0..10 {
                let mut rng = StdRng::seed_from_u64(seed);
                let placements = layout_circles(&mut rng, area, &input, dense);
                let bounds = placements
                    .iter()
                    .map(|p| p.rect)
                    .reduce(|a, b| a.union(b))
                    .unwrap();
                total += bounds.area() as u64;
            }
            total
        };
        assert!(
            spread(true) < spread(false),
            "密集配置の方が狭い範囲に集まること"
        );
    }

    #[test]
    fn dense_layout_overlaps_more_than_loose_layout() {
        let area = Rect::new(0, 0, 160, 50);
        let input = circles(20, &ALL);
        let overlap = |dense: bool| {
            (0..10)
                .map(|seed| {
                    let mut rng = StdRng::seed_from_u64(seed);
                    total_overlap(&layout_circles(&mut rng, area, &input, dense))
                })
                .sum::<u64>()
        };
        let (dense, loose) = (overlap(true), overlap(false));
        assert!(dense > loose, "密集{dense} > 通常{loose}");
    }

    #[test]
    fn dense_layout_stays_in_central_region_when_it_fits() {
        // 広いエリアなら、密集配置の円は全て中央の領域に収まる
        let area = Rect::new(0, 0, 160, 50);
        let region = central_region(area, DENSE_REGION_RATIO);
        assert_eq!(region, Rect::new(8, 2, 144, 45));
        for seed in 0..5 {
            let mut rng = StdRng::seed_from_u64(seed);
            let placements = layout_circles(&mut rng, area, &circles(20, &ALL), true);
            assert_eq!(placements.len(), 20);
            assert_inside(region, &placements);
        }
    }

    #[test]
    fn layout_in_too_small_area_places_what_fits_with_numbers_readable() {
        // 全部は収まらないほど小さいエリアでもpanicせず、置けた分の数字は読める
        let area = Rect::new(0, 0, 6, 2);
        let mut rng = StdRng::seed_from_u64(1);
        let placements = layout_circles(&mut rng, area, &circles(20, &ALL), true);
        assert!(!placements.is_empty() && placements.len() < 20);
        assert_inside(area, &placements);
        assert_numbers_readable(&placements);
        assert!(
            layout_circles(&mut rng, Rect::new(0, 0, 0, 0), &circles(3, &ALL), false).is_empty()
        );
    }

    // --- 数字の範囲 ---

    #[test]
    fn protected_area_covers_label_and_center_and_stays_inside_circle() {
        for tier in 0..SIZE_TIERS.len() {
            for size in ALL {
                let (w, h) = size_dims(tier, size);
                let rect = Rect::new(5, 3, w, h);
                let label = label_area(rect);
                let protected = protected_area(rect);
                assert!(!label.is_empty() && label.height == 1);
                assert_eq!(
                    protected.intersection(label),
                    label,
                    "{rect:?}: 数字のセルを含む"
                );
                let (cx, cy) = center(rect);
                assert!(
                    crate::game::contains(protected, cx, cy),
                    "{rect:?}: 中心を含む"
                );
                for (x, y) in cells(protected) {
                    assert!(circle_contains(rect, x, y), "{rect:?}: ({x},{y})は円の内側");
                }
            }
        }
    }

    #[test]
    fn protected_area_of_large_circle_covers_two_digit_number() {
        // 画像の2桁の数字(幅約42%・高さ約30%)が隠れないよう、ラベルより広い範囲を守る
        let rect = Rect::new(0, 0, 14, 7);
        let protected = protected_area(rect);
        assert!(protected.width >= 6, "{protected:?}");
        assert!(protected.height >= 3, "{protected:?}");
    }

    #[test]
    fn label_area_is_centered_up_to_three_cells() {
        assert_eq!(label_area(Rect::new(10, 5, 12, 6)), Rect::new(14, 8, 3, 1));
        assert_eq!(label_area(Rect::new(0, 0, 2, 1)), Rect::new(0, 0, 2, 1));
        assert_eq!(label_area(Rect::new(0, 0, 4, 2)), Rect::new(0, 1, 3, 1));
    }

    // --- 描画順 ---

    #[test]
    fn back_to_front_puts_larger_first_and_keeps_order_among_same_size() {
        let input = [
            placement(1, 0, 0, 2, 1),
            placement(2, 0, 0, 14, 7),
            placement(3, 0, 0, 10, 5),
            placement(4, 5, 0, 14, 7),
            placement(5, 9, 0, 2, 1),
        ];
        let numbers: Vec<u8> = back_to_front(&input).iter().map(|p| p.number).collect();
        assert_eq!(numbers, [2, 4, 3, 1, 5]);
    }

    // --- 当たり判定 ---

    #[test]
    fn circle_contains_center_but_not_corners() {
        let rect = Rect::new(10, 5, 12, 6);
        assert!(circle_contains(rect, 16, 8), "中心付近は円の内側");
        assert!(circle_contains(rect, 15, 7), "中心付近は円の内側");
        assert!(!circle_contains(rect, 10, 5), "左上の角は円の外側");
        assert!(!circle_contains(rect, 21, 10), "右下の角は円の外側");
        assert!(!circle_contains(rect, 9, 8), "矩形の外は円の外側");
        assert!(!circle_contains(rect, 22, 8), "矩形の外は円の外側");
    }

    #[test]
    fn circle_contains_every_cell_of_smallest_circle() {
        // 最小サイズ(2x1)でも両方のセルがクリックできる
        let rect = Rect::new(0, 0, 2, 1);
        assert!(circle_contains(rect, 0, 0));
        assert!(circle_contains(rect, 1, 0));
    }

    #[test]
    fn hit_test_returns_number_of_clicked_visible_circle() {
        let placements = [placement(1, 0, 0, 10, 5), placement(2, 20, 0, 10, 5)];
        assert_eq!(hit_test(&placements, |_| true, 5, 2), Some(1));
        assert_eq!(hit_test(&placements, |_| true, 25, 2), Some(2));
        assert_eq!(
            hit_test(&placements, |_| true, 15, 2),
            None,
            "円の間は何も無い"
        );
        assert_eq!(
            hit_test(&placements, |_| true, 0, 0),
            None,
            "円の角(外側)は何も無い"
        );
        assert_eq!(
            hit_test(&placements, |n| n != 1, 5, 2),
            None,
            "消えた円はクリックできない"
        );
    }

    #[test]
    fn hit_test_prefers_smallest_overlapping_circle_regardless_of_order() {
        // 大きい円1の上に小さい円2が重なっている。重なった所は手前の小さい円に当たる
        let large = placement(1, 0, 0, 14, 7);
        let small = placement(2, 4, 2, 6, 3);
        for placements in [[large, small], [small, large]] {
            assert_eq!(hit_test(&placements, |_| true, 7, 3), Some(2));
            assert_eq!(
                hit_test(&placements, |_| true, 2, 3),
                Some(1),
                "小さい円の外は大きい円"
            );
        }
    }

    #[test]
    fn hit_test_among_same_size_prefers_later_drawn_circle() {
        let a = placement(1, 0, 0, 10, 5);
        let b = placement(2, 4, 0, 10, 5);
        assert_eq!(hit_test(&[a, b], |_| true, 6, 2), Some(2));
        assert_eq!(hit_test(&[b, a], |_| true, 6, 2), Some(1));
    }

    #[test]
    fn hit_test_falls_through_to_circle_below_when_top_is_removed() {
        let placements = [placement(1, 0, 0, 14, 7), placement(2, 4, 2, 6, 3)];
        assert_eq!(hit_test(&placements, |n| n != 2, 7, 3), Some(1));
    }

    // --- 上級(密集配置)の広さ・特大円の大きさ ---

    /// 変更前の特大の高さ(組ごと)
    const OLD_HUGE_HEIGHTS: [u16; 5] = [9, 8, 7, 5, 4];

    #[test]
    fn huge_circles_are_larger_than_before() {
        for (tier, old) in OLD_HUGE_HEIGHTS.into_iter().enumerate() {
            let (_, h) = size_dims(tier, CircleSize::Huge);
            assert!(
                h >= old,
                "tier={tier}: 特大{h}が変更前{old}より小さくならない"
            );
        }
        // 大きい端末で使う組では、はっきり大きくする
        for (tier, old) in OLD_HUGE_HEIGHTS.into_iter().enumerate().take(3) {
            let (_, h) = size_dims(tier, CircleSize::Huge);
            assert!(
                h >= old + 2,
                "tier={tier}: 特大{h}は変更前{old}より2段以上大きい"
            );
        }
    }

    /// 円(楕円)の内側にあるセルが、エリアの全セルに占める割合
    fn covered_ratio(area: Rect, placements: &[Placement]) -> f64 {
        let covered = cells(area)
            .filter(|&(x, y)| placements.iter().any(|p| circle_contains(p.rect, x, y)))
            .count();
        covered as f64 / f64::from(area.area())
    }

    /// 全ての円の外接矩形がエリアに占める割合
    fn spread_ratio(area: Rect, placements: &[Placement]) -> f64 {
        let bounds = placements
            .iter()
            .map(|p| p.rect)
            .reduce(|a, b| a.union(b))
            .unwrap_or_default();
        f64::from(bounds.area()) / f64::from(area.area())
    }

    /// 複数のseedで配置し、measureの平均を取る
    fn average(
        area: Rect,
        (n, levels, dense): (u8, &[CircleSize], bool),
        measure: fn(Rect, &[Placement]) -> f64,
    ) -> f64 {
        let seeds = 0..12u64;
        let count = seeds.clone().count() as f64;
        seeds
            .map(|seed| {
                let mut rng = StdRng::seed_from_u64(seed);
                measure(
                    area,
                    &layout_circles(&mut rng, area, &circles(n, levels), dense),
                )
            })
            .sum::<f64>()
            / count
    }

    /// ゲームで使う大きさの盤面(100x36の画面・大きめの端末)
    const GAME_BOARDS: [Rect; 2] = [Rect::new(1, 4, 98, 31), Rect::new(1, 4, 158, 45)];

    #[test]
    fn advanced_layout_spreads_over_most_of_the_board() {
        // 上級(密集配置)も盤面の中央に固まらず、盤面の大部分に広がる
        // (変更前は中央の60%四方=面積36%以内に収まっていた)
        let [_, _, advanced] = CONFIGS;
        for area in GAME_BOARDS {
            let spread = average(area, advanced, spread_ratio);
            assert!(spread >= 0.7, "{area:?}: 上級の広がり{spread:.2}");
        }
    }

    #[test]
    fn advanced_layout_covers_clearly_more_of_the_board_than_beginner() {
        // 上級は円が多く、中央に固めずに置くので、初級より画面の広い範囲を円で埋める
        // (実測は盤面98x31で上級約0.52/初級約0.41、158x45で約0.25/約0.18)
        let [beginner, _, advanced] = CONFIGS;
        for area in GAME_BOARDS {
            let b = average(area, beginner, covered_ratio);
            let a = average(area, advanced, covered_ratio);
            assert!(a >= b * 1.2, "{area:?}: 上級{a:.2} / 初級{b:.2}");
        }
    }

    #[test]
    fn advanced_layout_keeps_largest_tier_on_game_boards() {
        // 円の数が多い上級でも、ゲームの盤面では組を落とさず最大の特大円を使う
        let [_, _, (n, levels, dense)] = CONFIGS;
        let input = circles(n, levels);
        let huge_number = input
            .iter()
            .find(|(_, s)| *s == CircleSize::Huge)
            .unwrap()
            .0;
        for area in GAME_BOARDS {
            for seed in 0..5 {
                let mut rng = StdRng::seed_from_u64(seed);
                let placements = layout_circles(&mut rng, area, &input, dense);
                let huge = placements.iter().find(|p| p.number == huge_number).unwrap();
                assert_eq!(
                    (huge.rect.width, huge.rect.height),
                    size_dims(0, CircleSize::Huge),
                    "{area:?} seed={seed}"
                );
            }
        }
    }
}
