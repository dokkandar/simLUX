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

### E1. Ellipse, Ellipse-arc & Point commands don't start an interactive tool 🟡
- **Symptom (session dumps 2026-07-02/03):** `ellipse` errors `usage: ellipse
  cx,cy …`, `point` errors `usage: point x,y` (both **parametric**, needing
  coords — no interactive click-tool); `ellipsearc` → `unknown command`. Because
  the tool never switches, the next canvas click draws with the **previously-
  active tool** (e.g. a Rectangle). Same result whether typed OR clicked on the
  rail — the rail dispatches the identical token.
- **Where:** `DRAW_CMDS` tokens `ellipse` / `ellipsearc` / `point`; the parser/
  `run_command` provide no interactive tool for them.
- **Status:** **ellipse/ellipsearc FIX MERGED** from dokkandar (`f15b0c6`,
  parser.rs: bare `ellipse` enters the tool + adds `ellipsearc`) — pending owner
  visual verification. `point` may still need its own tool. Was surfaced by the
  Command-Registry work (the registry faithfully mirrors the arrays).

### E3. PLINE has no interactive sub-options 🔴
- **Symptom:** while the Polyline tool is active, typing `l` runs the global
  **Line** command (parser alias `l→line`) instead of a PLINE sub-option
  (AutoCAD: Line/Arc/Close/Undo/Length…). The tool switches to Line mid-run.
- **Status:** PRE-EXISTING; the parser resolves single-letter aliases globally
  and PLINE's tool state machine doesn't intercept them. Frozen (parser + tool
  state) — separate from the registry migration.

### E4. WALL tool — thickness sub-option + stale prompt line
- **Symptom (2026-07-02):** wall prompt offers `(t = thickness)` but not a `d`
  option the owner expects; and a **stale command-procedure line** renders above
  the command bar/pill during the wall run.
- **Status:** PRE-EXISTING; wall tool state machine + command-bar prompt render.
  Frozen tool logic — separate track.

### E5. DIM / osnap — vertex snap & radial dimensions
- **Symptom (2026-07-02):** DIM snaps the two free ends of a standalone line, but
  does NOT recognise the **shared joint vertex** of a continuous polyline;
  picking an **arc/circle** gives no radial (radius) dimension.
- **Status:** PRE-EXISTING; DIM tool + object-snap logic (related to E2). Frozen
  tool/snap code — separate track.

### E2. DObject-snap extension / tracking lines not implemented
- **Symptom:** object snaps don't project **extension lines** (AutoCAD-style
  osnap tracking — hovering an endpoint/edge should rubber-band an alignment
  guide). Requested 2026-07-02; not part of the original UI/registry work.
- **Status:** pending feature — parked for later.

### R1. Command-registry Phase-2 debug dump — issue to revisit ⚪
- **Reported (2026-07-03):** owner hit an issue with the temporary
  **Tools ▸ Debug ▸ "Command registry dump"** window (Phase 2 verification).
  Details to be captured when we revisit.
- **Status:** DEFERRED by owner — **revisit after the registry migration
  (Phases 5/6/6b/7) is finished**, then check it together. The dump is a
  temporary diagnostic, so this does not block the registry phases.
- **Where:** dump block in `cad_app/src/app.rs` (search `cmd_dump_open` /
  "Command registry dump"); data from `cad_app/src/command.rs` `build()`.

### M1. Dropdown-menu conformance (MENU_DROPDOWN_MENTOR) 🟡
- **Draw menu: DONE but UNCOMMITTED** (working tree, builds green). Custom-painted
  rows (`paint_menu_row`): 20px icon box / 14 gap / 26 band, cyan `(CODE)`, no □,
  surface-2 hover, aligned arrows, `Wall (t = thickness)`, dividers, hug width.
  Method/Insert flyouts decoupled to top level (`menu_flyout` + `render_menu_flyout`),
  **hover-open** (0.30s close delay), click=commit (`dispatch_method`). Edge-to-edge
  hover via zeroed horizontal `menu_margin` + `ui.set_width(w)` + highlight at
  `ui.max_rect().x_range()`. Flyout radius `radius::SM`(4).
- **Pending:**
  1. **Strip the TEMP measurement log** in `paint_menu_row` (the `std::fs::write`
     guarded `name=="Line"`, writes scratchpad `menu_hover_measure.log`) before the
     final commit. Owner still to visually confirm the hover reaches both borders
     (Δ=0 by construction — highlight IS the frame's inner rect).
  2. **COMMIT the Draw pass** (currently only in the working tree, not pushed).
  3. **Convert Modify** (and File/Edit/View/etc.) dropdowns to the same rows —
     they still use the old `menu_cmd_items`/plain-button style.
- **Where:** `paint_menu_row` / `render_menu_flyout` / `MenuIcon`/`Trailing`/
  `MenuFlyout` in `cad_app/src/app.rs`; Draw menu closure (`menu_button("Draw")`).

### M2. Dialog header conformance (HEADER_STANDARD_MENTOR §4) 🔵
- The ~25 `egui::Window` dialogs (Hatch, Block, Insert Block, DWG, raster,
  parametric, + managers) still use egui's default title bar. Adopt the shared
  **Floating `dock::header_band`** (32 chrome band, close ×). Deferred — separate
  pass from the palette (which already conforms).

### E6. "End command" gesture — right-click = Esc = smart end 🔵
- **Requested (2026-07-04):** in the middle of any command, **right-click and Esc
  should do the SAME thing** — an "end command" that resolves by context:
  - **Multi-point draw with enough points** (Line ≥2, Polyline ≥2, Spline ≥3,
    Wall run ≥1 seg) → **commit up to the last PLACED point** (finish, keep the
    geometry; drop the rubber-band segment to the cursor) → then fresh
    (`Tool::None`). The exact primitive already exists: `commit_active_draw()`.
  - **Too few points / non-draw commands / prompt flows / block-insert** →
    **cancel** to fresh (clear pending, cancel flows, `Tool::None`).
  - **Active sub-session** (trim/extend/hatch/offset/array/select) → finish the
    session (as its Enter/Esc do today).
- **Decisions to confirm before building:**
  1. This **changes Esc's current pline/spline behaviour** (today Esc *drops only
     the last vertex*, stays in the tool). New rule = Esc/right-click *finish the
     run*. → move "remove last vertex" to **Backspace** (recommended), or make
     ONLY right-click finish while Esc keeps drop-last (breaks "Esc = right-click").
  2. Which tools commit-partial: proposed **Line / Polyline / Spline / Wall**;
     Circle/Arc/Ellipse/Rectangle/Point just cancel.
  3. **Right-click isn't wired to command-end today** (only logged; primary does
     the clicking) — confirm it doesn't collide with pan / context-menu first.
- **Where:** Esc handler `cad_app/src/app.rs` (search `Key::Escape` ~L21101 — the
  pline drop-last block); Enter/finish path (`commit_active_draw` ~L17964, called
  ~L21550); canvas right-click (`PointerButton::Secondary` ~L23293).
- **Status:** DEFERRED by owner — pending the three decisions above.

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
- **dokkandar/Auto_RASM merge (2026-07-03, `7aecd7d`..`8ecbe47`, 13 commits on
  `windows-ui-session-2026-06-20`):** full GPU renderer (circle/arc/ellipse/line/
  fill pipelines + APX mode + hatch cache), backend crates wholesale +
  **cad_param & cad_raster**, ellipse parser fix (E1), hatch .pat, grips drag-only,
  open zoom-to-fit, PLINE Esc, wall X-junction + explode, DWG open (+ Windows
  dwgconv.cmd), raster→vector editor, parametric constraint mode. Whole workspace
  builds green. **Pending owner visual verification of each feature.** See memory
  `project_autorasm_dokkandar_merge`.
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
