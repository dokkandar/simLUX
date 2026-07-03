# RUST_CAD — TRIM (implementation guide, anchored on a recorder dump)

> How TRIM works end-to-end, written so another coding agent can implement the
> same: **how cutting edges (boundaries) are defined**, **how the different
> dobjects are highlighted**, and **how a target is rebuilt** (split at every
> cutter, drop the clicked piece, survivors inherit cutter status). Every claim is
> tied to the Session-Recorder dump in §0.
>
> Code: `cad_app/src/app.rs` (`apply_trim_pick` ~15086, dispatch ~4685, the
> cutter→targets transition ~4905, the target-click routing ~22845),
> `cad_kernel/src/trim.rs` (`trim_at`, `join_trim_survivors`). Line numbers from
> `9a4bcc7`. Reads with `EDIT_GEOMETRY_GUIDE.md`, `CLICK_DRAG_HANDLER.md`,
> `SESSION_RECORDER.md`.

---

## 0. The dump, decoded (one full trim session)

```
CMD "trim" → Trim
  select_mode  Off → ForCuttingEdges          ← open a CUTTER select session
  trim_state   Off → SelectingCutters
  trim_debug   OPENED                          ← the diagnostic window auto-opens

PRESS (202.9,284.1) … RELEASE (-428.9,-51.5)  drag=1079.9px   (R→L ↓)
  ✓ SEL [] → [0,1,2,3,4,5,6]
     add_window_selection  MODE=crossing  REASON=direction-default (p2.x<p1.x → crossing)
     candidates(7 from spatial index)={ #0 bbox… → HIT · #1 … HIT · … · #6 … HIT }
  → the crossing window picked all 7 dobjects as CUTTERS

(empty Enter)
  select_mode  ForCuttingEdges → Off
  trim_state   SelectingCutters → PickingTargets([0,1,2,3,4,5,6])   ← cutters CONFIRMED; now loop on target clicks

CLICK (-345.8,70.4)  hit=Some(6)  trim=PickingTargets([0..6])
  UNDO-SNAP depth=3 (snapshot_doc ~3192 bytes)                      ← one undo step PER click
  apply_trim_pick ok=true  dobj 7→8  target #6  n_pieces=2  EdgMod=true
  trim_state PickingTargets([0..6]) → PickingTargets([0..7])        ← #6 was a cutter → its 2 pieces inherit cutter status

CLICK (-150.7,50.5)  hit=Some(1)  → apply_trim_pick ok=true  dobj 8→8  target #1  n_pieces=1  (net 0: 1 survivor replaces target)
CLICK (-69.1,79.4)   hit=Some(2)  → ok  dobj 8→8  n_pieces=1
CLICK (-70.1,138.8)  hit=Some(2)  → ok  dobj 8→9  n_pieces=2   trim_state grows to [0..8]
CLICK (-84.1,182.5)  hit=Some(7)  → ok  dobj 9→9  n_pieces=1   (#7 is a piece born earlier — it's a cutter AND a valid target)
CLICK ( 69.3,224.7)  hit=Some(4)  → ok  dobj 9→10 n_pieces=2  trim_state grows to [0..9]
…                                                              (auto SNAP[1] at 10 dobj — recorder cadence)
CLICK (-309.2,134.0) hit=Some(5)  → ok  dobj 10→11 n_pieces=2 trim_state grows to [0..10]

(Esc/Enter) trim_state PickingTargets([0..10]) → Off            ← only the USER ends the session
```

Five facts to take from this:
1. Cutters are chosen in a **dedicated select session** (a crossing window here).
2. **Enter confirms** cutters → the state flips to `PickingTargets(list)` and you
   then **click targets one at a time** in a loop.
3. Each target click does **`apply_trim_pick`**, snapshotting first (per-click
   undo) and reporting `n_pieces` survivors + `EdgMod`.
4. `dobj` count change = `n_pieces − 1` (target removed, `n_pieces` survivors
   added). `n_pieces=2` → +1; `n_pieces=1` → 0.
5. When a **cutter** is itself trimmed, **its new pieces inherit cutter status**
   (the `PickingTargets` list grows: `[0..6]→[0..7]`).

---

## 1. State machine

```rust
enum TrimState {
    Off,
    SelectingCutters,             // a ForCuttingEdges select session is running
    PickingTargets(Vec<usize>),   // cutters confirmed (explicit list); loop on target clicks
    PickingTargetsAll,            // empty-Enter default: EVERY current dobject is a cutter, recomputed each click
}
```
Plus `pre_op_selection: Vec<usize>` (the user's selection, stashed during the
session and restored after). Trim is in `in_click_only_phase` so target picks fire
on press and never become a drag (`CLICK_DRAG_HANDLER.md`).

---

## 2. Defining the boundaries (cutting edges)

Dispatch (`Command::Trim`, ~4685):
```rust
self.pre_op_selection = std::mem::take(&mut self.selection);  // stash current selection
self.trim_state = TrimState::SelectingCutters;
self.begin_selection(SelectMode::ForCuttingEdges);            // a TYPED cutter select session
self.set_prompt("trim: pick CUTTING edges (Enter = all) [w/c/a/b/l/n  Esc=cancel]");
```
- The cutter session is a **normal selection session** (`SelectMode::ForCuttingEdges`):
  click, **window (L→R)** / **crossing (R→L)** drag, or the letter shortcuts
  `w/c/a/b/l/n`. The dump's crossing drag selected `[0..6]`; the `SEL` event shows
  the **spatial-index candidate set + per-candidate bbox HIT/miss** — that's your
  boundary set.
- **Enter confirms** (`finalise`, ~4905): the basket transfers out, the user's
  prior selection is restored from `pre_op_selection`, and:
  - **non-empty** → `TrimState::PickingTargets(cutters)`;
  - **empty Enter** → `TrimState::PickingTargetsAll` ("use every dobject as a
    cutter, recomputed each click" — the AutoCAD default; also the only way pieces
    created mid-session keep cutting). Empty doc → cancel.

So "boundaries" = the confirmed cutter index list (or *all*, dynamic).

---

## 3. Highlighting the different dobjects

Three distinct visual roles — keep them visually separate:
- **Cutting edges** render **warm orange** during the target-pick phase (prompt:
  *"cutter(s) ready (warm orange). Click each TARGET to cut."*). Extend uses
  **warm amber**. This tells the user *what will cut*.
- **The selection basket** uses the **dashed-gray** transient look — reserved for
  the user's own selection, NOT for cutters (don't reuse it, or cutters look like
  a basket).
- **The active target-pick badge** in the corner ("● TRIM target-pick active",
  `rgb(255,170,60)`).
- **The Trim Debug window** (auto-opens on `trim`) logs the whole pipeline:
  cutters captured (+ each cutter's full geometry), every target click (hit /
  VOID, cutter list), the pre-trim target geometry, each new-born piece's
  geometry, and the cutter-list patch. This is the first thing to read when trim
  "silently fails."

(Color refs in code: warm-orange/amber cutter highlight in the render loop; the
debug badge `rgb(255,170,60)`.)

---

## 4. Rebuilding the target — `apply_trim_pick(cutters, target_idx, pick) -> bool`

The heart (`app.rs:15086`). Returns **true only if the doc changed** (so the
caller patches the cutter list only on success).

```rust
let before = self.doc.dobjects.len();
self.snapshot_doc();                                   // (1) per-click undo  (the dump's UNDO-SNAP)
let edge_mode = self.env.EdgMod;
// (2) cutter geoms, EXCLUDING the target itself (a dobject can't cut itself);
//     BlockRef cutters are exploded in-memory so geometry INSIDE a block cuts.
let mut cutter_geoms = Vec::new();
for &i in cutters.iter().filter(|&&i| i != target_idx) {
    self.expand_cutter_geoms(&self.doc.dobjects[i].geom, &mut cutter_geoms, 0);
}
// (3) refuse if there's nothing to cut against — UNLESS the target is a polyline
//     (a polyline segment with no boundary is just removed). Roll back + false.
if cutter_geoms.is_empty() && !target_is_polyline { rollback(); return false; }
// (4) the PURE kernel cut:
match target.geom.trim_at(&cutter_geoms, pick, edge_mode) {
    Ok(pieces) => {
        let pieces = cad_kernel::join_trim_survivors(pieces);  // (5) re-merge touching survivors
        let n = pieces.len();
        self.doc.dobjects.remove(target_idx);                  // (6) remove the target
        for g in pieces { let mut d = DObject::new(g); d.style = target_style; self.doc.push(d); }  // append survivors (keep style)
        self.index_dirty = true; self.gpu_dirty = true; self.intersections.clear();
        true
    }
    Err(msg) => { self.undo_stack.pop().map(|prev| self.doc = prev); /* surface msg */ false }  // (7) rollback on failure
}
```

Key points:
- **(1) snapshot first, every click** — one Ctrl+Z = one trim.
- **(2) self-exclusion** lets a cutter also be a target (it just doesn't cut
  itself). BlockRef cutters are expanded to their real contents.
- **(4) the kernel does the geometry** — `trim_at` is pure.
- **(5) `join_trim_survivors`** re-merges consecutive survivors that still touch
  (a closed circle/ellipse is over-split at every cut; gaps from removed arcs are
  preserved) so you keep the natural run(s), not a pile of fragments.
- **(6) remove target + append survivors**, survivors inherit the target's
  `style`.
- **(7) on `Err`, roll back** (pop the snapshot) and surface the reason — never
  leave a half-applied trim or a junk undo step.

---

## 5. The divided parts — what `trim_at` returns (`cad_kernel/src/trim.rs`)

```rust
fn trim_at(&self, cutters: &[Geom], pick: Vec2, edge_mode: bool) -> Result<Vec<Geom>, &str>
```
- Splits the target at **EVERY** cutter intersection into **N+1 fragments**, then
  returns all fragments **except the one containing `pick`** (the clicked piece is
  the one removed). So `n_pieces` (survivors) = (#cuts) for a clicked middle
  piece, or fewer at the ends.
- **0 intersections** → `Err` (refuse) for a Line/Arc; a polyline segment with no
  boundary → `Ok(empty)` (delete the whole fragment).
- **Endpoint-only hits** → delete the whole fragment (`Ok(empty)`); **interior
  cuts** → split. Don't collapse those two cases.
- **Do NOT** merge survivors back into "two outer pieces" — keep every segment
  (then `join_trim_survivors` re-joins only the ones that genuinely touch).
- `edge_mode` (`EdgMod`) = treat cutter edges as their **infinite extensions** for
  "imaginary intersection" cuts (the dump shows `EdgMod=true` throughout).

---

## 6. Cutter-list patch + pieces-inherit-cutter (the growing list)

After a successful trim in **explicit-list** mode, the caller patches
`TrimState::PickingTargets(list)` (`app.rs:~22913`):
```rust
let n_pieces  = n_after + 1 - n_before;   // survivors
let first_new = n_after - n_pieces;       // new pieces appended at [first_new .. n_after)
if let TrimState::PickingTargets(c) = &mut self.trim_state {
    c.retain(|&i| i != tgt);              // drop the consumed target
    for ci in c.iter_mut() { if *ci > tgt { *ci -= 1; } }   // remove() shifted higher indices down by 1
    if tgt_was_cutter && n_pieces > 0 {   // INHERIT: a trimmed cutter's pieces are cutters too
        c.extend(first_new..n_after);
    }
}
```
This is exactly the dump's `[0..6] → [0..7]`: trimming cutter `#6` into 2 pieces
removed `#6` and appended pieces at the end, which inherit cutter status. In
**all-mode** there's no patch — the next click re-derives cutters from
`0..doc.len()`. (Memos: `trim_pieces_inherit_cutter_status`,
`cutter_list_patch_only_on_success`.)

---

## 7. Per-target-click routing (`app.rs:~22845`)

```rust
let cutters = match trim_state {
    PickingTargetsAll       => (0..doc.len()).collect(),   // dynamic, recomputed each click
    PickingTargets(c)       => c.clone(),
};
let hit = nearest_entity_under(world, 10.0/scale);         // which dobject is the target
if let Some(tgt) = hit {
    let tgt_was_cutter = cutters.contains(&tgt);
    let did = apply_trim_pick(&cutters, tgt, click_world);
    if did && !all_mode { /* patch cutter list (§6) */ }
} else { /* void click — log, session continues */ }
// loop continues until Esc/Enter
```
A **void click** (no dobject) is logged and ignored — the session keeps going.
The session ends ONLY on the user's Esc/Enter (`trim_state → Off`).

---

## 8. Invariants & gotchas (don't regress)

- **Two baskets:** cutters (`ForCuttingEdges`) then targets (the click loop). Keep
  `pre_op_selection` and restore the user's selection when the session ends.
- **Empty Enter at the cutter prompt = use ALL** (dynamic) — exempt from the
  2-stage select-cancel; it's the only way mid-session pieces keep cutting.
- **snapshot_doc() once per target click**, before the cut; on `Err` pop it.
- **`apply_trim_pick` returns bool**; patch the cutter list **only on success**
  (and only in explicit-list mode).
- **Self-exclusion:** a cutter can be a target; filter the target out of
  `cutter_geoms` (self-intersection is 0), don't refuse it.
- **Split at every cutter into N+1**, remove only the clicked piece; survivors
  inherit the target's style; trimmed cutters' pieces inherit **cutter** status.
- **`join_trim_survivors`** after the cut (over-split closed curves; preserve
  gaps). Endpoint-only/no-boundary cases delete the fragment; 0-hit Line/Arc
  refuses.
- **Index/GPU dirty + clear intersections** after a successful cut.
- Cutters render **warm orange**; the basket dashed-gray is reserved — keep them
  visually distinct.

---

## 9. Implementation checklist (port TRIM)

1. **Kernel:** `Geom::trim_at(&self, cutters, pick, edge_mode) -> Result<Vec<Geom>,_>`
   — split at every cutter into N+1, drop the piece containing `pick`; endpoint-only
   → empty; 0-hit → Err. Plus `join_trim_survivors(Vec<Geom>) -> Vec<Geom>`. Pure,
   unit-tested.
2. **State:** `TrimState { Off, SelectingCutters, PickingTargets(Vec<usize>), PickingTargetsAll }`
   + `pre_op_selection`.
3. **Define cutters:** `trim` → stash selection → `SelectingCutters` +
   `ForCuttingEdges` select session (window/crossing/shortcuts). Enter → non-empty
   `PickingTargets(list)`, empty `PickingTargetsAll`.
4. **Highlight:** render cutters warm-orange during target pick; keep the basket
   look separate; (optionally) a debug log of cutters/targets/pieces.
5. **Target loop:** each click → resolve cutters (all-mode = `0..len`) → hit-test →
   `apply_trim_pick`: snapshot → build cutter geoms (self-excluded, blocks
   expanded) → `trim_at` → `join_trim_survivors` → remove target + append survivors
   (inherit style) → dirty caches → return bool. Roll back on Err.
6. **Patch cutters on success** (explicit mode): drop target, shift indices > tgt
   down by 1, and if the target was a cutter, append the new piece indices
   (inherit cutter status).
7. **End only on Esc/Enter**; void clicks continue the session.
8. **EdgMod** SYSVAR feeds `edge_mode`.
