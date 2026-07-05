//! IES LM-63 photometric file support.
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, EngineResult};

/// Goniometer geometry declared in the IES header (Type A/B/C photometry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhotometryType {
    A,
    B,
    C,
}

/// A parsed IES luminous-intensity distribution.
///
/// `candela[h][v]` is indexed by horizontal-angle row then vertical-angle
/// column, matching the LM-63 candela block layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IesProfile {
    pub name: String,
    pub photometry: PhotometryType,
    /// Rated lumens per lamp × number of lamps.
    pub lumens: f64,
    /// Candela multiplier from the header.
    pub multiplier: f64,
    pub vertical_angles: Vec<f64>,
    pub horizontal_angles: Vec<f64>,
    pub candela: Vec<Vec<f64>>,
    /// Luminous-opening dimensions (metres): width, length, height.
    pub width: f64,
    pub length: f64,
    pub height: f64,
}

impl IesProfile {
    /// Bilinearly-interpolated luminous intensity (candela) toward the given
    /// vertical/horizontal angle in degrees.
    ///
    /// TODO(Phase 3.1): interpolate over the candela table (nearest-neighbour is
    /// visibly jagged — bilinear is the minimum for professional output).
    pub fn intensity(&self, _vertical_deg: f64, _horizontal_deg: f64) -> f64 {
        0.0
    }
}

/// Parse the contents of an IES LM-63 file into an [`IesProfile`].
///
/// TODO(Phase 3.1): implement the LM-63-2002 tokenizer — TILT handling, header
/// counts, the vertical/horizontal angle arrays, then the candela block.
pub fn parse(_contents: &str) -> EngineResult<IesProfile> {
    Err(EngineError::NotImplemented(
        "IES LM-63 parser — see ROADMAP Phase 3.1",
    ))
}
