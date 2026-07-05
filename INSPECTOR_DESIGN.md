# INSPECTOR — Design Rule (single source for the panel)

> ⚠️ **Superseded by INSPECTOR_DESIGN_MENTOR.md** (`mentor MD/INSPECTOR_DESIGN_MENTOR.md`).
> That doc is the current authority for the Inspector; where the two differ, it
> wins. This file stays as historical reference.

The finalized rule for the **Inspector** panel (formerly "Properties"). This is the
authority the build must match; the companion `inspector_mockup.html` implements
these exact values so they can be inspected in a browser. Parent tokens live in
[THEME_SYSTEM.md](THEME_SYSTEM.md) §5 — this doc is the Inspector-specific
application of them, with **every pixel value stated**.

> Rule of thumb: **no raw numbers in code** — each value below is a named token.
> If a value here changes, it changes in `theme.rs` and propagates.

---

## 1. Tokens used (exact values)

### 1.1 Spacing (compact scale, 4px base)
| Token | px | Where |
|---|---|---|
| `box` / control / icon height | **24** | every field box, dropdown, checkbox, icon |
| `row-gap` | **8** | between rows (row pitch = 24 + 8 = **32**) |
| `label→input` | **8** | horizontal gap, label column → field |
| `input-pad` | **8** | inside a field, edge → text/preview |
| `section→content` | **12** | section header row → its first field |
| `group-gap` | **12** | between one section's end and the next header |
| `column-gap` | **12** | Start ↔ End (coordinate) columns |
| `panel-edge` | **16** | panel inner side padding |
| `panel-header→content` | **24** | header band bottom → first content row |

### 1.2 Radius
| Token | px | Where |
|---|---|---|
| `xs` | **2** | color/linetype swatches, micro chips |
| `sm` | **4** | **inputs, value boxes, dropdowns** |
| `md` | **8** | buttons, icon buttons |
| `lg` | **12** | the panel card itself, menus |
| `full` | 9999 | the header type pill, toggles |

### 1.3 Color (dark teal-navy)
| Token | hex | Where |
|---|---|---|
| surface-0 | `#141C25` | **field/value box fill**, canvas |
| surface-1 | `#1A2430` | **panel body fill** |
| surface-2 | `#222B34` | raised control (hover fill) |
| surface-3 | `#2A3744` | popover / dropdown menu |
| chrome | `#223040` | **header band** fill |
| border | `#34414B` | all 1px borders / dividers |
| accent | `#00E5FF` | focus ring, active, type pill, checkbox on, slider fill |
| on-accent | `#063B45` | text/icon on a solid accent fill |
| text-primary | `#DAE3EF` | values, field text |
| text-secondary | `#AEB9C4` | body text |
| text-muted | `#93A1AC` | **labels, section headers, read-only values, placeholders** |
| text-disabled | `#5C6975` | disabled |
| danger | `#E5484D` | error border + message |

### 1.4 Type
| Role | Font | Size / weight | Where |
|---|---|---|---|
| title | Geist (UI sans) | **16 / 500** | header "Inspector" |
| body | Geist | **13 / 400** | labels, value names (layer/color names) |
| body-strong | Geist | 13 / 500 | — |
| caption | Geist | **11 / 500** | section headers, column headers, units |
| data-value | JetBrains Mono | **12 / 400** | numbers, coordinates, Length/Angle |
| data-code | JetBrains Mono | 11 / 400 | handles, badges |

- **Geist** for all UI text incl. layer/style names. **Mono** for numbers/units/handles.
- Cap any text at 16px.

---

## 2. Panel shell

```
┌─────────────────────────────────────────┐  ← card: surface-1, border 1px #34414B,
│  Inspector                    [ Line ]   │     radius lg(12)
├─────────────────────────────────────────┤  ← header band: chrome #223040, height 40,
│                                          │     bottom hairline #34414B
│   (panel-header→content = 24)            │
│   ▼ GENERAL                              │
│   Layer     [■ O-WALLS            ▾]     │
│   …                                      │
└─────────────────────────────────────────┘
```

- **Card:** fill `surface-1`, 1px `border`, radius `lg` (12). Width when docked = 264
  (float identical). Side padding inside the card = `panel-edge` (16) — content starts
  16px from the left/right card edges.
- **Header band:** fill `chrome`, height **40**, 1px bottom hairline `border`.
  - **Title** "Inspector" — title (Geist 16/500), `text-primary`, at `panel-edge` (16)
    from the left, vertically centered.
  - **Type pill** (right, 12px from the right edge): a `full`-radius chip. Fill =
    accent @ ~10% (`rgba(0,229,255,.10)`), text = `accent`, caption (11/500), padding
    `0 8px`, height **18**. Text = the dobject type, e.g. `Line`, or `Line (4)` /
    `Mixed (4)` for a multi-select. (Per §5.14 the type shows HERE, never as a field.)
  - **Close ×** (optional) sits left of the pill; 20×20 hit-box, muted, brightens on hover.
- **Panel-header→content:** first content row starts **24** below the header band.

---

## 3. Sections

Collapsible groups: **GENERAL**, **GEOMETRY**, then type extras (**HATCH**…), then **MISC**.

- **Header row:** height 18, full width.
  - Chevron: `▼` open / `▸` collapsed, `text-muted`, ~10px, at the left edge of the
    content column.
  - Label: caption (Geist 11/500), `text-muted`, uppercase, 8px right of the chevron.
- **section→content = 12** (header row → first field).
- **group-gap = 12** between a section's last row and the next section header.
- A 1px `border` **hairline divider** may sit above each section header (except the first).
- Collapsed section shows only its header row.

---

## 4. Rows (label-left)

Every property is one **row**, height = `box` (24), rows separated by `row-gap` (8)
→ **pitch 32**.

```
Layer            ■  O-WALLS                     ▾
└─ label col ──┘└─ value field (fills rest) ─────┘
   muted 13       surface-0, border, radius sm(4), height 24
```

- **Label column:** fixed width **84** (so all values share one **unified value
  start**). Text = body (Geist 13/400), `text-muted`, left-aligned, vertically centered.
- **label→input = 8** between the label column and the field.
- **Value field:** fills the remaining width. Height `box` (24), fill `surface-0`,
  1px `border`, radius `sm` (4). Inner padding `input-pad` (8) left/right.
- **Value text:** body (Geist 13/400) `text-primary` for names; data-value (Mono 12/400)
  for numbers. Left-aligned to the same start in every row.
- Rows that are **dropdowns** show a `▾` chevron (`text-muted`, 12px) at the right,
  8px from the field's right edge.

---

## 5. Specialized value renderers

| Field | Renders as |
|---|---|
| **Layer** | color **swatch** (13×13, radius `xs`=2) + **9px gap** + name (Geist 13) + `▾`. Swatch = the layer's resolved color. |
| **Color** | swatch (13×13) + 9px + name (`By Layer`, or ACI/RGB name) + `▾`. |
| **Linetype** | **dashed-line preview** (a ~34px wide dashed stroke, 1.5px, `text-primary`) + 9px + name (`Continuous`) + `▾`. |
| **Lineweight** | **thickness preview** (a short solid bar whose height = the weight, e.g. 0.25mm→1px, capped ~4px, `text-primary`) + 9px + value (`0.25 mm`, Mono 12) + `▾`. |
| **Lt scale / number** | plain field, value = Mono 12, right or left aligned to value start. |
| **Opacity / factor** | **slider**: track `surface-2` 4px tall radius full; fill `accent`; thumb 12px circle `accent`. Value `100%` (Mono 12, `text-muted`) at the right. |
| **Visible / bool** | **checkbox**: 16×16 box, radius `sm`, 1px `border`; when on → fill `accent`, check glyph `on-accent`. |
| **Coordinate pair** | two fields side by side under `Start` / `End` (or `X` / `Y`) captions; `column-gap` (12) between; each field as §4. |
| **Derived (Length, Angle, Area)** | **read-only**: NO field box, NO border; value = Mono 12, `text-muted`, at the value start. |
| **Mixed (multi-value differs)** | field shows `Mixed` in `text-muted` italic; editing writes to all. |

### 5.1 Coordinate block (GEOMETRY)
```
            Start           End           ← column headers: caption 11/500, muted,
X        [ 1250.00 ]     [ 1310.00 ]         above the two fields, column-gap 12
Y        [  840.50 ]     [  905.00 ]
Length      88.60                          ← derived: muted mono, no box
Angle       47.30°
```
- Column headers sit `2px` above the first coordinate row, aligned to each field's left.
- `column-gap` = 12 between the two fields; each field splits the value area:
  `field_w = (value_area − 12) / 2`.

---

## 6. States

- **Hover** (interactive field): border → `accent`; fill unchanged. Cursor pointer for
  dropdowns.
- **Focus** (keyboard): 2px `accent` ring, 2px offset (keyboard focus only).
- **Disabled:** text `text-disabled`, border desaturated, no hover.
- **Error:** field border `danger` + a message below in `danger` caption; the message
  row adds one `row-gap` above the next row.
- **Read-only** (derived): muted, no border, not focusable.

---

## 7. Element measurement table (what devtools should read)

| Element | width | height | radius | notes |
|---|---|---|---|---|
| Panel card | 264 | auto | 12 | border 1, fill surface-1 |
| Header band | 264 | 40 | — | fill chrome, bottom hairline |
| Type pill | auto | 18 | full | pad 0/8, accent @10% |
| Content column | 232 | — | — | = 264 − 2×16 (panel-edge) |
| Section header | 232 | 18 | — | chevron + caption |
| Row | 232 | 24 | — | pitch 32 (24 + row-gap 8) |
| Label column | 84 | 24 | — | muted 13 |
| Value field | 140 | 24 | 4 | = 232 − 84 − 8; surface-0 + border |
| Swatch | 13 | 13 | 2 | 9px gap to text |
| Dropdown chevron | 12 | — | — | 8px from field right |
| Coordinate field | 64 | 24 | 4 | = (140 − 12)/2 |
| Checkbox | 16 | 16 | 4 | accent when on |

*(widths assume the docked 264px panel; they scale with the panel but the fixed
tokens — label col 84, box height 24, radii, gaps — stay constant.)*

---

## 8. Do / Don't

- **Do** left-align every value to the 84px value start.
- **Do** use Mono for all numbers, Geist for names/labels.
- **Do** render derived values borderless + muted.
- **Don't** put the dobject type as a field — it lives in the header pill.
- **Don't** use square field corners — inputs are `sm` (4) radius.
- **Don't** hardcode spacing — every gap is a token from §1.1.
