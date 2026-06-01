# RUST_CAD — Project Roadmap

> Living document. Update at the start of every new slice. The three
> reference docs (`Variables.md`, `Dobject_DXF.md`, `Dobject_Properties.md`)
> are the *contracts*; this file tracks **what we're working on, why, and
> how we plan to get there**.

---

## Where we are now (2026-06-01)

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
| **H. `cad_io` (DXF reader / writer)** | ○ | Round-trip LINE / CIRCLE / ARC / ELLIPSE / ELLIPSE_ARC first; then per-entity dispatchers | Yes — open .dxf files |
| **I. `.rsm` binary format (AutoRASM-native)** | ○ | Fast load/save for big drawings; our own format | Yes |
| **J. Editing operations** | ○ | copy / rotate / scale / mirror / delete / undo. Each consumes selection via QueuedOp pattern | Yes |

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
