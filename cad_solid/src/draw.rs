//! 2D draw tools for sketches — recreates simLUX's `app.rs` draw interaction
//! (a `Tool` + a `pending: Vec<Vec2>` click buffer + a count-based finalise),
//! producing real `cad_kernel::Geom` that the sketch stores.
//!
//! The geometry MATH is reused verbatim from `cad_kernel` (the primitive structs
//! + the pure constructors `arc_three_points` / `ellipse_center_major_minor` / the
//! rectangle→closed-polyline form). Only the interaction is recreated here, so a
//! later merge with the app's draw layer is 1:1. Picks arrive as glam `Vec2` in
//! the sketch frame's `(u, v)`; they convert to `cad_kernel::Vec2` at construction.
//!
//! Behaviour mirrors the app: Line/Wall chain (re-arm from the last point), the
//! other tools re-arm for the next entity, Polyline accumulates until Enter, Esc
//! cancels the in-progress entity.

use cad_kernel::{
    arc_three_points, ellipse_center_major_minor, Circle, Geom, Line, Point, PolyVertex, Polyline, Vec2 as KVec2,
};
use glam::Vec2;

/// The active 2D draw tool. Mirrors the app's `ToolKind` (basic-drawing subset).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DrawTool {
    None,
    Line,
    Polyline,
    Circle,
    Arc,
    Rectangle,
    Ellipse,
    Point,
}

impl DrawTool {
    /// The user-selectable tools (excludes `None`).
    pub const ALL: [DrawTool; 7] = [
        DrawTool::Line,
        DrawTool::Polyline,
        DrawTool::Circle,
        DrawTool::Arc,
        DrawTool::Rectangle,
        DrawTool::Ellipse,
        DrawTool::Point,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DrawTool::None => "—",
            DrawTool::Line => "Line",
            DrawTool::Polyline => "Pline",
            DrawTool::Circle => "Circle",
            DrawTool::Arc => "Arc",
            DrawTool::Rectangle => "Rect",
            DrawTool::Ellipse => "Ellipse",
            DrawTool::Point => "Point",
        }
    }
}

#[inline]
fn kv(p: Vec2) -> KVec2 {
    KVec2::new(p.x as f64, p.y as f64)
}

/// An in-progress draw command: a tool + the picked points so far (frame `u,v`).
#[derive(Clone)]
pub struct Draw {
    pub tool: DrawTool,
    pub pending: Vec<Vec2>,
}

impl Default for Draw {
    fn default() -> Self {
        Self { tool: DrawTool::None, pending: Vec::new() }
    }
}

impl Draw {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self) -> bool {
        self.tool != DrawTool::None
    }

    /// Switch tool (clears any half-drawn entity), like picking a draw command.
    pub fn set_tool(&mut self, t: DrawTool) {
        self.tool = t;
        self.pending.clear();
    }

    /// Feed a pick (frame `u,v`). Returns a completed `Geom` to commit, or `None`
    /// while the entity still needs more points. Chains where the app chains.
    pub fn feed(&mut self, uv: Vec2) -> Option<Geom> {
        self.pending.push(uv);
        let n = self.pending.len();
        match self.tool {
            DrawTool::Line if n >= 2 => {
                let (a, b) = (self.pending[n - 2], self.pending[n - 1]);
                self.pending = vec![b]; // chain: continue from the last point
                Some(Geom::Line(Line { a: kv(a), b: kv(b) }))
            }
            DrawTool::Rectangle if n >= 2 => {
                let g = rect(self.pending[0], self.pending[1]);
                self.pending.clear();
                Some(g)
            }
            DrawTool::Circle if n >= 2 => {
                let (c, e) = (self.pending[0], self.pending[1]);
                self.pending.clear();
                Some(Geom::Circle(Circle { center: kv(c), radius: (e - c).length() as f64 }))
            }
            DrawTool::Arc if n >= 3 => {
                let g = arc_three_points(kv(self.pending[0]), kv(self.pending[1]), kv(self.pending[2])).map(Geom::Arc);
                self.pending.clear();
                g
            }
            DrawTool::Ellipse if n >= 3 => {
                let g = ellipse(self.pending[0], self.pending[1], self.pending[2]);
                self.pending.clear();
                g
            }
            DrawTool::Point if n >= 1 => {
                let p = self.pending[0];
                self.pending.clear();
                Some(Geom::Point(Point { location: kv(p), style: 0, size: 0.0 }))
            }
            _ => None,
        }
    }

    /// Enter: commit an in-progress polyline (≥2 pts) as an open polyline; also
    /// ends any Line chain. Returns the committed geom, if any.
    pub fn finish(&mut self) -> Option<Geom> {
        let out = if self.tool == DrawTool::Polyline && self.pending.len() >= 2 {
            let vertices = self.pending.iter().map(|p| PolyVertex { pos: kv(*p), bulge: 0.0 }).collect();
            Some(Geom::Polyline(Polyline { vertices, closed: false, widths: Vec::new() }))
        } else {
            None
        };
        self.pending.clear();
        out
    }

    /// Esc: drop the in-progress entity. Returns true if there was one (else the
    /// caller should leave sketch mode).
    pub fn cancel(&mut self) -> bool {
        if self.pending.is_empty() {
            false
        } else {
            self.pending.clear();
            true
        }
    }

    /// A short prompt for the current step (tool + how many picks so far).
    pub fn prompt(&self) -> String {
        let n = self.pending.len();
        let msg = match (self.tool, n) {
            (DrawTool::Line, 0) => "line: pick start",
            (DrawTool::Line, _) => "line: pick next point · Enter/Esc ends",
            (DrawTool::Polyline, 0) => "polyline: pick start",
            (DrawTool::Polyline, _) => "polyline: pick next · Enter finishes",
            (DrawTool::Circle, 0) => "circle: pick centre",
            (DrawTool::Circle, _) => "circle: pick radius",
            (DrawTool::Arc, 0) => "arc: pick start",
            (DrawTool::Arc, 1) => "arc: pick second point",
            (DrawTool::Arc, _) => "arc: pick end",
            (DrawTool::Rectangle, 0) => "rect: pick first corner",
            (DrawTool::Rectangle, _) => "rect: pick opposite corner",
            (DrawTool::Ellipse, 0) => "ellipse: pick centre",
            (DrawTool::Ellipse, 1) => "ellipse: pick major-axis end",
            (DrawTool::Ellipse, _) => "ellipse: pick minor distance",
            (DrawTool::Point, _) => "point: pick location",
            (DrawTool::None, _) => "pick a draw tool",
        };
        msg.to_string()
    }
}

/// Rectangle → a CLOSED 4-vertex polyline (the app's `rect_polyline`).
fn rect(a: Vec2, b: Vec2) -> Geom {
    let v = |x: f32, y: f32| PolyVertex { pos: KVec2::new(x as f64, y as f64), bulge: 0.0 };
    Geom::Polyline(Polyline {
        vertices: vec![v(a.x, a.y), v(b.x, a.y), v(b.x, b.y), v(a.x, b.y)],
        closed: true,
        widths: Vec::new(),
    })
}

/// Ellipse from centre + major-axis end + a point giving the minor half-width
/// (its perpendicular distance to the major axis). Reuses the kernel constructor.
fn ellipse(center: Vec2, major_end: Vec2, side: Vec2) -> Option<Geom> {
    let major = major_end - center;
    if major.length() < 1e-6 {
        return None;
    }
    let dir = major.normalize();
    let d = side - center;
    let semi_minor = (d - dir * d.dot(dir)).length();
    ellipse_center_major_minor(kv(center), kv(major_end), semi_minor as f64).map(Geom::Ellipse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_chains_from_last_point() {
        let mut d = Draw::new();
        d.set_tool(DrawTool::Line);
        assert!(d.feed(Vec2::new(0.0, 0.0)).is_none());
        let g = d.feed(Vec2::new(1.0, 0.0)).expect("segment committed");
        assert!(matches!(g, Geom::Line(_)));
        // pending re-armed with the last point so the next click continues.
        assert_eq!(d.pending.len(), 1);
        assert!((d.pending[0] - Vec2::new(1.0, 0.0)).length() < 1e-6);
    }

    #[test]
    fn circle_from_centre_and_radius() {
        let mut d = Draw::new();
        d.set_tool(DrawTool::Circle);
        d.feed(Vec2::ZERO);
        let g = d.feed(Vec2::new(3.0, 4.0)).unwrap();
        match g {
            Geom::Circle(c) => assert!((c.radius - 5.0).abs() < 1e-6),
            _ => panic!("expected circle"),
        }
        assert!(d.pending.is_empty(), "re-armed for next circle");
    }

    #[test]
    fn rectangle_is_a_closed_quad_polyline() {
        let mut d = Draw::new();
        d.set_tool(DrawTool::Rectangle);
        d.feed(Vec2::ZERO);
        let g = d.feed(Vec2::new(2.0, 1.0)).unwrap();
        match g {
            Geom::Polyline(p) => {
                assert_eq!(p.vertices.len(), 4);
                assert!(p.closed);
            }
            _ => panic!("expected polyline"),
        }
    }

    #[test]
    fn polyline_commits_on_finish() {
        let mut d = Draw::new();
        d.set_tool(DrawTool::Polyline);
        assert!(d.feed(Vec2::ZERO).is_none());
        assert!(d.feed(Vec2::new(1.0, 0.0)).is_none());
        assert!(d.feed(Vec2::new(1.0, 1.0)).is_none());
        let g = d.finish().unwrap();
        match g {
            Geom::Polyline(p) => {
                assert_eq!(p.vertices.len(), 3);
                assert!(!p.closed);
            }
            _ => panic!("expected polyline"),
        }
    }

    #[test]
    fn arc_three_points_builds_an_arc() {
        let mut d = Draw::new();
        d.set_tool(DrawTool::Arc);
        d.feed(Vec2::new(-1.0, 0.0));
        d.feed(Vec2::new(0.0, 1.0));
        let g = d.feed(Vec2::new(1.0, 0.0)).unwrap();
        assert!(matches!(g, Geom::Arc(_)));
    }
}
