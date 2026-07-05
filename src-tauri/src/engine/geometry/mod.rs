//! Geometric primitives: 2D plan geometry, 3D meshes, and the calculation plane.
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::model::MaterialId;

/// A 2D point in plan coordinates (metres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point2 {
    pub x: f32,
    pub y: f32,
}

/// A 2D line segment — a DXF edge or a wall centreline.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Line2 {
    pub start: Point2,
    pub end: Point2,
}

/// A 3D vertex (metres).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vertex {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    #[inline]
    pub fn to_vec3(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
    #[inline]
    pub fn from_vec3(v: Vec3) -> Self {
        Self { x: v.x, y: v.y, z: v.z }
    }
}

/// A mesh face, indexing three vertices in its parent [`Mesh`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Triangle {
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

/// A triangulated surface with a Lambertian material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<Triangle>,
    pub material: MaterialId,
}

/// A single wall: a 2D centreline with `thickness`, extruded to `height` (metres).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Wall {
    pub centerline: Line2,
    pub thickness: f32,
    pub height: f32,
}

/// A room: a closed loop of walls, with an id and display name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: u32,
    pub name: String,
    pub walls: Vec<Wall>,
}

/// The horizontal work plane on which illuminance is sampled.
///
/// A `rows x cols` grid of sensor points spans a `width` (x) by `depth` (y)
/// rectangle anchored at `origin` (its min corner), at height `origin.z`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CalculationPlane {
    pub origin: Vertex,
    pub width: f32,
    pub depth: f32,
    pub cols: u32,
    pub rows: u32,
}

impl CalculationPlane {
    /// World-space position of the sensor at grid cell `(col, row)`, taken at
    /// the centre of the cell.
    pub fn sample_point(&self, col: u32, row: u32) -> Vertex {
        let dx = self.width / self.cols as f32;
        let dy = self.depth / self.rows as f32;
        Vertex::new(
            self.origin.x + (col as f32 + 0.5) * dx,
            self.origin.y + (row as f32 + 0.5) * dy,
            self.origin.z,
        )
    }
}
