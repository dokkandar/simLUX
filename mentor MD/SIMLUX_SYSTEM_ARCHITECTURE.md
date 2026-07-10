# SIMLUX — System Architecture: DIALux Sections → Libraries → Wiring

> **For**: Coding Agent · **Maintainer**: supervisor (not a code author) · **Set**: 2026-07-10
> **Purpose**: the *load-bearing* architecture. Map every DIALux-evo section to the
> SIMLUX crate responsible for it, and define **how they wire together so that
> calculation, 3D adjustment, render, and daylight never force a rewrite** when a
> new authoring path or library is added later.
>
> Companions: `SIMLUX_DIALUX_PLAN.md` (product + calc/render decisions),
> `SIMLUX_SCENE_AND_DAYLIGHT_PLAN.md` (import/render/daylight), `3D_LIBRARY.md`
> (csgrs), `ARCHITECTURE.md` (crate layering). Section list below is taken from the
> DIALux evo screenshots in `DIALUX_SCREENSHOTS/`.

**We are NOT copying DIALux.** We take its *section map* as a checklist of
capabilities, and implement them on our own architecture — the point of this doc.

---

## 1. The one decision that prevents the refactor surprise

The danger the user named: two different 2D→3D authoring paths, each wired
*directly* into calculation / adjustment / render. If every downstream feature
knows about *both* authoring paths, then adding a third path (or changing one)
means editing calc **and** render **and** daylight **and** adjust — a combinatorial
refactor. That is the "tremendous surprise" to avoid.

**The fix is a narrow-waist (hourglass) architecture.** Many producers, **one
canonical model**, many consumers. Nothing on the right ever reaches across to the
left.

```
  PRODUCERS (authoring, plural)              CANONICAL MODEL (one)           CONSUMERS (plural)
  ─────────────────────────────             ─────────────────────           ──────────────────
  Path A: 2D draft → extrude  ┐                                          ┌ Calc      (cad_light: direct + radiosity, IES/daylight)
     (cad_kernel + cad_light) │             ┌───────────────────┐        │ 3D Adjust (heights, cutouts, move — edits the Generator)
  Path B: space → CSG solid   ┼──produces──▶│  cad_scene::Scene │──feeds─┼ Render    (cad_render: Cycles / glow preview)
     (cad_solid / csgrs)      │             │  (derived 3D IR)  │        │ Daylight  (cad_daylight: sun + sky through apertures)
  Import: DWG / glTF / IFC    ┘             └───────────────────┘        └ Report    (ComplianceStandard: EN 12464-1 verdict)
     (cad_io / cad_import_gltf)
```

**Invariant (the whole point):** a consumer depends **only** on `cad_scene`. Calc
does not know whether a wall came from an extrude or a CSG solid. Add Path C
tomorrow → you write one new producer, touch **zero** consumers.

### 1.1 The hybrid link for calc + 3D-adjustment (the subtle part)
The two paths **stay separate for creation** but **converge at `cad_scene`** for
everything downstream. To make *adjustment* work without branching, every
`cad_scene` element carries its **Generator** (provenance) — the same
generator→derived pattern already established for smart objects (see
`Smart_Dobjects.md`):

```
SceneElement {
    derived: Mesh + topology,        // what calc / render / daylight consume (uniform)
    material: MaterialRef,           // ρ for calc, PBR for render (§4 Materials)
    generator: Generator,            // HOW it was made — the round-trip handle
}
enum Generator {
    ExtrudeFromPlan { layer/handles, base_z, height },   // Path A
    Solid          { csg_tree },                          // Path B
    Imported       { gltf_node },                         // furniture / props
}
```
- **Consumers read `derived` only** — uniform, path-agnostic.
- **3D-adjust edits the `Generator`, then re-derives `derived`.** Raise a Path-A
  room's height → re-extrude. Cut a window in a Path-B solid → re-run the CSG.
  Same UI gesture, correct math per path, because the path is remembered, not
  guessed.
- This is the **hybrid path**: one adjustment surface, one calc surface, one render
  surface — many creation surfaces underneath.

### 1.2 `cad_scene` is DERIVED, not a second source of truth
`cad_kernel::Document` stays the **2D authoring** source of truth. `cad_scene` is
the **derived 3D** projection (+ solids + imports + fixtures + calc-objects). It
does **not** replace the Document. *(A reviewed external draft made `cad_scene` a
parallel authoring graph — that was wrong. Here it is a downstream projection with
provenance back to its producer.)*

---

## 2. Crate map (who owns what)

| Crate | New? | Owns |
|-------|------|------|
| `cad_kernel` | exists | 2D geometry, `Document`, layers, blocks, snap — **Path A authoring + all 2D plan/annotation** |
| `cad_io` | exists | DXF read/write; **glTF import** folds in here or a sibling `cad_import_gltf` |
| `cad_light` | exists | **Photometry** (IES/LDT), ray tracer, **radiosity** (planned), extrude (**Path A**), lux calc, calc-objects |
| `cad_solid` | **NEW** | csgrs wrapper — **Path B** solids, booleans (cutouts/apertures), roofs, room elements, ceilings; parry3d ray-pick |
| `cad_scene` | **NEW** | **the narrow waist** — the derived 3D `Scene` IR + `Generator` + material refs; the only thing consumers depend on |
| `cad_daylight` | **NEW** | Site + `SolarPosition` (sun path) + `SkyModel` (CIE/Perez calc, Hosek–Wilkie render); daylight through apertures |
| `cad_render` | **NEW** | Cycles back-end (Apache-2.0, feature-gated) + translation `cad_scene → Cycles`; glow preview stays in `cad_app` |
| `cad_app` | exists | egui UI, glow viewport, SIMLUX state, sidecar persistence; **orchestrates producers → `cad_scene` → consumers** |

**Seams already defined** (keep them — they are how each consumer stays swappable):
`Photometry`, `IndirectSolver`, `ComplianceStandard`, `Renderer`, `SolarPosition`,
`SkyModel`. This doc adds the **`Scene` waist** that they all sit behind.

---

## 3. Every DIALux section → responsible crate + wiring

Grouped by role. "Producer" writes into `cad_scene`; "Consumer" reads it.

### 3.1 Project / import
| DIALux section | Crate | Wiring |
|----------------|-------|--------|
| Start / project (new, open, edit) | `cad_app` + sidecar | `.rsm` + `drawing.simlux.json` |
| Import **Plan** (DWG/DXF) | `cad_io` → `cad_kernel` | underlay for **Path A** |
| Import **glTF / Furniture / Objects** | `cad_import_gltf` → `cad_scene` | props/fixtures as `Generator::Imported` |
| Import **Luminaire** (IES/LDT) | `cad_light` (`Photometry`) | file → one intensity table |
| Import **Daylighting system** | `cad_daylight` | aperture + shading presets |
| Import **IFC** (BIM) | *parked* | heavy; not near-term — flag as future |

### 3.2 Space creation — THE TWO PATHS (producers)
| DIALux section | Path | Crate |
|----------------|------|-------|
| Storey / **room construction** (walls, contours) | **A** draft→extrude | `cad_kernel` + `cad_light::extrude` |
| **Spaces** (draw rect/circular/polygon space) | **B** space→solid | `cad_solid` (csgrs) |
| **Room elements** (beam, column, platform, ramp) | **B** | `cad_solid` |
| **Roofs** (flat/hip/gable/…) | **B** | `cad_solid` |
| **Ceilings** (suspended) | **B** | `cad_solid` → surface + plenum in `cad_scene` |
| **Cutout** (rect/circular/polygon holes) | **B** | `cad_solid` boolean difference |
| **Apertures** (windows / skylights) | **B** | `cad_solid` void **+** `cad_daylight` light-entry (the calc/daylight coupling point) |
| **Façade elements** (blinds / shading) | consumer-side | `cad_daylight` (transmittance) + `cad_scene` (geom) |

Both paths **emit `cad_scene::SceneElement`s** with the right `Generator`. That is
the convergence.

### 3.3 Scene dressing (producers → `cad_scene`)
| DIALux section | Crate | Wiring |
|----------------|-------|--------|
| **Furniture & objects** (arrangements, primitives) | `cad_scene` | Obstruction-role meshes (shadows in calc + material in render) |
| **Materials** (reflectance / colour / texture) | `cad_scene` material → **split** | **ρ → `cad_light`** (calc), **PBR → `cad_render`** (render). One material, two projections — never cross them |
| **Help lines / labels / dimensions** | `cad_kernel` | 2D plan annotation |
| **Copy & arrange** | `cad_kernel` (2D) / `cad_scene` (3D) | modify ops |
| **Views / Save image** | `cad_render` + `cad_app` | camera + render trigger |

### 3.4 Lighting (the artificial-light authority)
| DIALux section | Crate | Wiring |
|----------------|-------|--------|
| **Luminaires** (place, arrange, photometry) | `cad_light` | **LUX blocks** carry IES; the *only* photometric source for the calc |
| **Light scenes** (groups, dimming, emergency) | `cad_light` scene state | a scene = fixture set + dimming + sky ref |
| Light-scene **Daylight** (reference sky, date/time) | `cad_daylight` | couples the scene to sun+sky |

### 3.5 Calculation & compliance (consumers — the deliverable)
| DIALux "Calculation objects" | Crate | Wiring |
|------------------------------|-------|--------|
| Horizontal / Vertical / Perpendicular illuminance | `cad_light` calc surfaces | reads `cad_scene` + fixtures |
| **UGR** / Glare (RG, simplified EN12464 / exact CIE 112) | `cad_light` (needs surface **luminance** from radiosity) | ties Decision 2 (radiosity) to glare |
| Cylindrical / Semi-cyl / Hemispherical illuminance | `cad_light` | EN 12464-1 metrics |
| **Daylight factor** | `cad_light` + `cad_daylight` | CIE overcast sky through apertures |
| **Site**: utilisation profile, maintenance, obtrusive-light standard | `ComplianceStandard` (in/over `cad_light`) | EN 12464-1 target rows, MF multiplier, EN 12464-2 outdoor |

---

## 4. Wiring rules (the invariants — do not break these)
1. **Consumers depend only on `cad_scene`.** Never on `cad_kernel::Document`,
   `cad_solid` internals, or a specific `Generator` variant. This is what makes new
   authoring paths free.
2. **Producers write `cad_scene`; they never read consumers.** No back-references.
3. **3D-adjust edits the `Generator`, then re-derives.** Never edit `derived`
   geometry in place — it desyncs from its source (same lesson as smart objects).
4. **Material is one object with two projections** — ρ for calc, PBR for render.
   The render's PBR channels never leak into the calc (photometry stays honest).
5. **Apertures are the single calc↔daylight coupling** — a window is a `cad_solid`
   void *and* a `cad_daylight` entry. One object, referenced by both; not copied.
6. **Calc of record vs render** stays separated (per `SIMLUX_DIALUX_PLAN.md` §9 /
   `SCENE_AND_DAYLIGHT` §1) — `cad_scene` feeds both, but their outputs never mix.
7. **`cad_kernel` / `cad_io` untouched** for SIMLUX state — sidecar only.

---

## 5. Build order (so the waist exists before the consumers lean on it)
1. **Define `cad_scene`** — `Scene`, `SceneElement`, `Generator`, `MaterialRef`.
   Small, pure data. This is the keystone; build it first.
2. **Wire Path A into it** — `cad_light::extrude` output → `SceneElement` with
   `Generator::ExtrudeFromPlan`. (Path A already exists; just route it through the
   waist instead of a private `room_meshes`.)
3. **Point the calc at `cad_scene`** (not the raw Document) — no behaviour change,
   but now the seam is real.
4. **Stand up `cad_solid` (Path B)** emitting the same `SceneElement`s. Now two
   producers, zero consumer edits — proof the waist works.
5. **3D-adjust** on `Generator` + re-derive.
6. Consumers extend behind the waist: `cad_daylight`, `cad_render`, UGR/radiosity.

**Litmus test for any new feature:** "does this make a consumer import a producer,
or vice-versa?" If yes, it is routed wrong — put the data in `cad_scene` instead.

---

## 6. What this buys you
- Add a 3rd authoring path (e.g. IFC/BIM, or a parametric room wizard) → one
  producer, no calc/render/daylight edits.
- Swap csgrs, or swap Cycles, or swap the radiosity solver → isolated behind its
  seam; the `Scene` waist is unchanged.
- The two 2D→3D paths the user wants to **keep** stay first-class and independent,
  while calc / adjust / render / daylight see **one** model. That is the hybrid,
  and it is what stops the future refactor.
