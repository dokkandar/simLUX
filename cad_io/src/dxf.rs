// DXF ASCII reader + writer.
//
// DXF format: alternating lines of (group code, value). Group codes are
// integers; values are strings/ints/floats depending on the code's type.
// Files are organized into SECTIONs (HEADER / TABLES / BLOCKS / ENTITIES
// / OBJECTS), each delimited by `0\nSECTION` ... `0\nENDSEC`.
//
// We implement a minimal subset sufficient to round-trip RUST_CAD's seven
// Geom variants plus layer + linetype tables. AutoCAD-specific niceties
// (handles, extrusion vectors, true-color, lineweight enum, dimstyles,
// blocks) are skipped on read (silently) and emitted on write only when
// the data exists. Files written by RUST_CAD open cleanly in LibreCAD
// and AutoCAD.

use cad_kernel::{
    Arc, Circle, Color, DObject, Document, Ellipse, EllipseArc, Geom, Layer,
    Line, Lineweight, Linetype, LinetypeTable, Point, PolyVertex,
    Polyline, Vec2,
};

// ============================================================================
//   READER
// ============================================================================

/// Parse DXF ASCII text into a fresh `Document`. Errors only on
/// fundamentally unreadable input (broken line pairs); unknown entities
/// and unknown group codes are silently skipped.
pub fn read_dxf(text: &str) -> Result<Document, String> {
    let pairs = parse_pairs(text)?;
    let mut doc = Document::default();
    let mut i = 0;
    while i < pairs.len() {
        let (code, value) = &pairs[i];
        if *code == 0 && value == "SECTION" && i + 1 < pairs.len() {
            let (c2, name) = &pairs[i + 1];
            if *c2 == 2 {
                match name.as_str() {
                    "TABLES"   => i = read_tables(&pairs, i + 2, &mut doc),
                    "ENTITIES" => i = read_entities(&pairs, i + 2, &mut doc),
                    _ => i = skip_to_endsec(&pairs, i + 2),
                }
                continue;
            }
        }
        i += 1;
    }
    Ok(doc)
}

/// Tokenize the source into (code, value) pairs. DXF files use CRLF or
/// LF; we tolerate either. The line *after* the code line is the value;
/// trailing whitespace is trimmed.
fn parse_pairs(text: &str) -> Result<Vec<(i32, String)>, String> {
    let mut lines = text.lines();
    let mut out = Vec::new();
    while let Some(code_line) = lines.next() {
        let code_str = code_line.trim();
        if code_str.is_empty() { continue; }
        let value_line = lines.next()
            .ok_or_else(|| "DXF: code line without value line".to_string())?;
        let code: i32 = code_str.parse()
            .map_err(|_| format!("DXF: bad group code '{}'", code_str))?;
        out.push((code, value_line.trim().to_string()));
    }
    Ok(out)
}

fn skip_to_endsec(pairs: &[(i32, String)], start: usize) -> usize {
    let mut i = start;
    while i < pairs.len() {
        let (c, v) = &pairs[i];
        if *c == 0 && v == "ENDSEC" { return i + 1; }
        i += 1;
    }
    pairs.len()
}

fn read_tables(pairs: &[(i32, String)], start: usize, doc: &mut Document) -> usize {
    let mut i = start;
    while i < pairs.len() {
        let (c, v) = &pairs[i];
        if *c == 0 && v == "ENDSEC" { return i + 1; }
        if *c == 0 && v == "TABLE" && i + 1 < pairs.len() {
            let (c2, name) = &pairs[i + 1];
            if *c2 == 2 {
                match name.as_str() {
                    "LAYER" => i = read_layer_table(pairs, i + 2, doc),
                    "LTYPE" => i = read_ltype_table(pairs, i + 2, doc),
                    _       => i = skip_to_endtab(pairs, i + 2),
                }
                continue;
            }
        }
        i += 1;
    }
    pairs.len()
}

fn skip_to_endtab(pairs: &[(i32, String)], start: usize) -> usize {
    let mut i = start;
    while i < pairs.len() {
        let (c, v) = &pairs[i];
        if *c == 0 && v == "ENDTAB" { return i + 1; }
        i += 1;
    }
    pairs.len()
}

fn read_layer_table(pairs: &[(i32, String)], start: usize, doc: &mut Document) -> usize {
    let mut i = start;
    while i < pairs.len() {
        let (c, v) = &pairs[i];
        if *c == 0 && v == "ENDTAB" { return i + 1; }
        if *c == 0 && v == "LAYER" {
            // Accumulate this layer's fields until the next 0-group.
            let mut name = String::new();
            let mut color = Color::Aci(7);   // ACI 7 = white default
            let mut lt_name = String::from("Continuous");
            let mut flags: i32 = 0;
            i += 1;
            while i < pairs.len() && pairs[i].0 != 0 {
                match pairs[i].0 {
                    2  => name    = pairs[i].1.clone(),
                    62 => {
                        let aci: i32 = pairs[i].1.parse().unwrap_or(7);
                        // Negative ACI = layer off; magnitude = the color.
                        let off = aci < 0;
                        let abs = aci.unsigned_abs() as u8;
                        color = Color::Aci(abs);
                        if off { flags |= 0x01; }   // mark hidden
                    }
                    6  => lt_name = pairs[i].1.clone(),
                    70 => flags |= pairs[i].1.parse::<i32>().unwrap_or(0),
                    _ => {}
                }
                i += 1;
            }
            if !name.is_empty() {
                // "0" already exists at id 0; reuse it instead of duplicating.
                let lt_id = doc.linetypes.find(&lt_name)
                    .unwrap_or(LinetypeTable::CONTINUOUS);
                if let Some(existing) = doc.layers.find(&name) {
                    if let Some(l) = doc.layers.get_mut(existing) {
                        l.color    = color;
                        l.linetype = lt_id;
                        l.visible  = (flags & 0x01) == 0;
                        l.frozen   = (flags & 0x01) != 0;
                        l.locked   = (flags & 0x04) != 0;
                    }
                } else {
                    doc.layers.add(Layer {
                        name,
                        color,
                        linetype:   lt_id,
                        lineweight: Lineweight::Default,
                        visible:    (flags & 0x01) == 0,
                        locked:     (flags & 0x04) != 0,
                        frozen:     (flags & 0x01) != 0,
                        plottable:  true,
                    });
                }
            }
            continue;
        }
        i += 1;
    }
    pairs.len()
}

fn read_ltype_table(pairs: &[(i32, String)], start: usize, doc: &mut Document) -> usize {
    let mut i = start;
    while i < pairs.len() {
        let (c, v) = &pairs[i];
        if *c == 0 && v == "ENDTAB" { return i + 1; }
        if *c == 0 && v == "LTYPE" {
            let mut name = String::new();
            let mut desc = String::new();
            let mut pattern: Vec<f32> = Vec::new();
            i += 1;
            while i < pairs.len() && pairs[i].0 != 0 {
                match pairs[i].0 {
                    2  => name = pairs[i].1.clone(),
                    3  => desc = pairs[i].1.clone(),
                    49 => {
                        // dash length (positive) or gap (negative) — convert to
                        // alternating positive lengths for our pattern repr
                        if let Ok(v) = pairs[i].1.parse::<f32>() {
                            pattern.push(v.abs());
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            if !name.is_empty() && doc.linetypes.find(&name).is_none() {
                doc.linetypes.add(Linetype { name, description: desc, pattern });
            }
            continue;
        }
        i += 1;
    }
    pairs.len()
}

fn read_entities(pairs: &[(i32, String)], start: usize, doc: &mut Document) -> usize {
    let mut i = start;
    while i < pairs.len() {
        let (c, v) = &pairs[i];
        if *c == 0 && v == "ENDSEC" { return i + 1; }
        if *c == 0 {
            let entity_kind = v.clone();
            // Collect this entity's fields until the next 0-group.
            let mut fields: Vec<(i32, String)> = Vec::new();
            i += 1;
            while i < pairs.len() && pairs[i].0 != 0 {
                fields.push(pairs[i].clone());
                i += 1;
            }
            if let Some(d) = build_entity(&entity_kind, &fields, doc) {
                doc.push(d);
            }
            continue;
        }
        i += 1;
    }
    pairs.len()
}

fn build_entity(kind: &str, fields: &[(i32, String)], doc: &Document) -> Option<DObject> {
    let mut layer_name = String::new();
    let mut color: Option<Color> = None;
    let mut linetype_name: Option<String> = None;
    let mut visible = true;
    // helpers — return None when the field is missing or unparseable
    let get_f = |code: i32| -> Option<f64> {
        fields.iter().find(|(c, _)| *c == code).and_then(|(_, v)| v.parse().ok())
    };
    let get_i = |code: i32| -> Option<i32> {
        fields.iter().find(|(c, _)| *c == code).and_then(|(_, v)| v.parse().ok())
    };

    for (c, v) in fields {
        match *c {
            8  => layer_name = v.clone(),
            62 => {
                if let Ok(aci) = v.parse::<i32>() {
                    color = Some(if aci == 256 { Color::ByLayer }
                                 else if aci == 0 { Color::ByBlock }
                                 else { Color::Aci(aci.unsigned_abs() as u8) });
                }
            }
            6  => linetype_name = Some(v.clone()),
            60 => visible = v.parse::<i32>().unwrap_or(0) == 0,
            _  => {}
        }
    }

    let geom = match kind {
        "LINE" => Geom::Line(Line {
            a: Vec2::new(get_f(10)?, get_f(20)?),
            b: Vec2::new(get_f(11)?, get_f(21)?),
        }),
        "CIRCLE" => Geom::Circle(Circle {
            center: Vec2::new(get_f(10)?, get_f(20)?),
            radius: get_f(40)?,
        }),
        "ARC" => {
            let sa = get_f(50)?.to_radians();
            let ea = get_f(51)?.to_radians();
            let sweep = (ea - sa).rem_euclid(std::f64::consts::TAU);
            let sweep = if sweep < 1e-9 { std::f64::consts::TAU } else { sweep };
            Geom::Arc(Arc {
                center: Vec2::new(get_f(10)?, get_f(20)?),
                radius: get_f(40)?,
                start_angle: sa.rem_euclid(std::f64::consts::TAU),
                sweep_angle: sweep,
            })
        }
        "ELLIPSE" => {
            // 10/20 = center, 11/21 = major-axis vector (relative to center),
            // 40 = ratio, 41 = start param, 42 = end param.
            let center = Vec2::new(get_f(10)?, get_f(20)?);
            let major  = Vec2::new(get_f(11)?, get_f(21)?);
            let ratio  = get_f(40)?;
            let el = Ellipse { center, major, ratio };
            let sp = get_f(41).unwrap_or(0.0);
            let ep = get_f(42).unwrap_or(std::f64::consts::TAU);
            // Full ellipse <-> partial: if start ~= 0 and end ~= TAU, treat as full.
            if (sp.abs() < 1e-9) && ((ep - std::f64::consts::TAU).abs() < 1e-9) {
                Geom::Ellipse(el)
            } else {
                let sweep = (ep - sp).rem_euclid(std::f64::consts::TAU);
                let sweep = if sweep < 1e-9 { std::f64::consts::TAU } else { sweep };
                Geom::EllipseArc(EllipseArc {
                    ellipse: el,
                    start_param: sp.rem_euclid(std::f64::consts::TAU),
                    sweep_param: sweep,
                })
            }
        }
        "POINT" => Geom::Point(Point {
            location: Vec2::new(get_f(10)?, get_f(20)?),
            style: 0, size: 0.0,
        }),
        "LWPOLYLINE" => {
            let count = get_i(90).unwrap_or(0) as usize;
            let flags = get_i(70).unwrap_or(0);
            let closed = (flags & 0x01) != 0;
            // For LWPOLYLINE, vertex coords are interleaved 10/20 group codes
            // (and 42 for bulge per vertex). We walk fields in order and pair them.
            let mut vertices: Vec<PolyVertex> = Vec::with_capacity(count);
            let mut cur: Option<Vec2> = None;
            let mut cur_bulge = 0.0_f64;
            for (c, v) in fields {
                match *c {
                    10 => {
                        if let Some(p) = cur.take() {
                            vertices.push(PolyVertex { pos: p, bulge: cur_bulge });
                            cur_bulge = 0.0;
                        }
                        cur = Some(Vec2 { x: v.parse().unwrap_or(0.0), y: 0.0 });
                    }
                    20 => {
                        if let Some(p) = cur.as_mut() {
                            p.y = v.parse().unwrap_or(0.0);
                        }
                    }
                    42 => cur_bulge = v.parse().unwrap_or(0.0),
                    _ => {}
                }
            }
            if let Some(p) = cur.take() {
                vertices.push(PolyVertex { pos: p, bulge: cur_bulge });
            }
            if vertices.is_empty() { return None; }
            Geom::Polyline(Polyline { vertices, closed })
        }
        _ => return None,   // unknown entity type — silently skip
    };

    // Build the DObject with the proper style.
    let mut style = cad_kernel::Style::default();
    if let Some(c) = color { style.color = c; }
    if !layer_name.is_empty() {
        if let Some(lid) = doc.layers.find(&layer_name) {
            style.layer = lid;
        }
        // If the layer wasn't seen in TABLES we'd have to create it here.
        // For now we silently fall back to layer "0" — TODO when needed.
    }
    if let Some(lt) = linetype_name {
        if let Some(ltid) = doc.linetypes.find(&lt) {
            style.linetype = ltid;
        }
    }
    style.visible = visible;

    Some(DObject::with_style(geom, style))
}

// ============================================================================
//   WRITER
// ============================================================================

/// Serialize a `Document` to DXF ASCII text. Writes a minimal HEADER,
/// the LAYER and LTYPE tables (so layer/linetype references in ENTITIES
/// resolve when read back), then every Dobject. The result reads
/// correctly in LibreCAD and AutoCAD.
pub fn write_dxf(doc: &Document) -> String {
    let mut s = String::with_capacity(64 * 1024);
    write_header(&mut s);
    write_tables(&mut s, doc);
    write_entities(&mut s, doc);
    s.push_str("0\nEOF\n");
    s
}

fn pair(s: &mut String, code: i32, value: &str) {
    s.push_str(&format!("{}\n{}\n", code, value));
}
fn pair_f(s: &mut String, code: i32, v: f64) {
    s.push_str(&format!("{}\n{}\n", code, v));
}
fn pair_i(s: &mut String, code: i32, v: i32) {
    s.push_str(&format!("{}\n{}\n", code, v));
}

fn write_header(s: &mut String) {
    pair(s, 0, "SECTION");
    pair(s, 2, "HEADER");
    pair(s, 9, "$ACADVER"); pair(s, 1, "AC1015");   // AutoCAD 2000 ASCII level
    pair(s, 0, "ENDSEC");
}

fn write_tables(s: &mut String, doc: &Document) {
    pair(s, 0, "SECTION");
    pair(s, 2, "TABLES");

    // ---- LTYPE table ----
    pair(s, 0, "TABLE"); pair(s, 2, "LTYPE");
    pair_i(s, 70, doc.linetypes.len() as i32);
    for lt in &doc.linetypes.linetypes {
        pair(s, 0, "LTYPE");
        pair(s, 2, &lt.name);
        pair_i(s, 70, 0);
        pair(s, 3, &lt.description);
        pair_i(s, 72, 65);          // alignment code 'A'
        pair_i(s, 73, lt.pattern.len() as i32);
        let total: f32 = lt.pattern.iter().sum();
        pair_f(s, 40, total as f64);
        for (i, p) in lt.pattern.iter().enumerate() {
            // Alternating dash/gap convention in our model maps to:
            // dash = positive, gap = negative.
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            pair_f(s, 49, (*p as f64) * sign);
        }
    }
    pair(s, 0, "ENDTAB");

    // ---- LAYER table ----
    pair(s, 0, "TABLE"); pair(s, 2, "LAYER");
    pair_i(s, 70, doc.layers.len() as i32);
    for layer in &doc.layers.layers {
        pair(s, 0, "LAYER");
        pair(s, 2, &layer.name);
        // Flags: bit 0 = frozen, bit 2 = locked.
        let mut flags = 0_i32;
        if layer.frozen { flags |= 0x01; }
        if layer.locked { flags |= 0x04; }
        pair_i(s, 70, flags);
        // Color: ACI index (negative = layer off)
        let aci = match layer.color {
            Color::Aci(i) => i as i32,
            // For TrueColor, pick a "close enough" ACI — 7 = white/black, fine fallback.
            _ => 7,
        };
        let aci_signed = if layer.visible { aci } else { -aci.abs().max(1) };
        pair_i(s, 62, aci_signed);
        // Linetype name reference
        let lt_name = doc.linetypes.get(layer.linetype)
            .map(|l| l.name.clone()).unwrap_or_else(|| "Continuous".into());
        pair(s, 6, &lt_name);
    }
    pair(s, 0, "ENDTAB");

    pair(s, 0, "ENDSEC");
}

fn write_entities(s: &mut String, doc: &Document) {
    pair(s, 0, "SECTION");
    pair(s, 2, "ENTITIES");

    for d in &doc.dobjects {
        write_entity(s, d, doc);
    }

    pair(s, 0, "ENDSEC");
}

fn write_entity(s: &mut String, d: &DObject, doc: &Document) {
    let layer_name = doc.layers.get(d.style.layer)
        .map(|l| l.name.clone()).unwrap_or_else(|| "0".into());
    let linetype_name = doc.linetypes.get(d.style.linetype)
        .map(|l| l.name.clone()).unwrap_or_else(|| "Continuous".into());
    let common = |s: &mut String, kind: &str| {
        pair(s, 0, kind);
        pair(s, 8, &layer_name);
        pair(s, 6, &linetype_name);
        if let Color::Aci(i) = d.style.color {
            pair_i(s, 62, i as i32);
        } else if let Color::ByBlock = d.style.color {
            pair_i(s, 62, 0);
        } else {
            // ByLayer (default) and TrueColor both go as 256 (ByLayer) for
            // now — TrueColor would need group code 420 (handled in a follow-up).
            pair_i(s, 62, 256);
        }
        if !d.style.visible { pair_i(s, 60, 1); }
    };

    match &d.geom {
        Geom::Line(l) => {
            common(s, "LINE");
            pair_f(s, 10, l.a.x); pair_f(s, 20, l.a.y); pair_f(s, 30, 0.0);
            pair_f(s, 11, l.b.x); pair_f(s, 21, l.b.y); pair_f(s, 31, 0.0);
        }
        Geom::Circle(c) => {
            common(s, "CIRCLE");
            pair_f(s, 10, c.center.x); pair_f(s, 20, c.center.y); pair_f(s, 30, 0.0);
            pair_f(s, 40, c.radius);
        }
        Geom::Arc(a) => {
            common(s, "ARC");
            pair_f(s, 10, a.center.x); pair_f(s, 20, a.center.y); pair_f(s, 30, 0.0);
            pair_f(s, 40, a.radius);
            pair_f(s, 50, a.start_angle.to_degrees());
            pair_f(s, 51, (a.start_angle + a.sweep_angle).to_degrees());
        }
        Geom::Ellipse(el) => {
            common(s, "ELLIPSE");
            pair_f(s, 10, el.center.x); pair_f(s, 20, el.center.y); pair_f(s, 30, 0.0);
            pair_f(s, 11, el.major.x);  pair_f(s, 21, el.major.y);  pair_f(s, 31, 0.0);
            pair_f(s, 40, el.ratio);
            pair_f(s, 41, 0.0);
            pair_f(s, 42, std::f64::consts::TAU);
        }
        Geom::EllipseArc(ea) => {
            common(s, "ELLIPSE");
            pair_f(s, 10, ea.ellipse.center.x); pair_f(s, 20, ea.ellipse.center.y); pair_f(s, 30, 0.0);
            pair_f(s, 11, ea.ellipse.major.x);  pair_f(s, 21, ea.ellipse.major.y);  pair_f(s, 31, 0.0);
            pair_f(s, 40, ea.ellipse.ratio);
            pair_f(s, 41, ea.start_param);
            pair_f(s, 42, ea.start_param + ea.sweep_param);
        }
        Geom::Point(pt) => {
            common(s, "POINT");
            pair_f(s, 10, pt.location.x); pair_f(s, 20, pt.location.y); pair_f(s, 30, 0.0);
        }
        Geom::Polyline(p) => {
            common(s, "LWPOLYLINE");
            pair_i(s, 90, p.vertices.len() as i32);
            pair_i(s, 70, if p.closed { 1 } else { 0 });
            for v in &p.vertices {
                pair_f(s, 10, v.pos.x);
                pair_f(s, 20, v.pos.y);
                if v.bulge.abs() > 1e-12 {
                    pair_f(s, 42, v.bulge);
                }
            }
        }
        // DXF HATCH is a substantial entity with pattern, seed point,
        // boundary edge data, and seed loop topology. MVP does NOT
        // export hatches yet — skip silently so a round-trip with a
        // hatch in the doc just drops the hatch but otherwise succeeds.
        Geom::Hatch(_) => {}
    }
}

// ============================================================================
//   TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(doc: &Document) -> Document {
        let text = write_dxf(doc);
        read_dxf(&text).expect("round-trip parse")
    }

    #[test]
    fn empty_doc_round_trip() {
        let doc = Document::default();
        let back = round_trip(&doc);
        // Layer "0" + 3 default linetypes round-trip.
        assert_eq!(back.layers.len(), doc.layers.len());
        assert!(back.dobjects.is_empty());
    }

    #[test]
    fn line_round_trip() {
        let mut doc = Document::default();
        doc.push(Line { a: Vec2::new(0.0, 0.0), b: Vec2::new(10.0, 5.0) }.into());
        let back = round_trip(&doc);
        assert_eq!(back.dobjects.len(), 1);
        match &back.dobjects[0].geom {
            Geom::Line(l) => {
                assert!((l.a.x - 0.0).abs() < 1e-9);
                assert!((l.b.x - 10.0).abs() < 1e-9);
                assert!((l.b.y - 5.0).abs() < 1e-9);
            }
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn circle_round_trip() {
        let mut doc = Document::default();
        doc.push(Circle { center: Vec2::new(3.0, 4.0), radius: 7.0 }.into());
        let back = round_trip(&doc);
        if let Geom::Circle(c) = &back.dobjects[0].geom {
            assert!((c.center.x - 3.0).abs() < 1e-9);
            assert!((c.radius - 7.0).abs() < 1e-9);
        } else { panic!(); }
    }

    #[test]
    fn arc_round_trip_preserves_sweep() {
        let mut doc = Document::default();
        doc.push(Arc {
            center: Vec2::ZERO, radius: 5.0,
            start_angle: 0.5_f64,
            sweep_angle: 1.2_f64,
        }.into());
        let back = round_trip(&doc);
        if let Geom::Arc(a) = &back.dobjects[0].geom {
            assert!((a.start_angle - 0.5).abs() < 1e-6);
            assert!((a.sweep_angle - 1.2).abs() < 1e-6);
        } else { panic!(); }
    }

    #[test]
    fn point_round_trip() {
        let mut doc = Document::default();
        doc.push(Point { location: Vec2::new(1.0, 2.0), style: 0, size: 0.0 }.into());
        let back = round_trip(&doc);
        if let Geom::Point(p) = &back.dobjects[0].geom {
            assert!((p.location.x - 1.0).abs() < 1e-9);
        } else { panic!(); }
    }

    #[test]
    fn polyline_round_trip_open() {
        let mut doc = Document::default();
        doc.push(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(5.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(5.0, 5.0), bulge: 0.0 },
            ],
            closed: false,
        }.into());
        let back = round_trip(&doc);
        if let Geom::Polyline(p) = &back.dobjects[0].geom {
            assert_eq!(p.vertices.len(), 3);
            assert!(!p.closed);
        } else { panic!(); }
    }

    #[test]
    fn polyline_round_trip_closed() {
        let mut doc = Document::default();
        doc.push(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(5.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(5.0, 5.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(0.0, 5.0), bulge: 0.0 },
            ],
            closed: true,
        }.into());
        let back = round_trip(&doc);
        if let Geom::Polyline(p) = &back.dobjects[0].geom {
            assert_eq!(p.vertices.len(), 4);
            assert!(p.closed);
        } else { panic!(); }
    }

    #[test]
    fn ellipse_round_trip() {
        let mut doc = Document::default();
        doc.push(Ellipse {
            center: Vec2::ZERO, major: Vec2::new(5.0, 0.0), ratio: 0.4,
        }.into());
        let back = round_trip(&doc);
        if let Geom::Ellipse(e) = &back.dobjects[0].geom {
            assert!((e.semi_major() - 5.0).abs() < 1e-9);
            assert!((e.ratio - 0.4).abs() < 1e-9);
        } else { panic!(); }
    }

    #[test]
    fn layer_round_trip_preserves_name_and_color() {
        let mut doc = Document::default();
        let walls = doc.layers.add(Layer {
            name: "WALLS".into(),
            color: Color::Aci(1),
            ..Layer::layer_zero()
        });
        doc.layers.active = walls;
        doc.push(Circle { center: Vec2::ZERO, radius: 5.0 }.into());
        let back = round_trip(&doc);
        // Layer must round-trip
        let id = back.layers.find("WALLS").expect("WALLS layer not preserved");
        assert!(matches!(back.layers.get(id).unwrap().color, Color::Aci(1)));
        // Dobject's style.layer must point at WALLS post-import
        assert_eq!(back.dobjects[0].style.layer, id);
    }
}
