# RUST_CAD — Hatch (command · boundary detection · patterns · all dependencies)

> Exhaustive handoff doc for the **HATCH** subsystem so another coding agent can
> reproduce it: the hatch dobject, the **BPOLY boundary tracer**, the **pattern
> system** (`.pat` loader + hardcoded catalog), the two creation flows
> (select-first + pick-point), rendering, the async worker, the debug window, and
> **every dependency**. Nothing here is meant to be skimmed.
>
> Files: `cad_kernel/src/{geom,patterns}.rs`, `cad_io/src/pat.rs`,
> `cad_app/src/hatch_trace.rs`, `cad_app/src/app.rs`, `assets/hatch/standard.pat`.
> Line numbers from `9a4bcc7`; grep the symbol if they drift. Reads with
> `MODIFY_GUIDE.md` (select-first), `SETTINGS.md` (HpMaxA/HpObjW), and the
> intersection notes.

---

## 0. Mental model

```
Geom::Hatch { boundary_handles: Vec<Handle>, pattern }   ← references its boundary by HANDLE; fill is DERIVED
   fill = resolve handles → loops (even-odd: outer, then islands) → clip the pattern to the loops, every frame

Two ways to make one:
  (A) SELECT-FIRST  — pick closed dobjects → one Hatch over them (even-odd islands)
  (B) PICK-POINT    — click inside a region → BPOLY tracer finds the enclosing boundary
       cheap path:  one closed dobject already contains the click → use it directly
       trace path:  ray-cast + loop-walk on a BACKGROUND WORKER (cancellable) → materialise loops
```

**Handle-referenced, not geometry-copied.** A hatch stores the *handles* of its
boundary dobjects; if a boundary moves, the hatch follows; if a handle no longer
resolves, that loop is silently dropped (the hatch shrinks gracefully). Hatch
**transforms are no-ops** (rotate/scale/mirror/translate) — it tracks its
boundaries. `bbox`/`distance_to_point` are conservative (ZERO / INFINITY) because
handles can't be resolved at the kernel-geometry level; the renderer resolves
them.

---

## 1. Data model (`cad_kernel/src/geom.rs:282`)

```rust
pub enum HatchPattern {
    Solid,                                       // fill the boundary with the dobject's colour
    Pattern { name: String, scale: f64, angle_deg: f64 },  // named catalog pattern + per-hatch transform
}
pub struct Hatch {
    pub boundary_handles: Vec<Handle>,  // loop order: outer first, then islands (even-odd at render)
    pub pattern: HatchPattern,
}
```
- `name` is a **case-insensitive** catalog key (e.g. `ANSI31`, `BRICK`). `scale`
  multiplies every family's spacing (1.0 = catalog default); `angle_deg` is added
  to every family's angle.
- `is_view_independent_bbox()` returns **true** (like BlockRef) → the spatial
  index keeps a hatch in every candidate set (its real extent is its boundaries').

---

## 2. Pattern system

### 2a. The hardcoded catalog (`cad_kernel/src/patterns.rs`) — what renders today
`lookup(name) -> Pattern` (case-insensitive) returns one of:
```rust
enum Pattern {
    Families(Vec<LineFamily>),                 // infinite parallel lines (ANSI31, NET, EARTH…)
    Tile { period_x, period_y, segments: Vec<PatternSegment>, circles: Vec<PatternCircle> },  // tiled finite cell (BRICK, TILE, CONCENTRIC…)
}
struct LineFamily { angle: f64 /*rad*/, base_x, base_y, spacing }   // spacing = perpendicular gap
struct PatternSegment { x1,y1,x2,y2 }   struct PatternCircle { cx,cy,radius }
```
`PATTERN_NAMES` (17): `SOLID, ANSI31, ANSI32, ANSI33, ANSI37, CROSS, NET, ANGLE,
BRICK, TILE, CONCRETE, EARTH, LINE, DOTS, DOUBLE, DASH, SQGRID, CONCENTRIC`.
BRICK/TILE/CONCENTRIC are **tiles** (BRICK/TILE derived from DXF references);
unknown name → `Pattern::empty()` (draws nothing).

### 2b. The `.pat` loader (`cad_io/src/pat.rs`) — infrastructure for AutoCAD patterns
`parse_pat(text) -> PatParse { patterns, warnings }` — never errors (malformed
lines → warnings). Format: `*NAME, description` header + line families
`angle, x-origin, y-origin, delta-x, delta-y [, dash1, dash2 …]` (`+`=dash
`-`=gap `0`=dot; none = solid; `;` comments). Produces:
```rust
struct PatLine    { angle: f64, base: (f64,f64), offset: (f64,f64), dashes: Vec<f64> }
struct PatPattern { name, description, lines: Vec<PatLine> }
```
`.pat` can only express **straight dashed lines** — no arcs/ornaments.
`assets/hatch/standard.pat` ships LINE/LINE45/ANSI31/ANSI37/NET/NET45/GRID/DASH/
BRICK/VERTICAL. **Status:** the parser + asset exist as infrastructure; the
*render path currently uses the hardcoded catalog* (2a), and the `.pat` dash
families aren't consumed yet (planned). Inspect a file with
`cargo run -p cad_io --example pat_extract -- assets/hatch/standard.pat`.

---

## 3. Boundary detection — the BPOLY pipeline (`cad_app/src/hatch_trace.rs`)

Given a seed point inside a region, find the enclosing boundary loop(s). Classic
AutoCAD BPOLY/BHATCH: **tessellate → split at intersections → cluster endpoints →
adjacency → prune dangles → ray-cast → CCW-turn loop walk → classify outer vs
islands**.

Pipeline (`trace_boundary_at_in_view_cancellable(doc, scope, seed, cancel)`):
1. **Tessellate** (`tessellate_doc_in_view*`): every dobject → straight
   `TessSeg { a, b, src }` (Arc 32-, Circle 64-, Ellipse 64-, EllipseArc 32-,
   Spline 64-sample; Polyline bulge-aware 24/seg; Wall → both side lines; Hatch +
   Point skipped). Viewport-scoped — only visible dobjects, critical for 400k+.
2. **Split at intersections** (`split_at_intersections` →
   `seg_seg_intersect_params`): O(N²) pairwise, **different-source only**, inserts
   crossing points → fragments. This is what makes overlapping circles / chained
   boundaries traceable (the "lens" case).
3. **Cluster endpoints** (`cluster_endpoints`, `JOIN_EPS = 1e-4`): group coincident
   endpoints into clusters → build a graph.
4. **Adjacency** (`build_adjacency`): per cluster, list `(seg, other_cluster,
   outgoing_angle)`.
5. **Prune dangles** (`prune_dangles`): drop degree-1 tree branches so the walk
   never enters open stubs (e.g. a chord with outside stubs traces the half-disc).
6. **Ray-cast** (`ray_cast_horiz`): +X ray from the seed → sorted hits; each hit
   seeds a candidate loop walk.
7. **Trace loop** (`trace_loop`, cap `MAX_TRACE_STEPS = 8192`): walk taking the
   **smallest CCW turn** (sharpest left) at each node → a closed loop (first vertex
   repeated). Left-face boundary.
8. **Classify** (`trace_boundary_from_segs`): the **smallest containing** loop =
   outer (so clicking inside an island makes the island the outer); others that
   sit inside become islands.
9. **Augment islands** (`augment_islands_from_closed_dobjects`): post-trace scan
   for closed dobjects fully inside the outer that don't contain the seed → add as
   islands. Result: `TracedBoundary { outer: Vec<Vec2>, islands: Vec<Vec<Vec2>> }`.

Polygon utils: `polygon_signed_area`, `point_in_polygon` (even-odd),
`polygon_bbox`, `polygons_equivalent` (dedup). Cancellation: an `AtomicBool`
checked every `CANCEL_CHECK_STRIDE = 256` iterations (so Esc aborts mid-trace on
the worker).

---

## 4. App command & flows (`cad_app/src/app.rs`)

### Command + dispatch (`Command::Hatch { pattern, scale, angle_deg }`, ~4714)
- **Re-entry guard:** if `hatch_confirm_open`, refuse (don't stack fills before
  the user accepts/rejects the previous preview — the original bug).
- **No args** → open the **Choose Hatch Attributes** dialog.
- **With args** → set `pending_hatch_pattern = (name, scale, angle_deg)` and either
  begin a select session (`QueuedOp::Hatch`) or apply to the current selection.

### (A) Select-first (`QueuedOp::Hatch`)
Empty selection → select session; on Enter, `apply_hatch()` collects each
**closed** dobject (closed Polyline / Circle / Ellipse) as a boundary handle and
makes **one Hatch** (outer + islands by even-odd). Snapshots the doc into a
preview, opens the confirm panel.

### (B) Pick-point (`apply_pick_point_hatch(seed) -> bool`, ~6336)
Armed via the dialog's **Pick Point** button (`hatch_pick_point_armed`,
`hatch_pick_point_session`). The session stays **armed across clicks** (each click
makes a hatch; Enter/Esc ends). Routing per click:
- collect cheap-path candidates (closed dobjects whose polygon contains the seed);
- **0 candidates** → trace path;
- **1 candidate, not crossed by others** → **cheap path** (use it directly +
  auto-detect islands inside → `apply_hatch`);
- **1 crossed, or 2+** → **trace path** (partial overlap needs the tracer).
- **Trace path** = `spawn_hatch_worker_scoped(seed, scope)` → a `thread::spawn`
  running `trace_boundary_at_in_view_cancellable`; `poll_hatch_worker()` drains the
  result, **materialises** outer+islands as closed Polylines (pushed to the doc),
  collects their handles, and creates the Hatch. `HatchWorkerResult =
  Success{tb,log} | Failed{log,error} | Cancelled{log}`. A new pick cancels the
  prior worker.

### Choose Hatch Attributes dialog (`render_hatch_dialog`, ~6849)
Thumbnail strip of `PATTERN_NAMES` (click to pick) · live preview pane (solid vs
pattern via `paint_pattern_preview` → `patterns::lookup`) · **Scale** slider
(0.05–20×, log) · **Angle** (0–360°) · buttons **Cancel / OK / Pick Point /
Select Objects**. (`render_hatch_pattern_library` is the bigger pattern grid.)

### Confirm panel (`render_hatch_confirm_panel`, ~7282)
After a hatch is created it's a **preview** awaiting a decision:
**Confirm(c)** (`hatch_confirm_accept` → promote the preview snapshot to undo),
**Discard(d)** (`hatch_confirm_discard` → restore `hatch_preview_snap`),
**Change(ch)** (`hatch_confirm_change` → edit mode, re-open dialog),
**+Point(p)** (re-arm pick-point), **+Dobject(D)** (select more boundaries to
append). Edit mode (`hatch_dialog_edit_mode`) patches the existing hatch's pattern
+ boundary instead of pushing a new one.

---

## 5. Rendering (`cad_app/src/app.rs:~5124`)

`render_hatch_fill(painter, rect, h, color)`:
1. `resolve_hatch_loops(h)` — walk `boundary_handles`, resolve each, tessellate
   closed types (Polyline/Circle/Ellipse) → `Vec<Vec<Vec2>>` loops.
2. Dispatch on `h.pattern`:
   - **Solid** → `render_hatch_solid` (even-odd: outer filled, islands overdraw
     with background).
   - **Pattern{name,scale,angle}** → `render_hatch_pattern`: `lookup(name)` →
     - `Families` → for each family, project loops' bbox onto the family normal to
       find the coverage band, generate parallel lines (cap 10 000/family), and
       **clip each line to the loops by even-odd** (sort line-loop intersections by
       t, pair consecutive hits).
     - `Tile` → `render_hatch_tile`: iterate tile cells over the (inverse-rotated)
       bbox (cap 200 000 cells), transform each segment/circle to world, clip to
       loops even-odd. `user_scale`/`user_angle` transform the whole pattern frame.

---

## 6. Debug window (`render_hatch_debug_window`, ~8818)

"Hatch Debug Log": Copy / Clear / **Dump Hatch State** (`dump_hatch_state` — per
hatch: pattern, handles, resolved loop count, per-loop bbox, estimated line count,
**warns if zero lines would draw** because spacing×scale is too large). Live
status (dialog open / pick-point armed / awaiting selection / idle) + the current
`pending_hatch_pattern`. `hatch_dbg(msg)` appends when the window is open;
`hatch_dbg_session_start()` auto-opens it. This is the first thing to read when a
hatch "falls" — it logs the whole pipeline: dialog → pattern/scale/angle →
Dobjects-vs-PickPoint → pick-point click → cheap-path candidates + winner →
apply_hatch params → resolved loops → render line counts.

---

## 7. Dependency graph (the full list)

```
HATCH
├─ cad_kernel::geom        Hatch{boundary_handles,pattern}, HatchPattern, all closed geom types
├─ cad_kernel::patterns    lookup(name) → Pattern::{Families|Tile}, PATTERN_NAMES (RENDER source today)
├─ cad_io::pat             parse_pat → PatPattern/PatLine (.pat loader — infra, not yet wired to render)
├─ assets/hatch/standard.pat   sample pattern pack
├─ cad_app::hatch_trace    the BPOLY tracer (tessellate→split→cluster→adjacency→prune→raycast→trace→islands)
│    └─ depends on: Document, the geom tessellators, point-in-polygon, seg-seg intersection
├─ cad_kernel::Document    dobjects + find_by_handle + layer/pen tables
├─ spatial index           viewport scope (only visible dobjects tessellated)
└─ std::{thread, sync::{mpsc, atomic::AtomicBool}}   async worker + cooperative cancel
```
Key call chain: `Command::Hatch` → dialog or `apply_hatch` / `apply_pick_point_hatch`
→ (cheap) `apply_hatch` | (trace) `spawn_hatch_worker_scoped` → `hatch_trace::trace_boundary_at_in_view_cancellable`
→ `poll_hatch_worker` → materialise loops → `apply_hatch` → confirm panel.
Render: `render_hatch_fill` → `resolve_hatch_loops` → `render_hatch_{solid|pattern|tile}` → `patterns::lookup`.

---

## 8. Persistence

The hatch is **handle-referenced**, so serialization must round-trip
`boundary_handles` + the `HatchPattern` (name/scale/angle or Solid) and rely on
the boundary dobjects' handles being stable on load. RSM (handle-stable binary) is
the natural home; DXF hatch/REGION round-trip is **not** a V1 priority (DXF
generally explodes derived fills). Treat full hatch persistence as a slice to
verify when wiring I/O — the renderer re-derives the fill from handles each frame,
so only handles+pattern need storing.

---

## 9. Tests (20)

- **Boundary trace** (`hatch_trace.rs`, 12): `square_from_four_lines`,
  `square_with_circle_island`, `click_inside_island_makes_island_the_outer`,
  `no_boundary_returns_none`, `lens_between_two_overlapping_circles`,
  `common_region_of_three_overlapping_circles`,
  `circle_chord_with_outside_stubs_traces_half_disc`,
  `circle_with_many_crossing_lines`, `island_above_seed_is_detected_by_doc_scan`,
  and 3 `split_at_intersections_*`.
- **Pattern catalog** (`patterns.rs`, 6): `every_named_pattern_resolves`,
  `unknown_pattern_is_empty`, `lookup_is_case_insensitive`,
  `brick_is_tile_with_4_segments`, `tile_has_8_segments_in_4x4_period`,
  `concentric_has_4_circles_no_segments`.
- **.pat parser** (`pat.rs`, 2): `parses_headers_families_and_dashes`,
  `malformed_lines_warn_but_dont_abort`.

---

## 10. Built vs. planned

**Built:** solid fill; 17-pattern catalog (line families + tiles); per-hatch
scale/angle; even-odd islands; BPOLY pick-point (cheap + trace paths); partial
overlap via intersection splitting; auto-island detection; viewport scoping;
async cancellable worker; the attributes dialog + live preview; the confirm panel
(Confirm/Discard/Change/+Point/+Dobject) + edit mode; the debug window.

**Planned / owed:** `HatchPattern::Composite` (terrazzo/stones — combine patterns
with per-instance palette, see the hatch-wishlist memo); consuming `.pat` files +
~40 imported patterns; dashed line families in the render path; spatial-index
broad-phase for the O(N²) intersection split; whole-trace-on-worker; spline/
open-polyline boundary chaining; DXF region serialization; adaptive tessellation.

---

## 11. Gotchas & invariants

- **Handle-referenced fill, derived every frame.** Don't copy boundary geometry
  into the hatch; store handles. Transforms are no-ops (move the boundaries).
- **"Smallest containing loop = outer"** — clicking inside an island makes the
  island the region (matches AutoCAD).
- **Split-at-intersections is mandatory** for overlapping/chained boundaries; it's
  O(N²) — scope to the viewport and run on the worker.
- **Trace runs on a background thread** with an `AtomicBool` cancel; a new pick or
  Esc cancels the prior one. The synchronous path can't honour mid-op Esc.
- **Re-entry guard:** block a new `hatch` while a preview is awaiting
  Confirm/Discard (or it stacks duplicate fills).
- **Pattern spacing is in drawing units** (no auto-zoom scaling) — use the scale
  slider; the debug window warns when spacing×scale would draw zero lines.
- **Even-odd clipping everywhere** — both line-family and tile rendering clip to
  loops by parity; the cheap path still needs island detection.
- **`.pat` ≠ render source yet** — today rendering uses the hardcoded catalog; the
  `.pat` loader is infrastructure.

---

## 12. Port recipe

1. **Data:** `Hatch { boundary_handles: Vec<Handle>, pattern: HatchPattern }`,
   `HatchPattern { Solid | Pattern{name,scale,angle_deg} }`; transforms no-op;
   view-independent bbox.
2. **Patterns:** port `patterns::lookup` (Families + Tile) + `PATTERN_NAMES`; add
   the `.pat` parser as infra.
3. **Tracer:** port `hatch_trace` wholesale (it's nearly pure): tessellate →
   split-at-intersections → cluster → adjacency → prune-dangles → ray-cast →
   CCW-turn `trace_loop` → classify outer/islands → augment islands. Unit-test
   with the 12 cases.
4. **Flows:** select-first (closed dobjects → one hatch) + pick-point (cheap path
   for a single containing closed dobject, else spawn the tracer on a worker with
   a cancel flag); materialise traced loops as closed polylines; reference them by
   handle.
5. **Render:** `resolve_hatch_loops` (handles→polygons) → solid even-odd OR
   line-family/tile generation clipped to loops by even-odd, transformed by
   scale/angle.
6. **UI:** attributes dialog (pattern picker + scale + angle + preview), a confirm
   panel (preview → accept/discard/change/+point/+dobject), and a debug log of the
   whole pipeline.
7. **Guards:** re-entry guard, viewport scoping, line/cell safety caps, cancel on
   Esc/new-pick.
