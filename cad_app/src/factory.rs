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

    /// DRAW3D: the open primitive dialog (`None` = closed). The dialog OWNS the
    /// live parameters, so tweaking them costs nothing until Create is pressed —
    /// csgrs walks a BSP per boolean, so we never re-evaluate per keystroke.
    pub draw3d: Option<Draw3dDialog>,

    /// An in-flight 3D modifier (move/copy/rotate/scale/mirror) over `selection`.
    /// `cad_solid::modify` has been spec-conformant and unit-tested since day one —
    /// it was simply never INVOKED, because the command line only ever drove the 2D
    /// app. This field is the missing link.
    pub modify: Option<cad_solid::modify::Modify>,
    /// Prompt for the running 3D op, echoed in the panel.
    pub status: String,
}

/// Which primitive the Draw3D dialog is editing. One entry per menu item.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Draw3dKind {
    Box,
    Sphere,
    Cylinder,
    Cone,
    Prism,
    Pyramid,
    Capsule,
    Torus,
    Tube,
    Ellipsoid,
}

impl Draw3dKind {
    /// Menu order — the owner's "basic 3D objects" list, minus the two that are
    /// NOT solids (Plane/Quad and Disk/Circle are 2D: that is what the sketch +
    /// plane system is for, not a CSG primitive).
    pub const ALL: [Draw3dKind; 10] = [
        Draw3dKind::Box,
        Draw3dKind::Sphere,
        Draw3dKind::Cylinder,
        Draw3dKind::Cone,
        Draw3dKind::Prism,
        Draw3dKind::Pyramid,
        Draw3dKind::Capsule,
        Draw3dKind::Torus,
        Draw3dKind::Tube,
        Draw3dKind::Ellipsoid,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Draw3dKind::Box => "Box / Cuboid",
            Draw3dKind::Sphere => "Sphere",
            Draw3dKind::Cylinder => "Cylinder",
            Draw3dKind::Cone => "Cone / Frustum",
            Draw3dKind::Prism => "Prism",
            Draw3dKind::Pyramid => "Pyramid",
            Draw3dKind::Capsule => "Capsule",
            Draw3dKind::Torus => "Torus",
            Draw3dKind::Tube => "Tube (hollow)",
            Draw3dKind::Ellipsoid => "Ellipsoid",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Draw3dKind::Box => "⬛",
            Draw3dKind::Sphere => "⬤",
            Draw3dKind::Cylinder => "⬮",
            Draw3dKind::Cone => "▲",
            Draw3dKind::Prism => "⬡",
            Draw3dKind::Pyramid => "◭",
            Draw3dKind::Capsule => "⬭",
            Draw3dKind::Torus => "◎",
            Draw3dKind::Tube => "◯",
            Draw3dKind::Ellipsoid => "⬯",
        }
    }
}

/// The live parameter set for the Draw3D dialog.
///
/// ONE struct holds every primitive's controllers (rather than one per shape) so
/// switching kinds keeps what you already typed — set a radius on Cylinder, switch
/// to Cone, and the radius carries over. Fields are named for the CONTROLLER, and
/// several are deliberately shared across shapes (`r`, `h`, `segments`).
#[derive(Clone, Debug)]
pub struct Draw3dDialog {
    pub kind: Draw3dKind,
    // lengths
    pub w: f32,
    pub d: f32,
    pub h: f32,
    pub r: f32,
    pub r_top: f32,
    pub r_inner: f32,
    pub major_r: f32,
    pub minor_r: f32,
    pub rx: f32,
    pub ry: f32,
    pub rz: f32,
    // tessellation (accuracy controllers)
    pub segments: u32,
    pub stacks: u32,
    pub sides: u32,
    pub seg_major: u32,
    pub seg_minor: u32,
}

impl Default for Draw3dDialog {
    fn default() -> Self {
        Self {
            kind: Draw3dKind::Box,
            w: 2.0,
            d: 2.0,
            h: 1.0,
            r: 1.0,
            r_top: 0.0,
            r_inner: 0.6,
            major_r: 2.0,
            minor_r: 0.5,
            rx: 1.0,
            ry: 1.5,
            rz: 0.75,
            segments: 32,
            stacks: 16,
            sides: 6,
            seg_major: 32,
            seg_minor: 16,
        }
    }
}

impl Draw3dDialog {
    pub fn new(kind: Draw3dKind) -> Self {
        Self { kind, ..Default::default() }
    }

    /// Build the primitive from the current controllers.
    ///
    /// Cone / Prism / Pyramid all map onto ONE `Primitive::Frustum` — they are the
    /// same solid with different controllers (`r_top = 0` → cone; `r_top = r` →
    /// prism; 4 sides + `r_top = 0` → pyramid). Keeping them as separate MENU items
    /// but one primitive is why there is no duplicated meshing code.
    pub fn build(&self) -> Primitive {
        match self.kind {
            Draw3dKind::Box => Primitive::Box { w: self.w, d: self.d, h: self.h },
            Draw3dKind::Sphere => {
                Primitive::Sphere { r: self.r, segments: self.segments, stacks: self.stacks }
            }
            Draw3dKind::Cylinder => {
                Primitive::Cylinder { r: self.r, h: self.h, sides: self.segments }
            }
            Draw3dKind::Cone => Primitive::Frustum {
                r_bottom: self.r,
                r_top: self.r_top,
                h: self.h,
                sides: self.segments,
            },
            Draw3dKind::Prism => Primitive::Frustum {
                r_bottom: self.r,
                r_top: self.r, // equal radii ⇒ a prism
                h: self.h,
                sides: self.sides,
            },
            Draw3dKind::Pyramid => Primitive::Frustum {
                r_bottom: self.r,
                r_top: 0.0, // apex
                h: self.h,
                sides: self.sides,
            },
            Draw3dKind::Capsule => Primitive::Capsule {
                r: self.r,
                h: self.h,
                segments: self.segments,
                stacks: self.stacks,
            },
            Draw3dKind::Torus => Primitive::Torus {
                major_r: self.major_r,
                minor_r: self.minor_r,
                seg_major: self.seg_major,
                seg_minor: self.seg_minor,
            },
            Draw3dKind::Tube => Primitive::Tube {
                r_outer: self.r,
                r_inner: self.r_inner,
                h: self.h,
                sides: self.segments,
            },
            Draw3dKind::Ellipsoid => Primitive::Ellipsoid {
                rx: self.rx,
                ry: self.ry,
                rz: self.rz,
                segments: self.segments,
                stacks: self.stacks,
            },
        }
    }

    /// Validity + the reason, shown live in the dialog so Create is never a
    /// guess (e.g. a tube whose bore is wider than its wall isn't a tube).
    pub fn problem(&self) -> Option<&'static str> {
        match self.kind {
            Draw3dKind::Tube if self.r_inner >= self.r => {
                Some("inner radius must be smaller than outer")
            }
            Draw3dKind::Torus if self.minor_r >= self.major_r => {
                Some("minor radius must be smaller than major (else it self-intersects)")
            }
            Draw3dKind::Cone if self.r_top >= self.r => Some("top radius must be < bottom (0 = cone)"),
            _ => None,
        }
    }
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
            draw3d: None,
            modify: None,
            status: String::new(),
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

    /// DRAW3D: commit the dialog's primitive into the model.
    pub fn add_primitive(&mut self, p: Primitive) {
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

    /// Ray-pick the front-most FEATURE (solid) under `cursor`, by world AABB.
    /// This is what the LEFT button does in the 3D view — selection, never camera.
    pub fn pick_feature(&self, cursor: egui::Pos2, rect: egui::Rect, mvp: &[f32; 16]) -> Option<u32> {
        let (orig, dir) = Self::ray(cursor, rect, mvp);
        let mut best: Option<(f32, u32)> = None;
        for f in &self.model.features {
            let (mn, mx) = f.world_aabb();
            if let Some(t) = cad_solid::ray_aabb(orig, dir, mn, mx) {
                if best.map_or(true, |(bt, _)| t < bt) {
                    best = Some((t, f.id));
                }
            }
        }
        best.map(|(_, id)| id)
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

    /// Unproject `cursor` onto the active construction plane (XY at z=0) — the 3D
    /// analog of the 2D canvas's screen→world. `None` if the ray is parallel to it.
    pub fn cursor_on_plane(&self, cursor: egui::Pos2, rect: egui::Rect, mvp: &[f32; 16]) -> Option<Vec3> {
        let (orig, dir) = Self::ray(cursor, rect, mvp);
        let n = Vec3::Z;
        let denom = dir.dot(n);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = -orig.dot(n) / denom;
        (t >= 0.0).then(|| orig + dir * t)
    }

    /// OSNAP for 3D picks — the nearest solid mesh VERTEX whose screen projection is
    /// within the aperture. Mirrors the 2D pickbox: snapping to a real corner is what
    /// makes "move this corner to that corner" exact instead of eyeballed.
    pub fn snap_vertex(
        &self,
        cursor: egui::Pos2,
        rect: egui::Rect,
        mvp: &[f32; 16],
    ) -> Option<(Vec3, egui::Pos2)> {
        let m = Mat4::from_cols_array(mvp);
        let aperture = 12.0f32;
        let mut best: Option<(f32, Vec3, egui::Pos2)> = None;
        for p in &self.cached.positions {
            let w = Vec3::from(*p);
            let ndc = m.project_point3(w);
            if !(-1.0..=1.0).contains(&ndc.z) {
                continue;
            }
            let sx = rect.left() + (ndc.x * 0.5 + 0.5) * rect.width();
            let sy = rect.top() + (0.5 - ndc.y * 0.5) * rect.height();
            let sp = egui::pos2(sx, sy);
            let d = sp.distance(cursor);
            if d <= aperture && best.map_or(true, |(bd, _, _)| d < bd) {
                best = Some((d, w, sp));
            }
        }
        best.map(|(_, w, sp)| (w, sp))
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

#[cfg(test)]
mod pick_tests {
    use super::*;

    fn view(st: &FactoryState, rect: egui::Rect) -> [f32; 16] {
        let aspect = rect.width() / rect.height();
        crate::light3d::mvp(st.cam_yaw, st.cam_pitch, st.cam_dist, st.cam_target, aspect)
    }

    /// The user reports "3D dobject not selecting". Picking is pure math (screen →
    /// ray → AABB), so it CAN be tested headlessly even though the click itself
    /// needs a live egui pointer. If this passes, selection math is sound and the
    /// fault is in reachability/routing, not geometry.
    #[test]
    fn clicking_the_centre_of_the_view_picks_the_solid_there() {
        let mut st = FactoryState::default();
        st.add_box();
        st.recompute();
        st.fit(); // aim the camera at the solid, as ⌖ Frame does
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let mvp = view(&st, rect);
        let hit = st.pick_feature(rect.center(), rect, &mvp);
        assert!(hit.is_some(), "a ray through the centre must hit the centred solid");
        assert_eq!(hit.unwrap(), st.model.features[0].id);
    }

    /// …and a ray into empty space must MISS (else everything is always selected).
    #[test]
    fn clicking_far_from_the_solid_misses() {
        let mut st = FactoryState::default();
        st.add_box();
        st.recompute();
        st.fit();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let mvp = view(&st, rect);
        let corner = egui::pos2(rect.left() + 2.0, rect.top() + 2.0);
        assert!(st.pick_feature(corner, rect, &mvp).is_none(), "corner ray must miss");
    }

    /// Face-pick (the right-click → "Draw on this face" path) must land ON the solid.
    #[test]
    fn face_pick_returns_a_frame_on_the_solid() {
        let mut st = FactoryState::default();
        st.add_box();
        st.recompute();
        st.fit();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let mvp = view(&st, rect);
        let f = st.pick_face(rect.center(), rect, &mvp);
        assert!(f.is_some(), "centre ray must hit a face of the centred solid");
    }
}
