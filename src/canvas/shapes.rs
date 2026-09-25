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

#[cfg(test)]
mod tests {
    use super::*;

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
}
