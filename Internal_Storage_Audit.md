# RUST_CAD — Internal Storage Audit

Generated 2026-06-02. This document maps every byte of the in-memory
representation, the on-disk RSM binary format, and the DXF reader's
group-code → struct-field translation. Read this before changing any
field on `DObject`, `Style`, `Color`, `Geom`, or `Document` — those
changes ripple through I/O and renderer code paths described here.

---

## 1. Top-level container: `Document`

File: `cad_kernel/src/document.rs`

```rust
pub struct Document {
    pub dobjects:   Vec<DObject>,       // the drafting dobjects
    pub layers:     LayerTable,         // named layers
    pub linetypes:  LinetypeTable,      // dash patterns
    pub pens:       PenTable,           // pre-set styles a user can apply
    pub truecolors: TrueColorTable,     // shared 24-bit color table
}
```

Every Dobject's color/linetype/layer reference resolves through these
tables. The Document is what's serialised to RSM / DXF; everything
else (selection, undo stack, UI state) lives outside.

Default state (`Document::default()`):
- `dobjects` empty
- `layers` has one layer: **"LAYER B"** (Base), color = white,
  visible, plottable. See §6 for the inheritance defaults.
- `linetypes` has the canonical CONTINUOUS (id 0) plus DASHED (id 1)
  and DASH-DOT-CENTER (id 2).
- `pens` ships with 7 preset entries (ByLayer + 6 named styles).
- `truecolors` empty — populated only when the user explicitly picks
  a TrueColor (not an ACI palette index).

---

## 2. The drafting dobject: `DObject`

File: `cad_kernel/src/dobject.rs`

```rust
pub struct DObject {
    pub geom:   Geom,      // ~48 bytes (enum tag + largest variant)
    pub style:  Style,     // ~28 bytes (after color refactor 2026-06-02)
    pub handle: Handle,    //   8 bytes (u64)
}
// Total: ~88 bytes per Dobject.
```

`Handle` is a process-wide monotonic u64 from `next_handle()`. Two
Dobjects never share a handle within one process lifetime.
Future-deferred: per-Document handle namespace so DXF imports can
preserve their hex handles losslessly.

### 2.1 Memory cost at scale

| Dobjects | `geom` | `style` | `handle` | **Total** |
|---|---|---|---|---|
| 100 k | 4.8 MB | 2.8 MB | 0.8 MB | **8.4 MB** |
| 1 M   | 48 MB  | 28 MB  | 8 MB   | **84 MB** |
| 9 M   | 432 MB | 252 MB | 72 MB  | **756 MB** |

Geometry coordinates dominate. Style is constant-per-dobject. To push
past 9 M dobjects on commodity hardware, the next-tier savings (deferred):
- StyleId indirection (collapse Style to a 4-byte ref into a dedup'd
  style table) → 252 MB → 36 MB at 9 M dobjects.
- SoA (Structure-of-Arrays) layout for Geom — separate `Vec<Line>`,
  `Vec<Circle>` etc. instead of `Vec<Geom>` — saves the enum
  discriminant.

---

## 3. Geometry: `Geom`

File: `cad_kernel/src/geom.rs`

```rust
pub enum Geom {
    Line(Line),                 // 32 B  — two Vec2 endpoints
    Circle(Circle),             // 24 B  — Vec2 center + f64 radius
    Arc(Arc),                   // 40 B  — center + radius + start_angle + sweep_angle
    Ellipse(Ellipse),           // 40 B  — center + major Vec2 + ratio f64
    EllipseArc(EllipseArc),     // 56 B  — Ellipse + start_param + sweep_param
    Point(Point),               // 24 B  — location + style u8 + size f32
    Polyline(Polyline),         // 32 B inline (Vec handle); vertices on heap
}
// Enum size: ~48 bytes (tag + max non-Vec variant, aligned).
// Polyline's vertex data lives in a Vec<PolyVertex> on the heap;
// each PolyVertex = 24 bytes (Vec2 pos + f64 bulge).
```

`Vec2` is two `f64` (16 bytes), aligned to 8.

### 3.1 `Polyline` heap cost

| Vertices | Heap (bytes) | Notes |
|---|---|---|
| 4 (closed square) | ~96 | Plus 24 B inline `Vec` handle. |
| 100 | ~2.4 kB | Typical wall trace. |
| 10 000 | ~240 kB | Large contour line. |

---

## 4. Style: per-Dobject visual properties

File: `cad_kernel/src/style.rs`

```rust
pub struct Style {
    pub layer:          LayerId,       // u32 → 4 B
    pub color:          Color,         // 4 B after the 2026-06-02 refactor
    pub linetype:       u32,           // 4 B — LinetypeId
    pub linetype_scale: f32,           // 4 B
    pub lineweight:     Lineweight,    // 8 B (enum + f32 payload)
    pub visible:        bool,          // 1 B (padded)
}
// Aligned: ~28 bytes.
```

Defaults for a fresh dobject (`Style::default()`):
- `layer`          = `LayerTable::LAYER_BASE` (the built-in "LAYER B")
- `color`          = `Color::ByLayer`
- `linetype`       = `LinetypeTable::CONTINUOUS`
- `linetype_scale` = 1.0
- `lineweight`     = `Lineweight::ByLayer`
- `visible`        = true

---

## 5. Color: the indirection refactor (2026-06-02)

File: `cad_kernel/src/color.rs`

```rust
pub enum Color {
    ByLayer,                  // 0-byte payload, inherits from style.layer
    ByBlock,                  // 0-byte payload, inherits from block (when blocks land)
    Aci(u8),                  // 1-byte ACI palette index (1..=255)
    TrueColorRef(u16),        // 2-byte index into Document::truecolors
}
// Enum size: 4 bytes (tag + max payload, aligned).

pub struct TrueColorTable {
    rgbs:   Vec<u32>,                       // index → 0x00RRGGBB
    by_rgb: HashMap<u32, u16>,              // RGB → existing index (dedup)
}
```

Cost per dobject color: **4 bytes always**, regardless of variant.
Was 8 bytes before the refactor (because `TrueColor(u32)` forced u32
alignment).

### 5.1 TrueColor dedup at scale

Every distinct RGB the user picks (via the picker or DXF import)
becomes ONE row in `truecolors.rgbs`. 1 M dobjects all in `#FF8040`
cost 4 MB for the per-dobject `Color` slot + **4 bytes** in the table
(plus the HashMap overhead, ~32 B per unique color).

### 5.2 The inheritance chain

`resolve_color(c, layer_id, &layers, &truecolors) -> (u8,u8,u8)` walks:

```
Color::TrueColorRef(idx)  →  truecolors.get(idx)             → (R,G,B)
Color::Aci(idx)           →  aci_palette(idx)                → (R,G,B)
Color::ByLayer / ByBlock  →  layers[layer_id].color          → recurse
                             (ByLayer/ByBlock fallback → white)
```

The loop is bounded: a layer's color can be ByLayer/ByBlock too, but
the second indirection breaks to white to avoid infinite recursion.

### 5.3 What "by Dobject" / "by Block" / "by Group" / "by Layer" means

| Source | How it's stored | Resolved at |
|---|---|---|
| **By Dobject** | `style.color = Color::Aci(n)` or `Color::TrueColorRef(idx)` on the dobject itself. Overrides everything. | Render time. |
| **By Block** | `style.color = Color::ByBlock`. Resolves to the BLOCK's color when inside a block reference. Falls back to ByLayer outside. **Not yet implemented** — block support is a future slice; today ByBlock is treated as ByLayer. | Render time, via block→layer→table chain. |
| **By Group** | Not a separate concept in our model. Groups (when added) inherit from the dobjects they contain; group selection is a UX concept, not a color source. | N/A. |
| **By Layer** | `style.color = Color::ByLayer`. Resolves via `layers[style.layer].color`. The layer holds the concrete `Color::Aci(n)` or `Color::TrueColorRef(idx)`. **This is the default for new Dobjects.** | Render time. |

Default on every fresh Dobject:
- `style.color = Color::ByLayer`
- `style.layer = LAYER_BASE` (the built-in "LAYER B")
- `layers["LAYER B"].color = Color::Aci(7)` (white)
→ Resolves to (255, 255, 255).

---

## 6. Layer table: `LayerTable`

File: `cad_kernel/src/layer.rs`

```rust
pub struct Layer {
    pub name:       String,
    pub color:      Color,
    pub linetype:   u32,         // LinetypeId
    pub lineweight: Lineweight,
    pub visible:    bool,
    pub locked:     bool,
    pub frozen:     bool,
    pub plottable:  bool,
}

pub struct LayerTable {
    pub layers: Vec<Layer>,      // index = LayerId (u32)
    pub active: LayerId,
}

impl LayerTable {
    pub const LAYER_BASE: LayerId = 0;     // renamed from LAYER_ZERO
}
```

`with_defaults()` populates one Layer at id 0 named **"LAYER B"**
(Base). `LAYER_BASE` is reserved — can't be deleted or renamed (DXF /
RSM round-trips would break).

### 6.1 Visibility / selectability gates

`Document::is_visible(idx)` returns true iff the Dobject's own
`style.visible` is true AND its layer renders (not hidden + not frozen).

`Document::is_selectable(idx)` adds: layer not locked.

---

## 7. Linetype + Lineweight + Pen tables

`LinetypeTable` (`linetype.rs`): each `Linetype` is `name + description
+ pattern: Vec<f32>` where positive = dash, negative = gap.
`CONTINUOUS` (id 0) has empty pattern.

`Lineweight` enum (`lineweight.rs`): `ByLayer | ByBlock | Default |
Custom(f32_mm)`. Same inheritance pattern as Color.

`PenTable` (`pen.rs`): `Pen` = `name + color + linetype + lineweight`.
Default table has 7 entries; clicking a row in the Pen panel applies
the pen's style to every currently selected Dobject.

---

## 8. Binary format: RSM

File: `cad_io/src/rsm.rs`. Little-endian throughout. Version constant:
`VERSION = 1`.

```
Offset  Size  Field
------  ----  -------------------------------------------------
 0       4    MAGIC "RSM\0"
 4       2    VERSION (u16)
 6       2    padding (u16 reserved)
 8       …    LINETYPE TABLE
 …       …    LAYER TABLE
 …       …    PEN TABLE
 …       …    DOBJECT BLOCK
```

### 8.1 Per-section structure

**LINETYPE TABLE**
```
 u32                            n (linetype count)
 repeated n times:
   str                          name           (u32 length + bytes)
   str                          description
   u32                          pattern length
   f32 × pattern_length         pattern entries
```

**LAYER TABLE**
```
 u32                            active LayerId
 u32                            n (layer count)
 repeated n times:
   str                          name
   color                        (see §8.2)
   u32                          linetype id
   lineweight                   (see §8.3)
   u8                           flags  (bit 0=visible, 1=locked,
                                        2=frozen, 3=plottable)
```

**PEN TABLE**
```
 u32                            n (pen count)
 repeated n times:
   str                          name
   color                        (see §8.2)
   u32                          linetype id
   lineweight                   (see §8.3)
```

**DOBJECT BLOCK**
```
 u32                            n (dobject count)
 repeated n times:
   u64                          handle
   u32                          style.layer
   color                        style.color
   u32                          style.linetype
   f32                          style.linetype_scale
   lineweight                   style.lineweight
   u8                           visible (0|1)
   geom                         (see §8.4)
```

### 8.2 Color encoding (on-disk format is INDIRECTION-AGNOSTIC)

```
 u8 tag
   0 = ByLayer
   1 = ByBlock
   2 = Aci(u8) — followed by u8 index
   3 = TrueColor — followed by u32 0x00RRGGBB
```

The on-disk encoding still inlines TrueColor as `u32`. The writer
dereferences `Color::TrueColorRef(idx)` via `truecolors.get(idx)`
to produce that u32. The reader interns each `u32` back into the
loading Document's `TrueColorTable`, returning `Color::TrueColorRef`.
Round-trip is lossless and the on-disk size is unchanged.

### 8.3 Lineweight encoding

```
 u8 tag
   0 = ByLayer
   1 = ByBlock
   2 = Default
   3 = Custom — followed by f32 millimetres
```

### 8.4 Geometry encoding

```
 u8 tag
   0 = Line       — 2 × Vec2 (4 × f64 = 32 B)
   1 = Circle     — Vec2 + f64 radius (24 B)
   2 = Arc        — Vec2 + radius + start + sweep (40 B)
   3 = Ellipse    — Vec2 + Vec2 major + f64 ratio (40 B)
   4 = EllipseArc — Ellipse + f64 start_param + f64 sweep_param (56 B)
   5 = Point      — Vec2 location + u8 style + f32 size
   6 = Polyline   — u8 closed + u32 vertex count + (Vec2 + f64 bulge)×n
```

`Vec2` is two f64 (16 B). No padding inserted; all fields are written
contiguously little-endian.

### 8.5 What's NOT in the RSM yet

- Block table (deferred — `pub blocks: BlockTable` is reserved on
  `Document` as a comment placeholder).
- Group table.
- Text styles, dim styles, UCS list, named views — all listed in
  `document.rs` as future fields.
- Per-dobject xdata (DXF application-defined data).

---

## 9. DXF reader: group code → struct field mapping

File: `cad_io/src/dxf.rs`. DXF is a tagged text format (one "group
code" + value per line). The reader does **two passes**:

1. Tokenise: each pair (code, value) is built into a `Vec<(i32, String)>`.
2. Walk the pairs, dispatching on group code `0` (the DXF spec's
   entity-type marker) and accumulating per-dobject fields.

### 9.1 LAYER table mapping

```
DXF group  →  Field on `Layer`
---------     ---------------------
  2        →  name
  62       →  color = Color::Aci(abs(value)); off if negative
  6        →  linetype name (looked up in LinetypeTable)
  70       →  flags (bit 0 = frozen)
```

The DXF spec also defines:
- `420` → TrueColor (24-bit). **Currently NOT read** — the reader
  ignores 420 on LAYER. Layer truecolor support is a follow-up.
- `430` → color book name. NOT supported.

### 9.2 Dobject common-style mapping

Every dobject reader (LINE, CIRCLE, ARC, ELLIPSE, LWPOLYLINE, POINT)
shares a Style accumulator pass:

```
DXF group  →  Field on `Style`
---------     ---------------------
  8        →  layer name (resolved to LayerId; falls back to LAYER_BASE
                if the named layer doesn't exist)
  62       →  color = Color::Aci(value) — ACI palette index
  420      →  color = Color::TrueColorRef(doc.truecolors.intern(0x00RRGGBB))
  6        →  linetype name
 370       →  lineweight in 0.01 mm units (-1=ByLayer, -2=ByBlock, -3=Default)
  60       →  visible flag (0=visible, 1=invisible)
```

### 9.3 LINE → Geom::Line

```
 10 / 20 / 30  →  start (X, Y, Z — Z ignored: we're 2D)
 11 / 21 / 31  →  end   (X, Y, Z — Z ignored)
```

### 9.4 CIRCLE → Geom::Circle

```
 10 / 20  →  center (X, Y)
 40       →  radius
```

### 9.5 ARC → Geom::Arc

```
 10 / 20  →  center
 40       →  radius
 50       →  start angle (degrees → radians)
 51       →  end   angle (degrees → radians; sweep computed = end - start mod 2π)
```

### 9.6 ELLIPSE → Geom::Ellipse / Geom::EllipseArc

```
 10 / 20  →  center
 11 / 21  →  major-axis endpoint, RELATIVE to center
 40       →  ratio = minor / major
 41       →  start parameter (for full ellipses: 0; for arcs: see EllipseArc)
 42       →  end   parameter
```

If `41 == 0.0 && 42 ≈ 2π` → full `Geom::Ellipse`. Otherwise
`Geom::EllipseArc`.

### 9.7 LWPOLYLINE → Geom::Polyline

```
 70       →  flags (bit 0 = closed)
 90       →  vertex count
 10 / 20  →  per-vertex X / Y (repeated)
 42       →  per-vertex bulge (optional; default 0 = straight)
```

### 9.8 POINT → Geom::Point

```
 10 / 20  →  location
```

`style` and `size` default to 0 (renderer uses cross-hair glyph).

### 9.9 What's NOT read yet

- Block table (`BLOCK` / `ENDBLK` / `INSERT` records): skipped.
- SPLINE / NURBS: skipped (no spline support in the kernel).
- TEXT / MTEXT: skipped.
- DIMENSION: skipped.
- HATCH: skipped.
- Xdata (1001+ codes): skipped.

---

## 10. Default render path summary

For a freshly-drawn Line via the Line tool:

1. `add_dobject(Geom::Line(...))` →
2. `Document::push(DObject::new(geom))` →
3. `DObject::new` calls `Style::default()` →
   - `layer = LAYER_BASE`, `color = ByLayer`, `linetype = CONTINUOUS`,
     `lineweight = ByLayer`, `visible = true`, `linetype_scale = 1.0`
4. `Document::push` rewrites `style.layer` to `layers.active` if the
   active layer differs from LAYER_BASE.
5. At render time: `resolve_color(ByLayer, layer, &layers, &truecolors)`
   → walks to `layers[active_layer].color` → that's `Color::Aci(7)`
   for LAYER B → `aci_palette(7)` → **(255, 255, 255) = white**.

That's the chain referenced by "dobject by default is drawn on LAYER B
(base) with colour 0 (white)" in the project convention.

---

## 11. Where to look when adding a new property

If you add a per-Dobject property X (e.g. transparency, plot style,
hyperlink):

1. **Struct field:** add to `Style` in `cad_kernel/src/style.rs`.
2. **Default value:** add to `Style::default()`.
3. **RSM I/O:** extend `write_dobjects` / `read_dobjects` in
   `cad_io/src/rsm.rs`. Bump RSM `VERSION` if the on-disk shape
   changes.
4. **DXF I/O:** extend the style-accumulator pass in
   `cad_io/src/dxf.rs` with the matching group code (e.g. 440 for
   transparency).
5. **Renderer:** the canvas render loop in `cad_app/src/app.rs`
   builds a `(r,g,b)` + stroke per Dobject. New properties either
   modify the resolved style (transparency → alpha) or add a new
   render channel.
6. **Property panel:** `Dobject_Properties.md` has the master list
   of planned properties — keep it in sync.
7. **Tests:** add a kernel test for the resolve chain + an RSM
   round-trip test in `cad_io`.

---

## 12. Storage refactors deferred (future)

In rough size-payoff order:

1. **StyleId indirection** — collapse Style to a u32 ref into a
   shared `StyleTable`. Two Dobjects with identical style share one
   row. At 9 M dobjects: 252 MB → 36 MB on Style.
2. **SoA Geom storage** — `Vec<Line>`, `Vec<Circle>` etc. instead of
   `Vec<Geom>`. Removes enum discriminant. ~10 % saving.
3. **Per-Document handle namespace** — required before DXF handles
   can be preserved on round-trip.
4. **Block / Group tables** — first-class containers, not just
   selection abstractions. Memory cost: per-block ~64 B + dobject
   references.

None scheduled. Trigger them only when actual test data shows the
ceiling (10 M+ dobjects, or DXF handle preservation needed).
