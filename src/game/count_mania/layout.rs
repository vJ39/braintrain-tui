//! カウントマニアの円の配置とクリック当たり判定。
//!
//! 円の大きさは「端末上で占めるセル数」で表す。端末のセルは縦長(横:縦 ≒ 1:2)なので、
//! 幅を高さの2倍にすると見た目がほぼ正円になる。配置はプレイエリアをセルのグリッドとして扱い、
//! 円ごとの矩形が重ならないようにランダムに置く。

use rand::seq::SliceRandom;
use rand::Rng;
use ratatui::layout::Rect;

/// 円のサイズ段階
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CircleSize {
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
/// 1段小さい組に落とす。どの組でも 大 > 中 > 小 を保ち、サイズ段階の違いが見えるようにする。
/// 幅は高さの2倍にする
const SIZE_TIERS: [[u16; 3]; 5] = [[7, 5, 3], [6, 4, 3], [5, 4, 2], [4, 3, 2], [3, 2, 1]];

/// 1つの組で配置をやり直す回数(ランダムな置き方の運で失敗することがあるため)
const ATTEMPTS_PER_TIER: usize = 3;

/// 配置の詰め具合。gapは円どうしの最小間隔(セル)、max_fillは円の占有面積(間隔込み)が
/// 配置領域に占めてよい割合の上限
struct Packing {
    gap_x: u16,
    gap_y: u16,
    max_fill: f64,
}

const LOOSE: Packing = Packing {
    gap_x: 2,
    gap_y: 1,
    max_fill: 0.45,
};

const DENSE: Packing = Packing {
    gap_x: 0,
    gap_y: 0,
    max_fill: 0.75,
};

/// 密集配置のとき、プレイエリアの中央に寄せる領域の割合(幅・高さそれぞれ)
const DENSE_REGION_RATIO: f64 = 0.7;

/// 指定の組(tier)でのサイズ段階ごとのセル数(幅, 高さ)
pub fn size_dims(tier: usize, size: CircleSize) -> (u16, u16) {
    let heights = SIZE_TIERS[tier.min(SIZE_TIERS.len() - 1)];
    let height = match size {
        CircleSize::Large => heights[0],
        CircleSize::Medium => heights[1],
        CircleSize::Small => heights[2],
    };
    (height * 2, height)
}

/// 円(番号, サイズ段階)をarea内に重ならないようランダムに配置する。
/// denseがtrueなら円どうしの間隔を詰め、プレイエリア中央に寄せて密集させる。
/// エリアが小さすぎる場合は円を小さい組に落として収める
pub fn layout_circles(
    rng: &mut impl Rng,
    area: Rect,
    circles: &[(u8, CircleSize)],
    dense: bool,
) -> Vec<Placement> {
    if circles.is_empty() || area.is_empty() {
        return Vec::new();
    }
    let packing = if dense { &DENSE } else { &LOOSE };
    // 密集配置はまず中央の領域に詰め、収まらなければエリア全体に広げる
    let mut regions = Vec::with_capacity(2);
    if dense {
        regions.push(central_region(area, DENSE_REGION_RATIO));
    }
    regions.push(area);

    for region in &regions {
        for tier in 0..SIZE_TIERS.len() {
            let sized = sized_circles(tier, circles);
            if !fits_by_area(region, &sized, packing) {
                continue;
            }
            for _ in 0..ATTEMPTS_PER_TIER {
                let placements = place_circles(rng, *region, &sized, packing, false);
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
        .map(|_| place_circles(rng, area, &sized, packing, true))
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

/// 円の占有面積(間隔込み)が配置領域の上限割合に収まり、どの円も領域に入る大きさか
fn fits_by_area(region: &Rect, sized: &[(u8, u16, u16)], packing: &Packing) -> bool {
    let every_circle_fits = sized
        .iter()
        .all(|&(_, w, h)| w <= region.width && h <= region.height);
    if !every_circle_fits {
        return false;
    }
    let occupied: u64 = sized
        .iter()
        .map(|&(_, w, h)| u64::from(w + packing.gap_x) * u64::from(h + packing.gap_y))
        .sum();
    // 領域の右端・下端には間隔が要らないので、その分だけ領域を広げて比べる
    let capacity =
        u64::from(region.width + packing.gap_x) * u64::from(region.height + packing.gap_y);
    occupied as f64 <= capacity as f64 * packing.max_fill
}

/// 大きい円から順に、region内のグリッドを走査して他の円(間隔込み)と重ならない位置を集め、
/// その中からランダムに選んで置く。置けない円があった場合、skip_unplaceableがfalseならそこで
/// 打ち切り、trueならその円を飛ばして残りを置き続ける。戻り値は置けた円(番号順)
fn place_circles(
    rng: &mut impl Rng,
    region: Rect,
    sized: &[(u8, u16, u16)],
    packing: &Packing,
    skip_unplaceable: bool,
) -> Vec<Placement> {
    let mut order: Vec<&(u8, u16, u16)> = sized.iter().collect();
    // 同じ大きさの円どうしの順番はランダムにする(番号順に偏った配置にしないため)
    order.shuffle(rng);
    order.sort_by_key(|&&(_, w, h)| std::cmp::Reverse(u32::from(w) * u32::from(h)));

    let mut placed: Vec<Placement> = Vec::with_capacity(sized.len());
    for &&(number, width, height) in &order {
        if width > region.width || height > region.height {
            if skip_unplaceable {
                continue;
            }
            break;
        }
        let candidates: Vec<(u16, u16)> = (region.y..=region.bottom() - height)
            .flat_map(|y| (region.x..=region.right() - width).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let rect = Rect::new(x, y, width, height);
                placed.iter().all(|p| !too_close(rect, p.rect, packing))
            })
            .collect();
        let Some(&(x, y)) = candidates.choose(rng) else {
            if skip_unplaceable {
                continue;
            }
            break;
        };
        placed.push(Placement {
            number,
            rect: Rect::new(x, y, width, height),
        });
    }
    placed.sort_by_key(|p| p.number);
    placed
}

/// 2つの矩形が重なる、または間隔(gap)未満まで近づいているか
fn too_close(a: Rect, b: Rect, packing: &Packing) -> bool {
    a.x < b.right() + packing.gap_x
        && b.x < a.right() + packing.gap_x
        && a.y < b.bottom() + packing.gap_y
        && b.y < a.bottom() + packing.gap_y
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

/// クリック座標にある円の番号を返す。is_visibleがfalseの円(消えた円)は対象外
pub fn hit_test(
    placements: &[Placement],
    is_visible: impl Fn(u8) -> bool,
    column: u16,
    row: u16,
) -> Option<u8> {
    placements
        .iter()
        .find(|p| is_visible(p.number) && circle_contains(p.rect, column, row))
        .map(|p| p.number)
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

    const ALL: [CircleSize; 3] = [CircleSize::Large, CircleSize::Medium, CircleSize::Small];

    fn assert_no_overlap(placements: &[Placement]) {
        for (i, a) in placements.iter().enumerate() {
            for b in &placements[i + 1..] {
                assert!(
                    !a.rect.intersects(b.rect),
                    "円{}({:?})と円{}({:?})が重なっている",
                    a.number,
                    a.rect,
                    b.number,
                    b.rect
                );
            }
        }
    }

    #[test]
    fn size_dims_keep_large_medium_small_order_in_every_tier() {
        for tier in 0..SIZE_TIERS.len() {
            let (lw, lh) = size_dims(tier, CircleSize::Large);
            let (mw, mh) = size_dims(tier, CircleSize::Medium);
            let (sw, sh) = size_dims(tier, CircleSize::Small);
            assert!(lh > mh && mh > sh, "tier={tier}: 大>中>小");
            assert!(sh >= 1);
            // 幅は高さの2倍(セルの縦横比を考えてほぼ正円に見せる)
            assert_eq!((lw, mw, sw), (lh * 2, mh * 2, sh * 2));
        }
    }

    #[test]
    fn layout_places_every_circle_inside_area_without_overlap() {
        let area = Rect::new(3, 5, 78, 19);
        for (seed, (n, dense)) in [(10u8, false), (14, false), (20, true)].iter().enumerate() {
            for trial in 0..20u64 {
                let mut rng = StdRng::seed_from_u64(seed as u64 * 100 + trial);
                let input = circles(*n, &ALL);
                let placements = layout_circles(&mut rng, area, &input, *dense);
                assert_eq!(placements.len(), *n as usize, "全ての円が配置されること");
                let mut numbers: Vec<u8> = placements.iter().map(|p| p.number).collect();
                numbers.sort();
                assert_eq!(numbers, (1..=*n).collect::<Vec<_>>(), "各番号が1つずつ");
                for p in &placements {
                    assert!(
                        area.contains(p.rect.as_position())
                            && p.rect.right() <= area.right()
                            && p.rect.bottom() <= area.bottom(),
                        "円{}がエリア内に収まること: {:?}",
                        p.number,
                        p.rect
                    );
                }
                assert_no_overlap(&placements);
            }
        }
    }

    #[test]
    fn layout_keeps_size_order_between_levels() {
        // 同じ配置の中では、大の円は中より、中の円は小より大きい
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
        let (l, m, s) = (
            height_of(CircleSize::Large),
            height_of(CircleSize::Medium),
            height_of(CircleSize::Small),
        );
        assert!(l > m && m > s, "大{l} > 中{m} > 小{s}");
    }

    #[test]
    fn layout_shrinks_circles_to_fit_small_area() {
        // 小さな端末でも全ての円が重ならずに収まる
        let area = Rect::new(0, 0, 40, 10);
        let mut rng = StdRng::seed_from_u64(3);
        let placements = layout_circles(&mut rng, area, &circles(20, &ALL), true);
        assert_eq!(placements.len(), 20);
        assert_no_overlap(&placements);
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
    fn dense_layout_stays_in_central_region_when_it_fits() {
        // 広いエリアなら、密集配置の円は全て中央70%の領域に収まる
        let area = Rect::new(0, 0, 160, 50);
        let region = central_region(area, DENSE_REGION_RATIO);
        assert_eq!(region, Rect::new(24, 7, 112, 35));
        for seed in 0..5 {
            let mut rng = StdRng::seed_from_u64(seed);
            let placements = layout_circles(&mut rng, area, &circles(20, &ALL), true);
            assert_eq!(placements.len(), 20);
            for p in &placements {
                assert_eq!(
                    region.intersection(p.rect),
                    p.rect,
                    "円{}が中央領域内",
                    p.number
                );
            }
        }
    }

    #[test]
    fn layout_in_too_small_area_places_what_fits_without_overlap() {
        // 全部は収まらないほど小さいエリアでもpanicせず、置けた分は重ならない
        let area = Rect::new(0, 0, 6, 2);
        let mut rng = StdRng::seed_from_u64(1);
        let placements = layout_circles(&mut rng, area, &circles(20, &ALL), true);
        assert!(!placements.is_empty() && placements.len() < 20);
        assert_no_overlap(&placements);
        assert!(
            layout_circles(&mut rng, Rect::new(0, 0, 0, 0), &circles(3, &ALL), false).is_empty()
        );
    }

    #[test]
    fn loose_layout_keeps_gap_between_circles() {
        let area = Rect::new(0, 0, 120, 40);
        let mut rng = StdRng::seed_from_u64(11);
        let placements = layout_circles(&mut rng, area, &circles(10, &ALL), false);
        for (i, a) in placements.iter().enumerate() {
            for b in &placements[i + 1..] {
                assert!(
                    !too_close(a.rect, b.rect, &LOOSE),
                    "円{}と円{}の間隔",
                    a.number,
                    b.number
                );
            }
        }
    }

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
        let placements = [
            Placement {
                number: 1,
                rect: Rect::new(0, 0, 10, 5),
            },
            Placement {
                number: 2,
                rect: Rect::new(20, 0, 10, 5),
            },
        ];
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
}
