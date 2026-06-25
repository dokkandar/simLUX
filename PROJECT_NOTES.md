# RUST-AutoRASM — Project Notes

Single consolidated place for working notes, design decisions, and pending
discussions from dev sessions. (Formal specs live in their own files:
`ARCHITECTURE.md`, `ROADMAP.md`, `COMMAND_LINE.md`, `Hatch_Pattern_Wishlist.md`,
etc. This file is the running log of decisions + open questions.)

---

## Build & dev environment (Windows)

Dev shifted from Arch Linux → Windows (2026-06-18).

- **Toolchain:** rustup + `stable-x86_64-pc-windows-msvc` (`winget install Rustlang.Rustup`).
  Links against the installed **Visual Studio Build Tools 2026** (MSVC + Windows SDK 10.0.26100).
- **Windows build fix:** `cad_app/Cargo.toml` had `eframe` features `wayland` + `x11`
  (Linux-only). Made them Linux-conditional via
  `[target.'cfg(target_os = "linux")'.dependencies]`, so it builds on Windows
  (native Win32 backend) and the old Arch setup. Re-apply if pulling fresh upstream.
- **Build:** `cargo build --workspace [--release]`. Release profile uses
  `lto=true` + `codegen-units=1` (slow link, ~1 min).

### Auto-rebuild dev loop
- `cargo dev` (alias in `.cargo/config.toml`) or `.\dev.ps1` (`-Release` switch)
  — `cargo-watch` rebuilds + relaunches the app on every save. Keep one
  PowerShell terminal running it; edits auto-reload (the app window blinks).
- It **restarts** the app (in-window state lost each reload). True
  state-preserving hot-reload was declined as too invasive for the ~22k-line `app.rs`.
- **Running-exe lock gotcha:** a running `rust_cad.exe` LOCKS its own file, so
  `cargo build --release` fails with `Access is denied (os error 5)` mid-link.
  Close the app (or kill the process) before rebuilding release. `cargo dev`
  avoids this (kills the old run first). Debug & release are separate files, so
  `cargo run -p cad_app` (debug) works even while an old release exe is open.

---

## ZOOM command (done, 2026-06-18)

AutoCAD-style `zoom`/`z` sub-option flow (`ZoomState` enum + `zoom_*` methods in
`cad_app/src/app.rs`); view driven by `scale` + `world_offset`.
- Wired: **All**(=Extents), **Center**, **Extents**, **Previous** (10-deep
  history), **Scale** (`nX` only), **Window** (+ live amber preview rectangle),
  **Object** (fit selection), **Real-time** (primary drag up/down).
- **Scope decisions (don't re-implement without asking):** All == Extents (no
  drawing-limits concept exists); **Dynamic** intentionally stubbed; Scale is
  `nX`-relative only (no `XP`/paper-space — model-space only — and no absolute scale).

---

## File Open / Save dialog (done, 2026-06-19)

`render_file_dialog` in `cad_app/src/app.rs`.
- **Path bar:** editable, commits on Enter / **Go** (no per-keystroke nav);
  pointing at a file jumps to its folder and preselects it.
- **Drive dropdown:** lists existing drive roots (probes `A:`–`Z:` on Windows).
- **File-type bar:** **DXF (\*.dxf)** | **Native (\*.rsm)**; list filters by type.
  Open defaults to `.rsm`. (NOTE: native extension is **`.rsm`**, not `.rasm`.)
- **Preview pane:** parses the selected `.dxf`/`.rsm` once (cached) and renders a
  fit-to-rect wireframe by temporarily swapping in the preview Document + a fit
  transform; shows `N object(s) · M layer(s)`.
- **Hidden/system filter:** skips Windows hidden (0x2) / system (0x4) entries
  (`$RECYCLE.BIN`, `System Volume Information`) so the list matches Explorer.
- **Layout (2026-06-20):** pinned **top** (path bar) + **bottom** (Type/File/
  buttons) panels via `TopBottomPanel::show_inside`, with the list+preview in a
  filling `CentralPanel`. Window is capped to the screen → always fits, footer
  always visible, **height freely resizable** (earlier `available_height` body
  blew past the screen and clipped the controls).
- **Save (2026-06-20):** `File ▸ Save` writes to the current file (tracked via
  `current_file`, set on open/save); falls back to Save As if none yet.

---

## Selection & editing model (2026-06-20)

- **Pointer-mode selection:** plain click / drag-window = **fresh** selection
  (replace); **Shift** = add; **Alt** = remove. Empty plain click clears.
  Implemented in `click_select(i, shift, alt, fresh)` + `add_window_selection(..,
  shift, alt, fresh)`. KEY FIX: pointer-mode drag applies DIRECTLY (no
  `begin_selection`, which clears) so Shift/Alt+drag add/remove vs the existing
  bunch instead of replacing.
- **Select-session shortcuts** (while a command asks for a selection): `p` =
  previous, `L` = last drafted, `D` = deselect mode. Intercepted in
  `run_command` only when `select_mode != Off`.
- **Del key** erases the selection (focus-independent).
- **LINE is connected:** segments chain (last endpoint → next start); **Esc**
  ends + exits. Re-run `line` for a separate segment.
- **Copy / Paste = Edit menu only** (no Ctrl+C/V keys). Copy → clipboard of
  dobject clones. Paste = placement flow (`PasteState`): pick BASE → DESTINATION
  with a green ghost preview (mirrors COPY), commits clones with fresh handles,
  selected. Esc cancels.

---

## Groups — PLANNED (deferred 2026-06-20)

Requested: an AutoCAD-style **GROUP** — a named set where objects stay
individual/editable but selecting one selects the whole set. The app currently
has **Blocks** (named INSTANCE collections) but **no Group concept**.

Driving use case (user's AutoLISP `c:PasteGroup`): paste clipboard as individual
entities, place them, then ask *"Group pasted objects? [Yes/No]"* and, if Yes,
`_.group` them under a unique name.

**Decision 2026-06-20:** SKIP grouping for now — current Paste already pastes
individual entities + places them (base→dest ghost) + leaves them selected,
which covers the non-group part. Build real Groups later as its own task:
- data model: `groups: Vec<{ name, members: Vec<Handle> }>` (handles, not
  indices, so they survive reindex);
- selection: clicking a grouped object selects the whole group (toggle to edit
  one);
- persistence: store in `.rsm`; DXF GROUPS section later;
- then wire the post-paste "Group? [Y/N]" prompt to create one.

---

## Hatch rules — TO DISCUSS / DECIDE (open)

**Observed (2026-06-19):** selecting a hatch and running Move selects it but it
doesn't move.

**Reason — current model is purely associative.** `cad_kernel`'s `Hatch` stores
ONLY `boundary_handles` (references to other dobjects) and **no geometry of its
own**; the fill is resolved from those boundary entities each frame
(`resolve_hatch_loops`). Consequences:
- `translated()`/`rotated()`/`scaled()` are **no-ops for Hatch**
  (`geom.rs` arms just `h.clone()`), so Move/Copy/Rotate/Scale/Mirror don't move it.
- Deleting/moving a boundary line makes the hatch shrink/vanish or stay put.
- `Hatch::bbox()` is `(0,0)`; hit-test needs the Document to resolve loops.

**Proposed model (user direction, 2026-06-19) — decide later:**
- A hatch should **own a baked, invisible copy of its boundary loops** so it is
  self-sufficient: if the source boundary is erased/removed, the hatch **maintains**
  its stored boundary and stays put.
- Editing the hatch boundary happens only when the user **selects the hatch** and
  changes it (then re-bake the owned loops).
- **Associativity becomes an optional extra layer:** keep `boundary_handles` as a
  link that *re-bakes* the owned loops when a linked source changes — but the owned
  loops are the source of truth. (This is essentially AutoCAD's model.)
- Once a hatch owns geometry, `translated()`/`rotated()`/`scaled()` operate on the
  owned loops → Move/Copy/Rotate "just work", and bbox/hit-test no longer need the
  Document.

**Status:** not implemented. Discuss + decide the mechanism (owned loops format,
when to break vs keep the associative link, migration of existing hatches).

### Move preview (related, pending)
Requested: while moving, show a ghost preview of the selected dobjects following
the cursor (base → destination). COPY and PASTE already have this ghost; MOVE
does not yet. Straightforward; no design fork. Not yet built.

---

## PEDIT + JOIN — in progress (2026-06-22)

`pedit`/`pe` (alias) polyline editor + JOIN. Subcommands: Close/Open, Join (J),
Width (W), Undo (U). `pedit_start` converts a clicked Line/Arc into a polyline so
it can be edited. JOIN (`j` or PEDIT→J) picks entities and chains touching
Line/Arc/open-Polyline/Spline into one polyline.

### Architecture
- `pedit_join_selected` (app.rs): EXPLODES every picked entity into Line/Arc
  primitives (`explode_polyline`, spline tessellate) BEFORE join, so the kernel
  never mis-handles a whole polyline as one straight segment. Then calls
  `cad_kernel::join_geoms`. Result = merged piece(s) + any unconsumed primitive
  (geometry never lost).
- `cad_kernel::join_geoms` 3 passes: (1) collinear-line merge, (2) concentric-arc
  merge, (3) touching chain → polyline (`find_touching_chain` + `chain_to_polyline`).

### Geometry bugs fixed THIS session (all in cad_kernel/src/geom.rs unless noted)
1. **chain endpoint tolerance** — added `CHAIN_EPS = 1e-3` (vs precision
   `JOIN_EPS = 1e-6`) for endpoint coincidence in find_touching_chain /
   chain_to_polyline. Arc ends reconstructed via trig sit a few×1e-6 off.
2. **`Arc::reversed()` was broken** — it set start=old_end but KEPT the positive
   sweep, producing a DIFFERENT arc (far end at P(start+2·sweep)). It stalled the
   join walk AND corrupted the standalone `reverse` command (teleported arcs).
   Fix: an Arc can't encode CW under positive-sweep, so reversed() now returns the
   SAME arc (identical geometry). EllipseArc::reversed() same fix. Test updated.
   Also: chain_to_polyline no longer routes the far-endpoint through reversed() —
   it reads it directly from endpoints + entry side.
3. **collinear merge bridged trim gaps** — `find_collinear_line_group` merged any
   two collinear lines regardless of distance, so joining the two stubs a trim
   leaves (line cut at a circle crossing) REDREW the removed middle piece. Fix:
   gap-aware — only a contiguous, touching run (gap ≤ CHAIN_EPS) collapses.
4. **`bulge_from_arc` sign wrong for MAJOR arcs** — sign came from chord-side,
   valid only for minor arcs (sweep < π). For a major arc the chord-side flips
   while tan(sweep/4) already encodes the wide span → inverted curvature. Fix:
   sign now from traversal direction (compare CCW angle start→end vs swept
   magnitude). Renderer (`append_pline_segment_screen_pts`) and
   `polyline_segments` were verified to compute the SAME center as the kernel.
5. **polyline picking ignored bulges** — `Polyline::bbox()` and
   `distance_to_point()` treated every segment as a straight chord. A major arc
   bows outside its chord, so its bbox excluded the arc → spatial index culled the
   click → only the line part was selectable. Fix: both now expand each bulged
   segment into its true Arc via `polyline_segments` and union/min over those.

Kernel tests: 170 pass (added regressions for #3, #4, #5).

### OPEN BUG — arc curvature after `pe` + `j` (UNRESOLVED, next session)
User reports: after merge the arc "curves the wrong way." Curvature was confirmed
OK at the 18:04 build (fix #4); only the picking fix (#5, bbox/distance, read-only)
landed after, which can't change curvature. My round-trip analysis says all four
cases (minor/major × forward/reverse) preserve curvature. Likely a different arc
orientation exposes a remaining case, OR something not visible from analysis.

**Diagnostic added** (app.rs `pedit_join_selected`, build 18:21): before "merged
into polyline" it prints each `src arc` (center/r/start°/sweep°/endpoints) and each
resulting `pl v[i]` (pos + bulge). NEXT STEP: get a session dump with those lines,
compare stored bulge vs source arc to determine if sign/magnitude/wrong-vertex.
Remove the diagnostic once fixed.

### Other deferred
- Real Groups file persistence (in-session only now).
- Move ghost preview.
- Reconcile origin/main's +commits with this branch (overlapping file-dialog/zoom
  work — do NOT blind-merge).

---

## Module split refactor (2026-06-23) — pure code movement, no behaviour change

To let multiple agents edit features without colliding in the monolith files,
trim / join / fillet-chamfer-offset / pedit were extracted into their own files.

**Kernel** (`cad_kernel/src/`):
- `geom.rs` (was 4402 lines) now holds ONLY type defs (Geom, Line, Arc, …) and
  their core methods (bbox, distance_to_point, endpoints, transforms, grips).
- `join.rs` — `join_geoms` + chain/collinear/concentric helpers + bulge math
  (`bulge_arc`, `bulge_from_arc`, `polyline_segments`) + `JOIN_EPS`/`CHAIN_EPS`.
- `trim.rs` — `Geom::trim_at`, `join_trim_survivors`, `circular_union`, `same_ellipse`.
- `modify.rs` — `Geom::offset`, `fillet_lines`, `chamfer_lines`, offset helpers.
- `lib.rs` re-exports unchanged at the crate root (`cad_kernel::join_geoms`, etc.)
  so external callers are unaffected. `JOIN_EPS` is `pub(crate)` (shared join+trim).
- Tests stayed in geom.rs (the 3 `#[cfg(test)]` modules) with added
  `use crate::{join,trim,modify}::*;`. 170 tests pass. (Could relocate tests into
  each module later — optional follow-up.)

**App** (`cad_app/src/`):
- `app/pedit.rs` — all `pedit_*` methods (child module of `app`, so it reaches
  CadApp's private fields/helpers via `use super::*`; methods are `pub(crate)`).
  `app.rs` declares `mod pedit;` near the top. `explode_polyline` + `idx_of_handle`
  stayed in app.rs (shared with non-pedit code).

Rule for future splits: child-module-of-app for app-layer features (keeps private
access); separate top-level module + `pub use` re-export for kernel features.

---

## 2026-06-23 session — module split, pedit, arc/pline tools, tapered width

All on branch `windows-ui-session-2026-06-20` (16 commits, 0a0e460 → 76f2704). NOT pushed.

### Done & verified
- **Module split refactor** (`0a0e460`): `geom.rs` split into `join.rs` / `trim.rs`
  / `modify.rs`; PEDIT moved to `cad_app/src/app/pedit.rs` (child module). Crate-
  root re-exports unchanged. 170 kernel tests pass. (See "Module split refactor".)
- **PEDIT**: join elliptical arcs (tessellated) + Enter-repeats-pedit (`831998f`);
  accept ellipse-arc/spline as the pedit TARGET (`54fba67`); AutoCAD-style
  "select object" entry when nothing pre-selected (`5ba29e2`).
- **parser**: bare `arc` / `pl`/`pline` now start the interactive tool (`db6af5b`).
- **PLINE sub-commands**: Close wired (was leaking to global Copy) (`b191cb8`);
  Arc Direction `d` wired (`e0a2275`).
- **Tapered width** (the big one): per-segment `(start,end)` width on `Polyline`
  (model stage 1, subagent-assisted ripple). Tool `w`/`h` entry, live preview,
  sticky forward-only width, empty-Enter-accepts-default fix, trim preserves
  width (wrap_with_width). Commits b6cd8b5, 679453d, 0cb318b, cbf71d6, 467cde6.

### Width RENDERING — iterated a lot, STILL not perfect (resume here)
Rendering a variable-width polyline as filled strips went through several attempts
(all in `fill_width_strip`, cad_app/src/app.rs):
1. per-segment quads + round discs → round corners (user wanted sharp).
2. miter triangles → black notches at corners.
3. mitered shared offset points → SPIKES on sharp/reflex/self-intersecting.
4. independent rects + convex-hull joints → no spikes but FACETED outer edge.
5. **current (`76f2704`)**: per-vertex miter decision — within 8× half-width
   limit, adjacent quads share the miter apex (smooth seam); beyond it, each
   segment uses its own normal + convex-hull bevel fill (no spike).
**STATUS:** user's latest screenshot still shows some edge imperfection on a
self-intersecting wide scribble (slight overlap/jagged in places). Acceptable for
normal polylines; pathological scribbles still rough. Revisit if needed — consider
building proper left/right offset polylines with bevel-insertion (two points at
clamped corners) and filling as a single triangle strip.

### KNOWN WIDTH ISSUES / TODO
- **Width-change boundary taper bug:** `polyline_width_centerline` assigns each
  vertex ONE width (the incoming segment's END width). At a width change (e.g.
  2→10) the first new-width segment tapers from the old end-width to the new end-
  width instead of being uniform. Fix: store start/end widths per segment in the
  centerline rather than one width per point.
- **Trim splits into separate 1-segment polylines** → joints BETWEEN pieces
  butt-cap (no miter across separate dobjects). To keep sharp corners after trim,
  rework polyline trim to emit CONNECTED multi-segment runs (split only at the
  removed part) retaining widths. (Width itself IS preserved per piece.)
- **Stage 4 NOT done — DXF/RSM persistence of widths.** Save/reload currently
  DROPS width (read paths default `widths: Vec::new()`). Need DXF LWPOLYLINE
  group codes 40/41 (start/end width) + 43 (const width), and RSM serialization.

### Notes
- A running `rust_cad.exe` locks the release exe — kill before `cargo build
  --release`. cargo not on PATH in PowerShell: prefix `$env:Path +=
  ";$env:USERPROFILE\.cargo\bin"`.
- OPEN from earlier: circular-arc "curves wrong after pe+j" — diagnostic in
  pedit_join_selected prints `src arc:` / `pl v[i]`; still need a dump to close.

---

## 2026-06-25 session — generalized FILLET/CHAMFER (polylines, arcs, P option)

Branch `windows-ui-session-2026-06-20`. See cmt.txt for the per-commit handoff.

- FILLET/CHAMFER were line+line only; now handle line/arc pieces and polylines:
  two segments of one polyline (round/bevel that corner in place), a polyline
  END segment vs a separate object, and the AutoCAD `P` option (pick one polyline
  → all corners). Bare Line↔Arc and Arc↔Arc now work too.
- New kernel module `cad_kernel/src/fillet.rs`: offset-locus tangent-circle solver
  (line→parallel offset line, arc→concentric R±r circle; intersect loci, pick the
  centre inside the corner nearest the vertex). Public: fillet_geoms/chamfer_geoms,
  fillet_polyline_corner/chamfer_polyline_corner, fillet_polyline_all/
  chamfer_polyline_all, nearest_polyline_segment. 5 tests; 175 kernel tests pass.
- app.rs: apply_fillet/apply_chamfer are now dispatchers (corner / lines+walls /
  general). `p` sub-command toggles poly-all mode; prompts show ", POLY" + p=poly.
- OPEN: spline/ellipse-arc fillet (no offset locus — tessellate or numerical);
  non-adjacent polyline-segment fillet (AutoCAD removes intervening segments).

---

## 2026-06-24 session — trim/extend/fillet, width polish, last-point, cmt.txt

Branch `windows-ui-session-2026-06-20` (pushed). See cmt.txt (repo root) for the
full per-commit handoff log.

- pline width render: final approach = mitered shared offset (within 8x limit)
  else convex-hull bevel; ONE convex polygon per fill (no triangle-fan seams).
  Clean solid edges, sharp corners, no spikes.
- EXTEND now works on polylines (extends end segment nearest the pick).
- TRIM: open polylines keep CONNECTED runs (rest stays one polyline, mitred,
  widths preserved); clicking a segment that meets NO cutter REMOVES it (works
  even when the polyline is the only selection — kernel + app guards bypassed
  for polyline targets). Closed polylines still explode (v1).
- FILLET: continuous by default (loop until Esc); `r` changes radius mid-command
  (persists as env.FltRad default); `m` single-shot; `t` trim toggle.
- DRAW last-point: Enter/Space at a draw tool's first-point prompt continues
  from the last picked point (line/arc/circle/ellipse/polyline/rectangle/
  spline/point). CadApp.last_point + feed_first_point_from_last.
- Added cmt.txt — full change log + keys/feature reference + architecture notes
  + open work, as a handoff for other coding agents.

OPEN (next): DXF/RSM width persistence (Stage 4, still not done); width-change
boundary taper quirk; chamfer continuous-by-default (optional); groups .rsm
persistence; circular-arc "curves wrong after pe+j" (needs dump); reconcile
origin/main (~14 overlapping commits) before merging to main.
