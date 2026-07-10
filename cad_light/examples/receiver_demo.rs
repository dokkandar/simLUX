//! Slice-1 demo: the field evaluator measures the SAME light field onto different
//! receiver orientations. Build a small box room with one ceiling downlight, then
//! print horizontal / vertical / custom illuminance side by side so you can watch
//! the receiver-normal rule bite.
//!
//! Run:  cargo run -p cad_light --example receiver_demo
use std::collections::HashMap;

use cad_light::{
    box_room, calculate_receiver, default_materials, CalcPlane, IesProfile, Luminaire,
    PhotometryType, RaySettings, ReceiverNormal, Vertex,
};

/// A trivial 1000 cd isotropic-ish downlight (flat over 0–90° vertical).
fn flat_1000cd() -> IesProfile {
    let va: Vec<f64> = (0..=90).map(|d| d as f64).collect();
    IesProfile {
        name: "flat".into(),
        photometry: PhotometryType::C,
        lumens: -1.0,
        multiplier: 1.0,
        candela: vec![vec![1000.0; va.len()]],
        vertical_angles: va,
        horizontal_angles: vec![0.0],
        watts: 0.0,
        width: 0.0,
        length: 0.0,
        height: 0.0,
    }
}

fn main() {
    let (w, d, h) = (4.0f32, 4.0f32, 3.0f32);
    let meshes = box_room(w, d, h);

    let mut profiles = HashMap::new();
    profiles.insert("flat".to_string(), flat_1000cd());

    let lums = vec![Luminaire {
        id: 1,
        profile: "flat".into(),
        position: Vertex::new(w / 2.0, d / 2.0, h),
        rotation_deg: 0.0,
        dimming: 1.0,
    }];

    let plane = CalcPlane { origin: Vertex::new(0.0, 0.0, 0.0), width: w, depth: d, cols: 24, rows: 24 };
    let mats = default_materials();
    let settings = RaySettings { rays_per_point: 64, max_bounces: 1, shadows: true };

    let cases = [
        ("Horizontal (Eh, work plane)", ReceiverNormal::Horizontal),
        ("Vertical   (Ev, faces +X)", ReceiverNormal::Vertical { azimuth_deg: 0.0 }),
        ("Custom     (45deg up to +X)", ReceiverNormal::Custom { x: 1.0, y: 0.0, z: 1.0 }),
    ];

    println!("Box room {w}x{d}x{h} m, one 1000 cd downlight at the ceiling centre.\n");
    println!("{:<30} {:>8} {:>8} {:>8} {:>6}", "receiver", "avg", "min", "max", "Uo");
    println!("{}", "-".repeat(66));
    for (label, recv) in cases {
        let g = calculate_receiver(&meshes, &lums, &profiles, &mats, &plane, &settings, recv);
        let uo = if g.avg > 0.0 { g.min / g.avg } else { 0.0 };
        println!("{label:<30} {:>8.1} {:>8.1} {:>8.1} {:>6.2}", g.avg, g.min, g.max, uo);
    }
    println!(
        "\nSame field, three receiver normals: horizontal reads highest under a\n\
         downlight, vertical grazes it, custom sits between. That is Slice 1."
    );
}
