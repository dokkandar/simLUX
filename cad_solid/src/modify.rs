//! Modifier subsystem — the 3D-solid analog of simLUX's 2D modify layer.
//!
//! Deliberately mirrors `cad_app/src/app.rs` so a later merge maps 1:1:
//!
//! | 2D (app.rs)                        | here                                   |
//! |------------------------------------|----------------------------------------|
//! | `MoveState`/`RotateState`/…        | `ModifyOp` + `Modify.first` (in-flight) |
//! | `card_anchor()`/`apply_constraints`| `card_lock()` (cardinal H/V snap)       |
//! | `apply_move`/`apply_rotate`/…      | `Modify::feed()` → `Feature::translated`/`rotated`/… |
//! | `QueuedOp` + select-first basket   | `Modify::new(op, targets)` (targets = selection) |
//!
//! **Select-first:** the caller builds a selection, then starts a `Modify`; each
//! pick — a world point ON the active construction plane — is fed in, and the op
//! applies on the second pick (COPY repeats until cancelled). CARD locks MOVE to
//! one axis and ROTATE to 90° steps, exactly like the 2D `CrdEnb` behaviour. The
//! transform math is the same planar logic as the 2D `Geom` methods, applied in
//! the active plane's `(u, v)` frame.

use std::f32::consts::FRAC_PI_2;

use glam::{Vec2, Vec3};

use crate::{Model, Plane};

/// The basic set-transform modifiers (the 2D app's move/copy/rotate/scale/mirror).
/// Offset/trim/extend/fillet are 2D-geometry ops that will apply to *sketches*,
/// not to parametric solids — those arrive with the sketching slice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModifyOp {
    Move,
    Copy,
    Rotate,
    Scale,
    Mirror,
}

impl ModifyOp {
    pub const ALL: [ModifyOp; 5] =
        [ModifyOp::Move, ModifyOp::Copy, ModifyOp::Rotate, ModifyOp::Scale, ModifyOp::Mirror];

    pub fn label(self) -> &'static str {
        match self {
            ModifyOp::Move => "Move",
            ModifyOp::Copy => "Copy",
            ModifyOp::Rotate => "Rotate",
            ModifyOp::Scale => "Scale",
            ModifyOp::Mirror => "Mirror",
        }
    }

    /// (first-pick prompt, second-pick prompt) — mirrors the 2D `set_prompt` text.
    fn prompts(self) -> (&'static str, &'static str) {
        match self {
            ModifyOp::Move => ("pick BASE point", "pick DESTINATION"),
            ModifyOp::Copy => ("pick BASE point", "pick destination (repeats · Esc ends)"),
            ModifyOp::Rotate => ("pick PIVOT", "pick angle point"),
            ModifyOp::Scale => ("pick BASE point", "pick scale reference (dist = factor)"),
            ModifyOp::Mirror => ("pick mirror line START", "pick mirror line END"),
        }
    }
}

/// Outcome of feeding a pick to an in-flight op.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feed {
    /// Stored the base/pivot; waiting for the second point.
    NeedMore,
    /// Applied and finished — the caller should drop the `Modify`.
    Applied,
    /// Applied a COPY and is ready for another drop — keep the `Modify` alive.
    AppliedContinue,
}

/// An in-flight modify command over a set of selected features, echoing the 2D
/// select-first → base → second-point flow.
#[derive(Clone, Debug)]
pub struct Modify {
    pub op: ModifyOp,
    pub targets: Vec<u32>,
    /// base / pivot / mirror-A, in active-plane `(u, v)`; `None` until first pick.
    first: Option<Vec2>,
}

impl Modify {
    pub fn new(op: ModifyOp, targets: Vec<u32>) -> Self {
        Self { op, targets, first: None }
    }

    /// The current prompt (which point we're waiting for).
    pub fn prompt(&self) -> String {
        let (p0, p1) = self.op.prompts();
        let which = if self.first.is_none() { p0 } else { p1 };
        format!("{}: {}", self.op.label().to_lowercase(), which)
    }

    /// The gathered base/pivot in world space, if any (for a rubber-band preview).
    pub fn anchor_world(&self, plane: &Plane) -> Option<Vec3> {
        self.first.map(|uv| plane.from_uv(uv))
    }

    /// Feed a pick — a world point on `plane`. First call stores the base/pivot;
    /// the second applies the op to every target in `model`.
    pub fn feed(&mut self, world: Vec3, plane: &Plane, model: &mut Model, card: bool) -> Feed {
        let uv = plane.to_uv(world);
        let Some(first) = self.first else {
            self.first = Some(uv);
            return Feed::NeedMore;
        };

        match self.op {
            ModifyOp::Move => {
                let wd = plane_delta(plane, card_lock(uv - first, card));
                for id in &self.targets {
                    if let Some(f) = model.get_mut(*id) {
                        *f = f.translated(wd);
                    }
                }
                Feed::Applied
            }
            ModifyOp::Copy => {
                // Each drop duplicates the ORIGINAL targets at (pick − base); base
                // stays fixed so repeated clicks drop repeated copies (AutoCAD COPY).
                let wd = plane_delta(plane, card_lock(uv - first, card));
                let dupes: Vec<_> = self
                    .targets
                    .iter()
                    .filter_map(|id| model.features.iter().find(|f| f.id == *id).copied())
                    .map(|f| f.translated(wd))
                    .collect();
                for d in dupes {
                    model.push_feature(d);
                }
                Feed::AppliedContinue
            }
            ModifyOp::Rotate => {
                let dir = uv - first;
                let mut ang = dir.y.atan2(dir.x);
                if card {
                    ang = (ang / FRAC_PI_2).round() * FRAC_PI_2;
                }
                let pivot = plane.from_uv(first);
                let axis = plane.normal();
                for id in &self.targets {
                    if let Some(f) = model.get_mut(*id) {
                        *f = f.rotated(pivot, axis, ang);
                    }
                }
                Feed::Applied
            }
            ModifyOp::Scale => {
                let k = (uv - first).length().max(0.01);
                let pivot = plane.from_uv(first);
                for id in &self.targets {
                    if let Some(f) = model.get_mut(*id) {
                        *f = f.scaled(pivot, k);
                    }
                }
                Feed::Applied
            }
            ModifyOp::Mirror => {
                let b = if card { first + card_lock(uv - first, true) } else { uv };
                let a_w = plane.from_uv(first);
                let b_w = plane.from_uv(b);
                let line = (b_w - a_w).normalize_or_zero();
                let mirror_n = plane.normal().cross(line).normalize_or_zero();
                for id in &self.targets {
                    if let Some(f) = model.get_mut(*id) {
                        *f = f.mirrored(a_w, mirror_n);
                    }
                }
                Feed::Applied
            }
        }
    }
}

/// Lift an in-plane `(u, v)` delta to a world vector.
fn plane_delta(plane: &Plane, d: Vec2) -> Vec3 {
    plane.from_uv(d) - plane.origin()
}

/// CARD cardinal lock: collapse a delta to its dominant axis (the 2D
/// `apply_constraints` H/V snap).
fn card_lock(d: Vec2, card: bool) -> Vec2 {
    if card {
        if d.x.abs() >= d.y.abs() {
            Vec2::new(d.x, 0.0)
        } else {
            Vec2::new(0.0, d.y)
        }
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoolOp, Placement, Primitive};

    fn model_with_box(u: f32, v: f32) -> (Model, u32) {
        let mut m = Model::default();
        let id = m.push(
            BoolOp::Union,
            Plane::default(),
            Placement { u, v, lift: 0.0, spin_deg: 0.0 },
            Primitive::Box { w: 1.0, d: 1.0, h: 1.0 },
        );
        (m, id)
    }

    #[test]
    fn move_two_picks_translates_target() {
        let (mut m, id) = model_with_box(0.0, 0.0);
        let mut op = Modify::new(ModifyOp::Move, vec![id]);
        let plane = Plane::default();
        assert_eq!(op.feed(Vec3::ZERO, &plane, &mut m, false), Feed::NeedMore);
        assert_eq!(op.feed(Vec3::new(2.0, 1.0, 0.0), &plane, &mut m, false), Feed::Applied);
        let o = m.get_mut(id).unwrap().world_origin();
        assert!((o - Vec3::new(2.0, 1.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn move_with_card_locks_to_dominant_axis() {
        let (mut m, id) = model_with_box(0.0, 0.0);
        let mut op = Modify::new(ModifyOp::Move, vec![id]);
        let plane = Plane::default();
        op.feed(Vec3::ZERO, &plane, &mut m, true);
        // drag mostly +x with a little +y → CARD keeps x only.
        op.feed(Vec3::new(3.0, 0.4, 0.0), &plane, &mut m, true);
        let o = m.get_mut(id).unwrap().world_origin();
        assert!((o - Vec3::new(3.0, 0.0, 0.0)).length() < 1e-4, "y locked out, got {o:?}");
    }

    #[test]
    fn copy_adds_a_feature_and_continues() {
        let (mut m, id) = model_with_box(0.0, 0.0);
        let mut op = Modify::new(ModifyOp::Copy, vec![id]);
        let plane = Plane::default();
        op.feed(Vec3::ZERO, &plane, &mut m, false);
        let r = op.feed(Vec3::new(4.0, 0.0, 0.0), &plane, &mut m, false);
        assert_eq!(r, Feed::AppliedContinue);
        assert_eq!(m.features.len(), 2, "copy adds one feature");
        // original unmoved, copy at x=4
        assert!((m.features[0].world_origin() - Vec3::ZERO).length() < 1e-4);
        assert!((m.features[1].world_origin() - Vec3::new(4.0, 0.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn rotate_90_moves_target_across() {
        let (mut m, id) = model_with_box(1.0, 0.0);
        let mut op = Modify::new(ModifyOp::Rotate, vec![id]);
        let plane = Plane::default();
        op.feed(Vec3::ZERO, &plane, &mut m, false); // pivot at origin
        op.feed(Vec3::new(0.0, 1.0, 0.0), &plane, &mut m, false); // angle point on +v → +90°
        let o = m.get_mut(id).unwrap().world_origin();
        assert!((o - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-4, "rotated (1,0)→(0,1), got {o:?}");
    }
}
