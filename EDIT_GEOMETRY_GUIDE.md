# RUST_CAD — Edit-Geometry Modifiers (Trim · Extend · Fillet · Chamfer · Offset · Join · Break · Lengthen · Reverse)

> Focused, exhaustive doc for the **edit-geometry** group of the Modify menu
> (the nine in the screenshot). These differ from the *transform* group
> (move/copy/rotate/scale/mirror/stretch/align): transforms relocate a selection;
> these **change the geometry itself** — cut it, grow it, round/bevel a corner,
> parallel-copy it, merge it, split it, lengthen an end, or flip its direction.
>
> This is the detailed companion to `MODIFY_GUIDE.md` (the whole modify layer).
> Same contract — **pure kernel op + thin app state machine + snapshot-and-apply**
> — but here every one of the nine is documented in full: kernel op, flow,
> sub-options, defaults, edge cases, gotchas. Line numbers from `9a4bcc7`
> (`cad_kernel/src/{trim,modify,fillet,join,geom}.rs`, `cad_app/src/app.rs`); grep
> the symbol if they drift.

---

## 0. The four interaction shapes in this group

| Shape | Members | How the targets are gathered |
|---|---|---|
| **Two-basket** | Trim, Extend | basket 1 = cutters/boundaries (typed select session) → Enter → loop clicking targets |
| **2-pick continuous** | Fillet, Chamfer | no selection; click obj-1 then obj-2; repeats until Esc; pick points choose the corner |
| **Pick-one-at-a-time** | Offset | no selection; click an object, click the side; repeats until Esc |
| **Basket-then-act** | Join, Break, Lengthen, Reverse | operate on the current selection (Join opens a select session; Break/Lengthen/Reverse require a pre-selected basket) |

All nine: the geometry op is a **pure kernel function**; the app gathers picks,
calls it, then `snapshot_doc()` + writes back + sets `index_dirty`/`gpu_dirty`.
On an `Err` result the app re-prompts and does **not** snapshot.

---

## 1. TRIM  (`trim`, `tr`)

**Does:** cut the clicked piece of a target out at its intersections with the
cutting edges.

- **Kernel:** `Geom::trim_at(&self, cutters: &[Geom], pick: Vec2, edge_mode: bool) -> Result<Vec<Geom>, &str>`
  (`trim.rs:156`). Splits the target at **every** cutter intersection into N+1
  pieces and returns them; the app drops the piece containing `pick`.
- **State:** `TrimState { Off, SelectingCutters, PickingTargets(Vec<usize>), PickingTargetsAll }`.
- **Flow:**
  1. `trim` → stash `pre_op_selection`, `begin_selection(ForCuttingEdges)`,
     prompt "pick CUTTING edges (Enter = all)".
  2. Pick cutters (or none) → **Enter**: with a basket → `PickingTargets(cutters)`;
     **empty Enter → `PickingTargetsAll`** (every visible dobject is a cutter,
     recomputed each click — the AutoCAD default).
  3. Each target click → `trim_at(cutters, pick, EdgMod)` → survivors replace the
     target; **loop** until Esc/Enter.
- **Sub/keys:** `w/c/a/b/l/n` during the cutter select session (window/crossing/
  all/before/last/none). `EdgMod` SYSVAR = treat edges as infinite extensions for
  "imaginary intersection" cuts.
- **Edge cases / rules:** 0 hits → `Err` (refuse). Endpoint-only hits → delete the
  whole fragment. Interior cuts → split. Pieces created this session **inherit
  cutter status** (added to the basket) so chained trims keep working. Don't merge
  survivors into two outer pieces — keep every segment.

## 2. EXTEND  (`extend`, `ex`)

**Does:** grow the end of a target nearest the click until it meets a boundary.

- **Kernel:** `Geom::extend_to(&self, boundaries: &[Geom], pick: Vec2, edge_mode: bool) -> Result<Geom, &str>`
  (`trim.rs:448`). Moves the free endpoint nearest `pick` to the closest boundary
  intersection; recomputes arc bulge; preserves width.
- **State:** `ExtendState { Off, SelectingBoundaries, PickingTargets(Vec<usize>), PickingTargetsAll }`
  — the mirror of Trim.
- **Flow:** `extend` → `ForBoundaryEdges` session → Enter (empty = all boundaries)
  → click the END side of each target to extend; loop until Esc.
- **Notes:** also handles **polyline** extend (extends the end *segment* nearest
  the pick). `EdgMod` applies. Same `w/c/a/b/l/n` shortcuts.

## 3. FILLET  (`fillet`, `flt`, `f`)

**Does:** round the corner between two objects with an arc of radius `r` (r=0 ⇒
just trim/extend them to a sharp corner).

- **Kernel:** `fillet_geoms(a, pa, b, pb, radius, trim) -> Result<Vec<Geom>, String>`
  (`fillet.rs:426`) for two objects (lines/arcs/polylines); the **pick points**
  `pa`/`pb` choose which corner/solution. Polyline helpers:
  `fillet_polyline_corner(pl, seg_a, seg_b, r)`, `fillet_polyline_all(pl, r) -> (Polyline, count)`,
  `nearest_polyline_segment(pl, p)`.
- **State:** `FilletState { Off, WaitingForFirst(r), WaitingForSecond(r, idx1, pt1) }`.
- **Flow:** `fillet [r]` → `WaitingForFirst(FltRad)` (drops any active draw tool).
  Click obj-1 → `WaitingForSecond(r, idx1, pt1)`. Click obj-2 →
  `fillet_geoms(...TrmMd)` → apply → **continuous: loops to WaitingForFirst until
  Esc** (`fillet_multiple = true` by default).
- **Sub-keys (typed mid-command):** `R` set radius (persists to `FltRad`); `T`
  toggle trim (`TrmMd`); `M` toggle single-shot vs continuous; `P` polyline mode
  (round **every** corner of one picked polyline via `fillet_polyline_all`,
  `fillet_poly_all`).
- **Defaults:** `FltRad` (radius, inline `fillet 10` persists it), `TrmMd` (trim
  originals to the corner vs keep + add the arc). Guards re-prompt instead of
  exiting on too-big radius (overlap); re-applying with a new radius **updates**
  the rounding instead of stacking; suppresses object snap while picking objects.

## 4. CHAMFER  (`chamfer`, `cha`)

**Does:** bevel the corner between two objects with distances `d1` (first line)
and `d2` (second line).

- **Kernel:** `chamfer_geoms(a, pa, b, pb, d1, d2, trim) -> Result<Vec<Geom>, String>`
  (`fillet.rs:466`); polyline: `chamfer_polyline_corner(...)`,
  `chamfer_polyline_all(pl, d1, d2)`. (Older line-only path:
  `modify.rs::chamfer_lines`.)
- **State:** `ChamferState { Off, WaitingForFirst(d1, d2), WaitingForSecond(d1, d2, idx1, pt1) }`.
- **Flow:** identical to Fillet, two distances. Continuous by default
  (`chamfer_multiple = true`).
- **Sub-keys:** `D` set distances (persist to `ChmDs1`/`ChmDs2`); `T` trim toggle;
  `M` single-shot toggle; `P` polyline-all (`chamfer_poly_all`).
- **Defaults:** `ChmDs1`, `ChmDs2` (CHAMFERA/B), `TrmMd`.

## 5. OFFSET  (`offset`, `o`)

**Does:** make a parallel copy of an object at a distance, on the clicked side.

- **Kernel:** `Geom::offset(&self, dist: f64, side: Vec2) -> Result<Geom, &str>`
  (`modify.rs:20`). `side` = a point on the desired side.
- **State:** `OffsetState { Off, WaitingForObject(OffsetMode), WaitingForSide(OffsetMode, idx) }`,
  with `OffsetMode { Distance(f64), Through }`.
- **Flow:** `offset [d]` → `WaitingForObject(Distance(OfsDis))`. Click an object →
  `WaitingForSide(mode, idx)`. Click the side (or, in `Through` mode, the
  through-point) → `offset(...)` → apply → **loops back to WaitingForObject**
  (offset many, one at a time) until Esc/Enter.
- **Sub-options (typed mid-command):**
  - `t` / `through` → switch to **Through-point** mode (`OffsetMode::Through` — the
    next click is a point the copy must pass through, distance inferred).
  - `e` → toggle **erase** originals (`offset_erase`).
  - `l` → toggle new-dobject layer: **SOURCE** vs **CURRENT** (`offset_layer_src`).
  - `u` → undo the last offset this session (`offset_applied_count`).
  - a **number** → set the distance (persists to `OfsDis`).
- **Defaults:** `OfsDis` (inline `offset 5` persists it; bare `offset` reuses it).
  Transient flags (`offset_erase`, `offset_layer_src`, `offset_applied_count`)
  reset each invocation.

## 6. JOIN  (`join`, `j`)

**Does:** merge selected dobjects into longer lines / arcs / a single polyline.

- **Kernel:** `join_geoms(geoms: &[(usize, Geom)]) -> JoinOut` (`join.rs:112`).
  **Three passes:** (1) collinear lines merge (gap-aware — won't bridge trim
  gaps); (2) concentric arcs merge; (3) touching chain → one polyline
  (`CHAIN_EPS = 1e-3` fuzzy endpoint match; `JOIN_EPS = 1e-6` precision).
- **Selection model:** select-first — `join` with empty basket opens a select
  session with `QueuedOp::Join`; on Enter the basket is merged. With a basket it
  joins immediately.
- **Notes:** arcs use the bulge convention (`bulge_from_arc`); `Arc::reversed()`
  can't encode CW under positive sweep, so chaining reads the far endpoint
  directly. PEDIT-Join (`QueuedOp::PeditJoin`) is the polyline-targeted variant.

## 7. BREAK  (`break`, `br`)

**Does:** split one selected dobject into two at the clicked point.

- **Kernel:** `Geom::split_at(&self, at: Vec2) -> Result<(Geom, Geom), &str>`
  (`trim.rs:586`).
- **State:** `BreakState { Off, WaitingForPoint }`.
- **Flow:** requires a **pre-selected** basket (empty → "select first"). `break`
  → `WaitingForPoint` → click the cut point → `split_at` → the target becomes two
  dobjects.

## 8. LENGTHEN  (`lengthen`, `len`)

**Does:** grow/shrink a line/arc/elliptical-arc end by a signed delta.

- **Kernel:** `Geom::lengthened(&self, delta: f64, near: Vec2) -> Result<Geom, &str>`
  (`geom.rs:864`). Moves the endpoint nearest `near` by `delta` along the curve
  (negative shrinks).
- **State:** `LengthenState { Off, WaitingForSide(f64) }`.
- **Flow:** the command **requires the delta as an argument** —
  `Command::Lengthen(f64)` (bare `lengthen` is a usage error; the menu sends
  `lengthen 1`). Requires a pre-selected basket. `lengthen <d>` →
  `WaitingForSide(d)` → click the END to extend → `lengthened(d, click)`.

## 9. REVERSE  (`reverse`, `rev`)

**Does:** flip the direction of every direction-aware dobject in the selection.

- **Kernel:** `Geom::reversed(&self) -> Geom` (`geom.rs:983`). Flips
  **Line / Arc / EllipseArc / Polyline**; everything else is a no-op
  (direction-agnostic). Polyline reverses vertices + swaps per-segment widths.
- **App:** `apply_reverse` (`app.rs:14504`) — applies **directly to the current
  selection** (no pick phase). Empty basket → "empty basket" message; else
  snapshot + flip each, reporting `{flipped} flipped, {noop} no-op`.

---

## 10. Summary table

| Verb (aliases) | Kernel op | State machine | Selection model | Sub-options | SYSVAR defaults |
|---|---|---|---|---|---|
| trim (tr) | `trim_at` | TrimState | two-basket | Enter=all · w/c/a/b/l/n | EdgMod |
| extend (ex) | `extend_to` | ExtendState | two-basket | Enter=all · w/c/a/b/l/n | EdgMod |
| fillet (f/flt) | `fillet_geoms` / `fillet_polyline_*` | FilletState | 2-pick continuous | R · T · M · P | FltRad · TrmMd |
| chamfer (cha) | `chamfer_geoms` / `chamfer_polyline_*` | ChamferState | 2-pick continuous | D · T · M · P | ChmDs1 · ChmDs2 · TrmMd |
| offset (o) | `offset(dist, side)` | OffsetState (+OffsetMode) | pick-one-at-a-time | t · e · l · u · number | OfsDis |
| join (j) | `join_geoms` (3-pass) | QueuedOp::Join | select-first | — | — |
| break (br) | `split_at` | BreakState | basket required | — | — |
| lengthen (len) | `lengthened` | LengthenState | basket required (delta arg) | — | — |
| reverse (rev) | `reversed` | (direct apply) | basket required | — | — |

---

## 11. Gotchas & invariants

- **Pick points carry meaning** for trim/fillet/chamfer/offset — pass the snapped
  world point into the kernel op (it chooses the piece/corner/side/solution).
- **Two-basket vs select-first vs basket-required** are three different selection
  models in this one group — match the table above; don't unify blindly.
  *(Note: Break/Lengthen/Reverse currently require a pre-selected basket and just
  error on empty, rather than opening a select session like the universal model
  elsewhere — a known inconsistency if you're porting to a uniform select-first.)*
- **Fillet/Chamfer are continuous by default** (`*_multiple = true`); `M` toggles
  single-shot; only Esc/Enter ends them.
- **Trim splits at EVERY cutter** into N+1 pieces and pieces inherit cutter
  status; **Enter at an empty cutter/boundary basket = use ALL** (exempt from the
  2-stage select cancel).
- **Inline numeric sets persist the SYSVAR** (`fillet 10`→FltRad, `offset 5`→
  OfsDis, `chamfer 2 3`→ChmDs1/2) via `env.save()`.
- **snapshot_doc() once, on success only**; set `index_dirty`+`gpu_dirty` after
  geometry changes; `intersections.clear()`.
- These all live in `in_click_only_phase` (see `CLICK_DRAG_HANDLER.md`) so their
  picks fire on press and never demote to a drag.

---

## 12. Port checklist

For each of the nine: (1) port the **pure kernel op** + unit-test it; (2) add the
**state machine** with the right interaction shape (§0); (3) gather picks
(snapped) and feed the world point in; (4) call the op, then snapshot + write back
+ invalidate caches (or re-prompt on `Err`); (5) wire the sub-options as
typed-letter intercepts; (6) read defaults from the SYSVARs; (7) register the
state in your click-only-phase set; (8) expose the verb to command line + menu +
ribbon, all via the one `run_command` path. See `MODIFY_GUIDE.md` for the shared
contract and `SETTINGS.md` for the SYSVAR briefings.
