/// 正規化座標系(-1.0〜1.0)の頂点リストで定義した図形
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub points: Vec<(f64, f64)>,
}

impl Shape {
    pub fn new(points: Vec<(f64, f64)>) -> Self {
        Self { points }
    }

    /// 原点中心にangle_rad回転させた図形を返す
    pub fn rotated(&self, angle_rad: f64) -> Self {
        let (sin, cos) = angle_rad.sin_cos();
        let points = self
            .points
            .iter()
            .map(|&(x, y)| (x * cos - y * sin, x * sin + y * cos))
            .collect();
        Self { points }
    }

    /// X軸で鏡像反転(左右反転)させた図形を返す
    pub fn mirrored_x(&self) -> Self {
        let points = self.points.iter().map(|&(x, y)| (-x, y)).collect();
        Self { points }
    }

    /// ratatui canvasのLine描画用に、頂点を結ぶ線分リストへ変換する
    pub fn to_lines(&self) -> Vec<((f64, f64), (f64, f64))> {
        let n = self.points.len();
        if n < 2 {
            return Vec::new();
        }
        (0..n)
            .map(|i| (self.points[i], self.points[(i + 1) % n]))
            .collect()
    }
}

/// 出題に使う基本図形セット
pub fn base_shapes() -> Vec<Shape> {
    vec![
        // 直角三角形(向きで回転/鏡像の区別がつく)
        Shape::new(vec![(-0.6, -0.6), (0.6, -0.6), (-0.6, 0.6)]),
        // 矢印状の非対称五角形
        Shape::new(vec![
            (0.0, -0.8),
            (0.5, 0.2),
            (0.2, 0.2),
            (0.2, 0.8),
            (-0.2, 0.8),
        ]),
        // L字型
        Shape::new(vec![
            (-0.6, -0.6),
            (0.0, -0.6),
            (0.0, 0.0),
            (0.6, 0.0),
            (0.6, 0.6),
            (-0.6, 0.6),
        ]),
        // T字型
        Shape::new(vec![
            (-0.7, -0.6),
            (0.7, -0.6),
            (0.7, -0.2),
            (0.2, -0.2),
            (0.2, 0.7),
            (-0.2, 0.7),
            (-0.2, -0.2),
            (-0.7, -0.2),
        ]),
        // 稲妻(Z字)型
        Shape::new(vec![
            (-0.5, -0.7),
            (0.5, -0.7),
            (0.0, -0.1),
            (0.5, -0.1),
            (-0.5, 0.7),
            (0.0, 0.1),
            (-0.5, 0.1),
        ]),
        // 旗型(非対称四角形)
        Shape::new(vec![(-0.6, -0.7), (0.6, -0.4), (-0.2, 0.0), (-0.6, 0.7)]),
        // 階段(2段)型
        Shape::new(vec![
            (-0.7, -0.7),
            (-0.1, -0.7),
            (-0.1, -0.1),
            (0.5, -0.1),
            (0.5, 0.7),
            (-0.7, 0.7),
        ]),
        // 鉤(フック)型
        Shape::new(vec![
            (-0.4, -0.7),
            (0.2, -0.7),
            (0.2, 0.2),
            (0.5, 0.2),
            (0.0, 0.7),
            (-0.5, 0.2),
            (-0.2, 0.2),
            (-0.2, -0.4),
            (-0.4, -0.4),
        ]),
    ]
}

// --- ジグソーピース ---

/// ジグソーピース本体(正方形)の半辺長。ピースは原点中心の[-h, h]四方に置く
pub const JIGSAW_HALF: f64 = 0.6;

/// 辺の接続部の凹凸の向き
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Knob {
    /// 外側へ張り出す出っ張り
    Tab,
    /// 内側へ凹むくぼみ
    Blank,
}

impl Knob {
    /// 凹凸を反転した向き
    pub fn opposite(self) -> Self {
        match self {
            Knob::Tab => Knob::Blank,
            Knob::Blank => Knob::Tab,
        }
    }

    /// 辺の外向きを正とした張り出しの符号(タブ=外へ+1、ブランク=内へ-1)
    fn sign(self) -> f64 {
        match self {
            Knob::Tab => 1.0,
            Knob::Blank => -1.0,
        }
    }
}

/// ピースの1辺の接続部。形状番号(jigsaw_profile_count()未満)と凹凸の向きで決まる
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JigsawEdge {
    pub profile: usize,
    pub knob: Knob,
}

impl JigsawEdge {
    pub fn new(profile: usize, knob: Knob) -> Self {
        Self { profile, knob }
    }
}

/// 左のピースの右辺(right)と右のピースの左辺(left)が隙間なく噛み合うか。
/// 形状番号が同じで、凹凸が逆(タブとブランク)の組み合わせだけが噛み合う
pub fn edges_interlock(right: JigsawEdge, left: JigsawEdge) -> bool {
    right.profile == left.profile && right.knob == left.knob.opposite()
}

/// 形状番号ごとの名前。並び順は「基本形3種 → そのサイズ違い3種 → 紛らわしい形3種」で、
/// puzzle_connectは難易度ごとに先頭から何種類使うかを決めている
const JIGSAW_PROFILE_NAMES: [&str; 9] = [
    "三角",
    "四角",
    "丸",
    "小さな三角",
    "小さな四角",
    "小さな丸",
    "あり型",
    "キノコ型",
    "二つ山",
];

/// 用意しているタブ/ブランクの形状の種類数
pub fn jigsaw_profile_count() -> usize {
    JIGSAW_PROFILE_NAMES.len()
}

/// 形状番号profileの名前(回答後のフィードバック文言に使う)
pub fn jigsaw_profile_name(profile: usize) -> &'static str {
    JIGSAW_PROFILE_NAMES[profile]
}

/// 形状番号profileと見た目が似ている(サイズ違い・輪郭が近い)形状番号の一覧
pub fn jigsaw_similar_profiles(profile: usize) -> &'static [usize] {
    match profile {
        0 => &[3],       // 三角 ↔ 小さな三角
        1 => &[4, 6, 8], // 四角 ↔ 小さな四角・あり型・二つ山
        2 => &[5, 7],    // 丸 ↔ 小さな丸・キノコ型
        3 => &[0],
        4 => &[1, 6],
        5 => &[2],
        6 => &[1, 4, 7], // あり型 ↔ 四角・小さな四角・キノコ型
        7 => &[2, 6],
        8 => &[1],
        _ => &[],
    }
}

/// 首(neck)の先に円い玉がつく出っ張りの輪郭。
/// neck_half=首の半幅、neck_len=首の長さ、center=玉の中心までの張り出し量
fn round_knob_outline(neck_half: f64, neck_len: f64, center: f64) -> Vec<(f64, f64)> {
    const ARC_SEGMENTS: usize = 12;
    let dx = center - neck_len;
    let radius = dx.hypot(neck_half);
    // 首の付け根から玉の外周を回って反対側の首へ戻る角度範囲(d軸を0度とする)
    let half_sweep = std::f64::consts::PI - neck_half.atan2(dx);
    let mut outline = vec![(0.0, -neck_half)];
    outline.extend((0..=ARC_SEGMENTS).map(|i| {
        let angle = -half_sweep + 2.0 * half_sweep * i as f64 / ARC_SEGMENTS as f64;
        (center + radius * angle.cos(), radius * angle.sin())
    }));
    outline.push((0.0, neck_half));
    outline
}

/// 形状番号profileの出っ張りの輪郭。(辺から外側への張り出し量d, 辺に沿った位置t)の点列で、
/// tの小さい方から大きい方へ進み、両端は辺の上(d=0)にある
fn profile_outline(profile: usize) -> Vec<(f64, f64)> {
    match profile {
        0 => vec![(0.0, -0.25), (0.32, 0.0), (0.0, 0.25)],
        1 => vec![(0.0, -0.2), (0.3, -0.2), (0.3, 0.2), (0.0, 0.2)],
        2 => round_knob_outline(0.09, 0.08, 0.2),
        3 => vec![(0.0, -0.14), (0.17, 0.0), (0.0, 0.14)],
        4 => vec![(0.0, -0.11), (0.16, -0.11), (0.16, 0.11), (0.0, 0.11)],
        5 => round_knob_outline(0.05, 0.04, 0.11),
        // 先へ行くほど広がる台形(あり溝)
        6 => vec![(0.0, -0.1), (0.3, -0.24), (0.3, 0.24), (0.0, 0.1)],
        // 細い首の先に横長の頭
        7 => vec![
            (0.0, -0.08),
            (0.14, -0.08),
            (0.14, -0.24),
            (0.3, -0.24),
            (0.3, 0.24),
            (0.14, 0.24),
            (0.14, 0.08),
            (0.0, 0.08),
        ],
        // 2本並んだ出っ張り
        8 => vec![
            (0.0, -0.26),
            (0.24, -0.26),
            (0.24, -0.08),
            (0.0, -0.08),
            (0.0, 0.08),
            (0.24, 0.08),
            (0.24, 0.26),
            (0.0, 0.26),
        ],
        _ => panic!("存在しない形状番号: {profile}"),
    }
}

/// 右辺にedgeの接続部を持つ正方形ピース(左辺・上辺・下辺はまっすぐ)
pub fn jigsaw_piece_right(edge: JigsawEdge) -> Shape {
    let h = JIGSAW_HALF;
    let mut points = vec![(-h, -h), (h, -h)];
    points.extend(right_edge_contour(edge));
    points.extend([(h, h), (-h, h)]);
    Shape::new(points)
}

/// 左辺にedgeの接続部を持つ正方形ピース(右辺・上辺・下辺はまっすぐ)
pub fn jigsaw_piece_left(edge: JigsawEdge) -> Shape {
    let h = JIGSAW_HALF;
    let mut points = vec![(-h, -h), (h, -h), (h, h), (-h, h)];
    points.extend(left_edge_contour(edge));
    Shape::new(points)
}

/// 右辺の接続部の輪郭(下から上へ。両端は正方形の辺上 x = h)
fn right_edge_contour(edge: JigsawEdge) -> Vec<(f64, f64)> {
    let s = edge.knob.sign();
    profile_outline(edge.profile)
        .into_iter()
        .map(|(d, t)| (JIGSAW_HALF + s * d, t))
        .collect()
}

/// 左辺の接続部の輪郭(上から下へ。両端は正方形の辺上 x = -h)
fn left_edge_contour(edge: JigsawEdge) -> Vec<(f64, f64)> {
    let s = edge.knob.sign();
    profile_outline(edge.profile)
        .into_iter()
        .rev()
        .map(|(d, t)| (-JIGSAW_HALF - s * d, t))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn all_edges() -> Vec<JigsawEdge> {
        (0..jigsaw_profile_count())
            .flat_map(|p| {
                [
                    JigsawEdge::new(p, Knob::Tab),
                    JigsawEdge::new(p, Knob::Blank),
                ]
            })
            .collect()
    }

    /// 2つの輪郭が同じ点列か(頂点数と座標が一致)
    fn contours_coincide(a: &[(f64, f64)], b: &[(f64, f64)]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(p, q)| (p.0 - q.0).abs() < EPS && (p.1 - q.1).abs() < EPS)
    }

    /// 左辺の輪郭を、右のピースとして左のピースの隣(x方向に2h)へ置いたときの点列
    /// (右辺の輪郭と同じ向き=下から上へ並べ直す)
    fn left_contour_placed_to_the_right(edge: JigsawEdge) -> Vec<(f64, f64)> {
        left_edge_contour(edge)
            .into_iter()
            .rev()
            .map(|(x, y)| (x + 2.0 * JIGSAW_HALF, y))
            .collect()
    }

    #[test]
    fn knob_opposite_swaps_tab_and_blank() {
        assert_eq!(Knob::Tab.opposite(), Knob::Blank);
        assert_eq!(Knob::Blank.opposite(), Knob::Tab);
    }

    #[test]
    fn jigsaw_has_at_least_three_profiles() {
        assert!(jigsaw_profile_count() >= 3);
    }

    #[test]
    fn jigsaw_profile_names_are_unique_and_non_empty() {
        let names: Vec<&str> = (0..jigsaw_profile_count())
            .map(jigsaw_profile_name)
            .collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "形状名が重複している: {names:?}");
        assert!(names.iter().all(|n| !n.is_empty()));
    }

    #[test]
    fn same_profile_tab_and_blank_interlock_in_both_orders() {
        for p in 0..jigsaw_profile_count() {
            let tab = JigsawEdge::new(p, Knob::Tab);
            let blank = JigsawEdge::new(p, Knob::Blank);
            assert!(edges_interlock(tab, blank), "形状{p}: タブ→ブランク");
            assert!(edges_interlock(blank, tab), "形状{p}: ブランク→タブ");
        }
    }

    #[test]
    fn same_knob_pairs_never_interlock() {
        for p in 0..jigsaw_profile_count() {
            for knob in [Knob::Tab, Knob::Blank] {
                let e = JigsawEdge::new(p, knob);
                assert!(!edges_interlock(e, e), "形状{p}: {knob:?}同士");
            }
        }
    }

    #[test]
    fn different_profiles_never_interlock() {
        for a in all_edges() {
            for b in all_edges().into_iter().filter(|b| b.profile != a.profile) {
                assert!(!edges_interlock(a, b), "{a:?} と {b:?}");
            }
        }
    }

    #[test]
    fn interlock_judgement_matches_the_drawn_contours() {
        // 判定関数が真になる組み合わせだけ、並べたときに輪郭がぴったり重なる(隙間も重なりも無い)
        for a in all_edges() {
            for b in all_edges() {
                let coincide =
                    contours_coincide(&right_edge_contour(a), &left_contour_placed_to_the_right(b));
                assert_eq!(coincide, edges_interlock(a, b), "{a:?} と {b:?}");
            }
        }
    }

    #[test]
    fn edge_contours_start_and_end_on_the_square_side() {
        let h = JIGSAW_HALF;
        for e in all_edges() {
            let right = right_edge_contour(e);
            let left = left_edge_contour(e);
            for (contour, x) in [(&right, h), (&left, -h)] {
                assert!(contour.len() >= 3, "{e:?}: 接続部は3点以上");
                let (first, last) = (contour[0], contour[contour.len() - 1]);
                assert!(
                    (first.0 - x).abs() < EPS && (last.0 - x).abs() < EPS,
                    "{e:?}"
                );
                // 接続部は辺の途中にだけある(角までは届かない)
                assert!(contour.iter().all(|p| p.1.abs() < h), "{e:?}");
            }
            // 右辺は下から上、左辺は上から下へ進む
            assert!(right[0].1 < right[right.len() - 1].1, "{e:?}");
            assert!(left[0].1 > left[left.len() - 1].1, "{e:?}");
        }
    }

    #[test]
    fn tabs_protrude_outward_and_blanks_dent_inward() {
        let h = JIGSAW_HALF;
        for p in 0..jigsaw_profile_count() {
            let tab_r = right_edge_contour(JigsawEdge::new(p, Knob::Tab));
            let blank_r = right_edge_contour(JigsawEdge::new(p, Knob::Blank));
            assert!(tab_r.iter().all(|q| q.0 >= h - EPS) && tab_r.iter().any(|q| q.0 > h + 0.1));
            assert!(
                blank_r.iter().all(|q| q.0 <= h + EPS) && blank_r.iter().any(|q| q.0 < h - 0.1)
            );
            let tab_l = left_edge_contour(JigsawEdge::new(p, Knob::Tab));
            let blank_l = left_edge_contour(JigsawEdge::new(p, Knob::Blank));
            assert!(tab_l.iter().all(|q| q.0 <= -h + EPS) && tab_l.iter().any(|q| q.0 < -h - 0.1));
            assert!(
                blank_l.iter().all(|q| q.0 >= -h - EPS) && blank_l.iter().any(|q| q.0 > -h + 0.1)
            );
        }
    }

    #[test]
    fn different_profiles_have_different_contours() {
        // 形状番号が違えば見た目の輪郭も違う(同じ凹凸の向きで比べる)
        let n = jigsaw_profile_count();
        for a in 0..n {
            for b in (0..n).filter(|&b| b != a) {
                let ca = right_edge_contour(JigsawEdge::new(a, Knob::Tab));
                let cb = right_edge_contour(JigsawEdge::new(b, Knob::Tab));
                assert!(!contours_coincide(&ca, &cb), "形状{a}と{b}が同じ輪郭");
            }
        }
    }

    #[test]
    fn right_piece_is_a_square_with_the_knob_only_on_its_right_side() {
        let h = JIGSAW_HALF;
        for e in all_edges() {
            let piece = jigsaw_piece_right(e);
            let contour = right_edge_contour(e);
            // 右辺の接続部の輪郭がそのままピースに含まれる
            let found = piece
                .points
                .windows(contour.len())
                .any(|w| contours_coincide(w, &contour));
            assert!(found, "{e:?}: 接続部の輪郭がピースに含まれていない");
            // 4つの角を持ち、接続部以外の点は左辺・上辺・下辺の直線上にある
            for corner in [(-h, -h), (h, -h), (h, h), (-h, h)] {
                assert!(piece.points.contains(&corner), "{e:?}: 角{corner:?}");
            }
            let others = piece.points.iter().filter(|p| !contour.contains(p));
            for p in others {
                assert!(
                    (p.0.abs() - h).abs() < EPS && (p.1.abs() - h).abs() < EPS,
                    "{e:?}: 接続部以外は角だけ {p:?}"
                );
            }
        }
    }

    #[test]
    fn left_piece_is_a_square_with_the_knob_only_on_its_left_side() {
        let h = JIGSAW_HALF;
        for e in all_edges() {
            let piece = jigsaw_piece_left(e);
            let contour = left_edge_contour(e);
            let found = piece
                .points
                .windows(contour.len())
                .any(|w| contours_coincide(w, &contour));
            assert!(found, "{e:?}: 接続部の輪郭がピースに含まれていない");
            for corner in [(-h, -h), (h, -h), (h, h), (-h, h)] {
                assert!(piece.points.contains(&corner), "{e:?}: 角{corner:?}");
            }
            let others = piece.points.iter().filter(|p| !contour.contains(p));
            for p in others {
                assert!(
                    (p.0.abs() - h).abs() < EPS && (p.1.abs() - h).abs() < EPS,
                    "{e:?}: 接続部以外は角だけ {p:?}"
                );
            }
        }
    }

    #[test]
    fn similar_profiles_are_symmetric_and_exclude_itself() {
        let n = jigsaw_profile_count();
        for p in 0..n {
            for &s in jigsaw_similar_profiles(p) {
                assert!(s < n && s != p, "形状{p}の類似{s}");
                assert!(
                    jigsaw_similar_profiles(s).contains(&p),
                    "類似関係は対称: {p}と{s}"
                );
            }
        }
    }

    #[test]
    fn jigsaw_pieces_fit_within_normalized_bounds() {
        for e in all_edges() {
            for piece in [jigsaw_piece_right(e), jigsaw_piece_left(e)] {
                for &(x, y) in &piece.points {
                    assert!(
                        (-1.0..=1.0).contains(&x) && (-1.0..=1.0).contains(&y),
                        "{e:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn rotated_by_full_turn_returns_original() {
        let shape = Shape::new(vec![(1.0, 0.0), (0.0, 1.0)]);
        let rotated = shape.rotated(std::f64::consts::TAU);
        for (a, b) in shape.points.iter().zip(rotated.points.iter()) {
            assert!((a.0 - b.0).abs() < 1e-9);
            assert!((a.1 - b.1).abs() < 1e-9);
        }
    }

    #[test]
    fn rotated_by_quarter_turn_swaps_axes() {
        let shape = Shape::new(vec![(1.0, 0.0)]);
        let rotated = shape.rotated(std::f64::consts::FRAC_PI_2);
        assert!((rotated.points[0].0 - 0.0).abs() < 1e-9);
        assert!((rotated.points[0].1 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn mirrored_x_flips_only_x() {
        let shape = Shape::new(vec![(0.3, 0.4), (-0.2, 0.1)]);
        let mirrored = shape.mirrored_x();
        assert_eq!(mirrored.points, vec![(-0.3, 0.4), (0.2, 0.1)]);
    }

    #[test]
    fn to_lines_connects_points_as_closed_polygon() {
        let shape = Shape::new(vec![(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
        let lines = shape.to_lines();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2], ((0.0, 1.0), (0.0, 0.0)));
    }

    #[test]
    fn to_lines_on_degenerate_shape_is_empty() {
        let shape = Shape::new(vec![(0.0, 0.0)]);
        assert!(shape.to_lines().is_empty());
    }

    #[test]
    fn base_shapes_has_eight_variations() {
        assert_eq!(base_shapes().len(), 8);
    }

    #[test]
    fn base_shapes_are_not_degenerate() {
        for shape in base_shapes() {
            assert!(
                shape.points.len() >= 3,
                "図形は多角形として描画できるよう3点以上必要"
            );
        }
    }

    #[test]
    fn base_shapes_fit_within_normalized_bounds() {
        for shape in base_shapes() {
            for &(x, y) in &shape.points {
                assert!((-1.0..=1.0).contains(&x), "x={x}が正規化範囲外");
                assert!((-1.0..=1.0).contains(&y), "y={y}が正規化範囲外");
            }
        }
    }
}
