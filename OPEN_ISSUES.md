# RUST-AutoRASM — Open Issues / Unfinished Work

A living list of bugs that are NOT fully fixed and jobs not finished. Updated as
work progresses. Ask any time and I'll give you the current list.

Branch: `windows-ui-session-2026-06-20`
Last updated: 2026-07-01

## ⏸ HELD — come back to fix (dock/panel polish, revisit together)
- **Inspector dock resize won't stick.** When a dobject is selected, dragging
  the docked Inspector's left-edge splitter snaps back to the content's natural
  width and won't accept a smaller size (works fine when nothing is selected).
  Root cause: egui refuses to shrink a `SidePanel` below its content min width;
  fixing needs `render_props_body` rows to allow shrinking (ellipsis/clip values,
  flexible label column) or a self-managed width.
- **Inspector float header not full-width in all cases.** Floating Inspector is
  hard-capped at `float_w` (264) so the header spans it, but a wide selection's
  values can still clip; a proper fix ties into the resize/content-width rework
  above (flexible/clipping rows). Same root cause.
- **Inspector float height doesn't cap at 50% of canvas.** The floating
  Inspector grows to content height; the intended `float_max_h_frac = 0.5` cap
  isn't visibly limiting it. Revisit with the width rework.

All three share the "content drives the panel size" root cause — fix as one pass.

## ▶ RESUME HERE (next session)
UI redesign of the top bar is in progress and looking good. Done so far: two-line
top bar (customizable Quick Access row + menu categories), tall logo column with
the real PNG (Lanczos-downscaled), slim "Quick access" drop window under the
chevron with painted checkmarks + auto-width. Latest commit: `63c8936`.
**Next, pick up with:**
  1. **Window buttons (min/max/close)** next to "AutoRASM 2026" — needs the user's
     **A** (frameless custom title bar, modern look, needs Win resize handling) vs
     **B** (keep OS title bar). Waiting on the user's choice. (issue U3)
  2. Possibly enlarge the logo column if it still reads small/soft.
  3. The command-icon (drafting tools) strip restyle — "later" per the user.
  4. QAT persistence across restarts (issue U2).
Also still open and parked: the **line↔polyline FILLET bug** (issue #1) — needs a
session dump + the polyline's vertex data.
NOT pushed yet — many local commits (UI + geometry fixes); push when the user OKs.

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

## 🟡 SETTINGS / VARIABLES — registry-driven (mostly done)
- **Single source of truth is now `cad_app/src/varreg.rs`** (240 vars, 40 wired
  = the real UserEnv fields). `Variables.md` + the HTML mockup are superseded by
  it — keep `varreg.rs` updated; reconcile `Variables.md` to match when time
  permits. Decisions locked: show ALL vars, code defaults win, Option A (unwired
  disabled), all three CLI styles.
- DONE: registry `c4998fb`; settings page (sidebar + typed rows + status badges)
  `0199360`; CLI `setvar`/bare-name `b9b53c3`.
- OPEN follow-ups: (1) wire the ~6 overlap vars to LIVE state (OsnOpt↔snap_enabled,
  PkAdd↔select_remove_mode, SnpPri, audit colours) instead of independent fields;
  (2) flip Planned→Active + wire read-sites as features land; (3) old env_bool/
  env_u8/draw_settings_preview helpers now dead (remove or repurpose);
  (4) section icons in the sidebar (mockup has emoji per section).

## 🟡 UI REDESIGN — in progress

### U2. Quick Access Toolbar — persistence + real icons
- `qat_actions` customization is in-session only (resets on restart) — persist it
  (e.g. in UserEnv / a small config). Shortcut glyphs are simple painted
  placeholders; swap for real icon art when the command-icon set is designed.

### U3. Custom title bar / window buttons (min · max · close)
- User wants min/maximize/close buttons next to "AutoRASM 2026" and questions the
  OS "RUST_CAD" title bar. Proper fix = frameless window (`with_decorations(false)`)
  + a custom draggable title bar with the three window buttons, plus resize
  handling on Windows (borderless winit windows need explicit edge-resize, e.g.
  `ViewportCommand::BeginResize`). Deferred pending user OK (resize risk). The OS
  title was renamed to "AutoRASM 2026" in the meantime.

## 🔵 NOT STARTED / DEFERRED

### E1. Ellipse & Ellipse-arc rail commands don't start an interactive tool 🔴
- **Symptom (session dump 2026-07-02):** picking/typing `ellipse` errors
  `usage: ellipse cx,cy …` (it's a **parametric** command needing coords, no
  interactive click-tool); typing `ellipsearc` → `unknown command 'ellipsearc'`
  (the parser has no such command). Because the tool never switches, the next
  canvas click draws with the **previously-active tool** — e.g. a Rectangle.
- **Where:** `DRAW_CMDS` dispatch tokens `("ellipse","ellipse",…)` and
  `("ellarc","ellipsearc",…)` in `cad_app/src/app.rs`; the parser/`run_command`
  don't provide an interactive ellipse / ellipse-arc tool for these tokens.
- **Status:** PRE-EXISTING; surfaced by the Command-Registry work (the registry
  faithfully mirrors the arrays). **Out of scope for the registry migration** —
  `run_command`/parser are frozen there. Needs a dedicated tool/parser fix
  (add interactive ellipse + ellipse-arc tools, or correct the dispatch tokens).

### E2. DObject-snap extension / tracking lines not implemented
- **Symptom:** object snaps don't project **extension lines** (AutoCAD-style
  osnap tracking — hovering an endpoint/edge should rubber-band an alignment
  guide). Requested 2026-07-02; not part of the original UI/registry work.
- **Status:** pending feature — parked for later.

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
- UI: two-line top bar — customizable Quick Access row + menu categories,
  logo column spanning both rows (real PNG asset), slim "Quick access" drop
  window. `4a300d5` `6a8e075` `d733340`
