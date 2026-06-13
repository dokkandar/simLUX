//! Session recorder — high-fidelity event tap for short debug sessions.
//!
//! Captures EVERY user action, state transition, doc mutation, and
//! memory incident during a few-second recording. Output is structured
//! `DbgEvent` records with: timestamp, kind-specific payload, and the
//! source-code `Location` of the caller that fired the event (via
//! `#[track_caller]` + `Location::caller()`).
//!
//! Two use-cases:
//!   1. **Bug repro** — paste the JSON dump into a bug report; reader
//!      sees the EXACT input sequence + the EXACT outcome at each step.
//!   2. **Algorithm extraction** — record yourself performing the
//!      intended workflow; read the resulting event list as a spec for
//!      a new command.
//!
//! Design choices for fidelity:
//!   * Snapshots taken at session start, on demand, AND every N events
//!     (default 50). Document is `Clone`, so the cost is one large
//!     memcpy per snapshot — fine at ~100KB/snap.
//!   * No global-allocator override for memory tracking. Instead the
//!     "memory incident" payloads include sizes the kernel exposes:
//!     `Document.dobjects.len()`, undo-stack depth, spatial-index entry
//!     count. That's enough to spot the bugs we hit in practice.
//!   * Event push is hot (≤1 µs typical). Snapshot push is the cost
//!     spike (1–5 ms). Recording is OFF by default — never runs unless
//!     the user pressed Start in the Recorder window.

use cad_kernel::{Document, Vec2};
use std::panic::Location;
use std::time::Instant;

// ===========================================================================
//                              EVENT KINDS
// ===========================================================================

/// One captured event. Variants are deliberately chunky — a CmdRun
/// includes the parsed result and source, a CanvasClick includes the
/// hit-test outcome and drag-classification result, etc. We want
/// READABLE events, not raw key strokes.
#[derive(Clone, Debug)]
pub enum DbgEvent {
    /// Recorder armed / disarmed (matches Start/Stop button presses).
    SessionStart { reason: String },
    SessionStop  { reason: String, event_count: usize },

    /// Manual annotation — note dropped via the "📝 Note" button so the
    /// reader can mark "this is where the bug fired".
    Note { message: String },

    /// One full `Document` snapshot. Heavy. Tagged with the reason
    /// (auto-cadence / manual / pre-undo / post-snapshot_doc).
    DocSnapshot {
        reason:         String,
        dobject_count:  usize,
        undo_depth:     usize,
        redo_depth:     usize,
        layer_count:    usize,
        index_in_dump:  usize,  // index into DbgRecorder::snapshots
    },

    /// `run_command(raw)` fired. Records the RAW input + the parsed
    /// `Command` debug-print + where the call came from (typed in cmd
    /// line, menu button, replay).
    CmdRun {
        raw:           String,
        parsed_debug:  String,
        source:        CmdSource,
    },

    /// Canvas mouse click — fully decoded.
    CanvasClick {
        screen:        (f32, f32),
        world:         Vec2,
        modifiers:     KeyModifiers,
        hit_dobject:   Option<usize>,
        active_tool:   String,
        active_state:  String,    // one-line summary of every state machine
    },
    /// Mouse press / release / drag (less decoded — just positions).
    CanvasPress   { screen: (f32, f32), world: Vec2, button: String },
    CanvasRelease { screen: (f32, f32), world: Vec2, button: String, drag_px: f32 },

    /// FULL gesture decoder — fires AFTER every press-release pair on
    /// the canvas, regardless of whether it was classified as click or
    /// drag. Captures both egui's classification AND the app's outcome,
    /// so a glance at this event tells you "the user dragged 263 px
    /// R→L but it was demoted to a click and nothing was selected".
    GestureClassification {
        press_screen:        (f32, f32),
        release_screen:      (f32, f32),
        press_world:         Vec2,
        release_world:       Vec2,
        motion_px:           f32,
        motion_dir:          String,    // "L→R ↓ 263 px", "stationary", "vertical ↑"
        egui_clicked:        bool,
        egui_drag_stopped:   bool,
        hit_at_press:        Option<usize>,
        hit_at_release:      Option<usize>,
        in_select_mode:      bool,
        active_tool:         String,
        selection_before:    Vec<usize>,
        selection_after:     Vec<usize>,
        app_action_taken:    String,    // "click_select(i=2)", "window_first=Some(...)", "add_window_selection", "NOOP — gesture had no effect"
        outcome_summary:     String,    // human-readable verdict
    },
    CanvasDrag {
        from_screen: (f32, f32),
        to_screen:   (f32, f32),
        from_world:  Vec2,
        to_world:    Vec2,
        button:      String,
    },

    /// Selection mutated — what was the basket BEFORE vs AFTER.
    SelectChange {
        basket_before: Vec<usize>,
        basket_after:  Vec<usize>,
        cause:         String,    // "click_select(i=5, shift=false)" etc.
    },

    /// Tool changed. Tools mean "what's the user drafting / picking".
    ToolChange { from: String, to: String, cause: String },

    /// Generic state machine transition. `state_name` is the field name
    /// (`"trim_state"`, `"fillet_state"`, etc.). `before`/`after` are
    /// Debug-printed.
    StateChange { state_name: String, before: String, after: String, cause: String },

    /// `Document::push` — new dobject added. Records its index, the
    /// Geom variant name, the handle, the kind-summary.
    DocPush {
        index:         usize,
        geom_kind:     String,
        handle:        u64,
        summary:       String,
    },
    /// `Vec::remove(i)` on doc.dobjects — index + Geom kind + summary.
    DocRemove {
        index:         usize,
        geom_kind:     String,
        summary:       String,
    },

    /// `snapshot_doc()` called — the moment that pushes onto undo_stack.
    UndoSnapshotTaken {
        undo_depth_after: usize,
        bytes_estimate:   usize,
    },
    /// `do_undo()` fired — what fell off / what came back.
    UndoFired { from_depth: usize, to_depth: usize },
    /// `do_redo()` fired.
    RedoFired { from_depth: usize, to_depth: usize },

    /// One of the `apply_*` methods ran. Generic envelope.
    ApplyOp {
        name:               String,    // "apply_trim_pick", "apply_chprop", "apply_hatch"
        before_dobj_count:  usize,
        after_dobj_count:   usize,
        success:            bool,
        detail:             String,    // free-form per-op summary
    },

    /// Memory incident — Doc clone, grid rebuild, ACI table grow, etc.
    /// `bytes` is the BEST-EFFORT size estimate; `name` describes WHAT.
    MemoryEvent {
        name:        String,
        bytes:       usize,
        elapsed_us:  u64,
    },

    /// Dialog or palette state changed.
    WindowToggle { name: String, opened: bool },
    /// Menu button clicked from the menu bar.
    MenuClick    { path: String },
    /// Keyboard event handled outside the cmd-line text edit (Esc, F-keys).
    KeyEvent     { key: String, modifiers: KeyModifiers },
}

/// Where a `run_command` invocation came from. Lets the inspector
/// distinguish "user typed it" from "menu triggered it" from "replay".
#[derive(Clone, Debug)]
pub enum CmdSource {
    Typed,
    Menu(&'static str),
    Replay,
    Internal(&'static str),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl:  bool,
    pub alt:   bool,
}

// ===========================================================================
//                              RECORDER
// ===========================================================================

/// One recorded event with timestamp + file:line of the caller.
#[derive(Clone, Debug)]
pub struct DbgRecord {
    pub elapsed_ms: f64,
    pub event:      DbgEvent,
    pub location:   &'static Location<'static>,
}

/// Snapshot of `Document` at a specific event index, plus a tag.
#[derive(Clone)]
pub struct DbgSnapshot {
    pub event_index: usize,
    pub tag:         String,
    pub doc:         Document,
}

/// The recorder. One per `CadApp`. When `recording == false` every
/// `record(...)` is a tiny no-op so we can leave the calls in for
/// production builds.
pub struct DbgRecorder {
    pub recording:        bool,
    pub session_started:  Option<Instant>,
    pub events:           Vec<DbgRecord>,
    pub snapshots:        Vec<DbgSnapshot>,
    /// Auto-snapshot cadence — every N events. 0 disables.
    pub auto_snap_every:  usize,
    /// Ring-buffer cap. Once exceeded, oldest events drop. 0 = no cap.
    pub max_events:       usize,
    /// `true` → capture full Backtrace per event (very slow). Off by default.
    pub capture_backtrace: bool,
}

impl Default for DbgRecorder {
    fn default() -> Self {
        Self {
            recording:        false,
            session_started:  None,
            events:           Vec::new(),
            snapshots:        Vec::new(),
            auto_snap_every:  50,
            max_events:       100_000,
            capture_backtrace: false,
        }
    }
}

impl DbgRecorder {
    pub fn start(&mut self, reason: &str) {
        self.recording = true;
        self.session_started = Some(Instant::now());
        self.events.clear();
        self.snapshots.clear();
        // Stamp the start event without an auto-snapshot — the caller
        // (CadApp::dbg_start) is responsible for pushing the initial
        // doc snapshot via `take_snapshot`. Decoupled because the
        // recorder doesn't own the Document.
        self.events.push(DbgRecord {
            elapsed_ms: 0.0,
            event: DbgEvent::SessionStart { reason: reason.to_string() },
            location: Location::caller(),
        });
    }

    pub fn stop(&mut self, reason: &str) {
        let count = self.events.len();
        if self.recording {
            self.events.push(DbgRecord {
                elapsed_ms: self.elapsed_ms(),
                event: DbgEvent::SessionStop {
                    reason: reason.to_string(),
                    event_count: count,
                },
                location: Location::caller(),
            });
        }
        self.recording = false;
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.snapshots.clear();
    }

    /// Push one event. Cheap when `!recording`. The `loc` is the
    /// CALLER's location, threaded through by the `dbg_event!` macro.
    pub fn push(&mut self, event: DbgEvent, loc: &'static Location<'static>) {
        if !self.recording { return; }
        if self.max_events > 0 && self.events.len() >= self.max_events {
            // Ring eviction — drop oldest 5% to amortise the move cost.
            let drop = self.max_events / 20;
            self.events.drain(..drop);
        }
        self.events.push(DbgRecord {
            elapsed_ms: self.elapsed_ms(),
            event,
            location: loc,
        });
    }

    /// Push a Document snapshot. Caller (CadApp) owns the Document and
    /// passes a clone. Records a `DocSnapshot` event referencing the
    /// snapshot's index into `self.snapshots`.
    pub fn take_snapshot(
        &mut self,
        doc: &Document,
        reason: &str,
        undo_depth: usize,
        redo_depth: usize,
        loc: &'static Location<'static>,
    ) {
        if !self.recording { return; }
        let idx = self.snapshots.len();
        self.snapshots.push(DbgSnapshot {
            event_index: self.events.len(),
            tag:         reason.to_string(),
            doc:         doc.clone(),
        });
        let dobject_count = doc.dobjects.len();
        let layer_count   = doc.layers.len();
        self.push(
            DbgEvent::DocSnapshot {
                reason:         reason.to_string(),
                dobject_count,
                undo_depth,
                redo_depth,
                layer_count,
                index_in_dump:  idx,
            },
            loc,
        );
    }

    /// Should auto-cadence fire a snapshot AT THIS POINT? Counts events
    /// since the last snapshot.
    pub fn want_auto_snap(&self) -> bool {
        if !self.recording || self.auto_snap_every == 0 { return false; }
        let last_snap_at = self.snapshots.last().map(|s| s.event_index).unwrap_or(0);
        let since = self.events.len().saturating_sub(last_snap_at);
        since >= self.auto_snap_every
    }

    fn elapsed_ms(&self) -> f64 {
        self.session_started
            .map(|t| t.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0)
    }

    /// Export the whole session as a human-readable text dump.
    /// Format: one event per line, time-stamped, file:line stamped.
    /// Designed for paste-into-bug-report + read-by-eye.
    pub fn dump_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "=== SESSION DUMP ({} events, {} snapshots) ===\n",
            self.events.len(), self.snapshots.len()));
        for (i, r) in self.events.iter().enumerate() {
            out.push_str(&format!(
                "[{:6.1} ms] #{:05} @ {}:{} — {}\n",
                r.elapsed_ms,
                i,
                r.location.file().rsplit('/').next().unwrap_or(r.location.file()),
                r.location.line(),
                format_event_oneline(&r.event),
            ));
        }
        out.push_str(&format!(
            "=== END SESSION ({} snapshots in side-buffer) ===\n",
            self.snapshots.len()));
        out
    }
}

/// One-line summary of an event for the text dump. Verbose enough to
/// read at a glance, terse enough that 1000 events fit on screen.
fn format_event_oneline(e: &DbgEvent) -> String {
    match e {
        DbgEvent::SessionStart { reason } =>
            format!("◆ SESSION START — {}", reason),
        DbgEvent::SessionStop { reason, event_count } =>
            format!("◆ SESSION STOP — {} ({} events)", reason, event_count),
        DbgEvent::Note { message } =>
            format!("📝 NOTE: {}", message),
        DbgEvent::DocSnapshot { reason, dobject_count, undo_depth, redo_depth, index_in_dump, .. } =>
            format!("📷 SNAP[{}] {} dobj, undo={}, redo={}  ({})",
                index_in_dump, dobject_count, undo_depth, redo_depth, reason),
        DbgEvent::CmdRun { raw, parsed_debug, source } =>
            format!("⌨ CMD \"{}\" → {}  [{:?}]", raw, parsed_debug, source),
        DbgEvent::CanvasClick { world, hit_dobject, active_tool, active_state, .. } =>
            format!("🖱 CLICK world=({:.3},{:.3})  hit={:?}  tool={}  state={}",
                world.x, world.y, hit_dobject, active_tool, active_state),
        DbgEvent::CanvasPress { world, button, .. } =>
            format!("🖱 PRESS {} @ ({:.3},{:.3})", button, world.x, world.y),
        DbgEvent::CanvasRelease { world, button, drag_px, .. } =>
            format!("🖱 RELEASE {} @ ({:.3},{:.3})  drag={:.1}px", button, world.x, world.y, drag_px),
        DbgEvent::GestureClassification {
            motion_px, motion_dir, egui_clicked, egui_drag_stopped,
            press_world, release_world, hit_at_press, hit_at_release,
            in_select_mode, active_tool, selection_before, selection_after,
            app_action_taken, outcome_summary, ..
        } => {
            // Multi-line so the timeline reader can SEE the whole story.
            format!(
                "🔍 GESTURE press=({:.2},{:.2}) hit_press={:?}  →  release=({:.2},{:.2}) hit_release={:?}\n         \
                 motion={} px ({})  egui: clicked={} drag_stopped={}  select_mode={}  tool={}\n         \
                 sel: {:?} → {:?}  | action: {}\n         \
                 verdict: {}",
                press_world.x, press_world.y, hit_at_press,
                release_world.x, release_world.y, hit_at_release,
                motion_px, motion_dir, egui_clicked, egui_drag_stopped,
                in_select_mode, active_tool,
                selection_before, selection_after,
                app_action_taken, outcome_summary)
        }
        DbgEvent::CanvasDrag { from_world, to_world, button, .. } =>
            format!("🖱 DRAG {} ({:.3},{:.3})→({:.3},{:.3})",
                button, from_world.x, from_world.y, to_world.x, to_world.y),
        DbgEvent::SelectChange { basket_before, basket_after, cause } =>
            format!("✓ SEL {:?} → {:?}  ({})", basket_before, basket_after, cause),
        DbgEvent::ToolChange { from, to, cause } =>
            format!("🔧 TOOL {} → {}  ({})", from, to, cause),
        DbgEvent::StateChange { state_name, before, after, cause } =>
            format!("🔁 {} {} → {}  ({})", state_name, before, after, cause),
        DbgEvent::DocPush { index, geom_kind, handle, summary } =>
            format!("➕ PUSH #{} ({}) handle={:#x} — {}", index, geom_kind, handle, summary),
        DbgEvent::DocRemove { index, geom_kind, summary } =>
            format!("➖ REMOVE #{} ({}) — {}", index, geom_kind, summary),
        DbgEvent::UndoSnapshotTaken { undo_depth_after, bytes_estimate } =>
            format!("💾 UNDO-SNAP depth={} (~{} bytes)", undo_depth_after, bytes_estimate),
        DbgEvent::UndoFired { from_depth, to_depth } =>
            format!("↶ UNDO {} → {}", from_depth, to_depth),
        DbgEvent::RedoFired { from_depth, to_depth } =>
            format!("↷ REDO {} → {}", from_depth, to_depth),
        DbgEvent::ApplyOp { name, before_dobj_count, after_dobj_count, success, detail } =>
            format!("⚙ {} ok={} dobj {}→{} — {}",
                name, success, before_dobj_count, after_dobj_count, detail),
        DbgEvent::MemoryEvent { name, bytes, elapsed_us } =>
            format!("🧠 {} ~{} bytes in {} µs", name, bytes, elapsed_us),
        DbgEvent::WindowToggle { name, opened } =>
            format!("🪟 {} {}", name, if *opened {"OPENED"} else {"CLOSED"}),
        DbgEvent::MenuClick { path } =>
            format!("☰ MENU {}", path),
        DbgEvent::KeyEvent { key, modifiers } =>
            format!("⌨ KEY {} {:?}", key, modifiers),
    }
}

// ===========================================================================
//                       PER-FRAME WATCHED STATE
// ===========================================================================
//
// Snapshot of every field the recorder watches. CadApp keeps the
// previous frame's snapshot and diffs against the current one — every
// changed field emits a `StateChange` (or category-specific) event. No
// per-assignment instrumentation needed.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchedState {
    pub tool:                String,
    pub select_mode:         String,
    pub trim_state:          String,
    pub extend_state:        String,
    pub fillet_state:        String,
    pub chamfer_state:       String,
    pub offset_state:        String,
    pub dist_state:          String,
    pub text_draft:          String,
    pub matchprops_state:    String,
    pub align_state:         String,
    pub stretch_state:       String,
    pub break_state:         String,
    pub lengthen_state:      String,
    /// Block / insert POINT-PICK phases. These capture a coordinate on a
    /// single click; if a pick "catches grips" or selects instead, the
    /// transition here vs. the click that fired is the smoking gun.
    pub block_def_state:     String,
    pub insert_state:        String,
    /// Block dialog "Pick ⊕" base-point capture in progress.
    pub block_pick_base:     bool,
    pub grip_drag:           bool,
    pub selection:           Vec<usize>,
    /// Queued select-mode follow-up op (erase, move, hatch, …). When
    /// the basket is empty and the user types one of these, the cmd
    /// puts itself here and waits for the user to pick. If you see
    /// `queued_op != None` linger past Enter, the queued op never
    /// fired — almost always the bug shape.
    pub queued_op:           String,
    /// Override for the next window — `Some(true)` = force inside
    /// (user typed `w`), `Some(false)` = force crossing (`c`), `None`
    /// = use direction default. Consumed by the FIRST completing
    /// window. Lingering `Some` past a gesture = the override didn't
    /// fire — diagnostic gold.
    pub armed_window_inside: String,
    /// Captured first corner of a two-click window gesture. While
    /// `Some`, the next click commits the window. `None` after a
    /// successful or aborted gesture.
    pub window_first:        String,
    pub doc_dobjects_len:    usize,
    pub undo_depth:          usize,
    pub redo_depth:          usize,
    /// All Window-state flags concatenated as bits → "open/closed"
    /// map of every palette. One field per Window.
    pub window_flags:        WindowFlags,
    /// Persisted SYSVARs we want to see flip live. Cherry-picked —
    /// not every byte of `env` (would flood the timeline).
    pub sysvar_summary:      String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct WindowFlags {
    pub cmd_window:        bool,
    pub layers_window:     bool,
    pub pens_window:       bool,
    pub info_window:       bool,
    pub dobjects_window:   bool,
    pub snap_window:       bool,
    pub trim_debug:        bool,
    pub hatch_debug:       bool,
    pub hatch_dialog:      bool,
    pub hatch_confirm:     bool,
    pub text_style_dialog: bool,
    pub dim_style_dialog:  bool,
    pub dbg_window:        bool,
}

impl WindowFlags {
    /// Pretty-print which windows are open vs closed.
    pub fn summarize(&self) -> String {
        let mut open = Vec::new();
        let mut closed = Vec::new();
        macro_rules! flag {
            ($name:literal, $field:ident) => {
                if self.$field { open.push($name); } else { closed.push($name); }
            };
        }
        flag!("cmd",          cmd_window);
        flag!("layers",       layers_window);
        flag!("pens",         pens_window);
        flag!("info",         info_window);
        flag!("dobjects",     dobjects_window);
        flag!("snap",         snap_window);
        flag!("trim_debug",   trim_debug);
        flag!("hatch_debug",  hatch_debug);
        flag!("hatch_dialog", hatch_dialog);
        flag!("hatch_confirm",hatch_confirm);
        flag!("text_style",   text_style_dialog);
        flag!("dim_style",    dim_style_dialog);
        flag!("dbg_recorder", dbg_window);
        format!("open=[{}] closed=[{}]", open.join(","), closed.join(","))
    }
}

/// Diff two WatchedState snapshots and push one event per changed
/// field. Cause string is `"state poll (frame end)"` so the inspector
/// shows the source of the inference. Returns the number of changes.
pub fn diff_watched(
    rec: &mut DbgRecorder,
    prev: &WatchedState,
    curr: &WatchedState,
    loc: &'static Location<'static>,
) -> usize {
    if !rec.recording { return 0; }
    let mut n = 0;
    macro_rules! diff_field {
        ($name:expr, $field:ident) => {
            if prev.$field != curr.$field {
                rec.push(DbgEvent::StateChange {
                    state_name: $name.to_string(),
                    before:     format!("{:?}", prev.$field),
                    after:      format!("{:?}", curr.$field),
                    cause:      "state poll (frame end)".to_string(),
                }, loc);
                n += 1;
            }
        };
    }
    // Tool gets its own dedicated event variant for easier filtering.
    if prev.tool != curr.tool {
        rec.push(DbgEvent::ToolChange {
            from:  prev.tool.clone(),
            to:    curr.tool.clone(),
            cause: "state poll (frame end)".to_string(),
        }, loc);
        n += 1;
    }
    diff_field!("select_mode",       select_mode);
    diff_field!("trim_state",        trim_state);
    diff_field!("extend_state",      extend_state);
    diff_field!("fillet_state",      fillet_state);
    diff_field!("chamfer_state",     chamfer_state);
    diff_field!("offset_state",      offset_state);
    diff_field!("dist_state",        dist_state);
    diff_field!("text_draft",        text_draft);
    diff_field!("matchprops_state",  matchprops_state);
    diff_field!("align_state",       align_state);
    diff_field!("stretch_state",     stretch_state);
    diff_field!("break_state",       break_state);
    diff_field!("lengthen_state",    lengthen_state);
    diff_field!("block_def_state",   block_def_state);
    diff_field!("insert_state",      insert_state);
    if prev.block_pick_base != curr.block_pick_base {
        rec.push(DbgEvent::StateChange {
            state_name: "block_pick_base".into(),
            before:     prev.block_pick_base.to_string(),
            after:      curr.block_pick_base.to_string(),
            cause:      "state poll (frame end)".into(),
        }, loc);
        n += 1;
    }
    diff_field!("queued_op",           queued_op);
    diff_field!("armed_window_inside", armed_window_inside);
    diff_field!("window_first",        window_first);
    if prev.grip_drag != curr.grip_drag {
        rec.push(DbgEvent::StateChange {
            state_name: "grip_drag".into(),
            before:     prev.grip_drag.to_string(),
            after:      curr.grip_drag.to_string(),
            cause:      "state poll (frame end)".into(),
        }, loc);
        n += 1;
    }
    // Per-window toggle event for any flag flipped.
    macro_rules! window_diff {
        ($name:literal, $field:ident) => {
            if prev.window_flags.$field != curr.window_flags.$field {
                rec.push(DbgEvent::WindowToggle {
                    name:   $name.to_string(),
                    opened: curr.window_flags.$field,
                }, loc);
                n += 1;
            }
        };
    }
    window_diff!("cmd",           cmd_window);
    window_diff!("layers",        layers_window);
    window_diff!("pens",          pens_window);
    window_diff!("info",          info_window);
    window_diff!("dobjects",      dobjects_window);
    window_diff!("snap",          snap_window);
    window_diff!("trim_debug",    trim_debug);
    window_diff!("hatch_debug",   hatch_debug);
    window_diff!("hatch_dialog",  hatch_dialog);
    window_diff!("hatch_confirm", hatch_confirm);
    window_diff!("text_style",    text_style_dialog);
    window_diff!("dbg_recorder",  dbg_window);
    diff_field!("sysvar_summary", sysvar_summary);
    n
}

// ===========================================================================
//                              MACRO
// ===========================================================================

/// Push an event with the CALLER's `Location` automatically attached.
/// Cheap no-op when not recording.
#[macro_export]
macro_rules! dbg_event {
    ($app:expr, $event:expr) => {{
        if $app.dbg.recording {
            $app.dbg.push($event, std::panic::Location::caller());
        }
    }};
}

/// Take a Document snapshot tagged with `reason`. Forwards undo/redo
/// depths and the call site.
#[macro_export]
macro_rules! dbg_snapshot {
    ($app:expr, $reason:expr) => {{
        if $app.dbg.recording {
            $app.dbg.take_snapshot(
                &$app.doc,
                $reason,
                $app.undo_stack.len(),
                $app.redo_stack.len(),
                std::panic::Location::caller(),
            );
        }
    }};
}
