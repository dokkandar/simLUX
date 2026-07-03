# RUST_CAD — Wall Subsystem (complete explanatory guide)

> Exhaustive handoff doc for the **Wall** feature. A wall is a **smart dobject**:
> it stores only a centerline + thickness + style + bulge, and **derives** its
> two visible faces (and miters/junctions) every frame. Nothing here is meant to
> be skimmed — it's the full reference for re-implementing or extending walls.
>
> Three layers: **kernel data** (`cad_kernel/src/geom.rs` `Wall` +
> `wallstyle.rs`), the **junction solver** (`cad_wall` crate), and the **app**
> (tool/command/render/dialogs in `cad_app/src/app.rs`). Line numbers from
> `9a4bcc7`; grep the symbol if they drift. See `SETTINGS.md` for the SYSVARs and
> `MODIFY_GUIDE.md` for how transforms operate on the centerline.

---

## 0. Mental model

```
stored:   Wall { start, end, thickness, style, bulge }      ← the IDENTITY (a centerline)
derived:  left/right faces  +  miters at shared endpoints  +  X-crossing gaps  +  poché fill
          ──────────────────────────────────────────────────────────────────────────────
          recomputed every frame from the centerline + neighbours; never stored
```

Firm rule: **the centerline is permanent identity; faces are derived, not
stored.** Every transform (move/rotate/scale/mirror/lengthen) acts on the
centerline; the faces re-derive automatically. Editing the thickness keeps the
centerline and just re-offsets.

---

## 1. Data model

### `Wall` (`cad_kernel/src/geom.rs`, the `Geom::Wall` payload)
```rust
pub struct Wall {
    pub start: Vec2,     // centerline start
    pub end:   Vec2,     // centerline end
    pub thickness: f64,  // FULL width (faces are ±thickness/2 from centerline)
    pub style: u32,      // WallStyle id (0 = STANDARD) — drives poché fill, face colour, default thickness
    pub bulge: f64,      // DXF bulge tan(sweep/4): 0 = straight, ≠0 = circular-arc centerline (rounded walls)
}
```
Methods: `is_curved()` (`bulge.abs() > 1e-9`), `length()`, `centerline() -> Line`,
`normal() -> Option<Vec2>` (CCW unit perp), `centerline_polyline(n)` (tessellate
the arc), and the face derivers in §2.

### `WallStyle` / `WallStyleTable` (`cad_kernel/src/wallstyle.rs`)
```rust
pub struct WallStyle {
    pub name: String,        // "Dry Wall", "Structural", …
    pub thickness: f64,      // default full width for this type
    pub fill_color: u32,     // poché fill ACI (0 = hollow, no fill)
    pub face_color: u32,     // face-line ACI (0 = ByLayer/ByBlock)
    pub insulation: bool,    // draw batt-insulation sine wave in the cavity
    pub description: String, // shown in the Wall Style Manager
}
pub struct WallStyleTable { pub styles: Vec<WallStyle> }   // STANDARD const = id 0
```
Table API: `with_defaults()` (seeds `[standard()]`), `get(id)`, `add(s) -> id`,
`find(name) -> Option<id>` (case-insensitive). `WallStyle::standard()` =
thickness 0.2, no fill, no face colour, no insulation. **Storage:**
`Document.wall_styles: WallStyleTable`; walls are `Geom::Wall` in
`Document.dobjects`.

---

## 2. Derived geometry (centerline → faces)

`Wall::face_polylines(n) -> Option<(Vec<Vec2>, Vec<Vec2>)>` returns the **(left,
right)** face point-lists:
- **Straight wall:** two 2-point lines = `left_line()` / `right_line()`, each the
  centerline offset ±thickness/2 along `normal()`.
- **Curved wall (bulge ≠ 0):** tessellate the centerline arc to `n` samples; at
  each sample offset ±t/2 along the **true radial direction** (not finite-diff
  normals) → exact concentric arcs that meet tangent straight neighbours with
  zero gap at fillet joints.

`left_line()` / `right_line()` return the straight face `Line`s (offset +t/2 /
−t/2 from start→end along the normal).

### Bulge helpers (`cad_kernel/src/join.rs`)
- `bulge_arc(a, b, bulge) -> Option<(center, radius, start_angle, sweep)>` —
  DXF bulge → arc geometry. `r = chord·(1+b²)/(4|b|)`, sweep `= 4·atan(b)`.
- `bulge_from_arc(start, end, center, sweep_abs) -> f64` — inverse; signs by
  traversal direction, handles major/minor (>π) arcs.

A curved wall is produced by `fillet` r>0 on a wall corner: it trims the two
straights to tangent and spawns a curved corner wall (`bulge` set).

---

## 3. The junction solver (`cad_wall` crate)

Pure geometry, no app state. This is what makes adjacent walls clean up.

```rust
pub struct WallFaces { pub left: (Vec2,Vec2), pub right: (Vec2,Vec2) }

pub fn solve_faces(this: &Wall, all: &[Wall]) -> Option<WallFaces>;
pub fn solve_face_segments(this: &Wall, all: &[Wall])
    -> Option<(Vec<(Vec2,Vec2)>, Vec<(Vec2,Vec2)>)>;
```

### `solve_faces` — L-corner miter
For each endpoint of `this`, find a neighbour wall whose endpoint coincides
(within `JOIN_TOL = 1e-4`). Relative to each wall's **outgoing** direction at the
shared node:
```
miter_inner = this.leftOut  ∩  neighbour.rightOut
miter_outer = this.rightOut ∩  neighbour.leftOut
```
computed with `line_intersect()` (infinite-line intersection of the facing face
lines). The matched face endpoints move to the miter points. Skips curved
neighbours (they meet tangentially). **Straight walls only.**

### `solve_face_segments` — X-crossing cleanup
1. `solve_faces()` for the L-mitred single-segment faces.
2. Find **crossers**: other straight walls whose centerline crosses this one in
   BOTH interiors (true X), via `centerline_cross(a0,a1,b0,b1) -> Option<(u,v)>`
   (parametric, `u,v ∈ [0,1]`).
3. For each face segment, clip against each crosser's quad footprint
   (`wall_quad(w) -> [Vec2;4]` CCW) with `clip_segment_convex` (Liang–Barsky →
   the `(t_in, t_out)` interval inside the quad).
4. `subtract_intervals(removed)` removes those intervals from `[0,1]` → the
   surviving sub-segments. So a face becomes a **list of disjoint pieces** with a
   clean gap where the other wall passes through.

Helpers: `line_intersect`, `same_wall` (dup/self guard), `centerline_cross`,
`clip_segment_convex`, `subtract_intervals`, `wall_quad`.

---

## 4. App side (`cad_app/src/app.rs`)

### Tool + command
- `Tool::Wall`. Parser: `Command::Wall(Option<f64>)` — bare `wall` → `None`
  (use `env.WlThk`); `wall <t>` → `Some(t)` (validated > 0, **persists** to
  `env.WlThk`).
- Dispatch sets `tool = Tool::Wall`, clears `pending`, prompts for the first
  centerline point.

### Chained-run draw flow
Each pair of clicks commits one `Geom::Wall { start, end, thickness: env.WlThk,
style: current_wall_style, bulge: 0.0 }`, then **keeps the end as the next
segment's start** (`pending = vec![end]`) — a connected run. Adjacent walls share
endpoints, so `solve_faces` auto-miters the corners. **Enter/Esc ends the run.**
Mid-run `t` arms `wall_waiting_thickness` so the next number sets thickness.

### Rendering pipeline
`wall_face_screen_pts(w)` → screen-space face pieces:
- curved: `w.face_polylines(n)` (adaptive `n`, clamped [12,256]) → one polyline
  per side;
- straight: `cad_wall::solve_face_segments(w, all_walls)` → mitred + X-cleaned
  pieces per side (fallback: raw left/right lines).

`draw` then: resolve `WallStyle` → **poché fill** (only when `fill_color≠0` AND
no X-split, alpha ≈ 80) → **face lines** (every segment, `face_color`) →
**insulation** sine wave (`wall_insulation_wave(w)`: amplitude `t/2·0.72`,
wavelength `t`, ~14 samples/wave) if `style.insulation` → **centerline** dashed
overlay (6 on / 4 off, ~110/255 alpha) if `env.WlCnL`.

### Explode (`apply_explode` wall arm)
`Geom::Wall` → its boundary "particles": left face + right face (Line if 2 pts,
else Polyline via `face_polylines(48)`) + start cap + end cap (Lines connecting
the face ends). All inherit the wall's style.

---

## 5. Wall Style system

- `wallstyle` / `wstyle` command → opens the **Wall Style Manager**
  (`render_wall_style_manager`): lists styles, shows "Current wall style", a
  preview (`draw_wall_style_preview` — a wall strip with poché + faces + dashed
  centerline + "t = …" label), and **Set Current / New… / Modify… / Close**.
- **Set Current** sets `current_wall_style` AND syncs `env.WlThk =
  style.thickness` + `env.save()` so the next wall draws at the style's width.
- **New…/Modify…** open `render_wall_style_dialog` (the `WallStyleDialog` form):
  Name (unique, non-empty) · Thickness · Fill Color (ACI picker) · Face Color
  (ACI picker) · Insulation checkbox. OK creates/updates `doc.wall_styles`.
- Mirrors the Dim Style Manager pattern exactly (see `DIMENSION_GUIDE.md`).

---

## 6. SYSVARs

- **`WlThk`** (Float, default 0.20) — default wall thickness; `wall <t>` sets it;
  Wall-Style "Set Current" syncs it.
- **`WlCnL`** (Bool, default true) — render the dashed centerline overlay. Pure
  display; never changes geometry.

(Full briefings in `SETTINGS.md` §12.)

---

## 7. Tests (`cad_wall/src/lib.rs`, 6)

`lone_wall_keeps_full_faces`, `l_corner_90deg_miters_both_faces`,
`l_corner_any_angle_meets_at_a_point`, `lone_wall_faces_are_single_segments`,
`x_crossing_breaks_each_face_into_two_with_a_gap`,
`parallel_neighbour_does_not_trim`. (Curved walls + T-junctions are not yet
covered — see §8.)

---

## 8. Built vs. owed

**Built:** straight-wall draw (chained), L-corner miter (any angle), X-crossing
gap cleanup, curved walls (bulge, concentric arc faces, tangent joins), wall
styles (thickness/poché/face colour/insulation/description) + Manager + dialog,
poché fill, insulation symbol, centerline overlay, explode → faces+caps, all
transforms on the centerline.

**Owed:** T-junction (trim branch faces to the through near-face), collinear and
3+-walls-at-a-node junctions, poché fill on X-split bands (currently skipped when
faces split), convert-closed-polyline→wall, curved-wall junction tests. The
junction algorithm is extracted scenario-by-scenario from user session dumps.

---

## 9. Invariants & gotchas

- **Centerline = identity; derive don't store.** Never persist faces/miters.
- **Transforms act on the centerline only**; thickness is direction-invariant;
  faces re-derive. Mirror flips `bulge` sign (winding reverses); rotate preserves
  it.
- **Miter = infinite-line intersection** of facing faces relative to outgoing
  direction; only at coincident endpoints (`JOIN_TOL 1e-4`); curved neighbours
  skipped.
- **X-crossing = clip + subtract**: faces become *lists* of pieces; the renderer
  + explode must handle multi-piece faces (poché skips split bands).
- **Snap should expose the centerline** (endpoints + the line) so a second wall
  can join onto the spine — not just face endpoints.
- **Style id 0 is STANDARD** and always present; `Set Current` is the only thing
  that syncs `WlThk`.

---

## 10. Port recipe

1. **Kernel data:** add a `Wall { start, end, thickness, style, bulge }` variant
   and a `WallStyle`/`WallStyleTable` (port `wallstyle.rs`). Store styles on the
   document.
2. **Derive faces:** port `face_polylines`/`left_line`/`right_line`/`normal` +
   the `bulge_arc`/`bulge_from_arc` helpers for curves.
3. **Junctions:** port the `cad_wall` crate wholesale (it's pure) —
   `solve_faces` + `solve_face_segments` + the 6 helpers. Unit-test with the 6
   cases.
4. **Transforms:** route Wall through your transform layer operating on
   start/end (+ bulge sign on mirror).
5. **App:** a `Tool::Wall` chained-draw (commit pair → keep end), a `wall
   [t]` command persisting a `WlThk` default, a renderer calling
   `solve_face_segments` (+ poché/insulation/centerline), an explode arm, and the
   Style Manager+dialog (clone the Dim Style one).
6. **SYSVARs:** `WlThk`, `WlCnL`.
