//! Project data model — the serialisable state shared with the UI and saved to
//! `.lux` project files.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::engine::calc::RayTracingSettings;
use crate::engine::geometry::{CalculationPlane, Line2, Mesh, Room, Vertex};
use crate::engine::ies::IesProfile;

/// Index into the project's material table.
pub type MaterialId = u32;

/// A Lambertian surface material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    pub id: MaterialId,
    pub name: String,
    /// Diffuse reflectance, 0.0–1.0.
    pub reflectance: f32,
    /// Linear RGB tint, 0.0–1.0 per channel.
    pub color: [f32; 3],
}

/// A luminaire placed in the scene: an IES profile at a pose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuminaireInstance {
    pub id: u32,
    /// Key into [`Project::profiles`].
    pub profile: String,
    pub position: Vertex,
    /// Rotation about the vertical axis (degrees).
    pub rotation_deg: f32,
    /// Dimming / output scale, 0.0–1.0.
    pub dimming: f32,
}

/// Top-level project state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub rooms: Vec<Room>,
    pub luminaires: Vec<LuminaireInstance>,
    pub materials: Vec<Material>,
    pub profiles: HashMap<String, IesProfile>,
    /// Raw imported DXF geometry (plan underlay).
    pub dxf_lines: Vec<Line2>,
    /// Triangulated scene geometry the ray tracer bounces light off.
    pub meshes: Vec<Mesh>,
    pub calc_plane: Option<CalculationPlane>,
    pub settings: RayTracingSettings,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            name: "Untitled".to_string(),
            rooms: Vec::new(),
            luminaires: Vec::new(),
            materials: vec![
                Material { id: 0, name: "Floor".into(),   reflectance: 0.20, color: [0.6, 0.6, 0.6] },
                Material { id: 1, name: "Wall".into(),    reflectance: 0.50, color: [0.8, 0.8, 0.8] },
                Material { id: 2, name: "Ceiling".into(), reflectance: 0.70, color: [0.9, 0.9, 0.9] },
            ],
            profiles: HashMap::new(),
            dxf_lines: Vec::new(),
            meshes: Vec::new(),
            calc_plane: None,
            settings: RayTracingSettings::default(),
        }
    }
}
