// Geometric primitives. Tight, Copy, no virtual dispatch.

use crate::math::{Vec2, EPS, norm_angle};

#[derive(Clone, Copy, Debug)]
pub struct Line { pub a: Vec2, pub b: Vec2 }

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

/// Pure geometry — the shape side of a `DObject`. Style / layer / handle
/// live on the outer `DObject` struct (see [`crate::dobject`]).
///
/// Future variants land here: Text, MText, Hatch, BlockRef, Dim*,
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
            _ => Err("lengthen: only Line / Arc / EllipseArc are supported"),
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

    /// Trim this geometry by the given cutting edges. `pick` is a click point
    /// indicating the segment to REMOVE (AutoCAD convention). `edge_mode`
    /// controls whether the cutters are treated as their infinite extensions.
    ///
    /// Returns the surviving piece(s) — 0 (whole curve removed), 1 (single
    /// piece), or 2 (clicked segment was in the middle; outer pieces survive).
    ///
    /// Supported targets in v1: Line, Arc, EllipseArc. Other variants return
    /// an Err so the caller can leave them untouched and report.
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
            // Skip self-as-cutter (same handle would be ideal; here equality
            // by geometry shape is enough to avoid trivial 0-length cuts).
            let c_eff = if edge_mode { c.extended_for_edgemode() } else { c.clone() };
            hits.extend(intersect(self, &c_eff));
        }
        if hits.is_empty() {
            return Err("trim: target has no intersection with the cutting edges");
        }

        match self {
            Geom::Line(l) => {
                let d = l.b - l.a;
                let len_sq = d.len_sq();
                if len_sq < EPS { return Err("trim: zero-length line"); }
                let len = len_sq.sqrt();
                // Project each hit and the pick onto the parameter t ∈ [0,1].
                let to_t = |p: Vec2| -> f64 { (p - l.a).dot(d) / len_sq };
                let pick_t = to_t(pick).clamp(0.0, 1.0);
                let mut params: Vec<f64> = hits.iter()
                    .map(|&p| to_t(p))
                    .filter(|&t| t > EPS / len && t < 1.0 - EPS / len)
                    .collect();
                params.sort_by(|a, b| a.partial_cmp(b).unwrap());
                params.dedup_by(|a, b| (*a - *b).abs() < EPS / len);
                // Find the bounds of the click's segment.
                let before = params.iter().rev().find(|&&t| t < pick_t).copied();
                let after  = params.iter().find(|&&t| t > pick_t).copied();
                let mut out: Vec<Geom> = Vec::new();
                match (before, after) {
                    (Some(t1), Some(t2)) => {
                        // Middle segment removed; outer pieces survive.
                        if t1 > EPS / len {
                            out.push(Geom::Line(Line { a: l.a, b: l.a + d * t1 }));
                        }
                        if t2 < 1.0 - EPS / len {
                            out.push(Geom::Line(Line { a: l.a + d * t2, b: l.b }));
                        }
                    }
                    (Some(t1), None) => {
                        // Click is past the last intersection — trim toward b
                        out.push(Geom::Line(Line { a: l.a, b: l.a + d * t1 }));
                    }
                    (None, Some(t2)) => {
                        // Click is before the first intersection — trim toward a
                        out.push(Geom::Line(Line { a: l.a + d * t2, b: l.b }));
                    }
                    (None, None) => {
                        return Err("trim: pick on the wrong side of all intersections");
                    }
                }
                Ok(out)
            }
            Geom::Arc(arc) => {
                if arc.radius < EPS { return Err("trim: zero-radius arc"); }
                let to_local = |p: Vec2| -> f64 {
                    ((p - arc.center).angle() - arc.start_angle)
                        .rem_euclid(std::f64::consts::TAU)
                };
                let pick_t = to_local(pick).clamp(0.0, arc.sweep_angle);
                let mut params: Vec<f64> = hits.iter()
                    .map(|&p| to_local(p))
                    .filter(|&t| t > EPS && t < arc.sweep_angle - EPS)
                    .collect();
                params.sort_by(|a, b| a.partial_cmp(b).unwrap());
                params.dedup_by(|a, b| (*a - *b).abs() < EPS);
                let before = params.iter().rev().find(|&&t| t < pick_t).copied();
                let after  = params.iter().find(|&&t| t > pick_t).copied();
                let mk = |s: f64, w: f64| Geom::Arc(Arc {
                    center: arc.center, radius: arc.radius,
                    start_angle: (arc.start_angle + s).rem_euclid(std::f64::consts::TAU),
                    sweep_angle: w,
                });
                let mut out: Vec<Geom> = Vec::new();
                match (before, after) {
                    (Some(t1), Some(t2)) => {
                        if t1 > EPS { out.push(mk(0.0, t1)); }
                        if t2 < arc.sweep_angle - EPS {
                            out.push(mk(t2, arc.sweep_angle - t2));
                        }
                    }
                    (Some(t1), None) => out.push(mk(0.0, t1)),
                    (None, Some(t2)) => out.push(mk(t2, arc.sweep_angle - t2)),
                    (None, None) => return Err("trim: pick on the wrong side of all intersections"),
                }
                Ok(out)
            }
            Geom::EllipseArc(ea) => {
                // Same shape as Arc, but with parameter space (sweep_param).
                let to_local = |p: Vec2| -> f64 {
                    (ea.ellipse.nearest_param(p) - ea.start_param)
                        .rem_euclid(std::f64::consts::TAU)
                };
                let pick_t = to_local(pick).clamp(0.0, ea.sweep_param);
                let mut params: Vec<f64> = hits.iter()
                    .map(|&p| to_local(p))
                    .filter(|&t| t > EPS && t < ea.sweep_param - EPS)
                    .collect();
                params.sort_by(|a, b| a.partial_cmp(b).unwrap());
                params.dedup_by(|a, b| (*a - *b).abs() < EPS);
                let before = params.iter().rev().find(|&&t| t < pick_t).copied();
                let after  = params.iter().find(|&&t| t > pick_t).copied();
                let mk = |s: f64, w: f64| Geom::EllipseArc(EllipseArc {
                    ellipse: ea.ellipse,
                    start_param: (ea.start_param + s).rem_euclid(std::f64::consts::TAU),
                    sweep_param: w,
                });
                let mut out: Vec<Geom> = Vec::new();
                match (before, after) {
                    (Some(t1), Some(t2)) => {
                        if t1 > EPS { out.push(mk(0.0, t1)); }
                        if t2 < ea.sweep_param - EPS {
                            out.push(mk(t2, ea.sweep_param - t2));
                        }
                    }
                    (Some(t1), None) => out.push(mk(0.0, t1)),
                    (None, Some(t2)) => out.push(mk(t2, ea.sweep_param - t2)),
                    (None, None) => return Err("trim: pick on the wrong side of all intersections"),
                }
                Ok(out)
            }
            Geom::Circle(_) =>
                Err("trim: Circle requires two-pick cut (v2) — pick removed segment on an Arc instead"),
            Geom::Ellipse(_) =>
                Err("trim: Ellipse requires two-pick cut (v2)"),
            Geom::Polyline(_) =>
                Err("trim: Polyline trim not implemented yet (per-segment dispatch TBD)"),
            Geom::Point(_) =>
                Err("trim: Point has nothing to trim"),
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
            _ => Err("extend: only Line / Arc are supported in v1"),
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
            Geom::Ellipse(_) | Geom::EllipseArc(_) =>
                Err("offset on ellipse not implemented (true offset is not an ellipse)"),
            Geom::Polyline(_) =>
                Err("offset on polyline not implemented yet (corner math TBD)"),
            Geom::Point(_) =>
                Err("offset on point is undefined"),
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
            Geom::Circle(_) | Geom::Ellipse(_) | Geom::Point(_) => self.clone(),
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
            Geom::Arc(a) => (
                Vec2::new(a.center.x - a.radius, a.center.y - a.radius),
                Vec2::new(a.center.x + a.radius, a.center.y + a.radius),
            ),
            Geom::Ellipse(e)     => e.bbox(),
            Geom::EllipseArc(ea) => ea.ellipse.bbox(),
            Geom::Point(pt) => (pt.location, pt.location),
            Geom::Polyline(pl) => pl.bbox(),
        }
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
    fn offset_polyline_errors_politely() {
        let g = Geom::Polyline(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::ZERO, bulge: 0.0 },
                PolyVertex { pos: Vec2::new(1.0, 0.0), bulge: 0.0 },
            ],
            closed: false,
        });
        assert!(g.offset(1.0, Vec2::ZERO).is_err());
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
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let c1 = Geom::Line(Line { a: Vec2::new(3.0, -5.0), b: Vec2::new(3.0, 5.0) });
        let c2 = Geom::Line(Line { a: Vec2::new(7.0, -5.0), b: Vec2::new(7.0, 5.0) });
        let out = target.trim_at(&[c1, c2], Vec2::new(5.0, 0.0), false).unwrap();
        assert_eq!(out.len(), 2);
        // Outer pieces 0→3 and 7→10
        let mut xs: Vec<(f64, f64)> = out.iter().map(|g| {
            if let Geom::Line(l) = g { (l.a.x, l.b.x) } else { panic!() }
        }).collect();
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        assert!(approx_eq(xs[0].0, 0.0)); assert!(approx_eq(xs[0].1, 3.0));
        assert!(approx_eq(xs[1].0, 7.0)); assert!(approx_eq(xs[1].1, 10.0));
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
