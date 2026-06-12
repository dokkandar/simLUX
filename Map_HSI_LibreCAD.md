# Map HSI LibreCAD — session resume notes

**Date span:** 2026-06-09 → 2026-06-10
**Project:** `~/workspace/RUST_CAD` (pure-Rust 2D CAD math workbench, eframe+glow)
**State at save:** `cargo build --release` clean (~30 s); 11/11 `cad_kernel::dim::tests` pass; 11/11 `cad_app::hatch_trace::tests` pass; binary at `~/workspace/RUST_CAD/target/release/rust_cad`

---

## 1. What shipped this session

### 1.1 Selection model — drag-to-window-select fix
**Symptom user reported:** 1244 px R→L drags in idle pointer mode triggered NO selection. Recorder verdict: "drag DEMOTED TO CLICK".

**Root cause (two stacked bugs):**
1. The unified click/drag classifier only treated `in_select && hold_threshold_passed` as a window-drag — pointer-mode-idle (Tool::None, no edit phase) was silently demoted to a click. Per [feedback_rust_cad_pointer_is_selector](.claude/projects/-home-HSI-workspace-qlcplus-master/memory/feedback_rust_cad_pointer_is_selector.md), pointer mode IS the always-on selection tool.
2. Even with that fixed, `press_release_dist` was reading 0.0 because egui's `Pointer::press_origin()` is cleared on the same frame as `drag_stopped()` fires. So the `> 1.0` motion gate always failed.

**Fix (file: [cad_app/src/app.rs](cad_app/src/app.rs)):**
- New `press_pos: Option<(egui::Pos2, Vec2)>` field on `CadApp` (mirrors `press_time` — populated on `primary_pressed`, cleared on `primary_released`)
- Classifier captures `press_pos_this_frame = self.press_pos` BEFORE the release handler clears it, then reads from the snapshot
- Window-drag application + rubber-band preview both read from the snapshot too
- Classifier now triggers `drag_intent_is_window` on: `in_select && passed` OR `pointer_mode_idle && passed` OR `shift_held`

### 1.2 Text input dialog (discoverable popup)
**Symptom:** User clicked Text tool, then click on canvas, then typed their complaint as the body — they had no idea a popup was supposed to open.

**Fix:**
- New floating dialog `render_text_input_dialog` in [cad_app/src/app.rs](cad_app/src/app.rs)
- Opens automatically at the click anchor (`current_pos` only on the first frame after open → draggable thereafter)
- Single-line TextEdit auto-focused
- Style picker (combo) reads from `doc.text_styles`; selecting a style with `default_height > 0` auto-fills Height
- Height field seeded from `TxHt` SYSVAR
- **"+ New…" button** opens the existing TextStyleDialog (`style` cmd) so user can create Title/Dimension/Drawing-Title styles without leaving — auto-selects the newest after OK via `text_input_dialog_style_count_before` sentinel
- Enter commits / Esc cancels / OK / Cancel / window-X all wired
- Cmd-line capture is suppressed while dialog is open so they don't race
- Live canvas preview now reads from `text_input_dialog_buf` when dialog is open (uses dialog Height + selected style font)
- Font choices expanded in TextStyleDialog: `standard` + `monospace` (was hardcoded to standard only)
- New helper `font_id_for_font_name(name, size_px)` centralizes the mapping; used by committed-text renderer + live preview

### 1.3 Hatch boundary detection — TWO bugs in one screenshot
**Setup:** Big circle + 4 lines that cross it + a small island circle inside; user picks inside a chord-bounded sub-region.

**Bug A — trace returns None when lines have dangle stubs**
- After `split_at_intersections`, each crossing line becomes 3 segments: outside-left / inside / outside-right
- The two outside stubs' outer endpoints are degree-1 graph nodes (dangles)
- The CCW walker walked INTO a dangle, found no valid next edge, returned None
- App fell back to cheap path which used the WHOLE circle as outer (ignoring the chord lines)

**Fix:** New `prune_dangles()` pass in [cad_app/src/hatch_trace.rs](cad_app/src/hatch_trace.rs) (~line 647-682). Iteratively removes degree-1 clusters + their adjacent segments until stable. After convergence every surviving edge is on at least one cycle. Hits on pruned segments are filtered from the ray-cast results.

**Bug B — islands not crossed by the +X ray are missed**
- After fix A, the chord-bounded region traces correctly. But the small island circle ABOVE the seed's y-coordinate was hatched-through because the +X ray never hit it
- Trace algorithm only finds loops the ray crosses

**Fix:** New `augment_islands_from_closed_dobjects()` in [cad_app/src/hatch_trace.rs](cad_app/src/hatch_trace.rs). After the main trace, scan every closed kernel dobject (Circle / Ellipse / closed Polyline) in scope:
- bbox check vs outer
- seed must NOT be inside the candidate poly
- every vertex of candidate must be inside outer
- dedup vs existing islands
- if all pass → add as island

Wired into all 3 entry points (`trace_boundary_at`, `trace_boundary_at_in_view`, `trace_boundary_at_in_view_cancellable`).

**Tests added (file: [cad_app/src/hatch_trace.rs](cad_app/src/hatch_trace.rs)):**
- `circle_chord_with_outside_stubs_traces_half_disc`
- `circle_with_many_crossing_lines`
- `island_above_seed_is_detected_by_doc_scan`

### 1.4 Dimensions slice 1 — full end-to-end smart `dim` command
User picked: **Linear + Radius + Diameter, single auto-decide command, FULL DIMVAR-parity DimStyle (~70 fields)**.

**Kernel — new file [cad_kernel/src/dim.rs](cad_kernel/src/dim.rs):**
- `DimKind` enum: `Linear { p1, p2, dimline_pos, ortho }`, `Radius { center, on_circle, leader_end }`, `Diameter { ... }`
- `LinearOrtho`: Horizontal / Vertical / Aligned
- `Dim` struct: kind, style (u32), text_override (Option<String>)
- `DimStyle` struct with ~70 DIMVAR-equivalents using DESCRIPTIVE Rust names (`arrow_size` not `dimasz`). Defaults match AutoCAD STANDARD.
- `DimStyleTable` analog of TextStyleTable, STANDARD at id 0
- Methods on `Dim`: `measured_value()`, `formatted_text(&style)`, `with_points_mapped(f)`, `bbox()`, `grip_points()`
- Helpers: `round_to`, `suppress_zeros` (DIMZIN bits 4/8 leading/trailing), `parse_dimpost` (`<>` placeholder)
- 11 unit tests, all passing

**Kernel — [cad_kernel/src/geom.rs](cad_kernel/src/geom.rs):**
- `Geom::Dimension(Dim)` variant
- 3 new `GripRole`s: `DimP1`, `DimP2`, `DimLeader`
- Match arms added to EVERY method (rotated/scaled/mirrored/translated/reversed/distance_to_point/bbox/grip_points/with_grip_moved; Err on lengthened catch-all / trim_at / extend_to / offset / split_at)

**Kernel — other files:**
- [cad_kernel/src/document.rs](cad_kernel/src/document.rs): `pub dim_styles: DimStyleTable` field
- [cad_kernel/src/lib.rs](cad_kernel/src/lib.rs): re-exports
- [cad_kernel/src/snap.rs](cad_kernel/src/snap.rs): Dim arms for End/Mid/Cen/Per/Tan/nearest_on_geom (snap to def points)
- [cad_kernel/src/intersect.rs](cad_kernel/src/intersect.rs): Dim returns empty (annotations don't intersect)
- [cad_kernel/src/parser.rs](cad_kernel/src/parser.rs): `Command::Dim` + `Command::DimStyle(Option<String>)`; keywords `dim`/`dimension`/`dimstyle`/`ddim`

**IO:**
- [cad_io/src/rsm.rs](cad_io/src/rsm.rs): tag 11 = Dimension; round-trips all 3 kinds + style id + text_override (dim_styles table NOT yet round-tripped — reader uses Default)
- [cad_io/src/dxf.rs](cad_io/src/dxf.rs): writes as exploded TEXT for v1 — full DIMENSION group codes deferred

**App — [cad_app/src/app.rs](cad_app/src/app.rs):**
- `Tool::Dim` variant
- `DimDraftState` enum: `Off | WaitingForP1 | WaitingForP2 { p1 } | WaitingForDimLinePos { kind }`
- `DimDraftKind`: `Linear { p1, p2, ortho }` | `Radius { center, on_circle }` | `Diameter { center, on_circle }` — the "half-built" kind before the leader/dimline_pos click
- `dim_draft` field on `CadApp` + default
- `Command::Dim` handler: sets `Tool::Dim` + `WaitingForP1` + prompt
- `Command::DimStyle` handler: stub for now (dialog ships next slice)
- `handle_dim_click()`: auto-decide flow. First click on Circle/Arc → Radius (jumps to WaitingForDimLinePos); else → linear waiting for p2
- Click intercept in canvas update at the spot where `pending.push(click_world)` happens — Dim flow bypasses `pending` entirely
- Cmd-line intercept: while `WaitingForDimLinePos` with Radius kind, typing `D`/`dia`/`diameter` + Enter flips to Diameter (and `R`/`rad`/`radius` flips back)
- Esc handler resets `dim_draft = Off`
- **`draw_dimension()` + `draw_filled_arrow()`** — full renderer for all 3 kinds:
  - Linear: extension lines (with offset/extend per style) + dim line + 2 inward arrows + centered text above dim line
  - Radius: center→on_circle leader + on_circle→leader_end leg + arrow at on_circle pointing toward center + "R<value>" text
  - Diameter: two-arrow leader through center (on_circle ↔ antipode) + leg to leader_end + "⌀<value>" text
- Live ghost preview during WaitingForP2 (chord guide line) and WaitingForDimLinePos (full ghost dim follows cursor)
- Toolbar icon arm: horizontal dim line with arrows + two extension lines
- Status-strip arm: shows phase ("click first point" / "click second point" / "click dim line position")
- All other completeness arms (selection-count tally, trim-debug kind label, `dobject_kind_name`, `describe`, `describe_verbose`, `list_full_details`, `draw_grips`, `draw_dobject`, `draw_dobject_dashed`, hatch_trace tessellator)
- [cad_cli/src/main.rs](cad_cli/src/main.rs): accepts Dim/DimStyle in the editing-op ignore list

---

## 2. Decisions baked in (don't re-litigate)

- **Single `dim` command, auto-decide** — NOT separate `dimlinear` / `dimradius` etc. AutoCAD-style smart command. User explicitly picked this.
- **DimStyle field names are descriptive Rust** (`arrow_size`, `text_height`) — NOT cryptic DIMVAR codes. Per [feedback_rust_cad_settings_naming](.claude/projects/-home-HSI-workspace-qlcplus-master/memory/feedback_rust_cad_settings_naming.md), cryptic naming is reserved for UserEnv settings; per-entity style data uses readable names. DXF serializer maps to DIMVAR codes at write time.
- **3 grip roles per Dim** (DimP1/DimP2/DimLeader) — uniform across kinds; `with_grip_moved` interprets per-kind.
- **RSM tag 11 = Dimension**; encoding inline-documented.
- **DXF write as exploded TEXT for v1**; full DIMENSION group-code support deferred to DXF parity pass.
- **`dim_styles` does NOT round-trip through RSM yet** — reader uses `Default::default()`. Follow-up.
- **AutoCAD chord-trace fix**: pruning dangles is correct because every surviving edge needs to be on at least one cycle for the CCW walker to close.
- **Island doc-scan**: only closed primitives (Circle, Ellipse, closed Polyline) — line-bounded sub-regions still need the ray to find them. Most-likely-fine for v1.

---

## 3. What's STILL OWED (next session)

### Walls / smart dobjects (in progress 2026-06-10)
**▶ RESUME HERE: curved-wall S1–S3 DONE 2026-06-12 (uncommitted).** Next up, in order: **S4 curved-wall correctness** (pick/snap/bbox/trim — own slice) OR **T-junction (#2)**, user's call; then X-crossing / collinear. Project plan (mentor-review sequence) still pending: doc re-baseline → RSM style-table round-trip → matchprop source-first rework. Build GREEN, all workspace tests pass (154 kernel / 17 app / 3 wall).
- [x] **Scenario 1b — rounded corner (fillet r>0 on walls)** — DONE 2026-06-11. `Wall.bulge` field (DXF tan(sweep/4)) makes a curved-centerline wall; `bulge_arc`/`bulge_from_arc` in geom.rs. `fillet` r>0 on walls trims the two straights to tangent + spawns a CURVED corner wall (bulge from the fillet arc). Render offsets a tessellated centerline per-point → concentric face arcs. `solve_faces` skips curved neighbours (tangent join). Transforms preserve bulge (mirror/reverse negate). Round-trip test passes.
- [x] **Wall styles + Wall Style Manager** — DONE 2026-06-11. `WallStyle`{name,thickness,fill_color,face_color,description} + `WallStyleTable` (kernel `wallstyle.rs`) on Document; `Wall.style:u32`. `wallstyle`/`wstyle` command + **Wall menu** open the Manager (list + preview + Set Current / New / Modify), New/Modify → `WallStyleDialog` form, colors via the shared ACI wheel (`WallColorSlot`). Set Current syncs `WlThk`. New walls carry `current_wall_style`. Render: face color from style + **poché fill** (solid tint of `fill_color`, straight walls). RSM defaults wall_styles (not round-tripped yet).
- [x] **Scenario 1 — L-corner sharp miter** — DONE. New `cad_app/src/wall.rs` (`mod wall` in main.rs): `solve_faces(this, all_walls)` derives mitred faces wherever a wall's end coincides (`JOIN_TOL`) with another wall's end. Model A (independent segments, per-frame from coincidence, centerline kept as identity). Render Wall arm calls it; falls back to raw ±t/2 faces when no neighbour. 3 tests (90° miter, any-angle meet-at-a-point, lone wall). Spec: `Smart_Dobjects.md`.
- [x] **Wall `t` thickness sub-option** — DONE 2026-06-10. While the wall tool is active, typing `t` (or `thickness`) arms a wait → next number sets `WlThk`; `t5`/`t 5` sets directly. New field `wall_waiting_thickness`; interceptor sits with the dim-`D`/polyline-`c` sub-option handlers (eaten before the parser); Esc resets it; prompts/hint mention it. (`wall <t>` at command start still works too.)
- [x] **Chained wall drawing** — DONE 2026-06-10. `try_finalise` Wall arm now RETAINS the segment end as the next start (was `pending.clear()`), so click-click-click draws a connected run whose shared endpoints auto-mitre live via the solver. Enter ends the run (added to the empty-Enter cascade; tool stays active for a new run); Esc cancels (existing). Degenerate click drops the dup point, keeps the anchor. Prompts + status strip updated. Build clean, 3 wall tests pass.
- [x] **Reach + corner (gap between walls) via fillet-on-walls** — DONE 2026-06-10. `apply_fillet` now accepts Wall (operates on its centerline): r=0 extends/trims the two centerlines to their intersection, re-wraps them as Walls (identity kept) → ends coincide → `solve_faces` mitres. Fixes the user's failed `fillet` on walls (was Line+Line only). r>0 rejected (needs scenario 1b). Honors TrmMd. Build clean; wall 3/3, kernel fillet 27/27.
- [ ] **Scenario 1b — rounded corner** — algorithm captured (fillet centerlines by R, then offset → concentric-arc faces; reuse `fillet_lines`; SYSVAR `WlCorR`). NOT built.
- [ ] **Scenario 2 — T-junction (any angle)** — captured (trim branch faces to through near-face + open the through face between them; detect END-on-interior-of-centerline). NOT built.
- [ ] N-way nodes (T/X) in `solve_faces` picks first neighbour only; rounded render; openings; rooms.
- [ ] **Wall-as-full-smart-dobject track** (user 2026-06-10, parallel to the junction scenarios): `WallStyle`+`WallStyleTable`+`Wall.style` (mirror DimStyle); **Wall Style Manager dialog** (adapt the DimStyle Manager) with named types (Dry Wall / Structural / …); **hatch fill inside** (poché — needs `solve_faces` to also emit a closed footprint polygon); **convert closed dobject → wall** (path = centerline, offset ±t/2). Full plan in `Smart_Dobjects.md` §2.5. Reuses DimStyle Manager + hatch fill + ACI wheel + polyline offset.

### BUG FIX: draw-capture ignored CARD + grid (2026-06-12, uncommitted)
- [x] Found via session dump: line preview/CLICK log showed the CARD-locked point (Y=anchor) but the COMMITTED line used the raw cursor. Root cause: TWO `click_world` computations — the draw-tool path (app.rs ~15365, feeds `pending`→geometry + `handle_dim_click`) returned **raw `world`** in its no-snap branch, while the move/copy path (app.rs ~14725) and the live preview (`cursor_world_constrained`) both `apply_constraints`. So drawn geometry silently ignored CARD **and** grid-snap. Fix: no-snap branch now returns `self.apply_constraints(world)` (priority osnap > CARD > grid > raw). Preview and commit now agree across all draw tools (line/pline/spline/wall/text/dim). Affected EVERY draw tool, not just lines.
- [x] **Part 2 (preview rubber-band)**: the draw-tool preview block (app.rs ~16637) had the SAME class of bug independently — its no-snap branch tracked the raw cursor (`self.s2w(raw_cursor, rect)`), so with CARD on the rubber-band drew diagonal to the cursor even though the commit went horizontal. Fix: the preview now goes through the shared `cursor_world_constrained(Some(raw_cursor), rect, snap_hit)` helper (osnap > CARD > grid > raw), with `cursor` = `w2s(cw)` so the band end and the snap-marker glyph coincide. Preview now shows the line going to the CARD-allocated point. Both `cw` and `cursor` names preserved → all downstream tool arms (Line/Wall/Circle/Polyline/…) unchanged.

### BUG FIX: fillet/chamfer bare-Enter ignored "keep current value" (2026-06-12, uncommitted)
- [x] User: "once fillet says current value is 0.5, if user doesn't enter a value and presses Enter, treat it as the current value and continue." Root cause: the "Enter = keep" logic lives in `run_command`'s `fillet_waiting_radius` branch (app.rs ~1651), but `run_command` is ONLY called for a NON-empty cmd line (app.rs ~14111 `if !self.cmd.trim().is_empty()`). So a bare Enter never reached it — it fell through the empty-Enter cascade (~12524) to "repeat last command", leaving the radius prompt stuck. Fix: added cascade branches (BEFORE the repeat-last `else`): (1) `fillet_waiting_radius` → keep `FltRad`, advance to `WaitingForFirst`, refresh prompt; (2) main `fillet_state` WaitingForFirst/Second → keep current radius, re-issue prompt (consume Enter, don't restart); (3+4) chamfer mirror — `chamfer_dist_wait` WaitingD1 (keep ChmDs1 → ask D2) / WaitingD2 (d2=d1, → WaitingForFirst), and main `chamfer_state` keep-and-stay. Walk-whole-pipeline: fixed chamfer too (identical structural bug).

### BUG FIX: fillet arc swept the wrong way on non-right corners (2026-06-12, uncommitted)
- [x] User screenshot: fillet arc bulged OUT of the corner instead of rounding it. Root cause in `cad_kernel::fillet_lines` (geom.rs ~3027): start-angle selection rotated `v1` toward the I-direction and accepted the CCW start on `dot > 0` — a **90°-wide acceptance window**. Hand-traced θ=120°: it picked start=v1 + CCW sweep that ends at the WRONG point (the minor arc there is CW from v1). Only θ=90° happened to land correctly (knife-edge dot=0/1). Fix: the fillet arc is always the MINOR arc (central angle π−θ) and — because I lies outside the circle (dist r/sin(θ/2) > r) — the minor arc always bulges toward the corner vertex. New selection: `d_ccw = CCW(tp1→tp2)`; `start = tp1 if d_ccw ≤ π else tp2`; sweep = π−θ (always CCW/positive). Deterministic, no dot threshold. Regression test `fillet_obtuse_120deg_arc_bulges_toward_corner` asserts endpoints land on both tangent points AND midpoint bulges toward I (old code put it diametrically opposite). Existing right-angle test still green (only checked center/radius/sweep-magnitude, all unchanged). Also fixes curved-WALL fillet (same arc feeds `bulge_from_arc`). Kernel tests 157 total.

### NEW: Rectangle command `rec` + recorder command renamed (2026-06-12, uncommitted)
- [x] `rec` (also `rectangle`/`rectang`) now draws an axis-aligned rectangle. **Two modes** (user spec): (a) click first corner → click OPPOSITE corner; (b) click first corner → type `W H` (or `W,H`) width/height — signed, negatives extend left/down. Committed as ONE **closed 4-vertex Polyline** (LWPOLYLINE, like AutoCAD RECTANG). New `ToolKind::Rectangle` (parser) + `Tool::Rectangle` (app). Plumbing: SetTool map, `current_hint`, `try_finalise (Tool::Rectangle,2)` via shared `rect_polyline(a,b)` free fn, `card_anchor` (CARD works, first corner = anchor), tool-icon glyph (square + 2 corner dots), status badge, toolbar button "rect" (after line), Draw menu item, live rubber-band preview (`egui::Rect::from_two_pos`, honors osnap/CARD/grid via the shared constrained-cursor). Typed-dims intercept in `run_command` runs BEFORE the main parser (else "5 3" errors); tool stays active after each rectangle (like line).
- [x] **Recorder command renamed**: `rec` previously aliased the Session Recorder (`DbgRecorder`). Freed `rec` for rectangle; recorder now opens via `recorder` or `dbg` only (parser.rs).

### Block dialog rebuilt + smart-block flag (2026-06-12, uncommitted)
- [x] User: "block dialog is very simple; minimum = preview of selection, insertion point, colour, select dobject; make it smart block (algorithm later)". New `BlockDialog { name, base_x, base_y, color_aci, smart }` + `BlockDialog::new(base)`/`base_point()`. Dialog now has: **live PREVIEW** (wireframe of the selection via new `preview_world_polylines(&self, g)` — samples curves, exact Wall faces, recursive BlockRef, bbox-rect for Text/Dim, hatch loops — drawn into a clipped box with a uniform fit transform + per-dobject `resolve_color`); **Select objects…** button (round-trip: `block_dialog_stash` + `QueuedOp::BlockReopen`, restored on Enter); **Insertion point** X/Y fields + **Pick ⊕** (round-trip via `block_dialog_pick_base`, next click fills X/Y & reopens); **Color** swatch+ACI… (new `AciPickRequest::BlockForm` → writes `color_aci`) + "inherit"; **Smart block** checkbox; OK validates name/dupe/empty-selection and calls `apply_block_create(name, base, color_aci, smart)` directly (no separate base-click); Existing list shows ⚙ for smart blocks.
- [x] Kernel `Block` gained `smart: bool`. `apply_block_create` sig +color_aci +smart (explicit ACI overrides inherited color). RSM: reader defaults `smart:false` (NOT persisted yet — ships with the algorithm, like dim/wall styles); 4 Block literals updated. Esc clears stash + pick flag. Classic `block <name>` base-click path passes (None,false). `blocks_round_trip` still green.

### Mirror: axis preview + keep-original prompt (2026-06-12, uncommitted)
- [x] User: "mirror should show the preview of the mirror axis; should ask whether keep original, default yes, else type n/no/ni". Added `MirrorState::AwaitingKeep(a,b)`. Flow: pick A → pick B → **prompt** "keep original? [Y]/n". Answer: Enter (empty-Enter cascade) / `y`/`yes`/`keep` = keep a COPY (default); `n`/`no`/`ni` (run_command intercept, before main parser) = erase original. `apply_mirror(a,b,keep_original)`: keep → append mirrored copies (fresh handles, like `apply_copy`); !keep → flip originals in place (old behavior). **Axis preview** overlay (violet, marching-ants, extended ~40px past both ends) + base blip + **ghost of the mirrored selection** shown during WaitingForB (live cursor as B) AND AwaitingKeep (fixed B). Esc cancels (existing handler).

### Full LibreCAD linetype set + graphical picker + multi-segment renderer (2026-06-12, uncommitted)
- [x] User: bring LibreCAD's standard linetypes into RustCAD + a graphical picker (showing line/dash/dots). Source: LibreCAD `rs_linetypepattern.cpp`. `LinetypeTable::with_defaults` now has the full **25**: Continuous + 6 families (Dot/Dash/Dash Dot/Divide/Center/Border) × 4 sizes (normal/tiny/small/large), in the picker's order. Patterns converted from LibreCAD's signed (+dash/−gap, 0.15-0.2 dash = dot) to our all-positive `[dash,gap,…]` convention. `Linetype::new(name,&[f32])` added. IDs renumbered (Continuous stays 0) — old RSM linetype ids remap, fine in active dev; the full table round-trips in RSM (already saved).
- [x] **Multi-segment renderer** `paint_pattern_polyline` (free fn): walks the FULL pattern (not just dash[0]/gap[0] like before), draws sub-pixel dashes as filled DOTS, carries phase across polyline corners, honours per-dobject `linetype_scale`. `paint_dobject_with_style` rewritten to tessellate any of Line/Polyline/Circle/Arc/Ellipse/EllipseArc/Spline (via `preview_world_polylines`) → pattern stroke; Wall/Hatch/Text/Dim/BlockRef keep their solid render (fill/glyphs/recursion). Previously only Line+Wall dashed, single dash+gap.
- [x] **Graphical picker** `graphical_linetype_combo` + `paint_linetype_sample` (normalises ~2.2 cycles to the row width so every type shows its character). Wired into the Layer panel linetype combo (rows = rendered sample + name, like the LibreCAD dropdown); inline current-sample before the combo. Kernel tests 161.

### Stretch reworked to the selection model + PER-during-edit fix (2026-06-12, uncommitted)
- [x] User reported 2 bugs: (1) stretch couldn't Shift-exclude objects; (2) PER snap was unresponsive during stretch. Replaced the earlier custom stretch-window-drag with the **universal selection model**: `st` (empty sel) → `begin_selection(ForSelect)` + `QueuedOp::Stretch` → crossing window selects, **Shift-click / Shift-drag excludes** (add_window_selection now gets the real `shift` when in a session; idle shift-drag still adds via `shift && !was_off`), Enter → capture box → base → dest. Select-FIRST also works (`st` with a selection → box = selection bbox → straight to base, whole-object move). The crossing box is stashed in `stretch_window_box` during the window-drag (last window wins). `apply_stretch` now moves ONLY the SELECTED dobjects' in-box vertices (was: all dobjects) via the shared `stretch_one`. StretchState simplified to Off/WaitingForBase/WaitingForDest (removed WaitingForWin1/2 + the custom classifier `stretch_window_phase`). Ghost preview now iterates the selection.
- [x] **PER/TAN unresponsive during edit ops** (bug 2, general): TWO gates rejected it. (a) `find_all_snaps` was fed only `pending.last()` as the "from" anchor → empty during edit picks. Now fed `card_anchor()`. (b) FOLLOW-UP (user "still per not active"): the `Command::SnapOverride` handler ALSO refused to ARM PER/TAN when `pending.is_empty() || tool==None` — always true during stretch — so snap_override was never set. Changed that guard to `card_anchor().is_none()`. Both gates now use card_anchor → PER/TAN work during stretch dest + move/copy/mirror/rotate.
- [x] **Double-fire on press-fires-click op completion**: when a press-fires-click op ENDS on the press (stretch/move/copy dest), the phase leaves click-only and the matching RELEASE was re-read as a pointer click → spurious select. New `pending_release_swallow` swallows that release.

### CARD for rotate + Direct Distance Entry (2026-06-12, uncommitted)
- [x] User: "CARD should be considered for rotate, mirror, move" + "once CARD is open, line/copy/move: type a value = second point along the CARD direction". Move/copy/mirror/stretch were ALREADY in `card_anchor`; **added rotate** (WaitingForAngle/RefSrc1/RefSrc2/RefTgt → anchor = pivot) so the angle pick snaps to cardinal dirs (commit already went through `apply_constraints`; also fixed the rotate **preview** to use `cursor_world_constrained` so ghost+baseline match the snapped commit).
- [x] **Direct Distance Entry (DDE)**: new `last_cursor_raw_world` field stashed every canvas frame (`resp.hover_pos()→s2w`, kept when cursor leaves canvas). New intercept in `run_command` (before the main parser): when Tool::Line has 1 pending pt, OR Move/Copy is WaitingForDest, a single typed number = distance from the anchor along the direction to the CONSTRAINED cursor (`apply_constraints(raw) − anchor`, normalized). CARD on → that direction is the locked H/V axis, so "100 ⏎" drops the point exactly 100 units H/V; CARD off → throws toward the cursor (standard AutoCAD DDE). Line → push+`try_finalise`; Move → `apply_move(p−base)`; Copy → `apply_copy(p−base)`. Degenerate direction (cursor on anchor) / no hover → hint, no-op. Single-number only (so it never collides with rectangle "W H" or coordinate entry). Rotate keeps its own typed-ANGLE entry (unchanged).

### DXF/RSM open+save wired to a UI file browser (2026-06-12, uncommitted)
- [x] User: "opening and saving DXF, wire in the UI". The File menu previously ran hardcoded `/tmp/in.dxf` / `/tmp/out.dxf` paths. Built an **in-app file browser** (`FileDialog { mode, dir, filename, ext, error }` + `FileDialogMode::{Open,Save}`) — PURE `std::fs` (no `rfd`/native-dialog dep, per the permissive/pure-Rust/no-Qt policy). Lists sub-folders + `.dxf`/`.rsm` files, `📁 ..` up-nav, click folder = cd, click file = select (double-click = confirm), editable Path bar (paste+Enter to jump), filename field, and a DXF/RSM **format toggle** (Save). Confirm routes to the existing `do_open`/`do_save`. Remembers last dir in `file_dialog_dir`. Esc/Cancel/X close it. File menu now: Open…, Save As .dxf…, Save As .rsm… → `open_file_dialog(mode, ext)`. Backend `cad_io::dxf` already covers all geom variants (Hatch/Spline skipped on write; BlockRef exploded — documented interop debt). `render_file_dialog` called in `update` next to the other dialogs.

### BUG FIX: offset didn't work on polylines (incl. rectangles) (2026-06-12, uncommitted)
- [x] User dump: pick dobject #12 → click side → nothing (state looped back to WaitingForObject, snapshot popped). Root cause: `Geom::offset` Polyline arm returned `Err("offset on polyline not implemented yet (corner math TBD)")` — and the NEW `rec` command makes a CLOSED polyline, so offsetting a rectangle silently failed (error went to history only). Implemented `offset_polyline(p, dist, side)` in kernel geom.rs: global hand from the nearest-segment chord vs the click; straight segments shift along their normal; **arc (bulge) segments → concentric** (radius `r − hand·sign(sweep)·dist`, same swept angle → same bulge); adjacent straight offsets joined at the true **line-line miter intersection** (`line_line_inf`), arc-touching joints fall back to the midpoint of the two offset ends (exact for tangent). Open + closed both handled; no self-intersection trimming (matches plain OFFSET). 4 tests: closed-rect inward (→(1,1)-(9,5)), outward (→(-1,-1)-(11,7)), open L-miter (→(9,1)), arc-segment concentric (r1→r2, bulge preserved). Kernel tests 160.

### Stretch reworked: `st` alias + crossing DRAG + ghost (2026-06-12, uncommitted)
- [x] User dump: `st` → ParseErr; dragging during stretch produced no crossing window ("drag-window not available in this phase"); a leftover `Rectangle` tool was active (its click-only semantics forced every press to a click). Fixes: (1) parser `st` alias (`stretch | st | s`). (2) `Command::Stretch` now clears `self.tool` + `pending` so a draw tool can't hijack the gesture. (3) **Crossing window via DRAG**: new `stretch_window_phase` (WaitingForWin1/Win2) opts OUT of press-fires-click and INTO `drag_intent_is_window` (time-gated by SelDmTm) while STAYING in `in_click_only_phase` (so pointer-mode grip/selection handlers don't also fire). New stretch drag handler captures press→release (or clicked-corner→release) as the box → WaitingForBase; two-click fallback still works. Rubber-band preview now shows during the stretch drag. (4) **Ghost preview**: new free fn `stretch_one(g,wmin,wmax,v)` (shares the per-vertex rule with `apply_stretch`); WaitingForBase/Dest draws the captured green box, and WaitingForDest draws base→cursor dashed vector + a live ghost of the stretched result (bbox-filtered). `apply_stretch` (vertices-in-box move by dest−base) unchanged.

### BUG FIX: RSM dropped wall poché fill (+ all style tables) on reopen — RSM v3 (2026-06-12, uncommitted)
- [x] User: saved RSM with solid-hatch walls; reopened without hatch; "whole memory map should be saved including all styles". TWO causes, both fixed: (1) the style TABLES `text_styles`/`dim_styles`/`wall_styles` were NEVER serialized — reader reset them to Default, so a wall's `style` id pointed into a table with no fill. (2) the **Wall geom payload itself dropped `style` AND `bulge`** (write_geom tag 9 wrote only start/end/thickness) — so even with the table back, the wall pointed at style 0, and curved walls reopened straight.
- [x] **RSM v2→v3**: `write_rsm` now appends text/dim/wall style tables after blocks; Wall geom tag 9 now writes `style`(u32)+`bulge`(f64); block table writes the `smart` flag. Reader is version-gated: v3 reads the new sections, **v2 files still load** (Wall style/bulge default 0/0.0, no style tables, smart=false) via `ver` threaded through `read_rsm`→`read_dobjects`→`read_geom` and `read_block_table`. DimStyle (~75 fields) uses ONE `dim_style_fields!` macro driving both writer + reader → impossible to desync; added `PartialEq` to DimStyle + TextStyle. Text/Dimension already saved their style ids (only Wall was missing). New test `style_tables_round_trip` (wall fill + curved-wall bulge + dim style whole-struct eq + text style + smart block). cad_io tests 17.
- [x] This RETIRES the long-standing "dim_styles/wall_styles not round-tripped (data loss)" backlog item — RSM is now a full document snapshot.

### CARD mode (2026-06-12, uncommitted)
- [x] **CARD** = cardinal-directions drafting lock (ONLY horizontal or vertical from the anchor). USER RULE (in memory): the term ORTHO must not appear anywhere — badge/F8/history all say CARD; command is `card` / `card on|off` (NO `ortho` alias); SYSVAR renamed `OrtEnb`→`CrdEnb` + `card_anchor()` fn; loader still ACCEPTS legacy `OrtEnb` key from old user_env.txt (sole permitted occurrence). Scope note: dim's `LinearOrtho` (H/V/Aligned dim geometry) is a different concept, name kept. Constraint math unchanged; priority osnap > CARD > grid.

### Blocks slice 1 — SHIPPED 2026-06-12 (uncommitted)
The plan's "biggest architectural step". v1 model: **uniform scale + rotation** (similarity transform — circles stay circles; non-uniform scale deferred), `p_world = insert + R(rot)·s·(p_def − base)`.
- **Kernel** `block.rs`: `Block { name, base, dobjects }` (definition space), `BlockTable` on Document, `BlockRef { block, insert, scale, rotation }` + `transform_geom` (2 tests). `Geom::BlockRef` variant + ALL arms: transforms exact (mirror reflects insert+rotation but NOT handedness — documented v1 limit; explode first); trim/split/offset → Err "explode first"; bbox/distance = placeholders + `is_view_independent_bbox` (Hatch pattern); `GripRole::BlockInsert` (drag = move); snap = insertion point only (snap-through deferred); intersect = empty.
- **Commands**: `block <name>`/`b` (universal selection model → click base → definition stored + originals replaced by ONE instance at base, AutoCAD behavior); `insert <name>`/`i` (click point; lists available names when wrong/missing); `explode`/`xp` (select-first; one level; ByBlock contents take instance color; fresh handles via `with_style`). Esc cancels; Draw menu: disabled "Block…" hint + "Insert Block ▸" name submenu.
- **Render** `draw_blockref`: transformed contents, per-content resolved color, **ByBlock → instance color** (ByBlock finally means something), nested recursion (depth cap 8; cycles impossible v1 — no redefinition), dangling ref = red marker. Dashed-selection overlay recurses contents.
- **Pick/select**: `nearest_entity_under` blockref fallback (transformed-content distance; nested = insert pt, one level); `add_window_selection` resolves real bbox via `resolved_blockref_bbox`.
- **RSM v2**: VERSION 1→2, reader now accepts OLDER versions (was strict-equal — old files load); blocks table after dobjects (reuses dobject reader/writer recursively); geom tag 12 = BlockRef. `blocks_round_trip` test incl. nested instance.
- **DXF v1 POLICY (interop debt)**: instances written EXPLODED (transformed contents, ByBlock resolved); BLOCK/INSERT parity = roadmap DXF pass. hatch_trace skips instances (explode to hatch).
- **Block dialog** (user feedback: bare `block` printed usage and felt broken): bare `block` / Draw → Block… now opens a dialog — Name field (Enter = OK; spaces allowed, only TYPED `insert <name>` needs single words), what-happens-next hint (selection count aware), and an **Existing blocks list with one-click Insert** (sets insert_state directly, bypassing the parser). Shared `start_block_def(name)` used by dialog OK + typed `block <name>`; dialog stays open on name errors (empty/duplicate); Esc closes.
- Deferred: snap-through, insert ghost preview + scale/rot sub-options, block redefinition, mirrored flag, Library Browser. NOTE: kernel tag-11 dim test count now 156.

### Curved-wall render defects — S1–S3 FIXED 2026-06-12
- [x] **S1** `Wall::face_polylines(n)` in kernel geom.rs: exact concentric-arc face samples via TRUE radial normals (`±radial·t/2`, side = −sign(sweep)); endpoints coincide with a tangent straight wall's face ends to 1e-9 (regression test `curved_wall_faces_meet_tangent_straight_wall_exactly` + concentric-radius asserts). Zoom-gap eliminated.
- [x] **S2** Poché fill = egui::Mesh triangle strip between the two face polylines (shared verts, no AA seams) — ONE path fills both the straight quad and the concave curved band. Curved corners now fill.
- [x] **S3** New shared `wall_face_screen_pts(app, rect, w)` (zoom-adaptive sample count from on-screen outer-arc length, clamp 12..256) consumed by ALL THREE paths: solid `draw_dobject_thick`, dashed-selection `draw_dobject_dashed`, linetype `paint_dobject_with_style` (+ both WlCnL centerline overlays now arc-aware, same sample count as faces). Bonus: selection overlay now shows MITRED straight faces (matched solid render; previously raw chords).
- [ ] **S4 still owed** (own slice): curved-wall pick/snap all chord-based (distance_to_point, END/MID/NEA/PER/TAN); Wall bbox chord-based → window-select can MISS curved walls; mid-grip floats off-body; trim_at/break FLATTEN bulge→0.
- Hygiene noted, not done: solve_faces clones ALL walls per wall per frame (O(N²)); GPU-mode fallback ignores linetypes.

Original audit (2026-06-11):
User report: filleted solid wall → curved corner not filled + outer face line not reaching the arc when zoomed. 5-agent audit confirmed BOTH causes + full defect list:
- **Cause 1 (no fill):** poché deliberately skipped for curved walls — `fill_aci != 0 && !w.is_curved()` (app.rs ~17403); curved band is concave, code only fills a convex quad.
- **Cause 2 (gap at joint):** curved faces offset tessellated centerline samples along FINITE-DIFFERENCE chord normals (app.rs ~17365-79). Endpoint chord tilts from true tangent by sweep/(2·steps)=1.6° (90°, 28 steps) → face endpoint lands 0.0042 world units off (verified = (t/2)·sweep/(2·steps) for t=0.3). Gap on one face, equal overlap on the other; constant world-size, fixed 28 steps → grows linearly in px with zoom. Tangent points themselves coincide to 1e-15 (fillet_lines exact) — no other error source (adversarially checked).
- Fix plan: **S1** kernel helper `Wall::face_polylines(n)` returning EXACT concentric-arc samples (radial normals `±(p-center)/r`, endpoints exactly start/end±radial·t/2) + zoom-adaptive n (mirror Arc renderer). **S2** curved poché = egui::Mesh triangle strip between the two face polylines (shared verts, no AA seams). **S3** the other two render paths actively draw WRONG straight chords for curved walls — make `draw_dobject_dashed` Wall arm + `paint_dobject_with_style` linetype Wall arm (+ its straight WlCnL centerline) use the shared helper.
- **S4 (separate slice — curved-wall correctness):** pick/snap all chord-based (distance_to_point, END/MID/NEA/PER/TAN); **Wall bbox chord-based → window-select can MISS curved walls** (same class as the fixed arc-bbox bug); mid-grip floats off-body; **trim_at/break FLATTEN bulge→0**.
- Hygiene noted, not in this fix: solve_faces clones ALL walls per wall per frame (O(N²)) — cache per frame; GPU-mode fallback uses draw_dobject (linetypes ignored in GPU mode).

### Styles consolidation
- [x] **Styles menu** — DONE 2026-06-11. New top-level menu (between Wall and Tools), the ONE home for style tables, designed to map 1:1 onto a future ribbon Styles tab: Managers (Text Style… / Dimension Style… / Wall Style…) + Current quick-switchers (✔-marked submenu radio for current dim + wall style; wall switch syncs WlThk like the Manager's Set Current) + disabled "Opening Style… (planned)" placeholder. Old entries in Dimension/Wall menus kept for muscle memory.
- [ ] **Opening styles (door/window/NICHE)** — PLANNED, spec in `Smart_Dobjects.md` §2.6: Opening rides a host wall (parametric `t` along centerline), OpeningStyleTable mirrors WallStyle, niche = depth<thickness opens only the near face. Gated on T-junction's face-splitting primitive; prefers own-dobject storage → wants handle-first Document API. Do NOT build until asked.

### Universal selection model conformance
- [x] **Align select-first flow** — DONE 2026-06-11. `align` with an empty basket now opens a ForSelect session + `QueuedOp::Align` (was: error "select first"), exactly like Move/Copy/Rotate; Enter → `AlignState::WaitingForSrc1`. Pre-selected basket still skips straight to point capture. All 4 point phases now keep the cmd prompt current (`set_prompt` per phase, cleared on apply). Per [[feedback_rust_cad_universal_selection_model]] — every modify command enters select-phase; Stretch remains the intentional outlier. AUDIT NOTE: matchprop is the remaining non-conformer (targets-first; rework to source-first is in-flight, see plan item #3).

### High priority (user-visible polish)
- [x] **Dimension Style Manager** — DONE 2026-06-10. `dimstyle`/`ddim` now opens a full AutoCAD-style manager (`render_dim_style_manager` in `cad_app/src/app.rs`), not the bare form: Styles list (✔ marks current), live **preview**, and Set Current / New… / Modify… / Override… / Compare… buttons + List combo / Description / Close / Help. Preview = `draw_dim_style_preview()`, OUR OWN sample (rounded-corner plate + bolt hole) annotated with H+V linear, diameter, radius — driven by the selected style's arrow_size/text_height/decimal_places/separator/color (NOT AutoCAD's L-bracket). New…/Modify… launch the existing `DimStyleDialog` add/edit sub-form. Added `current_dim_style: u32` (+ `dim_style_manager_open/_sel`) to CadApp; **new dims + the ghost preview now use `self.current_dim_style`** instead of hardcoded STANDARD (commit site + ghost site updated). Set Current is wired; double-click a style = set current. **Override… / Compare… are honest stubs** (history note only) — wire later. Manager window is NOT yet in the recorder `WindowFlags` (only the sub-form's `dim_style_dialog` is).
- [x] **Dim toolbar button + Dimension menu** — DONE 2026-06-10. Added `tool_button(Tool::Dim, "dim")` to the top toolbar row (icon arm already existed). The button only flips `self.tool`, so entry is routed through `run_command("dim")` (sets draft state + prompt) when the tool transitions to Dim, and `dim_draft` is reset to `Off` when leaving Dim. New dedicated **"Dimension" menu** between Modify and Tools: "Dimension (smart: linear · radius · diameter)" → `dim`, separator, "Dimension Style…" → `dimstyle`. Note: there's no ribbon-tab system — the "toolbar" is a flat two-row strip + a classic menu bar (File/Edit/View/Draw/Modify/**Dimension**/Tools/Help).
- [x] **Dim render fixes** — DONE 2026-06-10 (from a screenshot bug report). Two problems: (1) selected dims drew a placeholder dashed *triangle* through the 3 grips; (2) arrowheads + measurement text were invisible because they're sized in world units (0.18) and went sub-pixel against larger geometry (text was also silently skipped below 4 px). Fix: extracted `dim_render_geometry()` in `cad_app/src/app.rs` as the SINGLE source of truth for a dim's structural lines + arrowheads + text anchor; both `draw_dimension` (solid) and `draw_dobject_dashed` (dashed selection) now consume it, so they can't drift. Annotations clamp to a screen-space floor (`DIM_MIN_ARROW_PX` = 8, `DIM_MIN_TEXT_PX` = 11) — world sizing still applies above the floor. Linear text now lifts clear of the dim line by gap + ½ text height. Ghost preview inherits the visible-arrow/text behavior for free.
- [x] **DimStyle dialog** — DONE 2026-06-10. `dimstyle`/`ddim` (optional name) opens a New/Edit form (`DimStyleDialog` in `cad_app/src/app.rs`, parallel to `TextStyleDialog`): name + arrow_size + text_height + decimal_places + single ACI color (ByBlock default, swatch preview). OK validates name (non-empty + dup-check, edit allows same id), clones the source style (STANDARD for new, edited style for edit) and patches only the exposed fields so the other ~65 DIMVARs survive, then appends/replaces in `doc.dim_styles`. Renderer (`draw_dimension`) now honors the style color: a non-ByBlock `color_dim_line` overrides the dobject's resolved color (one ACI written to all three element colors). Recorder `WindowFlags` gained `dim_style_dialog`. **Still NOT round-tripped through RSM** (see Medium priority — dim_styles table still uses Default on read).
- [ ] **Auto-ortho detection for linear dims** — currently always `Aligned`. Should infer Horizontal/Vertical from the dimline_pos drag direction (perpendicular offset → H if |dy| > |dx| of offset, else V). User can force via keys H/V/A.
- [ ] **Text horizontal alignment for radius/diameter labels** — currently left/right based on x-comparison; better to use the leader direction angle.
- [ ] **Live preview during WaitingForP1** — show a crosshair / hover marker so user knows the click will start a dim.

### Dimension render features (user-requested from a sample, 2026-06-10)
All DONE 2026-06-10. Driven off `DimStyle` fields; exposed in the Modify form (now sectioned Lines & Arrows / Text / Units) + reflected in the manager preview.
- [x] **Per-element colors selectable** — ext-line / dim-line / text each get their own ACI via the shared wheel (`DimColorSlot` on `AciPickRequest::DimStyleForm`). Renderer uses `color_ext_line` / `color_dim_line` / `color_text`, each falling back to the dobject color when 0 (ByBlock).
- [x] **Text location** — `text_vert_pos` (DIMTAD): Centered (on line) / Above / Below, chosen in the form + shown in preview.
- [x] **Text aligned vs horizontal** — `text_inside_horiz`/`text_outside_horiz` toggled by the form's "Align with dimension line"; renderer rotates the text via `egui::epaint::TextShape.angle` (readability-corrected, never upside-down).
- [x] **Dim line trimmed for centered text** — when text sits ON the line (DIMTAD 0), `draw_dimension` breaks the dim line into two segments leaving a gap sized to the galley width + text_gap.
- [x] **Filled vs hollow arrows** — new kernel field `DimStyle.arrow_filled` (default true). Hollow draws the triangle as outline only.
- [x] **Architectural tick** — driven by existing `tick_size` (>0). Form's "Arrow type" combo = Filled / Hollow / Architectural tick (tick defaults its size to arrow_size).

Renderer refactor: `dim_render_geometry` now returns a role-tagged `DimGeo` struct (ext_lines / dim_line / leaders / arrows / text_pos / text_angle / text_on_dim_line); `DimGeo::all_lines()` feeds the dashed overlay. STANDARD default DIMTAD is 0 → text centered with the line broken (matches the screenshot fix).

### Dimension Style Manager — follow-ups (user-requested 2026-06-10)
- [x] **Dim color uses the shared ACI wheel** — DONE 2026-06-10. The Dim Style add/edit form's Color row no longer uses a raw `DragValue`; it now shows the layer-panel swatch affordance (click chip / "Pick ACI…") that opens the shared polar `render_aci_picker_window`. New `AciPickRequest::DimStyleForm` variant routes the chosen ACI back into `dim_style_dialog.color_aci`. Picker render runs AFTER the form each frame, so the dialog is restored to `Some` when the pick lands. Consistent with [[feedback_rust_cad_color_aci_primary]] + [[reference_rust_cad_aci_picker_ui]] — wheel everywhere a color is chosen.
- [x] **Manager was non-floating + huge vertical gap** — FIXED same day. Cause: `.anchor(CENTER_CENTER)` re-pinned it every frame (couldn't drag) and the Styles `ScrollArea` had `auto_shrink([false,false])` with NO `max_height`, so it filled the whole window and pushed the bottom row down. Fix: dropped the anchor (→ `.movable(true)` + `.default_pos`), set list `max_height(258)`. Resizable set false for now.
- [ ] **Compare… — units-aware** (user wants this): Compare two styles AND render the preview using the units the user is choosing (DimStyle `linear_unit_format` / `decimal_places` / `decimal_separator` / `linear_scale`). Implies the Modify form should expose a Units page first. Currently Compare… is a stub.
- [ ] **Override…** still a stub.
- [ ] egui_dock NOT needed for dialogs — but it's a candidate for unifying the side panels (layers/pens/info/snap) into a dockable workspace later. Policy-OK (MIT/Apache, pure Rust, no Qt). Separate decision from this dialog.
- [ ] Manager window not in recorder `WindowFlags` yet.

### Medium priority (correctness)
- [ ] **Hover-over hint when WaitingForP1 hovers a Circle/Arc** — tooltip "click for Radius (D for Diameter)" so the auto-decide isn't a surprise.
- [ ] **dim_styles RSM round-trip** — currently dropped; reader synthesizes Default. Add tag(s) for the style table.
- [ ] **Dim line text positioning per `text_vert_pos` (DIMTAD)** — currently always above the dim line; should honor 0 (centered) / 1 (above) / 4 (below).
- [ ] **Tolerance display** — DIMTOL fields exist on DimStyle but renderer ignores them.

### Lower priority (next slices per the original plan)
- [ ] **Angular dimensions** (3-click: 2 lines + arc position)
- [ ] **Arc length dimensions**
- [ ] **Ordinate dimensions**
- [ ] **Leader** (a Dim variant or a separate Geom?)
- [ ] **Center mark** command (DIMCEN-driven)
- [ ] **DXF DIMENSION group-code parity** — round-trip without exploding to TEXT
- [ ] **Multiple text styles per Document with LFF / SHX font loading** (DimStyle.text_style_name currently honored only by name; falls back to STANDARD)

### Memos to keep in mind
- [Walk whole pipeline before fixing](.claude/projects/-home-HSI-workspace-qlcplus-master/memory/feedback_walk_whole_pipeline_before_fixing.md) — burned 2 turns × 3 bugs in 2026-06-09 by stopping at first plausible cause. **Always enumerate every step that could produce the symptom before claiming "fix is in".**
- [Always link created files](.claude/projects/-home-HSI-workspace-qlcplus-master/memory/feedback_always_link_created_files.md) — every file mentioned in chat gets a clickable markdown link.
- [RUST_CAD run command in summaries](.claude/projects/-home-HSI-workspace-qlcplus-master/memory/feedback_rust_cad_run_command.md) — append `~/workspace/RUST_CAD/target/release/rust_cad` to every build/commit/push summary.
- [RUST_CAD mentor/inspector role](.claude/projects/-home-HSI-workspace-qlcplus-master/memory/feedback_rust_cad_mentor_inspector_role.md) — DEFAULT = coding agent. Mentor mode only on explicit opt-in.

---

## 4. Verification (current state)

```bash
cd ~/workspace/RUST_CAD
cargo build --release
# → Finished `release` profile [optimized] target(s) in ~30s
# → 21 warnings, 0 errors

cargo test --release -p cad_kernel dim::
# → 11 passed; 0 failed; 0 ignored

cargo test --release -p cad_app hatch_trace
# → 12 passed; 0 failed; 0 ignored  (includes 3 new regressions from this session)

~/workspace/RUST_CAD/target/release/rust_cad
# → launches; type `dim`, click two points, click dim line position → committed Linear dim with extension lines + arrows + length text
```

---

## 5. Pickup checklist for next session

When you open a new chat:

1. **Read this file first** — establishes the world.
2. Check `git status` in `~/workspace/RUST_CAD` — current diff is everything from this session if not committed yet.
3. Decide first task. Recommended order:
   - DimStyle dialog (parallel to TextStyleDialog, smaller UI)
   - Then auto-ortho detection (1-line math in `handle_dim_click`)
   - Then label-alignment polish
4. Run the binary and exercise `dim` before/after each change.
5. After each task: `cargo build --release` + relevant tests.

---

## 6. File index

| File | Role |
|---|---|
| [cad_kernel/src/dim.rs](cad_kernel/src/dim.rs) | NEW. Dim, DimKind, DimStyle, DimStyleTable, 11 tests |
| [cad_kernel/src/geom.rs](cad_kernel/src/geom.rs) | Geom::Dimension variant + 3 grip roles + all match arms |
| [cad_kernel/src/document.rs](cad_kernel/src/document.rs) | `dim_styles` field |
| [cad_kernel/src/lib.rs](cad_kernel/src/lib.rs) | re-exports |
| [cad_kernel/src/parser.rs](cad_kernel/src/parser.rs) | Command::Dim / DimStyle + keywords |
| [cad_kernel/src/snap.rs](cad_kernel/src/snap.rs) | Dim arms for End/Mid/Cen/Per/Tan |
| [cad_kernel/src/intersect.rs](cad_kernel/src/intersect.rs) | Dim returns empty |
| [cad_app/src/app.rs](cad_app/src/app.rs) | Tool::Dim, DimDraftState, handle_dim_click, draw_dimension, ghost preview, Esc reset, toolbar icon |
| [cad_app/src/hatch_trace.rs](cad_app/src/hatch_trace.rs) | prune_dangles + augment_islands_from_closed_dobjects + 3 new tests |
| [cad_io/src/rsm.rs](cad_io/src/rsm.rs) | RSM tag 11 |
| [cad_io/src/dxf.rs](cad_io/src/dxf.rs) | DXF exploded-text writer |
| [cad_cli/src/main.rs](cad_cli/src/main.rs) | accepts new commands |

---

`~/workspace/RUST_CAD/target/release/rust_cad`
