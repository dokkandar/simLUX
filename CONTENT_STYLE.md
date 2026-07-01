# CONTENT_STYLE

The content & voice rulebook: terminology, capitalization, phrasing, number/unit
formatting, and message copy. Every label, message, and number in the UI —
including plugin-authored ones — follows this. Parent:
[ARCHITECTURE.md](ARCHITECTURE.md) §4; pairs with [THEME_SYSTEM.md](THEME_SYSTEM.md)
(visual tokens) as the *content* half of the design system.

> Status: **rules decided.** Enforcement helpers (§6) are *Proposed*.

---

## 1. Terminology (glossary)

One canonical word per concept. Enforceable — plugins must comply.

| Concept | Use | Avoid |
|---|---|---|
| a drawing entity | **dobject** / `DObject` | object |
| the properties panel | **Inspector** | Properties panel |
| object snap | **Dobject snap** (button: `DSnap`) | object snap, osnap *(UI text)* |
| linetype scale | **Lt scale** | LTScale, LinetypeScale *(UI text)* |
| remove a dobject | **Delete** | Erase *(UI label)* |
| layer visibility | **Hide / Show** | on/off *(as a verb)* |
| layer freeze | **Freeze / Thaw** | |
| lock state | **Lock / Unlock** | |
| make active | **Set current** | set active |

Notes:
- **Delete** is the UI verb; the command-line keywords `erase` / `e` **stay as
  aliases** (AutoCAD muscle memory) — aliases are not UI text.
- The glossary grows over time; add a row before shipping a new term.
- Canonical action verbs (reuse exactly): Delete · Duplicate · Rename · Move ·
  Copy · Rotate · Scale · Mirror · Offset · Trim · Extend · Fillet · Chamfer ·
  Hide · Show · Freeze · Thaw · Lock · Unlock · Set current.

---

## 2. Capitalization

**Sentence case everywhere** — commands, labels, menu items, tab titles,
tooltips, buttons. Proper nouns only (RUST_CAD, DXF). Never Title Case, never
ALL CAPS — **exception:** the mono `SECTION` headers use caps as a deliberate
style treatment (a visual token, not content).

---

## 3. Phrasing & labels

- **Verb-first for actions** ("Delete layer", not "Layer deletion"). **Noun for
  labels/fields** ("Line weight", "Lt scale").
- **No terminal punctuation** on labels, buttons, or menu items.
- **Trailing `…`** means *opens a dialog / needs more input* (`Block…`, `Hatch…`,
  `Settings…`). A command that acts immediately has no `…`.
- Keep field labels short; the unit or type carries the rest ("Weight" + `mm`).

---

## 4. Number & unit formatting  *(single source of truth)*

All displayed numbers go through the formatters in §6 — never `format!("{}", v)`
ad hoc.

- **Precision follows the settings panel.** Linear, angular, and coordinate
  precision read the current display-precision variables (see
  [Variables.md](Variables.md) / [SETTINGS.md](SETTINGS.md)); the formatters
  re-read them live when settings change. No hard-coded decimal counts.
- **Trailing zeros** are shown to the active precision (`1.00`, not `1`).
- **Units:** linear = value + space + unit (`0.25 mm`); angle = value + `°`, no
  space (`45.00°`); area = value + space + unit² (`12.48 m²`).
- **Coordinates:** `X, Y` order, `, ` separator, linear precision.
- **Angles:** decimal degrees, CCW positive, 0° = east. **DMS deferred** — add a
  format toggle later; the `format_angle` helper is the seam for it.
- **No thousands separator** (CAD convention — avoids coordinate ambiguity).
- **Indeterminate (multi-select) value = `Mixed`** everywhere. This **unifies and
  replaces** the current `*VARIES*` / `various` strings.
- **Special values** spelled: `By Layer`, `By Block`, `Default`, `Continuous`.

---

## 5. Message patterns

- **Errors** — what happened, then what to do; one line; **no "Error:" prefix**,
  **never raw exception text** (AGENTS.md rule 10). "Fillet needs exactly 2
  lines." Field validation: red border + message below the field.
- **Empty states** — an invitation: headline names the space, one-line body, a
  verb CTA. Not "Nothing here yet."
- **Confirmations** — past tense, terse, no first person, no "!": "Layer deleted."
- **Prompts** (command line) — consistent phrasing; see
  [COMMAND_LINE.md](COMMAND_LINE.md).
- **Tooltips** — name + alias + accelerator, e.g. `Undo (U · Ctrl+Z)`.
- **Placeholders** — a real example of valid input, not the label repeated.

---

## 6. Enforcement

- A shared **`fmt`** module is the only place numbers become strings:
  `format_length` · `format_angle` · `format_coord` · `format_area` ·
  `format_scale`, plus a multi-value wrapper that renders `Mixed`. All read
  current settings precision.
- Panels and plugins use the **glossary** for terms and the **formatters** for
  numbers — never their own copies. This keeps every surface (including plugin
  UI) consistent and lets a precision or terminology change propagate app-wide.

---

## 7. Reconciliation

Related sources of truth this doc governs the *presentation* of, without
overriding their data:
- [Dobject_Properties.md](Dobject_Properties.md) — property names / schema.
- [Variables.md](Variables.md) / [SETTINGS.md](SETTINGS.md) — precision + unit
  sysvars the formatters read.
- [COMMAND_LINE.md](COMMAND_LINE.md) — prompt phrasing.
- [feedback: dobject terminology] — the "dobject not object" rule (memory).
