//! Command-driven 2D drafting — an AutoCAD-style command environment.
//!
//! Reuses Auto_RASM's pure parser (`cad_kernel::parse`), geometry model
//! (`cad_kernel::Document`), and edit operations (`Geom::offset/trim_at/extend_to`,
//! `DObject::translated/rotated/scaled/mirrored`). The interactive orchestration
//! (prompts, selection, point/entity collection, coordinate entry) is rebuilt
//! here — Auto_RASM's dispatch is bound to its native egui UI.
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

/// The active operation and the input it's collecting.
enum Op {
    Draw { tool: ToolKind, pts: Vec<Vec2>, thickness: f64 },
    Move { copy: bool, base: Option<Vec2> },
    Rotate { base: Option<Vec2> },
    Scale { base: Option<Vec2> },
    Mirror { first: Option<Vec2> },
    Offset { dist: f64, ent: Option<usize> },
    Trim,
    Extend,
}

/// The drafting session: the document, the active op, and the current selection.
pub struct Draft {
    pub doc: Document,
    op: Option<Op>,
    selected: Vec<usize>,
    thickness: f64,
}

impl Default for Draft {
    fn default() -> Self {
        Self { doc: Document::default(), op: None, selected: Vec::new(), thickness: 0.1 }
    }
}

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

#[derive(Serialize)]
pub struct CmdResult {
    pub ok: bool,
    pub message: String,
    pub prompt: String,
    pub active: bool,
    pub active_tool: Option<String>,
    pub active_pts: Vec<[f32; 2]>,
    pub selected: Vec<u32>,
    pub geometry: Vec<GeomDto>,
}

fn a2(v: Vec2) -> [f32; 2] {
    [v.x as f32, v.y as f32]
}

fn tool_word(t: ToolKind) -> &'static str {
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
}

fn op_word(op: &Op) -> String {
    match op {
        Op::Draw { tool, .. } => tool_word(*tool).to_string(),
        Op::Move { copy: false, .. } => "move".into(),
        Op::Move { copy: true, .. } => "copy".into(),
        Op::Rotate { .. } => "rotate".into(),
        Op::Scale { .. } => "scale".into(),
        Op::Mirror { .. } => "mirror".into(),
        Op::Offset { .. } => "offset".into(),
        Op::Trim => "trim".into(),
        Op::Extend => "extend".into(),
    }
}

fn op_pts(op: &Op) -> Vec<[f32; 2]> {
    match op {
        Op::Draw { pts, .. } => pts.iter().map(|p| a2(*p)).collect(),
        Op::Move { base: Some(b), .. } | Op::Rotate { base: Some(b) } | Op::Scale { base: Some(b) } => vec![a2(*b)],
        Op::Mirror { first: Some(f) } => vec![a2(*f)],
        _ => Vec::new(),
    }
}

impl Draft {
    fn snapshot(&self) -> Vec<GeomDto> {
        self.doc
            .dobjects
            .iter()
            .filter_map(|d| match &d.geom {
                Geom::Line(l) => Some(GeomDto::Line { a: a2(l.a), b: a2(l.b) }),
                Geom::Wall(w) => Some(GeomDto::Wall { a: a2(w.start), b: a2(w.end), thickness: w.thickness as f32 }),
                Geom::Polyline(p) => Some(GeomDto::Polyline { pts: p.vertices.iter().map(|v| a2(v.pos)).collect(), closed: p.closed }),
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
            active: self.op.is_some(),
            active_tool: self.op.as_ref().map(op_word),
            active_pts: self.op.as_ref().map(op_pts).unwrap_or_default(),
            selected: self.selected.iter().map(|&i| i as u32).collect(),
            geometry: self.snapshot(),
        }
    }

    // ---- command entry ----

    /// Handle one command-line line (a command, a coordinate, or a keyword).
    pub fn exec(&mut self, input: &str) -> CmdResult {
        let input = input.trim();

        if self.op.is_some() {
            let is_draw = matches!(self.op, Some(Op::Draw { .. }));
            if is_draw {
                if input.is_empty() {
                    return self.finish(false);
                }
                match input.to_ascii_lowercase().as_str() {
                    "c" | "close" => return self.finish(true),
                    "u" | "undo" => {
                        if let Some(Op::Draw { pts, .. }) = &mut self.op {
                            pts.pop();
                        }
                        return self.result(true, "Undo.", "Specify next point:");
                    }
                    _ => {}
                }
            } else if input.is_empty() {
                self.op = None;
                return self.result(true, "*Cancel*", "");
            }
            // Numeric answer for scale factor / rotate angle.
            if let Ok(v) = input.parse::<f64>() {
                match self.op.take() {
                    Some(Op::Scale { base: Some(b) }) => {
                        self.apply_scale(b, v);
                        return self.result(true, "Scaled.", "");
                    }
                    Some(Op::Rotate { base: Some(b) }) => {
                        self.apply_rotate(b, v.to_radians());
                        return self.result(true, "Rotated.", "");
                    }
                    other => self.op = other,
                }
            }
            let last = self.last_point();
            return match parse_coord(input, last) {
                Some(p) => self.point_input(p),
                None => self.result(false, &format!("Invalid point: {input}"), "Specify point:"),
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
                ToolKind::Line | ToolKind::Polyline | ToolKind::Rectangle | ToolKind::Circle | ToolKind::Arc | ToolKind::Point => {
                    let th = self.thickness;
                    self.start_draw(kind, th)
                }
                _ => self.result(false, &format!("'{}' tool isn't supported yet.", tool_word(kind)), ""),
            },
            Ok(Command::Wall(opt)) => {
                if let Some(t) = opt {
                    self.thickness = t;
                }
                let th = self.thickness;
                self.start_draw(ToolKind::Wall, th)
            }
            Ok(Command::Clear) => {
                self.doc = Document::default();
                self.selected.clear();
                self.result(true, "Cleared.", "")
            }
            Ok(Command::Move) => self.start_modify(Op::Move { copy: false, base: None }, "Specify base point:"),
            Ok(Command::Copy) => self.start_modify(Op::Move { copy: true, base: None }, "Specify base point:"),
            Ok(Command::Rotate) => self.start_modify(Op::Rotate { base: None }, "Specify base point:"),
            Ok(Command::Scale) => self.start_modify(Op::Scale { base: None }, "Specify base point:"),
            Ok(Command::Mirror) => self.start_modify(Op::Mirror { first: None }, "Specify first point of mirror line:"),
            Ok(Command::Offset(opt)) => {
                self.op = Some(Op::Offset { dist: opt.unwrap_or(0.2), ent: None });
                self.result(true, "", "Select an object to offset (click it):")
            }
            Ok(Command::Trim) => {
                self.op = Some(Op::Trim);
                self.result(true, "", "Select an object to trim (Esc to finish):")
            }
            Ok(Command::Extend) => {
                self.op = Some(Op::Extend);
                self.result(true, "", "Select an object to extend (Esc to finish):")
            }
            Ok(Command::DeleteSelected) => {
                let n = self.erase();
                self.result(true, &format!("Erased {n}."), "")
            }
            Ok(Command::SelectNone) => {
                self.selected.clear();
                self.result(true, "Deselected.", "")
            }
            Ok(Command::Select) => self.result(true, "Click objects to select.", ""),
            Ok(_) => self.result(false, &format!("'{input}' isn't wired yet."), ""),
            Err(e) => self.result(false, &format!("Unknown command '{input}' ({e})."), ""),
        }
    }

    /// A click at a world point with a world-space pick tolerance.
    pub fn click(&mut self, x: f32, y: f32, tol: f32) -> CmdResult {
        let p = Vec2::new(x as f64, y as f64);
        let tol = tol as f64;
        match &self.op {
            Some(Op::Trim) => {
                if let Some(i) = self.hit_test(p, tol) {
                    self.apply_trim(i, p);
                }
                self.result(true, "", "Select an object to trim (Esc to finish):")
            }
            Some(Op::Extend) => {
                if let Some(i) = self.hit_test(p, tol) {
                    self.apply_extend(i, p);
                }
                self.result(true, "", "Select an object to extend (Esc to finish):")
            }
            Some(Op::Offset { ent: None, .. }) => {
                if let Some(i) = self.hit_test(p, tol) {
                    if let Some(Op::Offset { ent, .. }) = &mut self.op {
                        *ent = Some(i);
                    }
                    return self.result(true, "", "Specify which side to offset:");
                }
                self.result(false, "No object there.", "Select an object to offset:")
            }
            Some(_) => self.point_input(p),
            None => {
                match self.hit_test(p, tol) {
                    Some(i) => {
                        if let Some(pos) = self.selected.iter().position(|&x| x == i) {
                            self.selected.remove(pos);
                        } else {
                            self.selected.push(i);
                        }
                    }
                    None => self.selected.clear(),
                }
                self.result(true, &format!("{} selected", self.selected.len()), "")
            }
        }
    }

    pub fn cancel(&mut self) -> CmdResult {
        self.op = None;
        self.result(true, "*Cancel*", "")
    }

    pub fn snapshot_result(&self) -> CmdResult {
        self.result(true, "", "")
    }

    // ---- helpers ----

    fn last_point(&self) -> Option<Vec2> {
        match &self.op {
            Some(Op::Draw { pts, .. }) => pts.last().copied(),
            Some(Op::Move { base, .. }) | Some(Op::Rotate { base }) | Some(Op::Scale { base }) => *base,
            Some(Op::Mirror { first }) => *first,
            _ => None,
        }
    }

    fn hit_test(&self, p: Vec2, tol: f64) -> Option<usize> {
        let mut best = None;
        let mut bd = tol.max(0.05);
        for (i, d) in self.doc.dobjects.iter().enumerate() {
            let dist = d.distance_to_point(p);
            if dist < bd {
                bd = dist;
                best = Some(i);
            }
        }
        best
    }

    fn start_draw(&mut self, tool: ToolKind, thickness: f64) -> CmdResult {
        self.op = Some(Op::Draw { tool, pts: Vec::new(), thickness });
        let prompt = match tool {
            ToolKind::Circle => "Specify centre point:",
            ToolKind::Rectangle => "Specify first corner:",
            _ => "Specify first point:",
        };
        self.result(true, &format!("{}: ", tool_word(tool)), prompt)
    }

    fn start_modify(&mut self, op: Op, prompt: &str) -> CmdResult {
        if self.selected.is_empty() {
            return self.result(false, "Select objects first (click them), then run the command.", "");
        }
        self.op = Some(op);
        self.result(true, "", prompt)
    }

    // ---- point routing ----

    fn point_input(&mut self, p: Vec2) -> CmdResult {
        match self.op.take() {
            Some(Op::Draw { tool, pts, thickness }) => {
                self.op = Some(Op::Draw { tool, pts, thickness });
                self.draw_point(p)
            }
            Some(Op::Move { copy, base }) => match base {
                None => {
                    self.op = Some(Op::Move { copy, base: Some(p) });
                    self.result(true, "", "Specify destination point:")
                }
                Some(b) => {
                    self.apply_move(p - b, copy);
                    self.result(true, if copy { "Copied." } else { "Moved." }, "")
                }
            },
            Some(Op::Rotate { base }) => match base {
                None => {
                    self.op = Some(Op::Rotate { base: Some(p) });
                    self.result(true, "", "Specify rotation angle (number) or a point:")
                }
                Some(b) => {
                    self.apply_rotate(b, (p.y - b.y).atan2(p.x - b.x));
                    self.result(true, "Rotated.", "")
                }
            },
            Some(Op::Scale { base }) => match base {
                None => {
                    self.op = Some(Op::Scale { base: Some(p) });
                    self.result(true, "", "Specify scale factor (number) or a point:")
                }
                Some(b) => {
                    let f = (p - b).len();
                    if f > 1e-9 {
                        self.apply_scale(b, f);
                    }
                    self.result(true, "Scaled.", "")
                }
            },
            Some(Op::Mirror { first }) => match first {
                None => {
                    self.op = Some(Op::Mirror { first: Some(p) });
                    self.result(true, "", "Specify second point of the mirror line:")
                }
                Some(a) => {
                    self.apply_mirror(a, p);
                    self.result(true, "Mirrored.", "")
                }
            },
            Some(Op::Offset { dist, ent }) => match ent {
                Some(i) => {
                    self.apply_offset(i, dist, p);
                    self.result(true, "Offset.", "")
                }
                None => {
                    self.op = Some(Op::Offset { dist, ent: None });
                    self.result(false, "Click the object to offset.", "Select an object to offset:")
                }
            },
            other => {
                self.op = other;
                self.result(false, "Click an object.", "")
            }
        }
    }

    fn draw_point(&mut self, p: Vec2) -> CmdResult {
        let (tool, n) = match &mut self.op {
            Some(Op::Draw { tool, pts, .. }) => {
                pts.push(p);
                (*tool, pts.len())
            }
            _ => return self.result(false, "", ""),
        };
        let pts: Vec<Vec2> = match &self.op {
            Some(Op::Draw { pts, .. }) => pts.clone(),
            _ => Vec::new(),
        };
        match tool {
            ToolKind::Point => {
                self.doc.push(DObject::new(Geom::Point(KPoint { location: pts[0], style: 0, size: 0.0 })));
                self.op = None;
                self.result(true, "Point placed.", "")
            }
            ToolKind::Rectangle if n >= 2 => {
                let (c0, c1) = (pts[0], pts[1]);
                let verts = [c0, Vec2::new(c1.x, c0.y), c1, Vec2::new(c0.x, c1.y)]
                    .iter()
                    .map(|&pos| PolyVertex { pos, bulge: 0.0 })
                    .collect();
                self.doc.push(DObject::new(Geom::Polyline(KPoly { vertices: verts, closed: true, widths: Vec::new() })));
                self.op = None;
                self.result(true, "Rectangle added.", "")
            }
            ToolKind::Rectangle => self.result(true, "", "Specify opposite corner:"),
            ToolKind::Circle if n >= 2 => {
                let r = (pts[1] - pts[0]).len();
                self.doc.push(DObject::new(Geom::Circle(KCircle { center: pts[0], radius: r })));
                self.op = None;
                self.result(true, "Circle added.", "")
            }
            ToolKind::Circle => self.result(true, "", "Specify radius (a point on the circle):"),
            ToolKind::Arc if n >= 3 => {
                if let Some(arc) = arc_three_points(pts[0], pts[1], pts[2]) {
                    self.doc.push(DObject::new(Geom::Arc(arc)));
                }
                self.op = None;
                self.result(true, "Arc added.", "")
            }
            ToolKind::Arc => self.result(true, "", if n == 1 { "Specify second point:" } else { "Specify end point:" }),
            _ => self.result(true, "", "Specify next point or [Close/Undo]:"),
        }
    }

    fn finish(&mut self, closed: bool) -> CmdResult {
        if let Some(Op::Draw { tool, pts, thickness }) = self.op.take() {
            if pts.len() >= 2 {
                let mut segs: Vec<(Vec2, Vec2)> = pts.windows(2).map(|w| (w[0], w[1])).collect();
                if closed {
                    segs.push((pts[pts.len() - 1], pts[0]));
                }
                match tool {
                    ToolKind::Wall => {
                        for (a, b) in segs {
                            self.doc.push(DObject::new(Geom::Wall(KWall { start: a, end: b, thickness, style: 0, bulge: 0.0 })));
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
        }
        self.result(true, if closed { "Closed." } else { "Done." }, "")
    }

    // ---- modify application (reusing cad_kernel edit ops) ----

    fn apply_move(&mut self, off: Vec2, copy: bool) {
        let sel = self.selected.clone();
        if copy {
            let news: Vec<DObject> = sel.iter().filter_map(|&i| self.doc.dobjects.get(i)).map(|d| d.translated(off)).collect();
            self.doc.dobjects.extend(news);
        } else {
            for &i in &sel {
                if let Some(d) = self.doc.dobjects.get(i) {
                    self.doc.dobjects[i] = d.translated(off);
                }
            }
        }
    }

    fn apply_rotate(&mut self, pivot: Vec2, angle: f64) {
        let sel = self.selected.clone();
        for &i in &sel {
            if let Some(d) = self.doc.dobjects.get(i) {
                self.doc.dobjects[i] = d.rotated(pivot, angle);
            }
        }
    }

    fn apply_scale(&mut self, pivot: Vec2, factor: f64) {
        let sel = self.selected.clone();
        for &i in &sel {
            if let Some(d) = self.doc.dobjects.get(i) {
                self.doc.dobjects[i] = d.scaled(pivot, factor);
            }
        }
    }

    fn apply_mirror(&mut self, a: Vec2, b: Vec2) {
        let sel = self.selected.clone();
        let news: Vec<DObject> = sel.iter().filter_map(|&i| self.doc.dobjects.get(i)).map(|d| d.mirrored(a, b)).collect();
        self.doc.dobjects.extend(news);
    }

    fn apply_offset(&mut self, i: usize, dist: f64, side: Vec2) {
        if let Some(d) = self.doc.dobjects.get(i) {
            if let Ok(g) = d.geom.offset(dist, side) {
                self.doc.dobjects.push(DObject::new(g));
            }
        }
    }

    fn apply_trim(&mut self, i: usize, pick: Vec2) {
        let cutters: Vec<Geom> = self
            .doc
            .dobjects
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, d)| d.geom.clone())
            .collect();
        let survivors = match self.doc.dobjects[i].geom.trim_at(&cutters, pick, false) {
            Ok(s) => s,
            Err(_) => return,
        };
        self.doc.dobjects.remove(i);
        self.selected.clear();
        for g in survivors {
            self.doc.dobjects.push(DObject::new(g));
        }
    }

    fn apply_extend(&mut self, i: usize, pick: Vec2) {
        let boundaries: Vec<Geom> = self
            .doc
            .dobjects
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, d)| d.geom.clone())
            .collect();
        if let Ok(g) = self.doc.dobjects[i].geom.extend_to(&boundaries, pick, false) {
            self.doc.dobjects[i].geom = g;
        }
    }

    fn erase(&mut self) -> usize {
        let mut sel = self.selected.clone();
        sel.sort_unstable();
        sel.dedup();
        for &i in sel.iter().rev() {
            if i < self.doc.dobjects.len() {
                self.doc.dobjects.remove(i);
            }
        }
        self.selected.clear();
        sel.len()
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

// ---- extrusion: Document geometry → surfaces (one line = one surface) ----

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
        assert!(d.exec("line 0,0 3,0").ok);
        assert_eq!(d.doc.dobjects.len(), 1);
    }

    #[test]
    fn interactive_rectangle_and_extrude() {
        let mut d = Draft::default();
        d.exec("rectangle");
        d.exec("0,0");
        d.exec("4,3");
        assert_eq!(d.doc.dobjects.len(), 1);
        assert_eq!(extrude(&d.doc, 3.0).len(), 6);
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
        for p in ["0,0", "4,0", "4,4", "0,4"] {
            d.exec(p);
        }
        d.exec("close");
        assert_eq!(d.doc.dobjects.len(), 4);
    }

    #[test]
    fn select_and_erase() {
        let mut d = Draft::default();
        d.exec("line 0,0 3,0");
        d.exec("line 0,1 3,1");
        assert_eq!(d.doc.dobjects.len(), 2);
        d.click(0.0, 0.0, 0.2); // select first line at its endpoint
        assert_eq!(d.selected.len(), 1);
        d.exec("erase");
        assert_eq!(d.doc.dobjects.len(), 1);
    }

    #[test]
    fn move_selection() {
        let mut d = Draft::default();
        d.exec("line 0,0 2,0");
        d.click(0.0, 0.0, 0.2);
        d.exec("move");
        d.exec("0,0");
        d.exec("0,5"); // move up 5
        if let Geom::Line(l) = &d.doc.dobjects[0].geom {
            assert!((l.a.y - 5.0).abs() < 1e-6);
        } else {
            panic!("expected a line");
        }
    }

    #[test]
    fn trim_line_at_crossing() {
        let mut d = Draft::default();
        d.exec("line 0,0 4,0"); // horizontal
        d.exec("line 2,-1 2,1"); // vertical cutter crossing at (2,0)
        // Trim the right half of the horizontal line (pick at x=3).
        d.exec("trim");
        let before = d.doc.dobjects.len();
        d.click(3.0, 0.0, 0.2);
        assert!(d.doc.dobjects.len() >= before - 1 + 1); // trimmed piece(s) present
        // The surviving horizontal piece should no longer reach x=4.
        let reaches_4 = d.doc.dobjects.iter().any(|o| match &o.geom {
            Geom::Line(l) => (l.a.x - 4.0).abs() < 1e-6 || (l.b.x - 4.0).abs() < 1e-6,
            _ => false,
        });
        assert!(!reaches_4, "right half should have been trimmed");
    }
}
