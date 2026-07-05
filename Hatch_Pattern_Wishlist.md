# Hatch Pattern Wishlist

Captured 2026-06-05 from a tester reference image. The current
`HatchPattern` engine handles **line-based repeating patterns only** (8
families: ANSI31, ANSI32, BRICK, DOTS, etc.). The reference images
show four distinct categories of hatch the engine needs to support
eventually — listed here in rough order of architectural reach.

The user's specific call-out: the **last two categories (terrazzo,
stones/riverrock)** are not line patterns. Each tile or chip is a
**solid-filled region with its own color/shade**. That's a new render
path, not a new entry in the line-pattern table.

---

## 1. Line-pattern extensions (same engine, just more entries)

These slot directly into the existing `HatchPattern::Pattern` mechanism.
Each is a definition of strokes (start point, direction, dash sequence,
offset between rows). Cheap to add once the pattern definition file
exists. LibreCAD's `hatch_patterns/` directory has many of these in
`.dxf` format and is the obvious starting reference.

### Masonry / paving (reference image 1)
- Running bond brick (current BRICK is close — needs `stagger=0.5`)
- Stack bond brick (no stagger)
- Stretcher bond, English bond, Flemish bond
- Soldier course
- Herringbone — 45° and 90° variants
- Basket weave — 2×1, 3×1, 4×1
- Cobblestone hex
- Plank / wood floor tile
- Cube / 3D-illusion brick (three-tone basket weave)
- Voronoi-style random tile

### Geometric / linear (reference image 2)
- Plain horizontal at varied spacings (3 weights)
- Concentric circles in a grid (bullseye fill)
- Decorative tile bullseye (with center dot or rosette)
- Wave / sine pattern
- Heavy diagonal lines
- Dashed horizontals with dot midpoints
- Wood grain (multiple stylizations — straight, figured, heavy)
- Random dot fills (sparse / medium / dense)
- Chevron / zigzag
- Rain / grass (short diagonals)
- Spider web (concentric circles + radial lines)
- Wavy diagonal in varied amplitude/frequency

**All of the above** can be added by:
1. Defining a `.pat`-style pattern table (AutoCAD's `acad.pat` format
   is the standard; LibreCAD's hatches mirror it).
2. Wiring a loader for the pattern table.
3. Extending the hatch-fill renderer to consume tessellated strokes
   instead of hard-coding the 8 current families.

Estimate: ~200 lines of pattern definitions + a parser. The hard
work (clipping the strokes against the boundary loops) is already done
in the existing `render_hatch_pattern`.

---

## 2. Composite solid-fill patterns (NEW engine path)

The user's last two examples — **terrazzo** and **stones / riverrock**
— are fundamentally different. The pattern is not a set of repeating
strokes; it's a tile of **N filled regions, each with its own color**.

### Terrazzo (reference image 3)
- White / light-grey background
- Random scattering of small **filled polygons** (chips):
  - ~70% medium-grey chips of varied irregular shape
  - ~30% black chips, smaller, denser
  - Occasional accent colors possible (red, gold, green — user-tunable)
- Chip shapes are random polygons (not regular)
- Tiles seamlessly when laid out across the boundary

### Stones / Riverrock (reference image 4)
- Outlined irregular polygons (the "stones")
- Each stone is **solid-filled** with a color the user can pick
  individually OR via a palette (e.g., 5 shades of grey, randomly
  distributed)
- Black grout/gap between stones
- Sizes vary across the tile

### What's required architecturally

This is **not** a line pattern. The data model needs:

```rust
enum HatchPattern {
    Solid,                              // existing
    Pattern { name, scale, angle_deg }, // existing — line strokes
    Composite(CompositePattern),        // NEW
}

struct CompositePattern {
    /// Repeating tile in pattern-local units.
    tile_size:    Vec2,
    /// Each region inside the tile.
    pieces:       Vec<CompositePiece>,
    /// Color palette the user can adjust per hatch instance.
    /// Each piece references a palette index (not a color directly)
    /// so changing one swatch recolors all matching pieces.
    palette:      Vec<Color>,
    /// Optional randomness seed for procedural variation
    /// (e.g. chip placement). If None, pattern is fully deterministic.
    rng_seed:     Option<u64>,
}

struct CompositePiece {
    /// Closed polygon in pattern-local coords.
    polygon:      Vec<Vec2>,
    /// Index into the parent pattern's palette.
    palette_idx:  usize,
}
```

The renderer for `Composite`:
1. Compute the tile grid that covers the hatch boundary.
2. For each tile: clip each piece's polygon to the boundary loops.
3. Fill each clipped polygon with the palette color.
4. Optional: draw piece outlines if `outline_stroke` is set (gives the
   stones their black gap).

### Per-instance color editing (the user's specific ask)

Each hatch dobject carries its OWN palette override (defaults to the
pattern's defaults). UI: the Hatch dialog gains a swatch row when a
Composite pattern is selected, one swatch per palette slot. Clicking a
swatch opens the ACI picker. The picker already exists in the codebase.

```rust
struct Hatch {
    boundary_handles: Vec<Handle>,
    pattern:          HatchPattern,
    /// Only set if pattern is Composite — overrides the pattern's
    /// default palette. Indices must match palette size.
    palette_override: Option<Vec<Color>>,
}
```

This keeps the pattern definition shared / library-stored, but the
per-instance shading is local to each hatch on the canvas. Two
terrazzo fills in the same drawing can have different chip colors.

### Procedural variation (optional, for "natural" feel)

The terrazzo image looks random because the chips aren't on a regular
grid. Two approaches:

1. **Pre-baked tile**: define a single tile with ~50 chips placed
   pseudo-randomly. Same tile repeats. Fine for most renders; visible
   repetition at extreme zoom-out.
2. **Procedural**: deterministic noise function (`rng_seed`) generates
   chip placements per tile cell. No repetition, but slower to render
   and harder to round-trip to DXF (would need to bake to chips on
   export).

Recommend (1) for the first slice; (2) only if a tester explicitly
complains about visible repetition.

---

## 3. Tessellated / image-based fills (further out)

Real-world materials (granite, marble, fabric weaves) ultimately need
**raster** fills — a tile of pixels, not vectors. That's:

- A new `HatchPattern::Image { tile_handle, tile_size_world }` variant
- The kernel needs an Image dobject type (currently missing — see
  LibreCAD parity audit, kernel gaps section)
- GPU upload + tile-and-clip render path

This is the right long-term answer for terrazzo/stones too once Image
support exists. Composite-fill (section 2) is the vector-only path that
works without Image entities.

---

## Recommended slicing order (when this work is picked up)

1. **`.pat` loader** for line patterns. Adds ~40 named patterns in one
   slice. Covers reference images 1 + 2 entirely.
2. **`Composite` pattern variant + UI** for solid-tile patterns.
   Initial pattern set: terrazzo (3 chip colors), riverrock (5 stone
   shades), brick-with-mortar. Per-instance palette override in the
   Hatch dialog.
3. **Image dobject** + raster-tile hatch (way after blocks/text/dims
   per the LibreCAD parity audit; this is a polish feature).

Estimate for slices 1 + 2: 2–3 days each.

---

## DXF round-trip notes

- AutoCAD's `.pat` line patterns: standard DXF, full round-trip works.
- AutoCAD's solid-fill via SOLID dobjects: standard DXF; would need
  composite patterns to bake out to many SOLID + LWPOLYLINE entries on
  export (or store as a "composite hatch" object in XDATA — non-
  portable).
- Image-based hatch: AutoCAD uses `IMAGE` entities, not hatch patterns.
  Our `HatchPattern::Image` would need to translate to that on export.

The composite-fill path is a **lossy export** in DXF terms (it would
bake to many polygons). That's an accepted trade for visual fidelity.

---

## Reference images

The four reference images from 2026-06-05 are described in §1–§2 above.
Recommend keeping a `Hatch_Pattern_Reference/` subfolder with the
images themselves once the work is scheduled, so the implementation
agent has visual ground truth alongside this spec.
