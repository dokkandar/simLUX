# Command Line — architecture spec

The command line is a **first-class subsystem**, not an afterthought. It must be
flexible enough to later route input through an AI / external resolver while
keeping typed CAD commands instant and deterministic. This spec defines the
wiring agreed with the mentor review.

> Status: **design, not yet implemented.** Current code: `parse()` + `Command`
> in `cad_kernel/src/parser.rs` (58 variants); dispatch + ~14 modal intercepts
> inline in `CadApp::run_command` (`cad_app/src/app.rs`, ~1,650 lines); context
> implicit across ~18 `*_state` fields. This spec replaces that with a layered,
> pure, testable, AI-pluggable design.

## Principles

1. **Deterministic-first, AI on-demand.** `parse()` is synchronous and free;
   typed commands (`l`, `c`, `100⏎`) must never wait on an LLM. The AI resolver
   fires only on a **parse miss** or an **explicit NL trigger** (leading space /
   `?` prefix / toggled "ask" mode).
2. **One intent IR.** Every input — including modal answers — becomes a
   `Command`. The module emits intent; the app executes it. No inline mutation
   in the parse/resolve path.
3. **Pure core, thin boundary.** Text→intent is pure and unit-testable.
   Execution stays in the app behind a trait.
4. **Async never blocks.** The AI resolver runs as a background op; the UI shows
   "thinking… (Esc cancels)" and stays responsive.

## Layers

```
cad_kernel (pure):   Command · parse · InputContext · Resolution · resolve()   ← no app, no egui, fully testable
        │ Command (intent IR)
        ▼
cad_app/command_line.rs:  buffer · prompt/feedback · parse→resolve · async AI hook   ← standalone MODULE
        │ trait CmdHost: apply(Command), scene_snapshot()
        ▼
CadApp:  apply(Command) mutates doc/tool/state                                  ← stays in the app
```

"Standalone" = this wiring, not headless execution. Generating intent is fully
decoupled; *executing* it is the app's job by definition.

## The pure core (in `cad_kernel`)

### `InputContext` — what the line expects right now
Replaces "infer from 18 scattered `*_state` fields." Small, cheap, drives
parsing + handler selection:

```rust
enum InputContext {
    TopLevel,
    AwaitingPoint   { op: OpId },
    AwaitingValue   { name: String, default: f64 },   // insert param, etc.
    AwaitingAngle   { /* insert rotation */ },
    AwaitingYesNo   { question: String },             // mirror keep-original
    AwaitingDistance{ /* DDE anchor known to app */ },
    SubOption       { set: &'static [&'static str] }, // pline/fillet/chamfer/offset/rotate/scale
    Resolving       { /* AI in flight; Esc cancels */ },
}
```

### `Resolution` + `resolve()` — the contract
```rust
enum Resolution {
    Command(Command),    // execute this intent
    Clarify(String),     // ask the user (updates the prompt line)
    Reject(String),      // show error, keep input, don't break flow
    Defer,               // not mine → fall through to parse()
}

fn resolve(ctx: &InputContext, raw: &str) -> Resolution
```

- **Pure**: reads `ctx` + text only. Never mutates state.
- **`match InputContext`, not `Vec<Box<dyn>>`** — 14 arms in a readable match
  beats a dynamic registry; same flexibility, zero indirection, trivially
  testable. One context = one arm. No chaining; `Defer` = fall to `parse()`.

## Command IR completeness (the real work — Slice 2)

Today's intercepts mutate state inline. To make the core pure, each modal answer
becomes a `Command` variant; `apply()` realizes it against current state:

| Today (inline mutation)            | Becomes (intent)                  |
|------------------------------------|-----------------------------------|
| mirror keep-original Y/n           | `MirrorAnswer(bool)`              |
| DDE typed distance                 | `DistanceEntry(f64)`             |
| insert param value / Enter=default | `ParamValue(f64)` / `AcceptDefault` |
| insert rotation angle              | `InsertAngle(f64)`              |
| pline Arc/Line/Close sub-options   | `PlineMode(..)` / `PlineClose`  |
| fillet/chamfer set radius/dist     | `SetRadius(f64)` / `SetChamfer(..)` |
| offset/rotate/scale sub-args       | dedicated variants               |

`Command` grows ~58 → ~75. A `Command` is an **intent** ("apply distance 100");
`apply()` uses current state (anchor, active op) to realize it. That's what keeps
the core pure and the app the sole executor.

## The boundary trait (in `cad_app`)

```rust
trait CmdHost {
    fn apply(&mut self, cmd: Command);          // execute intent → mutate state
    fn scene_snapshot(&self) -> SceneSnapshot;  // read-only, for the AI resolver
}
```

## Feedback model

The module owns the feedback line; the renderer just paints it:
```rust
struct CmdFeedback { prompt: String, error: Option<String> }
```
- `Clarify(s)` → `prompt = s` (e.g. AI: "which wall?").
- `Reject(s)` → `error = Some(s)`, rendered **red in the command line,
  non-destructive** (input preserved, flow unbroken).
- `current_prompt` moves out of `CadApp` into the module.

## Context for the AI (no hot-path coupling)

Two separate contexts:
- `InputContext` — cheap, "what the line expects," per-keystroke.
- `SceneSnapshot` — assembled **only when the resolver fires**: selection IDs,
  active layer, current tool, view bbox, last N commands. Passed **read-only**
  to the AI so "make it thicker" / "copy that thing over there" resolve against
  real state — zero cost until AI is invoked.

## Async resolver (reuse the Background Ops Pattern)

Per `Background_Ops_Pattern.md` (PURE on worker · APPLY on main ·
cancel-drops-result):
1. Parse miss / NL trigger → spawn the resolver on a worker with
   `(raw, InputContext, SceneSnapshot)`.
2. `InputContext::Resolving` → prompt shows "thinking… (Esc cancels)".
3. Worker returns a `Resolution`; **applied on the main thread**.
4. Esc cancels → result dropped, prior context restored.

The AI resolver is just another background op — the project already has the idiom.

## Testing (purity is the payoff)

- `resolve(ctx, input) -> Resolution` is pure → **table-test every modal answer**.
- Whole pipeline is text→`Command` → **golden-stream regression**: replay input
  sequences, assert the emitted `Command` stream. Seed the corpus from the
  **Session Recorder** (real sessions). Pin all 14 modal interactions *before*
  migration, so each handler move is provably behavior-preserving.

## Slice plan

1. **IR + seam, no behavior change.** Add `InputContext`, `Resolution`,
   `resolve()` (resolver = current `parse()`) in the kernel + golden tests. AI
   attach point exists; nothing else moves.
2. **Purify, one context at a time.** Each intercept → a `Command` variant + a
   `match InputContext` arm, guarded by a golden test. (Side-effects → commands.)
3. **Lift to `cad_app/src/command_line.rs`.** Module owns buffer, context,
   feedback, parse+resolve calls; app implements `CmdHost`.
4. **Async AI resolver** behind the `resolve()` seam, via the Background Ops
   Pattern + `Resolving` state. (When wanted.)

The honest cost is **Slice 2** (completing the IR) — but it's what turns the
command line from a 1,650-line cascade into a pure, testable, AI-pluggable
subsystem.

---

# Inventory — what we have vs. what's awaiting

Everything the command line touches, in two buckets: **✔ built** (working today,
mostly as the inline cascade this spec refactors) and **◻ awaiting** (not built —
the backlog). Nothing is dropped; un-built infrastructure lives in the Awaiting
list, not in someone's head.

## ✔ What we have today (existing infrastructure)

**Parse + dispatch**
- ✔ `parse(line) -> Result<Command, String>` — synchronous, pure, in
  `cad_kernel/src/parser.rs` (~58 `Command` variants).
- ✔ Inline dispatch in `CadApp::run_command` (`cad_app/src/app.rs`, ~1,650 lines).
- ✔ Single-letter **aliases** resolved to a canonical command in `parse()`
  (hardcoded; `F`→Fillet, `l`→line, `co`→copy, …) + an "aliased" echo hint.

**Command vocabulary (the 58 variants), by family**
- ✔ Draw: line, circle, arc, ellipse, point, polyline, polygon, spline, rect, text.
- ✔ Modify: move, copy, rotate, scale, mirror, offset, fillet, chamfer, trim,
  extend, stretch, lengthen, break, align, join, pedit, array, hatch.
- ✔ Boolean: union, difference, intersection, xor.
- ✔ Inquiry: dist, list.
- ✔ Properties: chprop, chlayer, matchprops, linetype.
- ✔ Structure / blocks: wall, wallstyle, block, insert, explode, blockdiff, btr
  (block-task recorder), finish.
- ✔ Annotation: dim, dimstyle, text, textstyle.
- ✔ View / edit / file: zoom, grips, undo, redo, delete, open, saveas.
- ✔ Modes / misc: card, ucs, recorder/dbg, help, clear, snap overrides.

**Modal state machines (the "context implicit across *_state fields")** — ~24:
- ✔ MoveState, CopyState, RotateState, ScaleState, MirrorState, OffsetState,
  FilletState, ChamferState, TrimState, ExtendState, StretchState, LengthenState,
  BreakState, AlignState, PeditState, MatchPropsState, BlockDefState, InsertState,
  PasteState, DistState, ZoomState, PreviewState, TextDraftState, DimDraftState.
- ✔ Each is the live, inline version of an `InputContext` arm this spec will purify.

**Selection subsystem** (a full modal interaction — but NOT yet in the pure model):
- ✔ `SelectMode { Off, ForList, ForSelect, ForCuttingEdges, ForBoundaryEdges }`
  + `QueuedOp` + `begin_selection` / `finalise_selection`.
- ✔ Pointer = always-on selector: click = replace, Shift = add, Alt = remove.
- ✔ Select sub-commands typed in the line during a session: **W/C/A/B/L/N**
  (window / crossing / all / before / last / none), intercepted before `parse()`.
- ✔ Click/drag **classifier**: `SelDmTm` time-gate; L→R = inside window,
  R→L = crossing; window vs single-pick.
- ✔ 2-stage empty-Enter cancel; "Enter = use ALL" for cutter/boundary modes.

**Input niceties**
- ✔ **DDE** — typed distance along the cursor direction while awaiting a point.
- ✔ **Snap overrides** — `SnapOverride(kind)`; typed END/MID/CEN/NEA/INT/PER/…
- ✔ **Space = Enter** on an *empty* line (advance / repeat-last); history flows
  upward; Enter on empty repeats `last_command`; `current_prompt` line.
- ✔ Coordinate entry — **absolute `x,y` only** (`parse_line` / `parse_point`).

## ◻ Awaiting list (infrastructure not yet built)

Ordered by what blocks the refactor first. `◻` = not built.

### Critical — needed before Slice 2 can be behavior-preserving
- ◻ **`AwaitingSelection` context + selection sub-command resolution.** The
  select-objects loop (W/C/A/B/L/N, Enter-to-finish, accumulate, 2-stage cancel)
  is the most-used modal interaction yet has **no `InputContext` arm** and is
  absent from the Command-IR table. Add `AwaitingSelection { for_op: OpId }` and
  make W/C/A/B/L/N resolved keywords.
- ◻ **Coordinate-input grammar.** Relative `@dx,dy`, polar `@dist<angle`, and
  `@` = last point. Today only absolute `x,y` parses. Belongs in `parse()` /
  `AwaitingPoint`; couples to snap + last-point state.
- ◻ **Context-transition FSM.** The spec lists `InputContext` *states* but not
  the begin → advance → finalise machinery that moves between them. Define who
  owns the transitions (the analogue of `begin_selection`/`finalise_selection`).
- ◻ **Keyword / option grammar.** `[Arc/Close/Undo]` bracket presentation,
  single capital-letter selection, `<default>` rendering — the contract `parse()`
  needs to map a letter to a `SubOption`.
- ◻ **Key / cancel semantics, written down.** Enter = submit; **Space = Enter**;
  Enter-on-empty = repeat-last; Esc = cancel ladder (current op → TopLevel);
  right-click = Enter (AutoCAD). Currently only AI-cancel (Esc) is specified.

### Important — real CAD command-line behavior we lack entirely
- ◻ **Transparent commands** — `'zoom` / `'pan` / mid-command osnap toggle:
  suspend the active op, run, resume. Distinct from the async `Resolving` state.
- ◻ **History recall** — Up/Down arrow through prior commands; persisted history
  across sessions.
- ◻ **Undo grouping** — one command = one undo transaction; a multi-phase op
  (select → base → dest) collapses to a single undo step.
- ◻ **User-editable aliases** — a PGP-style alias file at project root + in-app
  editor (see the alias/scripting roadmap); today aliases are hardcoded in `parse()`.

### Later — productivity / power features
- ◻ **Autocomplete / Tab completion** + as-you-type command suggestions.
- ◻ **Scripting / macro execution** — replay a `Command` stream from a file
  (the IR + golden-stream corpus make this nearly free; ties to the scripting roadmap).
- ◻ **Dynamic input** — cursor-anchored tooltips for coordinate / distance / angle entry.
- ◻ **Function-key / accelerator → command** bindings.

### The pure refactor itself (this spec — design-only today)
- ◻ Slice 1: `InputContext`, `Resolution`, `resolve()`, `CmdHost` seam + golden tests.
- ◻ Slice 2: purify each intercept → `Command` variant + `match` arm (incl. the
  Critical items above, which expand the IR beyond the ~58→~75 estimate).
- ◻ Slice 3: lift buffer/context/feedback into `cad_app/src/command_line.rs`.
- ◻ Slice 4 (AI): async resolver, `SceneSnapshot` assembly, `Resolving` state,
  NL trigger, **AI-disabled parse-miss behavior**, and a `Reject` **error
  taxonomy** (unknown-command vs bad-arg vs out-of-context) for golden tests.

> Cross-refs: `Background_Ops_Pattern.md` (async resolver), the Session Recorder
> (golden corpus), the alias + scripting roadmaps, and `Map_HSI_LibreCAD.md`
> ("What's STILL OWED"). A behavioral subset of the deterministic path (selection
> session, Move/Copy click-sessions, the click/drag classifier, Space=Enter) has
> also been ported to the Farzad fork (`~/workspace/AUTO_RASM_FARZAD`,
> `docs/INTERACTION_PORT.md`) — useful as a reference implementation for several
> Critical items above.
