//! Wall / line extrusion — the "3D geometry engine".
//!
//! Rule: a single drafted line (or wall) extrudes to a single vertical
//! **surface** — not a solid box. When the drafted segments form a closed loop
//! in draw order, a floor and ceiling are added too. These surfaces are what the
//! lux engine lights; DXF stays a reference underlay.
use crate::engine::geometry::{Mesh, Point2, Triangle, Vertex, WallSeg};
use crate::model::MaterialId;

const FLOOR: MaterialId = 0;
const WALL: MaterialId = 1;
const CEILING: MaterialId = 2;

/// One vertical surface from the segment `a → b`, extruded to `height`.
fn surface(a: Point2, b: Point2, height: f32) -> Mesh {
    Mesh {
        vertices: vec![
            Vertex::new(a.x, a.y, 0.0),
            Vertex::new(b.x, b.y, 0.0),
            Vertex::new(b.x, b.y, height),
            Vertex::new(a.x, a.y, height),
        ],
        triangles: vec![Triangle { a: 0, b: 1, c: 2 }, Triangle { a: 0, b: 2, c: 3 }],
        material: WALL,
    }
}

/// Extrude each segment to a surface, plus a floor + ceiling if the segments
/// close a loop.
pub fn extrude(walls: &[WallSeg], height: f32) -> Vec<Mesh> {
    let mut meshes: Vec<Mesh> = walls.iter().map(|w| surface(w.start, w.end, height)).collect();

    if let Some(poly) = footprint_loop(walls) {
        let tris = triangulate(&poly);
        if !tris.is_empty() {
            meshes.push(polygon_mesh(&poly, &tris, 0.0, FLOOR));
            meshes.push(polygon_mesh(&poly, &tris, height, CEILING));
        }
    }
    meshes
}

/// The room footprint when the segments form a closed loop in draw order
/// (each segment's end meets the next segment's start). `None` otherwise.
pub fn footprint_loop(walls: &[WallSeg]) -> Option<Vec<Point2>> {
    if walls.len() < 3 {
        return None;
    }
    let close = |a: Point2, b: Point2| (a.x - b.x).hypot(a.y - b.y) < 1e-3;
    for i in 0..walls.len() {
        let next = walls[(i + 1) % walls.len()];
        if !close(walls[i].end, next.start) {
            return None;
        }
    }
    Some(walls.iter().map(|w| w.start).collect())
}

fn polygon_mesh(poly: &[Point2], tris: &[[usize; 3]], z: f32, material: MaterialId) -> Mesh {
    Mesh {
        vertices: poly.iter().map(|p| Vertex::new(p.x, p.y, z)).collect(),
        triangles: tris
            .iter()
            .map(|t| Triangle { a: t[0] as u32, b: t[1] as u32, c: t[2] as u32 })
            .collect(),
        material,
    }
}

// --- ear-clipping triangulation for a simple polygon (no holes) ---

fn signed_area(poly: &[Point2]) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (p, q) = (poly[i], poly[(i + 1) % n]);
            p.x * q.y - q.x * p.y
        })
        .sum::<f32>()
        * 0.5
}

fn cross(o: Point2, a: Point2, b: Point2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

fn in_tri(p: Point2, a: Point2, b: Point2, c: Point2) -> bool {
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Ear-clip a simple polygon into triangle index triples.
pub fn triangulate(poly: &[Point2]) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    let mut idx: Vec<usize> = (0..n).collect();
    if signed_area(poly) < 0.0 {
        idx.reverse(); // work CCW
    }
    let mut tris = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < 10_000 {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (poly[ia], poly[ib], poly[ic]);
            if cross(a, b, c) <= 0.0 {
                continue; // reflex vertex — not an ear
            }
            let mut ear = true;
            for &j in &idx {
                if j != ia && j != ib && j != ic && in_tri(poly[j], a, b, c) {
                    ear = false;
                    break;
                }
            }
            if ear {
                tris.push([ia, ib, ic]);
                idx.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break; // degenerate; stop rather than loop forever
        }
    }
    if idx.len() == 3 {
        tris.push([idx[0], idx[1], idx[2]]);
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(ax: f32, ay: f32, bx: f32, by: f32) -> WallSeg {
        WallSeg { start: Point2 { x: ax, y: ay }, end: Point2 { x: bx, y: by }, thickness: 0.1 }
    }

    #[test]
    fn triangulate_square() {
        let sq = vec![
            Point2 { x: 0.0, y: 0.0 },
            Point2 { x: 1.0, y: 0.0 },
            Point2 { x: 1.0, y: 1.0 },
            Point2 { x: 0.0, y: 1.0 },
        ];
        assert_eq!(triangulate(&sq).len(), 2);
    }

    #[test]
    fn triangulate_l_shape() {
        let l = vec![
            Point2 { x: 0.0, y: 0.0 },
            Point2 { x: 2.0, y: 0.0 },
            Point2 { x: 2.0, y: 1.0 },
            Point2 { x: 1.0, y: 1.0 },
            Point2 { x: 1.0, y: 2.0 },
            Point2 { x: 0.0, y: 2.0 },
        ];
        assert_eq!(triangulate(&l).len(), 4);
    }

    #[test]
    fn square_room_extrudes_surfaces_plus_floor_ceiling() {
        let w = 4.0;
        let walls = vec![
            seg(0.0, 0.0, w, 0.0),
            seg(w, 0.0, w, w),
            seg(w, w, 0.0, w),
            seg(0.0, w, 0.0, 0.0),
        ];
        assert!(footprint_loop(&walls).is_some());
        let m = extrude(&walls, 3.0);
        // 4 wall surfaces + floor + ceiling.
        assert_eq!(m.len(), 6);
        assert!(m.iter().all(|mm| mm.vertices.len() >= 3));
        assert!(m.iter().any(|mm| mm.material == FLOOR));
        assert!(m.iter().any(|mm| mm.material == CEILING));
        assert_eq!(m.iter().filter(|mm| mm.material == WALL).count(), 4);
    }

    #[test]
    fn open_line_extrudes_to_one_surface() {
        let m = extrude(&[seg(0.0, 0.0, 5.0, 0.0)], 3.0);
        assert_eq!(m.len(), 1); // single surface, no floor
        assert_eq!(m[0].material, WALL);
    }
}
