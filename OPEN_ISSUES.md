# RUST-AutoRASM — Open Issues / Unfinished Work

A living list of bugs that are NOT fully fixed and jobs not finished. Updated as
work progresses. Ask any time and I'll give you the current list.

Branch: `windows-ui-session-2026-06-20`
Last updated: 2026-06-25

Legend: 🔴 broken / confirmed not working · 🟡 partial / needs follow-up · 🔵 not started · ⚪ needs info from user

---

## 🔴 OPEN BUGS (confirmed still broken)

### 1. FILLET line ↔ polyline still produces a wrong result
- **Symptom:** filleting a line to a polyline distorts the polyline. Exploding
  the polyline first, then filleting a bare segment, works correctly.
- **Status:** attempted fix `c78af96` (move corner-side free tip / refuse interior)
  did NOT rectify it. Still broken.
- **What I need to diagnose:** ⚪ a SESSION DUMP of the failing fillet **plus the
  polyline's data** — specifically: is the polyline open or closed? how many
  vertices, and which segment is clicked? Possible causes still on the table:
  (a) the polyline is closed → my code errors, but user sees a change from a
  prior step; (b) the clicked segment is interior so it should error but doesn't;
  (c) the solver picks a tangent point far from the pick so the free tip still
  yanks; (d) the fillet arc is added but the trim direction is wrong.
- **Files:** `cad_kernel/src/fillet.rs` (`fillet_geoms`, `rebuild_side`,
  `poly_moved_vertex`), `cad_app/src/app.rs` (`apply_fillet_general`).

---

## 🟡 PARTIAL / NEEDS FOLLOW-UP

### 2. Two-segment polyline corner fillet doesn't auto-update on re-fillet
- Re-filleting with a new radius UPDATES corners only in **P (whole-polyline)**
  mode. Picking the same two segments again after a fillet won't update (they're
  no longer adjacent — an arc sits between them).
- **Files:** `cad_kernel/src/fillet.rs` (`fillet_polyline_corner`).

### 3. FILLET/CHAMFER on SPLINE and ELLIPSE-ARC not implemented
- They have no simple offset locus. Plan: tessellate to a dense polyline first
  (result becomes a polyline), or add a numerical tangent-circle solver.
- Currently returns a clear "not supported" message.

### 4. FILLET/CHAMFER of two NON-ADJACENT segments of one polyline
- Currently errors ("segments must be adjacent"). AutoCAD removes the
  intervening segments. Not implemented.

---

## 🔵 NOT STARTED / DEFERRED

### 5. Groups not persisted to `.rsm`
- Groups are in-session only; saving/reloading drops them.

### 6. Hatch self-sufficiency
- Hatch is purely associative (no own baked boundary) → Move on a hatch is a
  no-op; deleting a boundary shrinks/removes it. User wants hatches to own a
  baked invisible boundary. (See PROJECT_NOTES "Hatch rules".)

---

## ⚪ NEEDS INFO / OLD UNCONFIRMED

### 7. "Arc curves the wrong way after pe + j" (old report)
- A diagnostic was added to `pedit_join_selected` (prints `src arc:` / `pl v[i]`).
  Needs a session dump with those lines to confirm whether it's still happening.

---

## 🔧 HOUSEKEEPING

### UI baseline snapshot (2026-06-25)
- Before the UI redesign, the current UI state was saved as branch + tag
  **`ui-baseline-2026-06-25`** (at commit `5e27101`). To revert the whole UI:
  `git checkout ui-baseline-2026-06-25`. To revert just one file:
  `git checkout ui-baseline-2026-06-25 -- cad_app/src/app.rs`.

### 8. Unpushed commits
- The branch has local commits not yet pushed to GitHub
  (`b0a2291` → `c78af96`, plus the Ctrl+Z work `a5a6327`). Push when ready.

### 9. Reconcile with `origin/main`
- `origin/main` is ~14 commits ahead from another machine with OVERLAPPING
  file-dialog / zoom / block work. Do NOT blind-merge — review first.

---

## ✅ RESOLVED THIS SESSION (for reference — move out when superseded)
- EXTEND line → tangent circle (line∩circle tangent on large/long lines) `0afaa97`
- TTR tangent-object pick: object-snap suppressed `0afaa97`
- DIMENSION click-pick on the dim line `68e80c7`
- Fillet/chamfer object-snap noise while picking `73f0634`
- Fillet P re-apply updates radius (no stacking) `3fed3b2`
- Fillet too-big radius guard + re-prompt `113cab0`
- Generalized fillet/chamfer (polylines/arcs/P option) `b0a2291`
  — NOTE: the line↔polyline part is still buggy, see issue #1.
