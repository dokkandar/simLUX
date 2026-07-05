//! Tauri command handlers — the API surface exposed to the frontend.
use std::fs;

use serde::Serialize;
use tauri::State;

use crate::engine::calc::{self, LuxGrid};
use crate::engine::dxf;
use crate::engine::geometry::{box_room, CalculationPlane, Line2, Point2, Vertex, WallSeg};
use crate::engine::ies::{self, IesProfile};
use crate::engine::wall;
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

/// Append a 2D wall segment (traced over the underlay). Returns the project.
#[tauri::command]
pub fn add_wall(
    state: State<AppState>,
    start_x: f32,
    start_y: f32,
    end_x: f32,
    end_y: f32,
    thickness: f32,
) -> Project {
    let mut project = state.project.lock().unwrap();
    project.walls.push(WallSeg {
        start: Point2 { x: start_x, y: start_y },
        end: Point2 { x: end_x, y: end_y },
        thickness,
    });
    project.clone()
}

/// Translate a wall (by index) — the "move" modifier. Returns the project.
#[tauri::command]
pub fn move_wall(state: State<AppState>, index: usize, dx: f32, dy: f32) -> Project {
    let mut project = state.project.lock().unwrap();
    if let Some(w) = project.walls.get_mut(index) {
        w.start.x += dx;
        w.start.y += dy;
        w.end.x += dx;
        w.end.y += dy;
    }
    project.clone()
}

/// Offset a wall (by index) perpendicular to its centreline — the "offset"
/// modifier. Returns the project.
#[tauri::command]
pub fn offset_wall(state: State<AppState>, index: usize, dist: f32) -> Project {
    let mut project = state.project.lock().unwrap();
    if let Some(w) = project.walls.get_mut(index) {
        let (dxl, dyl) = (w.end.x - w.start.x, w.end.y - w.start.y);
        let len = (dxl * dxl + dyl * dyl).sqrt();
        if len > 1e-6 {
            let (nx, ny) = (-dyl / len * dist, dxl / len * dist);
            w.start.x += nx;
            w.start.y += ny;
            w.end.x += nx;
            w.end.y += ny;
        }
    }
    project.clone()
}

/// Remove all walls and the room they produced (keeps the underlay). Returns the project.
#[tauri::command]
pub fn clear_walls(state: State<AppState>) -> Project {
    let mut project = state.project.lock().unwrap();
    project.walls.clear();
    project.meshes.clear();
    project.clone()
}

/// Stitch + extrude the drawn walls to `height`, lay a work-plane grid over the
/// footprint at `plane_height`, and (if a profile is loaded and no luminaire
/// exists) drop a ceiling-centre downlight. Returns the project.
#[tauri::command]
pub fn build_room(state: State<AppState>, height: f32, plane_height: f32) -> Project {
    let mut project = state.project.lock().unwrap();
    project.room_height = height;
    project.meshes = wall::extrude(&project.walls, height);

    if let Some((min_x, min_y, max_x, max_y)) = walls_bbox(&project.walls) {
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

fn walls_bbox(walls: &[WallSeg]) -> Option<(f32, f32, f32, f32)> {
    if walls.is_empty() {
        return None;
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for w in walls {
        for p in [w.start, w.end] {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
    }
    Some((min_x, min_y, max_x, max_y))
}

/// Compute the lux grid for the current project.
#[tauri::command]
pub fn calculate_lux(state: State<AppState>) -> EngineResult<LuxGrid> {
    let project = state.project.lock().unwrap();
    calc::calculate(&project)
}
