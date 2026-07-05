//! Wall system — the "3D geometry engine".
//!
//! Turns user-drawn 2D walls into the room meshes the lux engine lights. Walls
//! are stitched at their shared nodes with `cad_wall::solve_faces` (the same
//! mitre solver used in Auto_RASM), then extruded — sides, tops, and a
//! triangulated floor/ceiling — to the room height. DXF stays a reference
//! underlay; the lit geometry comes from here.
use cad_kernel::{wall_sides, Vec2, Wall as KWall};
use cad_wall::{solve_faces, WallFaces};

use crate::engine::geometry::{Mesh, Point2, Triangle, Vertex, WallSeg};
use crate::model::MaterialId;

const FLOOR: MaterialId = 0;
const WALL: MaterialId = 1;
const CEILING: MaterialId = 2;

fn kwall(w: &WallSeg) -> KWall {
    KWall {
        start: Vec2::new(w.start.x as f64, w.start.y as f64),
        end: Vec2::new(w.end.x as f64, w.end.y as f64),
        thickness: w.thickness as f64,
        style: 0,
        bulge: 0.0,
    }
}

fn vtx(p: Vec2, z: f32) -> Vertex {
    Vertex::new(p.x as f32, p.y as f32, z)
}

/// Vertical quad from ground edge `a → b` up to `height`.
fn wall_quad(a: Vec2, b: Vec2, height: f32) -> Mesh {
    Mesh {
        vertices: vec![vtx(a, 0.0), vtx(b, 0.0), vtx(b, height), vtx(a, height)],
        triangles: vec![Triangle { a: 0, b: 1, c: 2 }, Triangle { a: 0, b: 2, c: 3 }],
        material: WALL,
    }
}

/// Horizontal quad (a wall top) at height `z`.
fn top_quad(a: Vec2, b: Vec2, c: Vec2, d: Vec2, z: f32) -> Mesh {
    Mesh {
        vertices: vec![vtx(a, z), vtx(b, z), vtx(c, z), vtx(d, z)],
        triangles: vec![Triangle { a: 0, b: 1, c: 2 }, Triangle { a: 0, b: 2, c: 3 }],
        material: WALL,
    }
}

/// Mitred faces for one wall — falls back to a plain centreline offset if the
/// junction solver bows out (degenerate input).
fn faces_of(w: &KWall, all: &[KWall]) -> WallFaces {
    if let Some(f) = solve_faces(w, all) {
        return f;
    }
    if let Some((l, r)) = wall_sides(w.start, w.end, w.thickness) {
        return WallFaces { left: (l.a, l.b), right: (r.a, r.b) };
    }
    WallFaces { left: (w.start, w.end), right: (w.start, w.end) }
}

/// Extrude stitched walls plus floor + ceiling into meshes.
pub fn extrude(walls: &[WallSeg], height: f32) -> Vec<Mesh> {
    let mut meshes = Vec::new();
    let kwalls: Vec<KWall> = walls.iter().map(kwall).collect();

    for kw in &kwalls {
        let f = faces_of(kw, &kwalls);
        let (l0, l1) = f.left;
        let (r0, r1) = f.right;
        meshes.push(wall_quad(l0, l1, height)); // left face
        meshes.push(wall_quad(r0, r1, height)); // right face
        meshes.push(wall_quad(l0, r0, height)); // start cap
        meshes.push(wall_quad(l1, r1, height)); // end cap
        meshes.push(top_quad(l0, l1, r1, r0, height)); // top
    }

    if let Some(poly) = footprint_loop(walls) {
        let tris = triangulate(&poly);
        if !tris.is_empty() {
            meshes.push(polygon_mesh(&poly, &tris, 0.0, FLOOR));
            meshes.push(polygon_mesh(&poly, &tris, height, CEILING));
        }
    }
    meshes
}

/// The room footprint when the walls form a closed loop in draw order
/// (each wall's end meets the next wall's start). `None` otherwise.
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
        WallSeg { start: Point2 { x: ax, y: ay }, end: Point2 { x: bx, y: by }, thickness: 0.2 }
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
        // Concave (L) polygon → 4 triangles.
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
    fn square_room_extrudes_with_floor_and_ceiling() {
        let w = 4.0;
        let walls = vec![
            seg(0.0, 0.0, w, 0.0),
            seg(w, 0.0, w, w),
            seg(w, w, 0.0, w),
            seg(0.0, w, 0.0, 0.0),
        ];
        assert!(footprint_loop(&walls).is_some());
        let m = extrude(&walls, 3.0);
        // 5 quads per wall + floor + ceiling.
        assert!(m.len() >= 4 * 5 + 2, "got {} meshes", m.len());
        assert!(m.iter().any(|mm| mm.material == FLOOR && !mm.triangles.is_empty()));
        assert!(m.iter().any(|mm| mm.material == CEILING && !mm.triangles.is_empty()));
    }
}
