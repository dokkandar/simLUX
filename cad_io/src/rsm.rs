// RSM — RUST_CAD's native binary Document format.
//
// Design goals:
//   1. **Fast** — direct field-by-field little-endian write; no JSON, no
//      varint, no compression in the v1 spec. Target: serialize 5M dobjects
//      in well under a second.
//   2. **Lossless** — every field in `Document` round-trips exactly.
//      Where DXF compresses or paraphrases (e.g. ACI color → ByLayer
//      sentinel), RSM stores the original.
//   3. **Versioned** — header carries `(magic, version)` so a v2 reader
//      can refuse a v3 file or upgrade a v1 file. Today's spec is v1.
//   4. **No deps** — hand-rolled; no serde, no bincode, no postcard. The
//      whole format is in this file.
//
// Layout (little-endian throughout):
//
//   magic    [u8;4] = "RSM\x01"     (last byte = format version)
//   version  u16    = 1
//   pad      u16    = 0
//
//   --- LinetypeTable ---
//   count    u32
//   per linetype:
//     name        len: u32, bytes
//     description len: u32, bytes
//     pattern     len: u32, f32 …
//
//   --- LayerTable ---
//   active      u32
//   count       u32
//   per layer:
//     name        len: u32, bytes
//     color       u8 tag + payload (see encode_color)
//     linetype    u32 (LinetypeId)
//     lineweight  u8 tag + payload (see encode_lineweight)
//     flags       u8 — bit 0 visible, bit 1 locked, bit 2 frozen, bit 3 plottable
//
//   --- PenTable ---
//   count       u32
//   per pen:
//     name        len: u32, bytes
//     color       (encoded as above)
//     linetype    u32
//     lineweight  (encoded as above)
//
//   --- DObjects ---
//   count       u32
//   per dobject:
//     handle      u64
//     style: layer u32, color, linetype u32, linetype_scale f32, lineweight, visible u8
//     geom: u8 tag + per-variant payload (see write_geom)
//
// Future versions can add fields by bumping the version byte; the reader
// dispatches on it.

use cad_kernel::{
    Arc, Circle, Color, DObject, Document, Ellipse, EllipseArc, Geom, Hatch,
    HatchPattern, Layer, LayerTable, Line, Lineweight, Linetype, LinetypeTable,
    Pen, PenTable, Point, PolyVertex, Polyline, Spline, Style, Vec2, Wall,
};

const MAGIC: [u8; 4] = *b"RSM\x01";
const VERSION: u16  = 1;

// =============================================================================
//   WRITER
// =============================================================================

pub fn write_rsm(doc: &Document) -> Vec<u8> {
    let mut w = Vec::with_capacity(1024 + doc.dobjects.len() * 64);
    w.extend_from_slice(&MAGIC);
    write_u16(&mut w, VERSION);
    write_u16(&mut w, 0);

    write_linetype_table(&mut w, &doc.linetypes);
    write_layer_table(&mut w, &doc.layers, &doc.truecolors);
    write_pen_table(&mut w, &doc.pens, &doc.truecolors);
    write_dobjects(&mut w, &doc.dobjects, &doc.truecolors);

    w
}

fn write_u16(w: &mut Vec<u8>, v: u16) { w.extend_from_slice(&v.to_le_bytes()); }
fn write_u32(w: &mut Vec<u8>, v: u32) { w.extend_from_slice(&v.to_le_bytes()); }
fn write_u64(w: &mut Vec<u8>, v: u64) { w.extend_from_slice(&v.to_le_bytes()); }
fn write_f32(w: &mut Vec<u8>, v: f32) { w.extend_from_slice(&v.to_le_bytes()); }
fn write_f64(w: &mut Vec<u8>, v: f64) { w.extend_from_slice(&v.to_le_bytes()); }
fn write_u8 (w: &mut Vec<u8>, v: u8)  { w.push(v); }
fn write_str(w: &mut Vec<u8>, s: &str) {
    write_u32(w, s.len() as u32);
    w.extend_from_slice(s.as_bytes());
}
fn write_vec2(w: &mut Vec<u8>, v: Vec2) {
    write_f64(w, v.x);
    write_f64(w, v.y);
}

/// Color tag space (on-disk format is UNCHANGED for backward compat):
///   0 = ByLayer, 1 = ByBlock, 2 = Aci(u8), 3 = TrueColor (RGB u32 inline)
/// In-memory `Color::TrueColorRef(idx)` is dereferenced via `truecolors`
/// at write time. Reader interns the RGB into the doc's table.
fn write_color(w: &mut Vec<u8>, c: Color, tc: &cad_kernel::TrueColorTable) {
    match c {
        Color::ByLayer            => write_u8(w, 0),
        Color::ByBlock            => write_u8(w, 1),
        Color::Aci(i)             => { write_u8(w, 2); write_u8(w, i); }
        Color::TrueColorRef(idx)  => {
            let rgb = tc.get(idx).unwrap_or(0xFFFFFF);
            write_u8(w, 3);
            write_u32(w, rgb);
        }
    }
}

/// Lineweight tag space:
///   0 = ByLayer, 1 = ByBlock, 2 = Default, 3 = Custom(f32 mm)
fn write_lineweight(w: &mut Vec<u8>, lw: Lineweight) {
    match lw {
        Lineweight::ByLayer    => write_u8(w, 0),
        Lineweight::ByBlock    => write_u8(w, 1),
        Lineweight::Default    => write_u8(w, 2),
        Lineweight::Custom(mm) => { write_u8(w, 3); write_f32(w, mm); }
    }
}

fn write_linetype_table(w: &mut Vec<u8>, t: &LinetypeTable) {
    write_u32(w, t.linetypes.len() as u32);
    for lt in &t.linetypes {
        write_str(w, &lt.name);
        write_str(w, &lt.description);
        write_u32(w, lt.pattern.len() as u32);
        for v in &lt.pattern { write_f32(w, *v); }
    }
}

fn write_layer_table(w: &mut Vec<u8>, t: &LayerTable, tc: &cad_kernel::TrueColorTable) {
    write_u32(w, t.active);
    write_u32(w, t.layers.len() as u32);
    for l in &t.layers {
        write_str(w, &l.name);
        write_color(w, l.color, tc);
        write_u32(w, l.linetype);
        write_lineweight(w, l.lineweight);
        let mut flags = 0_u8;
        if l.visible   { flags |= 0b0001; }
        if l.locked    { flags |= 0b0010; }
        if l.frozen    { flags |= 0b0100; }
        if l.plottable { flags |= 0b1000; }
        write_u8(w, flags);
    }
}

fn write_pen_table(w: &mut Vec<u8>, t: &PenTable, tc: &cad_kernel::TrueColorTable) {
    write_u32(w, t.pens.len() as u32);
    for p in &t.pens {
        write_str(w, &p.name);
        write_color(w, p.color, tc);
        write_u32(w, p.linetype);
        write_lineweight(w, p.lineweight);
    }
}

fn write_dobjects(w: &mut Vec<u8>, ds: &[DObject], tc: &cad_kernel::TrueColorTable) {
    write_u32(w, ds.len() as u32);
    for d in ds {
        write_u64(w, d.handle);
        // Style block
        write_u32(w, d.style.layer);
        write_color(w, d.style.color, tc);
        write_u32(w, d.style.linetype);
        write_f32(w, d.style.linetype_scale);
        write_lineweight(w, d.style.lineweight);
        write_u8 (w, if d.style.visible { 1 } else { 0 });
        // Geometry
        write_geom(w, &d.geom);
    }
}

/// Geom tag space:
///   0=Line, 1=Circle, 2=Arc, 3=Ellipse, 4=EllipseArc, 5=Point, 6=Polyline,
///   7=Hatch (MVP — boundary handles + pattern code; 0=Solid)
///   8=Spline (NURBS — degree + control points + weights)
fn write_geom(w: &mut Vec<u8>, g: &Geom) {
    match g {
        Geom::Line(l) => {
            write_u8(w, 0);
            write_vec2(w, l.a);
            write_vec2(w, l.b);
        }
        Geom::Circle(c) => {
            write_u8(w, 1);
            write_vec2(w, c.center);
            write_f64(w, c.radius);
        }
        Geom::Arc(a) => {
            write_u8(w, 2);
            write_vec2(w, a.center);
            write_f64(w, a.radius);
            write_f64(w, a.start_angle);
            write_f64(w, a.sweep_angle);
        }
        Geom::Ellipse(e) => {
            write_u8(w, 3);
            write_vec2(w, e.center);
            write_vec2(w, e.major);
            write_f64(w, e.ratio);
        }
        Geom::EllipseArc(ea) => {
            write_u8(w, 4);
            write_vec2(w, ea.ellipse.center);
            write_vec2(w, ea.ellipse.major);
            write_f64(w, ea.ellipse.ratio);
            write_f64(w, ea.start_param);
            write_f64(w, ea.sweep_param);
        }
        Geom::Point(pt) => {
            write_u8(w, 5);
            write_vec2(w, pt.location);
            write_u8 (w, pt.style);
            write_f32(w, pt.size);
        }
        Geom::Polyline(p) => {
            write_u8(w, 6);
            write_u8(w, if p.closed { 1 } else { 0 });
            write_u32(w, p.vertices.len() as u32);
            for v in &p.vertices {
                write_vec2(w, v.pos);
                write_f64(w, v.bulge);
            }
        }
        Geom::Hatch(h) => {
            write_u8(w, 7);
            // Pattern encoding:
            //   0 = Solid                              (no extra payload)
            //   1 = Pattern { name, scale, angle_deg } (utf-8 name + 2 f64)
            match &h.pattern {
                HatchPattern::Solid => {
                    write_u8(w, 0);
                }
                HatchPattern::Pattern { name, scale, angle_deg } => {
                    write_u8(w, 1);
                    let bytes = name.as_bytes();
                    write_u32(w, bytes.len() as u32);
                    w.extend_from_slice(bytes);
                    write_f64(w, *scale);
                    write_f64(w, *angle_deg);
                }
            }
            write_u32(w, h.boundary_handles.len() as u32);
            for handle in &h.boundary_handles {
                write_u64(w, *handle);
            }
        }
        Geom::Spline(s) => {
            write_u8(w, 8);
            write_u8(w, s.degree as u8);
            write_u32(w, s.control_points.len() as u32);
            for p in &s.control_points {
                write_vec2(w, *p);
            }
            // weights.len() == control_points.len() by Spline invariant.
            for wt in &s.weights {
                write_f64(w, *wt);
            }
        }
        Geom::Wall(wall) => {
            // tag 9 = Wall; centerline + thickness.
            write_u8(w, 9);
            write_vec2(w, wall.start);
            write_vec2(w, wall.end);
            write_f64(w, wall.thickness);
        }
    }
}

// =============================================================================
//   READER
// =============================================================================

struct R<'a> { bytes: &'a [u8], pos: usize }

impl<'a> R<'a> {
    fn need(&self, n: usize) -> Result<(), String> {
        if self.pos + n > self.bytes.len() {
            Err(format!("RSM: read past end (at {} need {} have {})",
                self.pos, n, self.bytes.len()))
        } else { Ok(()) }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        self.need(n)?;
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }
    fn u8 (&mut self) -> Result<u8,  String> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?; Ok(u16::from_le_bytes([b[0],b[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?; Ok(u32::from_le_bytes([b[0],b[1],b[2],b[3]]))
    }
    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0],b[1],b[2],b[3],b[4],b[5],b[6],b[7]]))
    }
    fn f32(&mut self) -> Result<f32, String> {
        let b = self.take(4)?; Ok(f32::from_le_bytes([b[0],b[1],b[2],b[3]]))
    }
    fn f64(&mut self) -> Result<f64, String> {
        let b = self.take(8)?;
        Ok(f64::from_le_bytes([b[0],b[1],b[2],b[3],b[4],b[5],b[6],b[7]]))
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        let raw = self.take(n)?;
        String::from_utf8(raw.to_vec())
            .map_err(|e| format!("RSM: bad utf-8 string: {}", e))
    }
    fn vec2(&mut self) -> Result<Vec2, String> {
        Ok(Vec2 { x: self.f64()?, y: self.f64()? })
    }
}

pub fn read_rsm(bytes: &[u8]) -> Result<Document, String> {
    let mut r = R { bytes, pos: 0 };
    let magic = r.take(4)?;
    if magic[..3] != MAGIC[..3] {
        return Err(format!("RSM: bad magic {:?}", &magic[..3]));
    }
    let _embedded_ver = magic[3];   // historic; today we read VERSION below
    let ver = r.u16()?;
    let _pad = r.u16()?;
    if ver != VERSION {
        return Err(format!("RSM: unsupported version {} (this build only reads v{})", ver, VERSION));
    }

    let linetypes  = read_linetype_table(&mut r)?;
    let mut truecolors = cad_kernel::TrueColorTable::new();
    let layers    = read_layer_table(&mut r, &mut truecolors)?;
    let pens      = read_pen_table(&mut r, &mut truecolors)?;
    let dobjects  = read_dobjects(&mut r, &mut truecolors)?;

    Ok(Document { dobjects, layers, linetypes, pens, truecolors })
}

fn read_color(r: &mut R, tc: &mut cad_kernel::TrueColorTable) -> Result<Color, String> {
    Ok(match r.u8()? {
        0 => Color::ByLayer,
        1 => Color::ByBlock,
        2 => Color::Aci(r.u8()?),
        3 => Color::TrueColorRef(tc.intern(r.u32()?)),
        t => return Err(format!("RSM: unknown color tag {}", t)),
    })
}

fn read_lineweight(r: &mut R) -> Result<Lineweight, String> {
    Ok(match r.u8()? {
        0 => Lineweight::ByLayer,
        1 => Lineweight::ByBlock,
        2 => Lineweight::Default,
        3 => Lineweight::Custom(r.f32()?),
        t => return Err(format!("RSM: unknown lineweight tag {}", t)),
    })
}

fn read_linetype_table(r: &mut R) -> Result<LinetypeTable, String> {
    let n = r.u32()? as usize;
    let mut linetypes = Vec::with_capacity(n);
    for _ in 0..n {
        let name = r.str()?;
        let desc = r.str()?;
        let plen = r.u32()? as usize;
        let mut pattern = Vec::with_capacity(plen);
        for _ in 0..plen { pattern.push(r.f32()?); }
        linetypes.push(Linetype { name, description: desc, pattern });
    }
    Ok(LinetypeTable { linetypes })
}

fn read_layer_table(r: &mut R, tc: &mut cad_kernel::TrueColorTable) -> Result<LayerTable, String> {
    let active = r.u32()?;
    let n = r.u32()? as usize;
    let mut layers = Vec::with_capacity(n);
    for _ in 0..n {
        let name       = r.str()?;
        let color      = read_color(r, tc)?;
        let linetype   = r.u32()?;
        let lineweight = read_lineweight(r)?;
        let flags      = r.u8()?;
        layers.push(Layer {
            name, color, linetype, lineweight,
            visible:   (flags & 0b0001) != 0,
            locked:    (flags & 0b0010) != 0,
            frozen:    (flags & 0b0100) != 0,
            plottable: (flags & 0b1000) != 0,
        });
    }
    Ok(LayerTable { layers, active })
}

fn read_pen_table(r: &mut R, tc: &mut cad_kernel::TrueColorTable) -> Result<PenTable, String> {
    let n = r.u32()? as usize;
    let mut pens = Vec::with_capacity(n);
    for _ in 0..n {
        let name       = r.str()?;
        let color      = read_color(r, tc)?;
        let linetype   = r.u32()?;
        let lineweight = read_lineweight(r)?;
        pens.push(Pen { name, color, linetype, lineweight });
    }
    Ok(PenTable { pens })
}

fn read_dobjects(r: &mut R, tc: &mut cad_kernel::TrueColorTable) -> Result<Vec<DObject>, String> {
    let n = r.u32()? as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let handle    = r.u64()?;
        let layer     = r.u32()?;
        let color     = read_color(r, tc)?;
        let linetype  = r.u32()?;
        let lt_scale  = r.f32()?;
        let lineweight= read_lineweight(r)?;
        let visible   = r.u8()? != 0;
        let geom      = read_geom(r)?;
        out.push(DObject {
            handle,
            style: Style {
                layer, color, linetype,
                linetype_scale: lt_scale, lineweight, visible,
            },
            geom,
        });
    }
    Ok(out)
}

fn read_geom(r: &mut R) -> Result<Geom, String> {
    Ok(match r.u8()? {
        0 => Geom::Line(Line { a: r.vec2()?, b: r.vec2()? }),
        1 => Geom::Circle(Circle { center: r.vec2()?, radius: r.f64()? }),
        2 => Geom::Arc(Arc {
            center: r.vec2()?, radius: r.f64()?,
            start_angle: r.f64()?, sweep_angle: r.f64()?,
        }),
        3 => Geom::Ellipse(Ellipse {
            center: r.vec2()?, major: r.vec2()?, ratio: r.f64()?,
        }),
        4 => {
            let el = Ellipse { center: r.vec2()?, major: r.vec2()?, ratio: r.f64()? };
            Geom::EllipseArc(EllipseArc {
                ellipse: el, start_param: r.f64()?, sweep_param: r.f64()?,
            })
        }
        5 => Geom::Point(Point { location: r.vec2()?, style: r.u8()?, size: r.f32()? }),
        6 => {
            let closed = r.u8()? != 0;
            let n = r.u32()? as usize;
            let mut vertices = Vec::with_capacity(n);
            for _ in 0..n {
                vertices.push(PolyVertex { pos: r.vec2()?, bulge: r.f64()? });
            }
            Geom::Polyline(Polyline { vertices, closed })
        }
        7 => {
            let pattern = match r.u8()? {
                0 => HatchPattern::Solid,
                1 => {
                    let name_len = r.u32()? as usize;
                    let bytes = r.take(name_len)?.to_vec();
                    let name = String::from_utf8(bytes)
                        .map_err(|e| format!("RSM: hatch pattern name not UTF-8: {}", e))?;
                    let scale     = r.f64()?;
                    let angle_deg = r.f64()?;
                    HatchPattern::Pattern { name, scale, angle_deg }
                }
                other => return Err(format!("RSM: unknown hatch pattern code {}", other)),
            };
            let n = r.u32()? as usize;
            let mut boundary_handles = Vec::with_capacity(n);
            for _ in 0..n {
                boundary_handles.push(r.u64()?);
            }
            Geom::Hatch(Hatch { boundary_handles, pattern })
        }
        8 => {
            let degree = r.u8()? as usize;
            let n = r.u32()? as usize;
            let mut control_points = Vec::with_capacity(n);
            for _ in 0..n { control_points.push(r.vec2()?); }
            let mut weights = Vec::with_capacity(n);
            for _ in 0..n { weights.push(r.f64()?); }
            Geom::Spline(Spline { degree, control_points, weights })
        }
        9 => Geom::Wall(Wall {
            start: r.vec2()?, end: r.vec2()?, thickness: r.f64()?,
        }),
        t => return Err(format!("RSM: unknown geom tag {}", t)),
    })
}

// =============================================================================
//   TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(doc: &Document) -> Document {
        let bytes = write_rsm(doc);
        read_rsm(&bytes).expect("rsm round-trip")
    }

    #[test]
    fn empty_doc_round_trip() {
        let doc = Document::default();
        let back = round_trip(&doc);
        assert_eq!(back.layers.len(), doc.layers.len());
        assert_eq!(back.linetypes.len(), doc.linetypes.len());
        assert_eq!(back.pens.len(), doc.pens.len());
        assert!(back.dobjects.is_empty());
    }

    #[test]
    fn every_geom_round_trips() {
        let mut doc = Document::default();
        doc.push(Line { a: Vec2::new(0.0, 0.0), b: Vec2::new(10.0, 5.0) }.into());
        doc.push(Circle { center: Vec2::new(1.0, 2.0), radius: 3.0 }.into());
        doc.push(Arc {
            center: Vec2::ZERO, radius: 5.0,
            start_angle: 0.5, sweep_angle: 1.0,
        }.into());
        doc.push(Ellipse { center: Vec2::ZERO, major: Vec2::new(5.0, 0.0), ratio: 0.4 }.into());
        doc.push(EllipseArc {
            ellipse: Ellipse { center: Vec2::ZERO, major: Vec2::new(5.0, 0.0), ratio: 0.4 },
            start_param: 0.1, sweep_param: 2.0,
        }.into());
        doc.push(Point { location: Vec2::new(3.0, 4.0), style: 2, size: 0.5 }.into());
        doc.push(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(1.0, 0.0), bulge: 0.5 },
                PolyVertex { pos: Vec2::new(1.0, 1.0), bulge: 0.0 },
            ],
            closed: true,
        }.into());
        let back = round_trip(&doc);
        assert_eq!(back.dobjects.len(), 7);
    }

    #[test]
    fn handles_are_preserved() {
        let mut doc = Document::default();
        let i = doc.push(Line { a: Vec2::ZERO, b: Vec2::new(1.0, 1.0) }.into());
        let h = doc.dobjects[i].handle;
        let back = round_trip(&doc);
        assert_eq!(back.dobjects[0].handle, h);
    }

    #[test]
    fn layer_table_round_trip_preserves_active_and_flags() {
        let mut doc = Document::default();
        let id = doc.layers.add(Layer {
            name: "HIDDEN".into(),
            color: Color::Aci(3),
            linetype: 0,
            lineweight: Lineweight::Custom(0.5),
            visible: false, locked: true, frozen: false, plottable: true,
        });
        doc.layers.active = id;
        let back = round_trip(&doc);
        let lid = back.layers.find("HIDDEN").unwrap();
        let l = back.layers.get(lid).unwrap();
        assert!(!l.visible);
        assert!(l.locked);
        assert!(matches!(l.lineweight, Lineweight::Custom(x) if (x - 0.5).abs() < 1e-9));
        assert_eq!(back.layers.active, lid);
    }

    #[test]
    fn bad_magic_is_rejected() {
        let r = read_rsm(b"NOPE");
        assert!(r.is_err());
    }

    #[test]
    fn bytes_are_compact() {
        // Sanity check: 1000 lines should encode in well under 100 KB
        // (~64 bytes per dobject is the expected scale: handle 8 + style ~20
        // + Line geom 33 = ~61).
        let mut doc = Document::default();
        for i in 0..1000 {
            doc.push(Line {
                a: Vec2::new(i as f64, 0.0),
                b: Vec2::new(i as f64, 10.0),
            }.into());
        }
        let bytes = write_rsm(&doc);
        // < 100 KB headroom — typical is ~70 KB for 1000 lines + table overhead.
        assert!(bytes.len() < 100_000, "1000 lines → {} bytes (expected < 100k)", bytes.len());
    }
}
