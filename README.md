# Auto-RASM(Real-time Automated Simulation Module)

**A pure-Rust 2D CAD workbench.** (internal name: `RUST_CAD`, binary: `rust_cad`)

Auto-RASM is a from-scratch 2D drafting application written entirely in Rust. It
is **not** a port of any existing CAD program — it has its own geometry kernel,
its own intersection and object-snap math, and its own on-disk format. The goal
is an accurate, hackable, dependency-light CAD core that an engineer (or an AI
agent) can reason about end-to-end, paired with a fast GPU UI.

> Status: **active development / alpha.** The drafting, editing, blocks, walls,
> dimensions, text, snap and file-IO layers are working; the raster→vector and
> AI-driven command subsystems are being built out (see [Roadmap](#roadmap)).

---

## Why it exists

Most CAD code is a tangle of UI and math. Auto-RASM keeps them strictly apart:

- **The kernel is pure, testable, and UI-free.** Every intersection is a free
  function with `#[cfg(test)]` coverage. You can pipe commands into a CLI
  (`cad_cli`) and inspect the geometry output line by line — no window required.
- **Almost no external dependencies.** The whole CAD core — geometry, snapping,
  DXF/RSM I/O, walls, NURBS — compiles with **zero third-party crates**. Only the
  GUI shell (egui/glow) and the raster image decoder pull dependencies.
- **Data lives in the kernel; logic lives in feature crates; pixels live in the
  app.** This layering is enforced by the crate graph, so feature work stays
  isolated and several people can work in parallel without stepping on each other.

---

## Features

### Geometry & drafting
- Primitives: **line, circle, arc, ellipse, elliptical arc, point, polyline
  (with bulges), NURBS spline, hatch, text, dimension, wall, block reference.**
- A 6-way pairwise **intersection kernel** (line/circle/arc and beyond) with a
  uniform-grid **spatial index** for fast picking on large drawings.
- **Object snap** engine: endpoint, midpoint, center, quadrant, intersection,
  perpendicular, tangent, nearest — with one-shot inline overrides typed mid-command.
- **CARD** (cardinal H/V) drafting lock.

### Editing
- **Trim / extend** with a basket-first selection model (separate cutting-edge
  and target baskets), break-into-all-segments, and automatic re-join of
  over-split survivors.
- **Fillet, chamfer, offset, join, stretch** (with interactive grips and
  snap-on-release), copy, move.
- **Match properties** (source → multiple targets: layer, colour, linetype,
  wall style) with window/crossing target selection.
- Bounded **undo / redo**.

### Selection
- The pointer tool is always-on **select** (click adds, Shift-click removes;
  selected geometry renders as a transient dashed overlay).
- Window / crossing rubber-band, plus single-letter shortcuts (W/C/A/B/L/N) and
  a reusable **selection bank**.

### Styles & organisation
- **AutoCAD Color Index (ACI)** as the primary colour model, picked from a polar
  256-swatch wheel; TrueColor as a secondary option. `ByLayer` / `ByBlock`.
- **Layers**, **linetypes** (named dash/gap patterns with a graphical picker),
  **lineweights**.

### Architectural
- **Walls** as first-class objects: chained drawing, automatic junction cleanup,
  justification, wall styles, and **batt-insulation** rendering.
- **Smart objects** (in progress): a wall keeps its centerline as permanent
  identity and re-derives its visible geometry when parameters change.

### Blocks (parametric)
- Block table, block references, transparent **explode** (snap/trim/render).
- An **isolated Block Editor** — an embedded canvas where you record a
  *parameter* by demonstrating a stretch; the value later drives the amount via a
  direction · gain model. (A genuinely simple parametric workflow.)
- **Door-cuts-wall**: mark cut edges inside the block; on insert the block asks
  for a point and an angle, then clips the host wall opening automatically.

### Text & dimensions
- Single-line **text** with text styles and alignment.
- **Dimensions** (linear, aligned, radius, diameter) driven by definition points
  + a dimension style; extension lines, arrows and text are derived each frame.

### Command line
- A prompt-driven command flow (e.g. **CIRCLE**: center/radius/diameter/2-point/
  3-point/**Ttr** tangent-tangent-radius), ribbon buttons that route through the
  same flow, live drafting preview, a command **transcript**, and an Esc-driven
  `command:` prompt that fills upward.
- Being refactored into an AI-pluggable subsystem (one Command IR, deterministic
  core, see `COMMAND_LINE.md`).

### File I/O
- **DXF** read/write.
- **RSM** — a native binary format carrying full styles, blocks and walls.

### Raster → Vector (new)
- Import a scanned drawing and open it in a dedicated editor.
- **Adjustments**: grayscale, brightness/contrast, threshold, invert,
  colour-isolation; plus an advisory "is this convertible?" analyzer.
- **Buffer layers = a destructive Photoshop-style carve.** Brush-marking *moves*
  ownership of those pixels (at their original positions) into the active layer,
  exclusively, so the source image peels apart layer by layer. Each layer carries
  its target geometry (lines / arcs / NURBS), CAD layer, and colour. Per-layer
  visibility and tint let you watch the split happen.
- A **GIMP-style file browser** (Places sidebar, breadcrumb path, Name/Size/Type/
  Modified columns, and a rich preview pane with pixel dimensions).
- Type-aware **trace engines** (mask → real DObjects) are the next slice.

### Developer tooling
- A built-in **Session Recorder** that captures the event sequence, per-click
  hit-tests, state transitions, and memory events — the primary tool for
  debugging interaction bugs.

---

## Architecture

A Cargo workspace. Feature crates depend on the kernel; data stays in the kernel,
logic in the feature crate, and egui only in the app.

| Crate | Role | External deps |
|-------|------|---------------|
| `cad_kernel` | Geometry, intersections, snap, spatial index, styles, layers, blocks, dimensions, text, walls data, DObject/Document model, command parser | **none** |
| `cad_nurbs`  | Self-contained NURBS/spline math (so the kernel can use it without a cycle) | **none** |
| `cad_snap`   | Object-snap re-exports / helpers over the kernel | **none** |
| `cad_io`     | DXF + native RSM read/write | **none** |
| `cad_wall`   | Architectural wall command (chained draw, junctions, openings) | **none** |
| `cad_raster` | Raster→vector: layer stack, adjustments, convertibility analysis, trace dispatch | `image` |
| `cad_cli`    | Headless command pipe for inspecting kernel output | **none** |
| `cad_app`    | The GPU application: eframe/egui + glow renderer, all interaction | `eframe`, `egui`, `egui_glow`, `glow`, `image` |

~43k lines of Rust across the workspace.

---

## Build & run

Requires a recent stable Rust toolchain.

```bash
# build the GUI application
cargo build --release -p cad_app

# run it
./target/release/rust_cad
```

```bash
# run the whole test suite (kernel math, IO, etc.)
cargo test

# headless geometry inspector
cargo run -p cad_cli
```

The app uses egui + glow and runs on Linux (Wayland / X11) and Windows.

---

## Roadmap

Implemented and stable: drafting primitives, intersections, object snap, the
editing suite (trim/extend/fillet/chamfer/offset/stretch/matchprop), selection,
styles (ACI/layers/linetypes/lineweights), walls + wall styles, parametric blocks
+ Block Editor + door-cuts-wall, text, dimensions, hatch, the command flow, and
DXF/RSM I/O.

In progress / planned:

- **Raster→vector trace engines** — centerline tracing, Hough lines, least-squares
  arc fit, OCR for text, dimension recognition, furniture/template matching, and
  double-line→wall detection (`RASTER_TO_VECTOR.md`).
- **AI-pluggable command line** — a single Command IR with a deterministic core and
  an optional AI resolver (`COMMAND_LINE.md`).
- **Smart objects** — generator/centerline identity that re-derives visible
  geometry on edit (`Smart_Dobjects.md`).
- **User Coordinate Systems**, an expanded **hatch pattern library**, scripting +
  command aliases, and a 2D **constraint solver**.

Design notes for each of these live as Markdown documents at the repository root
(`ARCHITECTURE.md`, `ROADMAP.md`, `Map_HSI_LibreCAD.md`, and the per-feature
specs).

---

## License

To be determined. Third-party dependencies are restricted to permissive
(MIT / Apache-2.0) pure-Rust crates by project policy.
