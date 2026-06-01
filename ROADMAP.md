# RUST_CAD — Project Roadmap

> Living document. Update at the start of every new slice. The three
> reference docs (`Variables.md`, `Dobject_DXF.md`, `Dobject_Properties.md`)
> are the *contracts*; this file tracks **what we're working on, why, and
> how we plan to get there**.

---

## Where we are now (2026-06-01)

**Slice M.1 + M.2 — Trim / Extend: ● DONE.** Two-basket flow per the
`feedback_rust_cad_trim_extend_selection_model` memory. New
`EdgMod` SYSVAR (default ON) controls whether cutting / boundary
edges are treated as their infinite extensions for "imaginary
intersections" — works exactly like AutoCAD's EDGEMODE 1. The user's
main `selection` is stashed at session start and restored at finalise
or cancel, so the trim cutter basket is genuinely independent. Wand-
drag mode for the target-pick phase is deferred to v2; v1 is
single-click per target. Kernel: `Geom::trim_at` (Line / Arc /
EllipseArc; Circle / Ellipse error pending 2-pick v2), `Geom::extend_to`
(Line / Arc), `extended_for_edgemode` helper. 4 new kernel tests
covering the cutter cascade and the EdgMod imaginary-intersection path.

**Slice J — Editing operations: ● DONE.** `Geom::rotated()`,
`Geom::scaled()`, `Geom::mirrored()` added to the kernel (delegating
methods on `DObject` preserve style + handle). Six new commands wired
into the parser: `copy`, `rotate`, `scale`, `mirror`, `delete`,
`undo`. Each uses the QueuedOp pattern: if nothing is selected, the
command opens a selection session and re-enters the op when Enter
finalises. Interactive flows (pivot + reference + target clicks) live
in new state machines per op. **Snapshot-based undo** — every mutation
clones the Document onto a 64-deep stack; `undo` pops and restores.
4 new geom transform tests; 89 tests passing workspace-wide.

**Slice I — .rsm binary format: ● DONE.** Hand-rolled little-endian
versioned format in `cad_io::rsm`. Lossless `Document` round-trip
including every Geom variant, every layer/linetype/pen field, and
**stable handles** (DXF can't preserve our handles — RSM does). No
external deps, no compression in v1 (~64 B per Line, well under 100 KB
for 1000 lines). Wired into the same `open <path.rsm>` / `save
<path.rsm>` commands as DXF — file dispatcher picks by extension. 6
RSM round-trip tests.

**Slice H — cad_io / DXF: ● DONE.** New `cad_io` crate with
`dxf::read_dxf(&str) -> Document` and `dxf::write_dxf(&Document) -> String`.
Round-trips all current `Geom` variants (Line / Circle / Arc / Ellipse /
EllipseArc / Point / LWPolyline) plus the LAYER and LTYPE tables. Files
written by RUST_CAD open cleanly in LibreCAD. Wired into cad_app as
`open <path.dxf>` and `save <path.dxf>` commands on the command line —
no native file dialog yet. 9 DXF round-trip tests.

**Slice E (Point + Polyline only): ● DONE.** `Geom` enum gains two new
variants — `Point { location, style, size }` (AutoCAD POINT) and
`Polyline { vertices: Vec<PolyVertex>, closed }` (LWPOLYLINE; bulge
field on PolyVertex accepted but renderer treats every segment as
straight today). Drafting tools wired to the toolbar (`point`,
`pline`); polyline draws on click and finishes on Enter (`c` Enter
closes). `From<Point>` / `From<Polyline>` impls keep ergonomic
construction. All snap kinds extended (END snaps polyline vertices,
MID snaps every segment midpoint, NEA/PER project onto each segment,
Point acts as its own END snap). 70 tests still passing.

Text / MText / DimRotated are deliberately deferred — they need new
tables (TextStyleTable, DimStyleTable) which are slices of their own.

**Slice D — Entity Info panel: ● DONE.** Egui dock with two modes:
single-Dobject (full geometry breakdown + editable Layer / Visibility /
Color / Linetype / LinetypeScale / Lineweight) and multi-selection
(Geom-type counts + bulk layer reassign, bulk show/hide, bulk
"ByLayer reset"). Combos for Layer/Linetype/Lineweight are populated
live from `Document.layers` / `linetypes` so editing one is reflected
everywhere.

**Slice C — Pen palette: ● DONE.** Egui dock to the left of the Layer
panel; toggle from the toolbar ("pens ▾/▸"). 7 default presets in
`Document.pens` (a new `PenTable` in cad_kernel) — ByLayer, Red/Green/Blue
0.25 mm, Heavy black 0.7 mm, Dashed gray, Dash-dot center. Each row
shows a color swatch + name + linetype/lineweight description, with an
"apply" button that rewrites `style.color / linetype / lineweight` on
every Dobject in the current selection.

**Slice B — Layer panel: ● DONE.** Egui dock at left side; toggle from
toolbar ("layers ▾/▸"). Per-layer visibility / freeze / lock, color
swatch (egui color picker), double-click name to rename, active-layer
radio. Toolbar buttons: ➕ add (auto-named `Layer1`, `Layer2`, …) and
🗑 delete (active layer; Dobjects on it reassigned to layer "0"; layer
"0" is reserved and cannot be deleted). Demo layers "WALLS" (red) and
"HIDDEN" (green, invisible) created at startup so the panel is
non-empty.

**Slice A — Property foundation: ● DONE.**

Workspace builds clean, 70 tests pass.

| What landed | Files |
|---|---|
| `Color { ByLayer / ByBlock / Aci / TrueColor }` + `resolve_color()` chain | `cad_kernel/src/color.rs` |
| `Lineweight { ByLayer / ByBlock / Default / Custom(mm) }` + resolver | `cad_kernel/src/lineweight.rs` |
| `Linetype` + `LinetypeTable` (Continuous / Dashed / DashDot built-ins) | `cad_kernel/src/linetype.rs` |
| `Layer` + `LayerTable` (layer "0" reserved, can't delete) | `cad_kernel/src/layer.rs` |
| `Style` struct (layer + color + linetype + linetype_scale + lineweight + visible) | `cad_kernel/src/style.rs` |
| `DObject` struct = `geom: Geom` + `style: Style` + `handle: Handle` | `cad_kernel/src/dobject.rs` |
| `Document` container = dobjects + layers + linetypes | `cad_kernel/src/document.rs` |
| Rename: existing `DObject` enum → `Geom` enum across the whole workspace | (149 refs swept) |
| Renderer resolves `Color::ByLayer` + honours `style.visible` + `layer.visible/frozen` | `cad_app/src/app.rs` |

**Slice B — Layer panel: ○ NEXT.** Egui dock equivalent to LibreCAD's
`qg_layerwidget` — add / rename / delete layers, visibility / lock / freeze
toggles, click to set active. First visible UI deliverable on top of the
foundation.

---

## North-star objectives

1. **A pure-Rust 2D CAD math workbench** that scales to millions of
   Dobjects. No webview, no Qt. eframe (egui + glow) gives us a GL context
   on the main thread and zero IPC.
2. **Bring LibreCAD's QT panels in, but more complete.** Layer / Pen /
   Blocks / Library Browser / Entity Info / Command Line / UCS / Named
   Views — each as an egui dock with feature parity at minimum.
3. **AutoCAD-grade interop.** DXF round-trip via the `dxf…` / `dob…` /
   `xd…` group-code dictionary in `Dobject_DXF.md`. Eventually our own
   binary `.rsm` format for AutoRASM-native fast load/save.
4. **AutoCAD-feel settings.** User-Environment Settings (the SYSVAR
   analog) with cryptic short names like `SpTGSZ`, `GrpEnb`, surfaced in
   a settings window and persisted to `~/.config/rust_cad/user_env.txt`.

---

## How we implement — foundation-first, slice-by-slice

**The rule**: every behavior toggle or hardcoded constant goes into
`UserEnv` *first* (with a row in `Variables.md`), then gets wired.
Every Dobject type lands as a `Geom` variant *after* the property model
is in place so it inherits layer/color/linetype/lineweight for free.

### Slice progression

| Slice | Status | Scope | First visible-to-user moment |
|-------|--------|-------|------------------------------|
| **A. Property foundation** (kernel) | ● Done | Layer/Linetype/Color/Lineweight types, Style struct, DObject wrapper, Document container, renderer resolves ByLayer | (internals — visible to next slice) |
| **B. Layer panel** (UI) | ● Done | Egui dock — list/add/rename/delete/freeze/lock/visibility/active | Yes — first new panel |
| **C. Pen palette** (UI) | ● Done | Egui dock — pen presets (color + linetype + lineweight bundles), "Apply to selection" | Yes |
| **D. Entity Info panel** (UI) | ● Done | Read-only / partially-editable property inspector for current selection | Yes |
| **E. New Dobject types** | ◐ Partial | `DobjectPoint` ● + `DobjectPolyline` ● done. `Text` / `MText` / `DimRotated` deferred — they need `TextStyleTable` / `DimStyleTable` first (each is its own slice). | Yes — two new shapes |
| **F. Block table + Block panel** | ○ | `BlockTable` on `Document`; INSERT references; egui Blocks dock | Yes |
| **G. UCS / Named Views / Library Browser / Command Line panel** | ○ | Lighter dependencies, can land in any order | Yes |
| **H. `cad_io` (DXF reader / writer)** | ● Done | Round-trips LINE / CIRCLE / ARC / ELLIPSE / ELLIPSE_ARC / POINT / LWPOLYLINE; LAYER + LTYPE tables. `open` / `save` commands on cmd line. File dialog (rfd) is a small follow-up. | Yes — `open file.dxf` / `save file.dxf` |
| **I. `.rsm` binary format (AutoRASM-native)** | ● Done | Hand-rolled LE binary, versioned header, lossless. Handle preservation; no deps. | Yes — `open file.rsm` / `save file.rsm` |
| **J. Editing operations** | ● Done | copy / rotate / scale / mirror / delete / undo. All consume selection via QueuedOp pattern; snapshot-based undo stack (64-deep). | Yes |

### Operating principles

- **Kernel changes before UI changes.** Every UI panel reads from a kernel
  table that already exists. Slice B can build the Layer panel because
  Slice A landed `LayerTable`.
- **Document model is THE container.** New tables (blocks, text styles,
  dim styles, ucs, named views) get added as `Document` fields. Nothing
  lives loose on `CadApp`.
- **Common properties live on `DObject`, not on each variant.** Adding
  a new `Geom` variant must not require touching style infrastructure.
  This is the architectural payoff of Slice A.
- **Three docs are the contracts** — keep in sync as types evolve:
  - [`Variables.md`](Variables.md) — user-settable SYSVARS
  - [`Dobject_DXF.md`](Dobject_DXF.md) — file-format I/O dictionary
  - [`Dobject_Properties.md`](Dobject_Properties.md) — in-memory property model
- **Snap / spatial / intersect API split**: pure-geom helpers take `&Geom`,
  index-returning APIs (`find_snap`, `UniformGrid::build`) take `&[DObject]`.
- **Cad_snap dual maintenance**: changes to `cad_kernel::snap` public API
  must update `cad_snap` re-exports, example, README in the same change.

---

## Edit / Modify actions — extended roadmap (slices K–N)

Slice J landed the core 6 (move / copy / rotate / scale / mirror / delete + undo).
Following slices extend the editor in tiers — every action consumes the
basket via the existing select engine (basket-first, command on top), so
selection and editing stay orthogonal.

### Slice K — Simple (this run)

| # | Action | What it does | Inputs | Status |
|---|---|---|---|---|
| K.1 | **redo** | Re-apply the last undone op (mirror of undo) | (none) | ● Done |
| K.2 | **matchprop** / **mp** | Copy style (layer + color + linetype + lineweight + visibility) from a clicked source to every dobject in the basket | 1 click on source | ● Done |
| K.3 | **reverse** / **rev** | Flip direction of every selected Line / Arc / EllipseArc / Polyline | (none) | ● Done |
| K.4 | **chlayer** / **cl** | Bulk-set basket's layer to the active layer | (none) | ● Done |

### Slice L — Medium (this run)

| # | Action | What it does | Inputs | Status |
|---|---|---|---|---|
| L.1 | **offset** / **o** | Parallel copy at distance d on side of click. Line / Circle / Arc in v1; Ellipse / Polyline politely skipped | typed distance + side click | ● Done |
| L.2 | **lengthen** / **len** | Delta-mode only in v1: extend length of selected Line / Arc / EllipseArc by signed delta; click side to extend | typed delta + side click | ● Done |
| L.3 | **break** / **br** | For each dobject in basket: project click onto the curve and split. Line → two Lines, Arc → two Arcs, Polyline → two Polylines. Circle requires 2 clicks (v2) — v1 errors gracefully | 1 click on the cut point | ● Done |
| L.4 | **align** | Move + rotate the basket so source pair (s1, s2) maps to target pair (t1, t2). No scale in v1 | 4 clicks | ● Done |
| L.5 | **stretch** | Crossing window selects which vertices move; clicked base/dest gives the delta | crossing window + 2 clicks | ● Done |

### Slice M — Complex (M.1 + M.2 done; M.3–M.5 queued)

| # | Action | Spec | Status |
|---|---|---|---|
| M.1 | **trim** / **tr** | Two-basket flow per `feedback_rust_cad_trim_extend_selection_model` memory: prompt "Select cutting edges" → user picks via the project's existing select engine → Enter confirms cutting-edge basket → prompt "Pick targets" → click each target to trim. **`EdgMod` SYSVAR (ON/OFF)** controls infinite-extension intersections. Main editing basket stashed and restored. **No LibreCAD selection-idiom inheritance.** Wand-drag mode deferred to v2 — single-click in v1. | ● Done |
| M.2 | **extend** / **ex** | Symmetric: boundary basket → click each target end to extend toward the nearest boundary intersection (same `EdgMod`). | ● Done |
| M.3 | **fillet** | Tangent arc between two curves at radius | typed r + 2 clicks |
| M.4 | **chamfer** | Bevel between two lines with two distances | typed d1, d2 + 2 line clicks |
| M.5 | **join** | Merge collinear lines / coincident polylines / arcs sharing center+radius | (none) |

### Slice N — Strange / Exotic (deferred indefinitely)

PEDIT (per-vertex polyline edit) · explode · stretch-by-grip · polar
array · path array · group / ungroup. Each is its own slice when a real
need surfaces.

---

## Crate layout

| Crate | Role |
|-------|------|
| `cad_kernel` | Geometry primitives, intersection math, snap engine, spatial index, parser, the new property model (`color`, `lineweight`, `linetype`, `layer`, `style`, `dobject`, `document`). Zero UI deps. |
| `cad_app` | egui front-end. Pure visualization + command dispatch + interactive draw tools. All math comes from cad_kernel. |
| `cad_snap` | Thin facade over `cad_kernel::snap` for distributing the snap engine as a library. Has its own README + example. |
| `cad_cli` | Headless REPL — pipe commands in, get a structured intersection report out. For verifying the math line-by-line. |

---

## Naming conventions

| Concept | Rule |
|---------|------|
| Drafting object type | `DObject` struct (geom + style + handle). Was an enum pre-Slice-A; now a struct. |
| Inner geometry enum | `Geom` (Line, Circle, Arc, Ellipse, EllipseArc, …) |
| Field / variable | `dobject` / `dobjects` (snake_case) |
| Variant dispatch | `match &d.geom { Geom::Line(l) => …, … }` |
| Storage | `Document { dobjects, layers, linetypes }` |
| **SYSVAR identifier** | cryptic 5–7 char mixed-case no-underscore: `SpTGSZ`, `GrpEnb`, `AtDlgM`. ONLY for `UserEnv` fields. |
| **Regular kernel/app fields** | idiomatic Rust `snake_case`: `start_angle`, `sweep_param`, `dobjects`, `selection`. |
| **DXF dictionary prefixes** | `dxf…` (structural), `dob…` (Dobject common), `xd…` (extended data). |
| **External Dobject naming** (docs only) | `DobjectLine`, `DobjectCircle`, … — the conceptual name. In code the struct is `Line`/`Circle` nested in `Geom`. |

---

## Deferred / parked

- **Spline** — math-heavy; deferred indefinitely. Plan in `Dobject_Properties.md`.
- **3D types** (SubDMesh, NurbsSurface, 3D Solid) — deferred until RUST_CAD goes 3D.
- **Niche entity types** (REGION / MLINE / HELIX / UNDERLAY / FIELD / LIGHT / CAMERA / etc.) — tracked in `Dobject_Properties.md` "Possibly missing" table. Parked, no action.
- **Code cleanups** (e.g. rename `Line.a`/`Line.b` → `start`/`end`) — `~/.claude/...memory/project_rust_cad_future_cleanups.md`. Own pass, never inside a feature PR.

---

## What "save and commit" means here

`RUST_CAD/` was not under git until now. Initial commit captures the
state at the end of Slice A (everything above is in that snapshot).
Future slices each get their own commit, with the slice title and the
status changes in this doc as the message body.
