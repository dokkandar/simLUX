//! Tauri command handlers — the API surface exposed to the frontend.
use std::fs;

use serde::Serialize;
use tauri::State;

use crate::engine::calc::{self, LuxGrid};
use crate::engine::draft::{self, CmdResult};
use crate::engine::dxf;
use crate::engine::geometry::{box_room, CalculationPlane, Line2, Vertex};
use crate::engine::ies::{self, IesProfile};
use crate::error::EngineResult;
use crate::model::{LuminaireInstance, Project};
use crate::state::AppState;

/// Basic engine metadata — a zero-argument command for verifying the JS⇄Rust bridge.
#[derive(Serialize)]
pub struct EngineInfo {
    pub name: String,
    pub version: String,
    pub phase: String,
}

#[tauri::command]
pub fn engine_info() -> EngineInfo {
    EngineInfo {
        name: "SIMLUX".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        phase: "Phase 3.2 — command-line 2D drafting".into(),
    }
}

/// Return a snapshot of the current project.
#[tauri::command]
pub fn get_project(state: State<AppState>) -> Project {
    state.project.lock().unwrap().clone()
}

/// Import an IES file from disk, store it in the project, and return it.
#[tauri::command]
pub fn import_ies(state: State<AppState>, path: String) -> EngineResult<IesProfile> {
    let contents = fs::read_to_string(&path)?;
    let profile = ies::parse(&contents)?;
    let mut project = state.project.lock().unwrap();
    project.profiles.insert(profile.name.clone(), profile.clone());
    Ok(profile)
}

/// Load 2D plan geometry from a DXF file and store it as the reference underlay.
#[tauri::command]
pub fn load_dxf(state: State<AppState>, path: String) -> EngineResult<Vec<Line2>> {
    let contents = fs::read_to_string(&path)?;
    let lines = dxf::load_lines(&contents)?;
    state.project.lock().unwrap().dxf_lines = lines.clone();
    Ok(lines)
}

// ---- Command-line 2D drafting (cad_kernel::Document via engine::draft) ----

/// Run one command-line line: a command (`line`, `circle`, `rect`, `wall`, …),
/// a coordinate (`3,0`, `@2,0`, `@5<90`), or a keyword (`close`, `undo`).
#[tauri::command]
pub fn exec_command(state: State<AppState>, input: String) -> CmdResult {
    state.draft.lock().unwrap().exec(&input)
}

/// Feed a clicked world point to the active command (or select an entity when
/// idle). `tol` is the world-space pick radius (pixels ÷ zoom).
#[tauri::command]
pub fn pick_point(state: State<AppState>, x: f32, y: f32, tol: f32) -> CmdResult {
    state.draft.lock().unwrap().click(x, y, tol)
}

/// Cancel the active command (Esc).
#[tauri::command]
pub fn cancel_command(state: State<AppState>) -> CmdResult {
    state.draft.lock().unwrap().cancel()
}

/// Current drafting snapshot (geometry + prompt), no mutation.
#[tauri::command]
pub fn get_geometry(state: State<AppState>) -> CmdResult {
    state.draft.lock().unwrap().snapshot_result()
}

// ---- Scene: luminaires + room build ----

/// Place a luminaire (using an already-imported IES profile) at a world point.
#[tauri::command]
pub fn add_luminaire(state: State<AppState>, x: f32, y: f32, z: f32, profile: String) -> Project {
    let mut project = state.project.lock().unwrap();
    let id = project.luminaires.iter().map(|l| l.id).max().unwrap_or(0) + 1;
    project.luminaires.push(LuminaireInstance {
        id,
        profile,
        position: Vertex::new(x, y, z),
        rotation_deg: 0.0,
        dimming: 1.0,
    });
    project.clone()
}

/// Extrude the drafted geometry to `height`, lay a work-plane grid over its
/// footprint, and (if a profile is loaded and no luminaire exists) add a
/// ceiling-centre downlight. Returns the project.
#[tauri::command]
pub fn build_room(state: State<AppState>, height: f32, plane_height: f32) -> Project {
    let draft = state.draft.lock().unwrap();
    let mut project = state.project.lock().unwrap();
    project.room_height = height;
    project.meshes = draft::extrude(&draft.doc, height);

    if let Some((min_x, min_y, max_x, max_y)) = draft::bbox(&draft.doc) {
        let (w, d) = (max_x - min_x, max_y - min_y);
        let cols = ((w / 0.2).round() as u32).clamp(8, 48);
        let rows = ((d / 0.2).round() as u32).clamp(8, 48);
        project.calc_plane = Some(CalculationPlane {
            origin: Vertex::new(min_x, min_y, plane_height),
            width: w,
            depth: d,
            cols,
            rows,
        });
        if project.luminaires.is_empty() {
            if let Some(profile) = project.profiles.keys().next().cloned() {
                project.luminaires.push(LuminaireInstance {
                    id: 1,
                    profile,
                    position: Vertex::new(min_x + w / 2.0, min_y + d / 2.0, height),
                    rotation_deg: 0.0,
                    dimming: 1.0,
                });
            }
        }
    }
    project.clone()
}

/// Quick box room + calc grid + downlight, without drafting (a fallback demo).
#[tauri::command]
pub fn add_demo_room(
    state: State<AppState>,
    width: f32,
    depth: f32,
    height: f32,
    plane_height: f32,
) -> Project {
    let mut project = state.project.lock().unwrap();
    project.meshes = box_room(width, depth, height);
    project.calc_plane = Some(CalculationPlane {
        origin: Vertex::new(0.0, 0.0, plane_height),
        width,
        depth,
        cols: 24,
        rows: 24,
    });
    if project.luminaires.is_empty() {
        if let Some(profile) = project.profiles.keys().next().cloned() {
            project.luminaires.push(LuminaireInstance {
                id: 1,
                profile,
                position: Vertex::new(width / 2.0, depth / 2.0, height),
                rotation_deg: 0.0,
                dimming: 1.0,
            });
        }
    }
    project.clone()
}

/// Compute the lux grid for the current project.
#[tauri::command]
pub fn calculate_lux(state: State<AppState>) -> EngineResult<LuxGrid> {
    let project = state.project.lock().unwrap();
    calc::calculate(&project)
}
