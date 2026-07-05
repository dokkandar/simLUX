//! The lux calculation engine.
//!
//! Direct illuminance is the inverse-square + cosine law summed over luminaires,
//! shadow-tested against scene geometry. Indirect ("light reflecting off walls")
//! is progressive radiosity done the Monte-Carlo way: from each sensor point we
//! shoot cosine-weighted rays into the hemisphere, and each surface they hit
//! re-emits its own (Lambertian) reflected illuminance. Bounces recurse up to
//! `settings.max_bounces`.
use std::collections::HashMap;
use std::f64::consts::PI;

use glam::Vec3;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::engine::geometry::{CalculationPlane, Mesh};
use crate::engine::ies::IesProfile;
use crate::engine::rt::{cosine_sample, Ray, Rng, RtScene, Tri};
use crate::error::{EngineError, EngineResult};
use crate::model::{LuminaireInstance, Material, MaterialId, Project};

/// Surface offset for shadow/bounce ray origins, metres.
const EPS: f32 = 1e-3;

/// Ray-tracing / radiosity controls.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RayTracingSettings {
    /// Hemisphere samples per point for the indirect pass.
    pub rays_per_point: u32,
    /// Maximum number of diffuse bounces (0 = direct only).
    pub max_bounces: u32,
    /// Whether to cast shadow rays against scene geometry.
    pub shadows: bool,
}

impl Default for RayTracingSettings {
    fn default() -> Self {
        // Snappy defaults — "reflect light off walls" without heavy render times.
        Self { rays_per_point: 64, max_bounces: 1, shadows: true }
    }
}

/// A computed illuminance grid (lux) over a [`CalculationPlane`], row-major.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuxGrid {
    pub cols: u32,
    pub rows: u32,
    pub values: Vec<f64>,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
}

impl LuxGrid {
    /// Build a grid from raw row-major lux values, computing summary stats.
    pub fn from_values(cols: u32, rows: u32, values: Vec<f64>) -> Self {
        let (mut min, mut max, mut sum) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
        for &v in &values {
            min = min.min(v);
            max = max.max(v);
            sum += v;
        }
        let n = values.len().max(1) as f64;
        LuxGrid {
            cols,
            rows,
            min: if min.is_finite() { min } else { 0.0 },
            max: if max.is_finite() { max } else { 0.0 },
            avg: sum / n,
            values,
        }
    }
}

/// Immutable scene context shared across all sensor points (read-only → `Sync`).
struct Ctx<'a> {
    scene: &'a RtScene,
    luminaires: &'a [LuminaireInstance],
    profiles: &'a HashMap<String, IesProfile>,
    materials: &'a [Material],
    settings: &'a RayTracingSettings,
}

impl Ctx<'_> {
    fn reflectance(&self, id: MaterialId) -> f64 {
        self.materials
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.reflectance as f64)
            .unwrap_or(0.5)
    }
}

fn v3(v: crate::engine::geometry::Vertex) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// Luminous intensity (candela) a luminaire emits toward `point`, applying the
/// IES distribution, the luminaire's azimuth rotation, and its dimming.
///
/// Convention: the luminaire's photometric nadir (vertical angle 0°) points
/// down −Z; horizontal 0° is +X, increasing toward +Y.
fn intensity_toward(prof: &IesProfile, lum: &LuminaireInstance, point: Vec3) -> f64 {
    let d = point - v3(lum.position);
    let dist = d.length();
    if dist < 1e-6 {
        return 0.0;
    }
    let dir = d / dist;
    let gamma = (-dir.z).clamp(-1.0, 1.0).acos().to_degrees() as f64;
    let phi = (dir.y.atan2(dir.x).to_degrees() as f64) - lum.rotation_deg as f64;
    prof.intensity(gamma, phi) * lum.dimming as f64
}

/// Direct illuminance (lux) at a surface point with the given outward `normal`.
fn direct(ctx: &Ctx, point: Vec3, normal: Vec3) -> f64 {
    let mut e = 0.0;
    for lum in ctx.luminaires {
        let Some(prof) = ctx.profiles.get(&lum.profile) else {
            continue;
        };
        let lpos = v3(lum.position);
        let to_light = lpos - point;
        let dist = to_light.length();
        if dist < 1e-6 {
            continue;
        }
        let cos_inc = normal.dot(to_light / dist) as f64;
        if cos_inc <= 0.0 {
            continue;
        }
        let intensity = intensity_toward(prof, lum, point);
        if intensity <= 0.0 {
            continue;
        }
        if ctx.settings.shadows && ctx.scene.occluded(point + normal * EPS, lpos) {
            continue;
        }
        e += intensity * cos_inc / (dist as f64 * dist as f64);
    }
    e
}

/// Total illuminance (direct + up to `bounces` diffuse reflections) at a point.
fn illuminance(ctx: &Ctx, point: Vec3, normal: Vec3, bounces: u32, rng: &mut Rng) -> f64 {
    let e = direct(ctx, point, normal);
    if bounces == 0 {
        return e;
    }
    let n = ctx.settings.rays_per_point.max(1);
    let mut acc = 0.0;
    for _ in 0..n {
        let w = cosine_sample(normal, rng);
        let Some(hit) = ctx.scene.closest_hit(&Ray { o: point + normal * EPS, d: w }) else {
            continue;
        };
        let rho = ctx.reflectance(hit.material);
        if rho <= 0.0 {
            continue;
        }
        // Orient the hit surface's normal back toward the incoming ray.
        let wn = if hit.normal.dot(w) < 0.0 { hit.normal } else { -hit.normal };
        let e_surface = illuminance(ctx, hit.point, wn, bounces - 1, rng);
        acc += rho * e_surface / PI; // Lambertian exitant radiance.
    }
    // Cosine-weighted estimator for irradiance: E ≈ (π/N) Σ Lᵢ.
    e + acc * PI / n as f64
}

/// Convert the project's meshes into ray-tracer triangles.
fn build_tris(meshes: &[Mesh]) -> Vec<Tri> {
    let mut tris = Vec::new();
    for m in meshes {
        for t in &m.triangles {
            let (Some(a), Some(b), Some(c)) = (
                m.vertices.get(t.a as usize),
                m.vertices.get(t.b as usize),
                m.vertices.get(t.c as usize),
            ) else {
                continue;
            };
            tris.push(Tri { a: v3(*a), b: v3(*b), c: v3(*c), material: m.material });
        }
    }
    tris
}

/// Compute the lux grid for the whole project.
pub fn calculate(project: &Project) -> EngineResult<LuxGrid> {
    let plane = project
        .calc_plane
        .as_ref()
        .ok_or_else(|| EngineError::Geometry("no calculation plane defined".into()))?;

    let scene = RtScene::new(build_tris(&project.meshes));
    let ctx = Ctx {
        scene: &scene,
        luminaires: &project.luminaires,
        profiles: &project.profiles,
        materials: &project.materials,
        settings: &project.settings,
    };
    grid_for(&ctx, plane)
}

/// Evaluate every sensor point of `plane` in parallel. The calc plane faces up (+Z).
fn grid_for(ctx: &Ctx, plane: &CalculationPlane) -> EngineResult<LuxGrid> {
    let cols = plane.cols.max(1);
    let rows = plane.rows.max(1);
    let bounces = ctx.settings.max_bounces;
    let normal = Vec3::Z;
    let count = (cols * rows) as usize;

    let values: Vec<f64> = (0..count)
        .into_par_iter()
        .map(|i| {
            let (col, row) = (i as u32 % cols, i as u32 / cols);
            let p = v3(plane.sample_point(col, row));
            // Distinct, fixed seed per point → reproducible across runs.
            let mut rng = Rng::seeded((i as u64).wrapping_mul(0x9E3779B9_7F4A7C15) ^ 0xD1B54A3);
            illuminance(ctx, p, normal, bounces, &mut rng)
        })
        .collect();

    Ok(LuxGrid::from_values(cols, rows, values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::geometry::{box_room, CalculationPlane, Vertex};
    use crate::engine::ies::PhotometryType;
    use crate::model::LuminaireInstance;

    /// A uniform 1000 cd source over the whole lower hemisphere (0–90°).
    fn flat_1000cd() -> IesProfile {
        let va: Vec<f64> = (0..=90).map(|d| d as f64).collect();
        let candela = vec![vec![1000.0; va.len()]];
        IesProfile {
            name: "flat".into(),
            photometry: PhotometryType::C,
            lumens: -1.0,
            multiplier: 1.0,
            vertical_angles: va,
            horizontal_angles: vec![0.0],
            candela,
            watts: 0.0,
            width: 0.0,
            length: 0.0,
            height: 0.0,
        }
    }

    fn demo_project(bounces: u32) -> Project {
        let (w, d, h) = (4.0f32, 4.0f32, 3.0f32);
        let mut p = Project::default();
        p.meshes = box_room(w, d, h);
        p.profiles.insert("flat".into(), flat_1000cd());
        p.luminaires = vec![LuminaireInstance {
            id: 1,
            profile: "flat".into(),
            position: Vertex::new(w / 2.0, d / 2.0, h),
            rotation_deg: 0.0,
            dimming: 1.0,
        }];
        p.calc_plane = Some(CalculationPlane {
            origin: Vertex::new(0.0, 0.0, 0.0),
            width: w,
            depth: d,
            cols: 24,
            rows: 24,
        });
        p.settings.max_bounces = bounces;
        p
    }

    #[test]
    fn direct_center_matches_inverse_square() {
        // Light 3 m above the floor centre, 1000 cd straight down → E = I/d² = 111 lx.
        let grid = calculate(&demo_project(0)).unwrap();
        assert!(
            (grid.max - 1000.0 / 9.0).abs() < 6.0,
            "peak direct lux {} should be ≈111",
            grid.max
        );
    }

    #[test]
    fn indirect_adds_reflected_light() {
        let direct = calculate(&demo_project(0)).unwrap();
        let bounced = calculate(&demo_project(1)).unwrap();
        // Reflected wall/ceiling light can only add to the floor illuminance.
        assert!(
            bounced.avg > direct.avg * 1.02,
            "indirect avg {} should exceed direct avg {}",
            bounced.avg,
            direct.avg
        );
    }
}
