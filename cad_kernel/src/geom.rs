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
