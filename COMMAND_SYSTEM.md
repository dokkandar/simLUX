# COMMAND_SYSTEM

The `CommandRegistry` — the **single source of truth** from which every command
surface is generated. Parent: [ARCHITECTURE.md](ARCHITECTURE.md) §4.

> Status: **Proposed.** Today commands are dispatched ad-hoc through
> `run_command` + hand-authored rails/menus + the parser. This doc is the target:
> one registry, many surfaces.

---

## 1. Principle

A command is defined **once** and appears **everywhere** automatically. The
registry is the authoritative source for:

- Navigation · Menu hierarchy · Toolbar/rail actions · Context menus
- Command palette · Keyboard shortcuts
- Panel commands · Plugin commands
- Search indexing · (future) Permissions · (future) Command metadata

UI components **render themselves from the registry** wherever possible —
they never hold their own command lists.

---

## 2. The `Command` model

```rust
// Interface sketch — not final.
pub struct Command {
    pub id: CommandId,              // stable, namespaced: "draw.line", "layer.new"
    pub title: &'static str,        // "Line", sentence case, verb-first
    pub category: Category,         // Draw, Modify, Layer, View, Theme, …
    pub aliases: &'static [&'static str],   // "L", "LINE" — command-line typing
    pub shortcut: Option<Accel>,    // Ctrl+Z, F3 — true keyboard accelerators
    pub icon: Option<IconId>,       // for rails/menus
    pub enabled: fn(&Ctx) -> bool,  // greys out / hides when false
    pub visible: fn(&Ctx) -> bool,  // context filter (see §5)
    pub run: CommandHandler,        // the effect; pushes UndoEntry if mutating
    // future: permissions, telemetry tag, help text, params schema
}
```

- **`id`** is stable and namespaced (`panel.verb`); surfaces, shortcuts, and
  persistence reference it. Renaming a title never breaks bindings.
- **`aliases`** feed the command line; **`shortcut`** is the OS-style accelerator.
  Tooltips show both (`Undo (U · Ctrl+Z)`), per the design-system nav rules.
- **`title`/`category`** are content-governed: sentence case, verb-first, no
  terminal punctuation.

---

## 3. The `CommandRegistry`

```rust
pub trait CommandRegistry {
    fn register(&mut self, source: SourceId, cmd: Command);
    fn unregister_source(&mut self, source: SourceId);   // plugin/panel unload

    fn get(&self, id: CommandId) -> Option<&Command>;
    fn by_category(&self, c: Category) -> Vec<&Command>;
    fn search(&self, query: &str, ctx: &Ctx) -> Vec<CommandHit>;  // palette
    fn run(&mut self, id: CommandId, ctx: &mut Ctx);
}
```

- Aggregates commands from **global** registration + every **panel**
  (`Panel::commands()`) + every **plugin**.
- `source: SourceId` lets a panel/plugin's commands be removed wholesale when it
  unloads.
- `search` powers the command palette with fuzzy match over title + aliases,
  filtered by `visible`/`enabled` for the current `Ctx`.

---

## 4. One command, many surfaces

Each surface is a **view** over the registry:

| Surface | Built from |
|---|---|
| Rails (left) | category/order config → `Command`s (icons + tooltips). |
| Menu bar | category tree → `Command`s (labels, shortcuts, marks). |
| Context menu | `visible(ctx)` filtered `Command`s for the right-click target. |
| Command palette | `search(query, ctx)`. |
| Keyboard | the `shortcut`/`alias` → `id` map. |
| Command line | `aliases` → `id`, then param parsing (see §6). |

Adding a command makes it appear in all applicable surfaces with no edits to
those surfaces. The rail's *which icons + order* and the menu's *category tree*
are **configuration** (user-customizable, persisted), not code.

---

## 5. Context-sensitive commands

Commands carry `visible(ctx)` / `enabled(ctx)` predicates. The **Inspector** and
other panels emit commands dynamically from `Panel::commands()` based on state —
e.g. with a line selected: `inspector.set-length`, `inspector.set-angle`,
`layer.assign`; with nothing selected those simply aren't produced. This is how
"different commands per selected dobject" works without special-casing surfaces.

---

## 6. Command-line integration

The command line resolves typed input against `aliases` → `Command`, then runs
its parameter flow. This unifies with the existing parser and variable registry:

- Bare command (`line`, `AR`) → start the command's flow.
- Sub-options / values continue through the command's own prompt flow.
- Variables (`setvar`, bare var name) remain a command category, so the existing
  `varreg` surface is just another set of registered commands.
- `U` / internal undo and other command-flow keywords are handled by the active
  command, not the registry (see [COMMAND_LINE.md](COMMAND_LINE.md)).

---

## 7. Shortcuts & conflicts

A single **shortcut map** (`Accel → CommandId`) is derived from the registry.
Conflicts are detected at registration (two commands claiming the same accel) and
surfaced as a warning; user overrides live in settings and win over defaults.
Command-line-first means most keys flow to the command line — accelerators are a
curated set (undo/redo, panel toggles, view ops), not every command.

---

## 8. Plugin commands

Plugins register `Command`s through the same `register(source, cmd)` API
([PLUGIN_API.md](PLUGIN_API.md)). They appear in every surface and the palette
identically to built-ins. Unloading a plugin calls `unregister_source`.

---

## 9. Future-facing fields (reserved)

`permissions` (who may run), `metadata` (help text, docs link, telemetry tag),
and a `params` schema (typed args for palette/automation) are reserved on
`Command` now so adding them later is non-breaking.

---

## 10. Phased plan

- **P1** — `Command` + `CommandRegistry`; migrate Draw/Modify into it; generate
  the **rails** from it. Keep menus hand-authored temporarily.
- **P2** — generate menus + context menus + palette from the registry; shortcut
  map + conflict detection; user-customizable rail/menu config (persisted).
- **P3** — plugin command registration; reserved fields (permissions/metadata).
