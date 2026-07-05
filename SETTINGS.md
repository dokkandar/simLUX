# RUST-AutoRASM — Settings & Variables Reference

The living reference for the user-environment variable system (RUST_CAD's
analogue of AutoCAD SYSVARs): the settings page, the command-line access, and
the variable catalog. **Keep this updated whenever the variable system changes**
— it's our review/reference doc.

> Single source of truth in code: **`cad_app/src/varreg.rs`**. The tables at the
> bottom of this file are GENERATED from it — regenerate after editing the
> registry (see "Regenerating the tables").

Last updated: 2026-06-26 (branch windows-ui-session-2026-06-20)

---

## 1. Architecture — where it lives (and what it must NOT touch)

Everything here is **application layer**, never the kernel.

| Piece | File | Crate |
|---|---|---|
| Variable registry (the catalog + validated setter) | `cad_app/src/varreg.rs` | `cad_app` |
| `UserEnv` struct + persistence (`user_env.txt`) | `cad_app/src/settings.rs` | `cad_app` |
| Settings page UI (sidebar + typed rows) | `cad_app/src/app.rs` (`settings_window`/`settings_row`/`settings_input`) | `cad_app` |
| Command line (`setvar`, bare name) | `cad_app/src/app.rs` (`handle_setvar`/`var_set_pending`) | `cad_app` |

**Hard rule:** `cad_kernel` is pure geometry/math with **no UI/settings
dependencies** (its Cargo.toml depends only on `cad_nurbs`). No variable,
setting, or UI code goes into the kernel. Dependency direction is always
`cad_app → cad_kernel`, never the reverse.

## 2. Locked design decisions

- **Show ALL variables** (~240) in the settings page, status-badged.
- **Code defaults win** every conflict between the code and the old mockup/docs.
- **Option A**: variables that aren't wired yet are **shown but disabled**
  (read-only, showing their default) — no dead controls that silently do nothing.
- **Command line: all three styles** — `setvar`, bare variable name, and
  sub-command options where natural.
- **One validated setter** — the settings page, `setvar`, the bare-name prompt,
  and the file loader all funnel through `varreg::env_set` (parse + clamp per
  kind), so a value entered any way is verified identically.

## 3. Variable status (the badge meanings)

| Badge | Meaning |
|---|---|
| **ACTIVE** | Fully wired — the value affects behavior today. |
| **PLANNED** | Defined + surfaced, but the feature it controls isn't built yet (one-line wire-up when it lands). |
| **STUB** | Present for forward-compat / pasting AutoCAD configs; not planned to implement. |
| **TENTATIVE** | Kept but uncertain we need it — revisit (promote or remove). |

`wired = true` ⇒ editable + persisted (maps to a real `UserEnv` field).
`wired = false` ⇒ catalogued only; the control is disabled (Option A).

## 4. How a value is captured (control + validation per type)

| Kind | Settings-page control | CLI / file accepts | Validation |
|---|---|---|---|
| `Bool` | checkbox (On/Off) | `true/1/on/yes` · `false/0/off/no` | — |
| `U8 {min,max}` | number drag | integer | clamped to [min,max] |
| `Int {min,max}` | number drag | integer | clamped |
| `Float {min,max}` | number drag | number | clamped; NaN rejected |
| `Choice([…])` | dropdown | the option **word** OR its **index** (0-based) | must resolve to a valid option |
| `Color` | colour swatch | `#RRGGBB` · `0xRRGGBB` · `r,g,b` | stored as `0xRRGGBB` (u32) |
| `Text` | text field | any string | — |

## 5. Command-line usage

```
setvar                      list every editable (wired) variable + current value
setvar ?                    same as above
setvar SpTGSZ               query: shows current, then prompts "Enter new value …"
setvar SpTGSZ 20            set inline (validated + persisted)
SpTGSZ                      bare name = same as `setvar SpTGSZ`
SpTGSZ 20                   bare name + inline value = set
```
- Names are **case-insensitive** and resolve to the canonical spelling.
- The value prompt consumes the next line; **empty Enter keeps** the current
  value; **bad input re-prompts**; **Esc cancels**.
- **Unwired** vars are read-only from the CLI too: querying shows value + status;
  setting is refused with a clear message (parity with the page).
- **Sub-command options** that already exist are unchanged: `TrmMd` via `t`/`nt`
  inside fillet/chamfer, `FltRad` = `fillet <r>`, `OfsDis` = `offset <d>`,
  `ChmDs1/2` via chamfer `d`, etc.

## 6. Settings page

Open via the **settings…** toolbar button.
- **Left sidebar** = the sections (name + variable count, click to switch).
- **Right panel** = one row per variable: cryptic name (mono) · description ·
  status badge · the type-appropriate control. Header has the status legend;
  footer has **Save now / Reload from disk / Reset to defaults**.
- Wired rows are editable and persist on change (via `env_set` + `env.save()`);
  unwired rows are disabled and show the default.

## 7. Reconciliation — conflicts resolved (history)

- **Dup names:** `TrmMd` kept (the active one), `TrmMod` dropped. `IntClr` tagged
  "alias of IntsCol". `HitTolPx` tagged "overlaps PkBxSz".
- **Enum types** that the old mockup mis-typed as bool/int are now `Choice`
  dropdowns: `DrDspM, WpFrmM, XrLdMd, UcsMod, LodAnc, GpuRnd, RubBnd, MvDdsp,
  RsmCmp, StartUp`.
- **Defaults:** code values won (`SpTGSZ 16`, `XrLdMd 2`, grip colours blue/red…).
- **Added** the real fields the mockup was missing (sections "Grid & CARD",
  "UCS Icon", "Drafting Defaults").

## 8. Open follow-ups

1. Wire the ~6 **overlap** vars to LIVE state instead of independent fields:
   `OsnOpt`/`SnpAct` ↔ `snap_enabled`, `PkAdd` ↔ `select_remove_mode`, `SnpPri` ↔
   `SnapKind::priority()`, the Code-Audit colours ↔ their draw-site constants.
2. As features land, flip **Planned → Active** and wire the read-site (then mark
   it here + in `varreg.rs`).
3. Remove the now-dead legacy helpers (`env_bool`/`env_u8`/… , `draw_settings_preview`).
4. Optional: section emoji icons in the sidebar (the mockup had them).
5. Reconcile/retire the old `Variables.md` — `varreg.rs` is now authoritative.

## 9. Process — adding or changing a variable

1. Add/edit the row in `cad_app/src/varreg.rs` (`VARS`): name, section, desc,
   `Kind`, `Status`, default, `wired`.
2. If it's **wired**, also: add the field to `UserEnv` (`settings.rs`) with the
   same default + a save/load/`set` arm, and add the `env_get`/`env_set` arms in
   `varreg.rs`.
3. Wire the read-site (replace the hardcoded value with `self.env.<Field>`); flip
   status to **Active**.
4. **Regenerate the tables in this file** (section below) and add a changelog row.

## 10. Regenerating the tables

The "Editable variables" and "Full catalog" tables below are generated from
`varreg.rs` by **`tools/gen-settings.ps1`**, which runs **automatically** via the
**`.githooks/pre-commit`** hook whenever `cad_app/src/varreg.rs` is part of a
commit (regenerates the tables + re-stages this file). Regenerate by hand:
`powershell -ExecutionPolicy Bypass -File tools/gen-settings.ps1`. Activate the
hook once per clone: `git config core.hooksPath .githooks`. Only the section
below the marker is rewritten; the prose above it is hand-maintained.

## 11. Changelog

| Date | Commit | Change |
|---|---|---|
| 2026-06-25 | `c4998fb` | Added reconciled variable registry `varreg.rs` (240 vars, 40 wired) |
| 2026-06-25 | `0199360` | Registry-driven settings page (sidebar + typed rows + status badges, Option A) |
| 2026-06-25 | `b9b53c3` | Command-line variable access (`setvar` + bare name) via the shared validated setter |
| 2026-06-26 | `2885905` | Created SETTINGS.md living reference |
| 2026-06-26 | `e6060b2` | Auto-regenerate tables via tools/gen-settings.ps1 + pre-commit hook |
| 2026-06-26 | (this)    | Combined `Variables.md` in; added detailed per-variable briefings (§12), the code-derived status check (§13), and the code-audit briefing (§14). `Variables.md` retired to a pointer. |

---

## 12. Detailed briefings — the variables that DO something today

> The generated tables (bottom) list **all 240** variables tersely. This section
> is the **hand-written deep reference** for the ones a coding agent actually
> touches: the **wired** variables (the 40 that map to real `UserEnv` fields).
> Each entry = what it does · type/range/default · AutoCAD analog · where it's
> read in code · gotchas. Live behaviour (status **Active**) is marked ⚙; wired
> but not-yet-acting (**Planned/Stub/Tentative**) is marked ◌.
>
> "Wired" is the line that matters: `wired=true` ⇒ it has a `UserEnv` field +
> `env_get`/`env_set` arms, so the settings page **and** `setvar` can change it
> and it persists to `~/.config/rust_cad/user_env.txt`. Unwired vars are
> catalogue-only (read-only, shown disabled) — see the generated tables.

### Snap & picking
- ⚙ **`SpTGSZ`** — object-snap target height (px). U8 4–80, default **16**.
  AutoCAD `APERTURE`. The cursor must be within this many *screen* pixels of a
  candidate snap point for the snap to fire. Read in the snap search:
  `world_radius = SpTGSZ / scale` at the `find_all_snaps()` call site; also the
  snap-window slider. Bigger = snappier but grabbier.
- ◌ **`PkBxSz`** — pickbox height (px) = click hit-test tolerance. U8 1–40,
  default **10**. AutoCAD `PICKBOX`. Overlaps the hardcoded `HitTolPx`
  (see §14) — these should be unified. Tentative until hit-testing is centralized.
- ◌ **`CrsHrS`** — crosshair size as % of the viewport's shorter side. U8 1–100,
  default **5**. Read in the crosshair render (`CrsHrS/100 * short`).

### Selection & grips
- ⚙ **`SelDmTm`** — selection-drag activation hold time (ms). Int 50–2000,
  default **250**. *The* hold-gate in the click/drag classifier: a press becomes
  a window-drag only after being held this long; a faster press-drag is treated
  as a click. The rubber-band preview honours the same gate. Full mechanics in
  `CLICK_DRAG_HANDLER.md`. Shift-drag bypasses it.
- ⚙ **`GrpHvR`** — grip hover + grab radius (px). U8 4–80, default **25**. Within
  this distance a grip highlights and a click/drag grabs it. No exact AutoCAD
  analog (LibreCAD's grip hot-radius ≈ 20).
- ⚙ **`GrpEnb`** — enable/disable grips. Bool, default **true**. Toolbar "grips"
  button, `Command::GripsToggle`, `if self.env.GrpEnb` in the render loop.
- ◌ **`GrpSz`** — grip size (px). U8 1–20, default **4**. `draw_grips`.
- ◌ **`GrClrU` / `GrClrS`** — unselected / selected (hot) grip colours
  (0xRRGGBB). Defaults **0x4099FF** (blue) / **0xFF6464** (red-pink). `draw_grips`.
- ◌ **`SelPrv`** — preview-highlight a dobject on cursor-over (pre-click). Bool,
  default true. `if env.SelPrv` gates the hover preview.
- ◌ **`HltSel`** — highlight selected dobjects with a distinct colour. Bool,
  default true. `if env.HltSel` picks the highlight colour.
- ◌ **`GrpBlk`** — show grips on dobjects inside blocks. Bool false. Stub until
  blocks expose interior grips.

### Editing & drafting defaults
- ◌ **`EdgMod`** — edge-mode for TRIM/EXTEND. Bool, default **true**. AutoCAD
  `EDGEMODE`. ON = treat cutting/boundary edges as their infinite extensions for
  "imaginary intersection" cuts; OFF = only real curve intersections.
- ⚙ **`FltRad`** — default fillet radius. Float, default **0.0**. AutoCAD
  `FILLETRAD`. Set inline with `fillet <r>`; the new value persists.
- ◌ **`ChmDs1` / `ChmDs2`** — default chamfer distances on the first / second
  picked line. Float, default 0.0. AutoCAD `CHAMFERA` / `CHAMFERB`. Set via
  chamfer `d`.
- ⚙ **`OfsDis`** — default offset distance. Float, default **1.0**. AutoCAD
  `OFFSETDIST`. `offset <d>` sets it; bare `offset` reuses it.
- ⚙ **`WlThk`** — default wall thickness (±t/2 about the centerline). Float,
  default **0.20**. `wall <t>` sets it. Synced when a wall style is set current.
- ⚙ **`TxHt`** — default text height (world units). Float, default **0.25**.
- ⚙ **`WlCnL`** — wall centerline visible (dashed half-alpha overlay on every
  `Geom::Wall`). Bool, default **true**.
- ⚙ **`TrmMd`** — trim mode shared by Fillet & Chamfer. Bool, default **true**.
  AutoCAD `TRIMMODE`. true = trim originals back to the corner; false = keep them
  full-length and add the arc/bevel separately. Toggle `t` / `nt` at the prompt.
  *(Canonical name `TrmMd`; the old catalog spelling `TrmMod` was dropped.)*

### Grid & CARD
- ⚙ **`GrdEnb`** — background grid display. Bool, default **true**. AutoCAD
  `GRIDMODE`. **F7** toggles.
- ⚙ **`GrdSnp`** — snap cursor to grid intersections. Bool, default **false**.
  AutoCAD `SNAPMODE`. **F9** toggles. Object snap wins over it.
- ⚙ **`GrdSpc`** — grid spacing (world units), shared by the display grid AND
  snap rounding. Float, default **10.0**. AutoCAD `GRIDUNIT`.
- ⚙ **`CrdEnb`** — **CARD**, the cardinal-directions drafting lock (cursor H or V
  only from the anchor). Bool, default **false**. AutoCAD `ORTHOMODE`. **F8** /
  `card on|off` / status badge. Legacy settings key `OrtEnb` is accepted on load.

### UCS icon
- ⚙ **`UcsIcn`** — UCS origin-marker icon on/off. Bool, default **true**. AutoCAD
  `UCSICON`. Renders the origin dot + X/Y axis arrows.
- ⚙ **`UcsMod`** — icon placement. Choice `corner` / `origin`, default **0**
  (corner). `origin` = anchor at world (0,0) when visible (AutoCAD `UCSICON ORigin`).
- ⚙ **`UcsAvP`** — path to a PNG/SVG drawn on the UCS icon's X-axis (empty →
  placeholder). Text. Persisted so the user sets it once.

### Display / UI / dialogs / xrefs (wired but mostly Planned)
- ◌ **`DrDspM`** drag display during MOVE/COPY (off/on/auto, default auto) ·
  ◌ **`RllTp`** rollover tooltips (true) · ◌ **`WpFrmM`** wipeout frame display
  (off/on/sel-only) · ◌ **`LodAnc`** APX draft dot-anchor (bbox center / primitive
  center / first vertex).
- ◌ **`MnuBar`** classic menu bar (false) · ◌ **`TltEnb`** toolbar/ribbon
  tooltips (true).
- ◌ **`AtDlgM` / `AtPrmM`** attribute dialog / prompting on INSERT (Tentative —
  no attribute system yet) · ◌ **`CmDlgM`** plot dialogs (Stub).
- ◌ **`FlDlgM`** suppress file-navigation dialogs (true) — Active-class but gated
  on the file-I/O subsystem.
- ◌ **`XrLdMd`** xref demand-loading (off / on / on-with-copy, default 2) ·
  ◌ **`XrTmpP`** temp xref copy path — both persisted; xref runtime not built.

## 13. Re-deriving status from code (run the same)

`varreg.rs` is the source of truth, but it's hand-maintained — so verify it
against the actual `UserEnv` + read-sites instead of trusting badges. From the
repo root:

```bash
# Fields that REALLY exist in UserEnv:
sed -n '/pub struct UserEnv/,/^}/p' cad_app/src/settings.rs \
  | grep -oE 'pub [A-Za-z0-9_]+:' | sed 's/pub //; s/://'

# Fields READ by behaviour code (not just the settings UI). A field read
# outside settings.rs / varreg.rs is genuinely Active:
grep -rhoE 'env\.[A-Z][A-Za-z0-9_]+' \
    cad_app/src/app.rs cad_app/src/app/ cad_app/src/gpu.rs cad_kernel/src/ cad_wall/src/ \
  | grep -vE 'env\.(save|set|get|txt)' | sort | uniq -c | sort -rn

# Registry rows whose wired=true but with no matching UserEnv field (drift):
#   cross-check the first list against `wired: true` rows in varreg.rs.
```
Rule: **Active** = read by behaviour code; **wired-but-Planned** = has a field +
control but only read by its own settings widget; **catalogue-only** = not a
`UserEnv` field at all. When you wire a Planned var, flip its `Status` in
`varreg.rs`, then regenerate (§10).

## 14. Code-audit variables — hardcoded values awaiting promotion

The **"Code-Audit Hardcoded" (34)** section in the catalog below is different
from the AutoCAD-derived rows: these are **magic numbers / hex colours that
already exist in the code** (`app.rs`, `gpu.rs`) and *should* become real
settings so the user can tune them without a rebuild. They're all **Planned** +
unwired today. Examples: `DefDClr`/`SelClr`/`SnpClr` (the dobject/selection/snap
colours), the `Ext*` imaginary-extension dash params, the `SelDsh*`/`SelPls*`
selection-overlay dash + pulse params, `HitTolPx` (overlaps `PkBxSz`), the
`Tess*` tessellation factors, `DfltZm`, `DemoOn`. To promote one: follow §9
(add the `UserEnv` field + `env_get`/`env_set`, replace the hardcoded read site
with `self.env.<Field>`, flip to Active), then regenerate.

> **`Variables.md` is retired** — it's now a pointer to this file. `varreg.rs` +
> this doc are authoritative for the variable system.

---
<!-- ===== GENERATED TABLES BELOW (from varreg.rs) — do not hand-edit ===== -->

## Editable variables (wired = 40)

| Name | Section | Type | Default | Status | Description |
|---|---|---|---|---|---|
| `CrsHrS` | Display & Visual Feedback | U8 1-100 | `5` | Planned | Crosshair size (screen %) |
| `DrDspM` | Display & Visual Feedback | Choice(off/on/auto) | `2` | Planned | Dragging display during MOVE/COPY |
| `HltSel` | Display & Visual Feedback | Bool | `true` | Planned | Highlight selected objects |
| `RllTp` | Display & Visual Feedback | Bool | `true` | Planned | Tooltips on dobject rollover |
| `SelPrv` | Display & Visual Feedback | Bool | `true` | Planned | Preview highlight of selection |
| `WpFrmM` | Display & Visual Feedback | Choice(off/on/on for selection only) | `2` | Stub | Frame display of wipeouts |
| `LodAnc` | Display & Visual Feedback | Choice(bbox center/primitive center/first vertex) | `0` | Planned | APX draft dot-anchor strategy |
| `GrClrS` | Selection & Grips | Color | `0xFF6464` | Planned | Selected (hot) grip colour |
| `GrClrU` | Selection & Grips | Color | `0x4099FF` | Planned | Unselected grip colour |
| `GrpBlk` | Selection & Grips | Bool | `false` | Stub | Show grips inside blocks |
| `GrpEnb` | Selection & Grips | Bool | `true` | Active | Enable/disable grips |
| `GrpSz` | Selection & Grips | U8 1-20 | `4` | Planned | Grip size (pixels) |
| `SelDmTm` | Selection & Grips | Int 50-2000 | `250` | Active | Selection-drag activation hold time (ms) |
| `GrpHvR` | Selection & Grips | U8 4-80 | `25` | Active | Grip hover/grab radius (pixels) |
| `PkBxSz` | Object Snaps & Precision | U8 1-40 | `10` | Tentative | Pickbox height (pixels) |
| `SpTGSZ` | Object Snaps & Precision | U8 4-80 | `16` | Active | Object-snap target height (pixels) |
| `AtDlgM` | Editing & Behavior | Bool | `true` | Tentative | Attribute entry dialog on INSERT |
| `AtPrmM` | Editing & Behavior | Bool | `true` | Tentative | Attribute prompting during INSERT |
| `CmDlgM` | Editing & Behavior | Bool | `true` | Stub | Dialog boxes for PLOT, etc. |
| `EdgMod` | Editing & Behavior | Bool | `true` | Planned | Edge-mode for trim/extend |
| `XrLdMd` | Xrefs & Images | Choice(off/on/on with copy) | `2` | Active | External-reference demand-loading |
| `XrTmpP` | Xrefs & Images | Text | `` | Active | Path for temporary xref copies |
| `MnuBar` | UI & Workspace | Bool | `false` | Planned | Display the classic menu bar |
| `TltEnb` | UI & Workspace | Bool | `true` | Planned | Show toolbar/ribbon tooltips |
| `FlDlgM` | System & Performance | Bool | `true` | Active | Suppress file-navigation dialogs |
| `GrdEnb` | Grid & CARD | Bool | `true` | Active | Background grid display (GRIDMODE) |
| `GrdSnp` | Grid & CARD | Bool | `false` | Active | Snap cursor to grid intersections (SNAPMODE) |
| `GrdSpc` | Grid & CARD | Float | `10` | Active | Grid spacing in world units (GRIDUNIT) |
| `CrdEnb` | Grid & CARD | Bool | `false` | Active | CARD cardinal-directions drafting lock |
| `UcsIcn` | UCS Icon | Bool | `true` | Active | UCS indicator on/off (UCSICON) |
| `UcsMod` | UCS Icon | Choice(corner/origin) | `0` | Active | UCS icon placement mode |
| `UcsAvP` | UCS Icon | Text | `` | Active | Path to UCS X-axis avatar image |
| `FltRad` | Drafting Defaults | Float | `0` | Active | Default fillet radius (FILLETRAD) |
| `ChmDs1` | Drafting Defaults | Float | `0` | Planned | Default chamfer distance, first line (CHAMFERA) |
| `ChmDs2` | Drafting Defaults | Float | `0` | Planned | Default chamfer distance, second line (CHAMFERB) |
| `OfsDis` | Drafting Defaults | Float | `1` | Active | Default offset distance (OFFSETDIST) |
| `WlThk` | Drafting Defaults | Float | `0.2` | Active | Default wall thickness |
| `TxHt` | Drafting Defaults | Float | `0.25` | Active | Default text height (world units) |
| `WlCnL` | Drafting Defaults | Bool | `true` | Active | Wall centerline visible |
| `TrmMd` | Drafting Defaults | Bool | `true` | Active | Trim mode shared by Fillet and Chamfer (TRIMMODE) |

## Full catalog (240 variables, by section)

### Display & Visual Feedback  (34)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `AperBx` | Bool | `false` | Stub |  | Show the snap-target box around the crosshair (size = SpTGSZ) |
| `BkgPlt` | Bool | `false` | Stub |  | Print/plot in the background while you keep working |
| `CrsACol` | Color | `0x78E678` | Planned |  | Crossing-selection area colour |
| `CrsHrS` | U8 1-100 | `5` | Planned | yes | Crosshair size (screen %) |
| `DrDspM` | Choice(off/on/auto) | `2` | Planned | yes | Dragging display during MOVE/COPY |
| `GalVw` | Bool | `false` | Stub |  | Block gallery view on/off |
| `HltSel` | Bool | `true` | Planned | yes | Highlight selected objects |
| `HpQckP` | Bool | `false` | Stub |  | Hatch quick preview on/off |
| `ImgHlt` | Bool | `false` | Stub |  | Image frame highlight on/off |
| `IntsCol` | Color | `0xFF5A5A` | Planned |  | Colour of the intersection (∩) markers where objects cross |
| `IntsDsp` | Bool | `true` | Planned |  | Show the intersection (∩) markers where objects cross |
| `LnFade` | Bool | `false` | Stub |  | Line fading in edit mode |
| `LtGlyD` | Bool | `false` | Stub |  | Light glyph display |
| `LyLkFd` | U8 0-100 | `50` | Planned |  | Locked-layer fade percentage |
| `MTxtFx` | Bool | `false` | Stub |  | Mtext fixed-width editor on/off |
| `OleHid` | Bool | `false` | Stub |  | Hide OLE objects on/off |
| `PcBnd` | Bool | `false` | Stub |  | Point-cloud bounding-box display |
| `PcClpF` | Bool | `false` | Stub |  | Show the crop-boundary outline of a 3D point cloud (laser scan) |
| `PrvFlt` | Bool | `false` | Stub |  | Object types skipped during hover preview-highlight |
| `RllTp` | Bool | `true` | Planned | yes | Tooltips on dobject rollover |
| `RvClCrM` | Bool | `false` | Stub |  | How revision clouds are drawn (the change-marking cloud outline) |
| `RvClGrp` | Bool | `false` | Stub |  | Revcloud grip display |
| `SelAr` | Bool | `true` | Planned |  | Selection area effect |
| `SelPrv` | Bool | `true` | Planned | yes | Preview highlight of selection |
| `SelPrvL` | Int 0-1_000_000 | `2000` | Planned |  | Selection preview dobject limit |
| `TrkPth` | Bool | `false` | Stub |  | Tracking path display mode |
| `TrnDsp` | Bool | `false` | Stub |  | Draw objects' transparency (see-through) level on/off |
| `TryIco` | Bool | `false` | Stub |  | Tray icon display |
| `TryTim` | Int 1-1_000_000 | `5` | Stub |  | Tray notification timeout |
| `WinACol` | Color | `0x78AAFF` | Planned |  | Window-selection area colour |
| `WmfBkg` | Color | `0xFFFFFF` | Stub |  | WMF background colour |
| `WmfFrg` | Color | `0x000000` | Stub |  | WMF foreground colour |
| `WpFrmM` | Choice(off/on/on for selection only) | `2` | Stub | yes | Frame display of wipeouts |
| `LodAnc` | Choice(bbox center/primitive center/first vertex) | `0` | Planned | yes | APX draft dot-anchor strategy |

### Selection & Grips  (20)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `GrClrS` | Color | `0xFF6464` | Planned | yes | Selected (hot) grip colour |
| `GrClrU` | Color | `0x4099FF` | Planned | yes | Unselected grip colour |
| `GrpBlk` | Bool | `false` | Stub | yes | Show grips inside blocks |
| `GrpEnb` | Bool | `true` | Active | yes | Enable/disable grips |
| `GrpObjL` | Int 0-1_000_000 | `1000` | Planned |  | Maximum dobjects for grip display |
| `GrpSz` | U8 1-20 | `4` | Planned | yes | Grip size (pixels) |
| `GrpTip` | Bool | `true` | Stub |  | Grip hover tooltips on/off |
| `HidTxt` | Bool | `false` | Stub |  | Hide text during move/rotate |
| `ObjIsoM` | Bool | `false` | Stub |  | Object isolation mode |
| `OsnNdLg` | Bool | `false` | Stub |  | Osnap node legacy mode |
| `OsnOpt` | Int 0-1_000_000 | `63` | Planned |  | Object snap options |
| `PkAdd` | Bool | `true` | Planned |  | Selection add mode |
| `PkAuto` | Bool | `true` | Planned |  | Implied window selection |
| `PkDrag` | Bool | `true` | Planned |  | Selection by dragging |
| `PkFrst` | Bool | `true` | Planned |  | Noun/verb selection |
| `SelCyc` | Bool | `true` | Planned |  | Selection cycling on/off |
| `SelOfSc` | Bool | `false` | Planned |  | Select off-screen dobjects |
| `SubSelM` | Bool | `false` | Stub |  | Subobject selection mode |
| `SelDmTm` | Int 50-2000 | `250` | Active | yes | Selection-drag activation hold time (ms) |
| `GrpHvR` | U8 4-80 | `25` | Active | yes | Grip hover/grab radius (pixels) |

### Object Snaps & Precision  (8)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `OsnCrd` | Bool | `true` | Planned |  | Osnap coordinate override keyboard |
| `PkBxSz` | U8 1-40 | `10` | Tentative | yes | Pickbox height (pixels) |
| `PolAdA` | Text | `` | Planned |  | Polar additional angles |
| `PolAng` | Int 0-360 | `90` | Planned |  | Polar angle setting |
| `PolDst` | Float | `1` | Planned |  | Polar snap distance |
| `PolMod` | Bool | `true` | Planned |  | Polar tracking mode |
| `SpTGSZ` | U8 4-80 | `16` | Active | yes | Object-snap target height (pixels) |
| `TmpOvr` | Bool | `true` | Planned |  | Temporary override keys |

### Editing & Behavior  (25)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `AtDlgM` | Bool | `true` | Tentative | yes | Attribute entry dialog on INSERT |
| `AtPrmM` | Bool | `true` | Tentative | yes | Attribute prompting during INSERT |
| `BActBM` | Bool | `false` | Stub |  | Block action bar display mode |
| `BlkEdLk` | Bool | `false` | Stub |  | Lock block editor from editing |
| `BlkEdtr` | Bool | `false` | Stub |  | Block editor open/close state |
| `BlkMrL` | Int 0-1_000_000 | `5` | Stub |  | Block MRU list length |
| `BndTyp` | Int 0-1_000_000 | `0` | Stub |  | Xref bind type |
| `CmDlgM` | Bool | `true` | Stub | yes | Dialog boxes for PLOT, etc. |
| `DblClkE` | Bool | `true` | Planned |  | Double-click editing on/off |
| `EdgMod` | Bool | `true` | Planned | yes | Edge-mode for trim/extend |
| `HpMaxA` | Int 0-1_000_000 | `100` | Stub |  | Maximum hatch area for preview |
| `HpObjW` | Int 0-1_000_000 | `500` | Stub |  | Hatch dobject warning limit |
| `HpSep` | Bool | `false` | Stub |  | Separate hatch dobjects on/off |
| `InpHMd` | Bool | `true` | Planned |  | Dynamic input history display mode |
| `MTjigS` | Text | `Sample` | Stub |  | Mtext sample string for jig |
| `PedAcc` | Bool | `false` | Stub |  | Suppress PEDIT convert prompt |
| `PrsPul` | Bool | `false` | Stub |  | Presspull behavior mode |
| `RefPtTp` | Int 0-1_000_000 | `0` | Stub |  | Reference path type |
| `SavFid` | Bool | `false` | Stub |  | Save visual fidelity for annotative |
| `SbyLyr` | Bool | `false` | Planned |  | SetByLayer mode |
| `SrfAsc` | Bool | `false` | Stub |  | Surface associativity |
| `TblInd` | Bool | `false` | Stub |  | Table cell indicator on/off |
| `TblTbr` | Bool | `false` | Stub |  | Table toolbar on/off |
| `XEdit` | Bool | `false` | Stub |  | Edit in-place on/off |
| `XFdCtl` | Bool | `false` | Stub |  | Ref-edit object fading |

### File & Save  (15)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `AudCtl` | Bool | `false` | Planned |  | Create audit report file |
| `AutoPub` | Bool | `false` | Stub |  | Automatic publish on save/close |
| `DgnMpP` | Text | `` | Stub |  | DGN mapping file path |
| `DwgChk` | Bool | `false` | Stub |  | Check for non-Autodesk DWG files |
| `IsvBak` | Bool | `true` | Planned |  | Incremental save backup creation |
| `IsvPrc` | U8 0-100 | `10` | Planned |  | Incremental save percentage |
| `LogFlM` | Bool | `false` | Planned |  | Log file on/off |
| `LogFlP` | Text | `` | Planned |  | Log file path |
| `OpnPrt` | Bool | `false` | Stub |  | Open partial DWG file |
| `RcovMd` | Bool | `true` | Planned |  | Drawing recovery mode |
| `SavFP` | Text | `./autosave/` | Planned |  | Automatic save file path |
| `SavTim` | Int 1-1_000_000 | `10` | Planned |  | Automatic save interval (minutes) |
| `SigWarn` | Bool | `false` | Stub |  | Digital signature warning |
| `SldChk` | Bool | `false` | Stub |  | 3D solid validation on/off |
| `TrstPth` | Text | `` | Stub |  | Trusted file paths |

### Xrefs & Images  (14)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `XrLdMd` | Choice(off/on/on with copy) | `2` | Active | yes | External-reference demand-loading |
| `XrTmpP` | Text | `` | Active | yes | Path for temporary xref copies |
| `XrCtl` | Bool | `false` | Stub |  | Xref log file on/off |
| `XrLyr` | Text | `0` | Stub |  | Default layer for xref insertion |
| `XrNtfy` | Bool | `true` | Stub |  | Xref change notification |
| `XrTyp` | Int 0-1_000_000 | `0` | Stub |  | Default xref type |
| `XdwFd` | U8 0-100 | `0` | Stub |  | Xref drawing fade percentage |
| `RastDpi` | Int 0-1_000_000 | `300` | Stub |  | Raster image DPI for plotting |
| `RastPrc` | U8 0-100 | `20` | Stub |  | Raster image memory percentage |
| `RastThr` | Int 0-1_000_000 | `100` | Stub |  | Raster image memory threshold |
| `OleQlty` | Int 0-1_000_000 | `1` | Stub |  | OLE plot quality |
| `OleStrt` | Bool | `false` | Stub |  | OLE application startup on load |
| `PdfShx` | Bool | `false` | Stub |  | PDF SHX text handling |
| `PdfShxL` | Text | `` | Stub |  | PDF SHX text layer |

### UI & Workspace  (19)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `DobMenu` | Text | `` | Stub |  | Enterprise CUI menu file |
| `LokUI` | Bool | `false` | Stub |  | Lock toolbars/palettes position |
| `MnuBar` | Bool | `false` | Planned | yes | Display the classic menu bar |
| `MnuCtl` | Bool | `false` | Stub |  | Menu control (screen menu) |
| `NavBar` | Bool | `false` | Stub |  | Navigation bar display |
| `NavCube` | Bool | `false` | Stub |  | ViewCube display |
| `PalOpq` | U8 0-100 | `100` | Stub |  | Palette transparency |
| `QpLoc` | Int 0-1_000_000 | `0` | Stub |  | Quick-properties location |
| `QpMod` | Bool | `false` | Stub |  | Quick-properties mode |
| `RibSta` | Bool | `false` | Stub |  | Ribbon minimized state |
| `ScrnBx` | Bool | `false` | Stub |  | Screen menu boxes (legacy) |
| `ShctMn` | Bool | `true` | Planned |  | Shortcut menu on/off |
| `StartUp` | Choice(off/on) | `0` | Planned |  | Startup dialog mode |
| `TbCust` | Bool | `false` | Stub |  | Toolbar customize on/off |
| `TltEnb` | Bool | `true` | Planned | yes | Show toolbar/ribbon tooltips |
| `TltMrg` | Bool | `false` | Stub |  | Tooltip merge on/off |
| `TltTrn` | U8 0-100 | `0` | Stub |  | Tooltip transparency |
| `TpPalP` | Text | `` | Stub |  | Tool palette path |
| `TxtEd` | Text | `` | Stub |  | Text editor application |

### Plot & Publish  (4)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `PapUpd` | Bool | `true` | Stub |  | Paper-size update warning |
| `PStPlc` | Int 0-1_000_000 | `0` | Stub |  | Plot style policy for new drawings |
| `PubAll` | Bool | `false` | Stub |  | Publish all sheets |
| `PubHch` | Bool | `false` | Stub |  | Publish hatch on/off |

### System & Performance  (16)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `FlDlgM` | Bool | `true` | Active | yes | Suppress file-navigation dialogs |
| `FntAlt` | Text | `` | Stub |  | Alternate font when font not found |
| `FntMap` | Text | `` | Stub |  | Font mapping file path |
| `LspAsD` | Bool | `false` | Stub |  | Load acad.lsp into every drawing |
| `MxActVp` | Int 0-1_000_000 | `64` | Stub |  | Maximum active viewports |
| `MxSort` | Int 0-1_000_000 | `1000` | Planned |  | Maximum list sort size |
| `PrxNot` | Bool | `false` | Stub |  | Proxy dobject notice |
| `PrxShw` | Bool | `false` | Stub |  | Proxy dobject display |
| `PrxWeb` | Bool | `false` | Stub |  | Proxy web search on/off |
| `StdViol` | Bool | `false` | Stub |  | Standards-violation notification |
| `SysMon` | Bool | `false` | Planned |  | System-variable monitor on/off |
| `TreMax` | Int 0-1_000_000 | `100000` | Planned |  | Tree memory limit |
| `TxtFil` | Bool | `false` | Stub |  | Text fill on/off |
| `TxtQlt` | Int 0-1_000_000 | `50` | Stub |  | Text quality |
| `UntMod` | Int 0-1_000_000 | `0` | Planned |  | Unit display mode |
| `WhipArc` | Int 0-1_000_000 | `8` | Planned |  | Arc/circle smoothness |

### View & Navigation  (13)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `GeoLoc` | Bool | `false` | Stub |  | Geolocation marker visibility |
| `LayTab` | Bool | `false` | Stub |  | Model/Layout tab display |
| `RtDsp` | Bool | `true` | Planned |  | Real-time pan/zoom display |
| `StepSz` | Float | `1` | Stub |  | Walk/fly step size |
| `StpPrSc` | Int 0-1_000_000 | `30` | Stub |  | Walk/fly steps per second |
| `SunPrW` | Bool | `false` | Stub |  | Sun properties window on/off |
| `UcsOrt` | Bool | `false` | Stub |  | Orthographic UCS toggle |
| `VtDur` | Int 0-1_000_000 | `300` | Planned |  | Smooth view transition duration |
| `VtEnbl` | Bool | `true` | Planned |  | Smooth view transition on/off |
| `VtFps` | Int 0-1_000_000 | `60` | Planned |  | Smooth view transition speed (FPS) |
| `VwUpdA` | Bool | `true` | Planned |  | View update automatic |
| `ZmFact` | Float | `0.0015` | Planned |  | Mouse wheel zoom factor |
| `ZmWhl` | Bool | `true` | Planned |  | Mouse wheel zoom direction |

### Miscellaneous  (9)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `Chrma` | Bool | `false` | Stub |  | Colour-book display mode |
| `LyrDlgM` | Bool | `false` | Planned |  | Layer properties manager mode |
| `LyrFlA` | Bool | `false` | Planned |  | Layer-filter alert on/off |
| `LyrNtf` | Bool | `false` | Planned |  | Layer notification on/off |
| `MTxtEd` | Text | `` | Stub |  | Multiline text editor application |
| `PrjNam` | Text | `` | Planned |  | Project file search path |
| `SsmAuto` | Bool | `false` | Stub |  | Sheet Set Manager auto open |
| `SsmPol` | Int 0-1_000_000 | `60` | Stub |  | Sheet Set Manager poll time |
| `SsmSta` | Bool | `false` | Stub |  | Sheet Set Manager status |

### RUST_CAD-specific  (14)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `GpuRnd` | Choice(CPU/GPU-auto/GPU-forced) | `1` | Planned |  | Rendering path: CPU / GPU-auto / GPU-forced |
| `FpsDsp` | Bool | `true` | Planned |  | FPS overlay visibility |
| `IdxDsp` | Bool | `true` | Planned |  | Spatial-index status overlay |
| `IdxCel` | Float | `10` | Planned |  | Spatial-index target cells per dobject |
| `BgCol` | Color | `0x12161C` | Planned |  | Canvas background colour |
| `SnpPri` | Text | `end,mid,cen,int` | Planned |  | Snap priority order |
| `SnpAct` | Int 0-1_000_000 | `255` | Planned |  | Default SnapSet at startup |
| `TabCyc` | Bool | `true` | Planned |  | Tab cycling between snap candidates |
| `CmdEcho` | Bool | `true` | Planned |  | Echo commands to history |
| `CmdHisM` | Int 0-1_000_000 | `500` | Planned |  | Command history retention size |
| `RubBnd` | Choice(solid/dashed/animated) | `0` | Planned |  | Rubber-band style |
| `MvDdsp` | Choice(ghost/outline/off) | `0` | Planned |  | Move-tool ghost render style |
| `RsmCmp` | Choice(uncompressed/LZ4/zstd) | `1` | Planned |  | .rsm save format |
| `RsmBak` | Bool | `true` | Planned |  | Keep .rsm.bak on save |

### Code-Audit Hardcoded  (34)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `DefDClr` | Color | `0xAAC8E6` | Planned |  | Default dobject colour |
| `SelClr` | Color | `0xFFC850` | Planned |  | Selected dobject highlight colour |
| `SnpSrcClr` | Color | `0x78F0FF` | Planned |  | Snap-source entity highlight colour |
| `SnpClr` | Color | `0x50E6F0` | Planned |  | Snap glyph + label colour |
| `IntClr` | Color | `0xFF5A5A` | Planned |  | Intersection marker colour (alias of IntsCol) |
| `ExtClr` | Color | `0xFFC85A` | Planned |  | Imaginary-extension dashed-line colour |
| `PreClr` | Color | `0xFFDC64` | Planned |  | Preview / rubber-band colour |
| `ExtSpd` | Float | `60` | Planned |  | Extension-dash drift speed (px/sec) |
| `ExtFade` | Float | `0.55` | Planned |  | Extension-dash alpha base |
| `ExtDshL` | Float | `7` | Planned |  | Extension-dash length (px) |
| `ExtGapL` | Float | `4` | Planned |  | Extension-dash gap (px) |
| `WinDshSpd` | Float | `40` | Planned |  | Selection-window dash drift speed |
| `SelDshClr` | Color | `0xB4D2E6` | Planned |  | Selection-basket dashed overlay colour |
| `SelDshW` | Float | `1.6` | Planned |  | Selection-basket dashed overlay stroke width |
| `SelDshL` | Float | `6` | Planned |  | Selection-basket dash length |
| `SelDshG` | Float | `4` | Planned |  | Selection-basket dash gap |
| `SelPlsMin` | Float | `0.15` | Planned |  | Selection-basket pulse alpha min |
| `SelPlsMax` | Float | `0.85` | Planned |  | Selection-basket pulse alpha max |
| `SelPlsHz` | Float | `1.4` | Planned |  | Selection-basket pulse frequency |
| `HitTolPx` | Float | `10` | Planned |  | Hit-test tolerance (pixels) (overlaps PkBxSz) |
| `IntRad` | Float | `50` | Planned |  | ∩ click search radius |
| `PairLim` | Int 0-1_000_000_000 | `5000000` | Planned |  | Max candidate pair count |
| `TabCycR` | Float | `4` | Planned |  | Cursor-move px before Tab cycle resets |
| `ArrCol` | Int 0-1_000_000 | `10` | Planned |  | Array default columns |
| `ArrRow` | Int 0-1_000_000 | `10` | Planned |  | Array default rows |
| `ArrDX` | Float | `50` | Planned |  | Array delta X |
| `ArrDY` | Float | `50` | Planned |  | Array delta Y |
| `DfltZm` | Float | `6` | Planned |  | Default zoom scale |
| `DemoOn` | Bool | `true` | Planned |  | Load demo dobjects on startup |
| `GpuRgWd` | Float | `1` | Planned |  | GPU circle ring thickness |
| `TessCirc` | Float | `0.5` | Planned |  | Circle CPU tessellation factor |
| `TessArc` | Float | `0.5` | Planned |  | Arc CPU tessellation factor |
| `TessEll` | Float | `0.7` | Planned |  | Ellipse tessellation factor |
| `TessEArc` | Float | `0.7` | Planned |  | EllipseArc tessellation factor |

### Grid & CARD  (4)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `GrdEnb` | Bool | `true` | Active | yes | Background grid display (GRIDMODE) |
| `GrdSnp` | Bool | `false` | Active | yes | Snap cursor to grid intersections (SNAPMODE) |
| `GrdSpc` | Float | `10` | Active | yes | Grid spacing in world units (GRIDUNIT) |
| `CrdEnb` | Bool | `false` | Active | yes | CARD cardinal-directions drafting lock |

### UCS Icon  (3)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `UcsIcn` | Bool | `true` | Active | yes | UCS indicator on/off (UCSICON) |
| `UcsMod` | Choice(corner/origin) | `0` | Active | yes | UCS icon placement mode |
| `UcsAvP` | Text | `` | Active | yes | Path to UCS X-axis avatar image |

### Drafting Defaults  (8)

| Name | Type | Default | Status | Edit | Description |
|---|---|---|---|---|---|
| `FltRad` | Float | `0` | Active | yes | Default fillet radius (FILLETRAD) |
| `ChmDs1` | Float | `0` | Planned | yes | Default chamfer distance, first line (CHAMFERA) |
| `ChmDs2` | Float | `0` | Planned | yes | Default chamfer distance, second line (CHAMFERB) |
| `OfsDis` | Float | `1` | Active | yes | Default offset distance (OFFSETDIST) |
| `WlThk` | Float | `0.2` | Active | yes | Default wall thickness |
| `TxHt` | Float | `0.25` | Active | yes | Default text height (world units) |
| `WlCnL` | Bool | `true` | Active | yes | Wall centerline visible |
| `TrmMd` | Bool | `true` | Active | yes | Trim mode shared by Fillet and Chamfer (TRIMMODE) |
