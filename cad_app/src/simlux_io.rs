//! SIMLUX sidecar persistence — `<drawing>.simlux.json` beside the `.rsm`/`.dxf`.
//!
//! All SIMLUX-specific state (which layers extrude in 3D + their heights, the
//! load-once IES library, the LUX-block→IES map, materials, ray settings) lives
//! here, NOT in the (2D) `cad_kernel` document. Keyed by STABLE NAMES (layer
//! name, profile name, block-def name) so it survives save/reopen even though
//! layer/block ids are positional. `cad_kernel` / `cad_io` stay UNTOUCHED
//! (decision D5, SIMLUX_LUX_WORKFLOW.md).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cad_light::{IesProfile, Material, RaySettings};
use serde::{Deserialize, Serialize};

/// Everything SIMLUX persists next to a drawing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SimluxConfig {
    /// Layer NAME → extrude height (m). Presence ⇒ "use for 3D".
    #[serde(default)]
    pub layers_3d: BTreeMap<String, f32>,
    /// IES library — profile name → profile. Entered ONCE, referenced by name.
    #[serde(default)]
    pub ies_library: BTreeMap<String, IesProfile>,
    /// Selected / active IES profile name.
    #[serde(default)]
    pub active_profile: String,
    /// LUX block DEFINITION name → IES profile name (Slice 3; type-level D4).
    #[serde(default)]
    pub lux_block_ies: BTreeMap<String, String>,
    /// Surface materials [floor, wall, ceiling].
    #[serde(default)]
    pub materials: Vec<Material>,
    /// Ray-tracer controls.
    #[serde(default)]
    pub settings: RaySettings,
    /// Default room height + work-plane height + grid cell size (metres).
    #[serde(default)]
    pub room_height: f32,
    #[serde(default)]
    pub plane_height: f32,
    #[serde(default)]
    pub cell_size: f32,
}

/// The sidecar path for a drawing: `foo.rsm` → `foo.simlux.json`.
pub fn sidecar_path(drawing: &Path) -> PathBuf {
    drawing.with_extension("simlux.json")
}

/// Read the sidecar for `drawing`, if present. `Ok(None)` = no sidecar there.
pub fn load(drawing: &Path) -> Result<Option<SimluxConfig>, String> {
    let p = sidecar_path(drawing);
    if !p.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let cfg: SimluxConfig = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(Some(cfg))
}

/// Write the sidecar for `drawing`. Returns the path written.
pub fn save(drawing: &Path, cfg: &SimluxConfig) -> Result<PathBuf, String> {
    let p = sidecar_path(drawing);
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&p, text).map_err(|e| e.to_string())?;
    Ok(p)
}
