# RUST_CAD — Modify Commands (comprehensive wiring guide)

> Instruction doc for another coding agent. Covers **every modify command**
> (move, copy, rotate, scale, mirror, stretch, align, trim, extend, fillet,
> chamfer, offset, join, break, lengthen, erase, match-props, array, explode)
> end-to-end: the **pure kernel op**, the **app-side state machine**, the
> **select-first / pick flow**, and the **apply + undo** step. Reviewing this
> should be enough to re-wire the whole modify layer into another repo.
>
> Code map: kernel ops in `cad_kernel/src/{modify,trim,join,fillet}.rs` +
> transforms in `geom.rs`; app state machines + dispatch + `apply_*` in
> `cad_app/src/app.rs`. Line numbers from `9a4bcc7`; grep the symbol if stale.
> Reads with `CLICK_DRAG_HANDLER.md` (click routing) and
> `COMMAND_LINE_CURRENT.md` (dispatch).

---

## 1. The one contract every modifier follows

```
typed/menu/ribbon "verb"
   → run_command dispatch arm
      → (select-first?) if selection empty: begin_selection + queue the op;  else start the state machine
         → state machine advances on canvas CLICKS (and typed sub-options)
            → at the final pick: call a PURE kernel function (no app state)
               → apply: snapshot_doc()  +  mutate self.doc  +  invalidate index/gpu
```

**Two halves, strictly separated:**
- **Kernel (pure):** takes geometry + numbers, returns new geometry or a
  `Result`. No selection, no clicks, no `self`. Fully unit-testable. This is what
  you port first and trust.
- **App (interaction):** a small state machine that gathers the picks/params,
  calls the kernel op, and writes the result back with one undo snapshot.

Port the kernel op as-is; re-create the thin state machine around your own
input/selection system.

---

## 2. Kernel ops (pure — the part to copy verbatim)

All are pure functions / methods on `Geom`. Signatures (from
`cad_kernel/src/*`):

### Transforms — methods on `Geom` (`geom.rs`)
```rust
fn translated(&self, off: Vec2) -> Geom;                       // move/copy/stretch delta
fn rotated(&self, pivot: Vec2, angle: f64) -> Geom;            // rotate (radians)
fn scaled(&self, pivot: Vec2, factor: f64) -> Geom;            // uniform scale
fn scaled_xy(&self, pivot: Vec2, sx: f64, sy: f64) -> Geom;    // anisotropic (parametric)
fn mirrored(&self, a: Vec2, b: Vec2) -> Geom;                  // reflect across line a-b
fn lengthened(&self, delta: f64, near: Vec2) -> Result<Geom,&'static str>; // lengthen end nearest `near`
fn with_grip_moved(&self, role: GripRole, new_pos: Vec2) -> Geom;          // grip edit
```
Circles/arcs under non-uniform scale promote to ellipses/elliptical-arcs (handled
inside `scaled_xy`). Polylines carry `widths` through every transform.

### Trim / Extend / Break (`trim.rs`)
```rust
fn trim_at(&self, cutters: &[Geom], pick: Vec2, edge_mode: bool)
    -> Result<Vec<Geom>, &'static str>;   // split target at cutter intersections, drop the picked piece
fn extend_to(&self, boundaries: &[Geom], pick: Vec2, edge_mode: bool)
    -> Result<Geom, &'static str>;        // grow the end near `pick` to the nearest boundary
fn split_at(&self, at: Vec2) -> Result<(Geom, Geom), &'static str>;   // BREAK
fn join_trim_survivors(pieces: Vec<Geom>) -> Vec<Geom>;              // re-merge touching survivors
```
`edge_mode` = the `EdgMod` SYSVAR (treat edges as infinite extensions for
"imaginary intersection" cuts).

### Fillet / Chamfer (`fillet.rs` — the rich path; `modify.rs` has the older line-only `fillet_lines`/`chamfer_lines`)
```rust
fn fillet_geoms(a:&Geom, pa:Vec2, b:&Geom, pb:Vec2, radius:f64, trim:bool)
    -> Result<Vec<Geom>, String>;   // two objects (lines/arcs/polylines), pick points choose the corner
fn chamfer_geoms(a:&Geom, pa:Vec2, b:&Geom, pb:Vec2, d1:f64, d2:f64, trim:bool)
    -> Result<Vec<Geom>, String>;
fn fillet_polyline_corner(pl:&Polyline, seg_a:usize, seg_b:usize, radius:f64) -> Result<Polyline,String>;
fn chamfer_polyline_corner(pl:&Polyline, seg_a:usize, seg_b:usize, d1:f64, d2:f64) -> Result<Polyline,String>;
fn fillet_polyline_all(pl:&Polyline, radius:f64) -> Result<(Polyline,usize),String>;   // P (all corners)
fn chamfer_polyline_all(pl:&Polyline, d1:f64, d2:f64) -> Result<(Polyline,usize),String>;
fn nearest_polyline_segment(pl:&Polyline, p:Vec2) -> Option<usize>;
```
`trim` = the `TrmMd` SYSVAR (trim originals to the corner vs keep + add the arc/bevel).

### Offset (`modify.rs`)
```rust
fn offset(&self, dist: f64, side: Vec2) -> Result<Geom, &'static str>;   // `side` = a point on the desired side
```

### Join (`join.rs`)
```rust
fn join_geoms(geoms: &[(usize, Geom)]) -> JoinOut;   // 3-pass: collinear lines → concentric arcs → chain→polyline
```

---

## 3. App state machines (one per interactive modifier)

Each is a tiny enum; the variant carries the picks gathered so far. (`app.rs`)

| Modifier | Enum | Phases (→ = a click/value advances) |
|---|---|---|
| Move | `MoveState` | `WaitingForBase → WaitingForDest(base) →` apply `translated(dest-base)` |
| Copy | `CopyState` | same as Move, but appends copies (loops for multiple) |
| Paste | `PasteState` | same shape; source = clipboard |
| Rotate | `RotateState` | `WaitingForPivot → WaitingForAngle(pivot)`; sub: `R`=reference (`RefSrc1→RefSrc2→RefTgt`), `C`=copy, or type degrees |
| Scale | `ScaleState` | `WaitingForPivot → WaitingForFactor`; sub: `R`=reference (`RefStart→RefEnd→NewLength`), `C`=copy, or type factor |
| Mirror | `MirrorState` | `WaitingForA → WaitingForB(a) → AwaitingKeep(a,b)` (Enter/Y keep copy, n erase original) |
| Stretch | `StretchState` | crossing-window picks the verts → `WaitingForBase(box) → WaitingForDest(box,base) →` move verts inside box |
| Align | `AlignState` | `WaitingForSrc1 → Src2 → Tgt1 → Tgt2 →` translate+rotate (+optional scale) |
| Trim | `TrimState` | `SelectingCutters →(Enter)→ PickingTargets(cutters)` loop; empty Enter = `PickingTargetsAll` |
| Extend | `ExtendState` | `SelectingBoundaries →(Enter)→ PickingTargets` loop; empty Enter = `PickingTargetsAll` |
| Fillet | `FilletState` | `WaitingForFirst(r) → WaitingForSecond(r,idx,pt) →` apply; continuous (loops until Esc); sub `R/T/M/P` |
| Chamfer | `ChamferState` | `WaitingForFirst(d1,d2) → WaitingForSecond(...) →` apply; sub `D/T/M/P` |
| Offset | `OffsetState` | pick object → `WaitingForSide(dist)` → click side → apply; sub `t/e/l/u` |
| Break | `BreakState` | `WaitingForPoint` → click → `split_at` |
| Lengthen | `LengthenState` | `WaitingForSide(delta)` → click end → `lengthened` |
| MatchProps | `MatchPropsState` | `WaitingForSource` → click source → target phase is a normal select session (`QueuedOp::MatchPropPaint`) |
| Dist (inquiry) | `DistState` | `WaitingForP1 → WaitingForP2(p1)` → report (no mutation) |

All of these are listed in `in_click_only_phase` (see `CLICK_DRAG_HANDLER.md` §6)
so their picks fire on PRESS and never get demoted to a drag.

---

## 4. The two start patterns (dispatch arms)

### (a) Select-first transforms — Move/Copy/Rotate/Scale/Mirror/Stretch/Align/Erase/Array/Join/Hatch/Explode/ChProp/MatchProp
Empty selection → open a select session and **queue** the op; non-empty → start
the state machine immediately. Canonical arm (`Command::Move`):
```rust
Ok(Command::Move) => {
    if self.selection.is_empty() {
        self.begin_selection(SelectMode::ForSelect);
        self.queued_op = QueuedOp::Move;                 // ← remembered across the session
        self.set_prompt("move: select dobjects, Enter to continue");
    } else {
        self.move_state = MoveState::WaitingForBase;     // ← straight into the flow
        self.set_prompt("move (N): click BASE point");
    }
}
```
When the select session ends (Enter), the finalise step reads `self.queued_op`
and transitions into the matching state machine (so the picks gathered become the
basket the op transforms). `QueuedOp` (full list in `app.rs:1438`) is how a
command "remembers what to do after the user finishes selecting."

### (b) Two-basket — Trim / Extend
Stash the current selection, run a *typed* select session for the
cutters/boundaries, then loop on target clicks:
```rust
Ok(Command::Trim) => {
    self.pre_op_selection = std::mem::take(&mut self.selection);
    self.trim_state = TrimState::SelectingCutters;
    self.begin_selection(SelectMode::ForCuttingEdges);
    self.set_prompt("trim: pick CUTTING edges (Enter = all) [w/c/a/b/l/n]");
}
```
Enter with an empty cutter basket → `PickingTargetsAll` ("trim against everything
visible", recomputed each click — the AutoCAD default). Each target click calls
`trim_at(cutters, pick, edge_mode)`; survivors replace the target (pieces created
this session re-join the cutter set — see the wall/trim memos).

### (c) 2-pick interactive — Fillet / Chamfer
No selection needed; the command sets `WaitingForFirst(params)` from the SYSVAR
defaults (`FltRad` / `ChmDs1`,`ChmDs2`, `TrmMd`). Click obj1 → `WaitingForSecond`
(stores idx + pick point) → click obj2 → `fillet_geoms`/`chamfer_geoms`. The pick
*points* pick which corner/solution. **Continuous by default** (loops until Esc);
typed sub-options mid-command: `R` radius / `D` distances / `T` trim toggle / `M`
single-shot toggle / `P` all-corners on a polyline.

---

## 5. The apply step (every modifier ends here)

One shape, always: snapshot for undo → mutate `self.doc` → invalidate caches.
```rust
fn apply_move(&mut self, v: Vec2) {
    if v.len() < EPS { return; }
    self.snapshot_doc();                                // push undo state FIRST
    for &i in &self.selection {
        if let Some(d) = self.doc.dobjects.get_mut(i) {
            *d = d.translated(v);                        // ← the pure kernel op
        }
    }
    self.intersections.clear();
    self.index_dirty = true;                            // spatial index stale
    self.gpu_dirty = true;                              // GPU buffers stale
    // (copy/array push NEW dobjects instead of mutating in place)
}
```
Rules: **`snapshot_doc()` is the first line** (so one Ctrl+Z reverts the whole
op); set `index_dirty`/`gpu_dirty` after any geometry change; ops that fail
(`Result::Err`) must NOT snapshot — re-prompt instead.

---

## 6. End-to-end traces

**Move** (with nothing pre-selected)
```
"move" → dispatch: selection empty → begin_selection(ForSelect) + queued_op=Move
user clicks objects → Enter → finalise reads queued_op=Move → move_state=WaitingForBase
click BASE → WaitingForDest(base)
click DEST → apply_move(dest-base) → snapshot + translated() per selected → done
```

**Trim**
```
"trim" → pre_op_selection=take(selection); trim_state=SelectingCutters; ForCuttingEdges session
pick cutters (or empty) → Enter → PickingTargets(cutters)  [or PickingTargetsAll]
click a target → trim_at(cutters, pick, EdgMod) → survivors replace target → loop
Esc/Enter → end session, restore pre_op_selection
```

**Fillet**
```
"fillet 10" → FilletState::WaitingForFirst(10)   (or bare "fillet" uses FltRad)
click obj1 → WaitingForSecond(10, idx1, pt1)
click obj2 → fillet_geoms(o1,pt1,o2,pt2,10,TrmMd) → snapshot + replace/append → loop (continuous)
"R" → change radius;  "T" → toggle trim;  "P" → all corners of a polyline;  Esc → end
```

---

## 7. Recipe — wire a modifier into another repo

For each modifier, replicate the contract:

1. **Port the kernel op** (§2) as a pure function returning new geom / `Result`.
   Unit-test it in isolation — this is the load-bearing part.
2. **Add a state enum** (§3) whose variants carry the picks gathered so far.
3. **Dispatch arm** (§4): pick the start pattern —
   - transform of a set → **select-first + queued op**;
   - boundary-based (trim/extend) → **two-basket**;
   - corner op (fillet/chamfer) → **2-pick interactive, continuous**;
   - single object (offset/break/lengthen) → pick + side/point.
4. **Advance on input**: each click (snapped via your osnap/CARD/grid) fills the
   next state variant; typed sub-options (`R/T/M/D/P/C`) mutate params or mode.
5. **Apply** (§5): `snapshot_undo()` → call the kernel op → write back →
   invalidate index/GPU. On `Err`, re-prompt without snapshotting.
6. **Register the state** in your click-only-phase set so picks fire on press and
   aren't demoted to drags.
7. **Expose three entry points** to the same logic: command word, menu item,
   ribbon button — all calling `run_command("verb")` (see `TOP_MENU_GUIDE.md` /
   `RIBBON_GUIDE.md`).

---

## 8. Per-command quick reference

| Verb (aliases) | Kernel op | State | Sub-options | Notes |
|---|---|---|---|---|
| `move` (m) | `translated` | MoveState | — | select-first |
| `copy` (c/cp/co) | `translated` (append) | CopyState | — | loops for multiple drops |
| `rotate` (ro) | `rotated` | RotateState | R ref, C copy, type ° | reference = 3 picks |
| `scale` (sc) | `scaled` | ScaleState | R ref, C copy, type factor | |
| `mirror` (mi) | `mirrored` | MirrorState | keep/erase original | 2 clicks = axis |
| `stretch` (s/st) | `translated` on verts in box | StretchState | crossing window | verts inside box move |
| `align` | translate+rotate(+scale) | AlignState | — | 2 src + 2 tgt picks |
| `trim` (tr) | `trim_at` | TrimState | Enter=all, w/c/a/b/l/n | two-basket, EdgMod |
| `extend` (ex) | `extend_to` | ExtendState | Enter=all | two-basket, EdgMod |
| `fillet` (f/flt) | `fillet_geoms` / `fillet_polyline_*` | FilletState | R/T/M/P | continuous; FltRad, TrmMd |
| `chamfer` (cha) | `chamfer_geoms` / `chamfer_polyline_*` | ChamferState | D/T/M/P | ChmDs1/2, TrmMd |
| `offset` (o) | `offset` | OffsetState | t/e/l/u | OfsDis default |
| `join` (j) | `join_geoms` | (select-first, QueuedOp::Join) | — | 3-pass merge |
| `break` (br) | `split_at` | BreakState | — | one pick |
| `lengthen` (len) | `lengthened` | LengthenState | signed delta | pick end |
| `erase` (e/delete) | remove | (select-first, QueuedOp::Erase) | — | batch delete |
| `matchprop` (mp) | copy style | MatchPropsState→QueuedOp | — | source click → target session |
| `array` | `translated` grid | (select-first, QueuedOp::Array) | rows/cols/dx/dy | dialog-driven |
| `explode` (xp) | decompose | (select-first, QueuedOp::Explode) | — | blocks/walls/plines→segments |

---

## 9. Invariants & gotchas

- **Kernel ops are pure** — no `self`, no selection, no clicks. If you're tempted
  to read app state in a kernel op, pass it as a parameter (`edge_mode`, `trim`,
  radius). This keeps them testable and portable.
- **`snapshot_doc()` once, first, on success only.** Never snapshot on an `Err`
  path (re-prompt instead) or you litter the undo stack with no-ops.
- **Pick points carry meaning.** Fillet/chamfer/trim/offset use the *click
  position* (not just the object index) to choose the corner / piece / side. Pass
  the snapped world point into the kernel op.
- **Select-first uses `QueuedOp`**; two-basket uses `pre_op_selection` +
  `ForCuttingEdges/ForBoundaryEdges`. Don't mix them.
- **Continuous ops (fillet/chamfer, trim/extend target loop) end only on
  Esc/Enter** — the program never auto-ends (see `feedback_rust_cad_user_terminates_sessions`).
- **Invalidate `index_dirty` + `gpu_dirty`** after any geometry mutation, and
  `intersections.clear()` so cached ∩ queries rebuild.
- **Trim splits at EVERY cutter intersection** into N+1 pieces, then drops only
  the clicked piece; pieces inherit cutter status. **Don't** merge survivors into
  two outer pieces (see the trim memos).
- **Defaults come from SYSVARs** (`FltRad`, `ChmDs1/2`, `OfsDis`, `WlThk`,
  `TrmMd`, `EdgMod`) — see `SETTINGS.md`. Setting one inline (`fillet 10`)
  persists it.
