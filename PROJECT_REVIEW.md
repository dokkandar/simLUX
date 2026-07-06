# PROJECT REVIEW — RUST-AutoRASM (for an incoming AI agent)

Read this first. It is a status + orientation brief so another agent can pick up
the project without re-discovering everything. It states **what the project is,
how it's built, what's done, what's in flight, what's parked, and the rules that
must not be broken.** Written 2026-07-01, last updated 2026-07-04, against
branch `windows-ui-session-2026-06-20`.

> **Since 2026-07-02 two large tracks landed:** (1) the **Command Registry**
> (Phases 0–7, complete) and (2) a **merge from the sibling fork
> `dokkandar/Auto_RASM`** bringing the full GPU renderer, two new crates
> (`cad_param`, `cad_raster`), DWG open, a raster→vector editor, and parametric
> mode. See §7. Both are pushed; features await the owner's visual verification.

For deep detail, follow the links into the topic docs (indexed in §12). This doc
is the map, not the territory.

---

## 1. What the project is

**RUST-AutoRASM** — a 2D CAD application (AutoCAD-like) written in **Rust**, GUI
in **egui / eframe** (immediate-mode). It draws/edits *dobjects* (lines, circles,
arcs, ellipses, polylines, splines, hatches, walls, text, dimensions, blocks) on
a GPU-accelerated canvas, with an AutoCAD-style command line, left tool rails, a
right **Inspector** (properties) panel, layers, linetypes, and a variable system.

- Owner/context: HSI Lighting. Developer works on **Windows (MSVC toolchain)**;
  the project was originally developed on Arch Linux, so a few platform shims
  exist (see `PROJECT_NOTES.md`).
- Terminology is deliberate: the app calls its entities **"dobjects"** (never
  "objects") and the properties panel is the **"Inspector"** (never
  "Properties"). Honor this in UI text and docs.

---

## 2. Repo / workspace layout

Cargo workspace, members:

| Crate | Role |
|---|---|
| `cad_kernel` | Geometry + document model. `math.rs` (`Vec2`), `geom.rs` (`Geom` enum: Line/Circle/Arc/…), fillet, trim, modify, snap, parser. **No UI.** |
| `cad_snap` | Object-snap solving. |
| `cad_nurbs` | Spline/NURBS math. |
| `cad_io` | File I/O (DXF etc.). |
| `cad_wall` | Architectural wall logic (centerline → side lines; X-junction face solver). |
| `cad_param` | **(new, from the dokkandar merge)** Independent 2D geometric constraint solver for parametric sketches. Depends on `cad_kernel` for `Vec2` only; the core never depends on it. |
| `cad_raster` | **(new, from the dokkandar merge)** Raster→vector subsystem (image decode, analyze, layer-by-layer carve, trace to dobjects). Depends on `cad_kernel` + the `image` crate. |
| **`cad_app`** | **The application** — egui UI, tools, rendering, all interaction. This is where ~all recent work lives. |
| `cad_cli` | A headless CLI harness for the kernel. |

### `cad_app/src` modules
| File | What |
|---|---|
| **`app.rs`** | **The monolith (~30,200 lines).** `CadApp` struct + `eframe::App::update`, every tool, the canvas, all panels/menus, the Inspector. Most work happens here. |
| `main.rs` | Entry point + module list + `eframe` window setup. |
| `command.rs` | **Command Registry** (Phases 0–7) — a `CommandInfo` metadata registry (id/dispatch/title/icon/keywords/section/visible/enabled) + `Ctx` projection. Drives rails, menus, and the palette; `run_command`/parser untouched. See §7. |
| `param_editor.rs` | **(new)** Parametric constraint session/solver UI backing `cad_param` (standalone module). |
| `dock.rs` | **Unified docking abstraction** (`DockHost` trait + `EguiDockHost`). See §5. |
| `theme.rs` | **Design tokens** (color/spacing/radius + `typ` type-scale tokens) + `theme::apply(ctx)` global Visuals + `theme::install_fonts(ctx)` (embeds Geist + Geist Medium + JetBrains Mono). Single source for the look. |
| `gpu.rs` | **GPU renderer** — 5 instanced pipelines (circle / analytic arc / analytic ellipse / line / triangle-fill), camera-relative f32 coords. See §4. |
| `settings.rs` | Settings/variables page. |
| `varreg.rs` | Variable registry (240 vars; single source of truth for settings). |
| `aci_picker.rs` | ACI color-wheel picker. |
| `hatch_trace.rs` | Hatch boundary tracing worker. |
| `dbg_recorder.rs` | Session recorder — timestamped event log; the user dumps `=== SESSION DUMP ===` text to report bugs. |

> **Note on `app.rs`:** it is a very large single file. It is over the 256 KB
> read limit — read it with `offset`/`limit` or grep for the symbol you need.
> Do NOT try to read it whole.

---

## 3. Build & run

```bash
cd /d/HSI_cad_Rust-AutoRASM          # the Cargo.toml lives here, NOT in the parent
taskkill //F //IM rust_cad.exe        # kill any running instance FIRST (Windows exe lock)
cargo build -p cad_app                # binary is "rust_cad"
(cargo run -p cad_app >/dev/null 2>&1 &)   # launch detached
```

Gotchas:
- The binary is named **`rust_cad`**. A running instance **locks the exe**, so a
  rebuild fails with "failed to remove rust_cad.exe" until you `taskkill` it.
- The default shell working dir may be a parent (`D:\AndroidApps`) with no
  `Cargo.toml` — always `cd` into the repo in the same command.
- Windows line-ending warnings on commit (LF→CRLF) are benign.

---

## 4. Architecture at a glance

- **Kernel/app split:** `cad_kernel` owns geometry + the document; `cad_app`
  owns all UI/interaction and calls the kernel. Panels/UI must never reach into
  kernel internals beyond the public API.
- **Immediate-mode UI:** everything re-renders each frame in `update()`. State
  lives on the `CadApp` struct. There is no retained widget tree.
- **The big four dockable surfaces** — **left rails** (Draw + Modify tool icons),
  the **right Inspector**, and the **bottom command bar** — all now render
  through the one docking abstraction (`dock.rs`). See §5.
- **Theme:** `theme::apply(ctx)` installs the token palette as egui's global
  `Visuals` every frame, so menus/dialogs/buttons inherit the teal-navy theme.
  Components should read `crate::theme::*` tokens, not hard-coded hex.
- **GPU:** the canvas has a full instanced OpenGL renderer (`gpu.rs`,
  `GpuShapeRenderer`) — circles/arcs/ellipses drawn analytically (SDF), lines/
  splines/polylines/walls batched to a line pipeline, hatch/poché as fills, with
  camera-relative f32 coords for precision far from origin. **`RenderMode` is one
  mutually-exclusive axis — `Cpu` (egui painter) / `Gpu` (pipelines) / `Apx`
  (every dobject a single instanced dot).** Switch via the status-bar badges or
  Tools ▸ Debug radios (`set_render_mode`). A per-frame CPU draw-budget + advisory
  banners keep heavy drawings responsive.

---

## 5. Docking system — a hard architectural rule

> **The application must never depend on a specific docking engine.** `egui_dock`
> is the preferred *future* engine, but all code is written so another engine can
> replace it with minimal changes. This is a standing constraint from the owner.

Implementation: `cad_app/src/dock.rs`.
- `trait DockHost { fn show(ctx, cfg, state, open, body) -> Rect; }` is the
  **replaceable boundary**. `EguiDockHost` is today's hand-rolled impl; `HOST`
  is the active `const`. To swap engines, add another `impl DockHost` and
  repoint `HOST` — **call sites don't change.**
- `DockConfig` describes a panel: `id`, `title`, `badge` (header chip),
  `dock_region` (**each panel docks to exactly ONE edge** — Inspector=Right,
  command bar=Bottom, rails=Left), size/min/max, `resizable`, `flush_body`
  (panel paints its own padding), `float_w`, `float_max_h_frac`.
- Behavior: drag the header out → floats; drag to the panel's allowed edge and
  **release** → docks (docking only on release, never mid-drag, so a panel never
  grabs the wrong side). Undock lifts the float clear of the edge.
- `header_band()` is the ONE chrome header shared by every bar (title + type
  pill + × close, whole-band drag).

Full rationale/spec: [WORKSPACE_SYSTEM.md](WORKSPACE_SYSTEM.md).

---

## 6. The design system

A full design language was locked and is the authority for all UI:

- **[THEME_SYSTEM.md](THEME_SYSTEM.md) §5** — the **locked token registry**:
  spacing (4px compact scale), radius, teal-navy surfaces, cyan `#00E5FF` accent,
  text colors, type (Geist UI / JetBrains Mono data). Code side = `theme.rs`.
- **[INSPECTOR_DESIGN.md](INSPECTOR_DESIGN.md)** — the **Inspector rule**: every
  pixel value, the panel shell, sections, rows, specialized renderers, states,
  and a per-element measurement table. This is the current implementation target.
- **[inspector_mockup.html](inspector_mockup.html)** — a browser-inspectable
  mockup built to that spec (open in a browser, use devtools to measure).
- Companion architecture docs: [ARCHITECTURE.md](ARCHITECTURE.md) (index),
  [COMMAND_SYSTEM.md](COMMAND_SYSTEM.md), [PANEL_SYSTEM.md](PANEL_SYSTEM.md),
  [PLUGIN_API.md](PLUGIN_API.md), [CONTENT_STYLE.md](CONTENT_STYLE.md).

**Token discipline:** no raw hex or magic spacing numbers in components — add a
named token in `theme.rs` and reference it. Recent bugs traced directly to
values that were documented but never turned into tokens/applied.

---

## 7. Current state — recently completed (this session)

### 2026-07-03/04 — Command Registry + dokkandar merge (newest)

- **Command Registry — Phases 0–7 COMPLETE** (`cad_app/src/command.rs`, authority
  `mentor MD/COMMAND_REGISTRY_MENTOR.md`): a metadata registry (`CommandInfo` =
  id/dispatch/title/tooltip/category/icon/keywords/section/visible/enabled) built
  once; execution flows ONLY through an `execute(id)` seam → `run_command(cmd.
  dispatch)`. **`run_command` and the parser were never modified.** Powers the
  Draw/Modify **rails**, the **menus** (curated ordered id-lists, not category
  dumps), a per-command **method memory** (`command_method`), context
  **predicates** (`visible`/`enabled` fn-pointers over a read-only `Ctx`), and a
  fuzzy **command palette** (Ctrl+Shift+P). Non-negotiables: id ≠ dispatch; the
  registry holds no closures / `&mut CadApp`; `Ctx` is a projection, not a
  gateway. (The old §8 "roadmap: CommandRegistry" item is now this — done.)
- **Merge from `dokkandar/Auto_RASM`** (sibling co-dev fork; shared ancestor
  `e1b2380`). 13 commits pulled their work into our line. Landed: the **full GPU
  renderer** (§4) + `RenderMode::Apx`; **all backend crates taken wholesale**
  (kernel/io/wall/nurbs) + the **new `cad_param` and `cad_raster` crates**;
  **ellipse/ellipsearc parser fix (closed OPEN_ISSUES E1)**; hatch `.pat`
  extractor; grips drag-only; open zoom-to-fit; PLINE Esc; wall X-junction +
  explode; **DWG open** (external ACadSharp converter, cross-platform — added
  `tools/dwgconv/dwgconv.cmd` for Windows); the **raster→vector editor**
  (File ▸ Import ▸ Image, buffer/carve layers, underlays); **parametric
  constraint mode** (File ▸ New parametric sketch). Whole workspace builds +
  links. Method: our fork only ever diverged on `cad_app/{app.rs, command.rs}`,
  so every other file took ext's version conflict-free; only `app.rs` was
  hand-ported (cherry-pick fails — their commits sit on 47 others). **Convergence
  handoff for dokkandar:** [MERGE_HANDOFF_FOR_DOKKANDAR.md](MERGE_HANDOFF_FOR_DOKKANDAR.md).

### Earlier this session — docking / theme / Inspector

- **Unified docking** (`dock.rs`): Inspector, command bar, and both rails all run
  through `DockHost`. Per-panel single-edge docking, drag-float / drag-dock on
  release, unified header + full-width footer, 2-click rail-icon remove, stable
  rail columns (1 or 2, no scrollbar), and a shared theme.
- **Theme-token migration:** `theme::apply` global Visuals themes menus/dialogs;
  panel surfaces migrated off hard-coded hex; accent unified to `#00E5FF`.
- **Inspector redesign toward `INSPECTOR_DESIGN.md`:** header band 40 / title at
  panel-edge / spec type pill; panel padding 16 / 24; 4px rounded fields; spec
  swatches (13×13 @2px, 9px gap); caption section headers; explicit spacing
  tokens; a proper **GEOMETRY** block (editable X/Y + read-only Length/Angle);
  painted dropdown triangles (the `▾` char was tofu); Lt Scale text centered.
- **UI-inspect dev tool** (Tools ▸ Debug ▸ "UI inspect"): a devtools-style ruler.
  Every instrumented element records **rich, HTML-reconstructable** detail (box
  fill/border/radius/size, text content/color/font + measured padding, arrow).
  Hover = multi-line readout; click = logged with full box hierarchy into a
  copyable "UI Inspect Log" window (Copy-dump button). Built specifically so the
  owner can click a mis-sized box and paste an exact report.
- **Menus/popups on surface-3** (2026-07-02, commit `f071617`): `theme::apply`
  set `window_fill = SURFACE_3` (was SURFACE_2). egui 0.30 shares one
  `window_fill` across `Frame::window`/`menu`/`popup` (no menu-specific fill),
  so menus + comboboxes + dialogs all now read the popover tone and lift off the
  surface-1 panels (THEME_SYSTEM §5.3/§5.10; §5.9 treats dialogs as overlay too).
- **Real fonts loaded** (2026-07-02, commit `9898dca`): `theme::install_fonts()`
  embeds **Geist** (UI) + **JetBrains Mono** (data) via `include_bytes!` from
  `cad_app/assets/fonts/` and installs a `FontDefinitions` at startup (main.rs,
  before first frame) — Geist at front of Proportional, Mono at front of
  Monospace, egui defaults kept as fallbacks (THEME_SYSTEM §5.7). The app already
  uses `FontId::proportional()`/`.monospace()` everywhere, so this re-points all
  text at once. (Weight 500 was added next — see below.)
- **Geist Medium + `theme::typ` type tokens** (2026-07-02, commit `90443ed`):
  egui's `FontId` has no weight axis, so the spec's 500-weight roles couldn't
  render from Regular. Geist-Medium is embedded as its own named family
  `GeistMedium` (fallback chain GeistMedium → Geist → egui defaults); Monospace
  gets no Medium (both mono styles are 400, §5.7). A new `theme::typ` module
  exposes the six §5.7 roles as `FontId` tokens — `title` (Medium 16), `body`
  (Regular 13), `body_strong` (Medium 13), `caption` (Medium 11), `data_value`
  (Mono 12), `data_code` (Mono 11). Components call these instead of inline
  `FontId`s (§1). Infra only — no consumers in this commit.
- **Type-token sweep** (2026-07-02, commit `d2f00f6`): 36 clean call sites
  routed onto `theme::typ` — all `monospace(11/12)` → `data_code`/`data_value`
  (size-preserving, weight-unambiguous); Inspector labels/values (prop 13) →
  `body` (INSPECTOR_DESIGN §4); column/section headers (prop 11) → `caption`;
  dock header title → `title`, type pill → `caption`. Section/column headers
  now render **Medium** vs Regular body labels. Off-spec sizes, ambiguous
  weights, canvas text-entity rendering, and modifier-laden mono `RichText`
  were **left untouched** and reported for owner decisions (see §8).
- **`typ::hint` role added** (2026-07-02, commit `059e11d`): §5.7 gained a
  seventh role — `hint` = Geist Regular **11/400**, the lighter 11px counterpart
  to `caption` (11/500) for secondary/subtitle text. Doc + infra only.
- **Parked type-token decisions applied** (2026-07-02, commit `99c2392`,
  owner-verified): calls from §8 landed — `menu_heading` → `caption`; settings
  subtitle → `hint`; Hatch-library picker name labels (verified UI chrome, not
  canvas entities) → `body` (11px → 13px; grid checked OK); "✔ Confirm"/"✗
  Discard" dialog buttons → `body_strong`; angle `"{:.1}°"` and thickness
  `"t = …"` readouts → `data_value` (proportional → **Mono**, intended). Off-spec
  sizes + modifier-laden mono `RichText` remain owner-deferred (see §8).
- **UI-inspect spacing readout** (2026-07-02): the inspector now shows a
  devtools-style **gap dimension** when the pointer is in the whitespace between
  two captured boxes — amber fill + measure line + px label, vertical and/or
  horizontal. Lets row/section spacing be read directly and checked against the
  §5.1 tokens. New `pp_gap_dim` helper + overlay logic in `app.rs` (see §11).
- **Inspector template revision** (2026-07-02, per `INSPECTOR_DESIGN_MENTOR.md`,
  7 commits `16724c8`→`76f5eca`): shared docked-panel header band 40→**32**
  (`HEADER_BAND` token, affects Inspector + command bar + rails); Inspector min
  width 220→**264**; the **type pill** moved out of the header to a centered
  full-width capsule below it (`HEADER_TO_PILL`/`PILL_H`/`PILL_TO_SECTION`);
  **Line Type** = dash-preview-first + abbreviated name (`Div (s)`, full on hover);
  **Line Weight** = matched thickness bar + Mono value (shared `pp_preview_len`);
  **Visible** = 16×16 checkbox (accent/on-accent, Mixed=indeterminate);
  **coordinate fields** repainted as `pp_box` + frameless Mono-12 DragValue
  (`pp_num_field`, no more egui-default chrome), **column gap 12→8**, Start/End
  headers lighter (`COLUMN_HEADER` #66707A, 11/400); **1px section dividers** above
  every section but GENERAL. Type-specific sections (§6) deliberately left for
  later. Awaiting owner visual verification.

---

## 8. In progress / next

The active thread is **making the Inspector match `INSPECTOR_DESIGN_MENTOR.md`**
(the current authority — supersedes `INSPECTOR_DESIGN.md`), verified with the
UI-inspect tool (owner clicks a wrong box → Copy dump → paste → fix the token).

The template revision (shell, header, pill, GENERAL/GEOMETRY renderers) landed
2026-07-02 — see §7. Resolved there: ~~Linetype dash-first~~, ~~Lineweight bar~~,
~~Visible checkbox~~, ~~coordinate-field styling~~, ~~section dividers~~, header 32,
pill below header, min 264, column gap 8. **Remaining Inspector finish items:**
- Sweep every element's measured values against the spec with UI-inspect (owner).
- **Type-specific dobject sections** (spec §6) — defined later, one dobject type
  at a time (Hatch, Text, Wall, Dim, Block, Circle/Arc/…). Not yet built.

**Type-token follow-ups — OWNER-DEFERRED** (the ambiguous-weight + proportional-
number decisions from the `d2f00f6` sweep were resolved and applied 2026-07-02;
see §7. What remains is deferred to a later "size-normalization + §5.12 icon-
token" pass):
- **Off-spec sizes** (no matching token; left as-is): prop **12** (10238, 18814,
  13061, 24007) and 7/7.5/9/9.5/10/12.5/13.5/14/15/11.5 across command-rail
  glyphs, badges, dialog titles, settings rows, logo glyphs; mono **9/10** debug
  HUD readouts. Decide whether command-glyph/badge sizes get their own tokens.
- **Modifier-laden mono `RichText`** (`.monospace().small()/.strong()`, no explicit
  size): status-bar items + debug section headers — mapping would drop `.small`/
  `.strong` semantics; needs a token story for those.
- **Out of scope:** CAD text-*entity* rendering (`font_id_for_font_name`, dim
  previews) is user-sized canvas text, not UI chrome — not governed by §5.7.

Then: the broader roadmap — ~~CommandRegistry (registry-driven rails + menus)~~
**done, see §7**; a **PanelRegistry** (Inspector-as-a-panel), a Theme Editor
panel, window min/max/close buttons, and the parked geometry bug (below).

**Post-merge follow-ups (from the dokkandar merge, §7):**
- **Owner visual verification** of the merged features — CPU/GPU/APX rendering,
  DWG open, the raster editor, parametric mode, and the ellipse tool (E1). Not
  yet eyeballed.
- **Shortcut map** (accel → id) and **real predicates** for `visible`/`enabled`
  were flagged during the registry work but deliberately deferred — owner's
  separate decisions.
- **dokkandar reconciliation** — they pull our branch and resolve the ~16
  `app.rs` hunks per [MERGE_HANDOFF_FOR_DOKKANDAR.md](MERGE_HANDOFF_FOR_DOKKANDAR.md)
  (take ours, except their recorder logging + wall-face rendering).

---

## 9. Held / open issues (do not claim fixed)

Living tracker: **[OPEN_ISSUES.md](OPEN_ISSUES.md)**. Highlights:

- ⏸ **HELD (shared root cause — "content drives panel size"):** docked Inspector
  resize won't shrink below content min width; floating Inspector header isn't
  full-width for wide selections; floating Inspector height doesn't visibly cap
  at 50%. To be fixed as one pass (flexible/clipping rows) — deferred by owner
  until the rest of the Inspector redesign is done.
- 🔴 **FILLET line ↔ polyline** produces a wrong result (explode-first works).
  Needs a session dump + the polyline's vertex data to diagnose.
- 🟡 Various fillet/chamfer edge cases; groups/hatch persistence; QAT persistence;
  custom title-bar window buttons (needs an owner A/B decision).

---

## 10. Conventions & rules

- **Terminology:** "dobject" not "object"; "Inspector" not "Properties";
  "DObject Snap" not "Object Snap"; "Delete" as the canonical verb; "Mixed"
  unifies "\*VARIES\*".
- **Design tokens over magic numbers** (§6).
- **Docking engine independence** (§5) — never hard-wire `egui_dock` or any
  engine into call sites.
- **[AGENTS.md](AGENTS.md)** — 21 coding/architecture/quality rules from the
  project. ⚠️ Some describe an older `.so`-plugin architecture; verify against
  the current `cad_kernel`/`cad_app` layout before enforcing literally.
- When applying spacing/token changes, **don't alter composition/design intent**;
  flag clashes before applying.
- **Don't claim something is fixed** until the owner confirms — several items
  looked done but regressed; the owner verifies visually.

---

## 11. Diagnostic tools (use these to verify)

- **UI inspect** (Tools ▸ Debug ▸ "UI inspect (element sizes)") — the ruler + the
  copyable "UI Inspect Log". Best way to check the Inspector against the spec.
  Hovering a box highlights it (cyan) with a size/detail tooltip; **pointing in
  the whitespace between two boxes draws an amber spacing dimension** (fill +
  ticks + px label) for the vertical and/or horizontal gap — so row/section gaps
  can be read straight off the screen and checked against the §5.1 tokens
  (`ROW_GAP` 8, `GROUP_GAP`/`SECTION_GAP` 12). Impl: `pp_gap_dim` + the overlay
  block in `app.rs`. The **UI Inspect Log** window has a title-bar **× close**
  that turns the whole tool off (overlay + window); re-enable from Tools ▸ Debug.
- **Session recorder** (`dbg_recorder.rs`) — Start/Stop, then the owner pastes
  `=== SESSION DUMP ===` text. Gestures/geometry are logged for bug repros.
- **Screen Stats / Render mode / Trim & Hatch debug logs** — under the same
  Debug menu.

---

## 12. Document index

| Doc | Topic |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | System architecture + index to the rest |
| [WORKSPACE_SYSTEM.md](WORKSPACE_SYSTEM.md) | Docking/workspace model (the `DockHost` boundary) |
| [COMMAND_SYSTEM.md](COMMAND_SYSTEM.md) | CommandRegistry design (commands drive rails + menus) |
| [COMMAND_REGISTRY_MENTOR.md](mentor%20MD/COMMAND_REGISTRY_MENTOR.md) | **Command Registry — FROZEN spec, the authority** (Phases 0–7, now implemented) |
| [PANEL_SYSTEM.md](PANEL_SYSTEM.md) | Panel trait/registry, Inspector as a panel |
| [THEME_SYSTEM.md](THEME_SYSTEM.md) | **Locked design tokens (§5)** |
| [INSPECTOR_DESIGN_MENTOR.md](mentor%20MD/INSPECTOR_DESIGN_MENTOR.md) | **Inspector rule — CURRENT AUTHORITY** (supersedes INSPECTOR_DESIGN.md) |
| [INSPECTOR_DESIGN.md](INSPECTOR_DESIGN.md) | Inspector pixel-level rule — **superseded**, historical reference |
| [inspector_mockup.html](inspector_mockup.html) | Browser-inspectable Inspector mockup |
| [PLUGIN_API.md](PLUGIN_API.md) | Plugin surface |
| [CONTENT_STYLE.md](CONTENT_STYLE.md) | Wording / number formatting |
| [OPEN_ISSUES.md](OPEN_ISSUES.md) | **Living bug/task tracker** |
| [MERGE_HANDOFF_FOR_DOKKANDAR.md](MERGE_HANDOFF_FOR_DOKKANDAR.md) | Reconciliation guide for the dokkandar fork merge |
| [RASTER_TO_VECTOR.md](RASTER_TO_VECTOR.md) | Raster→vector editor (the `cad_raster` subsystem) |
| [GPU_RENDER_UPDATE.md](GPU_RENDER_UPDATE.md) | GPU renderer change report |
| [Dobject_Properties.md](Dobject_Properties.md) | Per-dobject property schema |
| [SETTINGS.md](SETTINGS.md) | Variable/settings system |
| [AGENTS.md](AGENTS.md) | Coding rules (verify vs current arch) |
| [PROJECT_NOTES.md](PROJECT_NOTES.md) | Platform shims, misc notes |

---

## 13. TL;DR for the incoming agent

1. It's a Rust/egui 2D CAD app; the action is in **`cad_app/src/app.rs`** (~30k
   lines — grep, don't read whole) plus `command.rs` (registry), `dock.rs`,
   `theme.rs`, `gpu.rs`, and the new `cad_param`/`cad_raster` crates.
2. Build: `cd` to repo, `taskkill rust_cad.exe`, `cargo build -p cad_app`.
3. The **design system is locked** (THEME_SYSTEM §5 + INSPECTOR_DESIGN_MENTOR.md)
   — match it via **tokens**, verify with the **UI-inspect** tool.
4. **Never hard-wire a docking engine** (dock.rs `DockHost` is the seam); commands
   flow only through the registry's **`execute(id)` seam** — never touch
   `run_command`/the parser.
5. Respect the **terminology** and **don't claim fixes** before owner sign-off.
6. Two big tracks just landed (§7): the **Command Registry** (done) and the
   **dokkandar merge** (GPU/raster/param/DWG) — all pushed, awaiting the owner's
   visual verification. Older focus (Inspector finish, held items, FILLET bug)
   remains in **OPEN_ISSUES.md**.
