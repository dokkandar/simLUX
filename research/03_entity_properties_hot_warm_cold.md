# Entity Properties — Hot / Warm / Cold Storage Split

*Research note for RUST_CAD math lab.*
*Date: 2026-05-20.*
*Context: discussion of how to attach properties (Color, Layer, Linetype, …)
to geometric entities without bloating per-entity memory the way LibreCAD does.*

---

## Quick context — where we are when this matters

At the time of writing, `cad_kernel::geom::Entity` is pure geometry:

```rust
pub struct Line   { pub a: Vec2, pub b: Vec2 }
pub struct Circle { pub center: Vec2, pub radius: f64 }
pub struct Arc    { pub center: Vec2, pub radius: f64,
                    pub start_angle: f64, pub sweep_angle: f64 }
```

Color is hard-coded in the renderer (blue normal, yellow selected). There is no
layer, linetype, lineweight, transparency, plot style — nothing. This was
deliberate; properties are deferred until the math / spatial index / render
basics work.

This note records **how we will add them when the time comes**, and the
boolean / hatch rendering split that affects that storage design.

---

## 1. Boolean operations and hatch — CPU or GPU?

### Boolean (union / intersection / subtract on 2D regions)

**CPU. Always.** The work is topological, not vectorisable:

1. Find all intersections between the input boundaries (analytic math).
2. Walk each boundary, classify every sub-segment as "inside" or "outside" the
   other operand using a winding-number or point-in-polygon test.
3. Stitch the surviving sub-segments together into the boundary of the result.
4. Emit one or more new entities representing that boundary.

This is symbolic computation on curves, not pixel arithmetic. There is no
useful GPU mapping — the output is a *new geometric object*, not pixels.

### Hatch

Hatch has two distinct lives:

| Phase | Where it runs | Why |
|---|---|---|
| **Boundary + pattern generation** | CPU | Same topology work as boolean: clip the pattern (a family of parallel lines) against the boundary. Output is N short line segments stored on the hatch entity. |
| **Display** | GPU | Once the segments exist, drawing them is identical to drawing N lines — instanced rendering handles it. |

So hatch is conceptually a CPU-produced entity whose `data` field happens to
be a big list of line segments. It plugs into the rendering pipeline the same
way everything else does.

### Consequence for the storage design

Boolean and hatch *create* new entities. They don't change the shape of the
property storage — they just produce additional rows in the entity vector
with whatever color/layer/linetype the user chose at the time of the
operation. The hot/warm/cold split below isn't affected.

---

## 2. AutoCAD's 7 standard properties — frequency of access

| Property | Read by | Frequency |
|---|---|---|
| **Layer** (visibility + lock state) | renderer, hit-test, snap, every query | every frame, every entity |
| **Color** | renderer | every frame |
| **Lineweight** | renderer | every frame |
| **Linetype** | renderer | every frame (mostly via layer default) |
| **Linetype scale** | renderer | every frame (mostly default) |
| **Transparency** | renderer | every frame |
| **Plot style** | plot subsystem only | once per plot, never interactive |

Two natural groupings:

- **Hot** (`Layer`, `Color`, `Lineweight`): touched on every frame for every
  entity. Must be packed inline with the geometry so the render loop reads
  them with no indirection and no hash lookup.
- **Warm** (`Linetype`, `Linetype scale`, `Transparency`): touched on every
  frame *only when overridden* — the vast majority of entities inherit the
  layer default. Storing them inline wastes memory for the common case;
  better to put them in a sparse side-map.
- **Cold** (`Plot style`): touched only at plot time. Belongs in its own
  side-map, never enters the render path.

---

## 3. Proposed structure for RUST_CAD

The pattern is "pack the hot fields inline, sparse-map the warm ones, sparse-map the cold ones, share the lookup tables on the Document":

```rust
// HOT — read on every render frame
pub struct Entity {
    pub geom:        GeometryKind,    // existing enum, ~40 bytes
    pub layer_id:    LayerId,         // u16  — index into Document.layers
    pub color:       ColorRef,        // 4 bytes: ByLayer | Indexed(u8) | RGB(rgb)
    pub lineweight:  LineweightRef,   // 2 bytes: ByLayer | Mm(u8, in 0.05 mm steps)
}
// Total ~50 bytes; tight, cache-friendly, no heap alloc per entity.

// WARM — stored only when the user has overridden the layer default
pub struct WarmOverrides {
    map: ahash::AHashMap<EntityId, Warm>,
}
pub struct Warm {
    pub linetype:       Option<LinetypeId>,
    pub linetype_scale: Option<f32>,
    pub transparency:   Option<u8>,     // 0 = opaque, 255 = invisible
}

// COLD — touched only at plot time
pub struct PlotProps {
    map: ahash::AHashMap<EntityId, PlotStyleId>,
}

// SHARED — small lookup tables, kept on the document
pub struct Document {
    pub entities:  Vec<Entity>,
    pub layers:    IdTable<Layer>,       // handful → hundreds
    pub linetypes: IdTable<Linetype>,    // < 32 typical
    pub colors:    IdTable<Color>,       // < 256
    pub warm:      WarmOverrides,
    pub plot:      PlotProps,
}
```

`ByLayer` is the magic value meaning "inherit from the layer table". Almost
every entity in a real drawing has `ColorRef::ByLayer` and
`LineweightRef::ByLayer`, so the inline fields are only 4–6 bytes of overhead
per entity in the common case.

---

## 4. Why this is the right pattern — numbers

At **5 M entities**:

| Path | Bytes/entity | Total |
|---|---|---|
| Current (pure geometry) | ~32 | ~160 MB |
| Proposed (hot fields inline) | ~50 | ~250 MB |
| LibreCAD-style (`unique_ptr<Impl>` per entity) | ~200+ | ~1 GB |

The proposed pattern adds 18 B/entity over the current pure-geometry case, in
exchange for fully functional layer / color / lineweight. The warm and cold
maps add nothing per entity until the user actually overrides something.

LibreCAD's pattern, recorded in [01_intersections.md](01_intersections.md)
and [02_pi_and_curve_representation.md](02_pi_and_curve_representation.md),
adds:

- a `unique_ptr<Impl>` per entity (8 B pointer + heap-allocated block),
- inside `Impl`: a `std::map<QString, QString>` (red-black tree, even when
  empty has node-header overhead),
- a `std::vector<std::shared_ptr<DRW_Variant>>` (vector + per-variant
  ref-counted heap alloc),
- 6 quint32 metadata fields.

This conflates "data the user might attach" with "core entity identity",
fragments memory, and makes 5 M entities infeasible without major refactoring.
Avoiding this is the entire reason for the split.

---

## 5. Renderer is the only hot consumer

The render loop reads `geom`, `layer_id` (to test visibility / lock), `color`
(resolving `ByLayer` against `layers[layer_id]`), and `lineweight`. Nothing
else. It never touches `WarmOverrides` or `PlotProps`. This is what makes the
frame budget independent of how many entities have linetype overrides or plot
styles.

When the GPU rendering layer goes in, the instance buffer packs only what the
shader actually needs:

```
struct InstanceData {
    // geometry: per shape kind
    f32 x, y;
    f32 r;             // or length for line
    u32 kind_and_pad;  // 0=line, 1=circle, 2=arc, etc.

    // resolved hot properties
    u32 color_rgba;    // resolved from ColorRef + layer
    f32 lineweight_mm; // resolved
}
```

Cold data never enters the upload. Warm overrides are resolved once during the
upload pass (look up the warm map; if absent, use layer default).

---

## 6. Migration order — when we actually add this

1. **Layers first** — `Layer` table + `layer_id` on `Entity`. Renderer reads
   it for visibility / lock. Color / lineweight still hard-coded.
2. **ColorRef + LineweightRef with `ByLayer` defaults.** Renderer becomes
   "ask the layer if the entity doesn't override". One extra branch per
   render call; negligible.
3. **Linetype table + warm overrides.** Linetype patterns (dashed, dotted,
   centerline …) live as named patterns; the renderer turns each polyline
   into stippled sub-segments at render time. CPU pre-pass, then GPU.
4. **Transparency**, **plot style** — only when needed. Trivial additions
   once the warm / cold maps exist.

Each step is a contained slice with no UI rework. The eventual property
dialog plugs into the same tables.

---

## 7. What this note doesn't decide

- **Entity IDs vs. indices.** Right now we use `Vec<Entity>` and `usize`
  indices. Sparse maps need stable IDs (because indices shift on delete).
  Two options: (a) generational IDs (`(index, generation)`), (b) use a
  `SlotMap` from the `slotmap` crate. Pick whichever when we get there;
  doesn't affect the property layout.
- **How layers handle visibility/lock at query time.** The spatial index
  ignores layer state. The renderer / snap / hit-test layer should filter
  by `layers[entity.layer_id].visible && !locked`. Cheap filter; one
  comparison per hit.
- **Whether the hot block stays an `Entity` struct or splits into Struct-of-
  Arrays (`Vec<Geometry>`, `Vec<LayerId>`, `Vec<ColorRef>`, …) for even
  better cache density.** SoA wins for full-table scans (e.g. "all entities
  on this layer"); AoS wins for per-entity work. Mostly a perf tuning step
  later.

---

*End of note.*
