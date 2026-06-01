# Dobject Property Model — Per-Type Field Catalog

The **in-memory shape** of every Dobject type RUST_CAD will eventually support.
Separate from the other two reference docs:

| Doc | Scope |
|-----|-------|
| `Variables.md` | User-settable SYSVARS (`SpTGSZ`, `GrpEnb`, …) — what the user *configures* |
| `Dobject_DXF.md` | DXF I/O group-code dictionary (`dobLayer` → code 8, …) — what we *serialize* |
| **`Dobject_Properties.md`** *(this file)* | Rust struct shape — what we *hold in memory* |

The three are deliberately allowed to drift in naming: a Rust field
`start_angle` may serialize to DXF group code 50 (`dobAngle50`), and the
user may tune its display via SYSVAR `RtDsp`. Each doc speaks its own
audience's language.

## Status legend

| Status | Meaning |
|--------|---------|
| `●` | **Modeled** — Rust struct exists in `cad_kernel/src/geom.rs`, field present |
| `◐` | **Partial** — struct exists but this specific field not stored yet (often *derived* in code) |
| `○` | **Planned** — entity type not yet defined in code |

## Naming rule

- **Conceptual / external name** (DXF, docs, UI, your lists): `DobjectLine`, `DobjectCircle`, …
- **Rust struct name** (in `geom.rs`): `Line`, `Circle`, … nested inside the `DObject` enum variants
- **Rust field names**: `snake_case` — already used: `start_angle`, `sweep_angle`, `center`, `major`, `ratio`, `start_param`, `sweep_param`
- **No cryptic prefixes** here. (Cryptic short names are *only* for SYSVAR fields in `UserEnv`; see `feedback_rust_cad_settings_naming`.)

## Common properties (all Dobjects)

Common fields live on the `DObject` struct and its `style: Style` field
(see [`cad_kernel::style::Style`](cad_kernel/src/style.rs) and
[`cad_kernel::dobject::DObject`](cad_kernel/src/dobject.rs)).

| Property | Rust Field | Type | DXF Code | Status |
|----------|-----------|------|----------|--------|
| Handle | `dobject.handle` | `u64` (hex string on export) | 5 | ● |
| Owner | `owner_id` | `Option<u64>` (handle of containing block/layout) | 330 | ○ |
| Layer | `style.layer` | `LayerId` (u32 into `Document.layers`) | 8 | ● |
| Color | `style.color` | `Color` enum (ByLayer / ByBlock / Aci(u8) / TrueColor(u32)) | 62 / 420 | ● |
| Linetype | `style.linetype` | `LinetypeId` (u32 into `Document.linetypes`) | 6 | ● *(stored; renderer doesn't dash yet)* |
| LinetypeScale | `style.linetype_scale` | `f32` | 48 | ● *(stored; renderer doesn't use yet)* |
| Lineweight | `style.lineweight` | `Lineweight` (ByLayer / ByBlock / Default / Custom(mm)) | 370 | ● *(stored; renderer always uses 1.6 px today)* |
| Visible | `style.visible` | `bool` | 60 | ● *(renderer honours; combined with `layers.renders`)* |
| Transparency | `transparency` | `u8` (0–90, where ByLayer = sentinel) | 440 | ○ |
| PlotStyleName | `plot_style` | `String` | 390 | ○ |
| Material | `material` | `String` | 347 | ○ |
| ExtendedData | `xdata` | `Vec<XData>` | 1000–1071 | ○ |
| ExtendedDictionary | `xdict` | `Option<u64>` | 102 | ○ |
| AnnotationScale | `annotation_scale` | `Option<f64>` | — | ○ |
| IsAnnotative | `is_annotative` | `bool` | — | ○ |

> **As of the property-foundation slice**: every `DObject` carries a `Style`
> (layer + color + linetype + linetype_scale + lineweight + visibility) and a
> stable `handle`. The renderer resolves `Color::ByLayer` through
> `Document.layers`. Layer visibility + per-Dobject `style.visible` both gate
> render. The Layer / Pen panels (Slices B & C) will surface these to the
> user; today they default sensibly (everything on layer "0", color ByLayer,
> linetype Continuous, lineweight ByLayer, visible).

> The geometry side lives in `Geom` (Line / Circle / Arc / Ellipse /
> EllipseArc). Future Dobject types — Polyline, Text, MText, Hatch,
> BlockRef, Dim*, etc. — slot in as new `Geom` variants; they inherit the
> full Common property set for free because Style lives on the outer
> `DObject`, not on each variant.

---

# Implemented types

These are defined in [`cad_kernel/src/geom.rs`](cad_kernel/src/geom.rs).

## DobjectLine — `Line` struct ([geom.rs:6](cad_kernel/src/geom.rs#L6))

| Your name | Rust field | Type | Status | Note |
|-----------|-----------|------|--------|------|
| StartPoint | `a` | `Vec2` | ● | 2D today (no Z) |
| EndPoint | `b` | `Vec2` | ● | 2D today (no Z) |
| Angle | *derived* | `f64` (rad) | ◐ | `(b - a).angle()` — not stored |
| Length | *derived* | `f64` | ◐ | `(b - a).len()` — not stored |
| DeltaX, DeltaY | *derived* | `f64` | ◐ | `b.x - a.x`, `b.y - a.y` |
| DeltaZ | — | — | ○ | No Z yet |
| + Common | (all `○`) | | | |

> **FUTURE TASK — confirmed deferred** (user said "flag for future"):
> rename `Line.a` / `Line.b` → `start` / `end`. The single-letter names
> are terse to the point of confusing in non-trivial code (e.g.,
> `Arc::endpoints` uses local `s, e` for start/end angle, which would
> shadow). Wired everywhere today, so the rename is a sweeping but
> mechanical change — left for a dedicated cleanup pass, no urgency.
> Tracked in memory `project_rust_cad_future_cleanups.md`.

## DobjectCircle — `Circle` struct ([geom.rs:9](cad_kernel/src/geom.rs#L9))

| Your name | Rust field | Type | Status | Note |
|-----------|-----------|------|--------|------|
| Center | `center` | `Vec2` | ● | |
| Radius | `radius` | `f64` | ● | |
| Circumference | *derived* | `f64` | ◐ | `2π · radius` |
| Area | *derived* | `f64` | ◐ | `π · radius²` |
| Normal | — | `Vec3` (extrusion) | ○ | Implied `(0, 0, 1)` in 2D — add when 3D lands |
| + Common | (all `○`) | | | |

## DobjectArc — `Arc` struct ([geom.rs:12](cad_kernel/src/geom.rs#L12))

| Your name | Rust field | Type | Status | Note |
|-----------|-----------|------|--------|------|
| Center | `center` | `Vec2` | ● | |
| Radius | `radius` | `f64` | ● | |
| StartAngle | `start_angle` | `f64` (rad, normalized `[0, 2π)`) | ● | |
| EndAngle | *derived* | `f64` | ◐ | `start_angle + sweep_angle` |
| SweepAngle | `sweep_angle` | `f64` (rad, `(0, 2π]`) | ● | We store sweep, not end — avoids wrap-around edge cases |
| StartPoint, EndPoint | *derived* | `(Vec2, Vec2)` | ◐ | `Arc::endpoints()` |
| TotalAngle | = `sweep_angle` | | ● | Same thing under a different name |
| Length | *derived* | `f64` | ◐ | `radius · sweep_angle` |
| Normal | — | `Vec3` | ○ | Implied 2D |
| + Common | (all `○`) | | | |

## DobjectEllipse — `Ellipse` struct ([geom.rs:43](cad_kernel/src/geom.rs#L43))

| Your name | Rust field | Type | Status | Note |
|-----------|-----------|------|--------|------|
| Center | `center` | `Vec2` | ● | |
| MajorAxis | `major` | `Vec2` | ● | Vector from center to ellipse endpoint — encodes both rotation (direction) and semi-major length (magnitude) |
| MinorAxisRatio | `ratio` | `f64` (in `(0, 1]`) | ● | Semi-minor / semi-major |
| StartAngle | — | — | ○ | Full ellipse — see `EllipseArc` below for partial |
| EndAngle | — | — | ○ | Same |
| IsClosed | always `true` for `Ellipse` | | ● | Implied by type — full ellipse |
| + Common | (all `○`) | | | |

## DobjectEllipseArc — `EllipseArc` struct ([geom.rs:54](cad_kernel/src/geom.rs#L54))

> **Decided: follow RUST_CAD split** (user-confirmed). Where AutoCAD's
> source list combines "partial ellipse" under `DobjectEllipse` with an
> `IsClosed` boolean, RUST_CAD splits into two `DObject` variants
> (`Ellipse` and `EllipseArc`). Reason: full-ellipse algorithms don't pay
> the cost of a sweep-range check on every call. Externally we can still
> report them as a single "ellipse" concept (e.g., DXF export emits one
> `ELLIPSE` entity for both, with the closed flag implied by the variant).

| Your name (when partial) | Rust field | Type | Status | Note |
|--------------------------|-----------|------|--------|------|
| (the underlying ellipse) | `ellipse` | `Ellipse` | ● | Inline, not a handle |
| StartAngle | `start_param` | `f64` (rad) | ● | **Parameter** angle, NOT geometric angle — differs for non-circular ellipse |
| EndAngle | *derived* | `f64` | ◐ | `start_param + sweep_param` |
| SweepParam | `sweep_param` | `f64` (rad, `(0, 2π]`) | ● | |
| IsClosed | always `false` for `EllipseArc` | | ● | Implied by type — partial |
| + Common | (all `○`) | | | |

---

# Planned types (not in code yet)

Suggested Rust struct names and field shapes. All status `○`.
Listed roughly in order of likely implementation priority.

## DobjectPoint — `Point` struct (● modeled)

AutoCAD `POINT` entity — a single marker point.

| Your name | Suggested Rust field | Type | Note |
|-----------|---------------------|------|------|
| Location | `location` | `Vec2` (or `Vec3` when 3D) | |
| PointStyle | `style` | `u8` (0–99, PDMODE values) | Per-document setting in AutoCAD (`PDMODE` SYSVAR); per-entity here for flexibility |
| PointSize | `size` | `f64` | PDSIZE in AutoCAD |

## DobjectPolyline — `Polyline` struct (● modeled — straight segments only, bulges accepted but not yet rendered)

2D lightweight polyline.

| Your name | Suggested Rust field | Type | Note |
|-----------|---------------------|------|------|
| Vertices | `vertices` | `Vec<PolyVertex>` | Each vertex: `pos: Vec2, bulge: f64, start_width: f64, end_width: f64` |
| Elevation | `elevation` | `f64` | Z value for all vertices |
| Width | `constant_width` | `Option<f64>` | `None` = use per-vertex widths |
| IsClosed | `closed` | `bool` | |
| LinetypeGeneration | `continuous_linetype` | `bool` | Linetype runs through whole polyline vs per-segment |
| Area | *derived* | `f64` | When closed |
| Length | *derived* | `f64` | |

> Bulge encodes arc segments — `tan(angle/4)`. Standard DXF LWPOLYLINE convention.

## Dobject3DPolyline — `Polyline3D` struct (planned)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| Vertices | `vertices` | `Vec<Vec3>` |
| IsClosed | `closed` | `bool` |
| Length | *derived* | `f64` |

## DobjectSpline — `Spline` struct (planned, deferred)

> **Project note**: spline is deliberately deferred indefinitely per
> earlier discussion. The math (NURBS evaluation + de Boor + knot
> insertion) is heavy and intersection with ellipse/arc adds combinatorial
> complexity. We'll revisit when there's a concrete need.

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| Degree | `degree` | `u8` |
| FitPoints | `fit_points` | `Vec<Vec3>` |
| ControlPoints | `control_points` | `Vec<Vec3>` |
| Knots | `knots` | `Vec<f64>` |
| StartTangent | `start_tangent` | `Option<Vec3>` |
| EndTangent | `end_tangent` | `Option<Vec3>` |
| IsClosed | `closed` | `bool` |
| IsPeriodic | `periodic` | `bool` |

## DobjectText — `Text` struct (planned, single-line)

| Your name | Suggested Rust field | Type | Note |
|-----------|---------------------|------|------|
| InsertionPoint | `insertion` | `Vec2` | |
| TextString | `text` | `String` | |
| Height | `height` | `f64` | |
| Rotation | `rotation` | `f64` (rad) | |
| ObliqueAngle | `oblique` | `f64` (rad) | Italics |
| WidthFactor | `width_factor` | `f64` | |
| HorizontalAlignment | `h_align` | enum `HAlign` (Left/Center/Right/Align/Fit/Middle) | |
| VerticalAlignment | `v_align` | enum `VAlign` (Baseline/Bottom/Middle/Top) | |
| StyleName | `style` | `String` | |

## DobjectMText — `MText` struct (planned, multi-line)

| Your name | Suggested Rust field | Type | Note |
|-----------|---------------------|------|------|
| Location | `insertion` | `Vec2` | |
| TextString | `text` | `String` | With MText formatting codes (`\f`, `\H`, `\C`, `\P`) |
| TextHeight | `height` | `f64` | |
| Width | `width` | `f64` | Reference rectangle width |
| Rotation | `rotation` | `f64` (rad) | |
| AttachmentPoint | `attachment` | enum `Attachment` (9 corners + center) | |
| LineSpacingFactor | `line_spacing` | `f64` | |
| LineSpacingStyle | `spacing_style` | enum `SpacingStyle` (AtLeast / Exactly) | |
| BackgroundFill | `bg_fill` | `Option<BgFill>` | `{ color, offset, transparency }` |
| StyleName | `style` | `String` | |

## DobjectHatch — `Hatch` struct (planned)

| Your name | Suggested Rust field | Type | Note |
|-----------|---------------------|------|------|
| PatternName | `pattern_name` | `String` | e.g. `"ANSI31"` |
| PatternType | `pattern_type` | enum `PatternType` (User / Predefined / Custom) | |
| Angle | `angle` | `f64` (rad) | |
| Scale | `scale` | `f64` | |
| Spacing | `spacing` | `f64` | User-defined only |
| DoubleHatch | `double` | `bool` | Crosshatch |
| Associative | `associative` | `bool` | Linked to boundary entities |
| BoundaryPaths | `boundaries` | `Vec<BoundaryLoop>` | Each loop: vec of boundary edges (line / arc / ellipse / spline / poly) |
| IsSolid | `solid_fill` | `bool` | |
| Color | (use Common `color`) | | When `solid_fill` |
| Transparency | (use Common `transparency`) | | |

## DobjectBlockRef — `BlockRef` struct (planned, INSERT)

> **Dependency**: needs a block-definition table (`BlockRecord`) before
> references make sense. Plan: add `BlockTable` to the document model
> alongside `Vec<DObject>`.

| Your name | Suggested Rust field | Type | Note |
|-----------|---------------------|------|------|
| BlockName | `block_name` | `String` | Lookup key into BlockTable |
| InsertionPoint | `insertion` | `Vec2` | |
| ScaleX, ScaleY, ScaleZ | `scale` | `Vec3` | |
| Rotation | `rotation` | `f64` (rad) | |
| ColumnCount, RowCount | `array` | `Option<(u32, u32)>` | None = single insert |
| ColumnSpacing, RowSpacing | `array_spacing` | `Vec2` | |
| Attributes | `attributes` | `Vec<AttRef>` | Inline attribute values |

## DobjectAttDef — `AttDef` struct (planned)

Attribute *definition* — lives inside a block definition.

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| Tag | `tag` | `String` |
| Prompt | `prompt` | `String` |
| Default | `default` | `String` |
| InsertionPoint | `insertion` | `Vec2` |
| TextHeight | `height` | `f64` |
| Mode | `mode` | `AttMode` bitflags (Invisible, Constant, Verify, Preset) |
| TextString | `value` | `String` |
| StyleName | `style` | `String` |

## DobjectAttRef — `AttRef` struct (planned)

Attribute *reference* — lives inside a `BlockRef`.

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| Tag | `tag` | `String` |
| TextString | `value` | `String` |
| Position | `position` | `Vec2` |
| TextHeight | `height` | `f64` |
| Rotation | `rotation` | `f64` (rad) |

## DobjectDim — Dimension family (planned)

> **One enum, many variants** is the cleanest mapping. Suggest:
> ```rust
> pub enum Dim {
>     Rotated(DimRotated),
>     Aligned(DimAligned),
>     Angular3Point(DimAngular3Point),
>     Angular2Line(DimAngular2Line),
>     Radial(DimRadial),
>     Diameter(DimDiameter),
>     Ordinate(DimOrdinate),
>     Arc(DimArc),
> }
> ```

### DobjectDimRotated — `DimRotated` (linear dimension)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| DefinitionPoint | `dim_line_pt` | `Vec2` |
| Line1Point, Line2Point | `extension_origins` | `(Vec2, Vec2)` |
| TextMidPoint | `text_mid` | `Vec2` |
| TextString | `text_override` | `Option<String>` (None = use `measurement`) |
| DimStyle | `style` | `String` |
| Measurement | *derived* | `f64` |
| TextRotation | `text_rotation` | `f64` (rad) |
| Arrowhead1Type, Arrowhead2Type | `arrowheads` | `(ArrowType, ArrowType)` |

Other dimension types (Aligned, Angular, Radial, Diameter, Ordinate, Arc)
get their own structs with shape-specific points but share most styling
via `DimStyle`. **TBD**: enumerate each in detail when DIMSTYLE work begins.

## DobjectImage — `RasterImage` struct (planned)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| ImageDefId | `image_def` | `u64` (handle to image definition) |
| InsertionPoint | `insertion` | `Vec2` |
| ScaleFactor | `scale` | `f64` |
| Rotation | `rotation` | `f64` (rad) |
| Width, Height | `size` | `Vec2` |
| Brightness, Contrast, Fade | `display` | `RasterDisplay` `{ brightness: u8, contrast: u8, fade: u8 }` |
| ShowClipped | `show_clipped` | `bool` |
| ClipBoundary | `clip_boundary` | `Option<Vec<Vec2>>` |

## DobjectWipeout — `Wipeout` struct (planned)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| BoundaryPoints | `boundary` | `Vec<Vec2>` |
| Normal | `normal` | `Vec3` |
| ClipMode | `frame_visible` | `bool` |
| Color | (Common `color`) | usually opaque white |

## DobjectViewport — `Viewport` struct (planned)

Paper-space viewport into model space.

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| CenterPoint | `paper_center` | `Vec2` (paper space) |
| Width, Height | `paper_size` | `Vec2` |
| ViewCenter | `model_center` | `Vec2` |
| ViewHeight | `model_height` | `f64` |
| ViewTarget | `view_target` | `Vec3` |
| ViewDirection | `view_direction` | `Vec3` |
| Scale | `scale` | `f64` |
| DisplayLocked | `locked` | `bool` |
| OnOff | `on` | `bool` |

## DobjectSolid2D — `Solid2D` struct (planned, filled triangle/quad)

| Your name | Suggested Rust field | Type | Note |
|-----------|---------------------|------|------|
| Corner1..Corner4 | `corners` | `[Vec2; 4]` | If only 3 corners (triangle), set `corners[3] == corners[2]` per DXF convention |
| Color | (Common `color`) | | |

## DobjectRay — `Ray` struct (planned)

Infinite half-line.

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| BasePoint | `base` | `Vec2` |
| DirectionVector | `direction` | `Vec2` (should be normalized; intersection/snap algos must handle |dir|=1) |

## DobjectXline — `Xline` struct (planned)

Infinite construction line (both directions).

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| BasePoint | `base` | `Vec2` |
| DirectionVector | `direction` | `Vec2` |

## DobjectLeader — `Leader` struct (planned, legacy)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| Vertices | `vertices` | `Vec<Vec2>` |
| ArrowheadType | `arrowhead` | `ArrowType` |
| Annotation | `annotation` | `Option<u64>` (handle to MText/Tolerance/BlockRef) |

## DobjectMLeader — `MLeader` struct (planned)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| LeaderLines | `leader_lines` | `Vec<LeaderLine>` |
| Content | `content` | enum `MLeaderContent` (`MText / Block / Tolerance / None`) |
| LandingGap | `landing_gap` | `f64` |
| DoglegLength | `dogleg_length` | `f64` |
| ArrowheadSize | `arrowhead_size` | `f64` |
| TextAngle | `text_angle_style` | enum `TextAngleStyle` (Horizontal / Aligned) |

## DobjectTolerance — `Tolerance` struct (planned)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| InsertionPoint | `insertion` | `Vec2` |
| FrameType | `frame` | `u8` |
| ToleranceText | `text` | `String` |
| Direction | `direction` | enum `TolDir` (Horizontal / Vertical) |

## DobjectTable — `Table` struct (planned)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| InsertionPoint | `insertion` | `Vec2` |
| NumRows, NumColumns | `dimensions` | `(u32, u32)` |
| RowHeight, ColumnWidth | `row_heights`, `col_widths` | `Vec<f64>` each |
| CellText | `cells` | `Vec<Vec<CellContent>>` |
| TableStyle | `style` | `String` |
| GridColor, BorderLineweight | per-cell on `CellStyle` | |

---

# 3D types (deferred — RUST_CAD is 2D today)

These get scaffolded only when RUST_CAD adds a Z axis.

## DobjectSubDMesh — `SubDMesh` struct (deferred)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| VertexPositions | `vertices` | `Vec<Vec3>` |
| FaceIndices | `faces` | `Vec<Vec<u32>>` |
| SubdivisionLevel | `subd_level` | `u8` |
| Smoothness | `smoothness` | `f64` |

## DobjectSurface — `NurbsSurface` struct (deferred)

| Your name | Suggested Rust field | Type |
|-----------|---------------------|------|
| DegreeU, DegreeV | `degree_u`, `degree_v` | `u8` |
| ControlPoints | `control_points` | `Vec<Vec<Vec3>>` |
| KnotsU, KnotsV | `knots_u`, `knots_v` | `Vec<f64>` |
| IsClosedU, IsClosedV | `closed_u`, `closed_v` | `bool` |

## DobjectSolid3D — `Solid3D` struct (deferred, ACIS)

| Your name | Suggested Rust field | Type | Note |
|-----------|---------------------|------|------|
| Volume | *derived* | `f64` | |
| SurfaceArea | *derived* | `f64` | |
| BoundingBox | *derived* | `(Vec3, Vec3)` | |
| History | `keep_history` | `bool` | |
| DecomposedInto | `regions` | `Vec<…>` | ACIS data — out of scope until we adopt an ACIS-equivalent geometry kernel |
| *ACIS body* | `acis_data` | `Vec<u8>` | Binary SAT — would need a kernel like OpenCASCADE |

> `DobjectSolid2D` (above) and `DobjectSolid3D` are deliberately distinct
> names — the AutoCAD source called both "SOLID" but they're unrelated
> entities. We use suffixed names to keep them apart in code.

---

# Possibly missing from your list — FUTURE / no action now

> **Status: parked.** User has flagged these for *future updates only* —
> no decision is needed today and no agent should re-prompt for one. The
> table stays here as a memo so we don't lose track of these AutoCAD
> entity types. Revisit only when (a) a real drawing needs one, (b) DXF
> import hits one and has to either skip-or-handle, or (c) the user
> explicitly opens this topic.

These are real AutoCAD entity types the original list didn't cover. The
"Recommendation" column is my opinion only — kept for reference; the
**user has not committed to anything in this table**.

| Entity | Conceptual name | Why it might matter | My recommendation |
|--------|-----------------|--------------------|--------------------|
| REGION | `DobjectRegion` | 2D bounded planar surface (Boolean ops between regions) | Drop — niche, can fake with closed polyline + hatch |
| MLINE | `DobjectMLine` | Multi-line (parallel lines like a wall section) | Keep — common in arch drawings |
| HELIX | `DobjectHelix` | 3D spring/spiral path | Drop — 3D only |
| 3DFACE | `DobjectFace3D` | Legacy 3D face (4 corners) | Drop — superseded by mesh / solid |
| POLYFACEMESH | `DobjectPolyFaceMesh` | Legacy multi-face mesh | Drop — superseded by `SubDMesh` |
| POLYGONMESH | `DobjectPolygonMesh` | Legacy grid mesh | Drop — superseded by `SubDMesh` |
| MLEADERSTYLE | (table, not entity) | Style defs for MLeader | Goes in `MLeaderStyleTable`, not here |
| UNDERLAY | `DobjectUnderlay` | PDF/DGN/DWF underlays | Drop until interop matters |
| GEODATA | `DobjectGeoData` | Geographic projection + lat/long origin | Drop — GIS-adjacent, off-roadmap |
| SHAPE | `DobjectShape` | Legacy SHX shape glyph | Drop — obsolete |
| TRACE | `DobjectTrace` | Legacy filled band | Drop — superseded by 2D solid |
| FIELD | `DobjectField` | Auto-updating text (filename, date, formula) | Drop — text feature, not geometric |
| LIGHT | `DobjectLight` | 3D rendering light source | Drop — render-only |
| CAMERA | `DobjectCamera` | 3D view camera definition | Drop — render-only |

# Slice progression

| Slice | Status | What it lands |
|-------|--------|---------------|
| **A. Property foundation** | ● **Done** | `Color`, `Lineweight`, `Linetype`, `Layer` types; `Style` struct; `DObject` wrapper around `Geom` + Style + Handle; `Document` container holding `dobjects` + `layers` + `linetypes`; renderer resolves `Color::ByLayer` and honours `style.visible` + `layer.visible/frozen` |
| **B. Layer panel (UI)** | ○ Next | Egui dock — list `Document.layers`, add/rename/delete, visibility/lock/freeze toggles, click to set active |
| **C. Pen palette (UI)** | ○ | Egui dock — pen presets (color + linetype + lineweight bundles), "Apply to selection" |
| **D. Entity Info panel** | ○ | Egui dock showing the property tables in this doc for the current selection |
| **E. New Dobject types** | ○ | `DobjectPoint` → `DobjectPolyline` → `DobjectText` → `DobjectMText` → `DobjectDimRotated`. Each is a new `Geom` variant + per-type renderer; the Common property set comes for free |
| **F. Block table + Block panel** | ○ | First entry on `Document` beyond layers/linetypes; introduces document hierarchy needed for INSERT |
| **G. UCS / Named Views / Library / Command Line panels** | ○ | Lighter dependencies, can land in any order |

Each step flips the relevant `○` to `●` in this doc.
