# SIMLUX Roadmap

Phased plan distilled from [docs/Guide-for-simLUX.txt](./docs/Guide-for-simLUX.txt).
Each phase is shippable on its own.

Legend: ✅ done · 🚧 in progress · ⬜ planned

## Workflow (the concept)

```
1. INSERT REFERENCE   DWG ─(dwgconv)→ DXF ┐
   (background only)   DXF ───────────────┴→ read_dxf → Line2  ── dimmed floor-plan underlay
                        (reference only — NOT lit, NOT 3D geometry)

2. DRAW WALLS (2D)     trace over the underlay with the Wall tool →
                        WallSeg { start, end, thickness }
   modifiers            move · offset · (trim · extend · fillet — planned) · cope(=miter)

3. STITCH              cad_wall::solve_faces → mitred faces at shared nodes → room footprint

4. GIVE HEIGHT         extrude stitched walls + floor + ceiling to room height → 3D meshes

5. LIGHT               IES luminaires at ceiling → lux engine → heatmap
```

Reused from `dokkandar/Auto_RASM`: `cad_io` (DXF read), `cad_wall` (junction
miter), `cad_kernel` (Wall + geometry + modifiers). DWG import is a convert step
(`tools/dwgconv`, C#/ACadSharp) — deferred until DWG is needed.

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

## Phase 3.2 — Wall drawing & 3D scene 🚧

- [x] DXF demoted to a **dimmed reference underlay** (not lit, not 3D).
- [x] **Draw walls** interactively over the underlay (click-to-draw polyline,
      endpoint snapping to wall nodes + DXF endpoints; Esc to finish).
- [x] **Stitch** walls at junctions via `cad_wall::solve_faces` (mitre).
- [x] **Extrude** stitched walls + floor + ceiling to room height → `Mesh`es
      (ear-clip triangulation for the floor/ceiling — handles L-shapes).
- [x] Modifiers: **move**, **offset** (commands). Ray tracer already lights the
      resulting meshes (BVH, rayon).
- [ ] Modifiers: **trim / extend / fillet** (reuse `Geom::trim_at/extend_to`,
      `fillet_geoms`) — need two-entity pick UX.
- [ ] Curved walls (`bulge`) + wall styles/thickness table.
- [ ] Footprint from an open/branching wall set (currently a closed draw-order
      loop); inner-face floor instead of centreline footprint.

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
