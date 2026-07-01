# RUST_CAD — Architecture

Authoritative entry point for the whole application. There are **two
independent architecture axes**, and code must respect both:

1. **Library / crate layering** — *where logic lives* (model → feature → app).
   Owned by this document (§3).
2. **Application / UI architecture** — *how the running app is structured*
   (workspace, docking, commands, panels, theme, plugins). Overviewed here
   (§4) and specified in the subsystem docs below.

> Status: §3 (crate layering) reflects the **current** code. §4 and the
> subsystem docs are the **target architecture** — partially implemented today,
> authoritative for all new work. Where a system is not yet built it is marked
> *Proposed*. Keep these docs updated **before or alongside** the code that
> changes them (see §8).

---

## 1. Document map

| Doc | Owns |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | This file — overall structure + the two axes + index. |
| [WORKSPACE_SYSTEM.md](WORKSPACE_SYSTEM.md) | Regions, docking abstraction, floating windows, workspaces, layout persistence. |
| [COMMAND_SYSTEM.md](COMMAND_SYSTEM.md) | `CommandRegistry` as the single source of truth for every command surface. |
| [PANEL_SYSTEM.md](PANEL_SYSTEM.md) | Panel trait, registration, lifecycle, docking/floating, persistence, taxonomy. |
| [THEME_SYSTEM.md](THEME_SYSTEM.md) | Design tokens, Theme Editor, inheritance, live updates, the locked token registry. |
| [CONTENT_STYLE.md](CONTENT_STYLE.md) | Content half of the design system: terminology, capitalization, number/unit formatting, message copy. |
| [PLUGIN_API.md](PLUGIN_API.md) | Panel + command registration, extension points, future FFI plugin contract. |

Supporting existing docs: [MODULES.md](MODULES.md) (cad_app module map),
[AGENTS.md](AGENTS.md) (coding rules), [COMMAND_LINE.md](COMMAND_LINE.md),
[SETTINGS.md](SETTINGS.md) / [Variables.md](Variables.md) (the variable
registry), [OPEN_ISSUES.md](OPEN_ISSUES.md), [ROADMAP.md](ROADMAP.md).

---

## 2. The two axes

The two axes are orthogonal. A feature usually touches both:

```
                 application / UI architecture  (§4, subsystem docs)
                 ───────────────────────────────────────────────►
   library   │   Workspace · Dock · Command · Panel · Theme · Plugin
   layering  │   ───────────────────────────────────────────────
   (§3)      │   all of the above live in the APP layer (cad_app)
     │       │
     ▼       │   feature crates (cad_wall, cad_dim, cad_text) — UI-free logic
             │   cad_kernel (+ cad_nurbs) — model + math, UI-free
```

The application architecture (docking, commands, panels, theme) is entirely an
**app-layer (`cad_app`)** concern. The kernel and feature crates never know it
exists. This keeps the headless `cad_cli` runner and all UI-free tests valid.

---

## 3. Library & crate layering

*(Current code. Read before adding a feature or moving code.)*

### 3.1 Layering principle

Three layers, bottom to top:

1. **MODEL** — `cad_kernel` (+ `cad_nurbs`). Pure data + math, UI-free. The
   `Geom` enum, every entity data type (Line, Arc, Circle, Ellipse, Wall, Dim,
   Text, Polyline, Spline, Hatch…), style tables, the `Document`, and the
   per-variant behavior that must `match` the enum (transform, snap, intersect,
   bbox, grips, parser).
2. **FEATURE crates** — one per major smart feature: `cad_wall`, `cad_dim`,
   `cad_text`. They depend on `cad_kernel` and hold feature *algorithms* that
   are not just data. Pure, UI-free, headless-testable.
3. **APP / IO** — `cad_io` (file formats), `cad_app` (the egui GUI),
   `cad_cli` (headless command runner).

Feature crates depend on the kernel (not the reverse): the `Geom` enum must
*name* `Wall`/`Dim`/`Text`, so those data types live in the kernel; the logic
lives one layer up. This is cycle-free and user-approved.

### 3.2 Crates

| Crate | Type | Role | Depends on |
|---|---|---|---|
| `cad_nurbs` | lib | Pure-Rust NURBS / B-spline math (leaf). | — |
| `cad_kernel` | lib | Model core: `Geom`, entity data, styles, `Document`, transform/snap/intersect/bbox/parser. | cad_nurbs |
| `cad_wall` | lib | Wall feature logic (`solve_faces`, curved-wall derive, junctions). | cad_kernel |
| `cad_dim` | lib | *(planned)* Dimension render-geometry + formatting. | cad_kernel |
| `cad_text` | lib | *(planned)* Text layout / wrapping / alignment. | cad_kernel |
| `cad_io` | lib | File I/O: `dxf`, `rsm` (native). | cad_kernel |
| `cad_snap` | lib | Thin facade over `cad_kernel::snap` (external API). | cad_kernel |
| `cad_cli` | bin | Headless command runner (no GUI). | cad_kernel |
| `cad_app` | bin `rust_cad` | eframe/egui GUI: paint, dialogs, tool input, **and all UI subsystems in §4**. | cad_kernel, cad_io, cad_wall, … |

```
cad_nurbs
   └─ cad_kernel ─┬─ cad_wall ─┐
                  ├─ cad_dim  ─┤
                  ├─ cad_text ─┤
                  ├─ cad_io  ──┤
                  ├─ cad_snap  │   (facade; not consumed internally)
                  ├─ cad_cli   │
                  └────────────┴─ cad_app
```

### 3.3 Where new code goes

- New entity data type / `Geom` variant / per-variant logic → **`cad_kernel`**.
- UI-free feature algorithm → the **feature crate**.
- egui rendering, dialogs, tool input, **and the UI subsystems (§4)** → **`cad_app`**.
- File format reader/writer → **`cad_io`**.

Litmus test: *"Could this run headless with no egui?"* If yes and feature-
specific, it belongs in the feature crate.

---

## 4. Application / UI architecture  *(target)*

The running app is organised into **regions**, populated by **panels**, driven
by a **command registry**, styled by **design tokens**, with the docking engine
hidden behind an **interface** so it can be replaced.

### 4.1 Workspace regions

```
┌───────────────────────── Menu bar ─────────────────────────┐
│  Left   │            Center Workspace            │  Right   │
│  Rail   │   ┌──────── Canvas ─────────┐          │  Dock    │
│ (rails) │   │                         │          │  Area    │
│         │   └─────────────────────────┘          │ (tabs /  │
│         │   ┌──── Command bar ────────┐          │  stacks) │
│         │   └─────────────────────────┘          │          │
├─────────┴───────── Bottom Dock Area (future) ────┴──────────┤
│                       Status bar                            │
└─────────────────────────────────────────────────────────────┘
        Floating Windows overlay any region (multi-monitor → P3)
```

- **Left Rail**, **Right Dock Area**, and (reserved) **Bottom Dock Area** span
  full height, menu → status bar.
- The **Canvas** and **Command bar** live only in the **Center Workspace**.
- **Floating Windows** overlay the canvas; long-term they become separate OS
  windows (multi-viewport) for multi-monitor.

The egui panel-add order produces this for free — see
[WORKSPACE_SYSTEM.md](WORKSPACE_SYSTEM.md) §"Panel ordering".

### 4.2 Core abstractions

| Abstraction | Role | Doc |
|---|---|---|
| `Panel` | A self-contained tool UI (Inspector, Layers, Theme Editor…). Knows nothing about docking or other panels. | PANEL_SYSTEM |
| `PanelRegistry` | Holds all registered panels; the Window menu + dock read it. | PANEL_SYSTEM |
| `DockHost` (interface) | The replaceable docking boundary: `open/close/dock/float/focus` + `save/restore_layout`. `egui_dock` is one adapter. | WORKSPACE_SYSTEM |
| `Command` + `CommandRegistry` | Every actionable thing, registered once, rendered by **all** command surfaces. | COMMAND_SYSTEM |
| `DesignTokens` / `ThemeStore` | The single set of values every component reads. The Theme Editor edits these live. | THEME_SYSTEM |
| Plugin contract | Panels + commands registered by external code. | PLUGIN_API |

### 4.3 Two registration flows

Every panel registers itself **twice** — its UI with the dock, its actions with
the command registry:

```
   Inspector · Layers · Theme editor · Variables · (+ plugins)
        │ register UI                    │ register commands
        ▼                                ▼
   PanelRegistry                    CommandRegistry
        │                                │
        ▼                                ▼
   DockHost  (interface)           Command bar · rails · menus ·
        │                          context menus · palette · shortcuts
        ▼
   egui_dock  (swappable impl)
```

### 4.4 Key data flows

- **Selection → Inspector.** Selecting dobjects recomputes the Inspector's
  `ui()` + `commands()` from the selection (line → length/angle/layer/color;
  multi → common props + *various*). No layout special-casing.
- **Command invocation.** Any surface (rail click, menu, typed alias, shortcut,
  palette) resolves a `Command` from the registry and runs it; mutating
  commands push an `UndoEntry`.
- **Theme edit → repaint.** The Theme Editor writes a token in the `ThemeStore`,
  which invalidates and `request_repaint()`s; every component re-reads tokens.

### 4.5 Principles

1. **Decoupling over convenience.** Panels ↔ dock ↔ commands communicate only
   through interfaces/registries. No panel imports the dock engine; no UI
   surface hard-codes a command list.
2. **Single source of truth.** Commands live once in the `CommandRegistry`;
   visual values live once in `DesignTokens`. UI renders *from* these.
3. **Replaceable engine.** `egui_dock` sits behind `DockHost`; swapping it is an
   adapter change, not an app change.
4. **Extensible by registration.** New panels/commands/plugins *register*; the
   layout engine never changes. ([PLUGIN_API.md](PLUGIN_API.md))
5. **Command-line-first.** Keyboard stays on the command line by default; panels
   are not in the global tab order (see the design-system accessibility rules).
6. **Canvas is sacred.** It never lags; chrome animates subtly, canvas is 0ms.

---

## 5. Module placement in `cad_app`

*Proposed — see [MODULES.md](MODULES.md) for the current module map.* The UI
subsystems get dedicated modules so `app.rs` stops being a monolith:

```
cad_app/src/
  workspace/   region layout, DockHost trait, egui_dock adapter, workspaces
  command/     Command, CommandRegistry, palette, shortcut map
  panels/      Panel trait, PanelRegistry, inspector/, layers/, theme_editor/, …
  theme/       DesignTokens, ThemeStore, serialization
  plugin/      registration API (future FFI boundary)
  app.rs       wires the above together; shrinks over time
```

---

## 6. Status & migration

- **Now:** single workspace; Properties/Layers/etc. are floating `egui::Window`s;
  rails + menus + command line are hand-authored; tokens are scattered constants.
- **Target:** regions + `DockHost`; panels behind `Panel`/`PanelRegistry`;
  all command surfaces generated from `CommandRegistry`; all values from
  `DesignTokens`.
- **Order of work** (high level): Panel/Command abstractions (no visible change)
  → `DockHost` + right Dock Area → command-driven rails/menus → theme tokens +
  Theme Editor → floating/multi-viewport → plugin API. Each subsystem doc owns
  its own phased plan.

---

## 7. Relationship to AGENTS.md

[AGENTS.md](AGENTS.md) describes a future `.so`/FFI **entity/tool** plugin
surface (the `DObject` trait). That is a *kernel-level* extension axis. The
**UI** plugin surface in [PLUGIN_API.md](PLUGIN_API.md) (panels + commands) is a
separate, app-level axis. Both are designed to coexist; PLUGIN_API §"FFI
readiness" keeps the UI contract expressible across an FFI boundary later.

---

## 8. Documentation governance

These docs are the **authoritative source of truth** for implementation and
future change. Rule: **whenever a new reusable capability is introduced, update
the corresponding doc before or alongside the code.** A PR that adds a panel
type, a command surface, a token category, or a docking behavior must update the
matching subsystem doc in the same change. Docs always reflect the current
architecture.
