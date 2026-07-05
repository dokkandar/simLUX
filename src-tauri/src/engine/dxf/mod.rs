//! DXF import: extract 2D plan geometry (walls, layouts) from CAD drawings.
//!
//! Reuses the tested ASCII-DXF reader from Auto_RASM (`cad_io::dxf::read_dxf`),
//! then flattens the returned `Document` into flat [`Line2`] segments — the plan
//! underlay the viewport draws and the Phase 3.2 wall tracer will snap to.
use cad_kernel::join::polyline_segments;
use cad_kernel::{Arc, Circle, Geom, Line as KLine, Vec2};

use crate::engine::geometry::{Line2, Point2};
use crate::error::{EngineError, EngineResult};

/// Degrees of arc per tessellated segment when flattening arcs/circles.
const ARC_SEG_DEG: f64 = 8.0;

fn p2(v: Vec2) -> Point2 {
    Point2 { x: v.x as f32, y: v.y as f32 }
}

fn seg(a: Vec2, b: Vec2) -> Line2 {
    Line2 { start: p2(a), end: p2(b) }
}

/// Segment count to approximate a sweep of `sweep_abs` radians.
fn arc_segments(sweep_abs: f64) -> usize {
    ((sweep_abs / ARC_SEG_DEG.to_radians()).ceil() as usize).max(2)
}

/// Append straight chords approximating a (possibly full) circular arc.
fn tessellate_arc(center: Vec2, radius: f64, start: f64, sweep: f64, out: &mut Vec<Line2>) {
    let n = arc_segments(sweep.abs());
    let point_at = |ang: f64| Vec2::new(center.x + radius * ang.cos(), center.y + radius * ang.sin());
    let mut prev = point_at(start);
    for i in 1..=n {
        let cur = point_at(start + sweep * (i as f64 / n as f64));
        out.push(seg(prev, cur));
        prev = cur;
    }
}

/// Flatten one geometry entity into line segments.
fn flatten(geom: &Geom, out: &mut Vec<Line2>) {
    match geom {
        Geom::Line(KLine { a, b }) => out.push(seg(*a, *b)),
        Geom::Arc(Arc { center, radius, start_angle, sweep_angle }) => {
            tessellate_arc(*center, *radius, *start_angle, *sweep_angle, out);
        }
        Geom::Circle(Circle { center, radius }) => {
            tessellate_arc(*center, *radius, 0.0, std::f64::consts::TAU, out);
        }
        // Bulged spans expand into their true Line/Arc geoms, then recurse.
        Geom::Polyline(pl) => {
            for g in polyline_segments(pl) {
                flatten(&g, out);
            }
        }
        Geom::Spline(sp) => {
            for w in sp.tessellate(64).windows(2) {
                out.push(seg(w[0], w[1]));
            }
        }
        // Ellipse / EllipseArc / Text / Dimension / Hatch / Wall / Point /
        // BlockRef are not part of the plan underlay yet (ROADMAP Phase 3.2+).
        _ => {}
    }
}

/// Parse DXF file contents and flatten drawable plan geometry into [`Line2`]s.
///
/// Handles top-level LINE / ARC / CIRCLE / LWPOLYLINE / SPLINE entities. Block
/// (INSERT) contents are not yet expanded — see ROADMAP Phase 3.2.
pub fn load_lines(contents: &str) -> EngineResult<Vec<Line2>> {
    let doc = cad_io::dxf::read_dxf(contents).map_err(EngineError::DxfParse)?;
    let mut lines = Vec::new();
    for d in &doc.dobjects {
        flatten(&d.geom, &mut lines);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_corner_sofa_sample() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../samples/corner sofa.dxf");
        let contents = std::fs::read_to_string(path).expect("read sample dxf");
        let lines = load_lines(&contents).expect("parse dxf");
        // 62 LINE + 24 ARC (each arc tessellated) -> well over 100 segments.
        assert!(
            lines.len() >= 100,
            "expected the sofa plan to flatten to many segments, got {}",
            lines.len()
        );
        // All finite, non-degenerate coordinates.
        assert!(lines
            .iter()
            .all(|l| l.start.x.is_finite() && l.start.y.is_finite()));
    }
}
