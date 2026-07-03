# RUST_CAD — How the Command Line Works (current implementation)

> Handoff doc for another coding agent. This describes the command line **as it
> actually works today** in the code. It is *not* the redesign spec — that's
> `COMMAND_LINE.md` (a future "one Command IR + CmdHost trait" refactor that is
> **not started**). When the two disagree, this file wins for present-day code.
>
> Line numbers are accurate as of commit `b952aaf` (2026-06-24). They drift —
> grep the function name if a number is stale.

---

## 1. The 10-second mental model

The command line is a single text buffer (`CadApp.cmd: String`) rendered at the
bottom of the window. Typing fills it; **Enter or Space submits it**. Submission
funnels through one method:

```
fn run_command(&mut self, raw: &str)        // cad_app/src/app.rs:2845
```

`run_command` has **two layers**, in this order:

1. **Stateful intercepts (pre-parser).** If a modal flow is live (a circle flow,
   zoom, pedit, a select session, …) the raw text is routed *there* and the
   function returns early. These never reach the parser.
2. **Stateless parse + dispatch.** Otherwise the text is handed to the pure
   parser in `cad_kernel`, which returns a `Command` enum value, and a big
   `match` executes it.

Empty input never reaches `run_command` (the TextEdit only submits non-empty
text). Empty Enter/Space is handled separately in the `update()` loop as a
**contextual cascade** (finish the current drawing, advance a flow, repeat the
last command, …). That cascade is the second thing to understand (section 6).

```
 keystrokes ─▶ self.cmd ──Enter/Space(non-empty)──▶ run_command(raw)
                  │                                     │
                  │                              ┌──────┴───────┐
                  │                          intercepts?   parse+dispatch
                  │                         (cmd_flow,zoom,  (cad_kernel::
                  │                          pedit,select…)   parser::parse)
                  └──Enter/Space(empty)──▶ update() empty-trigger cascade
                                            (finish draw / repeat last / …)
```

---

## 2. File / function map (where to look)

| Concern | Location |
|---|---|
| **Grammar / parser** (pure, no app state) | `cad_kernel/src/parser.rs` — `parse()`, `Command`, `ToolKind` |
| **Command dispatcher** | `cad_app/src/app.rs` — `run_command()` @2845 |
| **Empty Enter/Space cascade** | `app.rs` `update()` @~18888–19180 |
| **Repeat-last-command** | `app.rs` @~19134 (`self.last_command`) |
| **Prompt / history helpers** | `set_prompt()`/`clear_prompt()` @2835; `self.history`, `self.current_prompt` |
| **Selection sessions** | `begin_selection()` @4729, `SelectMode` @1375, `QueuedOp` @1406 |
| **New prompt-driven flow (CIRCLE)** | `CmdFlow` @1764, `CircleStep` @1774, `circle_flow_start()` @13437, `flow_input_text()` @13666 |
| **ZOOM flow** | `ZoomState` @1800, `zoom_start/zoom_input_text/zoom_input_point/zoom_finish` |
| **PEDIT flow** | `PeditState` @1489, `pedit_start/pedit_input/pedit_exit`; methods in `cad_app/src/app/pedit.rs` |
| **PLINE tool sub-flow** | `PlineMode`/`PlineArcSub`/`PlineWidthCap` @48–70, `update_pline_prompt()` @15840 |
| **Drawing tools** | `Tool` enum @42; clicks accumulate into `self.pending` |
| **Session recorder hook** | `DbgEvent::CmdRun` emitted at top of `run_command` |

---

## 3. The data path of one typed command

Take the user typing `f 10` then Enter (fillet, radius 10):

1. **Buffer.** Keystrokes land in `self.cmd` (the bottom TextEdit).
2. **Submit.** The TextEdit reports `lost_focus() && Enter`, OR the `update()`
   loop sees `enter_now`/`space_now` with a non-empty buffer → it calls
   `self.run_command(self.cmd.clone())` and clears `self.cmd`.
3. **Echo + record.** `run_command` pushes `command: f 10` into `self.history`
   (prefix is `command:` at the idle prompt, `>` when replying to an active
   prompt — see @2849) and emits a `DbgEvent::CmdRun` for the Session Recorder.
4. **Intercepts.** None match (no flow active), so we fall through.
5. **Select-mode letter rewrite.** `effective` = `raw` unless a select session is
   active (section 8). Here `effective = "f 10"`.
6. **Parse.** `cad_kernel::parser::parse("f 10")` → `Ok(Command::Fillet(Some(10.0)))`.
7. **last_command.** Because the parse succeeded and the text is non-empty,
   `self.last_command = Some("f 10")` (@3895) — so a later empty Enter repeats it.
8. **Canonical echo.** `f` ≠ `fillet`, so it logs `  command: Fillet` (@3909) for
   a readable history regardless of which alias was typed.
9. **Dispatch.** `match parsed { Ok(Command::Fillet(r)) => … }` starts the fillet
   pick flow and sets a prompt.

---

## 4. The parser (`cad_kernel/src/parser.rs`)

Pure function, **no app state**, fully unit-testable:

```rust
pub fn parse(line: &str) -> Result<Command, String>
```

- **Tokenization:** `line.split_whitespace()`. `toks[0]` lowercased = the head.
  Everything is **case-insensitive**.
- **Snap keywords first** (@305): a *single* token that names a `SnapKind`
  (`end`, `mid`, `cen`, `int`, `per`, `tan`, `nea`, `qua`, …) returns
  `Command::SnapOverride(kind)` — a one-shot object-snap override for the next
  click (section 11).
- **Big `match head`** (@311) maps every command word + aliases to a `Command`.
  Aliases are inline in the arm, e.g. `"fillet" | "flt" | "f"`,
  `"copy" | "c" | "cp" | "co"`, `"erase" | "delete" | "e"`.
- **Points** parse as `x,y` (comma, no space) via `parse_pt` (@551). Numbers are
  plain `f64`. Bad input returns `Err(String)`, which the app prints as a hint.
- **Bare keyword ⇒ tool.** Drawing words *with no arguments* return
  `Command::SetTool(ToolKind::…)` (e.g. bare `line` @563, `circle` @609,
  `pline` @590, `arc` @646). *With* arguments they return `Command::Add(geom)`
  built immediately (e.g. `line 0,0 10,10`). This is why `line` enters
  interactive mode but `line 0,0 10,10` draws at once.
- **`Command` enum** (@26) is the full vocabulary — every line/arc variant,
  every modify op (trim/extend/fillet/chamfer/offset/…), selection sub-commands
  (`SelectAll/Window/Crossing/Last/…`), block ops, styles, etc.
- **`Command::canonical_name()`** (@212) returns the capitalized display name
  used for the history "log book" echo (step 8 above).

`ToolKind` (@279) is the parser's mirror of the app's `Tool` enum; the app maps
one to the other when handling `SetTool`.

> **Important:** the parser knows nothing about app modes. Single letters like
> `c` always parse to one fixed command (`c` → `Copy`). Context-sensitive
> meanings (`c` = "crossing" while selecting, or "close" while drawing a pline)
> are imposed by the app *before* it ever calls the parser — see sections 5 & 8.

---

## 5. Stateful intercepts (inside `run_command`, before the parser)

These run top-to-bottom; **the first match returns early**. Order is load-bearing.

| Guard (in order) | Routes to | Why it's first |
|---|---|---|
| `self.cmd_flow.is_some()` @2874 | `flow_input_text()` | a prompt-driven flow (CIRCLE) owns the line |
| text == `circle`/`ci` & not selecting @2878 | `circle_flow_start()` | starts that flow |
| `self.zoom_state != Off` @2890 | `zoom_input_text()` | ZOOM sub-options must not hit the parser |
| `self.pedit_state != Off` @2895 | `pedit_input()` | PEDIT sub-options likewise |
| text == `pedit`/`pe` @2899 | `pedit_start()` (sets `last_command`) | starts PEDIT |
| text == `zoom`/`z` (± arg) @2907 | `zoom_start()` | starts ZOOM |
| text == `group`/`ungroup` @2918 | `group_selection()` / `ungroup_selection()` | act on current selection |
| select session active & text ∈ {p,l,d} @2929 | `run_command("previous"/"last"/"remove")` | session shortcuts |
| `preview [on/off]` @~2937 | toggles `self.draft_preview` | UI toggle |

After all intercepts, the **select-mode letter rewrite** runs (@3868): while a
select session is live, a bare letter is rewritten so the parser yields the
*selection* sub-command, not the same-letter global command:

```
w→window   c|cr→crossing   a→all   b|bef→before   l→last   n→none
```

Then `parse(effective)` and the big dispatch `match` (@3912+).

---

## 6. The empty Enter/Space cascade (in `update()`)

The TextEdit only submits **non-empty** text. An **empty** Enter/Space is a
"I'm done / advance / repeat" signal, handled in `update()` @~18888 onward.

Key setup:
```rust
let enter_now = key_pressed(Enter);
let space_now = key_pressed(Space);
let cmd_is_empty = self.cmd.trim().is_empty();
let in_text_body = /* typing a TEXT string or a height */;
let trigger = enter_now || (space_now && cmd_is_empty && !in_text_body);
```
`Space` doubles as Enter **only on an empty line** and **not while typing text**
(otherwise the first space in "Hello world" would fire it). After firing,
leading whitespace is stripped (`if space_now { self.cmd.clear(); }`).

The handlers form a **mutually-exclusive cascade** — *one Enter = exactly one
state transition per frame* (memo: `feedback_rust_cad_user_terminates_sessions`;
the program never auto-chains through phases, the user does). Roughly in order:

1. Hatch pick-point session armed → end it (@18908)
2. Block Task Recorder waiting-to-name / empty basket → finish (@18926)
3. Draw tool at first-point prompt → continue from last point (@18937,
   `feed_first_point_from_last`)
4. Insert waiting-for-angle → 0° place (@18942)
5. ZOOM flow → advance/exit (@18952)
6. PEDIT menu → exit (Width step → back to menu) (@18970)
7. PLINE width/arc sub-flow → accept default width / cancel arc (@18983)
8. Select session, empty basket, non-cutter/boundary → **2-stage cancel**
   (first Enter warns, second cancels) (@18999)
9. (further down) finish the in-progress LINE/POLYLINE/SPLINE
10. (further down) **repeat last command** (@19134): `if let Some(last) =
    self.last_command.clone() { self.run_command(&last) }`

If you add a new modal flow, you almost always need to add **one arm here** for
its empty-Enter behavior, plus an Esc arm (section 12).

---

## 7. Repeat-last-command

`self.last_command: Option<String>` (@760). Set **only on a successful parse of
non-empty text** (@3895). Reasons it's gated that way (@3890): a typo like `1`
used to overwrite it and then empty-Enter would re-run the typo forever. Flows
that bypass the parser (circle/zoom/pedit) set `last_command` manually where
appropriate (e.g. pedit @2903, circle/zoom in their start methods). Empty Enter
at the idle prompt re-runs it (@19134).

---

## 8. Selection sessions (`SelectMode`, select-first pattern)

Many editing commands are **select-first**: typed with an empty selection they
enter a select session and stash what to do next in `QueuedOp` (@1406); on Enter
the queued op runs against the basket.

- `SelectMode` (@1375): `Off`, `ForList`, `ForSelect`, `ForCuttingEdges`
  (trim), `ForBoundaryEdges` (extend).
- `begin_selection(mode)` @4729 starts a session; finishing transfers the basket
  to the command and restores the user's prior selection from
  `pre_op_selection`.
- **Session letter shortcuts** (case-insensitive, rewritten @3868 before the
  parser): `W`=window, `C`/`CR`=crossing, `A`=all, `B`/`BEF`=before,
  `L`=last-drawn, `N`=none; plus `p`/`l`/`d` intercepted @2929 →
  previous/last/remove. (Memo: `feedback_rust_cad_selection_shortcuts`.)
- **Trim/Extend** are two-basket: `ForCuttingEdges`→targets,
  `ForBoundaryEdges`→targets. Empty-Enter at the cutters/boundary step means
  "use ALL dobjects" (so it is *excluded* from the 2-stage-cancel rule, @19005).
- The click-time selection model (replace / Shift-add / Alt-remove, drag =
  crossing/window) is in `click_select` / `add_window_selection`; see
  `feedback_rust_cad_pointer_is_selector` and the unified classifier memo.

---

## 9. Two sub-flow styles (old vs new) — know the difference

There are **two patterns** for multi-step commands, because the CLI is
mid-migration toward the `COMMAND_LINE.md` design.

### (a) NEW: prompt-driven `CmdFlow` — currently only CIRCLE
- `CmdFlow { name, circle: CircleStep }` (@1764). `CircleStep` (@1774) is the
  state machine: `Center → Radius/Diameter`, plus `3P`, `2P`, `Ttr` branches.
- `circle_flow_start()` @13437 sets `cmd_flow = Some(...)`.
- While `cmd_flow.is_some()`, `run_command` routes typed text to
  `flow_input_text()` @13666, and canvas clicks route via `flow_wants_point()`
  @13531 / the flow point handler @13546. The prompt string comes from a flow
  prompt fn @13506. This is the template to copy for future commands.

### (b) OLD: dedicated state enum + intercept — ZOOM, PEDIT, PLINE, etc.
- Each has its own `*State` enum (`ZoomState` @1800, `PeditState` @1489) or tool
  sub-state (`PlineArcSub`, `PlineWidthCap`), its own `*_input` router reached
  via an intercept in `run_command`, its own empty-Enter arm in the cascade, and
  its own Esc arm. More boilerplate, scattered across the file.

**When adding a new multi-step command, prefer style (a).**

---

## 10. Drawing tools (`Tool`, clicks, sub-keys)

- `Tool` enum (@42): `None` (the always-on selector), `Line`, `Circle`, `Arc`,
  `Ellipse`, `EllipseArc`, `Point`, `Polyline`, `Spline`, `Wall`, `Text`, `Dim`,
  `Rectangle`.
- Bare keyword (or toolbar button) → `Command::SetTool` → `self.tool = …`.
- Canvas clicks accumulate into `self.pending` (Vec of points); the tool builds
  a phantom/preview each frame and **commits on Enter** (or auto-closes, etc.).
- **PLINE sub-keys while drawing** (typed at the line, routed before the parser
  via the tool's own handling): `A` arc / `L` line / `C` close / `U` undo vertex
  / `S` 3-pt arc second point / `D` arc start direction / `W`/`H`
  width/halfwidth. Prompt text via `update_pline_prompt()` @15840.
- **RECTANGLE**: after the first corner, accepts a typed `width height` (signed)
  or a second click; commits as a closed 4-vertex polyline.

---

## 11. Inline snap overrides

Typing `END`/`MID`/`CEN`/`INT`/`PER`/`TAN`/`NEA`/`QUA` (single token) during any
active command parses to `Command::SnapOverride(kind)` and **arms a one-shot
snap** for the very next click — it supersedes the persistent snap setting for
that one pick. (Memo: `feedback_rust_cad_inline_snap_override_supersedes`.)

---

## 12. Prompts, history, transcript, Esc

- **`self.history`** is the scrolling "log book". `run_command` echoes each input
  (`command: …` at idle, `> …` mid-prompt) and the canonical command name.
- **`self.current_prompt`** is the active one-line prompt (e.g. "Specify radius:").
  `set_prompt()` @2835 sets it; `clear_prompt()` clears it. The active prompt is
  drawn in green (`#184C04`).
- **Transcript** (the new flow system) records prompt+reply pairs for replay.
- **Esc** is the symmetric "cancel one thing" cascade (search `Key::Escape` in
  `update()`): cancels the active tool / flow / select session, clears the
  selection, and refocuses the command line. Like Enter, it is *one cancel per
  press*.

---

## 13. How to add a new command (checklist)

**Stateless command** (e.g. a new one-shot op):
1. Add a variant to `Command` (`parser.rs` @26) + `canonical_name()` (@212).
2. Add a `match head` arm with aliases (@311).
3. Add a dispatch arm in `run_command`'s big `match` (`app.rs` @3912+).
4. Add a unit test in `parser.rs` if it parses args.

**Stateful / multi-step command** — prefer the NEW flow style (§9a):
1. Add a step enum (like `CircleStep`) and store it in `CmdFlow` (or a new flow
   struct).
2. Add a `*_flow_start()` and a `*_input_text()`; route to it from a `run_command`
   intercept (§5) guarded by `cmd_flow`/your state.
3. Add canvas-point routing via `flow_wants_point()` / the point handler.
4. Add an empty-Enter arm (§6) and an Esc arm (§12).
5. Set `self.last_command` in the start method so empty-Enter repeats it.

---

## 14. Invariants & gotchas (don't regress these)

- **Single-letter ambiguity is resolved by the app, not the parser.** `c` =
  Copy globally, but Crossing while selecting (rewrite @3868) and Close while
  drawing a pline (tool handler). Never push context into the parser.
- **`last_command` only on successful, non-empty parse** (@3895) — or empty-Enter
  re-runs typos.
- **Space = Enter only on an empty line and not in a text body** (@18895) — or it
  breaks typing strings/heights.
- **One Enter (and one Esc) = one transition per frame.** The empty-Enter cascade
  is mutually exclusive; do not let a single press chain phases
  (`feedback_rust_cad_user_terminates_sessions`).
- **Empty input never reaches `run_command`** — handle "done/advance/repeat" in
  the `update()` cascade, not in `run_command`.
- **Intercept order in `run_command` matters** — a live flow must consume the
  line before any word-matching can re-interpret it.
- **Trim/Extend cutter/boundary step** is exempt from the 2-stage select cancel
  (empty Enter there = "use ALL").

---

## 15. The redesign (context only)

`COMMAND_LINE.md` describes a planned refactor: a single Command IR, a pure core
in the kernel, a `command_line.rs` module, and a `CmdHost` trait so an AI can
drive the CLI deterministically. **It is not started.** The `CmdFlow`/CircleStep
work (§9a) is the first concrete step in that direction; everything else still
uses the old per-command intercept+state pattern. Build new flows the §9a way to
keep moving toward the redesign rather than adding more §9b boilerplate.
