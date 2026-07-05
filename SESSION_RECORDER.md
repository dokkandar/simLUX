# RUST_CAD — Session Recorder (how it's built, the UI, and how to port it)

> Handoff doc for another coding agent. Explains the **Session Recorder** — the
> high-fidelity debug/spec capture tool — exactly as built in this repo, how it
> appears in the UI, and a step-by-step recipe to reproduce it in another repo.
>
> Code: `cad_app/src/dbg_recorder.rs` (self-contained, ~737 lines) + ~10 wiring
> points in `cad_app/src/app.rs`. Line numbers are from commit `b952aaf`
> (2026-06-24); grep the symbol if a number drifts.

---

## 1. What it is and why it exists

A **session recorder** is an opt-in tap that, while armed, captures EVERY user
action, state-machine transition, document mutation, and snapshot during a
short (few-second) session, then exports a timestamped, source-stamped text
timeline you can paste into a chat or read by eye.

It serves **two** jobs:

1. **Bug repro / diagnosis.** For interaction bugs (clicked the wrong thing, a
   select didn't take, a gesture got mis-classified) the dump shows the *exact*
   input sequence and the *exact* outcome at each step — including per-click
   hit-tests and every state transition. This is the **primary debug tool** for
   interaction bugs in this project: read the dump FIRST, before hypothesizing.
2. **Algorithm extraction (programming-by-demonstration).** Record yourself
   performing the intended workflow (e.g. stretching a block into a variant);
   the resulting event list — especially `StretchRecord` + `GeometryCapture` —
   is a literal spec for building a new parametric command.

**Design north star:** the output should let a reader who wasn't there
reconstruct what happened with zero ambiguity, and it must cost ~nothing when
not recording (so the `dbg_event!` calls can live permanently in hot paths).

---

## 2. Architecture at a glance

```
        ┌─────────────────────── cad_app/src/dbg_recorder.rs ───────────────────────┐
        │  DbgEvent (enum)      — one chunky, readable variant per kind of event      │
        │  DbgRecord            — { elapsed_ms, event, location: &'static Location }   │
        │  DbgSnapshot          — { event_index, tag, doc: Document (full clone) }     │
        │  DbgRecorder          — events: Vec<DbgRecord>, snapshots: Vec<DbgSnapshot>  │
        │  WatchedState         — flat snapshot of every state field we poll           │
        │  diff_watched()       — emit one event per changed field (frame-end poller)  │
        │  dump_text()          — render the whole session to a clipboard-ready string │
        │  dbg_event! / dbg_snapshot! (macros, #[track_caller] location capture)       │
        └────────────────────────────────────────────────────────────────────────────┘
                                   ▲ owned by CadApp as `self.dbg: DbgRecorder`
                                   │
   THREE capture mechanisms feed it:
   (a) explicit   dbg_event!(self, DbgEvent::X{..})   at ~28 hot sites
   (b) diff poll  dbg_poll_state()  every frame-end   → auto StateChange/ToolChange/…
   (c) snapshots  dbg_snapshot!(self,"reason")        at start / manual / auto / pre-undo
```

The recorder is **off by default** (`recording: false`). Every `push`/`take_…`
early-returns when not recording, so the instrumentation is free in production.

---

## 3. The three capture mechanisms (the important design)

### (a) Explicit events — `dbg_event!`
At each interesting code site we drop one call:

```rust
crate::dbg_event!(self, crate::dbg_recorder::DbgEvent::CmdRun {
    raw:          raw.to_string(),
    parsed_debug: format!("{:?}", parse(trimmed)),
    source:       CmdSource::Typed,
});
```

The macro (defined at `dbg_recorder.rs:714`) is:

```rust
#[macro_export]
macro_rules! dbg_event {
    ($app:expr, $event:expr) => {{
        if $app.dbg.recording {
            $app.dbg.push($event, std::panic::Location::caller());
        }
    }};
}
```

`std::panic::Location::caller()` captures the **file:line of the call site** for
free (no `#[track_caller]` annotation needed on the macro user). That's why
every line in the dump is stamped `app.rs:2861`. There are ~28 such sites
(CmdRun, CanvasClick, CanvasPress/Release, GestureClassification, DocPush,
ApplyOp, SelectChange, UndoFired, StretchRecord, GeometryCapture, Note, …).

### (b) Frame-end diff polling — the elegant part
We do **not** instrument every `self.foo_state = …` assignment. Instead we keep
a flat `WatchedState` struct (`dbg_recorder.rs:514`) holding a Debug-printed
copy of every state field we care about (tool, select_mode, all the
`*_state` machines, selection, queued_op, window-open flags, a SYSVAR summary,
undo/redo depths, …). Once per frame, at the very end of `update()`:

```rust
self.dbg_poll_state();        // app.rs:8312 / called at 20444
```
```rust
pub fn dbg_poll_state(&mut self) {
    if !self.dbg.recording { return; }
    let curr = self.dbg_watched_now();                 // build current snapshot
    if let Some(prev) = self.dbg_last_watched.take() {
        diff_watched(&mut self.dbg, &prev, &curr, Location::caller());  // emit per-field
    }
    self.dbg_last_watched = Some(curr);
}
```

`diff_watched` (`dbg_recorder.rs:613`) compares prev vs curr field-by-field and
pushes a `StateChange` (or a dedicated `ToolChange` / `WindowToggle`) for each
field that differs. **Result: every state transition shows up in the timeline
automatically, even transitions we never explicitly instrumented.** This is what
makes the recorder behave like an "agent inspector". Cost is a few short string
formats per frame while recording.

> Known blind spot: states that exist only *within* a single frame's
> pick-handling (e.g. transient pick-phase sub-states) never differ across two
> frame-end polls, so they aren't captured by (b). Cover those with an explicit
> `dbg_event!` (mechanism a) if needed.

### (c) Document snapshots — `dbg_snapshot!`
Full `Document` clones into a side-buffer (`snapshots`), each tagged + linked to
the event index where it was taken:

```rust
dbg_snapshot!(self, "pre-undo");     // macro forwards &self.doc + undo/redo depths + caller
```

Taken at: **session start** (anchor), **manual** ("📷 Snap" button), **auto
cadence** (every `auto_snap_every` events, default 50; `dbg_maybe_auto_snap()`
at frame-end), and **before destructive ops** (`snapshot_doc`). `Document: Clone`
makes this a single ~100 KB memcpy — acceptable for short sessions.

---

## 4. The event model

`DbgEvent` (`dbg_recorder.rs:41`) variants are **deliberately chunky** — we want
readable events, not raw keystrokes. The high-value ones:

| Variant | Captures | Why it matters |
|---|---|---|
| `CmdRun{raw,parsed_debug,source}` | command-line input + parse result + origin | every command, typed vs menu vs replay |
| `CanvasClick{world,screen,modifiers,hit_dobject,active_tool,active_state}` | a fully-decoded click | *what was under the cursor and what mode we were in* |
| `GestureClassification{…}` | press→release decoded: motion px/dir, egui clicked vs drag_stopped, hit-at-press/release, selection before/after, **app_action_taken**, **outcome_summary** | the single most useful event for click/drag bugs — "dragged 263px R→L, demoted to click, NOOP" |
| `SelectChange{before,after,cause}` | selection basket delta | catches mis-selects |
| `ToolChange` / `StateChange{state_name,before,after,cause}` | any state-machine transition | mostly auto-emitted by the poller |
| `DocPush` / `DocRemove` | dobject added/removed (index, kind, handle, summary) | doc mutations |
| `UndoSnapshotTaken` / `UndoFired` / `RedoFired` | undo-stack movement | |
| `ApplyOp{name,before_count,after_count,success,detail}` | one `apply_*` op ran | generic op envelope |
| `DocSnapshot{…,index_in_dump}` | references a heavy snapshot in the side-buffer | |
| `StretchRecord{box,base,dest,vector,affected[]}` | a demonstrated stretch with full before→after coords | parametric authoring |
| `GeometryCapture{label,entries[]}` | full coords of selected dobjects (or a block's definition) | the BASE geometry a parametric rule transforms |
| `Note{message}` | manual annotation | "bug fired here" |
| `WindowToggle` / `MenuClick` / `KeyEvent` | UI events | |

`CmdSource` (`:210`) distinguishes `Typed / Menu(&str) / Replay / Internal(&str)`.

Each stored row is a `DbgRecord { elapsed_ms, event, location }` (`:230`).

---

## 5. The recorder struct & lifecycle

`DbgRecorder` (`dbg_recorder.rs:247`):
```rust
pub struct DbgRecorder {
    pub recording: bool,                 // off by default
    pub session_started: Option<Instant>,
    pub events: Vec<DbgRecord>,
    pub snapshots: Vec<DbgSnapshot>,
    pub auto_snap_every: usize,          // default 50; 0 disables
    pub max_events: usize,               // default 100_000; ring-evicts oldest 5% when full
    pub capture_backtrace: bool,         // off (very slow)
}
```
Methods: `start(reason)` (clears + stamps SessionStart), `stop(reason)` (stamps
SessionStop + count), `clear()`, `push(event, loc)` (the hot path; no-op when
off; ring-evicts at cap), `take_snapshot(doc, reason, undo_d, redo_d, loc)`,
`want_auto_snap()`, and `dump_text()`.

---

## 6. Output format (`dump_text`)

One line per event, time- and source-stamped, emoji-prefixed for skimming:

```
=== SESSION DUMP (42 events, 3 snapshots) ===
[   0.0 ms] #00000 @ app.rs:8146  — ◆ SESSION START — user pressed Start
[  12.4 ms] #00001 @ app.rs:2861  — ⌨ CMD "circle" → Ok(SetTool(Circle))  [Typed]
[  98.1 ms] #00002 @ app.rs:1xxxx — 🖱 CLICK world=(3.250,4.100)  hit=Some(2)  tool=Circle  state=…
[ 110.7 ms] #00003 @ app.rs:8317  — 🔧 TOOL Circle → None  (state poll (frame end))
[ 250.0 ms] #00010 @ app.rs:xxxxx — 🔍 GESTURE press=(..) hit_press=Some(2)  →  release=(..) hit_release=Some(2)
                                              motion=3 px (stationary)  egui: clicked=true drag_stopped=false  select_mode=true …
                                              sel: [] → [2]  | action: click_select(i=2)
                                              verdict: replaced selection with dobject #2
=== END SESSION (3 snapshots in side-buffer) ===
```

The per-line renderer is `format_event_oneline` (`:402`); multi-line blocks
(gesture / stretch / geometry-capture) indent continuation lines so the timeline
stays readable. `dump_text` is what the "📋 Copy timeline" button copies.

---

## 7. The UI window

`render_dbg_recorder_window` (`app.rs:8518`) is an `egui::Window` titled
**"🛰 Session Recorder"**, shown only when `self.dbg_window_open`. Opened via the
`DbgRecorder` command (`dbg` / `recorder`, parser arm) — dispatch toggles
`dbg_window_open` (`app.rs:4291`) — or the Tools menu.

Layout, top to bottom:
1. **▶ Start / ■ Stop** (green/red, mutually enabled by `recording`) →
   `dbg_start()` / `dbg_stop()`. **🗑 Clear**. **📷 Snap** (manual snapshot).
2. **Status line:** `🔴 RECORDING / ⚪ idle · N events · M snapshots`.
3. **Smart-dobject authoring:** a blue **📐 Capture smart dobject** button →
   `capture_smart_geometry()` (dumps full coords of the selection — exploded set
   or a block's definition) + a "N selected" label + a one-line how-to
   ("Start ▶ · select · 📐 Capture · do your stretches · Stop ■ · 📋 Copy").
4. **Annotate:** a text field + **Drop note** (also on Enter) → `Note` event.
5. **📋 Copy timeline** → `ctx.copy_text(self.dbg.dump_text())`.
6. **Capture backtrace (slow)** checkbox; **Auto-snap every: N events** DragValue.

> The same pattern is reused for two narrower logs — **Trim Debug Log**
> (`render_trim_debug_window` :8638) and **Hatch Debug** (:8701) — each a
> scrollable monospace log with Copy/Clear. They predate the general recorder;
> the general recorder is the one to port.

**UX principles to preserve when porting:** off by default; one-click
Start/Stop; a live event/snapshot counter so the user knows it's working; a
big obvious Copy button (the whole point is paste-into-chat); inline notes so the
user can mark "the bug is here"; and a manual snapshot for "capture state right
now".

---

## 8. Integration points in `CadApp`

Fields (on the app struct):
```rust
dbg:              DbgRecorder,                  // the recorder
dbg_window_open:  bool,                         // window visibility (:1286)
dbg_last_watched: Option<WatchedState>,         // prev-frame poll snapshot
dbg_note_buf:     String,                       // note text field
```
Methods: `dbg_start` (:8140), `dbg_stop` (:8153), `dbg_watched_now` (:8162,
builds the WatchedState), `dbg_poll_state` (:8312), `dbg_maybe_auto_snap`
(:8326), `capture_smart_geometry` (:14727).

Frame-end wiring inside `update()` (order matters — poll AFTER all input/state
mutation for the frame):
```rust
self.render_dbg_recorder_window(ctx);   // :20435
…
self.dbg_poll_state();                  // :20444  (diff vs last frame)
self.dbg_maybe_auto_snap();             // :20447  (cadence snapshot)
```
Plus the ~28 `dbg_event!` sites and the `dbg_snapshot!` calls in `snapshot_doc`
/ pre-undo.

---

## 9. Recipe: reproduce this in another repo

Assuming an egui/eframe-style app with a central app struct and a per-frame
`update()`. Steps:

1. **Drop in the module.** Copy `dbg_recorder.rs`. Strip it to your needs but
   keep: `DbgEvent` (start with a handful of variants), `DbgRecord`,
   `DbgRecorder` (recording/events/snapshots/auto_snap_every/max_events),
   `dump_text`, `format_event_oneline`, and the `dbg_event!`/`dbg_snapshot!`
   macros. Replace `Document` with your doc/scene type (must be `Clone`).

2. **Own a recorder.** Add `dbg: DbgRecorder`, `dbg_window_open: bool`,
   `dbg_last_watched: Option<WatchedState>`, `dbg_note_buf: String` to your app
   struct; default `dbg = DbgRecorder::default()` (recording off).

3. **The macro is the contract.** Keep `dbg_event!($app, $event)` no-op-when-off
   + `Location::caller()`. This is what makes leaving calls in cheap and gives
   you file:line stamps for free.

4. **Explicit events at hot sites.** Add `dbg_event!` at: command/input entry,
   canvas click/press/release (decode the hit-test + active mode INTO the
   event — that decoding is what makes the dump useful), each `apply_*`/mutation,
   and undo/redo. Make the variants chunky and self-explanatory.

5. **Diff polling (do this — it's the multiplier).** Define a flat
   `WatchedState` of Debug-printed copies of every state field + open-window
   flags. Add `watched_now()` to build it, and a `poll_state()` that diffs
   prev vs curr and pushes one event per change. Call it **once at the end of
   `update()`**. Now you get transitions for free.

6. **Snapshots.** Call `dbg_snapshot!(self,"reason")` at session start, before
   destructive ops, and on an N-event cadence (`want_auto_snap` +
   `maybe_auto_snap` at frame-end). Keep them in a side-buffer keyed by event
   index.

7. **The window.** Build the egui window from §7: Start/Stop/Clear/Snap, a live
   counter, a notes field, and — most importantly — a **Copy** button that does
   `ctx.copy_text(self.dbg.dump_text())`. Toggle it from a command/menu.

8. **Wire the three frame-end calls** (`render_window`, `poll_state`,
   `maybe_auto_snap`) and you're done.

---

## 10. Design principles (preserve these — they're why it works)

- **Off by default, free when off.** Every push early-returns on `!recording`;
  instrumentation lives permanently in hot paths with no production cost.
- **`Location::caller()` everywhere.** Every event is source-stamped for free.
- **Decode at capture time.** A click event stores the hit-test result, active
  tool, and mode — not just coordinates. A reader shouldn't need the code to
  understand the event.
- **Diff-poll state, don't instrument every assignment.** One `WatchedState` +
  one frame-end diff captures all transitions and stays maintainable.
- **Chunky, readable, emoji-tagged events.** Optimized for paste-and-read, not
  machine parsing. (If you later want replay, add a JSON serializer alongside.)
- **Heavy snapshots in a side-buffer**, referenced by event index, taken
  sparingly (start / manual / cadence / pre-undo). The recorder does NOT own the
  document — the app passes clones, so there's no borrow tangle.
- **Two products from one tool:** a bug-repro timeline AND a
  programming-by-demonstration spec (the `StretchRecord` + `GeometryCapture`
  pair). Design event variants rich enough to serve the second use.

---

## 11. Gotchas

- Recording must be **explicitly started** — a blank dump means nobody pressed ▶.
- Snapshots are full clones; with a huge document, lower `auto_snap_every` or
  disable cadence (0) and snap manually.
- `std::time::Instant` is used for timestamps — fine in a desktop app; if you
  port into a headless/deterministic context, swap the clock.
- The frame-end poller misses **intra-frame** transient states (see §3b) — use
  explicit `dbg_event!` for those.
- `max_events` ring-evicts the oldest 5% when full — a very long session loses
  its head; bump the cap or stop/copy sooner for long captures.
