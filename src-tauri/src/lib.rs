//! SIMLUX — a physically-based lighting (lux) simulator.
//!
//! Architecture (see `ROADMAP.md`):
//!   - `engine::ies`      — IES LM-63 photometry
//!   - `engine::dxf`      — DXF plan import
//!   - `engine::geometry` — 2D/3D primitives, meshes, the calculation plane
//!   - `engine::calc`     — direct lux now, progressive radiosity later
//!   - `engine::math`     — shared vector/photometry math
//!   - `model`            — serialisable project state
//!   - `commands`         — the Tauri API surface
pub mod commands;
pub mod engine;
pub mod error;
pub mod model;
pub mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::engine_info,
            commands::get_project,
            commands::import_ies,
            commands::load_dxf,
            commands::add_luminaire,
            commands::add_demo_room,
            commands::calculate_lux,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
