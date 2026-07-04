# DROPDOWN MENU — category menus (mentor spec)

The menubar category dropdowns (Draw, Modify, …) and their submenus. Defines the
row layout, icon column, current-method marker, hover, spacing (matched to the
command palette), and the **arrow-column alignment rule** (which applies to every
menu/submenu app-wide).

Shares: the label notation and current-method marker from `METHOD_ACCESS_MENTOR.md`
(§1/§5); the icon language from `THEME_SYSTEM.md` §5.12. Parent tokens in
`THEME_SYSTEM.md` §5. **No raw numbers — every value is a named token in `theme.rs`.**

A menubar dropdown is a **flyout**, not a titled surface → it gets **no header band**.

---

## 1. Row layout

Every command is one row, left → right:

```
[icon]  Name (CODE)                         ▸
└20┘ 12 └──── label zone ────┘  6  └arrow┘
```

- **Icon column (all commands):** every command shows an icon — **20px box**, one
  **uniform thin (~1px) stroke**, muted tone, in a single aligned column. If a command
  genuinely has no glyph, **reserve the empty 20px slot** so names stay aligned.
  Method commands (Circle/Arc/Fillet) show the **method-aware** glyph (current method's
  construction glyph). Icon box follows the **shared icon-box rule** (`box = band − 6`,
  glyph scales to fill) — `METHOD_ACCESS_MENTOR.md` §4/§7 — so an icon is the same
  physical size here as in the palette.
- **icon → name gap: 12.**
- **Name:** body (Geist 13/400), `text-primary`.
- **(CODE):** only on method commands — the current method, `data-code` (Mono 11),
  **cyan** (see §4), one **name↔code gap (6)** before it. Format `Name (CODE)` UPPERCASE
  (`METHOD_ACCESS` §1).
- **Arrow ▸:** only where the row opens a submenu (method submenu OR real submenu like
  Insert Block). One **unified size + tone** for all arrows (`text-muted`).

Metrics are **matched to the command palette**: same 13 text, same 20 icon, same 12
icon-gap, same **7px vertical row padding** → the row height / hover box is identical
to a palette row.

---

## 2. Arrow-column alignment (rule — applies to EVERY menu/submenu)

- The **arrow column x = (right edge of the longest full line in the menu) + the
  name↔code gap (6)**. The longest line includes any parenthetical (e.g.
  `Wall (t = thickness)`).
- **All** submenu arrows in that menu align to that single column.
- The menu is **exactly wide enough** to fit `icon + longest line + gap + arrow +
  edge padding` — no far-edge void, no arbitrary width.
- This is the alignment reference for the whole menu; every arrow follows it.

---

## 3. Hover

Full-width row fill one elevation step above the menu fill (`surface-2`-equivalent),
text unchanged, **instant** (no animation) — identical to the palette/rails hover.

---

## 4. Current-method marker (method commands)

`Circle (CR)`, `Arc (3P)` — the `(CODE)` is **cyan** (`accent`); the command name is
normal tone. **No `□` checkbox, no `●`.** Colour is the only marker (shared with
`METHOD_ACCESS_MENTOR.md` §5). Split behavior unchanged: click the **name** → run the
remembered method; click the **▸** → method submenu (`METHOD_ACCESS` §2).

---

## 5. Labels & dividers

- **Wall:** display `Wall (t = thickness)` — the parenthetical is a **muted hint**
  (`text-muted`); drop the `chained run —` text.
- **Dialog-opening commands** keep the `…` suffix (`Hatch…`, `Block…`).
- **Group dividers:** keep the existing 1px `border` hairlines between groups (e.g.
  above `Block…` / `Insert Block`).

---

## 6. Exit criteria

- Every command has an icon (or a reserved slot); all icons one stroke + one size,
  matched to the palette; method commands show the method-aware glyph.
- No `□`/`●`; current method shown by cyan `(CODE)`.
- Hover fill matches the palette; row height/spacing matches the palette.
- All arrows one size; arrows align to the longest-line + 6 column; menu width hugs
  content (no void). Rule holds for submenus too.
- `Wall (t = thickness)` (no `chained run —`); `…` kept on dialog commands; group
  dividers intact.
- No header band on the dropdown.
