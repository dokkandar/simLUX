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

use cad_kernel::Vec2 as KVec2;
use glam::{Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

mod csg;
pub mod dbg_recorder; // copied VERBATIM from cad_app (identical to RUST_CAD's recorder)
pub mod draw;
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
    /// Sphere. `segments` = longitude slices, `stacks` = latitude rings.
    /// Rests ON the plane (lifted by `r`), per this module's convention.
    Sphere { r: f32, segments: u32, stacks: u32 },
    /// Cone / frustum / prism / pyramid — one primitive, four shapes:
    /// `r_top = 0` → **cone** · `r_top = r_bottom` → **prism** ·
    /// `sides = 4, r_top = 0` → **pyramid** · else → **frustum**.
    /// (This is why there is no separate Cone/Prism/Pyramid variant.)
    Frustum { r_bottom: f32, r_top: f32, h: f32, sides: u32 },
    /// Torus — `major_r` = ring radius, `minor_r` = tube thickness.
    Torus { major_r: f32, minor_r: f32, seg_major: u32, seg_minor: u32 },
    /// Capsule — a cylinder of length `h` with hemispherical caps of radius `r`.
    /// **COMPOSED** (csgrs has no capsule): cylinder ∪ sphere ∪ sphere.
    /// Total height = `h + 2r`.
    Capsule { r: f32, h: f32, segments: u32, stacks: u32 },
    /// Hollow tube — **COMPOSED** (csgrs has no tube): outer cylinder ∖ inner.
    Tube { r_outer: f32, r_inner: f32, h: f32, sides: u32 },
    /// Ellipsoid with independent radii. Rests on the plane (lifted by `rz`).
    Ellipsoid { rx: f32, ry: f32, rz: f32, segments: u32, stacks: u32 },
}

impl Primitive {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Primitive::Box { .. } => "Box",
            Primitive::Cylinder { .. } => "Cylinder",
            Primitive::Sphere { .. } => "Sphere",
            Primitive::Frustum { r_top, sides, r_bottom, .. } => {
                // one variant, four shapes — name it by what it actually is
                if *r_top <= 1e-6 {
                    if *sides == 4 { "Pyramid" } else { "Cone" }
                } else if (*r_top - *r_bottom).abs() <= 1e-6 {
                    "Prism"
                } else {
                    "Frustum"
                }
            }
            Primitive::Torus { .. } => "Torus",
            Primitive::Capsule { .. } => "Capsule",
            Primitive::Tube { .. } => "Tube",
            Primitive::Ellipsoid { .. } => "Ellipsoid",
        }
    }

    /// Local-space axis-aligned bounds `(min, max)` before placement (footprint
    /// centred on the local origin, resting on z = 0).
    pub fn local_aabb(&self) -> (Vec3, Vec3) {
        match *self {
            Primitive::Box { w, d, h } => (Vec3::new(-w / 2.0, -d / 2.0, 0.0), Vec3::new(w / 2.0, d / 2.0, h)),
            Primitive::Cylinder { r, h, .. } => (Vec3::new(-r, -r, 0.0), Vec3::new(r, r, h)),
            // sphere/ellipsoid are lifted so they REST on the plane
            Primitive::Sphere { r, .. } => (Vec3::new(-r, -r, 0.0), Vec3::new(r, r, 2.0 * r)),
            Primitive::Ellipsoid { rx, ry, rz, .. } => {
                (Vec3::new(-rx, -ry, 0.0), Vec3::new(rx, ry, 2.0 * rz))
            }
            Primitive::Frustum { r_bottom, r_top, h, .. } => {
                let r = r_bottom.max(r_top);
                (Vec3::new(-r, -r, 0.0), Vec3::new(r, r, h))
            }
            // torus lies flat, lifted by minor_r → rests on the plane
            Primitive::Torus { major_r, minor_r, .. } => {
                let o = major_r + minor_r;
                (Vec3::new(-o, -o, 0.0), Vec3::new(o, o, 2.0 * minor_r))
            }
            // capsule: caps of r at each end of an h-long barrel → total h + 2r
            Primitive::Capsule { r, h, .. } => (Vec3::new(-r, -r, 0.0), Vec3::new(r, r, h + 2.0 * r)),
            Primitive::Tube { r_outer, h, .. } => {
                (Vec3::new(-r_outer, -r_outer, 0.0), Vec3::new(r_outer, r_outer, h))
            }
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
            // Uniform scale: every LENGTH scales, every SEGMENT COUNT does not.
            Primitive::Sphere { r, .. } => *r = (*r * k).max(0.001),
            Primitive::Frustum { r_bottom, r_top, h, .. } => {
                *r_bottom = (*r_bottom * k).max(0.0); // 0 is legal — that IS a cone
                *r_top = (*r_top * k).max(0.0);
                *h = (*h * k).max(0.001);
            }
            Primitive::Torus { major_r, minor_r, .. } => {
                *major_r = (*major_r * k).max(0.001);
                *minor_r = (*minor_r * k).max(0.001);
            }
            Primitive::Capsule { r, h, .. } => {
                *r = (*r * k).max(0.001);
                *h = (*h * k).max(0.0); // 0 is legal — that IS a sphere
            }
            Primitive::Tube { r_outer, r_inner, h, .. } => {
                *r_outer = (*r_outer * k).max(0.001);
                *r_inner = (*r_inner * k).max(0.0);
                *h = (*h * k).max(0.001);
            }
            Primitive::Ellipsoid { rx, ry, rz, .. } => {
                *rx = (*rx * k).max(0.001);
                *ry = (*ry * k).max(0.001);
                *rz = (*rz * k).max(0.001);
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

/// Ray vs triangle (Möller–Trumbore). Returns the forward hit distance, or `None`.
/// Used to pick a solid's **surface** (and its face normal) for sketch-on-face.
pub fn ray_triangle(orig: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-7 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = orig - a;
    let u = tv.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = tv.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let dist = e2.dot(q) * inv;
    (dist > 1e-6).then_some(dist)
}

/// The **face** containing triangle `start`: all triangles reachable from it
/// through shared edges whose normal matches (a maximal coplanar-connected
/// region of the surface mesh). `positions` is the flat triangle soup (3 per
/// tri). Returns the triangle indices of the face. Used for face selection.
pub fn coplanar_face(positions: &[[f32; 3]], start: usize) -> Vec<usize> {
    use std::collections::HashMap;
    let ntri = positions.len() / 3;
    if start >= ntri {
        return Vec::new();
    }
    // Weld vertices onto a grid so shared edges match despite float noise.
    let key = |p: [f32; 3]| -> (i64, i64, i64) {
        let q = 1.0e4;
        ((p[0] as f64 * q).round() as i64, (p[1] as f64 * q).round() as i64, (p[2] as f64 * q).round() as i64)
    };
    let edge = |t: usize, e: usize| {
        let (mut a, mut b) = (key(positions[3 * t + e]), key(positions[3 * t + (e + 1) % 3]));
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        (a, b)
    };
    let normal = |t: usize| -> Vec3 {
        let a = Vec3::from(positions[3 * t]);
        let b = Vec3::from(positions[3 * t + 1]);
        let c = Vec3::from(positions[3 * t + 2]);
        (b - a).cross(c - a).normalize_or_zero()
    };
    // Edge → adjacent triangles.
    let mut edges: HashMap<((i64, i64, i64), (i64, i64, i64)), Vec<usize>> = HashMap::new();
    for t in 0..ntri {
        for e in 0..3 {
            edges.entry(edge(t, e)).or_default().push(t);
        }
    }
    let n0 = normal(start);
    let mut visited = vec![false; ntri];
    let mut face = Vec::new();
    let mut stack = vec![start];
    visited[start] = true;
    while let Some(t) = stack.pop() {
        face.push(t);
        for e in 0..3 {
            if let Some(adj) = edges.get(&edge(t, e)) {
                for &nt in adj {
                    if !visited[nt] && normal(nt).dot(n0) > 0.999 {
                        visited[nt] = true;
                        stack.push(nt);
                    }
                }
            }
        }
    }
    face
}

/// A free (arbitrary) construction plane: an orthonormal `(u, v)` frame at an
/// `origin` in world space. Unlike [`Plane`] (locked to XY/XZ/YZ), a `Frame` sits
/// on any picked surface — the basis for **sketch-on-face**.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub origin: Vec3,
    pub u: Vec3,
    pub v: Vec3,
}

impl Frame {
    /// Build a right-handed frame (`u × v = normal`) at `origin` facing `normal`.
    /// The in-plane axes are chosen deterministically from a world reference.
    pub fn from_point_normal(origin: Vec3, normal: Vec3) -> Self {
        let n = normal.normalize_or_zero();
        let reference = if n.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
        let u = reference.cross(n).normalize_or_zero();
        let v = n.cross(u).normalize_or_zero();
        Self { origin, u, v }
    }
    pub fn normal(&self) -> Vec3 {
        self.u.cross(self.v).normalize_or_zero()
    }
    pub fn to_uv(&self, w: Vec3) -> Vec2 {
        let d = w - self.origin;
        Vec2::new(d.dot(self.u), d.dot(self.v))
    }
    pub fn from_uv(&self, uv: Vec2) -> Vec3 {
        self.origin + self.u * uv.x + self.v * uv.y
    }
}

/// A 2D sketch on a [`Frame`] (typically a picked solid face). It holds a real
/// `cad_kernel::Document` — the SAME 2D document the main app edits — so the app's
/// draw / modifier / osnap code operates on it unchanged (flat sketch mode).
#[derive(Clone)]
pub struct Sketch {
    pub frame: Frame,
    pub doc: cad_kernel::Document,
    /// Reference geometry (the selected face's outline, in u,v) — shown faintly so
    /// the user sees WHERE they're drawing, and offered as osnap targets. Not part
    /// of the drawing itself.
    pub reference: Vec<cad_kernel::Geom>,
}

impl Sketch {
    pub fn new(frame: Frame) -> Self {
        Self { frame, doc: cad_kernel::Document::default(), reference: Vec::new() }
    }
}

/// Flatten a kernel geom into uv-space polyline paths (each inner `Vec` is one
/// path; closed shapes include the wrap point). Reuses the kernel's own geometry
/// (arc/ellipse samplers + DXF bulge). Point returns a small cross (two paths).
pub fn geom_outlines(g: &cad_kernel::Geom) -> Vec<Vec<Vec2>> {
    use cad_kernel::Geom as G;
    match g {
        G::Line(l) => vec![vec![gvec(l.a), gvec(l.b)]],
        G::Circle(c) => vec![circle_path(c.center, c.radius, 64)],
        G::Arc(a) => vec![arc_path(a.center, a.radius, a.start_angle, a.sweep_angle, 48)],
        G::Ellipse(e) => {
            let n = 72;
            vec![(0..=n).map(|i| gvec(e.point_at(std::f64::consts::TAU * i as f64 / n as f64))).collect()]
        }
        G::EllipseArc(ea) => {
            let n = 48;
            vec![(0..=n)
                .map(|i| gvec(ea.ellipse.point_at(ea.start_param + ea.sweep_param * i as f64 / n as f64)))
                .collect()]
        }
        G::Polyline(p) => vec![polyline_path(p)],
        G::Point(pt) => {
            let c = gvec(pt.location);
            let s = 0.15;
            vec![
                vec![c - Vec2::new(s, 0.0), c + Vec2::new(s, 0.0)],
                vec![c - Vec2::new(0.0, s), c + Vec2::new(0.0, s)],
            ]
        }
        _ => Vec::new(),
    }
}

#[inline]
fn gvec(p: KVec2) -> Vec2 {
    Vec2::new(p.x as f32, p.y as f32)
}

fn circle_path(c: KVec2, r: f64, n: usize) -> Vec<Vec2> {
    (0..=n)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            gvec(KVec2::new(c.x + r * t.cos(), c.y + r * t.sin()))
        })
        .collect()
}

fn arc_path(c: KVec2, r: f64, start: f64, sweep: f64, n: usize) -> Vec<Vec2> {
    (0..=n)
        .map(|i| {
            let t = start + sweep * i as f64 / n as f64;
            gvec(KVec2::new(c.x + r * t.cos(), c.y + r * t.sin()))
        })
        .collect()
}

/// Flatten a (possibly bulged / closed) polyline into a single uv path.
fn polyline_path(p: &cad_kernel::Polyline) -> Vec<Vec2> {
    let n = p.vertices.len();
    if n == 0 {
        return Vec::new();
    }
    let mut path = Vec::new();
    let segs = if p.closed { n } else { n - 1 };
    for i in 0..segs {
        let a = p.vertices[i];
        let b = p.vertices[(i + 1) % n];
        if a.bulge.abs() < 1e-9 {
            path.push(gvec(a.pos));
        } else if let Some((c, r, sa, sw)) = cad_kernel::bulge_arc(a.pos, b.pos, a.bulge) {
            let steps = 16;
            for k in 0..steps {
                let t = sa + sw * k as f64 / steps as f64;
                path.push(gvec(KVec2::new(c.x + r * t.cos(), c.y + r * t.sin())));
            }
        } else {
            path.push(gvec(a.pos));
        }
    }
    path.push(gvec(p.vertices[if p.closed { 0 } else { n - 1 }].pos));
    path
}

/// The parametric solid: an ordered boolean feature history, plus any 2D sketches
/// drawn on faces (not yet persisted — sketch serialization arrives with S5).
/// (No `Debug` derive — `cad_kernel::Document` on `Sketch` isn't `Debug`.)
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub features: Vec<Feature>,
    #[serde(skip)]
    pub sketches: Vec<Sketch>,
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

    #[test]
    fn ray_triangle_hits_and_misses() {
        // Triangle in the z=1 plane; ray from below straight up hits at t=1.
        let (a, b, c) = (Vec3::new(-1.0, -1.0, 1.0), Vec3::new(1.0, -1.0, 1.0), Vec3::new(0.0, 1.0, 1.0));
        let t = ray_triangle(Vec3::ZERO, Vec3::Z, a, b, c);
        assert!(t.map_or(false, |t| (t - 1.0).abs() < 1e-4));
        // A ray off to the side misses.
        assert!(ray_triangle(Vec3::new(5.0, 5.0, 0.0), Vec3::Z, a, b, c).is_none());
    }

    #[test]
    fn frame_uv_round_trips_on_a_tilted_face() {
        let f = Frame::from_point_normal(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 1.0, 0.0));
        let uv = Vec2::new(0.7, -1.3);
        let back = f.to_uv(f.from_uv(uv));
        assert!((back - uv).length() < 1e-4, "uv→world→uv is identity");
        // The reconstructed world point lies in the plane (normal component ~0).
        let w = f.from_uv(uv);
        assert!((w - f.origin).dot(f.normal()).abs() < 1e-4);
    }

    #[test]
    fn coplanar_face_groups_only_the_same_plane() {
        // Two coplanar tris (z=0 quad) + one tri on the x=0 plane sharing an edge.
        let positions = vec![
            [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], // tri 0 (z=0)
            [0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], // tri 1 (z=0, adjacent)
            [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], // tri 2 (x=0)
        ];
        let face = coplanar_face(&positions, 0);
        assert_eq!(face.len(), 2, "the z=0 face is the two coplanar tris");
        assert!(face.contains(&0) && face.contains(&1));
        assert!(!face.contains(&2), "the perpendicular tri is a different face");
    }

    #[test]
    fn circle_geom_outlines_to_a_closed_loop() {
        let g = cad_kernel::Geom::Circle(cad_kernel::Circle { center: KVec2::new(0.0, 0.0), radius: 2.0 });
        let paths = geom_outlines(&g);
        assert_eq!(paths.len(), 1);
        let p = &paths[0];
        assert!(p.len() > 8);
        // first ≈ last (closed) and every point is ~radius from centre.
        assert!((p[0] - p[p.len() - 1]).length() < 1e-3);
        assert!(p.iter().all(|q| (q.length() - 2.0).abs() < 1e-3));
    }
}
