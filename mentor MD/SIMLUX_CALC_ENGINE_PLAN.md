# SIMLUX — The Lux Calculation Engine: The Whole Story & Plan

> **For**: Coding Agent · **Maintainer**: supervisor (not a code author) · **Set**: 2026-07-10
> **Scope**: everything the light-calculation engine (`cad_light`) does — from a
> luminaire's photometry to a pass/fail on EN 12464-1. This is the "fix it all
> before moving on" plan for the calc part. Calc-object list is taken from the
> DIALux "Calculation objects" panel in `DIALUX_SCREENSHOTS/`.
>
> Reads with: `SIMLUX_DIALUX_PLAN.md` (Decision 2 = hybrid raytrace+radiosity;
> the seams), `SIMLUX_SYSTEM_ARCHITECTURE.md` (the engine is a **consumer** of
> `cad_scene`), `SIMLUX_SCENE_AND_DAYLIGHT_PLAN.md` (daylight side),
> `Accuracy_Test_Plan.md` (validation).

---

## 1. The whole story (end-to-end narrative)

A **luminaire** is a photometric body: an IES/LDT file is a cloud of luminous
intensities in **candela**, one value per direction. We place it in the scene at a
pose, dim it, and age it with a maintenance factor. Then we ask: *how much light
lands where, and does it meet the standard?* Five acts:

- **Act 1 — Direct.** From each measurement point, look back at every luminaire. If
  a **shadow ray** through the BVH is clear, that luminaire contributes
  `I(dir)/d² · cos(incidence)` to the point. Sum over luminaires → **direct
  illuminance**. Exact, fast — this is what the engine does *today*.
- **Act 2 — Bounce (interreflection).** Rooms are bright in the shadows because
  light reflects. Chop surfaces into **patches**, seed each with (direct light ×
  reflectance), then let patches trade light — the **radiosity solve** — until it
  converges. Each patch ends with a radiosity → a **luminance**. Gather at the
  point → **indirect illuminance**. Bonus: surface luminance falls out for free —
  which **glare** needs. *(Decision 2: hybrid — direct by ray trace, indirect by
  radiosity. Today this is Monte-Carlo; radiosity replaces it behind the seam.)*
- **Act 3 — Daylight (optional).** The sky is a giant area source, the sun a sharp
  one. Through the **apertures**, sky patches + sun add illuminance and bounce too.
  Under a standard overcast sky we also report the **Daylight Factor**.
- **Act 4 — Measure.** The light field now exists everywhere. But "illuminance" is
  **not one number** — it depends on which way the receiver faces. A desk faces up
  (**horizontal**), a whiteboard sideways (**vertical**), a face is a little
  cylinder (**cylindrical**). Each metric is a **projection of the same field onto
  a receiver orientation**. **Glare (UGR)** is the exception: it asks how bright the
  luminaires *look* to an eye against the background.
- **Act 5 — Judge.** Compare measured **Ēm, U₀, UGR, Ez, Ra** against the
  **EN 12464-1** row for this space type → **pass/fail**. That verdict is the
  product.

**The design consequence (the backbone):** build the **light field once**, then
add every metric as a thin **projection**. Do not write a separate calculator per
metric — that is the refactor trap. One evaluator, many receiver rules, plus one
glare evaluator. §4 makes this concrete.

---

## 2. Inputs (what the engine reads)

All geometry/materials come from **`cad_scene`** (the engine never asks *how* a wall
was authored — §1 of the architecture doc):

- **Surfaces** — triangulated, each with **reflectance ρ** (from the material's
  diffuse albedo; the render's PBR channels are NOT used here — calc stays honest).
- **Luminaires** — from **LUX-block instances**: pose (position + orientation),
  **IES/LDT reference**, **dimming**, per-fixture light-loss. *(Imported glTF lamps
  never feed the calc — IES-only, per the scene doc.)*
- **Photometry** — IES/LDT parsed into ONE internal intensity distribution behind
  the **`Photometry`** seam (bilinear lookup `I(C, γ)`).
- **Apertures** — windows/skylights (the `cad_solid` voids) = the daylight entry.
- **Calc objects** — the measurement surfaces/grids + **metric type** + **receiver
  rule** + **height offset** (+ step width / viewing angles for glare).
- **Environment** — **maintenance factor**, sky+sun (if daylight on).
- **Settings** — grid cell size, patch density, convergence tolerance, bounce
  budget, quality preset.

---

## 3. The physics pipeline (stages)

| Stage | What | Where | Notes |
|-------|------|-------|-------|
| **0 Prep** | tessellate scene → patches; build **BVH**; bake each luminaire's oriented photometry | `rt.rs`, new `patches.rs` | BVH already exists |
| **1 Direct** | per calc point: `Σ_L I_L(dir)/d² · cos(ε) · V · LLF` | `calc.rs` | exists (horizontal only today) |
| **2 Indirect** | radiosity: `B_i = ρ_i(E0_i + Σ_j F_ij B_j)`; solve; gather at points | new `radiosity.rs` behind `IndirectSolver` | MC today → radiosity (Decision 2). Reuse BVH for form-factor occlusion. **Don't double-count** the direct term |
| **3 Daylight** | CIE/Perez sky (Tregenza patches) + sun through apertures; bounce | `cad_daylight` + Stage 2 | optional |
| **4 Combine** | `E = MF · (E_direct + E_indirect + E_daylight)` | `calc.rs` | apply maintenance factor once |
| **5 Metrics** | project the field per calc-object; evaluate glare | `metrics.rs` (new) | §4–§5 |

**Formulae of record:**
- Direct: `E = Σ_L [ I_L(θ,φ) / d² ] · cos(ε) · V(P,L) · LLF_L`, `cos(ε)=(P→L)·n_receiver`.
- Radiosity: `B_i = E0_i·ρ_i + ρ_i Σ_j F_ij B_j`; patch **luminance** `L_i = B_i/π`.
- Interreflected illuminance at P: gather patch radiances over the visible hemisphere.

---

## 4. The backbone: one field evaluator + receiver rules

Model a **`Receiver`** = a point + a **normal rule**, and a single core:

```
evaluate_illuminance(point, normal) -> lux
   = direct(point, normal)        // per-luminaire projection onto `normal`
   + indirect(point, normal)      // gather from radiosity patches
   + daylight(point, normal)      // if enabled
```

Because direct is computed per-luminaire (we know each luminaire's direction), we
can project onto **any** `normal` cheaply — so every illuminance metric is just a
choice of normal (and, for cylindrical/hemispherical, an integration over normals).
This is the "build the field once" payoff.

**Glare is separate** — it needs source **luminance** as seen from an eye, not an
illuminance projection. A `evaluate_glare(observer, view_dir)` uses the direct
luminaire luminances + the radiosity background luminance `Lb`.

---

## 5. Metrics catalog — every DIALux calc object → how it's derived

| DIALux calc object | Kind | How SIMLUX derives it |
|--------------------|------|-----------------------|
| **Horizontal illuminance** Eh | illuminance | `evaluate(P, n=+Z)` — the classic work-plane lux |
| **Vertical illuminance** Ev | illuminance | `evaluate(P, n=horizontal dir)` — walls, faces |
| **Perpendicular illuminance** | illuminance | `evaluate(P, n=surface normal)` |
| **Perpendicular (Adaptive)** | illuminance | `n` adapts per point (dominant/curved-surface normal) |
| **Cylindrical** Ez | integral | mean vertical over all azimuths: `(1/2π)∮ Ev(φ)dφ` — modelling / facial recognition (EN 12464-1) |
| **Semi-cylindrical** Esc | integral | mean vertical over 180° about a direction |
| **Hemispherical** | integral | mean illuminance over the hemisphere at P |
| **Camera-oriented** | illuminance | `evaluate(P, n=camera view dir)` |
| **Custom direction** | illuminance | `evaluate(P, n=given rotation)` |
| **UGR** (Unified Glare Rating) | glare | `UGR = 8·log10[ (0.25/Lb) Σ (L_i²·ω_i / p_i²) ]` — observer pos, eye height ~1.2 m, **viewing angles from→to at a step width** = rotate the observer, tabulate/worst; `p` = Guth position index |
| **Glare Rating** RG (outdoor) | glare | CIE 112 / EN 12464-2: `GR = 27 + 24·log10(Lvl/Lve^0.9)`; **simplified (EN 12464)** vs **exact (CIE 112)** switch |
| **Daylight factor** DF | ratio | daylight-only `E_indoor / E_exterior,unobstructed × 100` under CIE overcast sky |

Every row above is either a **receiver-normal rule** (± an azimuth integration) into
the §4 evaluator, or the **glare evaluator**. That is the whole metric surface —
adding "camera-oriented" or "custom" is a few lines, not a new engine.

**Grid statistics** (per calc object): Ē (average), E_min, E_max, **U₀ = E_min/Ē**
(uniformity), and for glare the tabulated UGR/GR. These are the numbers the report
and the EN 12464-1 verdict consume.

---

## 6. The compliance layer (Act 5)

Behind the **`ComplianceStandard`** seam (EN 12464-1 first):
- A **space type** (DIALux "utilisation profile") selects a **target row**:
  `Ēm, U₀_min, UGR_L (max), Ra (min), Ez` (+ task / surrounding / background zones).
- The engine reports **measured vs required** per zone → **pass/fail**.
- **Maintenance factor** method (DIN 5035 / CIE 97 / IESNA) sets the MF applied in
  Stage 4.
- Outdoor obtrusive-light (EN 12464-2) reuses the same shape, later.

The metric engine computes numbers **once**; the standard only **judges** them —
so ANSI/IESNA later is a table, not a new calc path.

---

## 7. Seams & placement
- Engine is a **consumer of `cad_scene`** — never reaches into a producer.
- **`Photometry`** (IES + LDT → one intensity table).
- **`IndirectSolver`** (MC today → radiosity; keep MC as default until radiosity
  passes the same validation cases, then flip — no big-bang).
- **`ComplianceStandard`** (EN 12464-1 → IESNA later).
- Metrics live behind the §4 evaluator so calc-objects are thin.

---

## 8. Execution model
- **Off the UI thread.** Real grids exceed today's ≤64×64 clamp — run the calc as a
  background op (`Background_Ops_Pattern.md`: pure on worker, apply on main,
  cancel drops the result) with **progress + cancel**.
- **`rayon`-parallel** over calc points (direct) and patches (radiosity).
- **BVH** for both shadow rays and form-factor occlusion (one structure).
- **Convergence**, not fixed iterations: radiosity stops on an energy-delta
  tolerance; log patch/iteration counts (overflow warnings per `AGENTS.md` §11).
- **Quality presets** (draft/standard/high) tune grid, patch density, bounce budget.

---

## 9. Validation (GATING — nothing ships without a number)
- **Analytical:** point source → inverse-square; infinite uniform diffuse surface →
  closed form; single-bounce box vs hand calc.
- **CIE 171:2006** test cases — the industry suite; the bar radiosity must clear
  before it becomes default.
- **Cross-check** identical scenes against DIALux; record the delta.
- Wire all of the above into CI via `Accuracy_Test_Plan.md`.

---

## 10. Current state → what's owed

| Piece | Today (`cad_light`) | Target |
|-------|---------------------|--------|
| Photometry | IES LM-63 (bilinear) | + **LDT** behind `Photometry` |
| Direct | ✅ inverse-square + cos + shadow | keep; extend to any receiver normal |
| Indirect | Monte-Carlo Lambertian bounces | **radiosity** behind `IndirectSolver` (+ surface luminance) |
| Daylight | ❌ | CIE/Perez sky + sun through apertures; **DF** |
| Metrics | **horizontal only** (LuxGrid: Ē/min/max/U₀) | the **full §5 catalog** via the field evaluator |
| Glare | ❌ | **UGR** + outdoor **RG** |
| Compliance | ❌ | **EN 12464-1** zones + verdict + MF method |
| Execution | UI thread, ≤64×64 | background + cancel + presets |
| Validation | 6 unit tests | CIE 171 + analytical + DIALux cross-check in CI |

---

## 11. Build order (slices — fix the calc, in order)
1. **Refactor to the field evaluator** (§4): make the existing direct+MC path go
   through `evaluate_illuminance(P, normal)`. No new physics — just the backbone.
   *Immediately unlocks vertical / camera / custom / perpendicular metrics.*
2. **Metric catalog + grid stats** (§5): cylindrical, semi-cyl, hemispherical
   (azimuth integration); per-object Ē/min/max/U₀.
3. **LDT parser** behind `Photometry`.
4. **EN 12464-1 compliance** (§6): space-type target rows + verdict + MF method.
5. **UGR** (needs surface luminance — pairs with radiosity).
6. **Radiosity** behind `IndirectSolver`, validated, then flip default (§7).
7. **Background execution** + presets (§8).
8. **Daylight** (§3.3) + **Daylight factor**; **outdoor RG** last.

Slices 1–4 turn SIMLUX from "horizontal heatmap" into "a standards verdict" without
touching the solver. 5–8 deepen the physics behind seams that already exist.

**Litmus test:** any new metric that isn't either a receiver-normal rule into §4 or
a case in the glare evaluator is being built wrong — route it through the field.
