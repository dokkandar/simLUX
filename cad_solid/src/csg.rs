//! The csgrs boundary — the ONLY place glam↔nalgebra conversion happens.
//!
//! `Model` (glam f32, UI-facing) → csgrs `Mesh<()>` (nalgebra f64, BSP CSG) →
//! [`SolidMesh`] (glam f32, render/wire-in). Keeping this in one file means the
//! rest of `cad_solid` never sees nalgebra or csgrs.

use csgrs::csg::CSG;
use csgrs::mesh::Mesh;
use nalgebra::{Matrix4, Vector3};

use crate::{BoolOp, Feature, Model, Primitive, SolidMesh};

/// csgrs mesh with no per-face metadata.
type CsgMesh = Mesh<()>;

/// glam `Mat4` (f32, column-major) → nalgebra `Matrix4<f64>` (also column-major).
fn to_na(m: glam::Mat4) -> Matrix4<f64> {
    let c = m.to_cols_array();
    Matrix4::from_column_slice(&c.map(|x| x as f64))
}

/// Build a primitive as a csgrs mesh in canonical LOCAL coords: footprint centred
/// on the local origin, resting on the plane (local z = 0) and rising +Z.
fn local_mesh(p: &Primitive) -> CsgMesh {
    match *p {
        Primitive::Box { w, d, h } => {
            // csgrs cuboid is corner-anchored at the origin → recentre in u,v so it
            // sits centred on the placement point (base still on the plane).
            let m = CsgMesh::cuboid(w as f64, d as f64, h as f64, ());
            m.transform(&Matrix4::new_translation(&Vector3::new(
                -(w as f64) / 2.0,
                -(d as f64) / 2.0,
                0.0,
            )))
        }
        // csgrs cylinder already rises +Z from a base at z=0, centred on the axis.
        Primitive::Cylinder { r, h, sides } => {
            CsgMesh::cylinder(r as f64, h as f64, sides.max(3) as usize, ())
        }
    }
}

/// A feature's primitive, placed into world coords on its plane.
fn world_mesh(f: &Feature) -> CsgMesh {
    let local = local_mesh(&f.primitive);
    local.transform(&to_na(f.plane.world_matrix(&f.placement)))
}

/// Fold the feature history left→right through csgrs. The first feature is the
/// base; each subsequent one combines via its [`BoolOp`].
pub fn eval(model: &Model) -> SolidMesh {
    let mut acc: Option<CsgMesh> = None;
    for f in &model.features {
        let m = world_mesh(f);
        acc = Some(match acc {
            None => m,
            Some(a) => match f.op {
                BoolOp::Union => a.union(&m),
                BoolOp::Difference => a.difference(&m),
                BoolOp::Intersection => a.intersection(&m),
            },
        });
    }
    acc.as_ref().map(to_solid).unwrap_or_default()
}

/// csgrs mesh → flat triangle soup (fan-triangulating each polygon).
fn to_solid(m: &CsgMesh) -> SolidMesh {
    let mut out = SolidMesh::default();
    for poly in &m.polygons {
        for tri in poly.triangulate() {
            for v in tri {
                let p = v.position.coords;
                let n = v.normal;
                out.positions.push([p.x as f32, p.y as f32, p.z as f32]);
                out.normals.push([n.x as f32, n.y as f32, n.z as f32]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Placement, Plane, PlaneKind};

    fn boxf(w: f32, d: f32, h: f32) -> Primitive {
        Primitive::Box { w, d, h }
    }

    #[test]
    fn single_box_has_12_triangles() {
        let mut m = Model::default();
        m.push(BoolOp::Union, Plane::default(), Placement::default(), boxf(2.0, 2.0, 1.0));
        let mesh = m.eval();
        // A box = 6 quad faces = 12 triangles.
        assert_eq!(mesh.tri_count(), 12, "a plain box should tessellate to 12 tris");
    }

    #[test]
    fn difference_adds_geometry_and_stays_bounded() {
        // 2×2×2 box minus a 1×1 cylinder punched through the top.
        let mut m = Model::default();
        m.push(BoolOp::Union, Plane::default(), Placement::default(), boxf(2.0, 2.0, 2.0));
        m.push(
            BoolOp::Difference,
            Plane::default(),
            Placement { u: 0.0, v: 0.0, lift: 0.5, spin_deg: 0.0 },
            Primitive::Cylinder { r: 0.5, h: 2.0, sides: 24 },
        );
        let cut = m.eval();
        let plain = {
            let mut b = Model::default();
            b.push(BoolOp::Union, Plane::default(), Placement::default(), boxf(2.0, 2.0, 2.0));
            b.eval()
        };
        // Subtracting a hole cannot yield fewer triangles than the plain box.
        assert!(cut.tri_count() > plain.tri_count(), "difference should add cut geometry");
        // Result stays within the original box footprint (±small epsilon).
        let (mn, mx) = cut.bounds().expect("non-empty");
        assert!(mn[0] >= -1.01 && mx[0] <= 1.01, "x within box");
        assert!(mn[1] >= -1.01 && mx[1] <= 1.01, "y within box");
    }

    #[test]
    fn eval_is_deterministic() {
        let mut m = Model::default();
        m.push(BoolOp::Union, Plane { kind: PlaneKind::XZ, offset: 0.3 }, Placement::default(), boxf(1.0, 1.0, 1.0));
        let a = m.eval();
        let b = m.eval();
        assert_eq!(a.positions, b.positions, "same model must yield the same mesh");
    }
}
