# GPU Render Update — Implementation Report

*A reply to the author of `GPU_RENDERING_GUIDE.md`.*

Thanks for the guide — it was a solid map and I followed its spine closely
(unified `GpuShapeRenderer`, per-primitive pipelines, one `PaintCallback`,
`view_matrix`, SDF strokes). This document records **what shipped, how, and the
effects**, calls out where I deliberately diverged from the guide (and why),
and explains the one shape I intentionally did **not** port: **Dimension**.

Everything below is live in `cad_app/src/gpu.rs` and the `RenderMode::Gpu`
branch of `cad_app/src/app.rs`. It builds clean and is pushed to `auto_rasm`.

---

## 0. One correction to the guide's premise (important)

The guide's state table says non-ported shapes "**Fall through to CPU
`egui::Painter`**" as if that means CPU rasterization. It does **not**.
`egui::Painter` output is tessellated on the CPU and then **rasterized on the
GPU by `egui_glow`**. So a shape "on the egui painter" is *already GPU-drawn* —
just through egui's mesh pipeline instead of ours.

This reframes the whole effort. The win from a bespoke pipeline is **not**
"move it to the GPU" (egui already did). It's:

1. **Kill per-frame CPU tessellation** of curves (arcs/ellipses/circles) — the
   thing that scales to millions of segments.
2. **Cut vertex bandwidth** — one compact instance vs. a fat triangle mesh.
3. **Analytic quality** — SDF shapes are pixel-perfect at any zoom; tessellated
   ones facet when you zoom in.

That distinction drove several of the deviations below (e.g. analytic arc/
ellipse instead of tessellation, and *not* bothering to port Dimension).

---

## 1. What shipped vs. the guide's phases

| Guide phase | Status | Notes |
|---|---|---|
| **Phase 1** — line + circle pipelines; emit line/circle/arc/ellipse/point/polyline/spline | ✅ done | curves initially via tessellation, then upgraded (see Phase 4) |
| **Phase 2** — triangle fill pipeline (solid hatch, wall poché) | ✅ done | as a **triangle soup**, not per-triangle instancing (deviation) |
| **Phase 3** — wall, linetype dashes, pattern hatch, dimension | ✅ mostly | wall ✅, linetype dashes ✅, pattern hatch ✅ — **dimension intentionally not ported (see §7)** |
| **Phase 4** — analytic arc **and** ellipse SDF | ✅ done | arc exact; ellipse a gradient-approx SDF (exact at the stroke) |
| **§8.1** — reduce per-frame rebuild | ✅ (hatch) | did the part that mattered: a **hatch geometry cache** |

Net: **circle, arc, ellipse, line, point, spline, straight polyline, pattern
hatch, solid hatch, wall, and dashed linetypes** all render on our own
pipelines. Text, Dimension, BlockRef and curved/wide Polyline stay on egui
(already GPU-drawn).

---

## 2. Architecture (`gpu.rs`)

One `GpuShapeRenderer` behind `Arc<Mutex<>>`, five pipelines sharing a static
unit-quad VBO:

```rust
pub struct GpuShapeRenderer {
    circle:   Option<GpuPipeline>,   // instanced ring SDF
    arc:      Option<GpuPipeline>,   // instanced ring SDF + angular clamp
    ellipse:  Option<GpuPipeline>,   // instanced rotated-ellipse ring SDF
    line:     Option<GpuPipeline>,   // instanced segment SDF
    fill:     Option<GpuPipeline>,   // NON-instanced triangle soup
    quad_vbo: Option<glow::Buffer>,
}
```

Instance / vertex types (all `#[repr(C)]`, packed RGBA `u32`):

```rust
CircleInstance  { x, y, r, color }                       // 16 B
ArcInstance     { x, y, r, a0, sweep, color }            // 24 B   sweep normalised (0,TAU] CCW
EllipseInstance { x, y, a, b, rot, color }               // 24 B   a/b semi-axes, rot major angle
LineInstance    { ax, ay, bx, by, half_w, color }        // 24 B
FillVertex      { x, y, color }                          // 12 B   (per-vertex, not instanced)
```

Render entry point, one call per non-empty pipeline, fills first so strokes
land on top:

```rust
r.render(gl, &fills, &circles, &arcs, &ellipses, &lines, &view);
//            fill    circle     arc     ellipse    line
```

The `RenderMode::Gpu` branch walks the viewport candidate set once, resolves
each dobject's colour (selection = amber, snap source = cyan, else resolved
style colour), and pushes into the matching `Vec`. One `PaintCallback` uploads
and draws all of them.

---

## 3. Per-shape mapping

| Shape | Pipeline | How |
|---|---|---|
| Line | LINE | one `LineInstance` |
| Point | LINE | two crossing `LineInstance` |
| Straight thin Polyline | LINE | segment per edge (+ closing edge) |
| Spline | LINE | `tessellate(64)` → segments |
| Circle | **CIRCLE** | one `CircleInstance` (analytic) |
| **Arc** | **ARC** | one `ArcInstance` — **analytic**, angular-clamped ring |
| **Ellipse** | **ELLIPSE** | one `EllipseInstance` — **analytic** gradient-SDF |
| EllipseArc | LINE | tessellated (analytic elliptical-arc SDF not worth it yet) |
| **pattern Hatch** | LINE + CIRCLE | clipped lines / tile circles (cached) |
| **solid Hatch** | FILL | ear-clipped triangles, even-odd holes (cached) |
| **Wall** | FILL + LINE | poché fill + faces / insulation / centerline |
| **non-continuous linetype** (any stroked geom) | LINE | world-space dash walk |
| Text / Dimension / BlockRef / curved-wide Polyline | egui | already GPU via egui |

---

## 4. Where I diverged from the guide (and why)

### 4.1 Coordinates are **camera-relative f32** (the biggest change)
The guide keeps instances in raw world f32 and folds `world_offset` into
`view_matrix`. I instead fold the offset into the **instance coordinates**
(`(world + world_offset) as f32`, added in f64 first) and call
`view_matrix(w, h, scale, 0.0, 0.0)`.

**Why:** it fixes the guide's own edge-case #5 ("very large shapes near
extents → floating precision"). Instance magnitudes stay small near the
viewport, so f32 doesn't wobble when the drawing lives far from the origin.
Same reason game engines render camera-relative.

### 4.2 Analytic **arc and ellipse** (Phase 4 now, not "later")
The guide recommends tessellating arcs/ellipses to the line pipeline first,
SDF "for better quality later." Given §0 (tessellation is the real cost), I
went straight to analytic:

- **Arc** = the circle ring SDF plus an angular clamp in the fragment
  (`mod(angle − a0, TAU) > sweep → discard`). `sweep` is normalised to
  `(0, TAU]` CCW on the CPU so the test is a single compare; full circles skip
  the clamp. Butt ends, matching CAD.
- **Ellipse** = quad sized to the semi-major; fragment rotates into the ellipse
  frame, forms `f = (x/a)² + (y/b)² − 1`, and divides by `|∇f|` for a
  **first-order distance**. This is exact where the stroke is (f ≈ 0), which is
  the only place a 1-px ring lives. Elliptical **arcs** stay tessellated (the
  angular clamp in elliptical-parameter space is genuinely hard and ellipse-
  arcs are rare).

**Effect:** circles/arcs/ellipses are pixel-perfect at any zoom, cost one
instance each, and never facet — and they no longer inflate the line buffer.

### 4.3 Fill is a **triangle soup**, not `TriangleInstance`
The guide models fills as per-triangle instances (a 6-vertex quad instanced
per triangle — 2× the vertex work and an instancing setup for 1 triangle).
I use a plain per-vertex `FillVertex` buffer drawn with one non-instanced
`glDrawArrays(TRIANGLES)`. Simpler and cheaper.

### 4.4 Solid-hatch holes via even-odd **over-draw**, not stencil/earcut-holes
Each boundary loop is ear-clipped independently (`ear_clip`, concave-safe);
**even** loops fill with the hatch colour, **odd** loops over-draw in the
canvas bg — the same poor-man's even-odd the CPU path already used. No stencil
buffer dependency (egui's FBO may not have stencil bits) and no
triangulation-with-holes. Trade-off: the outer fill edge isn't anti-aliased
(the crisp edge comes from the boundary dobjects, which are drawn separately).

### 4.5 No `GpuDrawBatch` struct; `half_w` naming
I pass typed slices straight to `render()` instead of a batch struct
(functionally identical, less indirection), and `LineInstance` stores
`half_w` (world-space **half** stroke width) rather than `thickness` — same
value; the shader also enforces a ~1-px screen minimum so hairlines never
vanish.

### 4.6 One shared hatch **geometry generator**
Pattern-hatch line/circle geometry is produced by a single
`hatch_pattern_geometry` used by **both** the egui painter (CPU mode) and the
GPU path, so the two can't drift. Same for `wall_face_world_pts` (the screen
variant maps from it).

---

## 5. The performance work (beyond the guide)

### 5.1 Hatch geometry cache — the fix that made hatches usable
Hatch generation (pattern scanline-clipping, solid ear-clipping) is the
expensive part, and it was re-running **for every hatch, every frame** — dense
or numerous hatches froze the app. Now:

- Each hatch's **colour-less** world geometry (`segs / circs / fills / holes`)
  is generated **once** and cached by dobject handle
  (`hatch_cache: HashMap<Handle, HatchCacheEntry>`).
- The cache is cleared only when the document mutates (`gpu_dirty`), so
  **pan/zoom reuse it** — zero regeneration.
- **Colour is applied at draw time**, so selection/snap highlight still work
  off a cached entry.
- A **per-frame work budget** (`HATCH_GEN_WORK_BUDGET`, in generated
  primitives) means a huge paste/array of hatches builds over a few frames
  instead of freezing one; a small left-corner note shows how many remain.

This is the `§8.1` idea applied where it actually mattered (the expensive
CPU generation), rather than caching the cheap-to-rebuild line/circle buffers.

### 5.2 Heavy-scene safety (so nothing force-closes)
- **CPU per-frame draw budget** (`CPU_DRAW_BUDGET`): CPU mode stops after N
  dobjects/frame so the app stays responsive on huge drawings (with a note to
  switch to GPU/APX).
- **Array memory guard**: the array op caps by a **memory budget**
  (`~1.5 GB / size_of::<DObject>()`), not a raw count, and auto-suggests APX
  for large grids — because APX speeds *rendering*, not *storage*.

### 5.3 Effects (measured behaviour)
- Arcs/ellipses/circles: crisp at any zoom, ~1 instance each, no tessellation.
- Thousands of hatches: pan/zoom is cheap (cache); a big batch builds
  incrementally instead of hanging.
- Far-from-origin drawings: stable (no f32 wobble).
- Dashed linetypes: now correct in GPU mode (previously drawn solid).

---

## 6. Render-mode UX (related, shipped alongside)

- `RenderMode` is a single **3-way exclusive enum** `{ Cpu, Gpu, Apx }`.
- The status bar shows **three directly-selectable badges** (not a cycling
  toggle) — so on a heavy CPU frame you jump **straight** to GPU.
- **No forced auto-switch.** An earlier version yanked the mode to APX under
  load; that was disturbing, so heaviness is now only **advised** via a small
  **left-corner notice** (`building hatch cache…`, `press GPU/APX for speed`,
  `too heavy even for APX`). The CPU budget keeps the app responsive so the
  user can act on the advice themselves.

---

## 7. Dimension — intentionally NOT ported (the reason)

The guide (§2.10) proposes decomposing Dimension into `LineInstance` +
`TriangleInstance` + CPU text. I chose **not** to, and kept Dimension on the
egui painter. Reasons, in order of weight:

1. **A dimension is a text-bearing composite.** Its defining content is the
   measurement **text** (with formatting, tolerances, alignment, rotation).
   The guide itself keeps **Text on CPU** (§2.9) for exactly this reason — and
   a dimension is Text plus a few lines. Porting the lines but leaving the text
   on egui means every dimension still needs the egui path anyway, so you pay
   for **two** code paths per dimension for a partial win.

2. **It's already GPU-drawn (see §0).** Dimensions render through egui →
   `egui_glow` → GPU today. There is **no correctness gap** and **no "it's
   secretly on the CPU" problem** to fix. Moving the lines to our pipeline is
   pure architecture, not a user-visible improvement.

3. **No perf gain.** The whole point of the custom pipelines is avoiding
   per-frame CPU tessellation at scale. Dimensions are **rare** (you don't have
   a million of them), and each is cheap for egui. They are never the
   bottleneck, so there's nothing to speed up.

4. **Real complexity / risk.** `draw_dimension` handles extension lines, the
   dim line with a **text gap**, arrowhead geometry, and the various dim
   styles/variants. Re-deriving all of that as instances is bug-prone for zero
   payoff.

So Dimension sits with **Text and BlockRef** as "already GPU via egui, leave
it." This is a deliberate scope call, not an oversight.

**If you still want it:** the tractable slice is routing only the non-text
parts — extension lines + dim line → LINE pipeline, arrowheads → FILL
(triangles) — while text stays on egui. That's a clean follow-up if a profile
ever shows dimensions mattering; today it doesn't.

---

## 8. Known limitations / honest caveats

- **Solid-hatch holes** use bg-colour over-draw (matches the CPU path); correct
  only while the canvas bg matches the over-draw colour. Real even-odd (stencil
  or triangulation-with-holes) is a future refinement.
- **Ellipse SDF** is a first-order (gradient) approximation — visually exact at
  the stroke, not a true global distance field. Fine for a thin ring.
- **Hatch cache** invalidates on `gpu_dirty`. If an edit mutated a hatch's
  boundary without setting `gpu_dirty`, the hatch could show stale until the
  next `gpu_dirty` (most edit ops set it).
- **EllipseArc**, **curved/wide Polyline** still tessellate / use egui.
- **Wall centerline** draws solid on GPU (dashed centerline waits on the
  linetype-on-GPU path being applied there too).

---

## 9. File map

| Area | Where |
|---|---|
| Pipelines, instance types, shaders, `render()` | `cad_app/src/gpu.rs` |
| `RenderMode::Gpu` branch (emit per shape) | `cad_app/src/app.rs` |
| Hatch cache + `build_hatch_cache_entry` + `HatchCacheEntry` | `cad_app/src/app.rs` |
| `ear_clip`, `point_in_tri`, `pack_rgba` (fill helpers) | `cad_app/src/app.rs` |
| `dash_world_segments`, `effective_dash_pattern` (linetype) | `cad_app/src/app.rs` |
| `wall_face_world_pts` (world-space wall faces) | `cad_app/src/app.rs` |
| `hatch_pattern_geometry` (shared CPU/GPU generator) | `cad_app/src/app.rs` |
| render-mode badges, left-corner notice, CPU/array budgets | `cad_app/src/app.rs` |
