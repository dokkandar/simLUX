//! Command-driven 2D drafting — an AutoCAD-style command environment.
//!
//! Reuses Auto_RASM's pure parser (`cad_kernel::parse`) and geometry model
//! (`cad_kernel::Document`). The interactive orchestration (prompts, point
//! collection, coordinate entry) is rebuilt here because Auto_RASM's dispatch is
//! bound to its native egui UI and can't be ported. Draw commands only for now;
//! modify commands (trim/extend/offset/…) parse but report "not yet".
use cad_kernel::{
    arc_three_points, parse, Arc as KArc, Circle as KCircle, Command, DObject, Document, Geom,
    Line as KLine, Point as KPoint, PolyVertex, Polyline as KPoly, ToolKind, Vec2, Wall as KWall,
};
use serde::Serialize;

use crate::engine::geometry::{Mesh, Point2, Triangle, Vertex};
use crate::engine::wall::triangulate;
use crate::model::MaterialId;

const FLOOR: MaterialId = 0;
const WALL: MaterialId = 1;
const CEILING: MaterialId = 2;
const CURVE_SEGMENTS: usize = 48;

/// A command currently collecting points.
struct Active {
    tool: ToolKind,
    pts: Vec<Vec2>,
    thickness: f64,
}

/// The drafting session: the document plus any in-progress command.
pub struct Draft {
    pub doc: Document,
    active: Option<Active>,
    thickness: f64,
}

impl Default for Draft {
    fn default() -> Self {
        Self { doc: Document::default(), active: None, thickness: 0.1 }
    }
}

/// A serialisable drawing entity for the frontend (2D render + snapping).
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GeomDto {
    Line { a: [f32; 2], b: [f32; 2] },
    Wall { a: [f32; 2], b: [f32; 2], thickness: f32 },
    Polyline { pts: Vec<[f32; 2]>, closed: bool },
    Circle { c: [f32; 2], r: f32 },
    Arc { c: [f32; 2], r: f32, start_deg: f32, sweep_deg: f32 },
    Point { p: [f32; 2] },
}

/// The result of a command-line step — feedback plus the full geometry snapshot.
#[derive(Serialize)]
pub struct CmdResult {
    pub ok: bool,
    pub message: String,
    pub prompt: String,
    pub active: bool,
    pub active_tool: Option<String>,
    pub active_pts: Vec<[f32; 2]>,
    pub geometry: Vec<GeomDto>,
}

fn a2(v: Vec2) -> [f32; 2] {
    [v.x as f32, v.y as f32]
}

fn tool_word(t: ToolKind) -> String {
    match t {
        ToolKind::Line => "line",
        ToolKind::Polyline => "polyline",
        ToolKind::Rectangle => "rectangle",
        ToolKind::Circle => "circle",
        ToolKind::Arc => "arc",
        ToolKind::Wall => "wall",
        ToolKind::Point => "point",
        _ => "tool",
    }
    .to_string()
}

impl Draft {
    fn snapshot(&self) -> Vec<GeomDto> {
        self.doc
            .dobjects
            .iter()
            .filter_map(|d| match &d.geom {
                Geom::Line(l) => Some(GeomDto::Line { a: a2(l.a), b: a2(l.b) }),
                Geom::Wall(w) => Some(GeomDto::Wall { a: a2(w.start), b: a2(w.end), thickness: w.thickness as f32 }),
                Geom::Polyline(p) => Some(GeomDto::Polyline {
                    pts: p.vertices.iter().map(|v| a2(v.pos)).collect(),
                    closed: p.closed,
                }),
                Geom::Circle(c) => Some(GeomDto::Circle { c: a2(c.center), r: c.radius as f32 }),
                Geom::Arc(a) => Some(GeomDto::Arc {
                    c: a2(a.center),
                    r: a.radius as f32,
                    start_deg: a.start_angle.to_degrees() as f32,
                    sweep_deg: a.sweep_angle.to_degrees() as f32,
                }),
                Geom::Point(p) => Some(GeomDto::Point { p: a2(p.location) }),
                _ => None,
            })
            .collect()
    }

    fn result(&self, ok: bool, message: &str, prompt: &str) -> CmdResult {
        CmdResult {
            ok,
            message: message.to_string(),
            prompt: prompt.to_string(),
            active: self.active.is_some(),
            active_tool: self.active.as_ref().map(|a| tool_word(a.tool)),
            active_pts: self.active.as_ref().map(|a| a.pts.iter().map(|p| a2(*p)).collect()).unwrap_or_default(),
            geometry: self.snapshot(),
        }
    }

    fn start(&mut self, tool: ToolKind, thickness: f64) -> CmdResult {
        self.active = Some(Active { tool, pts: Vec::new(), thickness });
        let prompt = match tool {
            ToolKind::Circle => "Specify centre point:",
            ToolKind::Rectangle => "Specify first corner:",
            _ => "Specify first point:",
        };
        self.result(true, &format!("{}: ", tool_word(tool)), prompt)
    }

    /// Feed one world point to the active command (from a typed coord or a click).
    pub fn point(&mut self, p: Vec2) -> CmdResult {
        let Some(active) = self.active.as_mut() else {
            return self.result(false, "No active command.", "");
        };
        active.pts.push(p);
        let (tool, n) = (active.tool, active.pts.len());
        let pts = active.pts.clone();
        match tool {
            ToolKind::Point => {
                self.doc.push(DObject::new(Geom::Point(KPoint { location: pts[0], style: 0, size: 0.0 })));
                self.active = None;
                self.result(true, "Point placed.", "")
            }
            ToolKind::Rectangle if n >= 2 => {
                let (c0, c1) = (pts[0], pts[1]);
                let verts = [c0, Vec2::new(c1.x, c0.y), c1, Vec2::new(c0.x, c1.y)]
                    .iter()
                    .map(|&pos| PolyVertex { pos, bulge: 0.0 })
                    .collect();
                self.doc.push(DObject::new(Geom::Polyline(KPoly { vertices: verts, closed: true, widths: Vec::new() })));
                self.active = None;
                self.result(true, "Rectangle added.", "")
            }
            ToolKind::Rectangle => self.result(true, "", "Specify opposite corner:"),
            ToolKind::Circle if n >= 2 => {
                let r = (pts[1] - pts[0]).len();
                self.doc.push(DObject::new(Geom::Circle(KCircle { center: pts[0], radius: r })));
                self.active = None;
                self.result(true, "Circle added.", "")
            }
            ToolKind::Circle => self.result(true, "", "Specify radius (a point on the circle):"),
            ToolKind::Arc if n >= 3 => {
                if let Some(arc) = arc_three_points(pts[0], pts[1], pts[2]) {
                    self.doc.push(DObject::new(Geom::Arc(arc)));
                }
                self.active = None;
                self.result(true, "Arc added.", "")
            }
            ToolKind::Arc => self.result(true, "", if n == 1 { "Specify second point:" } else { "Specify end point:" }),
            // Line / Polyline / Wall: chained until finish / close.
            _ => self.result(true, "", "Specify next point or [Close/Undo]:"),
        }
    }

    fn commit_chain(&mut self, closed: bool) {
        let Some(active) = self.active.take() else { return };
        let pts = &active.pts;
        if pts.len() < 2 {
            return;
        }
        let mut segs: Vec<(Vec2, Vec2)> = pts.windows(2).map(|w| (w[0], w[1])).collect();
        if closed {
            segs.push((pts[pts.len() - 1], pts[0]));
        }
        match active.tool {
            ToolKind::Wall => {
                for (a, b) in segs {
                    self.doc.push(DObject::new(Geom::Wall(KWall {
                        start: a, end: b, thickness: active.thickness, style: 0, bulge: 0.0,
                    })));
                }
            }
            ToolKind::Polyline => {
                let verts = pts.iter().map(|&pos| PolyVertex { pos, bulge: 0.0 }).collect();
                self.doc.push(DObject::new(Geom::Polyline(KPoly { vertices: verts, closed, widths: Vec::new() })));
            }
            _ => {
                for (a, b) in segs {
                    self.doc.push(DObject::new(Geom::Line(KLine { a, b })));
                }
            }
        }
    }

    /// Handle one command-line line (a command, a coordinate, or a keyword).
    pub fn exec(&mut self, input: &str) -> CmdResult {
        let input = input.trim();

        if self.active.is_some() {
            if input.is_empty() {
                self.commit_chain(false);
                return self.result(true, "Done.", "");
            }
            match input.to_ascii_lowercase().as_str() {
                "c" | "close" => {
                    self.commit_chain(true);
                    return self.result(true, "Closed.", "");
                }
                "u" | "undo" => {
                    if let Some(a) = self.active.as_mut() {
                        a.pts.pop();
                    }
                    return self.result(true, "Undo point.", "Specify next point:");
                }
                _ => {}
            }
            let last = self.active.as_ref().and_then(|a| a.pts.last().copied());
            return match parse_coord(input, last) {
                Some(p) => self.point(p),
                None => self.result(false, &format!("Invalid point: {input}"), "Specify next point:"),
            };
        }

        if input.is_empty() {
            return self.result(true, "", "");
        }
        match parse(input) {
            Ok(Command::Add(geom)) => {
                self.doc.push(DObject::new(geom));
                self.result(true, "Added.", "")
            }
            Ok(Command::SetTool(kind)) => match kind {
                ToolKind::Line
                | ToolKind::Polyline
                | ToolKind::Rectangle
                | ToolKind::Circle
                | ToolKind::Arc
                | ToolKind::Point => {
                    let th = self.thickness;
                    self.start(kind, th)
                }
                _ => self.result(false, &format!("'{}' tool isn't supported yet.", tool_word(kind)), ""),
            },
            Ok(Command::Wall(opt)) => {
                if let Some(t) = opt {
                    self.thickness = t;
                }
                let th = self.thickness;
                self.start(ToolKind::Wall, th)
            }
            Ok(Command::Clear) => {
                self.doc = Document::default();
                self.result(true, "Cleared.", "")
            }
            Ok(_) => self.result(false, &format!("'{input}' — modify commands land next; draw only for now."), ""),
            Err(e) => self.result(false, &format!("Unknown command '{input}' ({e})."), ""),
        }
    }

    /// A click at a world point: feeds the active command, else no-op (select later).
    pub fn click(&mut self, x: f32, y: f32) -> CmdResult {
        if self.active.is_some() {
            self.point(Vec2::new(x as f64, y as f64))
        } else {
            self.result(false, "", "")
        }
    }

    /// Cancel the active command (Esc). Chained draws commit what's complete? No —
    /// Esc discards the in-progress command entirely, matching AutoCAD.
    pub fn cancel(&mut self) -> CmdResult {
        self.active = None;
        self.result(true, "*Cancel*", "")
    }

    pub fn snapshot_result(&self) -> CmdResult {
        self.result(true, "", "")
    }
}

/// Parse a coordinate token: `x,y` absolute, `@dx,dy` relative, `@d<a` polar
/// (relative), or `d<a` polar from origin.
fn parse_coord(tok: &str, last: Option<Vec2>) -> Option<Vec2> {
    let t = tok.trim();
    if let Some(rest) = t.strip_prefix('@') {
        let base = last.unwrap_or(Vec2::ZERO);
        if let Some((d, a)) = rest.split_once('<') {
            let (dist, ang) = (d.trim().parse::<f64>().ok()?, a.trim().parse::<f64>().ok()?);
            let r = ang.to_radians();
            return Some(Vec2::new(base.x + dist * r.cos(), base.y + dist * r.sin()));
        }
        let (dx, dy) = rest.split_once(',')?;
        return Some(Vec2::new(base.x + dx.trim().parse::<f64>().ok()?, base.y + dy.trim().parse::<f64>().ok()?));
    }
    if let Some((d, a)) = t.split_once('<') {
        let (dist, ang) = (d.trim().parse::<f64>().ok()?, a.trim().parse::<f64>().ok()?);
        let r = ang.to_radians();
        return Some(Vec2::new(dist * r.cos(), dist * r.sin()));
    }
    let (x, y) = t.split_once(',')?;
    Some(Vec2::new(x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?))
}

// --- extrusion: Document geometry → surfaces (one line = one surface) ---

fn surface(a: Vec2, b: Vec2, height: f32, material: MaterialId) -> Mesh {
    let v = |p: Vec2, z: f32| Vertex::new(p.x as f32, p.y as f32, z);
    Mesh {
        vertices: vec![v(a, 0.0), v(b, 0.0), v(b, height), v(a, height)],
        triangles: vec![Triangle { a: 0, b: 1, c: 2 }, Triangle { a: 0, b: 2, c: 3 }],
        material,
    }
}

fn cap(poly: &[Vec2], z: f32, material: MaterialId, out: &mut Vec<Mesh>) {
    let p2: Vec<Point2> = poly.iter().map(|v| Point2 { x: v.x as f32, y: v.y as f32 }).collect();
    let tris = triangulate(&p2);
    if tris.is_empty() {
        return;
    }
    out.push(Mesh {
        vertices: p2.iter().map(|p| Vertex::new(p.x, p.y, z)).collect(),
        triangles: tris.iter().map(|t| Triangle { a: t[0] as u32, b: t[1] as u32, c: t[2] as u32 }).collect(),
        material,
    });
}

fn extrude_path(pts: &[Vec2], closed: bool, height: f32, out: &mut Vec<Mesh>) {
    for w in pts.windows(2) {
        out.push(surface(w[0], w[1], height, WALL));
    }
    if closed && pts.len() >= 3 {
        out.push(surface(pts[pts.len() - 1], pts[0], height, WALL));
        cap(pts, 0.0, FLOOR, out);
        cap(pts, height, CEILING, out);
    }
}

fn circle_pts(center: Vec2, radius: f64) -> Vec<Vec2> {
    (0..CURVE_SEGMENTS)
        .map(|i| {
            let a = std::f64::consts::TAU * (i as f64 / CURVE_SEGMENTS as f64);
            Vec2::new(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect()
}

fn arc_pts(a: &KArc) -> Vec<Vec2> {
    (0..=CURVE_SEGMENTS)
        .map(|i| {
            let t = i as f64 / CURVE_SEGMENTS as f64;
            let ang = a.start_angle + a.sweep_angle * t;
            Vec2::new(a.center.x + a.radius * ang.cos(), a.center.y + a.radius * ang.sin())
        })
        .collect()
}

/// Extrude every drafted entity to surfaces (closed paths also get floor + ceiling).
pub fn extrude(doc: &Document, height: f32) -> Vec<Mesh> {
    let mut out = Vec::new();
    for d in &doc.dobjects {
        match &d.geom {
            Geom::Line(l) => extrude_path(&[l.a, l.b], false, height, &mut out),
            Geom::Wall(w) => extrude_path(&[w.start, w.end], false, height, &mut out),
            Geom::Polyline(p) => {
                let v: Vec<Vec2> = p.vertices.iter().map(|x| x.pos).collect();
                extrude_path(&v, p.closed, height, &mut out);
            }
            Geom::Circle(c) => extrude_path(&circle_pts(c.center, c.radius), true, height, &mut out),
            Geom::Arc(a) => extrude_path(&arc_pts(a), false, height, &mut out),
            _ => {}
        }
    }
    out
}

/// Bounding box of all drafted geometry (x-min, y-min, x-max, y-max).
pub fn bbox(doc: &Document) -> Option<(f32, f32, f32, f32)> {
    let (mut mnx, mut mny, mut mxx, mut mxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut any = false;
    let mut add = |v: Vec2| {
        any = true;
        mnx = mnx.min(v.x as f32);
        mny = mny.min(v.y as f32);
        mxx = mxx.max(v.x as f32);
        mxy = mxy.max(v.y as f32);
    };
    for d in &doc.dobjects {
        match &d.geom {
            Geom::Line(l) => { add(l.a); add(l.b); }
            Geom::Wall(w) => { add(w.start); add(w.end); }
            Geom::Polyline(p) => p.vertices.iter().for_each(|x| add(x.pos)),
            Geom::Circle(c) => {
                add(Vec2::new(c.center.x - c.radius, c.center.y - c.radius));
                add(Vec2::new(c.center.x + c.radius, c.center.y + c.radius));
            }
            Geom::Arc(a) => arc_pts(a).into_iter().for_each(&mut add),
            _ => {}
        }
    }
    any.then_some((mnx, mny, mxx, mxy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_line_adds_entity() {
        let mut d = Draft::default();
        let r = d.exec("line 0,0 3,0");
        assert!(r.ok);
        assert_eq!(d.doc.dobjects.len(), 1);
    }

    #[test]
    fn interactive_rectangle_and_extrude() {
        let mut d = Draft::default();
        d.exec("rectangle"); // or "rect"
        d.exec("0,0");
        let r = d.exec("4,3");
        assert!(r.ok);
        assert_eq!(d.doc.dobjects.len(), 1); // one closed polyline
        let meshes = extrude(&d.doc, 3.0);
        // 4 wall surfaces + floor + ceiling.
        assert_eq!(meshes.len(), 6);
    }

    #[test]
    fn relative_and_polar_coords() {
        let base = Vec2::new(1.0, 1.0);
        let rel = parse_coord("@2,0", Some(base)).unwrap();
        assert!((rel.x - 3.0).abs() < 1e-9 && (rel.y - 1.0).abs() < 1e-9);
        let pol = parse_coord("@2<90", Some(base)).unwrap();
        assert!((pol.x - 1.0).abs() < 1e-6 && (pol.y - 3.0).abs() < 1e-6);
    }

    #[test]
    fn chained_wall_loop_closes() {
        let mut d = Draft::default();
        d.exec("wall");
        d.exec("0,0");
        d.exec("4,0");
        d.exec("4,4");
        d.exec("0,4");
        d.exec("close");
        assert_eq!(d.doc.dobjects.len(), 4); // 4 wall segments
    }
}
