//! 3D Factory — the `cad_solid` 3D solid layer, wired into the real app.
//!
//! This is the sandbox's core (`cad_solid/examples/sandbox.rs`) brought inside `cad_app`,
//! where all ~31.8k lines of 2D drafting + modify already work — so every plane can get the
//! FULL 2D toolset with nothing reimplemented. See `mentor MD/VENUE_DECISION_2D_ON_EVERY_PLANE.md`.
//!
//! What is deliberately NOT here: a renderer, a camera math fn, a command line, a cursor.
//! The app already has all of those. We reuse [`crate::light3d`]'s `Scene3dRenderer` + `mvp`
//! (the sandbox had duplicated both) and drive them with a `cad_solid::Model`.

use cad_solid::{BoolOp, Frame, Model, Placement, Plane, Primitive, SolidMesh};
use glam::{Mat4, Vec2, Vec3};

use crate::light3d::V3;

/// An open sketch-on-plane session.
///
/// **The core trick of 3D_Factory:** while this is live, the app's active `doc` IS the
/// sketch's `Document`. Every 2D tool in `cad_app` only ever knows `self.doc` — so draw,
/// fillet (with its R/T/M/P options), trim, extend, offset, chamfer, break, the command
/// line, snaps and layers ALL operate on the plane, **unchanged and complete**, with
/// nothing reimplemented. That is the whole thesis of this fork.
///
/// `undo_stack`/`redo_stack` are `Vec<Document>` (full snapshots), so they must be parked
/// alongside the model-space doc — otherwise an undo inside the sketch would restore a
/// model-space document over the sketch. The sketch gets a fresh, empty undo history.
pub struct SketchSession {
    /// Index into `Model::sketches`.
    pub idx: usize,
    pub saved_doc: cad_kernel::Document,
    pub saved_undo: Vec<cad_kernel::Document>,
    pub saved_redo: Vec<cad_kernel::Document>,
}

/// Fixed key light, matching `light3d`'s shading so the two 3D views look alike.
fn shade(base: [f32; 3], n: Vec3) -> [f32; 3] {
    let dir = Vec3::new(0.35, 0.25, 0.9).normalize();
    let k = 0.35 + 0.65 * n.dot(dir).abs();
    [base[0] * k, base[1] * k, base[2] * k]
}

fn v(p: Vec3, c: [f32; 3]) -> V3 {
    V3 { x: p.x, y: p.y, z: p.z, r: c[0], g: c[1], b: c[2] }
}

/// The 8 corners of an AABB, bit order x=1, y=2, z=4 (same as the sandbox's `corners_of`).
fn corners_of(mn: Vec3, mx: Vec3) -> [Vec3; 8] {
    let mut o = [Vec3::ZERO; 8];
    for (i, slot) in o.iter_mut().enumerate() {
        *slot = Vec3::new(
            if i & 1 == 0 { mn.x } else { mx.x },
            if i & 2 == 0 { mn.y } else { mx.y },
            if i & 4 == 0 { mn.z } else { mx.z },
        );
    }
    o
}

fn seg(out: &mut Vec<V3>, a: Vec3, b: Vec3, c: [f32; 3]) {
    out.push(v(a, c));
    out.push(v(b, c));
}

/// The 12 edges of an AABB.
fn aabb_lines(out: &mut Vec<V3>, mn: Vec3, mx: Vec3, c: [f32; 3]) {
    let k = corners_of(mn, mx);
    // pairs differing by exactly one bit = the 12 edges
    for i in 0..8usize {
        for bit in [1usize, 2, 4] {
            let j = i | bit;
            if j != i {
                seg(out, k[i], k[j], c);
            }
        }
    }
}

/// 3D Factory state — the model + its view. Lives on `CadApp` as one field.
pub struct FactoryState {
    pub open: bool,
    pub model: Model,
    /// Evaluated CSG mesh, rebuilt only when `dirty` (csgrs is not cheap).
    pub cached: SolidMesh,
    pub dirty: bool,
    pub selection: Vec<u32>,

    // orbit camera — `cam_target` is STORED, never recomputed from bounds each frame,
    // so the view does not jump when a solid is added or moved (sandbox lesson).
    pub cam_yaw: f32,
    pub cam_pitch: f32,
    pub cam_dist: f32,
    pub cam_target: [f32; 3],

    /// Live sketch-on-plane session (the app's `doc` is swapped while `Some`).
    pub session: Option<SketchSession>,
    /// Face picked by the last right-click — what the context menu acts on.
    pub pending_face: Option<Frame>,

    /// Boolean op applied when the NEXT primitive is added.
    pub next_op: BoolOp,
    pub box_w: f32,
    pub box_d: f32,
    pub box_h: f32,
    pub cyl_r: f32,
    pub cyl_h: f32,
    pub cyl_sides: u32,
}

impl Default for FactoryState {
    fn default() -> Self {
        Self {
            open: false,
            model: Model::default(),
            cached: SolidMesh::default(),
            dirty: false,
            selection: Vec::new(),
            cam_yaw: 0.9,
            cam_pitch: 0.5,
            cam_dist: 12.0,
            cam_target: [0.0, 0.0, 0.0],
            session: None,
            pending_face: None,
            next_op: BoolOp::Union,
            box_w: 2.0,
            box_d: 2.0,
            box_h: 1.0,
            cyl_r: 0.5,
            cyl_h: 2.0,
            cyl_sides: 24,
        }
    }
}

impl FactoryState {
    pub fn add_box(&mut self) {
        let p = Primitive::Box { w: self.box_w, d: self.box_d, h: self.box_h };
        let id = self.model.push(self.next_op, Plane::default(), Placement::default(), p);
        self.selection = vec![id];
        self.dirty = true;
    }

    pub fn add_cylinder(&mut self) {
        let p = Primitive::Cylinder { r: self.cyl_r, h: self.cyl_h, sides: self.cyl_sides.max(3) };
        let id = self.model.push(self.next_op, Plane::default(), Placement::default(), p);
        self.selection = vec![id];
        self.dirty = true;
    }

    pub fn erase_selection(&mut self) {
        for id in std::mem::take(&mut self.selection) {
            self.model.remove(id);
        }
        self.dirty = true;
    }

    pub fn clear(&mut self) {
        self.model = Model::default();
        self.selection.clear();
        self.dirty = true;
    }

    /// Re-evaluate the CSG tree. Call ONLY when idle — csgrs walks a BSP per boolean.
    pub fn recompute(&mut self) {
        self.cached = self.model.eval();
        self.dirty = false;
    }

    /// Zoom-extents: the ONLY thing that moves `cam_target`.
    pub fn fit(&mut self) {
        if let Some((mn, mx)) = self.cached.bounds() {
            self.cam_target = [
                (mn[0] + mx[0]) * 0.5,
                (mn[1] + mx[1]) * 0.5,
                (mn[2] + mx[2]) * 0.5,
            ];
            let span = (mx[0] - mn[0]).max(mx[1] - mn[1]).max(mx[2] - mn[2]);
            self.cam_dist = (span * 2.5).clamp(1.0, 400.0);
        } else {
            self.cam_target = [0.0, 0.0, 0.0];
            self.cam_dist = 12.0;
        }
    }

    /// Flat-shaded triangle soup for the evaluated solid.
    pub fn scene_verts(&self) -> Vec<V3> {
        let base = [0.62, 0.68, 0.78];
        self.cached
            .positions
            .iter()
            .zip(self.cached.normals.iter().chain(std::iter::repeat(&[0.0, 0.0, 1.0])))
            .map(|(p, n)| {
                let c = shade(base, Vec3::from(*n));
                v(Vec3::from(*p), c)
            })
            .collect()
    }

    /// Grid on the construction plane + a cyan AABB around each selected feature.
    pub fn overlay_lines(&self) -> Vec<V3> {
        let mut out = Vec::new();
        let g = [0.22, 0.25, 0.30];
        let n = 10i32;
        let s = 1.0f32;
        for i in -n..=n {
            let t = i as f32 * s;
            let e = n as f32 * s;
            seg(&mut out, Vec3::new(t, -e, 0.0), Vec3::new(t, e, 0.0), g);
            seg(&mut out, Vec3::new(-e, t, 0.0), Vec3::new(e, t, 0.0), g);
        }
        for id in &self.selection {
            if let Some(f) = self.model.features.iter().find(|f| f.id == *id) {
                let (mn, mx) = f.world_aabb();
                aabb_lines(&mut out, mn, mx, [0.0, 0.9, 1.0]);
            }
        }
        out
    }

    /// Screen cursor → world ray (origin, unit dir), by inverting the MVP.
    fn ray(cursor: egui::Pos2, rect: egui::Rect, mvp: &[f32; 16]) -> (Vec3, Vec3) {
        let ndc_x = 2.0 * (cursor.x - rect.left()) / rect.width().max(1.0) - 1.0;
        let ndc_y = 1.0 - 2.0 * (cursor.y - rect.top()) / rect.height().max(1.0);
        let inv = Mat4::from_cols_array(mvp).inverse();
        let near = inv.project_point3(Vec3::new(ndc_x, ndc_y, -1.0));
        let far = inv.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (near, (far - near).normalize_or_zero())
    }

    /// Ray-pick the front-most solid FACE under `cursor` and return a sketch [`Frame`]
    /// sitting on it — the basis for sketch-on-face. `None` if the ray misses.
    pub fn pick_face(&self, cursor: egui::Pos2, rect: egui::Rect, mvp: &[f32; 16]) -> Option<Frame> {
        let (orig, dir) = Self::ray(cursor, rect, mvp);
        let mut best: Option<(f32, Vec3, Vec3)> = None;
        for tri in self.cached.positions.chunks_exact(3) {
            let (a, b, c) = (Vec3::from(tri[0]), Vec3::from(tri[1]), Vec3::from(tri[2]));
            if let Some(t) = cad_solid::ray_triangle(orig, dir, a, b, c) {
                if best.map_or(true, |(bt, _, _)| t < bt) {
                    let n = (b - a).cross(c - a).normalize_or_zero();
                    best = Some((t, orig + dir * t, n));
                }
            }
        }
        best.map(|(_, p, n)| Frame::from_point_normal(p, n))
    }

    /// The ground (XY) plane at the origin — the fallback sketch surface when the
    /// right-click misses a solid, so you can always start drawing.
    pub fn ground_frame() -> Frame {
        Frame::from_point_normal(Vec3::ZERO, Vec3::Z)
    }

    /// Every sketch's geometry, lifted from its frame's `(u,v)` back into world space,
    /// as GL_LINES. This is what makes 2D work drawn on a plane visible in 3D.
    pub fn sketch_lines(&self) -> Vec<V3> {
        let mut out = Vec::new();
        for (i, sk) in self.model.sketches.iter().enumerate() {
            // the sketch being edited right now is drawn hot, the others cool
            let active = self.session.as_ref().is_some_and(|s| s.idx == i);
            let c = if active { [1.0, 0.62, 0.12] } else { [0.55, 0.62, 0.72] };
            for d in &sk.doc.dobjects {
                for poly in cad_solid::geom_outlines(&d.geom) {
                    for w in poly.windows(2) {
                        seg(
                            &mut out,
                            sk.frame.from_uv(Vec2::new(w[0].x, w[0].y)),
                            sk.frame.from_uv(Vec2::new(w[1].x, w[1].y)),
                            c,
                        );
                    }
                }
            }
            // frame axes, so an empty sketch plane is still visible
            if active {
                let o = sk.frame.origin;
                seg(&mut out, o, o + sk.frame.u * 1.5, [1.0, 0.3, 0.3]);
                seg(&mut out, o, o + sk.frame.v * 1.5, [0.3, 1.0, 0.3]);
            }
        }
        out
    }

    pub fn tri_count(&self) -> usize {
        self.cached.tri_count()
    }

    pub fn feature_count(&self) -> usize {
        self.model.features.len()
    }
}
