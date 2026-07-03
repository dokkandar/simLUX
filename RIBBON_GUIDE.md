# RUST_CAD — Ribbon / Toolbar (build-one-like-ours guide)

> Instruction doc for another coding agent. Explains how the RUST_CAD ribbon is
> built so you can reproduce the same look-and-feel in another egui/eframe app.
> The whole thing is **custom-painted with `egui::Painter`** — no image assets,
> no icon font, no `egui::Button`. Every button is a 56×52 rectangle we draw
> ourselves with vector strokes. That's the trick that makes it look like a CAD
> ribbon instead of a row of default buttons.
>
> Code: `cad_app/src/app.rs` — panels at `update()`; helpers `tool_button`
> @18061, `cmd_button`/`cmd_button_resp` @17715/17727, `panel_button` @17682,
> `arc_tool_button` @~18360, glyph painters `draw_cmd_glyph` @17773 +
> `paint_arc_method_icon` @18487. Line numbers from `36ee804`.

---

## 1. The big picture

```
TopBottomPanel::top("menubar")   ← classic dropdown menus (File / Edit / Draw / …)
TopBottomPanel::top("toolbar")   ← the RIBBON: two horizontal strips of icon buttons
   row 1: pointer · line · rect · circle · hatch · ellipse · … · arc methods · | · array · snap · grips · settings · layers · pens
   row 2: undo redo | move copy rotate scale mirror stretch align | trim extend fillet chamfer offset join break … | array match →layer | erase | dist list | block insert explode
CentralPanel                     ← the canvas
TopBottomPanel::bottom("status_bar")
```

Two stacked `TopBottomPanel::top` panels (declared in that order so menubar sits
above the ribbon). The ribbon itself is **two `ui.horizontal` rows** of
custom-painted buttons, with `ui.add_space(N)` as group separators.

There are **three button families**, all 52 px tall and color-matched so the
strip reads as one piece:

| Helper | Shape | Purpose | Returns |
|---|---|---|---|
| `tool_button` | 56×52, painted icon | pick a persistent **draw tool** (`self.tool`) | `bool` clicked |
| `cmd_button` | 56×52, painted glyph + label | run a **one-shot command** (move/trim/erase…) | `bool` clicked |
| `panel_button` | text width × 52 | **toggle a panel / flag** (settings, grips, layers) | `bool` clicked |

(`cmd_button_resp` is `cmd_button` that returns the full `Response` so you can
hang a popup off it; `arc_tool_button` is a `tool_button` variant that also sets
a sub-method.)

---

## 2. The shared visual contract (copy these constants)

Every button uses the same palette + geometry so the ribbon is one consistent
strip:

```rust
const BTN: egui::Vec2 = egui::vec2(56.0, 52.0);   // standard icon-button size
let rounding = 5.0;
let bg_selected = egui::Color32::from_rgb( 60, 110, 175);  // active tool / open panel (blue)
let bg_hover    = egui::Color32::from_rgb( 48,  58,  72);
let bg_idle     = egui::Color32::from_rgb( 28,  34,  42);
let border      = egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 80, 95));
let ink         = egui::Color32::from_rgb(225, 235, 245);  // icon + label colour
```

The state→colour rule is identical everywhere: **selected/active → blue**,
**hover → light slate**, **idle → dark slate**. That single rule is what makes
the disparate buttons feel like one toolbar.

---

## 3. The three button helpers (the whole pattern)

### Tool button — `tool_button`
Picks a persistent drawing tool. Note: it's `allocate_painter(BTN, Sense::click())`,
then we paint the background and a per-tool vector icon, and flip `*current` on
click.

```rust
fn tool_button(ui: &mut egui::Ui, current: &mut Tool, this: Tool, label: &str) -> bool {
    let selected = *current == this;
    let (resp, painter) = ui.allocate_painter(egui::vec2(56.0, 52.0), egui::Sense::click());
    let rect = resp.rect;
    let bg = if selected { BG_SELECTED } else if resp.hovered() { BG_HOVER } else { BG_IDLE };
    painter.rect(rect, 5.0, bg, BORDER);

    let c   = rect.center() - egui::vec2(0.0, 4.0);     // icon centre (nudged up for the label)
    let pen = egui::Stroke::new(1.8, INK);
    let dot = |p| painter.circle_filled(p, 1.8, INK);
    match this {                                         // ← the icon is drawn here
        Tool::None  => { /* arrow strokes */ }
        Tool::Line  => { painter.line_segment([c+vec2(-14.,10.), c+vec2(14.,-10.)], pen); dot(..); dot(..); }
        Tool::Circle=> { painter.circle_stroke(c, 13.0, pen); dot(c); }
        // … one arm per Tool variant …
    }
    // (label is painted under the icon — see cmd_button for the exact call)
    if resp.clicked() { *current = this; }
    resp.clicked()
}
```

### Command button — `cmd_button`
One-shot ops. The icon comes from a `GlyphKind` enum routed through
`draw_cmd_glyph`; a small label sits at the bottom; a hover tooltip explains it.

```rust
fn cmd_button_resp(ui: &mut egui::Ui, label: &str, glyph: GlyphKind, tip: &'static str) -> egui::Response {
    let (resp, painter) = ui.allocate_painter(egui::vec2(56.0, 52.0), egui::Sense::click());
    let rect = resp.rect;
    let bg = if resp.hovered() { BG_HOVER } else { BG_IDLE };
    painter.rect(rect, 5.0, bg, BORDER);
    let c = rect.center() - egui::vec2(0.0, 6.0);
    let pen = egui::Stroke::new(1.6, INK);
    draw_cmd_glyph(&painter, c, glyph, pen, |p| painter.circle_filled(p,1.8,INK), INK);
    painter.text(egui::pos2(rect.center().x, rect.bottom()-9.0),
                 egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(9.5), INK);
    resp.on_hover_text(tip)
}
fn cmd_button(ui, label, glyph, tip) -> bool { cmd_button_resp(ui, label, glyph, tip).clicked() }
```

### Panel toggle — `panel_button`
Text-only, width fits the label (min 56), blue when its panel/flag is on:

```rust
fn panel_button(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let galley = ui.painter().layout_no_wrap(label.into(), FontId::proportional(12.0), INK);
    let size = egui::vec2((galley.size().x + 20.0).max(56.0), 52.0);
    let (resp, painter) = ui.allocate_painter(size, egui::Sense::click());
    let bg = if active { BG_SELECTED } else if resp.hovered() { BG_HOVER } else { BG_IDLE };
    painter.rect(resp.rect, 5.0, bg, BORDER);
    painter.galley(resp.rect.center() - galley.size()*0.5, galley, INK);
    resp.clicked()
}
```

---

## 4. Drawing the icons (no assets)

Icons are **a few vector primitives drawn relative to the button centre** —
`painter.line_segment`, `circle_stroke`, `rect_stroke`, `circle_filled` (dots).
Two routing styles:

- **Tool icons:** a `match this { Tool::X => … }` inside `tool_button` (each tool
  draws its own strokes). Example: Line = one diagonal segment + two endpoint
  dots; Circle = `circle_stroke` + centre dot.
- **Command icons:** a `GlyphKind` enum (`Move, Copy, Rotate, Trim, Fillet, …`)
  + a `draw_cmd_glyph(painter, center, kind, pen, dot, ink)` dispatcher. Example:
  Move = four-headed arrow; Copy = two overlapping `rect_stroke`s; Rotate =
  a 270° polyline arc + arrowhead.

A helper closure `let v = |x,y| c + vec2(x,y);` makes the stroke coordinates
readable (offsets from centre). The code calls these **placeholders** — they're
intentionally simple vector sketches; swap for real graphics later without
changing any call sites.

---

## 5. The ribbon assembly

```rust
egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
    ui.add_space(4.0);
    // ---- ROW 1: draw tools + panel toggles ----
    ui.horizontal(|ui| {
        let prev = self.tool;
        tool_button(ui, &mut self.tool, Tool::None, "pointer");
        ui.add_space(4.0);
        tool_button(ui, &mut self.tool, Tool::Line, "line");
        tool_button(ui, &mut self.tool, Tool::Rectangle, "rect");
        if tool_button(ui, &mut self.tool, Tool::Circle, "circle") {  // special routing:
            self.tool = Tool::None; self.circle_flow_start();          // circle uses the cmd_flow
        }
        if hatch_command_button(ui) { self.run_command("hatch"); }     // one-shot icon
        // … ellipse, point, pline, spline, wall, text, dim, arc methods …
        if self.tool != prev { self.pending.clear(); /* + last_command, dim routing */ }
        ui.add_space(20.0);
        if panel_button(ui, "settings…", self.settings_open) { self.settings_open ^= true; }
        if panel_button(ui, "layers",    self.layer_panel_open) { self.layer_panel_open ^= true; }
        // … snap / grips / pens / info …
    });
    // ---- ROW 2: modify commands, data-driven groups ----
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if cmd_button(ui, "undo", GlyphKind::Undo, "Undo last edit") { self.run_command("undo"); }
        if cmd_button(ui, "redo", GlyphKind::Redo, "Redo")           { self.run_command("redo"); }
        ui.add_space(8.0);                                            // ← group separator
        for (lbl, kind, cmd, tip) in [
            ("move",   GlyphKind::Move,   "move",   "Translate selection by 2 picks"),
            ("copy",   GlyphKind::Copy,   "copy",   "Copy selection by 2 picks"),
            ("rotate", GlyphKind::Rotate, "rotate", "Rotate selection around a pivot"),
            // …
        ] { if cmd_button(ui, lbl, kind, tip) { self.run_command(cmd); } }
        ui.add_space(8.0);
        // Edit-geometry group: trim/extend/fillet/chamfer/offset/join/break/… (same table pattern)
    });
});
```

**Group separation** is just `ui.add_space(8.0)` between logical clusters
(History / Transform / Edit-geometry / Properties / Blocks). The **table-driven
`for (lbl, kind, cmd, tip) in [ … ]`** loop is the cleanest way to add buttons —
one row of data per command, the body is one line.

---

## 6. Wiring a button to behavior

- **Tool buttons** only flip `self.tool`. After the row, one guard does the
  rest: `if self.tool != prev { self.pending.clear(); … }` — clear any
  in-progress draft, record `self.last_command` (so empty-Enter repeats the
  tool), and special-case tools that need a flow (`Dim` → `run_command("dim")`,
  `Circle` → `circle_flow_start()`). Keep that routing in **one** place, not per
  button.
- **Command buttons** call `self.run_command("move")` etc. — the *same* entry
  point as typing the command, so the ribbon and the command line share all
  logic (and the Session Recorder logs both identically). This is the key
  principle: **a ribbon button is just a typed command with an icon.**
- **Panel buttons** toggle a `bool` (`self.settings_open ^= true`) and pass that
  same bool as `active` so the button lights blue while the panel is open.

---

## 7. Popups hanging off a button (e.g. Insert ▾ block list)

Use `cmd_button_resp` (returns `Response`) + a persistent popup id:

```rust
let resp = cmd_button_resp(ui, "insert", GlyphKind::Insert, "Insert a block");
let popup_id = ui.make_persistent_id("toolbar_insert_popup");
if resp.clicked() { ui.memory_mut(|m| m.toggle_popup(popup_id)); }
egui::popup_below_widget(ui, popup_id, &resp, /* … list of block names … */);
```

---

## 8. Recipe — reproduce it in another egui app

1. **Constants:** define `BTN = vec2(56,52)`, the 3 bg colours, `border`, `ink`,
   `rounding = 5.0` (§2).
2. **Three helpers:** copy `tool_button`, `cmd_button(_resp)`, `panel_button`
   (§3). Each is `allocate_painter(BTN, Sense::click())` → paint bg by state →
   paint icon/label → return `clicked()`.
3. **Icon dispatch:** a `GlyphKind` enum + `draw_cmd_glyph()` for commands, and a
   `match` on your tool enum inside `tool_button` for tools. Draw with
   `line_segment`/`circle_stroke`/`rect_stroke` relative to centre (§4).
4. **Assemble:** `TopBottomPanel::top("toolbar")` → one or two `ui.horizontal`
   rows → buttons separated into groups with `ui.add_space(8.0)` (§5).
5. **Data-drive the big groups** with `for (lbl, kind, cmd, tip) in [ … ]`.
6. **Wire to one command entry point** (`run_command`) so the ribbon mirrors the
   command line; flip `self.tool` for draw tools and handle the diff once (§6).
7. (Optional) menubar panel above it; popups via `cmd_button_resp` (§7).

---

## 9. Conventions & gotchas

- **Custom-paint, don't use `egui::Button`.** Uniform 56×52 sizing + vector
  icons + the exact state→colour rule are what sell the "CAD ribbon" look;
  default buttons can't give you that consistently.
- **One state→colour rule across all three helpers** (selected=blue,
  hover=slate, idle=dark). Don't let any button drift.
- **Buttons are commands with icons.** Always route through `run_command` (or the
  flow starter) — never duplicate command logic in the button handler. The
  Session Recorder + repeat-last + history all depend on this.
- **Handle the `self.tool` change once**, after the row, not per button — that's
  where `pending.clear()` + `last_command` + flow routing live.
- Icons are **placeholders** by design (a few strokes). They're swappable later;
  call sites won't change.
- `panel_button` width is label-driven (min 56) so multi-word/`\n` labels
  (e.g. `"grips\nON"`) lay out cleanly while staying 52 tall.
