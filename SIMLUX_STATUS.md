# SIMLUX — Status / What's Done

A native **egui** physically‑based lighting (lux) designer, built **on top of the
Auto_RASM CAD environment**. You draw the room with a real CAD workspace
(command line, object snap, modify commands), give it a height, place IES
luminaires, and SIMLUX computes and visualises the illuminance (lux) on the
floor — in 2D on the plan and in a 3D view.

This repository was migrated from the earlier Tauri/React prototype to this
native egui app (commit *"Migrate SIMLUX to a native egui lighting designer"*).

---

## Architecture

Cargo workspace (reused Auto_RASM CAD crates + one new lighting crate):

| Crate | Role |
|-------|------|
| `cad_kernel` | 2D geometry, `Document`, command parser, snap |
| `cad_io` | DXF read (floor‑plan underlay) |
| `cad_wall`, `cad_raster`, `cad_param`, `cad_nurbs`, `cad_snap`, `cad_cli` | CAD support |
| `cad_app` | The egui/glow application (drawing env + **SIMLUX UI**) |
| **`cad_light`** | **NEW** — the pure‑Rust lighting engine |

### `cad_light` (the engine)
- **IES LM‑63** photometry parser (`ies.rs`) with bilinear intensity lookup.
- **Ray tracer** (`rt.rs`): Möller–Trumbore triangle intersection + median‑split
  **BVH**, deterministic RNG, cosine‑weighted hemisphere sampling.
- **Lux calculation** (`calc.rs`): direct illuminance (inverse‑square + cosine,
  shadow‑tested) **plus** Monte‑Carlo indirect (Lambertian bounces), `rayon`‑parallel.
- **Extruder** (`extrude.rs`): turns a `cad_kernel::Document` into 3D surfaces —
  one drafted line/wall → one vertical surface; closed path → + floor + ceiling.
- UI‑agnostic. **6 unit tests pass** (IES, BVH vs brute force, direct ≈ inverse‑square, indirect adds light).

### SIMLUX UI (inside `cad_app`)
- `light.rs` — `LightState` (IES profiles, materials, room height, ray settings,
  luminaires, computed `LuxGrid`) + the **Light panel** + false‑colour ramp.
- `light3d.rs` — an **offscreen‑FBO glow renderer** for the 3D viewport.

---

## Features (done)

### Ribbon — independent SIMLUX section
A top‑level **`SIMLUX`** menu in the ribbon (between *Tools* and *Help*):
⚡ **Calculate lux** · Light panel · 3D view · **Place luminaire** · Display toggles · Import IES.

### P1 — Light panel + 2D heatmap
- Import IES (path + Load) or use the built‑in cosine downlight.
- Editable floor / wall / ceiling **reflectances**, **room height**, **work‑plane
  height**, grid cell size, **quality** (indirect bounces, rays/point, shadows).
- **Calculate** → 2D **false‑colour lux overlay** on the plan, tracking pan/zoom.
- Readouts: **average / min / max / uniformity Uo** + gradient legend.

### P2 — 3D viewport
- Docked, resizable right panel. Room rendered via an **offscreen FBO**
  (colour + depth) composited into the panel — no dependency on the window
  depth buffer, and egui's GL state is fully preserved.
- **Orbit** (drag) + **zoom** (scroll); ceiling hidden so you can look in.

### P3 — 3D floor heatmap + markers
- Floor coloured by **per‑cell lux** using the same ramp as the 2D overlay.
- **Luminaire markers**: gold cross‑hairs on the 2D plan, octahedrons in 3D.
- 3D panel colorbar + avg/min/max readout.

### P4 — Luminaire placement
- **Place mode** → click the 2D plan to drop fixtures at the mount height, using
  the active IES profile. Esc / untoggle to stop.
- **Fixtures list** in the Light panel: id + position, per‑fixture **dimming**,
  delete, clear all. Multiple fittings all feed one Calculate.
- The Photometry combo is the **IES library** (built‑in + any imported `.ies`).

### P5 — Polish + branding
- **Colour‑scale** control: auto (grid max) or a fixed lux ceiling for comparable runs.
- **`simlux`‑branded binary**; window title "SIMLUX — Lighting Designer".

---

## How to run

```
cargo run -p cad_app          # binary is named `simlux`
# or
cargo build -p cad_app && ./target/debug/simlux.exe
```

## Typical workflow

1. Draw a **closed room** (polyline / walls) in the CAD workspace.
2. **SIMLUX ▸ Place luminaire** → click point(s) on the plan (set mount height).
3. **SIMLUX ▸ ⚡ Calculate lux** → heatmap on the plan + results in the Light panel.
4. Tick **3D view** to orbit the lit room (floor heatmap + fixture markers).

---

## Known limitations / next steps

- `Calculate` runs on the UI thread (grids clamped ≤ 64×64, sub‑second for normal
  rooms). Move to a worker thread when pushing resolution up.
- IES import is a path field (no native file dialog dependency yet).
- DXF floor‑plan underlay is read but not yet wired as a pure reference layer.
- The repo currently carries the full Auto_RASM workspace (all crates + docs);
  product‑repo pruning is a later cleanup.

---

## Tech notes

- egui / eframe 0.30, glow **0.16** (note: `tex_image_2d` takes `PixelUnpackData`).
- Engine world is **Z‑up**; geometry `f32`, photometry `f64`; IES nadir = −Z.
- 3D uses raw glow with a per‑vertex‑colour shader + a blit shader for the FBO.
