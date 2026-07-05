# SIMLUX Roadmap

Phased plan distilled from [docs/Guide-for-simLUX.txt](./docs/Guide-for-simLUX.txt).
Each phase is shippable on its own.

Legend: ✅ done · 🚧 in progress · ⬜ planned

## Workflow (the concept)

```
1. INSERT REFERENCE   DWG ─(dwgconv)→ DXF ┐
   (background only)   DXF ───────────────┴→ read_dxf → Line2  ── dimmed floor-plan underlay
                        (reference only — NOT lit, NOT 3D geometry)

2. DRAFT (2D)          Construction tab — a **command line** (reusing cad_kernel::parse)
                        + tools: line/polyline/rectangle/circle/arc/point/wall. Typed
                        coordinates (3,0 · @2,0 · @5<90) or clicks. → cad_kernel::Document
   modify               move · offset · trim · extend · fillet — planned

3. GIVE HEIGHT         extrude each drafted line/wall → a single vertical SURFACE
                        (a closed loop also gets a floor + ceiling) → 3D meshes

4. LIGHT               IES luminaires at ceiling → lux engine → heatmap
```

Rule: **one line → one surface** (no solid boxes). Reused from
`dokkandar/Auto_RASM`: `cad_io` (DXF read), `cad_kernel` (`parse` + `Command` +
`Document` + geometry). The command *orchestration* is reimplemented (Auto_RASM's
dispatch is native egui; `COMMAND_LINE.md` there is a design spec, unimplemented).
DWG import is a convert step (`tools/dwgconv`, C#/ACadSharp) — deferred.

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

## Phase 3.2 — Command-line 2D drafting & 3D scene 🚧

- [x] **Command environment**: `engine::draft` adopts `cad_kernel::Document` and
      reuses `cad_kernel::parse`; a stateful session (exec / pick / cancel) with
      prompts. Command-line bar with transcript.
- [x] **Draw commands**: line · polyline · rectangle · circle · arc · point ·
      wall — via typed command + coords (`3,0`, `@2,0`, `@5<90`) **or** clicks;
      `close` / `undo` keywords.
- [x] **Construction view** (SVG): pan, zoom-about-cursor, adaptive grid,
      snapping to entity nodes + grid; renders the Document; tool palette issues
      commands.
- [x] DXF = dimmed reference underlay (shared coordinate frame with 3D).
- [x] **Extrude** the Document: each line/wall → one surface; closed paths +
      circles get floor + ceiling; arcs/circles tessellated.
- [ ] **Modify commands**: move · copy · erase · offset · trim · extend · fillet
      · rotate · scale · mirror (kernel has them; need pick UX). Currently they
      parse and report "not yet".
- [ ] Select/grip editing; snap overrides (END/MID/CEN/INT/PER); dimensions,
      rulers, ortho/polar tracking; ellipse/spline; curved walls (`bulge`).

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
