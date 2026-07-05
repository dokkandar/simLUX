# RUST_CAD — Polyline (PLINE) Command (complete explanatory guide)

> Exhaustive handoff doc for the **PLINE** command — drawing model, interactive
> tool, every sub-command (Arc/Line/Close/Undo/Second/Direction/Width/Halfwidth),
> tapered width, arc bulge math, rendering, persistence, and interplay with
> trim/extend/explode/join/pedit. Nothing here is meant to be skimmed.
>
> Code: `cad_kernel/src/{geom,parser,join,trim}.rs`, `cad_io/src/{dxf,rsm}.rs`,
> `cad_app/src/app.rs`. Line numbers from `9a4bcc7`; grep the symbol if they
> drift. Reads with `COMMAND_LINE_CURRENT.md` (how typed sub-commands route),
> `CLICK_DRAG_HANDLER.md` (how clicks are captured), `SETTINGS.md` (PkBxSz).

---

## 0. A worked example (from a real Session Recorder dump)

```
CMD "pline" (tool=Polyline)        ← bare pline → SetTool(Polyline)
CLICK (-80.4, 23.7)                ← vertex 0   (each press fires the click; tool stays Polyline)
CLICK (-25.0, 63.7)                ← vertex 1
CLICK (-45.8,-120.2)               ← vertex 2 …
CMD "a"  parsed→SetTool(Arc)       ← BUT tool stays Polyline: 'a' is the PLINE "arc mode" sub-command,
                                      intercepted before it can become the global Arc tool
CMD "s"  parsed→Stretch            ← 's' = the 3-point-arc "second point" sub-command (not global Stretch)
CMD "l"  parsed→SetTool(Line)      ← 'l' = back to line mode (not the global Line tool)
… more CLICK vertices …
CLICK (-80.4, 23.7)  → PUSH #0 (Polyline) 10 verts (closed)  "auto-closed on first vertex"
```

Two things this teaches:
1. **Single letters typed while a polyline is being drawn are PLINE
   sub-commands, not global commands.** The recorder logs the *parser's* guess
   (`a`→SetTool(Arc)) because it records before the intercepts run — but the
   PLINE sub-command intercept catches `a/l/c/u/s/d/w/h` first, so the tool stays
   `Polyline`. (See `COMMAND_LINE_CURRENT.md` §5.)
2. **Clicking back on the first vertex auto-closes** the polyline (≥3 verts) — no
   need to type `c`.

---

## 1. Data model (`cad_kernel/src/geom.rs`)

```rust
pub struct PolyVertex { pub pos: Vec2, pub bulge: f64 }   // bulge = DXF tan(sweep/4) of the segment LEAVING this vertex
pub struct Polyline {
    pub vertices: Vec<PolyVertex>,
    pub closed:   bool,
    pub widths:   Vec<(f64, f64)>,   // per-SEGMENT (start_width, end_width); EMPTY = thin
}
```
- **Bulge** lives on the vertex but describes the segment **to the next** vertex:
  `0` = straight; `>0` = CCW arc; `<0` = CW arc. `bulge = tan(included_angle/4)`.
- **Widths** parallel the *segments*: `len == verts-1` (open) or `len == verts`
  (closed). Empty vector ⇒ a plain thin polyline (rendered as a stroke). Widths
  are **world units** (scale with zoom).

Arc math (`join.rs`): `bulge_arc(a,b,bulge) -> (center, radius, start_angle,
signed_sweep)` (`r = chord·(1+b²)/(4|b|)`, `sweep = 4·atan(b)`); inverse
`bulge_from_arc(start,end,center,sweep_abs)` (signs by CCW-vs-swept comparison so
**major arcs >π** invert correctly).

---

## 2. Parser (`cad_kernel/src/parser.rs:586`)

`parse_polyline`:
- bare `pl` / `pline` / `polyline` → `Command::SetTool(ToolKind::Polyline)`
  (interactive).
- with args: `polyline x1,y1 x2,y2 [x3,y3 …] [close]` → `Command::Add(Polyline)`;
  a trailing `close`/`closed` token (case-insensitive) sets `closed: true`. All
  bulges 0 (straight); widths empty.

---

## 3. The interactive tool — state

`Tool::Polyline`. The in-progress run lives in transient fields (`app.rs:964+`):

| Field | Purpose |
|---|---|
| `pending: Vec<Vec2>` | vertex positions so far |
| `pending_bulges: Vec<f64>` | bulge for segment i→i+1 |
| `pending_widths: Vec<(f64,f64)>` | per-segment widths, drained on commit |
| `pline_mode: PlineMode` | `Line` or `Arc` — affects the NEXT segment |
| `pline_arc_sub: PlineArcSub` | arc sub-flow (see below) |
| `pline_dir_override: Option<Vec2>` | one-arc start-tangent override |
| `pline_next_width: (f64,f64)` | **sticky** width carried to the next segment (PLINEWID) |
| `pline_width_cap: PlineWidthCap` | width-entry sub-flow |

Enums (`app.rs:48–72`):
```rust
enum PlineMode    { Line, Arc }
enum PlineArcSub  { Normal, AwaitingSecondPt, AwaitingSecondPtEnd(Vec2), AwaitingDirection }
enum PlineWidthCap{ None, AwaitingStart{half:bool}, AwaitingEnd{half:bool, start:f64} }
```

---

## 4. Click flow (vertex commit + auto-close)

Per canvas click while `Tool::Polyline` (`app.rs:~23380`):

1. **Arc sub-flow first** (if `pline_arc_sub != Normal`):
   - `AwaitingSecondPt` → store the on-arc point → `AwaitingSecondPtEnd(mid)`.
   - `AwaitingSecondPtEnd(mid)` → 3-point arc: `bulge_from_three_points(start,mid,end)`,
     push the vertex, back to `Normal`.
   - `AwaitingDirection` → `pline_dir_override = (click-last).normalized()`, back
     to `Normal`.
2. **Auto-close** (if `pending.len() >= 3` AND the click lands within the pickbox
   tolerance — `env.PkBxSz` px → world — of `pending[0]`): commit as **closed**
   via `drain_pline_pending(true)`. *(This is the dump's "auto-closed on first
   vertex".)*
3. **Regular vertex**: bulge = `pline_arc_bulge_to(click)` in Arc mode else `0.0`
   → push to `pending_bulges`; push `pline_next_width` to `pending_widths`; carry
   the end width forward (`pline_next_width = (end, end)`); clear the one-arc
   direction override; push the point to `pending`.

**Finish conditions:** empty **Enter** (not in a width sub-flow) → finish as
**open**; `c` (or `c` then Enter, or auto-close) → **closed**. **Esc removes only
the LAST placed vertex** (one segment per press), never the whole run — once no
vertices remain it exits the tool. **Placed vertices are never discarded:**
ending the command by any other means (switching tools, running another command)
COMMITS the run as a finished open dobject via `commit_active_draw()` (≥2 verts
for a polyline, ≥3 controls for a spline). Picks fire on PRESS (PLINE is in
`in_click_only_phase`), so a small drag is still a vertex, never a window.

---

## 5. Sub-commands typed while drawing (`app.rs:~3493`)

Intercepted **before the parser** so they mean PLINE, not the global command:

| Key(s) | Effect |
|---|---|
| `a` / `arc` | switch to **Arc** mode (next segments curve) |
| `l` / `line` | switch back to **Line** mode |
| `c` / `close` | commit the run as **closed**, tool stays active *(historically leaked to global Copy — now intercepted)* |
| `u` / `undo` | undo the last vertex, OR cancel the active arc sub-flow |
| `s` / `second` | (Arc mode) start a **3-point arc**: click on-arc point, then endpoint |
| `d` / `direction` | (Arc mode) arm a **start-tangent override**: click a point OR type an angle° (one arc only) |
| `w` / `width` | enter **width** flow: type start width, then end width |
| `h` / `halfwidth` | same as `w` but the entered value is **doubled** (you type half-width) |

Prompts come from `update_pline_prompt()` (`app.rs:~16609`), which shows the menu
per mode/sub-state (`[Arc / Halfwidth / Length / Undo / Width | Enter=finish, 'c'
Enter=close]`, etc.).

---

## 6. Width feature (tapered, sticky)

- **Entry** (`pline_width_cap`): `w`/`h` → `AwaitingStart{half}` → type start (≥0;
  halved value ×2 if `h`) → `AwaitingEnd{half,start}` → type end (empty Enter =
  same as start; non-numeric = cancel, keep previous) → sets `pline_next_width =
  (start,end)`, back to `None`. **Entering a width does NOT finish the polyline.**
- **Empty-Enter guard** (`pline_width_accept_default`, `app.rs:~16689`): an empty
  Enter *inside* the width flow advances the step (accept default) instead of
  finishing the polyline — without this, pressing Enter to accept a default width
  used to commit the whole polyline and drop the width.
- **Sticky / forward taper:** `pline_next_width` **persists across polylines**
  (AutoCAD PLINEWID) — only an explicit `w`/`h` changes it. After each segment the
  end width becomes the next segment's start, so width tapers smoothly across the
  run.
- **Halfwidth:** type half; stored as full (prompt shows the halved value).

---

## 7. Arc mode (bulge, direction, 3-point, tangent continuity)

- `pline_arc_bulge_to(end)` (`app.rs:~16812`) computes the rubber-band/commit
  bulge: tangent priority = `pline_dir_override` → `pline_previous_exit_tangent()`
  → default +X. `alpha = angle(chord, tangent)`, `bulge = tan(alpha/2)`. So
  consecutive arcs are **G1 tangent-continuous** by default.
- `pline_previous_exit_tangent()` (`app.rs:~16657`) derives the previous segment's
  exit direction from its bulge (`alpha = 2·atan(bulge)`, rotate the chord).
- **Direction override** (`d`): one-arc-only start tangent (point or typed angle);
  cleared after that arc commits.
- **3-point arc** (`s`): click an on-arc point then the endpoint →
  `bulge_from_three_points`.

---

## 8. Rendering

- **Thin** (widths empty): standard stroke via the normal `draw_dobject` path.
- **Wide** (widths present): `draw_polyline_widths` → `polyline_width_centerline`
  (`app.rs:~26060`, tessellates bulged segments to arc samples, linearly tapers
  width per segment, **re-emits a coincident point at a width step** so a width
  change is a sharp jump not a false taper) → `fill_width_strip` (`app.rs:~26101`,
  **solid fill**: per-segment quads with **miter joins** within an 8×-half-width
  limit, else **convex-hull bevel** — no spikes, no internal seams; one convex
  polygon per joint).
- **Live preview:** `pline_phantom_dobject` (snap-only, every frame, never
  inserted) feeds the snap engine the uncommitted vertices;
  `draw_pline_preview_segment` draws each committed segment + the rubber-band to
  the cursor (arc rubber-band uses `pline_arc_bulge_to(cursor)` in Arc mode).

---

## 9. Commit + drain

`drain_pline_pending(closed)` (`app.rs:~16711`) turns `pending` /
`pending_bulges` / `pending_widths` into `(Vec<PolyVertex>, Vec<(f64,f64)>)`,
clears all transient draw state (mode, arc-sub, dir-override, width-cap) **but
keeps `pline_next_width` sticky**, and drops the widths vector entirely if every
segment is thin (space saving). The committed `Geom::Polyline` is pushed via
`add_dobject`.

---

## 10. Interplay with other commands

- **Explode** (`explode_polyline`, `app.rs:~166`): polyline → individual `Line`
  (straight) / `Arc` (bulged) segments; widths discarded (Lines/Arcs have none).
- **Trim** (`trim_polyline_connected`, `trim.rs:~17`): trimming a wide polyline
  keeps **connected before/after runs WITH their widths**; isolated survivors are
  wrapped in 1-segment polylines (`wrap_with_width`) to carry width.
- **Extend** (`trim.rs`): extends the end *segment* nearest the pick; preserves
  width; recomputes bulge.
- **PEDIT** (`app/pedit.rs`): converts a single Line→2-vertex polyline (bulge 0),
  Arc→2-vertex (computed bulge), Ellipse/Spline→tessellated polyline; sub-menu
  Open/Close/Join/Width/Undo/eXit.
- **Join** (`join_geoms`, `join.rs:112`): chains touching Line/Arc → one polyline
  (3-pass). Chain-joined polylines have **empty widths** (Lines/Arcs carry none).

---

## 11. Persistence

- **DXF LWPOLYLINE** (`cad_io/src/dxf.rs:~421`): per-vertex `10/20` (x/y), `42`
  (bulge), `40`/`41` (segment start/end width), `43` (constant width), `70` bit 0
  (closed). Reader truncates widths to segment count, clears if all thin.
- **RSM** (`cad_io/src/rsm.rs`): widths written/read only at **format version ≥ 7**
  (older v4–v6 files have none — see `SETTINGS.md`/the RSM version note). Per
  segment: `(start:f64, end:f64)`.

---

## 12. Tests

- DXF: `polyline_widths_round_trip` (widths (2,2)&(1,3) via 40/41),
  `polyline_round_trip_open`, `polyline_round_trip_closed`.
- RSM: `polyline_widths_round_trip` (v7 round-trip).

---

## 13. Gotchas & invariants

- **Sub-commands intercept before the parser** — `a/l/c/u/s/d/w/h` are PLINE
  sub-commands while drawing, not the global Arc/Line/Copy/etc. (the recorder logs
  the parser's pre-intercept guess; the intercept wins).
- **Auto-close needs ≥3 verts** and a click within `PkBxSz` of vertex 0.
- **Width entry never finishes the polyline**; the empty-Enter-in-width guard is
  load-bearing.
- **`pline_next_width` is sticky** across polylines (PLINEWID); only `w`/`h`
  changes it. End width carries forward to the next segment's start.
- **Width step needs a re-emitted coincident point** or the segment after a width
  change false-tapers (known quirk fixed in `polyline_width_centerline`).
- **Width is a SOLID FILL**, not a thick stroke — per-segment quads + miter/bevel
  joins (8× half-width miter limit), one convex polygon per joint (no AA seams).
- **Direction override is one-arc-only**; cleared after the arc commits.
- **Bulge sign** must survive major arcs — use `bulge_from_arc`'s CCW-vs-swept
  comparison, not a chord-side test.

---

## 14. Port recipe

1. **Data:** `Polyline { vertices: Vec<PolyVertex{pos,bulge}>, closed, widths:
   Vec<(f64,f64)> }`; bulge = tan(sweep/4); widths empty = thin. Port
   `bulge_arc`/`bulge_from_arc`.
2. **Tool:** accumulate `pending` vertices; commit each click (bulge from arc
   mode); auto-close on first-vertex hit (≥3 verts, within pickbox).
3. **Sub-commands:** intercept `a/l/c/u/s/d/w/h` *before* your command parser
   while the tool is active.
4. **Width:** a 2-step entry sub-flow (start→end, halfwidth doubles), sticky
   `next_width`, forward taper, the empty-Enter-advances guard.
5. **Arc:** tangent-continuous bulge from the previous exit tangent; direction
   override; 3-point arc.
6. **Render:** thin = stroke; wide = centerline-with-width → solid fill strip
   (per-segment quads + miter/bevel, re-emit point at width steps).
7. **Preview:** a phantom polyline for snapping + rubber-band the last segment to
   the cursor.
8. **I/O:** DXF 10/20/42/40/41/43/70; RSM width gated behind a format version.
