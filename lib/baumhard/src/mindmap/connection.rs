use glam::Vec2;
use crate::mindmap::model::ControlPoint;

/// A point sampled along a connection path.
#[derive(Debug, Clone)]
pub struct SampledPoint {
    pub position: Vec2,
}

/// Represents the geometric path of a connection between two nodes.
#[derive(Debug, Clone)]
pub enum ConnectionPath {
    Straight {
        start: Vec2,
        end: Vec2,
    },
    CubicBezier {
        start: Vec2,
        control1: Vec2,
        control2: Vec2,
        end: Vec2,
    },
}

/// Number of subdivisions for arc-length approximation on Bezier curves.
const ARC_LENGTH_SUBDIVISIONS: usize = 256;

/// Resolves the anchor point on a node's bounding box.
///
/// - `node_pos`: top-left corner of the node
/// - `node_size`: (width, height) of the node
/// - `anchor`: 0=auto, 1=top, 2=right, 3=bottom, 4=left
/// - `other_center`: center of the other node (used for auto resolution)
pub fn resolve_anchor_point(
    node_pos: Vec2,
    node_size: Vec2,
    anchor: i32,
    other_center: Vec2,
) -> Vec2 {
    let half_w = node_size.x * 0.5;
    let half_h = node_size.y * 0.5;

    match anchor {
        1 => Vec2::new(node_pos.x + half_w, node_pos.y),                    // Top
        2 => Vec2::new(node_pos.x + node_size.x, node_pos.y + half_h),      // Right
        3 => Vec2::new(node_pos.x + half_w, node_pos.y + node_size.y),      // Bottom
        4 => Vec2::new(node_pos.x, node_pos.y + half_h),                    // Left
        _ => {
            // Auto: pick the edge midpoint closest to the other node's center
            let candidates = [
                Vec2::new(node_pos.x + half_w, node_pos.y),                    // Top
                Vec2::new(node_pos.x + node_size.x, node_pos.y + half_h),      // Right
                Vec2::new(node_pos.x + half_w, node_pos.y + node_size.y),      // Bottom
                Vec2::new(node_pos.x, node_pos.y + half_h),                    // Left
            ];
            candidates.into_iter()
                .min_by(|a, b| {
                    let da = a.distance_squared(other_center);
                    let db = b.distance_squared(other_center);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap()
        }
    }
}

/// Returns the center of a node given its top-left position and size.
fn node_center(pos: Vec2, size: Vec2) -> Vec2 {
    Vec2::new(pos.x + size.x * 0.5, pos.y + size.y * 0.5)
}

/// Builds a connection path from edge data.
///
/// Control points are interpreted as offsets from the respective node centers:
/// - 0 control points: straight line between anchors
/// - 1 control point: quadratic Bezier (promoted to cubic), offset from source node center
/// - 2 control points: cubic Bezier, offsets from source and target node centers respectively
pub fn build_connection_path(
    from_pos: Vec2,
    from_size: Vec2,
    anchor_from: i32,
    to_pos: Vec2,
    to_size: Vec2,
    anchor_to: i32,
    control_points: &[ControlPoint],
) -> ConnectionPath {
    let from_center = node_center(from_pos, from_size);
    let to_center = node_center(to_pos, to_size);
    let start = resolve_anchor_point(from_pos, from_size, anchor_from, to_center);
    let end = resolve_anchor_point(to_pos, to_size, anchor_to, from_center);

    match control_points.len() {
        0 => ConnectionPath::Straight { start, end },
        1 => {
            // Quadratic Bezier: promote to cubic
            // Control point is offset from source node center
            let qp = from_center + Vec2::new(control_points[0].x as f32, control_points[0].y as f32);
            // Quadratic -> Cubic: C1 = P0 + 2/3*(Q - P0), C2 = P2 + 2/3*(Q - P2)
            let c1 = start + (2.0 / 3.0) * (qp - start);
            let c2 = end + (2.0 / 3.0) * (qp - end);
            ConnectionPath::CubicBezier { start, control1: c1, control2: c2, end }
        }
        _ => {
            // Cubic Bezier: control points are offsets from respective node centers
            let c1 = from_center + Vec2::new(control_points[0].x as f32, control_points[0].y as f32);
            let c2 = to_center + Vec2::new(control_points[1].x as f32, control_points[1].y as f32);
            ConnectionPath::CubicBezier { start, control1: c1, control2: c2, end }
        }
    }
}

/// Evaluates a cubic Bezier curve at parameter t in [0, 1].
fn cubic_bezier_point(t: f32, p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> Vec2 {
    let u = 1.0 - t;
    let uu = u * u;
    let uuu = uu * u;
    let tt = t * t;
    let ttt = tt * t;
    uuu * p0 + 3.0 * uu * t * p1 + 3.0 * u * tt * p2 + ttt * p3
}

/// Computes the total arc length of a connection path.
pub fn path_length(path: &ConnectionPath) -> f32 {
    match path {
        ConnectionPath::Straight { start, end } => start.distance(*end),
        ConnectionPath::CubicBezier { start, control1, control2, end } => {
            let mut length = 0.0f32;
            let mut prev = *start;
            for i in 1..=ARC_LENGTH_SUBDIVISIONS {
                let t = i as f32 / ARC_LENGTH_SUBDIVISIONS as f32;
                let pt = cubic_bezier_point(t, *start, *control1, *control2, *end);
                length += prev.distance(pt);
                prev = pt;
            }
            length
        }
    }
}

/// Samples points along a connection path at the given spacing.
///
/// Returns evenly-spaced points including the start point. The last point
/// may be slightly before the path endpoint if the remaining distance is
/// less than `spacing`.
pub fn sample_path(path: &ConnectionPath, spacing: f32) -> Vec<SampledPoint> {
    if spacing <= 0.0 {
        return Vec::new();
    }

    match path {
        ConnectionPath::Straight { start, end } => {
            sample_straight(*start, *end, spacing)
        }
        ConnectionPath::CubicBezier { start, control1, control2, end } => {
            sample_cubic_bezier(*start, *control1, *control2, *end, spacing)
        }
    }
}

fn sample_straight(start: Vec2, end: Vec2, spacing: f32) -> Vec<SampledPoint> {
    let total_length = start.distance(end);
    if total_length < f32::EPSILON {
        return vec![SampledPoint { position: start }];
    }
    let count = (total_length / spacing).floor() as usize + 1;
    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        let t = (i as f32 * spacing) / total_length;
        let t = t.min(1.0);
        let position = start.lerp(end, t);
        points.push(SampledPoint { position });
    }
    points
}

fn sample_cubic_bezier(
    start: Vec2,
    control1: Vec2,
    control2: Vec2,
    end: Vec2,
    spacing: f32,
) -> Vec<SampledPoint> {
    // Build arc-length lookup table
    let n = ARC_LENGTH_SUBDIVISIONS;
    let mut arc_lengths = Vec::with_capacity(n + 1);
    arc_lengths.push(0.0f32);
    let mut prev = start;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let pt = cubic_bezier_point(t, start, control1, control2, end);
        let prev_len = arc_lengths[i - 1];
        arc_lengths.push(prev_len + prev.distance(pt));
        prev = pt;
    }
    let total_length = *arc_lengths.last().unwrap();
    if total_length < f32::EPSILON {
        return vec![SampledPoint { position: start }];
    }

    // Sample at even arc-length intervals
    let count = (total_length / spacing).floor() as usize + 1;
    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        let target_len = (i as f32 * spacing).min(total_length);
        let t = arc_length_to_t(&arc_lengths, target_len, n);
        let position = cubic_bezier_point(t, start, control1, control2, end);
        points.push(SampledPoint { position });
    }
    points
}

/// Binary search the arc-length table to find the t value for a given arc length.
fn arc_length_to_t(arc_lengths: &[f32], target_len: f32, n: usize) -> f32 {
    let mut lo = 0usize;
    let mut hi = n;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if arc_lengths[mid] < target_len {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        return 0.0;
    }
    let seg_start = arc_lengths[lo - 1];
    let seg_end = arc_lengths[lo];
    let seg_len = seg_end - seg_start;
    let frac = if seg_len > f32::EPSILON {
        (target_len - seg_start) / seg_len
    } else {
        0.0
    };
    ((lo - 1) as f32 + frac) / n as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anchor_top() {
        let pos = Vec2::new(100.0, 200.0);
        let size = Vec2::new(80.0, 40.0);
        let pt = resolve_anchor_point(pos, size, 1, Vec2::ZERO);
        assert_eq!(pt, Vec2::new(140.0, 200.0));
    }

    #[test]
    fn test_anchor_right() {
        let pos = Vec2::new(100.0, 200.0);
        let size = Vec2::new(80.0, 40.0);
        let pt = resolve_anchor_point(pos, size, 2, Vec2::ZERO);
        assert_eq!(pt, Vec2::new(180.0, 220.0));
    }

    #[test]
    fn test_anchor_bottom() {
        let pos = Vec2::new(100.0, 200.0);
        let size = Vec2::new(80.0, 40.0);
        let pt = resolve_anchor_point(pos, size, 3, Vec2::ZERO);
        assert_eq!(pt, Vec2::new(140.0, 240.0));
    }

    #[test]
    fn test_anchor_left() {
        let pos = Vec2::new(100.0, 200.0);
        let size = Vec2::new(80.0, 40.0);
        let pt = resolve_anchor_point(pos, size, 4, Vec2::ZERO);
        assert_eq!(pt, Vec2::new(100.0, 220.0));
    }

    #[test]
    fn test_anchor_auto_picks_nearest() {
        let pos = Vec2::new(0.0, 0.0);
        let size = Vec2::new(100.0, 50.0);
        // Other node is far to the right -- should pick right edge midpoint
        let other = Vec2::new(500.0, 25.0);
        let pt = resolve_anchor_point(pos, size, 0, other);
        assert_eq!(pt, Vec2::new(100.0, 25.0)); // right edge midpoint
    }

    #[test]
    fn test_anchor_auto_picks_top() {
        let pos = Vec2::new(0.0, 100.0);
        let size = Vec2::new(100.0, 50.0);
        // Other node is above
        let other = Vec2::new(50.0, -500.0);
        let pt = resolve_anchor_point(pos, size, 0, other);
        assert_eq!(pt, Vec2::new(50.0, 100.0)); // top edge midpoint
    }

    #[test]
    fn test_build_straight_path() {
        let path = build_connection_path(
            Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0), 2,  // from: right anchor
            Vec2::new(200.0, 0.0), Vec2::new(100.0, 50.0), 4, // to: left anchor
            &[],
        );
        match path {
            ConnectionPath::Straight { start, end } => {
                assert_eq!(start, Vec2::new(100.0, 25.0));
                assert_eq!(end, Vec2::new(200.0, 25.0));
            }
            _ => panic!("Expected Straight path"),
        }
    }

    #[test]
    fn test_build_cubic_path() {
        let cps = vec![
            ControlPoint { x: 50.0, y: 0.0 },
            ControlPoint { x: -50.0, y: 0.0 },
        ];
        let path = build_connection_path(
            Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0), 2,
            Vec2::new(300.0, 0.0), Vec2::new(100.0, 50.0), 4,
            &cps,
        );
        match path {
            ConnectionPath::CubicBezier { start, control1, control2, end } => {
                assert_eq!(start, Vec2::new(100.0, 25.0));
                assert_eq!(end, Vec2::new(300.0, 25.0));
                // control1 = from_center + offset = (50,25) + (50,0) = (100, 25)
                assert_eq!(control1, Vec2::new(100.0, 25.0));
                // control2 = to_center + offset = (350,25) + (-50,0) = (300, 25)
                assert_eq!(control2, Vec2::new(300.0, 25.0));
            }
            _ => panic!("Expected CubicBezier path"),
        }
    }

    #[test]
    fn test_straight_path_length() {
        let path = ConnectionPath::Straight {
            start: Vec2::new(0.0, 0.0),
            end: Vec2::new(100.0, 0.0),
        };
        let len = path_length(&path);
        assert!((len - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_straight_sampling() {
        let path = ConnectionPath::Straight {
            start: Vec2::new(0.0, 0.0),
            end: Vec2::new(100.0, 0.0),
        };
        let points = sample_path(&path, 10.0);
        assert_eq!(points.len(), 11); // 0, 10, 20, ..., 100
        // First point at start
        assert!((points[0].position.x - 0.0).abs() < 0.01);
        // Last point at or near 100
        assert!((points[10].position.x - 100.0).abs() < 0.01);
        // All y should be 0
        for p in &points {
            assert!((p.position.y).abs() < 0.01);
        }
    }

    #[test]
    fn test_straight_sampling_diagonal() {
        let path = ConnectionPath::Straight {
            start: Vec2::new(0.0, 0.0),
            end: Vec2::new(30.0, 40.0), // length = 50
        };
        let points = sample_path(&path, 10.0);
        assert_eq!(points.len(), 6); // 0, 10, 20, 30, 40, 50
    }

    #[test]
    fn test_collinear_bezier_length() {
        // Control points on the line -> arc length should equal straight distance
        let path = ConnectionPath::CubicBezier {
            start: Vec2::new(0.0, 0.0),
            control1: Vec2::new(33.33, 0.0),
            control2: Vec2::new(66.67, 0.0),
            end: Vec2::new(100.0, 0.0),
        };
        let len = path_length(&path);
        assert!((len - 100.0).abs() < 0.5, "Expected ~100, got {}", len);
    }

    #[test]
    fn test_curved_bezier_longer_than_straight() {
        // Control points perpendicular -> arc length > straight distance
        let path = ConnectionPath::CubicBezier {
            start: Vec2::new(0.0, 0.0),
            control1: Vec2::new(33.0, 100.0),
            control2: Vec2::new(67.0, -100.0),
            end: Vec2::new(100.0, 0.0),
        };
        let straight_dist = 100.0f32;
        let arc_len = path_length(&path);
        assert!(arc_len > straight_dist, "Arc length {} should exceed straight {}", arc_len, straight_dist);
    }

    #[test]
    fn test_curved_bezier_sampling() {
        let path = ConnectionPath::CubicBezier {
            start: Vec2::new(0.0, 0.0),
            control1: Vec2::new(33.0, 100.0),
            control2: Vec2::new(67.0, -100.0),
            end: Vec2::new(100.0, 0.0),
        };
        let points = sample_path(&path, 10.0);
        // Curved path is longer than 100, so should have more than 11 points
        assert!(points.len() > 11, "Expected >11 points, got {}", points.len());
        // First point near start
        assert!(points[0].position.distance(Vec2::ZERO) < 1.0);
    }

    #[test]
    fn test_sample_path_zero_spacing() {
        let path = ConnectionPath::Straight {
            start: Vec2::new(0.0, 0.0),
            end: Vec2::new(100.0, 0.0),
        };
        let points = sample_path(&path, 0.0);
        assert!(points.is_empty());
    }

    #[test]
    fn test_sample_path_degenerate() {
        // Zero-length path
        let path = ConnectionPath::Straight {
            start: Vec2::new(50.0, 50.0),
            end: Vec2::new(50.0, 50.0),
        };
        let points = sample_path(&path, 10.0);
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn test_build_quadratic_promotion() {
        let cps = vec![ControlPoint { x: 0.0, y: 100.0 }];
        let path = build_connection_path(
            Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0), 0,
            Vec2::new(200.0, 0.0), Vec2::new(100.0, 50.0), 0,
            &cps,
        );
        match path {
            ConnectionPath::CubicBezier { .. } => { /* promoted correctly */ }
            _ => panic!("Expected CubicBezier from quadratic promotion"),
        }
    }
}
