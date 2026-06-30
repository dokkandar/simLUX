# WORKSPACE_SYSTEM

Layout regions, the docking abstraction, floating windows, named workspaces, and
layout persistence. Parent: [ARCHITECTURE.md](ARCHITECTURE.md) §4.

> Status: **Proposed.** Today panels are floating `egui::Window`s and rails are
> hand-placed `SidePanel`s. This doc is the target the migration follows.

---

## 1. Goals

- A **generic dock area** that hosts any registered panel — never a hard-coded
  Properties panel.
- The application must **never depend on a specific docking implementation**.
  `egui_dock` is the preferred backend *today*, sitting behind a `DockHost`
  interface so another engine can replace it with minimal changes.
- Feel like **Adobe Illustrator / Photoshop**: tabs, stacks, splits, drag-to-
  dock, tear-off floating — not a web app.

---

## 2. Regions

```
Workspace
├── Left Rail            full height (menu → status). Command rails.
├── Center Workspace     canvas + command bar ONLY.
│   ├── Canvas
│   └── Command bar      docked bottom-center or floating.
├── Right Dock Area      full height. Generic panel host (tabs/stacks/splits).
├── Bottom Dock Area     RESERVED (future, hidden). Console/Logs/AI/Diagnostics…
└── Floating Windows     overlay; → separate OS windows for multi-monitor (P3).
```

The Right Dock Area and Left Rail **extend menu → status bar**; the Command bar
and Canvas exist **only** between them. The Bottom Dock Area is understood by
the layout engine now but renders nothing until a panel targets it.

### Panel ordering (how the regions fall out of egui)

egui panel-add order controls nesting. Adding in this order yields the region
rule for free:

1. `TopBottomPanel::top` — menu bar (full width)
2. `TopBottomPanel::bottom` — status bar (full width, pinned bottom)
3. `SidePanel::left` — Left Rail (spans menu → status)
4. `SidePanel::right` — Right Dock Area (spans menu → status)
5. *(reserved)* `TopBottomPanel::bottom` — Bottom Dock Area (center only)
6. `TopBottomPanel::bottom` — Command bar (center only)
7. `CentralPanel` — Canvas (fills remainder)

Steps 5–7 are added **after** the side panels, so they occupy only the center
column. The Bottom Dock Area, when enabled, sits below the command bar but above
the status bar, still center-only.

---

## 3. The `DockHost` interface (the replaceable boundary)

App code talks **only** to this interface. `egui_dock` is one adapter; a hand-
rolled or future engine is another. No panel or app module imports `egui_dock`.

```rust
// Interface sketch — not final.
pub enum DockRegion { LeftRail, RightDock, BottomDock, Center, Floating }

pub trait DockHost {
    fn open(&mut self, panel: PanelId, region: DockRegion);
    fn close(&mut self, panel: PanelId);
    fn focus(&mut self, panel: PanelId);
    fn is_open(&self, panel: PanelId) -> bool;
    fn move_to(&mut self, panel: PanelId, region: DockRegion);
    fn float(&mut self, panel: PanelId, at: Option<Rect>);
    fn redock(&mut self, panel: PanelId);

    // Per-frame: render every open panel by asking the PanelRegistry for its ui().
    fn show(&mut self, ctx: &Context, registry: &mut PanelRegistry);

    // Persistence — opaque blob the host serializes (see §6).
    fn save_layout(&self) -> LayoutState;
    fn restore_layout(&mut self, state: LayoutState);
}
```

**Swap rule:** replacing the engine means writing a new `impl DockHost`. The
`LayoutState` is engine-specific and may need migration, but no panel/command/
app code changes.

---

## 4. Dock model

A region's contents are a **tree**: leaves are **tab groups** (one or more
panels sharing a rectangle, one active tab); internal nodes are **splits**
(horizontal/vertical with a draggable splitter). This is the model `egui_dock`,
VS Code, and Photoshop use, and it is what `LayoutState` serializes.

Supported interactions (delegated to the backend):
- single panel · multiple stacked panels · tabbed panels
- drag-to-reorder tabs · drag-to-dock (edge = split, center = tab)
- tear-off to floating · drag floating back to dock
- splitter resize · collapse-to-icon (P2)

Drop targets show highlighted zones (edge bands = split; center = add tab).

---

## 5. Floating windows & multi-monitor

- **Phase 1–2:** floating panels are in-window `egui::Area`s above the canvas.
- **Phase 3:** a floated panel becomes its own **eframe viewport** (a real OS
  window) hosting the same `Panel`, enabling multi-monitor. Because panels only
  know the `Panel` trait, no panel code changes — the `DockHost` adapter chooses
  in-window vs viewport.

Guards: a **min canvas width/height** clamp prevents docks/floats from starving
the canvas. Floating positions are remembered and offered a snap-back.

---

## 6. Persistence

Saved to app settings (alongside the existing env/settings store):

- `LayoutState` — the dock tree per region (splits, tab groups, sizes), engine-
  produced (`egui_dock::DockState` is serde-serializable).
- The **open-panel set** and each panel's **floating geometry**.
- Per-panel state is owned by the panel (see [PANEL_SYSTEM.md](PANEL_SYSTEM.md)
  §"Persistence").

**Version/plugin safety:** on restore, unknown `PanelId`s are **dropped
silently** (a removed panel or an uninstalled plugin must never break the
layout). Missing panels that should be open fall back to their `default_location`.

---

## 7. Workspaces (named layouts)

A **workspace** is a saved `LayoutState` + open set under a name ("Drafting",
"Modeling", "Theme authoring"), Illustrator-style. Commands: *Save workspace*,
*Switch workspace*, *Reset workspace* (restore the built-in default). Workspaces
are themselves registry-driven commands (see [COMMAND_SYSTEM.md](COMMAND_SYSTEM.md)).

---

## 8. Phased plan

- **P1** — `DockHost` trait + `egui_dock` adapter; Right Dock Area hosting the
  Inspector; tabs/splits/reorder; panel-ordering rule wired. Bottom Dock Area
  region reserved (no UI).
- **P2** — tear-off/redock floating (in-window); `save/restore_layout`; named
  workspaces; reset; min-canvas guard; collapse-to-icon.
- **P3** — multi-viewport floating (multi-monitor); Bottom Dock Area enabled
  when its first panel ships.

---

## 9. Open questions

- Exact drop-zone visuals/threshold (tune against `egui_dock` defaults vs custom).
- Whether the Left Rail is itself a `DockHost` region or a fixed structure (lean:
  fixed structure, but rails are still command-registry-driven).
- Tab overflow behavior (scroll vs "»" overflow menu) — decide in P1.
