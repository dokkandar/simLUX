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

        /// Drop the segment containing `pick_t` from a list of consecutive
        /// param bounds; return every other segment as (t_start, t_end).
        fn surviving_segments(bounds: &[f64], pick_t: f64, eps: f64) -> Vec<(f64, f64)> {
            let mut out = Vec::new();
            for i in 0..bounds.len() - 1 {
                let t1 = bounds[i];
                let t2 = bounds[i + 1];
                if (t2 - t1) <= eps { continue; }   // skip empty segments
                let click_inside = pick_t > t1 - eps && pick_t < t2 + eps;
                if click_inside { continue; }       // drop the clicked one
                out.push((t1, t2));
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
    fn trim_line_with_three_cutters_breaks_into_separate_pieces() {
        // Critical multi-cut test: line 0→10 with cuts at x=2, 5, 8.
        // 3 cuts = 4 segments. Click in segment (2..5) → 3 separate pieces
        // remain: (0..2), (5..8), (8..10). NOT merged into 2 outer chunks.
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) });
        let cs: Vec<Geom> = [2.0, 5.0, 8.0].iter().map(|&x| {
            Geom::Line(Line { a: Vec2::new(x, -5.0), b: Vec2::new(x, 5.0) })
        }).collect();
        let out = target.trim_at(&cs, Vec2::new(3.5, 0.0), false).unwrap();
        assert_eq!(out.len(), 3, "expected 3 surviving pieces, got {}", out.len());
        let mut xs: Vec<(f64, f64)> = out.iter().map(|g| {
            if let Geom::Line(l) = g { (l.a.x, l.b.x) } else { panic!() }
        }).collect();
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        assert!(approx_eq(xs[0].0, 0.0) && approx_eq(xs[0].1, 2.0));
        assert!(approx_eq(xs[1].0, 5.0) && approx_eq(xs[1].1, 8.0));
        assert!(approx_eq(xs[2].0, 8.0) && approx_eq(xs[2].1, 10.0));
    }

    #[test]
    fn trim_line_with_five_cutters_makes_5_pieces_on_middle_click() {
        // 5 cuts = 6 segments. Click in segment 3 → 5 pieces survive.
        let target = Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(12.0, 0.0) });
        let cs: Vec<Geom> = [2.0, 4.0, 6.0, 8.0, 10.0].iter().map(|&x| {
            Geom::Line(Line { a: Vec2::new(x, -5.0), b: Vec2::new(x, 5.0) })
        }).collect();
        // Click in segment (4..6), at x=5.
        let out = target.trim_at(&cs, Vec2::new(5.0, 0.0), false).unwrap();
        assert_eq!(out.len(), 5, "expected 5 surviving pieces, got {}", out.len());
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

impl Geom {
    /// Characteristic "grip" points the UI can render as drag handles.
    /// v1 grip semantics: dragging any grip translates the whole dobject
    /// by the cursor delta. Per-grip role behaviour (e.g. circle-quadrant
    /// → change radius) is deferred to v2.
    ///   - Line:       endpoints + midpoint
    ///   - Arc:        endpoints + midpoint + center
    ///   - Circle:     center + 4 quadrants
    ///   - Ellipse:    center + 4 axis tips
    ///   - EllipseArc: endpoints + center
    ///   - Polyline:   every vertex
    ///   - Point:      the point itself
    pub fn grip_points(&self) -> Vec<Vec2> {
        match self {
            Geom::Line(l) => vec![l.a, l.b, (l.a + l.b) * 0.5],
            Geom::Circle(c) => {
                let r = c.radius;
                vec![
                    c.center,
                    c.center + Vec2::new( r, 0.0),
                    c.center + Vec2::new( 0.0,  r),
                    c.center + Vec2::new(-r, 0.0),
                    c.center + Vec2::new( 0.0, -r),
                ]
            }
            Geom::Arc(a) => {
                let (e1, e2) = a.endpoints();
                let mid_t = a.start_angle + a.sweep_angle * 0.5;
                let mid   = a.center + Vec2::new(
                    a.radius * mid_t.cos(),
                    a.radius * mid_t.sin(),
                );
                vec![e1, e2, mid, a.center]
            }
            Geom::Ellipse(el) => {
                let half = std::f64::consts::FRAC_PI_2;
                vec![
                    el.center,
                    el.point_at(0.0),
                    el.point_at(half),
                    el.point_at(std::f64::consts::PI),
                    el.point_at(std::f64::consts::PI + half),
                ]
            }
            Geom::EllipseArc(ea) => {
                let (e1, e2) = ea.endpoints();
                vec![e1, e2, ea.ellipse.center]
            }
            Geom::Polyline(p) => p.vertices.iter().map(|v| v.pos).collect(),
            Geom::Point(p) => vec![p.location],
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

    // Pick start_angle / sweep so that the midpoint of the arc lies on the
    // bisector between I and `center` (i.e. on the corner side, not the far
    // side). Both CCW candidates have the same magnitude (π - θ); only the
    // start endpoint differs.
    let arc_angle = std::f64::consts::PI - theta;
    let v1 = tp1 - center;
    let v2 = tp2 - center;
    let mid_dir_expected = (i_pt - center) / (i_pt - center).len();
    // Rotate v1 CCW by arc_angle/2 and see if it lines up with expected mid.
    let s = arc_angle * 0.5;
    let mid_v_ccw_from_1 = Vec2::new(
        v1.x * s.cos() - v1.y * s.sin(),
        v1.x * s.sin() + v1.y * s.cos(),
    );
    let dot_from_1 = (mid_v_ccw_from_1 / mid_v_ccw_from_1.len()).dot(mid_dir_expected);
    let start_angle = if dot_from_1 > 0.0 {
        v1.angle().rem_euclid(std::f64::consts::TAU)
    } else {
        v2.angle().rem_euclid(std::f64::consts::TAU)
    };
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
}
