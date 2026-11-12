//! `map.visibility-limits`: squaremap's `VisibilityLimitImpl` with its four shapes
//! (`RectangleShape`, `CircleShape`, `PolygonShape`, `WorldBorderShape`; MIT, Copyright (c)
//! 2021-2024 Jason Penilla). A column, chunk or region is drawn when it is inside any
//! shape, or when there is no shape at all.
use crate::config::VisibilityLimit;

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Inclusive block bounds.
    Rectangle { min_x: i32, min_z: i32, max_x: i32, max_z: i32 },
    Circle { x: i32, z: i32, radius: i32 },
    Polygon { points: Vec<(i32, i32)>, bounds: (i32, i32, i32, i32) },
    /// The world border's `[min_x, max_x, min_z, max_z]` when the shape was resolved
    /// (`WorldBorderShape.Bounds`); unlimited while the border isn't known.
    WorldBorder(Option<[i32; 4]>),
}

/// The world border as squaremap reads it: centre truncated, radius `ceil(size / 2)`.
/// `bounds` is `[min_x, min_z, max_x, max_z]` as the game gives it.
pub fn border_shape(bounds: [f64; 4]) -> [i32; 4] {
    let [x0, z0, x1, z1] = bounds;
    let (cx, cz) = (((x0 + x1) / 2.0) as i32, ((z0 + z1) / 2.0) as i32);
    let radius = ((x1 - x0) / 2.0).ceil().min(f64::from(i32::MAX / 2)) as i32;
    [cx.saturating_sub(radius), cx.saturating_add(radius), cz.saturating_sub(radius), cz.saturating_add(radius)]
}

/// The shapes of a world's config, with the border it has now.
pub fn resolve(limits: &[VisibilityLimit], border: Option<[i32; 4]>) -> Vec<Shape> {
    let mut out = Vec::new();
    for l in limits.iter().filter(|l| l.enabled) {
        match l.kind.as_str() {
            "rectangle" if l.max_x >= l.min_x && l.max_z >= l.min_z => {
                out.push(Shape::Rectangle { min_x: l.min_x, min_z: l.min_z, max_x: l.max_x, max_z: l.max_z })
            }
            "circle" if l.radius > 0 => out.push(Shape::Circle { x: l.center_x, z: l.center_z, radius: l.radius }),
            "polygon" => match parse_points(&l.points) {
                Ok(points) => {
                    let bounds = bounds(&points);
                    out.push(Shape::Polygon { points, bounds });
                }
                Err(e) => eprintln!("[squaremap] polygon visibility limit skipped: {e}"),
            },
            "world-border" => out.push(Shape::WorldBorder(border)),
            "rectangle" | "circle" => {}
            other => eprintln!("[squaremap] unknown visibility limit type '{other}' skipped"),
        }
    }
    out
}

/// `"x, z"` strings (squaremap splits on commas and spaces).
fn parse_points(points: &[String]) -> Result<Vec<(i32, i32)>, String> {
    points
        .iter()
        .map(|s| {
            let n: Vec<i32> = s
                .split([',', ' '])
                .filter(|p| !p.trim().is_empty())
                .map(|p| p.trim().parse::<i32>().map_err(|e| format!("'{s}': {e}")))
                .collect::<Result<_, _>>()?;
            match n[..] {
                [x, z, ..] => Ok((x, z)),
                _ => Err(format!("'{s}' is not a point")),
            }
        })
        .collect()
}

/// `PolygonShape.calculateBounds`: (x, z, width, height).
fn bounds(points: &[(i32, i32)]) -> (i32, i32, i32, i32) {
    let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(x, z) in points {
        x0 = x0.min(x);
        x1 = x1.max(x);
        z0 = z0.min(z);
        z1 = z1.max(z);
    }
    (x0, z0, x1.wrapping_sub(x0), z1.wrapping_sub(z0))
}

impl Shape {
    pub fn column(&self, x: i32, z: i32) -> bool {
        match self {
            Shape::Rectangle { min_x, min_z, max_x, max_z } => (*min_x..=*max_x).contains(&x) && (*min_z..=*max_z).contains(&z),
            Shape::Circle { x: cx, z: cz, radius } => {
                let (dx, dz) = (i64::from(x) - i64::from(*cx), i64::from(z) - i64::from(*cz));
                dx * dx + dz * dz <= i64::from(*radius) * i64::from(*radius)
            }
            Shape::Polygon { points, bounds } => polygon_contains(points, *bounds, f64::from(x), f64::from(z)),
            Shape::WorldBorder(None) => true,
            Shape::WorldBorder(Some([x0, x1, z0, z1])) => x >= *x0 && x < *x1 && z >= *z0 && z < *z1,
        }
    }

    pub fn chunk(&self, cx: i32, cz: i32) -> bool {
        match self {
            Shape::Rectangle { min_x, min_z, max_x, max_z } => {
                (min_x >> 4..=max_x >> 4).contains(&cx) && (min_z >> 4..=max_z >> 4).contains(&cz)
            }
            Shape::Circle { x, z, radius } => circle_corner(*x, *z, *radius, cx, cz, 16),
            Shape::Polygon { .. } => (0..16).any(|dx| (0..16).any(|dz| self.column(cx * 16 + dx, cz * 16 + dz))),
            Shape::WorldBorder(None) => true,
            Shape::WorldBorder(Some([x0, x1, z0, z1])) => {
                cx >= x0 >> 4 && cx <= x1 >> 4 && cz >= z0 >> 4 && cz <= z1 >> 4
            }
        }
    }

    pub fn region(&self, rx: i32, rz: i32) -> bool {
        match self {
            Shape::Rectangle { min_x, min_z, max_x, max_z } => {
                (min_x >> 9..=max_x >> 9).contains(&rx) && (min_z >> 9..=max_z >> 9).contains(&rz)
            }
            Shape::Circle { x, z, radius } => circle_corner(*x, *z, *radius, rx, rz, 512),
            Shape::Polygon { .. } => (0..32).any(|dx| (0..32).any(|dz| self.chunk(rx * 32 + dx, rz * 32 + dz))),
            Shape::WorldBorder(None) => true,
            Shape::WorldBorder(Some([x0, x1, z0, z1])) => {
                rx >= x0 >> 9 && rx <= x1 >> 9 && rz >= z0 >> 9 && rz <= z1 >> 9
            }
        }
    }
}

/// `CircleShape.shouldRenderChunk`/`shouldRenderRegion`: the corner of the square
/// (`size` blocks) nearest the centre, moved only towards a centre to its east or south,
/// as squaremap does.
fn circle_corner(x: i32, z: i32, radius: i32, sx: i32, sz: i32, size: i64) -> bool {
    let (x, z) = (i64::from(x), i64::from(z));
    let mut bx = i64::from(sx) * size;
    let mut bz = i64::from(sz) * size;
    if bx < x {
        bx += (size - 1).min(x - bx);
    }
    if bz < z {
        bz += (size - 1).min(z - bz);
    }
    (bx - x) * (bx - x) + (bz - z) * (bz - z) <= i64::from(radius) * i64::from(radius)
}

/// `PolygonShape.contains`: java.awt.Polygon's even-odd crossing test.
fn polygon_contains(points: &[(i32, i32)], (bx, bz, bw, bh): (i32, i32, i32, i32), x: f64, y: f64) -> bool {
    let n = points.len();
    let (x0, y0) = (f64::from(bx), f64::from(bz));
    if n <= 2 || !(x >= x0 && y >= y0 && x < x0 + f64::from(bw) && y < y0 + f64::from(bh)) {
        return false;
    }
    let mut hits = 0;
    let (mut lastx, mut lasty) = points[n - 1];
    for &(curx, cury) in points {
        let (cx, cy, lx, ly) = (f64::from(curx), f64::from(cury), f64::from(lastx), f64::from(lasty));
        let step = (|| {
            if cury == lasty {
                return 0;
            }
            let leftx;
            if curx < lastx {
                if x >= lx {
                    return 0;
                }
                leftx = cx;
            } else {
                if x >= cx {
                    return 0;
                }
                leftx = lx;
            }
            let (test1, test2);
            if cury < lasty {
                if y < cy || y >= ly {
                    return 0;
                }
                if x < leftx {
                    return 1;
                }
                test1 = x - cx;
                test2 = y - cy;
            } else {
                if y < ly || y >= cy {
                    return 0;
                }
                if x < leftx {
                    return 1;
                }
                test1 = x - lx;
                test2 = y - ly;
            }
            i32::from(test1 < test2 / (ly - cy) * (lx - cx))
        })();
        hits += step;
        lastx = curx;
        lasty = cury;
    }
    hits & 1 != 0
}

/// Whether anything is drawn there: inside any shape, or no shapes.
pub fn any(shapes: &[Shape], f: impl Fn(&Shape) -> bool) -> bool {
    shapes.is_empty() || shapes.iter().any(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(kind: &str) -> VisibilityLimit {
        VisibilityLimit { kind: kind.into(), ..VisibilityLimit::default() }
    }

    #[test]
    fn rectangle_is_inclusive_and_bad_ones_are_dropped() {
        let mut r = limit("rectangle");
        (r.min_x, r.min_z, r.max_x, r.max_z) = (-16, 0, 15, 31);
        let s = &resolve(&[r.clone()], None)[0];
        assert!(s.column(15, 31) && s.column(-16, 0) && !s.column(16, 0));
        assert!(s.chunk(-1, 1) && !s.chunk(1, 0) && s.region(-1, 0) && !s.region(1, 0));
        r.max_x = -20;
        assert!(resolve(&[r], None).is_empty());
    }

    #[test]
    fn circle_follows_squaremap() {
        let mut c = limit("circle");
        (c.center_x, c.center_z, c.radius) = (0, 0, 20);
        let s = &resolve(&[c], None)[0];
        assert!(s.column(20, 0) && !s.column(15, 15));
        // Chunk (1, 1) starts at (16, 16): 16^2 + 16^2 > 400.
        assert!(s.chunk(0, 0) && s.chunk(1, 0) && !s.chunk(1, 1));
        // Chunk (-2, 0) spans -32..-17: its nearest corner moves to x = -17.
        assert!(s.chunk(-2, 0) && !s.chunk(-3, 0));
        assert!(s.region(0, 0) && s.region(-1, -1) && !s.region(1, 0));
    }

    #[test]
    fn polygon_is_awts_even_odd_rule() {
        let mut p = limit("polygon");
        p.points = vec!["0, 0".into(), "10,0".into(), "10 10".into(), "0, 10".into()];
        let s = &resolve(&[p], None)[0];
        assert!(s.column(0, 0) && s.column(9, 9) && !s.column(10, 5) && !s.column(-1, 5));
        assert!(s.chunk(0, 0) && !s.chunk(1, 0) && s.region(0, 0) && !s.region(0, 1));
        let mut tri = limit("polygon");
        tri.points = vec!["0,0".into(), "20,0".into(), "0,20".into()];
        let t = &resolve(&[tri], None)[0];
        assert!(t.column(2, 2) && !t.column(15, 15));
        let mut bad = limit("polygon");
        bad.points = vec!["x, 1".into()];
        assert!(resolve(&[bad], None).is_empty());
    }

    #[test]
    fn world_border_uses_the_border_now_and_disabled_shapes_are_skipped() {
        let b = border_shape([-100.5, -50.0, 99.5, 150.0]);
        assert_eq!(b, [-100, 100, -50, 150]);
        let s = &resolve(&[limit("world-border")], Some(b))[0];
        assert!(s.column(-100, -50) && !s.column(100, 0) && s.chunk(6, 9) && !s.chunk(7, 0));
        assert!(resolve(&[limit("world-border")], None)[0].column(1 << 24, 0));
        let mut off = limit("circle");
        (off.radius, off.enabled) = (5, false);
        assert!(resolve(&[off], None).is_empty());
        assert!(any(&[], |_| false));
    }
}
