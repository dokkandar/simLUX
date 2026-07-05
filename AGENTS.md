# AGENTS.md — Coding Guidelines for AI Agents

Mistakes that happened in PR #1 (dokkandar → auto_rasm integration). Do not repeat these.

## Architecture Rules

### 1. Host crate MUST NOT depend on plugin-internal crates
The host (`src/`) links only `dobject`. All plugin logic lives behind `.so` FFI.
If the host needs data from a `*_core` crate, export it through the corresponding
`plugin_*` crate as `#[no_mangle] pub extern "C"` functions.

❌ `src/app.rs` calling `hatch_core::patterns::PATTERN_NAMES` or `hatch_core::fill::*`
✅ `src/app.rs` calling FFI exports from `plugin_hatch` (e.g., `hatch_pattern_count`, `hatch_trace_boundary`)

### 2. No dual render paths — GPU (wgpu) only
All shape rendering goes through `WgpuState::render()`. Never add CPU-side egui
`add_shape()` loops for dashed lines, outlines, or any other visual style.
Linetypes/hatches render through their existing plugin pipelines — if something
doesn't draw, fix the pipeline, don't add a parallel CPU path.

### 3. Remove dead code, don't `#[allow]` it
Dead fields in structs, unused enum variants, dead ribbon glyphs, commented-out
save format versions — remove them. Never `#[allow(dead_code)]` on something
that has no future use case.

### 4. Single flat save format — no version branching
`.rsm` save/load uses ONE format. No `if version < N` branches. If a field is
removed, remove it from both save and load completely. No backward compat cruft
unless a format has actually shipped to users.

## GPU Rendering Checklist

### 5. `sd.visible` MUST be checked in the GPU instance builder
Setting `sd.visible = false` in app.rs is not enough. The instance-building loops
in `wgpu_state.rs` (`update_instances`) must skip invisible shapes:
```rust
for sd in shapes {
    if !sd.visible { continue; }
    // ...
}
```
All three code paths: `do_full`, `has_dirty`, and the fallback `else` branch.

### 6. Any UI change that affects the canvas must call `request_repaint()`
When toggling layer visibility, frozen, changing colors, adding/removing shapes,
or any other state that changes what the GPU renders:
```rust
ui.ctx().request_repaint();
```

## Code Quality

### 7. No nightly-only features
The project targets stable Rust. `std::panic::Location::caller()` is nightly-only.
Use `file!()` + `line!()` macros instead, or the `#[track_caller]` attribute for
panic context.

### 8. Check plugin tool name uniqueness (and that prefix matching doesn't collide)
Every `register_tool` call must use a name that doesn't collide with any other
plugin. If two plugins offer "Polyline", one must be renamed. Also: `resolve_tool`
uses case-insensitive exact-match first, then `starts_with` fallback. Avoid names
that are prefixes of each other (e.g., "Ellipse" and "Ellipse Arc").

### 9. Add undo support for every new operation
Every new shape creation, modification, delete, layer change, or operation must
have a corresponding `UndoEntry` variant. If you add a hatch, offset, fillet, or
similar operation, add `UndoEntry::HatchOp(undo)`, etc.

### 10. Never silently swallow failures
Fillet, chamfer, hatch, offset — if the operation fails (wrong selection count,
invalid geometry, etc.), surface it to the user via the status bar:
```rust
self.status_msg = Some("Fillet requires exactly 2 lines".into());
```
Never `eprintln!` and continue silently.

### 11. Hot-path performance
- `O(N^2)` in rendering/trace loops → use spatial hash or r-tree
- No `.clone()` inside rendering hot loops
- Large iteration limits must have overflow warnings (e.g., `MAX_TRACE_STEPS`)

### 12. Keep layer conventions minimal
No mandatory BASE/0 layer, no protected-layer-name logic. One default layer
("Layer 1") on new document. The user creates/deletes/reorders layers freely.
`ensure_minimum_layers()` ensures at least one layer exists — nothing more.

## Render-Performance Rules

These caused a 4-second-per-frame regression with ~1M shapes. EVERY rule below
was violated at some point. Do not repeat.

### 13. No O(N) loops in the render block that run every frame
The render block enters on every repaint (cursor movement, pan, zoom). Any loop
over `self.shapes` inside it must be skipped on stable frames. Example: layer-color
pre-processing and visibility filtering must be gated behind a dirty flag.

✅ `if self.needs_color_apply { for sd in &mut self.shapes { ... } self.needs_color_apply = false; }`
❌ `for sd in &mut self.shapes { ... }` — running unconditionally

### 14. Cache per-layer instance buffers — skip rebuilds on stable frames
Instance data (`cached_counts`, `instance_chunks`, `bind_groups`) must be keyed
by `(PluginId, layer)` not just `PluginId`. Each layer's data survives
independently across passes and frames. On stable frames (no shape/layer changes),
`update_instances` must return immediately without iterating a single shape.

### 15. Pass `plugin_ids` must only include plugins that actually have shapes
A pass grouping ALL registered fill/stroke plugins causes the per-layer cache
check to fail (plugins with zero shapes in that layer have no cache entries).
Track per-layer `HashSet<PluginId>` during the shape scan.

❌ `passes.push(PassDescriptor { plugin_ids: all_fill_ids.clone(), ... })` — includes empty plugins
✅ Collect only `pid`s that appear in this layer during the scan, pass those

### 16. Snap R-tree queries must use 1x search radius unless PER/TAN active
The snap engine queries `self.shape_index` on every hover frame. The wide 20×
radius is only needed for Perpendicular/Tangent extension snaps. Without those
active, use `world_radius * 1.0` — otherwise thousands of candidates get their
`snap_geometry()` computed every frame.

### 17. Invalidate bind groups when instance chunks are recreated
When `ensure_instance_chunks` recreates GPU buffers (capacity changed between
layer passes), the old bind groups still reference stale buffers. Call
`self.bind_groups.remove(&key)` whenever chunks are cleared/recreated.

### 18. Shape sort, pass building, and color application must be conditional
- `self.shapes.sort_by(...)` only when layers or shapes change (`layout_changed`)
- `self.cached_passes` rebuild only when layout changes
- Layer color apply/restore must run every frame the render block executes
  (skipping it causes cached instance data to go stale with wrong colors)

### 19. Skip invisible/frozen layers at pass-build time — not per-shape
If a layer's `shown()` returns false, don't generate any passes for it. No need
to iterate all shapes setting `sd.visible = false` as a pre-processing hack.
The per-layer `layer_filter` already isolates each pass to its layer.

## File Organization

### 20. All shape data lives in `src/app.rs`
`ShapeData` struct, layer definitions, save/load, undo/redo — all in `src/app.rs`.
Don't split these across multiple files. Module-level concerns (snapping,
grid, ribbon) can have dedicated files.

### 21. DObject is the ONLY host-plugin interface
`dobject/` defines `DObject` trait. All shape types implement it. The host only
interacts with shapes through `DObject` methods. Never add host-specific fields
to plugin crates.
