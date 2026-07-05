# SIMLUX Roadmap

Phased plan distilled from [docs/Guide-for-simLUX.txt](./docs/Guide-for-simLUX.txt).
Each phase is shippable on its own.

Legend: ✅ done · 🚧 in progress · ⬜ planned

## Workflow (the concept)

```
1. INSERT REFERENCE   DWG ─(dwgconv)→ DXF ┐
   (background only)   DXF ───────────────┴→ read_dxf → Line2  ── dimmed floor-plan underlay
                        (reference only — NOT lit, NOT 3D geometry)

2. DRAFT (2D)          Construction tab — Line / Polyline / Rectangle / Wall tools
                        on a top-down grid, with snapping. → WallSeg { start, end, thickness }
   modifiers            move · offset · (trim · extend · fillet — planned)

3. GIVE HEIGHT         extrude each drafted line/wall → a single vertical SURFACE
                        (a closed loop also gets a floor + ceiling) → 3D meshes

4. LIGHT               IES luminaires at ceiling → lux engine → heatmap
```

Rule: **one line → one surface** (no solid boxes). Reused from
`dokkandar/Auto_RASM`: `cad_io` (DXF read), `cad_kernel` (geometry types). The
tools are reimplemented in the web frontend (Auto_RASM's UI is native egui).
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

## Phase 3.2 — 2D drafting & 3D scene 🚧

- [x] **Construction tab**: a top-down 2D CAD view (SVG) — pan (drag), zoom
      (wheel, about cursor), adaptive grid, snapping to nodes + grid.
- [x] Drafting tools: **Select / Line / Polyline / Rectangle / Wall**.
- [x] DXF is a **dimmed reference underlay** (shared coordinate frame with 3D).
- [x] **Extrude**: each drafted line/wall → one vertical surface; a closed loop
      also gets floor + ceiling (ear-clip triangulation, handles L-shapes).
- [x] Modifiers: **move**, **offset** (commands). Ray tracer lights the result.
- [x] Tabbed layout (Construction / 3D & Light) + tool palette.
- [ ] Modifiers: **trim / extend / fillet** (reuse `Geom::trim_at/extend_to`,
      `fillet_geoms`) — need two-entity pick UX in the 2D view.
- [ ] Select tool: pick + move/delete drawn segments in-canvas.
- [ ] Dimensions, rulers, ortho/polar tracking; curved walls (`bulge`).

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
