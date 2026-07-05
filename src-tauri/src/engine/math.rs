//! Shared math primitives.
//!
//! Geometry uses `glam` f32 types (ample precision at room scale, in metres).
//! Photometric scalars (candela, lux, lumens) use `f64`.
pub use glam::{Mat4, Vec2, Vec3};

/// Point-source illuminance via the inverse-square + cosine law.
///
/// * `intensity` — luminous intensity toward the point (candela).
/// * `distance` — metres from source to point.
/// * `cos_incidence` — cosine of the angle between the incoming ray and the
///   receiving surface's normal (Lambert's cosine law).
///
/// Returns illuminance in lux. This is the kernel of the Phase 3.1 direct pass:
/// `E = I(θ,ψ)·cos(ε) / d²`.
#[inline]
pub fn point_illuminance(intensity: f64, distance: f64, cos_incidence: f64) -> f64 {
    if distance <= f64::EPSILON || cos_incidence <= 0.0 {
        return 0.0;
    }
    intensity * cos_incidence / (distance * distance)
}
