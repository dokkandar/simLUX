//! Shared application state, guarded for Tauri's multi-threaded command runtime.
use std::sync::Mutex;

use crate::engine::draft::Draft;
use crate::model::Project;

/// The single source of truth, handed to Tauri via `.manage()`.
#[derive(Default)]
pub struct AppState {
    pub project: Mutex<Project>,
    /// The 2D drafting session (command environment + `cad_kernel::Document`).
    pub draft: Mutex<Draft>,
}
