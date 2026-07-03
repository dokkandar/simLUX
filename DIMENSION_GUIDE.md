# RUST_CAD — Dimension Subsystem (complete explanatory guide)

> Exhaustive handoff doc for the **Dimension** feature. A dimension is a **smart
> dobject**: it stores only its defining points + a kind + a style id, and
> **derives** the measured value, the formatted text, and the drawn geometry
> (extension lines, dim line, arrows, leader, text) every frame. Nothing here is
> meant to be skimmed.
>
> Layers: **kernel data + behaviour** (`cad_kernel/src/dim.rs`, wrapped as
> `Geom::Dimension`), **transforms/grips** (`cad_kernel/src/geom.rs`), and the
> **app** (smart-dim flow, render, Style Manager in `cad_app/src/app.rs`). Line
> numbers from `9a4bcc7`; grep the symbol if they drift. See `SETTINGS.md`,
> `MODIFY_GUIDE.md`, and `WALL_GUIDE.md` (the Style-Manager pattern is shared).

---

## 0. Mental model

```
stored:   Dim { kind, style, text_override }       ← defining points + which style
derived:  measured_value()  →  formatted_text(style)  →  render geometry (lines+arrows+text)
          recomputed every frame; dragging a grip just moves a defining point and it all re-derives
```

---

## 1. Data model (`cad_kernel/src/dim.rs`)

### `Dim`
```rust
pub struct Dim {
    pub kind: DimKind,
    pub style: u32,                      // index into Document.dim_styles (0 = STANDARD)
    pub text_override: Option<String>,   // None = auto; "<>" inside = "prefix <measured> suffix"
}
```

### `DimKind` (V1: three variants; enum is extensible)
```rust
pub enum DimKind {
    Linear   { p1: Vec2, p2: Vec2, dimline_pos: Vec2, ortho: LinearOrtho },
    Radius   { center: Vec2, on_circle: Vec2, leader_end: Vec2 },
    Diameter { center: Vec2, on_circle: Vec2, leader_end: Vec2 },
}
pub enum LinearOrtho { Horizontal, Vertical, Aligned }
```
- **Linear**: `p1`/`p2` are the two defining points; `dimline_pos` is any point
  the dim line passes through (sets the offset); `ortho` picks the measurement
  axis. *(`LinearOrtho` is a distinct concept from the CARD/`CrdEnb` drafting
  lock — don't conflate; it keeps its name.)*
- **Radius/Diameter**: `center` + `on_circle` (point on the circumference) +
  `leader_end` (text/leader-tail position). Auto-detected from a circle/arc pick.
- **Planned kinds:** Angular, arc-length, ordinate, leader — adding one doesn't
  break serialization.

### `DimStyle` — ~70 fields = AutoCAD DIMVAR parity
V1 *renders* a subset; the rest round-trip (clone+patch preserves them). Grouped:
- **Arrows:** `arrow_size` (DIMASZ), `arrow_block`/`_1`/`_2` (DIMBLK/1/2),
  `separate_arrows` (DIMSAH), `leader_block` (DIMLDRBLK), `tick_size` (DIMTSZ —
  >0 = architectural tick instead of arrowhead), `arrow_filled`.
- **Text:** `text_height` (DIMTXT), `text_gap` (DIMGAP), `text_style_name`
  (DIMTXSTY → `Document.text_styles`), `text_vert_pos` (DIMTAD: 0 centered / 1
  above / 4 below), `text_horiz_just` (DIMJUST), `text_vert_offset` (DIMTVP),
  `text_inside_horiz` (DIMTIH), `text_outside_horiz` (DIMTOH),
  `text_force_inside` (DIMTIX), `text_force_dimline` (DIMTOFL),
  `text_user_positioned` (DIMUPT), `text_move_rule` (DIMTMOVE).
- **Linear units:** `linear_unit_format` (DIMLUNIT), `decimal_places` (DIMDEC),
  `rounding` (DIMRND), `zero_suppress` (DIMZIN), `fraction_format` (DIMFRAC),
  `decimal_separator` (DIMDSEP), `linear_scale` (DIMLFAC), `linear_post`
  (DIMPOST, `<>` placeholder).
- **Alternate units:** `alt_units_enabled` (DIMALT) + format/decimals/scale/
  rounding/zero-suppress/post, `arc_length_symbol` (DIMARCSYM).
- **Angular units:** `angular_unit_format` (DIMAUNIT), `angular_decimal_places`
  (DIMADEC), `angular_zero_suppress` (DIMAZIN).
- **Tolerance / limits:** plus/minus, decimal places, text scale, vert just,
  zero-suppress, limits, alt-tolerance.
- **Extension lines:** `ext_line_extend` (DIMEXE), `ext_line_offset` (DIMEXO),
  `ext_suppress_1/2` (DIMSE1/2), `ext_fixed_length`/`_on` (DIMFXL/DIMFXLON),
  `ext_linetype_1/2`.
- **Dim line:** `dim_line_extend` (DIMDLE), `dim_line_baseline_inc` (DIMDLI),
  `dim_suppress_1/2`/`_outside` (DIMSD1/2/DIMSOXD), `dim_linetype`.
- **Colors:** `color_dim_line`/`_ext_line`/`_text` (DIMCLRD/E/T, ACI, 0=ByBlock),
  `text_fill_mode` (DIMTFILL), `text_fill_color`.
- **Lineweights:** `lineweight_dim_line`/`_ext_line` (DIMLWD/E).
- **Scale + radius:** `overall_scale` (DIMSCALE — multiplies all lengths),
  `center_mark_size` (DIMCEN), `jog_angle` (DIMJOGANG).
- **Arrow-fit:** `arrow_text_fit` (DIMATFIT).

`DimStyle::standard()`: arrow_size 0.18, text_height 0.18, text_gap 0.09,
decimal_places 4, ext_line_extend 0.18, ext_line_offset 0.0625, overall_scale
1.0, center_mark_size 0.09, jog_angle 45°, arrow_filled true, text_vert_pos 0,
arrow_text_fit 3.

### `DimStyleTable`
`styles: Vec<DimStyle>`, `STANDARD = id 0`. API: `with_defaults`, `get(id)`,
`add(s) -> id`, `find(name) -> Option<id>`. Stored on `Document.dim_styles`.

---

## 2. Kernel behaviour

### `measured_value(&self) -> f64`
- Linear: `Horizontal` → `|p2.x-p1.x|`; `Vertical` → `|p2.y-p1.y|`; `Aligned` →
  `|p2-p1|`.
- Radius → `|on_circle-center|`; Diameter → `2·|on_circle-center|`.

### `formatted_text(&self, style) -> String`
- If `text_override` is Some & non-empty: contains `<>` → substitute the
  formatted measured value; else return verbatim.
- Else: format the measured value. **Prefix** = `R` (Radius) / `⌀` U+2300
  (Diameter) / none (Linear). Apply `linear_post` (DIMPOST) prefix/suffix split
  on `<>`. Apply `linear_scale` (DIMLFAC), `rounding` (DIMRND, `round_to`),
  `zero_suppress` (DIMZIN: bit 4 leading / bit 8 trailing), `decimal_separator`
  (DIMDSEP). **No expression/`muParser` evaluation in V1** (planned).

### Transforms — `with_points_mapped(f)`
Maps every defining point through a closure, preserving `ortho`/`style`/
`text_override`. `geom.rs` transforms delegate to it: `translated`, `rotated`
(text angle updated), `scaled`, `scaled_xy`, `mirrored` (text angle reflected).

### Hit-test + grips
- `grip_points()` → 3 grips: `(DimP1, DimP2, DimLeader)` = (p1,p2,dimline_pos)
  for Linear; (center,on_circle,leader_end) for Radius/Diameter.
- `outline_segments() -> Vec<(Vec2,Vec2)>` for click selection: Linear = 2
  extension lines (p1/p2 projected onto the dim line per `ortho`) + the dim line;
  Radius = center→on_circle + on_circle→leader_end; Diameter = opposite→on_circle
  + leader leg.
- `bbox()` — conservative over all defining points.

---

## 3. App side — the smart `dim` command (`cad_app/src/app.rs`)

### Command
- Parser: bare `dim`/`dimension` → `Command::Dim`; `dimstyle [name]`/`ddim` →
  `Command::DimStyle(Option<String>)`.
- `Command::Dim` dispatch: `tool = Tool::Dim`, `dim_draft =
  DimDraftState::WaitingForP1`, prompt "click first point (or a circle/arc for
  radius/diameter)".

### `DimDraftState` state machine
```rust
enum DimDraftState {
    Off,
    WaitingForP1,                              // auto-detects circle/arc vs linear
    WaitingForP2 { p1 },                       // linear: need the second point
    WaitingForDimLinePos { kind: DimDraftKind },// final placement click
}
enum DimDraftKind {
    Linear   { p1, p2, ortho },   // ortho starts Aligned (re-pickable via grips)
    Radius   { center, on_circle },
    Diameter { center, on_circle },
}
```

### Click flow (`handle_dim_click`)
1. **WaitingForP1:** hit-test for a circle/arc. **Hit** → `WaitingForDimLinePos{
   Radius{center, on_circle=nearest-circumference-point} }` (prompt: "click
   leader position — D toggles to Diameter"). **Miss** → `WaitingForP2{p1=click}`.
2. **WaitingForP2{p1}:** → `WaitingForDimLinePos{ Linear{p1, p2=click,
   ortho:Aligned} }` (prompt: "click dim line position").
3. **WaitingForDimLinePos{kind}:** the click fills `dimline_pos` (Linear) or
   `leader_end` (Radius/Diameter); build `Geom::Dimension(Dim{ kind, style:
   current_dim_style, text_override: None })`; `add_dobject`; reset to `Off`.

### Radius ↔ Diameter toggle
While `Tool::Dim` + `WaitingForDimLinePos`, typing `d`/`dia`/`diameter` flips
Radius→Diameter; `r`/`rad`/`radius` flips back (intercepted on Enter, before the
parser). `current_dim_style` (default 0) is stamped on every new dim.

---

## 4. Rendering

### `dim_render_geometry(d, style) -> DimGeo`
`DimGeo { ext_lines, dim_line, leaders, arrows, text_pos, text_angle,
text_on_dim_line }`. All lengths scaled by `overall_scale` (DIMSCALE).
- **Linear:** direction `u` + normal `n` from `ortho` (Aligned → along p1→p2;
  Horizontal → (1,0); Vertical → (0,1)). Dim-line offset = `(dimline_pos-p1)·n`;
  project p1,p2 onto the dim line (a,b). Extension lines from p1/p2 (+offset) to
  a/b (+extend). Arrows at a (dir b→a) and b (dir a→b). Text placed per DIMTAD
  (centered/above/below) and rotated per DIMTIH (aligned rotates, else
  horizontal).
- **Radius/Diameter:** leaders center→on_circle, on_circle→leader_end; arrow at
  on_circle pointing toward center; text outward from leader_end.

### `draw_dimension(d)`
Resolve `DimStyle` (fallback id 0). Per-element colours (`color_dim_line/
ext_line/text` if ≠0, else dobject colour). **Min screen clamps:** text ≥11px,
arrows ≥8px (so they don't vanish at zoom-out). Draw: extension lines + leaders →
text (size-clamped) → dim line (full, or broken with a gap where text sits on it)
→ arrows (filled triangle / hollow outline / architectural 45° tick per
`arrow_filled`+`tick_size`).

---

## 5. Dim Style system

- `dimstyle`/`ddim` → **Dimension Style Manager** (`render` around app.rs:7868):
  list of styles (✔ = current) · live preview (`draw_dim_style_preview` — a
  sample plate with a hole + rounded corner showing a horizontal linear, a
  vertical, a ⌀ diameter, and an R radius) · **Set Current / New… / Modify… /
  Override… (stub) / Compare… (stub)** · a description line.
- **Set Current** → `current_dim_style = id`.
- **New…/Modify…** → `DimStyleDialog` (form): Name (unique, non-empty), Arrow
  size + kind (Filled/Hollow/Tick), Colors (dim/ext/text via ACI picker, None =
  ByBlock), Text height/color/placement(DIMTAD)/alignment, Decimal places. OK
  uses the **clone+patch pattern**: clone the source style (STANDARD for new, the
  edited one for modify), patch only the exposed fields, **leave the other ~65
  DIMVARs untouched** (round-trip fidelity), convert ArrowKind → `arrow_filled` +
  `tick_size`, then append or replace in `doc.dim_styles`.

---

## 6. Grip editing

`GripRole::{DimP1, DimP2, DimLeader}` (geom.rs). `with_grip_moved(role, new_pos)`
updates the matching defining point (p1/center, p2/on_circle, dimline_pos/
leader_end), preserving the rest; everything re-derives next frame. (TODO noted
in the click handler: snap grips to circle quadrants.)

---

## 7. Tests (`cad_kernel/src/dim.rs`, 11)

`standard_present_at_id_zero`, `measured_value_linear_aligned` (3,4→5),
`measured_value_linear_horizontal_ignores_y`, `measured_value_diameter_is_twice_radius`,
`formatted_text_includes_radius_prefix`, `formatted_text_diameter_prefix`,
`text_override_with_placeholder_substitutes_value`,
`zero_suppression_trailing_works`, `zero_suppression_leading_works`,
`linear_scale_multiplies_value`, `rounding_step_applies`.

---

## 8. Persistence status

- **DXF (`cad_io/src/dxf.rs`):** V1 **explodes** dimensions on write — only the
  measurement TEXT is emitted; defining points are NOT round-tripped. Full
  AutoCAD `DIMENSION` entity + DIMVAR round-trip is queued.
- **RSM:** the `DimStyleTable` is written/read with the doc (v3+); the clone+patch
  dialog keeps unexposed DIMVARs intact for fidelity once full I/O lands.

---

## 9. Built vs. planned

**Built:** Linear (Horizontal/Vertical/Aligned), Radius, Diameter; smart
auto-kind from the first pick; D/R toggle; ~70-field DimStyle (subset rendered,
all preserved); Dim Style Manager + dialog (clone+patch); grips; measured-value +
formatted-text with DIMPOST/DIMZIN/DIMRND/DIMLFAC/DIMDSEP; per-element colours;
min-px clamps; arrow styles (filled/hollow/tick).

**Planned:** Angular / arc-length / ordinate / baseline / continue / leader
kinds; tolerance rendering; expression evaluation in text overrides; full DXF
DIMENSION round-trip; Override…/Compare… in the Manager; grip snap to quadrants;
alternate-units rendering.

---

## 10. Invariants, gotchas & port recipe

**Gotchas:** value/text/geometry are **always derived** — never store the text;
`text_override` with `<>` means "prefix `<measured>` suffix"; `LinearOrtho`
(measurement axis) ≠ CARD lock; the dialog must **clone+patch** so it never drops
the unexposed DIMVARs; min-px clamps keep dims legible when zoomed out; new dims
always stamp `current_dim_style`.

**Port recipe:**
1. Kernel: `Dim { kind, style, text_override }` + `DimKind`/`LinearOrtho` +
   `DimStyle` (start with the rendered subset, keep the rest as fields for
   round-trip) + `DimStyleTable` (STANDARD=0) on the document.
2. Behaviour: `measured_value`, `formatted_text` (prefix + DIMPOST + scale +
   round + zero-suppress + separator), `with_points_mapped` for transforms,
   `grip_points`/`outline_segments` for selection.
3. App: a `Tool::Dim` + `DimDraftState` (P1 → auto-detect circle/arc → P2/leader
   → placement), the D/R toggle, `current_dim_style`.
4. Render: a `DimGeo` builder (ortho dir/normal, project defining points, arrows,
   text per DIMTAD/DIMTIH) + a draw fn with per-element colour + min-px clamps +
   the three arrow styles.
5. Style UI: Manager (list/preview/Set-Current/New/Modify) + a clone+patch dialog
   — clone the Wall Style Manager (`WALL_GUIDE.md` §5).
6. Persistence: write `DimStyleTable`; explode-to-text DXF is an acceptable V1.
