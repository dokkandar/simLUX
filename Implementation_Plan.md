# RUST_CAD — Critical Dobjects + Draw-Tool Dependency Plan

> **Naming reminder**: in RUST_CAD the drawing entity is always called a
> **Dobject** — never "entity". This applies to docs, code, panels, UI
> labels. (`Entity Info` panel → `Dobject Info`. `RS_Line` → `DobjectLine`
> conceptually; in code it's `Line` inside the `Geom` enum, wrapped by the
> `DObject` struct.)

This document is the **compass for the next slices** — what to build, in
what order, and why each thing waits for the thing before it. It is the
companion to `ROADMAP.md` (high-level slices) and `Audit.html` (parity
checkpoint). Update at the start of every slice that introduces a new
Dobject type or draw tool.

---

## Part 1 — Critical Dobjects in priority order

Filtered for **what a real drawing needs**. Parabola, Hyperbola, LC_Rect,
and other LibreCAD-specific extras are dropped — Spline (NURBS) covers
every smooth curve we'll realistically need.

### ● Done (Slices A–J)

| # | Dobject | Variant | Note |
|---|---------|---------|------|
| 1 | DobjectLine       | `Geom::Line`       | finite segment |
| 2 | DobjectCircle     | `Geom::Circle`     | full circle |
| 3 | DobjectArc        | `Geom::Arc`        | partial circular arc |
| 4 | DobjectEllipse    | `Geom::Ellipse`    | full ellipse |
| 5 | DobjectEllipseArc | `Geom::EllipseArc` | partial elliptical arc |
| 6 | DobjectPoint      | `Geom::Point`      | POINT entity with PDMODE-style + PDSIZE |
| 7 | DobjectPolyline   | `Geom::Polyline`   | LWPOLYLINE; bulge stored, rendered straight today |

### Tier 1 — Critical for any real drawing

Build these next. Most drawings *cannot* be authored without them.

| # | Dobject | Depends on | Why critical |
|---|---------|------------|--------------|
| 8  | DobjectText      | `TextStyleTable` on Document | Single-line annotation; gates Dim* and BlockRef attribute rendering. |
| 9  | DobjectMText     | DobjectText infra | Multi-line annotation with formatting (`\f`, `\H`, `\C`, `\P`). |
| 10 | DobjectHatch     | `PatternTable` on Document + boundary loop representation | Filled regions; required for arch/mech drawings. |
| 11 | DobjectDimLinear | `DimStyleTable` on Document + DobjectText | First dimension type — validates the whole dim pipeline. |

### Tier 2 — Standard CAD essentials

Big breadth gain vs LibreCAD; needed for serious interop.

| # | Dobject | Depends on | Why important |
|---|---------|------------|---------------|
| 12 | DobjectBlockRef + BlockTable    | TextStyleTable (so attributes render correctly) | INSERT references; biggest architectural step. |
| 13 | DobjectSpline (NURBS)           | de Boor evaluator + Newton intersection | Smooth curves; **replaces parabola / hyperbola / LC_Rect**. |
| 14 | DobjectDimAligned               | DimLinear infra | Linear dim along arbitrary axis. |
| 15 | DobjectDimAngular               | DimLinear infra + angle math | Angle dim. |
| 16 | DobjectDimRadial                | DimLinear infra | Radius dim on circles/arcs. |
| 17 | DobjectDimDiameter              | DimLinear infra | Diameter dim on circles. |
| 18 | DobjectDimArc                   | DimLinear infra | Arc-length dim. |
| 19 | DobjectDimOrdinate              | DimLinear infra | Ordinate (X / Y) dim. |
| 20 | DobjectLeader / DobjectMLeader  | DobjectText + arrow shapes | Annotation pointers. |

### Tier 3 — Useful but not blocking

Land when need arises; not gating anyone.

| # | Dobject | Note |
|---|---------|------|
| 21 | DobjectRasterImage | PDF / PNG / JPG / TIFF inserts. Needs an image-decoder crate (`image`). |
| 22 | DobjectRay         | Infinite half-line; trivial geometry, one new Geom variant. |
| 23 | DobjectXline       | Construction line, infinite both directions; trivial. |
| 24 | DobjectSolid2D     | Filled triangle / quad. Trivial. |
| 25 | DobjectWipeout     | Polygon mask. Depends on Solid2D-style polygon fill. |

### ◌ Deferred — explicitly OUT OF SCOPE

These are LibreCAD-extension entities, AutoCAD legacy, or 3D-only.
They are **not gaps**; they are decisions.

- **Spline-replaceable**: DobjectParabola, DobjectHyperbola, LC_Rect — Spline (Tier 2 #13) covers all of these.
- **3D-only**: SubDMesh, NurbsSurface, Solid3D, Helix, 3DFace, PolyFaceMesh, PolygonMesh.
- **Render-only**: Light, Camera.
- **Niche legacy**: Shape, Trace, Field.
- **Niche modern**: Region (Spline-bounded; redundant with Hatch+Spline), MLine (multi-parallel line), Underlay (PDF/DGN reference), GeoData (GIS projection).
- **Extreme niche**: DobjectTolerance (GD&T feature control frame) — revisit only on explicit demand.

---

## Part 2 — Draw-tool dependency-ordered plan

LibreCAD ships ~106 separate draw actions. We don't need 106 — many are
duplicates of the same Dobject construction expressed as different click
flows. **Implementation rule**: each phase below must complete before the
next begins. Within a phase, items are roughly equal-priority.

### ● Done (Slice E + earlier)

| Tool | Dobject | Construction |
|------|---------|-------------|
| Line              | Line      | click 2 points |
| Circle            | Circle    | click centre, click radius point |
| Arc — 3 methods   | Arc       | ThreePoints, StartCenterEnd, CenterStartEnd |
| Ellipse           | Ellipse   | centre + major-end + minor-side |
| EllipseArc        | EllipseArc| 5-click flow (centre, major, minor, start, end) |
| Point             | Point     | single click |
| Polyline          | Polyline  | click N points, Enter / `close` to finish |
| Arc — `arccr`     | Arc       | command-line only — chord + radius |
| Arc — `arccl`     | Arc       | command-line only — chord + arc length |

### Phase 1 — Quick wins: Polyline-derived + analytical primitives

**No new Dobject types.** Pure UX expansion on what's already there.
Each is < 1 day's work; together they double the toolbar surface.

| # | Tool | Builds on | Notes |
|---|------|-----------|-------|
| 1 | Rectangle              | Polyline               | 2-corner click → 4-vertex closed Polyline |
| 2 | Polygon (regular N-gon)| Polyline               | Centre + circumradius click + N count → closed Polyline |
| 3 | Circle 2P              | Circle                 | Diameter endpoints → centre = midpoint, radius = half-distance |
| 4 | Circle 3P              | Circle                 | Three points → analytical solver (same as `arc3p` math) |
| 5 | Circle CR (named)      | Circle                 | Explicit "centre + radius value" command — `circle cx,cy r` already in CLI; surface to toolbar |
| 6 | Arc TTR (tangent-tangent-radius) | Arc          | Needs tangent solver — leverages existing snap PER/TAN math |

### Phase 2 — Constraint-based line variants

**No new Dobject types.** All produce Line; differ only in constraint.
Most can ship as modifiers on the existing Line tool (Shift = ortho,
typed angle = angle-relative) instead of separate tools.

| # | Tool | Builds on | Notes |
|---|------|-----------|-------|
| 7  | Line ortho (H / V)     | Line                 | Shift modifier during draw, or per-axis tool |
| 8  | Line angle-relative    | Line                 | Type `@d<a` from current point — distance + angle |
| 9  | Line parallel          | Line + snap NEA      | Click existing line → click offset point |
| 10 | Line perpendicular     | Line + snap PER      | Click existing line → click endpoint |
| 11 | Line tangent           | Line + snap TAN      | Click circle/arc → click endpoint |
| 12 | Line bisector          | Line + angle math    | Click two lines → angle bisector |
| 13 | Arc 2P + radius        | Arc + chord math     | Existing `arccr` math, click-driven |
| 14 | Arc 2P + length        | Arc + chord math     | Existing `arccl` math, click-driven |
| 15 | Arc 2P + angle / height| Arc                  | LibreCAD variants — same construction with different inputs |

### Phase 3 — Construction lines (tiny new Dobject types)

| # | Tool | New Dobject | Effort |
|---|------|-------------|--------|
| 16 | Ray (one-end infinite)   | DobjectRay (Tier 3 #22)   | 1 day — new Geom variant + 2-click draw |
| 17 | Xline (both-end infinite)| DobjectXline (Tier 3 #23) | 1 day — new Geom variant + 2-click draw |

### Phase 4 — Text foundation

**Gate**: `TextStyleTable` must land on `Document` first as its own kernel
slice. Then both Dobjects + both tools.

| # | Tool | Dobject (must land first) | Notes |
|---|------|---------------------------|-------|
| 18 | Text                | DobjectText (Tier 1 #8)  | Single-line; insertion point + height + rotation |
| 19 | MText               | DobjectMText (Tier 1 #9) | Reference rectangle + formatting code editor |

### Phase 5 — Dimensions

**Gate**: `DimStyleTable` + DobjectText complete.
**Strategy**: DimLinear first to debug the whole dim pipeline (extension
lines, dim line, arrowheads, text placement, override text). Other dim
variants then reuse 90 % of the same infrastructure.

| # | Tool | Dobject | Notes |
|---|------|---------|-------|
| 20 | Dim — Linear        | DobjectDimLinear (Tier 1 #11) | Validates whole pipeline |
| 21 | Dim — Aligned       | DobjectDimAligned (Tier 2 #14) | Reuses DimLinear infra |
| 22 | Dim — Angular       | DobjectDimAngular (Tier 2 #15) | Add angle math |
| 23 | Dim — Radial        | DobjectDimRadial (Tier 2 #16) | Centre + edge click |
| 24 | Dim — Diameter      | DobjectDimDiameter (Tier 2 #17) | Two-side variant of Radial |
| 25 | Dim — Arc length    | DobjectDimArc (Tier 2 #18) | Sweep-aware |
| 26 | Dim — Ordinate      | DobjectDimOrdinate (Tier 2 #19) | X or Y from UCS origin |
| 27 | Dim — Baseline      | DobjectDimLinear chain | Per-AutoCAD: extends previous Linear/Aligned |

### Phase 6 — Hatch

**Gate**: `PatternTable` (predefined patterns shipped as data) + boundary
detection algorithm.

| # | Tool | Dobject | Notes |
|---|------|---------|-------|
| 28 | Hatch (predefined pattern) | DobjectHatch (Tier 1 #10) | ANSI31 etc. + scale + angle |
| 29 | Hatch (solid fill)         | DobjectHatch                | Same Dobject, solid flag |
| 30 | Hatch (user-defined / parallel-line) | DobjectHatch      | Spacing + angle |

### Phase 7 — Annotation arrows

**Gate**: DobjectText.

| # | Tool | Dobject | Notes |
|---|------|---------|-------|
| 31 | Leader               | DobjectLeader (Tier 2 #20)  | N-segment polyline + arrowhead + text annotation handle |
| 32 | MLeader              | DobjectMLeader (Tier 2 #20) | Multi-leader with shared content block |

### Phase 8 — Blocks

**Gate**: Phases 4 + 5 done — blocks frequently contain text & dims, and
those must render *inside* an INSERT.

| # | Tool | Dobject | Notes |
|---|------|---------|-------|
| 33 | Block — Make       | BlockTable entry        | Select set of Dobjects → name → base point |
| 34 | Block — Insert     | DobjectBlockRef (Tier 2 #12) | Scale, rotation, attribute-fill prompts |
| 35 | Block — Edit       | BlockTable edit         | In-place block editor (sub-document context) |
| 36 | Image — Insert     | DobjectRasterImage (Tier 3 #21) | Independent of Block but same UX shape |

### Phase 9 — Splines (Tier 2 #13)

**Gate**: design decision — fit-point input vs control-point input vs
both. NURBS math is heavy; want one solid implementation, not three.

| # | Tool | Dobject | Notes |
|---|------|---------|-------|
| 37 | Spline — fit points     | DobjectSpline (Tier 2 #13) | Click points, Enter; auto-solve for control points |
| 38 | Spline — control points | DobjectSpline              | Click control points directly (advanced) |
| 39 | Spline intersections    | (math, not a tool)         | Add Geom::Spline arms to `intersect()` — multi-seed Newton |

### Phase 10 — Specialty fills

Low priority — land when concrete drawings need them.

| # | Tool | Dobject |
|---|------|---------|
| 40 | Solid2D (filled triangle / quad) | DobjectSolid2D (Tier 3 #24) |
| 41 | Wipeout                          | DobjectWipeout (Tier 3 #25) |

### Out of scope (Spline supersedes)

These LibreCAD draw-tool families are NOT planned:

- Line freehand / snake (rarely used; trivially polyline)
- Hyperbola FP / FD / dual-branch / FF
- Parabola FD / 4-point / dual-axis / FF
- Circle "by-arc" (degenerate)
- GD&T Feature Control Frame
- LC_Rect (use Rectangle from Phase 1)

---

## Phase summary

| Phase | New Dobjects | New tools | Approx. dev sessions |
|-------|-------------|-----------|-----------------------|
| 1 — Polyline-derived + analytical | 0 | 6 | 3–4 |
| 2 — Constraint lines / arcs       | 0 | 9 | 3–4 |
| 3 — Construction lines             | 2 | 2 | 1–2 |
| 4 — Text foundation                | 2 + TextStyleTable | 2 | 4–6 |
| 5 — Dimensions                     | 8 + DimStyleTable | 8 | 8–12 |
| 6 — Hatch                          | 1 + PatternTable | 3 | 4–6 |
| 7 — Annotation arrows              | 2 | 2 | 3–4 |
| 8 — Blocks                         | 1 + BlockTable | 4 | 6–8 |
| 9 — Splines                        | 1 + NURBS math | 3 | 6–8 |
| 10 — Specialty fills               | 2 | 2 | 2–3 |

**Total**: ~24 new Dobjects (vs the 13 we already have); ~41 new draw
tools (vs the 8 we already have).

**Discussion items for the engineering pair**:

1. **Phase 1 ordering**: are Rectangle / Polygon worth shipping before
   Circle 2P / 3P, or is the inverse better (more analytical, less UX)?
2. **Phase 4 vs Phase 6**: does Hatch (boundary detection — purely
   geometric) wait for Text, or can it leapfrog? My current ordering
   says Text first because most hatched regions carry annotation, but
   the slices are independent in code.
3. **Phase 9 NURBS choice**: do we own the implementation, or vendor
   (`nurbs` crate, license review needed)?
4. **DimStyleTable scope**: AutoCAD's DIMSTYLE has 70+ knobs. Ship a
   minimal one (text style, arrow size, extension offsets, decimal
   places) and grow from there — or model the full surface up front?
5. **Snap PER/TAN already implemented**: Phase 2's parallel /
   perpendicular / tangent line tools can lean on the existing snap
   engine instead of re-deriving math. Confirm this is the path.
