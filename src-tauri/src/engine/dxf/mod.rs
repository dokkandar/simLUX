//! DXF import: extract 2D plan geometry (walls, layouts) from CAD drawings.
use crate::engine::geometry::Line2;
use crate::error::{EngineError, EngineResult};

/// Extract 2D line geometry (LWPOLYLINE / LINE entities) from DXF file contents.
///
/// TODO(Phase 3.1): parse with the `dxf` crate, flatten LWPOLYLINE vertices into
/// [`Line2`] edges, and carry layer names through for later room selection.
pub fn load_lines(_contents: &str) -> EngineResult<Vec<Line2>> {
    Err(EngineError::NotImplemented(
        "DXF LWPOLYLINE loader — see ROADMAP Phase 3.1",
    ))
}
