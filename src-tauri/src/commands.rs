//! Tauri command handlers — the API surface exposed to the frontend.
use std::fs;

use serde::Serialize;
use tauri::State;

use crate::engine::calc::{self, LuxGrid, Scene};
use crate::engine::geometry::{Line2, Mesh};
use crate::engine::ies::{self, IesProfile};
use crate::engine::dxf;
use crate::error::{EngineError, EngineResult};
use crate::model::Project;
use crate::state::AppState;

/// Basic engine metadata — a zero-argument command handy for verifying the
/// JS⇄Rust bridge is live from the frontend.
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
        phase: "Scaffold — Phase 3.1 (direct-only) in progress".into(),
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

/// Load 2D plan geometry from a DXF file and store it as the plan underlay.
#[tauri::command]
pub fn load_dxf(state: State<AppState>, path: String) -> EngineResult<Vec<Line2>> {
    let contents = fs::read_to_string(&path)?;
    let lines = dxf::load_lines(&contents)?;
    state.project.lock().unwrap().dxf_lines = lines.clone();
    Ok(lines)
}

/// Compute the lux grid for the current project.
#[tauri::command]
pub fn calculate_lux(state: State<AppState>) -> EngineResult<LuxGrid> {
    let project = state.project.lock().unwrap();
    let plane = project
        .calc_plane
        .as_ref()
        .ok_or_else(|| EngineError::Geometry("no calculation plane defined".into()))?;

    // Scene meshes stay empty until the mesher lands (Phase 3.2).
    let meshes: Vec<Mesh> = Vec::new();
    let scene = Scene {
        luminaires: &project.luminaires,
        meshes: &meshes,
        plane,
    };
    calc::calculate_direct(&scene, &project.settings)
}
