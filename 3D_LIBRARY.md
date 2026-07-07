# SIMLUX — 3D Handling Library (living document)

**Purpose.** The interactive‑3D / free‑solid‑modeling track needs a real 3D
geometry library (the `cad_kernel` is strictly 2D — `Vec2 {x, y}` — and the 3D
today is only an *extrusion* of the plan). This file records **which library we
use for 3D handling, why, its license, and how it plugs in.** It is maintained
**as the 3D code is written** (user directive 2026‑07‑08).

> Policy this obeys: **permissive licence only, pure‑Rust, no Qt/C++, new work in
> INDEPENDENT crates — never touch `cad_kernel` (2D) or the existing render.**

---

## What SIMLUX already has for 3D (not starting from zero)
| Piece | Where | Role |
|-------|-------|------|
| Triangle mesh types (`Mesh`, `Triangle`, `Vertex`) | `cad_light/src/types.rs` | the render + raytrace unit |
| Extruder (2D plan → wall/floor/ceiling meshes) | `cad_light/src/extrude.rs` | plan → 3D surfaces |
| Ray tracer (Möller–Trumbore + BVH) | `cad_light/src/rt.rs` | lux + (reusable for 3D picking) |
| Offscreen‑FBO 3D renderer | `cad_app/src/light3d.rs` | draws meshes via **glow** |
| 3D camera math | `cad_app` (**glam** 0.29) | orbit / MVP |

So the new library must **produce triangle meshes** we can hand to the existing
glow renderer + ray tracer, and (ideally) give **ray‑casting** for 3D picking.

## What the 3D‑modeling track needs
- **Solids + boolean ops** — extrude rooms, **cut** door/window openings
  (difference), merge volumes (union).
- **Extrude / revolve / sweep / loft** — architectural massing.
- **3D pick / ray‑cast** — click in the 3D viewport → hit geometry (the
  interactive‑viewport feature).
- **Mesh I/O** — STL/OBJ in/out for imported furniture, fixtures.
- Pure Rust, permissive, mesh‑compatible with our pipeline.

---

## Candidates (verified 2026‑07‑08)
| Library | Licence | Pure Rust | Model | Verdict |
|---------|---------|-----------|-------|---------|
| **csgrs** | **MIT** | ✅ (nalgebra · parry · geo) | mesh CSG (BSP) | ✅ **CHOSEN** |
| **truck** | Apache‑2.0 | ✅ (cgmath · wgpu) | B‑rep + NURBS | ⏳ future (exact surfaces) |
| Fornjot | 0BSD | ✅ | B‑rep | ❌ **discontinued** |
| manifold‑csg | MIT wrapper | ❌ C++ `manifold` | mesh CSG | ❌ pure‑Rust policy |

### Choice: **csgrs** (MIT)
- **Mesh‑first** → drops straight into our `Mesh`/glow/raytrace pipeline (truck
  is B‑rep/NURBS and ships a **wgpu** renderer + **cgmath** — an impedance
  mismatch with our **glow/glam**; we'd only use its headless geometry crates,
  and it's heavier than we need for architectural rooms).
- Has exactly the ops we need: **union / difference / intersection / xor**,
  **extrude / vector‑extrude / revolve / sweep / loft**, **STL + DXF** I/O.
- Built on the **Dimforge** stack (**nalgebra**, **parry3d**, `geo`) — and
  **parry3d gives us ray‑casting for free** → the 3D‑viewport pick step.
- 100 % Rust, no C/C++.

### `truck` kept as the future option
If we later need **exact curved surfaces / true NURBS B‑rep** (not tessellated
approximations), `truck` (Apache‑2.0) is the pure‑Rust path — used headless
(`truck-geometry` / `-topology` / `-modeling` / `-shapeops` / `-meshalgo`),
tessellated to our `Mesh`, ignoring `truck-platform`/`-rendimpl` (wgpu).

---

## Integration plan (independent crate)
```
cad_solid  (NEW crate)                 // depends on: csgrs
   └─ wraps csgrs solids/booleans
   └─ tessellate() -> cad_light::Mesh   // hand off to glow + ray tracer
   └─ picking via parry3d ray-cast      // for the interactive 3D viewport
cad_kernel  (UNCHANGED — stays 2D)
cad_light   (UNCHANGED — render/raytrace)
cad_app     (adds the 3D-viewport input + active-viewport routing)
```
- **Math boundary:** we use **glam**, csgrs uses **nalgebra** — convert
  `Vec3`/`Mat4` at the crate edge (cheap, isolated in `cad_solid`).
- Nothing in `cad_kernel` or the existing render changes; upstream merges stay clean.

## Open questions (fill in as code lands)
- Exact csgrs version + feature flags (f32 vs f64, parallel).
- Do 3D solids persist to `.rsm`, or stay a derived/importable layer?
- Picking precision: parry3d ray‑cast vs reusing `cad_light`'s BVH.

## Sources
- truck — https://github.com/ricosjp/truck (Apache‑2.0)
- csgrs — https://github.com/timschmidt/csgrs (MIT)
- Fornjot (discontinued) — https://github.com/hannobraun/fornjot
