# SIMLUX Roadmap

Phased plan distilled from [docs/Guide-for-simLUX.txt](./docs/Guide-for-simLUX.txt).
Each phase is shippable on its own. The current tree is the **scaffold**: every
module and command exists and compiles, engine functions return
`EngineError::NotImplemented` pointing back here.

Legend: ✅ done · 🚧 in progress · ⬜ planned

---

## Scaffold ✅

- Tauri v2 + React (react-three-fiber) + zustand app that builds and runs.
- Rust engine module tree: `ies`, `dxf`, `geometry`, `calc`, `math`, `model`.
- Command bridge (`engine_info`, `get_project`, `import_ies`, `load_dxf`,
  `calculate_lux`) with `AppState` (`Mutex<Project>`) and serialisable errors.
- 3D viewport shell: grid, orbit controls, DXF line rendering, heatmap-ready.

## Phase 3.1 — Direct-only calculation 🚧

_Goal: load a DXF, define one calc plane, place one IES luminaire, get a
direct-only lux grid rendered as a heatmap._

- [ ] **IES parser** (`engine/ies`): tokenize LM-63-2002, handle TILT, read
      angle arrays + candela block; store lumens, multiplier, dimensions.
- [ ] **Candela interpolation**: bilinear over `candela[h][v]` in
      `IesProfile::intensity`.
- [ ] **DXF loader** (`engine/dxf`): read `LWPOLYLINE` / `LINE` → `Vec<Line2>`
      (add the `dxf` crate).
- [ ] **Direct engine** (`engine/calc::calculate_direct`): per grid point,
      `E = Σ I(θ,ψ)·cos(ε) / d²`, with a single shadow ray per luminaire.
- [ ] **Frontend**: draw/size a calculation plane, place a light (XY + Z),
      render the returned `LuxGrid` as a heatmap on the plane.

## Phase 3.2 — Wall drawing & 3D scene ⬜

- [ ] Trace DXF lines into `Wall`s (thickness + height) grouped into `Room`s.
- [ ] Extrude + triangulate walls/floor/ceiling into `Mesh`es (`earcutr`).
- [ ] Shadow test upgraded to `ray vs. triangle` over all meshes, parallelised
      with `rayon`; add a BVH (`parry3d`) as scenes grow.

## Phase 3.3 — Radiosity & indirect light ⬜

- [ ] Per-mesh `Reflectance` (already on `Material`).
- [ ] Progressive radiosity: shoot from the highest unshot-radiosity surface,
      accumulate onto floor sensors and other surfaces; iterate to convergence.

## Phase 3.4 — Professional features ⬜

- [ ] Hundreds of luminaires (BVH mandatory).
- [ ] Luminaires as luminous areas (sub-source grids) for near-field accuracy.
- [ ] Colour: radiosity in `(R,G,B)` instead of scalar.
- [ ] Discrete calculation points; UGR / glare.
- [ ] Report export (CSV / PDF); `.lux` project save/load (serde is ready).

---

### Coordinate conventions

- **Plan space**: DXF `(x, y)` in metres.
- **World space**: `(x, up=y, -planeY=z)` — plan `+y` reads as "north".
- **Photometry**: geometry in `f32` (glam); candela/lux/lumens in `f64`.
