//! cad_solid — a free-3D **parametric solid modeler** for SIMLUX.
//!
//! UI-agnostic pure Rust: a construction-plane system, parametric [`Primitive`]s,
//! and a boolean **feature history** ([`Model`]) evaluated through **csgrs** (BSP
//! CSG) into a neutral triangle soup ([`SolidMesh`]). The standalone sandbox
//! (`cargo run -p cad_solid --example sandbox`) drives it; nothing here depends on
//! egui/eframe.
//!
//! Design: `Model{ features }` is an ordered CSG tree flattened into a history —
//! each [`Feature`] applies its [`Primitive`] (posed on a [`Plane`]) to the running
//! result via a [`BoolOp`]. Params are the identity; the mesh is DERIVED, so
//! editing any param and re-`eval()`ing reproduces the solid (the "generator
//! re-derives" contract). The eventual simLUX wire-in (S5) converts [`SolidMesh`]
//! → `cad_light::Mesh` — the single coupling point.

use glam::{Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

mod csg;
pub mod modify;

/// A flat triangle soup: `positions`/`normals` in lock-step, 3 consecutive
/// entries = one triangle (metres, Z-up, f32). cad_solid's neutral output; the
/// app converts it to `cad_light::Mesh` at wire-in.
#[derive(Clone, Debug, Default)]
pub struct SolidMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

impl SolidMesh {
    /// Number of triangles.
    pub fn tri_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Axis-aligned bounds `(min, max)`, or `None` when empty.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut it = self.positions.iter();
        let first = *it.next()?;
        let (mut mn, mut mx) = (first, first);
        for p in self.positions.iter() {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
        Some((mn, mx))
    }
}

/// Which world plane the active construction plane is parallel to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaneKind {
    XY,
    XZ,
    YZ,
}

impl PlaneKind {
    pub const ALL: [PlaneKind; 3] = [PlaneKind::XY, PlaneKind::XZ, PlaneKind::YZ];
    pub fn label(self) -> &'static str {
        match self {
            PlaneKind::XY => "XY",
            PlaneKind::XZ => "XZ",
            PlaneKind::YZ => "YZ",
        }
    }
}

/// A construction / drawing plane: a world plane pushed `offset` along its normal.
/// (3-point arbitrary planes are S2.)
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Plane {
    pub kind: PlaneKind,
    pub offset: f32,
}

impl Default for Plane {
    fn default() -> Self {
        Self { kind: PlaneKind::XY, offset: 0.0 }
    }
}

impl Plane {
    /// The in-plane `(u, v)` axes. The normal is DERIVED as `u × v`, so the frame
    /// `[u | v | n]` is always right-handed (det +1) — csgrs booleans depend on
    /// consistent winding, and a mirrored (det −1) basis would flip every normal.
    pub fn axes(&self) -> (Vec3, Vec3) {
        match self.kind {
            PlaneKind::XY => (Vec3::X, Vec3::Y),
            PlaneKind::XZ => (Vec3::X, Vec3::Z),
            PlaneKind::YZ => (Vec3::Y, Vec3::Z),
        }
    }

    pub fn normal(&self) -> Vec3 {
        let (u, v) = self.axes();
        u.cross(v).normalize()
    }

    /// World position of the plane's local origin.
    pub fn origin(&self) -> Vec3 {
        self.normal() * self.offset
    }

    /// Local → world transform for a primitive posed at `place`: rotate about the
    /// plane normal (spin), then map local `X→u, Y→v, Z→n` and translate to the
    /// placement point on the plane.
    pub fn world_matrix(&self, place: &Placement) -> Mat4 {
        let (u, v) = self.axes();
        let n = u.cross(v).normalize();
        let o = self.origin() + u * place.u + v * place.v + n * place.lift;
        let basis = Mat4::from_cols(u.extend(0.0), v.extend(0.0), n.extend(0.0), o.extend(1.0));
        basis * Mat4::from_rotation_z(place.spin_deg.to_radians())
    }

    /// World point → this plane's local `(u, v)` (drops the normal component) —
    /// the 3D analog of a 2D pick on the drawing plane, used by the modifiers.
    pub fn to_uv(&self, w: Vec3) -> Vec2 {
        let (u, v) = self.axes();
        let d = w - self.origin();
        Vec2::new(d.dot(u), d.dot(v))
    }

    /// Local `(u, v)` → world point on the plane.
    pub fn from_uv(&self, uv: Vec2) -> Vec3 {
        let (u, v) = self.axes();
        self.origin() + u * uv.x + v * uv.y
    }
}

/// Pose of a primitive on its plane: in-plane `(u, v)`, `lift` along the normal,
/// and `spin` about the normal.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default)]
pub struct Placement {
    pub u: f32,
    pub v: f32,
    pub lift: f32,
    pub spin_deg: f32,
}

/// A parametric leaf shape, built in canonical local coords (footprint centred on
/// the local origin, extruding +Z / sitting on the plane). The S2 primitives
/// (Prism / Step / Ramp / Wall / Column) slot in here.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Primitive {
    /// Rectangular box `w×d` footprint, `h` tall.
    Box { w: f32, d: f32, h: f32 },
    /// Cylinder radius `r`, `h` tall, `sides` facets.
    Cylinder { r: f32, h: f32, sides: u32 },
}

impl Primitive {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Primitive::Box { .. } => "Box",
            Primitive::Cylinder { .. } => "Cylinder",
        }
    }

    /// Local-space axis-aligned bounds `(min, max)` before placement (footprint
    /// centred on the local origin, resting on z = 0).
    pub fn local_aabb(&self) -> (Vec3, Vec3) {
        match *self {
            Primitive::Box { w, d, h } => (Vec3::new(-w / 2.0, -d / 2.0, 0.0), Vec3::new(w / 2.0, d / 2.0, h)),
            Primitive::Cylinder { r, h, .. } => (Vec3::new(-r, -r, 0.0), Vec3::new(r, r, h)),
        }
    }
}

/// A boolean combination operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoolOp {
    Union,
    Difference,
    Intersection,
}

impl BoolOp {
    pub const ALL: [BoolOp; 3] = [BoolOp::Union, BoolOp::Difference, BoolOp::Intersection];
    pub fn label(self) -> &'static str {
        match self {
            BoolOp::Union => "Union",
            BoolOp::Difference => "Difference",
            BoolOp::Intersection => "Intersection",
        }
    }
    pub fn glyph(self) -> &'static str {
        match self {
            BoolOp::Union => "∪",
            BoolOp::Difference => "−",
            BoolOp::Intersection => "∩",
        }
    }
}

/// One step of the CSG history: apply `primitive` (posed on `plane` at `placement`)
/// to the running result with `op`. The FIRST feature is the base (its `op` is
/// ignored — nothing to combine with yet).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Feature {
    pub id: u32,
    pub op: BoolOp,
    pub plane: Plane,
    pub placement: Placement,
    pub primitive: Primitive,
}

impl Feature {
    /// World-space AABB of this feature's primitive, posed on its plane — used for
    /// ray-pick selection and the selection highlight.
    pub fn world_aabb(&self) -> (Vec3, Vec3) {
        let (lmn, lmx) = self.primitive.local_aabb();
        let m = self.plane.world_matrix(&self.placement);
        let mut mn = Vec3::splat(f32::INFINITY);
        let mut mx = Vec3::splat(f32::NEG_INFINITY);
        for i in 0..8 {
            let c = Vec3::new(
                if i & 1 == 0 { lmn.x } else { lmx.x },
                if i & 2 == 0 { lmn.y } else { lmx.y },
                if i & 4 == 0 { lmn.z } else { lmx.z },
            );
            let w = m.transform_point3(c);
            mn = mn.min(w);
            mx = mx.max(w);
        }
        (mn, mx)
    }

    // ── Pure transforms — the 3D analogs of cad_kernel `Geom::translated`/
    //    `rotated`/`scaled`/`mirrored`. Each returns a NEW feature (params are the
    //    identity; the mesh re-derives). The modifier layer routes through these,
    //    exactly as the 2D `apply_*` route through the `Geom` methods.

    /// World position of the feature's local origin (footprint centre on its plane).
    pub fn world_origin(&self) -> Vec3 {
        let (u, v) = self.plane.axes();
        self.plane.origin() + u * self.placement.u + v * self.placement.v + self.plane.normal() * self.placement.lift
    }

    /// Copy of this feature relocated so its local origin sits at world point `w`
    /// (keeps its plane, spin, and primitive — only the placement is recomputed).
    pub fn with_world_origin(&self, w: Vec3) -> Feature {
        let (u, v) = self.plane.axes();
        let d = w - self.plane.origin();
        let mut f = *self;
        f.placement.u = d.dot(u);
        f.placement.v = d.dot(v);
        f.placement.lift = d.dot(self.plane.normal());
        f
    }

    /// Translate by a world vector (MOVE / COPY).
    pub fn translated(&self, world_delta: Vec3) -> Feature {
        self.with_world_origin(self.world_origin() + world_delta)
    }

    /// Rotate `angle_rad` about `pivot` around `axis` (ROTATE). When the feature's
    /// plane normal is parallel to the axis (the coplanar, in-plane case), the spin
    /// is updated too so the primitive turns; otherwise only the origin orbits.
    pub fn rotated(&self, pivot: Vec3, axis: Vec3, angle_rad: f32) -> Feature {
        let axis = axis.normalize_or_zero();
        let o = self.world_origin();
        let q = Quat::from_axis_angle(axis, angle_rad);
        let mut f = self.with_world_origin(pivot + q * (o - pivot));
        let align = self.plane.normal().dot(axis);
        if align.abs() > 0.999 {
            f.placement.spin_deg += angle_rad.to_degrees() * align.signum();
        }
        f
    }

    /// Uniform scale by `k` about `pivot` (SCALE) — moves the origin away from the
    /// pivot and scales the primitive's dimensions.
    pub fn scaled(&self, pivot: Vec3, k: f32) -> Feature {
        let o = self.world_origin();
        let mut f = self.with_world_origin(pivot + (o - pivot) * k);
        match &mut f.primitive {
            Primitive::Box { w, d, h } => {
                *w = (*w * k).max(0.001);
                *d = (*d * k).max(0.001);
                *h = (*h * k).max(0.001);
            }
            Primitive::Cylinder { r, h, .. } => {
                *r = (*r * k).max(0.001);
                *h = (*h * k).max(0.001);
            }
        }
        f
    }

    /// Reflect across the plane through `plane_pt` with unit-ish normal `plane_n`
    /// (MIRROR). The origin reflects; the primitive (axis-symmetric) is unchanged.
    pub fn mirrored(&self, plane_pt: Vec3, plane_n: Vec3) -> Feature {
        let n = plane_n.normalize_or_zero();
        let o = self.world_origin();
        let refl = o - n * (2.0 * (o - plane_pt).dot(n));
        self.with_world_origin(refl)
    }
}

/// Ray vs axis-aligned box (slab method). Returns the entry distance (clamped to
/// ≥ 0) when the ray `orig + t·dir` hits the box, else `None`. `dir` need not be
/// unit. Zero components are handled via `f32` infinities.
pub fn ray_aabb(orig: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let inv = dir.recip();
    let t0 = (min - orig) * inv;
    let t1 = (max - orig) * inv;
    let enter = t0.min(t1).max_element();
    let exit = t0.max(t1).min_element();
    (exit >= enter.max(0.0)).then(|| enter.max(0.0))
}

/// The parametric solid: an ordered boolean feature history.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Model {
    pub features: Vec<Feature>,
}

impl Model {
    /// Append a feature, assigning it a fresh id; returns that id.
    pub fn push(&mut self, op: BoolOp, plane: Plane, placement: Placement, primitive: Primitive) -> u32 {
        let id = self.features.iter().map(|f| f.id).max().map_or(1, |m| m + 1);
        self.features.push(Feature { id, op, plane, placement, primitive });
        id
    }

    /// Next free feature id.
    pub fn next_id(&self) -> u32 {
        self.features.iter().map(|f| f.id).max().map_or(1, |m| m + 1)
    }

    /// Append a fully-formed feature, re-stamping it with a fresh id (COPY).
    pub fn push_feature(&mut self, mut f: Feature) -> u32 {
        let id = self.next_id();
        f.id = id;
        self.features.push(f);
        id
    }

    /// Mutable access to a feature by id.
    pub fn get_mut(&mut self, id: u32) -> Option<&mut Feature> {
        self.features.iter_mut().find(|f| f.id == id)
    }

    /// Remove the feature with `id`; returns true if one was removed.
    pub fn remove(&mut self, id: u32) -> bool {
        let n = self.features.len();
        self.features.retain(|f| f.id != id);
        self.features.len() != n
    }

    /// Fold the whole history through csgrs into a triangle soup. Re-run on ANY
    /// param edit — this IS the parametric re-derivation.
    pub fn eval(&self) -> SolidMesh {
        csg::eval(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_box_ahead_and_misses_beside() {
        let (mn, mx) = (Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        // Ray from -Z straight at the box.
        let hit = ray_aabb(Vec3::new(0.0, 0.0, -5.0), Vec3::Z, mn, mx);
        assert!(hit.is_some());
        assert!((hit.unwrap() - 4.0).abs() < 1e-4, "enters the box at t=4");
        // Ray parallel but offset well outside → miss.
        assert!(ray_aabb(Vec3::new(5.0, 0.0, -5.0), Vec3::Z, mn, mx).is_none());
    }

    #[test]
    fn world_aabb_follows_placement_and_offset() {
        let f = Feature {
            id: 1,
            op: BoolOp::Union,
            plane: Plane { kind: PlaneKind::XY, offset: 2.0 },
            placement: Placement { u: 3.0, v: 0.0, lift: 0.0, spin_deg: 0.0 },
            primitive: Primitive::Box { w: 2.0, d: 2.0, h: 1.0 },
        };
        let (mn, mx) = f.world_aabb();
        // Centred at u=3 on X → x spans 2..4; sits on plane at z=offset=2 → z 2..3.
        assert!((mn.x - 2.0).abs() < 1e-4 && (mx.x - 4.0).abs() < 1e-4);
        assert!((mn.z - 2.0).abs() < 1e-4 && (mx.z - 3.0).abs() < 1e-4);
    }

    fn box_at(u: f32, v: f32) -> Feature {
        Feature {
            id: 1,
            op: BoolOp::Union,
            plane: Plane::default(),
            placement: Placement { u, v, lift: 0.0, spin_deg: 0.0 },
            primitive: Primitive::Box { w: 1.0, d: 1.0, h: 1.0 },
        }
    }

    #[test]
    fn translate_moves_world_origin() {
        let f = box_at(1.0, 1.0).translated(Vec3::new(2.0, 0.0, 0.0));
        let o = f.world_origin();
        assert!((o - Vec3::new(3.0, 1.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn rotate_90_about_z_at_origin() {
        // A feature at (1,0) rotated +90° about Z lands at (0,1), spin +90.
        let f = box_at(1.0, 0.0).rotated(Vec3::ZERO, Vec3::Z, std::f32::consts::FRAC_PI_2);
        let o = f.world_origin();
        assert!((o - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-4, "origin orbits to (0,1), got {o:?}");
        assert!((f.placement.spin_deg - 90.0).abs() < 1e-3, "spin picks up +90°");
    }

    #[test]
    fn scale_2x_doubles_dims_and_pushes_origin() {
        let f = box_at(1.0, 0.0).scaled(Vec3::ZERO, 2.0);
        assert!((f.world_origin() - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-4);
        match f.primitive {
            Primitive::Box { w, d, h } => assert!((w - 2.0).abs() < 1e-4 && (d - 2.0).abs() < 1e-4 && (h - 2.0).abs() < 1e-4),
            _ => panic!("still a box"),
        }
    }
}
