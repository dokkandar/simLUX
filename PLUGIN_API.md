# PLUGIN_API

Extension points: panel registration, command registration, and the future
plugin contract. Parent: [ARCHITECTURE.md](ARCHITECTURE.md) §4, §7.

> Status: **Proposed.** No plugin loader exists yet. The internal registries
> (`PanelRegistry`, `CommandRegistry`, `ThemeStore`) are designed *now* to be the
> extension surface, so a loader can be added later without redesign.

---

## 1. Two extension axes

There are two distinct plugin surfaces; do not conflate them:

1. **UI extensions (this doc)** — add **panels** and **commands** at the app
   layer. Owns the Dock Area / Command Registry contract.
2. **Entity / tool extensions** — add `Geom` variants / tools at the kernel
   layer via the `DObject` trait and a future `.so`/FFI boundary
   ([AGENTS.md](AGENTS.md)). Kernel-level, separate lifecycle.

Both can coexist; this doc specifies axis 1 and keeps it FFI-expressible for the
day axis 2's loader lands.

---

## 2. The extension contract

A plugin (built-in module today, dynamic module tomorrow) is given a registration
context and registers capabilities under its own `SourceId`:

```rust
// Interface sketch — not final.
pub trait Extension {
    fn id(&self) -> SourceId;                 // "com.hsi.layers", "acme.bom"
    fn register(&self, reg: &mut Registrar);
}

pub struct Registrar<'a> {
    pub panels:   &'a mut dyn PanelRegistry,    // add tool panels
    pub commands: &'a mut dyn CommandRegistry,  // add commands
    pub theme:    &'a dyn ThemeStore,           // READ tokens (no write)
    // future: menus/rails config hooks, settings schema, file-format hooks
}
```

Everything an extension contributes is keyed by its `SourceId`, so **unload =
`unregister_source(id)`** across all registries — panels disappear from the dock,
commands from every surface, with the layout dropping unknown ids gracefully.

---

## 3. What extensions may and may not touch

| May | May not |
|---|---|
| Register `Panel`s (own UI) | Import or call the dock engine (`egui_dock`) |
| Register `Command`s | Hard-code colors/sizes — must read tokens |
| Read `DesignTokens` | Mutate other extensions' panels/commands |
| Read selection / document via `AppCtx` API | Reach into `cad_app` internals directly |
| Add file-format hooks (future) | Block the UI thread / canvas frame |

The boundary is the registries + a stable `AppCtx` facade. Extensions never see
the layout engine, the monolith, or each other.

---

## 4. Capabilities surfaced automatically

Once registered, an extension's contributions get — for free — everything the
core systems provide:

- **Panels**: open/close, dock/float, drag-dock, persistence, Window-menu entry
  ([PANEL_SYSTEM.md](PANEL_SYSTEM.md)).
- **Commands**: rails/menus/context-menus/palette/shortcuts/search indexing
  ([COMMAND_SYSTEM.md](COMMAND_SYSTEM.md)).
- **Persistence**: layout + panel state survive restart; unknown ids dropped
  safely when the plugin is absent.

No layout, command-surface, or persistence code changes when an extension is
added — the whole point of the registry architecture.

---

## 5. FFI readiness (future)

To allow out-of-process / `.so` plugins later, the contract stays expressible
across an FFI boundary:

- `SourceId`, `PanelId`, `CommandId` are stable strings.
- `Command::run` and `Panel::ui` are dispatched through handles, not Rust
  closures crossing the boundary — a C-ABI shim can forward them.
- `DesignTokens` is serializable (plugins receive a snapshot).
- Registration is data-first (descriptors) so a host can validate/version it.

This mirrors AGENTS.md's `register_tool`/`resolve_tool` and `#[no_mangle] extern
"C"` direction for the entity axis; the UI axis reuses the same discipline.

---

## 6. Versioning & safety (future)

- **API version** advertised by the host; extensions declare a min version.
- **Capability allow-list** per extension (reserved `permissions` on `Command`).
- **Crash isolation** for out-of-process plugins (axis 2 / FFI).
- **Settings schema**: extensions may register typed settings (reuse the
  variable-registry pattern, [Variables.md](Variables.md)).

---

## 7. Phased plan

- **P1** — formalize `PanelRegistry` + `CommandRegistry` with `SourceId`; move
  built-in panels/commands to register through them (dogfood the API internally).
- **P2** — `Extension` trait + `Registrar`; an in-tree example extension
  (e.g. a sample panel) proving add/remove with no layout changes.
- **P3** — dynamic loading + FFI shim + versioning + permissions, aligned with
  the AGENTS.md entity-plugin axis.
