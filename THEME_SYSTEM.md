# THEME_SYSTEM

Design tokens, the Theme Editor, token inheritance, and live updates. Parent:
[ARCHITECTURE.md](ARCHITECTURE.md) §4. This doc also holds the **locked token
registry** (the design-system decisions made so far).

> Status: **token values are decided** (design-system review). The
> `ThemeStore` + Theme Editor are **Proposed**. Today values are scattered
> constants in `cad_app` (e.g. `PP_*`); the target is one token store every
> component reads.

---

## 1. Principles

1. **Components consume tokens, never hard-coded values.** No `Color32::from_rgb`
   or magic px in component code — read a token.
2. **Three token layers** (resolution order):
   `primitive` (raw value) → `semantic` (role) → `component` (optional override).
   A component reads the most specific available, falling back up the chain.
3. **One active theme**, swappable; multiple **named themes** stored.
4. **Live editing**: changing a token updates the whole app immediately.
5. **Dark-only** today; the layering supports a future light variant without
   touching components.

---

## 2. Token model

```rust
// Interface sketch — not final.
pub struct DesignTokens {              // the resolved active theme
    pub color:   ColorTokens,
    pub surface: SurfaceTokens,
    pub text:    TextTokens,
    pub space:   SpaceScale,
    pub radius:  RadiusScale,
    pub type_:   TypeScale,
    pub state:   StateTokens,
    pub motion:  MotionTokens,
    pub elevation: ElevationTokens,
    // component-level overrides keyed by component id (optional)
}

pub trait ThemeStore {
    fn tokens(&self) -> &DesignTokens;            // what components read
    fn set(&mut self, key: TokenKey, value: TokenValue);  // edit → invalidate+repaint
    fn undo(&mut self); fn redo(&mut self);
    fn active(&self) -> ThemeId;
    fn switch(&mut self, id: ThemeId);
    fn save(&mut self, name: &str) -> ThemeId;
    fn duplicate(&mut self, id: ThemeId) -> ThemeId;
    fn import(&mut self, data: &[u8]) -> Result<ThemeId>;
    fn export(&self, id: ThemeId) -> Vec<u8>;
    fn reset(&mut self);                          // back to built-in default
}
```

---

## 3. Live update flow

```
Theme editor edits a token
   └▶ ThemeStore.set(key, value)        record undo step
        └▶ invalidate cached visuals
             └▶ ctx.request_repaint()
                  └▶ components re-read DesignTokens next frame  → whole-app update
```

Edits are **undoable/redoable** as a dedicated theme-edit history (separate from
document undo). Import/Export uses a stable serialized format (JSON or RON);
named themes + a (future) version field travel with the file.

---

## 4. Theme Editor (a `Tools` panel)

Lives under **Tools → Theme editor** as a normal `Panel`
([PANEL_SYSTEM.md](PANEL_SYSTEM.md)). It edits **tokens**, never individual
widget properties. Sub-sections (one per token domain):

`Colors · Typography · Buttons · Icons · Spacing · Radius · Animations ·
Elevation · States · Data tables · Charts · Forms · Navigation`

Commands (registered, so they appear in palette/menus):
*Change primary color · Reset theme · Save theme · Duplicate theme · Import theme
· Export theme · Undo / Redo theme edit · Switch theme.*

> The Theme Editor's sub-sections **are** this project's design system — editing
> the spec becomes a shipped feature.

---

## 5. Locked token registry

The decided values (dark theme). These seed the built-in default theme.

### 5.1 Spacing (4px base)
`xxs 2 · xs 4 · sm 8 · md 12 · lg 16 · xl 24 · xxl 32`
Relationships: field/control height **24**, row→row **8** (pitch 32), label→input
**8**, input padding **8**, section header→content **12**, group gap **12**,
Start↔End column gap **12**, panel edge **16**, panel header→content **24**.
Icon box **24**.

### 5.2 Radius
`xs 2 (swatches/micro) · sm 4 (inputs, value boxes, chips) · md 8 (buttons, icon
buttons, dropdowns) · lg 12 (cards, panels, menus) · full (pills, toggles, nav)`

### 5.3 Surfaces (teal-navy)
`surface-0 #141C25` canvas · `surface-1 #1A2430` panel · `surface-2 #222B34`
raised · `surface-3 #2A3744` popover · `surface-chrome #223040` header/footer ·
`border #34414B` · `accent #00E5FF`.

### 5.4 State
`hover` white 6% · `hover-accent rgba(0,229,255,0.12)` · focus ring **2px cyan,
2px offset** (`:focus-visible` only; in egui shown only on keyboard focus) ·
pressed = darken + `scale(0.96)` · disabled = `text-disabled` + desaturated.

### 5.5 Text
`primary #DAE3EF · secondary #AEB9C4 · muted #93A1AC (labels/placeholders) ·
disabled #5C6975 · accent/link #00E5FF · on-accent #063B45`.
Contrast: primary/secondary/muted pass AA on surfaces; disabled intentionally
exempt.

### 5.6 Semantic
`success #34D399 · warning #F2B53D · danger #E5484D` (no `info` — would clash
with cyan). Each: solid + 15% tint bg + bright text; on-solid = white on danger,
dark on success/warning. **Status = icon + text + color**, never color alone.

### 5.7 Type
Two faces: **Geist** (UI) + **JetBrains Mono** (data). Two weights (400/500).
`title Geist 16/500 · body 13/400 · body-strong 13/500 · caption 11/500 ·
hint Geist 11/400 · data-value Mono 12/400 · data-code Mono 11/400`. Cap at 16px.
Geist for UI text incl. layer/style names; Mono for numbers, coordinates, units,
handles, badges.

> **Post-lock addition (2026-07-02, owner-approved):** `hint` (Geist 11/400) —
> the secondary / subtitle role. It is the only 11px Regular style; `caption`
> stays 11/500 for headings.

### 5.8 Motion
`instant 0 (canvas/cursor/values) · fast 80 (hover/focus/press) · snap 120
(menus/tooltips/toggles) · base 160 (section/dock)`. Easing `ease-out (0.2,0,0,1)`
for enters, `ease-standard (0.4,0,0.2,1)` otherwise. No bounce. Honors reduced-
motion. Canvas/real-time interactions are always 0ms.

### 5.9 Elevation
Depth via the surface ladder (§5.3); in-flow elements flat (border + tone). One
`shadow-popover` reserved for floating/overlay layers only.

### 5.10 Menus
Item 28px (26 in selects), square corners, `surface-3` + popover shadow, hover
white 6%, right-aligned muted-mono shortcuts, `›` submenu arrow, hairline
dividers; **toggle = stroked box (empty off / cyan-check on)**, **current =
solid cyan ■**, 16px aligned lead column, disabled = dimmed + reason.

### 5.11 Data tables
Header on `surface-chrome`, muted-mono labels; names left (Geist) / numbers right
(mono); list rows **28px**, hairline dividers; hover white 6%; **selected = cyan
tint + 2px left bar**; sortable headers; frozen = `text-disabled`; no zebra;
multi-select (Ctrl toggle / Shift range).

### 5.12 Icons
**Lucide** line set, outline-only, **1px** stroke (Lucide generated at
stroke-width 1; CAD command glyphs hand-drawn at the matching ~1px weight),
rounded caps/joins, monochrome (inherits colour). Sizes **16** (inline/menu) ·
**18** (rail/toolbar) · **24** box. idle = muted · active = accent · disabled =
text-disabled · destructive = danger. Two families read as one: hand-drawn CAD
glyphs (need true geometry) + Lucide UI icons. Replace all legacy emoji glyphs.

### 5.13 Global states
- **Loading — non-blocking.** Indeterminate spinner or determinate progress bar
  in the status bar / panel header; the canvas never blocks. See
  [Background_Ops_Pattern.md](Background_Ops_Pattern.md).
- **Empty — invitation, not apology.** Centered muted icon + headline naming the
  space + one-line hint + optional CTA (Inspector no-selection, empty Block
  library, history greeting).
- **Error — inline, non-modal, actionable, semantic danger, never raw
  exceptions** (AGENTS.md rule 10): field validation = red border + ring +
  message under the field; op failure = danger status chip ("Fillet needs
  exactly 2 lines"); recoverable = re-prompt in the command flow; file/IO =
  danger banner. Toasts/notifications deferred to the reserved Bottom Dock.

### 5.14 Forms
Field types: text · number+unit · dropdown · checkbox (stroked box) · segmented ·
slider · read-only/computed · **Mixed** (multi-value). Specialized renderers:
**layer** = color swatch + name · **color** = swatch + name · **linetype** =
dashed-line preview + name · **lineweight** = thickness preview + value (9px gap
from preview to text). Coordinate pair = two boxes in Start/End columns; derived
values (Length, Angle) = read-only, muted, no border. **Value text left-aligned
to a unified start.** Layouts: label-left (Inspector/dense) · stacked (dialogs) ·
two-column grids. **Commit:** Inspector = live-apply + Ctrl+Z; dialogs =
commit-on-Enter/OK; Esc reverts. **Validate** on commit/blur = red border +
message below; number fields drag-to-scrub; no "required" asterisk. Sections
separated by **hairline dividers**; collapsed = `▸` header. The **dobject type
shows in the Inspector header**, not as a field. Button labels use **body-strong**
(Medium 13) — actions carry emphasis over plain labels. (Per-type property schema →
[Dobject_Properties.md](Dobject_Properties.md); wording/number formatting →
[CONTENT_STYLE.md](CONTENT_STYLE.md).)

> **Design-system review complete.** All 16 points + Forms locked
> (Post-lock additions are dated inline — see §5.7 `hint`.); the content
> half lives in [CONTENT_STYLE.md](CONTENT_STYLE.md). **Charts are deferred** —
> not useful for this app yet (diagnostics only); a lean set (line/area, bar,
> donut, sparkline; 1px lines, cyan + violet/amber categorical) was sketched but
> is unbuilt. The Theme Editor's Charts sub-section stays a placeholder until a
> diagnostics/analysis panel needs it.

---

## 6. Phased plan

- **P1** — `DesignTokens` + `ThemeStore`; migrate scattered constants (`PP_*`,
  rail/menu colors) to read tokens. No Theme Editor yet (values fixed in code).
- **P2** — Theme Editor panel editing tokens live; undo/redo; reset.
- **P3** — named themes; import/export; (future) versioning, light variant.
