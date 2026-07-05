//! The lux calculation engine: direct illuminance now, progressive radiosity later.
use serde::{Deserialize, Serialize};

use crate::engine::geometry::{CalculationPlane, Mesh};
use crate::error::{EngineError, EngineResult};
use crate::model::LuminaireInstance;

/// Ray-tracing / radiosity controls.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RayTracingSettings {
    /// Hemisphere samples per grid point for the indirect pass.
    pub rays_per_point: u32,
    /// Maximum number of diffuse bounces (0 = direct only).
    pub max_bounces: u32,
    /// Whether to cast shadow rays against scene geometry.
    pub shadows: bool,
}

impl Default for RayTracingSettings {
    fn default() -> Self {
        Self {
            rays_per_point: 128,
            max_bounces: 2,
            shadows: true,
        }
    }
}

/// Everything the engine needs to compute a lux grid.
pub struct Scene<'a> {
    pub luminaires: &'a [LuminaireInstance],
    pub meshes: &'a [Mesh],
    pub plane: &'a CalculationPlane,
}

/// A computed illuminance grid (lux) over a [`CalculationPlane`], row-major.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuxGrid {
    pub cols: u32,
    pub rows: u32,
    pub values: Vec<f64>,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
}

impl LuxGrid {
    /// Build a grid from raw row-major lux values, computing summary stats.
    pub fn from_values(cols: u32, rows: u32, values: Vec<f64>) -> Self {
        let (mut min, mut max, mut sum) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
        for &v in &values {
            min = min.min(v);
            max = max.max(v);
            sum += v;
        }
        let n = values.len().max(1) as f64;
        LuxGrid {
            cols,
            rows,
            min: if min.is_finite() { min } else { 0.0 },
            max: if max.is_finite() { max } else { 0.0 },
            avg: sum / n,
            values,
        }
    }
}

/// Direct-only illuminance: for each grid point, sum each luminaire's
/// `E = I(θ,ψ)·cos(ε) / d²`, optionally shadow-tested against `scene.meshes`.
///
/// TODO(Phase 3.1): iterate `scene.plane` sample points, query each luminaire's
/// [`IesProfile::intensity`](crate::engine::ies::IesProfile::intensity), and
/// accumulate via [`point_illuminance`](crate::engine::math::point_illuminance).
/// Parallelise across grid points with `rayon` once shadows land (Phase 3.2).
pub fn calculate_direct(_scene: &Scene, _settings: &RayTracingSettings) -> EngineResult<LuxGrid> {
    Err(EngineError::NotImplemented(
        "direct lux engine — see ROADMAP Phase 3.1",
    ))
}
