// Geometric primitives. Tight, Copy, no virtual dispatch.

use crate::math::{Vec2, EPS, norm_angle};

#[derive(Clone, Copy, Debug)]
pub struct Line { pub a: Vec2, pub b: Vec2 }

/// Architectural smart-dobject: a wall is its IMPLICIT centerline
/// (`start` → `end`) plus a perpendicular `thickness`. Renders as two
/// parallel side lines ±thickness/2 from the centerline. All editing
/// operations (translate / rotate / scale / mirror / lengthen / trim /
/// extend) modify the centerline; the two visible sides re-derive
/// automatically. This is the "smart" part — one user gesture, both
/// sides move.
///
/// Forms the foundation of the AEC-primitive family (future Window
/// and Door dobjects will reference a Wall by handle + a position
/// along its centerline).
#[derive(Clone, Copy, Debug)]
pub struct Wall {
    pub start:     Vec2,
    pub end:       Vec2,
    pub thickness: f64,
    /// WallStyle id (drywall / structural / …). 0 = STANDARD. Drives the
    /// poché fill + (optionally) thickness; see `WallStyleTable`.
    pub style:     u32,
    /// Centerline bulge (polyline/DXF convention = tan(sweep/4)): 0 = a
    /// straight segment; ≠ 0 = the centerline is a circular arc from
    /// start→end. Used for rounded wall corners — a fillet (r>0) on two
    /// straight walls spawns a curved corner wall.
    pub bulge:     f64,
}

impl Wall {
    /// True when the centerline is an arc (rounded corner wall).
    pub fn is_curved(&self) -> bool { self.bulge.abs() > 1e-9 }

    /// Tessellate the CENTERLINE into a polyline: 2 points for a straight
    /// wall, `n`+1 points along the arc for a curved one. Faces are derived
    /// by offsetting each point ±t/2 along the local normal (sign-robust).
    pub fn centerline_polyline(&self, n: usize) -> Vec<Vec2> {
        if !self.is_curved() {
            return vec![self.start, self.end];
        }
        match bulge_arc(self.start, self.end, self.bulge) {
            Some((center, r, a0, sweep)) => {
                let steps = n.max(2);
                (0..=steps).map(|i| {
                    let t = a0 + sweep * (i as f64 / steps as f64);
                    Vec2::new(center.x + r * t.cos(), center.y + r * t.sin())
                }).collect()
            }
            None => vec![self.start, self.end],
        }
    }

    /// Face polylines `(left, right)` derived from the centerline,
    /// UNJOINED (no neighbour miters — `cad_wall::solve_faces` adds those
    /// for straight walls). Straight wall → two 2-point lines (identical
    /// to `left_line`/`right_line`). Curved wall → EXACT concentric arc
    /// samples: every centerline sample is offset ±t/2 along the TRUE
    /// radial direction, so the end face points land exactly at
    /// `start/end ± perp(tangent)·t/2` and meet a tangent straight wall's
    /// face endpoints with no gap. (The old render path used
    /// finite-difference chord normals, which tilt the end normals by
    /// sweep/(2·steps) and left a zoom-visible gap ≈ (t/2)·sweep/(2·steps)
    /// at fillet joints.)
    pub fn face_polylines(&self, n: usize) -> Option<(Vec<Vec2>, Vec<Vec2>)> {
        let half = self.thickness * 0.5;
        if !self.is_curved() {
            let l = self.left_line()?;
            let r = self.right_line()?;
            return Some((vec![l.a, l.b], vec![r.a, r.b]));
        }
        let Some((center, radius, a0, sweep)) =
            bulge_arc(self.start, self.end, self.bulge)
        else {
            // Degenerate chord — fall back to the straight faces.
            let l = self.left_line()?;
            let r = self.right_line()?;
            return Some((vec![l.a, l.b], vec![r.a, r.b]));
        };
        // "Left" = CCW-perp of the travel direction. On a CCW arc
        // (sweep > 0) that perp points INWARD (−radial); on a CW arc it
        // points OUTWARD (+radial).
        let side = if sweep >= 0.0 { -1.0 } else { 1.0 };
        let steps = n.max(2);
        let mut left  = Vec::with_capacity(steps + 1);
        let mut right = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let t = a0 + sweep * (i as f64 / steps as f64);
            let radial = Vec2::new(t.cos(), t.sin());
            let p = center + radial * radius;
            left.push(p + radial * (side * half));
            right.push(p - radial * (side * half));
        }
        Some((left, right))
    }

    /// CCW unit normal of the centerline direction. `None` if the
    /// centerline is degenerate (start ≈ end).
    pub fn normal(&self) -> Option<Vec2> {
        let d = self.end - self.start;
        let len = d.len();
        if len < EPS { return None; }
        Some((d / len).perp())
    }

    /// The "left" (CCW-side) face line of the wall.
    pub fn left_line(&self) -> Option<Line> {
        let n = self.normal()?;
        let off = n * (self.thickness * 0.5);
        Some(Line { a: self.start + off, b: self.end + off })
    }

    /// The "right" (CW-side) face line of the wall.
    pub fn right_line(&self) -> Option<Line> {
        let n = self.normal()?;
        let off = n * (self.thickness * 0.5);
        Some(Line { a: self.start - off, b: self.end - off })
    }

    /// The implicit centerline as a regular Line.
    pub fn centerline(&self) -> Line {
        Line { a: self.start, b: self.end }
    }

    /// Centerline length.
    pub fn length(&self) -> f64 {
        (self.end - self.start).len()
    }
}

/// Circular arc through `a`→`b` with DXF `bulge` (= tan(sweep/4)). Returns
/// `(center, radius, start_angle, signed_sweep)`; signed sweep is
/// `4·atan(bulge)` so positive = CCW, negative = CW. `None` if degenerate.
pub fn bulge_arc(a: Vec2, b: Vec2, bulge: f64) -> Option<(Vec2, f64, f64, f64)> {
    let chord = b - a;
    let l = chord.len();
    if l < EPS || bulge.abs() < 1e-12 { return None; }
    let r = l * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    let mid = (a + b) * 0.5;
    let perp = chord.perp() / l;
    let d = r * (1.0 - bulge * bulge) / (1.0 + bulge * bulge);
    let center = mid + perp * (d * bulge.signum());
    let start_angle = (a - center).angle();
    let sweep = 4.0 * bulge.atan();
    Some((center, r, start_angle, sweep))
}

/// The DXF bulge for the arc start→end whose centre is `center`
/// (= tan(sweep/4), signed: + when the centre is on the LEFT of start→end).
pub fn bulge_from_arc(start: Vec2, end: Vec2, center: Vec2, sweep_abs: f64) -> f64 {
    let perp = (end - start).perp();
    let sign = if (center - start).dot(perp) >= 0.0 { 1.0 } else { -1.0 };
    sign * (sweep_abs * 0.5 * 0.5).tan()
}

#[derive(Clone, Copy, Debug)]
pub struct Circle { pub center: Vec2, pub radius: f64 }

#[derive(Clone, Copy, Debug)]
pub struct Arc {
    pub center: Vec2,
    pub radius: f64,
    pub start_angle: f64,   // radians, in [0, 2π)
    pub sweep_angle: f64,   // radians, in (0, 2π], positive = CCW from start
}

impl Arc {
    /// True if the given absolute angle lies on the arc's swept range.
    pub fn contains_angle(&self, abs_angle: f64) -> bool {
        let d = norm_angle(abs_angle - self.start_angle);
        d <= self.sweep_angle + EPS
    }

    pub fn endpoints(&self) -> (Vec2, Vec2) {
        let s = self.start_angle;
        let e = self.start_angle + self.sweep_angle;
        let p1 = self.center + Vec2::new(self.radius * s.cos(), self.radius * s.sin());
        let p2 = self.center + Vec2::new(self.radius * e.cos(), self.radius * e.sin());
        (p1, p2)
    }
}

/// Full ellipse, possibly rotated. The major-axis VECTOR stores both the
/// rotation (direction) and the semi-major length (magnitude); `ratio` is
/// the semi-minor / semi-major ratio in (0, 1]. ratio = 1 means circle.
///
/// Parametric form:
///   P(t) = center + a · cos(t) · û  +  b · sin(t) · v̂
/// where û = major̂, v̂ = û rotated 90° CCW, a = |major|, b = a·ratio.
#[derive(Clone, Copy, Debug)]
pub struct Ellipse {
    pub center: Vec2,
    pub major:  Vec2,
    pub ratio:  f64,
}

/// Partial ellipse — the elliptical analogue of `Arc`. `start_param` and
/// `sweep_param` are values of the parameter `t` (radians), NOT geometric
/// angles measured at the centre. For a circle they coincide; for a
/// stretched ellipse they don't.
#[derive(Clone, Copy, Debug)]
pub struct EllipseArc {
    pub ellipse:     Ellipse,
    pub start_param: f64,    // in [0, 2π)
    pub sweep_param: f64,    // (0, 2π], positive = CCW (in parameter space)
}

/// A 2D point primitive — AutoCAD POINT entity. Has a location and a
/// per-instance display style (PDMODE-like, 0..=99). The renderer maps
/// the integer to a glyph (cross / X / circle-with-cross / etc.).
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub location: Vec2,
    pub style:    u8,    // PDMODE — 0 = single pixel dot, 2 = +, 3 = ×, 4 = |, …
    pub size:     f32,   // PDSIZE — drawing units; 0.0 = use renderer default
}

/// A 2D polyline — AutoCAD LWPOLYLINE. Stores vertices with optional
/// per-vertex `bulge` (tan of one-quarter the arc segment's included
/// angle). Bulge = 0 means a straight segment to the next vertex; bulge
/// != 0 means an arc segment whose mid-deviation = `chord_len * bulge / 2`.
#[derive(Clone, Copy, Debug)]
pub struct PolyVertex {
    pub pos:   Vec2,
    pub bulge: f64,
}

#[derive(Clone, Debug)]
pub struct Polyline {
    pub vertices: Vec<PolyVertex>,
    pub closed:   bool,
}

impl Polyline {
    /// AABB of all vertices (does NOT account for arc bulges beyond the
    /// vertex positions — conservative bbox is enough for viewport culling
    /// and the grid index. Tighter bbox is a future optimisation).
    pub fn bbox(&self) -> (Vec2, Vec2) {
        if self.vertices.is_empty() {
            return (Vec2::ZERO, Vec2::ZERO);
        }
        let mut min = self.vertices[0].pos;
        let mut max = min;
        for v in &self.vertices[1..] {
            if v.pos.x < min.x { min.x = v.pos.x; }
            if v.pos.y < min.y { min.y = v.pos.y; }
            if v.pos.x > max.x { max.x = v.pos.x; }
            if v.pos.y > max.y { max.y = v.pos.y; }
        }
        (min, max)
    }

    /// Distance from the polyline's piecewise-linear envelope to a point.
    /// Arc bulges are approximated as straight segments today (acceptable
    /// for hit-test radius at typical zooms; refine when needed).
    pub fn distance_to_point(&self, p: Vec2) -> f64 {
        if self.vertices.is_empty() { return f64::INFINITY; }
        let n = self.vertices.len();
        let mut best = f64::INFINITY;
        let pairs = if self.closed { n } else { n - 1 };
        for i in 0..pairs {
            let a = self.vertices[i].pos;
            let b = self.vertices[(i + 1) % n].pos;
            let d = b - a;
            let len_sq = d.len_sq();
            let dist = if len_sq < EPS {
                p.dist(a)
            } else {
                let t = ((p - a).dot(d) / len_sq).clamp(0.0, 1.0);
                p.dist(a + d * t)
            };
            if dist < best { best = dist; }
        }
        best
    }

    /// Total length (sum of straight chords; arc bulges add the true arc
    /// length on top — TODO when bulge math lands).
    pub fn length(&self) -> f64 {
        if self.vertices.len() < 2 { return 0.0; }
        let n = self.vertices.len();
        let pairs = if self.closed { n } else { n - 1 };
        let mut sum = 0.0;
        for i in 0..pairs {
            sum += self.vertices[i].pos.dist(self.vertices[(i + 1) % n].pos);
        }
        sum
    }
}

/// Hatch pattern — the visual style applied INSIDE a hatch boundary.
/// `Solid` fills the polygon area. `Pattern { name, scale, angle_deg }`
/// references a named line-family pattern from `crate::patterns::lookup`
/// (ANSI31, BRICK, EARTH, NET, …); the renderer clips parallel lines
/// against the resolved boundary using even-odd.
///
/// `scale` and `angle_deg` are per-hatch transforms applied on top of
/// the catalog entry: spacing multiplies by `scale`, every line family's
/// angle gets `angle_deg` added. Defaults (1.0, 0.0) mean "render the
/// pattern as the catalog defines it". Negative scale, angle outside
/// [0, 360) are allowed and behave consistently.
#[derive(Clone, Debug)]
pub enum HatchPattern {
    /// Fill the entire boundary with the dobject's solid color.
    Solid,
    /// Named pattern from the built-in catalog.
    Pattern {
        /// Canonical name (`"ANSI31"`, `"BRICK"`, …) — case-insensitive
        /// lookup. Unknown names render as nothing (the hatch still
        /// exists in the doc; user can rename it later).
        name:      String,
        /// Multiplier applied to every family's spacing. 1.0 = catalog.
        scale:     f64,
        /// Degrees added to every family's angle. 0.0 = catalog.
        angle_deg: f64,
    },
}

/// An AutoCAD-style HATCH entity. Holds REFERENCES to its boundary
/// dobject(s) by handle rather than owning a copy of their vertices.
/// This matches the user's "smart hatch" requirement: moving / editing
/// a boundary dobject automatically updates the hatch fill because
/// rendering re-resolves the boundary every frame.
///
/// Multiple boundaries → multiple loops. Even-odd fill rule at render
/// time, so the second loop becomes a hole inside the first (and the
/// fourth a hole inside the third, etc.) — basic AutoCAD island support.
///
/// Geom-level methods (bbox / distance_to_point / transforms) can't
/// resolve handles on their own (no Document access from here), so
/// they return conservative defaults. Hit-test and bbox happen at the
/// app level where the Document is available.
///
/// Style of the hatch — fill color, layer, lineweight — still lives on
/// the outer DObject as for any other Geom variant.
#[derive(Clone, Debug)]
pub struct Hatch {
    /// Handles of boundary dobjects, in loop order. Outer loop first;
    /// successive loops alternate as holes via even-odd fill at render.
    /// A handle that no longer resolves is silently skipped — the
    /// hatch shrinks gracefully if the user deletes a boundary.
    pub boundary_handles: Vec<crate::dobject::Handle>,
    pub pattern:          HatchPattern,
}

impl Hatch {
    /// Conservative bbox — empty, since this geom can't resolve its
    /// own boundaries. Callers that need a real bbox (the spatial
    /// index, viewport culling) should resolve via the Document and
    /// union the boundary dobjects' bboxes there.
    pub fn bbox(&self) -> (Vec2, Vec2) {
        (Vec2::ZERO, Vec2::ZERO)
    }

    /// Geom-level distance — INFINITY, because point-in-polygon needs
    /// the resolved boundary geometry. The app-level hit-test does
    /// the actual check by resolving handles against the Document.
    pub fn distance_to_point(&self, _p: Vec2) -> f64 {
        f64::INFINITY
    }
}

/// 2D NURBS spline. Carries everything `cad_nurbs::NurbsCurve` needs to
/// reproduce the curve: degree, control points, per-control-point
/// weights. The knot vector is implicit (clamped/open uniform —
/// derived from degree + control-point count) for v1; user-supplied
/// knot vectors land when the kernel exposes more sophisticated
/// curve-fitting paths.
///
/// All weights equal to 1.0 reduces to a plain B-spline (non-rational).
/// Distinct weights enable EXACT conics — circles, ellipses, parabolas —
/// that polynomial B-splines can only approximate.
///
/// Bbox is the convex hull's bbox (i.e. the control points' bbox),
/// which is a valid superset of the curve since a NURBS lies entirely
/// inside its control polygon's convex hull. Transforms apply
/// element-wise to control points — that's the whole point of the
/// representation.
#[derive(Clone, Debug)]
pub struct Spline {
    pub degree:         usize,
    pub control_points: Vec<Vec2>,
    /// One weight per control point. Must satisfy
    /// `weights.len() == control_points.len()` and each weight > 0.
    /// Use `Spline::new_bspline` to build a non-rational (all-1)
    /// variant without constructing the weight vector by hand.
    pub weights:        Vec<f64>,
}

impl Spline {
    /// Build a rational NURBS spline. Panics if weight count mismatches
    /// control-point count or if the curve is degenerate
    /// (control_points <= degree).
    pub fn new(degree: usize, control_points: Vec<Vec2>, weights: Vec<f64>) -> Self {
        assert_eq!(weights.len(), control_points.len(),
            "Spline: weights count must match control_points count");
        assert!(control_points.len() > degree,
            "Spline: need more control points than degree");
        Self { degree, control_points, weights }
    }

    /// Non-rational B-spline (all weights = 1.0). Same constraints as
    /// `new`.
    pub fn new_bspline(degree: usize, control_points: Vec<Vec2>) -> Self {
        let n = control_points.len();
        Self::new(degree, control_points, vec![1.0; n])
    }

    /// Bounding box of the control polygon (= valid superset of the
    /// curve's bbox by the convex-hull property). Tighter bbox would
    /// need sample-based bounds; deferred until a profiler asks for it.
    pub fn bbox(&self) -> (Vec2, Vec2) {
        if self.control_points.is_empty() {
            return (Vec2::ZERO, Vec2::ZERO);
        }
        let mut min = self.control_points[0];
        let mut max = min;
        for v in &self.control_points[1..] {
            if v.x < min.x { min.x = v.x; }
            if v.y < min.y { min.y = v.y; }
            if v.x > max.x { max.x = v.x; }
            if v.y > max.y { max.y = v.y; }
        }
        (min, max)
    }

    /// Tessellate the curve to `n_samples` evenly-spaced points across
    /// its parameter domain. Convenience wrapper around the cad_nurbs
    /// rational evaluator — converts to/from the kernel's Vec2 type at
    /// the boundary.
    pub fn tessellate(&self, n_samples: usize) -> Vec<Vec2> {
        use cad_nurbs::{NurbsCurve, Vec2 as NV};
        let ctrls: Vec<NV> = self.control_points.iter()
            .map(|v| NV::new(v.x, v.y)).collect();
        let curve = NurbsCurve::new_clamped(self.degree, ctrls, self.weights.clone());
        curve.tessellate(n_samples).into_iter()
            .map(|p| Vec2::new(p.x, p.y))
            .collect()
    }

    /// Distance from the visible curve to a point. Uses a 64-sample
    /// tessellation then per-segment distance — accurate enough for
    /// pickbox hit-testing at typical zoom levels. Refine to a
    /// projection iteration when sub-pixel accuracy is needed.
    pub fn distance_to_point(&self, p: Vec2) -> f64 {
        let samples = self.tessellate(64);
        if samples.len() < 2 { return f64::INFINITY; }
        let mut best = f64::INFINITY;
        for w in samples.windows(2) {
            let a = w[0]; let b = w[1];
            let d = b - a;
            let len_sq = d.len_sq();
            let dist = if len_sq < EPS {
                p.dist(a)
            } else {
                let t = ((p - a).dot(d) / len_sq).clamp(0.0, 1.0);
                p.dist(a + d * t)
            };
            if dist < best { best = dist; }
        }
        best
    }
}

/// Pure geometry — the shape side of a `DObject`. Style / layer / handle
/// live on the outer `DObject` struct (see [`crate::dobject`]).
///
/// Future variants land here: Text, MText, BlockRef, Dim*,
/// Image, Wipeout, Viewport, Solid2D, Ray, Xline, Leader, MLeader, Tolerance,
/// Table. Each addition is a new arm + a new entry in every match below.
#[derive(Clone, Debug)]
pub enum Geom {
    Line(Line),
    Circle(Circle),
    Arc(Arc),
    Ellipse(Ellipse),
    EllipseArc(EllipseArc),
    Point(Point),
    Polyline(Polyline),
    Hatch(Hatch),
    Spline(Spline),
    /// Architectural wall — see `Wall` for the design. All transforms
    /// operate on the centerline; the visible side lines re-derive
    /// automatically.
    Wall(Wall),
    /// Single-line text. Stored as data (position + height + angle +
    /// string + alignment + style ref); rendered by the app via
    /// `egui::Painter::text`. MText and special escape codes are
    /// deferred. See `text::Text` for the data definition.
    Text(crate::text::Text),
    /// Dimension entity. The `Dim` carries a `DimKind` (Linear /
    /// Aligned / Radius / Diameter for slice 1) plus a style id. The
    /// renderer derives extension lines, dim line, arrows, and text
    /// from the def points + DimStyle every frame; the kernel stores
    /// only the inputs. See `dim::Dim` for the full data model.
    Dimension(crate::dim::Dim),
    /// Placed block instance — references `Document.blocks` by id and
    /// carries a similarity transform (insert + rotation + uniform
    /// scale). Like Hatch, it can't resolve its own contents without the
    /// Document, so kernel-level bbox/distance are placeholders and the
    /// app resolves through the block table. See `block::BlockRef`.
    BlockRef(crate::block::BlockRef),
}

impl Geom {
    /// Return a copy rotated by `angle` radians around `pivot` (CCW).
    pub fn rotated(&self, pivot: Vec2, angle: f64) -> Geom {
        let c = angle.cos();
        let s = angle.sin();
        let rot = |p: Vec2| -> Vec2 {
            let d = p - pivot;
            Vec2 { x: pivot.x + d.x * c - d.y * s, y: pivot.y + d.x * s + d.y * c }
        };
        let rot_dir = |v: Vec2| -> Vec2 {
            // direction vectors don't shift by pivot
            Vec2 { x: v.x * c - v.y * s, y: v.x * s + v.y * c }
        };
        match self {
            Geom::Line(l) => Geom::Line(Line { a: rot(l.a), b: rot(l.b) }),
            Geom::Circle(c) => Geom::Circle(Circle { center: rot(c.center), radius: c.radius }),
            Geom::Arc(a) => Geom::Arc(Arc {
                center: rot(a.center),
                radius: a.radius,
                start_angle: (a.start_angle + angle).rem_euclid(std::f64::consts::TAU),
                sweep_angle: a.sweep_angle,
            }),
            Geom::Ellipse(e) => Geom::Ellipse(Ellipse {
                center: rot(e.center),
                major:  rot_dir(e.major),
                ratio:  e.ratio,
            }),
            Geom::EllipseArc(ea) => Geom::EllipseArc(EllipseArc {
                ellipse: Ellipse {
                    center: rot(ea.ellipse.center),
                    major:  rot_dir(ea.ellipse.major),
                    ratio:  ea.ellipse.ratio,
                },
                // Parameter space is local to the ellipse's own frame, which
                // we rotated by `angle` (the major direction moved). The
                // start_param stays — it's measured against the (now rotated)
                // local axes.
                start_param: ea.start_param,
                sweep_param: ea.sweep_param,
            }),
            Geom::Point(pt) => Geom::Point(Point {
                location: rot(pt.location), style: pt.style, size: pt.size,
            }),
            Geom::Polyline(p) => Geom::Polyline(Polyline {
                vertices: p.vertices.iter()
                    .map(|v| PolyVertex { pos: rot(v.pos), bulge: v.bulge })
                    .collect(),
                closed: p.closed,
            }),
            // Hatch transforms are NO-OPS — the hatch follows its
            // boundary dobjects (which transform on their own) via the
            // handles in `boundary_handles`. To "move a hatch", select
            // its boundary too (or do that automatically at the app
            // level — TODO follow-up).
            Geom::Hatch(h) => Geom::Hatch(h.clone()),
            // Splines transform their control points — the curve
            // follows because NURBS evaluation is linear in control
            // points (Σ N_i(u) * P_i / Σ N_i(u) * w_i; rotating each
            // P_i rotates the whole curve by the same R). Weights
            // and degree are invariant.
            Geom::Spline(s) => Geom::Spline(Spline {
                degree:         s.degree,
                control_points: s.control_points.iter().map(|p| rot(*p)).collect(),
                weights:        s.weights.clone(),
            }),
            // Wall — rotate the centerline; thickness is direction-
            // invariant. Side lines re-derive from the new endpoints.
            Geom::Wall(w) => Geom::Wall(Wall {
                start:     rot(w.start),
                end:       rot(w.end),
                thickness: w.thickness,
                style:     w.style,
                bulge:     w.bulge,        // rotation preserves arc winding
            }),
            // Text — rotate the anchor + bump the text's own angle.
            Geom::Text(t) => {
                let mut nt = t.clone();
                nt.position = rot(t.position);
                nt.angle    = t.angle + angle;
                Geom::Text(nt)
            }
            // Dimension — rotate every def point. Text orientation
            // re-derives at render time from the dim-line direction.
            Geom::Dimension(d) => Geom::Dimension(d.with_points_mapped(rot)),
            // BlockRef — rotate the insertion point about the pivot and
            // add the angle to the instance rotation. Exact (similarity
            // transforms compose).
            Geom::BlockRef(br) => Geom::BlockRef(crate::block::BlockRef {
                insert:   rot(br.insert),
                rotation: br.rotation + angle,
                ..*br
            }),
        }
    }

    /// Return a copy scaled uniformly by `factor` around `pivot`.
    /// Non-uniform scale (different x/y) isn't supported because it
    /// turns circles into ellipses — a separate refactor.
    pub fn scaled(&self, pivot: Vec2, factor: f64) -> Geom {
        let sc = |p: Vec2| -> Vec2 {
            pivot + (p - pivot) * factor
        };
        let sc_dir = |v: Vec2| -> Vec2 { v * factor };
        let f_abs = factor.abs();
        match self {
            Geom::Line(l) => Geom::Line(Line { a: sc(l.a), b: sc(l.b) }),
            Geom::Circle(c) => Geom::Circle(Circle {
                center: sc(c.center), radius: c.radius * f_abs,
            }),
            Geom::Arc(a) => Geom::Arc(Arc {
                center: sc(a.center), radius: a.radius * f_abs,
                start_angle: a.start_angle, sweep_angle: a.sweep_angle,
            }),
            Geom::Ellipse(e) => Geom::Ellipse(Ellipse {
                center: sc(e.center), major: sc_dir(e.major), ratio: e.ratio,
            }),
            Geom::EllipseArc(ea) => Geom::EllipseArc(EllipseArc {
                ellipse: Ellipse {
                    center: sc(ea.ellipse.center),
                    major:  sc_dir(ea.ellipse.major),
                    ratio:  ea.ellipse.ratio,
                },
                start_param: ea.start_param, sweep_param: ea.sweep_param,
            }),
            Geom::Point(pt) => Geom::Point(Point {
                location: sc(pt.location), style: pt.style, size: pt.size * factor as f32,
            }),
            Geom::Polyline(p) => Geom::Polyline(Polyline {
                vertices: p.vertices.iter()
                    .map(|v| PolyVertex { pos: sc(v.pos), bulge: v.bulge })
                    .collect(),
                closed: p.closed,
            }),
            // No-op for the same reason as `rotated`.
            Geom::Hatch(h) => Geom::Hatch(h.clone()),
            Geom::Spline(s) => Geom::Spline(Spline {
                degree:         s.degree,
                control_points: s.control_points.iter().map(|p| sc(*p)).collect(),
                weights:        s.weights.clone(),
            }),
            // Wall — scale the centerline + thickness uniformly so
            // the wall stays geometrically similar.
            Geom::Wall(w) => Geom::Wall(Wall {
                start:     sc(w.start),
                end:       sc(w.end),
                thickness: w.thickness * f_abs,
                style:     w.style,
                bulge:     w.bulge,        // tan(sweep/4) is scale-invariant
            }),
            // Text — scale anchor + height (angle invariant under
            // uniform scale).
            Geom::Text(t) => {
                let mut nt = t.clone();
                nt.position = sc(t.position);
                nt.height   = t.height * f_abs;
                Geom::Text(nt)
            }
            // Dimension — scale every def point. Renderer uses the
            // current DimStyle's text_height + arrow_size; those are
            // already in world units so they scale with the camera.
            Geom::Dimension(d) => Geom::Dimension(d.with_points_mapped(sc)),
            // BlockRef — scale the insertion point about the pivot and
            // multiply the uniform instance scale (|factor| like Wall
            // thickness; negative factors don't reflect a block in v1).
            Geom::BlockRef(br) => Geom::BlockRef(crate::block::BlockRef {
                insert: sc(br.insert),
                scale:  br.scale * f_abs,
                ..*br
            }),
        }
    }

    /// Return a copy mirrored across the line through `a` and `b`.
    pub fn mirrored(&self, a: Vec2, b: Vec2) -> Geom {
        let dir = b - a;
        let len_sq = dir.len_sq();
        if len_sq < EPS {
            return self.clone();   // degenerate axis — no-op
        }
        let mirror = |p: Vec2| -> Vec2 {
            let d = p - a;
            let t = d.dot(dir) / len_sq;
            let foot = a + dir * t;
            foot * 2.0 - p
        };
        let mirror_dir = |v: Vec2| -> Vec2 {
            // direction reflection is the same formula without the `a` shift.
            let t = v.dot(dir) / len_sq;
            let foot = dir * t;
            foot * 2.0 - v
        };
        match self {
            Geom::Line(l) => Geom::Line(Line { a: mirror(l.a), b: mirror(l.b) }),
            Geom::Circle(c) => Geom::Circle(Circle {
                center: mirror(c.center), radius: c.radius,
            }),
            Geom::Arc(arc) => {
                // Mirroring flips CCW → CW; we keep CCW convention by starting
                // from the OTHER endpoint and sweeping the same magnitude.
                let (_e1, e2) = arc.endpoints();
                let m2 = mirror(e2);
                let new_center = mirror(arc.center);
                let new_start = (m2 - new_center).angle();
                Geom::Arc(Arc {
                    center: new_center, radius: arc.radius,
                    start_angle: new_start.rem_euclid(std::f64::consts::TAU),
                    sweep_angle: arc.sweep_angle,
                })
            }
            Geom::Ellipse(e) => Geom::Ellipse(Ellipse {
                center: mirror(e.center), major: mirror_dir(e.major), ratio: e.ratio,
            }),
            Geom::EllipseArc(ea) => Geom::EllipseArc(EllipseArc {
                ellipse: Ellipse {
                    center: mirror(ea.ellipse.center),
                    major:  mirror_dir(ea.ellipse.major),
                    ratio:  ea.ellipse.ratio,
                },
                start_param: ea.start_param, sweep_param: ea.sweep_param,
            }),
            Geom::Point(pt) => Geom::Point(Point {
                location: mirror(pt.location), style: pt.style, size: pt.size,
            }),
            Geom::Polyline(p) => Geom::Polyline(Polyline {
                vertices: p.vertices.iter()
                    .map(|v| PolyVertex { pos: mirror(v.pos), bulge: v.bulge })
                    .collect(),
                closed: p.closed,
            }),
            // No-op for the same reason as `rotated`.
            Geom::Hatch(h) => Geom::Hatch(h.clone()),
            Geom::Spline(s) => Geom::Spline(Spline {
                degree:         s.degree,
                control_points: s.control_points.iter().map(|p| mirror(*p)).collect(),
                weights:        s.weights.clone(),
            }),
            // Wall — mirror the centerline; thickness unchanged.
            Geom::Wall(w) => Geom::Wall(Wall {
                start:     mirror(w.start),
                end:       mirror(w.end),
                thickness: w.thickness,
                style:     w.style,
                bulge:     -w.bulge,       // reflection flips arc winding
            }),
            // Text — mirror the anchor; reflect the text angle by the
            // mirror axis. Text content remains readable (would appear
            // BACKWARDS without the `TextGeneration::Backward` flag we
            // haven't modelled yet — defer to MText slice).
            Geom::Text(t) => {
                let mut nt = t.clone();
                nt.position = mirror(t.position);
                // Mirror axis angle: atan2(b-a).y, x). Reflected angle
                // = 2*axis_angle - angle.
                let axis_angle = (b - a).angle();
                nt.angle = 2.0 * axis_angle - t.angle;
                Geom::Text(nt)
            }
            // Dimension — mirror every def point. Text reads naturally
            // because the dim line direction is recomputed; no need to
            // flip the text angle separately.
            Geom::Dimension(d) => Geom::Dimension(d.with_points_mapped(mirror)),
            // BlockRef — v1 LIMITATION (documented in block.rs): the
            // insertion point and rotation are reflected, but the content
            // keeps its handedness (no `mirrored` flag yet). Symmetric
            // blocks look right; mirror asymmetric blocks by exploding
            // first. Reflected rotation = 2·axis_angle − rotation.
            Geom::BlockRef(br) => {
                let axis_angle = (b - a).angle();
                Geom::BlockRef(crate::block::BlockRef {
                    insert:   mirror(br.insert),
                    rotation: 2.0 * axis_angle - br.rotation,
                    ..*br
                })
            }
        }
    }

    /// Extend the length by signed `delta` at the end nearer to `near`.
    /// Negative delta shortens.
    /// - Line: move whichever endpoint is closer to `near` further along
    ///   the segment's direction by `delta`.
    /// - Arc / EllipseArc: extend the start or end angle/param so the
    ///   total arc length changes by `delta`.
    /// Other variants return Err.
    pub fn lengthened(&self, delta: f64, near: Vec2) -> Result<Geom, &'static str> {
        if delta.abs() < EPS { return Ok(self.clone()); }
        match self {
            Geom::Line(l) => {
                let dir = l.b - l.a;
                let len = dir.len();
                if len < EPS { return Err("lengthen: zero-length line"); }
                let u = dir / len;
                let at_b = near.dist(l.b) < near.dist(l.a);
                if at_b {
                    Ok(Geom::Line(Line { a: l.a, b: l.b + u * delta }))
                } else {
                    Ok(Geom::Line(Line { a: l.a - u * delta, b: l.b }))
                }
            }
            Geom::Arc(a) => {
                if a.radius < EPS { return Err("lengthen: zero-radius arc"); }
                let d_angle = delta / a.radius;
                let (e1, e2) = a.endpoints();
                let at_end = near.dist(e2) < near.dist(e1);
                let new_sweep = a.sweep_angle + d_angle;
                if new_sweep <= 0.0 || new_sweep >= std::f64::consts::TAU {
                    return Err("lengthen: would close arc or invert");
                }
                if at_end {
                    Ok(Geom::Arc(Arc {
                        center: a.center, radius: a.radius,
                        start_angle: a.start_angle, sweep_angle: new_sweep,
                    }))
                } else {
                    let new_start = (a.start_angle - d_angle)
                        .rem_euclid(std::f64::consts::TAU);
                    Ok(Geom::Arc(Arc {
                        center: a.center, radius: a.radius,
                        start_angle: new_start, sweep_angle: new_sweep,
                    }))
                }
            }
            Geom::EllipseArc(ea) => {
                // For an ellipse, "arc length" varies with parameter — we
                // approximate by scaling sweep_param by (delta / current
                // chord), which is correct only for near-circular ellipses
                // but acceptable for v1 (refine when needed).
                let (e1, e2) = ea.endpoints();
                let approx_len = e1.dist(e2).max(EPS);
                let dp = ea.sweep_param * (delta / approx_len);
                let at_end = near.dist(e2) < near.dist(e1);
                let new_sweep = ea.sweep_param + dp;
                if new_sweep <= 0.0 || new_sweep >= std::f64::consts::TAU {
                    return Err("lengthen: would close ellipse-arc or invert");
                }
                if at_end {
                    Ok(Geom::EllipseArc(EllipseArc {
                        ellipse: ea.ellipse,
                        start_param: ea.start_param, sweep_param: new_sweep,
                    }))
                } else {
                    let new_start = (ea.start_param - dp)
                        .rem_euclid(std::f64::consts::TAU);
                    Ok(Geom::EllipseArc(EllipseArc {
                        ellipse: ea.ellipse,
                        start_param: new_start, sweep_param: new_sweep,
                    }))
                }
            }
            Geom::Wall(w) => {
                // Lengthen the centerline; thickness unchanged.
                let line = w.centerline();
                let g = Geom::Line(line).lengthened(delta, near)?;
                if let Geom::Line(new_line) = g {
                    Ok(Geom::Wall(Wall {
                        start: new_line.a,
                        end:   new_line.b,
                        thickness: w.thickness,
                        style: w.style,
                        bulge: 0.0,        // lengthen yields a straight wall
                    }))
                } else { Err("lengthen wall: unexpected non-Line result") }
            }
            _ => Err("lengthen: only Line / Arc / EllipseArc / Wall are supported"),
        }
    }

    /// Return an "infinite" / "extended" copy of this geometry for use as a
    /// cutting / boundary edge in trim / extend with `EdgMod` ON. The point
    /// is to make intersections fire for "imaginary" geometry that the
    /// visible curve alone would miss.
    /// - Line: extended HUGE in both directions (still a Line, just very
    ///   long — keeps the segment-based intersect math working)
    /// - Arc → full Circle of the same center+radius
    /// - EllipseArc → full Ellipse
    /// - Already-closed (Circle, Ellipse) or dimensionless (Point) → clone
    /// - Polyline: per-segment "extend" doesn't generalise; clone for now
    pub fn extended_for_edgemode(&self) -> Geom {
        const EXT: f64 = 1.0e6;     // big enough to clear any drawing
        match self {
            Geom::Line(l) => {
                let d = l.b - l.a;
                let len = d.len();
                if len < EPS { return Geom::Line(*l); }
                let u = d / len;
                Geom::Line(Line { a: l.a - u * EXT, b: l.b + u * EXT })
            }
            Geom::Arc(a) => Geom::Circle(Circle { center: a.center, radius: a.radius }),
            Geom::EllipseArc(ea) => Geom::Ellipse(ea.ellipse),
            other => other.clone(),
        }
    }

    /// Trim this geometry by the given cutting edges.
    ///
    /// **Semantics (matches AutoCAD's TRIM):** the target is broken at
    /// EVERY intersection with the cutters into `N+1` separate segments;
    /// the segment containing the click is REMOVED; every other segment
    /// is returned as its OWN piece. The caller wraps each piece in a
    /// fresh `DObject` with the target's preserved style.
    ///
    /// Returns `Vec<Geom>` of the surviving sub-segments. For a target
    /// with N cuts, the user clicks one segment; you get back exactly N
    /// surviving pieces.
    ///
    /// `edge_mode` ON treats cutters as their infinite extensions for
    /// the intersection step (see `extended_for_edgemode`).
    ///
    /// Supported targets in v1: Line, Arc, EllipseArc. Other variants
    /// return an `Err` so the caller can leave them untouched.
    pub fn trim_at(
        &self,
        cutters: &[Geom],
        pick: Vec2,
        edge_mode: bool,
    ) -> Result<Vec<Geom>, &'static str> {
        use crate::intersect::intersect;

        // Gather intersection points with every cutter.
        let mut hits: Vec<Vec2> = Vec::new();
        for c in cutters {
            let c_eff = if edge_mode { c.extended_for_edgemode() } else { c.clone() };
            hits.extend(intersect(self, &c_eff));
        }
        if hits.is_empty() {
            return Err("trim: target has no intersection with the cutting edges");
        }

        /// AutoCAD-correct TRIM survivors. `bounds` = sorted parameters
        /// [target_start, …intersection_ts…, target_end]. The clicked
        /// interval is the one containing `pick_t`; that interval is the
        /// only thing removed. Everything to the LEFT of the clicked
        /// interval stays as ONE continuous piece (target_start → left
        /// boundary of clicked interval), and everything to the RIGHT
        /// stays as ONE continuous piece (right boundary → target_end).
        /// Cutter intersections that lie OUTSIDE the clicked interval do
        /// NOT cause splits — the line passes through them uninterrupted.
        ///
        /// Net survivors: 0, 1, or 2 pieces.
        ///   0 → clicked interval spans the whole target (degenerate cut).
        ///   1 → click is in the FIRST or LAST interval; the other end
        ///       survives as one continuous piece (typical case for a
        ///       line crossing a closed cutter from outside).
        ///   2 → click is in a MIDDLE interval; removing it disconnects
        ///       the target into two pieces.
        ///
        /// Earlier the algorithm kept every non-clicked interval as its
        /// own piece (over-split — the trim docs in
        /// `feedback_rust_cad_trim_breaks_into_all_segments` reflect the
        /// old rule). Confirmed bug 2026-06-08: trimming a line outside
        /// an ellipse produced 2 separate dobjects (Inside + Outside-B)
        /// when only Outside-A should have been removed.
        fn surviving_segments(bounds: &[f64], pick_t: f64, eps: f64) -> Vec<(f64, f64)> {
            let n = bounds.len();
            if n < 2 { return Vec::new(); }
            // Find the interval containing `pick_t`.
            let mut clicked: Option<(f64, f64)> = None;
            for i in 0..n - 1 {
                let t1 = bounds[i];
                let t2 = bounds[i + 1];
                if (t2 - t1) <= eps { continue; }   // skip empty intervals
                if pick_t >= t1 - eps && pick_t <= t2 + eps {
                    clicked = Some((t1, t2));
                    break;
                }
            }
            let Some((left, right)) = clicked else {
                // Pick fell outside all intervals (shouldn't happen for
                // clamped pick_t) — defensively keep the whole target.
                return vec![(bounds[0], bounds[n - 1])];
            };
            let mut out = Vec::new();
            // Left survivor: target_start → left edge of clicked interval.
            if left - bounds[0] > eps {
                out.push((bounds[0], left));
            }
            // Right survivor: right edge of clicked interval → target_end.
            if bounds[n - 1] - right > eps {
                out.push((right, bounds[n - 1]));
            }
            out
        }

        match self {
            Geom::Line(l) => {
                let d = l.b - l.a;
                let len_sq = d.len_sq();
                if len_sq < EPS { return Err("trim: zero-length line"); }
                let to_t = |p: Vec2| -> f64 { (p - l.a).dot(d) / len_sq };
                let pick_t = to_t(pick).clamp(0.0, 1.0);
                let mut params: Vec<f64> = hits.iter().map(|&p| to_t(p))
                    .filter(|&t| t > 1e-9 && t < 1.0 - 1e-9).collect();
                params.sort_by(|a, b| a.partial_cmp(b).unwrap());
                params.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
                // Endpoint-only hits → this is a stray fragment between two
                // cutters; the user wants it removed entirely. See memo
                // `feedback_rust_cad_trim_fragment_endpoint_only_deletes`.
                if params.is_empty() {
                    return Ok(Vec::new());
                }
                let mut bounds = vec![0.0_f64];
                bounds.extend(&params);
                bounds.push(1.0);
                Ok(surviving_segments(&bounds, pick_t, 1e-9).into_iter()
                    .map(|(t1, t2)| Geom::Line(Line {
                        a: l.a + d * t1,
                        b: l.a + d * t2,
                    })).collect())
            }
            Geom::Arc(arc) => {
                if arc.radius < EPS { return Err("trim: zero-radius arc"); }
                let to_local = |p: Vec2| -> f64 {
                    ((p - arc.center).angle() - arc.start_angle)
                        .rem_euclid(std::f64::consts::TAU)
                };
                let pick_t = to_local(pick).clamp(0.0, arc.sweep_angle);
                let mut params: Vec<f64> = hits.iter().map(|&p| to_local(p))
                    .filter(|&t| t > EPS && t < arc.sweep_angle - EPS).collect();
                params.sort_by(|a, b| a.partial_cmp(b).unwrap());
                params.dedup_by(|a, b| (*a - *b).abs() < EPS);
                if params.is_empty() {
                    return Ok(Vec::new());
                }
                let mut bounds = vec![0.0_f64];
                bounds.extend(&params);
                bounds.push(arc.sweep_angle);
                Ok(surviving_segments(&bounds, pick_t, EPS).into_iter()
                    .map(|(t1, t2)| Geom::Arc(Arc {
                        center: arc.center,
                        radius: arc.radius,
                        start_angle: (arc.start_angle + t1).rem_euclid(std::f64::consts::TAU),
                        sweep_angle: t2 - t1,
                    })).collect())
            }
            Geom::EllipseArc(ea) => {
                let to_local = |p: Vec2| -> f64 {
                    (ea.ellipse.nearest_param(p) - ea.start_param)
                        .rem_euclid(std::f64::consts::TAU)
                };
                let pick_t = to_local(pick).clamp(0.0, ea.sweep_param);
                let mut params: Vec<f64> = hits.iter().map(|&p| to_local(p))
                    .filter(|&t| t > EPS && t < ea.sweep_param - EPS).collect();
                params.sort_by(|a, b| a.partial_cmp(b).unwrap());
                params.dedup_by(|a, b| (*a - *b).abs() < EPS);
                if params.is_empty() {
                    return Ok(Vec::new());
                }
                let mut bounds = vec![0.0_f64];
                bounds.extend(&params);
                bounds.push(ea.sweep_param);
                Ok(surviving_segments(&bounds, pick_t, EPS).into_iter()
                    .map(|(t1, t2)| Geom::EllipseArc(EllipseArc {
                        ellipse: ea.ellipse,
                        start_param: (ea.start_param + t1).rem_euclid(std::f64::consts::TAU),
                        sweep_param: t2 - t1,
                    })).collect())
            }
            Geom::Circle(c) => {
                // Closed loop: 2+ cuts break it into N arcs.
                // Find all intersection angles (relative to angle 0); sort;
                // build segments; drop the one containing pick_angle.
                if c.radius < EPS { return Err("trim: zero-radius circle"); }
                let to_ang = |p: Vec2| (p - c.center).angle().rem_euclid(std::f64::consts::TAU);
                let pick_t = to_ang(pick);
                let mut params: Vec<f64> = hits.iter().map(|&p| to_ang(p)).collect();
                params.sort_by(|a, b| a.partial_cmp(b).unwrap());
                params.dedup_by(|a, b| (*a - *b).abs() < EPS);
                if params.len() < 2 {
                    return Err("trim: circle needs at least 2 intersections to break");
                }
                // Wrap segments end-to-end around the circle.
                let mut out = Vec::new();
                let n = params.len();
                for i in 0..n {
                    let t1 = params[i];
                    let t2 = params[(i + 1) % n];
                    let sweep = (t2 - t1).rem_euclid(std::f64::consts::TAU);
                    // Pick-angle in this arc iff (t1 → pick_t → t2) in CCW order.
                    let pick_offset = (pick_t - t1).rem_euclid(std::f64::consts::TAU);
                    let click_inside = pick_offset > EPS && pick_offset < sweep - EPS;
                    if click_inside { continue; }
                    out.push(Geom::Arc(Arc {
                        center: c.center, radius: c.radius,
                        start_angle: t1, sweep_angle: sweep,
                    }));
                }
                Ok(out)
            }
            Geom::Ellipse(el) => {
                // Closed loop, same shape as the Circle case but in ellipse
                // parameter space. Each intersection point maps to its t via
                // `nearest_param` (exact for points on the curve).
                if el.semi_major() < EPS {
                    return Err("trim: degenerate ellipse");
                }
                let to_t = |p: Vec2| el.nearest_param(p).rem_euclid(std::f64::consts::TAU);
                let pick_t = to_t(pick);
                let mut params: Vec<f64> = hits.iter().map(|&p| to_t(p)).collect();
                params.sort_by(|a, b| a.partial_cmp(b).unwrap());
                params.dedup_by(|a, b| (*a - *b).abs() < EPS);
                if params.len() < 2 {
                    return Err("trim: ellipse needs at least 2 intersections to break");
                }
                let mut out = Vec::new();
                let n = params.len();
                for i in 0..n {
                    let t1 = params[i];
                    let t2 = params[(i + 1) % n];
                    let sweep = (t2 - t1).rem_euclid(std::f64::consts::TAU);
                    let pick_offset = (pick_t - t1).rem_euclid(std::f64::consts::TAU);
                    let click_inside = pick_offset > EPS && pick_offset < sweep - EPS;
                    if click_inside { continue; }
                    out.push(Geom::EllipseArc(EllipseArc {
                        ellipse:     *el,
                        start_param: t1,
                        sweep_param: sweep,
                    }));
                }
                Ok(out)
            }
            Geom::Polyline(p) => {
                // v1 semantic: EXPLODE the polyline into independent Line
                // / Arc segments, trim the one nearest the click, leave
                // every other segment intact. The polyline structure
                // dissolves — user can `join` them back if needed.
                let segs = polyline_segments(p);
                if segs.is_empty() {
                    return Err("trim: polyline has no segments");
                }
                // Nearest-segment-to-pick.
                let mut best_i = 0usize;
                let mut best_d = f64::INFINITY;
                for (i, s) in segs.iter().enumerate() {
                    let d = s.distance_to_point(pick);
                    if d < best_d { best_d = d; best_i = i; }
                }
                let mut out = Vec::new();
                for (i, s) in segs.into_iter().enumerate() {
                    if i == best_i {
                        match s.trim_at(cutters, pick, edge_mode) {
                            Ok(pieces) => out.extend(pieces),
                            Err(_) => out.push(s),
                        }
                    } else {
                        out.push(s);
                    }
                }
                Ok(out)
            }
            Geom::Point(_) =>
                Err("trim: Point has nothing to trim"),
            Geom::Hatch(_) =>
                Err("trim: hatch entities cannot be trimmed"),
            Geom::Spline(_) =>
                Err("trim: spline entities cannot be trimmed in v1 (knot insertion + split + reparametrise pending)"),
            Geom::Wall(w) => {
                // Trim the centerline; wrap each surviving sub-segment
                // as a new Wall with the same thickness. Side lines
                // re-derive on render.
                let line = Geom::Line(w.centerline());
                let pieces = line.trim_at(cutters, pick, edge_mode)?;
                Ok(pieces.into_iter().filter_map(|g| {
                    if let Geom::Line(seg) = g {
                        Some(Geom::Wall(Wall {
                            start: seg.a, end: seg.b, thickness: w.thickness,
                            style: w.style, bulge: 0.0,
                        }))
                    } else { None }
                }).collect())
            }
            Geom::Text(_) =>
                Err("trim: text entities have no curve to cut"),
            Geom::Dimension(_) =>
                Err("trim: dimensions have no curve to cut"),
            Geom::BlockRef(_) =>
                Err("trim: explode the block first"),
        }
    }

    /// Extend this geometry toward the nearest boundary intersection on the
    /// side indicated by `pick`. Symmetric to `trim_at`. Supported targets
    /// in v1: Line and Arc (extend at whichever endpoint the click is closer to).
    pub fn extend_to(
        &self,
        boundaries: &[Geom],
        pick: Vec2,
        edge_mode: bool,
    ) -> Result<Geom, &'static str> {
        use crate::intersect::intersect;
        // Build intersections of the target's INFINITE form with each
        // (possibly extended) boundary — extension is the whole point.
        let target_infinite = self.extended_for_edgemode();
        let mut hits: Vec<Vec2> = Vec::new();
        for b in boundaries {
            let b_eff = if edge_mode { b.extended_for_edgemode() } else { b.clone() };
            hits.extend(intersect(&target_infinite, &b_eff));
        }
        if hits.is_empty() {
            return Err("extend: target has no intersection with the boundary");
        }
        match self {
            Geom::Line(l) => {
                let d = l.b - l.a;
                let len_sq = d.len_sq();
                if len_sq < EPS { return Err("extend: zero-length line"); }
                let to_t = |p: Vec2| -> f64 { (p - l.a).dot(d) / len_sq };
                let at_b = pick.dist(l.b) < pick.dist(l.a);
                if at_b {
                    // Extend forward: smallest t > 1
                    let candidate = hits.iter().map(|&p| to_t(p))
                        .filter(|&t| t > 1.0 + EPS).fold(f64::INFINITY, f64::min);
                    if candidate.is_infinite() {
                        return Err("extend: no boundary intersection past the end of the line");
                    }
                    Ok(Geom::Line(Line { a: l.a, b: l.a + d * candidate }))
                } else {
                    // Extend backward: largest t < 0
                    let candidate = hits.iter().map(|&p| to_t(p))
                        .filter(|&t| t < -EPS).fold(f64::NEG_INFINITY, f64::max);
                    if candidate.is_infinite() {
                        return Err("extend: no boundary intersection before the start of the line");
                    }
                    Ok(Geom::Line(Line { a: l.a + d * candidate, b: l.b }))
                }
            }
            Geom::Arc(arc) => {
                if arc.radius < EPS { return Err("extend: zero-radius arc"); }
                let to_local = |p: Vec2| -> f64 {
                    ((p - arc.center).angle() - arc.start_angle)
                        .rem_euclid(std::f64::consts::TAU)
                };
                let (e1, e2) = arc.endpoints();
                let at_end = pick.dist(e2) < pick.dist(e1);
                if at_end {
                    // Extend sweep: smallest t > sweep_angle
                    let candidate = hits.iter().map(|&p| to_local(p))
                        .filter(|&t| t > arc.sweep_angle + EPS).fold(f64::INFINITY, f64::min);
                    if candidate.is_infinite() || candidate >= std::f64::consts::TAU {
                        return Err("extend: no boundary intersection past the arc end");
                    }
                    Ok(Geom::Arc(Arc {
                        center: arc.center, radius: arc.radius,
                        start_angle: arc.start_angle, sweep_angle: candidate,
                    }))
                } else {
                    // Extend start backward: largest t < 0 (or equivalently t > sweep going CCW past TAU)
                    let candidate = hits.iter().map(|&p| {
                        let raw = to_local(p);
                        if raw > arc.sweep_angle + EPS { raw - std::f64::consts::TAU } else { raw }
                    }).filter(|&t| t < -EPS).fold(f64::NEG_INFINITY, f64::max);
                    if candidate.is_infinite() {
                        return Err("extend: no boundary intersection before the arc start");
                    }
                    let new_start = (arc.start_angle + candidate)
                        .rem_euclid(std::f64::consts::TAU);
                    Ok(Geom::Arc(Arc {
                        center: arc.center, radius: arc.radius,
                        start_angle: new_start,
                        sweep_angle: arc.sweep_angle - candidate,
                    }))
                }
            }
            Geom::Wall(w) => {
                let line = Geom::Line(w.centerline());
                let g = line.extend_to(boundaries, pick, edge_mode)?;
                if let Geom::Line(new_line) = g {
                    Ok(Geom::Wall(Wall {
                        start: new_line.a, end: new_line.b,
                        thickness: w.thickness,
                        style: w.style, bulge: 0.0,
                    }))
                } else { Err("extend wall: unexpected non-Line result") }
            }
            _ => Err("extend: only Line / Arc / Wall are supported in v1"),
        }
    }

    /// Split into two pieces at the projection of `at` onto the curve.
    /// Both pieces inherit nothing from style — the caller wraps them in
    /// DObjects with the original's style.
    /// Returns Err for Circle (single click can't define which side to keep)
    /// and Point (nothing to split). Closed polylines split into two open
    /// polylines.
    pub fn split_at(&self, at: Vec2) -> Result<(Geom, Geom), &'static str> {
        match self {
            Geom::Line(l) => {
                let d = l.b - l.a;
                let len_sq = d.len_sq();
                if len_sq < EPS { return Err("split: zero-length line"); }
                let t = ((at - l.a).dot(d) / len_sq).clamp(EPS, 1.0 - EPS);
                let mid = l.a + d * t;
                Ok((Geom::Line(Line { a: l.a, b: mid }),
                    Geom::Line(Line { a: mid, b: l.b })))
            }
            Geom::Arc(a) => {
                if a.radius < EPS { return Err("split: zero-radius arc"); }
                let ang = ((at - a.center).angle() - a.start_angle)
                    .rem_euclid(std::f64::consts::TAU);
                let split = ang.clamp(EPS, a.sweep_angle - EPS);
                Ok((Geom::Arc(Arc {
                    center: a.center, radius: a.radius,
                    start_angle: a.start_angle, sweep_angle: split,
                }), Geom::Arc(Arc {
                    center: a.center, radius: a.radius,
                    start_angle: (a.start_angle + split).rem_euclid(std::f64::consts::TAU),
                    sweep_angle: a.sweep_angle - split,
                })))
            }
            Geom::EllipseArc(ea) => {
                let t = ea.ellipse.nearest_param(at);
                let local = (t - ea.start_param).rem_euclid(std::f64::consts::TAU);
                let split = local.clamp(EPS, ea.sweep_param - EPS);
                Ok((Geom::EllipseArc(EllipseArc {
                    ellipse: ea.ellipse,
                    start_param: ea.start_param, sweep_param: split,
                }), Geom::EllipseArc(EllipseArc {
                    ellipse: ea.ellipse,
                    start_param: (ea.start_param + split).rem_euclid(std::f64::consts::TAU),
                    sweep_param: ea.sweep_param - split,
                })))
            }
            Geom::Polyline(p) => {
                if p.vertices.len() < 2 { return Err("split: polyline needs 2+ vertices"); }
                // Find the segment closest to `at`; split that one.
                let n = p.vertices.len();
                let pairs = if p.closed { n } else { n - 1 };
                let mut best: Option<(usize, f64, Vec2)> = None;
                for i in 0..pairs {
                    let a = p.vertices[i].pos;
                    let b = p.vertices[(i + 1) % n].pos;
                    let d = b - a;
                    let len_sq = d.len_sq();
                    if len_sq < EPS { continue; }
                    let t = ((at - a).dot(d) / len_sq).clamp(0.0, 1.0);
                    let foot = a + d * t;
                    let dist = foot.dist(at);
                    if best.map_or(true, |(_, bd, _)| dist < bd) {
                        best = Some((i, dist, foot));
                    }
                }
                let (seg, _, foot) = best.ok_or("split: degenerate polyline")?;
                // Build first piece: vertices[0..=seg] + foot
                let mut first: Vec<PolyVertex> = p.vertices[..=seg].iter().cloned().collect();
                first.push(PolyVertex { pos: foot, bulge: 0.0 });
                // Build second piece: foot + vertices[seg+1..] (or wrap for closed)
                let mut second: Vec<PolyVertex> =
                    vec![PolyVertex { pos: foot, bulge: 0.0 }];
                if p.closed {
                    for i in 0..n {
                        let idx = (seg + 1 + i) % n;
                        second.push(p.vertices[idx].clone());
                        if idx == seg { break; }
                    }
                } else {
                    for v in &p.vertices[seg + 1..] {
                        second.push(v.clone());
                    }
                }
                Ok((Geom::Polyline(Polyline { vertices: first,  closed: false }),
                    Geom::Polyline(Polyline { vertices: second, closed: false })))
            }
            Geom::Circle(_) =>
                Err("split: circle needs TWO break points (1-click break not allowed)"),
            Geom::Ellipse(_) =>
                Err("split: closed ellipse needs TWO break points"),
            Geom::Point(_) =>
                Err("split: cannot split a point"),
            Geom::Hatch(_) =>
                Err("split: hatch entities cannot be split"),
            Geom::Spline(_) =>
                Err("split: spline entities cannot be split in v1 (knot insertion pending)"),
            Geom::Wall(w) => {
                // Split the centerline at `at`; wrap each piece as a
                // Wall with the same thickness.
                let line = Geom::Line(w.centerline());
                let (g1, g2) = line.split_at(at)?;
                match (g1, g2) {
                    (Geom::Line(l1), Geom::Line(l2)) => Ok((
                        Geom::Wall(Wall { start: l1.a, end: l1.b, thickness: w.thickness, style: w.style, bulge: 0.0 }),
                        Geom::Wall(Wall { start: l2.a, end: l2.b, thickness: w.thickness, style: w.style, bulge: 0.0 }),
                    )),
                    _ => Err("split wall: unexpected non-Line result"),
                }
            }
            Geom::Text(_) =>
                Err("split: cannot split a text entity"),
            Geom::Dimension(_) =>
                Err("split: cannot split a dimension entity"),
            Geom::BlockRef(_) =>
                Err("split: explode the block first"),
        }
    }

    /// Return a parallel copy offset by `dist` to the side of `side` (a
    /// world point used to disambiguate which of the two parallel results
    /// to return — the one closer to `side` wins).
    ///
    /// Returns `Err` for Geom types we don't yet offset (Ellipse,
    /// EllipseArc — true offset of an ellipse isn't an ellipse;
    /// Polyline — corner intersection math TBD; Point — no meaningful
    /// offset).
    pub fn offset(&self, dist: f64, side: Vec2) -> Result<Geom, &'static str> {
        if dist.abs() < EPS { return Ok(self.clone()); }
        match self {
            Geom::Line(l) => {
                let d = l.b - l.a;
                let len_sq = d.len_sq();
                if len_sq < EPS { return Err("offset: zero-length line"); }
                let n = d.perp().normalized();
                // Project (side - midpoint) onto n; sign chooses direction.
                let mid = (l.a + l.b) * 0.5;
                let sgn = if (side - mid).dot(n) >= 0.0 { 1.0 } else { -1.0 };
                let shift = n * (dist * sgn);
                Ok(Geom::Line(Line { a: l.a + shift, b: l.b + shift }))
            }
            Geom::Circle(c) => {
                // Side-of-radius — inside or outside.
                let v = side - c.center;
                let outside = v.len() >= c.radius;
                let new_r = if outside { c.radius + dist.abs() } else { c.radius - dist.abs() };
                if new_r <= EPS { return Err("offset: would collapse to a point or smaller"); }
                Ok(Geom::Circle(Circle { center: c.center, radius: new_r }))
            }
            Geom::Arc(a) => {
                let v = side - a.center;
                let outside = v.len() >= a.radius;
                let new_r = if outside { a.radius + dist.abs() } else { a.radius - dist.abs() };
                if new_r <= EPS { return Err("offset: would collapse"); }
                Ok(Geom::Arc(Arc {
                    center: a.center, radius: new_r,
                    start_angle: a.start_angle, sweep_angle: a.sweep_angle,
                }))
            }
            Geom::Ellipse(el) => {
                // True offset of an ellipse is a quartic, NOT an ellipse —
                // so we return a Polyline approximation. Each sample point
                // is offset along its local outward normal; `side` picks
                // inside vs outside.
                let pts = offset_ellipse_samples(*el, 0.0, std::f64::consts::TAU,
                                                  dist, side, true);
                if pts.len() < 3 { return Err("offset: ellipse degenerate"); }
                Ok(Geom::Polyline(Polyline {
                    vertices: pts.into_iter()
                        .map(|p| PolyVertex { pos: p, bulge: 0.0 })
                        .collect(),
                    closed: true,
                }))
            }
            Geom::EllipseArc(ea) => {
                // Same polyline approximation, but only over the swept range.
                let end_param = ea.start_param + ea.sweep_param;
                let pts = offset_ellipse_samples(ea.ellipse, ea.start_param,
                                                  end_param, dist, side, false);
                if pts.len() < 2 { return Err("offset: ellipse arc degenerate"); }
                Ok(Geom::Polyline(Polyline {
                    vertices: pts.into_iter()
                        .map(|p| PolyVertex { pos: p, bulge: 0.0 })
                        .collect(),
                    closed: false,
                }))
            }
            Geom::Polyline(p) => offset_polyline(p, dist, side),
            Geom::Point(_) =>
                Err("offset on point is undefined"),
            Geom::Hatch(_) =>
                Err("offset on hatch is undefined (offset the boundary instead)"),
            Geom::Spline(_) =>
                Err("offset on spline not implemented yet (true offset of a NURBS isn't a NURBS — needs sampling + refit)"),
            Geom::Wall(w) => {
                // Offset the centerline; new Wall keeps the same
                // thickness on the offset centerline.
                let g = Geom::Line(w.centerline()).offset(dist, side)?;
                if let Geom::Line(l) = g {
                    Ok(Geom::Wall(Wall {
                        start: l.a, end: l.b, thickness: w.thickness,
                        style: w.style, bulge: 0.0,
                    }))
                } else { Err("offset wall: unexpected non-Line result") }
            }
            Geom::Text(_) =>
                Err("offset on text is undefined"),
            Geom::Dimension(_) =>
                Err("offset on dimension is undefined"),
            Geom::BlockRef(_) =>
                Err("offset: explode the block first"),
        }
    }

    /// Return a copy with direction flipped, where direction is defined.
    /// - Line: swap a/b
    /// - Arc / EllipseArc: start at the OTHER end, same sweep magnitude
    ///   (still CCW in our convention — the swept curve is identical, just
    ///   parameterised from the opposite endpoint)
    /// - Polyline: reverse vertex order, flip every per-vertex bulge sign
    ///   (bulge is signed by direction)
    /// - Circle / Ellipse / Point: no observable direction; returns a clone
    pub fn reversed(&self) -> Geom {
        match self {
            Geom::Line(l) => Geom::Line(Line { a: l.b, b: l.a }),
            Geom::Arc(a) => {
                let new_start = (a.start_angle + a.sweep_angle)
                    .rem_euclid(std::f64::consts::TAU);
                Geom::Arc(Arc {
                    center: a.center, radius: a.radius,
                    start_angle: new_start, sweep_angle: a.sweep_angle,
                })
            }
            Geom::EllipseArc(ea) => {
                let new_start = (ea.start_param + ea.sweep_param)
                    .rem_euclid(std::f64::consts::TAU);
                Geom::EllipseArc(EllipseArc {
                    ellipse: ea.ellipse,
                    start_param: new_start,
                    sweep_param: ea.sweep_param,
                })
            }
            Geom::Polyline(p) => {
                // Bulge belongs to the segment FROM this vertex to the next.
                // After reversing the vertex order, new segment `i` (verts
                // [i] → [i+1]) corresponds to the OLD segment between
                // (n-1-i) and (n-2-i) traversed in reverse — so its bulge
                // is the OLD segment's bulge with sign flipped.
                let n = p.vertices.len();
                let mut new_verts: Vec<PolyVertex> = p.vertices.iter().rev()
                    .map(|v| PolyVertex { pos: v.pos, bulge: 0.0 })
                    .collect();
                for i in 0..n {
                    let old_seg = if p.closed {
                        // closed has n segments; new segment i = reverse of
                        // old segment (n-1-i) % n. bulge[k] = segment k→k+1.
                        (n - 1 - i) % n
                    } else {
                        // open has n-1 segments; vertex (n-1)'s bulge slot
                        // is unused, so skip when i+1 >= n.
                        if i + 1 >= n { continue; }
                        n - 2 - i
                    };
                    new_verts[i].bulge = -p.vertices[old_seg].bulge;
                }
                Geom::Polyline(Polyline { vertices: new_verts, closed: p.closed })
            }
            // Direction-agnostic — return a deep copy.
            // Spline: reversing a NURBS curve = reverse control points
            //         + reverse weights + reverse knot vector. Since
            //         we use an implicit clamped/open uniform knot
            //         vector that's symmetric, reversing control
            //         points + weights is enough.
            Geom::Spline(s) => Geom::Spline(Spline {
                degree:         s.degree,
                control_points: s.control_points.iter().rev().copied().collect(),
                weights:        s.weights.iter().rev().copied().collect(),
            }),
            Geom::Circle(_) | Geom::Ellipse(_) | Geom::Point(_) | Geom::Hatch(_) => self.clone(),
            // Wall — reverse the centerline; thickness unchanged. The
            // visible side-line naming (left/right) swaps because the
            // CCW normal flips, but the geometry is identical.
            Geom::Wall(w) => Geom::Wall(Wall {
                start: w.end, end: w.start, thickness: w.thickness,
                style: w.style, bulge: -w.bulge,   // reversing flips winding
            }),
            // Text — reversal has no geometric meaning. Clone.
            Geom::Text(t) => Geom::Text(t.clone()),
            // Dimension — direction-agnostic; clone.
            Geom::Dimension(d) => Geom::Dimension(d.clone()),
            // BlockRef — direction-agnostic; clone.
            Geom::BlockRef(br) => Geom::BlockRef(*br),
        }
    }

    /// Return a copy of this geometry translated by `off`.
    pub fn translated(&self, off: Vec2) -> Geom {
        match self {
            Geom::Line(l) => Geom::Line(Line {
                a: l.a + off, b: l.b + off,
            }),
            Geom::Circle(c) => Geom::Circle(Circle {
                center: c.center + off, radius: c.radius,
            }),
            Geom::Arc(a) => Geom::Arc(Arc {
                center: a.center + off,
                radius: a.radius,
                start_angle: a.start_angle,
                sweep_angle: a.sweep_angle,
            }),
            Geom::Ellipse(e) => Geom::Ellipse(Ellipse {
                center: e.center + off,
                major:  e.major,
                ratio:  e.ratio,
            }),
            Geom::EllipseArc(ea) => Geom::EllipseArc(EllipseArc {
                ellipse:     Ellipse {
                    center: ea.ellipse.center + off,
                    major:  ea.ellipse.major,
                    ratio:  ea.ellipse.ratio,
                },
                start_param: ea.start_param,
                sweep_param: ea.sweep_param,
            }),
            Geom::Point(pt) => Geom::Point(Point {
                location: pt.location + off,
                style:    pt.style,
                size:     pt.size,
            }),
            Geom::Polyline(p) => Geom::Polyline(Polyline {
                vertices: p.vertices.iter()
                    .map(|v| PolyVertex { pos: v.pos + off, bulge: v.bulge })
                    .collect(),
                closed:   p.closed,
            }),
            // No-op for the same reason as `rotated`.
            Geom::Hatch(h) => Geom::Hatch(h.clone()),
            Geom::Spline(s) => Geom::Spline(Spline {
                degree:         s.degree,
                control_points: s.control_points.iter().map(|p| *p + off).collect(),
                weights:        s.weights.clone(),
            }),
            Geom::Wall(w) => Geom::Wall(Wall {
                start: w.start + off, end: w.end + off, thickness: w.thickness,
                style: w.style, bulge: w.bulge,
            }),
            Geom::Text(t) => {
                let mut nt = t.clone();
                nt.position = t.position + off;
                Geom::Text(nt)
            }
            Geom::Dimension(d) => Geom::Dimension(d.with_points_mapped(|p| p + off)),
            Geom::BlockRef(br) => Geom::BlockRef(crate::block::BlockRef {
                insert: br.insert + off,
                ..*br
            }),
        }
    }

    /// Minimum distance from the dobject (its visible curve) to a point.
    pub fn distance_to_point(&self, p: Vec2) -> f64 {
        match self {
            Geom::Line(l)        => l.distance_to_point(p),
            Geom::Circle(c)      => c.distance_to_point(p),
            Geom::Arc(a)         => a.distance_to_point(p),
            Geom::Ellipse(e)     => e.distance_to_point(p),
            Geom::EllipseArc(ea) => ea.distance_to_point(p),
            Geom::Point(pt)      => pt.location.dist(p),
            Geom::Polyline(pl)   => pl.distance_to_point(p),
            Geom::Hatch(h)       => h.distance_to_point(p),
            Geom::Spline(s)      => s.distance_to_point(p),
            // Wall — min distance to either visible side line. The
            // centerline ITSELF is invisible (a debug overlay) so it
            // doesn't participate in pick-test.
            Geom::Wall(w) => {
                let l = w.left_line();
                let r = w.right_line();
                match (l, r) {
                    (Some(l), Some(r)) =>
                        l.distance_to_point(p).min(r.distance_to_point(p)),
                    _ => f64::INFINITY,
                }
            }
            // Text — distance to the anchor point. Good enough for
            // click-pick; refine to bbox-distance when text starts
            // occupying significant screen area.
            Geom::Text(t) => t.position.dist(p),
            // Dimension — min distance to the dimension's def points.
            // The renderer's extension/dim lines are derived state;
            // picking on those would require re-running the renderer
            // for hit-test. v1 picks on the def points; refine later.
            Geom::Dimension(d) => {
                let mut best = f64::INFINITY;
                for gp in d.grip_points() {
                    let dist = gp.dist(p);
                    if dist < best { best = dist; }
                }
                best
            }
            // BlockRef — contents live in Document.blocks, which the
            // kernel can't reach from here (same situation as Hatch).
            // INFINITY keeps the generic pick loop from matching; the
            // app resolves block contents in its pick fallback.
            Geom::BlockRef(_) => f64::INFINITY,
        }
    }
}

impl Line {
    /// Distance from this segment to a point (perpendicular if the foot is on
    /// the segment, otherwise the nearer endpoint).
    pub fn distance_to_point(&self, p: Vec2) -> f64 {
        let d = self.b - self.a;
        let len_sq = d.len_sq();
        if len_sq < EPS { return p.dist(self.a); }
        let t = ((p - self.a).dot(d) / len_sq).clamp(0.0, 1.0);
        let foot = self.a + d * t;
        p.dist(foot)
    }
}

impl Circle {
    /// Distance from this circle's curve to a point (always positive).
    pub fn distance_to_point(&self, p: Vec2) -> f64 {
        (p.dist(self.center) - self.radius).abs()
    }
}

impl Arc {
    /// Distance from this arc's visible curve to a point. If the point's angle
    /// from the centre is within the swept range, this is the radial distance;
    /// otherwise it's the distance to the nearer endpoint.
    pub fn distance_to_point(&self, p: Vec2) -> f64 {
        let v = p - self.center;
        let ang = v.angle();
        if self.contains_angle(ang) {
            (v.len() - self.radius).abs()
        } else {
            let (e1, e2) = self.endpoints();
            p.dist(e1).min(p.dist(e2))
        }
    }
}

impl Geom {
    /// Axis-aligned bounding box (min, max). For arcs / elliptical arcs this
    /// is the conservative bbox of the full underlying curve, not the tight
    /// per-quadrant one — good enough for viewport culling and fast to compute.
    pub fn bbox(&self) -> (Vec2, Vec2) {
        match self {
            Geom::Line(l) => (
                Vec2::new(l.a.x.min(l.b.x), l.a.y.min(l.b.y)),
                Vec2::new(l.a.x.max(l.b.x), l.a.y.max(l.b.y)),
            ),
            Geom::Circle(c) => (
                Vec2::new(c.center.x - c.radius, c.center.y - c.radius),
                Vec2::new(c.center.x + c.radius, c.center.y + c.radius),
            ),
            // TIGHT arc bbox: the two endpoints, plus any cardinal extreme
            // (+x/+y/-x/-y) the arc actually sweeps through. The old
            // full-circle bbox broke window selection — an arc visually
            // inside a window was rejected because its parent circle poked
            // out of the box.
            Geom::Arc(a) => {
                let (e1, e2) = a.endpoints();
                let mut min = Vec2::new(e1.x.min(e2.x), e1.y.min(e2.y));
                let mut max = Vec2::new(e1.x.max(e2.x), e1.y.max(e2.y));
                for k in 0..4 {
                    let ang = k as f64 * std::f64::consts::FRAC_PI_2;
                    let rel = (ang - a.start_angle).rem_euclid(std::f64::consts::TAU);
                    if rel <= a.sweep_angle + 1e-12 {
                        let p = Vec2::new(
                            a.center.x + a.radius * ang.cos(),
                            a.center.y + a.radius * ang.sin());
                        min.x = min.x.min(p.x); min.y = min.y.min(p.y);
                        max.x = max.x.max(p.x); max.y = max.y.max(p.y);
                    }
                }
                (min, max)
            }
            Geom::Ellipse(e)     => e.bbox(),
            // TIGHT elliptical-arc bbox by sampling — the full-ellipse bbox
            // had the same window-selection bug as Arc.
            Geom::EllipseArc(ea) => {
                let n = 48;
                let mut min = Vec2::new(f64::INFINITY, f64::INFINITY);
                let mut max = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                for i in 0..=n {
                    let t = ea.start_param + ea.sweep_param * (i as f64 / n as f64);
                    let p = ea.ellipse.point_at(t);
                    min.x = min.x.min(p.x); min.y = min.y.min(p.y);
                    max.x = max.x.max(p.x); max.y = max.y.max(p.y);
                }
                (min, max)
            }
            Geom::Point(pt) => (pt.location, pt.location),
            Geom::Polyline(pl) => pl.bbox(),
            Geom::Hatch(h)  => h.bbox(),
            Geom::Spline(s) => s.bbox(),
            // Wall bbox = centerline bbox EXPANDED by thickness/2 in
            // both axes (loose but cheap; the rotated side-line corners
            // are always within this box).
            Geom::Wall(w) => {
                let h = w.thickness * 0.5;
                let min = Vec2::new(
                    w.start.x.min(w.end.x) - h,
                    w.start.y.min(w.end.y) - h,
                );
                let max = Vec2::new(
                    w.start.x.max(w.end.x) + h,
                    w.start.y.max(w.end.y) + h,
                );
                (min, max)
            }
            // Text — unrotated bbox (loose; ignores `angle`). Same
            // approximation Wall uses for its rotated side-line corners.
            Geom::Text(t) => t.bbox_unrotated(),
            // Dimension — bbox of the def points. The renderer's text
            // and extension lines can fall slightly outside this; v1
            // accepts the loose bbox.
            Geom::Dimension(d) => d.bbox(),
            // BlockRef — placeholder (can't resolve contents without the
            // Document). `is_view_independent_bbox()` returns true so
            // spatial indexes never cull on this; the app computes the
            // real resolved bbox where it matters (window selection).
            Geom::BlockRef(br) => (br.insert, br.insert),
        }
    }

    /// True iff this dobject's bbox is NOT a reliable view-culling key.
    ///
    /// Hatches reference their boundary by handle, so the kernel can't
    /// compute the bbox without the Document. Bbox returns (0, 0) as a
    /// placeholder — which means a spatial-index bbox-query may filter
    /// the hatch out unless the camera happens to include the origin.
    ///
    /// Spatial indexes (e.g. `UniformGrid`) MUST treat
    /// view-independent dobjects as ALWAYS in the candidate set, so
    /// the resolved-by-handle render path still gets a chance to draw
    /// them. App-level renderers MUST short-circuit them BEFORE
    /// viewport-bbox culls.
    ///
    /// Future variants (Region, BlockRef, Xref) that store references
    /// rather than vertices will override this the same way Hatch
    /// does today.
    pub fn is_view_independent_bbox(&self) -> bool {
        // Hatch and BlockRef both resolve their real extent through the
        // Document (boundary handles / block table), so their kernel
        // bboxes are placeholders and spatial indexes must keep them in
        // every candidate set.
        matches!(self, Geom::Hatch(_) | Geom::BlockRef(_))
    }
}

// ---- Ellipse / EllipseArc geometry ----------------------------------------

impl Ellipse {
    /// Semi-major axis length, `a`.
    pub fn semi_major(&self) -> f64 { self.major.len() }
    /// Semi-minor axis length, `b = a · ratio`.
    pub fn semi_minor(&self) -> f64 { self.semi_major() * self.ratio }

    /// Unit vector along the major axis (the "u" direction).
    pub fn u_hat(&self) -> Vec2 { self.major.normalized() }
    /// Unit vector along the minor axis (u rotated 90° CCW).
    pub fn v_hat(&self) -> Vec2 { self.u_hat().perp() }

    /// Point on the ellipse curve at parameter t (radians).
    /// P(t) = center + a·cos(t)·û + b·sin(t)·v̂
    pub fn point_at(&self, t: f64) -> Vec2 {
        let a = self.semi_major();
        let b = self.semi_minor();
        self.center + self.u_hat() * (a * t.cos()) + self.v_hat() * (b * t.sin())
    }

    /// Tangent vector (un-normalized) at parameter t. dP/dt.
    pub fn tangent_at(&self, t: f64) -> Vec2 {
        let a = self.semi_major();
        let b = self.semi_minor();
        self.u_hat() * (-a * t.sin()) + self.v_hat() * (b * t.cos())
    }

    /// Axis-aligned bbox of the FULL ellipse (regardless of any swept range).
    /// Derived from the rotated parametric form:
    ///   x_half = sqrt(a²·cos²θ + b²·sin²θ)
    ///   y_half = sqrt(a²·sin²θ + b²·cos²θ)
    /// In our representation (major = a · û, ratio = b/a) this simplifies to:
    ///   x_half = sqrt(major.x² + (ratio · major.y)²)
    ///   y_half = sqrt(major.y² + (ratio · major.x)²)
    pub fn bbox(&self) -> (Vec2, Vec2) {
        let mx = self.major.x;
        let my = self.major.y;
        let r2 = self.ratio * self.ratio;
        let hx = (mx * mx + r2 * my * my).sqrt();
        let hy = (my * my + r2 * mx * mx).sqrt();
        (Vec2::new(self.center.x - hx, self.center.y - hy),
         Vec2::new(self.center.x + hx, self.center.y + hy))
    }

    /// Closest parameter t to a world point `p`, found by Newton iteration on
    /// `f(t) = (P(t) - p) · P'(t) = 0`. Closed-form solving requires a
    /// quartic; this is fast, robust, and accurate enough for snap / hit-test.
    /// The initial guess is the angle of `p - center` in the ellipse's local
    /// frame, which is usually 1–2 iterations from the true root.
    pub fn nearest_param(&self, p: Vec2) -> f64 {
        let a = self.semi_major();
        if a < EPS { return 0.0; }
        let b = self.semi_minor();
        // Initial guess: rotate `p - center` into local frame, then take atan2
        // using the scaled coordinates so the angle matches PARAMETER space.
        let d = p - self.center;
        let lx = d.dot(self.u_hat());
        let ly = d.dot(self.v_hat());
        let mut t = (ly * a).atan2(lx * b);
        // 5 Newton iterations is more than enough for 1e-9 convergence in
        // double precision for any reasonable ratio.
        for _ in 0..5 {
            let pt = self.point_at(t);
            let dp = self.tangent_at(t);
            let d2 = -self.u_hat() * (a * t.cos()) - self.v_hat() * (b * t.sin());
            let f  = (pt - p).dot(dp);
            let fd = (pt - p).dot(d2) + dp.dot(dp);
            if fd.abs() < EPS { break; }
            t -= f / fd;
        }
        t.rem_euclid(std::f64::consts::TAU)
    }

    pub fn distance_to_point(&self, p: Vec2) -> f64 {
        let t = self.nearest_param(p);
        self.point_at(t).dist(p)
    }
}

#[cfg(test)]
mod transform_tests {
    use super::*;
    use crate::math::approx_eq;

    #[test]
    fn line_rotated_90_around_origin() {
        let g = Geom::Line(Line { a: Vec2::new(1.0, 0.0), b: Vec2::new(2.0, 0.0) });
        let r = g.rotated(Vec2::ZERO, std::f64::consts::FRAC_PI_2);
        if let Geom::Line(l) = r {
            assert!(approx_eq(l.a.x, 0.0)); assert!(approx_eq(l.a.y, 1.0));
            assert!(approx_eq(l.b.x, 0.0)); assert!(approx_eq(l.b.y, 2.0));
        } else { panic!(); }
    }

    #[test]
    fn circle_scaled_2x_around_origin() {
        let g = Geom::Circle(Circle { center: Vec2::new(5.0, 0.0), radius: 3.0 });
        let s = g.scaled(Vec2::ZERO, 2.0);
        if let Geom::Circle(c) = s {
            assert!(approx_eq(c.center.x, 10.0));
            assert!(approx_eq(c.radius, 6.0));
        } else { panic!(); }
    }

    #[test]
    fn line_mirrored_across_x_axis() {
        // Axis from (-10,0) to (10,0). Point (3,4) mirrors to (3,-4).
        let g = Geom::Line(Line { a: Vec2::new(3.0, 4.0), b: Vec2::new(8.0, 2.0) });
        let m = g.mirrored(Vec2::new(-10.0, 0.0), Vec2::new(10.0, 0.0));
        if let Geom::Line(l) = m {
            assert!(approx_eq(l.a.x, 3.0)); assert!(approx_eq(l.a.y, -4.0));
            assert!(approx_eq(l.b.x, 8.0)); assert!(approx_eq(l.b.y, -2.0));
        } else { panic!(); }
    }

    #[test]
    fn line_reversed_swaps_endpoints() {
        let g = Geom::Line(Line { a: Vec2::new(1.0, 2.0), b: Vec2::new(7.0, 9.0) });
        if let Geom::Line(l) = g.reversed() {
            assert!(approx_eq(l.a.x, 7.0)); assert!(approx_eq(l.a.y, 9.0));
            assert!(approx_eq(l.b.x, 1.0)); assert!(approx_eq(l.b.y, 2.0));
        } else { panic!(); }
    }

    #[test]
    fn arc_reversed_swaps_endpoint_param() {
        // Arc from 0° to 90°. Reversing puts start at 90° with same sweep.
        let g = Geom::Arc(Arc {
            center: Vec2::ZERO, radius: 5.0,
            start_angle: 0.0,
            sweep_angle: std::f64::consts::FRAC_PI_2,
        });
        if let Geom::Arc(a) = g.reversed() {
            assert!(approx_eq(a.start_angle, std::f64::consts::FRAC_PI_2));
            assert!(approx_eq(a.sweep_angle, std::f64::consts::FRAC_PI_2));
        } else { panic!(); }
    }

    #[test]
    fn polyline_reversed_flips_vertex_order_and_bulges() {
        let g = Geom::Polyline(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.2 },
                PolyVertex { pos: Vec2::new(1.0, 0.0), bulge: -0.4 },
                PolyVertex { pos: Vec2::new(1.0, 1.0), bulge: 0.0 },
            ],
            closed: false,
        });
        if let Geom::Polyline(p) = g.reversed() {
            assert_eq!(p.vertices[0].pos, Vec2::new(1.0, 1.0));
            assert_eq!(p.vertices[2].pos, Vec2::new(0.0, 0.0));
            // bulge[0] of reversed = -original bulge[1] (shifted+sign-flipped)
            assert!(approx_eq(p.vertices[0].bulge, 0.4));
            assert!(approx_eq(p.vertices[1].bulge, -0.2));
        } else { panic!(); }
    }

    #[test]
    fn offset_line_picks_side_by_hint() {
        // Horizontal line from (0,0) to (10,0). Side hint above → offset up.
        let g = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let up = g.offset(2.0, Vec2::new(5.0, 5.0)).unwrap();
        if let Geom::Line(l) = up {
            assert!(approx_eq(l.a.y, 2.0)); assert!(approx_eq(l.b.y, 2.0));
        } else { panic!(); }
        let dn = g.offset(2.0, Vec2::new(5.0, -5.0)).unwrap();
        if let Geom::Line(l) = dn {
            assert!(approx_eq(l.a.y, -2.0)); assert!(approx_eq(l.b.y, -2.0));
        } else { panic!(); }
    }

    #[test]
    fn offset_circle_outward_grows_radius() {
        let g = Geom::Circle(Circle { center: Vec2::ZERO, radius: 5.0 });
        let outside_hint = Vec2::new(10.0, 0.0);
        let out = g.offset(2.0, outside_hint).unwrap();
        if let Geom::Circle(c) = out { assert!(approx_eq(c.radius, 7.0)); } else { panic!(); }
        let inside_hint = Vec2::new(1.0, 0.0);
        let inn = g.offset(2.0, inside_hint).unwrap();
        if let Geom::Circle(c) = inn { assert!(approx_eq(c.radius, 3.0)); } else { panic!(); }
    }

    #[test]
    fn offset_closed_rectangle_inward_shrinks() {
        // 10×6 rectangle, CCW. Click INSIDE → offset shrinks it by `dist`
        // on every edge (8×4, same 4 corners pulled in).
        let g = Geom::Polyline(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(10.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(10.0, 6.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(0.0, 6.0), bulge: 0.0 },
            ],
            closed: true,
        });
        let out = g.offset(1.0, Vec2::new(5.0, 3.0)).expect("offset ok");
        let Geom::Polyline(pl) = out else { panic!("not a polyline") };
        assert!(pl.closed);
        assert_eq!(pl.vertices.len(), 4);
        let corners: Vec<Vec2> = pl.vertices.iter().map(|x| x.pos).collect();
        // Expect the inset rectangle (1,1)-(9,5).
        let want = [
            Vec2::new(1.0, 1.0), Vec2::new(9.0, 1.0),
            Vec2::new(9.0, 5.0), Vec2::new(1.0, 5.0),
        ];
        for w in &want {
            assert!(corners.iter().any(|c| approx_eq(c.x, w.x) && approx_eq(c.y, w.y)),
                "missing inset corner {:?} in {:?}", w, corners);
        }
    }

    #[test]
    fn offset_closed_rectangle_outward_grows() {
        let g = Geom::Polyline(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(10.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(10.0, 6.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(0.0, 6.0), bulge: 0.0 },
            ],
            closed: true,
        });
        // Click OUTSIDE (below the bottom edge) → grow.
        let out = g.offset(1.0, Vec2::new(5.0, -3.0)).expect("offset ok");
        let Geom::Polyline(pl) = out else { panic!("not a polyline") };
        let corners: Vec<Vec2> = pl.vertices.iter().map(|x| x.pos).collect();
        let want = [
            Vec2::new(-1.0, -1.0), Vec2::new(11.0, -1.0),
            Vec2::new(11.0, 7.0), Vec2::new(-1.0, 7.0),
        ];
        for w in &want {
            assert!(corners.iter().any(|c| approx_eq(c.x, w.x) && approx_eq(c.y, w.y)),
                "missing grown corner {:?} in {:?}", w, corners);
        }
    }

    #[test]
    fn offset_polyline_arc_segment_concentric() {
        // Single arc segment: quarter circle (1,0)→(0,1), centre origin,
        // CCW (bulge = tan(22.5°)). Offset OUTWARD → concentric radius 2,
        // same bulge.
        let b = (std::f64::consts::FRAC_PI_8).tan();   // tan(22.5°)
        let g = Geom::Polyline(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(1.0, 0.0), bulge: b },
                PolyVertex { pos: Vec2::new(0.0, 1.0), bulge: 0.0 },
            ],
            closed: false,
        });
        let out = g.offset(1.0, Vec2::new(1.0, 1.0)).expect("offset ok");
        let Geom::Polyline(pl) = out else { panic!("not a polyline") };
        assert_eq!(pl.vertices.len(), 2);
        assert!(approx_eq(pl.vertices[0].pos.x, 2.0) && approx_eq(pl.vertices[0].pos.y, 0.0),
            "arc start {:?} should scale to (2,0)", pl.vertices[0].pos);
        assert!(approx_eq(pl.vertices[1].pos.x, 0.0) && approx_eq(pl.vertices[1].pos.y, 2.0),
            "arc end {:?} should scale to (0,2)", pl.vertices[1].pos);
        assert!(approx_eq(pl.vertices[0].bulge, b), "bulge preserved (concentric)");
    }

    #[test]
    fn offset_open_polyline_two_segments() {
        // L-shape (open): (0,0)-(10,0)-(10,10). Offset to the +Y/-X inside.
        let g = Geom::Polyline(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(10.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(10.0, 10.0), bulge: 0.0 },
            ],
            closed: false,
        });
        let out = g.offset(1.0, Vec2::new(5.0, 1.0)).expect("offset ok");
        let Geom::Polyline(pl) = out else { panic!("not a polyline") };
        assert!(!pl.closed);
        assert_eq!(pl.vertices.len(), 3);
        // The miter corner moves from (10,0) to (9,1).
        let mid = pl.vertices[1].pos;
        assert!(approx_eq(mid.x, 9.0) && approx_eq(mid.y, 1.0),
            "miter corner {:?} should be (9,1)", mid);
    }

    #[test]
    fn lengthen_line_extends_at_clicked_end() {
        let g = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        // Click near (10, 0) — the b-end — to extend it.
        let longer = g.lengthened(3.0, Vec2::new(10.0, 1.0)).unwrap();
        if let Geom::Line(l) = longer { assert!(approx_eq(l.b.x, 13.0)); } else { panic!(); }
        // Click near (0, 0) — the a-end — to extend backwards.
        let earlier = g.lengthened(3.0, Vec2::new(-1.0, 0.0)).unwrap();
        if let Geom::Line(l) = earlier { assert!(approx_eq(l.a.x, -3.0)); } else { panic!(); }
    }

    #[test]
    fn trim_line_at_single_cutter_keeps_other_side() {
        // Horizontal line 0→10. Vertical cutter at x=5. Click at x=7 (right
        // of the cut) → that side is removed; we keep 0→5.
        let target  = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let cutter  = Geom::Line(Line { a: Vec2::new(5.0, -5.0), b: Vec2::new(5.0, 5.0) });
        let out = target.trim_at(&[cutter], Vec2::new(7.0, 0.0), false).unwrap();
        assert_eq!(out.len(), 1);
        if let Geom::Line(l) = &out[0] {
            assert!(approx_eq(l.a.x, 0.0)); assert!(approx_eq(l.b.x, 5.0));
        } else { panic!(); }
    }

    #[test]
    fn trim_line_between_two_cutters_keeps_outer_pieces() {
        // 0→10 cut at x=3 and x=7; click at x=5 (middle) removes 3..7.
        // Two cuts = three segments; click middle → 2 outer pieces survive.
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let c1 = Geom::Line(Line { a: Vec2::new(3.0, -5.0), b: Vec2::new(3.0, 5.0) });
        let c2 = Geom::Line(Line { a: Vec2::new(7.0, -5.0), b: Vec2::new(7.0, 5.0) });
        let out = target.trim_at(&[c1, c2], Vec2::new(5.0, 0.0), false).unwrap();
        assert_eq!(out.len(), 2);
        let mut xs: Vec<(f64, f64)> = out.iter().map(|g| {
            if let Geom::Line(l) = g { (l.a.x, l.b.x) } else { panic!() }
        }).collect();
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        assert!(approx_eq(xs[0].0, 0.0)); assert!(approx_eq(xs[0].1, 3.0));
        assert!(approx_eq(xs[1].0, 7.0)); assert!(approx_eq(xs[1].1, 10.0));
    }

    #[test]
    fn trim_line_with_three_cutters_two_outer_pieces() {
        // AutoCAD-correct: line 0→10 with cuts at x=2, 5, 8. Click in
        // interval (2..5) → ONLY that interval is removed. Surviving
        // pieces (NOT further split at x=8): (0..2) and (5..10).
        // Updated from the prior N+1 over-split rule (2026-06-08 — user
        // bug report: trimming outside a closed cutter incorrectly split
        // the line at the far intersection too).
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let cs: Vec<Geom> = [2.0, 5.0, 8.0].iter().map(|&x| {
            Geom::Line(Line { a: Vec2::new(x, -5.0), b: Vec2::new(x, 5.0) })
        }).collect();
        let out = target.trim_at(&cs, Vec2::new(3.5, 0.0), false).unwrap();
        assert_eq!(out.len(), 2,
            "expected 2 surviving pieces (AutoCAD-style), got {}", out.len());
        let mut xs: Vec<(f64, f64)> = out.iter().map(|g| {
            if let Geom::Line(l) = g { (l.a.x, l.b.x) } else { panic!() }
        }).collect();
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        assert!(approx_eq(xs[0].0, 0.0) && approx_eq(xs[0].1, 2.0));
        assert!(approx_eq(xs[1].0, 5.0) && approx_eq(xs[1].1, 10.0));   // not split at 8!
    }

    #[test]
    fn trim_line_with_five_cutters_two_pieces_on_middle_click() {
        // AutoCAD-correct: 5 cuts at x=2,4,6,8,10; click in interval
        // (4..6) → 2 surviving pieces: (0..4) and (6..12). Non-bounding
        // intersections at x=2, 8, 10 do NOT cause splits.
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(12.0, 0.0) });
        let cs: Vec<Geom> = [2.0, 4.0, 6.0, 8.0, 10.0].iter().map(|&x| {
            Geom::Line(Line { a: Vec2::new(x, -5.0), b: Vec2::new(x, 5.0) })
        }).collect();
        let out = target.trim_at(&cs, Vec2::new(5.0, 0.0), false).unwrap();
        assert_eq!(out.len(), 2,
            "expected 2 surviving pieces (AutoCAD-style), got {}", out.len());
        let mut xs: Vec<(f64, f64)> = out.iter().map(|g| {
            if let Geom::Line(l) = g { (l.a.x, l.b.x) } else { panic!() }
        }).collect();
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        assert!(approx_eq(xs[0].0, 0.0) && approx_eq(xs[0].1, 4.0));
        assert!(approx_eq(xs[1].0, 6.0) && approx_eq(xs[1].1, 12.0));
    }

    #[test]
    fn trim_line_outside_closed_cutter_keeps_one_continuous_piece() {
        // User-reported bug 2026-06-08: line crosses an ellipse at TWO
        // points; click on the OUTSIDE-A portion (before first
        // intersection). Survivor should be ONE continuous piece from
        // intersection1 → line.end (passing through the inside AND
        // outside-B without further splits).
        let target = Geom::Line(Line {
            a: Vec2::new(0.0, 0.0), b: Vec2::new(10.0, 0.0),
        });
        // Two perpendicular cutters at x=3 and x=7 (simulate ellipse's
        // two intersections with the line).
        let c1 = Geom::Line(Line { a: Vec2::new(3.0, -2.0), b: Vec2::new(3.0, 2.0) });
        let c2 = Geom::Line(Line { a: Vec2::new(7.0, -2.0), b: Vec2::new(7.0, 2.0) });
        // Click before first intersection — on the OUTSIDE-A portion.
        let out = target.trim_at(&[c1, c2], Vec2::new(1.5, 0.0), false).unwrap();
        assert_eq!(out.len(), 1,
            "expected ONE continuous piece (3 → 10), got {}", out.len());
        if let Geom::Line(l) = &out[0] {
            assert!(approx_eq(l.a.x, 3.0));
            assert!(approx_eq(l.b.x, 10.0));
        }
    }

    #[test]
    fn trim_arc_with_two_cutters_breaks_into_two_subarcs_on_middle_click() {
        use std::f64::consts::FRAC_PI_2;
        // Half-arc 0°→180° at radius 5. Cuts at 60° and 120°. Click at 90°
        // (between the cuts) → segments (0..60) and (120..180) survive.
        let arc = Geom::Arc(Arc {
            center: Vec2::ZERO, radius: 5.0,
            start_angle: 0.0, sweep_angle: std::f64::consts::PI,
        });
        let cx_60 = 5.0 * (60.0_f64).to_radians().cos();
        let cy_60 = 5.0 * (60.0_f64).to_radians().sin();
        let cx_120 = 5.0 * (120.0_f64).to_radians().cos();
        let cy_120 = 5.0 * (120.0_f64).to_radians().sin();
        // Radial cut lines from origin through each angle, long enough.
        let c1 = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(cx_60 * 2.0, cy_60 * 2.0) });
        let c2 = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(cx_120 * 2.0, cy_120 * 2.0) });
        // Click at 90° (top of arc): (0, 5)
        let out = arc.trim_at(&[c1, c2], Vec2::new(0.0, 5.0), false).unwrap();
        assert_eq!(out.len(), 2, "expected 2 sub-arcs, got {}", out.len());
        for g in &out {
            if let Geom::Arc(a) = g {
                let sweep_deg = a.sweep_angle.to_degrees();
                assert!((sweep_deg - 60.0).abs() < 1.0,
                    "each surviving sub-arc should sweep ~60°, got {}°", sweep_deg);
            } else { panic!() }
        }
        let _ = FRAC_PI_2;
    }

    #[test]
    fn trim_uses_edge_mode_for_imaginary_intersection() {
        // Target line 0→10 at y=0. Cutter is a SHORT segment that, in its
        // visible form, does NOT cross the target. With EdgMod ON it
        // extends to a full line that DOES cross at x=5.
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let short_cutter = Geom::Line(Line {
            a: Vec2::new(5.0, 3.0), b: Vec2::new(5.0, 4.0),    // y=3..4 only
        });
        // EdgMod OFF — no intersection; trim fails.
        assert!(target.trim_at(&[short_cutter.clone()], Vec2::new(7.0, 0.0), false).is_err());
        // EdgMod ON — imaginary intersection at (5,0); right side trimmed.
        let out = target.trim_at(&[short_cutter], Vec2::new(7.0, 0.0), true).unwrap();
        assert_eq!(out.len(), 1);
        if let Geom::Line(l) = &out[0] {
            assert!(approx_eq(l.b.x, 5.0));
        } else { panic!(); }
    }

    #[test]
    fn trim_arc_at_single_radial_cutter_keeps_other_side() {
        // Quarter-arc 0°→90° at origin, radius 5. Cutter is a vertical line
        // at x = 5·cos(45°) ≈ 3.536, which intersects the arc at 45°.
        // Click at angle 22° (lower-left half) → removes that side, keeps 45°→90°.
        let arc = Geom::Arc(Arc {
            center: Vec2::ZERO, radius: 5.0,
            start_angle: 0.0, sweep_angle: std::f64::consts::FRAC_PI_2,
        });
        let cut_x = 5.0 * std::f64::consts::FRAC_1_SQRT_2;
        let cutter = Geom::Line(Line {
            a: Vec2::new(cut_x, -10.0), b: Vec2::new(cut_x, 10.0),
        });
        // Click below the cutter (small angle on the arc)
        let pick = Vec2::new(4.5, 1.0);   // ≈12° on the arc
        let out = arc.trim_at(&[cutter], pick, false).unwrap();
        assert_eq!(out.len(), 1);
        if let Geom::Arc(a) = &out[0] {
            // Should start at ≈45° and sweep to ≈90°
            assert!(a.start_angle > 0.5 && a.start_angle < 1.0,
                "start_angle = {} rad ({}°)", a.start_angle, a.start_angle.to_degrees());
            assert!((a.sweep_angle - std::f64::consts::FRAC_PI_4).abs() < 0.05,
                "sweep_angle = {} rad", a.sweep_angle);
        } else { panic!(); }
    }

    #[test]
    fn trim_target_with_disjoint_cutters_errors_when_pick_is_outside() {
        // Line 0→10. Cutter at x=15 (no intersection on the visible line).
        // EdgMod OFF — no imaginary intersection either. trim should fail.
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let cutter = Geom::Line(Line {
            a: Vec2::new(15.0, -1.0), b: Vec2::new(15.0, 1.0),
        });
        let r = target.trim_at(&[cutter], Vec2::new(5.0, 0.0), false);
        assert!(r.is_err());
    }

    #[test]
    fn trim_with_pick_on_endpoint_side_of_intersection() {
        // Line 0→10 cut at x=5. Click at x=2 (left of cut) → trims toward a;
        // result keeps the right half (5..10).
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let cutter = Geom::Line(Line { a: Vec2::new(5.0, -1.0), b: Vec2::new(5.0, 1.0) });
        let out = target.trim_at(&[cutter], Vec2::new(2.0, 0.0), false).unwrap();
        assert_eq!(out.len(), 1);
        if let Geom::Line(l) = &out[0] {
            assert!(approx_eq(l.a.x, 5.0)); assert!(approx_eq(l.b.x, 10.0));
        } else { panic!(); }
    }

    #[test]
    fn trim_uses_visible_intersection_first_with_edgemode_off() {
        // Cutter VISIBLY intersects target. EdgMod OFF: should use the real
        // intersection (no need to extend). Confirms we don't accidentally
        // always extend.
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let cutter = Geom::Line(Line { a: Vec2::new(5.0, -5.0), b: Vec2::new(5.0, 5.0) });
        let out = target.trim_at(&[cutter.clone()], Vec2::new(7.0, 0.0), false).unwrap();
        // Same result as the EdgMod-ON case for this geometry; assert we
        // got the trimmed left half.
        if let Geom::Line(l) = &out[0] {
            assert!(approx_eq(l.b.x, 5.0));
        } else { panic!(); }
    }

    #[test]
    fn extend_arc_lengthens_sweep_to_boundary() {
        // Quarter-arc 0°→90° at radius 5. Boundary line at y=5 (already
        // tangent-ish to top of arc at 90°). Pick a point past the 90°
        // endpoint to extend forward; expect sweep grows past 90°.
        // Simpler boundary: vertical line at x = -2 (cuts arc extension
        // at angle just past 90°).
        let arc = Geom::Arc(Arc {
            center: Vec2::ZERO, radius: 5.0,
            start_angle: 0.0, sweep_angle: std::f64::consts::FRAC_PI_2,
        });
        let boundary = Geom::Line(Line {
            a: Vec2::new(-2.0, -10.0), b: Vec2::new(-2.0, 10.0),
        });
        // End of arc is at (0, 5). Click slightly past it (above and left).
        let pick = Vec2::new(-0.5, 5.5);
        let out = arc.extend_to(&[boundary], pick, true).unwrap();
        if let Geom::Arc(a) = out {
            // Sweep should be > original PI/2 but < PI (since x=-2 → angle ≈113°)
            assert!(a.sweep_angle > std::f64::consts::FRAC_PI_2,
                "sweep didn't grow: {}", a.sweep_angle);
            assert!(a.sweep_angle < std::f64::consts::PI,
                "sweep overshot: {}", a.sweep_angle);
        } else { panic!(); }
    }

    #[test]
    fn document_level_apply_trim_pick_shape() {
        // Simulate what apply_trim_pick does in cad_app: trim target,
        // remove it from the Document, push the surviving pieces. Confirms
        // the index-shift logic produces a coherent Document.
        use crate::Document;
        let mut doc = Document::default();
        // Cutter at index 0, target at index 1.
        let cutter_i = doc.push(Line {
            a: Vec2::new(5.0, -5.0), b: Vec2::new(5.0, 5.0),
        }.into());
        let target_i = doc.push(Line {
            a: Vec2::ZERO, b: Vec2::new(10.0, 0.0),
        }.into());
        assert_eq!(cutter_i, 0);
        assert_eq!(target_i, 1);

        // Mirror apply_trim_pick:
        let cutter_geom = doc.dobjects[cutter_i].geom.clone();
        let target_style = doc.dobjects[target_i].style;
        let pieces = doc.dobjects[target_i].geom
            .trim_at(&[cutter_geom], Vec2::new(7.0, 0.0), true)
            .unwrap();
        doc.dobjects.remove(target_i);
        for g in pieces {
            let mut d = crate::DObject::new(g);
            d.style = target_style;
            doc.push(d);
        }
        // Now doc.dobjects = [cutter, piece]; total 2, both Lines.
        assert_eq!(doc.dobjects.len(), 2);
        // Cutter still at index 0 (unchanged).
        if let Geom::Line(l) = &doc.dobjects[0].geom {
            assert!(approx_eq(l.a.x, 5.0));
        } else { panic!("cutter shifted or mutated"); }
        // Piece is the original LEFT half (since pick was on the right at x=7).
        if let Geom::Line(l) = &doc.dobjects[1].geom {
            assert!(approx_eq(l.a.x, 0.0));
            assert!(approx_eq(l.b.x, 5.0));
        } else { panic!(); }
    }

    #[test]
    fn extend_line_grows_toward_boundary() {
        // Target line 0→4 at y=0. Boundary at x=10.
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(4.0, 0.0) });
        let boundary = Geom::Line(Line { a: Vec2::new(10.0, -5.0), b: Vec2::new(10.0, 5.0) });
        // Click near the right end (b) — extend that side toward x=10.
        let out = target.extend_to(&[boundary], Vec2::new(4.0, 0.5), false).unwrap();
        if let Geom::Line(l) = out {
            assert!(approx_eq(l.a.x, 0.0));
            assert!(approx_eq(l.b.x, 10.0));
        } else { panic!(); }
    }

    #[test]
    fn break_line_at_midpoint_makes_two() {
        let g = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let (l1, l2) = g.split_at(Vec2::new(5.0, 0.0)).unwrap();
        if let (Geom::Line(a), Geom::Line(b)) = (l1, l2) {
            assert!(approx_eq(a.b.x, 5.0));
            assert!(approx_eq(b.a.x, 5.0));
            assert!(approx_eq(b.b.x, 10.0));
        } else { panic!(); }
    }

    #[test]
    fn break_circle_errors() {
        let g = Geom::Circle(Circle { center: Vec2::ZERO, radius: 5.0 });
        assert!(g.split_at(Vec2::new(5.0, 0.0)).is_err());
    }

    #[test]
    fn translate_then_rotate_then_translate_back() {
        // Translation invariance under rotation around the SAME pivot
        let g = Geom::Circle(Circle { center: Vec2::new(7.0, 3.0), radius: 2.0 });
        let g2 = g.translated(Vec2::new(10.0, 0.0));
        let g3 = g2.rotated(Vec2::new(17.0, 3.0), std::f64::consts::PI);
        let g4 = g3.translated(Vec2::new(-10.0, 0.0));
        if let Geom::Circle(c) = g4 {
            // Should be rotated 180° about (7,3) which moves (7,3)+r=2 to the same spot
            assert!(approx_eq(c.center.x, 7.0));
            assert!(approx_eq(c.center.y, 3.0));
            assert!(approx_eq(c.radius, 2.0));
        } else { panic!(); }
    }
}

#[cfg(test)]
mod ellipse_tests {
    use super::*;
    use crate::math::approx_eq;

    fn close(p: Vec2, x: f64, y: f64) -> bool {
        approx_eq(p.x, x) && approx_eq(p.y, y)
    }

    #[test]
    fn axis_aligned_ellipse_point_at() {
        // a = 5, b = 2, no rotation
        let e = Ellipse { center: Vec2::ZERO, major: Vec2::new(5.0, 0.0), ratio: 0.4 };
        assert!(close(e.point_at(0.0), 5.0, 0.0));
        assert!(close(e.point_at(std::f64::consts::FRAC_PI_2), 0.0, 2.0));
        assert!(close(e.point_at(std::f64::consts::PI), -5.0, 0.0));
    }

    #[test]
    fn rotated_ellipse_bbox() {
        // Rotate 90°: major now points up. Bbox half-extents swap.
        let e = Ellipse { center: Vec2::ZERO, major: Vec2::new(0.0, 5.0), ratio: 0.4 };
        let (mn, mx) = e.bbox();
        assert!(approx_eq(mx.x, 2.0));   // semi-minor along x
        assert!(approx_eq(mx.y, 5.0));   // semi-major along y
        assert!(approx_eq(mn.x, -2.0));
        assert!(approx_eq(mn.y, -5.0));
    }

    #[test]
    fn nearest_param_on_circle_is_atan2() {
        // ratio=1.0 means circle — nearest point on a circle is the radial.
        let e = Ellipse { center: Vec2::ZERO, major: Vec2::new(5.0, 0.0), ratio: 1.0 };
        let p = Vec2::new(10.0, 10.0);
        let t = e.nearest_param(p);
        let pt = e.point_at(t);
        // pt should be on the circle of r=5 in the same direction as p.
        assert!(approx_eq(pt.len(), 5.0));
        assert!(approx_eq((pt.y / pt.x).atan(), (10.0_f64 / 10.0_f64).atan()));
    }

    #[test]
    fn ellipse_arc_endpoints_and_contains() {
        let e = Ellipse { center: Vec2::ZERO, major: Vec2::new(5.0, 0.0), ratio: 0.4 };
        let ea = EllipseArc {
            ellipse: e,
            start_param: 0.0,
            sweep_param: std::f64::consts::FRAC_PI_2,
        };
        let (p1, p2) = ea.endpoints();
        assert!(close(p1, 5.0, 0.0));
        assert!(close(p2, 0.0, 2.0));
        assert!(ea.contains_param(0.0));
        assert!(ea.contains_param(std::f64::consts::FRAC_PI_4));
        assert!(!ea.contains_param(std::f64::consts::PI));
    }
}

impl EllipseArc {
    /// True if parameter `t` lies in the swept range, mod TAU.
    pub fn contains_param(&self, t: f64) -> bool {
        let d = (t - self.start_param).rem_euclid(std::f64::consts::TAU);
        d <= self.sweep_param + EPS
    }

    pub fn endpoints(&self) -> (Vec2, Vec2) {
        (self.ellipse.point_at(self.start_param),
         self.ellipse.point_at(self.start_param + self.sweep_param))
    }

    /// Distance from the visible arc to a point. If the nearest-on-full-
    /// ellipse parameter lies in the swept range, that's the foot; otherwise
    /// the answer is whichever endpoint is closer.
    pub fn distance_to_point(&self, p: Vec2) -> f64 {
        let t = self.ellipse.nearest_param(p);
        if self.contains_param(t) {
            self.ellipse.point_at(t).dist(p)
        } else {
            let (e1, e2) = self.endpoints();
            p.dist(e1).min(p.dist(e2))
        }
    }
}

/// Characteristic role of a grip point — drives `Geom::with_grip_moved`.
/// Two opposing quadrant/axis tips (e.g. circle +X and -X) share the same
/// role because their drag semantic is symmetric (radius change / axis
/// reflection); the renderer still draws all four squares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GripRole {
    LineEndA,
    LineEndB,
    LineMid,
    CircleCenter,
    CircleQuadrant,
    ArcEndStart,
    ArcEndEnd,
    ArcMid,
    ArcCenter,
    EllipseCenter,
    EllipseMajorTip,        // either +major or -major tip
    EllipseMinorTip,        // either +minor or -minor tip
    EllipseArcEndStart,
    EllipseArcEndEnd,
    EllipseArcCenter,
    PolyVertex(usize),
    PointLoc,
    /// Block instance insertion point — dragging it translates the
    /// whole instance.
    BlockInsert,
    /// Control point of a NURBS spline at the given index. Dragging
    /// reshapes the curve via control-point edit (the dragged point
    /// stays put; the curve bends towards it).
    SplineCtrlPt(usize),
    /// Dimension def-point grips. Three per Dim:
    ///   * `DimP1`     — Linear.p1, or Radius/Diameter.center
    ///   * `DimP2`     — Linear.p2, or Radius/Diameter.on_circle
    ///   * `DimLeader` — Linear.dimline_pos, or Radius/Diameter.leader_end
    /// `with_grip_moved` interprets the role against the current
    /// DimKind.
    DimP1,
    DimP2,
    DimLeader,
}

impl Geom {
    /// Characteristic grip points + their roles. Renderer draws a square
    /// at each position; `with_grip_moved` knows how to edit the geometry
    /// based on the role.
    pub fn grip_points(&self) -> Vec<(Vec2, GripRole)> {
        match self {
            Geom::Line(l) => vec![
                (l.a, GripRole::LineEndA),
                (l.b, GripRole::LineEndB),
                ((l.a + l.b) * 0.5, GripRole::LineMid),
            ],
            Geom::Circle(c) => {
                let r = c.radius;
                vec![
                    (c.center, GripRole::CircleCenter),
                    (c.center + Vec2::new( r, 0.0), GripRole::CircleQuadrant),
                    (c.center + Vec2::new( 0.0,  r), GripRole::CircleQuadrant),
                    (c.center + Vec2::new(-r, 0.0), GripRole::CircleQuadrant),
                    (c.center + Vec2::new( 0.0, -r), GripRole::CircleQuadrant),
                ]
            }
            Geom::Arc(a) => {
                let (e1, e2) = a.endpoints();
                let mid_t = a.start_angle + a.sweep_angle * 0.5;
                let mid   = a.center + Vec2::new(
                    a.radius * mid_t.cos(),
                    a.radius * mid_t.sin(),
                );
                vec![
                    (e1,        GripRole::ArcEndStart),
                    (e2,        GripRole::ArcEndEnd),
                    (mid,       GripRole::ArcMid),
                    (a.center,  GripRole::ArcCenter),
                ]
            }
            Geom::Ellipse(el) => {
                let half = std::f64::consts::FRAC_PI_2;
                vec![
                    (el.center,                                GripRole::EllipseCenter),
                    (el.point_at(0.0),                          GripRole::EllipseMajorTip),
                    (el.point_at(half),                         GripRole::EllipseMinorTip),
                    (el.point_at(std::f64::consts::PI),         GripRole::EllipseMajorTip),
                    (el.point_at(std::f64::consts::PI + half),  GripRole::EllipseMinorTip),
                ]
            }
            Geom::EllipseArc(ea) => {
                let (e1, e2) = ea.endpoints();
                vec![
                    (e1, GripRole::EllipseArcEndStart),
                    (e2, GripRole::EllipseArcEndEnd),
                    (ea.ellipse.center, GripRole::EllipseArcCenter),
                ]
            }
            Geom::Polyline(p) => p.vertices.iter().enumerate()
                .map(|(i, v)| (v.pos, GripRole::PolyVertex(i)))
                .collect(),
            Geom::Point(p) => vec![(p.location, GripRole::PointLoc)],
            // Hatch MVP exposes no grips. The boundary vertices could
            // become PolyVertex grips later; for now editing happens by
            // recreating the hatch from a different boundary.
            Geom::Hatch(_) => Vec::new(),
            // Spline grips = every control point. Dragging one
            // reshapes the curve locally (NURBS basis support is
            // narrow — drag at index i only affects a span of degree+1
            // segments around i).
            Geom::Spline(s) => s.control_points.iter().enumerate()
                .map(|(i, p)| (*p, GripRole::SplineCtrlPt(i)))
                .collect(),
            // Wall — grip at each centerline endpoint + midpoint.
            // Re-uses Line grip roles so the existing renderer + apply
            // path "just work"; `with_grip_moved` below maps them back.
            Geom::Wall(w) => vec![
                (w.start, GripRole::LineEndA),
                (w.end,   GripRole::LineEndB),
                ((w.start + w.end) * 0.5, GripRole::LineMid),
            ],
            // Text — one grip at the anchor position. Re-uses PointLoc
            // role so `with_grip_moved` repositions text the same way
            // it repositions Points.
            Geom::Text(t) => vec![(t.position, GripRole::PointLoc)],
            // Dimension — three role-tagged grips matching the def
            // points. Roles are uniform across kinds so the renderer
            // can treat them generically.
            Geom::Dimension(d) => {
                use crate::dim::DimKind;
                match &d.kind {
                    DimKind::Linear { p1, p2, dimline_pos, .. } => vec![
                        (*p1,          GripRole::DimP1),
                        (*p2,          GripRole::DimP2),
                        (*dimline_pos, GripRole::DimLeader),
                    ],
                    DimKind::Radius { center, on_circle, leader_end } |
                    DimKind::Diameter { center, on_circle, leader_end } => vec![
                        (*center,     GripRole::DimP1),
                        (*on_circle,  GripRole::DimP2),
                        (*leader_end, GripRole::DimLeader),
                    ],
                }
            }
            // BlockRef — one grip at the insertion point; dragging it
            // translates the whole instance.
            Geom::BlockRef(br) => vec![(br.insert, GripRole::BlockInsert)],
        }
    }

    /// Produce a new geometry with the given grip moved to `new_pos`.
    /// All math reuses kernel methods already in this file — this is
    /// just per-role dispatch.
    pub fn with_grip_moved(&self, role: GripRole, new_pos: Vec2) -> Geom {
        match (self, role) {
            // ---- Line --------------------------------------------------
            (Geom::Line(l), GripRole::LineEndA) =>
                Geom::Line(Line { a: new_pos, b: l.b }),
            (Geom::Line(l), GripRole::LineEndB) =>
                Geom::Line(Line { a: l.a, b: new_pos }),
            (Geom::Line(l), GripRole::LineMid) => {
                let delta = new_pos - (l.a + l.b) * 0.5;
                Geom::Line(Line { a: l.a + delta, b: l.b + delta })
            }
            // ---- Circle ------------------------------------------------
            (Geom::Circle(c), GripRole::CircleCenter) =>
                Geom::Circle(Circle { center: new_pos, radius: c.radius }),
            (Geom::Circle(c), GripRole::CircleQuadrant) => {
                let new_r = (new_pos - c.center).len().max(EPS);
                Geom::Circle(Circle { center: c.center, radius: new_r })
            }
            // ---- Arc ---------------------------------------------------
            (Geom::Arc(a), GripRole::ArcCenter) => {
                let delta = new_pos - a.center;
                Geom::Arc(Arc {
                    center: a.center + delta,
                    radius: a.radius,
                    start_angle: a.start_angle,
                    sweep_angle: a.sweep_angle,
                })
            }
            (Geom::Arc(a), GripRole::ArcEndStart) => {
                // Slide start endpoint along the radial direction at the
                // new angle; sweep adjusts so the END endpoint stays put.
                let new_start = (new_pos - a.center).angle()
                    .rem_euclid(std::f64::consts::TAU);
                let old_end_abs = (a.start_angle + a.sweep_angle)
                    .rem_euclid(std::f64::consts::TAU);
                let new_sweep = (old_end_abs - new_start)
                    .rem_euclid(std::f64::consts::TAU);
                Geom::Arc(Arc {
                    center: a.center,
                    radius: a.radius,
                    start_angle: new_start,
                    sweep_angle: new_sweep.max(EPS),
                })
            }
            (Geom::Arc(a), GripRole::ArcEndEnd) => {
                let new_end_abs = (new_pos - a.center).angle()
                    .rem_euclid(std::f64::consts::TAU);
                let new_sweep = (new_end_abs - a.start_angle)
                    .rem_euclid(std::f64::consts::TAU);
                Geom::Arc(Arc {
                    center: a.center,
                    radius: a.radius,
                    start_angle: a.start_angle,
                    sweep_angle: new_sweep.max(EPS),
                })
            }
            (Geom::Arc(a), GripRole::ArcMid) => {
                // v1: translate whole arc so the midpoint lands at new_pos.
                let mid_t = a.start_angle + a.sweep_angle * 0.5;
                let mid = a.center + Vec2::new(
                    a.radius * mid_t.cos(),
                    a.radius * mid_t.sin(),
                );
                let delta = new_pos - mid;
                Geom::Arc(Arc {
                    center: a.center + delta,
                    radius: a.radius,
                    start_angle: a.start_angle,
                    sweep_angle: a.sweep_angle,
                })
            }
            // ---- Ellipse -----------------------------------------------
            (Geom::Ellipse(el), GripRole::EllipseCenter) =>
                Geom::Ellipse(Ellipse {
                    center: new_pos,
                    major:  el.major,
                    ratio:  el.ratio,
                }),
            (Geom::Ellipse(el), GripRole::EllipseMajorTip) => {
                // Major axis becomes (new_pos - center). Ratio (b/a) stays
                // so b scales proportionally. Direction of the ellipse
                // also rotates to match.
                let new_major = new_pos - el.center;
                if new_major.len() < EPS {
                    return Geom::Ellipse(*el);
                }
                Geom::Ellipse(Ellipse {
                    center: el.center,
                    major:  new_major,
                    ratio:  el.ratio,
                })
            }
            (Geom::Ellipse(el), GripRole::EllipseMinorTip) => {
                // Hold major direction + length; change ratio so the
                // minor tip lands at the projection of new_pos onto v̂.
                let new_b = (new_pos - el.center).dot(el.v_hat()).abs().max(EPS);
                let new_ratio = (new_b / el.semi_major()).max(1e-6);
                Geom::Ellipse(Ellipse {
                    center: el.center,
                    major:  el.major,
                    ratio:  new_ratio,
                })
            }
            // ---- EllipseArc --------------------------------------------
            (Geom::EllipseArc(ea), GripRole::EllipseArcCenter) => {
                let delta = new_pos - ea.ellipse.center;
                Geom::EllipseArc(EllipseArc {
                    ellipse: Ellipse {
                        center: ea.ellipse.center + delta,
                        ..ea.ellipse
                    },
                    start_param: ea.start_param,
                    sweep_param: ea.sweep_param,
                })
            }
            (Geom::EllipseArc(ea), GripRole::EllipseArcEndStart) => {
                // Re-project new_pos to the ellipse parameter; new start_param
                // = that t; sweep_param adjusts so the END endpoint stays.
                let new_start = ea.ellipse.nearest_param(new_pos)
                    .rem_euclid(std::f64::consts::TAU);
                let old_end = (ea.start_param + ea.sweep_param)
                    .rem_euclid(std::f64::consts::TAU);
                let new_sweep = (old_end - new_start)
                    .rem_euclid(std::f64::consts::TAU);
                Geom::EllipseArc(EllipseArc {
                    ellipse: ea.ellipse,
                    start_param: new_start,
                    sweep_param: new_sweep.max(EPS),
                })
            }
            (Geom::EllipseArc(ea), GripRole::EllipseArcEndEnd) => {
                let new_end = ea.ellipse.nearest_param(new_pos)
                    .rem_euclid(std::f64::consts::TAU);
                let new_sweep = (new_end - ea.start_param)
                    .rem_euclid(std::f64::consts::TAU);
                Geom::EllipseArc(EllipseArc {
                    ellipse: ea.ellipse,
                    start_param: ea.start_param,
                    sweep_param: new_sweep.max(EPS),
                })
            }
            // ---- Polyline ----------------------------------------------
            (Geom::Polyline(p), GripRole::PolyVertex(i)) => {
                let mut new_verts = p.vertices.clone();
                if let Some(v) = new_verts.get_mut(i) { v.pos = new_pos; }
                Geom::Polyline(Polyline { vertices: new_verts, closed: p.closed })
            }
            // ---- Spline (control-point edit) ---------------------------
            (Geom::Spline(s), GripRole::SplineCtrlPt(i)) => {
                let mut new_ctrls = s.control_points.clone();
                if let Some(c) = new_ctrls.get_mut(i) { *c = new_pos; }
                Geom::Spline(Spline {
                    degree:         s.degree,
                    control_points: new_ctrls,
                    weights:        s.weights.clone(),
                })
            }
            // ---- Point -------------------------------------------------
            (Geom::Point(p), GripRole::PointLoc) =>
                Geom::Point(Point { location: new_pos, style: p.style, size: p.size }),
            // ---- Text — re-uses PointLoc role for its single anchor grip.
            (Geom::Text(t), GripRole::PointLoc) => {
                let mut nt = t.clone();
                nt.position = new_pos;
                Geom::Text(nt)
            }
            // ---- Wall — re-uses Line grip roles --------------------------
            // Grip drags reshape the centerline; both side lines re-derive
            // on render so the wall moves coherently as one entity.
            (Geom::Wall(w), GripRole::LineEndA) => Geom::Wall(Wall {
                start: new_pos, end: w.end, thickness: w.thickness,
                style: w.style, bulge: w.bulge,
            }),
            (Geom::Wall(w), GripRole::LineEndB) => Geom::Wall(Wall {
                start: w.start, end: new_pos, thickness: w.thickness,
                style: w.style, bulge: w.bulge,
            }),
            (Geom::Wall(w), GripRole::LineMid) => {
                // Move the whole wall — translate by (new_pos - current mid).
                let mid = (w.start + w.end) * 0.5;
                let off = new_pos - mid;
                Geom::Wall(Wall {
                    start: w.start + off, end: w.end + off,
                    thickness: w.thickness,
                    style: w.style, bulge: w.bulge,
                })
            }
            // ---- Dimension --------------------------------------------------
            // DimP1/P2/Leader move whichever def point corresponds to
            // the role for the current DimKind. Renderer re-derives
            // extension lines, dim line, and arrows on the next frame.
            (Geom::Dimension(d), GripRole::DimP1) => {
                use crate::dim::DimKind;
                let new_kind = match &d.kind {
                    DimKind::Linear { p2, dimline_pos, ortho, .. } => DimKind::Linear {
                        p1: new_pos, p2: *p2, dimline_pos: *dimline_pos, ortho: *ortho,
                    },
                    DimKind::Radius { on_circle, leader_end, .. } => DimKind::Radius {
                        center: new_pos, on_circle: *on_circle, leader_end: *leader_end,
                    },
                    DimKind::Diameter { on_circle, leader_end, .. } => DimKind::Diameter {
                        center: new_pos, on_circle: *on_circle, leader_end: *leader_end,
                    },
                };
                Geom::Dimension(crate::dim::Dim {
                    kind: new_kind, style: d.style, text_override: d.text_override.clone(),
                })
            }
            (Geom::Dimension(d), GripRole::DimP2) => {
                use crate::dim::DimKind;
                let new_kind = match &d.kind {
                    DimKind::Linear { p1, dimline_pos, ortho, .. } => DimKind::Linear {
                        p1: *p1, p2: new_pos, dimline_pos: *dimline_pos, ortho: *ortho,
                    },
                    DimKind::Radius { center, leader_end, .. } => DimKind::Radius {
                        center: *center, on_circle: new_pos, leader_end: *leader_end,
                    },
                    DimKind::Diameter { center, leader_end, .. } => DimKind::Diameter {
                        center: *center, on_circle: new_pos, leader_end: *leader_end,
                    },
                };
                Geom::Dimension(crate::dim::Dim {
                    kind: new_kind, style: d.style, text_override: d.text_override.clone(),
                })
            }
            (Geom::Dimension(d), GripRole::DimLeader) => {
                use crate::dim::DimKind;
                let new_kind = match &d.kind {
                    DimKind::Linear { p1, p2, ortho, .. } => DimKind::Linear {
                        p1: *p1, p2: *p2, dimline_pos: new_pos, ortho: *ortho,
                    },
                    DimKind::Radius { center, on_circle, .. } => DimKind::Radius {
                        center: *center, on_circle: *on_circle, leader_end: new_pos,
                    },
                    DimKind::Diameter { center, on_circle, .. } => DimKind::Diameter {
                        center: *center, on_circle: *on_circle, leader_end: new_pos,
                    },
                };
                Geom::Dimension(crate::dim::Dim {
                    kind: new_kind, style: d.style, text_override: d.text_override.clone(),
                })
            }
            // BlockRef — the insertion grip carries the whole instance.
            (Geom::BlockRef(br), GripRole::BlockInsert) => {
                Geom::BlockRef(crate::block::BlockRef {
                    insert: new_pos, ..*br
                })
            }
            // Mismatched (role, geom) — return unchanged.
            (g, _) => g.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Polyline segments — explode into independent Line / Arc geoms.
//
// Each vertex `i` owns the bulge for segment `i → (i+1)`. Straight when
// bulge == 0; otherwise an Arc derived from chord + DXF bulge formula.
// Closed polylines also produce the closing segment (last → first).
// ---------------------------------------------------------------------------
pub fn polyline_segments(p: &Polyline) -> Vec<Geom> {
    let n = p.vertices.len();
    if n < 2 { return Vec::new(); }
    let seg_count = if p.closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(seg_count);
    for i in 0..seg_count {
        let v_i = p.vertices[i];
        let v_n = p.vertices[(i + 1) % n];
        if v_i.bulge.abs() < EPS {
            out.push(Geom::Line(Line { a: v_i.pos, b: v_n.pos }));
        } else {
            let chord = v_n.pos - v_i.pos;
            let l = chord.len();
            if l < EPS { continue; }
            let b = v_i.bulge;
            let r = l * (1.0 + b * b) / (4.0 * b.abs());
            let mid = (v_i.pos + v_n.pos) * 0.5;
            let perp = chord.perp() / l;
            let d = r * (1.0 - b * b) / (1.0 + b * b);
            let center = mid + perp * (d * b.signum());
            let start_angle = (v_i.pos - center).angle().rem_euclid(std::f64::consts::TAU);
            let end_angle   = (v_n.pos - center).angle().rem_euclid(std::f64::consts::TAU);
            let raw_sweep = (end_angle - start_angle).rem_euclid(std::f64::consts::TAU);
            let arc = if b > 0.0 {
                Arc { center, radius: r, start_angle, sweep_angle: raw_sweep }
            } else {
                let rev_sweep = std::f64::consts::TAU - raw_sweep;
                Arc { center, radius: r, start_angle: end_angle, sweep_angle: rev_sweep }
            };
            out.push(Geom::Arc(arc));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Ellipse offset helper.
//
// True parallel offset of an ellipse is a quartic, NOT an ellipse, so we
// approximate by sampling the curve and offsetting each sample along its
// local outward normal. Sign chosen from `side` projected onto the normal
// at the first sample.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Polyline offset — segment-offset + corner-intersection (AutoCAD OFFSET).
//
// Each segment is offset to ONE consistent hand (left/right of the directed
// polyline, decided from where the user clicked). Straight segments shift
// along their normal; arc (bulge) segments become CONCENTRIC arcs (radius
// ±dist, same swept angle → same bulge). Adjacent straight offsets are
// joined at their true line-line intersection (miter); joints touching an
// arc fall back to the midpoint of the two offset ends (exact for tangent
// joints, close otherwise). Self-intersection trimming is NOT done (matches
// a plain OFFSET — the user trims afterwards if needed).
// ---------------------------------------------------------------------------

/// Point-to-segment distance — used only to find the nearest segment when
/// resolving which hand the click is on.
fn point_seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let d = b - a;
    let l2 = d.len_sq();
    if l2 < EPS { return p.dist(a); }
    let t = ((p - a).dot(d) / l2).clamp(0.0, 1.0);
    p.dist(a + d * t)
}

/// Intersection of the infinite lines (p0 + t·d0) and (p1 + s·d1). `None`
/// when (near-)parallel.
fn line_line_inf(p0: Vec2, d0: Vec2, p1: Vec2, d1: Vec2) -> Option<Vec2> {
    let denom = d0.x * d1.y - d0.y * d1.x;
    if denom.abs() < 1e-12 { return None; }
    let dp = p1 - p0;
    let t = (dp.x * d1.y - dp.y * d1.x) / denom;
    Some(p0 + d0 * t)
}

fn offset_polyline(p: &Polyline, dist: f64, side: Vec2) -> Result<Geom, &'static str> {
    let v = &p.vertices;
    let n = v.len();
    if n < 2 { return Err("offset: polyline needs ≥ 2 vertices"); }
    let seg_count = if p.closed { n } else { n - 1 };
    let amt = dist.abs();

    // --- 1. global hand from the nearest segment's chord ---
    let mut best = (f64::INFINITY, 0usize);
    for i in 0..seg_count {
        let a = v[i].pos;
        let b = v[(i + 1) % n].pos;
        let d = point_seg_dist(side, a, b);
        if d < best.0 { best = (d, i); }
    }
    let na = v[best.1].pos;
    let nb = v[(best.1 + 1) % n].pos;
    if (nb - na).len() < EPS { return Err("offset: zero-length polyline segment"); }
    let left = (nb - na).perp().normalized();
    let mid  = (na + nb) * 0.5;
    let hand = if (side - mid).dot(left) >= 0.0 { 1.0 } else { -1.0 };
    let off  = amt * hand;   // signed offset toward the clicked hand (+ = left)

    // --- 2. per-segment offset geometry ---
    struct OffSeg { a: Vec2, b: Vec2, bulge: f64, line: bool, dir: Vec2 }
    let mut segs: Vec<OffSeg> = Vec::with_capacity(seg_count);
    for i in 0..seg_count {
        let a = v[i].pos;
        let b = v[(i + 1) % n].pos;
        let bulge = v[i].bulge;
        if bulge.abs() < 1e-9 {
            let dir = b - a;
            if dir.len() < EPS { return Err("offset: zero-length polyline segment"); }
            let shift = dir.perp().normalized() * off;
            segs.push(OffSeg { a: a + shift, b: b + shift, bulge: 0.0,
                               line: true, dir: dir.normalized() });
        } else {
            let Some((c, r, _sa, sweep)) = bulge_arc(a, b, bulge) else {
                return Err("offset: degenerate arc segment");
            };
            let s_arc = if sweep >= 0.0 { 1.0 } else { -1.0 };
            let rp = r - hand * s_arc * amt;   // concentric radius
            if rp <= EPS { return Err("offset: arc segment collapses"); }
            let scale = rp / r;
            segs.push(OffSeg {
                a: c + (a - c) * scale,
                b: c + (b - c) * scale,
                bulge,                 // same swept angle → same bulge
                line: false, dir: Vec2::ZERO,
            });
        }
    }

    // --- 3. join into a vertex list ---
    let join = |s0: &OffSeg, s1: &OffSeg| -> Vec2 {
        if s0.line && s1.line {
            if let Some(p) = line_line_inf(s0.a, s0.dir, s1.a, s1.dir) { return p; }
        }
        (s0.b + s1.a) * 0.5
    };
    let mut out: Vec<PolyVertex> = Vec::with_capacity(n);
    if p.closed {
        for i in 0..seg_count {
            let prev = (i + seg_count - 1) % seg_count;
            out.push(PolyVertex { pos: join(&segs[prev], &segs[i]), bulge: segs[i].bulge });
        }
    } else {
        out.push(PolyVertex { pos: segs[0].a, bulge: segs[0].bulge });
        for i in 1..seg_count {
            out.push(PolyVertex { pos: join(&segs[i - 1], &segs[i]), bulge: segs[i].bulge });
        }
        out.push(PolyVertex { pos: segs[seg_count - 1].b, bulge: 0.0 });
    }
    Ok(Geom::Polyline(Polyline { vertices: out, closed: p.closed }))
}

fn offset_ellipse_samples(
    el: Ellipse,
    t_start: f64,
    t_end: f64,
    dist: f64,
    side: Vec2,
    closed: bool,
) -> Vec<Vec2> {
    let a = el.semi_major();
    if a < EPS { return Vec::new(); }
    // Sample density scales with size; minimum 64 for visual smoothness.
    let n = (64.0_f64 + a.log10().max(0.0) * 32.0).round().max(48.0) as usize;
    // Sign: compare `side` direction to the CCW-perp tangent at t_start.
    let p0 = el.point_at(t_start);
    let tg0 = el.tangent_at(t_start);
    let nrm0 = tg0.perp();
    let nl   = nrm0.len();
    if nl < EPS { return Vec::new(); }
    let n0u  = nrm0 / nl;
    let sgn  = if (side - p0).dot(n0u) >= 0.0 { 1.0 } else { -1.0 };
    let span = t_end - t_start;
    let count = if closed { n } else { n + 1 };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let t = t_start + (i as f64 / n as f64) * span;
        let p = el.point_at(t);
        let tg = el.tangent_at(t);
        let nm = tg.perp();
        let nl = nm.len();
        if nl < EPS { continue; }
        out.push(p + (nm / nl) * (dist.abs() * sgn));
    }
    out
}

// ---------------------------------------------------------------------------
// Fillet / Chamfer — Slices M.3 + M.4.
//
// v1 supports LINE + LINE only. Other combinations return Err; line-arc and
// arc-arc are deferred to v2 per the user's scoping decision.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct FilletOut {
    /// L1 trimmed back to the tangent point on its kept side.
    pub g1_new: Geom,
    /// L2 trimmed back to the tangent point on its kept side.
    pub g2_new: Geom,
    /// The fillet arc itself. `None` for radius 0 — the two lines just meet
    /// at the corner intersection.
    pub arc: Option<Geom>,
}

/// Fillet two lines with `radius`. `p1` / `p2` are the user's click points
/// on each line — they determine which SIDE of each line is kept (the side
/// nearest the click). Radius 0 produces a sharp corner.
pub fn fillet_lines(
    l1: &Line, p1: Vec2,
    l2: &Line, p2: Vec2,
    radius: f64,
) -> Result<FilletOut, &'static str> {
    if radius < 0.0 { return Err("fillet: radius must be ≥ 0"); }
    let d1v = l1.b - l1.a;
    let d2v = l2.b - l2.a;
    let len1 = d1v.len();
    let len2 = d2v.len();
    if len1 < EPS || len2 < EPS { return Err("fillet: zero-length line"); }

    // Infinite-line intersection.
    let cross = d1v.x * d2v.y - d1v.y * d2v.x;
    if cross.abs() < EPS { return Err("fillet: lines are parallel"); }
    let dx = l2.a.x - l1.a.x;
    let dy = l2.a.y - l1.a.y;
    let t1 = (dx * d2v.y - dy * d2v.x) / cross;
    let i_pt = l1.a + d1v * t1;

    // Unit directions from I toward each pick.
    let u1 = d1v / len1;
    let u2 = d2v / len2;
    let dir1 = if (p1 - i_pt).dot(u1) >= 0.0 { u1 } else { -u1 };
    let dir2 = if (p2 - i_pt).dot(u2) >= 0.0 { u2 } else { -u2 };

    // Corner angle at I (between the two kept-side outgoing rays).
    let cos_th = dir1.dot(dir2).clamp(-1.0, 1.0);
    let theta = cos_th.acos();
    if theta < 1e-6 || (std::f64::consts::PI - theta) < 1e-6 {
        return Err("fillet: lines are collinear at the corner");
    }

    // Endpoint of each line to KEEP — the one on the same side of I as the
    // click. We compare projections onto dirN; whichever endpoint projects
    // further along dirN from I is the kept endpoint.
    let endpoint_of_l1 = if (l1.a - i_pt).dot(dir1) > (l1.b - i_pt).dot(dir1) { l1.a } else { l1.b };
    let endpoint_of_l2 = if (l2.a - i_pt).dot(dir2) > (l2.b - i_pt).dot(dir2) { l2.a } else { l2.b };

    if radius < EPS {
        // Sharp corner: both lines run from their kept endpoint to I.
        return Ok(FilletOut {
            g1_new: Geom::Line(Line { a: endpoint_of_l1, b: i_pt }),
            g2_new: Geom::Line(Line { a: i_pt, b: endpoint_of_l2 }),
            arc: None,
        });
    }

    // Tangent-point distance along each kept-side ray: t = r / tan(θ/2).
    let tan_half = (theta / 2.0).tan();
    let t = radius / tan_half;
    // The kept segment must be long enough to reach the tangent point.
    let kept_len1 = (endpoint_of_l1 - i_pt).len();
    let kept_len2 = (endpoint_of_l2 - i_pt).len();
    if kept_len1 < t || kept_len2 < t {
        return Err("fillet: radius too large for these lines");
    }

    let tp1 = i_pt + dir1 * t;
    let tp2 = i_pt + dir2 * t;

    // Arc center: bisector direction from I, distance r/sin(θ/2).
    let bis_raw = dir1 + dir2;
    let bis = bis_raw / bis_raw.len();
    let center = i_pt + bis * (radius / (theta / 2.0).sin());

    // The fillet arc is the MINOR arc between the two tangent points
    // (central angle π − θ). Because I lies OUTSIDE the circle (its distance
    // r/sin(θ/2) > r), the minor arc is the one whose chord faces I — i.e.
    // it always bulges toward the corner vertex, giving the rounded inside
    // corner. We render arcs CCW (positive sweep), so the only decision is
    // which tangent point is the START such that a CCW sweep of π − θ stays
    // on that minor arc:
    //   d_ccw = CCW angle from tp1 to tp2. If d_ccw ≤ π the minor arc runs
    //   CCW from tp1; otherwise it runs CCW from tp2 (CW from tp1), so start
    //   there. sweep is always the minor magnitude π − θ.
    //
    // (The previous heuristic rotated v1 toward the I-direction and accepted
    // on `dot > 0` — a 90°-wide window that mis-fired for non-right corners,
    // e.g. θ = 120°, rendering the arc sweeping the wrong way out of the
    // corner. It only happened to be correct at θ = 90°.)
    let arc_angle = std::f64::consts::PI - theta;
    let v1 = tp1 - center;
    let v2 = tp2 - center;
    let a1 = v1.angle().rem_euclid(std::f64::consts::TAU);
    let a2 = v2.angle().rem_euclid(std::f64::consts::TAU);
    let d_ccw = (a2 - a1).rem_euclid(std::f64::consts::TAU);
    let start_angle = if d_ccw <= std::f64::consts::PI { a1 } else { a2 };
    let arc = Geom::Arc(Arc {
        center,
        radius,
        start_angle,
        sweep_angle: arc_angle,
    });

    Ok(FilletOut {
        g1_new: Geom::Line(Line { a: endpoint_of_l1, b: tp1 }),
        g2_new: Geom::Line(Line { a: tp2, b: endpoint_of_l2 }),
        arc: Some(arc),
    })
}

#[derive(Clone, Debug)]
pub struct ChamferOut {
    pub g1_new: Geom,
    pub g2_new: Geom,
    /// The chamfer line itself, connecting the two tangent points.
    pub bridge: Geom,
}

/// Chamfer two lines with distances `d1` along L1 and `d2` along L2 from the
/// intersection. Same click-side convention as fillet.
pub fn chamfer_lines(
    l1: &Line, p1: Vec2,
    l2: &Line, p2: Vec2,
    d1: f64, d2: f64,
) -> Result<ChamferOut, &'static str> {
    if d1 < 0.0 || d2 < 0.0 { return Err("chamfer: distances must be ≥ 0"); }
    let d1v = l1.b - l1.a;
    let d2v = l2.b - l2.a;
    let len1 = d1v.len();
    let len2 = d2v.len();
    if len1 < EPS || len2 < EPS { return Err("chamfer: zero-length line"); }

    let cross = d1v.x * d2v.y - d1v.y * d2v.x;
    if cross.abs() < EPS { return Err("chamfer: lines are parallel"); }
    let dx = l2.a.x - l1.a.x;
    let dy = l2.a.y - l1.a.y;
    let t1 = (dx * d2v.y - dy * d2v.x) / cross;
    let i_pt = l1.a + d1v * t1;

    let u1 = d1v / len1;
    let u2 = d2v / len2;
    let dir1 = if (p1 - i_pt).dot(u1) >= 0.0 { u1 } else { -u1 };
    let dir2 = if (p2 - i_pt).dot(u2) >= 0.0 { u2 } else { -u2 };

    let endpoint_of_l1 = if (l1.a - i_pt).dot(dir1) > (l1.b - i_pt).dot(dir1) { l1.a } else { l1.b };
    let endpoint_of_l2 = if (l2.a - i_pt).dot(dir2) > (l2.b - i_pt).dot(dir2) { l2.a } else { l2.b };

    let kept_len1 = (endpoint_of_l1 - i_pt).len();
    let kept_len2 = (endpoint_of_l2 - i_pt).len();
    if kept_len1 < d1 || kept_len2 < d2 {
        return Err("chamfer: distance exceeds available line length");
    }

    let tp1 = i_pt + dir1 * d1;
    let tp2 = i_pt + dir2 * d2;
    Ok(ChamferOut {
        g1_new: Geom::Line(Line { a: endpoint_of_l1, b: tp1 }),
        g2_new: Geom::Line(Line { a: tp2, b: endpoint_of_l2 }),
        bridge: Geom::Line(Line { a: tp1, b: tp2 }),
    })
}

// ---------------------------------------------------------------------------
// Join — Slice M.5.
//
// Three merge classes, applied in order:
//   1. Collinear Lines  → one Line covering the union extent.
//   2. Concentric Arcs of equal radius with touching sweeps → one Arc.
//   3. Any chain of touching segments (Lines + Arcs, end-to-end) → Polyline.
//      Open chain → open polyline; closed chain → closed polyline.
// Unjoinable inputs come back unchanged in `unmodified`.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct JoinOut {
    /// The merged Geoms produced this run. Each replaces one or more inputs.
    pub merged: Vec<Geom>,
    /// Indices INTO the input slice that participated in some merge.
    /// Callers remove these from the doc before appending `merged`.
    pub consumed_indices: Vec<usize>,
}

/// Try to merge the given geoms (referenced by `indices` into the doc) into
/// one or more bigger geoms. Each input index is paired with its `Geom`.
pub fn join_geoms(geoms: &[(usize, Geom)]) -> JoinOut {
    let mut merged       = Vec::new();
    let mut consumed     = Vec::new();
    let mut available: Vec<(usize, Geom)> = geoms.iter().cloned().collect();

    // -- pass 1: collinear lines ------------------------------------------------
    loop {
        let group = find_collinear_line_group(&available);
        if group.len() < 2 { break; }
        let line_geoms: Vec<Line> = group.iter()
            .filter_map(|&i| match &available[i].1 { Geom::Line(l) => Some(*l), _ => None })
            .collect();
        if let Some(merged_line) = merge_collinear_lines(&line_geoms) {
            for &local_i in &group { consumed.push(available[local_i].0); }
            // remove from `available` in descending index order
            let mut sorted = group.clone();
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            for li in sorted { available.remove(li); }
            merged.push(Geom::Line(merged_line));
        } else { break; }
    }

    // -- pass 2: concentric arcs (same center, same radius, touching sweeps) ----
    loop {
        let group = find_concentric_arc_group(&available);
        if group.len() < 2 { break; }
        let arcs: Vec<Arc> = group.iter()
            .filter_map(|&i| match &available[i].1 { Geom::Arc(a) => Some(*a), _ => None })
            .collect();
        if let Some(merged_arc) = merge_concentric_arcs(&arcs) {
            for &local_i in &group { consumed.push(available[local_i].0); }
            let mut sorted = group.clone();
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            for li in sorted { available.remove(li); }
            merged.push(Geom::Arc(merged_arc));
        } else { break; }
    }

    // -- pass 3: chain of touching Lines + Arcs → Polyline ---------------------
    loop {
        let chain = find_touching_chain(&available);
        if chain.len() < 2 { break; }
        if let Some(pl) = chain_to_polyline(&chain.iter().map(|&i| available[i].1.clone()).collect::<Vec<_>>()) {
            for &local_i in &chain { consumed.push(available[local_i].0); }
            let mut sorted = chain.clone();
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            for li in sorted { available.remove(li); }
            merged.push(Geom::Polyline(pl));
        } else { break; }
    }

    JoinOut { merged, consumed_indices: consumed }
}

const JOIN_EPS: f64 = 1e-6;

fn find_collinear_line_group(items: &[(usize, Geom)]) -> Vec<usize> {
    // Returns local indices of the first run of >= 2 collinear Lines we find.
    for i in 0..items.len() {
        let li = if let Geom::Line(l) = &items[i].1 { *l } else { continue };
        let dir_i = li.b - li.a;
        let len_i = dir_i.len();
        if len_i < JOIN_EPS { continue; }
        let u_i = dir_i / len_i;
        let mut group = vec![i];
        for j in (i + 1)..items.len() {
            let lj = if let Geom::Line(l) = &items[j].1 { *l } else { continue };
            let dir_j = lj.b - lj.a;
            let len_j = dir_j.len();
            if len_j < JOIN_EPS { continue; }
            // Same infinite line: parallel + lj.a lies on li's infinite line.
            let cross = u_i.x * dir_j.y - u_i.y * dir_j.x;
            if cross.abs() > JOIN_EPS * len_j { continue; }
            // Perp distance from lj.a to li's infinite line.
            let perp = u_i.perp();
            if (lj.a - li.a).dot(perp).abs() > JOIN_EPS { continue; }
            group.push(j);
        }
        if group.len() >= 2 { return group; }
    }
    Vec::new()
}

fn merge_collinear_lines(lines: &[Line]) -> Option<Line> {
    if lines.is_empty() { return None; }
    let l0 = lines[0];
    let u = (l0.b - l0.a).normalized();
    // Project every endpoint onto u; take min/max.
    let project = |p: Vec2| (p - l0.a).dot(u);
    let mut t_min = f64::INFINITY;
    let mut t_max = f64::NEG_INFINITY;
    for l in lines {
        for p in [l.a, l.b] {
            let t = project(p);
            if t < t_min { t_min = t; }
            if t > t_max { t_max = t; }
        }
    }
    Some(Line { a: l0.a + u * t_min, b: l0.a + u * t_max })
}

fn find_concentric_arc_group(items: &[(usize, Geom)]) -> Vec<usize> {
    for i in 0..items.len() {
        let ai = if let Geom::Arc(a) = &items[i].1 { *a } else { continue };
        let mut group = vec![i];
        for j in (i + 1)..items.len() {
            let aj = if let Geom::Arc(a) = &items[j].1 { *a } else { continue };
            if (aj.center - ai.center).len() > JOIN_EPS { continue; }
            if (aj.radius - ai.radius).abs() > JOIN_EPS { continue; }
            group.push(j);
        }
        // Only return the group if at least two of them have sweeps that
        // CONNECT (one's end is another's start, within EPS).
        if group.len() >= 2 && arcs_form_a_chain(&group.iter()
            .filter_map(|&k| if let Geom::Arc(a) = &items[k].1 { Some(*a) } else { None })
            .collect::<Vec<_>>())
        {
            return group;
        }
    }
    Vec::new()
}

fn arcs_form_a_chain(arcs: &[Arc]) -> bool {
    // True iff the union of sweep intervals (mod 2π) forms a single
    // contiguous range (or full circle).
    if arcs.len() < 2 { return false; }
    let mut spans: Vec<(f64, f64)> = arcs.iter()
        .map(|a| {
            let s = a.start_angle.rem_euclid(std::f64::consts::TAU);
            (s, s + a.sweep_angle)
        })
        .collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut hi = spans[0].1;
    for i in 1..spans.len() {
        if spans[i].0 > hi + JOIN_EPS { return false; }
        if spans[i].1 > hi { hi = spans[i].1; }
    }
    true
}

fn merge_concentric_arcs(arcs: &[Arc]) -> Option<Arc> {
    if arcs.is_empty() { return None; }
    let mut spans: Vec<(f64, f64)> = arcs.iter()
        .map(|a| {
            let s = a.start_angle.rem_euclid(std::f64::consts::TAU);
            (s, s + a.sweep_angle)
        })
        .collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let start = spans[0].0;
    let end   = spans.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
    let sweep = (end - start).min(std::f64::consts::TAU);
    Some(Arc {
        center: arcs[0].center,
        radius: arcs[0].radius,
        start_angle: start,
        sweep_angle: sweep,
    })
}

fn endpoints_of(g: &Geom) -> Option<(Vec2, Vec2)> {
    match g {
        Geom::Line(l)       => Some((l.a, l.b)),
        Geom::Arc(a)        => Some(a.endpoints()),
        Geom::EllipseArc(e) => Some(e.endpoints()),
        Geom::Polyline(p) if !p.closed && !p.vertices.is_empty() =>
            Some((p.vertices.first()?.pos, p.vertices.last()?.pos)),
        _ => None,
    }
}

fn find_touching_chain(items: &[(usize, Geom)]) -> Vec<usize> {
    // BFS from each item; connect via shared endpoints (within EPS).
    if items.is_empty() { return Vec::new(); }
    for start in 0..items.len() {
        if endpoints_of(&items[start].1).is_none() { continue; }
        let mut seen = vec![false; items.len()];
        seen[start] = true;
        let mut queue = vec![start];
        let mut group = vec![start];
        while let Some(cur) = queue.pop() {
            let Some((ca, cb)) = endpoints_of(&items[cur].1) else { continue };
            for j in 0..items.len() {
                if seen[j] { continue; }
                let Some((ja, jb)) = endpoints_of(&items[j].1) else { continue };
                let touches = ca.dist(ja) < JOIN_EPS || ca.dist(jb) < JOIN_EPS
                           || cb.dist(ja) < JOIN_EPS || cb.dist(jb) < JOIN_EPS;
                if touches {
                    seen[j] = true;
                    queue.push(j);
                    group.push(j);
                }
            }
        }
        if group.len() >= 2 { return group; }
    }
    Vec::new()
}

fn chain_to_polyline(geoms: &[Geom]) -> Option<Polyline> {
    // Walk the chain endpoint-to-endpoint, emitting one PolyVertex per
    // vertex along the way. Arcs contribute a `bulge` to the vertex BEFORE
    // them (DXF convention: bulge belongs to the segment FROM that vertex).
    if geoms.len() < 2 { return None; }
    // Build an unordered set, then traverse.
    let mut remaining: Vec<Geom> = geoms.to_vec();
    // Pick a starting endpoint: any vertex that's only touched ONCE among
    // the unordered set is an open-chain end. If every vertex is touched
    // twice, the chain is closed.
    let mut endpoint_count: Vec<(Vec2, usize)> = Vec::new();
    let bump = |list: &mut Vec<(Vec2, usize)>, p: Vec2| {
        for (q, c) in list.iter_mut() {
            if q.dist(p) < JOIN_EPS { *c += 1; return; }
        }
        list.push((p, 1));
    };
    for g in &remaining {
        let Some((a, b)) = endpoints_of(g) else { return None; };
        bump(&mut endpoint_count, a);
        bump(&mut endpoint_count, b);
    }
    let chain_closed = endpoint_count.iter().all(|&(_, c)| c == 2);
    let start_pt: Vec2 = if chain_closed {
        endpoint_count[0].0
    } else {
        endpoint_count.iter().find(|&&(_, c)| c == 1).map(|&(p, _)| p)?
    };

    let mut current = start_pt;
    let mut verts: Vec<PolyVertex> = vec![PolyVertex { pos: current, bulge: 0.0 }];
    while !remaining.is_empty() {
        // Find a segment that touches `current`.
        let mut found: Option<usize> = None;
        let mut reverse = false;
        for (i, g) in remaining.iter().enumerate() {
            let Some((a, b)) = endpoints_of(g) else { continue };
            if a.dist(current) < JOIN_EPS { found = Some(i); reverse = false; break; }
            if b.dist(current) < JOIN_EPS { found = Some(i); reverse = true;  break; }
        }
        let i = found?;
        let seg = remaining.remove(i);
        let oriented = if reverse { seg.reversed() } else { seg };
        let (_, next) = endpoints_of(&oriented)?;
        // Compute bulge for the segment OUT of `current`. DXF bulge =
        // tan(included_angle / 4) with sign by CCW (positive) / CW (negative).
        let bulge_for_last = match &oriented {
            Geom::Line(_) => 0.0,
            Geom::Arc(a) => {
                // included angle from start_angle to end of sweep is sweep_angle.
                // CCW arcs in our struct → positive bulge.
                (a.sweep_angle / 4.0).tan()
            }
            _ => 0.0,
        };
        if let Some(last) = verts.last_mut() { last.bulge = bulge_for_last; }
        if remaining.is_empty() && chain_closed {
            // Don't push the closing vertex — `closed: true` carries it.
            break;
        }
        verts.push(PolyVertex { pos: next, bulge: 0.0 });
        current = next;
    }

    Some(Polyline { vertices: verts, closed: chain_closed })
}

#[cfg(test)]
mod fillet_chamfer_join_tests {
    use super::*;
    use crate::math::approx_eq;

    fn ln(ax: f64, ay: f64, bx: f64, by: f64) -> Line {
        Line { a: Vec2::new(ax, ay), b: Vec2::new(bx, by) }
    }

    // --- fillet -----------------------------------------------------------

    #[test]
    fn fillet_right_angle_radius_1() {
        let l1 = ln(0.0, 0.0, 5.0, 0.0);  // along +X
        let l2 = ln(0.0, 0.0, 0.0, 5.0);  // along +Y
        let p1 = Vec2::new(3.0, 0.0);
        let p2 = Vec2::new(0.0, 3.0);
        let out = fillet_lines(&l1, p1, &l2, p2, 1.0).unwrap();
        if let Geom::Line(l) = out.g1_new {
            assert!(approx_eq(l.a.x, 5.0));
            assert!(approx_eq(l.b.x, 1.0));
            assert!(approx_eq(l.b.y, 0.0));
        } else { panic!("g1_new not a Line") }
        if let Geom::Line(l) = out.g2_new {
            assert!(approx_eq(l.b.y, 5.0));
            assert!(approx_eq(l.a.x, 0.0));
            assert!(approx_eq(l.a.y, 1.0));
        } else { panic!("g2_new not a Line") }
        let arc = out.arc.expect("expected an arc for r>0");
        if let Geom::Arc(a) = arc {
            assert!(approx_eq(a.radius, 1.0));
            assert!(approx_eq(a.center.x, 1.0));
            assert!(approx_eq(a.center.y, 1.0));
            assert!(approx_eq(a.sweep_angle, std::f64::consts::FRAC_PI_2));
        } else { panic!("arc not an Arc") }
    }

    #[test]
    fn fillet_obtuse_120deg_arc_bulges_toward_corner() {
        // L1 along +X, L2 along 120° — an obtuse (θ = 120°) corner at the
        // origin. Regression for the arc-DIRECTION bug: the rounded corner
        // must bulge TOWARD the corner vertex I, not sweep out the far side.
        // The old `dot > 0` start-angle heuristic mis-fired here and rendered
        // the arc on the wrong side (only θ = 90° happened to be correct).
        use std::f64::consts::PI;
        let theta = 2.0 * PI / 3.0;                       // 120°
        let dir2  = Vec2::new(theta.cos(), theta.sin());
        let l1 = ln(0.0, 0.0, 10.0, 0.0);
        let l2 = Line { a: Vec2::new(0.0, 0.0), b: dir2 * 10.0 };
        let p1 = Vec2::new(8.0, 0.0);
        let p2 = dir2 * 8.0;
        let r  = 2.0;
        let out = fillet_lines(&l1, p1, &l2, p2, r).unwrap();

        // Expected geometry from first principles.
        let t      = r / (theta / 2.0).tan();
        let tp1    = Vec2::new(t, 0.0);
        let tp2    = dir2 * t;
        let bis    = Vec2::new(1.0, 0.0) + dir2;
        let center = bis / bis.len() * (r / (theta / 2.0).sin());
        let i_pt   = Vec2::new(0.0, 0.0);
        let mid_exp = center + (i_pt - center) / (i_pt - center).len() * r;

        let Geom::Arc(a) = out.arc.expect("expected an arc") else { panic!("not an arc") };
        let pt = |ang: f64| Vec2::new(
            a.center.x + a.radius * ang.cos(),
            a.center.y + a.radius * ang.sin());
        let start = pt(a.start_angle);
        let end   = pt(a.start_angle + a.sweep_angle);
        let mid   = pt(a.start_angle + a.sweep_angle * 0.5);
        let close = |p: Vec2, q: Vec2| approx_eq(p.x, q.x) && approx_eq(p.y, q.y);

        assert!(approx_eq(a.sweep_angle, PI - theta), "sweep should be π−θ (60°)");
        assert!(close(a.center, center), "center {:?} != {:?}", a.center, center);
        // Endpoints land on the two tangent points (either traversal order).
        assert!(
            (close(start, tp1) && close(end, tp2)) ||
            (close(start, tp2) && close(end, tp1)),
            "arc endpoints {:?},{:?} should be tangent pts {:?},{:?}", start, end, tp1, tp2);
        // Midpoint bulges toward the corner vertex I (the bug put it on the
        // opposite side of the circle).
        assert!(close(mid, mid_exp), "arc mid {:?} should bulge toward I ({:?})", mid, mid_exp);
    }

    #[test]
    fn fillet_radius_zero_makes_sharp_corner() {
        let l1 = ln(0.0, 0.0, 5.0, 0.0);
        let l2 = ln(2.0, -3.0, 2.0, 4.0);    // intersects l1 at (2, 0)
        let p1 = Vec2::new(4.5, 0.0);
        let p2 = Vec2::new(2.0, 3.0);
        let out = fillet_lines(&l1, p1, &l2, p2, 0.0).unwrap();
        assert!(out.arc.is_none());
        if let Geom::Line(l) = out.g1_new {
            assert!(approx_eq(l.b.x, 2.0));
            assert!(approx_eq(l.b.y, 0.0));
        } else { panic!() }
    }

    #[test]
    fn fillet_parallel_lines_errs() {
        let l1 = ln(0.0, 0.0, 5.0, 0.0);
        let l2 = ln(0.0, 1.0, 5.0, 1.0);
        let p1 = Vec2::new(2.0, 0.0);
        let p2 = Vec2::new(2.0, 1.0);
        assert!(fillet_lines(&l1, p1, &l2, p2, 1.0).is_err());
    }

    #[test]
    fn fillet_radius_too_large_errs() {
        let l1 = ln(0.0, 0.0, 1.0, 0.0);  // 1-long
        let l2 = ln(0.0, 0.0, 0.0, 1.0);
        let p1 = Vec2::new(0.5, 0.0);
        let p2 = Vec2::new(0.0, 0.5);
        // radius 10 needs tangent point at (10, 0) — way past line end.
        assert!(fillet_lines(&l1, p1, &l2, p2, 10.0).is_err());
    }

    // --- chamfer ----------------------------------------------------------

    #[test]
    fn chamfer_right_angle_d1_d2() {
        let l1 = ln(0.0, 0.0, 5.0, 0.0);
        let l2 = ln(0.0, 0.0, 0.0, 5.0);
        let p1 = Vec2::new(3.0, 0.0);
        let p2 = Vec2::new(0.0, 3.0);
        let out = chamfer_lines(&l1, p1, &l2, p2, 1.0, 2.0).unwrap();
        if let Geom::Line(l) = out.g1_new {
            assert!(approx_eq(l.b.x, 1.0));
            assert!(approx_eq(l.b.y, 0.0));
        } else { panic!() }
        if let Geom::Line(l) = out.g2_new {
            assert!(approx_eq(l.a.x, 0.0));
            assert!(approx_eq(l.a.y, 2.0));
        } else { panic!() }
        if let Geom::Line(l) = out.bridge {
            assert!(approx_eq(l.a.x, 1.0)); assert!(approx_eq(l.a.y, 0.0));
            assert!(approx_eq(l.b.x, 0.0)); assert!(approx_eq(l.b.y, 2.0));
        } else { panic!() }
    }

    // --- join: collinear lines -------------------------------------------

    #[test]
    fn join_two_touching_collinear_lines() {
        let items = vec![
            (3usize, Geom::Line(ln(0.0, 0.0, 2.0, 0.0))),
            (7usize, Geom::Line(ln(2.0, 0.0, 5.0, 0.0))),
        ];
        let out = join_geoms(&items);
        assert_eq!(out.consumed_indices.len(), 2);
        assert_eq!(out.merged.len(), 1);
        if let Geom::Line(l) = &out.merged[0] {
            assert!(approx_eq(l.a.x, 0.0));
            assert!(approx_eq(l.b.x, 5.0));
        } else { panic!() }
    }

    #[test]
    fn join_overlapping_collinear_lines() {
        let items = vec![
            (0usize, Geom::Line(ln(0.0, 0.0, 3.0, 0.0))),
            (1usize, Geom::Line(ln(2.0, 0.0, 5.0, 0.0))),
        ];
        let out = join_geoms(&items);
        assert_eq!(out.merged.len(), 1);
        if let Geom::Line(l) = &out.merged[0] {
            assert!(approx_eq(l.a.x, 0.0));
            assert!(approx_eq(l.b.x, 5.0));
        } else { panic!() }
    }

    // --- join: concentric arcs -------------------------------------------

    #[test]
    fn join_two_touching_arcs_same_center_radius() {
        let a1 = Arc { center: Vec2::ZERO, radius: 1.0,
                       start_angle: 0.0, sweep_angle: std::f64::consts::FRAC_PI_2 };
        let a2 = Arc { center: Vec2::ZERO, radius: 1.0,
                       start_angle: std::f64::consts::FRAC_PI_2,
                       sweep_angle: std::f64::consts::FRAC_PI_2 };
        let items = vec![(0usize, Geom::Arc(a1)), (1usize, Geom::Arc(a2))];
        let out = join_geoms(&items);
        assert_eq!(out.merged.len(), 1);
        if let Geom::Arc(a) = &out.merged[0] {
            assert!(approx_eq(a.sweep_angle, std::f64::consts::PI));
            assert!(approx_eq(a.radius, 1.0));
        } else { panic!() }
    }

    // --- join: chain → polyline ------------------------------------------

    #[test]
    fn join_line_arc_line_chain_to_polyline() {
        let items = vec![
            (0, Geom::Line(ln(0.0, 0.0, 1.0, 0.0))),
            (1, Geom::Arc(Arc {
                center: Vec2::new(1.0, 1.0), radius: 1.0,
                start_angle: -std::f64::consts::FRAC_PI_2,
                sweep_angle: std::f64::consts::FRAC_PI_2,
            })),
            (2, Geom::Line(ln(2.0, 1.0, 2.0, 3.0))),
        ];
        let out = join_geoms(&items);
        // The collinear / arc-group passes shouldn't fire (only 1 line
        // direction; 1 arc). Chain pass should produce a polyline.
        assert!(out.merged.iter().any(|g| matches!(g, Geom::Polyline(_))));
    }

    // --- trim: closed Ellipse -------------------------------------------

    #[test]
    fn trim_ellipse_horizontal_cut_keeps_lower_half() {
        // a = 2, b = 1; cut by y = 0 → intersections at (±2, 0) (t = 0, π).
        // Click on upper half → upper EllipseArc dropped; lower survives.
        let el = Ellipse {
            center: Vec2::ZERO,
            major:  Vec2::new(2.0, 0.0),
            ratio:  0.5,
        };
        let g      = Geom::Ellipse(el);
        let cutter = Geom::Line(Line {
            a: Vec2::new(-3.0, 0.0),
            b: Vec2::new( 3.0, 0.0),
        });
        let pieces = g.trim_at(&[cutter], Vec2::new(0.0, 0.5), false).unwrap();
        assert_eq!(pieces.len(), 1, "expected one surviving EllipseArc");
        if let Geom::EllipseArc(ea) = &pieces[0] {
            assert!((ea.start_param - std::f64::consts::PI).abs() < 1e-4);
            assert!((ea.sweep_param - std::f64::consts::PI).abs() < 1e-4);
        } else {
            panic!("expected EllipseArc, got {:?}", pieces[0]);
        }
    }

    #[test]
    fn trim_ellipse_three_cuts_drops_clicked_arc() {
        // Three cuts → 3 arcs. Click inside one → 2 survive.
        let el = Ellipse {
            center: Vec2::ZERO,
            major:  Vec2::new(3.0, 0.0),
            ratio:  0.5,
        };
        let g = Geom::Ellipse(el);
        // Three horizontal/diagonal lines through the ellipse, picked so
        // each crosses the curve twice; total 6 intersections, dedup'd by
        // sort+dedup gives 6 distinct params → 6 arcs of small sweep,
        // not 3. Adjust: pick lines that share a common pair of points.
        // Easier: cut twice by parallel lines y=±0.4 — 4 intersections,
        // four sub-arcs.
        let c1 = Geom::Line(Line { a: Vec2::new(-4.0,  0.4), b: Vec2::new(4.0,  0.4) });
        let c2 = Geom::Line(Line { a: Vec2::new(-4.0, -0.4), b: Vec2::new(4.0, -0.4) });
        let pieces = g.trim_at(&[c1, c2], Vec2::new(0.0, 1.0), false).unwrap();
        // Click is on the top arc (between y=0.4 cuts at the top); 3 survive.
        assert_eq!(pieces.len(), 3);
        for p in &pieces {
            assert!(matches!(p, Geom::EllipseArc(_)));
        }
    }

    // --- offset: ellipse / EllipseArc → Polyline approximation --------

    // --- trim: Polyline (explode + trim) ---------------------------------

    // --- grips: per-role semantics ---------------------------------------

    #[test]
    fn grip_line_endpoint_moves_only_that_end() {
        let g = Geom::Line(ln(0.0, 0.0, 10.0, 0.0));
        let out = g.with_grip_moved(GripRole::LineEndA, Vec2::new(2.0, 5.0));
        if let Geom::Line(l) = out {
            assert!(approx_eq(l.a.x, 2.0)); assert!(approx_eq(l.a.y, 5.0));
            assert!(approx_eq(l.b.x, 10.0)); assert!(approx_eq(l.b.y, 0.0));
        } else { panic!(); }
    }

    #[test]
    fn grip_line_midpoint_translates_whole_line() {
        let g = Geom::Line(ln(0.0, 0.0, 10.0, 0.0));
        // Original midpoint = (5,0). Move to (5,3) → delta (0,3).
        let out = g.with_grip_moved(GripRole::LineMid, Vec2::new(5.0, 3.0));
        if let Geom::Line(l) = out {
            assert!(approx_eq(l.a.y, 3.0));
            assert!(approx_eq(l.b.y, 3.0));
            assert!(approx_eq(l.a.x, 0.0));
            assert!(approx_eq(l.b.x, 10.0));
        } else { panic!(); }
    }

    #[test]
    fn grip_circle_quadrant_changes_radius() {
        let g = Geom::Circle(Circle { center: Vec2::ZERO, radius: 1.0 });
        // Drag a quadrant to (3, 4) → new radius = 5.
        let out = g.with_grip_moved(GripRole::CircleQuadrant, Vec2::new(3.0, 4.0));
        if let Geom::Circle(c) = out {
            assert!(approx_eq(c.center.x, 0.0));
            assert!(approx_eq(c.center.y, 0.0));
            assert!(approx_eq(c.radius, 5.0));
        } else { panic!(); }
    }

    #[test]
    fn grip_circle_center_translates() {
        let g = Geom::Circle(Circle { center: Vec2::ZERO, radius: 2.0 });
        let out = g.with_grip_moved(GripRole::CircleCenter, Vec2::new(7.0, 8.0));
        if let Geom::Circle(c) = out {
            assert!(approx_eq(c.center.x, 7.0));
            assert!(approx_eq(c.center.y, 8.0));
            assert!(approx_eq(c.radius, 2.0));
        } else { panic!(); }
    }

    #[test]
    fn grip_polyline_vertex_moves_only_that_vertex() {
        let pl = Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(4.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(4.0, 4.0), bulge: 0.0 },
            ],
            closed: false,
        };
        let g = Geom::Polyline(pl);
        let out = g.with_grip_moved(GripRole::PolyVertex(1), Vec2::new(4.0, -2.0));
        if let Geom::Polyline(p) = out {
            assert!(approx_eq(p.vertices[0].pos.y, 0.0));
            assert!(approx_eq(p.vertices[1].pos.y, -2.0));
            assert!(approx_eq(p.vertices[2].pos.y, 4.0));
        } else { panic!(); }
    }

    #[test]
    fn trim_polyline_segment_drops_clicked_sub_segment() {
        // Open polyline: (0,0) → (4,0) → (4,4). A vertical line at x=2
        // crosses segment 0 (horizontal) at (2,0). Click between the cut
        // and (4,0) → that sub-segment is dropped. Other segments survive.
        let pl = Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(4.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(4.0, 4.0), bulge: 0.0 },
            ],
            closed: false,
        };
        let g = Geom::Polyline(pl);
        let cutter = Geom::Line(Line {
            a: Vec2::new(2.0, -1.0), b: Vec2::new(2.0, 5.0),
        });
        let pieces = g.trim_at(&[cutter], Vec2::new(3.0, 0.0), false).unwrap();
        // Expected: (0,0)→(2,0) plus the vertical (4,0)→(4,4). Two Lines.
        assert_eq!(pieces.len(), 2);
        for p in &pieces { assert!(matches!(p, Geom::Line(_))); }
    }

    #[test]
    fn intersect_line_polyline_returns_per_segment_hits() {
        use crate::intersect::intersect;
        let pl = Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(4.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(4.0, 4.0), bulge: 0.0 },
            ],
            closed: false,
        };
        let g = Geom::Polyline(pl);
        // Diagonal line crosses both segments.
        let line = Geom::Line(Line {
            a: Vec2::new(-1.0, -1.0), b: Vec2::new(5.0, 5.0),
        });
        let hits = intersect(&g, &line);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn offset_ellipse_outward_grows_bbox() {
        // Outward offset: pick `side` outside the ellipse. The resulting
        // polyline should be a closed loop whose bbox is wider than the
        // original.
        let el = Ellipse {
            center: Vec2::ZERO,
            major:  Vec2::new(2.0, 0.0),
            ratio:  0.5,
        };
        let g = Geom::Ellipse(el);
        let out = g.offset(0.5, Vec2::new(10.0, 0.0)).unwrap();
        if let Geom::Polyline(p) = out {
            assert!(p.closed, "outward ellipse offset must be a closed polyline");
            assert!(p.vertices.len() >= 48);
            // Every offset vertex should sit outside the original ellipse —
            // the parametric form satisfies x²/a² + y²/b² >= 1.
            for v in &p.vertices {
                let val = (v.pos.x / 2.0).powi(2) + (v.pos.y / 1.0).powi(2);
                assert!(val > 1.0,
                    "offset vertex {:?} lies inside original ellipse (val={})",
                    v.pos, val);
            }
        } else {
            panic!("expected Polyline, got {:?}", out);
        }
    }

    #[test]
    fn offset_ellipse_inward_shrinks_inside_bbox() {
        // Inward offset: pick `side` inside. Every vertex sits inside the
        // original ellipse.
        let el = Ellipse {
            center: Vec2::ZERO,
            major:  Vec2::new(2.0, 0.0),
            ratio:  0.5,
        };
        let g = Geom::Ellipse(el);
        let out = g.offset(0.3, Vec2::new(0.0, 0.0)).unwrap();
        if let Geom::Polyline(p) = out {
            assert!(p.closed);
            for v in &p.vertices {
                let val = (v.pos.x / 2.0).powi(2) + (v.pos.y / 1.0).powi(2);
                assert!(val < 1.0,
                    "offset vertex {:?} lies outside original ellipse (val={})",
                    v.pos, val);
            }
        } else {
            panic!("expected Polyline, got {:?}", out);
        }
    }

    #[test]
    fn offset_ellipse_arc_returns_open_polyline() {
        // Half-ellipse arc (top): start_param=0, sweep=π.
        let ea = EllipseArc {
            ellipse: Ellipse {
                center: Vec2::ZERO,
                major:  Vec2::new(2.0, 0.0),
                ratio:  0.5,
            },
            start_param: 0.0,
            sweep_param: std::f64::consts::PI,
        };
        let g = Geom::EllipseArc(ea);
        let out = g.offset(0.2, Vec2::new(0.0, 5.0)).unwrap();
        if let Geom::Polyline(p) = out {
            assert!(!p.closed, "ellipse arc offset must be an OPEN polyline");
            assert!(p.vertices.len() >= 49);  // n+1 samples
        } else {
            panic!("expected Polyline, got {:?}", out);
        }
    }

    #[test]
    fn trim_ellipse_single_tangent_intersection_errs() {
        let el = Ellipse {
            center: Vec2::ZERO,
            major:  Vec2::new(2.0, 0.0),
            ratio:  0.5,
        };
        let g = Geom::Ellipse(el);
        // Tangent at the top: y = 1. Touches at one point (0, 1).
        let tangent = Geom::Line(Line {
            a: Vec2::new(-3.0, 1.0),
            b: Vec2::new( 3.0, 1.0),
        });
        let err = g.trim_at(&[tangent], Vec2::new(0.0, 0.5), false).unwrap_err();
        assert!(err.contains("at least 2 intersections")
             || err.contains("no intersection"));
    }

    // ---- Wall ----------------------------------------------------------
    #[test]
    fn wall_translated_moves_centerline_keeps_thickness() {
        let w = Wall { start: Vec2::new(0.0, 0.0), end: Vec2::new(10.0, 0.0),
                       thickness: 2.0, style: 0, bulge: 0.0 };
        let g = Geom::Wall(w).translated(Vec2::new(5.0, 3.0));
        if let Geom::Wall(w2) = g {
            assert_eq!(w2.start, Vec2::new(5.0, 3.0));
            assert_eq!(w2.end,   Vec2::new(15.0, 3.0));
            assert_eq!(w2.thickness, 2.0);
        } else { panic!("translated lost variant"); }
    }

    #[test]
    fn wall_scaled_scales_thickness() {
        let w = Wall { start: Vec2::ZERO, end: Vec2::new(4.0, 0.0), thickness: 1.0, style: 0, bulge: 0.0 };
        let g = Geom::Wall(w).scaled(Vec2::ZERO, 2.5);
        if let Geom::Wall(w2) = g {
            assert!((w2.end.x - 10.0).abs() < 1e-12);
            assert!((w2.thickness - 2.5).abs() < 1e-12);
        } else { panic!("scaled lost variant"); }
    }

    #[test]
    fn wall_rotated_90_swaps_axes() {
        let w = Wall { start: Vec2::ZERO, end: Vec2::new(5.0, 0.0), thickness: 1.0, style: 0, bulge: 0.0 };
        let g = Geom::Wall(w).rotated(Vec2::ZERO, std::f64::consts::FRAC_PI_2);
        if let Geom::Wall(w2) = g {
            assert!((w2.end - Vec2::new(0.0, 5.0)).len() < 1e-9);
            assert_eq!(w2.thickness, 1.0);
        } else { panic!("rotated lost variant"); }
    }

    #[test]
    fn wall_distance_to_point_picks_nearer_side() {
        // Horizontal wall along the X-axis, thickness 2 → sides at y=±1.
        // Point at (5, 0.3) is distance 0.7 from the upper side and 1.3 from the lower.
        let w = Wall { start: Vec2::new(0.0, 0.0), end: Vec2::new(10.0, 0.0),
                       thickness: 2.0, style: 0, bulge: 0.0 };
        let d = Geom::Wall(w).distance_to_point(Vec2::new(5.0, 0.3));
        assert!((d - 0.7).abs() < 1e-9);
    }

    #[test]
    fn wall_bbox_includes_thickness() {
        let w = Wall { start: Vec2::ZERO, end: Vec2::new(10.0, 0.0), thickness: 2.0, style: 0, bulge: 0.0 };
        let (min, max) = Geom::Wall(w).bbox();
        // Loose bbox: expanded by thk/2 = 1.0 in both axes.
        assert!((min.y + 1.0).abs() < 1e-9);
        assert!((max.y - 1.0).abs() < 1e-9);
    }

    #[test]
    fn arc_bbox_is_tight_not_full_circle() {
        // 45°→135° top-cap arc, r=5 at origin. Tight bbox must hug the cap,
        // NOT return the full circle (that broke window selection).
        let a = Arc {
            center: Vec2::ZERO, radius: 5.0,
            start_angle: std::f64::consts::FRAC_PI_4,
            sweep_angle: std::f64::consts::FRAC_PI_2,
        };
        let (min, max) = Geom::Arc(a).bbox();
        assert!((max.y - 5.0).abs() < 1e-6, "top cardinal swept → max.y=5, got {}", max.y);
        assert!(min.y > 3.0, "bottom NOT swept → min.y ~3.54 (not -5), got {}", min.y);
        assert!((max.x - 3.5355).abs() < 1e-3, "max.x from endpoint, got {}", max.x);
        assert!((min.x + 3.5355).abs() < 1e-3, "min.x from endpoint, got {}", min.x);
    }

    #[test]
    fn curved_wall_faces_meet_tangent_straight_wall_exactly() {
        // Straight wall along +x ending at (10,0); tangent CCW quarter-arc
        // wall (10,0) → (15,5), centre (10,5), r=5 (bulge = tan(90°/4)).
        // This is exactly the configuration `fillet r>0` on two walls
        // produces. The curved wall's FIRST face points must coincide with
        // the straight wall's END face points — the old chord-normal
        // offsetting left a ≈(t/2)·sweep/(2·steps) gap here.
        let b = (std::f64::consts::FRAC_PI_2 / 4.0).tan();
        let s = Wall { start: Vec2::new(0.0, 0.0), end: Vec2::new(10.0, 0.0),
                       thickness: 0.3, style: 0, bulge: 0.0 };
        let c = Wall { start: Vec2::new(10.0, 0.0), end: Vec2::new(15.0, 5.0),
                       thickness: 0.3, style: 0, bulge: b };
        let (sl, sr) = s.face_polylines(1).unwrap();
        let (cl, cr) = c.face_polylines(28).unwrap();
        let gap_l = (sl[1] - cl[0]).len();
        let gap_r = (sr[1] - cr[0]).len();
        assert!(gap_l < 1e-9, "left-face gap at tangent joint: {}", gap_l);
        assert!(gap_r < 1e-9, "right-face gap at tangent joint: {}", gap_r);
        // Faces must be true concentric arcs: inner radius 5−0.15,
        // outer 5+0.15, for every sample.
        let centre = Vec2::new(10.0, 5.0);
        for p in &cl { assert!(((  *p - centre).len() - 4.85).abs() < 1e-9); }
        for p in &cr { assert!(((  *p - centre).len() - 5.15).abs() < 1e-9); }
    }

    #[test]
    fn bulge_roundtrip_reconstructs_arc() {
        // CCW quarter arc, radius 5, centre origin: (5,0) → (0,5), sweep 90°.
        let center = Vec2::new(0.0, 0.0);
        let (start, end) = (Vec2::new(5.0, 0.0), Vec2::new(0.0, 5.0));
        let sweep = std::f64::consts::FRAC_PI_2;
        let b = bulge_from_arc(start, end, center, sweep);
        let (c2, r2, _a0, sw) = bulge_arc(start, end, b).expect("arc");
        assert!((c2 - center).len() < 1e-6, "centre {:?}", c2);
        assert!((r2 - 5.0).abs() < 1e-6, "radius {}", r2);
        assert!((sw - sweep).abs() < 1e-6, "sweep {}", sw);
    }
}
