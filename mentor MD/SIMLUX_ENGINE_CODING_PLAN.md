# SIMLUX — Engine Coding Plan (reconciled, ordered, live)

> **For**: Coding Agent · **Set**: 2026-07-10 · **Status**: LIVE (updated as slices land)
>
> This is the *buildable backlog*. It reconciles the two governing specs into one
> ordered sequence with per-slice acceptance tests, grounded in the **actual**
> `cad_light` code (not the aspirational docs). Read the specs for the *why*; read
> this for *what to type next and how to prove it*.
>
> Governing specs (authoritative for design):
> - `SIMLUX_SYSTEM_ARCHITECTURE.md` — the narrow-waist: producers → `cad_scene` → consumers.
> - `SIMLUX_CALC_ENGINE_PLAN.md` — the engine backbone: one field evaluator, metrics as projections.
> - `SIMLUX_DIALUX_PLAN.md` — product direction + the seams + Decision 2 (radiosity).
> - `SIMLUX_SCENE_AND_DAYLIGHT_PLAN.md` — scene/import/render/daylight.

---

## 0. Where the engine actually is today (verified from source)

`cad_light` = **1,143 lines, 6 files, 6 passing tests**. A real working engine:

| File | Does | Note |
|------|------|------|
| `ies.rs` | IES LM-63 parse (A/B/C, **TILT=NONE only**) + bilinear `intensity(γ,φ)` | concrete `IesProfile` |
| `rt.rs` | Möller–Trumbore + BVH + cosine sampling + RNG | reuse for shadows **and** form factors |
| `calc.rs` | direct (inv-sq·cos·shadow) **+ recursive Monte-Carlo indirect**, rayon | `calculate(...) → LuxGrid` |
| `extrude.rs` | `Document → Vec<Mesh>`, `extrude_handles` (subset) | Path-A producer |
| `types.rs` | `Luminaire`, `Material`, `CalcPlane`, `LuxGrid`, `RaySettings` | — |

**Key truths that shape the plan:**
- **Zero seams exist in code.** `Photometry` / `IndirectSolver` / `ComplianceStandard` / `SolarPosition` / `SkyModel` / `Scene` are all conceptual today.
- The calc's `direct()` and `illuminance()` **already take a `normal`** — but `calculate()` hardcodes `Vec3::Z`. The field evaluator is *half-present*; slice 1 formalizes it and exposes non-horizontal normals.
- Luminaires are a **side-list**; `U₀` is computed in the **UI**, not the engine (so not authoritative).

---

## 1. Reconciling the two build orders

Both docs open with a "slice 1". They are **not** in conflict — they are different axes:

- Architecture slice 1 = **`cad_scene`** (the input *waist* every consumer reads).
- Calc slice 1 = **field evaluator** (the *internal* shape of the calc).

They are orthogonal: the evaluator refactor changes *how a point is evaluated*; `cad_scene` changes *what geometry feeds the calc*. **We sequence the field evaluator first** because it is the smallest provable change (guarded by the 6 existing tests), it unlocks the whole metric catalog immediately, and it does not touch the input plumbing that `cad_scene` will later replace. `cad_scene` is the very next keystone.

---

## 2. The ordered backlog

Legend: ☐ todo · ◐ in progress · ☑ done. Sizes S/M/L.

### Phase 0 — Calc backbone (engine-internal, provable now)
- **S1 ◐ Field evaluator** *(S, `cad_light/calc.rs`+`types.rs`)* — formalize
  `evaluate_illuminance(point, normal)` as the one core; add a `ReceiverNormal`
  rule (Horizontal / Vertical / Custom); add `calculate_receiver(...)`; make
  `calculate(...)` the horizontal special case. **No new physics.**
  *Done-when:* 6 existing tests pass unchanged; new tests prove Horizontal ≡ legacy
  and a vertical receiver under a downlight reads well below horizontal.

### Phase 1 — The narrow waist (architecture keystone)
- **S2 ☐ `cad_scene` crate** *(M, NEW)* — `Scene`, `SceneElement { derived, material, generator }`,
  `Generator::{ExtrudeFromPlan, Solid, Imported}`, `MaterialRef`. Pure data.
- **S3 ☐ Route Path A through the waist** *(M, `cad_app`+`cad_light`)* — extrude
  output → `SceneElement(Generator::ExtrudeFromPlan)`; point the calc at
  `cad_scene` instead of raw `room_meshes`. No behaviour change; the seam becomes real.

### Phase 2 — Metric surface → verdict (the DIALux jump; no solver change)
- **S4 ☐ Metric catalog + grid stats** *(M, `cad_light/metrics.rs` NEW)* — vertical,
  perpendicular, camera/custom (point-normal), then cylindrical / semi-cyl /
  hemispherical (azimuth integration over §4 evaluator). Per-object Ē / min / max /
  **U₀ in the engine** (move it out of the UI).
- **S5 ☐ LDT parser behind `Photometry` seam** *(M, `cad_light/photometry.rs`+`ldt.rs`)* —
  `enum Photometry { Ies, Ldt }` (serde-friendly, not `Box<dyn>`); both emit one
  `intensity(C,γ)`. `HashMap<String, IesProfile>` → `HashMap<String, Photometry>`.
- **S6 ☐ EN 12464-1 compliance behind `ComplianceStandard`** *(L)* — space-type target
  rows (Ēm, U₀, UGR_L, Ra, Ez), task/surround/background zones, **maintenance factor**,
  pass/fail verdict.

### Phase 3 — Physics + scene depth (behind seams; MC stays default)
- **S7 ☐ `cad_solid` (Path B) + 3D-adjust** *(L, NEW `cad_solid`)* — csgrs solids,
  booleans (cutouts/apertures), emit the same `SceneElement`s; adjust edits the
  `Generator` then re-derives.
- **S8 ☐ Radiosity behind `IndirectSolver` + CIE-171 harness** *(L, `cad_light/radiosity.rs`)* —
  reuse BVH for form factors; **don't double-count direct**; flip default only on
  validation parity. Then **UGR** (surface luminance falls out).
- **S9 ☐ Background execution + presets** *(S, `cad_app`)* — off the UI thread
  (`Background_Ops_Pattern`), progress + cancel, draft/standard/high presets.

### Phase 4 — Natural light + render (lands against today's engine)
- **S10 ☐ Site + SunClock + `SolarPosition` + sun-path diagram** *(M, NEW `cad_daylight`)*.
- **S11 ☐ glTF import → Obstruction meshes** *(M)* — **Y-up→Z-up mandatory**.
- **S12 ☐ Daylight factor (`SkyModel`: CIE overcast) via Role=Opening apertures** *(L)*.
- **S13 ☐ Render behind `Renderer` (Cycles headless, `cad_render` feature-gate)** *(L)*.

---

## 3. Standing rules (from the specs — binding on every slice)
1. **Seams as `enum`s, default = today's code, zero behaviour change** before adding a 2nd impl.
2. **Build the light field once; every metric is a projection** (`SIMLUX_CALC_ENGINE_PLAN §4`).
3. **Consumers read only `cad_scene`; producers never read consumers** (`SIMLUX_SYSTEM_ARCHITECTURE §4`).
4. **Calc of record ≠ render.** IES-only for the numbers; render is presentation.
5. **`cad_kernel` / `cad_io` untouched** — SIMLUX state in the sidecar.
6. **Nothing ships without a validation number** (`Accuracy_Test_Plan.md`).
7. Push target = **dokkandar `origin`** only; HSI upstream is read-only.

---

## 4. Litmus tests (catch a wrong turn early)
- New metric that isn't a receiver-normal rule into §4 or a glare case → **routed wrong**.
- A consumer that imports a producer (or vice-versa) → **route the data through `cad_scene`**.
- A 2nd photometry/solver/standard that isn't behind its seam → **stop, add the seam first**.
