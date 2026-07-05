# PANEL_SYSTEM

Panel trait, registration, lifecycle, docking/floating, persistence, and the
panel taxonomy. Parent: [ARCHITECTURE.md](ARCHITECTURE.md) §4.

> Status: **Proposed.** Today Properties/Layers/etc. are bespoke
> `egui::Window`s. This doc is the target: every tool panel is a uniform,
> registered `Panel`.

---

## 1. What a panel is

A **panel** is a self-contained tool UI hosted by the Dock Area. It knows
nothing about docking, layout, or other panels. It exposes a UI and a set of
commands. Examples: **Inspector**, **Layers**, **Variables**, **Theme editor**,
**Block library**.

> Naming: the former "Property Panel" is renamed **Inspector** throughout.

---

## 2. The `Panel` trait

```rust
// Interface sketch — not final.
pub trait Panel {
    fn id(&self) -> PanelId;                 // stable: "inspector", "layers"
    fn title(&self) -> &str;                 // tab label, sentence case
    fn icon(&self) -> IconId;
    fn category(&self) -> PanelCategory;     // Inspect, Organize, Author, Tools
    fn default_location(&self) -> DockRegion;

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut AppCtx);   // render body
    fn commands(&self, ctx: &AppCtx) -> Vec<Command>;        // → CommandRegistry

    fn save_state(&self) -> Option<PanelState> { None }      // serde blob
    fn restore_state(&mut self, _s: PanelState) {}

    fn on_open(&mut self, _ctx: &mut AppCtx) {}
    fn on_close(&mut self, _ctx: &mut AppCtx) {}
}
```

Panels consume **design tokens** only (never hard-coded values) and emit
**commands** through `commands()`. They never reference `egui_dock` or the
`DockHost` impl.

---

## 3. Registration

```rust
pub trait PanelRegistry {
    fn register(&mut self, source: SourceId, panel: Box<dyn Panel>);
    fn unregister_source(&mut self, source: SourceId);
    fn get_mut(&mut self, id: PanelId) -> Option<&mut dyn Panel>;
    fn all(&self) -> impl Iterator<Item = &dyn Panel>;       // Window menu, etc.
}
```

- Built-in panels register at startup; plugins register through the same API
  ([PLUGIN_API.md](PLUGIN_API.md)).
- The **Window menu** is generated from `all()` — every registered panel gets a
  toggle automatically. No layout code changes when a panel is added.
- A panel's `commands()` are pulled into the `CommandRegistry` under the panel's
  `SourceId`, so they vanish when it unloads.

---

## 4. Lifecycle

```
registered ──open──▶ mounted ──(dock|float|move)──▶ visible ──close──▶ unmounted
     ▲                                                                  │
     └──────────────────────── re-open ────────────────────────────────┘
```

- **Lazy render:** only **open** panels run `ui()` each frame. Closed/registered
  panels cost nothing (egui immediate-mode discipline).
- `on_open`/`on_close` bracket expensive setup/teardown (e.g. building a preview).
- Docking/floating/moving are requests to the `DockHost`; the panel is unaware of
  the outcome's mechanics.

---

## 5. Persistence

- **Layout** (where the panel sits) is owned by the `DockHost`
  ([WORKSPACE_SYSTEM.md](WORKSPACE_SYSTEM.md) §6).
- **Content state** (scroll, expanded sections, panel-local options) is owned by
  the panel via `save_state`/`restore_state`, serialized under its `PanelId`.
- Unknown `PanelId`s on restore are dropped silently (version/plugin safety).

---

## 6. The Inspector (context-sensitive)

The Inspector is an ordinary `Panel` whose `ui()` and `commands()` are computed
from the **current selection**:

- **One dobject** → its editable properties: layer, color, line type, line
  weight, plus type-specific geometry (line → length/angle/start/end; circle →
  center/radius; text → string/height; …) and feature sections (e.g. Hatch).
- **Multiple, same type** → shared fields editable; differing values render
  *various* / *Mixed* (indeterminate state from the design system).
- **Multiple, mixed types** → only the common base properties (layer, color,
  linetype, lineweight).
- **Empty selection** → document/drafting defaults or an empty state.

Its commands (`inspector.set-length`, `inspector.assign-layer`, …) are emitted
only when applicable, per [COMMAND_SYSTEM.md](COMMAND_SYSTEM.md) §5.

---

## 7. Panel taxonomy

| Category | Panels (current + planned) |
|---|---|
| **Inspect** | Inspector |
| **Organize** | Layers, Pens, Blocks/Block library |
| **Author** | Dimension style, Wall style, Text style, Variables |
| **Tools** | Theme editor, Session recorder, Diagnostics |
| **Bottom dock (future)** | Console, AI chat, Logs, History, Validation, Build/Output |

The taxonomy is open: new categories/panels register without layout changes.

---

## 8. Phased plan

- **P1** — `Panel` trait + `PanelRegistry`; reimplement the **Inspector** as a
  `Panel`; Window menu generated from the registry; lazy render.
- **P2** — migrate Layers, Variables, styles into `Panel`s; `save/restore_state`.
- **P3** — Theme editor panel ([THEME_SYSTEM.md](THEME_SYSTEM.md)); plugin panels;
  Bottom-dock panels as they ship.
