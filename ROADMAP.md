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

## Phase 3.1 — Direct + first-bounce indirect ✅ (mostly)

_Goal: import an IES + DXF, define a calc plane, place a luminaire, get a lux
grid rendered as a heatmap. Indirect (wall reflection) was pulled in early._

- [x] **IES parser** (`engine/ies`): LM-63 (TILT=NONE), angle arrays + candela
      block; stores lumens, multiplier, watts, dimensions. Tested vs `T1.ies`.
- [x] **Candela interpolation**: bilinear over `candela[h][v]` in
      `IesProfile::intensity` (0 outside measured vertical range).
- [x] **DXF loader** (`engine/dxf`): reuses `cad_io::dxf::read_dxf` from
      `dokkandar/Auto_RASM`; flattens Line/Arc/Circle/Polyline/Spline → `Line2`.
- [x] **Ray tracer** (`engine/rt`): Möller–Trumbore + median-split BVH +
      cosine-weighted sampling + deterministic RNG. Tested vs brute force.
- [x] **Lux engine** (`engine/calc`): direct `E = Σ I(θ,ψ)·cos(ε)/d²` with
      shadow rays, plus Monte-Carlo one-bounce indirect. rayon-parallel.
- [x] **Frontend**: room meshes, luminaire markers, and a `LuxGrid` heatmap on
      the calc plane; fit-to-scene camera. `Demo Room` builds a test box.
- [ ] **DXF → calc plane / real rooms**: today the demo room is a `box_room`;
      wiring a plane + walls from imported DXF is Phase 3.2.
- [ ] **Interactive placement**: drag a luminaire / size the plane in-canvas
      (currently via `add_luminaire` / `add_demo_room` commands).

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
