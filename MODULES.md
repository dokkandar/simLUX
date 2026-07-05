# RUST_CAD — Module & Library Inventory

Concrete list of every crate (library) and every module inside it, plus the
feature-crate split. Companion to `ARCHITECTURE.md` (which holds the layering
rules + workflow); this file is the *inventory*.

> **Crate vs module:** a **crate** is a compiled library with its own
> `Cargo.toml` (independently usable). A **module** is a `.rs` file *inside* a
> crate (a namespace, not independently usable). "Dimension / Wall / Text as
> individual libraries" = promote their **logic** into separate **crates**,
> while their **data** stays as **modules inside `cad_kernel`**.

---

## 1. Crates (7 today)

| Crate | Type | What it is | Depends on |
|---|---|---|---|
| `cad_nurbs` | lib | NURBS / B-spline curve math (leaf). | — |
| `cad_kernel` | lib | Model + math core (19 modules — §2). | cad_nurbs |
| `cad_wall` | lib | **Feature crate:** wall junction / derive logic. | cad_kernel |
| `cad_io` | lib | DXF + RSM file I/O. | cad_kernel |
| `cad_snap` | lib | Public facade over `cad_kernel::snap` (not consumed internally — orphaned). | cad_kernel |
| `cad_cli` | bin | Headless command runner. | cad_kernel |
| `cad_app` | bin `rust_cad` | The egui GUI (6 modules — §3). | cad_kernel, cad_io, cad_wall |

```
cad_nurbs → cad_kernel ─┬─ cad_wall ─┐
                        ├─ cad_io  ──┤
                        ├─ cad_snap  │ (facade)
                        ├─ cad_cli   │
                        └────────────┴─ cad_app
```

---

## 2. Modules inside `cad_kernel` (19)

| Module | Role |
|---|---|
| `math` | `Vec2`, angle helpers, `EPS` |
| `geom` | `Geom` enum + every primitive struct (Line / Circle / Arc / Ellipse / EllipseArc / Point / Polyline / Spline / **Wall**) + transform / bbox / grips / fillet / chamfer / join / `bulge_arc` |
| `dobject` | `DObject` = geom + style + handle |
| `document` | `Document` = dobjects + all resource tables |
| `layer` | `Layer`, `LayerTable` |
| `linetype` | `Linetype`, `LinetypeTable` (dash patterns) |
| `lineweight` | `Lineweight`, resolution |
| `color` | `Color`, ACI palette, `TrueColorTable`, `resolve_color` |
| `pen` | `PenTable` (named color+linetype+lineweight bundles) |
| `style` | per-dobject style (color / layer / linetype / lineweight) |
| `text` | **Text, TextStyle, TextStyleTable** |
| `dim` | **Dim, DimKind, DimStyle, DimStyleTable** |
| `wallstyle` | **WallStyle, WallStyleTable** |
| `patterns` | hatch pattern definitions |
| `intersect` | pairwise geometry intersection |
| `snap` | object-snap engine (END / MID / CEN / QUA / INT / PER / TAN / NEA) |
| `spatial` | uniform-grid spatial index (fast bbox / near queries) |
| `construct` | constructors (`wall_sides`, ellipse helpers) |
| `parser` | command-line `Command` parser |

---

## 3. Modules inside `cad_app` (6)

| Module | Role |
|---|---|
| `app` | the GUI core: `CadApp`, update loop, rendering, dialogs, tools, commands (~21k LOC) |
| `aci_picker` | the polar ACI color-wheel widget |
| `hatch_trace` | boundary tracing for hatch fill |
| `gpu` | the glow / GPU render path |
| `dbg_recorder` | session recorder (the `=== SESSION DUMP ===` traces) |
| `settings` | `UserEnv` / SYSVAR persistence |

*(`wall` module used to live here — now promoted to the `cad_wall` crate.)*

---

## 4. The feature-crate split — exactly what goes where

| Feature | DATA — stays in `cad_kernel` | LOGIC — its own crate | egui — stays in `cad_app` |
|---|---|---|---|
| **Wall** ✅ | `geom::Wall`, `wallstyle::*`, `bulge_arc` | **`cad_wall`**: `solve_faces` (+ future T/X-junctions, openings, rooms, convert closed→wall) | render arm, Wall Style Manager, tool / `t` flow |
| **Dim** ⬜ | `dim::*` (Dim / DimStyle) | **`cad_dim`** *(planned)*: `dim_render_geometry`, number formatting | render call, DimStyle Manager, `dim` tool |
| **Text** ⬜ | `text::*` (Text / TextStyle) | **`cad_text`** *(planned)*: layout / wrap / alignment | render arm, text dialog, text tool |

**Rule:** a data type or a `Geom` variant → `cad_kernel`; a UI-free algorithm →
the feature crate; anything egui → `cad_app`; a file format → `cad_io`.

Status: **Wall is a real individual library (`cad_wall`).** Dim and Text still
have their data as kernel modules and their logic in `cad_app`; promoting them
to `cad_dim` / `cad_text` is the remaining work, following the pattern
`cad_wall` proved.
