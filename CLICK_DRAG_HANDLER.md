# RUST_CAD — Click & Drag Handler (implementation guide)

> Handoff doc for another coding agent. This is the canvas pointer pipeline that
> turns raw egui press/release/drag events into one of: **a click** (point pick
> or select-toggle), **a window/crossing rubber-band**, or **a grip drag** —
> correctly across ~25 modal phases. It is the most subtlety-dense subsystem in
> the app and it works well, so copy the *rules*, not just the code.
>
> Code: `cad_app/src/app.rs`, the `CentralPanel` closure at ~20844 and the
> classifier at ~21140–21700. Selection mutators: `click_select` @6427,
> `add_window_selection` @6533. Line numbers from commit `36ee804`; grep if they
> drift.

---

## 1. Philosophy (read this first)

**egui's own click/drag classification does not get a vote.** egui will call a
2-pixel wobble a "drag" and clear `press_origin()` by the time `drag_stopped()`
fires. Both behaviors silently corrupt CAD picking. So the app:

1. **Stashes its own press position and press time** (`self.press_pos`,
   `self.press_time`) on the press frame, and reads them back on the release
   frame. It never trusts `egui::Pointer::press_origin()` for the decision.
2. **Decides click-vs-drag from app state + intent**, not from a motion
   threshold: *what phase are we in?* + *is Shift held?* + *was the button held
   past the time gate?* + *was there any real motion at all?*
3. **One gesture = one outcome per frame.** A press-release produces exactly one
   of {click, window, grip-commit, nothing}. Never two.

The result is AutoCAD-like muscle memory: pressing *at* a point captures *that*
point (pickbox feel); a deliberate held drag rubber-bands; a fast wobble is
still a click.

---

## 2. Vocabulary — the three phase predicates

Everything keys off **which of three mutually-exclusive phases** the app is in
when the gesture happens:

| Predicate | Meaning | Gesture semantics |
|---|---|---|
| `in_click_only_phase` | a drawing tool or any point-pick edit phase is active (huge OR-list, see §5) | **press = click**, no drag meaning at all |
| `in_select` (`select_mode != Off`) | a command asked for a selection (erase/move/trim cutters/…) | held drag = rubber-band window; click = toggle into basket |
| `pointer_mode_idle` (`!in_click_only_phase && !in_select`) | the bare canvas — `Tool::None`, no edit, no session | the always-on selector: held drag = window; click = replace-select; grips grabbable |

These are computed fresh every frame (`in_select` @21270, `in_click_only_phase`
@21306, `pointer_mode_idle` @21351). They are the backbone — get them right and
the rest follows.

---

## 3. State the handler owns (and why)

```rust
press_pos:   Option<(Pos2, Vec2)>,   // OUR stash of the press point (screen+world)
press_time:  Option<f64>,            // ctx time at press — for the hold gate
pending_release_swallow: bool,       // press fired a click that ended its op → eat the release
window_first: Option<Vec2>,          // first corner of a 2-click window (typed/explicit path)
grip_drag:    Option<GripDrag>,      // a grip is currently grabbed
last_point:   Option<Vec2>,          // last committed point (AutoCAD "@"/continue-from-last)
// recorder-only mirrors so the timeline can decode the gesture:
dbg_press_pos, dbg_press_hit, dbg_press_sel, dbg_pending_gesture
```

**Why `press_pos`/`press_time` exist:** egui clears `press_origin()` on the
release frame (exactly when `drag_stopped()` fires), so reading it there yields
`dist == 0` and every drag silently demotes to a click. The app stashes its own
copy at press time and reads it at release. This is the single most important
mechanical detail. (Comments at @21257 and @21286.)

---

## 4. Per-frame pipeline (order inside the CentralPanel)

```
let (resp, painter) = ui.allocate_painter(avail, Sense::click_and_drag());   // 20847
… pan/real-time-zoom drag handled first (resp.drag_delta) …
── recorder taps: CanvasPress / CanvasRelease / CanvasDrag (stash dbg_press_*)  // 21146
── compute press_release_dist from self.press_pos (NOT press_origin)            // 21262
── compute hold gate: press_held_secs >= SelDmTm  (BEFORE clearing press_time)  // 21282
── update press_time/press_pos for next frame (press sets, release clears)      // 21295
── compute in_click_only_phase / pointer_mode_idle                              // 21306
── classify: drag_intent_is_window, drag_was_a_click                            // 21368
── press-fires-click override (drafting/point-pick capture on PRESS)            // 21386
── grip-drag handler (grab on press/click, commit on release/click)            // 21404
── window-drag handler (apply window/crossing on release)                       // 21494
── release-swallow bookkeeping (prevent double-fire)                            // 21541
── click_fired = (click_now || drag_was_a_click) && !consumed && !swallow      // 21545
── if click_fired: the CLICK DISPATCH CASCADE (zoom→flow→hatch→…→select/tool)   // 21587
── emit GestureClassification (the recorder's full decode)                      // end of block
```

**Ordering is load-bearing.** The two classic bugs both come from reordering:
computing the hold gate *after* clearing `press_time` (gate always reads 0 →
windows lost), and reading the press point *after* the release handler clears it.

---

## 5. The classifier (the core rule)

```rust
// press_release_dist from OUR stash; hold gate from SelDmTm (default 250ms)
let drag_intent_is_window =
    ((in_select        && hold_threshold_passed)     // (1) select session
     || (pointer_mode_idle && hold_threshold_passed) // (2) bare canvas
     || (shift_held    && !in_click_only_phase))     // (3) Shift forces a window (no time gate)
    && press_release_dist > 1.0;                      // (4) any real motion at all
let drag_was_a_click = drag_stopped && !drag_intent_is_window;
```

In English — **a drag becomes a window only when**:
1. we're in a select session **and** the button was held past `SelDmTm`, OR
2. we're on the bare canvas (pointer-mode-idle) **and** held past `SelDmTm`, OR
3. the user held **Shift** (explicit window; exempt from the time gate),
and in all cases there was at least ~1px of motion.

**Otherwise every press-release is a click.** There is no 5px "tiny drag"
heuristic — egui's `drag_stopped()` that isn't a window gets *promoted back to a
click* (`drag_was_a_click`).

### Press-fires-click override (the pickbox feel)
For drawing tools and point-pick phases (`in_click_only_phase`), the click is
registered on **press**, not release — so pressing AT a point captures THAT
point even if the cursor drifts a few px before release (@21386):
```rust
let press_fires_click = in_click_only_phase;
let press_now = press_fires_click && !rt_zoom
    && primary_pressed() && resp.contains_pointer();
let click_now = if press_fires_click { press_now } else { click_now };
let drag_was_a_click = drag_was_a_click && !press_fires_click;  // don't also fire on release
```

### The time gate (`SelDmTm`)
`hold_threshold_passed = press_held_secs >= SelDmTm/1000`. A fast accidental
drag during a click stays a click; only a deliberate hold-then-drag opens a
window. Shift-drag bypasses the gate (the user already declared intent).
**Compute the gate before clearing `press_time`.**

---

## 6. `in_click_only_phase` — the full list

A drag has *no* meaning (so press = click) whenever ANY of these is active
(@21306): a drawing `tool != None`; trim/extend target-pick; move/copy/paste/
rotate/scale/mirror/align/break/lengthen/offset/stretch/matchprops/fillet/
chamfer/dist states; block base-pick / insert point / blockdiff pick /
match-props source pick; `cmd_flow` (the prompt-driven CIRCLE etc.); zoom
point-steps (`zoom_state.wants_point()`); intersect-pending click.

> When you add a new multi-step command with a point-pick step, **add its state
> to this OR-list** or grips on still-selected dobjects will steal the press and
> your point is never captured. (This is called out in the comments at @21327.)

---

## 7. Grip drag (pointer-mode only, `GrpEnb` on)

Two grab paths, both set `self.grip_drag` (@21404):
- **(a) drag-grab:** `resp.drag_started_by(Primary)` begins near a grip → commit
  on `drag_stopped`.
- **(b) click-grab:** a click lands near a grip while none is held → cursor moves
  → a second click anywhere places it.

Grab radius = `GrpHvR` px → world (matches the hover highlight, so anything that
looks lit grabs). Commit honors running osnap/CARD/grid for the drop point, then
`Geom::with_grip_moved(role, drop_world)` lets the kernel decide what the grip
does (circle quadrant → radius; line midpoint → translate). A grip grab sets
`grip_drag_consumed_click = true` so the same gesture isn't also read as a
select-toggle. The `pending_release_swallow` guard prevents a point-pick's
release tail from spuriously grabbing a grip (@21419).

---

## 8. Window / crossing drag

On release, if `drag_intent_is_window` and no grip consumed it (@21494):
read the press corner from the **stashed** `press_pos_this_frame`, take the
release corner, and call `add_window_selection(p1, p2, shift, alt, fresh)`.

**Window vs crossing decision** (`add_window_selection` @6548) — *hard rule,
do not reorder*:
```rust
let crossing = match self.armed_window_inside.take() {
    Some(true)  => false,     // typed `w` → inside-only window
    Some(false) => true,      // typed `c` → crossing
    None        => p2.x < p1.x,   // direction default: R→L = crossing, L→R = window
};
```
- A typed `w`/`c` override **always** beats drag direction (and is consumed by
  the first completing window via `.take()`).
- Otherwise AutoCAD-standard: **left→right = window** (fully-inside only),
  **right→left = crossing** (anything the geometry actually enters — bbox-overlap
  alone over-selects, so crossing tests real geometry, see @6583).

**Pointer-mode (`was_off`) applies DIRECTLY to the live selection** with
`fresh = was_off`: a plain idle drag replaces; Shift adds; Alt removes; inside a
session it accumulates. Do **not** `begin_selection()` there — it would clear the
basket and turn Shift/Alt into replace. Window-selecting a group member expands
to the whole group; STRETCH also records the box as its per-vertex test region.

---

## 9. The click dispatch cascade

When `click_fired` (@21587), capture the point with snap priority **osnap > CARD
> grid > raw**:
```rust
let click_world = snap_hit.map(|h| h.point)
    .unwrap_or_else(|| self.apply_constraints(world));   // CARD/grid
if self.tool != Tool::None || self.cmd_flow.is_some() { self.last_point = Some(click_world); }
```
Then a **priority cascade**, each consuming the click and returning:
zoom point-pick → `cmd_flow` point (CIRCLE) → hatch pick-point → blockdiff pick →
(… every other point-pick edit phase …) → finally selection (`click_select`) or
the active drawing tool's vertex push.

`last_point` is what lets Enter/Space at the next command's first prompt
"continue from the last point" (AutoCAD behavior).

---

## 10. Selection mutation helpers

`click_select(i, shift, alt, fresh)` @6427:
- `remove = alt || (select_remove_mode && !shift)` → drop `i` from the basket.
- else: if `!shift && fresh` → **clear first (replace)**; then add `i` if absent.
- So: **plain click = replace, Shift = add, Alt = remove**; inside a command
  session `fresh = false` so clicks **accumulate**.

`add_window_selection(p1, p2, shift, alt, fresh)` @6533: same replace/add/remove
semantics for the box; window-vs-crossing per §8; emits a recorder
`SelectChange` with every candidate's verdict (so "I dragged crossing but got 0
hits" is diagnosable).

---

## 11. Double-fire prevention

`pending_release_swallow` (@21541): if a press fired a click that *ended its own
op* (e.g. stretch/move destination), the op leaves click-only mode, so the
matching release would be re-read as a pointer-mode click and spuriously select.
Set the flag on the press, swallow the next release, clear it. The final
`click_fired` gate ANDs in `!grip_drag_consumed_click && !window_drag_consumed_click
&& !release_swallow`.

---

## 12. Recorder integration

Every stage taps the Session Recorder (see `SESSION_RECORDER.md`):
`CanvasPress` / `CanvasRelease` (with `drag_px`) / `CanvasDrag` (with direction),
`CanvasClick` (world + hit-test + active-state summary), and the big
`GestureClassification` emitted at the end of the block — it records BOTH egui's
view (clicked / drag_stopped) AND the app's outcome
(`click_select(i=2)` / `add_window_selection` / `NOOP`) plus a human verdict.
When a click/drag bug is reported, that one event usually tells the whole story.

---

## 13. Invariants — do NOT regress these

1. **Read the press point from `self.press_pos`, never `press_origin()`** (egui
   clears it by release).
2. **Compute the hold gate before clearing `press_time`** (else windows vanish).
3. **No motion-threshold demotion.** Non-window `drag_stopped` → promoted to
   click. egui doesn't classify for us.
4. **Edit/draft phases are always click** (`in_click_only_phase`) — and they
   fire on PRESS (pickbox).
5. **Typed `w`/`c` beats drag direction** and is consumed once (`.take()`).
6. **Pointer-mode window applies directly** (`fresh = was_off`) — never
   `begin_selection()` there.
7. **One outcome per gesture** — the `*_consumed_click` / `release_swallow`
   guards enforce it.
8. **Add new point-pick phases to `in_click_only_phase`** or grips steal the press.
9. **Shift-drag is exempt from the time gate**; everything else needs the hold.

---

## 14. Port recipe (for another egui/eframe app)

1. `allocate_painter(avail, Sense::click_and_drag())` for the canvas.
2. Add app fields: `press_pos: Option<(Pos2, WorldPt)>`, `press_time: Option<f64>`.
   On `primary_pressed() && contains_pointer()` stash both; on `primary_released()`
   clear both. **Compute any hold gate before clearing.**
3. Define your three phase predicates: `in_click_only_phase` (your draw/point-pick
   states), `in_select` (your selection session), `pointer_mode_idle = !either`.
4. Classifier: `drag_intent_is_window = ((in_select||pointer_idle) && held_past_gate
   || (shift && !click_only)) && dist > 1.0`; `drag_was_a_click = drag_stopped &&
   !drag_intent_is_window`.
5. Press-fires-click for click-only phases; suppress its release twin.
6. Window handler reads the **stashed** press corner; decide window/crossing by
   typed-override-then-direction; apply with replace/add/remove + `fresh`.
7. (Optional) grips: grab on drag-start/click near a grip, commit on
   release/second-click; mark `consumed_click`.
8. `click_fired` gate ANDs out grip/window/swallow; then run your point-pick
   priority cascade with snap-priority for the captured point.
9. (Strongly recommended) wire a gesture-classification debug event like §12 —
   it makes this subsystem debuggable instead of mysterious.

---

## 15. Gotchas

- A "drag selected nothing" report is almost always the hold gate (`SelDmTm` too
  high) or `press_pos` not stashed — check the `GestureClassification` event.
- A "my new tool's point pick selects/grabs instead of placing" is a missing
  entry in `in_click_only_phase`.
- A "Shift+drag replaced instead of added" is a stray `begin_selection()` in the
  pointer-mode window path (must apply directly with `fresh = was_off`).
- Crossing over-selecting long diagonal objects → you're testing bbox-overlap;
  test real geometry entry for the crossing case (@6583).
