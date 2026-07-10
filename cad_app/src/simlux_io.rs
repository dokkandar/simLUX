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
    /// Layer NAME → base elevation (m) the extrusion starts from. Default 0.
    #[serde(default)]
    pub layers_3d_base: BTreeMap<String, f32>,
    /// Layer NAME → extrude downward from the base instead of up. Default false.
    #[serde(default)]
    pub layers_3d_down: BTreeMap<String, bool>,
    /// IES library — profile name → profile. Entered ONCE, referenced by name.
    #[serde(default)]
    pub ies_library: BTreeMap<String, IesProfile>,
    /// Selected / active IES profile name.
    #[serde(default)]
    pub active_profile: String,
    /// LUX-block registry: block DEFINITION name → its luminaire descriptor
    /// (Slice 3; type-level D4). A block is a luminaire iff it has an entry here.
    #[serde(default)]
    pub lux_blocks: BTreeMap<String, LuxBlock>,
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

/// A luminaire block's photometry: **many IES linked, only one active** at a
/// time (the active one drives calc + render). The linked set lets one fixture
/// carry several lamp/optic options (e.g. one IES per mounting height 10′/20′/30′)
/// and switch between them without redefining the block. IES are referenced BY
/// NAME into the shared `ies_library` — entered once, never copied.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LuxBlock {
    /// IES profile names linked to this luminaire block — the candidate set.
    #[serde(default)]
    pub ies_options: Vec<String>,
    /// The ONE active option (must be in `ies_options`); `None` = luminaire block
    /// with no photometry assigned yet (draws its symbol, contributes no light).
    #[serde(default)]
    pub active: Option<String>,
}

impl LuxBlock {
    /// The active IES name, but only if it is still one of the linked options.
    pub fn active_ies(&self) -> Option<&String> {
        self.active.as_ref().filter(|a| self.ies_options.contains(a))
    }
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
