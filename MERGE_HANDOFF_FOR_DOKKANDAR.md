# Merge handoff — pulling HSI `windows-ui-session-2026-06-20` into `dokkandar/Auto_RASM`

**From:** HSI-Lighting/RUST-AutoRASM · **Date:** 2026-07-03
**What HSI added since our last shared point (`e1b2380`):** the **Command Registry** (Phases 0–7 — a metadata registry driving menus / rails / the command palette) plus an Inspector redesign. HSI *also* pulled in all of your GPU / raster / param / DWG work (so those features now exist on both forks).

Merging our branch gives you the **command registry** (the one thing you don't have) and reconciles the two histories so future syncs are clean.

---

## TL;DR

```bash
git fetch https://github.com/HSI-Lighting/RUST-AutoRASM.git windows-ui-session-2026-06-20
git checkout -b reconcile FETCH_HEAD~0        # or merge into your main
git merge <that-ref>
```

- **Only ONE file conflicts: `cad_app/src/app.rs`** (~16 hunks). Everything else auto-merges — HSI took your backend crates (`cad_kernel/io/wall/nurbs`, `cad_param`, `cad_raster`, `tools/`, docs) verbatim, so they're identical on both sides.
- **Default rule for `app.rs`: take HSI's side (`HEAD`/`ours`).** HSI's `app.rs` is a superset — it's *your* features re-integrated **through the command registry**. Our menu/rail/palette/status-bar code carries the registry wiring you need.
- **Three exceptions where you keep YOUR side** — see the table.

---

## Conflict-by-conflict (in `cad_app/src/app.rs`)

| Region (search text) | What it is | Resolve |
|---|---|---|
| `explode: select blocks / walls / polylines` | Explode prompt string | **Ours** (superset wording) |
| `do_open` … `.dwg` branch, `dbg_event!(self, … DWG open` | DWG-open converter + **your `dbg_event!` recorder logging** | **YOURS** — we stripped the `dbg_event!` calls only because our fork lacks your enhanced `dbg_recorder`. You have it → keep your logging. Keep our `convert_dwg_to_dxf` call structure. |
| `} else if shows {` (file-list filter) | Open lists `.dxf/.rsm/.dwg`; import modes list images | **Ours** (registry/import-aware filter) |
| file-dialog `Type`/`Open` button arms, `FileDialogMode::Open =>` | `FileDialogMode` match arms incl. `ImportImage`/`ImportRaster` | **Ours** |
| `block: id, insert: base, scale: 1.0, scale_y: 1.0` (×3 `BlockRef {…}`) | `BlockRef.scale_y`/`mirror_x` fields | **Either** — identical on both sides; take ours to be safe |
| `// CPU / GPU / APX are one mutually-exclusive axis` (debug window) | Render-mode **radios** (CPU/GPU/APX) | **Ours** |
| `// ---- Render-mode badges: [CPU] [GPU] [APX]` (status bar) | Render-mode **badges** (direct-select via `set_render_mode`) | **Ours** |
| `self.render_param_panel(ctx);` / `self.sync_underlay_textures(ctx)` | update() + canvas hooks for param / raster | **Either** — same calls; take ours |
| `fn wall_face_screen_pts` ↔ `fn wall_face_world_pts` (+ `to_screen`, `solve_face_segments`) | **Wall face rendering.** You refactored to world-space `wall_face_world_pts` (shared CPU/GPU) with X-junction `solve_face_segments`. HSI kept a screen-space `wall_face_screen_pts` and a *separate* GPU `wall_face_world_pts` built on `solve_faces` (X-junction splitting **not** ported). | **YOURS** — your world-space architecture is more complete. After taking yours, delete HSI's duplicate `wall_face_world_pts` (the `solve_faces` one, near the GPU-merge helper block) so there's a single definition; the GPU render block already calls `wall_face_world_pts(self, w)`, which resolves to yours. |

> Net: **take ours everywhere except (1) the `dbg_event!` lines in the `.dwg` open branch, and (2) the wall-face-rendering block — take yours there.**

---

## After resolving

```bash
cargo check --workspace     # must be green (0 errors)
cargo build  -p cad_app     # links rust_cad
```

Watch for a **duplicate `wall_face_world_pts`** (E0428) — that's the one HSI added for the GPU path; delete it and keep yours.

## What you gain
The **Command Registry**: `cad_app/src/command.rs` (a `CommandInfo` metadata registry — id/dispatch/title/icon/keywords/section/visible/enabled) + `execute(id)` seam, driving the Draw/Modify rails, the menus (curated ordered id-lists), and a **fuzzy command palette** (Ctrl+Shift+P). `run_command` and the parser are untouched. See `mentor MD/COMMAND_REGISTRY_MENTOR.md` on the branch.

Questions → HSI (dev@hsilighting.com).
