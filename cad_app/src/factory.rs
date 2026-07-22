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

/// The standard camera orientations the nav gizmo snaps to — the six orthographic
/// faces plus an isometric, exactly the set every 3D solid app puts in its corner cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StdView {
    Top, Bottom, Front, Back, Left, Right, Iso,
}

/// The 3D-Factory zoom mode — mirrors the 2D zoom command. Bare `z` → `Window` (the 2D
/// default: DRAG a box, or click two corners, with an amber "zoom window" rubber-band);
/// `z r` → `RealTime` (drag up/down dollies). `Off` = idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomMode {
    Off,
    Window,
    RealTime,
}

/// A promoted wall kept ALIVE — the Factory owns its **footprint** (the ground-plane
/// polyline) so the wall stays fully editable after promotion: change its height, or
/// move / add / delete a footprint vertex, and it re-derives.
///
/// The floor ring (`z = 0`) and the ceiling ring (`z = height`) are BOTH derived from the
/// SAME `footprint` points — so a vertex is a vertical edge present on *both* rings by
/// construction; they can never drift apart. This is why "add a vertex in Top view → it
/// lands on top AND bottom" is automatic (owner, 2026-07-22), not a special case: there is
/// only one set of points driving both rings.
///
/// Each consecutive footprint pair extrudes to one Box `Feature`; `segments[i]` is the
/// feature id of the i-th segment (`footprint.len() − 1` of them), in order. `rake`
/// (lean-from-vertical) is stored for the day the kernel gains a tilt DOF — today a
/// `Feature` is axis-aligned only, so it is not applied yet (and only then can top ≠
/// bottom, relaxing the "both rings" coupling).
#[derive(Clone, Debug)]
pub struct WallInst {
    /// Ground-plane footprint, ≥ 2 points. Shared by the floor and ceiling rings.
    pub footprint: Vec<Vec2>,
    /// One Box feature id per segment (`footprint.len() − 1` of them), in order.
    pub segments: Vec<u32>,
    pub thickness: f32,
    pub height: f32,
    pub rake_deg: f32,
}

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
    /// Parallel (orthographic) projection — TRUE after a standard-view snap (Top/Front/…/
    /// Iso) so a cylinder reads as a true CIRCLE in Top (no perspective barrel); FALSE while
    /// free-orbiting (perspective depth). CAD convention: standard views are orthographic.
    pub ortho: bool,

    /// Live sketch-on-plane session (the app's `doc` is swapped while `Some`).
    pub session: Option<SketchSession>,
    /// Face picked by the last right-click — what the context menu acts on.
    pub pending_face: Option<Frame>,

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

    /// DRAW3D edit-binding: when exactly one solid is selected, the dialog's
    /// controllers edit THAT feature live. This holds the id currently bound, so the
    /// dialog reloads its fields only when the selection changes — not every frame,
    /// which would stomp the user's edits mid-drag.
    pub draw3d_edit: Option<u32>,
    /// A primitive built in the Draw3D dialog and awaiting a placement CLICK in the 3D
    /// view — created at the picked point (a Box's corner / everything else centred),
    /// not at the origin. `None` = nothing waiting to be placed.
    pub place_pending: Option<Primitive>,

    /// 3D wall extrusion height — the ONE thing a 2D wall lacks. A promoted wall keeps
    /// its own (per-wall) thickness and rises to this height. Kept in the 3D layer, NOT
    /// cad_kernel's `WallStyle` (that's CORE, shared with the 2D app / RUST_CAD).
    pub wall_height: f32,
    /// Live wall records — every promoted wall, so its height stays editable after the
    /// fact (the "walls are alive" requirement). Keyed to model features by `feature_id`.
    pub walls: Vec<WallInst>,

    /// Zoom, mirroring the 2D command. `zoom`/`z` arms `RealTime` (drag to dolly) and
    /// shows the choice menu; typing `w` switches to `Window` (a left drag rubber-bands a
    /// box that reframes on release). `zoom_drag`/`zoom_cur` are the live box corners.
    pub zoom_mode: ZoomMode,
    pub zoom_drag: Option<egui::Pos2>,
    pub zoom_cur: Option<egui::Pos2>,
    /// Camera snapshot before the last zoom, for `zoom previous`: (yaw,pitch,dist,tx,ty,tz).
    pub cam_prev: Option<[f32; 6]>,
    /// Screen-zoom status captured at the start of a real-time drag, for the recorder.
    pub zoom_rt_before: Option<String>,

    /// An in-flight 3D modifier over `selection`. This is the SAME `move` command as
    /// 2D — only the objects and the algorithm differ ("check 2d or 3d, take the right
    /// move in the background"). `cad_solid::modify` is spec-conformant + unit-tested.
    pub modify: Option<cad_solid::modify::Modify>,
    /// A 3D op waiting on its selection — the 3D twin of the app's `queued_op`.
    /// `move` with nothing picked → queue it, gather, Enter dispatches into the picks.
    pub queued: Option<cad_solid::modify::ModifyOp>,
    /// Live prompt for the running/queued 3D op.
    pub status: String,
    /// The selected features' own mesh + the selection it was built from (the cache
    /// key). Rebuilt only when the selection changes — never per frame.
    sel_mesh: SolidMesh,
    sel_key: Vec<u32>,
}

/// CARD cardinal lock on a WORLD delta: collapse the in-plane part to its dominant
/// axis, preserving the out-of-plane component (the 3D reading of the 2D H/V lock —
/// same rule `cad_solid::modify` applies internally).
fn card_lock_world(d: Vec3) -> Vec3 {
    if d.x.abs() >= d.y.abs() {
        Vec3::new(d.x, 0.0, d.z)
    } else {
        Vec3::new(0.0, d.y, d.z)
    }
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

    /// Load the controllers FROM an existing primitive — the inverse of `build()` — so
    /// selecting a solid shows its real dimensions. The Frustum family (cone / prism /
    /// pyramid / frustum) is disambiguated the same way `Primitive::kind_label` does,
    /// by `r_top` and `sides`. Fields are set to match how `build()` reads them (e.g.
    /// cone/cylinder/tube take their facet count from `segments`, prism/pyramid from
    /// `sides`), so a load→build round-trip is stable.
    pub fn load_from(&mut self, p: &Primitive) {
        match *p {
            Primitive::Box { w, d, h } => {
                self.kind = Draw3dKind::Box;
                self.w = w; self.d = d; self.h = h;
            }
            Primitive::Sphere { r, segments, stacks } => {
                self.kind = Draw3dKind::Sphere;
                self.r = r; self.segments = segments; self.stacks = stacks;
            }
            Primitive::Cylinder { r, h, sides } => {
                self.kind = Draw3dKind::Cylinder;
                self.r = r; self.h = h; self.segments = sides;
            }
            Primitive::Frustum { r_bottom, r_top, h, sides } => {
                self.r = r_bottom; self.r_top = r_top; self.h = h;
                self.sides = sides; self.segments = sides;
                self.kind = if r_top <= 1e-6 {
                    if sides == 4 { Draw3dKind::Pyramid } else { Draw3dKind::Cone }
                } else if (r_top - r_bottom).abs() <= 1e-6 {
                    Draw3dKind::Prism
                } else {
                    Draw3dKind::Cone // a true frustum edits via the cone controllers (bottom/top/height)
                };
            }
            Primitive::Torus { major_r, minor_r, seg_major, seg_minor } => {
                self.kind = Draw3dKind::Torus;
                self.major_r = major_r; self.minor_r = minor_r;
                self.seg_major = seg_major; self.seg_minor = seg_minor;
            }
            Primitive::Capsule { r, h, segments, stacks } => {
                self.kind = Draw3dKind::Capsule;
                self.r = r; self.h = h; self.segments = segments; self.stacks = stacks;
            }
            Primitive::Tube { r_outer, r_inner, h, sides } => {
                self.kind = Draw3dKind::Tube;
                self.r = r_outer; self.r_inner = r_inner; self.h = h; self.segments = sides;
            }
            Primitive::Ellipsoid { rx, ry, rz, segments, stacks } => {
                self.kind = Draw3dKind::Ellipsoid;
                self.rx = rx; self.ry = ry; self.rz = rz;
                self.segments = segments; self.stacks = stacks;
            }
        }
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
            ortho: false,
            session: None,
            pending_face: None,
            box_w: 2.0,
            box_d: 2.0,
            box_h: 1.0,
            cyl_r: 0.5,
            cyl_h: 2.0,
            cyl_sides: 24,
            draw3d: None,
            draw3d_edit: None,
            place_pending: None,
            wall_height: 2.7,
            walls: Vec::new(),
            zoom_mode: ZoomMode::Off,
            zoom_drag: None,
            zoom_cur: None,
            cam_prev: None,
            zoom_rt_before: None,
            modify: None,
            queued: None,
            status: String::new(),
            sel_mesh: SolidMesh::default(),
            sel_key: Vec::new(),
        }
    }
}

impl FactoryState {
    pub fn add_box(&mut self) {
        let p = Primitive::Box { w: self.box_w, d: self.box_d, h: self.box_h };
        let id = self.model.push(BoolOp::Union, Plane::default(), Placement::default(), p);
        self.selection = vec![id];
        self.dirty = true;
    }

    /// DRAW3D: commit the dialog's primitive into the model (at the origin).
    pub fn add_primitive(&mut self, p: Primitive) {
        let id = self.model.push(BoolOp::Union, Plane::default(), Placement::default(), p);
        self.selection = vec![id];
        self.dirty = true;
    }

    /// DRAW3D: place a dialog-built primitive at a picked point. The click is a CORNER for
    /// a Box (it extends +w,+d,+h from there) and the CENTRE for everything else.
    pub fn place_primitive(&mut self, p: Primitive, at: Vec3) {
        let plane = Plane::default();
        let uv = plane.to_uv(at);
        let (ox, oy) = match p {
            Primitive::Box { w, d, .. } => (w * 0.5, d * 0.5), // click = the near corner
            _ => (0.0, 0.0),                                   // click = the centre
        };
        let placement = Placement { u: uv.x + ox, v: uv.y + oy, lift: 0.0, spin_deg: 0.0 };
        let id = self.model.push(BoolOp::Union, plane, placement, p);
        self.selection = vec![id];
        self.dirty = true;
    }

    pub fn add_cylinder(&mut self) {
        let p = Primitive::Cylinder { r: self.cyl_r, h: self.cyl_h, sides: self.cyl_sides.max(3) };
        let id = self.model.push(BoolOp::Union, Plane::default(), Placement::default(), p);
        self.selection = vec![id];
        self.dirty = true;
    }

    // ── 2D → 3D wall promotion ──────────────────────────────────────────────────────
    // The practical journey (owner, 2026-07-17): draft the wall in 2D with the real
    // `wall` tool (snapping / ortho / corner-join), select it, right-click → Make 3D
    // wall. Each selected `Geom::Wall`'s centerline becomes placed Boxes here.

    /// Extrude ONE footprint edge `a→b` to a placed Box and push it, returning its feature
    /// id (or `None` if degenerate). `a`,`b` are ground-plane centerline points (a 2D
    /// wall's coords ARE the ground uv); the Box keeps `thickness` and rises to `height`.
    /// Pure Box + Placement (see `Plane::world_matrix`), so no `cad_solid` change is needed.
    fn push_wall_box(&mut self, a: Vec2, b: Vec2, thickness: f32, height: f32) -> Option<u32> {
        let d = b - a;
        let len = d.length();
        if len < 1e-4 || thickness <= 0.0 || height <= 0.0 {
            return None; // ignore degenerate input
        }
        let mid = (a + b) * 0.5;
        let p = Primitive::Box { w: len, d: thickness, h: height };
        let placement = Placement {
            u: mid.x, v: mid.y, lift: 0.0, spin_deg: d.y.atan2(d.x).to_degrees(),
        };
        Some(self.model.push(BoolOp::Union, Plane::default(), placement, p))
    }

    /// Promote a **footprint** (≥ 2 ground-plane points) to a live wall: one Box per edge,
    /// all sharing `thickness` and `height`. The wall stays ALIVE — its footprint and
    /// height are remembered so vertices and rise can be edited later. Degenerate edges are
    /// skipped; returns the new wall's index, or `None` if every edge was degenerate.
    pub fn add_wall(&mut self, footprint: Vec<Vec2>, thickness: f32, height: f32) -> Option<usize> {
        if footprint.len() < 2 {
            return None;
        }
        let mut segments = Vec::new();
        for w in footprint.windows(2) {
            if let Some(id) = self.push_wall_box(w[0], w[1], thickness, height) {
                segments.push(id);
            }
        }
        if segments.is_empty() {
            return None;
        }
        self.walls.push(WallInst { footprint, segments, thickness, height, rake_deg: 0.0 });
        self.dirty = true;
        Some(self.walls.len() - 1)
    }

    /// Back-compat + simplest promotion: a single centerline segment → a 2-point wall.
    pub fn add_wall_segment(&mut self, a: Vec2, b: Vec2, thickness: f32, height: f32) {
        self.add_wall(vec![a, b], thickness, height);
    }

    /// Index of the live-wall record OWNING `feature_id` (any of its segments), if any.
    pub fn wall_index(&self, feature_id: u32) -> Option<usize> {
        self.walls.iter().position(|w| w.segments.contains(&feature_id))
    }

    /// Rebuild every segment Box of wall `wi` from its current footprint + params. The old
    /// Boxes are dropped and fresh ones pushed (the segment count changes when a vertex is
    /// added or removed). Both rings follow the one footprint, so they stay coincident.
    /// Segment feature ids change — callers that track a selection must refresh it.
    fn rederive_wall(&mut self, wi: usize) {
        if wi >= self.walls.len() {
            return;
        }
        for id in std::mem::take(&mut self.walls[wi].segments) {
            self.model.remove(id);
        }
        let fp = self.walls[wi].footprint.clone();
        let (t, h) = (self.walls[wi].thickness, self.walls[wi].height);
        let mut segments = Vec::new();
        for w in fp.windows(2) {
            if let Some(id) = self.push_wall_box(w[0], w[1], t, h) {
                segments.push(id);
            }
        }
        self.walls[wi].segments = segments;
        self.dirty = true;
    }

    /// Change a live wall's height and re-derive — the "walls are alive" edit. Updates each
    /// segment Box IN PLACE (feature ids stay stable, so a selection survives), keeping each
    /// segment's length and thickness; only the rise changes.
    pub fn set_wall_height(&mut self, feature_id: u32, height: f32) {
        let h = height.max(0.01);
        if let Some(i) = self.wall_index(feature_id) {
            self.walls[i].height = h;
            let t = self.walls[i].thickness;
            let fp = self.walls[i].footprint.clone();
            let segs = self.walls[i].segments.clone();
            for (k, w) in fp.windows(2).enumerate() {
                if let Some(&fid) = segs.get(k) {
                    let len = (w[1] - w[0]).length();
                    if let Some(f) = self.model.get_mut(fid) {
                        f.primitive = Primitive::Box { w: len, d: t, h };
                    }
                }
            }
            self.dirty = true;
        }
    }

    /// Move footprint vertex `vi` of wall `wi` to `to`, then re-derive — this is how a 3D
    /// handle drag "shifts the surface". Because both rings share the footprint, the whole
    /// vertical edge moves together.
    pub fn wall_move_vertex(&mut self, wi: usize, vi: usize, to: Vec2) {
        let ok = matches!(self.walls.get(wi), Some(w) if vi < w.footprint.len());
        if !ok {
            return;
        }
        self.walls[wi].footprint[vi] = to;
        self.rederive_wall(wi);
    }

    /// Insert a vertex at `at` into wall `wi`, splitting the segment between
    /// `footprint[seg]` and `footprint[seg + 1]`. The new corner exists on BOTH the floor
    /// and ceiling rings by construction (they share the footprint). Returns the new
    /// vertex index, or `None` if `seg` is out of range.
    pub fn wall_insert_vertex(&mut self, wi: usize, seg: usize, at: Vec2) -> Option<usize> {
        let n = self.walls.get(wi)?.footprint.len();
        if seg + 1 >= n {
            return None;
        }
        self.walls[wi].footprint.insert(seg + 1, at);
        self.rederive_wall(wi);
        Some(seg + 1)
    }

    /// Delete footprint vertex `vi` of wall `wi`, then re-derive. A wall keeps a minimum of
    /// 2 points (one segment); returns `false` if the delete was rejected.
    pub fn wall_delete_vertex(&mut self, wi: usize, vi: usize) -> bool {
        match self.walls.get(wi) {
            Some(w) if w.footprint.len() > 2 && vi < w.footprint.len() => {}
            _ => return false,
        }
        self.walls[wi].footprint.remove(vi);
        self.rederive_wall(wi);
        true
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
        self.walls.clear();
        self.dirty = true;
    }

    /// Re-evaluate the CSG tree. Call ONLY when idle — csgrs walks a BSP per boolean.
    pub fn recompute(&mut self) {
        self.cached = self.model.eval();
        self.sel_key.clear(); // the model changed → the selection's mesh is stale
        self.ensure_sel_mesh();
        self.dirty = false;
    }

    /// Refresh the selection mesh if the selection moved on (cheap no-op otherwise).
    pub fn sync_selection_mesh(&mut self) {
        self.ensure_sel_mesh();
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

    /// Snap the orbit camera to a standard view — the nav-gizmo action. Sets `(yaw,
    /// pitch)`; `cam_target`/`cam_dist` are left alone (Zoom-extents is the only thing
    /// that moves the target). `mvp` flips its up-vector near ±90° so Top/Bottom are
    /// stable even though the free-orbit drag clamps pitch to ±1.45.
    pub fn set_view(&mut self, v: StdView) {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        let (yaw, pitch) = match v {
            StdView::Top    => (-FRAC_PI_2,  FRAC_PI_2),
            StdView::Bottom => (-FRAC_PI_2, -FRAC_PI_2),
            StdView::Front  => (-FRAC_PI_2,  0.0),
            StdView::Back   => ( FRAC_PI_2,  0.0),
            StdView::Right  => ( 0.0,        0.0),
            StdView::Left   => ( PI,         0.0),
            StdView::Iso    => (-FRAC_PI_4,  0.6155), // 35.26° — the classic SE isometric
        };
        self.cam_yaw = yaw;
        self.cam_pitch = pitch;
        self.ortho = true; // standard views are orthographic (true CAD Top/Front/…)
    }

    /// Dolly the camera by a factor: `<1` zooms in (closer), `>1` zooms out. The same
    /// clamp as the scroll wheel, so command / gizmo / wheel all agree.
    pub fn zoom_by(&mut self, factor: f32) {
        self.cam_dist = (self.cam_dist * factor).clamp(0.4, 400.0);
    }

    /// Reframe the camera to a screen rectangle — the 2D "zoom window", in 3D. Moves the
    /// target under the box centre (on the target's view plane) and dollies in so the box
    /// fills the viewport height. `vp` is the viewport rect; `p0`,`p1` the drag corners.
    /// Snapshot the camera so `zoom previous` can restore it.
    pub fn zoom_save_prev(&mut self) {
        self.cam_prev = Some([
            self.cam_yaw, self.cam_pitch, self.cam_dist,
            self.cam_target[0], self.cam_target[1], self.cam_target[2],
        ]);
    }

    /// Restore the camera saved before the last zoom (`zoom previous`). No-op if none.
    pub fn zoom_restore_previous(&mut self) {
        if let Some(p) = self.cam_prev.take() {
            self.cam_yaw = p[0];
            self.cam_pitch = p[1];
            self.cam_dist = p[2];
            self.cam_target = [p[3], p[4], p[5]];
        }
    }

    pub fn zoom_window(&mut self, vp: egui::Rect, p0: egui::Pos2, p1: egui::Pos2) {
        self.zoom_save_prev();
        let bh = (p1.y - p0.y).abs().max(1.0);
        let bc = egui::pos2((p0.x + p1.x) * 0.5, (p0.y + p1.y) * 0.5);
        // box centre → normalised device coords (y up)
        let ndc_x = (bc.x - vp.center().x) / (vp.width() * 0.5).max(1.0);
        let ndc_y = -(bc.y - vp.center().y) / (vp.height() * 0.5).max(1.0);
        // camera basis — matches `light3d::mvp`
        let (cp, sp) = (self.cam_pitch.cos(), self.cam_pitch.sin());
        let (cy, sy) = (self.cam_yaw.cos(), self.cam_yaw.sin());
        let fwd = -Vec3::new(cp * cy, cp * sy, sp); // eye → target
        let up_world = if sp.abs() > 0.999 { Vec3::Y } else { Vec3::Z };
        let right = fwd.cross(up_world).normalize();
        let up = right.cross(fwd).normalize();
        // world half-extents on the target's view plane (45° vertical FOV, as in mvp)
        let half_h = (45f32.to_radians() * 0.5).tan() * self.cam_dist;
        let half_w = half_h * (vp.width() / vp.height().max(1.0));
        let t = Vec3::from(self.cam_target) + right * (ndc_x * half_w) + up * (ndc_y * half_h);
        self.cam_target = [t.x, t.y, t.z];
        let factor = (bh / vp.height().max(1.0)).clamp(0.02, 1.0);
        self.cam_dist = (self.cam_dist * factor).clamp(0.4, 400.0);
    }

    /// One-line screen-zoom status for the session recorder: how zoomed-in the camera is
    /// (`dist`), what it is centred on (`target`), and the orbit angles. Comparing this
    /// before vs after a zoom is how we tell whether the zoom actually did anything.
    pub fn zoom_status(&self) -> String {
        format!(
            "dist={:.2} target=({:.1},{:.1},{:.1}) yaw={:.0}° pitch={:.0}°",
            self.cam_dist,
            self.cam_target[0], self.cam_target[1], self.cam_target[2],
            self.cam_yaw.to_degrees(), self.cam_pitch.to_degrees(),
        )
    }

    /// The SELECTED features' own geometry, as a mesh.
    ///
    /// `cached` is the fused CSG result — after booleans, individual features have no
    /// identity in it, so the selected solid's triangles cannot be picked back out.
    /// This evaluates just the selection into its own mesh, which is what both the
    /// selection SHADE and the modifier GHOST draw.
    ///
    /// **Cached on the selection**, because csgrs walks a BSP per boolean — doing this
    /// per frame is precisely the lag source the whole panel is careful to avoid.
    fn ensure_sel_mesh(&mut self) {
        if self.sel_key == self.selection {
            return;
        }
        let mut m = Model::default();
        for id in &self.selection {
            if let Some(f) = self.model.features.iter().find(|f| f.id == *id) {
                let mut f = *f;
                f.op = BoolOp::Union; // isolated: a lone Difference would erase itself
                m.push_feature(f);
            }
        }
        self.sel_mesh = m.eval();
        self.sel_key = self.selection.clone();
    }

    /// Selection SHADE — the selected solids tinted in place (§0.6's "selected
    /// dobjects get a shade"). Drawn in the translucent overlay pass, which uses
    /// `depth_func(LEQUAL)` so coincident geometry tints instead of z-fighting.
    pub fn shade_verts(&self) -> Vec<V3> {
        if self.selection.is_empty() || self.modify.as_ref().is_some_and(|m| m.has_base()) {
            return Vec::new(); // once the base is picked the GHOST is the feedback
        }
        let c = [0.0, 0.75, 0.95];
        self.sel_mesh.positions.iter().map(|p| v(Vec3::from(*p), c)).collect()
    }

    /// GHOST — the selected solids under the op's LIVE transform, at the constrained
    /// cursor (spec §0.6: "while moving it shows the path").
    fn ghost_verts(&self, c: [f32; 3], xf: impl Fn(Vec3) -> Vec3) -> Vec<V3> {
        self.sel_mesh.positions.iter().map(|p| v(xf(Vec3::from(*p)), c)).collect()
    }

    /// The live ghost for the running op. Colours per §0.6: Move accent(255,200,100) ·
    /// Copy green(150,230,170) · Rotate/Scale white · Mirror violet(200,160,255).
    pub fn modify_ghost(&self, cursor_world: Vec3, card: bool) -> Vec<V3> {
        use cad_solid::modify::{rot_about, scale_about, ModifyOp};
        let Some(md) = &self.modify else { return Vec::new() };
        let plane = Plane::default();
        let Some(base) = md.anchor_world(&plane) else { return Vec::new() };
        match md.op {
            ModifyOp::Move | ModifyOp::Copy => {
                let d = cursor_world - base;
                let d = if card { card_lock_world(d) } else { d };
                let c = if md.op == ModifyOp::Move { [1.0, 0.78, 0.39] } else { [0.59, 0.90, 0.67] };
                self.ghost_verts(c, |p| p + d)
            }
            ModifyOp::Rotate => {
                let a = md.preview_angle(&plane, cursor_world, card).unwrap_or(0.0);
                self.ghost_verts([0.92, 0.92, 0.98], |p| rot_about(p, base, Vec3::Z, a))
            }
            ModifyOp::Scale => {
                let k = md.preview_factor(&plane, cursor_world).unwrap_or(1.0);
                self.ghost_verts([0.80, 0.95, 0.82], |p| scale_about(p, base, k))
            }
            ModifyOp::Mirror => {
                let line = (cursor_world - base).normalize_or_zero();
                let n = Vec3::Z.cross(line).normalize_or_zero();
                if n.length_squared() < 1e-9 { return Vec::new(); }
                self.ghost_verts([0.78, 0.63, 1.0], |p| p - n * (2.0 * (p - base).dot(n)))
            }
        }
    }

    /// Cancel any queued/running 3D op.
    pub fn abort_op(&mut self) {
        self.modify = None;
        self.queued = None;
        self.status.clear();
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
        crate::light3d::mvp(st.cam_yaw, st.cam_pitch, st.cam_dist, st.cam_target, aspect, st.ortho)
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

#[cfg(test)]
mod draw3d_edit_tests {
    use super::*;

    /// EDIT-MODE invariant (owner, 2026-07-17: "if one 3d dobject selected, with these
    /// controllers we should be able to change its dimension"). Selecting a solid loads
    /// it into the dialog via `load_from`; editing then rebuilds via `build`. If the two
    /// are not inverses, tweaking one field would silently corrupt the others. This
    /// proves `load_from → build` reproduces the primitive for every shape (compared by
    /// Debug, since `Primitive` isn't `PartialEq`). The Frustum family is the tricky one:
    /// cone / prism / pyramid all share one variant but different controllers.
    #[test]
    fn load_from_then_build_round_trips() {
        let cases = [
            Primitive::Box { w: 3.0, d: 4.0, h: 2.5 },
            Primitive::Cylinder { r: 1.2, h: 5.0, sides: 20 },
            Primitive::Sphere { r: 2.0, segments: 40, stacks: 18 },
            Primitive::Frustum { r_bottom: 2.0, r_top: 0.0, h: 3.0, sides: 24 }, // cone
            Primitive::Frustum { r_bottom: 1.5, r_top: 1.5, h: 2.0, sides: 6 },  // prism
            Primitive::Frustum { r_bottom: 2.0, r_top: 0.0, h: 3.0, sides: 4 },  // pyramid
            Primitive::Torus { major_r: 3.0, minor_r: 0.8, seg_major: 36, seg_minor: 18 },
            Primitive::Capsule { r: 0.7, h: 2.0, segments: 24, stacks: 12 },
            Primitive::Tube { r_outer: 2.0, r_inner: 1.0, h: 3.0, sides: 28 },
            Primitive::Ellipsoid { rx: 1.0, ry: 2.0, rz: 0.5, segments: 32, stacks: 16 },
        ];
        for p in cases {
            let mut dlg = Draw3dDialog::new(Draw3dKind::Box);
            dlg.load_from(&p);
            let rebuilt = dlg.build();
            assert_eq!(
                format!("{rebuilt:?}"), format!("{p:?}"),
                "load_from → build must reproduce the primitive"
            );
        }
    }
}

#[cfg(test)]
mod wall_tests {
    use super::*;

    /// 2D→3D wall promotion (owner, 2026-07-17): a centerline segment → ONE wall solid,
    /// a Box of length × thickness × height placed at the midpoint, spun along the run.
    #[test]
    fn wall_segment_is_a_placed_box() {
        let mut st = FactoryState::default();
        st.add_wall_segment(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), 0.3, 2.5); // 4 m along +X
        assert_eq!(st.model.features.len(), 1, "one segment → one solid");

        let f = &st.model.features[0];
        match f.primitive {
            Primitive::Box { w, d, h } => {
                assert!((w - 4.0).abs() < 1e-4, "length spans the centerline");
                assert!((d - 0.3).abs() < 1e-4, "depth = the wall's own thickness");
                assert!((h - 2.5).abs() < 1e-4, "height = the 3D wall height");
            }
            other => panic!("wall segment must be a Box, got {other:?}"),
        }
        assert!((f.placement.u - 2.0).abs() < 1e-4, "placed at the midpoint u");
        assert!(f.placement.v.abs() < 1e-4, "placed at the midpoint v");
        assert!(f.placement.spin_deg.abs() < 1e-4, "run along +X → spin 0°");
    }

    /// Orientation: a +Y run spins 90°; degenerate input is ignored.
    #[test]
    fn wall_segment_orientation_and_degenerate_guard() {
        let mut st = FactoryState::default();
        st.add_wall_segment(Vec2::new(0.0, 0.0), Vec2::new(0.0, 3.0), 0.2, 2.7); // +Y
        assert_eq!(st.model.features.len(), 1);
        assert!((st.model.features[0].placement.spin_deg - 90.0).abs() < 1e-3, "+Y run → spin 90°");

        st.add_wall_segment(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0), 0.2, 2.7); // zero length
        st.add_wall_segment(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), 0.0, 2.7); // zero thickness
        assert_eq!(st.model.features.len(), 1, "degenerate segments are ignored");
    }

    /// Walls stay ALIVE (owner, 2026-07-17): a promotion records a live wall whose height
    /// re-derives the Box on the fly, keeping its length and thickness.
    #[test]
    fn wall_stays_alive_height_re_derives() {
        let mut st = FactoryState::default();
        st.add_wall_segment(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), 0.3, 2.5);
        assert_eq!(st.walls.len(), 1, "promotion records a live wall");
        let fid = st.walls[0].segments[0];

        st.set_wall_height(fid, 3.2);
        assert!((st.walls[0].height - 3.2).abs() < 1e-4, "registry height updated");
        match st.model.get_mut(fid).unwrap().primitive {
            Primitive::Box { w, d, h } => {
                assert!((h - 3.2).abs() < 1e-4, "box height re-derived");
                assert!((w - 4.0).abs() < 1e-4 && (d - 0.3).abs() < 1e-4, "length & thickness kept");
            }
            _ => panic!("a wall is a Box"),
        }
        st.clear();
        assert!(st.walls.is_empty(), "clear drops the live-wall records too");
    }

    /// Footprint editing (owner, 2026-07-22): a wall is driven by ONE ground-plane
    /// footprint, so N points → N−1 Box segments, and adding/moving/deleting a vertex
    /// re-derives. The new corner is on BOTH rings by construction: every segment Box
    /// rises the full height from z=0, so the vertex exists at the floor AND the ceiling.
    #[test]
    fn footprint_wall_add_vertex_couples_rings_and_reshapes() {
        let mut st = FactoryState::default();
        // An L-shaped footprint: (0,0)-(4,0)-(4,3) → 2 segments.
        let wi = st
            .add_wall(vec![Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(4.0, 3.0)], 0.3, 2.7)
            .expect("L footprint promotes");
        assert_eq!(st.walls[wi].footprint.len(), 3);
        assert_eq!(st.walls[wi].segments.len(), 2, "N points → N−1 segments");
        assert_eq!(st.model.features.len(), 2);

        // Add a corner mid first edge, at (2,0): 4 points / 3 segments.
        let vi = st.wall_insert_vertex(wi, 0, Vec2::new(2.0, 0.0)).expect("split edge 0");
        assert_eq!(vi, 1);
        assert_eq!(st.walls[wi].footprint.len(), 4);
        assert_eq!(st.walls[wi].segments.len(), 3, "add vertex → +1 segment");

        // Both rings share the footprint: EVERY segment Box rises the full height from the
        // ground, so the new corner is present on both the floor (z=0) and ceiling (z=h).
        for &fid in &st.walls[wi].segments {
            match st.model.get_mut(fid).expect("segment feature").primitive {
                Primitive::Box { h, .. } => {
                    assert!((h - 2.7).abs() < 1e-4, "segment rises full height → vertex on floor & ceiling")
                }
                _ => panic!("a wall segment must be a Box"),
            }
        }

        // Drag the corner → the surface shifts; still 3 segments.
        st.wall_move_vertex(wi, 1, Vec2::new(2.0, 1.0));
        assert_eq!(st.walls[wi].segments.len(), 3);
        assert!((st.walls[wi].footprint[1] - Vec2::new(2.0, 1.0)).length() < 1e-6, "vertex moved");

        // Delete the corner → back to 3 points / 2 segments.
        assert!(st.wall_delete_vertex(wi, 1), "delete a corner");
        assert_eq!(st.walls[wi].footprint.len(), 3);
        assert_eq!(st.walls[wi].segments.len(), 2);
        // Delete down to the 2-point minimum (one segment), then reject any further delete.
        assert!(st.wall_delete_vertex(wi, 0), "delete down to a single segment");
        assert_eq!(st.walls[wi].footprint.len(), 2);
        assert_eq!(st.walls[wi].segments.len(), 1);
        assert!(!st.wall_delete_vertex(wi, 0), "a wall never drops below 2 points");
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::*;

    /// Zoom-window (owner, 2026-07-17: "we need zoom as it is in 2d"): a CENTERED box keeps
    /// the target where it is and dollies in by the box/viewport height ratio.
    #[test]
    fn zoom_window_centered_box_keeps_target_and_dollies_in() {
        let mut st = FactoryState::default();
        st.cam_target = [5.0, 5.0, 0.0];
        st.cam_dist = 20.0;
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let c = vp.center();
        // a centered box, half the viewport height (300 px) → target unchanged, dist halved
        st.zoom_window(vp, egui::pos2(c.x - 100.0, c.y - 150.0), egui::pos2(c.x + 100.0, c.y + 150.0));
        assert!((st.cam_target[0] - 5.0).abs() < 1e-3 && (st.cam_target[1] - 5.0).abs() < 1e-3,
                "a centered box keeps the target");
        assert!((st.cam_dist - 10.0).abs() < 1e-2, "a half-height box halves the distance");
    }

    /// An off-centre box shifts the target toward it (here: box to the RIGHT of centre).
    #[test]
    fn zoom_window_offcentre_box_shifts_target() {
        let mut st = FactoryState::default();
        st.cam_dist = 20.0; // Iso-ish default yaw/pitch
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let c = vp.center();
        let before = st.cam_target;
        st.zoom_window(vp, egui::pos2(c.x + 100.0, c.y - 50.0), egui::pos2(c.x + 300.0, c.y + 50.0));
        let moved = (st.cam_target[0] - before[0]).abs()
            + (st.cam_target[1] - before[1]).abs()
            + (st.cam_target[2] - before[2]).abs();
        assert!(moved > 1e-3, "an off-centre window must move the target");
    }

    /// `zoom previous` restores the camera saved before the last zoom.
    #[test]
    fn zoom_previous_restores_the_pre_zoom_camera() {
        let mut st = FactoryState::default();
        st.cam_dist = 20.0;
        st.cam_target = [1.0, 2.0, 3.0];
        st.zoom_save_prev();
        st.cam_dist = 5.0;
        st.cam_target = [9.0, 9.0, 9.0];
        st.zoom_restore_previous();
        assert!((st.cam_dist - 20.0).abs() < 1e-4, "distance restored");
        assert!((st.cam_target[0] - 1.0).abs() < 1e-4 && (st.cam_target[2] - 3.0).abs() < 1e-4,
                "target restored");
        // a second restore is a no-op (the snapshot was consumed)
        st.zoom_restore_previous();
        assert!((st.cam_dist - 20.0).abs() < 1e-4, "second restore is harmless");
    }
}

#[cfg(test)]
mod place_tests {
    use super::*;

    /// Point placement (owner, 2026-07-22): a Box places its NEAR CORNER at the click
    /// (extends +w,+d from there); every other primitive places its CENTRE at the click.
    #[test]
    fn box_corner_and_cylinder_centre() {
        let mut st = FactoryState::default();
        // Box 2×2×1, corner at (10, 20) → centre offset by half-extents (+1, +1).
        st.place_primitive(Primitive::Box { w: 2.0, d: 2.0, h: 1.0 }, Vec3::new(10.0, 20.0, 0.0));
        assert_eq!(st.model.features.len(), 1);
        let pl = st.model.features[0].placement;
        assert!((pl.u - 11.0).abs() < 1e-4 && (pl.v - 21.0).abs() < 1e-4,
                "box's near corner sits at the click");

        // Cylinder centred at the click.
        st.place_primitive(Primitive::Cylinder { r: 1.0, h: 2.0, sides: 24 }, Vec3::new(5.0, -5.0, 0.0));
        let pl2 = st.model.features[1].placement;
        assert!((pl2.u - 5.0).abs() < 1e-4 && (pl2.v + 5.0).abs() < 1e-4,
                "cylinder centre sits at the click");
    }
}
