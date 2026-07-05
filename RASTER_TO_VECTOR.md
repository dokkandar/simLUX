# Raster → Vector — design spec

Convert old scanned drawings into native DObjects. **Not** a one-shot auto-
vectorizer — a **human-in-the-loop midware**: the app proposes, the human
confirms, and the raster is **peeled layer by layer** so each step is easy.

> Status: **design + scaffold.** `cad_raster` crate exists with the layer model
> + adjustment + analyzer + trace-dispatch skeleton. Engines + editor UI owed.

## Three pillars

1. **Raster-layer buffer (Photoshop-style)** — prep + isolate, non-destructive.
2. **Human-marked semantic peel** — text → dims → furniture → structure → user.
3. **Type-aware convert → CAD layers** — each asset type uses its own engine.

```
load → RASTER LAYERS (mono / contrast / threshold / color-isolate)
     → MARK asset (detectors assist, scoped to a layer)
       → asset becomes a raster layer (mask)
         → CONVERT (type-aware engine) → DObjects on the mapped CAD layer
           → subtract asset from the working raster → repeat (raster gets simpler)
```

## Two distinct layer spaces
- **Raster layers** — image-editing workspace (the buffer). Prep/isolate here.
- **CAD layers** — the vector output. Converted assets are pushed here.
- Bridge = one action: convert a raster asset layer → DObjects on a CAD layer.
  A raster asset layer maps to a named CAD layer (`walls` → `WALLS`, etc.).

## Raster-layer model (`RasterDoc`)
Ordered stack composited top→bottom into a **working raster** (detection +
marking run on this):

| Layer type | Role |
|---|---|
| **Base** | the loaded scan (read-only original) |
| **Adjustment** | non-destructive image op applied to the composite |
| **Mask / asset** | a painted/detected region isolating ONE semantic asset |

Each layer: visibility · opacity · name · reorder. The peel loop = layer ops:
mark → new asset layer → convert → CAD layer → hide/subtract → repeat.

## Adjustments (why each helps)
- **Mono / grayscale** — kills colour noise → clean threshold/edges.
- **Contrast / brightness / levels** — bring up faint scans, or dim the base so
  marks/overlays pop.
- **Threshold / binarize** — crisp ink-vs-paper for centerline tracing.
- **Colour-range isolation** — biggest free win: colour-coded drawings (red
  dims, black walls) segment instantly by colour, no AI.
- **Denoise / despeckle** — drop scanner specks before tracing.
All non-destructive (adjustment stack); base never lost.

## Peel order (each removes complexity for the next)
1. **Text & annotations** — human disambiguates text vs dimension vs label →
   OCR → `Text`. Subtract.
2. **Dimensions** — extension lines + arrowheads + the OCR'd value → `Dimension`.
3. **Furniture / fixtures / symbols** — regions + repeat-symbol clusters →
   `Polyline`/`Hatch`/`BlockRef`.
4. **Structure (walls)** — parallel double-lines → centerline → `Wall`; singles
   → `Line/Polyline`.
5. **User-defined / leftovers** — human marks region + picks an engine.

## Stage 2 — type-aware extraction (engine follows the human's TAG, not global)
Outline AND centerline are both correct — for different content. The human's
layer type selects the engine, run on the masked sub-raster only:

| Layer (tagged) | Engine | Output |
|---|---|---|
| Text | OCR (no tracing) | `Text` |
| Dimension | extension-line + arrowhead recognition + link OCR'd value | `Dimension` |
| Furniture / fills | **outline trace** + curve-fit; repeat → template-match → block | `Polyline`/`Hatch`/`BlockRef` |
| Walls / structure | **centerline trace** (Zhang-Suen thin) + double-line→wall | `Wall`/`Line` |
| Generic line-art | centerline + Hough line/circle + least-squares arc fit | `Line`/`Arc`/`Circle`/`Spline` |

Then **optimize/fit** per layer (simplify → corner-split → primitive fit →
regularize; reuse `join_geoms` + `cad_nurbs`), with per-layer tolerances.

## Marker tools (the "help the app" layer)
- Scope: box / lasso / magic-wand (flood-fill) / paint-mask brush.
- Type stamp: tag region (text/dim/furniture/wall/line/ignore) → scopes the detector.
- Detect-assist overlay: app highlights candidates; human bulk-accepts/rejects/reclassifies.
- Correct: nudge endpoints, merge/split primitives.
- **Calibrate scale** (2 points + real distance) → vectors come out to scale.

## Analyzer (convertibility pre-check)
Score the working raster before tracing: colour count, edge density, stroke-width
consistency, connected-component shape → class (`line-art`/`filled-regions`/
`photo→not suitable`) + confidence. Gates the workflow.

## Open-source (pure-Rust, permissive — verify at integration)
- **image** (MIT/Apache) — decode + grayscale + pixel access. **dep** (brought in).
- **imageproc** (MIT) — threshold, contrast, Canny/Sobel, Hough, morphology,
  connected-components. dep or port-oracle (add when engines need it).
- **vtracer / visioncortex** (MIT) — outline trace → Béziers. port-oracle.
- **kurbo** (Apache/MIT) — curve fitting (the optimizer). dep when fitting lands.
- **ocrs + rten** (permissive) — pure-Rust OCR for text. dep for the text engine.
- Reject: potrace/autotrace (GPL), OpenCV (C++).

## Crate split (team-parallel)
- **`cad_raster`** (deps `cad_kernel`, `image`): `RasterDoc` + compositing +
  adjustments + analyzer + detection/trace engines. Pure, unit-testable, no UI.
- **`cad_app`**: the editor dialog — raster **layers panel**, canvas, adjustment
  controls, mask brush, convert→CAD-layer action. Isolated like the Block Editor.

## Slices
1. **Scaffold** (DONE): crate + `RasterDoc` layer model + adjustments + analyzer
   + `AssetKind`→engine dispatch skeleton + `image` dep.
2. Adjustments real (grayscale/contrast/threshold/colour-isolate) + analyzer real.
3. Centerline trace (thin + skeleton walk) + line/arc/circle fit → DObjects.
4. Editor UI in cad_app (layers panel + canvas + marks + convert action).
5. Per-type engines: OCR (text), dimension recognition, furniture/template, walls.
