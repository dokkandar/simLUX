//! Tauri command handlers — the API surface exposed to the frontend.
use std::fs;

use serde::Serialize;
use tauri::State;

use crate::engine::calc::{self, LuxGrid};
use crate::engine::dxf;
use crate::engine::geometry::{box_room, CalculationPlane, Line2, Vertex};
use crate::engine::ies::{self, IesProfile};
use crate::error::EngineResult;
use crate::model::{LuminaireInstance, Project};
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
        phase: "Phase 3.1 — direct + one-bounce indirect".into(),
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

/// Place a luminaire (using an already-imported IES profile) at a world point.
/// Returns the updated project.
#[tauri::command]
pub fn add_luminaire(
    state: State<AppState>,
    x: f32,
    y: f32,
    z: f32,
    profile: String,
) -> Project {
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

/// Build a rectangular demo room (floor/walls/ceiling), a work-plane grid, and —
/// if an IES profile is loaded and no luminaire exists yet — a ceiling-centre
/// downlight. Gives the ray tracer a real scene without the Phase 3.2 wall tools.
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
