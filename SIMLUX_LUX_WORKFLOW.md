# SIMLUX — Guided Lux-Plan Workflow (roadmap)

**Goal:** from a *completed drawing*, the user walks a clear pipeline and ends
with a **lux plan**:

```
Completed drawing
  → [1] Import DObjects (pick the room geometry)
  → [2] Extrude to 3D room (line/wall → surfaces; closed → floor+ceiling)
  → [3] Insert IES file + place luminaires
  → [4] Calculate lux
  → [5] Lux plan  (heatmap + stats + export)
```

The **engine already exists** (`cad_light`: IES parse, ray-traced lux, extruder;
`light.rs`/`light3d.rs`: panels, 2D heatmap, 3D viewport — P1–P5 in
`SIMLUX_STATUS.md`). This roadmap adds the **deliberate, step-based SIMLUX
section** around it, with *import-dobjects → extrude* as a first-class step
instead of the current implicit whole-document extrude at Calculate time.

Legend:  🟢 exists (reuse) · 🟡 exists but change · 🔴 new

---

## Data model (2026-07-08 — the architecture)
Two halves: **2D = drafting** (easy plan management), **3D = modeling + lux
calc**. State that isn't kernel geometry lives **SIMLUX-side** (keeps
`cad_kernel` untouched) and persists alongside the drawing.

- **Layers drive 3D.** Every layer can be flagged **"use for 3D"** — a
  SIMLUX-side per-layer flag keyed by layer id, NOT a new kernel `Layer` field.
  The room is drafted, then shifted onto a dedicated **SIMLUX** layer; any
  flagged layer extrudes into the 3D model at its per-layer height. *(This
  generalises the earlier "import layer into a room list" — the flag IS the
  room membership; supersedes B1's list model.)*
- **Luminaires are LUX blocks.** A luminaire is a real 2D dobject — a **block
  reference** on the plan (move/snap/copy like any block), tagged as a **LUX
  block**. Position + rotation come from the block instance; the calc **derives**
  `Luminaire`s from the LUX-block instances in the document *(replaces the
  `LightState.luminaires` side-list)*.
- **IES entered once, referenced many.** IES files load into a library
  (`profiles: name → IesProfile`, persisted with the drawing). A LUX block stores
  only the **IES name** (a reference), never a copy. N blocks → 1 IES entry —
  the block define-once / insert-many pattern applied to photometry.

Reuse: `cad_kernel::block` (BlockTable/Block/BlockRef) + the existing IES-by-name
reference. New (SIMLUX-side): the layer-3D flags, the LUX-block tag + IES-name
attribute, deriving luminaires from blocks, and persistence of all three.

## Phase A — SIMLUX workflow shell
Make the pipeline explicit and stateful instead of implicit.
- **A1** 🔴 Add a **Room** to `LightState`: `room_handles: Vec<Handle>` (the
  imported set) + `room_meshes` (extruded), distinct from "whole doc".
- **A2** 🔴 Turn the `SIMLUX` menu into a **step panel**: Import → Extrude →
  Fixtures → Calculate → Plan, each showing done/blocked state.

## Phase B — Import DObjects  *(your ask #1)*
- **B1** 🔴 **"Import from drawing"** — capture the current **selection** into the
  room set (store handles). *Decision D1 below.*
- **B2** 🔴 Imported summary (N lines / walls / polylines) + highlight them on
  the plan; re-import / clear.
- **B3** 🟡 Keep the room set stable across edits (handle-based, survives redraw).

## Phase C — Extrude line/wall  *(your ask #2)*
- **C1** 🟡 Extrude **only the imported set**, not the whole doc — extend
  `cad_light::extrude` to take a handle subset.
- **C2** 🟡 Extrude controls: **wall height**, floor/base Z; closed paths →
  floor + ceiling caps; open lines → single wall surfaces.
- **C3** 🟢→🟡 Live **3D preview** while adjusting height (reuse `light3d`).
- **C4** 🔴 (later) per-layer / per-group height (partitions vs full-height walls).

## Phase D — Insert IES + fixtures
- **D1** 🔴 **"Insert IES file…"** via the native file dialog (reuse the app's
  `FileDialog` infra) → parse → add to the IES library. Keep paste-path fallback.
- **D2** 🟢 Place fixtures by click (exists) + 🔴 **grid/array** placement.
- **D3** 🟢 Per-fixture IES / mount height / rotation / dimming + list (exists) — polish.

## Phase E — Calculate
- **E1** 🟢 Ray-traced direct+indirect lux (exists).
- **E2** 🔴 **Background** the calc for big grids (cancel + progress) — see
  `Background_Ops_Pattern.md`.

## Phase F — Lux plan (the deliverable)
- **F1** 🟡 Formalize the **Lux Plan** output: heatmap + legend + avg/min/max +
  **uniformity Uo** (mostly exists — present as *the* result).
- **F2** 🔴 (optional) **isolux contour** lines.
- **F3** 🔴 **Export**: PNG of the plan · CSV of the grid · one-page PDF report
  (title, fixtures, IES names, stats).

## Phase G — Scene persistence
- **G1** 🔴 Save/load the lighting scene (room set, heights, fixtures, IES refs,
  materials) inside the `.rsm` / document.

---

## Key decisions — RESOLVED 2026-07-08
- **D1 — Room source = a chosen LAYER.** Import captures all dobjects on the
  selected layer(s) into the room set, tagged by layer. (Not selection/whole-doc.)
- **D2 — Per-LAYER extrude height.** Each imported layer carries its own height
  (walls 3 m, partitions 1.2 m, …). Layers drive both import *and* height.
- **D3 — Interactive heatmap only.** No file export for now (Phase F3 dropped).
- **D4 — LUX block ↔ IES is TYPE-level.** One LUX block definition per fixture,
  IES assigned to the definition; every inserted instance shares that one IES.
- **D5 — Persistence = sidecar.** `drawing.simlux.json` beside `drawing.rsm`
  (serde) holds all SIMLUX state (layer-3D flags + heights, LUX-block-def↔IES
  map, the IES library, materials, ray settings). `cad_kernel` / `cad_io`
  stay UNTOUCHED. Load on open, write on save, keyed by the drawing path.

→ Consequence: the room model is **layer-grouped**. `room` = a set of
`{layer_name, height, handles[]}` groups; extrude iterates groups, each at its
own height. Import UI = a **layer picker** (list of the document's layers with a
height field per layer).

## Near-term focus
Per the ask, start with **Phase B (import dobjects) + Phase C (extrude
line/wall)** on top of the existing engine — that turns the vague "Calculate
extrudes everything" into the deliberate SIMLUX pipeline.
