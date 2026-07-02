# PROJECT REVIEW — RUST-AutoRASM (for an incoming AI agent)

Read this first. It is a status + orientation brief so another agent can pick up
the project without re-discovering everything. It states **what the project is,
how it's built, what's done, what's in flight, what's parked, and the rules that
must not be broken.** Written 2026-07-01, last updated 2026-07-02, against
branch `windows-ui-session-2026-06-20`.

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
| `cad_wall` | Architectural wall logic (centerline → side lines). |
| **`cad_app`** | **The application** — egui UI, tools, rendering, all interaction. This is where ~all recent work lives. |
| `cad_cli` | A headless CLI harness for the kernel. |

### `cad_app/src` modules
| File | What |
|---|---|
| **`app.rs`** | **The monolith (~27,500 lines).** `CadApp` struct + `eframe::App::update`, every tool, the canvas, all panels/menus, the Inspector. Most work happens here. |
| `main.rs` | Entry point + module list + `eframe` window setup. |
| `dock.rs` | **Unified docking abstraction** (`DockHost` trait + `EguiDockHost`). See §5. |
| `theme.rs` | **Design tokens** (color/spacing/radius) + `theme::apply(ctx)` global Visuals + `theme::install_fonts(ctx)` (embeds Geist + JetBrains Mono). Single source for the look. |
| `gpu.rs` | GPU instance buffers / render path. |
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
- **GPU:** the canvas builds instance buffers (`gpu.rs`) for fast redraw of many
  dobjects; there is a CPU fallback and an "APX" draft-display mode.

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
  text at once. Only Regular (400) is loaded; weight 500 (Medium) awaits the
  type-token task (egui can't pick weight within a family via `FontId`).

---

## 8. In progress / next

The active thread is **making the Inspector exactly match `INSPECTOR_DESIGN.md`**,
verified with the UI-inspect tool (owner clicks a wrong box → Copy dump → paste →
fix the token). Known remaining finish items:
- **Linetype** field: reorder to **dashed-preview-first**, then name.
- **Lineweight** field: add the **thickness-bar** preview.
- **Visible**: render as a real **checkbox** (16×16, 4px, accent-on), not text.
- **Coordinate fields**: verify vertical centering (DragValue path).
- Sweep every element's measured values against §7 of the spec.

Then: the broader roadmap — CommandRegistry/PanelRegistry (registry-driven rails
+ menus), a Theme Editor panel, window min/max/close buttons, and the parked
geometry bug (below).

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
| [COMMAND_SYSTEM.md](COMMAND_SYSTEM.md) | CommandRegistry (commands drive rails + menus) |
| [PANEL_SYSTEM.md](PANEL_SYSTEM.md) | Panel trait/registry, Inspector as a panel |
| [THEME_SYSTEM.md](THEME_SYSTEM.md) | **Locked design tokens (§5)** |
| [INSPECTOR_DESIGN.md](INSPECTOR_DESIGN.md) | **Inspector pixel-level rule** (current build target) |
| [inspector_mockup.html](inspector_mockup.html) | Browser-inspectable Inspector mockup |
| [PLUGIN_API.md](PLUGIN_API.md) | Plugin surface |
| [CONTENT_STYLE.md](CONTENT_STYLE.md) | Wording / number formatting |
| [OPEN_ISSUES.md](OPEN_ISSUES.md) | **Living bug/task tracker** |
| [Dobject_Properties.md](Dobject_Properties.md) | Per-dobject property schema |
| [SETTINGS.md](SETTINGS.md) | Variable/settings system |
| [AGENTS.md](AGENTS.md) | Coding rules (verify vs current arch) |
| [PROJECT_NOTES.md](PROJECT_NOTES.md) | Platform shims, misc notes |

---

## 13. TL;DR for the incoming agent

1. It's a Rust/egui 2D CAD app; the action is in **`cad_app/src/app.rs`** (huge —
   grep, don't read whole) plus `dock.rs` and `theme.rs`.
2. Build: `cd` to repo, `taskkill rust_cad.exe`, `cargo build -p cad_app`.
3. The **design system is locked** (THEME_SYSTEM §5 + INSPECTOR_DESIGN.md) — match
   it via **tokens**, verify with the **UI-inspect** tool.
4. **Never hard-wire a docking engine** (dock.rs `DockHost` is the seam).
5. Respect the **terminology** and **don't claim fixes** before owner sign-off.
6. Current focus: finish the **Inspector** to `INSPECTOR_DESIGN.md`; held items
   and the FILLET bug are logged in **OPEN_ISSUES.md**.
