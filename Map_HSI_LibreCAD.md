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
**▶ RESUME HERE (2026-06-12): next = curved-wall render fix S1–S3** (user-reported bug, root cause fully audited — see "Curved-wall render defects" section below; user has NOT yet said "go", confirm then build). After that: T-junction (#2), then X-crossing / collinear; S4 curved-wall correctness (pick/snap/bbox/trim) as its own slice. Project plan (mentor-review sequence) also pending: doc re-baseline → RSM style-table round-trip → matchprop source-first rework. Build GREEN; everything committed through "Styles menu + align select-phase" commit.
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

### Curved-wall render defects (audited 2026-06-11, plan approved-pending)
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
