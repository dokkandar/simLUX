# RUST_CAD — Top Menu Bar (wiring to command line, kernel & libraries)

> Instruction doc for another coding agent. Documents the top **menu bar**
> (`File / Edit / View / Draw / Modify / Dimension / Wall / Styles / Tools /
> Help`) and — the part you asked for — **exactly how each item is wired**: most
> go through the command line (`run_command`) into `cad_kernel` / the feature
> crates; a few call app methods or flip UI flags directly.
>
> Code: `cad_app/src/app.rs` — `TopBottomPanel::top("menubar")` @19226–19735
> (right above the ribbon at `"toolbar"` @19738). Line numbers from `df95549`.
> Read alongside `COMMAND_LINE_CURRENT.md` (the CLI it feeds) and
> `RIBBON_GUIDE.md` (the icon strip; same `run_command` wiring).

---

## 1. The one idea to take away

**A menu item is almost always just a typed command with a label.** The menu bar
is a *discoverable front-end to the command line*. The overwhelmingly common arm
is:

```rust
if ui.button("Trim").clicked() { self.run_command("trim"); ui.close_menu(); }
```

That single line ties the menu to **everything** the CLI already does: the
parser (`cad_kernel::parser::parse`), the dispatch `match`, the kernel op, the
Session Recorder (logs it as a `CmdRun`), and empty-Enter repeat (`last_command`).
So **menu = command line = kernel**, with no duplicated logic. Keep it that way.

---

## 2. The four wiring patterns (the taxonomy)

Every menu item is exactly one of these:

| # | Pattern | Code shape | When to use | Where it lands |
|---|---------|-----------|-------------|----------------|
| **A** | **→ command line** | `self.run_command("word")` | anything that already has a CLI verb (draw/modify/dim/wall/inquiry) | `parser.rs` → dispatch → `cad_kernel` / feature crate |
| **B** | **→ app method** | `self.zoom_extents()`, `self.copy_selection()`, `self.open_file_dialog(..)`, `self.ensure_index()` | view ops, clipboard, file dialogs, index — things with no CLI verb | a `CadApp` method, which itself calls the kernel/`cad_io` |
| **C** | **→ panel / flag toggle** | `self.settings_open ^= true`, `ui.checkbox(&mut self.layers_window_open, …)`, `self.env.GrpEnb = !..` | open a palette / flip a setting | UI state (+ `env.save()` for settings) |
| **D** | **→ direct field set** | `self.current_dim_style = id`, `self.scale = 20.0` | quick state switch (current style, reset view) | `CadApp` fields |

Rule of thumb: **prefer A.** Use B/C/D only when there is genuinely no command
verb (pure UI). If you find yourself writing op logic inside a menu closure,
stop — add a CLI verb and route the menu through it.

---

## 3. The full menu map (label → wiring → kernel/library)

### File
| Item | Pattern | Wiring | Lands in |
|------|---------|--------|----------|
| New (non-parametric) | A | `run_command("clear")` | `clear_all()` |
| New parametric sketch | B | `clear` + `param_editor::ParamSession::new()`, `parametric.active = true` | **`cad_param`** crate |
| Open .dxf / .rsm / .dwg… | B | `open_file_dialog(Open, ".dxf")` | **`cad_io`** (`dxf.rs` / `rsm.rs`) on load |
| Save (current file) | B | `do_save_current()` | **`cad_io`** writer |
| Import ▸ Image as raster | B | `open_file_dialog(ImportRaster, "")` | **`cad_raster`** + `image` crate |
| Import ▸ Image → vector | B | `open_file_dialog(ImportImage, "")` | **`cad_raster`** trace editor |
| Import ▸ Detach raster | B | `snapshot_doc()` + `doc.raster_images.clear()` | `cad_kernel::Document` |
| Save As .dxf / .rsm… | B | `open_file_dialog(Save, ext)` | **`cad_io`** writer |
| Exit | — | `ctx.send_viewport_cmd(Close)` | eframe |

### Edit
| Item | Pattern | Wiring |
|------|---------|--------|
| Undo / Redo | A | `run_command("undo" / "redo")` → undo/redo stacks |
| Copy / Paste | B | `copy_selection()` / `start_paste()` (+ `ctx.copy_text`) |
| Group / Add to Group / Ungroup | B | `group_selection()` / `add_to_group()` / `ungroup_selection()` |
| Select All | A | `run_command("select")` then `run_command("all")` |
| Deselect All | D | `selection.clear(); selected = None` |
| Erase selection | A | `run_command("erase")` |
| Match Properties | A | `run_command("matchprop")` |

### View
| Item | Pattern | Wiring |
|------|---------|--------|
| Zoom Extents | B | `zoom_extents()` |
| Zoom Window | B | `zoom_start("w")` (the ZOOM flow) |
| Zoom Previous | B | `zoom_previous()` |
| Reset View | D | `view_push_history(); scale = 20.0; world_offset = 0` |

### Draw  *(data-driven `for (label, cmd)` loop → all pattern A)*
`Line→"line"`, `Rectangle→"rec"`, `Circle→"circle"`, `Arc (3pt)→"arc"`,
`Ellipse→"ellipse"`, `Ellipse Arc→"ellipsearc"`, `Polyline→"polyline"`,
`Spline→"spline"`, `Point→"point"`, `Hatch…→"hatch"`. Plus `Block…→"block"`
and an **Insert Block** submenu that lists `doc.blocks` and fires
`run_command("insert <name>")`. → `cad_kernel::{parser, geom, construct}`;
hatch also uses **`cad_io::pat`** + `hatch_trace`; blocks use
`cad_kernel::block`.

### Modify  *(data-driven loops → pattern A)*
Transform: `Move/Copy/Rotate/Scale/Mirror/Stretch/Align`. Edit-geometry:
`Trim/Extend/Fillet/Chamfer/Offset/Join/Break/Lengthen "lengthen 1"/Reverse`.
Then `Array…` (C: `array_open = true`), `Explode` (A: `"explode"`),
`Properties…` (A: `"props"`), `Match Properties`/`Change Layer` (A), `Erase` (A).
→ transforms: `cad_kernel::geom` (`translated`/`rotated`/`scaled`/`scaled_xy`);
trim/extend: `cad_kernel::trim`; fillet/chamfer/offset: `cad_kernel::modify`;
join: `cad_kernel::join`.

### Dimension
`Dimension (smart)` → A `"dim"`; `Dimension Style…` → A `"dimstyle"`. →
`cad_kernel` dimension + dim-style table.

### Wall
`Wall` → A `"wall"`; `Wall Style…` → A `"wallstyle"`. → **`cad_wall`**
(`solve_faces` / `solve_face_segments`) + `cad_kernel::wallstyle`.

### Styles
- **Managers** (data-driven): `Text Style…→"style"`, `Dimension Style…→"dimstyle"`,
  `Wall Style…→"wallstyle"` (pattern A).
- **Current** quick-switchers: `Dim style ▸` / `Wall style ▸` submenus →
  pattern D (`current_dim_style = id` / `current_wall_style = id`); setting a wall
  style also syncs `env.WlThk = style.thickness` + `env.save()`.
- `Opening Style… (planned)` — `add_enabled(false, …)` honest-disabled placeholder.

### Tools
- **Palettes**: `ui.checkbox(&mut self.<window>_open, …)` (pattern C) for command /
  layers / pens / info / dobjects windows. `Snap window`, `Toggle Grips`
  (`env.GrpEnb` + save).
- `Text Style…` → A `"style"` (+ a `MenuClick` recorder event).
- `🛰 Session Recorder` checkbox → C `dbg_window_open` (+ `MenuClick` event).
- **Inquiry**: `Distance→"dist"`, `List selected→"list"` (pattern A).
- **Debug tools ▸** submenu: render/screen-stats/trim/hatch debug checkboxes (C);
  `Rebuild spatial index` → B `ensure_index()`; intersect visualizer flags (C);
  `Clear all (DESTRUCTIVE)` → B `clear_all()`.

### Help
`Command help` → A `"help"`; `About` → D (push two `history` lines).

---

## 4. End-to-end chains (menu → CLI → kernel)

**Draw ▸ Line**
```
ui.button("Line") → run_command("line")
  → cad_kernel::parser::parse("line")  →  Command::SetTool(ToolKind::Line)
  → dispatch sets self.tool = Tool::Line
  → canvas clicks accumulate → add_dobject(Geom::Line{..})   // cad_kernel::geom
```

**Modify ▸ Trim**
```
ui.button("Trim") → run_command("trim")
  → parse → Command::Trim → trim state machine (select cutters → pick targets)
  → cad_kernel::trim::trim_at(...)                            // the actual cut
```

**Modify ▸ Join**
```
"join" → Command::Join → selection session → cad_kernel::join::join_geoms(...)
```

**Wall ▸ Wall**
```
"wall" → Command::Wall(thickness) → wall draft → cad_wall::solve_faces / solve_face_segments
```

**File ▸ Open** (pattern B, no CLI verb)
```
open_file_dialog(Open) → file picked → cad_io::dxf::read_dxf / rsm::read_rsm → Document
```

The point: for pattern-A items the menu adds **zero** new logic — it reuses the
CLI's path into the kernel. The Session Recorder, history echo, and
repeat-last-command all light up for free.

---

## 5. Recipe — add a menu item like ours

1. **Does the action have a command verb?**
   - **Yes →** `if ui.button("Label").clicked() { self.run_command("verb"); ui.close_menu(); }`
     (pattern A). If the verb doesn't exist yet, add it in `parser.rs` +
     dispatch first (see `COMMAND_LINE_CURRENT.md` §13), then wire the menu to it.
   - **No (pure UI) →** call a `CadApp` method (B), toggle a flag (C), or set a
     field (D). Keep any real work in the method, not the closure.
2. **Always `ui.close_menu()`** after handling a click (except `ui.checkbox`,
   which stays open so the user can flip several).
3. **Group with `ui.separator()`** and label clusters with a small grey
   `RichText::small()` header (see Styles/Tools).
4. **Don't borrow `doc` inside a submenu closure** — snapshot the names first
   (`let dim_names: Vec<_> = …collect();`) then act on the picked id *after* the
   closure (see the Dim/Wall current-style switchers @19554).
5. **Data-drive repetitive groups** with `for (label, cmd) in [ … ] { if
   ui.button(label).clicked() { self.run_command(cmd); ui.close_menu(); } }`
   (Draw/Modify/Styles managers do this).
6. **Honest-disable** planned items with `ui.add_enabled(false, Button::new("… (planned)"))`
   + `on_disabled_hover_text(...)` instead of hiding them.
7. (Optional) emit a `DbgEvent::MenuClick { path: "Tools → …" }` for items worth
   tracing in the recorder.

---

## 6. Conventions & gotchas

- **The menubar is its own `TopBottomPanel::top("menubar")` declared *before*
  the `"toolbar"` panel**, so it sits at the very top. Wrap items in
  `egui::menu::bar(ui, |ui| { ui.menu_button("File", |ui| { … }); … })`.
- **Menu, ribbon, and typed command must all reach the same `run_command`.** If a
  menu item does something the CLI can't, that's a smell — add the verb.
- **`run_command` does the snapshotting/undo** for ops that mutate the doc; if you
  go pattern B and mutate `doc` directly (e.g. Detach raster), call
  `self.snapshot_doc()` yourself first so Undo works.
- **Settings toggles persist**: pair the flip with `let _ = self.env.save();`
  (e.g. Toggle Grips), matching the settings window.
- **Current-style switchers** set `current_dim_style` / `current_wall_style`
  AND sync any dependent SYSVAR (wall style → `env.WlThk`) so the next draw uses
  it — mirror whatever the Style Manager's "Set Current" does.
- Right-click **context menus** (canvas Group / Clipboard submenus @20867) follow
  the exact same patterns — reuse them there too.
