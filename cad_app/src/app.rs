// egui front-end. Pure visualization + command dispatch + interactive draw tools.
// All geometry comes from cad_kernel — no math defined in this file.

use eframe::egui;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::sync::Arc as StdArc;
use std::thread;

use cad_kernel::*;

use crate::gpu::{view_matrix, CircleInstance, GpuCircleRenderer};
use crate::settings::UserEnv;

// Soft cap on candidate-set pair count. Above this an ∩ query refuses to
// compute (with a message), to prevent multi-second / multi-minute freezes.
// 5 million pairs is roughly half a second on this CPU.
const PAIR_LIMIT: usize = 5_000_000;

/// Where the user's customised ACI-wheel permutation lives. Sits next to
/// the other project-root files (Audit.html, Variables.md, etc.) so the
/// arrangement travels with the codebase, not a per-user config dir.
fn aci_mapping_path() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR resolves to `<repo>/cad_app/` at compile time;
    // step one level up to reach the workspace root where the other
    // top-level project artefacts live.
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent()
        .map(|p| p.join("aci_mapping.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("aci_mapping.json"))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool { None, Line, Circle, Arc, Ellipse, EllipseArc, Point, Polyline, Spline }

/// Sub-mode for the polyline draw tool — mirrors AutoCAD PLINE's Line /
/// Arc toggle. `a` (or `arc`) switches Line→Arc; `l` (or `line`)
/// switches Arc→Line. Independent from `Tool::Arc` (a separate draw tool).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlineMode { Line, Arc }

/// Per-click flow modifier inside PLINE Arc mode. Default `Normal` =
/// tangent-continuous arc by endpoint click. `SecondPt` paths take TWO
/// clicks (the on-arc midpoint then the endpoint) and build a 3-point
/// arc instead. Resets to `Normal` after each arc commits, or on
/// mode switch / Esc.
#[derive(Clone, Copy, PartialEq, Debug)]
enum PlineArcSub {
    Normal,
    AwaitingSecondPt,            // user typed `s`; next click = on-arc midpoint
    AwaitingSecondPtEnd(Vec2),   // midpoint captured; next click = endpoint
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArcMethod {
    ThreePoints,
    StartCenterEnd,
    StartCenterAngle,
    StartCenterLength,
    StartEndAngle,
    StartEndDirection,
    StartEndRadius,
    CenterStartEnd,
    CenterStartAngle,
    CenterStartLength,
    Continue,
}

impl ArcMethod {
    fn name(&self) -> &'static str {
        match self {
            ArcMethod::ThreePoints       => "3-Point",
            ArcMethod::StartCenterEnd    => "Start, Center, End",
            ArcMethod::StartCenterAngle  => "Start, Center, Angle",
            ArcMethod::StartCenterLength => "Start, Center, Length",
            ArcMethod::StartEndAngle     => "Start, End, Angle",
            ArcMethod::StartEndDirection => "Start, End, Direction",
            ArcMethod::StartEndRadius    => "Start, End, Radius",
            ArcMethod::CenterStartEnd    => "Center, Start, End",
            ArcMethod::CenterStartAngle  => "Center, Start, Angle",
            ArcMethod::CenterStartLength => "Center, Start, Length",
            ArcMethod::Continue          => "Continue",
        }
    }
    /// Only purely-click-driven methods are wired now. Methods that need a
    /// numeric input (angle / length / radius) or that need previous-dobject
    /// tracking (Continue) are listed-but-frozen until that infra exists.
    fn enabled(&self) -> bool {
        matches!(self,
            ArcMethod::ThreePoints
          | ArcMethod::StartCenterEnd
          | ArcMethod::CenterStartEnd
        )
    }
    fn click_count(&self) -> usize { 3 }
    fn hint(&self, n: usize) -> &'static str {
        match (self, n) {
            (ArcMethod::ThreePoints,    0) => "arc 3p: click first point on arc",
            (ArcMethod::ThreePoints,    1) => "arc 3p: click second point",
            (ArcMethod::ThreePoints,    _) => "arc 3p: click third point    [Esc cancels]",
            (ArcMethod::StartCenterEnd, 0) => "arc S,C,E: click START",
            (ArcMethod::StartCenterEnd, 1) => "arc S,C,E: click CENTER",
            (ArcMethod::StartCenterEnd, _) => "arc S,C,E: click END    [Esc cancels]",
            (ArcMethod::CenterStartEnd, 0) => "arc C,S,E: click CENTER",
            (ArcMethod::CenterStartEnd, 1) => "arc C,S,E: click START",
            (ArcMethod::CenterStartEnd, _) => "arc C,S,E: click END (CCW)    [Esc cancels]",
            _ => "(frozen method — pick another from the arc menu)",
        }
    }
}

const ALL_ARC_METHODS: &[ArcMethod] = &[
    ArcMethod::ThreePoints,
    ArcMethod::StartCenterEnd,
    ArcMethod::StartCenterAngle,
    ArcMethod::StartCenterLength,
    ArcMethod::StartEndAngle,
    ArcMethod::StartEndDirection,
    ArcMethod::StartEndRadius,
    ArcMethod::CenterStartEnd,
    ArcMethod::CenterStartAngle,
    ArcMethod::CenterStartLength,
    ArcMethod::Continue,
];

fn current_hint(tool: Tool, arc_method: ArcMethod, n: usize) -> &'static str {
    match (tool, n) {
        (Tool::None,   _) => "select a tool above, or type a command below",
        (Tool::Line,   0) => "line: click first endpoint",
        (Tool::Line,   _) => "line: click second endpoint    [Esc cancels]",
        (Tool::Circle, 0) => "circle: click center",
        (Tool::Circle, _) => "circle: click point on circumference    [Esc cancels]",
        (Tool::Ellipse, 0) => "ellipse: click CENTER",
        (Tool::Ellipse, 1) => "ellipse: click END of major axis (sets rotation + a)",
        (Tool::Ellipse, _) => "ellipse: click a point on the minor side (sets b)    [Esc cancels]",
        (Tool::EllipseArc, 0) => "ell.arc: click CENTER",
        (Tool::EllipseArc, 1) => "ell.arc: click END of major axis",
        (Tool::EllipseArc, 2) => "ell.arc: click a point on the minor side",
        (Tool::EllipseArc, 3) => "ell.arc: click START point on the ellipse",
        (Tool::EllipseArc, _) => "ell.arc: click END point on the ellipse (CCW)    [Esc cancels]",
        (Tool::Point, _) => "point: click to place    [Esc cancels]",
        (Tool::Polyline, 0) => "polyline: click first vertex    [Esc cancels]",
        (Tool::Polyline, 1) => "polyline: click next vertex; Enter finishes (open); 'c' Enter closes",
        (Tool::Polyline, _) => "polyline: keep clicking vertices; Enter finishes (open); 'c' Enter closes",
        (Tool::Spline,   0) => "spline: click first control point    [Esc cancels]",
        (Tool::Spline,   1) => "spline: click next control point",
        (Tool::Spline,   2) => "spline: click next control point (Enter finishes after ≥3 ctrls)",
        (Tool::Spline,   _) => "spline: keep clicking control points; Enter finishes (open)",
        (Tool::Arc,    _) => arc_method.hint(n),
    }
}

pub struct CadApp {
    doc:           Document,
    intersections: Vec<Vec2>,
    cmd:           String,
    history:       Vec<String>,
    /// Short "what to do next" prompt for the current state, shown right
    /// above the command input. Replaces the long placeholder hint with
    /// a context-aware line — empty when idle. See `set_prompt`.
    current_prompt: String,
    /// Most recent command line the user actually ran (non-empty,
    /// non-whitespace). Pressing Enter on an EMPTY cmd at the top-level
    /// prompt re-runs this — AutoCAD's "repeat last command".
    last_command:  Option<String>,
    /// Counter for the 2-stage "Enter to cancel" pattern during a
    /// select-mode wait with an empty basket: 0 = no Enters yet, 1 =
    /// notice shown, 2 = next Enter cancels. Resets on any state
    /// transition or any cmd input.
    empty_enter_count_in_select: u8,
    /// Grip drag — Some(GripDrag) while the user is dragging a grip
    /// handle of a selected dobject. v1 semantic: dragging any grip
    /// translates the whole dobject by the cursor delta.
    grip_drag: Option<GripDrag>,
    /// Snapshot from the last render pass — how many dobjects exist,
    /// how many landed in the viewport, how many were painted, plus
    /// per-cull skip counters. Surfaced in the "Screen Stats" floating
    /// window so the user can verify the renderer's view of the doc.
    last_render_stats: RenderStats,
    /// APX (approximate / draft display) mode currently active. While
    /// true, every visible dobject renders as a single dot at its
    /// gravity point (per `env.LodAnc`) instead of full geometry —
    /// one instanced GPU draw call for the entire scene, FPS recovers
    /// from single-digit to 60+ on million-dobject drawings. Toggled
    /// by the [APX] badge in the status bar. The underlying data is
    /// unchanged; only the visual is approximate. Selection hit-
    /// testing still uses the real geometry.
    lod_active: bool,
    /// Whether the "Screen Stats" window is currently visible. Toggle
    /// via Tools menu.
    screen_stats_open: bool,
    /// Previous-frame value of `screen_stats_open`. Lets render code
    /// detect the false→true edge (window reopened via menu) and
    /// clear any stale dock-pos so the window appears at its default
    /// position instead of getting stuck behind a panel.
    screen_stats_was_open: bool,
    /// Background worker for hatch trace. `None` when no trace is in
    /// flight. While `Some`, every frame `poll_hatch_worker` tries to
    /// receive the result; on receipt the loops are materialised as
    /// boundary polylines + a Hatch dobject and this field clears.
    /// See `HatchWorker` doc comment for the full lifecycle.
    hatch_worker: Option<HatchWorker>,
    /// Cooperative-cancellation flag for long-running operations
    /// (hatch trace, intersect-everything, bulk modify, …). The flag
    /// is set by the global Esc handler; cancellable functions read
    /// it periodically and bail out early when set.
    ///
    /// CAVEAT: today the heavy ops run SYNCHRONOUSLY inside `update`,
    /// so the egui input snapshot doesn't refresh mid-operation —
    /// Esc presses that happen DURING a long op aren't seen until the
    /// op completes. To make mid-op Esc work we need to background
    /// the op (std::thread or rayon) and have the worker check this
    /// flag while the UI thread keeps spinning. That refactor is the
    /// next slice; this primitive is the API the worker will use.
    ///
    /// For now the flag is useful in two cases:
    ///   1. Esc PRE-set before a new op starts (resets at op begin)
    ///   2. Multi-phase ops can check between phases (already partial
    ///      protection — a long phase still freezes the UI)
    op_cancel: StdArc<AtomicBool>,
    /// Edge-docked positions for floating Windows. When the user drags
    /// a Window within `DOCK_THRESHOLD_PX` of any screen edge, we snap
    /// it flush to that edge and record the snapped position here;
    /// the next frame's window-show pass passes the stored position
    /// via `current_pos(...)` so the snap "sticks". Dragging the
    /// Window away further than the threshold removes the entry, so
    /// the snap is reversible.
    ///
    /// Stores both the snapped position AND the edge the window is
    /// docked to. Edge drives sizing: docked to TOP/BOTTOM forces the
    /// window to span the full screen width (it becomes a horizontal
    /// strip); docked to LEFT/RIGHT forces full screen height
    /// (vertical strip). Corner docks pin position but leave size
    /// content-driven.
    docked_window_pos: HashMap<&'static str, DockState>,
    /// Accumulated hold-time on each docked window's title bar. Once
    /// it reaches `DOCK_UNDOCK_HOLD_SEC` (~200 ms), the dock state is
    /// cleared and the window goes back to free positioning so the
    /// user can drag it elsewhere. Reset to zero whenever the button
    /// is released. Per-window so two windows being held simultaneously
    /// don't share a clock.
    dock_undock_hold: HashMap<&'static str, f32>,
    /// IDs of windows that the user is currently dragging. Used by
    /// `process_dock_after_show` to evaluate snap-to-edge ONLY at the
    /// end of a drag (drag-release transition). Without this gate,
    /// snap would fire on any idle frame where the window happens to
    /// sit near a screen edge — which auto-docks freshly-opened
    /// windows on appear, before the user has even touched them.
    dock_dragging: std::collections::HashSet<&'static str>,
    /// Timestamp (egui `ctx.input(|i| i.time)` units, seconds since
    /// app launch) of the most recent primary-button press on the
    /// canvas. Reset to None on release. Drives the time-gated drag
    /// classifier: a window-drag only activates after the press has
    /// been held longer than `env.SelDmTm`. Without this gate, fast
    /// accidental drags during a click registered as windows.
    press_time: Option<f64>,
    /// Screen-space rect of the canvas (central panel) from the last
    /// frame. Docking uses this — NOT `ctx.screen_rect()` — so docked
    /// strips align with the canvas area instead of overlapping the
    /// menubar / toolbar / status bar. Captured inside the
    /// CentralPanel show closure, read by `apply_dock_pos` /
    /// `process_dock_after_show` which run earlier in the same frame
    /// (so they actually read the PREVIOUS frame's rect — fine, since
    /// the canvas rect only changes when the user resizes the app
    /// window).
    canvas_screen_rect: Option<egui::Rect>,
    /// Open/closed state for each dockable Window panel. Default true
    /// for the most-used panels. Toggled from the Tools menu.
    cmd_window_open:     bool,
    layers_window_open:  bool,
    pens_window_open:    bool,
    info_window_open:    bool,
    dobjects_window_open: bool,
    /// Pattern args for the in-progress hatch op — set when the user
    /// runs `hatch [NAME] [scale] [angle]`, consumed by `apply_hatch`
    /// after the boundary-selection session finalises. Default
    /// (None, 1.0, 0.0) = solid fill.
    pending_hatch_pattern: (Option<String>, f64, f64),
    /// "Choose Hatch Attributes" modal — open when the user runs bare
    /// `hatch` so they can pick pattern + scale + angle with a live
    /// preview before any boundary is selected. Mirrors LibreCAD's
    /// dialog. State lives here so the dialog persists last-used
    /// choices across openings.
    hatch_dialog_open:  bool,
    hatch_dialog_solid: bool,
    hatch_dialog_name:  String,     // catalog name when solid==false
    hatch_dialog_scale: f64,
    hatch_dialog_angle: f64,
    /// "Click inside a region to hatch it" mode — armed by the
    /// dialog's "Pick Point" button. Next pointer-mode click runs the
    /// smallest-containing-closed-dobject search, hatches it, and —
    /// per user request — stays armed for additional picks until
    /// Enter or Esc ends the session. Pattern args are remembered
    /// across clicks via `hatch_pick_point_session`.
    hatch_pick_point_armed: bool,
    /// Pattern args for the active pick-point SESSION (the period
    /// from "Pick Point button clicked" through "Enter / Esc ends
    /// it"). Restored into `pending_hatch_pattern` before each
    /// click's apply, since `apply_hatch` consumes that field —
    /// without snapshotting, the second click would render SOLID
    /// instead of the originally-chosen pattern.
    hatch_pick_point_session: Option<(Option<String>, f64, f64)>,
    /// ACI polar-wheel picker — shared state for the floating picker
    /// window. The same window serves every call site; `pick_request`
    /// names who asked for the pick so the chosen ACI flows back to
    /// the right slot. See `aci_picker.rs` and the user's reference
    /// HTML at `~/workspace/RUST_CAD/ACI_Picker_UI.html`.
    aci_picker:           crate::aci_picker::AciPickerState,
    aci_pick_request:     Option<AciPickRequest>,
    selected:      Option<usize>,

    tool:          Tool,
    arc_method:    ArcMethod,
    arc_picker_open: bool,
    pending:       Vec<Vec2>,
    /// Per-segment bulge for the polyline tool. `pending_bulges[i]` is the
    /// AutoCAD bulge (tan(theta/4)) of the segment from `pending[i]` to
    /// `pending[i+1]`, so it's always one shorter than `pending` while
    /// drawing. `0.0` = straight segment. Maintained only by the polyline
    /// tool — other tools leave it empty.
    pending_bulges: Vec<f64>,
    /// Polyline draw sub-mode (AutoCAD PLINE's Line / Arc toggle).
    /// Toggled inline by typing `a` (Arc) or `l` (Line) while pline is
    /// active. Each captured vertex inherits this mode's segment kind.
    pline_mode:    PlineMode,
    /// Per-click flow inside PLINE Arc mode — `Normal` for the default
    /// tangent-continuous arc, `AwaitingSecondPt` after the user typed
    /// `s` (the next click captures a midpoint on the arc curve), then
    /// `AwaitingSecondPtEnd(mid)` until the endpoint click commits a
    /// 3-point arc. Resets to `Normal` after each arc commits.
    pline_arc_sub: PlineArcSub,

    scale:        f32,
    world_offset: egui::Vec2,

    // array dialog
    array_open:      bool,
    picking_source:  bool,   // dialog hidden, waiting for the user to click an dobject
    array_cols: usize,
    array_rows: usize,
    array_dx:   f64,
    array_dy:   f64,

    // intersection modes (no more global O(N²) auto-recompute)
    intersect_pending_click: bool,           // one-shot "intersect near next click"
    intersect_view_pending:  bool,           // deferred — needs canvas-rect to know what's visible
    last_visible: Option<(Vec2, Vec2)>,      // visible world bbox from the last frame
    last_intersect_label: String,            // shown next to the buttons

    // object-snap override, single-shot: armed by typing a snap code (PER, …),
    // consumed by the next canvas click during a draw.
    snap_override: Option<SnapKind>,

    // persistent osnap state — checkboxes in the floating snap window. The
    // screen-space search radius lives in `env.SpTGSZ` (User-Environment
    // Settings); same for grip enable in `env.GrpEnb`.
    snap_enabled:      SnapSet,
    snap_window_open:  bool,

    /// User-Environment Settings — cryptic-named field for each AutoCAD-
    /// style SYSVAR. Persisted to `$HOME/.config/rust_cad/user_env.txt`.
    env: UserEnv,
    /// Settings window visibility.
    settings_open: bool,

    // "Always-listen" command line: set when something else stole keyboard
    // focus (canvas click, window switch). The command-box renderer
    // reclaims focus on the next frame.
    refocus_cmd: bool,

    // Snap-candidate cycling. When multiple snap targets are within range
    // (e.g. CEN + NEA on the same arc, or two QUA quadrants), Tab cycles
    // through them. The index is reset to 0 whenever the cursor moves more
    // than a few pixels (a different hover position is a different question).
    snap_cycle_index:  usize,
    snap_cycle_anchor: Option<egui::Pos2>,

    // Multi-dobject selection (separate from the `selected: Option<usize>`
    // single-pick used by the array dialog). Built up by the `list` / `select`
    // commands; consumed when the user presses Enter to finalise. AutoCAD-
    // style sub-modes (Add / Remove / Previous / None / All) are typed at the
    // command line during the session.
    select_mode:        SelectMode,
    selection:          Vec<usize>,
    /// First corner of an in-progress window selection.
    window_first:       Option<Vec2>,
    /// When the user explicitly typed `w` / `c` to arm window/crossing
    /// mode, this overrides the drag-direction default for the next two
    /// clicks. None = direction-based default (L→R inside, R→L crossing).
    /// Some(true) = forced inside-window, Some(false) = forced crossing.
    armed_window_inside: Option<bool>,
    /// When true, canvas clicks REMOVE the under-cursor dobject from the
    /// selection instead of adding. Toggled by typing `remove` / `add`
    /// during the session.
    select_remove_mode: bool,
    /// The last finalised selection — `prev` re-adds these indices.
    selection_prev:     Vec<usize>,

    // ---- Move tool (uses the active selection) ----
    move_state: MoveState,
    /// Operation queued behind an in-progress selection session. When the
    /// user finalises the session with Enter, this op is dispatched (e.g.
    /// `Move` → enter MoveState::WaitingForBase). `None` means a plain
    /// `select` / `list` with no follow-up.
    queued_op:  QueuedOp,

    // FPS smoothing
    fps_smooth: f32,

    // spatial index — lazily (re)built on first ∩ query, kept around for both
    // ∩ modes and (when fresh) viewport culling.
    index:       Option<UniformGrid>,
    index_dirty: bool,
    index_label: String,

    // GPU renderer + render-mode switch (debug window)
    render_mode:  RenderMode,
    debug_open:   bool,
    gpu_renderer: StdArc<Mutex<GpuCircleRenderer>>,
    gpu_dirty:    bool,

    // ---- Layer panel (Slice B) ----
    /// Is the layer dock open? Toggled from the top toolbar.
    layer_panel_open: bool,
    /// LayerId currently being renamed in the panel (click name to enter
    /// rename mode); None = no rename in progress.
    layer_rename: Option<LayerId>,
    /// Scratch buffer for the in-progress rename text.
    layer_rename_buf: String,
    /// True on the first frame after a rename was activated — the
    /// rename TextEdit calls `request_focus()` once to steal focus from
    /// the always-listen command line. Cleared after the focus grab so
    /// the user's clicks within the field aren't fighting us.
    layer_rename_focus_pending: bool,
    /// Counter for default layer names ("Layer1", "Layer2", …).
    layer_name_counter: u32,

    // ---- Pen palette (Slice C) ----
    /// Is the pen palette dock open? Toggled from the top toolbar.
    pen_panel_open: bool,

    // ---- Entity Info panel (Slice D) ----
    /// Is the entity-info dock open? Toggled from the top toolbar.
    info_panel_open: bool,

    // ---- Editing operations (Slice J) ----
    copy_state:   CopyState,
    rotate_state: RotateState,
    /// Set by the `C` sub-command during a rotate session. When true,
    /// applying the rotation produces COPIES of the selected dobjects
    /// (originals untouched). Cleared when the session ends.
    rotate_copy:  bool,
    /// Same toggle for scale's `C` sub-command. When true, scale commits
    /// a duplicated, scaled copy instead of mutating the selection.
    scale_copy:   bool,
    scale_state:  ScaleState,
    mirror_state: MirrorState,
    /// Snapshot-based undo stack — every editing operation pushes the
    /// pre-mutation Document. `undo` pops and restores. Bounded so a
    /// long editing session doesn't grow without bound.
    undo_stack:   Vec<Document>,
    /// Companion redo stack. `undo` pops undo_stack and pushes current
    /// state to redo_stack. `redo` does the reverse. Any new editing op
    /// CLEARS redo_stack (new branch — can't redo onto a different
    /// history).
    redo_stack:   Vec<Document>,

    // ---- Slice K: matchprop click capture ----
    matchprops_state: MatchPropsState,

    // ---- Slice L: medium editing actions ----
    offset_state:   OffsetState,
    lengthen_state: LengthenState,
    break_state:    BreakState,
    align_state:    AlignState,
    stretch_state:  StretchState,

    // ---- Slices M.3 / M.4: fillet / chamfer ----
    fillet_state:   FilletState,
    /// AutoCAD "Multiple" mode for Fillet — when true, completing one
    /// fillet re-enters WaitingForFirst instead of returning to Off.
    /// Esc exits. Transient (not a SYSVAR — matches AutoCAD's behavior
    /// where each F command starts in single mode unless you type `m`).
    fillet_multiple: bool,
    /// True when fillet is waiting for the user to type a radius
    /// value (after they typed `r` alone with no arg). The very next
    /// non-empty cmd-line input is consumed as a number — NOT passed
    /// to the main parser. Cleared on success or Esc.
    fillet_waiting_radius: bool,
    chamfer_state:  ChamferState,
    /// Same as `fillet_multiple` for Chamfer.
    chamfer_multiple: bool,
    /// True when chamfer is waiting for distance value(s) after `d`
    /// alone. Next input is parsed as `<a>` or `<a> <b>`.
    chamfer_waiting_distance: bool,

    // ---- Slice M.1 / M.2: trim / extend (two-basket) ----
    trim_state:   TrimState,
    extend_state: ExtendState,
    /// When a trim/extend session begins, the main `selection` is stashed
    /// here so the cutting-edge/boundary-edge select-session can reuse
    /// `self.selection` as its working basket without nuking the user's
    /// real selection. Restored on finalise/cancel.
    pre_op_selection: Vec<usize>,

    // ---- Trim debug log (instrumentation for diagnosis) ----
    /// Detailed click-by-click log of the active trim/extend session.
    /// Auto-cleared at each `trim`/`extend` command start. User opens
    /// the Trim Debug window, copies the log, pastes to repro reports.
    trim_debug_log:  Vec<String>,
    trim_debug_open: bool,
    /// Hatch debug log — same shape as `trim_debug_log`. Records every
    /// state transition in the hatch flow (dialog open/buttons, pattern
    /// changes, pick-point search results, apply_hatch params, resolve
    /// loop counts, render kicks). Logged only when the window is open
    /// so the Vec doesn't grow forever in normal use.
    hatch_debug_log:  Vec<String>,
    hatch_debug_open: bool,
    /// Frame counter for log timestamping — gives ordering even when
    /// multiple clicks happen close in wall-clock time.
    trim_debug_frame: u64,
}

const UNDO_STACK_CAP: usize = 64;

/// Active selection-gathering session. `ForList` dumps the chosen dobjects
/// to the command history when finalised; `ForSelect` just keeps them as
/// the current selection for follow-up commands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectMode {
    Off,
    ForList,
    ForSelect,
    /// Selecting cutting edges for `trim`. On Enter, the basket
    /// transfers to `TrimState::PickingTargets` and the user's main
    /// `selection` is restored from `pre_op_selection`.
    ForCuttingEdges,
    /// Selecting boundary edges for `extend`. Symmetric to ForCuttingEdges.
    ForBoundaryEdges,
}

/// Displacement-tool state machine. `WaitingForBase` is entered when the
/// user runs `move` with a non-empty selection, or when a `move`-queued
/// select session finalises. The next click sets the base point and
/// transitions to `WaitingForDest(base)`; the click after that commits the
/// translation `(dest - base)` to every selected dobject.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MoveState {
    Off,
    WaitingForBase,
    WaitingForDest(Vec2),
}

/// Command queued behind a selection session — finalising the session
/// transitions straight into the queued operation instead of just
/// "keeping" the selection. Lets commands like `move` work nested:
///   `move` → auto-enter select mode → user picks → Enter → base/dest clicks.
///
/// Extend this enum when adding copy / rotate / scale / mirror / trim …
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueuedOp {
    None,
    Move,
    Copy,
    Rotate,
    Scale,
    Mirror,
    /// Join — applied on Enter when the selection session ends. Drives the
    /// kernel's three-pass merge (collinear lines, concentric arcs, chain
    /// → polyline).
    Join,
    /// Hatch — applied on Enter. For every closed polyline in the
    /// finalised selection, append a Hatch dobject whose boundary copies
    /// that polyline's vertices, filled with the active color.
    Hatch,
    /// Array — applied on Enter. The finalised selection becomes the
    /// SOURCES for the array generation; the array dialog re-shows
    /// itself and the user adjusts rows/cols/dx/dy then clicks
    /// Generate. Multi-source: every grid cell instantiates a copy of
    /// every dobject in the selection (offset by cell position).
    Array,
}

/// State machine for the interactive copy tool — same shape as MoveState.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CopyState {
    Off,
    WaitingForBase,
    WaitingForDest(Vec2),
}

/// State machine for the interactive rotate tool — AutoCAD ROTATE flow:
///   1. WaitingForPivot                        — click the pivot.
///   2. WaitingForAngle(pivot)                 — default: click → angle =
///      atan2(click − pivot); OR type a number in degrees; OR type `R`
///      to switch to reference mode; OR type `C` to toggle copy mode
///      (rotate produces a copy instead of moving the original).
///   3. Reference sub-states (3 picks total): RefSrc1 → RefSrc2 defines
///      the source direction (= atan2(s2 − s1)); RefTgt is ONE pick
///      anchored at the PIVOT (target direction = atan2(click − pivot)).
///      Matches scale's reference shape — after the 2 source picks,
///      the next click / typed value is measured from the pivot.
///      Rotation = target − source.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RotateState {
    Off,
    WaitingForPivot,
    WaitingForAngle(Vec2),                                  // pivot
    WaitingForRefSrc1(Vec2),                                // pivot
    WaitingForRefSrc2(Vec2, Vec2),                          // pivot, src1
    WaitingForRefTgt(Vec2, f64),                            // pivot, src_angle
}

/// State machine for the interactive scale tool — AutoCAD SCALE flow:
///   1. WaitingForPivot                       — click the base point.
///   2. WaitingForFactor(pivot)               — default: click → factor =
///      |click − pivot|; OR type a number directly; OR type `R` to switch
///      to reference mode; OR type `C` to toggle copy mode.
///   3. Reference sub-states (3 picks): RefStart → RefEnd → NewLength.
///      ref_d = |RefEnd − RefStart|; factor = NewLength / ref_d, where
///      NewLength is either |click − pivot| or a typed number.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ScaleState {
    Off,
    WaitingForPivot,
    WaitingForFactor(Vec2),                                 // pivot
    WaitingForRefStart(Vec2),                               // pivot
    WaitingForRefEnd(Vec2, Vec2),                           // pivot, ref_start
    WaitingForNewLength(Vec2, f64),                         // pivot, ref_d
}

/// State machine for the interactive mirror tool — two clicks define the axis.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MirrorState {
    Off,
    WaitingForA,
    WaitingForB(Vec2),
}

/// State machine for matchprop — one click selects a source dobject;
/// its `style` is then assigned to every dobject in the basket.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MatchPropsState {
    Off,
    WaitingForSource,
}

/// State machines for the Slice-L click-driven actions.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OffsetState   { Off, WaitingForSide(f64) }
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LengthenState { Off, WaitingForSide(f64) }
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BreakState    { Off, WaitingForPoint }

/// Grip drag — recorded when the user grabs a grip handle of a selected
/// dobject (either by pressing+dragging OR clicking on it). v2: each grip
/// has a role (`GripRole`) that decides what changes when the user moves
/// it (e.g. line endpoint moves only that endpoint; circle quadrant
/// changes radius; line midpoint translates the whole line). Math lives
/// in `cad_kernel::Geom::with_grip_moved`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GripDrag {
    pub dobject_idx: usize,
    pub role:        GripRole,
    pub grip_origin: Vec2,
}

/// Fillet — Slice M.3. Two-click flow: pick first object → pick second.
/// `radius` is captured at session start so re-running `fillet` with a
/// different value mid-flow doesn't change behaviour (sticky session).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FilletState {
    Off,
    WaitingForFirst(f64),
    WaitingForSecond(f64, usize, Vec2),   // radius, idx of first, click point
}

/// Chamfer — Slice M.4. Same shape as FilletState, with two distances.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ChamferState {
    Off,
    WaitingForFirst(f64, f64),
    WaitingForSecond(f64, f64, usize, Vec2),
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AlignState {
    Off,
    WaitingForSrc1,
    WaitingForSrc2(Vec2),
    WaitingForTgt1(Vec2, Vec2),
    WaitingForTgt2(Vec2, Vec2, Vec2),
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StretchState {
    Off,
    WaitingForWin1,
    WaitingForWin2(Vec2),         // first corner captured
    WaitingForBase(Vec2, Vec2),   // window captured, click base
    WaitingForDest(Vec2, Vec2, Vec2),   // window + base; click dest to apply
}

/// Trim/extend session state. Each holds the confirmed cutting/boundary
/// basket; the target-pick phase loops on canvas clicks until the user
/// presses Enter or Esc.
#[derive(Clone, Debug)]
pub enum TrimState {
    Off,
    SelectingCutters,             // running a ForCuttingEdges select session
    PickingTargets(Vec<usize>),   // cutters confirmed; loop on click
    /// "Empty Enter at cutter prompt" mode: every CURRENT dobject in the
    /// doc is a cutter, recomputed on every click. Pieces created by
    /// prior trims this session automatically join the cutter set. This
    /// is the AutoCAD default and the only way "trim against everything
    /// you see" can stay true across multiple clicks.
    PickingTargetsAll,
}

#[derive(Clone, Debug)]
pub enum ExtendState {
    Off,
    SelectingBoundaries,
    PickingTargets(Vec<usize>),
    /// Same "use every current dobject as a boundary" mode as
    /// `TrimState::PickingTargetsAll`.
    PickingTargetsAll,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderMode {
    Cpu,
    Gpu,
}

/// Which screen edge (if any) a floating Window is docked against.
/// Drives the size behavior in `apply_dock_pos`:
///   * Top/Bottom → full-width strip, content-fit height
///   * Left/Right → full-height strip, content-fit width
///   * Corners    → pinned position only, size content-fit
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DockEdge {
    Top, Bottom, Left, Right,
    TopLeft, TopRight, BottomLeft, BottomRight,
}

#[derive(Clone, Copy, Debug)]
pub struct DockState {
    pub pos:  egui::Pos2,
    pub edge: DockEdge,
}

/// Snapshot of what the render loop did this frame. Surfaced in the
/// "Screen Stats" floating window so the user can see at a glance how
/// many dobjects exist, how many are in the viewport, and how many
/// actually got painted (vs filtered by visibility/sub-pixel culls).
///
/// The `in_viewport` count tells you whether the spatial-index broad-
/// phase is doing useful work: `in_viewport == total` at any zoom
/// means the cull isn't helping (typically because the index is
/// stale or absent).
#[derive(Clone, Debug, Default)]
pub struct RenderStats {
    /// Total dobjects in the document.
    pub total:           usize,
    /// Passed the bbox-cull (spatial-index query OR full scan if index
    /// is stale). This is the candidate set the render loop iterates.
    pub in_viewport:     usize,
    /// Painted this frame. Less than `in_viewport` when some
    /// candidates were skipped (hidden / frozen layer / sub-pixel).
    pub drawn:           usize,
    /// Skipped: hidden Dobject style.visible == false OR layer is
    /// hidden/frozen.
    pub skipped_hidden:  usize,
    /// Skipped: bbox < 1 pixel (the micro-cull); no visible benefit
    /// to painting them.
    pub skipped_subpx:   usize,
    /// Frame time (seconds). Inverse of FPS.
    pub frame_dt:        f32,
    /// Last frame's spatial-index status string ("idx N entries" /
    /// "idx stale" / etc.).
    pub index_label:     String,
}

/// Result delivered from the hatch trace worker thread back to the
/// main UI thread via mpsc. The worker bundles its log buffer with
/// the result so the per-hit attempt diagnostics and the
/// tessellate/split/cluster counts arrive together with the loops
/// (otherwise we'd need a streaming log channel, which complicates
/// the lifecycle for no real gain).
pub enum HatchWorkerResult {
    /// Trace succeeded — `loops[0]` is the outer, the rest are islands.
    Success {
        loops:     Vec<Vec<Vec2>>,
        log_lines: Vec<String>,
    },
    /// Trace failed (dead-ended, no valid face contains the seed, etc.).
    /// Caller should fall back to cheap path with auto-islands.
    Failure {
        reason:    String,
        log_lines: Vec<String>,
    },
    /// User pressed Esc; the worker's cancel flag tripped a check
    /// somewhere in tessellate / split / cluster / trace.
    Cancelled {
        log_lines: Vec<String>,
    },
}

/// Handle on a background hatch-trace operation. Held in
/// `CadApp::hatch_worker` while a worker is alive; `poll_hatch_worker`
/// drains it each frame and materialises the result when ready.
///
/// Lifecycle:
///   * spawn: `apply_pick_point_hatch` decides trace path → clones the
///     Document (CHEAP via Arc-internals for handles + Vec for
///     dobjects), spawns a `std::thread`, stores the receiver here.
///   * poll: each `update` call tries `rx.try_recv`; on Ok the
///     worker output is materialised and the field is set to None.
///   * cancel: Esc handler sets `cancel` and drops the worker; the
///     thread reads the flag at its next CANCEL_CHECK_STRIDE point
///     and exits (its send may fail — that's fine, we already dropped
///     the receiver).
///   * replace: a fresh Pick Point click while a worker is in flight
///     cancels the current one + spawns a new one. The old thread
///     exits naturally; its result is discarded.
pub struct HatchWorker {
    pub seed:         Vec2,
    pub pattern:      cad_kernel::HatchPattern,
    pub active_layer: LayerId,
    #[allow(dead_code)]  // mirror of op_cancel; kept for direct debugging access
    pub cancel:       StdArc<AtomicBool>,
    pub rx:           mpsc::Receiver<HatchWorkerResult>,
}

/// Who asked the floating ACI picker for a color, so the chosen ACI flows
/// back to the right slot when the user clicks a circle in the wheel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AciPickRequest {
    /// Picker is editing a layer's color.
    Layer(LayerId),
    /// Picker is editing a dobject's color (Info palette dobject edit).
    Dobject(usize),
}

impl Default for CadApp {
    fn default() -> Self {
        let mut s = Self {
            doc:           Document::default(),
            intersections: Vec::new(),
            cmd:                String::new(),
            history:            Vec::new(),
            current_prompt:     String::new(),
            last_command:       None,
            empty_enter_count_in_select: 0,
            grip_drag: None,
            last_render_stats:   RenderStats::default(),
            lod_active:          false,
            screen_stats_open:   true,
            screen_stats_was_open: true,
            hatch_worker:        None,
            op_cancel:           StdArc::new(AtomicBool::new(false)),
            docked_window_pos:   HashMap::new(),
            dock_undock_hold:    HashMap::new(),
            dock_dragging:       std::collections::HashSet::new(),
            canvas_screen_rect:  None,
            press_time:          None,
            cmd_window_open:     true,
            layers_window_open:  true,
            pens_window_open:    false,
            info_window_open:    false,
            dobjects_window_open: false,
            aci_picker:          {
                let mut p = crate::aci_picker::AciPickerState::default();
                p.try_load_mapping(&aci_mapping_path());
                p
            },
            aci_pick_request:    None,
            selected:      None,
            tool:          Tool::None,
            arc_method:    ArcMethod::ThreePoints,
            arc_picker_open: false,
            pending:       Vec::new(),
            pending_bulges: Vec::new(),
            pline_mode:    PlineMode::Line,
            pline_arc_sub: PlineArcSub::Normal,
            scale:         6.0,
            world_offset:  egui::Vec2::ZERO,
            array_open:     false,
            picking_source: false,
            array_cols:     10,
            array_rows:     10,
            array_dx:       50.0,
            array_dy:       50.0,
            intersect_pending_click: false,
            intersect_view_pending:  false,
            last_visible:            None,
            last_intersect_label:    String::new(),
            snap_override:     None,
            snap_enabled:      SnapSet::defaults(),
            snap_window_open:  false,
            env:               UserEnv::load(),
            settings_open:     false,
            refocus_cmd:       true,
            snap_cycle_index:  0,
            snap_cycle_anchor: None,
            select_mode:        SelectMode::Off,
            selection:          Vec::new(),
            window_first:       None,
            armed_window_inside: None,
            select_remove_mode: false,
            selection_prev:     Vec::new(),
            move_state:         MoveState::Off,
            queued_op:          QueuedOp::None,
            pending_hatch_pattern: (None, 1.0, 0.0),
            hatch_dialog_open:  false,
            hatch_dialog_solid: false,
            hatch_dialog_name:  "ANSI31".into(),
            hatch_dialog_scale: 1.0,
            hatch_dialog_angle: 0.0,
            hatch_pick_point_armed: false,
            hatch_pick_point_session: None,
            fps_smooth: 0.0,
            index:       None,
            index_dirty: true,
            index_label: String::new(),
            render_mode:  RenderMode::Cpu,
            debug_open:   true,
            gpu_renderer: StdArc::new(Mutex::new(GpuCircleRenderer::default())),
            gpu_dirty:    true,
            layer_panel_open:   true,
            layer_rename:       None,
            layer_rename_buf:   String::new(),
            layer_rename_focus_pending: false,
            layer_name_counter: 0,
            pen_panel_open:     true,
            info_panel_open:    true,
            copy_state:   CopyState::Off,
            rotate_state: RotateState::Off,
            rotate_copy:  false,
            scale_copy:   false,
            scale_state:  ScaleState::Off,
            mirror_state: MirrorState::Off,
            undo_stack:   Vec::new(),
            redo_stack:   Vec::new(),
            matchprops_state: MatchPropsState::Off,
            offset_state:   OffsetState::Off,
            lengthen_state: LengthenState::Off,
            break_state:    BreakState::Off,
            align_state:    AlignState::Off,
            stretch_state:  StretchState::Off,
            fillet_state:   FilletState::Off,
            fillet_multiple: false,
            fillet_waiting_radius: false,
            chamfer_state:  ChamferState::Off,
            chamfer_multiple: false,
            chamfer_waiting_distance: false,
            trim_state:     TrimState::Off,
            extend_state:   ExtendState::Off,
            pre_op_selection: Vec::new(),
            trim_debug_log:  Vec::new(),
            trim_debug_open: false,
            hatch_debug_log:  Vec::new(),
            hatch_debug_open: false,
            trim_debug_frame: 0,
        };
        // Demo layers so the Layer panel has visible content at first
        // launch. ACI palette colors keep these out of the truecolor
        // table (per the "ACI is primary" memo + the storage refactor
        // that made TrueColors a shared, dedup'd table).
        let walls = s.doc.layers.add(Layer {
            name: "WALLS".into(), color: Color::Aci(1),   // red
            ..Layer::layer_zero()
        });
        let _hidden = s.doc.layers.add(Layer {
            name: "HIDDEN".into(), color: Color::Aci(3),  // green
            visible: false,
            ..Layer::layer_zero()
        });
        s.doc.layers.active = walls;

        // Demo dobjects so the canvas is never empty on first launch.
        s.doc.push(Line {
            a: Vec2::new(-40.0, -20.0), b: Vec2::new(40.0, 20.0),
        }.into());
        s.doc.push(Circle {
            center: Vec2::new(0.0, 0.0), radius: 30.0,
        }.into());
        s.doc.push(Arc {
            center: Vec2::new(0.0, 0.0), radius: 45.0,
            start_angle: 0.0, sweep_angle: std::f64::consts::PI,
        }.into());
        // Demo ellipse — tilted so its rotated QUA points are visible
        s.doc.push(Ellipse {
            center: Vec2::new(-60.0, -30.0),
            major:  Vec2::new(25.0, 12.0),    // semi-major ≈ 27.7, rotation ≈ 25.6°
            ratio:  0.55,                     // semi-minor ≈ 15.2
        }.into());
        // Demo point + polyline (Slice E preview).
        s.doc.push(Point {
            location: Vec2::new(60.0, -40.0), style: 0, size: 0.0,
        }.into());
        s.doc.push(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(50.0,  40.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(70.0,  60.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(90.0,  40.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(80.0,  20.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(55.0,  25.0), bulge: 0.0 },
            ],
            closed: true,
        }.into());
        s.recompute();
        s.history.push("RUST_CAD math workbench — three demo dobjects loaded.".into());
        s.history.push("Pick a tool from the top toolbar, or type 'help'.".into());
        s
    }
}

const HELP: &str = "\
line     x1,y1 x2,y2                 - draw line segment
circle   cx,cy r                     - draw circle

arc      cx,cy r start_deg end_deg   - center + radius + start/end angles (CCW)
arc3p    p1 p2 p3                    - through three points
arcse    cx,cy start end             - center + start point + end point (CCW)
arccr    start end r [major|minor]   - chord + radius (default minor)
arccl    start end length [left|right] - chord + arc length (default left)

del N                                - delete dobject N
clear                                - remove everything
help                                 - this message

toolbar:
  pointer  - no tool (commands only)
  line     - click two endpoints
  circle   - click center, then any point on the rim
  arc      - click center, start point, end point (CCW)
  Esc cancels an in-progress draw";

impl CadApp {
    // ---- commands & math -----------------------------------------------

    /// Update the live status line shown above the cmd input. Empty
    /// string clears it. Replaces history-pushed prompts so the user sees
    /// only the CURRENT instruction, not a growing pile.
    fn set_prompt<S: Into<String>>(&mut self, s: S) {
        self.current_prompt = s.into();
        self.empty_enter_count_in_select = 0;
    }
    fn clear_prompt(&mut self) {
        self.current_prompt.clear();
        self.empty_enter_count_in_select = 0;
    }

    fn run_command(&mut self, raw: &str) {
        self.history.push(format!("> {}", raw));
        let trimmed = raw.trim();
        // Any non-empty input cancels the 2-stage-Enter notice.
        self.empty_enter_count_in_select = 0;

        // ---- Pending-sub-arg intercepts (must run FIRST) ----------
        // When a fillet/chamfer sub-option has prompted for a numeric
        // arg (e.g. user typed `r` alone), the next non-empty input
        // is consumed as that number and must NOT reach the main
        // parser. Without this, typing `2` after `r` produced
        // `unknown command '2'`.
        if self.fillet_waiting_radius && !trimmed.is_empty() {
            match trimmed.parse::<f64>() {
                Ok(v) => {
                    self.env.FltRad = v;
                    let _ = self.env.save();
                    self.fillet_state = FilletState::WaitingForFirst(v);
                    self.fillet_waiting_radius = false;
                    self.history.push(format!("  fillet: radius → {}", v));
                    self.refresh_fillet_prompt();
                }
                Err(_) => {
                    self.history.push(format!(
                        "  ! fillet: '{}' is not a number — type a radius or Esc to cancel",
                        trimmed));
                }
            }
            return;
        }
        if self.chamfer_waiting_distance && !trimmed.is_empty() {
            let mut toks = trimmed.split_ascii_whitespace();
            let a = toks.next().and_then(|s| s.parse::<f64>().ok());
            let b = toks.next().and_then(|s| s.parse::<f64>().ok());
            match a {
                Some(d1) => {
                    let d2 = b.unwrap_or(d1);
                    self.env.ChmDs1 = d1;
                    self.env.ChmDs2 = d2;
                    let _ = self.env.save();
                    self.chamfer_state = ChamferState::WaitingForFirst(d1, d2);
                    self.chamfer_waiting_distance = false;
                    self.history.push(format!(
                        "  chamfer: distances → ({}, {})", d1, d2));
                    self.refresh_chamfer_prompt();
                }
                None => {
                    self.history.push(format!(
                        "  ! chamfer: '{}' is not a number — type `<a>` or `<a> <b>` or Esc",
                        trimmed));
                }
            }
            return;
        }

        // ---- PLINE sub-command intercept (AutoCAD PLINE Line/Arc flow) ----
        //
        // While the polyline tool is active, single-letter inputs are
        // PLINE SUB-COMMANDS, not the same-letter global commands.
        // Phase 1 wires the most-used three; the remaining options
        // (W/H/Length/Center/Direction/Radius/Second-pt/Angle) print
        // "not yet wired" so the user knows they're recognised but
        // queued. The prompt mentions all of them.
        if self.tool == Tool::Polyline {
            let lc = trimmed.to_ascii_lowercase();
            match lc.as_str() {
                "a" | "arc" => {
                    self.pline_mode = PlineMode::Arc;
                    self.pline_arc_sub = PlineArcSub::Normal;
                    self.update_pline_prompt();
                    return;
                }
                "l" | "line" if self.pline_mode == PlineMode::Arc => {
                    self.pline_mode = PlineMode::Line;
                    self.pline_arc_sub = PlineArcSub::Normal;
                    self.update_pline_prompt();
                    return;
                }
                "u" | "undo" => {
                    // If a Second-pt flow is mid-step, undo that sub-step
                    // first instead of yanking a committed vertex.
                    if self.pline_arc_sub != PlineArcSub::Normal {
                        self.pline_arc_sub = PlineArcSub::Normal;
                        self.history.push("  pline: cancelled Second-pt flow".into());
                        self.update_pline_prompt();
                        return;
                    }
                    if let Some(last) = self.pending.pop() {
                        self.pending_bulges.pop();
                        self.history.push(format!(
                            "  pline: removed vertex ({:.3},{:.3})",
                            last.x, last.y));
                        self.update_pline_prompt();
                    } else {
                        self.history.push("  ! pline: nothing to undo".into());
                    }
                    return;
                }
                // Second-pt: 3-click arc (start = last vertex, second pt on
                // arc, endpoint). Only meaningful in Arc mode and with at
                // least one captured vertex to start the arc from.
                "s" | "second" if self.pline_mode == PlineMode::Arc => {
                    if self.pending.is_empty() {
                        self.history.push(
                            "  ! pline: need a starting vertex before Second-pt".into());
                        return;
                    }
                    self.pline_arc_sub = PlineArcSub::AwaitingSecondPt;
                    self.update_pline_prompt();
                    return;
                }
                // Recognised-but-not-wired sub-options: tell the user
                // they're known so they don't keep retyping. Wire-up
                // lands in the Phase 2 slice.
                "w" | "width"
                | "h" | "halfwidth"
                | "len" | "length"
                | "ce" | "center"
                | "d" | "direction"
                | "r" | "radius"
                | "ang" | "angle" => {
                    self.history.push(format!(
                        "  pline: sub-option '{}' recognised but not yet wired (Phase 2)",
                        trimmed));
                    return;
                }
                _ => {}
            }
        }

        // ---- Fillet sub-command intercept ------------------------------
        // While fillet is active (waiting for first or second pick), the
        // user can type sub-options:
        //   r <num>  — set radius (also `r` alone re-prompts for it)
        //   t        — toggle trim mode (TrmMd SYSVAR)
        //   m        — toggle Multiple mode (loop after each fillet)
        // Anything else falls through to the normal parser.
        if matches!(self.fillet_state,
            FilletState::WaitingForFirst(_) | FilletState::WaitingForSecond(_, _, _))
        {
            let lc = trimmed.to_ascii_lowercase();
            let mut toks = lc.split_ascii_whitespace();
            match toks.next() {
                Some("t") | Some("trim") | Some("nt") | Some("notrim") => {
                    self.env.TrmMd = !self.env.TrmMd;
                    let _ = self.env.save();
                    self.history.push(format!(
                        "  fillet: trim mode → {}",
                        if self.env.TrmMd { "TRIM (lines cut to arc)" }
                        else { "NO TRIM (lines kept, arc added)" }));
                    self.refresh_fillet_prompt();
                    return;
                }
                Some("m") | Some("multiple") => {
                    self.fillet_multiple = !self.fillet_multiple;
                    self.history.push(format!(
                        "  fillet: multiple mode → {}",
                        if self.fillet_multiple { "ON (loops after each fillet, Esc to exit)" }
                        else { "OFF (one fillet then exit)" }));
                    self.refresh_fillet_prompt();
                    return;
                }
                Some("r") | Some("radius") => {
                    if let Some(num) = toks.next() {
                        if let Ok(v) = num.parse::<f64>() {
                            self.env.FltRad = v;
                            let _ = self.env.save();
                            self.fillet_state = FilletState::WaitingForFirst(v);
                            self.history.push(format!(
                                "  fillet: radius → {}", v));
                            self.refresh_fillet_prompt();
                            return;
                        }
                        self.history.push(
                            format!("  ! fillet: bad radius '{}'", num));
                        return;
                    }
                    // `r` alone — arm pending-radius-input. The next
                    // numeric input fires the pending-input intercept
                    // above and sets the radius. Without this flag,
                    // typing `2` next would hit the main parser as
                    // an unknown command.
                    self.fillet_waiting_radius = true;
                    self.set_prompt(format!(
                        "fillet: enter new radius (current {})  [Esc=cancel]",
                        self.env.FltRad));
                    return;
                }
                _ => {}
            }
        }

        // ---- Chamfer sub-command intercept -----------------------------
        // Same shape as Fillet's. Sub-options:
        //   d <a> <b> — set distances (b defaults to a)
        //   t         — toggle trim mode (shared TrmMd)
        //   m         — toggle Multiple mode
        if matches!(self.chamfer_state,
            ChamferState::WaitingForFirst(_, _) | ChamferState::WaitingForSecond(_, _, _, _))
        {
            let lc = trimmed.to_ascii_lowercase();
            let mut toks = lc.split_ascii_whitespace();
            match toks.next() {
                Some("t") | Some("trim") | Some("nt") | Some("notrim") => {
                    self.env.TrmMd = !self.env.TrmMd;
                    let _ = self.env.save();
                    self.history.push(format!(
                        "  chamfer: trim mode → {}",
                        if self.env.TrmMd { "TRIM" } else { "NO TRIM" }));
                    self.refresh_chamfer_prompt();
                    return;
                }
                Some("m") | Some("multiple") => {
                    self.chamfer_multiple = !self.chamfer_multiple;
                    self.history.push(format!(
                        "  chamfer: multiple mode → {}",
                        if self.chamfer_multiple { "ON" } else { "OFF" }));
                    self.refresh_chamfer_prompt();
                    return;
                }
                Some("d") | Some("distance") => {
                    if let Some(a) = toks.next().and_then(|s| s.parse::<f64>().ok()) {
                        let b = toks.next().and_then(|s| s.parse::<f64>().ok()).unwrap_or(a);
                        self.env.ChmDs1 = a;
                        self.env.ChmDs2 = b;
                        let _ = self.env.save();
                        self.chamfer_state = ChamferState::WaitingForFirst(a, b);
                        self.history.push(format!(
                            "  chamfer: distances → ({}, {})", a, b));
                        self.refresh_chamfer_prompt();
                        return;
                    }
                    // `d` alone — arm pending-distance-input. The
                    // next numeric input (or pair) fires the
                    // pending-input intercept above.
                    self.chamfer_waiting_distance = true;
                    self.set_prompt(format!(
                        "chamfer: enter `<a>` or `<a> <b>` (current {}, {})  [Esc=cancel]",
                        self.env.ChmDs1, self.env.ChmDs2));
                    return;
                }
                _ => {}
            }
        }

        // ---- Rotate sub-command intercept (AutoCAD ROTATE flow) -----
        // During WaitingForAngle: typing a NUMBER applies that rotation
        // (degrees, CCW positive); typing `r` switches to reference
        // mode; typing `c` toggles copy mode for the commit.
        if let RotateState::WaitingForAngle(pivot) = self.rotate_state {
            let lc = trimmed.to_ascii_lowercase();
            match lc.as_str() {
                "r" | "ref" | "reference" => {
                    self.rotate_state = RotateState::WaitingForRefSrc1(pivot);
                    self.set_prompt(
                        "rotate-R: click SOURCE point 1 (defines current direction)");
                    return;
                }
                "c" | "cp" | "copy" => {
                    self.rotate_copy = !self.rotate_copy;
                    self.set_prompt(format!(
                        "rotate (pivot=({:.2},{:.2})): copy {} — click to pick angle, type number, R=reference",
                        pivot.x, pivot.y,
                        if self.rotate_copy { "ON" } else { "off" }));
                    return;
                }
                _ => {
                    if let Ok(deg) = trimmed.parse::<f64>() {
                        let rad = deg.to_radians();
                        self.apply_rotate_or_copy(pivot, rad);
                        self.rotate_state = RotateState::Off;
                        self.rotate_copy  = false;
                        self.clear_prompt();
                        return;
                    }
                    // Not a number / sub-command: fall through to the
                    // parser (e.g. user typed `esc` or another global).
                }
            }
        }
        // ---- Scale sub-command intercept (AutoCAD SCALE flow) ------
        // Same shape as rotate: typed number = factor; `R` = reference;
        // `C` = toggle copy. WaitingForNewLength also accepts a typed
        // number as the new length (factor = new / ref_d).
        if let ScaleState::WaitingForFactor(pivot) = self.scale_state {
            let lc = trimmed.to_ascii_lowercase();
            match lc.as_str() {
                "r" | "ref" | "reference" => {
                    self.scale_state = ScaleState::WaitingForRefStart(pivot);
                    self.set_prompt("scale-R: click REFERENCE start (defines old length)");
                    return;
                }
                "c" | "cp" | "copy" => {
                    self.scale_copy = !self.scale_copy;
                    self.set_prompt(format!(
                        "scale (pivot=({:.2},{:.2})): copy {} — click for factor, type number, R=reference",
                        pivot.x, pivot.y,
                        if self.scale_copy { "ON" } else { "off" }));
                    return;
                }
                _ => {
                    if let Ok(factor) = trimmed.parse::<f64>() {
                        self.apply_scale_or_copy(pivot, factor);
                        self.scale_state = ScaleState::Off;
                        self.scale_copy  = false;
                        self.clear_prompt();
                        return;
                    }
                }
            }
        }
        if let ScaleState::WaitingForNewLength(pivot, ref_d) = self.scale_state {
            if let Ok(new_len) = trimmed.parse::<f64>() {
                if new_len > EPS && ref_d > EPS {
                    self.apply_scale_or_copy(pivot, new_len / ref_d);
                }
                self.scale_state = ScaleState::Off;
                self.scale_copy  = false;
                self.clear_prompt();
                return;
            }
        }
        // Rotate-R target step also accepts a typed angle (degrees).
        // dtheta = typed - src_angle.
        if let RotateState::WaitingForRefTgt(pivot, src_angle) = self.rotate_state {
            if let Ok(deg) = trimmed.parse::<f64>() {
                let tgt = deg.to_radians();
                let mut dtheta = (tgt - src_angle).rem_euclid(std::f64::consts::TAU);
                if dtheta > std::f64::consts::PI {
                    dtheta -= std::f64::consts::TAU;
                }
                self.apply_rotate_or_copy(pivot, dtheta);
                self.rotate_state = RotateState::Off;
                self.rotate_copy  = false;
                self.clear_prompt();
                return;
            }
        }
        // ---- Selection-mode shortcut intercept ----
        //
        // While a select session is active, single-letter input is a
        // SELECTION SUB-COMMAND, not the same-letter global command.
        // See memo `feedback_rust_cad_selection_shortcuts`. We rewrite
        // the input below so the parser hands back the right Command.
        let effective = if self.select_mode != SelectMode::Off {
            let lc = raw.trim().to_ascii_lowercase();
            match lc.as_str() {
                "w"                                => "window".to_string(),
                "c" | "cr"                         => "crossing".to_string(),
                "a"                                => "all".to_string(),
                "b" | "bef"                        => "before".to_string(),
                "l"                                => "last".to_string(),
                "n"                                => "none".to_string(),
                _                                  => raw.to_string(),
            }
        } else {
            raw.to_string()
        };
        let parsed = parse(&effective);
        // Remember the line as the "last command" for Enter-on-empty
        // repeat — ONLY on a successful parse. Set BEFORE dispatch so
        // each Ok arm can clear / overwrite it if its own semantics
        // demand. Sub-command intercepts (PLINE, rotate, scale, …)
        // happened above and returned early; numbers / R / C typed
        // inside those sessions never reach here.
        //
        // Why "successful only": a failed line like `1` typed by
        // mistake used to overwrite last_command, so the next Enter
        // re-ran `1` and printed "unknown command '1'" forever. The
        // user's rule (2026-06-06): Enter-on-empty must repeat the
        // last VALID command, never a sub-command and never a typo.
        if !trimmed.is_empty() && parsed.is_ok() {
            self.last_command = Some(trimmed.to_string());
        }
        // Echo the canonical command name into the history "log book"
        // so a glance shows `Fillet` whether the user typed `f`, `F`,
        // or `fillet`. Only fires for real commands, not Add/SetTool
        // (those are noisy + their own history lines already cover it).
        if let Ok(ref c) = parsed {
            let canon = c.canonical_name();
            let raw_lc = trimmed.to_ascii_lowercase();
            let aliased = !raw_lc.starts_with(&canon.to_ascii_lowercase());
            let interesting = !matches!(c,
                Command::Add(_) | Command::SetTool(_) | Command::SnapOverride(_));
            if aliased && interesting {
                self.history.push(format!("  command: {}", canon));
            }
        }
        match parsed {
            Ok(Command::Add(e))   => self.add_dobject(e, "command"),
            Ok(Command::Delete(i)) => {
                if i < self.doc.dobjects.len() {
                    self.doc.dobjects.remove(i);
                    self.history.push(format!("  - removed #{}", i));
                    self.selected = None;
                    self.intersections.clear();
                    self.index_dirty = true;
                } else {
                    self.history.push(format!("  ! no dobject #{}", i));
                }
            }
            Ok(Command::Clear) => {
                self.clear_all();
                self.history.push("  cleared".into());
            }
            Ok(Command::Help) => {
                for line in HELP.lines() {
                    self.history.push(format!("  {}", line));
                }
            }
            Ok(Command::SetTool(kind)) => {
                // Bare drawing keywords (`line`, `circle`, `ci`, `arc`,
                // `ellipse`, `polyline`, `point`) enter the matching draw
                // tool. The user then clicks to place points. Pending
                // points from any prior session are cleared.
                self.tool = match kind {
                    ToolKind::Line       => Tool::Line,
                    ToolKind::Circle     => Tool::Circle,
                    ToolKind::Arc        => Tool::Arc,
                    ToolKind::Ellipse    => Tool::Ellipse,
                    ToolKind::EllipseArc => Tool::EllipseArc,
                    ToolKind::Point      => Tool::Point,
                    ToolKind::Polyline   => Tool::Polyline,
                    ToolKind::Spline     => Tool::Spline,
                };
                self.pending.clear();
                self.set_prompt(current_hint(self.tool, self.arc_method, 0));
            }
            Ok(Command::SnapOverride(kind)) => {
                // PER and TAN need an anchor point — the last clicked point
                // of an in-progress draw. The other snap kinds (END, MID,
                // CEN, INT, NEA) work at any click, with or without a
                // previous pending point.
                if kind.requires_from() && (self.pending.is_empty() || self.tool == Tool::None) {
                    self.history.push(format!(
                        "  ! {} needs an anchor — first click of a draw, then type {}",
                        kind.name(), kind.name().to_lowercase()
                    ));
                } else {
                    self.snap_override = Some(kind);
                    self.history.push(format!(
                        "  ↳ {} armed — hover the target and click",
                        kind.name()
                    ));
                }
            }
            Ok(Command::GripsToggle) => {
                self.env.GrpEnb = !self.env.GrpEnb;
                self.history.push(format!(
                    "  grips {} (GrpEnb)",
                    if self.env.GrpEnb { "ON" } else { "OFF" }
                ));
            }
            Ok(Command::List) => {
                self.begin_selection(SelectMode::ForList);
                self.history.push(
                    "  list — Select dobjects: click to add/toggle, click empty corners for window (L→R inside, R→L crossing), Enter when done (Esc cancels)".into());
                self.history.push(
                    "         Sub-commands: all | before (re-select last) | none | remove | addmode".into());
            }
            Ok(Command::Select) => {
                self.begin_selection(SelectMode::ForSelect);
                self.history.push(
                    "  select — Select dobjects: click to add/toggle, click empty corners for window (L→R inside, R→L crossing), Enter when done (Esc cancels)".into());
                self.history.push(
                    "          Sub-commands: all | before (re-select last) | none | remove | addmode".into());
            }
            // ---- Selection sub-commands (only meaningful while a session
            //      is active; gracefully reject otherwise so a stray `all`
            //      doesn't surprise the user).
            Ok(Command::SelectAll) => {
                if self.select_mode == SelectMode::Off {
                    self.history.push("  ! `all` only works during a select session".into());
                } else {
                    let added = self.add_all_to_selection();
                    self.history.push(format!(
                        "    + {} dobject(s) via 'all' (current: {})",
                        added, self.selection.len()));
                }
            }
            Ok(Command::SelectPrevious) => {
                if self.select_mode == SelectMode::Off {
                    self.history.push("  ! `before` only works during a select session".into());
                } else if self.selection_prev.is_empty() {
                    self.history.push("  ! no previous selection to re-add".into());
                } else {
                    let mut added = 0usize;
                    let prev = self.selection_prev.clone();
                    for i in prev {
                        if i < self.doc.dobjects.len() && !self.selection.contains(&i) {
                            self.selection.push(i);
                            added += 1;
                        }
                    }
                    self.history.push(format!(
                        "    + {} dobject(s) via 'before' (current: {})",
                        added, self.selection.len()));
                }
            }
            Ok(Command::SelectNone) => {
                if self.select_mode == SelectMode::Off {
                    self.history.push("  ! `none` only works during a select session".into());
                } else {
                    let n = self.selection.len();
                    self.selection.clear();
                    self.window_first = None;
                    self.history.push(format!("    – cleared {} selected", n));
                }
            }
            Ok(Command::SelectRemoveMode) => {
                if self.select_mode == SelectMode::Off {
                    self.history.push("  ! `remove` only works during a select session".into());
                } else {
                    self.select_remove_mode = true;
                    self.history.push("    → REMOVE mode (clicks now subtract)".into());
                }
            }
            Ok(Command::SelectAddMode) => {
                if self.select_mode == SelectMode::Off {
                    self.history.push("  ! `addmode` only works during a select session".into());
                } else {
                    self.select_remove_mode = false;
                    self.history.push("    → ADD mode (clicks now add/toggle)".into());
                }
            }
            Ok(Command::SelectWindow) => {
                if self.select_mode == SelectMode::Off {
                    self.history.push("  ! `window` / `w` only works during a select session (run `select` or `trim` first)".into());
                } else {
                    self.window_first = None;
                    self.armed_window_inside = Some(true);   // forced inside
                    self.history.push(
                        "    window armed — click FIRST corner, then OPPOSITE. Only dobjects FULLY INSIDE the box are added.".into());
                }
            }
            Ok(Command::SelectCrossing) => {
                if self.select_mode == SelectMode::Off {
                    self.history.push("  ! `crossing` / `c` only works during a select session".into());
                } else {
                    self.window_first = None;
                    self.armed_window_inside = Some(false);  // forced crossing
                    self.history.push(
                        "    crossing armed — click FIRST corner, then OPPOSITE. Any dobject TOUCHING the box is added.".into());
                }
            }
            Ok(Command::SelectLast) => {
                if self.select_mode == SelectMode::Off {
                    self.history.push("  ! `last` / `l` only works during a select session".into());
                } else if self.doc.dobjects.is_empty() {
                    self.history.push("  ! last: document is empty".into());
                } else {
                    let last = self.doc.dobjects.len() - 1;
                    if !self.selection.contains(&last) {
                        self.selection.push(last);
                        self.history.push(format!(
                            "    + last drawn dobject #{} added (basket: {})",
                            last, self.selection.len()));
                    } else {
                        self.history.push(format!(
                            "    last drawn dobject #{} already in basket", last));
                    }
                }
            }
            Ok(Command::Open(path)) => {
                self.do_open(&path);
            }
            Ok(Command::SaveAs(path)) => {
                self.do_save(&path);
            }
            Ok(Command::Copy) => {
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Copy;
                    self.history.push(
                        "  copy — Select dobjects to copy, Enter to continue (Esc cancels)".into());
                } else {
                    self.copy_state = CopyState::WaitingForBase;
                    self.history.push(format!(
                        "  copy — {} dobject(s) selected. Click BASE point (Esc cancels)",
                        self.selection.len()));
                }
            }
            Ok(Command::Rotate) => {
                self.rotate_copy = false;
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Rotate;
                    self.set_prompt(
                        "rotate: select dobjects, Enter to continue  [Esc=cancel]");
                } else {
                    self.rotate_state = RotateState::WaitingForPivot;
                    self.set_prompt(format!(
                        "rotate ({} dobject(s)): click PIVOT point  [Esc=cancel]",
                        self.selection.len()));
                }
            }
            Ok(Command::Scale) => {
                self.scale_copy = false;
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Scale;
                    self.set_prompt(
                        "scale: select dobjects, Enter to continue  [Esc=cancel]");
                } else {
                    self.scale_state = ScaleState::WaitingForPivot;
                    self.set_prompt(format!(
                        "scale ({} dobject(s)): click PIVOT (base point)  [Esc=cancel]",
                        self.selection.len()));
                }
            }
            Ok(Command::Mirror) => {
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Mirror;
                    self.history.push(
                        "  mirror — Select dobjects, Enter to continue (Esc cancels)".into());
                } else {
                    self.mirror_state = MirrorState::WaitingForA;
                    self.history.push(format!(
                        "  mirror — {} dobject(s) selected. Click FIRST axis point",
                        self.selection.len()));
                }
            }
            Ok(Command::DeleteSelected) => {
                if self.selection.is_empty() {
                    self.history.push("  ! nothing selected — use `select` first or click a dobject".into());
                } else {
                    self.snapshot_doc();
                    // Remove from highest index downward so earlier indices stay valid.
                    let mut sorted = self.selection.clone();
                    sorted.sort_unstable();
                    sorted.dedup();
                    let n = sorted.len();
                    for &idx in sorted.iter().rev() {
                        if idx < self.doc.dobjects.len() {
                            self.doc.dobjects.remove(idx);
                        }
                    }
                    self.selection_prev = self.selection.clone();
                    self.selection.clear();
                    self.selected = None;
                    self.intersections.clear();
                    self.index_dirty = true;
                    self.gpu_dirty = true;
                    self.history.push(format!("  - deleted {} dobject(s)", n));
                }
            }
            Ok(Command::Undo) => self.do_undo(),
            Ok(Command::Redo) => self.do_redo(),
            Ok(Command::MatchProps) => {
                if self.selection.is_empty() {
                    self.history.push(
                        "  ! matchprop: select target dobjects first, then run matchprop, then click source".into());
                } else {
                    self.matchprops_state = MatchPropsState::WaitingForSource;
                    self.history.push(format!(
                        "  matchprop — {} dobject(s) in basket. Click SOURCE dobject (Esc cancels)",
                        self.selection.len()));
                }
            }
            Ok(Command::Reverse)     => self.apply_reverse(),
            Ok(Command::ChangeLayer) => self.apply_chlayer(),
            Ok(Command::Offset(d_opt)) => {
                // Resolve None to the persistent default (env.OfsDis).
                // When the user supplied a distance explicitly, persist
                // it so the next bare `offset` reuses it. Matches the
                // AutoCAD OFFSETDIST behavior.
                let d = match d_opt {
                    Some(v) => {
                        if (self.env.OfsDis - v).abs() > 1e-12 {
                            self.env.OfsDis = v;
                            let _ = self.env.save();
                        }
                        v
                    }
                    None => self.env.OfsDis,
                };
                if self.selection.is_empty() {
                    self.history.push(format!(
                        "  ! offset (d={:.3}): empty basket — `select` first", d));
                } else {
                    self.offset_state = OffsetState::WaitingForSide(d);
                    self.history.push(format!(
                        "  offset — distance {:.3}; {} in basket. Click SIDE to offset toward (Esc cancels)",
                        d, self.selection.len()));
                }
            }
            Ok(Command::Lengthen(d)) => {
                if self.selection.is_empty() {
                    self.history.push("  ! lengthen: empty basket — `select` first".into());
                } else {
                    self.lengthen_state = LengthenState::WaitingForSide(d);
                    self.history.push(format!(
                        "  lengthen — delta {:.3}; {} in basket. Click END to extend (Esc cancels)",
                        d, self.selection.len()));
                }
            }
            Ok(Command::Break) => {
                if self.selection.is_empty() {
                    self.history.push("  ! break: empty basket — `select` first".into());
                } else {
                    self.break_state = BreakState::WaitingForPoint;
                    self.history.push(format!(
                        "  break — {} in basket. Click CUT point (Esc cancels)",
                        self.selection.len()));
                }
            }
            Ok(Command::Align) => {
                if self.selection.is_empty() {
                    self.history.push("  ! align: empty basket — `select` first".into());
                } else {
                    self.align_state = AlignState::WaitingForSrc1;
                    self.history.push(format!(
                        "  align — {} in basket. Click SOURCE point 1 (Esc cancels)",
                        self.selection.len()));
                }
            }
            Ok(Command::Stretch) => {
                self.stretch_state = StretchState::WaitingForWin1;
                self.set_prompt(
                    "stretch: click FIRST corner of crossing window  [Esc=cancel]");
            }
            Ok(Command::Trim) => {
                self.pre_op_selection = std::mem::take(&mut self.selection);
                self.trim_dbg_session_start("TRIM");
                self.trim_state = TrimState::SelectingCutters;
                self.begin_selection(SelectMode::ForCuttingEdges);
                self.set_prompt(
                    "trim: pick CUTTING edges (Enter = all)  [w/c/a/b/l/n  Esc=cancel]");
            }
            Ok(Command::Extend) => {
                self.pre_op_selection = std::mem::take(&mut self.selection);
                self.trim_dbg_session_start("EXTEND");
                self.extend_state = ExtendState::SelectingBoundaries;
                self.begin_selection(SelectMode::ForBoundaryEdges);
                self.set_prompt(
                    "extend: pick BOUNDARY edges (Enter = all)  [w/c/a/b/l/n  Esc=cancel]");
            }
            Ok(Command::Move) => {
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Move;
                    self.set_prompt(
                        "move: select dobjects, Enter to continue  [Esc=cancel]");
                } else {
                    self.move_state = MoveState::WaitingForBase;
                    self.set_prompt(format!(
                        "move ({} dobject(s)): click BASE point  [Esc=cancel]",
                        self.selection.len()));
                }
            }
            Ok(Command::Hatch { pattern, scale, angle_deg }) => {
                // Auto-open the Hatch Debug window so the user sees the
                // live log without having to hunt for the toolbar
                // toggle. Mirrors the trim/extend pattern. Does NOT
                // clear prior entries — accumulating context is useful
                // when comparing successive `hatch` runs.
                self.hatch_dbg_session_start();
                self.hatch_dbg(format!(
                    "Command::Hatch parsed — pattern={:?}, scale={}, angle={}",
                    pattern, scale, angle_deg));
                // No args → open the attributes dialog so the user
                // picks pattern + scale + angle with a live preview
                // BEFORE picking the boundary. With args, run directly
                // (the scriptable / power-user path).
                if pattern.is_none() && (scale - 1.0).abs() < 1e-9
                    && angle_deg.abs() < 1e-9
                {
                    self.hatch_dialog_open = true;
                    self.hatch_dbg(
                        "  no args → opened Choose Hatch Attributes dialog".to_string());
                    self.set_prompt(
                        "hatch: pick pattern + scale + angle in the dialog, then click OK");
                    return;
                }
                self.pending_hatch_pattern = (pattern.clone(), scale, angle_deg);
                self.hatch_dbg(format!(
                    "  pending_hatch_pattern = ({:?}, {}, {})",
                    pattern, scale, angle_deg));
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Hatch;
                    let style = pattern.as_deref().unwrap_or("SOLID");
                    self.hatch_dbg(format!(
                        "  empty selection → ForSelect + QueuedOp::Hatch ({})", style));
                    self.set_prompt(format!(
                        "hatch ({}): pick CLOSED boundary dobject(s), Enter to fill  [Esc=cancel]",
                        style));
                } else {
                    self.hatch_dbg(format!(
                        "  selection has {} dobject(s) → applying immediately",
                        self.selection.len()));
                    self.apply_hatch();
                }
            }
            Ok(Command::Fillet(r_opt)) => {
                if let Some(r) = r_opt {
                    self.env.FltRad = r;
                    let _ = self.env.save();
                }
                let r = self.env.FltRad;
                self.fillet_state = FilletState::WaitingForFirst(r);
                self.fillet_multiple = false;       // each F starts single-mode
                self.refresh_fillet_prompt();
            }
            Ok(Command::Chamfer(opt)) => {
                if let Some((d1, d2_opt)) = opt {
                    let d2 = d2_opt.unwrap_or(d1);
                    self.env.ChmDs1 = d1;
                    self.env.ChmDs2 = d2;
                    let _ = self.env.save();
                }
                let d1 = self.env.ChmDs1;
                let d2 = self.env.ChmDs2;
                self.chamfer_state = ChamferState::WaitingForFirst(d1, d2);
                self.chamfer_multiple = false;
                self.refresh_chamfer_prompt();
            }
            Ok(Command::Join) => {
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Join;
                    self.set_prompt(
                        "join: select dobjects to merge, Enter to apply  [Esc=cancel]");
                } else {
                    self.apply_join();
                }
            }
            Err(e) => self.history.push(format!("  ! {}", e)),
        }
    }

    /// One-stop "wipe everything geometry-related". Called from the toolbar's
    /// "clear all" button AND the typed `clear` command — both used to diverge.
    fn clear_all(&mut self) {
        self.doc.dobjects.clear();
        self.intersections.clear();
        self.pending.clear();
        self.selected      = None;
        self.snap_override = None;
        self.index         = None;
        self.index_dirty   = true;
        self.index_label.clear();
        self.last_intersect_label.clear();
        self.gpu_dirty = true;
    }

    // ---- selection helpers (list / select commands) -------------------

    fn begin_selection(&mut self, mode: SelectMode) {
        // A fresh selection session — abandon any in-progress draw / pick /
        // move. The session starts empty; the user can grow it with clicks,
        // window drags, or the `before` sub-command.
        self.select_mode  = mode;
        self.selection.clear();
        self.window_first = None;
        self.select_remove_mode = false;
        self.tool         = Tool::None;
        self.pending.clear();
        self.picking_source = false;
        self.move_state   = MoveState::Off;
    }

    fn cancel_selection(&mut self) {
        self.select_mode        = SelectMode::Off;
        self.selection.clear();
        self.window_first       = None;
        self.armed_window_inside = None;
        self.select_remove_mode = false;
        let had_queued = self.queued_op != QueuedOp::None;
        self.queued_op          = QueuedOp::None;
        self.history.push(
            if had_queued { "  selection cancelled — pending operation aborted".into() }
            else { "  selection cancelled".into() });
    }

    fn finalise_selection(&mut self) {
        match self.select_mode {
            SelectMode::Off => return,
            SelectMode::ForList => {
                self.history.push(format!(
                    "  list — {} dobject(s) selected:", self.selection.len()));
                for &i in &self.selection {
                    if let Some(d) = self.doc.dobjects.get(i) {
                        self.history.push(format!("      #{:>5}  {}", i, describe(&d.geom)));
                    }
                }
            }
            SelectMode::ForSelect => {
                self.history.push(format!(
                    "  select — {} dobject(s) kept as the active selection",
                    self.selection.len()));
            }
            SelectMode::ForCuttingEdges => {
                let cutters = std::mem::take(&mut self.selection);
                // Restore the user's main selection — trim must not nuke it.
                self.selection = std::mem::take(&mut self.pre_op_selection);
                self.select_mode        = SelectMode::Off;
                self.window_first       = None;
                self.select_remove_mode = false;
                // Empty cutter basket = "use every current dobject as a
                // cutter, recomputed each click". This is AutoCAD's default
                // ("press Enter to select all") AND it's the only way pieces
                // created by THIS session's trims keep acting as cutters.
                // See memos `feedback_rust_cad_trim_default_all_cutters` +
                // `feedback_rust_cad_trim_breaks_into_all_segments`.
                if cutters.is_empty() {
                    if self.doc.dobjects.is_empty() {
                        self.history.push("  ! trim: document is empty — cancelled".into());
                        self.trim_dbg("CUTTERS = []  (empty doc → session cancelled)");
                        self.trim_state = TrimState::Off;
                        return;
                    }
                    self.history.push(format!(
                        "  trim — no cutters picked; using ALL dobjects (dynamic) as cutters"));
                    self.trim_dbg(format!(
                        "CUTTERS = ALL (dynamic; doc currently has {} dobjects, recomputed each click)",
                        self.doc.dobjects.len()));
                    self.history.push(
                        "  trim — every dobject is a cutter (warm orange). Click each TARGET to cut. Enter / Esc to finish.".into());
                    self.trim_state = TrimState::PickingTargetsAll;
                    return;
                }
                self.trim_dbg(format!(
                    "CUTTERS captured = {} indices: {:?}", cutters.len(), cutters));
                self.history.push(format!(
                    "  trim — {} cutter(s) ready (warm orange). Click each TARGET to cut. Enter / Esc to finish.",
                    cutters.len()));
                self.trim_state = TrimState::PickingTargets(cutters);
                return;
            }
            SelectMode::ForBoundaryEdges => {
                let bounds = std::mem::take(&mut self.selection);
                self.selection = std::mem::take(&mut self.pre_op_selection);
                self.select_mode        = SelectMode::Off;
                self.window_first       = None;
                self.select_remove_mode = false;
                if bounds.is_empty() {
                    if self.doc.dobjects.is_empty() {
                        self.history.push("  ! extend: document is empty — cancelled".into());
                        self.trim_dbg("BOUNDARIES = []  (empty doc → session cancelled)");
                        self.extend_state = ExtendState::Off;
                        return;
                    }
                    self.history.push(format!(
                        "  extend — no boundaries picked; using ALL dobjects (dynamic) as boundaries"));
                    self.trim_dbg(format!(
                        "BOUNDARIES = ALL (dynamic; doc currently has {} dobjects, recomputed each click)",
                        self.doc.dobjects.len()));
                    self.history.push(
                        "  extend — every dobject is a boundary (warm amber). Click each TARGET END. Enter / Esc to finish.".into());
                    self.extend_state = ExtendState::PickingTargetsAll;
                    return;
                }
                self.trim_dbg(format!(
                    "BOUNDARIES captured = {} indices: {:?}", bounds.len(), bounds));
                self.history.push(format!(
                    "  extend — {} boundary edge(s) ready (warm amber). Click each TARGET END to extend. Enter / Esc to finish.",
                    bounds.len()));
                self.extend_state = ExtendState::PickingTargets(bounds);
                return;
            }
        }
        self.select_mode        = SelectMode::Off;
        self.window_first       = None;
        self.select_remove_mode = false;
        // Snapshot for `before`. Only update when the finalised set is
        // non-empty — pressing Enter on an empty set shouldn't wipe the
        // previous memory.
        if !self.selection.is_empty() {
            self.selection_prev = self.selection.clone();
        }

        // Dispatch any operation that was queued behind this selection
        // (e.g. `move` opened the session; finalising it transitions
        // straight to base-point capture).
        let queued = std::mem::replace(&mut self.queued_op, QueuedOp::None);
        if queued != QueuedOp::None && self.selection.is_empty() {
            self.history.push(format!(
                "  ! {:?}: nothing selected — operation cancelled", queued
            ));
            return;
        }
        match queued {
            QueuedOp::None => {}
            QueuedOp::Move => {
                self.move_state = MoveState::WaitingForBase;
                self.set_prompt(format!(
                    "move ({} dobject(s)): click BASE point  [Esc=cancel]",
                    self.selection.len()));
            }
            QueuedOp::Copy => {
                self.copy_state = CopyState::WaitingForBase;
                self.set_prompt(format!(
                    "copy ({} dobject(s)): click BASE point  [Esc=cancel]",
                    self.selection.len()));
            }
            QueuedOp::Rotate => {
                self.rotate_state = RotateState::WaitingForPivot;
                self.set_prompt(format!(
                    "rotate ({} dobject(s)): click PIVOT  [Esc=cancel]",
                    self.selection.len()));
            }
            QueuedOp::Scale => {
                self.scale_state = ScaleState::WaitingForPivot;
                self.set_prompt(format!(
                    "scale ({} dobject(s)): click PIVOT  [Esc=cancel]",
                    self.selection.len()));
            }
            QueuedOp::Mirror => {
                self.mirror_state = MirrorState::WaitingForA;
                self.set_prompt(format!(
                    "mirror ({} dobject(s)): click FIRST axis point  [Esc=cancel]",
                    self.selection.len()));
            }
            QueuedOp::Join => {
                self.apply_join();
            }
            QueuedOp::Hatch => {
                self.apply_hatch();
            }
            QueuedOp::Array => {
                // Selection basket now holds the array source(s). Re-show
                // the array dialog so the user can set rows/cols/dx/dy
                // and click Generate. We don't drain the basket — the
                // dialog uses `self.selection` directly.
                self.array_open = true;
                self.clear_prompt();
                self.history.push(format!(
                    "  array: {} source dobject(s) picked — set rows/cols and Generate",
                    self.selection.len()));
            }
        }
        // self.selection persists so follow-up commands (move, list, …) can use it.
    }

    /// Resolve a hatch's boundary handles into a list of vertex loops
    /// in world coords. Each closed polyline boundary contributes one
    /// loop; bulges are tessellated to short chord segments so curved
    /// boundaries fill smoothly. Handles that no longer resolve (the
    /// user deleted a boundary) are silently skipped, so the hatch
    /// just shrinks rather than crashing.
    fn resolve_hatch_loops(&self, h: &cad_kernel::Hatch) -> Vec<Vec<Vec2>> {
        let mut loops: Vec<Vec<Vec2>> = Vec::with_capacity(h.boundary_handles.len());
        for handle in &h.boundary_handles {
            let Some(d) = self.doc.find_by_handle(*handle) else { continue; };
            // Any closed boundary dobject is fair game now —
            // Polyline (closed), Circle, Ellipse. Open dobjects and
            // arcs would need boundary-traversal (chain into a loop);
            // queued for the pick-point slice.
            let loop_verts: Option<Vec<Vec2>> = match &d.geom {
                Geom::Polyline(p) if polyline_is_effectively_closed(p) => {
                    Some(closed_dobject_polygon(&d.geom))
                }
                Geom::Circle(c) => {
                    Some(tessellate_circle_loop(c.center, c.radius, 64))
                }
                Geom::Ellipse(e) => {
                    Some(tessellate_ellipse_loop(e, 64))
                }
                _ => None,
            };
            if let Some(v) = loop_verts {
                loops.push(v);
            }
        }
        loops
    }

    /// Paint a hatch — solid fill of its resolved boundary loops with
    /// the AutoCAD-style even-odd rule (outer loop fills, next is a
    /// hole, then a hole-in-hole, etc.). For multi-loop fills we
    /// triangulate ourselves via ear-clipping with the loops cut into
    /// a single ring; for one loop we just hand to egui's path
    /// tessellator. Pattern variants (parallel lines, ANSI / ISO) will
    /// dispatch on `h.pattern` here later.
    fn render_hatch_fill(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        h: &cad_kernel::Hatch,
        color: egui::Color32,
    ) {
        let loops = self.resolve_hatch_loops(h);
        if loops.is_empty() { return; }
        match &h.pattern {
            cad_kernel::HatchPattern::Solid =>
                self.render_hatch_solid(painter, rect, &loops, color),
            cad_kernel::HatchPattern::Pattern { name, scale, angle_deg } =>
                self.render_hatch_pattern(
                    painter, rect, &loops, color, name, *scale, *angle_deg),
        }
    }

    /// Solid fill path — outer loop fills, alternating loops "subtract"
    /// by overdrawing in the canvas background color (poor man's even-
    /// odd; real tessellator pass is a follow-up).
    fn render_hatch_solid(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        loops: &[Vec<Vec2>],
        color: egui::Color32,
    ) {
        if loops.len() == 1 {
            let pts: Vec<egui::Pos2> = loops[0].iter()
                .map(|w| self.w2s(*w, rect)).collect();
            if pts.len() < 3 { return; }
            painter.add(egui::Shape::Path(egui::epaint::PathShape {
                points: pts, closed: true, fill: color,
                stroke: egui::epaint::PathStroke::NONE,
            }));
            return;
        }
        let bg = egui::Color32::from_rgb(18, 22, 28);
        for (i, l) in loops.iter().enumerate() {
            let pts: Vec<egui::Pos2> = l.iter()
                .map(|w| self.w2s(*w, rect)).collect();
            if pts.len() < 3 { continue; }
            let fill = if i % 2 == 0 { color } else { bg };
            painter.add(egui::Shape::Path(egui::epaint::PathShape {
                points: pts, closed: true, fill,
                stroke: egui::epaint::PathStroke::NONE,
            }));
        }
    }

    /// Named-pattern fill — for each line family in the catalog entry,
    /// generate parallel lines covering the boundary bbox, clip each
    /// against ALL loops via even-odd along the line, draw the
    /// surviving segments. Unknown pattern name → no lines drawn.
    fn render_hatch_pattern(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        loops: &[Vec<Vec2>],
        color: egui::Color32,
        name: &str,
        user_scale: f64,
        user_angle_deg: f64,
    ) {
        let families = cad_kernel::patterns::lookup(name);
        if families.is_empty() { return; }
        // Union bbox of all loops in world coords.
        let mut min = Vec2::new(f64::INFINITY, f64::INFINITY);
        let mut max = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for l in loops {
            for v in l {
                if v.x < min.x { min.x = v.x; }
                if v.y < min.y { min.y = v.y; }
                if v.x > max.x { max.x = v.x; }
                if v.y > max.y { max.y = v.y; }
            }
        }
        if !min.x.is_finite() || !max.x.is_finite() { return; }
        let user_angle = user_angle_deg.to_radians();
        let stroke = egui::Stroke::new(0.9, color);
        for fam in &families {
            // Effective angle + spacing after user transform.
            let theta = fam.angle + user_angle;
            let spacing = fam.spacing * user_scale.abs().max(1e-9);
            // Line direction u, normal n (CCW perp of u).
            let cos = theta.cos();
            let sin = theta.sin();
            let u = Vec2::new(cos, sin);
            let n = Vec2::new(-sin, cos);
            // Project bbox corners onto n to find the range of
            // s-values (offset along n) that the pattern needs to
            // cover. The bbox of a rotated axis-aligned rect is
            // bounded by the projection of its 4 corners.
            let corners = [
                Vec2::new(min.x, min.y), Vec2::new(max.x, min.y),
                Vec2::new(min.x, max.y), Vec2::new(max.x, max.y),
            ];
            let base = Vec2::new(fam.base_x, fam.base_y);
            let mut s_min = f64::INFINITY;
            let mut s_max = f64::NEG_INFINITY;
            for c in &corners {
                let s = (*c - base).dot(n);
                if s < s_min { s_min = s; }
                if s > s_max { s_max = s; }
            }
            // First line at s = ceil(s_min / spacing) * spacing.
            let mut s = (s_min / spacing).ceil() * spacing;
            // Safety cap: spacing too small for the world bbox would
            // generate millions of lines and freeze. Bail out if the
            // family would produce > 10 000 lines for this hatch.
            let line_count_estimate = ((s_max - s_min) / spacing).ceil();
            if line_count_estimate > 10_000.0 { continue; }
            while s <= s_max + 1e-9 {
                let line_origin = base + n * s;
                // Clip this infinite line against the loops — gather
                // every (t-value, edge-orientation) intersection, sort,
                // emit segments via even-odd.
                let mut hits: Vec<f64> = Vec::new();
                for l in loops {
                    let m = l.len();
                    if m < 2 { continue; }
                    for i in 0..m {
                        let a = l[i];
                        let b = l[(i + 1) % m];
                        if let Some(t) = line_segment_intersect_t(line_origin, u, a, b) {
                            hits.push(t);
                        }
                    }
                }
                if hits.len() >= 2 {
                    hits.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
                    // Pair consecutive hits — even-odd inside/outside.
                    let mut i = 0;
                    while i + 1 < hits.len() {
                        let t0 = hits[i];
                        let t1 = hits[i + 1];
                        if (t1 - t0).abs() > 1e-6 {
                            let p0 = line_origin + u * t0;
                            let p1 = line_origin + u * t1;
                            painter.line_segment(
                                [self.w2s(p0, rect), self.w2s(p1, rect)],
                                stroke);
                        }
                        i += 2;
                    }
                }
                s += spacing;
            }
        }
    }

    /// Pick-point boundary finder — returns the INDEX in
    /// `self.doc.dobjects` of the smallest closed dobject whose
    /// interior contains `world`. Used by the hatch dialog's "Pick
    /// Point" button. "Smallest" is measured by bbox area (a cheap
    /// proxy for actual enclosed area — fine for nested loops; falls
    /// down when two disjoint loops have similar bboxes but the
    /// click is in only one of them, which the point-in test filters
    /// out anyway).
    ///
    /// Closed dobject types considered today:
    ///   * Closed Polyline — even-odd ray cast on the vertex polygon
    ///   * Circle          — distance from centre < radius
    ///   * Ellipse         — local coords + (x/a)² + (y/b)² < 1
    ///
    /// Open chains (line + arc loops) and Splines are skipped — they
    /// need boundary-traversal to chain into a loop, which is the
    /// pick-point v2b slice's job.
    /// Same containment test as `find_smallest_containing_closed`, but
    /// returns ALL containing closed dobjects with their bbox area + kind
    /// label — useful for the hatch-debug log so the user can see every
    /// candidate the cheap path considered, not just the winner.
    fn collect_closed_containing(&self, world: Vec2) -> Vec<(usize, f64, &'static str)> {
        self.collect_closed_containing_scoped(world, None)
    }

    /// Scoped variant — when `scope` is `Some`, only iterates those
    /// indices (typically the viewport_scope set). Lets hatch avoid
    /// scanning all 400k+ dobjects when only ~50 are near the click.
    fn collect_closed_containing_scoped(
        &self,
        world: Vec2,
        scope: Option<&[usize]>,
    ) -> Vec<(usize, f64, &'static str)> {
        let mut out = Vec::new();
        let iter: Box<dyn Iterator<Item = (usize, &DObject)>> = match scope {
            Some(s) => Box::new(s.iter().filter_map(|&i| {
                self.doc.dobjects.get(i).map(|d| (i, d))
            })),
            None => Box::new(self.doc.dobjects.iter().enumerate()),
        };
        for (i, d) in iter {
            let contains = match &d.geom {
                // Treat polylines with coincident endpoints as closed too
                // (the "drew it as a loop but forgot to type c Enter" case).
                Geom::Polyline(p) if polyline_is_effectively_closed(p) => {
                    let verts = closed_dobject_polygon(&d.geom);
                    point_in_polygon(world, verts)
                }
                Geom::Circle(c) => (world - c.center).len() < c.radius,
                Geom::Ellipse(e) => {
                    let dvec = world - e.center;
                    let a = e.semi_major().max(1e-12);
                    let b = e.semi_minor().max(1e-12);
                    let u = dvec.dot(e.u_hat()) / a;
                    let v = dvec.dot(e.v_hat()) / b;
                    u * u + v * v < 1.0
                }
                _ => false,
            };
            if !contains { continue; }
            let (bmin, bmax) = d.geom.bbox();
            let area = (bmax.x - bmin.x).abs() * (bmax.y - bmin.y).abs();
            out.push((i, area, dobject_kind_name(&d.geom)));
        }
        out
    }

    /// Indices of dobjects whose bbox overlaps the current viewport
    /// world-bbox (`last_visible`). Uses the spatial index when it's
    /// fresh — O(visible cells), typically a few dozen entries — and
    /// falls back to the full N range when the index is stale or
    /// missing. Returns `None` only when no viewport has ever been
    /// rendered (first frame).
    ///
    /// THIS IS THE FIX for the 400k-dobjects perf glitch: every hatch
    /// helper (cheap-path candidate scan, island scan, verbose dump,
    /// trace tessellation) now restricts its iteration to this set
    /// instead of the full document. A click in a 100-dobject
    /// neighbourhood of a 400k-dobject drawing iterates ~100, not 400k.
    fn viewport_scope(&self) -> Option<Vec<usize>> {
        let (vmin, vmax) = self.last_visible?;
        let scope: Vec<usize> = if let (Some(g), false) =
            (self.index.as_ref(), self.index_dirty)
        {
            g.query_bbox(vmin, vmax).into_iter().map(|u| u as usize).collect()
        } else {
            (0..self.doc.dobjects.len()).collect()
        };
        Some(scope)
    }

    /// True if the dobject at `outer_idx` has its boundary crossed by
    /// any other visible dobject — which means the cheap path's "hatch
    /// the whole outer + auto islands" answer is WRONG for partial
    /// overlaps. In that case we route to the trace path so the
    /// planar-subdivision face containing the seed gets hatched
    /// instead of the whole outer.
    ///
    /// Detection reuses the trace path's tessellator + seg-seg
    /// intersection helper — same primitive that v2c splits on, just
    /// asked as a yes/no question without actually splitting.
    fn outer_has_crossings_with_others(&self, outer_idx: usize) -> bool {
        let segs = crate::hatch_trace::tessellate_doc(&self.doc);
        let outer_segs: Vec<usize> = segs.iter().enumerate()
            .filter(|(_, s)| s.src == outer_idx)
            .map(|(i, _)| i)
            .collect();
        if outer_segs.is_empty() { return false; }
        for &i in &outer_segs {
            for other in segs.iter() {
                if other.src == outer_idx { continue; }
                // Match split_at_intersections' tolerance — endpoint
                // touches don't count, only interior crossings do.
                if let Some((ti, tj, _)) = crate::hatch_trace::seg_seg_intersect_params(
                    &segs[i], other)
                {
                    if ti > 1e-6 && ti < 1.0 - 1e-6
                       && tj > 1e-6 && tj < 1.0 - 1e-6 {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Verdict-per-dobject log for the island scan: shows, for each
    /// other dobject in the doc, why it was or wasn't accepted as an
    /// island inside `outer_idx`. Returns lines for the caller to push
    /// into `hatch_dbg`. The actual island list is built by
    /// `collect_islands_inside` — this is pure instrumentation.
    fn dbg_island_scan_verdicts(&self, outer_idx: usize, seed: Vec2) -> Vec<String> {
        let mut out = Vec::new();
        let Some(outer_d) = self.doc.dobjects.get(outer_idx) else { return out; };
        let outer_polygon = closed_dobject_polygon(&outer_d.geom);
        if outer_polygon.len() < 3 {
            out.push(format!("    outer #{} has no polygon — cannot scan", outer_idx));
            return out;
        }
        let (omin, omax) = outer_d.geom.bbox();
        for (i, d) in self.doc.dobjects.iter().enumerate() {
            if i == outer_idx { continue; }
            let kind = dobject_kind_name(&d.geom);
            if !d.style.visible {
                out.push(format!("    #{:02} {} — skip (hidden)", i, kind));
                continue;
            }
            let is_closed_type = match &d.geom {
                Geom::Polyline(p) => polyline_is_effectively_closed(p),
                Geom::Circle(_) | Geom::Ellipse(_) => true,
                _ => false,
            };
            if !is_closed_type {
                out.push(format!("    #{:02} {} — skip (not a closed boundary type)", i, kind));
                continue;
            }
            let (bmin, bmax) = d.geom.bbox();
            let bbox_inside = bmin.x >= omin.x - 1e-9 && bmin.y >= omin.y - 1e-9
                            && bmax.x <= omax.x + 1e-9 && bmax.y <= omax.y + 1e-9;
            if !bbox_inside {
                out.push(format!(
                    "    #{:02} {} — skip (bbox ({:.2},{:.2})→({:.2},{:.2}) NOT fully inside outer bbox)",
                    i, kind, bmin.x, bmin.y, bmax.x, bmax.y));
                continue;
            }
            let cand_poly = closed_dobject_polygon(&d.geom);
            if cand_poly.len() < 3 {
                out.push(format!("    #{:02} {} — skip (degenerate polygon)", i, kind));
                continue;
            }
            if point_in_polygon(seed, cand_poly.clone()) {
                out.push(format!(
                    "    #{:02} {} — skip (CONTAINS SEED — would be outer, not island)",
                    i, kind));
                continue;
            }
            let n = cand_poly.len();
            let samples = [
                cand_poly[0],
                cand_poly[n / 5],
                cand_poly[(2 * n) / 5],
                cand_poly[(3 * n) / 5],
                cand_poly[(4 * n) / 5],
            ];
            let inside_count = samples.iter()
                .filter(|p| point_in_polygon(**p, outer_polygon.clone()))
                .count();
            if inside_count == samples.len() {
                out.push(format!(
                    "    #{:02} {} — ISLAND ACCEPTED (bbox inside, {}/{} boundary samples inside outer, seed-not-inside)",
                    i, kind, inside_count, samples.len()));
            } else {
                out.push(format!(
                    "    #{:02} {} — skip (only {}/{} boundary samples inside outer — partial overlap, not nested)",
                    i, kind, inside_count, samples.len()));
            }
        }
        out
    }

    /// After cheap path picks an OUTER, auto-detect any other closed
    /// dobjects whose bbox is fully inside the outer's bbox AND whose
    /// boundary samples are all inside the outer's polygon — those are
    /// islands. Matches AutoCAD BPOLY's "scan for nested shapes" pass.
    /// Used only by the cheap path; the trace path discovers islands
    /// via its own ray-cast analysis.
    fn collect_islands_inside(&self, outer_idx: usize, seed: Vec2) -> Vec<usize> {
        self.collect_islands_inside_scoped(outer_idx, seed, None)
    }

    /// Scoped variant — see `collect_closed_containing_scoped`.
    fn collect_islands_inside_scoped(
        &self,
        outer_idx: usize,
        seed: Vec2,
        scope: Option<&[usize]>,
    ) -> Vec<usize> {
        let Some(outer_d) = self.doc.dobjects.get(outer_idx) else { return Vec::new(); };
        let outer_polygon = closed_dobject_polygon(&outer_d.geom);
        if outer_polygon.len() < 3 { return Vec::new(); }
        let (omin, omax) = outer_d.geom.bbox();
        let mut islands = Vec::new();
        let iter: Box<dyn Iterator<Item = (usize, &DObject)>> = match scope {
            Some(s) => Box::new(s.iter().filter_map(|&i| {
                self.doc.dobjects.get(i).map(|d| (i, d))
            })),
            None => Box::new(self.doc.dobjects.iter().enumerate()),
        };
        for (i, d) in iter {
            if i == outer_idx { continue; }
            if !d.style.visible { continue; }
            // Must be a self-closed boundary type. Effectively-closed
            // open polylines (endpoint gap < ε) are accepted too.
            let is_candidate = match &d.geom {
                Geom::Polyline(p) => polyline_is_effectively_closed(p),
                Geom::Circle(_) | Geom::Ellipse(_) => true,
                _ => false,
            };
            if !is_candidate { continue; }
            // bbox-inside test (cheap reject) — must be FULLY inside outer's bbox
            let (bmin, bmax) = d.geom.bbox();
            if !(bmin.x >= omin.x - 1e-9 && bmin.y >= omin.y - 1e-9
                 && bmax.x <= omax.x + 1e-9 && bmax.y <= omax.y + 1e-9) { continue; }
            let cand_poly = closed_dobject_polygon(&d.geom);
            if cand_poly.len() < 3 { continue; }
            // If candidate contains the seed it's NOT an island — that
            // would have made it the outer instead (smaller bbox area).
            if point_in_polygon(seed, cand_poly.clone()) { continue; }
            // Sample test: every k-th vertex of candidate must be inside
            // outer's polygon. Five evenly-spaced samples is enough to
            // catch the common "small circle inside big circle" case
            // without paying full polygon-in-polygon containment.
            let n = cand_poly.len();
            let samples = [
                cand_poly[0],
                cand_poly[n / 5],
                cand_poly[(2 * n) / 5],
                cand_poly[(3 * n) / 5],
                cand_poly[(4 * n) / 5],
            ];
            if samples.iter().all(|p| point_in_polygon(*p, outer_polygon.clone())) {
                islands.push(i);
            }
        }
        islands
    }

    fn find_smallest_containing_closed(&self, world: Vec2) -> Option<usize> {
        self.find_smallest_containing_closed_scoped(world, None)
    }

    /// Scoped variant — see `collect_closed_containing_scoped`.
    fn find_smallest_containing_closed_scoped(
        &self,
        world: Vec2,
        scope: Option<&[usize]>,
    ) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        let iter: Box<dyn Iterator<Item = (usize, &DObject)>> = match scope {
            Some(s) => Box::new(s.iter().filter_map(|&i| {
                self.doc.dobjects.get(i).map(|d| (i, d))
            })),
            None => Box::new(self.doc.dobjects.iter().enumerate()),
        };
        for (i, d) in iter {
            let contains = match &d.geom {
                Geom::Polyline(p) if polyline_is_effectively_closed(p) => {
                    // Use the same tessellated polygon the renderer
                    // would resolve, so PIP and render agree on the
                    // shape (bulges + effective-closure both honoured).
                    let verts = closed_dobject_polygon(&d.geom);
                    point_in_polygon(world, verts)
                }
                Geom::Circle(c) => {
                    (world - c.center).len() < c.radius
                }
                Geom::Ellipse(e) => {
                    // Convert world point to ellipse-local coords:
                    // project onto u_hat (major direction) + v_hat
                    // (minor), divide by semi-axes, test sum of
                    // squares against 1.
                    let d = world - e.center;
                    let a = e.semi_major().max(1e-12);
                    let b = e.semi_minor().max(1e-12);
                    let u = d.dot(e.u_hat()) / a;
                    let v = d.dot(e.v_hat()) / b;
                    u * u + v * v < 1.0
                }
                _ => false,
            };
            if !contains { continue; }
            let (bmin, bmax) = d.geom.bbox();
            let area = (bmax.x - bmin.x).abs() * (bmax.y - bmin.y).abs();
            if best.map_or(true, |(_, ba)| area < ba) {
                best = Some((i, area));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Smart hatch finaliser: collect handles of every CLOSED polyline
    /// in the current selection and add ONE Hatch dobject that
    /// REFERENCES all of them as boundary loops (outer first, holes
    /// next via even-odd at render). Boundary dobjects stay where
    /// they are — moving / editing them later auto-updates the hatch
    /// because the render path resolves handles each frame.
    ///
    /// Non-closed-polyline selections are silently skipped with a
    /// message. Multi-loop selection = one hatch with islands; single
    /// loop = solid disc.
    fn apply_hatch(&mut self) {
        self.hatch_dbg(format!(
            "apply_hatch() entry — selection.len() = {}, idx = {:?}",
            self.selection.len(), self.selection));
        // Verbose dump of every selected dobject so the user can
        // verify the algorithm's view matches what's visible on the
        // canvas. Especially useful for polylines where `closed=false`
        // with a near-zero endpoint gap looks identical to the closed
        // form when rendered.
        for &idx in &self.selection.clone() {
            if let Some(d) = self.doc.dobjects.get(idx) {
                self.hatch_dbg(format!(
                    "    selected #{:02}: {}", idx, describe_verbose(&d.geom)));
            } else {
                self.hatch_dbg(format!("    selected #{:02}: <out of range>", idx));
            }
        }
        let mut handles: Vec<cad_kernel::Handle> = Vec::new();
        let mut skipped = 0_usize;
        // Capture per-idx decisions first WITHOUT borrowing self mutably,
        // then push the log lines + handles after — avoids the
        // hatch_dbg-vs-self.selection borrow conflict.
        let mut decisions: Vec<(usize, &'static str, Option<u64>, Option<usize>)> = Vec::new();
        for &idx in &self.selection {
            let Some(d) = self.doc.dobjects.get(idx) else { continue; };
            let kind = dobject_kind_name(&d.geom);
            match &d.geom {
                Geom::Polyline(p) if polyline_is_effectively_closed(p) => {
                    decisions.push((idx, kind, Some(d.handle), Some(p.vertices.len())));
                }
                Geom::Circle(_) | Geom::Ellipse(_) => {
                    decisions.push((idx, kind, Some(d.handle), None));
                }
                _ => decisions.push((idx, kind, None, None)),
            }
        }
        for (idx, kind, h_opt, verts_opt) in decisions {
            if let Some(h) = h_opt {
                handles.push(h);
                match verts_opt {
                    Some(nv) => self.hatch_dbg(format!(
                        "  accept #{} ({}, closed, {} verts)", idx, kind, nv)),
                    None => self.hatch_dbg(format!("  accept #{} ({})", idx, kind)),
                }
            } else {
                skipped += 1;
                self.hatch_dbg(format!("  skip   #{} ({}) — not a closed boundary", idx, kind));
            }
        }
        if handles.is_empty() {
            self.hatch_dbg("  → 0 boundaries accepted; aborting".to_string());
            self.history.push("  ! hatch: no closed boundary in selection".into());
            return;
        }
        self.snapshot_doc();
        let loop_count = handles.len();
        // Build pattern from the args the user provided to `hatch`.
        // None → Solid; Some(name) → Pattern { name, scale, angle }.
        // Reset to defaults after consuming so the next bare `hatch`
        // doesn't reuse a stale pattern.
        let (pat_name, pat_scale, pat_angle) =
            std::mem::replace(&mut self.pending_hatch_pattern, (None, 1.0, 0.0));
        let pattern = match pat_name {
            None       => cad_kernel::HatchPattern::Solid,
            Some(name) => cad_kernel::HatchPattern::Pattern {
                name:      name.to_ascii_uppercase(),
                scale:     pat_scale,
                angle_deg: pat_angle,
            },
        };
        let pattern_label = match &pattern {
            cad_kernel::HatchPattern::Solid => "SOLID".to_string(),
            cad_kernel::HatchPattern::Pattern { name, .. } => name.clone(),
        };
        self.hatch_dbg(format!(
            "  pushing Hatch dobject: pattern={}, scale={:.3}, angle={:.2}, {} boundary handle(s)",
            pattern_label, pat_scale, pat_angle, loop_count));
        self.doc.push(cad_kernel::Hatch {
            boundary_handles: handles,
            pattern,
        }.into());
        self.gpu_dirty = true;
        self.index_dirty = true;
        self.history.push(format!(
            "  + hatch ({}): 1 fill, {} boundary loop(s){}",
            pattern_label, loop_count,
            if skipped > 0 {
                format!("  ({} non-closed-polyline dobject(s) skipped)", skipped)
            } else { String::new() },
        ));
        // Auto-dump after each apply so the log captures bbox + line-
        // count diagnostics for the new hatch without making the user
        // press the button. Only logs when the debug window is open
        // (no-op otherwise via hatch_dbg).
        self.dump_hatch_state();
    }

    /// Pick-point hatch: BPOLY-style flow. First tries the cheap
    /// "self-closed dobject containing the seed" path; if that fails,
    /// runs the full trace pipeline (tessellate doc + endpoint graph
    /// + horizontal ray cast + CCW-turn loop walk + island classify)
    /// to discover a closed boundary surrounding the seed even when
    /// it's formed by a chain of separate line/arc/polyline dobjects.
    ///
    /// On trace success, materialises every loop (outer + islands) as
    /// a new closed Polyline dobject on the current layer, then pushes
    /// a Hatch referencing those new dobjects' handles. Materialised
    /// polylines are normal dobjects — the user can grip-edit them
    /// later (the hatch updates because it's handle-referenced).
    /// Spawn the trace pipeline on a worker thread. Mid-op Esc fires
    /// because the worker reads `op_cancel` between phases (and inside
    /// the O(N²) split scan) while the UI thread keeps spinning;
    /// `poll_hatch_worker` drains the result the frame after the
    /// worker finishes.
    ///
    /// Replaces any existing in-flight worker (cancel old + spawn
    /// new). The previous worker's result is silently discarded — its
    /// receiver gets dropped.
    fn spawn_hatch_worker(&mut self, seed: Vec2) {
        let scope = self.viewport_scope()
            .unwrap_or_else(|| (0..self.doc.dobjects.len()).collect());
        self.spawn_hatch_worker_scoped(seed, scope);
    }

    fn spawn_hatch_worker_scoped(&mut self, seed: Vec2, scope: Vec<usize>) {
        // If a worker is already running, cancel it. The old thread
        // will exit at its next cancel-check; its send will fail
        // harmlessly because we drop the receiver here.
        if self.hatch_worker.is_some() {
            self.op_cancel.store(true, Ordering::Relaxed);
            self.hatch_worker = None;
            self.hatch_dbg("  (cancelled previous in-flight hatch worker)");
        }
        // Build the pattern args from the current pick-point session
        // snapshot (or fall back to SOLID if no session — shouldn't
        // happen via Pick Point button, but defensive).
        let (pat_name, pat_scale, pat_angle) = self.hatch_pick_point_session
            .clone()
            .unwrap_or_else(|| self.pending_hatch_pattern.clone());
        let pattern = match pat_name {
            None       => cad_kernel::HatchPattern::Solid,
            Some(name) => cad_kernel::HatchPattern::Pattern {
                name:      name.to_ascii_uppercase(),
                scale:     pat_scale,
                angle_deg: pat_angle,
            },
        };
        let active_layer = self.doc.layers.active;
        // Fresh cancel flag for this worker — old flag may have just
        // been set to cancel the previous worker. Replace with new.
        let cancel = StdArc::new(AtomicBool::new(false));
        self.op_cancel = cancel.clone();
        // Snapshot only the scoped dobjects (~tens to a few thousand),
        // not the whole Document. At 9M dobjects, `self.doc.clone()`
        // is ~1 GB of memcpy — multi-second freeze per pick-point click.
        // The worker's tessellation only ever touches `scope`, so the
        // dobjects outside scope are dead weight in the snapshot.
        //
        // The new doc carries the small ancillary tables (layers,
        // linetypes, pens, truecolors) verbatim — they're cheap and
        // the worker may resolve colors / styles through them.
        // `scope_for_thread` is remapped to `[0..scoped_n)` since the
        // dobjects are now at fresh contiguous indices in the snapshot.
        let mut doc_snapshot = cad_kernel::Document::default();
        doc_snapshot.layers     = self.doc.layers.clone();
        doc_snapshot.linetypes  = self.doc.linetypes.clone();
        doc_snapshot.pens       = self.doc.pens.clone();
        doc_snapshot.truecolors = self.doc.truecolors.clone();
        doc_snapshot.dobjects   = scope.iter()
            .filter_map(|&i| self.doc.dobjects.get(i).cloned())
            .collect();
        let scoped_n = doc_snapshot.dobjects.len();
        let cancel_for_thread = cancel.clone();
        let (tx, rx) = mpsc::channel::<HatchWorkerResult>();
        // The worker's scope is now the full range of the snapshot
        // (every dobject in the snapshot WAS in the original scope).
        // No remapping needed downstream — poll_hatch_worker only
        // consumes the trace's vertex loops, not the src indices.
        let scope_for_thread: Vec<usize> = (0..scoped_n).collect();
        thread::spawn(move || {
            let mut log: Vec<String> = Vec::new();
            log.push(format!(
                "  worker started: seed=({:.3},{:.3}), {} dobjects in viewport scope ({} total)",
                seed.x, seed.y, scope_for_thread.len(), doc_snapshot.dobjects.len()));
            // Single end-to-end call — tessellates only viewport dobjects,
            // splits at intersections, traces. Cancellable between phases.
            let tb = crate::hatch_trace::trace_boundary_at_in_view_cancellable(
                &doc_snapshot, &scope_for_thread, seed, &cancel_for_thread);
            if cancel_for_thread.load(Ordering::Relaxed) {
                let _ = tx.send(HatchWorkerResult::Cancelled { log_lines: log });
                return;
            }
            match tb {
                Some(tb) => {
                    log.push(format!(
                        "  worker: traced outer {} verts, {} island(s)",
                        tb.outer.len(), tb.islands.len()));
                    let mut loops = Vec::with_capacity(1 + tb.islands.len());
                    loops.push(tb.outer);
                    loops.extend(tb.islands);
                    let _ = tx.send(HatchWorkerResult::Success {
                        loops, log_lines: log,
                    });
                }
                None => {
                    log.push("  worker: trace produced no boundary".to_string());
                    let _ = tx.send(HatchWorkerResult::Failure {
                        reason: "trace returned None".into(),
                        log_lines: log,
                    });
                }
            }
        });
        self.hatch_worker = Some(HatchWorker {
            seed, pattern, active_layer, cancel, rx,
        });
        self.hatch_dbg(format!(
            "  spawned hatch worker for seed ({:.3},{:.3}) — UI stays responsive; Esc cancels",
            seed.x, seed.y));
        self.set_prompt(
            "hatching… UI stays responsive — press Esc to cancel".to_string());
    }

    /// Called every frame from `update`. Drains the worker channel if
    /// the trace has completed; materialises the result into the doc
    /// (Success) or logs the failure / cancel and falls back to the
    /// cheap path if applicable (Failure).
    fn poll_hatch_worker(&mut self) {
        let Some(worker) = &self.hatch_worker else { return; };
        let result = match worker.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                // Worker thread died without sending (e.g. panic).
                // Clear the field and log; don't fall back blindly.
                self.hatch_dbg("  worker disconnected without result (likely cancelled or panicked)");
                self.hatch_worker = None;
                return;
            }
        };
        // Take ownership of the worker so we can mutate `self.doc` etc.
        let worker = self.hatch_worker.take().unwrap();
        match result {
            HatchWorkerResult::Success { loops, log_lines } => {
                for line in log_lines { self.hatch_dbg(line); }
                // Off-screen warning: if the traced outer loop extends
                // beyond the current viewport bbox, the user can't see
                // the full hatch. Tell them.
                if let (Some((vmin, vmax)), Some(outer)) =
                    (self.last_visible, loops.first())
                {
                    let mut omin = Vec2::new(f64::INFINITY, f64::INFINITY);
                    let mut omax = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                    for v in outer {
                        if v.x < omin.x { omin.x = v.x; }
                        if v.y < omin.y { omin.y = v.y; }
                        if v.x > omax.x { omax.x = v.x; }
                        if v.y > omax.y { omax.y = v.y; }
                    }
                    let outside = omin.x < vmin.x || omin.y < vmin.y
                               || omax.x > vmax.x || omax.y > vmax.y;
                    if outside {
                        self.history.push(
                            "  ⚠ traced hatch boundary extends off-screen — zoom out to see the full region"
                            .into());
                        self.hatch_dbg(
                            "  worker: outer bbox extends beyond viewport — zoom out to see full hatch");
                    }
                }
                self.snapshot_doc();
                let mut handles: Vec<cad_kernel::Handle> = Vec::new();
                for loop_verts in loops {
                    let mut verts: Vec<cad_kernel::PolyVertex> = loop_verts.iter()
                        .map(|v| cad_kernel::PolyVertex { pos: *v, bulge: 0.0 })
                        .collect();
                    if verts.len() >= 2 {
                        let first = verts[0].pos;
                        let last  = verts[verts.len() - 1].pos;
                        if (last - first).len() < crate::hatch_trace::JOIN_EPS {
                            verts.pop();
                        }
                    }
                    if verts.len() < 3 { continue; }
                    let pl = cad_kernel::Polyline { vertices: verts, closed: true };
                    let mut d = cad_kernel::DObject::from(pl);
                    d.style = cad_kernel::Style::on_layer(worker.active_layer);
                    let idx = self.doc.push(d);
                    handles.push(self.doc.dobjects[idx].handle);
                }
                if handles.is_empty() {
                    self.hatch_dbg("  worker result had no usable polylines");
                    self.clear_prompt();
                    return;
                }
                let pattern_label = match &worker.pattern {
                    cad_kernel::HatchPattern::Solid => "SOLID".to_string(),
                    cad_kernel::HatchPattern::Pattern { name, .. } => name.clone(),
                };
                let n_loops = handles.len();
                let mut d = cad_kernel::DObject::from(cad_kernel::Hatch {
                    boundary_handles: handles,
                    pattern: worker.pattern.clone(),
                });
                d.style = cad_kernel::Style::on_layer(worker.active_layer);
                self.doc.push(d);
                self.gpu_dirty = true;
                self.index_dirty = true;
                self.history.push(format!(
                    "  + hatch ({}): traced boundary, {} loop(s) materialised (async)",
                    pattern_label, n_loops));
                self.dump_hatch_state();
                // If pick-point session is still armed, restore prompt
                // for the next click.
                if self.hatch_pick_point_armed {
                    let style = self.hatch_pick_point_session.as_ref()
                        .and_then(|(n, _, _)| n.clone())
                        .unwrap_or_else(|| "SOLID".to_string());
                    self.set_prompt(format!(
                        "hatch ({}): click another region OR Enter to finish  [Esc=cancel]",
                        style));
                } else {
                    self.clear_prompt();
                }
            }
            HatchWorkerResult::Failure { reason, log_lines } => {
                for line in log_lines { self.hatch_dbg(line); }
                self.hatch_dbg(format!("  worker failure: {}", reason));
                // Fall back to cheap path with auto-islands — same
                // logic apply_pick_point_hatch's sync fallback uses.
                let seed = worker.seed;
                if let Some(idx) = self.find_smallest_containing_closed(seed) {
                    let kind = self.doc.dobjects.get(idx)
                        .map(|d| dobject_kind_name(&d.geom))
                        .unwrap_or("?");
                    let islands = self.collect_islands_inside(idx, seed);
                    self.hatch_dbg(format!(
                        "  fallback → CHEAP PATH outer #{} ({}) + {} island(s)",
                        idx, kind, islands.len()));
                    // Restore the pending pattern so apply_hatch picks it up.
                    if let Some(sess) = self.hatch_pick_point_session.clone() {
                        self.pending_hatch_pattern = sess;
                    }
                    self.selection = std::iter::once(idx).chain(islands).collect();
                    self.apply_hatch();
                    self.selection.clear();
                } else {
                    self.history.push(
                        "  ! hatch pick-point: no closed boundary contains the click".into());
                }
                // Restore prompt if session armed
                if self.hatch_pick_point_armed {
                    let style = self.hatch_pick_point_session.as_ref()
                        .and_then(|(n, _, _)| n.clone())
                        .unwrap_or_else(|| "SOLID".to_string());
                    self.set_prompt(format!(
                        "hatch ({}): click another region OR Enter to finish  [Esc=cancel]",
                        style));
                } else {
                    self.clear_prompt();
                }
            }
            HatchWorkerResult::Cancelled { log_lines } => {
                for line in log_lines { self.hatch_dbg(line); }
                self.hatch_dbg("  worker: trace cancelled by user (Esc)");
                self.history.push("  hatch trace cancelled".into());
                self.clear_prompt();
            }
        }
    }

    fn apply_pick_point_hatch(&mut self, seed: Vec2) -> bool {
        // === Viewport scope ===
        // Restrict EVERY hatch operation to the dobjects whose bbox
        // overlaps the current viewport — at 400k+ dobjects, iterating
        // the full doc per click is unworkable. The spatial index
        // makes this O(visible cells) instead of O(N). Fallback to
        // full doc only when the index is stale or no frame has
        // rendered yet.
        //
        // CRITICAL: rebuild the index FIRST if it's dirty. Otherwise
        // viewport_scope() falls back to full-doc range, which defeats
        // the whole point of scoping. This bug bit on the 2nd click in
        // a multi-pick session — the 1st hatch's `index_dirty = true`
        // turned the 2nd click into a full-doc scan.
        let _ = self.ensure_index();
        let view_scope: Vec<usize> = self.viewport_scope()
            .unwrap_or_else(|| (0..self.doc.dobjects.len()).collect());
        let total = self.doc.dobjects.len();
        let in_view = view_scope.len();
        self.hatch_dbg(format!(
            "  --- viewport scope: {} of {} dobjects in view ({:.1}%) — iterating scope only ---",
            in_view, total, 100.0 * in_view as f32 / total.max(1) as f32));

        // Verbose dump: every dobject + every coordinate the trace
        // pipeline will see this click. Lets the user verify "is this
        // polyline actually closed?", "do these line endpoints really
        // meet?" without guessing. Skipped when the debug window is
        // closed (`hatch_dbg` is a no-op then).
        // ONLY in-view dobjects are dumped — out-of-view ones can't
        // affect the hatch in the visible region.
        self.hatch_dbg(format!(
            "  --- doc snapshot at pick-point click ({:.3},{:.3})  zoom={:.2} px/world  world_per_px={:.4} ---",
            seed.x, seed.y, self.scale, 1.0 / (self.scale as f64).max(1e-9)));
        let doc_lines: Vec<String> = view_scope.iter().filter_map(|&i| {
                self.doc.dobjects.get(i).map(|d| (i, d))
            })
            .map(|(i, d)| {
                let (bmin, bmax) = d.geom.bbox();
                let closed_tag = match &d.geom {
                    Geom::Polyline(p) => {
                        if p.closed { " [closed]".to_string() }
                        else if polyline_is_effectively_closed(p) {
                            " [closed-by-gap]".to_string()
                        }
                        else { " [open]".to_string() }
                    }
                    Geom::Circle(_) | Geom::Ellipse(_) => " [closed]".to_string(),
                    _ => String::new(),
                };
                format!(
                    "    #{:02} [{}] bbox=({:.2},{:.2})→({:.2},{:.2}){} {}",
                    i,
                    if d.style.visible { "vis" } else { "hid" },
                    bmin.x, bmin.y, bmax.x, bmax.y,
                    closed_tag,
                    describe_verbose(&d.geom))
            })
            .collect();
        for line in doc_lines {
            self.hatch_dbg(line);
        }
        // Cheap path first: hits the typical "click inside one closed
        // shape" workflow without paying for the trace.
        // Per-dobject verdict log so the user can see exactly WHY each
        // dobject was accepted / rejected as a candidate. The actual
        // decision still comes from collect_closed_containing — this
        // is pure instrumentation.
        self.hatch_dbg("  --- cheap-path verdict per dobject (viewport only) ---");
        let verdict_lines: Vec<String> = view_scope.iter().filter_map(|&i| {
                self.doc.dobjects.get(i).map(|d| (i, d))
            })
            .map(|(i, d)| {
                let kind = dobject_kind_name(&d.geom);
                match &d.geom {
                    Geom::Polyline(p) => {
                        if !polyline_is_effectively_closed(p) {
                            format!("    #{:02} {} — SKIP (open polyline, gap≥{:.0e})",
                                i, kind, POLYLINE_EFFECTIVELY_CLOSED_EPS)
                        } else {
                            let verts = closed_dobject_polygon(&d.geom);
                            if point_in_polygon(seed, verts) {
                                let (bmin, bmax) = d.geom.bbox();
                                let area = (bmax.x - bmin.x).abs() * (bmax.y - bmin.y).abs();
                                format!("    #{:02} {} — CANDIDATE (contains seed, bbox area {:.3})",
                                    i, kind, area)
                            } else {
                                format!("    #{:02} {} — skip (closed but does NOT contain seed)", i, kind)
                            }
                        }
                    }
                    Geom::Circle(c) => {
                        let dist = (seed - c.center).len();
                        if dist < c.radius {
                            let area = (2.0 * c.radius).powi(2);
                            format!("    #{:02} {} — CANDIDATE (contains seed, dist {:.3} < r {:.3}, bbox area {:.3})",
                                i, kind, dist, c.radius, area)
                        } else {
                            format!("    #{:02} {} — skip (seed dist {:.3} ≥ r {:.3})",
                                i, kind, dist, c.radius)
                        }
                    }
                    Geom::Ellipse(e) => {
                        let dvec = seed - e.center;
                        let a = e.semi_major().max(1e-12);
                        let b = e.semi_minor().max(1e-12);
                        let u = dvec.dot(e.u_hat()) / a;
                        let v = dvec.dot(e.v_hat()) / b;
                        let val = u * u + v * v;
                        if val < 1.0 {
                            format!("    #{:02} {} — CANDIDATE (contains seed, u²+v²={:.3} < 1)",
                                i, kind, val)
                        } else {
                            format!("    #{:02} {} — skip (u²+v²={:.3} ≥ 1)", i, kind, val)
                        }
                    }
                    _ => format!("    #{:02} {} — SKIP (not a self-closed boundary type)", i, kind),
                }
            }).collect();
        for line in verdict_lines {
            self.hatch_dbg(line);
        }
        let cheap_candidates = self.collect_closed_containing_scoped(seed, Some(&view_scope));
        if !cheap_candidates.is_empty() {
            self.hatch_dbg(format!(
                "  --- cheap-path candidates ({} found, picked smallest by bbox area) ---",
                cheap_candidates.len()));
            for (idx, area, kind) in &cheap_candidates {
                self.hatch_dbg(format!(
                    "    #{:02} {} — bbox area {:.3}", idx, kind, area));
            }
        } else {
            self.hatch_dbg("  --- cheap-path: 0 self-closed dobjects contain the click ---");
        }
        // Routing rule:
        //  * 0 closed candidates contain the seed → trace path (the
        //    boundary is formed by open primitives chained together).
        //  * Exactly 1 candidate AND no other dobject crosses its
        //    boundary → cheap path (single self-closed boundary +
        //    auto-islands inside).
        //  * Exactly 1 candidate BUT its boundary is crossed by some
        //    other dobject → trace path (the planar subdivision has
        //    sub-faces inside the outer; the cheap path would
        //    incorrectly hatch the whole outer).
        //  * 2+ candidates contain the seed → trace path.
        let multiple = cheap_candidates.len() > 1;
        let single_outer_crossed = !multiple
            && cheap_candidates.len() == 1
            && self.outer_has_crossings_with_others(cheap_candidates[0].0);
        if single_outer_crossed {
            self.hatch_dbg(format!(
                "  --- single outer #{} has crossings with other dobjects → deferring to trace path ---",
                cheap_candidates[0].0));
        }
        if !multiple && !single_outer_crossed {
            if let Some(idx) = self.find_smallest_containing_closed_scoped(seed, Some(&view_scope)) {
                let kind = self.doc.dobjects.get(idx)
                    .map(|d| dobject_kind_name(&d.geom))
                    .unwrap_or("?");
                // AutoCAD BPOLY also auto-adds any closed dobjects sitting
                // ENTIRELY INSIDE the chosen outer as islands — the user
                // doesn't have to pre-select them. Scoped to viewport.
                self.hatch_dbg(format!(
                    "  --- island scan inside outer #{} (viewport only) ---", idx));
                let scan_lines = self.dbg_island_scan_verdicts(idx, seed);
                for line in scan_lines {
                    self.hatch_dbg(line);
                }
                let islands = self.collect_islands_inside_scoped(idx, seed, Some(&view_scope));
                if !islands.is_empty() {
                    self.hatch_dbg(format!(
                        "  → CHEAP PATH chose outer #{} ({}) + auto-detected {} island(s): {:?}",
                        idx, kind, islands.len(), islands));
                } else {
                    self.hatch_dbg(format!(
                        "  → CHEAP PATH chose smallest containing dobject #{} ({}), 0 auto-islands",
                        idx, kind));
                }
                self.selection = std::iter::once(idx).chain(islands).collect();
                self.apply_hatch();
                self.selection.clear();
                return true;
            }
        } else if multiple {
            self.hatch_dbg(format!(
                "  --- {} candidates contain seed → PARTIAL OVERLAP, deferring to trace path ---",
                cheap_candidates.len()));
        }
        // (the single_outer_crossed log was emitted above before the branch)
        // Full trace path — handles arbitrary chains AND partial overlaps
        // (intersection-splitting inside trace_boundary_at).
        //
        // Backgrounded: spawn the heavy work (tessellate + split +
        // cluster + trace) on a worker thread. The UI keeps spinning
        // and Esc presses are seen mid-op via the cancel flag.
        // `poll_hatch_worker` (called each frame from `update`)
        // drains the result and materialises the hatch when ready.
        self.hatch_dbg("  --- TRACE PATH engaged (async, worker thread) ---");
        self.spawn_hatch_worker(seed);
        true
    }

    /// Click on a dobject during a selection session. Plain click = ADD
    /// (no-op if already in the basket). Shift+click = REMOVE (no-op if
    /// not in the basket). The persistent `remove` sub-command from the
    /// command line flips the default: while it's on, plain clicks act
    /// like Shift+clicks and Shift+clicks act like plain clicks.
    fn click_select(&mut self, i: usize, shift: bool) {
        // Effective intent: Shift inverts whatever the current mode says.
        let want_remove = shift ^ self.select_remove_mode;
        if want_remove {
            if let Some(pos) = self.selection.iter().position(|&x| x == i) {
                self.selection.remove(pos);
                self.history.push(format!("    – #{} removed", i));
            } else {
                self.history.push(format!(
                    "    (skip) #{} not in the basket", i));
            }
        } else if !self.selection.contains(&i) {
            self.selection.push(i);
            self.history.push(format!("    + #{} added", i));
        } else {
            self.history.push(format!(
                "    (skip) #{} already in the basket — Shift+click to remove", i));
        }
    }

    /// Translate every dobject in `self.selection` by `v`. Used by the
    /// `move` command after the user clicks BASE and DESTINATION. Edits the
    /// dobjects in place; invalidates the spatial index and any cached
    /// intersections so the next ∩ query rebuilds. Snapshots the basket into
    /// `selection_prev` and clears the visible selection — the dobjects
    /// revert to their normal colour, but the user can re-grab them with
    /// `before` in the next selection session.
    fn apply_move(&mut self, v: Vec2) {
        if v.len() < EPS { return; }
        self.snapshot_doc();
        for &i in &self.selection {
            if let Some(d) = self.doc.dobjects.get_mut(i) {
                *d = d.translated(v);
            }
        }
        // Save for `before`, then clear the live highlight.
        if !self.selection.is_empty() {
            self.selection_prev = self.selection.clone();
        }
        self.selection.clear();
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty   = true;
    }

    /// Add every dobject index to the selection. Used by the `all` sub-command.
    fn add_all_to_selection(&mut self) -> usize {
        let mut added = 0usize;
        for i in 0..self.doc.dobjects.len() {
            if !self.selection.contains(&i) {
                self.selection.push(i);
                added += 1;
            }
        }
        added
    }

    /// Close a window-selection rectangle. Direction = mode:
    ///   L→R drag → "inside" window (only dobjects whose bbox is fully in).
    ///   R→L drag → "crossing" window (any overlap counts).
    /// Modifier = sign: `shift` (or the persistent `select_remove_mode`)
    /// makes the window SUBTRACT instead of ADD.
    fn add_window_selection(&mut self, p1: Vec2, p2: Vec2, shift: bool) {
        let bbox_min = Vec2::new(p1.x.min(p2.x), p1.y.min(p2.y));
        let bbox_max = Vec2::new(p1.x.max(p2.x), p1.y.max(p2.y));
        // === Hard rule (feedback_rust_cad_universal_selection_model) ===
        // Typed `w` / `c` ALWAYS beats drag direction. The .take() is
        // critical — the override is consumed by the first completing
        // window so it doesn't carry over to the next gesture. Do NOT
        // reorder this match without re-reading the memo: any future
        // "smart direction detection" must still fall under the
        // Some(_) arms, not over them.
        let crossing = match self.armed_window_inside.take() {
            Some(true)  => false,            // armed window → inside-only
            Some(false) => true,             // armed crossing
            None        => p2.x < p1.x,      // direction-default (R→L = crossing)
        };
        let want_remove = shift ^ self.select_remove_mode;

        let cands: Vec<usize> = match (self.index.as_ref(), self.index_dirty) {
            (Some(g), false) => g.query_bbox(bbox_min, bbox_max)
                .into_iter().map(|u| u as usize).collect(),
            _ => (0..self.doc.dobjects.len()).collect(),
        };

        let mut changed = 0usize;
        for i in cands {
            let (emin, emax) = self.doc.dobjects[i].bbox();
            let inside = if crossing {
                !(emax.x < bbox_min.x || emin.x > bbox_max.x
                    || emax.y < bbox_min.y || emin.y > bbox_max.y)
            } else {
                emin.x >= bbox_min.x && emax.x <= bbox_max.x
                    && emin.y >= bbox_min.y && emax.y <= bbox_max.y
            };
            if !inside { continue; }
            if want_remove {
                if let Some(pos) = self.selection.iter().position(|&x| x == i) {
                    self.selection.remove(pos);
                    changed += 1;
                }
            } else if !self.selection.contains(&i) {
                self.selection.push(i);
                changed += 1;
            }
        }
        self.history.push(format!(
            "    {} {} dobject(s) via {} window (current: {})",
            if want_remove { "−" } else { "+" },
            changed,
            if crossing { "crossing" } else { "inside" },
            self.selection.len(),
        ));
    }

    // ===================================================================
    // Slice B — Layer panel
    // ===================================================================
    //
    // Egui-port of LibreCAD's `qg_layerwidget`. Operates directly on
    // `self.doc.layers` (the `LayerTable` from cad_kernel). Active layer
    // = the one new Dobjects get assigned to on `Document::push`.

    /// "Choose Hatch Attributes" modal — pattern + scale + angle +
    /// live preview. Mirrors LibreCAD's hatch dialog. OK feeds into
    /// the same `apply_hatch` path as the command-line form; Cancel
    /// drops the pending state. Opens whenever the user types bare
    /// `hatch` with no args.
    /// Width within which a floating Window's edge is treated as "close
    /// enough to dock". Same value the user asked for.
    const DOCK_THRESHOLD_PX: f32 = 50.0;
    /// Minimum width docked top/bottom strips will use. Prevents the
    /// window from shrinking below this when the user resizes it
    /// after docking.
    const DOCK_STRIP_MIN_WIDTH:  f32 = 320.0;
    /// Minimum height docked left/right strips will use.
    const DOCK_STRIP_MIN_HEIGHT: f32 = 200.0;
    /// Hold the title bar this long (seconds) to release a docked
    /// window. Long enough that an accidental click doesn't undock,
    /// short enough that a deliberate "I want to move this" feels
    /// instant.
    const DOCK_UNDOCK_HOLD_SEC: f32 = 0.20;

    /// If `id` has a docked state stored, pin its position AND constrain
    /// its size based on which edge it's docked to:
    ///   * Top/Bottom → forces `min_width = screen_width` so the
    ///     window becomes a horizontal strip spanning the screen.
    ///   * Left/Right → forces `min_height = screen_height` for a
    ///     vertical strip.
    ///   * Corners    → pinned position only; size stays content-fit.
    /// Reversible — when the user drags away further than the
    /// threshold, `process_dock_after_show` removes the entry and the
    /// Window goes back to free positioning + sizing.
    fn apply_dock_pos<'a>(
        &self,
        _id: &'static str,
        _ctx: &egui::Context,
        window: egui::Window<'a>,
    ) -> egui::Window<'a> {
        // Auto-docking + auto-resize disabled per user request — was
        // wasting time on edge cases. Windows are now plain floating:
        // user moves and resizes manually. Will be redone in a dedicated
        // dock-system pass later.
        window
    }

    /// After a Window has been shown, compute whether any of its edges
    /// is within `DOCK_THRESHOLD_PX` of a screen edge. If so, snap it
    /// flush to that edge and store the snapped position. Otherwise
    /// clear any prior snap so subsequent drags are free.
    fn process_dock_after_show<R>(
        &mut self,
        id: &'static str,
        ctx: &egui::Context,
        resp: Option<egui::InnerResponse<R>>,
    ) {
        // Auto-snap disabled per user request. Was triggering false
        // positives (windows docking to top on first appear, all
        // windows piling up at the top, etc.). Will be redesigned in
        // a dedicated dock-system pass later. For now: leave windows
        // free-floating — user manages position/size manually.
        let _ = (id, ctx, resp);
    }

    fn render_hatch_dialog(&mut self, ctx: &egui::Context) {
        if !self.hatch_dialog_open { return; }
        let mut open = true;
        let mut clicked_dobjects   = false;
        let mut clicked_pick_point = false;
        egui::Window::new("Choose Hatch Attributes")
            .id(egui::Id::new("hatch_dialog"))
            .open(&mut open)
            .default_size(egui::vec2(560.0, 360.0))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // --- LEFT: pattern + scale + angle ---
                    ui.vertical(|ui| {
                        ui.heading("Pattern");
                        ui.add_space(4.0);
                        ui.checkbox(&mut self.hatch_dialog_solid, "Solid Fill");
                        ui.add_space(4.0);
                        ui.add_enabled_ui(!self.hatch_dialog_solid, |ui| {
                            egui::ComboBox::from_id_salt("hatch_pattern_combo")
                                .selected_text(&self.hatch_dialog_name)
                                .width(200.0)
                                .show_ui(ui, |ui| {
                                    for name in cad_kernel::patterns::PATTERN_NAMES {
                                        if *name == "SOLID" { continue; }
                                        ui.selectable_value(
                                            &mut self.hatch_dialog_name,
                                            (*name).to_string(),
                                            *name);
                                    }
                                });
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("Scale:");
                            ui.add(egui::DragValue::new(&mut self.hatch_dialog_scale)
                                .speed(0.05).range(0.05..=100.0));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Angle:");
                            ui.add(egui::DragValue::new(&mut self.hatch_dialog_angle)
                                .speed(1.0).range(-360.0..=360.0).suffix("°"));
                        });
                    });
                    ui.separator();
                    // --- RIGHT: live preview ---
                    ui.vertical(|ui| {
                        ui.heading("Preview");
                        ui.add_space(4.0);
                        let (rect, _resp) = ui.allocate_exact_size(
                            egui::vec2(240.0, 240.0), egui::Sense::hover());
                        let p = ui.painter_at(rect);
                        // Backdrop
                        p.rect_filled(rect, 0.0,
                            egui::Color32::from_rgb(18, 22, 28));
                        p.rect_stroke(rect, 0.0,
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 80, 95)));
                        // Square boundary in preview-local coords
                        let pad = 18.0_f32;
                        let bound = egui::Rect::from_min_max(
                            rect.left_top() + egui::vec2(pad, pad),
                            rect.right_bottom() - egui::vec2(pad, pad));
                        let accent = egui::Color32::from_rgb(255, 80, 80);
                        // Pattern fill preview
                        if self.hatch_dialog_solid {
                            p.rect_filled(bound, 0.0, accent);
                        } else {
                            // Render the chosen pattern inside `bound`
                            // by walking each LineFamily, generating
                            // parallel screen-space lines, clipping to
                            // bound. No world↔screen transform — the
                            // preview just shows the pattern's shape.
                            let families = cad_kernel::patterns::lookup(
                                &self.hatch_dialog_name);
                            let user_angle = self.hatch_dialog_angle.to_radians();
                            let user_scale = self.hatch_dialog_scale.max(0.05);
                            // World→preview scale: ~10 px per world unit
                            // so the catalog's natural spacings of a
                            // few mm read as visible textures.
                            let px_per_unit = 10.0_f32;
                            for fam in &families {
                                let theta = fam.angle + user_angle;
                                let spacing_px = (fam.spacing * user_scale) as f32 * px_per_unit;
                                if spacing_px < 1.0 { continue; }   // safety
                                let cos = theta.cos() as f32;
                                let sin = theta.sin() as f32;
                                // bbox diag = max distance from any
                                // point in bound to any other, gives a
                                // safe range to walk.
                                let diag = (bound.width()*bound.width()
                                            + bound.height()*bound.height()).sqrt();
                                let n_lines = (diag / spacing_px).ceil() as i32;
                                let centre = bound.center();
                                for k in -n_lines..=n_lines {
                                    let s = (k as f32) * spacing_px;
                                    // Line through centre+s*normal, direction = (cos, sin).
                                    let nx = -sin; let ny = cos;
                                    let mid = egui::pos2(
                                        centre.x + nx * s,
                                        centre.y + ny * s);
                                    let a = egui::pos2(
                                        mid.x - cos * diag,
                                        mid.y - sin * diag);
                                    let b = egui::pos2(
                                        mid.x + cos * diag,
                                        mid.y + sin * diag);
                                    // Clip the line to bound via
                                    // Cohen-Sutherland-lite (egui has
                                    // no built-in line clip; do a
                                    // parametric clamp against the
                                    // 4 edges).
                                    if let Some((p1, p2)) = clip_line_to_rect(a, b, bound) {
                                        p.line_segment([p1, p2],
                                            egui::Stroke::new(1.0, accent));
                                    }
                                }
                            }
                        }
                        // Centre marker (the "+" shown in LibreCAD's preview)
                        let c = rect.center();
                        let mk = egui::Stroke::new(1.0,
                            egui::Color32::from_rgb(80, 90, 105));
                        p.line_segment(
                            [egui::pos2(c.x - 8.0, c.y), egui::pos2(c.x + 8.0, c.y)], mk);
                        p.line_segment(
                            [egui::pos2(c.x, c.y - 8.0), egui::pos2(c.x, c.y + 8.0)], mk);
                    });
                });
                ui.add_space(10.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("◉ Pick Point")
                            .on_hover_text("Click inside a closed region — the app finds the smallest containing closed dobject and hatches it")
                            .clicked() { clicked_pick_point = true; }
                        if ui.button("☐ Dobject/s")
                            .on_hover_text("Select boundary dobjects (closed polylines / circles / ellipses), Enter to fill")
                            .clicked() { clicked_dobjects = true; }
                    });
                });
            });
        // X close button → dismiss (no commit, no follow-up state).
        if !open {
            self.hatch_dialog_open = false;
            self.hatch_dbg("dialog dismissed via X close".to_string());
            self.clear_prompt();
            return;
        }
        if clicked_dobjects || clicked_pick_point {
            self.hatch_dialog_open = false;
            let pattern = if self.hatch_dialog_solid {
                None
            } else {
                Some(self.hatch_dialog_name.clone())
            };
            self.pending_hatch_pattern = (
                pattern.clone(),
                self.hatch_dialog_scale,
                self.hatch_dialog_angle,
            );
            let style = pattern.as_deref().unwrap_or("SOLID").to_string();
            self.hatch_dbg(format!(
                "dialog committed: pattern={}, scale={:.3}, angle={:.2}",
                style, self.hatch_dialog_scale, self.hatch_dialog_angle));
            if clicked_dobjects {
                self.hatch_dbg(format!(
                    "  Dobject/s button — selection.len() = {}",
                    self.selection.len()));
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Hatch;
                    self.set_prompt(format!(
                        "hatch ({}): pick CLOSED boundary dobject(s), Enter to fill  [Esc=cancel]",
                        style));
                } else {
                    self.apply_hatch();
                }
            } else {
                self.hatch_pick_point_armed = true;
                // Remember the pattern for the whole pick-point session
                // so successive clicks all use it (apply_hatch consumes
                // pending_hatch_pattern; we re-fill it after each click).
                self.hatch_pick_point_session = Some((
                    pattern.clone(),
                    self.hatch_dialog_scale,
                    self.hatch_dialog_angle,
                ));
                self.hatch_dbg(
                    "  Pick Point button — armed canvas click (persistent until Enter)".to_string());
                self.set_prompt(format!(
                    "hatch ({}): click inside closed region(s); Enter to finish  [Esc=cancel]",
                    style));
            }
        }
    }

    /// Trim Debug floating window — instrumented log of every trim /
    /// extend state transition + canvas click. User pastes the log back
    /// when reporting a bug.
    fn render_trim_debug_window(&mut self, ctx: &egui::Context) {
        let mut open = self.trim_debug_open;
        let win = egui::Window::new("Trim Debug Log")
            .open(&mut open)
            .default_width(640.0)
            .default_height(400.0)
            .resizable(true);
        let win = self.apply_dock_pos("Trim Debug Log", ctx, win);
        let resp = win.show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("📋 Copy Log").on_hover_text("Copy the whole log to the clipboard").clicked() {
                        let text = self.trim_debug_log.join("\n");
                        ui.ctx().copy_text(text);
                        self.history.push("  trim debug log → clipboard".into());
                    }
                    if ui.button("🗑 Clear").clicked() {
                        self.trim_debug_log.clear();
                    }
                    ui.label(format!("{} entries", self.trim_debug_log.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let trim_active = matches!(
                            self.trim_state,
                            TrimState::PickingTargets(_) | TrimState::PickingTargetsAll);
                        let extend_active = matches!(
                            self.extend_state,
                            ExtendState::PickingTargets(_) | ExtendState::PickingTargetsAll);
                        if trim_active {
                            ui.colored_label(egui::Color32::from_rgb(255, 170, 60),
                                "● TRIM target-pick active");
                        } else if extend_active {
                            ui.colored_label(egui::Color32::from_rgb(255, 220, 90),
                                "● EXTEND target-pick active");
                        } else {
                            ui.colored_label(egui::Color32::from_rgb(140, 140, 150),
                                "○ no session running");
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if self.trim_debug_log.is_empty() {
                            ui.colored_label(egui::Color32::from_rgb(140, 140, 150),
                                "(empty — run `trim` or `extend` to start logging)");
                        }
                        for line in &self.trim_debug_log {
                            ui.monospace(line);
                        }
                    });
            });
        self.process_dock_after_show("Trim Debug Log", ctx, resp);
        self.trim_debug_open = open;
    }

    /// Hatch debug window — same shape as the trim debug log. Records
    /// every state transition in the hatch flow so the user can pin
    /// down where the hatch is "falling": dialog open / pattern +
    /// scale + angle changes / Dobjects vs Pick Point button / canvas
    /// click consumed for pick-point / smallest-containing search
    /// candidates + winner / apply_hatch params / resolved loops /
    /// render line counts. Toggle via Tools menu.
    fn render_hatch_debug_window(&mut self, ctx: &egui::Context) {
        let mut open = self.hatch_debug_open;
        let mut do_dump_state = false;
        let win = egui::Window::new("Hatch Debug Log")
            .open(&mut open)
            .default_width(720.0)
            .default_height(420.0)
            .resizable(true);
        let win = self.apply_dock_pos("Hatch Debug Log", ctx, win);
        let resp = win.show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("📋 Copy Log").on_hover_text("Copy the whole log to the clipboard").clicked() {
                        let text = self.hatch_debug_log.join("\n");
                        ui.ctx().copy_text(text);
                        self.history.push("  hatch debug log → clipboard".into());
                    }
                    if ui.button("🗑 Clear").clicked() {
                        self.hatch_debug_log.clear();
                    }
                    if ui.button("📸 Dump Hatch State")
                        .on_hover_text("Append a snapshot of every Hatch dobject in the doc (boundary handles, resolved loop vertex counts, pattern). Press anytime; no per-frame flood.")
                        .clicked()
                    {
                        do_dump_state = true;
                    }
                    ui.label(format!("{} entries", self.hatch_debug_log.len()));
                    // Live status — what's the hatch flow doing right now?
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.hatch_dialog_open {
                            ui.colored_label(egui::Color32::from_rgb(120, 220, 255),
                                "● dialog open");
                        } else if self.hatch_pick_point_armed {
                            ui.colored_label(egui::Color32::from_rgb(255, 200, 120),
                                "● pick-point armed");
                        } else if matches!(self.queued_op, QueuedOp::Hatch) {
                            ui.colored_label(egui::Color32::from_rgb(255, 220, 90),
                                "● awaiting boundary selection");
                        } else {
                            ui.colored_label(egui::Color32::from_rgb(140, 140, 150),
                                "○ idle");
                        }
                    });
                });
                ui.separator();
                // Real-time hatch attrs readout — always visible at the
                // top of the log so the user can SEE the pattern/scale/
                // angle that the next `apply_hatch` will use.
                let (pat, sc, ang) = &self.pending_hatch_pattern;
                ui.monospace(format!(
                    "pending: pattern={:<12} scale={:<6.3} angle={:>6.2}°    \
                     dialog(solid={}, name={:?}, scale={:.3}, angle={:.2})",
                    pat.as_deref().unwrap_or("(SOLID)"), sc, ang,
                    self.hatch_dialog_solid, self.hatch_dialog_name,
                    self.hatch_dialog_scale, self.hatch_dialog_angle));
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if self.hatch_debug_log.is_empty() {
                            ui.colored_label(egui::Color32::from_rgb(140, 140, 150),
                                "(empty — run `hatch` to start logging)");
                        }
                        for line in &self.hatch_debug_log {
                            ui.monospace(line);
                        }
                    });
            });
        self.process_dock_after_show("Hatch Debug Log", ctx, resp);
        self.hatch_debug_open = open;
        if do_dump_state {
            self.dump_hatch_state();
        }
    }

    /// Append a one-shot snapshot of every Hatch dobject in the doc
    /// to the debug log. Triggered by the debug window's button. Lists
    /// boundary handle indices + resolved loop vertex counts so the
    /// user can see if "the hatch entity exists but has no rendered
    /// fill" is a render bug or a boundary-resolve bug.
    fn dump_hatch_state(&mut self) {
        let mut entries: Vec<String> = Vec::new();
        for (i, d) in self.doc.dobjects.iter().enumerate() {
            if let Geom::Hatch(h) = &d.geom {
                let pat = match &h.pattern {
                    cad_kernel::HatchPattern::Solid => "SOLID".to_string(),
                    cad_kernel::HatchPattern::Pattern { name, scale, angle_deg } =>
                        format!("{} scale={:.3} angle={:.2}", name, scale, angle_deg),
                };
                let resolved = self.resolve_hatch_loops(h);
                let handle_indices: Vec<String> = h.boundary_handles.iter()
                    .map(|hh| {
                        self.doc.index_of_handle(*hh)
                            .map(|ix| format!("#{}", ix))
                            .unwrap_or_else(|| format!("(missing handle {:#x})", hh))
                    })
                    .collect();
                // Per-loop bbox + line-count estimate. This catches the
                // most common pattern-doesn't-show bug: pattern spacing
                // is larger than the loop, so 0 lines cross the boundary
                // and the fill looks blank. The diagnostic tells the
                // user "raise the scale".
                let mut loops_desc: Vec<String> = Vec::new();
                let mut union_min = Vec2::new(f64::INFINITY, f64::INFINITY);
                let mut union_max = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                for l in &resolved {
                    let mut lmin = Vec2::new(f64::INFINITY, f64::INFINITY);
                    let mut lmax = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                    for v in l {
                        if v.x < lmin.x { lmin.x = v.x; }
                        if v.y < lmin.y { lmin.y = v.y; }
                        if v.x > lmax.x { lmax.x = v.x; }
                        if v.y > lmax.y { lmax.y = v.y; }
                        if v.x < union_min.x { union_min.x = v.x; }
                        if v.y < union_min.y { union_min.y = v.y; }
                        if v.x > union_max.x { union_max.x = v.x; }
                        if v.y > union_max.y { union_max.y = v.y; }
                    }
                    let w = lmax.x - lmin.x;
                    let hh = lmax.y - lmin.y;
                    loops_desc.push(format!(
                        "{}v bbox={:.2}x{:.2}", l.len(), w, hh));
                }
                // Estimate line count for each family in the pattern at
                // the dobject's current scale + angle.
                let mut line_estimate = String::new();
                if let cad_kernel::HatchPattern::Pattern { name, scale, .. } = &h.pattern {
                    if union_max.x.is_finite() {
                        let diag = ((union_max.x - union_min.x).powi(2)
                                  + (union_max.y - union_min.y).powi(2)).sqrt();
                        let fams = cad_kernel::patterns::lookup(name);
                        let counts: Vec<String> = fams.iter().map(|f| {
                            let s = f.spacing * scale.abs().max(1e-9);
                            let n = (diag / s).ceil() as i64;
                            format!("{}({} lines @ spacing {:.3})",
                                (f.angle.to_degrees() as i32 % 360), n, s)
                        }).collect();
                        line_estimate = format!("  estimated families: [{}]",
                            counts.join(", "));
                        if counts.iter().any(|c| c.contains("(0 lines")) {
                            line_estimate.push_str("  ⚠ ZERO LINES — raise scale or pick a finer pattern");
                        }
                    }
                }
                entries.push(format!(
                    "hatch #{} pattern=[{}] boundary_handles=[{}] resolved_loops=[{}]{}",
                    i, pat, handle_indices.join(", "), loops_desc.join(", "),
                    line_estimate));
            }
        }
        if entries.is_empty() {
            self.hatch_dbg("dump: no hatch dobjects in the doc".to_string());
        } else {
            self.hatch_dbg(format!("dump: {} hatch dobject(s):", entries.len()));
            for e in entries {
                self.hatch_dbg(format!("  {}", e));
            }
        }
    }

    /// Screen Stats window — confirms the renderer's view of the doc
    /// (total / in viewport / drawn / skipped) so the user can verify
    /// the spatial-index broad-phase is doing useful work, the
    /// viewport cull isn't dropping things it shouldn't, etc.
    /// Per-frame numbers from `last_render_stats`. Toggleable via the
    /// Tools menu; open by default because that's the whole point —
    /// the user wanted to SEE this info.
    fn render_screen_stats_window(&mut self, ctx: &egui::Context) {
        // Detect false→true edge: user just reopened the window via
        // menu. Clear any stale snap-dock so the window appears at its
        // default_pos instead of getting stuck at an old snap position
        // (which can be off-screen or behind a panel after a resize).
        if self.screen_stats_open && !self.screen_stats_was_open {
            self.docked_window_pos.remove("Screen Stats");
        }
        self.screen_stats_was_open = self.screen_stats_open;
        if !self.screen_stats_open { return; }
        let mut open = self.screen_stats_open;
        let stats = self.last_render_stats.clone();
        let fps = if stats.frame_dt > 0.0 { 1.0 / stats.frame_dt } else { 0.0 };
        let win = egui::Window::new("Screen Stats")
            .open(&mut open)
            .default_pos(egui::pos2(20.0, 110.0))
            .default_size(egui::vec2(280.0, 220.0))
            .resizable(true)
            .collapsible(true);
        let win = self.apply_dock_pos("Screen Stats", ctx, win);
        let resp = win.show(ctx, |ui| {
            ui.style_mut().override_font_id = Some(egui::FontId::monospace(12.0));
            let cull_ratio = if stats.total > 0 {
                100.0 * stats.in_viewport as f32 / stats.total as f32
            } else { 0.0 };
            let draw_ratio = if stats.in_viewport > 0 {
                100.0 * stats.drawn as f32 / stats.in_viewport as f32
            } else { 0.0 };

            egui::Grid::new("stats_grid")
                .num_columns(2)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    ui.label("total dobjects:");
                    ui.label(format!("{}", stats.total));
                    ui.end_row();

                    ui.label("in viewport:");
                    ui.label(format!("{}  ({:.1}% of total)",
                        stats.in_viewport, cull_ratio));
                    ui.end_row();

                    ui.label("drawn:");
                    ui.label(format!("{}  ({:.1}% of viewport)",
                        stats.drawn, draw_ratio));
                    ui.end_row();

                    ui.label("skipped:");
                    ui.label(format!("{}  (hidden / sub-pixel)",
                        stats.skipped_hidden + stats.skipped_subpx));
                    ui.end_row();

                    ui.label("");
                    ui.label("");
                    ui.end_row();

                    ui.label("FPS:");
                    ui.label(format!("{:.1}", fps));
                    ui.end_row();

                    ui.label("frame:");
                    ui.label(format!("{:.2} ms", stats.frame_dt * 1000.0));
                    ui.end_row();

                    ui.label("render mode:");
                    ui.label(match self.render_mode {
                        RenderMode::Cpu => "CPU",
                        RenderMode::Gpu => "GPU",
                    });
                    ui.end_row();

                    ui.label("spatial idx:");
                    ui.label(&stats.index_label);
                    ui.end_row();
                });

            ui.separator();

            // Health hints — flag the common "renderer doesn't know
            // what's on screen" symptoms the user was worried about.
            if stats.in_viewport == stats.total && stats.total > 32 {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 80),
                    "⚠ viewport cull not active: all dobjects iterated.\n\
                     spatial index is stale or absent (zoom/pan/edit invalidates it).",
                );
            } else if stats.total > 0 && stats.drawn == 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 140, 140),
                    "⚠ 0 drawn — everything filtered (hidden layer? sub-pixel? off-screen?)",
                );
            } else if stats.in_viewport > 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(140, 220, 140),
                    "✓ renderer sees the viewport set",
                );
            }
        });
        self.process_dock_after_show("Screen Stats", ctx, resp);
        self.screen_stats_open = open;
    }

    fn render_layer_panel(&mut self, ctx: &egui::Context) {
        let mut open = self.layers_window_open;
        let win = egui::Window::new(format!("Layers ({})", self.doc.layers.len()))
            .open(&mut open)
            .default_pos(egui::pos2(10.0, 70.0))
            .default_size(egui::vec2(320.0, 480.0))
            .min_width(240.0)
            .resizable(true)
            .collapsible(true);
        let win = self.apply_dock_pos("Layers", ctx, win);
        let resp = win.show(ctx, |ui| {
                ui.separator();

                // ---- toolbar row: add + rename + delete -----------------
                ui.horizontal(|ui| {
                    if ui.button("➕ add").on_hover_text("Add a new layer").clicked() {
                        self.layer_name_counter += 1;
                        let mut name = format!("Layer{}", self.layer_name_counter);
                        // Bump the counter until the name is unique.
                        while self.doc.layers.find(&name).is_some() {
                            self.layer_name_counter += 1;
                            name = format!("Layer{}", self.layer_name_counter);
                        }
                        let id = self.doc.layers.add(Layer {
                            name,
                            ..Layer::layer_zero()
                        });
                        self.doc.layers.active = id;
                        self.history.push(format!(
                            "  + layer #{} (active)", id
                        ));
                    }
                    let active = self.doc.layers.active;
                    let can_delete = active != LayerTable::LAYER_ZERO;
                    ui.add_enabled_ui(can_delete, |ui| {
                        if ui.button("🗑 delete")
                            .on_hover_text("Delete the ACTIVE layer (Dobjects on it are NOT deleted; reassign them first)")
                            .clicked()
                        {
                            let name = self.doc.layers.get(active)
                                .map(|l| l.name.clone()).unwrap_or_default();
                            if self.doc.layers.remove(active) {
                                self.history.push(format!(
                                    "  - layer '{}' (#{}) deleted; active → 0", name, active
                                ));
                                // Reassign Dobjects on the removed layer to "0".
                                // Layers above `active` shifted down by 1; we
                                // need to remap their style.layer too.
                                for d in self.doc.dobjects.iter_mut() {
                                    if d.style.layer == active {
                                        d.style.layer = LayerTable::LAYER_ZERO;
                                    } else if d.style.layer > active {
                                        d.style.layer -= 1;
                                    }
                                }
                            }
                        }
                    });
                });
                ui.separator();

                // ---- header row -----------------------------------------
                egui::Grid::new("layer_header_grid")
                    .num_columns(5)
                    .spacing([6.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(""); // active
                        ui.label("👁");
                        ui.label("❄");
                        ui.label("🔒");
                        ui.label("name");
                        ui.end_row();
                    });

                // ---- one row per layer ----------------------------------
                let active = self.doc.layers.active;
                let mut new_active: Option<LayerId> = None;
                let mut rename_commit: Option<(LayerId, String)> = None;
                let mut rename_cancel = false;
                // Color edits can't intern into self.doc.truecolors directly
                // because the layer loop holds &mut self.doc.layers. Capture
                // (layer_id, packed_rgb) here; intern + assign after the loop.
                let mut color_change: Vec<(LayerId, u32)> = Vec::new();
                // Pick-button captures the layer id the user clicked; the
                // window-open assignment runs after the loop to stay clear
                // of the &mut self borrow chain.
                let mut pick_layer_color: Option<LayerId> = None;
                let n = self.doc.layers.len();

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        egui::Grid::new("layer_rows")
                            .num_columns(6)
                            .spacing([6.0, 4.0])
                            .striped(true)
                            .show(ui, |ui| {
                                for id in 0..(n as LayerId) {
                                    // Read-only first: pull the current
                                    // display color while no &mut layer
                                    // borrow exists, so we can dereference
                                    // self.doc.truecolors freely.
                                    let cur_color = self.doc.layers.get(id).map(|l| l.color);
                                    let rgb = match cur_color {
                                        Some(Color::TrueColorRef(idx)) => {
                                            let v = self.doc.truecolors.get(idx).unwrap_or(0xFFFFFF);
                                            (((v >> 16) & 0xFF) as u8,
                                             ((v >>  8) & 0xFF) as u8,
                                             ( v        & 0xFF) as u8)
                                        }
                                        Some(Color::Aci(i)) => aci_palette(i),
                                        _ => (255, 255, 255),
                                    };
                                    let layer = match self.doc.layers.get_mut(id) {
                                        Some(l) => l, None => continue,
                                    };

                                    // ----- active radio -------------------
                                    if ui.radio(id == active, "")
                                        .on_hover_text("Click to make this the active layer")
                                        .clicked()
                                    {
                                        new_active = Some(id);
                                    }

                                    // ----- visible toggle -----------------
                                    let mut v = layer.visible;
                                    if ui.checkbox(&mut v, "")
                                        .on_hover_text("Visible")
                                        .changed()
                                    {
                                        layer.visible = v;
                                    }

                                    // ----- freeze toggle ------------------
                                    let mut f = layer.frozen;
                                    if ui.checkbox(&mut f, "")
                                        .on_hover_text("Frozen (like hidden, also skipped on regen)")
                                        .changed()
                                    {
                                        layer.frozen = f;
                                    }

                                    // ----- lock toggle --------------------
                                    let mut l = layer.locked;
                                    if ui.checkbox(&mut l, "")
                                        .on_hover_text("Locked — Dobjects render but can't be selected")
                                        .changed()
                                    {
                                        layer.locked = l;
                                    }

                                    // ----- color swatch -------------------
                                    // Clickable swatch — opens the polar ACI
                                    // picker window for this layer. ACI is
                                    // the primary picker (see memo
                                    // `feedback_rust_cad_color_aci_primary`).
                                    let (swatch_rect, swatch_resp) = ui.allocate_exact_size(
                                        egui::vec2(22.0, 18.0), egui::Sense::click(),
                                    );
                                    ui.painter().rect_filled(
                                        swatch_rect, 2.0,
                                        egui::Color32::from_rgb(rgb.0, rgb.1, rgb.2),
                                    );
                                    ui.painter().rect_stroke(
                                        swatch_rect, 2.0,
                                        egui::Stroke::new(0.7, egui::Color32::from_rgb(70, 80, 95)),
                                    );
                                    if swatch_resp
                                        .on_hover_text("Click to pick an ACI color")
                                        .clicked()
                                    {
                                        pick_layer_color = Some(id);
                                    }

                                    // ----- name (click to rename) ---------
                                    if self.layer_rename == Some(id) {
                                        let resp = ui.text_edit_singleline(&mut self.layer_rename_buf);
                                        // First frame after rename activation —
                                        // steal focus from the always-listen
                                        // command line so keystrokes land here.
                                        if self.layer_rename_focus_pending {
                                            resp.request_focus();
                                            self.layer_rename_focus_pending = false;
                                        }
                                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                            rename_commit = Some((id, self.layer_rename_buf.clone()));
                                        }
                                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                            rename_cancel = true;
                                        }
                                    } else {
                                        let mut label = layer.name.clone();
                                        if id == LayerTable::LAYER_ZERO {
                                            label.push_str("  (reserved)");
                                        }
                                        let resp = ui.selectable_label(false, label);
                                        if resp.double_clicked() && id != LayerTable::LAYER_ZERO {
                                            self.layer_rename = Some(id);
                                            self.layer_rename_focus_pending = true;
                                            self.layer_rename_buf = layer.name.clone();
                                        }
                                    }
                                    ui.end_row();
                                }
                            });
                    });

                // Apply deferred mutations (made outside the borrow chain).
                if let Some(id) = new_active {
                    self.doc.layers.active = id;
                    self.history.push(format!("  active layer → #{}", id));
                }
                // Color edits captured during the layer loop — intern into
                // the truecolor table, then assign the ref. Two stages
                // because we couldn't borrow `doc.truecolors` mutably
                // inside the &mut layer scope.
                for (id, packed_rgb) in color_change.drain(..) {
                    let idx = self.doc.truecolors.intern(packed_rgb);
                    if let Some(l) = self.doc.layers.get_mut(id) {
                        l.color = Color::TrueColorRef(idx);
                    }
                }
                if let Some((id, new_name)) = rename_commit {
                    let trimmed = new_name.trim().to_string();
                    if !trimmed.is_empty() && self.doc.layers.rename(id, &trimmed) {
                        self.history.push(format!("  layer #{} renamed → '{}'", id, trimmed));
                    } else {
                        self.history.push(format!(
                            "  ! rename failed (empty or duplicate)"
                        ));
                    }
                    self.layer_rename = None;
                    self.layer_rename_buf.clear();
                }
                if rename_cancel {
                    self.layer_rename = None;
                    self.layer_rename_buf.clear();
                }
                if let Some(id) = pick_layer_color {
                    self.aci_pick_request = Some(AciPickRequest::Layer(id));
                }
            });
        self.process_dock_after_show("Layers", ctx, resp);
        self.layers_window_open = open;
    }

    // ===================================================================
    // Floating ACI color picker — the polar AutoRasm wheel.
    // ===================================================================
    //
    // One shared window serves every call site. The active request
    // (`aci_pick_request`) names who asked; when the user clicks a slot
    // in pick mode, the resulting ACI is written back to that target
    // and the window closes. Swap mode lets the user tune the wheel
    // arrangement; "Save mapping" persists the permutation to
    // `~/workspace/RUST_CAD/aci_mapping.json`.
    //
    // Spec: ~/workspace/RUST_CAD/ACI_Picker_UI.html
    fn render_aci_picker_window(&mut self, ctx: &egui::Context) {
        let Some(target) = self.aci_pick_request else { return };

        // Title — use the actual layer / dobject name so the user knows
        // which slot they're editing without having to remember its id.
        let title = match target {
            AciPickRequest::Layer(id) => {
                let name = self.doc.layers.get(id)
                    .map(|l| l.name.clone())
                    .unwrap_or_else(|| format!("layer #{}", id));
                format!("ACI color — {}", name)
            }
            AciPickRequest::Dobject(ix) => {
                let kind = self.doc.dobjects.get(ix)
                    .map(|d| dobject_kind_name(&d.geom).to_string())
                    .unwrap_or_else(|| "dobject".to_string());
                format!("ACI color — {} #{}", kind, ix)
            }
        };

        let mut open = true;
        let mut picked: Option<u8> = None;
        let mut do_save = false;
        // Reset the per-frame hover before rendering. Wheel + excluded
        // rows write to it as the cursor moves over them.
        self.aci_picker.hovered_aci = None;

        egui::Window::new(title)
            .id(egui::Id::new("aci_picker_window"))
            .open(&mut open)
            .default_size(egui::vec2(440.0, 640.0))
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                // Top control row — swap / reset / save.
                ui.horizontal(|ui| {
                    let swap_label = if self.aci_picker.swap_mode {
                        "Swap mode: ON"
                    } else { "Swap mode: OFF" };
                    if ui.selectable_label(self.aci_picker.swap_mode, swap_label)
                        .on_hover_text("Click two circles to swap their positions.\nApplies only to the main wheel.")
                        .clicked()
                    {
                        self.aci_picker.swap_mode = !self.aci_picker.swap_mode;
                    }
                    if ui.button("Reset layout").clicked() {
                        self.aci_picker.reset_to_default();
                    }
                    if ui.button("Save mapping").clicked() {
                        do_save = true;
                    }
                });

                ui.separator();

                // ---- Excluded row 1: named colors (ACI 1..=9) ----------
                ui.label("Named colors (ACI 1–9)");
                if let Some(aci) = self.aci_picker.excluded_row_ui(
                    ui, crate::aci_picker::EXCLUDED_NAMED.clone())
                {
                    picked = Some(aci);
                }
                ui.add_space(4.0);

                // ---- The wheel itself, centred horizontally ------------
                ui.vertical_centered(|ui| {
                    if let Some(aci) = self.aci_picker.wheel_ui(ui) {
                        picked = Some(aci);
                    }
                });

                ui.add_space(4.0);
                // ---- Excluded row 2: grayscale (ACI 250..=255) ---------
                ui.label("Grays (ACI 250–255)");
                if let Some(aci) = self.aci_picker.excluded_row_ui(
                    ui, crate::aci_picker::EXCLUDED_GRAY.clone())
                {
                    picked = Some(aci);
                }

                ui.separator();

                // ---- Manual ACI entry ----------------------------------
                ui.horizontal(|ui| {
                    ui.label("ACI #:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.aci_picker.manual_entry)
                            .desired_width(56.0)
                            .hint_text("0–255"),
                    );
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let set   = ui.button("Set").clicked();
                    if enter || set {
                        match self.aci_picker.manual_entry.trim().parse::<u16>() {
                            Ok(n) if n <= 255 => {
                                picked = Some(n as u8);
                                self.aci_picker.manual_entry.clear();
                            }
                            _ => {
                                // Leave the (bad) input visible so the
                                // user can correct it; no toast yet.
                            }
                        }
                    }
                });

                // ---- Hover readout -------------------------------------
                ui.add_space(2.0);
                let hover_label = if let Some(aci) = self.aci_picker.hovered_aci {
                    let (r, g, b) = aci_palette(aci);
                    format!(
                        "Hover: ACI {}   RGB {},{},{}   #{:02X}{:02X}{:02X}",
                        aci, r, g, b, r, g, b
                    )
                } else {
                    String::from("(hover a circle for ACI / RGB)")
                };
                ui.label(hover_label);
            });

        // Apply the pick to whoever asked.
        if let Some(aci) = picked {
            match target {
                AciPickRequest::Layer(id) => {
                    if let Some(l) = self.doc.layers.get_mut(id) {
                        l.color = Color::Aci(aci);
                    }
                    self.gpu_dirty = true;
                }
                AciPickRequest::Dobject(ix) => {
                    if let Some(d) = self.doc.dobjects.get_mut(ix) {
                        d.style.color = Color::Aci(aci);
                    }
                    self.gpu_dirty = true;
                }
            }
            self.aci_pick_request = None;
        }

        if do_save {
            match self.aci_picker.save_mapping(&aci_mapping_path()) {
                Ok(()) => self.history.push(format!(
                    "  ACI mapping saved → {}", aci_mapping_path().display()
                )),
                Err(e) => self.history.push(format!(
                    "  ! ACI mapping save failed: {}", e
                )),
            }
        }

        if !open {
            // User dismissed the window — abandon the pending request.
            self.aci_pick_request = None;
        }
    }

    // ===================================================================
    // Slice C — Pen palette
    // ===================================================================
    //
    // Egui-port of LibreCAD's `lc_penpalettewidget`. Each pen is a named
    // bundle of (color, linetype, lineweight). Clicking "Apply" rewrites
    // those three fields on every Dobject in the current selection.
    // Pens themselves are not persisted on Dobjects — they're a UI
    // shortcut for setting multiple style fields together.

    fn render_pen_palette(&mut self, ctx: &egui::Context) {
        let mut open = self.pens_window_open;
        let win = egui::Window::new(format!("Pens ({})", self.doc.pens.len()))
            .open(&mut open)
            .default_pos(egui::pos2(340.0, 70.0))
            .default_size(egui::vec2(280.0, 420.0))
            .min_width(220.0)
            .resizable(true)
            .collapsible(true);
        let win = self.apply_dock_pos("Pens", ctx, win);
        let resp = win.show(ctx, |ui| {
                ui.separator();

                if self.selection.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(180, 180, 200),
                        "(select dobjects first — `select` / Shift-click / window)",
                    );
                } else {
                    ui.label(format!("Apply to {} selected dobject(s):", self.selection.len()));
                }
                ui.add_space(4.0);

                let mut apply: Option<usize> = None;
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    egui::Grid::new("pen_rows")
                        .num_columns(3)
                        .spacing([8.0, 6.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for (i, pen) in self.doc.pens.pens.iter().enumerate() {
                                // ---- color swatch ----
                                let (r, g, b) = match pen.color {
                                    Color::TrueColorRef(idx) => {
                                        let v = self.doc.truecolors.get(idx).unwrap_or(0x808080);
                                        (((v >> 16) & 0xFF) as u8,
                                         ((v >>  8) & 0xFF) as u8,
                                         ( v        & 0xFF) as u8)
                                    }
                                    Color::Aci(idx) => aci_palette(idx),
                                    Color::ByLayer | Color::ByBlock => (180, 180, 200),
                                };
                                let arr = [r, g, b];
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(22.0, 18.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 2.0,
                                    egui::Color32::from_rgb(arr[0], arr[1], arr[2]));
                                ui.painter().rect_stroke(rect, 2.0,
                                    egui::Stroke::new(0.7, egui::Color32::from_rgb(70, 80, 95)));

                                // ---- name + description ----
                                ui.vertical(|ui| {
                                    ui.label(&pen.name);
                                    let lt = self.doc.linetypes.get(pen.linetype)
                                        .map(|l| l.name.as_str()).unwrap_or("?");
                                    let lw = match pen.lineweight {
                                        Lineweight::ByLayer    => "ByLayer".to_string(),
                                        Lineweight::ByBlock    => "ByBlock".to_string(),
                                        Lineweight::Default    => "Default".to_string(),
                                        Lineweight::Custom(mm) => format!("{:.2} mm", mm),
                                    };
                                    ui.small(format!("{} · {}", lt, lw));
                                });

                                // ---- apply button ----
                                let enabled = !self.selection.is_empty();
                                ui.add_enabled_ui(enabled, |ui| {
                                    if ui.button("apply").clicked() {
                                        apply = Some(i);
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });

                // Deferred mutation outside the borrow chain.
                if let Some(i) = apply {
                    if let Some(pen) = self.doc.pens.get(i) {
                        let (c, lt, lw) = (pen.color, pen.linetype, pen.lineweight);
                        let pen_name = pen.name.clone();
                        let count = self.selection.len();
                        for &idx in &self.selection {
                            if let Some(d) = self.doc.dobjects.get_mut(idx) {
                                d.style.color      = c;
                                d.style.linetype   = lt;
                                d.style.lineweight = lw;
                            }
                        }
                        self.history.push(format!(
                            "  pen '{}' applied to {} dobject(s)", pen_name, count
                        ));
                        self.gpu_dirty = true;
                    }
                }
            });
        self.process_dock_after_show("Pens", ctx, resp);
        self.pens_window_open = open;
    }

    // ===================================================================
    // Slice D — Entity Info panel
    // ===================================================================
    //
    // Egui-port of LibreCAD's `lc_quickinfowidget`. Two modes:
    //
    //   1. Single Dobject selected (via `self.selected` from the right
    //      panel) → full geometry breakdown + editable style fields.
    //   2. Multi-Dobject selection (`self.selection`) → summary counts +
    //      bulk-edit layer / visibility / color.

    fn render_info_panel(&mut self, ctx: &egui::Context) {
        let mut open = self.info_window_open;
        let win = egui::Window::new("Info / Properties")
            .open(&mut open)
            .default_pos(egui::pos2(640.0, 70.0))
            .default_size(egui::vec2(300.0, 520.0))
            .min_width(240.0)
            .resizable(true)
            .collapsible(true);
        let win = self.apply_dock_pos("Info / Properties", ctx, win);
        let resp = win.show(ctx, |ui| {
                ui.separator();

                // Decide which mode we're in.
                let n_sel = self.selection.len();
                let single = self.selected;

                match (single, n_sel) {
                    (None, 0) => {
                        ui.colored_label(
                            egui::Color32::from_rgb(180, 180, 200),
                            "(no selection — click a dobject in the right panel, or use `select`)",
                        );
                    }
                    (Some(i), _) if i < self.doc.dobjects.len() => {
                        self.render_info_single(ui, i);
                    }
                    (_, n) if n > 0 => {
                        self.render_info_multi(ui, n);
                    }
                    _ => {
                        ui.colored_label(
                            egui::Color32::from_rgb(180, 180, 200),
                            "(no selection)",
                        );
                    }
                }
            });
        self.process_dock_after_show("Info / Properties", ctx, resp);
        self.info_window_open = open;
    }

    fn render_info_single(&mut self, ui: &mut egui::Ui, idx: usize) {
        ui.label(format!("Single dobject #{}", idx));
        ui.separator();

        // ---- Geometry (read-only) ----
        let geom_desc = describe(&self.doc.dobjects[idx].geom);
        ui.label("Geometry");
        ui.add_space(2.0);
        ui.monospace(geom_desc);
        ui.add_space(8.0);

        // ---- Style (editable) ----
        ui.label("Style");
        ui.add_space(2.0);
        egui::Grid::new("info_single_style")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label("Handle"); ui.monospace(format!("0x{:X}", self.doc.dobjects[idx].handle));
                ui.end_row();

                // Layer — combo of all layers in the doc
                ui.label("Layer");
                let layer_id = self.doc.dobjects[idx].style.layer;
                let cur_name = self.doc.layers.get(layer_id)
                    .map(|l| l.name.clone()).unwrap_or_else(|| "?".into());
                let mut new_layer: Option<LayerId> = None;
                egui::ComboBox::new(("info_layer", idx), "")
                    .selected_text(cur_name)
                    .show_ui(ui, |ui| {
                        for lid in 0..(self.doc.layers.len() as LayerId) {
                            let name = match self.doc.layers.get(lid) {
                                Some(l) => l.name.clone(), None => continue,
                            };
                            if ui.selectable_label(lid == layer_id, name).clicked() {
                                new_layer = Some(lid);
                            }
                        }
                    });
                if let Some(lid) = new_layer {
                    self.doc.dobjects[idx].style.layer = lid;
                    self.gpu_dirty = true;
                }
                ui.end_row();

                // Visibility
                ui.label("Visible");
                let mut v = self.doc.dobjects[idx].style.visible;
                if ui.checkbox(&mut v, "").changed() {
                    self.doc.dobjects[idx].style.visible = v;
                }
                ui.end_row();

                // Color — ACI palette is the primary picker (8-bit);
                // TrueColor is the secondary fallback. See memo
                // `feedback_rust_cad_color_aci_primary`.
                ui.label("Color");
                let mut col = self.doc.dobjects[idx].style.color;
                let mut wants_pick = false;
                if aci_color_picker(ui, ("info_color", idx), &mut col,
                                    &mut self.doc.truecolors, &mut wants_pick) {
                    self.doc.dobjects[idx].style.color = col;
                    self.gpu_dirty = true;
                }
                if wants_pick {
                    self.aci_pick_request = Some(AciPickRequest::Dobject(idx));
                }
                ui.end_row();

                // Linetype (combo)
                ui.label("Linetype");
                let lt_id = self.doc.dobjects[idx].style.linetype;
                let cur = self.doc.linetypes.get(lt_id)
                    .map(|l| l.name.clone()).unwrap_or_else(|| "?".into());
                let mut new_lt: Option<u32> = None;
                egui::ComboBox::new(("info_lt", idx), "")
                    .selected_text(cur)
                    .show_ui(ui, |ui| {
                        for ltid in 0..(self.doc.linetypes.len() as u32) {
                            let name = match self.doc.linetypes.get(ltid) {
                                Some(l) => l.name.clone(), None => continue,
                            };
                            if ui.selectable_label(ltid == lt_id, name).clicked() {
                                new_lt = Some(ltid);
                            }
                        }
                    });
                if let Some(ltid) = new_lt {
                    self.doc.dobjects[idx].style.linetype = ltid;
                    self.gpu_dirty = true;
                }
                ui.end_row();

                // Linetype scale (editable)
                ui.label("Lt Scale");
                let mut scale = self.doc.dobjects[idx].style.linetype_scale;
                if ui.add(egui::DragValue::new(&mut scale).speed(0.05).range(0.01..=100.0)).changed() {
                    self.doc.dobjects[idx].style.linetype_scale = scale;
                }
                ui.end_row();

                // Lineweight (combo)
                ui.label("Lineweight");
                let lw = self.doc.dobjects[idx].style.lineweight;
                let lw_text = match lw {
                    Lineweight::ByLayer    => "ByLayer".to_string(),
                    Lineweight::ByBlock    => "ByBlock".to_string(),
                    Lineweight::Default    => "Default".to_string(),
                    Lineweight::Custom(mm) => format!("{:.2} mm", mm),
                };
                let mut new_lw: Option<Lineweight> = None;
                egui::ComboBox::new(("info_lw", idx), "")
                    .selected_text(lw_text)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(matches!(lw, Lineweight::ByLayer), "ByLayer").clicked() {
                            new_lw = Some(Lineweight::ByLayer);
                        }
                        if ui.selectable_label(matches!(lw, Lineweight::ByBlock), "ByBlock").clicked() {
                            new_lw = Some(Lineweight::ByBlock);
                        }
                        if ui.selectable_label(matches!(lw, Lineweight::Default), "Default").clicked() {
                            new_lw = Some(Lineweight::Default);
                        }
                        for mm in [0.05_f32, 0.13, 0.18, 0.25, 0.35, 0.5, 0.7, 1.0, 1.4, 2.0] {
                            let is_this = matches!(lw, Lineweight::Custom(x) if (x - mm).abs() < 1e-3);
                            if ui.selectable_label(is_this, format!("{:.2} mm", mm)).clicked() {
                                new_lw = Some(Lineweight::Custom(mm));
                            }
                        }
                    });
                if let Some(v) = new_lw {
                    self.doc.dobjects[idx].style.lineweight = v;
                }
                ui.end_row();
            });
    }

    fn render_info_multi(&mut self, ui: &mut egui::Ui, n: usize) {
        ui.label(format!("Multi-selection: {} dobject(s)", n));
        ui.separator();

        // Count by Geom variant.
        let (mut nl, mut nc, mut na, mut ne, mut nea, mut npt, mut npl, mut nh, mut nsp) =
            (0, 0, 0, 0, 0, 0, 0, 0, 0);
        for &i in &self.selection {
            if let Some(d) = self.doc.dobjects.get(i) {
                match &d.geom {
                    Geom::Line(_)       => nl  += 1,
                    Geom::Circle(_)     => nc  += 1,
                    Geom::Arc(_)        => na  += 1,
                    Geom::Ellipse(_)    => ne  += 1,
                    Geom::EllipseArc(_) => nea += 1,
                    Geom::Point(_)      => npt += 1,
                    Geom::Polyline(_)   => npl += 1,
                    Geom::Hatch(_)      => nh  += 1,
                    Geom::Spline(_)     => nsp += 1,
                }
            }
        }
        ui.monospace(format!(
            "  lines: {}\n  circles: {}\n  arcs: {}\n  ellipses: {}\n  \
             ellipse-arcs: {}\n  points: {}\n  polylines: {}\n  hatches: {}\n  splines: {}",
            nl, nc, na, ne, nea, npt, npl, nh, nsp
        ));
        ui.add_space(8.0);

        ui.label("Bulk edit");
        ui.add_space(2.0);
        egui::Grid::new("info_multi_bulk")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                // Move all selected to a chosen layer
                ui.label("Layer →");
                let mut chosen: Option<LayerId> = None;
                egui::ComboBox::new("info_multi_layer", "")
                    .selected_text("(set all…)")
                    .show_ui(ui, |ui| {
                        for lid in 0..(self.doc.layers.len() as LayerId) {
                            let name = match self.doc.layers.get(lid) {
                                Some(l) => l.name.clone(), None => continue,
                            };
                            if ui.selectable_label(false, name).clicked() {
                                chosen = Some(lid);
                            }
                        }
                    });
                if let Some(lid) = chosen {
                    let count = self.selection.len();
                    for &i in &self.selection {
                        if let Some(d) = self.doc.dobjects.get_mut(i) {
                            d.style.layer = lid;
                        }
                    }
                    let lname = self.doc.layers.get(lid)
                        .map(|l| l.name.clone()).unwrap_or_default();
                    self.history.push(format!(
                        "  {} dobject(s) moved to layer '{}'", count, lname
                    ));
                    self.gpu_dirty = true;
                }
                ui.end_row();

                // Visibility toggle for the whole selection
                ui.label("Visibility");
                ui.horizontal(|ui| {
                    if ui.small_button("show all").clicked() {
                        let count = self.selection.len();
                        for &i in &self.selection {
                            if let Some(d) = self.doc.dobjects.get_mut(i) {
                                d.style.visible = true;
                            }
                        }
                        self.history.push(format!("  shown: {} dobject(s)", count));
                        self.gpu_dirty = true;
                    }
                    if ui.small_button("hide all").clicked() {
                        let count = self.selection.len();
                        for &i in &self.selection {
                            if let Some(d) = self.doc.dobjects.get_mut(i) {
                                d.style.visible = false;
                            }
                        }
                        self.history.push(format!("  hidden: {} dobject(s)", count));
                        self.gpu_dirty = true;
                    }
                });
                ui.end_row();

                // ByLayer reset
                ui.label("Reset to");
                ui.horizontal(|ui| {
                    if ui.small_button("ByLayer (all)").clicked() {
                        let count = self.selection.len();
                        for &i in &self.selection {
                            if let Some(d) = self.doc.dobjects.get_mut(i) {
                                d.style.color      = Color::ByLayer;
                                d.style.linetype   = LinetypeTable::CONTINUOUS;
                                d.style.lineweight = Lineweight::ByLayer;
                            }
                        }
                        self.history.push(format!(
                            "  {} dobject(s) reset to ByLayer", count
                        ));
                        self.gpu_dirty = true;
                    }
                });
                ui.end_row();
            });
    }

    // ===================================================================
    // Slice H — File I/O (DXF for now; .rsm in Slice I)
    // ===================================================================

    fn do_open(&mut self, path: &str) {
        let lower = path.to_ascii_lowercase();
        let doc_result = if lower.ends_with(".dxf") {
            match std::fs::read_to_string(path) {
                Ok(text) => cad_io::dxf::read_dxf(&text),
                Err(e) => {
                    self.history.push(format!("  ! open '{}' failed: {}", path, e));
                    return;
                }
            }
        } else if lower.ends_with(".rsm") {
            match std::fs::read(path) {
                Ok(bytes) => cad_io::rsm::read_rsm(&bytes),
                Err(e) => {
                    self.history.push(format!("  ! open '{}' failed: {}", path, e));
                    return;
                }
            }
        } else {
            Err(format!("unknown extension on '{}': expected .dxf or .rsm", path))
        };
        match doc_result {
            Ok(doc) => {
                let n = doc.dobjects.len();
                let l = doc.layers.len();
                self.doc = doc;
                self.selection.clear();
                self.selection_prev.clear();
                self.selected = None;
                self.intersections.clear();
                self.index_dirty = true;
                self.gpu_dirty = true;
                self.history.push(format!(
                    "  opened '{}'  ({} dobject(s), {} layer(s))", path, n, l
                ));
            }
            Err(e) => self.history.push(format!("  ! open '{}': {}", path, e)),
        }
    }

    fn do_save(&mut self, path: &str) {
        let lower = path.to_ascii_lowercase();
        let bytes: Vec<u8> = if lower.ends_with(".dxf") {
            cad_io::dxf::write_dxf(&self.doc).into_bytes()
        } else if lower.ends_with(".rsm") {
            cad_io::rsm::write_rsm(&self.doc)
        } else {
            self.history.push(format!(
                "  ! save '{}': unknown extension (expected .dxf or .rsm)", path
            ));
            return;
        };
        match std::fs::write(path, &bytes) {
            Ok(()) => self.history.push(format!(
                "  saved '{}'  ({} bytes)", path, bytes.len()
            )),
            Err(e) => self.history.push(format!("  ! save '{}': {}", path, e)),
        }
    }

    // ===================================================================
    // Slice J — Editing operations
    // ===================================================================
    //
    // Each operation snapshots the Document before mutating so `undo` can
    // roll back. The snapshot stack is bounded (UNDO_STACK_CAP) — oldest
    // snapshots fall off when the stack is full.

    fn snapshot_doc(&mut self) {
        if self.undo_stack.len() >= UNDO_STACK_CAP {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(self.doc.clone());
        // A new editing op invalidates the redo branch — once you diverge
        // from the previously-redoable history you can't return to it.
        self.redo_stack.clear();
    }

    fn do_undo(&mut self) {
        match self.undo_stack.pop() {
            Some(prev) => {
                // Stash current state on the redo stack before restoring.
                if self.redo_stack.len() >= UNDO_STACK_CAP {
                    self.redo_stack.remove(0);
                }
                self.redo_stack.push(self.doc.clone());
                self.doc = prev;
                self.selection.clear();
                self.selected = None;
                self.intersections.clear();
                self.index_dirty = true;
                self.gpu_dirty = true;
                self.history.push(format!(
                    "  ↶ undo  (undo: {}  redo: {})",
                    self.undo_stack.len(), self.redo_stack.len()
                ));
            }
            None => self.history.push("  ! nothing to undo".into()),
        }
    }

    fn do_redo(&mut self) {
        match self.redo_stack.pop() {
            Some(next) => {
                // Stash current state back on the undo stack — symmetric.
                if self.undo_stack.len() >= UNDO_STACK_CAP {
                    self.undo_stack.remove(0);
                }
                self.undo_stack.push(self.doc.clone());
                self.doc = next;
                self.selection.clear();
                self.selected = None;
                self.intersections.clear();
                self.index_dirty = true;
                self.gpu_dirty = true;
                self.history.push(format!(
                    "  ↷ redo  (undo: {}  redo: {})",
                    self.undo_stack.len(), self.redo_stack.len()
                ));
            }
            None => self.history.push("  ! nothing to redo".into()),
        }
    }

    // ---- Slice K: matchprop / reverse / chlayer apply methods ----

    fn apply_matchprops(&mut self, source_idx: usize) {
        let Some(source) = self.doc.dobjects.get(source_idx) else {
            self.history.push("  ! matchprop: invalid source".into());
            return;
        };
        let src_style = source.style;
        if self.selection.is_empty() {
            self.history.push("  ! matchprop: empty basket".into());
            return;
        }
        self.snapshot_doc();
        let n = self.selection.len();
        for &i in &self.selection {
            if i == source_idx { continue; }   // self-match is a no-op
            if let Some(d) = self.doc.dobjects.get_mut(i) {
                d.style = src_style;
            }
        }
        self.history.push(format!(
            "  ✓ matchprop: style from #{} applied to {} dobject(s)",
            source_idx, n.saturating_sub(if self.selection.contains(&source_idx) {1} else {0})
        ));
        self.gpu_dirty = true;
    }

    fn apply_reverse(&mut self) {
        if self.selection.is_empty() {
            self.history.push("  ! reverse: empty basket".into());
            return;
        }
        self.snapshot_doc();
        let mut flipped = 0_usize;
        let mut noop = 0_usize;
        for &i in &self.selection {
            if let Some(d) = self.doc.dobjects.get_mut(i) {
                let direction_aware = matches!(d.geom,
                    Geom::Line(_) | Geom::Arc(_) | Geom::EllipseArc(_) | Geom::Polyline(_));
                if direction_aware {
                    d.geom = d.geom.reversed();
                    flipped += 1;
                } else {
                    noop += 1;
                }
            }
        }
        self.history.push(format!(
            "  ⇋ reverse: {} flipped, {} no-op (direction-agnostic)",
            flipped, noop
        ));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    // ---- Slice L apply methods ----

    fn apply_offset(&mut self, dist: f64, side: Vec2) {
        if self.selection.is_empty() { return; }
        self.snapshot_doc();
        let mut ok = 0usize;
        let mut errs: Vec<String> = Vec::new();
        let mut new_dobjects: Vec<DObject> = Vec::new();
        for &i in &self.selection {
            let Some(d) = self.doc.dobjects.get(i) else { continue };
            match d.offset(dist, side) {
                Ok(new_d) => { new_dobjects.push(new_d); ok += 1; }
                Err(msg)  => errs.push(format!("#{}: {}", i, msg)),
            }
        }
        for nd in new_dobjects { self.doc.push(nd); }
        self.history.push(format!(
            "  ⇉ offset {:.3} → {} new dobject(s); {} skipped",
            dist, ok, errs.len()));
        for e in errs.iter().take(3) { self.history.push(format!("    {}", e)); }
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    fn apply_lengthen(&mut self, delta: f64, near: Vec2) {
        if self.selection.is_empty() { return; }
        self.snapshot_doc();
        let mut ok = 0usize;
        let mut errs: Vec<String> = Vec::new();
        for &i in &self.selection {
            let Some(d) = self.doc.dobjects.get(i) else { continue };
            match d.geom.lengthened(delta, near) {
                Ok(new_geom) => {
                    if let Some(d_mut) = self.doc.dobjects.get_mut(i) {
                        d_mut.geom = new_geom;
                        ok += 1;
                    }
                }
                Err(msg) => errs.push(format!("#{}: {}", i, msg)),
            }
        }
        self.history.push(format!(
            "  ⟼ lengthen {:+.3} → {} ok, {} skipped",
            delta, ok, errs.len()));
        for e in errs.iter().take(3) { self.history.push(format!("    {}", e)); }
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    fn apply_break(&mut self, at: Vec2) {
        if self.selection.is_empty() { return; }
        self.snapshot_doc();
        // Process in reverse-index order so removals don't shift later indices.
        let mut sel = self.selection.clone();
        sel.sort_unstable();
        sel.dedup();
        let mut ok = 0usize;
        let mut errs: Vec<String> = Vec::new();
        let mut adds: Vec<(usize, DObject, DObject, cad_kernel::Style)> = Vec::new();
        for &i in sel.iter().rev() {
            let Some(d) = self.doc.dobjects.get(i) else { continue };
            match d.geom.split_at(at) {
                Ok((g1, g2)) => {
                    adds.push((i, DObject::new(g1), DObject::new(g2), d.style));
                    ok += 1;
                }
                Err(msg) => errs.push(format!("#{}: {}", i, msg)),
            }
        }
        // Apply: remove original at i, push both halves with preserved style.
        for (i, mut h1, mut h2, style) in adds {
            if i < self.doc.dobjects.len() { self.doc.dobjects.remove(i); }
            h1.style = style; h2.style = style;
            self.doc.dobjects.push(h1);
            self.doc.dobjects.push(h2);
        }
        self.selection.clear();
        self.selected = None;
        self.history.push(format!(
            "  ✂ break: {} split, {} skipped", ok, errs.len()));
        for e in errs.iter().take(3) { self.history.push(format!("    {}", e)); }
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    fn apply_align(&mut self, s1: Vec2, s2: Vec2, t1: Vec2, t2: Vec2) {
        if self.selection.is_empty() { return; }
        let src_len = s1.dist(s2);
        let tgt_len = t1.dist(t2);
        if src_len < EPS {
            self.history.push("  ! align: source points coincide".into());
            return;
        }
        if tgt_len < EPS {
            self.history.push("  ! align: target points coincide".into());
            return;
        }
        self.snapshot_doc();
        // Three-stage affine: translate s1→t1, rotate around t1 so the
        // (s1→s2) direction aligns with (t1→t2), then uniformly scale
        // around t1 so the source segment maps onto the target segment.
        // AutoCAD's ALIGN with two ref pairs does exactly this.
        let v = t1 - s1;
        let src_dir = (s2 - s1).angle();
        let tgt_dir = (t2 - t1).angle();
        let dtheta = (tgt_dir - src_dir).rem_euclid(std::f64::consts::TAU);
        let dtheta = if dtheta > std::f64::consts::PI {
            dtheta - std::f64::consts::TAU
        } else { dtheta };
        let scale = tgt_len / src_len;
        let n = self.selection.len();
        for &i in &self.selection {
            if let Some(d) = self.doc.dobjects.get_mut(i) {
                let translated = d.geom.translated(v);
                let rotated    = translated.rotated(t1, dtheta);
                d.geom         = rotated.scaled(t1, scale);
            }
        }
        self.history.push(format!(
            "  ⇲ align: {} dobject(s)  shifted ({:.2},{:.2})  rotated {:.2}°  scaled ×{:.3}  around ({:.2},{:.2})",
            n, v.x, v.y, dtheta.to_degrees(), scale, t1.x, t1.y));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    fn apply_stretch(&mut self, win_min: Vec2, win_max: Vec2, base: Vec2, dest: Vec2) {
        let v = dest - base;
        if v.len() < EPS { return; }
        self.snapshot_doc();
        let inside = |p: Vec2| -> bool {
            p.x >= win_min.x && p.x <= win_max.x
                && p.y >= win_min.y && p.y <= win_max.y
        };
        // Per-variant logic: move any vertex/center that lies inside the window.
        let mut touched = 0usize;
        for d in self.doc.dobjects.iter_mut() {
            let new_geom = match &d.geom {
                Geom::Line(l) => {
                    let na = if inside(l.a) { l.a + v } else { l.a };
                    let nb = if inside(l.b) { l.b + v } else { l.b };
                    if na != l.a || nb != l.b { touched += 1; }
                    Some(Geom::Line(Line { a: na, b: nb }))
                }
                Geom::Polyline(p) => {
                    let mut changed = false;
                    let new_verts: Vec<PolyVertex> = p.vertices.iter().map(|vt| {
                        if inside(vt.pos) {
                            changed = true;
                            PolyVertex { pos: vt.pos + v, bulge: vt.bulge }
                        } else { *vt }
                    }).collect();
                    if changed { touched += 1; }
                    Some(Geom::Polyline(Polyline { vertices: new_verts, closed: p.closed }))
                }
                // Translate as a whole iff the canonical "center" is inside.
                Geom::Circle(c) if inside(c.center) => {
                    touched += 1;
                    Some(Geom::Circle(Circle { center: c.center + v, radius: c.radius }))
                }
                Geom::Arc(a) if inside(a.center) => {
                    touched += 1;
                    Some(Geom::Arc(Arc {
                        center: a.center + v, radius: a.radius,
                        start_angle: a.start_angle, sweep_angle: a.sweep_angle,
                    }))
                }
                Geom::Ellipse(e) if inside(e.center) => {
                    touched += 1;
                    Some(Geom::Ellipse(Ellipse {
                        center: e.center + v, major: e.major, ratio: e.ratio,
                    }))
                }
                Geom::EllipseArc(ea) if inside(ea.ellipse.center) => {
                    touched += 1;
                    Some(Geom::EllipseArc(EllipseArc {
                        ellipse: Ellipse {
                            center: ea.ellipse.center + v,
                            major: ea.ellipse.major, ratio: ea.ellipse.ratio,
                        },
                        start_param: ea.start_param, sweep_param: ea.sweep_param,
                    }))
                }
                Geom::Point(pt) if inside(pt.location) => {
                    touched += 1;
                    Some(Geom::Point(Point {
                        location: pt.location + v, style: pt.style, size: pt.size,
                    }))
                }
                _ => None,
            };
            if let Some(g) = new_geom { d.geom = g; }
        }
        self.history.push(format!(
            "  ↔ stretch by ({:.2},{:.2})  touched {} dobject(s)",
            v.x, v.y, touched));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    // ---- Trim debug instrumentation ----

    /// Append one line to the trim debug log. Bounded so a runaway
    /// session can't blow memory; oldest entries drop first.
    fn trim_dbg<S: Into<String>>(&mut self, msg: S) {
        const CAP: usize = 1000;
        if self.trim_debug_log.len() >= CAP {
            self.trim_debug_log.drain(..200);
        }
        self.trim_debug_log.push(format!(
            "[{:>5}] {}", self.trim_debug_frame, msg.into()
        ));
    }

    /// Append a Hatch Debug Log entry. Cheap no-op when the window is
    /// closed so wide instrumentation doesn't waste memory in normal
    /// use. Per-frame counter prefix mirrors trim_dbg so cross-log
    /// timing matches.
    fn hatch_dbg<S: Into<String>>(&mut self, msg: S) {
        if !self.hatch_debug_open { return; }
        const CAP: usize = 1000;
        if self.hatch_debug_log.len() >= CAP {
            self.hatch_debug_log.drain(..200);
        }
        self.hatch_debug_log.push(format!(
            "[{:>5}] {}", self.trim_debug_frame, msg.into()
        ));
    }

    /// Auto-open the Hatch Debug window at the start of a fresh `hatch`
    /// command and stamp a session-start marker. Mirrors
    /// `trim_dbg_session_start`. Does NOT clear the prior log — the
    /// user can hit 🗑 Clear themselves if they want a fresh slate.
    fn hatch_dbg_session_start(&mut self) {
        self.hatch_debug_open = true;
        self.hatch_dbg(format!(
            "=== HATCH session START ===  doc.dobjects.len() = {}, selection.len() = {}",
            self.doc.dobjects.len(), self.selection.len()));
    }

    /// Wipe + open the debug log at the start of a fresh trim/extend
    /// session. Called from the command handlers.
    fn trim_dbg_session_start(&mut self, op: &str) {
        self.trim_debug_log.clear();
        self.trim_debug_open = true;
        self.trim_dbg(format!("=== {} session START ===", op));
        self.trim_dbg(format!(
            "  pre_op_selection = {:?}", self.pre_op_selection));
        self.trim_dbg(format!(
            "  doc.dobjects.len() = {}  EdgMod = {}",
            self.doc.dobjects.len(), self.env.EdgMod));
    }

    /// Format a Vec2 compactly for log entries.
    fn fmt_v(v: Vec2) -> String {
        format!("({:.3},{:.3})", v.x, v.y)
    }

    // ---- Slice M.1 / M.2: trim / extend apply methods ----

    /// Returns true iff the trim actually mutated the document. The
    /// caller uses this to gate cutter-list index patching — patching
    /// when the doc didn't change corrupts the list silently (the bug
    /// the user caught in commit ae54eef's debug log).
    fn apply_trim_pick(&mut self, cutters: &[usize], target_idx: usize, pick: Vec2) -> bool {
        // Snapshot ONCE per click so undo rolls back this single trim.
        self.snapshot_doc();
        let edge_mode = self.env.EdgMod;
        // Build cutter geoms, EXCLUDING the target itself — a dobject
        // never cuts itself (self-intersection = 0). This is what allows
        // trimming a cutter dobject in the basket: it's still a valid
        // target, just doesn't intersect with itself for cut math.
        let cutter_geoms: Vec<Geom> = cutters.iter()
            .filter(|&&i| i != target_idx)
            .filter_map(|&i| self.doc.dobjects.get(i).map(|d| d.geom.clone()))
            .collect();
        if cutter_geoms.is_empty() {
            // No OTHER dobjects to cut against → roll back, fail.
            if let Some(prev) = self.undo_stack.pop() { self.doc = prev; }
            self.history.push(format!(
                "  ! trim #{}: no other cutters available (target is the only candidate)",
                target_idx));
            return false;
        }
        let Some(target) = self.doc.dobjects.get(target_idx) else {
            if let Some(prev) = self.undo_stack.pop() { self.doc = prev; }
            return false;
        };
        let target_style = target.style;
        match target.geom.trim_at(&cutter_geoms, pick, edge_mode) {
            Ok(pieces) => {
                let n_pieces = pieces.len();
                self.doc.dobjects.remove(target_idx);
                for g in pieces {
                    let mut d = DObject::new(g);
                    d.style = target_style;
                    self.doc.push(d);
                }
                self.history.push(format!(
                    "  ✂ trim: #{} cut → {} piece(s) survive (EdgMod {})",
                    target_idx, n_pieces, if edge_mode {"ON"} else {"OFF"}));
                self.intersections.clear();
                self.index_dirty = true;
                self.gpu_dirty = true;
                true
            }
            Err(msg) => {
                if let Some(prev) = self.undo_stack.pop() { self.doc = prev; }
                // Surface the kernel error in BOTH history and the trim
                // debug log — the user needs the actual reason to diagnose
                // "trim silently fails" (the bug your 268-cutter log
                // exposed: every retry returned Err but the log only said
                // 'success=false', not which Err).
                let kind = match self.doc.dobjects.get(target_idx).map(|d| &d.geom) {
                    Some(Geom::Line(_))       => "Line",
                    Some(Geom::Circle(_))     => "Circle",
                    Some(Geom::Arc(_))        => "Arc",
                    Some(Geom::Ellipse(_))    => "Ellipse",
                    Some(Geom::EllipseArc(_)) => "EllipseArc",
                    Some(Geom::Polyline(_))   => "Polyline",
                    Some(Geom::Point(_))      => "Point",
                    Some(Geom::Hatch(_))      => "Hatch",
                    Some(Geom::Spline(_))     => "Spline",
                    None                      => "<gone>",
                };
                self.history.push(format!("  ! trim #{}: {}", target_idx, msg));
                self.trim_dbg(format!(
                    "  ! trim_at Err on #{} ({}): {}", target_idx, kind, msg));
                false
            }
        }
    }

    /// Returns true iff the extend actually mutated the document.
    fn apply_extend_pick(&mut self, bounds: &[usize], target_idx: usize, pick: Vec2) -> bool {
        self.snapshot_doc();
        let edge_mode = self.env.EdgMod;
        // Same self-exclusion rule as trim.
        let boundary_geoms: Vec<Geom> = bounds.iter()
            .filter(|&&i| i != target_idx)
            .filter_map(|&i| self.doc.dobjects.get(i).map(|d| d.geom.clone()))
            .collect();
        if boundary_geoms.is_empty() {
            if let Some(prev) = self.undo_stack.pop() { self.doc = prev; }
            self.history.push(format!(
                "  ! extend #{}: no other boundaries available", target_idx));
            return false;
        }
        let Some(target) = self.doc.dobjects.get(target_idx) else {
            if let Some(prev) = self.undo_stack.pop() { self.doc = prev; }
            return false;
        };
        match target.geom.extend_to(&boundary_geoms, pick, edge_mode) {
            Ok(new_geom) => {
                if let Some(d) = self.doc.dobjects.get_mut(target_idx) {
                    d.geom = new_geom;
                }
                self.history.push(format!(
                    "  ⟼ extend: #{} extended to boundary (EdgMod {})",
                    target_idx, if edge_mode {"ON"} else {"OFF"}));
                self.intersections.clear();
                self.index_dirty = true;
                self.gpu_dirty = true;
                true
            }
            Err(msg) => {
                if let Some(prev) = self.undo_stack.pop() { self.doc = prev; }
                self.history.push(format!("  ! extend #{}: {}", target_idx, msg));
                false
            }
        }
    }

    /// Re-issue the fillet prompt with the current radius, trim mode,
    /// and multiple-mode badges. Called after any sub-option toggle.
    fn refresh_fillet_prompt(&mut self) {
        let r = self.env.FltRad;
        let tm = if self.env.TrmMd { "trim" } else { "no-trim" };
        let mm = if self.fillet_multiple { ", multi" } else { "" };
        let phase = match self.fillet_state {
            FilletState::WaitingForFirst(_)  => "click FIRST line on SIDE to KEEP",
            FilletState::WaitingForSecond(..) => "click SECOND line",
            FilletState::Off => return,
        };
        self.set_prompt(format!(
            "fillet (r={}, {}{}): {}  [t=trim, m=multi, r=radius, Esc]",
            r, tm, mm, phase));
    }

    /// Re-issue the chamfer prompt — mirror of `refresh_fillet_prompt`.
    fn refresh_chamfer_prompt(&mut self) {
        let d1 = self.env.ChmDs1;
        let d2 = self.env.ChmDs2;
        let tm = if self.env.TrmMd { "trim" } else { "no-trim" };
        let mm = if self.chamfer_multiple { ", multi" } else { "" };
        let phase = match self.chamfer_state {
            ChamferState::WaitingForFirst(..)  => "click FIRST line",
            ChamferState::WaitingForSecond(..) => "click SECOND line",
            ChamferState::Off => return,
        };
        self.set_prompt(format!(
            "chamfer (d1={}, d2={}, {}{}): {}  [t=trim, m=multi, d=distance, Esc]",
            d1, d2, tm, mm, phase));
    }

    // ---------------------------------------------------------------------
    // Slice M.3 — Fillet (line-line). Two clicks; second click commits.
    // ---------------------------------------------------------------------
    fn apply_fillet(&mut self, r: f64, idx1: usize, pick1: Vec2,
                                  idx2: usize, pick2: Vec2) {
        if idx1 == idx2 {
            self.history.push("  ! fillet: same dobject clicked twice".into());
            return;
        }
        // Both must be Lines for v1.
        let Some(d1) = self.doc.dobjects.get(idx1) else { return; };
        let Some(d2) = self.doc.dobjects.get(idx2) else { return; };
        let (l1, l2) = match (&d1.geom, &d2.geom) {
            (Geom::Line(a), Geom::Line(b)) => (*a, *b),
            _ => {
                self.history.push(
                    "  ! fillet: v1 supports LINE + LINE only (line/arc + arc/arc deferred)".into());
                return;
            }
        };
        let style1 = d1.style;
        let style2 = d2.style;
        self.snapshot_doc();
        let trim = self.env.TrmMd;
        match cad_kernel::fillet_lines(&l1, pick1, &l2, pick2, r) {
            Ok(out) => {
                // Trim mode → replace originals with kernel's shortened
                // lines. No-trim mode → leave originals untouched, only
                // append the arc. See AutoCAD TRIMMODE behavior.
                if trim {
                    if let Some(d) = self.doc.dobjects.get_mut(idx1) { d.geom = out.g1_new; }
                    if let Some(d) = self.doc.dobjects.get_mut(idx2) { d.geom = out.g2_new; }
                }
                if let Some(arc) = out.arc {
                    let mut d = DObject::new(arc);
                    // Arc inherits style from the FIRST clicked line — same
                    // convention AutoCAD uses for the fillet entity.
                    d.style = style1;
                    let _ = style2;
                    self.doc.push(d);
                }
                self.history.push(format!(
                    "  ⌐ fillet ✓ r={} between #{} and #{} ({})",
                    r, idx1, idx2,
                    if trim {"trim"} else {"no-trim"}));
                self.intersections.clear();
                self.index_dirty = true;
                self.gpu_dirty = true;
            }
            Err(msg) => {
                if let Some(prev) = self.undo_stack.pop() { self.doc = prev; }
                self.history.push(format!("  ! fillet: {}", msg));
            }
        }
    }

    // ---------------------------------------------------------------------
    // Slice M.4 — Chamfer (line-line).
    // ---------------------------------------------------------------------
    fn apply_chamfer(&mut self, d1_dist: f64, d2_dist: f64,
                                idx1: usize, pick1: Vec2,
                                idx2: usize, pick2: Vec2) {
        if idx1 == idx2 {
            self.history.push("  ! chamfer: same dobject clicked twice".into());
            return;
        }
        let Some(da) = self.doc.dobjects.get(idx1) else { return; };
        let Some(db) = self.doc.dobjects.get(idx2) else { return; };
        let (l1, l2) = match (&da.geom, &db.geom) {
            (Geom::Line(a), Geom::Line(b)) => (*a, *b),
            _ => {
                self.history.push(
                    "  ! chamfer: v1 supports LINE + LINE only".into());
                return;
            }
        };
        let style1 = da.style;
        self.snapshot_doc();
        let trim = self.env.TrmMd;
        match cad_kernel::chamfer_lines(&l1, pick1, &l2, pick2, d1_dist, d2_dist) {
            Ok(out) => {
                if trim {
                    if let Some(d) = self.doc.dobjects.get_mut(idx1) { d.geom = out.g1_new; }
                    if let Some(d) = self.doc.dobjects.get_mut(idx2) { d.geom = out.g2_new; }
                }
                let mut bridge = DObject::new(out.bridge);
                bridge.style = style1;
                self.doc.push(bridge);
                self.history.push(format!(
                    "  ⌐ chamfer ✓ d=({}, {}) between #{} and #{} ({})",
                    d1_dist, d2_dist, idx1, idx2,
                    if trim {"trim"} else {"no-trim"}));
                self.intersections.clear();
                self.index_dirty = true;
                self.gpu_dirty = true;
            }
            Err(msg) => {
                if let Some(prev) = self.undo_stack.pop() { self.doc = prev; }
                self.history.push(format!("  ! chamfer: {}", msg));
            }
        }
    }

    // ---------------------------------------------------------------------
    // Slice M.5 — Join. Operates on `self.selection`. Three-pass merge in
    // the kernel; we remove consumed dobjects (descending) and append the
    // merged ones at the doc's tail, inheriting style from the lowest-
    // indexed contributor.
    // ---------------------------------------------------------------------
    fn apply_join(&mut self) {
        if self.selection.is_empty() {
            self.history.push("  ! join: empty basket".into());
            return;
        }
        let items: Vec<(usize, Geom)> = self.selection.iter()
            .filter_map(|&i| self.doc.dobjects.get(i).map(|d| (i, d.geom.clone())))
            .collect();
        if items.len() < 2 {
            self.history.push("  ! join: need ≥ 2 dobjects".into());
            return;
        }
        let out = cad_kernel::join_geoms(&items);
        if out.merged.is_empty() || out.consumed_indices.is_empty() {
            self.history.push(
                "  ! join: nothing in the basket could be merged (need collinear lines, concentric arcs, or a touching chain)".into());
            return;
        }
        self.snapshot_doc();
        // Inherit style from the LOWEST-indexed consumed dobject in each
        // merged piece — simple, deterministic, matches AutoCAD's "first
        // selected wins" convention.
        let inherit_style = out.consumed_indices.iter()
            .filter_map(|&i| self.doc.dobjects.get(i).map(|d| d.style))
            .next();
        // Remove consumed dobjects in DESCENDING index order so shifts
        // don't invalidate later removals.
        let mut to_remove: Vec<usize> = out.consumed_indices.clone();
        to_remove.sort_unstable_by(|a, b| b.cmp(a));
        for idx in &to_remove {
            if *idx < self.doc.dobjects.len() {
                self.doc.dobjects.remove(*idx);
            }
        }
        // Append merged geoms.
        for g in out.merged {
            let mut d = DObject::new(g);
            if let Some(s) = inherit_style { d.style = s; }
            self.doc.push(d);
        }
        // The selection's old indices are now stale — clear it.
        self.selection.clear();
        self.history.push(format!(
            "  ⧙ join ✓ {} dobject(s) merged → {} new piece(s)",
            to_remove.len(), self.doc.dobjects.len()));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    fn apply_chlayer(&mut self) {
        if self.selection.is_empty() {
            self.history.push("  ! chlayer: empty basket".into());
            return;
        }
        let target = self.doc.layers.active;
        let name = self.doc.layers.get(target)
            .map(|l| l.name.clone()).unwrap_or_else(|| "?".into());
        self.snapshot_doc();
        let n = self.selection.len();
        for &i in &self.selection {
            if let Some(d) = self.doc.dobjects.get_mut(i) {
                d.style.layer = target;
            }
        }
        self.history.push(format!(
            "  → chlayer: {} dobject(s) moved to active layer '{}'", n, name
        ));
        self.gpu_dirty = true;
    }

    /// Append translated copies of the current selection (`copy` op).
    fn apply_copy(&mut self, v: Vec2) {
        if v.x.abs() < EPS && v.y.abs() < EPS { return; }
        self.snapshot_doc();
        let copies: Vec<DObject> = self.selection.iter()
            .filter_map(|&i| self.doc.dobjects.get(i))
            .map(|d| {
                let g = d.geom.translated(v);
                let mut new = DObject::new(g);
                new.style = d.style;
                new
            })
            .collect();
        let n = copies.len();
        for c in copies { self.doc.push(c); }
        self.selection_prev = self.selection.clone();
        self.selection.clear();
        self.history.push(format!("  + copy: {} dobject(s) duplicated", n));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    /// Rotate the current selection in place by `angle` around `pivot`.
    fn apply_rotate(&mut self, pivot: Vec2, angle: f64) {
        if angle.abs() < EPS { return; }
        self.snapshot_doc();
        let n = self.selection.len();
        for &i in &self.selection {
            if let Some(d) = self.doc.dobjects.get_mut(i) {
                d.geom = d.geom.rotated(pivot, angle);
            }
        }
        self.history.push(format!(
            "  ⟳ rotate: {} dobject(s) by {:.2}° around ({:.2}, {:.2})",
            n, angle.to_degrees(), pivot.x, pivot.y
        ));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    /// Dispatch the rotate session's commit step: if `rotate_copy` is on,
    /// produce rotated COPIES (originals untouched) instead of modifying
    /// the selection in place. AutoCAD's `C` sub-command.
    fn apply_rotate_or_copy(&mut self, pivot: Vec2, angle: f64) {
        if angle.abs() < EPS { return; }
        if !self.rotate_copy {
            self.apply_rotate(pivot, angle);
            return;
        }
        self.snapshot_doc();
        let copies: Vec<DObject> = self.selection.iter().filter_map(|&i| {
            self.doc.dobjects.get(i).map(|d| {
                let mut copy = d.clone();
                copy.geom = copy.geom.rotated(pivot, angle);
                copy.handle = cad_kernel::next_handle();
                copy
            })
        }).collect();
        let n = copies.len();
        for c in copies { self.doc.push(c); }
        self.history.push(format!(
            "  ⟳+ rotate-copy: {} new dobject(s) at {:.2}° around ({:.2}, {:.2})",
            n, angle.to_degrees(), pivot.x, pivot.y));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    /// Scale the current selection in place by `factor` around `pivot`.
    fn apply_scale(&mut self, pivot: Vec2, factor: f64) {
        if (factor - 1.0).abs() < EPS || factor.abs() < EPS { return; }
        self.snapshot_doc();
        let n = self.selection.len();
        for &i in &self.selection {
            if let Some(d) = self.doc.dobjects.get_mut(i) {
                d.geom = d.geom.scaled(pivot, factor);
            }
        }
        self.history.push(format!(
            "  ⊕ scale: {} dobject(s) by {:.3}× around ({:.2}, {:.2})",
            n, factor, pivot.x, pivot.y
        ));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    /// Dispatch the scale session's commit step: if `scale_copy` is on,
    /// produce scaled COPIES (originals untouched). AutoCAD `C` sub-cmd.
    fn apply_scale_or_copy(&mut self, pivot: Vec2, factor: f64) {
        if (factor - 1.0).abs() < EPS || factor.abs() < EPS { return; }
        if !self.scale_copy {
            self.apply_scale(pivot, factor);
            return;
        }
        self.snapshot_doc();
        let copies: Vec<DObject> = self.selection.iter().filter_map(|&i| {
            self.doc.dobjects.get(i).map(|d| {
                let mut copy = d.clone();
                copy.geom = copy.geom.scaled(pivot, factor);
                copy.handle = cad_kernel::next_handle();
                copy
            })
        }).collect();
        let n = copies.len();
        for c in copies { self.doc.push(c); }
        self.history.push(format!(
            "  ⊕+ scale-copy: {} new dobject(s) by {:.3}× around ({:.2}, {:.2})",
            n, factor, pivot.x, pivot.y));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    /// Mirror the current selection in place across the axis A→B.
    fn apply_mirror(&mut self, a: Vec2, b: Vec2) {
        if a.dist(b) < EPS { return; }
        self.snapshot_doc();
        let n = self.selection.len();
        for &i in &self.selection {
            if let Some(d) = self.doc.dobjects.get_mut(i) {
                d.geom = d.geom.mirrored(a, b);
            }
        }
        self.history.push(format!(
            "  ⇄ mirror: {} dobject(s) across axis ({:.2},{:.2})–({:.2},{:.2})",
            n, a.x, a.y, b.x, b.y
        ));
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty = true;
    }

    fn add_dobject(&mut self, geom: Geom, origin: &str) {
        let d = describe(&geom);
        let i = self.doc.push(DObject::new(geom));
        self.history.push(format!(
            "  + #{} {}  [{}]", i, d, origin
        ));
        // No auto-recompute. Intersections are only computed when the user
        // presses an ∩ button — otherwise modifying dobjects silently
        // invalidates them. Index is now stale until next ensure_index().
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty   = true;
    }

    /// Rebuild the spatial index if it's missing or stale. Returns the build
    /// duration in milliseconds.
    fn ensure_index(&mut self) -> f64 {
        if !self.index_dirty && self.index.is_some() {
            return 0.0;
        }
        let t = std::time::Instant::now();
        let cs = UniformGrid::auto_cell_size(&self.doc.dobjects, 10.0);
        let g  = UniformGrid::build(&self.doc.dobjects, cs);
        let (cells_total, idx_entries, cell_size) = g.stats();
        self.index = Some(g);
        self.index_dirty = false;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let avg = if !self.doc.dobjects.is_empty() {
            idx_entries as f64 / self.doc.dobjects.len() as f64
        } else { 0.0 };
        self.index_label = format!(
            "{} ents · {}×{} cells (size {:.2}) · avg {:.1} cells/ent · built {:.1} ms",
            self.doc.dobjects.len(),
            (cells_total as f64).sqrt() as usize,
            (cells_total as f64).sqrt() as usize,
            cell_size, avg, ms,
        );
        self.history.push(format!("  index: {}", self.index_label));
        ms
    }

    fn recompute(&mut self) {
        let all: Vec<usize> = (0..self.doc.dobjects.len()).collect();
        self.intersect_indices(&all);
    }

    /// Run intersections on a chosen subset of dobject indices.
    fn intersect_indices(&mut self, idx: &[usize]) {
        self.intersections.clear();
        for a in 0..idx.len() {
            for b in (a + 1)..idx.len() {
                self.intersections.extend(intersect(
                    &self.doc.dobjects[idx[a]].geom,
                    &self.doc.dobjects[idx[b]].geom,
                ));
            }
        }
    }

    /// "Intersect view" — uses the spatial index to fetch candidate dobjects
    /// whose bbox intersects the viewport, then runs O(k²) pairwise on those.
    fn intersect_in_bbox(&mut self, v_min: Vec2, v_max: Vec2) {
        let build_ms = self.ensure_index();
        let t = std::time::Instant::now();

        let cands: Vec<usize> = self.index.as_ref()
            .map(|g| g.query_bbox(v_min, v_max)
                      .into_iter().map(|u| u as usize).collect())
            .unwrap_or_default();

        // Tight bbox cull on the candidates (grid gives loose cells).
        let mut filtered: Vec<usize> = cands.into_iter().filter(|&i| {
            let (emin, emax) = self.doc.dobjects[i].bbox();
            !(emax.x < v_min.x || emin.x > v_max.x
           || emax.y < v_min.y || emin.y > v_max.y)
        }).collect();
        let n = filtered.len();
        let pairs_est = n.saturating_mul(n.saturating_sub(1)) / 2;

        if pairs_est > PAIR_LIMIT {
            self.last_intersect_label = format!(
                "view: {} dobjects ({} pairs) > pair cap {} — zoom in",
                n, pairs_est, PAIR_LIMIT
            );
            self.history.push(format!("  intersect  {}", self.last_intersect_label));
            return;
        }
        filtered.sort_unstable();
        self.intersect_indices(&filtered);
        let calc_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.last_intersect_label = format!(
            "view: {} ents · {} pairs · {} hits · {:.1} ms{}",
            n, pairs_est, self.intersections.len(), calc_ms,
            if build_ms > 0.0 { format!(" (+{:.1} ms idx rebuild)", build_ms) } else { String::new() }
        );
        self.history.push(format!("  intersect  {}", self.last_intersect_label));
    }

    /// "Intersect near click" — uses the spatial index to fetch candidates
    /// inside the bbox of (click ± radius), then runs O(k²) pairwise on those.
    fn intersect_near(&mut self, click: Vec2, world_radius: f64) {
        let build_ms = self.ensure_index();
        let t = std::time::Instant::now();
        let r2 = world_radius * world_radius;

        let cands: Vec<usize> = self.index.as_ref()
            .map(|g| g.query_near(click, world_radius)
                      .into_iter().map(|u| u as usize).collect())
            .unwrap_or_default();

        let mut filtered: Vec<usize> = cands.into_iter().filter(|&i| {
            let (emin, emax) = self.doc.dobjects[i].bbox();
            let cx = click.x.clamp(emin.x, emax.x);
            let cy = click.y.clamp(emin.y, emax.y);
            let dx = click.x - cx;
            let dy = click.y - cy;
            dx * dx + dy * dy <= r2
        }).collect();
        let n = filtered.len();
        let pairs_est = n.saturating_mul(n.saturating_sub(1)) / 2;

        if pairs_est > PAIR_LIMIT {
            self.last_intersect_label = format!(
                "click: {} dobjects ({} pairs) > pair cap {} — shrink radius / zoom in",
                n, pairs_est, PAIR_LIMIT
            );
            self.history.push(format!("  intersect  {}", self.last_intersect_label));
            return;
        }
        filtered.sort_unstable();
        // Compute all hits in the candidate set, but keep only the single one
        // closest to the click point — that's what the user is actually
        // pointing at.
        let mut all_hits: Vec<Vec2> = Vec::new();
        for a in 0..filtered.len() {
            for b in (a + 1)..filtered.len() {
                all_hits.extend(intersect(
                    &self.doc.dobjects[filtered[a]].geom,
                    &self.doc.dobjects[filtered[b]].geom,
                ));
            }
        }
        self.intersections.clear();
        let total_hits = all_hits.len();
        if let Some(closest) = all_hits.into_iter().min_by(|p1, p2| {
            let d1 = (*p1 - click).len_sq();
            let d2 = (*p2 - click).len_sq();
            d1.partial_cmp(&d2).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            self.intersections.push(closest);
        }
        let calc_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.last_intersect_label = format!(
            "click ({:.1},{:.1}) r={:.1}: {} ents · {} pairs · {} hits (kept nearest) · {:.1} ms{}",
            click.x, click.y, world_radius,
            n, pairs_est, total_hits, calc_ms,
            if build_ms > 0.0 { format!(" (+{:.1} ms idx rebuild)", build_ms) } else { String::new() }
        );
        self.history.push(format!("  intersect  {}", self.last_intersect_label));
    }

    // ---- coordinate transforms ----------------------------------------

    fn w2s(&self, w: Vec2, rect: egui::Rect) -> egui::Pos2 {
        let c = rect.center();
        egui::pos2(
            c.x + (w.x as f32 + self.world_offset.x) * self.scale,
            c.y - (w.y as f32 + self.world_offset.y) * self.scale,
        )
    }

    fn s2w(&self, s: egui::Pos2, rect: egui::Rect) -> Vec2 {
        let c = rect.center();
        Vec2::new(
            ((s.x - c.x) / self.scale - self.world_offset.x) as f64,
            (-(s.y - c.y) / self.scale - self.world_offset.y) as f64,
        )
    }

    /// Live PLINE prompt — reflects current Line/Arc sub-mode + vertex
    /// count. AutoCAD bracket-list of options matches the spec the user
    /// provided. Phase-2 sub-options appear in the prompt so the user
    /// knows they're recognised even though they print "not yet wired"
    /// when typed.
    fn update_pline_prompt(&mut self) {
        let n = self.pending.len();
        let prompt = match (self.pline_mode, n) {
            (PlineMode::Line, 0) =>
                "pline: click FIRST vertex  [Esc=cancel]".to_string(),
            (PlineMode::Line, _) => format!(
                "pline ({} vert): click next  \
                [Arc / Halfwidth / Length / Undo / Width  |  Enter=finish, 'c' Enter=close]",
                n),
            (PlineMode::Arc,  _) => match self.pline_arc_sub {
                PlineArcSub::Normal => format!(
                    "pline·ARC ({} vert): click endpoint  \
                    [Angle / CEnter / Direction / Halfwidth / Line / Radius / Second / Undo / Width  |  Enter=finish, 'c' Enter=close]",
                    n),
                PlineArcSub::AwaitingSecondPt =>
                    "pline·ARC·SECOND: click a point ON the arc  [U=cancel sub-flow]".to_string(),
                PlineArcSub::AwaitingSecondPtEnd(_) =>
                    "pline·ARC·SECOND: click ENDPOINT  [U=cancel sub-flow]".to_string(),
            },
        };
        self.set_prompt(prompt);
    }

    /// Snap-only "phantom" DObject built from the in-progress polyline
    /// (vertices + bulges currently in `self.pending`). Returned when
    /// the pline tool is mid-flow with at least 2 vertices so that the
    /// snap engine can offer END/MID/CEN/etc snaps against vertices the
    /// user has just clicked but hasn't committed yet. NOT inserted in
    /// `self.doc.dobjects` — it's a fresh allocation per snap frame.
    fn pline_phantom_dobject(&self) -> Option<DObject> {
        if self.tool != Tool::Polyline { return None; }
        if self.pending.len() < 2 { return None; }
        let n = self.pending.len();
        let verts: Vec<PolyVertex> = (0..n).map(|i| {
            let bulge = if i + 1 < n {
                self.pending_bulges.get(i).copied().unwrap_or(0.0)
            } else { 0.0 };
            PolyVertex { pos: self.pending[i], bulge }
        }).collect();
        Some(Polyline { vertices: verts, closed: false }.into())
    }

    /// PLINE arc-mode helper: the exit tangent of the most recently
    /// captured segment, used to make the next arc tangent-continuous.
    /// Returns None if there's no previous segment (just the very first
    /// vertex captured) — callers fall back to a default direction.
    fn pline_previous_exit_tangent(&self) -> Option<Vec2> {
        let n = self.pending.len();
        if n < 2 { return None; }
        let a = self.pending[n - 2];
        let b = self.pending[n - 1];
        let chord = b - a;
        if chord.len() < EPS { return None; }
        // Bulge of the segment ENDING at the previous vertex.
        let bulge = self.pending_bulges.get(n - 2).copied().unwrap_or(0.0);
        let alpha = 2.0 * bulge.atan();
        // Rotate the chord by -alpha (CW by alpha) to get the exit
        // tangent at the end of the segment. For a straight segment
        // (bulge = 0) this is the chord direction unchanged.
        let c = (-alpha).cos();
        let s = (-alpha).sin();
        let rotated = Vec2::new(
            chord.x * c - chord.y * s,
            chord.x * s + chord.y * c,
        );
        let len = rotated.len();
        if len < EPS { None } else { Some(rotated / len) }
    }

    /// Pack the in-progress pline positions + per-segment bulges into a
    /// `Vec<PolyVertex>` ready for the kernel, and reset all transient
    /// pline state. `closed` selects whether the last bulge slot
    /// (segment from final vertex back to first) is set; it stays 0 in
    /// the MVP since `c`/close was a Line-mode action.
    fn drain_pline_pending(&mut self, closed: bool) -> Vec<PolyVertex> {
        let n = self.pending.len();
        let mut verts = Vec::with_capacity(n);
        for (i, p) in self.pending.drain(..).enumerate() {
            // bulge[i] is the segment from vertex i to vertex i+1. For
            // an open polyline only i < n-1 are meaningful; for closed
            // the (n-1)-th slot is the closing segment, defaulting to 0.
            let bulge = if i + 1 < n {
                self.pending_bulges.get(i).copied().unwrap_or(0.0)
            } else if closed {
                0.0   // closing segment — Line mode for now
            } else {
                0.0
            };
            verts.push(PolyVertex { pos: p, bulge });
        }
        self.pending_bulges.clear();
        self.pline_mode = PlineMode::Line;
        self.pline_arc_sub = PlineArcSub::Normal;
        verts
    }

    /// Tessellate one polyline-preview segment from `a` to `b` with the
    /// given bulge and paint it. Straight segment (bulge ≈ 0) renders as
    /// a single line_segment; non-zero bulge tessellates the arc into
    /// short chords. Used both for committed-segment preview and for
    /// the rubber-band to the cursor while in Arc mode.
    fn draw_pline_preview_segment(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        a: Vec2, b: Vec2, bulge: f64,
        stroke: egui::Stroke,
    ) {
        if bulge.abs() < 1e-9 {
            painter.line_segment([self.w2s(a, rect), self.w2s(b, rect)], stroke);
            return;
        }
        let chord = b - a;
        let chord_len = chord.len();
        if chord_len < EPS {
            painter.line_segment([self.w2s(a, rect), self.w2s(b, rect)], stroke);
            return;
        }
        // theta = 4 * atan(bulge) is the (signed) included angle.
        let theta = 4.0 * bulge.atan();
        let half = theta * 0.5;
        // radius r = chord / (2 * sin(half)); sin(half) shares sign with bulge.
        let r = chord_len / (2.0 * half.sin().abs());
        // Centre offset from chord midpoint along the chord's perpendicular.
        // The sagitta (mid-deviation) is r - r*cos(half) = r * (1 - cos(half)),
        // signed by bulge so positive-bulge arcs sit on the LEFT of the chord
        // when travelling from a to b.
        let chord_hat = chord / chord_len;
        let perp = Vec2::new(-chord_hat.y, chord_hat.x);   // CCW perp
        let mid  = (a + b) * 0.5;
        let centre_off = r * half.cos();
        // half positive (bulge > 0): centre is to the LEFT of the chord
        //                            (in the +perp direction)
        // half negative (bulge < 0): centre is to the RIGHT (-perp)
        let centre = mid + perp * (if bulge > 0.0 { centre_off } else { -centre_off });
        // Angles from centre to a, b. CCW sweep selected by sign of bulge.
        let start_ang = (a - centre).angle();
        let end_ang   = (b - centre).angle();
        let sweep = if bulge > 0.0 {
            (end_ang - start_ang).rem_euclid(std::f64::consts::TAU)
        } else {
            -((start_ang - end_ang).rem_euclid(std::f64::consts::TAU))
        };
        // Tessellation density grows with screen size of the arc.
        let arc_len_px = (r as f32 * self.scale) * sweep.abs() as f32;
        let n = (arc_len_px * 0.4).clamp(6.0, 256.0) as usize;
        let mut pts = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let ang = start_ang + sweep * t;
            let p = centre + Vec2::new(r * ang.cos(), r * ang.sin());
            pts.push(self.w2s(p, rect));
        }
        painter.add(egui::Shape::line(pts, stroke));
    }

    /// AutoCAD bulge for a tangent-continuous arc from the current last
    /// pending vertex to `end`. `bulge = tan(alpha / 2)` where alpha is
    /// the signed (CCW positive) angle from the chord direction to the
    /// start tangent.
    fn pline_arc_bulge_to(&self, end: Vec2) -> f64 {
        let n = self.pending.len();
        if n == 0 { return 0.0; }
        let start = self.pending[n - 1];
        let chord = end - start;
        if chord.len() < EPS { return 0.0; }
        // Default tangent: previous-segment exit; fall back to horizontal
        // (X+) when there's only the start vertex.
        let tangent = self.pline_previous_exit_tangent()
            .unwrap_or(Vec2::new(1.0, 0.0));
        // Signed angle from chord to tangent, CCW positive.
        let cross = chord.x * tangent.y - chord.y * tangent.x;
        let dot   = chord.x * tangent.x + chord.y * tangent.y;
        let alpha = cross.atan2(dot);
        (alpha / 2.0).tan()
    }

    /// The "from" point that ortho (lock drafting orientation) constrains
    /// the cursor against — the most recent locked-in point in whatever
    /// command is active. None means ortho has no anchor and therefore
    /// no effect this frame (e.g. waiting for the first endpoint of a
    /// line, or no active command at all).
    fn ortho_anchor(&self) -> Option<Vec2> {
        if let MoveState::WaitingForDest(base) = self.move_state { return Some(base); }
        if let CopyState::WaitingForDest(base) = self.copy_state { return Some(base); }
        if let MirrorState::WaitingForB(a)    = self.mirror_state { return Some(a); }
        if let StretchState::WaitingForDest(_, _, base) = self.stretch_state { return Some(base); }
        // Polyline / Line / Arc draw tools: the last captured point is
        // the ortho anchor for the next click.
        if matches!(self.tool, Tool::Line | Tool::Polyline | Tool::Spline | Tool::Arc | Tool::Ellipse | Tool::EllipseArc) {
            if let Some(p) = self.pending.last().copied() { return Some(p); }
        }
        None
    }

    /// Apply ortho + grid-snap constraints to a raw world position. Order:
    ///   1. Ortho first if enabled AND an anchor exists — projects onto
    ///      whichever axis from the anchor is closer (pure horizontal or
    ///      vertical move).
    ///   2. Grid-snap second if enabled — rounds to the nearest
    ///      `GrdSpc` multiple in both axes.
    /// Object-snap is NOT handled here; callers apply it BEFORE this so
    /// osnap always wins over both ortho and grid (the AutoCAD priority).
    fn apply_constraints(&self, raw: Vec2) -> Vec2 {
        let mut p = raw;
        if self.env.OrtEnb {
            if let Some(a) = self.ortho_anchor() {
                let dx = (p.x - a.x).abs();
                let dy = (p.y - a.y).abs();
                if dx >= dy { p.y = a.y; } else { p.x = a.x; }
            }
        }
        if self.env.GrdSnp && self.env.GrdSpc > 0.0 {
            let s = self.env.GrdSpc;
            p.x = (p.x / s).round() * s;
            p.y = (p.y / s).round() * s;
        }
        p
    }

    /// World position of the cursor with all current constraints applied
    /// in AutoCAD priority: osnap > ortho > grid-snap > raw. Used both
    /// for click-capture and for live-preview rendering so the user
    /// sees exactly what the click will produce.
    fn cursor_world_constrained(
        &self,
        screen_pos: Option<egui::Pos2>,
        rect: egui::Rect,
        snap_hit: Option<Vec2>,
    ) -> Option<Vec2> {
        if let Some(w) = snap_hit { return Some(w); }    // osnap wins
        screen_pos.map(|p| self.apply_constraints(self.s2w(p, rect)))
    }

    /// Find the dobject nearest to a world point, within `tol_world`. Uses the
    /// spatial index when available so it's cheap even at millions of dobjects.
    fn nearest_entity_under(&self, w: Vec2, tol_world: f64) -> Option<usize> {
        let cands: Vec<usize> = if let (Some(g), false) = (self.index.as_ref(), self.index_dirty) {
            g.query_near(w, tol_world).into_iter().map(|u| u as usize).collect()
        } else {
            (0..self.doc.dobjects.len()).collect()
        };
        let mut best: Option<(usize, f64)> = None;
        for i in cands {
            let d = self.doc.dobjects[i].distance_to_point(w);
            if d < tol_world {
                if best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
        }
        best.map(|(i, _)| i)
    }

    // ---- interactive draw: finalise dobject from clicked points ---------

    fn try_finalise(&mut self) {
        match (self.tool, self.pending.len()) {
            (Tool::Line, 2) => {
                let g = Geom::Line(Line { a: self.pending[0], b: self.pending[1] });
                self.pending.clear();
                self.add_dobject(g, "canvas");
            }
            (Tool::Point, 1) => {
                let loc = self.pending[0];
                self.pending.clear();
                self.add_dobject(Geom::Point(Point {
                    location: loc, style: 0, size: 0.0,
                }), "canvas");
            }
            // Polyline never finalises via the click-count path — it
            // accumulates clicks forever and finalises when the user
            // presses Enter (see `finish_polyline`).
            (Tool::Circle, 2) => {
                let c = self.pending[0];
                let p = self.pending[1];
                let r = c.dist(p);
                self.pending.clear();
                if r > EPS {
                    self.add_dobject(Geom::Circle(Circle { center: c, radius: r }), "canvas");
                } else {
                    self.history.push("  ! circle has zero radius".into());
                }
            }
            (Tool::Ellipse, 3) => {
                // 1) centre  2) end of major axis  3) any point on the minor
                // side; semi-minor is the perpendicular distance from the
                // major-axis line to that third click.
                let c   = self.pending[0];
                let me  = self.pending[1];
                let mp  = self.pending[2];
                self.pending.clear();
                let major = me - c;
                if major.len() < EPS {
                    self.history.push("  ! ellipse: zero-length major axis".into());
                    return;
                }
                // Project (mp - c) onto the minor-axis direction.
                let v_hat = major.normalized().perp();
                let semi_minor = (mp - c).dot(v_hat).abs();
                match ellipse_center_major_minor(c, me, semi_minor) {
                    Some(el) => self.add_dobject(Geom::Ellipse(el), "canvas"),
                    None => self.history.push(
                        "  ! ellipse: degenerate inputs (zero major or minor)".into()),
                }
            }
            (Tool::EllipseArc, 5) => {
                // 1)centre  2)major_end  3)minor side  4)start point  5)end point
                let c   = self.pending[0];
                let me  = self.pending[1];
                let mp  = self.pending[2];
                let sp  = self.pending[3];
                let ep  = self.pending[4];
                self.pending.clear();
                let major = me - c;
                if major.len() < EPS {
                    self.history.push("  ! ellipse arc: zero-length major axis".into());
                    return;
                }
                let v_hat = major.normalized().perp();
                let semi_minor = (mp - c).dot(v_hat).abs();
                let Some(el) = ellipse_center_major_minor(c, me, semi_minor) else {
                    self.history.push("  ! ellipse arc: degenerate inputs".into());
                    return;
                };
                // Convert start/end click points to parameters on the ellipse
                // (nearest_param projects them onto the curve, so the user
                // can click roughly near the ellipse and the system snaps the
                // bounds to it).
                let t_start = el.nearest_param(sp);
                let t_end   = el.nearest_param(ep);
                let sweep_raw = (t_end - t_start).rem_euclid(std::f64::consts::TAU);
                let sweep = if sweep_raw < 1e-6 { std::f64::consts::TAU } else { sweep_raw };
                self.add_dobject(Geom::EllipseArc(EllipseArc {
                    ellipse:     el,
                    start_param: t_start.rem_euclid(std::f64::consts::TAU),
                    sweep_param: sweep,
                }), "canvas");
            }
            (Tool::Arc, n) if n >= self.arc_method.click_count() => {
                let needed = self.arc_method.click_count();
                let pts: Vec<Vec2> = self.pending.drain(..needed).collect();
                let arc_opt = match self.arc_method {
                    ArcMethod::ThreePoints =>
                        arc_three_points(pts[0], pts[1], pts[2]),
                    // S,C,E: 1st = start, 2nd = center, 3rd = end → reorder for kernel
                    ArcMethod::StartCenterEnd =>
                        arc_center_start_end(pts[1], pts[0], pts[2]),
                    // C,S,E: 1st = center, 2nd = start, 3rd = end → kernel signature
                    ArcMethod::CenterStartEnd =>
                        arc_center_start_end(pts[0], pts[1], pts[2]),
                    _ => {
                        self.history.push(format!(
                            "  ! arc method '{}' not implemented yet",
                            self.arc_method.name()
                        ));
                        return;
                    }
                };
                let tag = format!("canvas ({})", self.arc_method.name());
                match arc_opt {
                    Some(arc) => self.add_dobject(Geom::Arc(arc), &tag),
                    None => self.history.push(
                        "  ! could not build arc (collinear / zero radius)".into()
                    ),
                }
            }
            _ => {}
        }
    }

    // ---- array generator -----------------------------------------------

    fn generate_array(&mut self) {
        // Multi-source: iterate `self.selection` (the standard basket).
        // Every grid cell instantiates a copy of every source, offset
        // by that cell's (c·dx, r·dy). The cell at (0, 0) is the
        // source itself — skipped so we don't duplicate the originals.
        let sources: Vec<DObject> = self.selection.iter()
            .filter_map(|&i| self.doc.dobjects.get(i).cloned())
            .collect();
        if sources.is_empty() {
            self.history.push("  ! array: no sources selected".into());
            return;
        }
        let cols = self.array_cols.max(1);
        let rows = self.array_rows.max(1);
        let dx   = self.array_dx;
        let dy   = self.array_dy;
        let cells = cols * rows;
        let new_dobjects = cells.saturating_sub(1) * sources.len();

        self.doc.dobjects.reserve(new_dobjects);
        for r in 0..rows {
            for c in 0..cols {
                if r == 0 && c == 0 { continue; }   // skip the source cell
                let off = Vec2::new(c as f64 * dx, r as f64 * dy);
                for s in &sources {
                    self.doc.dobjects.push(s.translated(off));
                }
            }
        }
        let new_total = self.doc.dobjects.len();
        self.intersections.clear();
        self.index_dirty = true;
        self.gpu_dirty   = true;
        self.history.push(format!(
            "  + array: {} cells × {} source(s) = {} new → {} total dobjects",
            cells, sources.len(), new_dobjects, new_total,
        ));
        self.ensure_index();
    }
}

/// Short one-letter badge string for the active osnap kinds, shown on the
/// toolbar button so the user can see at a glance what's enabled.
fn active_snap_letters(s: SnapSet) -> String {
    let mut buf = String::with_capacity(7);
    for k in SnapKind::ALL {
        if s.is_enabled(k) { buf.push(k.name().chars().next().unwrap()); }
    }
    if buf.is_empty() { buf.push('—'); }
    buf
}

// ---- Settings-window widgets ----------------------------------------------
//
// Each row pairs the cryptic field name (bold, monospace) with a plain-
// English description and a type-appropriate input. The cryptic name is
// what gets persisted; the description is just for humans.

fn env_row(ui: &mut egui::Ui, key: &str, desc: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([70.0, 18.0],
            egui::Label::new(egui::RichText::new(key).monospace().strong()));
        ui.add_sized([200.0, 18.0],
            egui::Label::new(egui::RichText::new(desc).small()));
        body(ui);
    });
}

fn env_bool(ui: &mut egui::Ui, key: &str, desc: &str, v: &mut bool) {
    env_row(ui, key, desc, |ui| { ui.checkbox(v, ""); });
}

fn env_u8(ui: &mut egui::Ui, key: &str, desc: &str, v: &mut u8, lo: u8, hi: u8) {
    env_row(ui, key, desc, |ui| {
        ui.add(egui::Slider::new(v, lo..=hi));
    });
}

fn env_u8_choice(ui: &mut egui::Ui, key: &str, desc: &str, v: &mut u8, choices: &[&str]) {
    env_row(ui, key, desc, |ui| {
        let sel = (*v as usize).min(choices.len().saturating_sub(1));
        egui::ComboBox::from_id_salt(key)
            .selected_text(choices.get(sel).copied().unwrap_or(""))
            .show_ui(ui, |ui| {
                for (i, label) in choices.iter().enumerate() {
                    ui.selectable_value(v, i as u8, *label);
                }
            });
    });
}

fn env_color(ui: &mut egui::Ui, key: &str, desc: &str, v: &mut u32) {
    env_row(ui, key, desc, |ui| {
        let mut rgb = [
            ((*v >> 16) & 0xFF) as u8,
            ((*v >> 8)  & 0xFF) as u8,
            ( *v        & 0xFF) as u8,
        ];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *v = ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | (rgb[2] as u32);
        }
        ui.monospace(format!("0x{:06X}", *v));
    });
}

fn env_text(ui: &mut egui::Ui, key: &str, desc: &str, v: &mut String) {
    env_row(ui, key, desc, |ui| {
        ui.add(egui::TextEdit::singleline(v).desired_width(180.0));
    });
}

/// Live preview of those settings that have a visible effect — currently
/// snap target / pickbox / crosshair (sizes shown around a virtual cursor)
/// and the grip colour + size on a sample line. Other settings (dialog
/// modes, xref load mode) have no meaningful visual preview and are
/// skipped here.
fn draw_settings_preview(ui: &mut egui::Ui, env: &UserEnv) {
    let u_to_col = |rgb: u32| egui::Color32::from_rgb(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >>  8) & 0xFF) as u8,
        ( rgb        & 0xFF) as u8,
    );
    let bg     = egui::Color32::from_rgb(18, 22, 28);
    let edge   = egui::Color32::from_rgb(70, 80, 95);
    let dobj   = egui::Color32::from_rgb(170, 200, 230);
    let cursor_col = egui::Color32::from_rgb(255, 220, 100);

    // ---- Panel 1: snap & picking ----
    ui.label(egui::RichText::new("Snap & picking").monospace());
    let (resp1, p1) = ui.allocate_painter(
        egui::vec2(240.0, 200.0), egui::Sense::hover());
    let r1 = resp1.rect;
    p1.rect_filled(r1, 2.0, bg);
    p1.rect_stroke(r1, 2.0, egui::Stroke::new(1.0, edge));

    // Cursor sits at the panel centre; draw crosshair lines spanning
    // CrsHrS% of the panel's shorter side, pickbox of PkBxSz, snap circle
    // of SpTGSZ.
    let c = r1.center();
    let short = r1.width().min(r1.height());
    let hair = short * (env.CrsHrS as f32 / 100.0) * 0.5;
    let pen_hair = egui::Stroke::new(1.0, cursor_col.gamma_multiply(0.6));
    p1.line_segment([egui::pos2(c.x - hair, c.y), egui::pos2(c.x + hair, c.y)], pen_hair);
    p1.line_segment([egui::pos2(c.x, c.y - hair), egui::pos2(c.x, c.y + hair)], pen_hair);

    // Snap target radius (SpTGSZ) — solid cyan circle around cursor
    p1.circle_stroke(c, env.SpTGSZ as f32,
        egui::Stroke::new(1.2, egui::Color32::from_rgb(80, 230, 240)));
    // Pickbox (PkBxSz) — yellow square around cursor
    let half = env.PkBxSz as f32 * 0.5;
    p1.rect_stroke(
        egui::Rect::from_min_max(
            egui::pos2(c.x - half, c.y - half),
            egui::pos2(c.x + half, c.y + half),
        ),
        0.0, egui::Stroke::new(1.0, cursor_col));
    // Tiny labels next to each visual
    p1.text(egui::pos2(c.x + hair + 4.0, c.y),
        egui::Align2::LEFT_CENTER, format!("CrsHrS={}%", env.CrsHrS),
        egui::FontId::monospace(10.0), pen_hair.color);
    p1.text(egui::pos2(c.x + env.SpTGSZ as f32 + 4.0, c.y + env.SpTGSZ as f32 + 4.0),
        egui::Align2::LEFT_TOP, format!("SpTGSZ={}", env.SpTGSZ),
        egui::FontId::monospace(10.0), egui::Color32::from_rgb(80, 230, 240));
    p1.text(egui::pos2(c.x + half + 4.0, c.y - half - 2.0),
        egui::Align2::LEFT_BOTTOM, format!("PkBxSz={}", env.PkBxSz),
        egui::FontId::monospace(10.0), cursor_col);

    ui.add_space(8.0);

    // ---- Panel 2: grips + highlight + selection preview ----
    ui.label(egui::RichText::new("Grips & highlight").monospace());
    let (resp2, p2) = ui.allocate_painter(
        egui::vec2(240.0, 180.0), egui::Sense::hover());
    let r2 = resp2.rect;
    p2.rect_filled(r2, 2.0, bg);
    p2.rect_stroke(r2, 2.0, egui::Stroke::new(1.0, edge));

    // Sample line — drawn highlighted IF HltSel is on, otherwise normal.
    let line_a = egui::pos2(r2.left() + 30.0, r2.top() + 50.0);
    let line_b = egui::pos2(r2.right() - 30.0, r2.bottom() - 50.0);
    let line_col = if env.HltSel {
        egui::Color32::from_rgb(255, 200, 80)   // selected = yellow
    } else { dobj };
    p2.line_segment([line_a, line_b], egui::Stroke::new(2.0, line_col));

    // Grips on this line — only drawn when GrpEnb is on.
    if env.GrpEnb {
        let g = env.GrpSz as f32;
        let unsel = u_to_col(env.GrClrU);
        let hot   = u_to_col(env.GrClrS);
        let mid = egui::pos2(0.5 * (line_a.x + line_b.x), 0.5 * (line_a.y + line_b.y));
        for (centre, col) in [(line_a, unsel), (mid, hot), (line_b, unsel)] {
            p2.rect(
                egui::Rect::from_center_size(centre, egui::vec2(g, g)),
                1.0, col,
                egui::Stroke::new(1.0, egui::Color32::WHITE),
            );
        }
        p2.text(line_a + egui::vec2(8.0, -14.0), egui::Align2::LEFT_BOTTOM,
            format!("GrpSz={}  GrClrU=0x{:06X}  GrClrS=0x{:06X}",
                env.GrpSz, env.GrClrU, env.GrClrS),
            egui::FontId::monospace(10.0), egui::Color32::from_rgb(180, 200, 220));
    } else {
        p2.text(r2.left_top() + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP, "GrpEnb = OFF (no grips drawn)",
            egui::FontId::monospace(10.0), egui::Color32::from_rgb(180, 200, 220));
    }

    // SelPrv preview cue — faint cyan ghost line above the sample, only
    // shown when the toggle is on.
    if env.SelPrv {
        let ghost_a = egui::pos2(r2.left() + 30.0, r2.top() + 25.0);
        let ghost_b = egui::pos2(r2.right() - 30.0, r2.top() + 25.0);
        p2.line_segment([ghost_a, ghost_b],
            egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 240, 255)));
        p2.text(ghost_a + egui::vec2(0.0, -2.0),
            egui::Align2::LEFT_BOTTOM,
            "SelPrv: hover preview shown",
            egui::FontId::monospace(10.0),
            egui::Color32::from_rgb(120, 240, 255));
    }

    ui.add_space(4.0);
    ui.small("Dialog / xref settings have no visual preview.");
}

fn snap_blurb(k: SnapKind) -> &'static str {
    match k {
        SnapKind::End => "endpoints of lines & arcs",
        SnapKind::Mid => "midpoints",
        SnapKind::Cen => "centres of circles & arcs",
        SnapKind::Qua => "quadrants of circles & arcs (E / N / W / S)",
        SnapKind::Int => "intersections between two dobjects",
        SnapKind::Per => "perpendicular foot   (needs anchor click)",
        SnapKind::Tan => "tangent point        (needs anchor click)",
        SnapKind::Nea => "nearest point on the curve",
    }
}

/// Tessellate a polyline (vertex chain + per-vertex bulges) into
/// connected screen-space points. Straight segments add just the
/// endpoint; arc segments add intermediate samples whose density
/// scales with the on-screen arc length so the curve stays smooth at
/// any zoom. Used by every polyline render path (normal / thick /
/// dashed) so a committed pline with arc segments shows the actual
/// arcs, not their chords.
fn polyline_tessellated_screen_pts(
    p: &Polyline,
    app: &CadApp,
    rect: egui::Rect,
) -> Vec<egui::Pos2> {
    if p.vertices.len() < 2 { return Vec::new(); }
    let n = p.vertices.len();
    let pairs = if p.closed { n } else { n - 1 };
    let mut out: Vec<egui::Pos2> = Vec::with_capacity(n);
    out.push(app.w2s(p.vertices[0].pos, rect));
    for i in 0..pairs {
        let a = p.vertices[i].pos;
        let b = p.vertices[(i + 1) % n].pos;
        let bulge = p.vertices[i].bulge;
        append_pline_segment_screen_pts(a, b, bulge, app, rect, &mut out);
    }
    out
}

/// Append the tessellation of ONE polyline segment from `a` to `b` with
/// the given bulge to `out`. `a` is assumed already present at the end
/// of `out`; this function adds the intermediate samples + `b`.
/// Straight segment (bulge ≈ 0) appends just `b`.
fn append_pline_segment_screen_pts(
    a: Vec2, b: Vec2, bulge: f64,
    app: &CadApp,
    rect: egui::Rect,
    out: &mut Vec<egui::Pos2>,
) {
    if bulge.abs() < 1e-9 {
        out.push(app.w2s(b, rect));
        return;
    }
    let chord = b - a;
    let chord_len = chord.len();
    if chord_len < EPS {
        out.push(app.w2s(b, rect));
        return;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let sin_half = half.sin();
    if sin_half.abs() < EPS {
        out.push(app.w2s(b, rect));
        return;
    }
    let r = chord_len / (2.0 * sin_half.abs());
    let chord_hat = chord / chord_len;
    let perp = Vec2::new(-chord_hat.y, chord_hat.x);   // CCW perp
    let mid  = (a + b) * 0.5;
    let centre_off = r * half.cos();
    let centre = mid + perp * (if bulge > 0.0 { centre_off } else { -centre_off });
    let start_ang = (a - centre).angle();
    let end_ang   = (b - centre).angle();
    let sweep = if bulge > 0.0 {
        (end_ang - start_ang).rem_euclid(std::f64::consts::TAU)
    } else {
        -((start_ang - end_ang).rem_euclid(std::f64::consts::TAU))
    };
    let arc_len_px = (r as f32 * app.scale) * sweep.abs() as f32;
    let n = (arc_len_px * 0.4).clamp(6.0, 256.0) as usize;
    // Skip i=0 (== a, already in `out`); end with i=n (== b).
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let ang = start_ang + sweep * t;
        let p = centre + Vec2::new(r * ang.cos(), r * ang.sin());
        out.push(app.w2s(p, rect));
    }
}

/// Append the world-space tessellation of ONE polyline segment from
/// `a` to `b` with the given bulge to `out`. `a` is assumed already
/// present at the end of `out`; this function adds the intermediate
/// samples + `b`. Straight segment (bulge ≈ 0) appends just `b`.
/// Sample density is fixed at 24 per arc segment — enough for smooth
/// hatch boundaries at any zoom without paying the per-zoom
/// retessellation cost of the screen-space variant.
fn append_arc_world_samples(a: Vec2, b: Vec2, bulge: f64, out: &mut Vec<Vec2>) {
    if bulge.abs() < 1e-9 {
        out.push(b);
        return;
    }
    let chord = b - a;
    let chord_len = chord.len();
    if chord_len < EPS {
        out.push(b);
        return;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let sin_half = half.sin();
    if sin_half.abs() < EPS {
        out.push(b);
        return;
    }
    let r = chord_len / (2.0 * sin_half.abs());
    let chord_hat = chord / chord_len;
    let perp = Vec2::new(-chord_hat.y, chord_hat.x);
    let mid  = (a + b) * 0.5;
    let centre_off = r * half.cos();
    let centre = mid + perp * (if bulge > 0.0 { centre_off } else { -centre_off });
    let start_ang = (a - centre).angle();
    let end_ang   = (b - centre).angle();
    let sweep = if bulge > 0.0 {
        (end_ang - start_ang).rem_euclid(std::f64::consts::TAU)
    } else {
        -((start_ang - end_ang).rem_euclid(std::f64::consts::TAU))
    };
    let n: usize = 24;
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let ang = start_ang + sweep * t;
        out.push(centre + Vec2::new(r * ang.cos(), r * ang.sin()));
    }
}

/// Even-odd ray-cast point-in-polygon test. Returns true iff `p` lies
/// in the interior of the closed polygon traced by the iterator. Half-
/// open Y-test avoids double-counting horizontal edge endpoints — the
/// standard correct PIP. Used today by the hatch pick-point boundary
/// finder; would also fit hit-testing closed splines / regions when
/// those land.
fn point_in_polygon<I: IntoIterator<Item = Vec2>>(p: Vec2, verts: I) -> bool {
    let vs: Vec<Vec2> = verts.into_iter().collect();
    let n = vs.len();
    if n < 3 { return false; }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let pi = vs[i];
        let pj = vs[j];
        if (pi.y > p.y) != (pj.y > p.y) {
            let x_int = pi.x + (p.y - pi.y) * (pj.x - pi.x) / (pj.y - pi.y);
            if p.x < x_int { inside = !inside; }
        }
        j = i;
    }
    inside
}

/// Clip a screen-space line segment to an axis-aligned rectangle —
/// parametric Liang-Barsky-lite. Returns the visible portion or None
/// if the segment misses the rect entirely. Used only by the hatch
/// preview's pattern renderer; the canvas hatch render uses the
/// polygon-clip path against the real boundary loops.
fn clip_line_to_rect(a: egui::Pos2, b: egui::Pos2, r: egui::Rect)
    -> Option<(egui::Pos2, egui::Pos2)>
{
    let mut t0 = 0.0_f32;
    let mut t1 = 1.0_f32;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let clip = |p: f32, q: f32, t0: &mut f32, t1: &mut f32| -> bool {
        if p.abs() < 1e-9 { return q >= 0.0; }
        let r = q / p;
        if p < 0.0 {
            if r > *t1 { return false; }
            if r > *t0 { *t0 = r; }
        } else {
            if r < *t0 { return false; }
            if r < *t1 { *t1 = r; }
        }
        true
    };
    if !clip(-dx, a.x - r.left(),   &mut t0, &mut t1) { return None; }
    if !clip( dx, r.right() - a.x,  &mut t0, &mut t1) { return None; }
    if !clip(-dy, a.y - r.top(),    &mut t0, &mut t1) { return None; }
    if !clip( dy, r.bottom() - a.y, &mut t0, &mut t1) { return None; }
    if t1 < t0 { return None; }
    Some((
        egui::pos2(a.x + dx * t0, a.y + dy * t0),
        egui::pos2(a.x + dx * t1, a.y + dy * t1),
    ))
}

/// Tessellate a circle into an N-vertex closed loop (world coords).
/// Sample density is fixed — fill tessellation doesn't need to track
/// zoom. 64 is smooth at typical printable sizes; bump to 128 if a
/// large-radius hatch looks faceted.
fn tessellate_circle_loop(centre: Vec2, radius: f64, n: usize) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = (i as f64) / (n as f64) * std::f64::consts::TAU;
        out.push(Vec2::new(
            centre.x + radius * t.cos(),
            centre.y + radius * t.sin(),
        ));
    }
    out
}

/// Tolerance for treating a polyline's first/last vertices as
/// coincident — i.e. the polyline is geometrically a loop even if its
/// `closed` flag is still false. The debug log surfaces this case with
/// "endpoint gap = … ← visually closed but `closed=false`".
const POLYLINE_EFFECTIVELY_CLOSED_EPS: f64 = 1e-3;

/// True if `p` should be treated as a closed loop for hatch purposes:
/// either its `closed` flag is set, OR its first and last vertices
/// coincide within `POLYLINE_EFFECTIVELY_CLOSED_EPS`. Polylines with
/// fewer than 3 vertices can't form a loop.
fn polyline_is_effectively_closed(p: &Polyline) -> bool {
    if p.vertices.len() < 3 { return false; }
    if p.closed { return true; }
    let first = p.vertices.first().map(|v| v.pos);
    let last  = p.vertices.last().map(|v| v.pos);
    match (first, last) {
        (Some(a), Some(b)) =>
            (a - b).len() < POLYLINE_EFFECTIVELY_CLOSED_EPS,
        _ => false,
    }
}

/// Tessellate any single closed geometry to a vertex loop in world
/// coords. Mirrors what `App::resolve_hatch_loops` does for one
/// boundary, but on raw geometry rather than via handle resolution —
/// used by the cheap-path island detector to PIP-test other dobjects'
/// boundaries against a chosen outer.
///
/// Open polylines whose endpoints meet within
/// `POLYLINE_EFFECTIVELY_CLOSED_EPS` are also accepted — this is the
/// "drew it as a closed loop but forgot to type `c` Enter" case the
/// user's polyline #6 surfaced. The duplicated last vertex (if any)
/// is dropped so the polygon doesn't double-count its starting edge.
///
/// Returns an empty Vec for everything else (Line / Arc / open
/// Polyline with separated endpoints / Spline / Point / Hatch).
fn closed_dobject_polygon(g: &Geom) -> Vec<Vec2> {
    match g {
        Geom::Polyline(p) if polyline_is_effectively_closed(p) => {
            let n = p.vertices.len();
            // Treat the polyline as if `closed=true`. If the last
            // vertex duplicates the first (the v06=(x,y) ≡ v00 case
            // from polyline #6 in the user log), drop it so the
            // implied closing edge isn't drawn twice.
            let effective_n = if !p.closed {
                let f = p.vertices[0].pos;
                let l = p.vertices[n - 1].pos;
                if (f - l).len() < POLYLINE_EFFECTIVELY_CLOSED_EPS { n - 1 } else { n }
            } else { n };
            let mut v: Vec<Vec2> = Vec::with_capacity(effective_n * 4);
            v.push(p.vertices[0].pos);
            for k in 0..effective_n {
                let a = p.vertices[k].pos;
                let b = p.vertices[(k + 1) % effective_n].pos;
                append_arc_world_samples(a, b, p.vertices[k].bulge, &mut v);
            }
            v
        }
        Geom::Circle(c)  => tessellate_circle_loop(c.center, c.radius, 64),
        Geom::Ellipse(e) => tessellate_ellipse_loop(e, 64),
        _ => Vec::new(),
    }
}

/// Tessellate an Ellipse to an N-vertex closed loop using the kernel's
/// own `point_at`. Same density rationale as circles.
fn tessellate_ellipse_loop(e: &Ellipse, n: usize) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = (i as f64) / (n as f64) * std::f64::consts::TAU;
        out.push(e.point_at(t));
    }
    out
}

/// Find the t-value at which an infinite line (origin `o`, unit
/// direction `u`) crosses the SEGMENT a→b. Returns None if the line
/// misses the segment (intersection lies outside [0,1] along the
/// segment) or if the line is parallel to the segment. Used by the
/// hatch-pattern renderer to clip each parallel line against each
/// boundary edge.
fn line_segment_intersect_t(o: Vec2, u: Vec2, a: Vec2, b: Vec2) -> Option<f64> {
    let d = b - a;
    // Parametric line: o + t*u; segment: a + s*d. Solve for (t, s).
    //   o.x + t*u.x = a.x + s*d.x
    //   o.y + t*u.y = a.y + s*d.y
    // → [u.x  -d.x] [t]   [a.x - o.x]
    //   [u.y  -d.y] [s] = [a.y - o.y]
    let det = u.x * (-d.y) - (-d.x) * u.y;
    if det.abs() < 1e-12 { return None; }   // parallel
    let rhs_x = a.x - o.x;
    let rhs_y = a.y - o.y;
    let t = (rhs_x * (-d.y) - (-d.x) * rhs_y) / det;
    let s = (u.x * rhs_y - u.y * rhs_x) / det;
    // Segment-end test — accept hits AT endpoints (half-open avoids
    // double-counting at shared vertices via the standard even-odd
    // ray-cast convention, but for hatch pattern lines we want every
    // edge crossing once).
    if s < -1e-9 || s > 1.0 + 1e-9 { return None; }
    Some(t)
}

/// AutoCAD polyline bulge for a 3-point arc through (p1, p2, p3) in
/// THAT ORDER. Returns 0.0 (straight segment) when the three points are
/// collinear or coincident. The sign matches the polyline convention:
/// positive bulge = arc bends to the LEFT of the chord p1→p3 (CCW).
fn bulge_from_three_points(p1: Vec2, p2: Vec2, p3: Vec2) -> f64 {
    use cad_kernel::arc_three_points;
    let Some(arc) = arc_three_points(p1, p2, p3) else { return 0.0 };
    let r = arc.radius;
    if r < EPS { return 0.0; }
    let center = arc.center;
    // Angles at the center for the two endpoints + the midpoint pick.
    let a1 = (p1 - center).angle();
    let a3 = (p3 - center).angle();
    let a2 = (p2 - center).angle();
    // Try CCW direction first: (a3 - a1) mod TAU. If the swept range
    // contains a2, the polyline travels CCW (positive bulge); else CW.
    let ccw_sweep = (a3 - a1).rem_euclid(std::f64::consts::TAU);
    let mid_offset = (a2 - a1).rem_euclid(std::f64::consts::TAU);
    let signed_theta = if mid_offset <= ccw_sweep + 1e-9 {
        ccw_sweep
    } else {
        -(std::f64::consts::TAU - ccw_sweep)
    };
    (signed_theta / 4.0).tan()
}

/// Short, capitalised label for a Dobject's underlying geometry. Used
/// in dialog/window titles where the user wants to know "what kind of
/// dobject am I editing?" without reading a full describe() line.
fn dobject_kind_name(g: &Geom) -> &'static str {
    match g {
        Geom::Line(_)       => "Line",
        Geom::Circle(_)     => "Circle",
        Geom::Arc(_)        => "Arc",
        Geom::Ellipse(_)    => "Ellipse",
        Geom::EllipseArc(_) => "EllipseArc",
        Geom::Point(_)      => "Point",
        Geom::Polyline(_)   => "Polyline",
        Geom::Hatch(_)      => "Hatch",
        Geom::Spline(_)     => "Spline",
    }
}

fn describe(g: &Geom) -> String {
    match g {
        Geom::Line(l) => format!(
            "line ({:.2},{:.2}) → ({:.2},{:.2})",
            l.a.x, l.a.y, l.b.x, l.b.y
        ),
        Geom::Circle(c) => format!(
            "circle c=({:.2},{:.2}) r={:.2}",
            c.center.x, c.center.y, c.radius
        ),
        Geom::Arc(a) => format!(
            "arc c=({:.2},{:.2}) r={:.2} {:.1}°+{:.1}°",
            a.center.x, a.center.y, a.radius,
            a.start_angle.to_degrees(),
            a.sweep_angle.to_degrees()
        ),
        Geom::Ellipse(el) => format!(
            "ellipse c=({:.2},{:.2}) a={:.2} ratio={:.3} rot={:.1}°",
            el.center.x, el.center.y, el.semi_major(), el.ratio,
            el.major.angle().to_degrees()
        ),
        Geom::EllipseArc(ea) => format!(
            "ellipsearc c=({:.2},{:.2}) a={:.2} ratio={:.3} {:.1}°+{:.1}°",
            ea.ellipse.center.x, ea.ellipse.center.y,
            ea.ellipse.semi_major(), ea.ellipse.ratio,
            ea.start_param.to_degrees(),
            ea.sweep_param.to_degrees()
        ),
        Geom::Point(pt) => format!(
            "point ({:.2},{:.2}) style={} size={:.2}",
            pt.location.x, pt.location.y, pt.style, pt.size
        ),
        Geom::Polyline(p) => format!(
            "polyline {} verts{} len={:.2}",
            p.vertices.len(),
            if p.closed { " (closed)" } else { "" },
            p.length()
        ),
        Geom::Hatch(h) => format!(
            "hatch {} boundary loop(s) ({:?})",
            h.boundary_handles.len(), h.pattern
        ),
        Geom::Spline(s) => format!(
            "spline degree={} {} ctrl pts{}",
            s.degree, s.control_points.len(),
            if s.weights.iter().all(|w| (*w - 1.0).abs() < 1e-9) { "" }
            else { " (rational)" }
        ),
    }
}

/// Verbose dump — every coordinate the algorithm sees, no summarisation.
/// Used by the hatch debug log so the user can verify "is this polyline
/// actually closed? do its first and last verts coincide?". Polyline
/// vertices include their bulge so arc segments don't appear straight in
/// the dump. Spline dumps control points + weights. Hatch lists handle
/// IDs in order. Coordinates print at 3 decimal places — enough to spot
/// "1e-3 gap between supposedly coincident endpoints" without flooding
/// the log on big polylines.
fn describe_verbose(g: &Geom) -> String {
    match g {
        Geom::Line(l) => format!(
            "line  a=({:.3},{:.3})  b=({:.3},{:.3})  len={:.3}",
            l.a.x, l.a.y, l.b.x, l.b.y, (l.b - l.a).len()
        ),
        Geom::Circle(c) => format!(
            "circle  c=({:.3},{:.3})  r={:.3}",
            c.center.x, c.center.y, c.radius
        ),
        Geom::Arc(a) => format!(
            "arc  c=({:.3},{:.3})  r={:.3}  start={:.2}°  sweep={:.2}°",
            a.center.x, a.center.y, a.radius,
            a.start_angle.to_degrees(), a.sweep_angle.to_degrees()
        ),
        Geom::Ellipse(el) => format!(
            "ellipse  c=({:.3},{:.3})  a={:.3}  ratio={:.3}  rot={:.2}°",
            el.center.x, el.center.y, el.semi_major(), el.ratio,
            el.major.angle().to_degrees()
        ),
        Geom::EllipseArc(ea) => format!(
            "ellipsearc  c=({:.3},{:.3})  a={:.3}  ratio={:.3}  start={:.2}°  sweep={:.2}°",
            ea.ellipse.center.x, ea.ellipse.center.y,
            ea.ellipse.semi_major(), ea.ellipse.ratio,
            ea.start_param.to_degrees(), ea.sweep_param.to_degrees()
        ),
        Geom::Point(pt) => format!(
            "point  ({:.3},{:.3})", pt.location.x, pt.location.y
        ),
        Geom::Polyline(p) => {
            let n = p.vertices.len();
            let head = format!(
                "polyline  {} verts  closed={}  len={:.3}",
                n, p.closed, p.length());
            let mut s = head;
            for (i, v) in p.vertices.iter().enumerate() {
                let suffix = if v.bulge.abs() > 1e-9 {
                    format!(" bulge={:.4}", v.bulge)
                } else { String::new() };
                s.push_str(&format!(
                    "\n        v{:02}=({:.3},{:.3}){}",
                    i, v.pos.x, v.pos.y, suffix));
            }
            if n >= 2 {
                let first = p.vertices[0].pos;
                let last  = p.vertices[n - 1].pos;
                let gap = (last - first).len();
                s.push_str(&format!(
                    "\n        endpoint gap (v00→v{:02}) = {:.6}{}",
                    n - 1, gap,
                    if !p.closed && gap < 1e-3 {
                        "  ← visually closed but `closed=false`"
                    } else { "" }));
            }
            s
        }
        Geom::Hatch(h) => {
            let mut s = format!(
                "hatch  pattern={:?}  {} boundary handle(s)",
                h.pattern, h.boundary_handles.len());
            for (i, hh) in h.boundary_handles.iter().enumerate() {
                s.push_str(&format!("\n        loop{} → handle #{}", i, hh));
            }
            s
        }
        Geom::Spline(s) => {
            let mut out = format!(
                "spline  degree={}  {} ctrl pts  rational={}",
                s.degree, s.control_points.len(),
                !s.weights.iter().all(|w| (*w - 1.0).abs() < 1e-9));
            for (i, p) in s.control_points.iter().enumerate() {
                out.push_str(&format!(
                    "\n        cp{:02}=({:.3},{:.3}) w={:.3}",
                    i, p.x, p.y, s.weights.get(i).copied().unwrap_or(1.0)));
            }
            out
        }
    }
}

// ---- icon tool-button -------------------------------------------------------

/// Color picker UI — ACI palette as PRIMARY, TrueColor as secondary.
/// Returns true if the value changed. See
/// `feedback_rust_cad_color_aci_primary` memo. TrueColor values are
/// interned via the document's `TrueColorTable` (see Color storage
/// refactor memo); the picker takes a &mut TrueColorTable for that.
///
/// Clicking the "Pick ACI…" button sets `*wants_pick = true`; the caller
/// then opens the shared polar-wheel window (see `render_aci_picker_window`
/// + the ACI picker UI reference at `~/workspace/RUST_CAD/ACI_Picker_UI.html`).
fn aci_color_picker(
    ui: &mut egui::Ui,
    _id: impl std::hash::Hash,
    value: &mut Color,
    truecolors: &mut TrueColorTable,
    wants_pick: &mut bool,
) -> bool {
    let mut changed = false;

    // Current-value summary swatch + label
    let (r, g, b) = match *value {
        Color::Aci(i)             => aci_palette(i),
        Color::TrueColorRef(idx)  => {
            let v = truecolors.get(idx).unwrap_or(0xFFFFFF);
            (((v >> 16) & 0xFF) as u8,
             ((v >>  8) & 0xFF) as u8,
             ( v        & 0xFF) as u8)
        }
        Color::ByLayer       => (180, 180, 200),
        Color::ByBlock       => (140, 140, 160),
    };
    let summary = match *value {
        Color::ByLayer            => "ByLayer".to_string(),
        Color::ByBlock            => "ByBlock".to_string(),
        Color::Aci(i)             => format!("ACI {}", i),
        Color::TrueColorRef(idx)  => {
            let v = truecolors.get(idx).unwrap_or(0xFFFFFF);
            format!("RGB #{:06X}", v & 0x00FFFFFF)
        }
    };

    ui.horizontal(|ui| {
        // Clickable summary chip — same affordance as the layer panel
        // swatch: click to open the polar ACI picker.
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(22.0, 18.0), egui::Sense::click());
        ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));
        ui.painter().rect_stroke(rect, 2.0,
            egui::Stroke::new(0.7, egui::Color32::from_rgb(70, 80, 95)));
        if resp.on_hover_text("Click to pick an ACI color").clicked() {
            *wants_pick = true;
        }
        ui.label(summary);
        if ui.small_button("Pick ACI…").clicked() {
            *wants_pick = true;
        }
    });

    // Secondary controls — ByLayer / ByBlock / TrueColor fallback
    ui.horizontal(|ui| {
        if ui.small_button("ByLayer").clicked() {
            *value = Color::ByLayer;
            changed = true;
        }
        if ui.small_button("ByBlock").clicked() {
            *value = Color::ByBlock;
            changed = true;
        }
        // TrueColor fallback — opens egui's RGB picker. Commits by
        // interning the RGB and storing only a small ref on `value`.
        let mut rgb = [r, g, b];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            let packed = ((rgb[0] as u32) << 16)
                       | ((rgb[1] as u32) << 8)
                       | (rgb[2] as u32);
            *value = Color::TrueColorRef(truecolors.intern(packed));
            changed = true;
        }
        ui.small("TrueColor…");
    });

    changed
}

/// Text-only toolbar button styled to match `tool_button`'s color scheme
/// and height so the toolbar reads as one consistent strip. Used for the
/// panel-toggle buttons (snap, grips, settings, layers, pens, info, array)
/// that don't have a drafted icon.
///
/// `active` highlights the button in the same blue as a selected drafting
/// tool — visual cue that the corresponding panel is open / feature is on.
fn panel_button(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    // Allocate space matching tool_button height (52 px) but text-width
    // sized so the label decides the width.
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(12.0),
        egui::Color32::from_rgb(225, 235, 245),
    );
    let pad_x = 10.0;
    let size = egui::vec2((galley.size().x + pad_x * 2.0).max(56.0), 52.0);
    let (resp, painter) = ui.allocate_painter(size, egui::Sense::click());
    let rect = resp.rect;
    let bg = if active {
        egui::Color32::from_rgb(60, 110, 175)
    } else if resp.hovered() {
        egui::Color32::from_rgb(48, 58, 72)
    } else {
        egui::Color32::from_rgb(28, 34, 42)
    };
    painter.rect(
        rect, 5.0, bg,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 80, 95)),
    );
    let text_pos = rect.center() - egui::vec2(galley.size().x * 0.5, galley.size().y * 0.5);
    painter.galley(text_pos, galley, egui::Color32::from_rgb(225, 235, 245));
    resp.clicked()
}

fn tool_button(ui: &mut egui::Ui, current: &mut Tool, this: Tool, label: &str) -> bool {
    let selected = *current == this;
    let (resp, painter) =
        ui.allocate_painter(egui::vec2(56.0, 52.0), egui::Sense::click());
    let rect = resp.rect;
    let bg = if selected {
        egui::Color32::from_rgb(60, 110, 175)
    } else if resp.hovered() {
        egui::Color32::from_rgb(48, 58, 72)
    } else {
        egui::Color32::from_rgb(28, 34, 42)
    };
    painter.rect(
        rect, 5.0, bg,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 80, 95)),
    );

    let c = rect.center() - egui::vec2(0.0, 4.0);
    let icon_col = egui::Color32::from_rgb(225, 235, 245);
    let pen = egui::Stroke::new(1.8, icon_col);
    let dot = |p: egui::Pos2| painter.circle_filled(p, 1.8, icon_col);
    match this {
        Tool::None => {
            // arrow / pointer
            painter.line_segment([c + egui::vec2(-8.0, -8.0), c + egui::vec2(6.0, 6.0)], pen);
            painter.line_segment([c + egui::vec2(-8.0, -8.0), c + egui::vec2(-3.0, 2.0)], pen);
            painter.line_segment([c + egui::vec2(-8.0, -8.0), c + egui::vec2(2.0, -3.0)], pen);
        }
        Tool::Line => {
            painter.line_segment(
                [c + egui::vec2(-14.0, 10.0), c + egui::vec2(14.0, -10.0)],
                pen,
            );
            dot(c + egui::vec2(-14.0, 10.0));
            dot(c + egui::vec2( 14.0, -10.0));
        }
        Tool::Circle => {
            painter.circle_stroke(c, 13.0, pen);
            dot(c);
        }
        Tool::Arc => {
            // half-circle + center dot + two endpoint dots (center-start-end variant)
            let n = 24;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = std::f32::consts::PI * (i as f32 / n as f32);
                pts.push(c + egui::vec2(-13.0 * t.cos(), -13.0 * t.sin()));
            }
            painter.add(egui::Shape::line(pts, pen));
            dot(c);
            dot(c + egui::vec2(-13.0, 0.0));
            dot(c + egui::vec2( 13.0, 0.0));
        }
        Tool::Ellipse => {
            // squashed ellipse — a 2:1 ratio so it reads distinctly from the circle
            let n = 32;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = std::f32::consts::TAU * (i as f32 / n as f32);
                pts.push(c + egui::vec2(14.0 * t.cos(), 7.0 * t.sin()));
            }
            painter.add(egui::Shape::line(pts, pen));
            dot(c);
            dot(c + egui::vec2(14.0, 0.0));   // major-end
            dot(c + egui::vec2(0.0, -7.0));   // minor-end
        }
        Tool::EllipseArc => {
            // top-half of a squashed ellipse — same proportions as the
            // ellipse icon, but only the upper sweep is drawn.
            let n = 24;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = std::f32::consts::PI * (i as f32 / n as f32);
                pts.push(c + egui::vec2(-14.0 * t.cos(), -7.0 * t.sin()));
            }
            painter.add(egui::Shape::line(pts, pen));
            dot(c);
            dot(c + egui::vec2(-14.0, 0.0));   // start
            dot(c + egui::vec2( 14.0, 0.0));   // end
        }
        Tool::Point => {
            // a small '+' glyph
            painter.line_segment([c + egui::vec2(-9.0, 0.0), c + egui::vec2(9.0, 0.0)], pen);
            painter.line_segment([c + egui::vec2(0.0, -9.0), c + egui::vec2(0.0, 9.0)], pen);
            dot(c);
        }
        Tool::Polyline => {
            // a 3-segment chevron-ish shape with vertex dots
            let p1 = c + egui::vec2(-14.0,  8.0);
            let p2 = c + egui::vec2( -4.0, -8.0);
            let p3 = c + egui::vec2(  6.0,  6.0);
            let p4 = c + egui::vec2( 14.0, -4.0);
            painter.line_segment([p1, p2], pen);
            painter.line_segment([p2, p3], pen);
            painter.line_segment([p3, p4], pen);
            dot(p1); dot(p2); dot(p3); dot(p4);
        }
        Tool::Spline => {
            // Smooth S-curve sampled from a cubic NURBS through 4
            // control points (rendered AS the icon — eats its own
            // dogfood). The 4 control dots show where the user clicks
            // when drafting; the curve shows the result. Hint pens
            // sketch the control polygon underneath so the icon also
            // teaches the data model at a glance.
            let p1 = c + egui::vec2(-14.0,  8.0);
            let p2 = c + egui::vec2( -5.0, -10.0);
            let p3 = c + egui::vec2(  5.0,  10.0);
            let p4 = c + egui::vec2( 14.0, -8.0);
            // Faint chord polygon (the "control polygon")
            let hint = egui::Stroke::new(0.6, egui::Color32::from_rgba_unmultiplied(
                icon_col.r(), icon_col.g(), icon_col.b(), 90));
            painter.line_segment([p1, p2], hint);
            painter.line_segment([p2, p3], hint);
            painter.line_segment([p3, p4], hint);
            // The curve itself — a cubic Bézier sample is close enough
            // visually to a degree-3 clamped uniform NURBS through 4
            // control points (which IS a single Bézier in this case).
            let mut prev = p1;
            for i in 1..=24 {
                let t = i as f32 / 24.0;
                let u = 1.0 - t;
                let pt = egui::pos2(
                    u*u*u*p1.x + 3.0*u*u*t*p2.x + 3.0*u*t*t*p3.x + t*t*t*p4.x,
                    u*u*u*p1.y + 3.0*u*u*t*p2.y + 3.0*u*t*t*p3.y + t*t*t*p4.y,
                );
                painter.line_segment([prev, pt], pen);
                prev = pt;
            }
            // Control-point dots
            dot(p1); dot(p2); dot(p3); dot(p4);
        }
    }

    painter.text(
        rect.center_bottom() - egui::vec2(0.0, 10.0),
        egui::Align2::CENTER_BOTTOM,
        label,
        egui::FontId::proportional(10.0),
        icon_col,
    );

    if resp.clicked() {
        *current = if selected { Tool::None } else { this };
        return true;
    }
    false
}

// ---- hatch command button (one-shot — opens dialog) -----------------------

/// Custom-painted Hatch button. Hatch isn't a persistent draw tool
/// (no `Tool::Hatch` variant), so this lives outside `tool_button` —
/// click → `run_command("hatch")` → opens the Choose Hatch Attributes
/// dialog. Icon: a square outline with 45° hatching, corner
/// registration ticks (so it reads as a "boundary"), and a small
/// pick-point cursor at the bottom-right (signifying the Pick Point
/// flow). User reference: the freenom-style "hatch" pictogram with
/// crosshairs at every corner.
fn hatch_command_button(ui: &mut egui::Ui) -> bool {
    let (resp, painter) =
        ui.allocate_painter(egui::vec2(56.0, 52.0), egui::Sense::click());
    let rect = resp.rect;
    let bg = if resp.hovered() {
        egui::Color32::from_rgb(48, 58, 72)
    } else {
        egui::Color32::from_rgb(28, 34, 42)
    };
    painter.rect(
        rect, 5.0, bg,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 80, 95)),
    );

    let c = rect.center() - egui::vec2(0.0, 4.0);
    let icon_col = egui::Color32::from_rgb(225, 235, 245);
    let pen      = egui::Stroke::new(1.6, icon_col);
    let thin     = egui::Stroke::new(1.0, icon_col);

    // Boundary square
    let half = 11.0_f32;
    let sq = egui::Rect::from_center_size(c, egui::vec2(half * 2.0, half * 2.0));
    painter.rect_stroke(sq, 0.0, pen);

    // Hatching: 5 diagonal lines at 45°, clipped to the square. Drawing
    // long lines and clipping is simpler than computing exact entry/exit
    // points per line — egui's `with_clip_rect` does the rest.
    let inner = painter.with_clip_rect(sq);
    let span = half * 4.0;
    let spacing = (half * 2.0) / 4.0;       // 5 lines: at -2s, -s, 0, +s, +2s
    for k in -2..=2 {
        let off = k as f32 * spacing;
        let p1 = egui::pos2(c.x - span + off, c.y + span + off);
        let p2 = egui::pos2(c.x + span + off, c.y - span + off);
        inner.line_segment([p1, p2], thin);
    }

    // Corner registration ticks — a short "L" extending outward from
    // each corner of the boundary. Reads as "this is a boundary I'm
    // selecting" rather than just an arbitrary outline.
    let ext = 4.5_f32;
    for &(corner, dx, dy) in &[
        (sq.left_top(),     -1.0_f32, -1.0_f32),
        (sq.right_top(),     1.0,     -1.0),
        (sq.left_bottom(),  -1.0,      1.0),
        (sq.right_bottom(),  1.0,      1.0),
    ] {
        painter.line_segment([corner, corner + egui::vec2(dx * ext, 0.0)], pen);
        painter.line_segment([corner, corner + egui::vec2(0.0, dy * ext)], pen);
    }

    // Pick-point cursor at lower-right: a small offset pickbox + arrow
    // pointing INTO the main square. Communicates "click inside to
    // pick the boundary" at a glance.
    let pb_center = sq.right_bottom() + egui::vec2(3.0, 3.0);
    let pb_half = 2.5_f32;
    let pb_rect = egui::Rect::from_center_size(
        pb_center, egui::vec2(pb_half * 2.0, pb_half * 2.0));
    painter.rect_stroke(pb_rect, 0.0, pen);
    // Arrow from pickbox toward sq's center
    let arrow_tip = pb_center + egui::vec2(-3.0, -3.0);
    let arrow_tail = pb_center + egui::vec2(-0.5, -0.5);
    painter.line_segment([arrow_tail, arrow_tip], pen);
    painter.line_segment([arrow_tip,   arrow_tip + egui::vec2(2.5, 0.0)], pen);
    painter.line_segment([arrow_tip,   arrow_tip + egui::vec2(0.0, 2.5)], pen);

    painter.text(
        rect.center_bottom() - egui::vec2(0.0, 10.0),
        egui::Align2::CENTER_BOTTOM,
        "hatch",
        egui::FontId::proportional(10.0),
        icon_col,
    );

    resp.clicked()
}

// ---- arc tool button (toolbar, one per quick-access method) ---------------

fn arc_tool_button(
    ui: &mut egui::Ui,
    current_tool:   &mut Tool,
    current_method: &mut ArcMethod,
    method: ArcMethod,
    label: &str,
) -> bool {
    let selected = *current_tool == Tool::Arc && *current_method == method;
    let (resp, painter) =
        ui.allocate_painter(egui::vec2(56.0, 52.0), egui::Sense::click());
    let rect = resp.rect;
    let bg = if selected {
        egui::Color32::from_rgb(60, 110, 175)
    } else if resp.hovered() {
        egui::Color32::from_rgb(48, 58, 72)
    } else {
        egui::Color32::from_rgb(28, 34, 42)
    };
    painter.rect(rect, 5.0, bg,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 80, 95)));

    let c = rect.center() - egui::vec2(0.0, 4.0);
    let icon_col = egui::Color32::from_rgb(225, 235, 245);
    let stroke   = egui::Stroke::new(1.6, icon_col);
    let dot = |pt: egui::Pos2| painter.circle_filled(pt, 2.2, icon_col);

    // shared half-arc
    let n = 24;
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = std::f32::consts::PI * (i as f32 / n as f32);
        pts.push(c + egui::vec2(-13.0 * t.cos(), -13.0 * t.sin()));
    }
    painter.add(egui::Shape::line(pts.clone(), stroke));

    // method-specific dots — crucially, ThreePoints has no centre dot
    match method {
        ArcMethod::ThreePoints => {
            dot(pts[0]);
            dot(pts[n / 2]);
            dot(pts[n]);
        }
        ArcMethod::StartCenterEnd | ArcMethod::CenterStartEnd => {
            dot(pts[0]);
            dot(c);          // centre
            dot(pts[n]);
        }
        _ => {
            dot(pts[0]);
            dot(pts[n]);
        }
    }

    painter.text(
        rect.center_bottom() - egui::vec2(0.0, 10.0),
        egui::Align2::CENTER_BOTTOM,
        label,
        egui::FontId::proportional(10.0),
        icon_col,
    );

    if resp.clicked() {
        *current_tool   = Tool::Arc;
        *current_method = method;
        return true;
    }
    false
}

// ---- arc method picker row ------------------------------------------------
//
// Each row paints a small representative icon on the left and the method name
// on the right. Selected rows are highlighted; frozen rows are dimmed and not
// clickable.

fn arc_method_row(ui: &mut egui::Ui, current: ArcMethod, this: ArcMethod) -> bool {
    let enabled = this.enabled();
    let row_w = ui.available_width().max(280.0);
    let (resp, painter) = ui.allocate_painter(
        egui::vec2(row_w, 40.0),
        if enabled { egui::Sense::click() } else { egui::Sense::hover() },
    );
    let rect = resp.rect;
    let selected = current == this;

    let bg = if !enabled {
        egui::Color32::TRANSPARENT
    } else if selected {
        egui::Color32::from_rgb(48, 95, 165)
    } else if resp.hovered() {
        egui::Color32::from_rgba_unmultiplied(80, 90, 110, 90)
    } else {
        egui::Color32::TRANSPARENT
    };
    if bg.a() > 0 {
        painter.rect(rect, 4.0, bg, egui::Stroke::NONE);
    }
    if selected {
        painter.rect_stroke(rect, 4.0,
            egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 180, 255)));
    }

    // ICON area
    let icon_c   = rect.left_center() + egui::vec2(28.0, 0.0);
    let line_col = if !enabled { egui::Color32::from_rgb(95,100,110) }
                   else if selected { egui::Color32::from_rgb(230,240,255) }
                   else { egui::Color32::from_rgb(225,235,250) };
    let dot_col  = if !enabled { egui::Color32::from_rgb(110,118,130) }
                   else { egui::Color32::from_rgb(80, 160, 250) };
    paint_arc_method_icon(&painter, icon_c, this, line_col, dot_col);

    // TEXT
    let text_col = if !enabled { egui::Color32::from_rgb(125,130,140) }
                   else if selected { egui::Color32::from_rgb(230, 245, 255) }
                   else { egui::Color32::from_rgb(225, 232, 245) };
    painter.text(
        egui::pos2(rect.left() + 62.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        this.name(),
        egui::FontId::proportional(13.5),
        text_col,
    );

    if !enabled {
        painter.text(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "frozen",
            egui::FontId::proportional(10.0),
            egui::Color32::from_rgb(130, 135, 150),
        );
    }

    enabled && resp.clicked()
}

fn paint_arc_method_icon(
    p: &egui::Painter,
    c: egui::Pos2,
    m: ArcMethod,
    line_col: egui::Color32,
    dot_col: egui::Color32,
) {
    use std::f32::consts::FRAC_PI_2;
    let stroke = egui::Stroke::new(1.5, line_col);
    let thin   = egui::Stroke::new(1.0, line_col);
    let dot = |pt: egui::Pos2| p.circle_filled(pt, 3.0, dot_col);

    // shared quarter-arc going from (-r, 0) to (0, -r) in icon-space
    let r = 16.0;
    let n = 20;
    let arc_pts: Vec<egui::Pos2> = (0..=n).map(|i| {
        let t = FRAC_PI_2 * (i as f32 / n as f32);
        c + egui::vec2(-r * t.cos(), -r * t.sin() + 6.0)
    }).collect();
    p.add(egui::Shape::line(arc_pts.clone(), stroke));

    let start = arc_pts[0];
    let mid   = arc_pts[n / 2];
    let end   = arc_pts[n];
    let center = c + egui::vec2(0.0, 6.0);

    // small arrow helper (a short segment with a chevron)
    let arrow = |p: &egui::Painter, from: egui::Pos2, to: egui::Pos2| {
        p.line_segment([from, to], thin);
        let dir = (to - from).normalized();
        let perp = egui::vec2(-dir.y, dir.x);
        let tip = to;
        let back = tip - dir * 4.0;
        p.line_segment([tip, back + perp * 2.0], thin);
        p.line_segment([tip, back - perp * 2.0], thin);
    };

    match m {
        ArcMethod::ThreePoints => {
            dot(start); dot(mid); dot(end);
        }
        ArcMethod::StartCenterEnd => {
            dot(start); dot(end);
            dot(center);
            p.line_segment([center, start], thin);
        }
        ArcMethod::CenterStartEnd => {
            dot(center); dot(start); dot(end);
            arrow(p, center, end - egui::vec2(2.0, 0.0));
        }
        ArcMethod::StartCenterAngle => {
            dot(start); dot(center);
            arrow(p, center + egui::vec2(8.0, 0.0), center + egui::vec2(8.0, -8.0));
        }
        ArcMethod::StartCenterLength => {
            dot(start); dot(center);
            p.line_segment([start, start + egui::vec2(14.0, -6.0)],
                egui::Stroke::new(1.0, dot_col));
        }
        ArcMethod::StartEndAngle => {
            dot(start); dot(end);
            arrow(p, c + egui::vec2(-6.0, -4.0), c + egui::vec2(2.0, -10.0));
        }
        ArcMethod::StartEndDirection => {
            dot(start); dot(end);
            arrow(p, start, start + egui::vec2(0.0, -12.0));
        }
        ArcMethod::StartEndRadius => {
            dot(start); dot(end);
            arrow(p, c + egui::vec2(2.0, 4.0), start + egui::vec2(2.0, 0.0));
        }
        ArcMethod::CenterStartAngle => {
            dot(center); dot(start);
            arrow(p, center + egui::vec2(6.0, 0.0), center + egui::vec2(6.0, -10.0));
        }
        ArcMethod::CenterStartLength => {
            dot(center); dot(start);
            arrow(p, start, end);
        }
        ArcMethod::Continue => {
            // arrow tail merging into the arc start
            arrow(p, start - egui::vec2(8.0, -4.0), start);
        }
    }
}

// ---- the app --------------------------------------------------------------

impl eframe::App for CadApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ensure continuous repaint, never frozen
        ctx.request_repaint();
        self.trim_debug_frame = self.trim_debug_frame.wrapping_add(1);

        // Drain any in-flight hatch trace worker. If the worker
        // finished this frame, this materialises the result (Success
        // → push polylines + hatch dobject; Failure → fall back to
        // cheap path; Cancelled → log + clear prompt). No-op when
        // no worker is running. Runs FIRST so the rest of the frame
        // sees the updated doc state.
        self.poll_hatch_worker();

        // FPS — exponential moving average so the number doesn't jitter
        let dt = ctx.input(|i| i.stable_dt);
        if dt > 0.0 {
            let instant = 1.0 / dt;
            self.fps_smooth = if self.fps_smooth == 0.0 {
                instant
            } else {
                self.fps_smooth * 0.9 + instant * 0.1
            };
        }


        // global drafting-mode toggles (AutoCAD F-keys). Fire on key-press
        // anywhere — they're modeless and don't interfere with text input
        // since F-keys aren't typeable characters. Each one persists via
        // env.save() so the state survives restart.
        if ctx.input(|i| i.key_pressed(egui::Key::F7)) {
            self.env.GrdEnb = !self.env.GrdEnb;
            let _ = self.env.save();
            self.history.push(format!("  GRID {}", if self.env.GrdEnb { "on" } else { "off" }));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F8)) {
            self.env.OrtEnb = !self.env.OrtEnb;
            let _ = self.env.save();
            self.history.push(format!("  ORTHO {}", if self.env.OrtEnb { "on" } else { "off" }));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F9)) {
            self.env.GrdSnp = !self.env.GrdSnp;
            let _ = self.env.save();
            self.history.push(format!("  SNAP {}", if self.env.GrdSnp { "on" } else { "off" }));
        }

        // global Esc: cancel any in-progress draw or pick / intersect / select mode
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending.clear();
            self.pending_bulges.clear();
            self.pline_mode = PlineMode::Line;
            self.pline_arc_sub = PlineArcSub::Normal;
            self.tool = Tool::None;
            self.picking_source = false;
            // Set the cooperative-cancellation flag for any long-
            // running op currently in flight (or queued — flag is
            // reset at op start). When async/threading lands, this
            // is the signal the worker thread reads to bail out.
            self.op_cancel.store(true, Ordering::Relaxed);
            self.hatch_pick_point_armed = false;
            self.hatch_pick_point_session = None;
            self.pending_hatch_pattern = (None, 1.0, 0.0);
            self.hatch_dialog_open = false;
            self.intersect_pending_click = false;
            self.intersect_view_pending  = false;
            self.snap_override = None;
            // Item 3: Esc clears the command line input AND any current
            // pretext shown above it. The 2-stage-Enter counter resets too.
            self.cmd.clear();
            self.clear_prompt();
            if self.select_mode != SelectMode::Off {
                self.cancel_selection();
            }
            if self.move_state != MoveState::Off {
                self.move_state = MoveState::Off;
                self.history.push("  move cancelled".into());
            }
            if self.copy_state != CopyState::Off {
                self.copy_state = CopyState::Off;
                self.history.push("  copy cancelled".into());
            }
            if self.rotate_state != RotateState::Off {
                self.rotate_state = RotateState::Off;
                self.rotate_copy  = false;
                self.history.push("  rotate cancelled".into());
            }
            if self.scale_state != ScaleState::Off {
                self.scale_state = ScaleState::Off;
                self.scale_copy  = false;
                self.history.push("  scale cancelled".into());
            }
            if self.mirror_state != MirrorState::Off {
                self.mirror_state = MirrorState::Off;
                self.history.push("  mirror cancelled".into());
            }
            if self.matchprops_state != MatchPropsState::Off {
                self.matchprops_state = MatchPropsState::Off;
                self.history.push("  matchprop cancelled".into());
            }
            if self.offset_state != OffsetState::Off {
                self.offset_state = OffsetState::Off;
                self.history.push("  offset cancelled".into());
            }
            if self.lengthen_state != LengthenState::Off {
                self.lengthen_state = LengthenState::Off;
                self.history.push("  lengthen cancelled".into());
            }
            if self.break_state != BreakState::Off {
                self.break_state = BreakState::Off;
                self.history.push("  break cancelled".into());
            }
            if self.align_state != AlignState::Off {
                self.align_state = AlignState::Off;
                self.history.push("  align cancelled".into());
            }
            if self.fillet_state != FilletState::Off {
                self.fillet_state = FilletState::Off;
                self.fillet_multiple = false;
                self.fillet_waiting_radius = false;
                self.history.push("  fillet cancelled".into());
            }
            if self.chamfer_state != ChamferState::Off {
                self.chamfer_state = ChamferState::Off;
                self.chamfer_multiple = false;
                self.chamfer_waiting_distance = false;
                self.history.push("  chamfer cancelled".into());
            }
            if self.grip_drag.is_some() {
                self.grip_drag = None;
                self.history.push("  grip drag cancelled".into());
            }
            if self.stretch_state != StretchState::Off {
                self.stretch_state = StretchState::Off;
                self.history.push("  stretch cancelled".into());
            }
            // Trim / extend cancel — also restore the stashed main selection
            // if we're still in the cutting/boundary select phase.
            let trim_running = !matches!(self.trim_state, TrimState::Off);
            let extend_running = !matches!(self.extend_state, ExtendState::Off);
            if trim_running {
                self.trim_dbg("=== TRIM session END (Esc cancel) ===");
                self.trim_state = TrimState::Off;
                self.history.push("  trim cancelled".into());
            }
            if extend_running {
                self.trim_dbg("=== EXTEND session END (Esc cancel) ===");
                self.extend_state = ExtendState::Off;
                self.history.push("  extend cancelled".into());
            }
            if trim_running || extend_running {
                // Esc clears EVERYTHING — including any selection that was
                // stashed when the op began. Restoring it would resurrect
                // dashed-gray ghosts of items the user just cancelled
                // (bug from screenshot 2026-06-02). pre_op_selection is
                // only restored on SUCCESSFUL finalise, never on cancel.
                self.pre_op_selection.clear();
                self.selection.clear();
            }
        }

        // Enter (when the command line is empty) finalises an in-progress
        // selection — this is the LibreCAD / AutoCAD convention. The cmd
        // box's own Enter handler only fires when the text isn't empty, so
        // there's no double-handling.
        // ONE Enter press = ONE state transition per frame. See memo
        // `feedback_rust_cad_user_terminates_sessions` — the program
        // never auto-terminates editing sessions; only the user does,
        // and a single Enter must not chain through multiple phases.
        let enter_now  = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        let space_now  = ctx.input(|i| i.key_pressed(egui::Key::Space));
        let cmd_is_empty = self.cmd.trim().is_empty();
        // Item 4 — Space on a truly empty cmd line acts like Enter for the
        // repeat-last + 2-stage cancel logic below. We let the TextEdit
        // also see the space (harmless: cmd stays "trim-empty"), then
        // strip any leading whitespace at the end of this block.
        let trigger = enter_now || (space_now && cmd_is_empty);
        // Persistent hatch pick-point — Enter ends the session.
        // Sits at the top of the cascade so it consumes Enter before
        // any of the other handlers (which might re-run the last
        // command, etc.).
        if trigger && cmd_is_empty && self.hatch_pick_point_armed {
            self.hatch_pick_point_armed = false;
            self.hatch_pick_point_session = None;
            self.pending_hatch_pattern = (None, 1.0, 0.0);
            self.clear_prompt();
            self.hatch_dbg("pick-point session ended via Enter");
            self.history.push("  hatch pick-point session ended".into());
            if space_now { self.cmd.clear(); }
            return;
        }
        if trigger && cmd_is_empty {
            if self.select_mode != SelectMode::Off {
                // Item 4 — 2-stage cancel for a select-mode wait with an
                // EMPTY basket on non-cutter/non-boundary sessions.
                // (TRIM ForCuttingEdges and EXTEND ForBoundaryEdges keep
                // their documented "Enter = use ALL dobjects" semantics.)
                let basket_empty = self.selection.is_empty();
                let is_cutter_or_bound = matches!(self.select_mode,
                    SelectMode::ForCuttingEdges | SelectMode::ForBoundaryEdges);
                if basket_empty && !is_cutter_or_bound {
                    self.empty_enter_count_in_select += 1;
                    if self.empty_enter_count_in_select == 1 {
                        self.set_prompt(
                            "please make a selection (Enter again to cancel)");
                    } else {
                        // 2nd empty Enter cancels the whole command.
                        self.cancel_selection();
                        self.queued_op = QueuedOp::None;
                        self.clear_prompt();
                    }
                } else {
                    // Non-empty basket OR cutter/boundary mode → finalise
                    // (the existing behaviour, including "Enter = all").
                    self.finalise_selection();
                }
            } else if matches!(
                self.trim_state,
                TrimState::PickingTargets(_) | TrimState::PickingTargetsAll)
            {
                self.trim_dbg("=== TRIM session END (Enter) ===");
                self.trim_state = TrimState::Off;
                self.clear_prompt();
            } else if matches!(
                self.extend_state,
                ExtendState::PickingTargets(_) | ExtendState::PickingTargetsAll)
            {
                self.trim_dbg("=== EXTEND session END (Enter) ===");
                self.extend_state = ExtendState::Off;
                self.clear_prompt();
            } else if self.tool == Tool::Polyline && self.pending.len() >= 2 {
                let verts = self.drain_pline_pending(false);
                self.add_dobject(Geom::Polyline(Polyline {
                    vertices: verts, closed: false,
                }), "canvas");
            } else if self.tool == Tool::Spline && self.pending.len() >= 3 {
                // SPLINE commit — degree-3 (cubic) clamped/open uniform
                // NURBS through the captured control points, all
                // weights = 1.0 (non-rational B-spline). Cubic is the
                // CAD default; smaller pending counts get a lower
                // effective degree to keep the curve well-formed.
                let n = self.pending.len();
                let degree = 3.min(n - 1);
                let ctrls: Vec<Vec2> = self.pending.drain(..).collect();
                self.pending_bulges.clear();
                let spline = cad_kernel::Spline::new_bspline(degree, ctrls);
                self.add_dobject(Geom::Spline(spline), "canvas");
            } else {
                // Item 4 — fully idle: Enter on empty cmd repeats last cmd.
                if let Some(last) = self.last_command.clone() {
                    self.run_command(&last);
                }
            }
            // The Space that TextEdit also saw may have left a single
            // whitespace char in self.cmd; clean it so the repeated cmd's
            // box stays empty.
            if space_now { self.cmd.clear(); }
        }
        // Polyline `c`/`close` then Enter — handled separately because
        // it consumes a non-empty cmd line, so it doesn't collide with
        // the empty-Enter cascade above.
        if self.tool == Tool::Polyline && enter_now && !cmd_is_empty {
            let trimmed = self.cmd.trim().to_ascii_lowercase();
            if trimmed == "c" || trimmed == "close" || trimmed == "closed" {
                if self.pending.len() >= 2 {
                    let verts = self.drain_pline_pending(true);
                    self.add_dobject(Geom::Polyline(Polyline {
                        vertices: verts, closed: true,
                    }), "canvas (closed)");
                    self.cmd.clear();
                } else {
                    self.history.push("  ! polyline needs at least 2 vertices".into());
                    self.pending.clear();
                    self.pending_bulges.clear();
                    self.pline_mode = PlineMode::Line;
                }
            }
        }

        // ---- UI.2: MENUBAR (very top) -----------------------------------
        // Declared BEFORE the toolbar so it sits at the absolute top.
        // Every menu item dispatches via `run_command` so its behaviour
        // matches typing the same cmd — keeps one source of truth.
        egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New").clicked() {
                        self.run_command("clear");
                        ui.close_menu();
                    }
                    if ui.button("Open .dxf / .rsm…").clicked() {
                        self.run_command("open /tmp/in.dxf");
                        ui.close_menu();
                    }
                    if ui.button("Save As .dxf").clicked() {
                        self.run_command("saveas /tmp/out.dxf");
                        ui.close_menu();
                    }
                    if ui.button("Save As .rsm").clicked() {
                        self.run_command("saveas /tmp/out.rsm");
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Undo").clicked() {
                        self.run_command("undo");
                        ui.close_menu();
                    }
                    if ui.button("Redo").clicked() {
                        self.run_command("redo");
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Select All").clicked() {
                        self.run_command("select");
                        self.run_command("all");
                        ui.close_menu();
                    }
                    if ui.button("Deselect All").clicked() {
                        self.selection.clear();
                        self.selected = None;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Erase selection").clicked() {
                        self.run_command("erase");
                        ui.close_menu();
                    }
                    if ui.button("Match Properties").clicked() {
                        self.run_command("matchprop");
                        ui.close_menu();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Zoom Extents (fit all)").clicked() {
                        if !self.doc.dobjects.is_empty() {
                            let mut min = self.doc.dobjects[0].bbox().0;
                            let mut max = self.doc.dobjects[0].bbox().1;
                            for d in &self.doc.dobjects {
                                let (a, b) = d.bbox();
                                if a.x < min.x { min.x = a.x; }
                                if a.y < min.y { min.y = a.y; }
                                if b.x > max.x { max.x = b.x; }
                                if b.y > max.y { max.y = b.y; }
                            }
                            let center = (min + max) * 0.5;
                            let w = (max.x - min.x).max(max.y - min.y).max(1.0);
                            let r = ctx.screen_rect();
                            let target_px = (r.width().min(r.height()) * 0.85) as f64;
                            self.scale = (target_px / w) as f32;
                            self.world_offset = egui::vec2(
                                -center.x as f32, -center.y as f32);
                        }
                        ui.close_menu();
                    }
                    if ui.button("Reset View").clicked() {
                        self.scale = 20.0;
                        self.world_offset = egui::vec2(0.0, 0.0);
                        ui.close_menu();
                    }
                });
                ui.menu_button("Draw", |ui| {
                    for (label, cmd) in [
                        ("Line",      "line"),
                        ("Circle",    "circle"),
                        ("Arc",       "arc"),
                        ("Ellipse",   "ellipse"),
                        ("Polyline",  "polyline"),
                        ("Point",     "point"),
                    ] {
                        if ui.button(label).clicked() {
                            self.run_command(cmd);
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("Modify", |ui| {
                    for (label, cmd) in [
                        ("Move",     "move"),
                        ("Copy",     "copy"),
                        ("Rotate",   "rotate"),
                        ("Scale",    "scale"),
                        ("Mirror",   "mirror"),
                    ] {
                        if ui.button(label).clicked() {
                            self.run_command(cmd);
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    for (label, cmd) in [
                        ("Trim",     "trim"),
                        ("Extend",   "extend"),
                        ("Fillet",   "fillet"),
                        ("Chamfer",  "chamfer"),
                        ("Offset…",  "offset 1.0"),
                        ("Join",     "join"),
                        ("Break",    "break"),
                        ("Align",    "align"),
                        ("Stretch",  "stretch"),
                    ] {
                        if ui.button(label).clicked() {
                            self.run_command(cmd);
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("Tools", |ui| {
                    ui.label(egui::RichText::new("Palettes").small().color(
                        egui::Color32::from_rgb(150, 165, 185)));
                    ui.checkbox(&mut self.cmd_window_open,      "Command palette");
                    ui.checkbox(&mut self.layers_window_open,   "Layers");
                    ui.checkbox(&mut self.pens_window_open,     "Pens");
                    ui.checkbox(&mut self.info_window_open,     "Info / Properties");
                    ui.checkbox(&mut self.dobjects_window_open, "DObjects list");
                    ui.separator();
                    if ui.button("Snap window").clicked() {
                        self.snap_window_open = !self.snap_window_open;
                        ui.close_menu();
                    }
                    if ui.button("Toggle Grips").clicked() {
                        self.env.GrpEnb = !self.env.GrpEnb;
                        let _ = self.env.save();
                        ui.close_menu();
                    }
                    ui.separator();
                    // ---- Debug tools submenu --------------------------
                    // All diagnostic / development toggles live here in
                    // one place. Adding new debug instruments? Put the
                    // toggle here, not on the toolbar.
                    ui.menu_button("Debug tools", |ui| {
                        ui.checkbox(&mut self.screen_stats_open, "Screen Stats")
                            .on_hover_text(
                                "Renderer's view of the doc: total / in viewport / drawn / skipped");
                        ui.checkbox(&mut self.debug_open, "Render mode (CPU/GPU + APX)")
                            .on_hover_text("CPU/GPU toggle, APX (draft display), render stats");
                        ui.checkbox(&mut self.trim_debug_open, "Trim Debug Log")
                            .on_hover_text("Log every click + state transition during trim/extend");
                        ui.checkbox(&mut self.hatch_debug_open, "Hatch Debug Log")
                            .on_hover_text("Log dialog + pick-point + apply + render for hatch");
                        ui.separator();
                        // Spatial index — rebuild + status
                        let idx_label = if self.index_dirty || self.index.is_none() {
                            "Rebuild spatial index ⟲"
                        } else {
                            "Spatial index ✓ (fresh)"
                        };
                        if ui.button(idx_label).clicked() {
                            self.ensure_index();
                            ui.close_menu();
                        }
                        ui.separator();
                        // Intersection visualizer (diagnostic — shows ∩
                        // points so the user can verify a query).
                        ui.label(egui::RichText::new("Intersect visualizer").small().color(
                            egui::Color32::from_rgb(150, 165, 185)));
                        if ui.button("∩ view (whole viewport)").clicked() {
                            self.intersect_view_pending = true;
                            ui.close_menu();
                        }
                        let click_lbl = if self.intersect_pending_click {
                            "∩ click — waiting for click…"
                        } else {
                            "∩ click — arm for next click"
                        };
                        if ui.button(click_lbl).clicked() {
                            self.intersect_pending_click = !self.intersect_pending_click;
                            ui.close_menu();
                        }
                        if ui.button("Clear ∩ overlay").clicked() {
                            self.intersections.clear();
                            self.last_intersect_label.clear();
                            ui.close_menu();
                        }
                        ui.separator();
                        // Destructive — moved to Debug tools because it
                        // wipes the entire document with no confirmation.
                        if ui.button("Clear all dobjects (DESTRUCTIVE)").clicked() {
                            self.clear_all();
                            self.history.push("  cleared".into());
                            ui.close_menu();
                        }
                    });
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("Command help").clicked() {
                        self.run_command("help");
                        ui.close_menu();
                    }
                    if ui.button("About RUST_CAD").clicked() {
                        self.history.push(
                            "  RUST_CAD — pure-Rust 2-D CAD math workbench".into());
                        self.history.push(
                            "  github.com/HSI-Lighting/RUST-AutoRASM".into());
                        ui.close_menu();
                    }
                });
            });
        });

        // ---- top toolbar ------------------------------------------------
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let prev = self.tool;
                tool_button(ui, &mut self.tool, Tool::None,   "pointer");
                ui.add_space(4.0);
                tool_button(ui, &mut self.tool, Tool::Line,     "line");
                tool_button(ui, &mut self.tool, Tool::Circle,   "circle");
                // Hatch is a one-shot command (not a persistent draw
                // tool); custom-painted icon — clicking opens the
                // Choose Hatch Attributes dialog same as the `hatch`
                // cmd-line word. Placed next to circle per user req.
                if hatch_command_button(ui) {
                    self.run_command("hatch");
                }
                tool_button(ui, &mut self.tool, Tool::Ellipse,    "ellipse");
                tool_button(ui, &mut self.tool, Tool::EllipseArc, "ell.arc");
                tool_button(ui, &mut self.tool, Tool::Point,    "point");
                tool_button(ui, &mut self.tool, Tool::Polyline, "pline");
                tool_button(ui, &mut self.tool, Tool::Spline,   "spline");
                // Three quick-access buttons for the functional arc methods.
                let prev_method = self.arc_method;
                arc_tool_button(ui, &mut self.tool, &mut self.arc_method,
                                ArcMethod::ThreePoints,    "3p");
                arc_tool_button(ui, &mut self.tool, &mut self.arc_method,
                                ArcMethod::StartCenterEnd, "SCE");
                arc_tool_button(ui, &mut self.tool, &mut self.arc_method,
                                ArcMethod::CenterStartEnd, "CSE");
                if ui.button("▾ more arcs")
                    .on_hover_text("all 11 arc construction methods incl. frozen")
                    .clicked()
                {
                    self.arc_picker_open = !self.arc_picker_open;
                }
                if self.tool != prev || self.arc_method != prev_method {
                    self.pending.clear();
                }
                ui.add_space(20.0);
                // ---- User-facing panel toggles ---------------------------
                // Styled to match the drafting tool buttons (same height,
                // same color scheme) so the toolbar reads as one strip.
                // Debug-only buttons moved to Tools → Debug tools menu.
                if panel_button(ui, "array…", self.array_open) {
                    self.array_open = !self.array_open;
                }
                // OSNAP settings (floating window) + active-snaps badge.
                let snap_btn = format!("snap…\n{}", active_snap_letters(self.snap_enabled));
                if panel_button(ui, &snap_btn, self.snap_window_open) {
                    self.snap_window_open = !self.snap_window_open;
                }
                // GRIPS toggle (also: cmd `grips`, or GrpEnb in settings).
                let grips_btn = if self.env.GrpEnb { "grips\nON" } else { "grips\noff" };
                if panel_button(ui, grips_btn, self.env.GrpEnb) {
                    self.env.GrpEnb = !self.env.GrpEnb;
                }
                if panel_button(ui, "settings…", self.settings_open) {
                    self.settings_open = !self.settings_open;
                }
                if panel_button(ui, "layers", self.layer_panel_open) {
                    self.layer_panel_open = !self.layer_panel_open;
                }
                if panel_button(ui, "pens", self.pen_panel_open) {
                    self.pen_panel_open = !self.pen_panel_open;
                }
                if panel_button(ui, "info", self.info_panel_open) {
                    self.info_panel_open = !self.info_panel_open;
                }
                ui.add_space(20.0);
                if !self.last_intersect_label.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(180, 200, 220),
                        &self.last_intersect_label,
                    );
                }
                ui.add_space(20.0);
                // Active-tool indicator
                let green = egui::Color32::from_rgb(120, 220, 160);
                let grey  = egui::Color32::from_rgb(140, 150, 165);
                let (label_s, color) = match self.tool {
                    Tool::None   => (String::from("idle — no tool active"), grey),
                    Tool::Line    => (String::from("DRAWING LINE"), green),
                    Tool::Circle  => (String::from("DRAWING CIRCLE"), green),
                    Tool::Ellipse    => (String::from("DRAWING ELLIPSE"), green),
                    Tool::EllipseArc => (String::from("DRAWING ELLIPTICAL ARC"), green),
                    Tool::Point     => (String::from("PLACING POINT"), green),
                    Tool::Polyline  => (format!("DRAWING POLYLINE ({} verts; Enter to finish, 'c' Enter to close)",
                        self.pending.len()), green),
                    Tool::Spline    => (format!("DRAWING SPLINE ({} ctrl pts; Enter finishes after \u{2265}3)",
                        self.pending.len()), green),
                    Tool::Arc     => (format!("DRAWING ARC ({})",
                        self.arc_method.name()), green),
                };
                ui.colored_label(color, egui::RichText::new(label_s)
                    .monospace().size(14.0).strong());
            });
            ui.add_space(4.0);
        });

        // ---- debug window (CPU/GPU render toggle + stats) -------------------
        if self.debug_open {
            let mut keep = true;
            let mode_before = self.render_mode;
            let dobject_count = self.doc.dobjects.len();
            let circle_count = self.doc.dobjects.iter()
                .filter(|d| matches!(d.geom, Geom::Circle(_))).count();
            let fps = self.fps_smooth;
            let win = egui::Window::new("DEBUG — render mode")
                .open(&mut keep)
                .resizable(true)
                .default_width(310.0)
                .min_width(280.0)
                .min_height(180.0)
                .default_pos(egui::pos2(20.0, 130.0));
            let win = self.apply_dock_pos("DEBUG — render mode", ctx, win);
            let resp = win.show(ctx, |ui| {
                    ui.label(egui::RichText::new("Render mode")
                        .monospace().strong());
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.render_mode,
                            RenderMode::Cpu, "CPU (egui painter)");
                    });
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.render_mode,
                            RenderMode::Gpu, "GPU (instanced circles + CPU lines/arcs)");
                    });
                    ui.separator();
                    // ---- APX (draft display) toggle ------------------
                    // Sits alongside the CPU/GPU choice because it's
                    // the same axis of trade-off: fidelity vs speed.
                    // When ON, every visible dobject collapses to a
                    // single dot at its bbox center — one instanced
                    // GPU draw call for the whole scene, FPS recovers
                    // from single-digit to 60+ on million-dobject
                    // drawings. Toggle off when you need to see real
                    // geometry. Click + drag, snap, pick, move, copy
                    // still work — only the visual is approximate.
                    ui.label(egui::RichText::new("APX (draft display)")
                        .monospace().strong());
                    let mut apx = self.lod_active;
                    let apx_label = if apx {
                        "● APX ON  — geometry shown as dots"
                    } else {
                        "○ APX OFF — full geometry"
                    };
                    if ui.add(egui::Button::new(
                        egui::RichText::new(apx_label).monospace()
                    ).fill(if apx {
                        egui::Color32::from_rgb(80, 50, 20)
                    } else {
                        egui::Color32::from_rgb(40, 45, 55)
                    })).clicked() {
                        apx = !apx;
                    }
                    if apx != self.lod_active {
                        self.lod_active = apx;
                        self.history.push(format!(
                            "  APX → {} (manual)",
                            if self.lod_active { "ON (geometry shown as dots)" }
                            else { "OFF (full geometry)" }));
                    }
                    ui.separator();
                    ui.monospace(format!("FPS         {:>6.1}", fps));
                    ui.monospace(format!("dobjects    {:>6}", dobject_count));
                    ui.monospace(format!("  circles   {:>6}", circle_count));
                    ui.monospace(format!("  other     {:>6}",
                        dobject_count.saturating_sub(circle_count)));
                    ui.separator();
                    ui.label(egui::RichText::new("Notes").small());
                    ui.small("• GPU path: one PaintCallback, one glDrawArraysInstanced");
                    ui.small("• GPU renders Circles only this slice; Lines/Arcs stay CPU");
                    ui.small("• APX: ALL types render as dots — works even in CPU mode");
                    ui.small("• Selection / snap / hit-test always use real geometry");
                });
            self.process_dock_after_show("DEBUG — render mode", ctx, resp);
            if !keep { self.debug_open = false; }
            if self.render_mode != mode_before {
                self.history.push(format!(
                    "  render mode → {:?}", self.render_mode
                ));
                self.gpu_dirty = true;
            }
        }

        // ---- snap settings window -----------------------------------------
        if self.snap_window_open {
            let mut keep = true;
            egui::Window::new("OBJECT SNAP — running osnaps")
                .open(&mut keep)
                .resizable(false)
                .collapsible(false)
                .default_width(280.0)
                .default_pos(egui::pos2(20.0, 360.0))
                .show(ctx, |ui| {
                    ui.label("Snaps to find automatically while you hover.");
                    ui.label("Type the same name in the command line to use it once.");
                    ui.separator();
                    for k in SnapKind::ALL {
                        let mut on = self.snap_enabled.is_enabled(k);
                        let label = format!("{:<5}  {}", k.name(), snap_blurb(k));
                        if ui.checkbox(&mut on, label).changed() {
                            self.snap_enabled.set(k, on);
                        }
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("search radius (SpTGSZ)");
                        ui.add(egui::Slider::new(&mut self.env.SpTGSZ, 4..=80)
                            .suffix(" px"));
                    });
                    ui.horizontal(|ui| {
                        if ui.button("All on").clicked() {
                            for k in SnapKind::ALL { self.snap_enabled.set(k, true); }
                        }
                        if ui.button("All off").clicked() {
                            self.snap_enabled = SnapSet::default();
                        }
                        if ui.button("Defaults").clicked() {
                            self.snap_enabled = SnapSet::defaults();
                        }
                    });
                });
            if !keep { self.snap_window_open = false; }
        }

        // ---- User-Environment Settings window ------------------------------
        if self.settings_open {
            let mut keep = true;
            let mut save_now = false;
            egui::Window::new("USER-ENVIRONMENT SETTINGS")
                .open(&mut keep)
                .resizable(true)
                .default_width(760.0)
                .default_height(560.0)
                .default_pos(egui::pos2(40.0, 80.0))
                .show(ctx, |ui| {
                    ui.label("AutoCAD-style SYSVARS for RUST_CAD. Persists to ~/.config/rust_cad/user_env.txt");
                    ui.separator();
                    // Horizontal split: settings list on the left, live
                    // preview on the right. Preview reflects current values
                    // in real time as the user drags sliders / toggles boxes.
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_min_width(450.0);
                            ui.set_max_width(520.0);
                            egui::ScrollArea::vertical()
                                .id_salt("env_scroll")
                                .max_height(440.0)
                                .show(ui, |ui| {
                        ui.heading("Snap & picking");
                        env_u8(ui, "SpTGSZ", "Object-snap target height (px)",
                            &mut self.env.SpTGSZ, 4, 80);
                        env_u8(ui, "PkBxSz", "Pickbox height (px)",
                            &mut self.env.PkBxSz, 1, 40);
                        env_u8(ui, "CrsHrS", "Crosshair size (% of viewport)",
                            &mut self.env.CrsHrS, 1, 100);

                        ui.separator();
                        ui.heading("Dialogs");
                        env_bool(ui, "AtDlgM", "Attribute entry dialog on INSERT",
                            &mut self.env.AtDlgM);
                        env_bool(ui, "AtPrmM", "Attribute prompting during INSERT",
                            &mut self.env.AtPrmM);
                        env_bool(ui, "CmDlgM", "Dialog boxes for PLOT, etc.",
                            &mut self.env.CmDlgM);
                        env_bool(ui, "FlDlgM", "Use OS file-navigation dialogs",
                            &mut self.env.FlDlgM);

                        ui.separator();
                        ui.heading("Display");
                        env_u8_choice(ui, "DrDspM", "Dragging display during MOVE/COPY",
                            &mut self.env.DrDspM, &["off", "on", "auto"]);
                        env_bool(ui, "MnuBar", "Classic menu bar",
                            &mut self.env.MnuBar);
                        env_bool(ui, "TltEnb", "Toolbar/ribbon tooltips",
                            &mut self.env.TltEnb);
                        env_bool(ui, "RllTp",  "Tooltips on dobject rollover",
                            &mut self.env.RllTp);
                        env_bool(ui, "SelPrv", "Preview-highlight on hover",
                            &mut self.env.SelPrv);
                        env_bool(ui, "HltSel", "Highlight selected dobjects",
                            &mut self.env.HltSel);
                        env_u8_choice(ui, "WpFrmM", "Wipeout frame display",
                            &mut self.env.WpFrmM, &["off", "on", "on for selection only"]);

                        ui.separator();
                        ui.heading("Grips");
                        env_bool(ui, "GrpEnb", "Enable grips",
                            &mut self.env.GrpEnb);
                        env_bool(ui, "GrpBlk", "Grips inside blocks",
                            &mut self.env.GrpBlk);
                        env_color(ui, "GrClrU", "Unselected grip colour",
                            &mut self.env.GrClrU);
                        env_color(ui, "GrClrS", "Selected (hot) grip colour",
                            &mut self.env.GrClrS);
                        env_u8(ui, "GrpSz",  "Grip size (px)",
                            &mut self.env.GrpSz, 1, 20);
                        env_u8(ui, "GrpHvR", "Grip hover + grab radius (px)",
                            &mut self.env.GrpHvR, 4, 80);

                        ui.separator();
                        ui.heading("External references");
                        env_u8_choice(ui, "XrLdMd", "Xref demand-loading mode",
                            &mut self.env.XrLdMd, &["off", "on", "on with copy"]);
                        env_text(ui, "XrTmpP", "Temp path for xref copies",
                            &mut self.env.XrTmpP);
                            });   // ← close inner ScrollArea
                        });       // ← close left vertical
                        ui.separator();
                        // Right column: live preview
                        ui.vertical(|ui| {
                            ui.heading("Live preview");
                            ui.small("Reflects current values in real time.");
                            ui.add_space(4.0);
                            draw_settings_preview(ui, &self.env);
                        });
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Save now").clicked() { save_now = true; }
                        if ui.button("Reload from disk").clicked() {
                            self.env = UserEnv::load();
                        }
                        if ui.button("Reset to defaults").clicked() {
                            self.env = UserEnv::default();
                        }
                    });
                });
            if !keep { self.settings_open = false; }
            if save_now {
                match self.env.save() {
                    Ok(_)  => self.history.push("  settings saved".into()),
                    Err(e) => self.history.push(format!("  ! settings save failed: {}", e)),
                }
            }
        }

        // ---- arc method picker ----------------------------------------------
        if self.arc_picker_open {
            let mut keep = true;
            egui::Window::new("ARC CREATION METHODS")
                .open(&mut keep)
                .resizable(false)
                .collapsible(false)
                .default_pos(egui::pos2(20.0, 80.0))
                .show(ctx, |ui| {
                    ui.set_min_width(310.0);
                    let mut chosen: Option<ArcMethod> = None;
                    for (i, &m) in ALL_ARC_METHODS.iter().enumerate() {
                        // visually group: 1 alone, 2-4 S,C,*, 5-7 S,E,*, 8-10 C,S,*, 11 Continue
                        if i == 1 || i == 4 || i == 7 || i == 10 {
                            ui.separator();
                        }
                        if arc_method_row(ui, self.arc_method, m) {
                            chosen = Some(m);
                        }
                    }
                    if let Some(m) = chosen {
                        self.arc_method = m;
                        self.tool = Tool::Arc;
                        self.pending.clear();
                        self.arc_picker_open = false;
                    }
                });
            if !keep { self.arc_picker_open = false; }
        }

        // ---- array dialog --------------------------------------------------
        // The dialog uses the STANDARD selection-basket flow for picking
        // sources: "Select sources ↓" begins a SelectMode::ForSelect
        // session with QueuedOp::Array. The array dialog HIDES during
        // that session (`self.select_mode != SelectMode::Off` is the
        // gate); user clicks dobjects (basket grows, dashed-gray
        // rendering), presses Enter; QueuedOp::Array's finalise handler
        // re-shows this dialog. Multi-source: every source goes into
        // every grid cell.
        let in_array_pick = self.array_open
            && self.queued_op == QueuedOp::Array
            && self.select_mode != SelectMode::Off;
        if self.array_open && !in_array_pick {
            {
                let mut do_generate = false;
                let mut close_it    = false;
                let mut start_pick  = false;
                let sources: Vec<(usize, String)> = self.selection.iter()
                    .filter_map(|&i| self.doc.dobjects.get(i)
                        .map(|d| (i, describe(&d.geom))))
                    .collect();
                egui::Window::new("Rectangular Array")
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.set_min_width(360.0);
                        ui.label("Duplicates the selected dobject(s) into a grid.");
                        ui.separator();

                        // Source row: "Select sources" button + count + first-source preview
                        ui.horizontal(|ui| {
                            if ui.button("Select sources ↓")
                                .on_hover_text("Begin a selection session — click dobjects, Enter to finish")
                                .clicked()
                            {
                                start_pick = true;
                            }
                            if sources.is_empty() {
                                ui.colored_label(
                                    egui::Color32::from_rgb(255, 140, 140),
                                    "no sources selected");
                            } else {
                                ui.label(format!("{} source(s):", sources.len()));
                            }
                        });
                        if !sources.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt("array_sources")
                                .max_height(80.0)
                                .show(ui, |ui| {
                                    for (i, d) in &sources {
                                        ui.monospace(format!("  #{} {}", i, d));
                                    }
                                });
                        }
                        ui.separator();

                        ui.horizontal(|ui| {
                            ui.label("columns");
                            ui.add(egui::DragValue::new(&mut self.array_cols)
                                .range(1..=3000_usize).speed(1));
                            ui.label("× rows");
                            ui.add(egui::DragValue::new(&mut self.array_rows)
                                .range(1..=3000_usize).speed(1));
                        });
                        ui.horizontal(|ui| {
                            ui.label("dx");
                            ui.add(egui::DragValue::new(&mut self.array_dx).speed(1.0));
                            ui.label("    dy");
                            ui.add(egui::DragValue::new(&mut self.array_dy).speed(1.0));
                        });
                        let cells = self.array_cols * self.array_rows;
                        let new_dobjects = cells.saturating_sub(1) * sources.len().max(1);
                        let total_after = self.doc.dobjects.len() + new_dobjects;
                        ui.label(format!(
                            "{} cell(s) × {} source(s) = {} new dobjects → {} total",
                            cells, sources.len(), new_dobjects, total_after
                        ));
                        if total_after > 1500 {
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 200, 80),
                                "• intersection recompute will be skipped above ~1500 (O(N²))",
                            );
                        }
                        if total_after > 50_000 {
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 140, 140),
                                "• rendering above ~50k dobjects may lag (CPU painter)",
                            );
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!sources.is_empty(),
                                              egui::Button::new("Generate")).clicked() {
                                do_generate = true;
                            }
                            if ui.button("Close").clicked() {
                                close_it = true;
                            }
                        });
                    });
                if start_pick {
                    // Begin a fresh selection session for picking
                    // sources. The basket may already hold whatever
                    // the user selected before opening the dialog —
                    // we DON'T clear it, so prior selections carry
                    // over (user can shift-click to remove).
                    self.queued_op = QueuedOp::Array;
                    self.begin_selection(SelectMode::ForSelect);
                    self.set_prompt(
                        "array: pick source dobject(s), Enter to finish  [Esc=cancel]".to_string());
                }
                if do_generate { self.generate_array(); }
                if close_it    { self.array_open = false; }
            }
        }

        // ---- floating: Screen Stats (renderer's view of the doc) -------
        // Always called — it checks `screen_stats_open` and bails if
        // closed. Open by default since the user wanted to confirm
        // the app knows what's on screen.
        self.render_screen_stats_window(ctx);

        // ---- left panel: Layer dock (Slice B) ---------------------------
        if self.layer_panel_open {
            self.render_layer_panel(ctx);
        }

        // ---- left panel (further left): Pen palette (Slice C) -----------
        if self.pen_panel_open {
            self.render_pen_palette(ctx);
        }

        // ---- left panel: Entity Info (Slice D) --------------------------
        if self.info_panel_open {
            self.render_info_panel(ctx);
        }

        // ---- floating: Trim Debug log (instrumentation) ----------------
        if self.trim_debug_open {
            self.render_trim_debug_window(ctx);
        }

        // ---- floating: ACI color picker (polar wheel) ------------------
        // Renders only when a call site has set `aci_pick_request`.
        self.render_aci_picker_window(ctx);

        // ---- floating: Hatch attributes dialog -------------------------
        // Modal-ish (resizable=false, collapsible=false), opens when
        // bare `hatch` is typed; closes on OK / Cancel / X.
        self.render_hatch_dialog(ctx);

        // ---- floating: Hatch Debug Log (instrumentation) ---------------
        if self.hatch_debug_open {
            self.render_hatch_debug_window(ctx);
        }

        // ---- DObjects palette — floating Window -------------------------
        let mut dobjects_open = self.dobjects_window_open;
        let dobjects_count = self.doc.dobjects.len();
        let win = egui::Window::new(format!("DObjects ({})", dobjects_count))
            .open(&mut dobjects_open)
            .default_pos(egui::pos2(
                ctx.screen_rect().right() - 320.0, 70.0))
            .default_size(egui::vec2(300.0, 520.0))
            .min_width(220.0)
            .resizable(true)
            .collapsible(true);
        let win = self.apply_dock_pos("DObjects", ctx, win);
        let resp = win.show(ctx, |ui| {
            if self.picking_source {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 220, 100),
                    "PICK MODE — click any dobject below or on the canvas",
                );
            }
            // Virtual scrolling — only renders rows actually on screen, so the
            // list cost is bounded by visible_rows, not by dobject count.
            let row_h = ui.text_style_height(&egui::TextStyle::Body);
            let dobject_count = self.doc.dobjects.len();
            let mut to_delete: Option<usize> = None;
            egui::ScrollArea::vertical()
                .id_salt("ent_scroll")
                .max_height(ui.available_height() * 0.55)
                .auto_shrink([false; 2])
                .show_rows(ui, row_h, dobject_count, |ui, range| {
                    for i in range {
                        let label = format!("#{:>6}  {}", i, describe(&self.doc.dobjects[i].geom));
                        ui.horizontal(|ui| {
                            let resp = ui.selectable_label(self.selected == Some(i), label);
                            if resp.clicked() {
                                self.selected = Some(i);
                                if self.picking_source {
                                    self.picking_source = false;
                                }
                            }
                            if ui.small_button("✕").clicked() {
                                to_delete = Some(i);
                            }
                        });
                    }
                });
            if let Some(i) = to_delete {
                self.doc.dobjects.remove(i);
                self.selected = None;
                self.intersections.clear();
                self.index_dirty = true;
            }

            ui.separator();
            ui.heading(format!("Intersections ({})", self.intersections.len()));
            egui::ScrollArea::vertical().id_salt("int_scroll").show(ui, |ui| {
                for (i, p) in self.intersections.iter().enumerate() {
                    ui.monospace(format!("{:>3}  ({:>10.4}, {:>10.4})", i, p.x, p.y));
                }
            });
        });
        self.process_dock_after_show("DObjects", ctx, resp);
        self.dobjects_window_open = dobjects_open;

        // ---- UI.1: STATUS BAR (very bottom) -----------------------------
        // Declared BEFORE the cmd panel so it sits at the absolute bottom
        // edge; cmd panel ends up above it (egui stacks bottoms inward).
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(22.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // ---- LEFT: cursor world coords -----------------------
                    let cursor_world = ctx.input(|i| i.pointer.hover_pos())
                        .and_then(|p| {
                            // Translate from screen to world via the same
                            // formula as self.s2w (we don't have `rect`
                            // here, so reproduce it from ctx).
                            let r = ctx.screen_rect();
                            let c = r.center();
                            Some(Vec2::new(
                                ((p.x - c.x) / self.scale - self.world_offset.x) as f64,
                                (-(p.y - c.y) / self.scale - self.world_offset.y) as f64,
                            ))
                        });
                    let coord_text = match cursor_world {
                        Some(w) => format!("{:>11.4}, {:>11.4}", w.x, w.y),
                        None    => format!("{:>11}, {:>11}", "—", "—"),
                    };
                    ui.label(egui::RichText::new(coord_text)
                        .monospace()
                        .color(egui::Color32::from_rgb(160, 200, 240)));

                    ui.separator();

                    // ---- ACTIVE LAYER ------------------------------------
                    let active_layer_name = self.doc.layers.get(self.doc.layers.active)
                        .map(|l| l.name.as_str()).unwrap_or("?").to_string();
                    let active_layer_col = self.doc.layers.get(self.doc.layers.active)
                        .map(|l| {
                            let (r, g, b) = resolve_color(
                                l.color, self.doc.layers.active,
                                &self.doc.layers, &self.doc.truecolors);
                            egui::Color32::from_rgb(r, g, b)
                        })
                        .unwrap_or(egui::Color32::WHITE);
                    let (swatch_rect, _) = ui.allocate_exact_size(
                        egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(swatch_rect, 1.0, active_layer_col);
                    ui.painter().rect_stroke(swatch_rect, 1.0,
                        egui::Stroke::new(0.6, egui::Color32::from_rgb(60, 70, 85)));
                    ui.label(egui::RichText::new(format!("Layer: {}", active_layer_name))
                        .monospace().small());

                    ui.separator();

                    // ---- SELECTION COUNT ---------------------------------
                    let sel_n = self.selection.len();
                    if sel_n > 0 {
                        ui.label(egui::RichText::new(format!("{} sel", sel_n))
                            .monospace().small()
                            .color(egui::Color32::from_rgb(180, 220, 100)));
                        ui.separator();
                    }

                    // ---- RIGHT-aligned controls --------------------------
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Zoom level (rightmost)
                        ui.label(egui::RichText::new(
                            format!("zoom {:.2}× ({:.2} px/u)",
                                self.scale, self.scale)
                        ).monospace().small().color(egui::Color32::from_rgb(150, 165, 185)));
                        ui.separator();
                        // ---- Render-perf badges: [APX] [GPU/CPU] -----
                        // Both are user-toggled, no auto-switch. APX
                        // collapses every visible dobject to a single
                        // dot (one instanced GPU draw call — fast even
                        // at 3M+ dobjects). GPU swaps the renderer to
                        // the instanced-circle path (circles fast,
                        // other primitives still CPU).
                        let perf_badge = |ui: &mut egui::Ui, label: &str, on: bool, tip: &str| {
                            let col = if on {
                                egui::Color32::from_rgb(255, 200, 80)   // warm amber when active
                            } else {
                                egui::Color32::from_rgb(80, 90, 105)
                            };
                            let resp = ui.add(egui::Label::new(
                                egui::RichText::new(label).monospace().small().strong().color(col)
                            ).sense(egui::Sense::click()));
                            resp.on_hover_text(tip).clicked()
                        };
                        let gpu_on = self.render_mode == RenderMode::Gpu;
                        if perf_badge(ui, "GPU", gpu_on,
                            "Renderer: GPU (instanced circles) vs CPU (egui painter). \
                             GPU is much faster for circle-heavy scenes; other primitives \
                             still go through CPU regardless.")
                        {
                            self.render_mode = if gpu_on { RenderMode::Cpu } else { RenderMode::Gpu };
                            self.gpu_dirty = true;
                            self.history.push(format!(
                                "  render mode → {:?} (manual)", self.render_mode));
                        }
                        if perf_badge(ui, "APX", self.lod_active,
                            "Approximate display: every visible dobject becomes a single \
                             dot at its bbox center. One GPU draw call for the whole scene — \
                             FPS recovers from single-digit to 60+ on million-dobject drawings. \
                             Selection / snap / hit-testing still use the real geometry.")
                        {
                            self.lod_active = !self.lod_active;
                            self.history.push(format!(
                                "  APX → {} (manual)",
                                if self.lod_active { "ON (geometry shown as dots)" }
                                else { "OFF (full geometry)" }));
                        }
                        ui.separator();
                        // EdgMod toggle
                        let mut em = self.env.EdgMod;
                        if ui.checkbox(&mut em, "EdgMod").changed() {
                            self.env.EdgMod = em;
                            let _ = self.env.save();
                        }
                        ui.separator();
                        // GrpEnb toggle
                        let mut ge = self.env.GrpEnb;
                        if ui.checkbox(&mut ge, "Grips").changed() {
                            self.env.GrpEnb = ge;
                            let _ = self.env.save();
                        }
                        ui.separator();
                        // Drafting-mode toggles: GRID (F7), SNAP (F9), ORTHO (F8).
                        // Clickable badges (same affordance as the osnap row).
                        let drafting_badge = |ui: &mut egui::Ui, label: &str, on: bool, tip: &str| {
                            let col = if on {
                                egui::Color32::from_rgb(120, 240, 255)
                            } else {
                                egui::Color32::from_rgb(80, 90, 105)
                            };
                            let resp = ui.add(egui::Label::new(
                                egui::RichText::new(label).monospace().small().color(col)
                            ).sense(egui::Sense::click()));
                            resp.on_hover_text(tip).clicked()
                        };
                        if drafting_badge(ui, "ORTHO", self.env.OrtEnb,
                            "Lock drafting orientation (F8) — cursor pulled to horizontal or vertical from the anchor point.")
                        {
                            self.env.OrtEnb = !self.env.OrtEnb;
                            let _ = self.env.save();
                        }
                        if drafting_badge(ui, "SNAP", self.env.GrdSnp,
                            "Snap to grid (F9) — cursor rounds to the nearest GrdSpc multiple.")
                        {
                            self.env.GrdSnp = !self.env.GrdSnp;
                            let _ = self.env.save();
                        }
                        if drafting_badge(ui, "GRID", self.env.GrdEnb,
                            "Show grid (F7) — dots at GrdSpc world-unit intervals.")
                        {
                            self.env.GrdEnb = !self.env.GrdEnb;
                            let _ = self.env.save();
                        }
                        ui.separator();
                        // Snap badges — click any letter to toggle.
                        for k in SnapKind::ALL {
                            let on = self.snap_enabled.is_enabled(k);
                            let label = k.name();   // "END", "MID", …
                            let col = if on {
                                egui::Color32::from_rgb(120, 240, 255)
                            } else {
                                egui::Color32::from_rgb(80, 90, 105)
                            };
                            let resp = ui.add(egui::Label::new(
                                egui::RichText::new(label).monospace().small().color(col),
                            ).sense(egui::Sense::click()));
                            if resp.clicked() {
                                self.snap_enabled.set(k, !on);
                            }
                            if resp.hovered() {
                                resp.on_hover_text(snap_blurb(k));
                            }
                        }
                    });
                });
            });

        // ---- Cmd palette — floating Window (AutoCAD-style) -------------
        // History + prompt + input combined in one draggable, resizable
        // window. Default position: lower-left of the screen.
        let cmd_default_pos = {
            let r = ctx.screen_rect();
            egui::pos2(r.left() + 360.0, r.bottom() - 220.0)
        };
        let mut cmd_open = self.cmd_window_open;
        let win = egui::Window::new("Command")
            .open(&mut cmd_open)
            .default_pos(cmd_default_pos)
            .default_size(egui::vec2(720.0, 180.0))
            .min_width(360.0)
            .min_height(120.0)
            .resizable(true)
            .collapsible(true);
        let win = self.apply_dock_pos("Command", ctx, win);
        let resp = win.show(ctx, |ui| {
                // Reserve space at the bottom for: prompt line (if any) +
                // the input row.
                let prompt_h = if self.current_prompt.is_empty() { 0.0 } else { 18.0 };
                let bottom_reserve = 32.0 + prompt_h;
                egui::ScrollArea::vertical()
                    .id_salt("hist_scroll")
                    .stick_to_bottom(true)
                    .max_height(ui.available_height() - bottom_reserve)
                    .show(ui, |ui| {
                        for h in &self.history {
                            ui.monospace(h);
                        }
                    });
                // Item 1 — the only pretext shown above the input is the
                // CURRENT prompt for the active command. Replaces the
                // growing pile of historical prompts in the history pane.
                if !self.current_prompt.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 220, 120),
                        &self.current_prompt,
                    );
                }
                ui.horizontal(|ui| {
                    ui.label(">");
                    let btn_w = 56.0_f32;
                    let row_h = ui.spacing().interact_size.y;
                    let text_resp = ui.add_sized(
                        [(ui.available_width() - btn_w - 8.0).max(40.0), row_h],
                        egui::TextEdit::singleline(&mut self.cmd),
                    );
                    let run_clicked = ui.button("run").clicked();
                    // Enter is detected both via the lost-focus pattern AND
                    // by a global pressed-this-frame check while focused, so
                    // the input never silently drops.
                    let enter_pressed = (text_resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        || (text_resp.has_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    // AutoCAD-style "Space submits". In the cmd line — and
                    // only in the cmd line — any non-empty input commits
                    // when the user presses Space, exactly as Enter would.
                    // (Other text edits like layer rename or the picker's
                    // manual ACI box still treat Space as a literal char,
                    // because this check is scoped to `text_resp.has_focus()`.)
                    //
                    // Trade-off: one-liner syntax like `line 0,0 10,10`
                    // cannot be typed at the prompt — the first space
                    // commits `line` and enters draw mode. The user can
                    // still feed multi-arg commands via menu actions or
                    // by typing each arg followed by Space at the next
                    // prompt (the AutoCAD interaction model).
                    let space_pressed = text_resp.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Space));
                    let submit_via_space = space_pressed
                        && !self.cmd.trim_end_matches(' ').is_empty();
                    if submit_via_space {
                        // Strip the trailing space the TextEdit already
                        // appended to self.cmd before we got control.
                        self.cmd = self.cmd.trim_end_matches(' ').to_string();
                    }
                    if enter_pressed || run_clicked || submit_via_space {
                        if !self.cmd.trim().is_empty() {
                            let c = std::mem::take(&mut self.cmd);
                            self.run_command(&c);
                        }
                        self.refocus_cmd = true;
                    }
                    // Always-listen: keep keyboard focus on the command line
                    // by default. We yield it only when some other widget
                    // ACTIVELY has focus (a DragValue being edited, another
                    // TextEdit). Canvas clicks land without grabbing focus
                    // themselves, but the OS may have dropped it — the
                    // `refocus_cmd` flag (set after canvas clicks) forces a
                    // reclaim on the next frame.
                    let other_focused = ctx.memory(|m| {
                        m.focused().is_some_and(|id| id != text_resp.id)
                    });
                    // Modal text edits elsewhere in the UI must own focus
                    // exclusively (layer rename today; will grow as more
                    // dialogs land). Suppress the always-listen reclaim
                    // while any are active.
                    let modal_textedit_active = self.layer_rename.is_some();
                    if modal_textedit_active {
                        self.refocus_cmd = false;
                    } else if self.refocus_cmd && !other_focused {
                        text_resp.request_focus();
                        self.refocus_cmd = false;
                    } else if !other_focused && !text_resp.has_focus() {
                        text_resp.request_focus();
                    }
                });
            });
        self.process_dock_after_show("Command", ctx, resp);
        self.cmd_window_open = cmd_open;

        // ---- central panel: canvas --------------------------------------
        egui::CentralPanel::default().show(ctx, |ui| {
            let avail = ui.available_size();
            let (resp, painter) =
                ui.allocate_painter(avail, egui::Sense::click_and_drag());
            let rect = resp.rect;
            // Stash the canvas rect for the dock helpers — they need
            // the canvas area (below toolbar, above status bar) not
            // the full window rect.
            self.canvas_screen_rect = Some(rect);

            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(18, 22, 28));

            // ---- Background grid (GrdEnb) ---------------------------------
            //
            // Renders a dot grid at GrdSpc world-unit intervals across the
            // visible viewport. Skips entirely if disabled, if spacing is
            // non-positive, or if the spacing would produce > 50 000 dots
            // (zoomed too far out — would be a black smear anyway). Drawn
            // BEFORE dobjects so they sit on top of it. The same GrdSpc
            // value is what GrdSnp rounds to.
            if self.env.GrdEnb && self.env.GrdSpc > 0.0 {
                let s = self.env.GrdSpc;
                let bl = self.s2w(rect.left_bottom(), rect);
                let tr = self.s2w(rect.right_top(),   rect);
                let x0 = (bl.x / s).floor() * s;
                let y0 = (bl.y / s).floor() * s;
                let x1 = (tr.x / s).ceil()  * s;
                let y1 = (tr.y / s).ceil()  * s;
                let cols = ((x1 - x0) / s).round() as i64 + 1;
                let rows = ((y1 - y0) / s).round() as i64 + 1;
                if cols > 0 && rows > 0 && (cols * rows) < 50_000 {
                    let dot_col = egui::Color32::from_rgb(60, 70, 85);
                    let mut y = y0;
                    while y <= y1 + s * 0.5 {
                        let mut x = x0;
                        while x <= x1 + s * 0.5 {
                            let p = self.w2s(Vec2::new(x, y), rect);
                            painter.circle_filled(p, 0.9, dot_col);
                            x += s;
                        }
                        y += s;
                    }
                }
            }

            // pan with middle/right drag
            if resp.dragged_by(egui::PointerButton::Middle)
                || resp.dragged_by(egui::PointerButton::Secondary)
            {
                let d = resp.drag_delta();
                self.world_offset += egui::vec2(d.x / self.scale, -d.y / self.scale);
            }

            // wheel zoom around cursor
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(cursor) = resp.hover_pos() {
                    let before = self.s2w(cursor, rect);
                    let factor = (scroll * 0.0015).exp();
                    self.scale = (self.scale * factor).clamp(0.01, 5000.0);
                    let after = self.s2w(cursor, rect);
                    let dx = (after.x - before.x) as f32;
                    let dy = (after.y - before.y) as f32;
                    self.world_offset += egui::vec2(dx, dy);
                }
            }

            // ---- snap candidates + Tab-cycling -----------------------------
            // Collect EVERY viable snap target at the current cursor, sorted
            // by (priority, distance). The first is the default; Tab cycles
            // through the rest. Cursor motion (> 4 px) resets the cycle.
            //
            // Snap fires ONLY for true point-pick phases — phases where the
            // click commits a precise coordinate (draw, move base/dest,
            // mirror axis pts, etc.). Pure hit-test phases (trim cutter
            // pick, trim target click, fillet pick-line, matchprops source,
            // …) suppress snap candidates because the click identifies a
            // dobject, not a point — snap markers are visual noise there.
            //
            // The typed-in-cmd snap override (END / MID / …) ALWAYS wins
            // via find_all_snaps's `forced` parameter and re-enables snap
            // for one click even in hit-test phases. See memo
            // `feedback_rust_cad_inline_snap_override_supersedes`.
            let snap_phase_active =
                self.tool != Tool::None
                || self.snap_override.is_some()
                || self.move_state       != MoveState::Off
                || self.copy_state       != CopyState::Off
                || self.rotate_state     != RotateState::Off
                || self.scale_state      != ScaleState::Off
                || self.mirror_state     != MirrorState::Off
                || self.align_state      != AlignState::Off
                || self.stretch_state    != StretchState::Off
                || self.break_state      != BreakState::Off;
            // Phantom dobject for the in-progress polyline so snap kinds
            // (END / MID / CEN / …) work against vertices the user has
            // just clicked but hasn't committed yet. Cheap — only built
            // when the pline tool is mid-flow with at least 2 vertices.
            let pline_phantom: Option<DObject> = self.pline_phantom_dobject();

            let snap_candidates: Vec<SnapHit> = if snap_phase_active
                && !self.picking_source && !self.intersect_pending_click
                && (!self.doc.dobjects.is_empty() || pline_phantom.is_some())
            {
                resp.hover_pos().map(|cur| {
                    let world = self.s2w(cur, rect);
                    let world_radius = self.env.SpTGSZ as f64 / self.scale as f64;
                    let grid = if self.index_dirty { None } else { self.index.as_ref() };
                    let mut hits = if self.doc.dobjects.is_empty() {
                        Vec::new()
                    } else {
                        find_all_snaps(
                            world, world_radius,
                            self.snap_enabled, self.snap_override,
                            self.pending.last().copied(),
                            &self.doc.dobjects, grid,
                        )
                    };
                    if let Some(ref phantom) = pline_phantom {
                        let phantom_slice = std::slice::from_ref(phantom);
                        let phantom_hits = find_all_snaps(
                            world, world_radius,
                            self.snap_enabled, self.snap_override,
                            self.pending.last().copied(),
                            phantom_slice, None,
                        );
                        hits.extend(phantom_hits);
                        // Re-sort merged list by (priority, distance) so the
                        // closest snap across both sources wins.
                        hits.sort_by(|a, b| {
                            a.kind.priority().cmp(&b.kind.priority())
                                .then(a.point.dist(world).partial_cmp(&b.point.dist(world))
                                    .unwrap_or(std::cmp::Ordering::Equal))
                        });
                    }
                    hits
                }).unwrap_or_default()
            } else {
                Vec::new()
            };

            // Reset cycle when cursor moves to a meaningfully different spot.
            if let Some(cur) = resp.hover_pos() {
                let moved_far = self.snap_cycle_anchor
                    .map_or(true, |anc| (cur - anc).length() > 4.0);
                if moved_far {
                    self.snap_cycle_index = 0;
                    self.snap_cycle_anchor = Some(cur);
                }
            }

            // Tab → cycle to next candidate. Consume the key so egui doesn't
            // shuffle widget focus (the cmd line will reclaim it anyway).
            let tab_pressed = ctx.input_mut(|i|
                i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
            );
            if tab_pressed && !snap_candidates.is_empty() {
                self.snap_cycle_index = (self.snap_cycle_index + 1) % snap_candidates.len();
            }
            // Clamp in case the candidate count shrank since last frame.
            if !snap_candidates.is_empty() && self.snap_cycle_index >= snap_candidates.len() {
                self.snap_cycle_index = 0;
            }
            let snap_hit: Option<SnapHit> = snap_candidates
                .get(self.snap_cycle_index).copied();

            // Left-click handling:
            //   - if SELECT MODE is active: toggle dobject under cursor, or
            //     start / close a window-selection rectangle.
            //   - if ∩ click is armed: compute intersections in 50px around it.
            //   - if PICK MODE (array source): hit-test dobjects.
            //   - else if a tool is active: register a draw point.
            // Diagnostic for "click on screen didn't start drawing" reports.
            // Logs which click handler branch a left-click ended up in. The
            // Trim Debug Log window already exists; we piggyback on it.
            // resp.clicked() fires only when egui classifies the press+release
            // as a click (small motion); drag_stopped() fires when a release
            // ends a drag — including a "click" that egui saw as a tiny drag.
            let click_now    = resp.clicked();
            let drag_stopped = resp.drag_stopped();
            // Unified click/drag classifier:
            //   1. select mode is active → DRAG is the rubber-band window
            //   2. Shift held during press → DRAG is an ad-hoc window
            //   3. anything else → press-release is ALWAYS a click
            // The 5-px motion heuristic is gone — egui's idea of "this was
            // a tiny drag" doesn't get a vote.
            let press_release_dist = match (
                ctx.input(|i| i.pointer.press_origin()),
                resp.interact_pointer_pos(),
            ) {
                (Some(p), Some(r)) => (r - p).length(),
                _ => 0.0,
            };
            let shift_held = ctx.input(|i| i.modifiers.shift);
            let in_select  = self.select_mode != SelectMode::Off;
            // Track press time so the classifier below can enforce a
            // hold-threshold before treating a drag as a window. See
            // feedback_rust_cad_universal_selection_model. Cleared on
            // release.
            let now = ctx.input(|i| i.time);
            if ctx.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer() {
                self.press_time = Some(now);
            }
            if ctx.input(|i| i.pointer.primary_released()) {
                self.press_time = None;
            }
            let hold_thresh_secs = (self.env.SelDmTm as f64) / 1000.0;
            let press_held_secs = self.press_time.map(|t0| now - t0).unwrap_or(0.0);
            let hold_threshold_passed = press_held_secs >= hold_thresh_secs;
            let in_click_only_phase =
                self.tool != Tool::None
                || matches!(self.trim_state,
                    TrimState::PickingTargets(_) | TrimState::PickingTargetsAll)
                || matches!(self.extend_state,
                    ExtendState::PickingTargets(_) | ExtendState::PickingTargetsAll)
                || self.move_state       != MoveState::Off
                || self.copy_state       != CopyState::Off
                || self.rotate_state     != RotateState::Off
                || self.scale_state      != ScaleState::Off
                || self.mirror_state     != MirrorState::Off
                || self.align_state      != AlignState::Off
                || self.break_state      != BreakState::Off
                || self.lengthen_state   != LengthenState::Off
                || self.offset_state     != OffsetState::Off
                || self.stretch_state    != StretchState::Off
                || self.matchprops_state != MatchPropsState::Off
                || self.fillet_state     != FilletState::Off
                || self.chamfer_state    != ChamferState::Off
                || self.picking_source
                || self.intersect_pending_click;
            // Drag is the rubber-band window only when (a) we're in
            // select mode, or (b) the user explicitly held Shift to
            // request a window drag. Edit phases (trim/draw/move/…)
            // keep the "always click" semantic too.
            //
            // Time-gated activation: the press must have been held
            // longer than env.SelDmTm (default 250 ms) before a drag
            // counts as a window. A fast accidental drag during a
            // click = still a click. The rubber-band preview honors
            // the same gate (see ~10333). Reference:
            // feedback_rust_cad_universal_selection_model.
            //
            // Shift-drag is exempt from the time gate — when the user
            // is explicitly holding Shift to force a window-drag,
            // they don't need to also hold the button to "prove" it.
            let drag_intent_is_window =
                ((in_select && hold_threshold_passed)
                 || (shift_held && !in_click_only_phase))
                && press_release_dist > 1.0;     // any real motion at all
            let drag_was_a_click = drag_stopped && !drag_intent_is_window;

            // ---- Drafting-mode PRESS-fires-click override ---------------
            //
            // Where the gesture has no drag semantic (every drawing tool
            // + every point-pick edit phase), register the click at PRESS
            // time instead of release. Same affordance as the AutoCAD
            // pickbox: pressing AT a point captures THAT point, even if
            // the cursor drifts a few pixels between press and release.
            // Drag-meaningful gestures (select-mode rubber-band, Shift-
            // drag window, grip drag) stay on release — they need both
            // endpoints. Visual cue: the square+cross drafting cursor
            // is drawn iff in_click_only_phase.
            let press_now = in_click_only_phase
                && ctx.input(|i| i.pointer.primary_pressed())
                && resp.contains_pointer();
            let click_now = if in_click_only_phase { press_now } else { click_now };
            // In drafting mode the press fired the click; suppress the
            // release-time drag-promoted click so we don't double-fire
            // when egui later reports a tiny accidental drag_stopped.
            let drag_was_a_click = drag_was_a_click && !in_click_only_phase;
            // ---- Grip drag handling (v2: per-grip role semantics) -----------
            // Two ways to grab a grip in pointer mode + GrpEnb:
            //   (a) press-and-drag (release = commit)
            //   (b) click-to-grab → cursor moves → click again to place
            // Both paths set self.grip_drag with the GripRole; the kernel's
            // Geom::with_grip_moved() decides what changes (e.g. circle
            // quadrant → radius; line midpoint → translate whole line).
            let pointer_mode_idle = !in_click_only_phase && self.select_mode == SelectMode::Off;
            let mut grip_drag_consumed_click = false;
            if pointer_mode_idle && self.env.GrpEnb {
                let drag_started = resp.drag_started_by(egui::PointerButton::Primary);
                // (a) Drag-grab: a primary-button drag begins near a grip.
                // (b) Click-grab: a clicked() event lands near a grip AND
                //     no grip is currently held.
                let try_grab = drag_started
                    || (click_now && self.grip_drag.is_none());
                if try_grab && self.grip_drag.is_none() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let cur_world = self.s2w(pos, rect);
                        // Match the visual hover-highlight radius so any
                        // grip that looks lit-up actually grabs on click.
                        // GrpHvR is in screen pixels; convert to world.
                        let tol = self.env.GrpHvR as f64 / self.scale as f64;
                        let mut targets: Vec<usize> = self.selection.clone();
                        if let Some(s) = self.selected { targets.push(s); }
                        targets.sort_unstable(); targets.dedup();
                        'outer: for &idx in &targets {
                            let Some(d) = self.doc.dobjects.get(idx) else { continue; };
                            for (gp, role) in d.geom.grip_points() {
                                if cur_world.dist(gp) < tol {
                                    self.grip_drag = Some(GripDrag {
                                        dobject_idx: idx,
                                        role,
                                        grip_origin: gp,
                                    });
                                    // Don't treat this click as a "select-
                                    // toggle click" — it's a grab.
                                    grip_drag_consumed_click = true;
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
                // Commit on drag_stopped (path a) OR on a subsequent click
                // anywhere on the canvas (path b).
                if let Some(gd) = self.grip_drag {
                    let drag_release  = resp.drag_stopped_by(egui::PointerButton::Primary);
                    let click_release = click_now && !grip_drag_consumed_click;
                    if drag_release || click_release {
                        if let Some(pos) = resp.interact_pointer_pos() {
                            let drop_world = self.s2w(pos, rect);
                            let delta = drop_world - gd.grip_origin;
                            // Suppress accidental no-op drags (tiny mouse
                            // jitter). Click-grab pairs may legitimately
                            // have zero motion if user clicks twice on the
                            // same spot — that's a no-op, just clear state.
                            if delta.len() > 1e-9 {
                                self.snapshot_doc();
                                if let Some(d) = self.doc.dobjects.get_mut(gd.dobject_idx) {
                                    d.geom = d.geom.with_grip_moved(gd.role, drop_world);
                                }
                                self.intersections.clear();
                                self.index_dirty = true;
                                self.gpu_dirty = true;
                                self.history.push(format!(
                                    "  ⊕ grip: #{} {:?} → ({:.3}, {:.3})",
                                    gd.dobject_idx, gd.role,
                                    drop_world.x, drop_world.y));
                            }
                        }
                        self.grip_drag = None;
                        grip_drag_consumed_click = true;
                    }
                }
            }
            // Drag-window handler: when drag_intent_is_window fires on
            // release, capture (press, release) as the two opposite
            // corners and apply a selection window. L→R = window (only
            // fully-inside dobjects); R→L = crossing (anything touching).
            // For ad-hoc Shift+drag (no select_mode), open a transient
            // ForSelect session, apply the window, then finalise
            // immediately so the basket persists.
            let mut window_drag_consumed_click = false;
            if drag_stopped && drag_intent_is_window && !grip_drag_consumed_click {
                if let (Some(p), Some(r)) = (
                    ctx.input(|i| i.pointer.press_origin()),
                    resp.interact_pointer_pos(),
                ) {
                    let press_world   = self.s2w(p, rect);
                    let release_world = self.s2w(r, rect);
                    let was_off = self.select_mode == SelectMode::Off;
                    if was_off {
                        // Shift+drag from idle — open a one-shot ForSelect
                        // window, apply, finalise.
                        self.begin_selection(SelectMode::ForSelect);
                    }
                    self.add_window_selection(press_world, release_world, false);
                    if was_off {
                        self.finalise_selection();
                    } else {
                        // Inside select_mode: stay in the session so the
                        // user can add more windows / clicks. window_first
                        // would re-arm naturally on next click.
                        self.window_first = None;
                    }
                    window_drag_consumed_click = true;
                }
            }
            let click_fired = (click_now || drag_was_a_click)
                && !grip_drag_consumed_click
                && !window_drag_consumed_click;
            if (click_now || drag_stopped) && self.trim_debug_open {
                // Only log when the user has the diagnostic window open, to
                // keep the log uncluttered.
                let drag_motion = if drag_stopped {
                    press_release_dist
                } else { 0.0 };
                let gates = format!(
                    "tool={:?} pending={} move={:?} copy={:?} rotate={:?} \
                     scale={:?} mirror={:?} align={:?} break={:?} lengthen={:?} \
                     offset={:?} stretch={:?} matchprops={:?} trim={} extend={} \
                     select_mode={:?} pick_src={} ∩pend={}",
                    self.tool, self.pending.len(),
                    self.move_state, self.copy_state, self.rotate_state,
                    self.scale_state, self.mirror_state, self.align_state,
                    self.break_state, self.lengthen_state,
                    self.offset_state, self.stretch_state, self.matchprops_state,
                    match self.trim_state {
                        TrimState::Off => "Off",
                        TrimState::SelectingCutters => "SelectingCutters",
                        TrimState::PickingTargets(_) => "PickingTargets(list)",
                        TrimState::PickingTargetsAll => "PickingTargetsAll",
                    },
                    match self.extend_state {
                        ExtendState::Off => "Off",
                        ExtendState::SelectingBoundaries => "SelectingBoundaries",
                        ExtendState::PickingTargets(_) => "PickingTargets(list)",
                        ExtendState::PickingTargetsAll => "PickingTargetsAll",
                    },
                    self.select_mode, self.picking_source, self.intersect_pending_click,
                );
                self.trim_dbg(format!(
                    "CLICK {} (press→release={:.1}px{}) | {}",
                    if click_now { "clicked()" } else { "drag_stopped()" },
                    drag_motion,
                    if drag_was_a_click { ", promoted to click" } else { "" },
                    gates,
                ));
            }
            if click_fired {
                if let Some(pos) = resp.interact_pointer_pos() {
                    // `world` keeps its old meaning (raw cursor world pos)
                    // for downstream branches that need it unconstrained
                    // (selection hit-test, intersect-click, etc.).
                    let world = self.s2w(pos, rect);
                    // AutoCAD priority for the captured POINT: osnap > ortho
                    // > grid-snap > raw. Used wherever a click commits a
                    // point (line endpoint, move base/dest, …).
                    let click_world = snap_hit.map(|h| h.point)
                        .unwrap_or_else(|| self.apply_constraints(world));

                    // Hatch pick-point — consumes the click before any
                    // other handler. BPOLY-style pipeline: tries a
                    // self-closed containing dobject first; falls
                    // through to ray-cast + boundary trace + island
                    // detect + materialise if needed.
                    //
                    // Session stays ARMED across clicks: each pick
                    // creates one hatch, then we re-fill
                    // pending_hatch_pattern from the session snapshot
                    // and update the prompt for the next pick.
                    // Enter / Esc ends the session.
                    if self.hatch_pick_point_armed {
                        self.hatch_dbg(format!(
                            "pick-point click at world ({:.3}, {:.3})",
                            world.x, world.y));
                        self.apply_pick_point_hatch(world);
                        // Restore the pattern for the next click in
                        // this session; refresh the prompt so the user
                        // knows we're still waiting for picks.
                        if let Some(pat) = self.hatch_pick_point_session.clone() {
                            self.pending_hatch_pattern = pat;
                            let style = self.hatch_pick_point_session.as_ref()
                                .and_then(|(n, _, _)| n.clone())
                                .unwrap_or_else(|| "SOLID".to_string());
                            self.set_prompt(format!(
                                "hatch ({}): click another region OR Enter to finish  [Esc=cancel]",
                                style));
                        }
                        return;
                    }

                    if self.move_state != MoveState::Off {
                        match self.move_state {
                            MoveState::WaitingForBase => {
                                self.move_state = MoveState::WaitingForDest(click_world);
                                self.history.push(format!(
                                    "    move: BASE = ({:.3}, {:.3}) — click DESTINATION",
                                    click_world.x, click_world.y));
                            }
                            MoveState::WaitingForDest(base) => {
                                let v = click_world - base;
                                self.apply_move(v);
                                self.move_state = MoveState::Off;
                                self.history.push(format!(
                                    "  move ✓ vector ({:.3}, {:.3}) applied to {} dobject(s)",
                                    v.x, v.y, self.selection.len()));
                            }
                            MoveState::Off => unreachable!(),
                        }
                        self.refocus_cmd = true;
                    } else if self.copy_state != CopyState::Off {
                        match self.copy_state {
                            CopyState::WaitingForBase => {
                                self.copy_state = CopyState::WaitingForDest(click_world);
                                self.history.push(format!(
                                    "    copy: BASE = ({:.3}, {:.3}) — click DESTINATION",
                                    click_world.x, click_world.y));
                            }
                            CopyState::WaitingForDest(base) => {
                                let v = click_world - base;
                                self.apply_copy(v);
                                self.copy_state = CopyState::Off;
                            }
                            CopyState::Off => unreachable!(),
                        }
                        self.refocus_cmd = true;
                    } else if self.rotate_state != RotateState::Off {
                        match self.rotate_state {
                            RotateState::WaitingForPivot => {
                                self.rotate_state = RotateState::WaitingForAngle(click_world);
                                self.set_prompt(format!(
                                    "rotate (pivot=({:.2},{:.2})): click to pick angle, or type number (CCW=+), R=reference, C={}",
                                    click_world.x, click_world.y,
                                    if self.rotate_copy { "copy ON" } else { "copy off" }));
                            }
                            RotateState::WaitingForAngle(pivot) => {
                                // Default: angle from pivot to click point.
                                // Zero baseline = +X axis (atan2 of vector
                                // from pivot to cursor). Positive = CCW.
                                let signed = (click_world - pivot).angle();
                                self.apply_rotate_or_copy(pivot, signed);
                                self.rotate_state = RotateState::Off;
                                self.rotate_copy = false;
                                self.clear_prompt();
                            }
                            // ---- Reference sub-command (3 picks) -------
                            // Mirrors scale-R: 2 picks define the source
                            // direction (anywhere), then ONE click anchored
                            // at the pivot defines the new direction.
                            RotateState::WaitingForRefSrc1(pivot) => {
                                self.rotate_state = RotateState::WaitingForRefSrc2(pivot, click_world);
                                self.set_prompt("rotate-R: click SOURCE point 2 (defines current direction)");
                            }
                            RotateState::WaitingForRefSrc2(pivot, s1) => {
                                let src_angle = (click_world - s1).angle();
                                self.rotate_state = RotateState::WaitingForRefTgt(pivot, src_angle);
                                self.set_prompt(format!(
                                    "rotate-R: click NEW direction (anchored at pivot) OR type angle [src={:.2}°]",
                                    src_angle.to_degrees()));
                            }
                            RotateState::WaitingForRefTgt(pivot, src_angle) => {
                                let tgt = (click_world - pivot).angle();
                                let mut dtheta = (tgt - src_angle).rem_euclid(std::f64::consts::TAU);
                                if dtheta > std::f64::consts::PI {
                                    dtheta -= std::f64::consts::TAU;
                                }
                                self.apply_rotate_or_copy(pivot, dtheta);
                                self.rotate_state = RotateState::Off;
                                self.rotate_copy = false;
                                self.clear_prompt();
                            }
                            RotateState::Off => unreachable!(),
                        }
                        self.refocus_cmd = true;
                    } else if self.scale_state != ScaleState::Off {
                        match self.scale_state {
                            ScaleState::WaitingForPivot => {
                                self.scale_state = ScaleState::WaitingForFactor(click_world);
                                self.set_prompt(format!(
                                    "scale (pivot=({:.2},{:.2})): click for factor (= distance from pivot), type number, R=reference, C={}",
                                    click_world.x, click_world.y,
                                    if self.scale_copy { "copy ON" } else { "copy off" }));
                            }
                            ScaleState::WaitingForFactor(pivot) => {
                                // Default: click distance from pivot = scale factor.
                                let factor = pivot.dist(click_world);
                                if factor < EPS {
                                    self.history.push("  ! click too close to pivot — factor would be 0".into());
                                } else {
                                    self.apply_scale_or_copy(pivot, factor);
                                }
                                self.scale_state = ScaleState::Off;
                                self.scale_copy  = false;
                                self.clear_prompt();
                            }
                            // ---- Reference sub-command (R) -------------
                            ScaleState::WaitingForRefStart(pivot) => {
                                self.scale_state = ScaleState::WaitingForRefEnd(pivot, click_world);
                                self.set_prompt("scale-R: click REFERENCE end (defines old length)");
                            }
                            ScaleState::WaitingForRefEnd(pivot, ref_start) => {
                                let ref_d = ref_start.dist(click_world);
                                if ref_d < EPS {
                                    self.history.push("  ! reference endpoints coincide".into());
                                    self.scale_state = ScaleState::Off;
                                    self.scale_copy  = false;
                                    self.clear_prompt();
                                } else {
                                    self.scale_state = ScaleState::WaitingForNewLength(pivot, ref_d);
                                    self.set_prompt(format!(
                                        "scale-R: click for NEW length (= distance from pivot) OR type number  [ref={:.3}]",
                                        ref_d));
                                }
                            }
                            ScaleState::WaitingForNewLength(pivot, ref_d) => {
                                let new_len = pivot.dist(click_world);
                                if new_len < EPS {
                                    self.history.push("  ! click too close to pivot".into());
                                } else {
                                    self.apply_scale_or_copy(pivot, new_len / ref_d);
                                }
                                self.scale_state = ScaleState::Off;
                                self.scale_copy  = false;
                                self.clear_prompt();
                            }
                            ScaleState::Off => unreachable!(),
                        }
                        self.refocus_cmd = true;
                    } else if matches!(
                        self.trim_state,
                        TrimState::PickingTargets(_) | TrimState::PickingTargetsAll)
                    {
                        // Resolve the effective cutter list. In "all" mode
                        // it's recomputed from doc.dobjects every click so
                        // pieces created by THIS session's trims keep
                        // acting as cutters.
                        let all_mode = matches!(self.trim_state, TrimState::PickingTargetsAll);
                        let cutters: Vec<usize> = if all_mode {
                            (0..self.doc.dobjects.len()).collect()
                        } else if let TrimState::PickingTargets(c) = &self.trim_state {
                            c.clone()
                        } else { Vec::new() };
                        let tol_world = 10.0 / self.scale as f64;
                        let hit = self.nearest_entity_under(world, tol_world);
                        self.trim_dbg(format!(
                            "TRIM target click  world={}  screen={}  hit={}  cutters={}",
                            Self::fmt_v(click_world),
                            format!("({:.1},{:.1})", pos.x, pos.y),
                            match hit {
                                Some(i) => format!("#{}", i),
                                None    => "VOID (no dobject under cursor)".into(),
                            },
                            if all_mode {
                                format!("ALL(dynamic, n={})", cutters.len())
                            } else {
                                format!("{:?}", cutters)
                            },
                        ));
                        if let Some(tgt) = hit {
                            let n_before = self.doc.dobjects.len();
                            // Note this BEFORE the trim — we use it to
                            // decide whether the new pieces inherit cutter
                            // status from a cutter parent (see memo
                            // `feedback_rust_cad_trim_pieces_inherit_cutter_status`).
                            let tgt_was_cutter = cutters.contains(&tgt);
                            let did_trim = self.apply_trim_pick(&cutters, tgt, click_world);
                            let n_after = self.doc.dobjects.len();
                            let net = n_after as i64 - n_before as i64;
                            self.trim_dbg(format!(
                                "  → apply_trim_pick success={}  dobjects {}→{}  (net {:+})",
                                did_trim, n_before, n_after, net));
                            // Patch the cutter list ONLY in explicit-list
                            // mode and ONLY when the doc actually changed.
                            // In all-mode the next click re-derives cutters
                            // from doc, so no patch is needed.
                            if did_trim && !all_mode {
                                // After remove(tgt) + append N pieces, the
                                // doc has (n_before - 1 + n_pieces) entries.
                                // n_pieces = n_after - (n_before - 1).
                                let n_pieces = n_after + 1 - n_before;
                                let first_new = n_after - n_pieces;
                                let patched: Vec<usize> = if let TrimState::PickingTargets(c) = &mut self.trim_state {
                                    c.retain(|&i| i != tgt);
                                    for c_i in c.iter_mut() {
                                        if *c_i > tgt { *c_i -= 1; }
                                    }
                                    // INHERIT: if the trimmed target was a
                                    // cutter, its new pieces are cutters too.
                                    if tgt_was_cutter && n_pieces > 0 {
                                        c.extend(first_new..n_after);
                                    }
                                    c.clone()
                                } else { Vec::new() };
                                if tgt_was_cutter && n_pieces > 0 {
                                    self.trim_dbg(format!(
                                        "  → cutters patched (parent #{} was a cutter → {} new pieces inherit) = {:?}",
                                        tgt, n_pieces, patched));
                                } else {
                                    self.trim_dbg(format!(
                                        "  → cutters patched = {:?}", patched));
                                }
                            } else if all_mode {
                                self.trim_dbg(
                                    "  → cutters: ALL mode (next click re-derives from doc)".to_string());
                            } else {
                                self.trim_dbg(
                                    "  → cutter list UNCHANGED (trim failed; preserving cutters)".to_string());
                            }
                        } else {
                            // Void click — log + do nothing. Session continues.
                            self.history.push(
                                "  trim — void click (no dobject) — session continues, click another target or press Enter".into());
                        }
                        self.refocus_cmd = true;
                    } else if matches!(
                        self.extend_state,
                        ExtendState::PickingTargets(_) | ExtendState::PickingTargetsAll)
                    {
                        let all_bounds_mode = matches!(self.extend_state, ExtendState::PickingTargetsAll);
                        let bounds: Vec<usize> = if all_bounds_mode {
                            (0..self.doc.dobjects.len()).collect()
                        } else if let ExtendState::PickingTargets(b) = &self.extend_state {
                            b.clone()
                        } else { Vec::new() };
                        let tol_world = 10.0 / self.scale as f64;
                        let hit = self.nearest_entity_under(world, tol_world);
                        self.trim_dbg(format!(
                            "EXTEND target click  world={}  hit={}  bounds={:?}",
                            Self::fmt_v(click_world),
                            match hit {
                                Some(i) => format!("#{}", i),
                                None    => "VOID".into(),
                            },
                            bounds,
                        ));
                        if let Some(tgt) = hit {
                            self.apply_extend_pick(&bounds, tgt, click_world);
                        } else {
                            self.history.push(
                                "  extend — void click — session continues, click another target or press Enter".into());
                        }
                        self.refocus_cmd = true;
                    } else if self.offset_state != OffsetState::Off {
                        if let OffsetState::WaitingForSide(d) = self.offset_state {
                            self.apply_offset(d, click_world);
                        }
                        self.offset_state = OffsetState::Off;
                        self.refocus_cmd = true;
                    } else if self.lengthen_state != LengthenState::Off {
                        if let LengthenState::WaitingForSide(d) = self.lengthen_state {
                            self.apply_lengthen(d, click_world);
                        }
                        self.lengthen_state = LengthenState::Off;
                        self.refocus_cmd = true;
                    } else if self.break_state != BreakState::Off {
                        self.apply_break(click_world);
                        self.break_state = BreakState::Off;
                        self.refocus_cmd = true;
                    } else if self.align_state != AlignState::Off {
                        match self.align_state {
                            AlignState::WaitingForSrc1 => {
                                self.align_state = AlignState::WaitingForSrc2(click_world);
                                self.history.push(format!(
                                    "    align: SRC1 = ({:.2},{:.2}) — click SOURCE point 2",
                                    click_world.x, click_world.y));
                            }
                            AlignState::WaitingForSrc2(s1) => {
                                self.align_state = AlignState::WaitingForTgt1(s1, click_world);
                                self.history.push(
                                    "    align: SRC2 captured — click TARGET point 1".into());
                            }
                            AlignState::WaitingForTgt1(s1, s2) => {
                                self.align_state = AlignState::WaitingForTgt2(s1, s2, click_world);
                                self.history.push(
                                    "    align: TGT1 captured — click TARGET point 2".into());
                            }
                            AlignState::WaitingForTgt2(s1, s2, t1) => {
                                self.apply_align(s1, s2, t1, click_world);
                                self.align_state = AlignState::Off;
                            }
                            AlignState::Off => unreachable!(),
                        }
                        self.refocus_cmd = true;
                    } else if self.stretch_state != StretchState::Off {
                        match self.stretch_state {
                            StretchState::WaitingForWin1 => {
                                self.stretch_state = StretchState::WaitingForWin2(click_world);
                                self.history.push(format!(
                                    "    stretch: window corner 1 = ({:.2},{:.2}) — click SECOND corner",
                                    click_world.x, click_world.y));
                            }
                            StretchState::WaitingForWin2(c1) => {
                                let wmin = Vec2 {
                                    x: c1.x.min(click_world.x),
                                    y: c1.y.min(click_world.y),
                                };
                                let wmax = Vec2 {
                                    x: c1.x.max(click_world.x),
                                    y: c1.y.max(click_world.y),
                                };
                                self.stretch_state = StretchState::WaitingForBase(wmin, wmax);
                                self.history.push(
                                    "    stretch: window captured — click BASE point".into());
                            }
                            StretchState::WaitingForBase(wmin, wmax) => {
                                self.stretch_state = StretchState::WaitingForDest(wmin, wmax, click_world);
                                self.history.push(
                                    "    stretch: BASE captured — click DESTINATION".into());
                            }
                            StretchState::WaitingForDest(wmin, wmax, base) => {
                                self.apply_stretch(wmin, wmax, base, click_world);
                                self.stretch_state = StretchState::Off;
                            }
                            StretchState::Off => unreachable!(),
                        }
                        self.refocus_cmd = true;
                    } else if self.matchprops_state != MatchPropsState::Off {
                        // matchprop is in source-pick mode — find the dobject
                        // under the cursor and use its style.
                        let tol_world = 10.0 / self.scale as f64;
                        if let Some(src) = self.nearest_entity_under(world, tol_world) {
                            self.apply_matchprops(src);
                            self.matchprops_state = MatchPropsState::Off;
                        } else {
                            self.history.push(
                                "  matchprop — no dobject under cursor (Esc to cancel)".into());
                        }
                        self.refocus_cmd = true;
                    } else if self.fillet_state != FilletState::Off {
                        // Slice M.3 — pick first object, then second.
                        let tol_world = 10.0 / self.scale as f64;
                        let hit = self.nearest_entity_under(world, tol_world);
                        match (self.fillet_state, hit) {
                            (FilletState::WaitingForFirst(r), Some(i)) => {
                                self.fillet_state = FilletState::WaitingForSecond(r, i, click_world);
                                self.history.push(format!(
                                    "  fillet — first = #{}. Click SECOND line on the side to KEEP.", i));
                            }
                            (FilletState::WaitingForSecond(r, i1, p1), Some(i2)) => {
                                self.apply_fillet(r, i1, p1, i2, click_world);
                                // Multiple-mode loop: re-enter the
                                // first-pick state with the same radius
                                // instead of returning to Off. Esc
                                // exits. Single-mode (default) → Off.
                                if self.fillet_multiple {
                                    self.fillet_state = FilletState::WaitingForFirst(r);
                                    self.refresh_fillet_prompt();
                                } else {
                                    self.fillet_state = FilletState::Off;
                                }
                            }
                            _ => self.history.push(
                                "  fillet — click ON a line; missed".into()),
                        }
                        self.refocus_cmd = true;
                    } else if self.chamfer_state != ChamferState::Off {
                        // Slice M.4 — pick first object, then second.
                        let tol_world = 10.0 / self.scale as f64;
                        let hit = self.nearest_entity_under(world, tol_world);
                        match (self.chamfer_state, hit) {
                            (ChamferState::WaitingForFirst(d1, d2), Some(i)) => {
                                self.chamfer_state =
                                    ChamferState::WaitingForSecond(d1, d2, i, click_world);
                                self.history.push(format!(
                                    "  chamfer — first = #{}. Click SECOND line.", i));
                            }
                            (ChamferState::WaitingForSecond(d1, d2, i1, p1), Some(i2)) => {
                                self.apply_chamfer(d1, d2, i1, p1, i2, click_world);
                                if self.chamfer_multiple {
                                    self.chamfer_state =
                                        ChamferState::WaitingForFirst(d1, d2);
                                    self.refresh_chamfer_prompt();
                                } else {
                                    self.chamfer_state = ChamferState::Off;
                                }
                            }
                            _ => self.history.push(
                                "  chamfer — click ON a line; missed".into()),
                        }
                        self.refocus_cmd = true;
                    } else if self.mirror_state != MirrorState::Off {
                        match self.mirror_state {
                            MirrorState::WaitingForA => {
                                self.mirror_state = MirrorState::WaitingForB(click_world);
                                self.history.push(format!(
                                    "    mirror: A = ({:.3}, {:.3}) — click SECOND axis point",
                                    click_world.x, click_world.y));
                            }
                            MirrorState::WaitingForB(a) => {
                                self.apply_mirror(a, click_world);
                                self.mirror_state = MirrorState::Off;
                            }
                            MirrorState::Off => unreachable!(),
                        }
                        self.refocus_cmd = true;
                    } else if self.select_mode != SelectMode::Off {
                        let shift = ctx.input(|i| i.modifiers.shift);
                        let tol_world = 10.0 / self.scale as f64;
                        // If the user explicitly typed `w` or `c`, their
                        // intent is unambiguous: this click starts a
                        // window-selection rectangle, not a single-dobject
                        // pick. Skip the nearest-entity heuristic — at high
                        // zoom-out the 10-px tol almost always hits SOMETHING
                        // and would steal the click. The armed flag survives
                        // here and gets consumed by add_window_selection on
                        // the second corner click.
                        let armed_window = self.armed_window_inside.is_some();
                        let hit = if armed_window {
                            None
                        } else {
                            self.nearest_entity_under(world, tol_world)
                        };
                        if let Some(i) = hit {
                            self.click_select(i, shift);
                            self.window_first = None;   // any half-started window is dropped
                        } else if let Some(first) = self.window_first.take() {
                            self.add_window_selection(first, world, shift);
                        } else {
                            self.window_first = Some(world);
                            let hint = match self.armed_window_inside {
                                Some(true)  => "    window (armed INSIDE): click OPPOSITE corner".to_string(),
                                Some(false) => "    window (armed CROSSING): click OPPOSITE corner".to_string(),
                                None        => "    window: click opposite corner (L→R inside, R→L crossing — hold Shift to subtract)".to_string(),
                            };
                            self.history.push(hint);
                        }
                        self.refocus_cmd = true;
                    } else if self.intersect_pending_click {
                        let world_r = 50.0 / self.scale as f64;
                        self.intersect_near(world, world_r);
                        self.intersect_pending_click = false;
                    } else if self.tool == Tool::None {
                        // Pointer-mode click — the always-on selector. Click
                        // on a dobject adds it to the basket; Shift removes.
                        // Click on EMPTY space deselects everything (AutoCAD
                        // convention — gives the user a clean slate without
                        // typing anything). Shift-click on empty preserves
                        // the basket so missing the dobject by a few pixels
                        // mid-shift-multi-select doesn't wipe it.
                        // See `feedback_rust_cad_pointer_is_selector` memo.
                        let shift = ctx.input(|i| i.modifiers.shift);
                        let tol_world = 10.0 / self.scale as f64;
                        if let Some(i) = self.nearest_entity_under(world, tol_world) {
                            self.click_select(i, shift);
                        } else if !shift {
                            self.selection.clear();
                            self.selected = None;
                        }
                        self.refocus_cmd = true;
                    } else if self.picking_source {
                        let tol_world = 10.0 / self.scale as f64;
                        match self.nearest_entity_under(world, tol_world) {
                            Some(i) => {
                                self.selected = Some(i);
                                self.picking_source = false;
                                self.history.push(format!("  + picked dobject #{}", i));
                            }
                            None => {
                                self.history.push("  ! no dobject near click — try clicking closer to the curve, or pick from the right panel".into());
                            }
                        }
                    } else if self.tool != Tool::None {
                        // Use the precomputed snap hit if one is available.
                        // One-shot typed overrides are consumed regardless of
                        // whether the hit succeeded — Esc still cancels.
                        let click_world = match snap_hit {
                            Some(h) => {
                                self.history.push(format!(
                                    "  ↳ {} → ({:.3},{:.3})",
                                    h.kind.name(), h.point.x, h.point.y
                                ));
                                h.point
                            }
                            None => {
                                if self.snap_override.is_some() {
                                    self.history.push(
                                        "  ! snap missed — used raw click".into());
                                }
                                world
                            }
                        };
                        if self.snap_override.is_some() {
                            self.snap_override = None;
                        }
                        // Polyline maintains a parallel bulge per segment:
                        // bulge[i] is the segment from pending[i] to
                        // pending[i+1]. In Arc sub-mode the just-clicked
                        // segment gets a tangent-continuous arc bulge;
                        // in Line sub-mode (or any other tool) it stays 0.
                        //
                        // Second-pt flow (typed `s` in Arc mode) is
                        // 2-stage: the first click captures an on-arc
                        // midpoint WITHOUT committing a vertex; the
                        // second click commits the endpoint using a
                        // 3-point-arc bulge.
                        let pline_handled = if self.tool == Tool::Polyline {
                            match self.pline_arc_sub {
                                PlineArcSub::AwaitingSecondPt => {
                                    self.pline_arc_sub =
                                        PlineArcSub::AwaitingSecondPtEnd(click_world);
                                    self.history.push(format!(
                                        "    pline·ARC second-pt: ({:.3},{:.3}) — click ENDPOINT",
                                        click_world.x, click_world.y));
                                    self.update_pline_prompt();
                                    true
                                }
                                PlineArcSub::AwaitingSecondPtEnd(mid) => {
                                    if let Some(&start) = self.pending.last() {
                                        let bulge = bulge_from_three_points(
                                            start, mid, click_world);
                                        self.pending_bulges.push(bulge);
                                        self.pending.push(click_world);
                                    }
                                    self.pline_arc_sub = PlineArcSub::Normal;
                                    self.update_pline_prompt();
                                    true
                                }
                                PlineArcSub::Normal => false,
                            }
                        } else { false };
                        if !pline_handled {
                            // PLINE auto-close: a click that lands on
                            // (or within pickbox-tolerance of) vertex[0]
                            // when at least 3 vertices are already in
                            // `pending` commits the polyline as CLOSED
                            // and exits drawing mode. Matches AutoCAD's
                            // behaviour where snapping back to the start
                            // ends the PLINE command.
                            let auto_closed = self.tool == Tool::Polyline
                                && self.pending.len() >= 3
                                && {
                                    let first = self.pending[0];
                                    let world_tol = (self.env.PkBxSz.max(4) as f64)
                                        / (self.scale as f64).max(1e-6);
                                    (click_world - first).len() < world_tol
                                };
                            if auto_closed {
                                let verts = self.drain_pline_pending(true);
                                self.add_dobject(Geom::Polyline(Polyline {
                                    vertices: verts, closed: true,
                                }), "canvas (auto-closed on first vertex)");
                                self.update_pline_prompt();
                            } else {
                                if self.tool == Tool::Polyline && !self.pending.is_empty() {
                                    let new_bulge = if self.pline_mode == PlineMode::Arc {
                                        self.pline_arc_bulge_to(click_world)
                                    } else {
                                        0.0
                                    };
                                    self.pending_bulges.push(new_bulge);
                                }
                                self.pending.push(click_world);
                                if self.tool == Tool::Polyline {
                                    self.update_pline_prompt();
                                }
                                self.try_finalise();
                            }
                        }
                        // canvas click steals focus away from the command box;
                        // restore it so typing keeps working without a manual
                        // click into the field.
                        self.refocus_cmd = true;
                    }
                }
            }

            // axes
            let origin = self.w2s(Vec2::ZERO, rect);
            let axis_col = egui::Color32::from_rgb(46, 56, 70);
            painter.line_segment(
                [egui::pos2(rect.left(), origin.y),
                 egui::pos2(rect.right(), origin.y)],
                egui::Stroke::new(1.0, axis_col),
            );
            painter.line_segment(
                [egui::pos2(origin.x, rect.top()),
                 egui::pos2(origin.x, rect.bottom())],
                egui::Stroke::new(1.0, axis_col),
            );
            painter.circle_stroke(
                origin, 5.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(240, 210, 70)),
            );

            // dobjects — viewport-culled. We compute the visible world rect once
            // and skip any dobject whose bbox doesn't overlap it. Still O(N) per
            // frame in the worst case (everything visible), but the painter cost
            // is the real bottleneck, and culling lets it scale far better when
            // you zoom in on a corner of a big drawing.
            let v_tl = self.s2w(rect.left_top(),     rect);
            let v_br = self.s2w(rect.right_bottom(), rect);
            let v_min = Vec2::new(v_tl.x.min(v_br.x), v_tl.y.min(v_br.y));
            let v_max = Vec2::new(v_tl.x.max(v_br.x), v_tl.y.max(v_br.y));
            self.last_visible = Some((v_min, v_max));

            // Execute deferred "∩ view" now that we know the viewport bbox.
            if self.intersect_view_pending {
                self.intersect_view_pending = false;
                self.intersect_in_bbox(v_min, v_max);
            }

            // Source the candidate indices: if a fresh index exists, query it
            // (O(visible cells)); otherwise fall back to O(N) iteration. The
            // index loop is dramatically faster at 1M+ dobjects.
            //
            // Rebuild the index FIRST if dirty — otherwise every move /
            // copy / array invalidates it and the renderer wastes a frame
            // iterating all N. One rebuild pays for itself in milliseconds
            // when N reaches the millions (≈100 ms rebuild vs ≈100 ms per
            // frame of full-N iteration, every frame, forever).
            //
            // Collected into a Vec so we can both COUNT the candidates
            // (= `in_viewport` in the screen-stats panel) and iterate
            // them twice (CPU vs GPU branches consume the same set).
            let _ = self.ensure_index();
            let candidates: Vec<usize> =
                if let (Some(g), false) = (self.index.as_ref(), self.index_dirty) {
                    g.query_bbox(v_min, v_max).into_iter().map(|u| u as usize).collect()
                } else {
                    (0..self.doc.dobjects.len()).collect()
                };
            let in_viewport = candidates.len();
            // Render mode (CPU/GPU) and APX (dots) are user-toggled via
            // the status-bar badges. No auto-switch — the user decides
            // when to trade fidelity for speed.
            let candidate_iter: Box<dyn Iterator<Item = usize>> =
                Box::new(candidates.into_iter());

            // DObject supplying the active snap, if any — highlighted in cyan
            // so the user can see "this is what I'm anchoring against" even
            // when the snap point lands far away on the dobject's extension.
            let snap_source: Option<usize> = snap_hit.and_then(|h| h.dobject);

            // ---- Cutter / boundary highlighting during trim / extend
            // target-pick phase. Cutters render in warm orange so the user
            // sees what's actually intersecting. See memo
            // `feedback_rust_cad_trim_default_all_cutters`.
            // `Some(None)` means ALL-mode (paint every visible dobject as
            // cutter/boundary); `Some(Some(&[..]))` means explicit list.
            let trim_cutters: Option<Option<&Vec<usize>>> = match &self.trim_state {
                TrimState::PickingTargets(c)    => Some(Some(c)),
                TrimState::PickingTargetsAll    => Some(None),
                _                               => None,
            };
            let extend_bounds: Option<Option<&Vec<usize>>> = match &self.extend_state {
                ExtendState::PickingTargets(b)  => Some(Some(b)),
                ExtendState::PickingTargetsAll  => Some(None),
                _                               => None,
            };
            let cutter_color   = egui::Color32::from_rgb(255, 170,  60); // warm orange
            let boundary_color = egui::Color32::from_rgb(255, 220,  90); // warm amber
            // Item 5 — pulse alpha for the cutter/boundary OVERLAY. Real
            // dobject color renders normally underneath so similar-coloured
            // neighbours stay distinguishable. The overlay pulses in/out
            // at ~1.4 Hz (≈ 700 ms full cycle), driven by ctx.input().time.
            // When the trim/extend session is live we request a repaint
            // every 80 ms so the animation stays smooth without burning
            // GPU at full vsync.
            // Keep the pulse animation refreshing whenever there's
            // anything pulsing: trim cutters, extend boundaries, OR a
            // non-empty selection basket (the dashed overlay shares
            // the same pulse). Without this the basket would freeze
            // at whatever phase it was in when the last user input
            // arrived.
            let cutter_or_bound_active =
                trim_cutters.is_some() || extend_bounds.is_some();
            let pulse_animation_active =
                cutter_or_bound_active || !self.selection.is_empty();
            if pulse_animation_active {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            }
            let pulse_t = ctx.input(|i| i.time);
            // sin: -1..1  →  pulse: 0.15..0.85
            let pulse = 0.5 + 0.35 * (pulse_t * std::f64::consts::TAU * 1.4).sin();
            let pulse_alpha = (pulse.clamp(0.15, 0.85) * 255.0) as u8;

            let mut drawn   = 0usize;
            let mut skipped = 0usize;
            let mut gpu_circles_count = 0usize;

            // === APX (draft display) render branch ===
            // When `lod_active`, every visible dobject becomes a single
            // dot at its gravity point. Dots are pushed into the GPU
            // instanced-circle pipeline as tiny world-radius circles —
            // one draw call for the entire scene, FPS recovers from
            // single-digit to 60+. The full-geometry render_mode match
            // below is skipped entirely while APX is active.
            //
            // Hit-testing, snap, and selection still use the underlying
            // bbox / geometry — only the visual is approximate.
            if self.lod_active {
                let mut dots: Vec<CircleInstance> = Vec::new();
                // Screen-target dot radius → world radius for this frame.
                // 1.5 px is small enough not to clutter at typical zooms
                // but visible as a clear dot.
                let dot_world_r = (1.5_f64 / (self.scale as f64).max(1e-9)) as f32;
                for i in candidate_iter {
                    let e = &self.doc.dobjects[i];
                    if !e.style.visible || !self.doc.layers.renders(e.style.layer) {
                        skipped += 1;
                        continue;
                    }
                    let (emin, emax) = e.bbox();
                    if emax.x < v_min.x || emin.x > v_max.x
                    || emax.y < v_min.y || emin.y > v_max.y {
                        continue;
                    }
                    // Anchor strategy. Only 0 (bbox center) implemented
                    // this slice; 1 (primitive center) and 2 (first
                    // vertex) fall back to bbox center.
                    let anchor = match self.env.LodAnc {
                        _ => Vec2::new((emin.x + emax.x) * 0.5,
                                       (emin.y + emax.y) * 0.5),
                    };
                    // Selection / snap highlight wins over per-dobject
                    // color so the user can still pick out selected items.
                    let in_selection = self.selection.contains(&i);
                    let color = if self.selected == Some(i) || in_selection {
                        egui::Color32::from_rgb(255, 200, 80)
                    } else if snap_source == Some(i) {
                        egui::Color32::from_rgb(120, 240, 255)
                    } else {
                        let (r, g, b) = resolve_color(
                            e.style.color, e.style.layer,
                            &self.doc.layers, &self.doc.truecolors);
                        egui::Color32::from_rgb(r, g, b)
                    };
                    let packed: u32 =
                          ((color.r() as u32) << 24)
                        | ((color.g() as u32) << 16)
                        | ((color.b() as u32) <<  8)
                        |  (color.a() as u32);
                    dots.push(CircleInstance {
                        x: anchor.x as f32,
                        y: anchor.y as f32,
                        r: dot_world_r,
                        color: packed,
                    });
                    drawn += 1;
                }
                gpu_circles_count = dots.len();
                if !dots.is_empty() {
                    let view = view_matrix(
                        rect.width(), rect.height(),
                        self.scale,
                        self.world_offset.x, self.world_offset.y,
                    );
                    let renderer = self.gpu_renderer.clone();
                    let cb = egui::PaintCallback {
                        rect,
                        callback: StdArc::new(
                            egui_glow::CallbackFn::new(
                                move |_info, gl_painter| {
                                    let gl = gl_painter.gl();
                                    let mut r = renderer.lock().unwrap();
                                    r.ensure_init(gl);
                                    r.upload_and_render(gl, &dots, &view);
                                },
                            ),
                        ),
                    };
                    painter.add(egui::Shape::Callback(cb));
                }
                self.gpu_dirty = false;
            } else { match self.render_mode {
                RenderMode::Cpu => {
                    for i in candidate_iter {
                        let e = &self.doc.dobjects[i];
                        // Layer-level visibility gate — hidden/frozen layers
                        // skip render entirely. Per-Dobject visibility is
                        // honoured the same way.
                        if !e.style.visible || !self.doc.layers.renders(e.style.layer) {
                            skipped += 1;
                            continue;
                        }
                        // ---- Hatch short-circuit (BEFORE viewport /
                        // micro-cull / state-branches). Hatch's bbox is a
                        // (0,0) placeholder (the kernel can't resolve
                        // boundary handles to a real bbox), so the
                        // viewport-bbox cull and the bbox_px micro-cull
                        // below would both wrongly drop it. The selection-
                        // dashed and trim-pulse branches also call
                        // render functions that stub Hatch as a no-op.
                        // Dispatch directly to render_hatch_fill here so
                        // none of that matters.
                        if let Geom::Hatch(h) = &e.geom {
                            let color = if self.selected == Some(i)
                                || self.selection.contains(&i)
                            {
                                egui::Color32::from_rgb(255, 200, 80)
                            } else if snap_source == Some(i) {
                                egui::Color32::from_rgb(120, 240, 255)
                            } else {
                                let (r, g, b) = resolve_color(
                                    e.style.color, e.style.layer,
                                    &self.doc.layers, &self.doc.truecolors);
                                egui::Color32::from_rgb(r, g, b)
                            };
                            self.render_hatch_fill(&painter, rect, h, color);
                            drawn += 1;
                            continue;
                        }
                        let (emin, emax) = e.bbox();
                        if emax.x < v_min.x || emin.x > v_max.x
                        || emax.y < v_min.y || emin.y > v_max.y {
                            continue;
                        }
                        let bbox_px = (emax.x - emin.x).max(emax.y - emin.y) as f32 * self.scale;
                        let in_selection = self.selection.contains(&i);
                        if bbox_px < 1.0
                            && self.selected != Some(i)
                            && snap_source != Some(i)
                            && !in_selection
                        {
                            skipped += 1;
                            continue;
                        }
                        // Trim/extend visualization: cutters render warm-orange,
                        // boundaries warm-amber. Solid thick lines (NOT dashed)
                        // so they're distinguishable from basket dashed-gray.
                        let is_cutter = match trim_cutters {
                            Some(None)    => true,                 // ALL-mode
                            Some(Some(c)) => c.contains(&i),
                            None          => false,
                        };
                        let is_boundary = match extend_bounds {
                            Some(None)    => true,
                            Some(Some(b)) => b.contains(&i),
                            None          => false,
                        };
                        if is_cutter || is_boundary {
                            // Real dobject color FIRST (so similar-coloured
                            // neighbours stay distinguishable; the user's
                            // "stop turning everything yellow" complaint).
                            let (r, g, b) = resolve_color(
                                e.style.color, e.style.layer, &self.doc.layers,
                                &self.doc.truecolors,
                            );
                            draw_dobject(&painter, rect, self, &e.geom,
                                egui::Color32::from_rgb(r, g, b));
                            // Pulsing overlay on top — warm orange for
                            // cutters, warm amber for boundaries, alpha
                            // breathing in/out so the user sees motion
                            // instead of a colour swap.
                            let base = if is_cutter { cutter_color } else { boundary_color };
                            let pulse_col = egui::Color32::from_rgba_unmultiplied(
                                base.r(), base.g(), base.b(), pulse_alpha,
                            );
                            draw_dobject_thick(&painter, rect, self, &e.geom,
                                pulse_col, 4.0);
                            drawn += 1;
                            continue;
                        }
                        // Basket members: render the real dobject (its
                        // resolved color) first, THEN overlay an
                        // animated dashed pulse on top. Mirrors the
                        // trim/extend method — same pulse_alpha
                        // breathing rate, just dashed instead of
                        // thick-solid.
                        //
                        // Color, width, dash/gap and pulse range are
                        // hardcoded for now; planned SYSVARs are
                        // listed in Variables.md (SelDshClr / SelDshW
                        // / SelDshL / SelDshG / SelPlsMin / SelPlsMax).
                        if in_selection {
                            let (r, g, b) = resolve_color(
                                e.style.color, e.style.layer, &self.doc.layers,
                                &self.doc.truecolors);
                            draw_dobject(&painter, rect, self, &e.geom,
                                egui::Color32::from_rgb(r, g, b));
                            // Pulsing dashed overlay — gray-cyan reads
                            // distinctly against both light and dark
                            // dobject colors.
                            let base = egui::Color32::from_rgb(180, 210, 230);
                            let pulse_col = egui::Color32::from_rgba_unmultiplied(
                                base.r(), base.g(), base.b(), pulse_alpha);
                            draw_dobject_dashed(&painter, rect, self, &e.geom,
                                pulse_col, 6.0, 4.0);
                            drawn += 1;
                            continue;
                        }
                        let color = if self.selected == Some(i) {
                            egui::Color32::from_rgb(255, 200, 80)
                        } else if snap_source == Some(i) {
                            egui::Color32::from_rgb(120, 240, 255)
                        } else {
                            // Resolve through ByLayer / ByBlock to a concrete RGB.
                            let (r, g, b) = resolve_color(
                                e.style.color, e.style.layer, &self.doc.layers,
                                &self.doc.truecolors,
                            );
                            egui::Color32::from_rgb(r, g, b)
                        };
                        // Hatch needs Document access (to resolve its
                        // boundary-handle references) — short-circuit to
                        // a dedicated renderer that has &self.
                        if let Geom::Hatch(h) = &e.geom {
                            self.render_hatch_fill(&painter, rect, h, color);
                            drawn += 1;
                            continue;
                        }
                        draw_dobject(&painter, rect, self, &e.geom, color);
                        drawn += 1;
                    }
                }
                RenderMode::Gpu => {
                    // GPU path: build instance buffer for circles only this slice.
                    // Non-circle dobjects still go through CPU (mixed rendering)
                    // so lines/arcs are visible. Future slices add their own
                    // instance kinds.
                    //
                    // Color resolution must MATCH the CPU branch: each
                    // dobject's own style.color (resolved through
                    // ByLayer / ByBlock / Aci / TrueColorRef) wins
                    // unless the dobject is selected (yellow) or the
                    // current snap source (cyan). The earlier GPU code
                    // hardcoded three flat colors and ignored every
                    // per-dobject color — fixed below.
                    let mut circles: Vec<CircleInstance> = Vec::new();
                    let snap_col = egui::Color32::from_rgb(120, 240, 255);
                    let sel_col  = egui::Color32::from_rgb(255, 200, 80);
                    for i in candidate_iter {
                        let e = &self.doc.dobjects[i];
                        let (emin, emax) = e.bbox();
                        if emax.x < v_min.x || emin.x > v_max.x
                        || emax.y < v_min.y || emin.y > v_max.y {
                            continue;
                        }
                        let in_selection = self.selection.contains(&i);
                        let color = if self.selected == Some(i) || in_selection {
                            sel_col
                        } else if snap_source == Some(i) {
                            snap_col
                        } else {
                            let (r, g, b) = resolve_color(
                                e.style.color, e.style.layer,
                                &self.doc.layers, &self.doc.truecolors);
                            egui::Color32::from_rgb(r, g, b)
                        };
                        match &e.geom {
                            Geom::Circle(c) => {
                                // Pack ARGB → 0xRRGGBBAA u32 for the
                                // instance buffer (vertex attrib uses
                                // unpack4x8unorm in shader = LE RGBA).
                                let packed: u32 =
                                      ((color.r() as u32) << 24)
                                    | ((color.g() as u32) << 16)
                                    | ((color.b() as u32) <<  8)
                                    |  (color.a() as u32);
                                circles.push(CircleInstance {
                                    x: c.center.x as f32,
                                    y: c.center.y as f32,
                                    r: c.radius as f32,
                                    color: packed,
                                });
                                drawn += 1;
                            }
                            _ => {
                                // Hatch needs Document access to
                                // resolve boundary handles — short-
                                // circuit before draw_dobject which is
                                // a no-op for Hatch.
                                if let Geom::Hatch(h) = &e.geom {
                                    self.render_hatch_fill(&painter, rect, h, color);
                                } else {
                                    draw_dobject(&painter, rect, self, &e.geom, color);
                                }
                                drawn += 1;
                            }
                        }
                    }
                    gpu_circles_count = circles.len();
                    if !circles.is_empty() {
                        // Build the world→clip matrix for our PaintCallback
                        // covering the canvas rect.
                        let view = view_matrix(
                            rect.width(), rect.height(),
                            self.scale,
                            self.world_offset.x, self.world_offset.y,
                        );
                        let renderer = self.gpu_renderer.clone();
                        let cb = egui::PaintCallback {
                            rect,
                            callback: StdArc::new(
                                egui_glow::CallbackFn::new(
                                    move |_info, gl_painter| {
                                        let gl = gl_painter.gl();
                                        let mut r = renderer.lock().unwrap();
                                        r.ensure_init(gl);
                                        r.upload_and_render(gl, &circles, &view);
                                    },
                                ),
                            ),
                        };
                        painter.add(egui::Shape::Callback(cb));
                    }
                    self.gpu_dirty = false;
                }
            } } // close `match self.render_mode` and outer `else` from APX branch
            // Grip handles on the selected dobject (drawn on top of the
            // geometry, under the snap marker / rubber band).
            if self.env.GrpEnb {
                if let Some(i) = self.selected {
                    if let Some(e) = self.doc.dobjects.get(i) {
                        draw_grips(&painter, rect, self, &e.geom);
                    }
                }
            }

            // HUD: FPS + drawn/skipped/total + index status + render mode.
            let idx_state = if self.index.is_some() && !self.index_dirty { "idx ✓" }
                            else { "idx stale" };
            let mode_str = if self.lod_active {
                format!("APX ({} dots instanced)", gpu_circles_count)
            } else {
                match self.render_mode {
                    RenderMode::Cpu => format!("CPU"),
                    RenderMode::Gpu => format!("GPU ({} circles instanced)", gpu_circles_count),
                }
            };
            painter.text(
                rect.right_top() + egui::vec2(-8.0, 8.0),
                egui::Align2::RIGHT_TOP,
                format!("FPS {:>5.1}    drawn {}  sub-px-skip {}  /{}    {}    {}",
                    self.fps_smooth, drawn, skipped, self.doc.dobjects.len(),
                    idx_state, mode_str),
                egui::FontId::monospace(11.0),
                egui::Color32::from_rgb(200, 220, 240),
            );
            // Snapshot for the Screen Stats panel — covers both CPU
            // and GPU render paths (they share `drawn`/`skipped`).
            self.last_render_stats = RenderStats {
                total:           self.doc.dobjects.len(),
                in_viewport,
                drawn,
                skipped_hidden:  skipped,    // combined hidden+subpx for now
                skipped_subpx:   0,          // (split is a future refinement)
                frame_dt:        dt,
                index_label:     if self.index_label.is_empty() {
                    idx_state.to_string()
                } else { self.index_label.clone() },
            };
            if !self.index_label.is_empty() {
                painter.text(
                    rect.right_top() + egui::vec2(-8.0, 24.0),
                    egui::Align2::RIGHT_TOP,
                    &self.index_label,
                    egui::FontId::monospace(10.0),
                    egui::Color32::from_rgb(140, 160, 180),
                );
            }

            // ∩ click preview — show the 50-pixel search circle on the cursor.
            if self.intersect_pending_click {
                if let Some(cur) = resp.hover_pos() {
                    painter.circle_stroke(
                        cur, 50.0,
                        egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 220, 100)),
                    );
                    painter.text(
                        cur + egui::vec2(0.0, 60.0),
                        egui::Align2::CENTER_CENTER,
                        "click to ∩ here (Esc cancels)",
                        egui::FontId::monospace(11.0),
                        egui::Color32::from_rgb(255, 220, 100),
                    );
                }
            }

            // intersection markers on top
            for p in &self.intersections {
                let sp = self.w2s(*p, rect);
                painter.circle_filled(sp, 4.5, egui::Color32::from_rgb(255, 90, 90));
                painter.circle_stroke(
                    sp, 4.5,
                    egui::Stroke::new(1.0, egui::Color32::WHITE),
                );
            }

            // pick-mode hover preview: highlight the dobject that would be selected
            if self.picking_source {
                if let Some(cur) = resp.hover_pos() {
                    let world = self.s2w(cur, rect);
                    let tol_world = 10.0 / self.scale as f64;
                    let mut best: Option<(usize, f64)> = None;
                    for (i, e) in self.doc.dobjects.iter().enumerate() {
                        let d = e.distance_to_point(world);
                        if d < tol_world && best.map_or(true, |(_, bd)| d < bd) {
                            best = Some((i, d));
                        }
                    }
                    if let Some((i, _)) = best {
                        draw_dobject(&painter, rect, self, &self.doc.dobjects[i].geom,
                                    egui::Color32::from_rgb(120, 240, 255));
                    }
                }
            }

            // OSNAP marker: glyph at the snap point + the dashed extension
            // line/arc from the on-dobject anchor when the foot lies on the
            // imaginary extension (PER/TAN past a segment endpoint or a
            // swept-arc boundary).
            if let Some(h) = snap_hit {
                let sp = self.w2s(h.point, rect);
                let glyph_col = egui::Color32::from_rgb(80, 230, 240);
                let from_anchor = self.pending.last().copied();

                // Faint connector from the cursor to the snap point — only
                // drawn when they're visibly apart. PER/TAN can land their
                // foot far from where the user is hovering (especially on
                // extensions); this thin line removes the "where IS my snap?"
                // confusion without making close hovers visually noisy.
                if let Some(cur) = resp.hover_pos() {
                    let gap_px = (cur - sp).length();
                    if gap_px > 20.0 {
                        painter.line_segment(
                            [cur, sp],
                            egui::Stroke::new(0.8, glyph_col.gamma_multiply(0.30)),
                        );
                    }
                }

                // The dashed indicator for the "imaginary extension":
                //   - on a line dobject, the extension is the infinite line, so
                //     a straight dashed segment between anchor and foot is right.
                //   - on an arc dobject, the extension is the rest of the
                //     underlying circle, so the dashes should curve along that
                //     circle from the arc endpoint (anchor) to the foot.
                //   - a circle dobject has no extension (PER's two feet are
                //     always on the circle), so this branch never fires for it.
                if let Some(anchor) = h.extension_anchor {
                    let time = ctx.input(|i| i.time) as f32;
                    let phase = time * 60.0;
                    let a = (0.55 + 0.35 * (time * 4.0).sin()).clamp(0.25, 0.95);
                    let ext_col = egui::Color32::from_rgba_unmultiplied(
                        255, 200, 90, (a * 255.0) as u8);
                    let ext_stroke = egui::Stroke::new(1.2, ext_col);
                    let geom_ref = h.dobject.and_then(|i| self.doc.dobjects.get(i)).map(|d| &d.geom);
                    match geom_ref {
                        Some(Geom::Arc(arc)) => {
                            // Walk the shorter way around the underlying circle
                            // from anchor angle to foot angle.
                            let ca = (anchor  - arc.center).angle();
                            let cf = (h.point - arc.center).angle();
                            let raw = (cf - ca).rem_euclid(std::f64::consts::TAU);
                            let sweep = if raw > std::f64::consts::PI {
                                raw - std::f64::consts::TAU
                            } else {
                                raw
                            };
                            draw_dashed_arc(
                                &painter, rect, self,
                                arc.center, arc.radius, ca, sweep,
                                7.0, 4.0, phase, ext_stroke,
                            );
                        }
                        _ => {
                            draw_dashed_line(
                                &painter,
                                self.w2s(anchor, rect), sp,
                                7.0, 4.0, phase, ext_stroke,
                            );
                        }
                    }
                }

                // Connector hint from the user's last pending click to the
                // snap point — only meaningful for snaps that "do something
                // with the anchor" (PER/TAN); for END/MID/CEN it's just a
                // soft guide.
                if let Some(from) = from_anchor {
                    painter.line_segment(
                        [self.w2s(from, rect), sp],
                        egui::Stroke::new(1.0, glyph_col.gamma_multiply(0.35)),
                    );
                }

                draw_snap_glyph(&painter, sp, h.kind, glyph_col);
                let label = if snap_candidates.len() > 1 {
                    format!("{}  ⇥ {}/{}",
                        h.kind.name(),
                        self.snap_cycle_index + 1,
                        snap_candidates.len())
                } else {
                    h.kind.name().to_string()
                };
                painter.text(
                    sp + egui::vec2(12.0, -12.0),
                    egui::Align2::LEFT_BOTTOM,
                    label,
                    egui::FontId::monospace(11.0),
                    glyph_col,
                );
                if snap_candidates.len() > 1 {
                    painter.text(
                        sp + egui::vec2(12.0, 12.0),
                        egui::Align2::LEFT_TOP,
                        "Tab: next snap",
                        egui::FontId::monospace(10.0),
                        glyph_col.gamma_multiply(0.7),
                    );
                }
            }

            // ---- Grip handles (Issue 3) ----------------------------------
            // Render small filled squares at each grip point of every
            // selected dobject when in pointer mode + GrpEnb is on. v1
            // semantic: dragging any grip translates the whole dobject.
            if self.env.GrpEnb && !in_click_only_phase && self.select_mode == SelectMode::Off {
                let mut grip_targets: Vec<usize> = self.selection.clone();
                if let Some(s) = self.selected { grip_targets.push(s); }
                grip_targets.sort_unstable(); grip_targets.dedup();
                // Render preview translation if dragging.
                let drag_delta: Option<Vec2> = self.grip_drag.and_then(|gd| {
                    let cur = resp.hover_pos()?;
                    let w   = self.s2w(cur, rect);
                    Some(w - gd.grip_origin)
                });
                let gsz = self.env.GrpSz as f32;
                let (cu_r, cu_g, cu_b) = (
                    (self.env.GrClrU >> 16 & 0xFF) as u8,
                    (self.env.GrClrU >>  8 & 0xFF) as u8,
                    (self.env.GrClrU       & 0xFF) as u8,
                );
                let (cs_r, cs_g, cs_b) = (
                    (self.env.GrClrS >> 16 & 0xFF) as u8,
                    (self.env.GrClrS >>  8 & 0xFF) as u8,
                    (self.env.GrClrS       & 0xFF) as u8,
                );
                let grip_col_u = egui::Color32::from_rgb(cu_r, cu_g, cu_b);
                let grip_col_s = egui::Color32::from_rgb(cs_r, cs_g, cs_b);
                for idx in &grip_targets {
                    let Some(d) = self.doc.dobjects.get(*idx) else { continue; };
                    // If this dobject is the active drag target, preview
                    // the role-specific edit at the cursor position.
                    let preview_geom = if let Some(gd) = self.grip_drag {
                        if gd.dobject_idx == *idx {
                            if let Some(cur) = resp.hover_pos() {
                                let w = self.s2w(cur, rect);
                                Some(d.geom.with_grip_moved(gd.role, w))
                            } else { None }
                        } else { None }
                    } else { None };
                    let _ = drag_delta;  // legacy; preview now uses with_grip_moved
                    let geom_ref = preview_geom.as_ref().unwrap_or(&d.geom);
                    // Ghost the dobject in dim white during the drag.
                    if preview_geom.is_some() {
                        draw_dobject(&painter, rect, self, geom_ref,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 160));
                    }
                    // Hover-highlight: any grip within GrpHvR px of the
                    // cursor lights up so the user knows clicking will
                    // grab it. Same threshold drives the grab-on-click
                    // tolerance below — no risk of "looks highlighted
                    // but doesn't grab". Skipped while a drag is in
                    // progress (the dragged grip already glows).
                    let cursor_screen = resp.hover_pos();
                    let hover_r2_px = (self.env.GrpHvR as f32).powi(2);
                    for (gp, _role) in geom_ref.grip_points() {
                        let sp = self.w2s(gp, rect);
                        let active_drag = self.grip_drag
                            .map(|gd| gd.dobject_idx == *idx
                                 && gd.grip_origin.dist(gp) < 1e-6)
                            .unwrap_or(false);
                        let hover = !active_drag
                            && self.grip_drag.is_none()
                            && cursor_screen.map(|c| {
                                let d = sp - c;
                                d.x*d.x + d.y*d.y <= hover_r2_px
                            }).unwrap_or(false);
                        let hot = active_drag || hover;
                        // Slightly larger square when hot so it reads
                        // as a "ready to grab" affordance, not just a
                        // color swap.
                        let half = if hot { gsz + 2.0 } else { gsz };
                        let r = egui::Rect::from_center_size(
                            sp, egui::vec2(half * 2.0, half * 2.0));
                        let col = if hot { grip_col_s } else { grip_col_u };
                        painter.rect_filled(r, 0.0, col);
                        // Pale outline ring around a hovered grip — extra
                        // visual cue the user can spot from the corner
                        // of their eye.
                        if hover {
                            painter.circle_stroke(sp, (gsz + 4.0).max(8.0),
                                egui::Stroke::new(1.2, egui::Color32::from_rgba_unmultiplied(
                                    255, 255, 255, 200)));
                        }
                        painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0,
                            egui::Color32::from_rgb(20, 20, 20)));
                    }
                }
            }

            // Live rubber-band preview while a window-drag is in progress
            // (select mode active OR Shift held in any other phase except
            // edit-active phases). L→R draws BLUE (window — fully-inside);
            // R→L draws GREEN (crossing — anything touching).
            //
            // Time-gated to match the classifier: no preview until the
            // user has held the button past env.SelDmTm. Without this
            // the visual would lie — preview appears, but the gesture
            // gets discarded as a click on release.
            if resp.dragged()
                && ((in_select && hold_threshold_passed)
                    || (shift_held && !in_click_only_phase))
            {
                if let (Some(p), Some(c)) = (
                    ctx.input(|i| i.pointer.press_origin()),
                    resp.hover_pos(),
                ) {
                    let r = egui::Rect::from_two_pos(p, c);
                    let crossing = c.x < p.x;
                    let (fill, stroke) = if crossing {
                        (egui::Color32::from_rgba_unmultiplied(140, 220, 100, 28),
                         egui::Color32::from_rgb(140, 220, 100))
                    } else {
                        (egui::Color32::from_rgba_unmultiplied(120, 170, 255, 28),
                         egui::Color32::from_rgb(120, 170, 255))
                    };
                    painter.rect_filled(r, 0.0, fill);
                    painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, stroke));
                }
            }

            // ---- Rotate live preview --------------------------------------
            // WaitingForAngle: ghost-render the selection rotated to the
            // current cursor angle (atan2(cursor − pivot)) so the user
            // sees the rotation form up before clicking. Also draws the
            // pivot mark + a baseline from pivot to cursor.
            // Reference sub-states: just show the points captured so far.
            match self.rotate_state {
                RotateState::WaitingForAngle(pivot) => {
                    let pivot_s = self.w2s(pivot, rect);
                    let mark    = egui::Color32::from_rgb(255, 200, 80);
                    painter.circle_stroke(pivot_s, 5.0, egui::Stroke::new(1.4, mark));
                    painter.line_segment(
                        [pivot_s + egui::vec2(-9.0, 0.0), pivot_s + egui::vec2(9.0, 0.0)],
                        egui::Stroke::new(0.8, mark));
                    painter.line_segment(
                        [pivot_s + egui::vec2(0.0, -9.0), pivot_s + egui::vec2(0.0, 9.0)],
                        egui::Stroke::new(0.8, mark));
                    if let Some(cur) = resp.hover_pos() {
                        let cur_world = self.s2w(cur, rect);
                        let angle = (cur_world - pivot).angle();
                        // Baseline pivot→cursor.
                        painter.line_segment(
                            [pivot_s, cur],
                            egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 200, 80, 180)));
                        // Ghost of the rotated selection.
                        let ghost = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 130);
                        for &i in &self.selection {
                            let Some(d) = self.doc.dobjects.get(i) else { continue; };
                            let g = d.geom.rotated(pivot, angle);
                            draw_dobject(&painter, rect, self, &g, ghost);
                        }
                        // Angle label near cursor.
                        let deg = angle.to_degrees();
                        painter.text(
                            cur + egui::vec2(12.0, -12.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!("{:.1}°{}", deg, if self.rotate_copy { "  (copy)" } else { "" }),
                            egui::FontId::monospace(12.0), mark);
                    }
                }
                RotateState::WaitingForRefSrc2(_, s1) => {
                    // First source point captured; show it + a cursor
                    // baseline indicating the in-progress source direction.
                    let mark = egui::Color32::from_rgb(180, 220, 120);
                    painter.circle_filled(self.w2s(s1, rect), 3.0, mark);
                    if let Some(cur) = resp.hover_pos() {
                        painter.line_segment(
                            [self.w2s(s1, rect), cur],
                            egui::Stroke::new(1.0,
                                egui::Color32::from_rgba_unmultiplied(180, 220, 120, 180)));
                    }
                }
                RotateState::WaitingForRefTgt(pivot, src_angle) => {
                    // Source direction captured; the NEW direction is
                    // anchored at the pivot. Pivot mark + live baseline
                    // pivot→cursor + ghost-rendered selection rotated to
                    // (cursor angle − src_angle).
                    let pivot_s = self.w2s(pivot, rect);
                    let mark    = egui::Color32::from_rgb(255, 200, 80);
                    painter.circle_stroke(pivot_s, 5.0, egui::Stroke::new(1.4, mark));
                    painter.line_segment(
                        [pivot_s + egui::vec2(-9.0, 0.0), pivot_s + egui::vec2(9.0, 0.0)],
                        egui::Stroke::new(0.8, mark));
                    painter.line_segment(
                        [pivot_s + egui::vec2(0.0, -9.0), pivot_s + egui::vec2(0.0, 9.0)],
                        egui::Stroke::new(0.8, mark));
                    if let Some(cur) = resp.hover_pos() {
                        let cur_world = self.s2w(cur, rect);
                        let tgt   = (cur_world - pivot).angle();
                        let dtheta = {
                            let mut d = (tgt - src_angle).rem_euclid(std::f64::consts::TAU);
                            if d > std::f64::consts::PI { d -= std::f64::consts::TAU; }
                            d
                        };
                        painter.line_segment(
                            [pivot_s, cur],
                            egui::Stroke::new(1.0,
                                egui::Color32::from_rgba_unmultiplied(255, 200, 80, 180)));
                        let ghost = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 130);
                        for &i in &self.selection {
                            let Some(d) = self.doc.dobjects.get(i) else { continue; };
                            let g = d.geom.rotated(pivot, dtheta);
                            draw_dobject(&painter, rect, self, &g, ghost);
                        }
                        painter.text(
                            cur + egui::vec2(12.0, -12.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!("{:.1}°{}", dtheta.to_degrees(),
                                if self.rotate_copy { "  (copy)" } else { "" }),
                            egui::FontId::monospace(12.0), mark);
                    }
                }
                _ => {}
            }

            // ---- Scale live preview ---------------------------------------
            // WaitingForFactor: ghost-render the selection scaled by the
            // current cursor distance from the pivot. Reference sub-states
            // visualise the captured ref endpoints.
            match self.scale_state {
                ScaleState::WaitingForFactor(pivot) => {
                    let pivot_s = self.w2s(pivot, rect);
                    let mark    = egui::Color32::from_rgb(255, 200, 80);
                    painter.circle_stroke(pivot_s, 5.0, egui::Stroke::new(1.4, mark));
                    painter.line_segment(
                        [pivot_s + egui::vec2(-9.0, 0.0), pivot_s + egui::vec2(9.0, 0.0)],
                        egui::Stroke::new(0.8, mark));
                    painter.line_segment(
                        [pivot_s + egui::vec2(0.0, -9.0), pivot_s + egui::vec2(0.0, 9.0)],
                        egui::Stroke::new(0.8, mark));
                    if let Some(cur) = resp.hover_pos() {
                        let cur_world = self.s2w(cur, rect);
                        let factor    = pivot.dist(cur_world);
                        // Baseline pivot→cursor.
                        painter.line_segment(
                            [pivot_s, cur],
                            egui::Stroke::new(1.0,
                                egui::Color32::from_rgba_unmultiplied(255, 200, 80, 180)));
                        // Ghost of the scaled selection.
                        let ghost = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 130);
                        for &i in &self.selection {
                            let Some(d) = self.doc.dobjects.get(i) else { continue; };
                            let g = d.geom.scaled(pivot, factor);
                            draw_dobject(&painter, rect, self, &g, ghost);
                        }
                        painter.text(
                            cur + egui::vec2(12.0, -12.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!("×{:.3}{}", factor,
                                if self.scale_copy { "  (copy)" } else { "" }),
                            egui::FontId::monospace(12.0), mark);
                    }
                }
                ScaleState::WaitingForRefEnd(_, s) => {
                    let mark = egui::Color32::from_rgb(180, 220, 120);
                    painter.circle_filled(self.w2s(s, rect), 3.0, mark);
                }
                ScaleState::WaitingForNewLength(pivot, _) => {
                    let pivot_s = self.w2s(pivot, rect);
                    let mark    = egui::Color32::from_rgb(255, 200, 80);
                    painter.circle_stroke(pivot_s, 5.0, egui::Stroke::new(1.4, mark));
                    if let Some(cur) = resp.hover_pos() {
                        painter.line_segment(
                            [pivot_s, cur],
                            egui::Stroke::new(1.0,
                                egui::Color32::from_rgba_unmultiplied(255, 200, 80, 180)));
                    }
                }
                _ => {}
            }

            // pending click points + rubber-band preview
            let preview_col = egui::Color32::from_rgb(255, 220, 100);
            for p in &self.pending {
                painter.circle_filled(self.w2s(*p, rect), 4.0, preview_col);
            }
            if self.tool != Tool::None {
                if let Some(raw_cursor) = resp.hover_pos() {
                    // When a snap is active the preview must agree with what
                    // the click will actually commit — the snap point, not
                    // the raw cursor position. Otherwise the rubber-band line
                    // ends one place and the marker glyph sits elsewhere.
                    let (cw, cursor) = match snap_hit {
                        Some(h) => (h.point, self.w2s(h.point, rect)),
                        None    => (self.s2w(raw_cursor, rect), raw_cursor),
                    };
                    let dash = egui::Stroke::new(1.0, preview_col);
                    let hint = egui::Stroke::new(0.5, preview_col.gamma_multiply(0.45));

                    // Tool::Line + Tool::Circle + Tool::Ellipse — independent
                    // of arc_method.
                    match (self.tool, self.pending.as_slice()) {
                        (Tool::Line, [a]) => {
                            painter.line_segment([self.w2s(*a, rect), cursor], dash);
                        }
                        (Tool::Circle, [c]) => {
                            let r_px = c.dist(cw) as f32 * self.scale;
                            painter.circle_stroke(self.w2s(*c, rect), r_px, dash);
                            painter.line_segment([self.w2s(*c, rect), cursor], hint);
                        }
                        // Polyline live preview — every captured segment
                        // (solid hint) plus a rubber-band from the last
                        // vertex to the cursor. Arc-mode segments
                        // tessellate from their bulge so the user sees
                        // the actual arc, not its chord. Each captured
                        // vertex gets a small filled dot.
                        (Tool::Polyline, verts) if !verts.is_empty() => {
                            let solid = egui::Stroke::new(
                                1.0, preview_col.gamma_multiply(0.7));
                            for w in verts.iter() {
                                painter.circle_filled(self.w2s(*w, rect), 3.0, preview_col);
                            }
                            for i in 0..verts.len().saturating_sub(1) {
                                let a = verts[i];
                                let b = verts[i + 1];
                                let bulge = self.pending_bulges
                                    .get(i).copied().unwrap_or(0.0);
                                self.draw_pline_preview_segment(
                                    &painter, rect, a, b, bulge, solid);
                            }
                            // Rubber-band from last captured to cursor.
                            // In Arc mode this is also a tessellated
                            // arc so the user sees the actual curvature
                            // the click will commit. The Second-pt
                            // sub-flow shows two distinct shapes for
                            // its two clicks.
                            if let Some(last) = verts.last() {
                                match (self.pline_mode, self.pline_arc_sub) {
                                    (PlineMode::Arc, PlineArcSub::AwaitingSecondPt) => {
                                        // Two reference lines (last→cursor and a
                                        // dotted hint suggesting the second point
                                        // will lie on the arc).
                                        painter.line_segment(
                                            [self.w2s(*last, rect), cursor], dash);
                                        painter.circle_filled(cursor, 4.0,
                                            preview_col.gamma_multiply(0.8));
                                    }
                                    (PlineMode::Arc, PlineArcSub::AwaitingSecondPtEnd(mid)) => {
                                        // 3-point arc through (last, mid, cursor).
                                        let mid_s = self.w2s(mid, rect);
                                        let bulge = bulge_from_three_points(*last, mid, cw);
                                        self.draw_pline_preview_segment(
                                            &painter, rect, *last, cw, bulge, dash);
                                        painter.circle_filled(mid_s, 4.0,
                                            preview_col.gamma_multiply(0.9));
                                    }
                                    (PlineMode::Arc, PlineArcSub::Normal) => {
                                        let bulge = self.pline_arc_bulge_to(cw);
                                        self.draw_pline_preview_segment(
                                            &painter, rect, *last, cw, bulge, dash);
                                    }
                                    (PlineMode::Line, _) => {
                                        painter.line_segment(
                                            [self.w2s(*last, rect), cursor], dash);
                                    }
                                }
                            }
                        }
                        // Spline live preview — control-polygon hint
                        // (thin chord lines between successive captured
                        // control points) PLUS the actual NURBS curve
                        // sampled through pending + cursor as the live
                        // next control point. Cubic by default (degree
                        // 3); lower degrees fall in until the user has
                        // ≥ 4 control points.
                        (Tool::Spline, verts) if !verts.is_empty() => {
                            // Captured control points as small dots.
                            for w in verts.iter() {
                                painter.circle_filled(self.w2s(*w, rect), 3.0, preview_col);
                            }
                            // Faint control polygon.
                            let hint_pen = egui::Stroke::new(
                                0.7, preview_col.gamma_multiply(0.4));
                            for pair in verts.windows(2) {
                                painter.line_segment(
                                    [self.w2s(pair[0], rect), self.w2s(pair[1], rect)],
                                    hint_pen);
                            }
                            // Last captured → cursor (control-polygon
                            // continuation, hinting where the next
                            // ctrl lands).
                            if let Some(last) = verts.last() {
                                painter.line_segment(
                                    [self.w2s(*last, rect), cursor], hint_pen);
                            }
                            // The CURVE — build a transient Spline
                            // including the cursor as the live next
                            // control point, tessellate, draw dashed.
                            let mut ctrls: Vec<Vec2> = verts.to_vec();
                            ctrls.push(cw);
                            if ctrls.len() >= 2 {
                                let degree = 3.min(ctrls.len() - 1);
                                let s = cad_kernel::Spline::new_bspline(degree, ctrls);
                                let n = (s.bbox().1 - s.bbox().0).len() as f32 * self.scale;
                                let n = (n * 0.5).clamp(32.0, 256.0) as usize;
                                let samples = s.tessellate(n);
                                if samples.len() >= 2 {
                                    let pts: Vec<egui::Pos2> = samples.iter()
                                        .map(|w| self.w2s(*w, rect)).collect();
                                    painter.add(egui::Shape::line(pts, dash));
                                }
                            }
                        }
                        // Ellipse 3-click flow.
                        // Stage 1 (pending=[centre]): rubber-band line from
                        // centre to cursor — defines the major axis.
                        (Tool::Ellipse, [c]) => {
                            painter.line_segment([self.w2s(*c, rect), cursor], dash);
                        }
                        // Stage 2 (pending=[centre, major_end]): show the
                        // major-axis line and a live ellipse using the
                        // current cursor for the minor.
                        (Tool::Ellipse, [c, me]) => {
                            draw_ellipse_preview(&painter, rect, self,
                                *c, *me, cw, dash, hint);
                        }
                        // ----- elliptical-arc 5-stage preview -----
                        (Tool::EllipseArc, [c]) => {
                            painter.line_segment([self.w2s(*c, rect), cursor], dash);
                        }
                        (Tool::EllipseArc, [c, me]) => {
                            draw_ellipse_preview(&painter, rect, self,
                                *c, *me, cw, dash, hint);
                        }
                        (Tool::EllipseArc, [c, me, mp]) => {
                            // Ellipse is now defined. Live preview shows the
                            // fixed full ellipse + a marker where the cursor
                            // projects onto it (= future start point).
                            let major = *me - *c;
                            if major.len() > EPS {
                                let v_hat = major.normalized().perp();
                                let semi_minor = (*mp - *c).dot(v_hat).abs();
                                if let Some(el) = ellipse_center_major_minor(*c, *me, semi_minor) {
                                    draw_polyline_full_ellipse(&painter, rect, self, &el, hint);
                                    let t = el.nearest_param(cw);
                                    let on = self.w2s(el.point_at(t), rect);
                                    painter.line_segment([self.w2s(*c, rect), on], dash);
                                    painter.circle_filled(on, 3.5, preview_col);
                                }
                            }
                        }
                        (Tool::EllipseArc, [c, me, mp, sp]) => {
                            // Ellipse fixed + start fixed; live preview shows
                            // a partial elliptical arc from start to cursor's
                            // projection (CCW).
                            let major = *me - *c;
                            if major.len() > EPS {
                                let v_hat = major.normalized().perp();
                                let semi_minor = (*mp - *c).dot(v_hat).abs();
                                if let Some(el) = ellipse_center_major_minor(*c, *me, semi_minor) {
                                    draw_polyline_full_ellipse(&painter, rect, self, &el, hint);
                                    let t_start = el.nearest_param(*sp);
                                    let t_end   = el.nearest_param(cw);
                                    let sweep_raw = (t_end - t_start)
                                        .rem_euclid(std::f64::consts::TAU);
                                    let sweep = if sweep_raw < 1e-6 {
                                        std::f64::consts::TAU
                                    } else {
                                        sweep_raw
                                    };
                                    let ea = EllipseArc {
                                        ellipse: el,
                                        start_param: t_start.rem_euclid(std::f64::consts::TAU),
                                        sweep_param: sweep,
                                    };
                                    let n = 64;
                                    let mut pts = Vec::with_capacity(n + 1);
                                    for i in 0..=n {
                                        let t = ea.start_param +
                                            (i as f64 / n as f64) * ea.sweep_param;
                                        pts.push(self.w2s(el.point_at(t), rect));
                                    }
                                    painter.add(egui::Shape::line(pts, dash));
                                }
                            }
                        }
                        _ => {}
                    }

                    // Arc preview depends on the current method, since the
                    // semantics of pending[0] / pending[1] differ per method.
                    let arc_polyline = |arc: Arc| -> Vec<egui::Pos2> {
                        let n = 64;
                        let mut pts = Vec::with_capacity(n + 1);
                        for i in 0..=n {
                            let t = arc.start_angle
                                  + (i as f64 / n as f64) * arc.sweep_angle;
                            let p = Vec2::new(
                                arc.center.x + arc.radius * t.cos(),
                                arc.center.y + arc.radius * t.sin(),
                            );
                            pts.push(self.w2s(p, rect));
                        }
                        pts
                    };
                    let ccw_arc_from_center_endpoints = |c: Vec2, s: Vec2, e: Vec2| -> Arc {
                        let radius = c.dist(s);
                        let sa = (s - c).angle();
                        let ea = (e - c).angle();
                        let sweep_raw = (ea - sa).rem_euclid(std::f64::consts::TAU);
                        let sweep = if sweep_raw < 1e-6 {
                            std::f64::consts::TAU
                        } else {
                            sweep_raw
                        };
                        Arc { center: c, radius, start_angle: sa, sweep_angle: sweep }
                    };

                    if self.tool == Tool::Arc {
                        match (self.arc_method, self.pending.as_slice()) {
                            // ---- 3-POINT: pending = points ON the arc, no centre at all ----
                            (ArcMethod::ThreePoints, [p1]) => {
                                // just a chord-hint line, no circle
                                painter.line_segment([self.w2s(*p1, rect), cursor], hint);
                            }
                            (ArcMethod::ThreePoints, [p1, p2]) => {
                                if let Some(arc) = arc_three_points(*p1, *p2, cw) {
                                    painter.add(egui::Shape::line(arc_polyline(arc), dash));
                                } else {
                                    // collinear → no preview, just guide chords
                                    painter.line_segment([self.w2s(*p1, rect), cursor], hint);
                                    painter.line_segment([self.w2s(*p2, rect), cursor], hint);
                                }
                            }

                            // ---- S,C,E: pending = [start, (center)] ----
                            (ArcMethod::StartCenterEnd, [s]) => {
                                // next click is the centre — show a hint line
                                painter.line_segment([self.w2s(*s, rect), cursor], hint);
                            }
                            (ArcMethod::StartCenterEnd, [s, c]) => {
                                // full radius circle hint + the CCW arc s→cursor around c
                                let r_px = c.dist(*s) as f32 * self.scale;
                                painter.circle_stroke(self.w2s(*c, rect), r_px, hint);
                                let arc = ccw_arc_from_center_endpoints(*c, *s, cw);
                                painter.add(egui::Shape::line(arc_polyline(arc), dash));
                            }

                            // ---- C,S,E: pending = [center, (start)] ----
                            (ArcMethod::CenterStartEnd, [c]) => {
                                // next click is the start — show radius line
                                painter.line_segment([self.w2s(*c, rect), cursor], hint);
                            }
                            (ArcMethod::CenterStartEnd, [c, s]) => {
                                let r_px = c.dist(*s) as f32 * self.scale;
                                painter.circle_stroke(self.w2s(*c, rect), r_px, hint);
                                let arc = ccw_arc_from_center_endpoints(*c, *s, cw);
                                painter.add(egui::Shape::line(arc_polyline(arc), dash));
                            }

                            // Frozen methods don't draw anything live.
                            _ => {}
                        }
                    }
                }
            }

            // HUD: cursor world coords + tool hint
            if let Some(pos) = resp.hover_pos() {
                let w = self.s2w(pos, rect);
                painter.text(
                    rect.left_top() + egui::vec2(10.0, 8.0),
                    egui::Align2::LEFT_TOP,
                    format!(
                        "cursor: ({:>9.3}, {:>9.3})   scale: {:>6.2} px/u",
                        w.x, w.y, self.scale
                    ),
                    egui::FontId::monospace(11.0),
                    egui::Color32::from_rgb(200, 220, 240),
                );
            }
            painter.text(
                rect.left_top() + egui::vec2(10.0, 28.0),
                egui::Align2::LEFT_TOP,
                current_hint(self.tool, self.arc_method, self.pending.len()),
                egui::FontId::monospace(11.0),
                egui::Color32::from_rgb(255, 220, 120),
            );

            // ---- move tool overlay (live preview + base→cursor arrow) --
            if self.move_state != MoveState::Off {
                let hint_text = match self.move_state {
                    MoveState::WaitingForBase =>
                        format!("MOVE: click BASE point for {} dobject(s)    [Esc cancels]",
                            self.selection.len()),
                    MoveState::WaitingForDest(_) =>
                        format!("MOVE: click DESTINATION ({} dobject(s) following)",
                            self.selection.len()),
                    MoveState::Off => unreachable!(),
                };
                painter.text(
                    rect.left_top() + egui::vec2(10.0, 48.0),
                    egui::Align2::LEFT_TOP,
                    hint_text,
                    egui::FontId::monospace(11.0),
                    egui::Color32::from_rgb(255, 200, 100),
                );

                if let MoveState::WaitingForDest(base) = self.move_state {
                    let cur_world = self.cursor_world_constrained(
                        resp.hover_pos(), rect, snap_hit.map(|h| h.point));
                    if let Some(cw) = cur_world {
                        let v = cw - base;
                        let base_s = self.w2s(base, rect);
                        let dest_s = self.w2s(cw, rect);
                        let accent = egui::Color32::from_rgb(255, 200, 100);
                        // base BLIP + animated dashed vector to cursor
                        draw_base_blip(&painter, base_s, accent);
                        let time = ctx.input(|i| i.time) as f32;
                        let phase = time * 60.0;   // marching-ants speed (px/s)
                        draw_dashed_line(&painter, base_s, dest_s,
                            6.0, 4.0, phase,
                            egui::Stroke::new(1.2, accent));
                        // ghost-render the selected dobjects at +v
                        let ghost_col = egui::Color32::from_rgba_unmultiplied(255, 200, 100, 180);
                        for &i in &self.selection {
                            if let Some(d) = self.doc.dobjects.get(i) {
                                let moved = d.geom.translated(v);
                                draw_dobject(&painter, rect, self, &moved, ghost_col);
                            }
                        }
                    }
                }
            }

            // ---- copy tool overlay (mirrors move; greener accent so the
            //      user can tell the two apart at a glance) --------------
            if self.copy_state != CopyState::Off {
                let hint_text = match self.copy_state {
                    CopyState::WaitingForBase =>
                        format!("COPY: click BASE point for {} dobject(s)    [Esc cancels]",
                            self.selection.len()),
                    CopyState::WaitingForDest(_) =>
                        format!("COPY: click DESTINATION ({} dobject(s) being duplicated)",
                            self.selection.len()),
                    CopyState::Off => unreachable!(),
                };
                painter.text(
                    rect.left_top() + egui::vec2(10.0, 48.0),
                    egui::Align2::LEFT_TOP,
                    hint_text,
                    egui::FontId::monospace(11.0),
                    egui::Color32::from_rgb(150, 230, 170),
                );

                if let CopyState::WaitingForDest(base) = self.copy_state {
                    let cur_world = self.cursor_world_constrained(
                        resp.hover_pos(), rect, snap_hit.map(|h| h.point));
                    if let Some(cw) = cur_world {
                        let v = cw - base;
                        let base_s = self.w2s(base, rect);
                        let dest_s = self.w2s(cw, rect);
                        let accent = egui::Color32::from_rgb(150, 230, 170);
                        draw_base_blip(&painter, base_s, accent);
                        let time = ctx.input(|i| i.time) as f32;
                        let phase = time * 60.0;
                        draw_dashed_line(&painter, base_s, dest_s,
                            6.0, 4.0, phase,
                            egui::Stroke::new(1.2, accent));
                        let ghost_col = egui::Color32::from_rgba_unmultiplied(150, 230, 170, 180);
                        for &i in &self.selection {
                            if let Some(d) = self.doc.dobjects.get(i) {
                                let copied = d.geom.translated(v);
                                draw_dobject(&painter, rect, self, &copied, ghost_col);
                            }
                        }
                    }
                }
            }

            // ---- selection mode overlay --------------------------------
            //
            // Rubber-band rectangle from the first-corner click to the
            // current cursor. Left-to-right drag = "inside" window (solid
            // blue); right-to-left = "crossing" window (dashed green).
            if self.select_mode != SelectMode::Off {
                let label = match self.select_mode {
                    SelectMode::ForList         => "LIST: select dobjects, Enter when done (Esc cancels)",
                    SelectMode::ForSelect       => "SELECT: pick dobjects, Enter when done (Esc cancels)",
                    SelectMode::ForCuttingEdges => "TRIM: pick CUTTING edges, Enter when done (Esc cancels)",
                    SelectMode::ForBoundaryEdges=> "EXTEND: pick BOUNDARY edges, Enter when done (Esc cancels)",
                    SelectMode::Off             => unreachable!(),
                };
                painter.text(
                    rect.left_top() + egui::vec2(10.0, 48.0),
                    egui::Align2::LEFT_TOP,
                    format!("{}    [{} selected]", label, self.selection.len()),
                    egui::FontId::monospace(11.0),
                    egui::Color32::from_rgb(255, 220, 120),
                );

                if let (Some(first), Some(cur)) = (self.window_first, resp.hover_pos()) {
                    let p1 = self.w2s(first, rect);
                    let p2 = cur;
                    let crossing = p2.x < p1.x;
                    let col = if crossing {
                        egui::Color32::from_rgba_unmultiplied(120, 230, 120, 60)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(120, 170, 255, 60)
                    };
                    let edge = if crossing {
                        egui::Color32::from_rgb(120, 230, 120)
                    } else {
                        egui::Color32::from_rgb(120, 170, 255)
                    };
                    let r = egui::Rect::from_two_pos(p1, p2);
                    painter.rect_filled(r, 0.0, col);
                    if crossing {
                        // dashed edges for the crossing window
                        let time = ctx.input(|i| i.time) as f32;
                        let phase = time * 40.0;
                        draw_dashed_line(&painter,
                            r.left_top(), r.right_top(),  6.0, 4.0, phase,
                            egui::Stroke::new(1.2, edge));
                        draw_dashed_line(&painter,
                            r.right_top(), r.right_bottom(), 6.0, 4.0, phase,
                            egui::Stroke::new(1.2, edge));
                        draw_dashed_line(&painter,
                            r.right_bottom(), r.left_bottom(), 6.0, 4.0, phase,
                            egui::Stroke::new(1.2, edge));
                        draw_dashed_line(&painter,
                            r.left_bottom(), r.left_top(), 6.0, 4.0, phase,
                            egui::Stroke::new(1.2, edge));
                    } else {
                        painter.rect_stroke(r, 0.0, egui::Stroke::new(1.2, edge));
                    }
                }
            }

            // ---- Drafting cursor overlay --------------------------------
            //
            // While drafting mode is active (drawing tool OR any
            // point-pick edit phase), draw a square (the "pickbox") with
            // a cross through it at the hover position. Same visual cue
            // AutoCAD uses to tell the user "I'm in command, click to
            // place a point". This is also when press-fires-click is in
            // effect — see the click pipeline override above.
            //
            // The cross sits at the CONSTRAINED position (after osnap /
            // ortho / grid-snap) so the user sees exactly where the
            // click will land, not where their raw cursor is. Drawn
            // last so it sits on top of dobjects and previews.
            if in_click_only_phase {
                if let Some(raw_p) = resp.hover_pos() {
                    let constrained_world = snap_hit.map(|h| h.point)
                        .unwrap_or_else(|| self.apply_constraints(self.s2w(raw_p, rect)));
                    let p = self.w2s(constrained_world, rect);
                    let half  = 7.0_f32;     // pickbox half-edge in px
                    let arm   = 14.0_f32;    // crosshair arm half-length
                    let col   = egui::Color32::from_rgb(235, 235, 245);
                    let stroke= egui::Stroke::new(1.0, col);
                    let sq = egui::Rect::from_center_size(p, egui::vec2(half*2.0, half*2.0));
                    painter.rect_stroke(sq, 0.0, stroke);
                    painter.line_segment(
                        [egui::pos2(p.x - arm, p.y), egui::pos2(p.x + arm, p.y)], stroke);
                    painter.line_segment(
                        [egui::pos2(p.x, p.y - arm), egui::pos2(p.x, p.y + arm)], stroke);
                }
            }

            ctx.request_repaint();
        });

        // Item 1 — clear the live status line when no edit phase remains
        // active (e.g. apply_fillet ended its state). Keeps the cmd area
        // free of stale prompts.
        let any_edit_active =
            self.tool != Tool::None
            || self.select_mode != SelectMode::Off
            || matches!(self.trim_state,
                TrimState::SelectingCutters
                | TrimState::PickingTargets(_)
                | TrimState::PickingTargetsAll)
            || matches!(self.extend_state,
                ExtendState::SelectingBoundaries
                | ExtendState::PickingTargets(_)
                | ExtendState::PickingTargetsAll)
            || self.move_state       != MoveState::Off
            || self.copy_state       != CopyState::Off
            || self.rotate_state     != RotateState::Off
            || self.scale_state      != ScaleState::Off
            || self.mirror_state     != MirrorState::Off
            || self.align_state      != AlignState::Off
            || self.break_state      != BreakState::Off
            || self.lengthen_state   != LengthenState::Off
            || self.offset_state     != OffsetState::Off
            || self.stretch_state    != StretchState::Off
            || self.matchprops_state != MatchPropsState::Off
            || self.fillet_state     != FilletState::Off
            || self.chamfer_state    != ChamferState::Off
            || self.picking_source
            || self.intersect_pending_click;
        if !any_edit_active && !self.current_prompt.is_empty() {
            self.clear_prompt();
        }
    }
}

// ---- snap markers & dashed extension line ---------------------------------

/// Per-snap-kind glyph at the snap point. Each kind gets a distinct shape so
/// the user knows what was matched without reading the label:
///   END  square outline
///   MID  triangle (point up)
///   CEN  small circle + centre dot
///   INT  X
///   PER  ⊥ symbol
///   TAN  circle with a tangent stub
///   NEA  hourglass (two opposing triangles)
fn draw_snap_glyph(p: &egui::Painter, c: egui::Pos2, k: SnapKind, col: egui::Color32) {
    let s = 6.0;     // half-extent
    let stroke = egui::Stroke::new(1.6, col);
    match k {
        SnapKind::End => {
            let r = egui::Rect::from_min_max(
                egui::pos2(c.x - s, c.y - s),
                egui::pos2(c.x + s, c.y + s),
            );
            p.rect_stroke(r, 0.0, stroke);
        }
        SnapKind::Mid => {
            let pts = vec![
                egui::pos2(c.x,       c.y - s),
                egui::pos2(c.x + s,   c.y + s),
                egui::pos2(c.x - s,   c.y + s),
                egui::pos2(c.x,       c.y - s),
            ];
            p.add(egui::Shape::line(pts, stroke));
        }
        SnapKind::Cen => {
            p.circle_stroke(c, s, stroke);
            p.circle_filled(c, 1.5, col);
        }
        SnapKind::Qua => {
            // Diamond — AutoCAD's quadrant marker
            let pts = vec![
                egui::pos2(c.x,       c.y - s),
                egui::pos2(c.x + s,   c.y),
                egui::pos2(c.x,       c.y + s),
                egui::pos2(c.x - s,   c.y),
                egui::pos2(c.x,       c.y - s),
            ];
            p.add(egui::Shape::line(pts, stroke));
        }
        SnapKind::Int => {
            p.line_segment([egui::pos2(c.x - s, c.y - s),
                            egui::pos2(c.x + s, c.y + s)], stroke);
            p.line_segment([egui::pos2(c.x - s, c.y + s),
                            egui::pos2(c.x + s, c.y - s)], stroke);
        }
        SnapKind::Per => {
            // upright ⊥: vertical stroke + horizontal baseline
            p.line_segment([egui::pos2(c.x, c.y - s),
                            egui::pos2(c.x, c.y + s)], stroke);
            p.line_segment([egui::pos2(c.x - s, c.y + s),
                            egui::pos2(c.x + s, c.y + s)], stroke);
        }
        SnapKind::Tan => {
            p.circle_stroke(c, s * 0.75, stroke);
            // horizontal tangent stub through the top of the small circle
            let y = c.y - s * 0.75;
            p.line_segment([egui::pos2(c.x - s, y), egui::pos2(c.x + s, y)], stroke);
        }
        SnapKind::Nea => {
            // hourglass / bowtie
            let pts = vec![
                egui::pos2(c.x - s, c.y - s),
                egui::pos2(c.x + s, c.y - s),
                egui::pos2(c.x - s, c.y + s),
                egui::pos2(c.x + s, c.y + s),
                egui::pos2(c.x - s, c.y - s),
            ];
            p.add(egui::Shape::line(pts, stroke));
        }
    }
}

/// Draw a dashed line a → b with the dash phase shifted by `phase` pixels so
/// the dashes appear to drift along the line. Used for the "imaginary
/// extension" trail of PER/TAN snaps on line dobjects.
/// Visible "blip" marker drawn at a captured base point (move base,
/// copy base, rotate pivot, scale pivot, …). Small filled square with
/// a + cross through it — same vocabulary as the drafting cursor so
/// the user reads it as "this is the locked-in point".
///
/// `color` is used for both fill and stroke so each command can have
/// its own accent (move = orange, copy = green, etc.). The marker is
/// drawn in screen-space px and doesn't scale with zoom.
fn draw_base_blip(p: &egui::Painter, pos: egui::Pos2, color: egui::Color32) {
    let half = 4.5_f32;        // filled square half-edge
    let arm  = 10.0_f32;       // cross arm half-length
    let sq = egui::Rect::from_center_size(pos, egui::vec2(half*2.0, half*2.0));
    p.rect_filled(sq, 1.0, color);
    p.rect_stroke(sq, 1.0, egui::Stroke::new(1.4, color));
    let stroke = egui::Stroke::new(1.4, color);
    p.line_segment([egui::pos2(pos.x - arm, pos.y), egui::pos2(pos.x + arm, pos.y)], stroke);
    p.line_segment([egui::pos2(pos.x, pos.y - arm), egui::pos2(pos.x, pos.y + arm)], stroke);
}

fn draw_dashed_line(
    p: &egui::Painter,
    a: egui::Pos2, b: egui::Pos2,
    dash_len: f32, gap_len: f32, phase: f32,
    stroke: egui::Stroke,
) {
    let d = b - a;
    let total = d.length();
    if total < 1e-3 { return; }
    let dir = d / total;
    let period = dash_len + gap_len;
    let mut t = -(phase.rem_euclid(period));
    while t < total {
        let s = t.max(0.0);
        let e = (t + dash_len).min(total);
        if e > s + 0.1 {
            p.line_segment([a + dir * s, a + dir * e], stroke);
        }
        t += period;
    }
}

/// Dashed arc indicator along a circle of (center_w, radius_w) starting at
/// `start_ang` (world radians) and sweeping `sweep` (signed, world radians).
/// Used for the PER/TAN "imaginary extension" on arc dobjects — the
/// extension follows the underlying circle's curvature, not a chord.
fn draw_dashed_arc(
    p: &egui::Painter,
    rect: egui::Rect, app: &CadApp,
    center_w: Vec2, radius_w: f64,
    start_ang: f64, sweep: f64,
    dash_len_px: f32, gap_len_px: f32, phase_px: f32,
    stroke: egui::Stroke,
) {
    let arc_len_px = (radius_w as f32 * app.scale) * sweep.abs() as f32;
    if arc_len_px < 1.0 { return; }
    let period = dash_len_px + gap_len_px;
    let mut t = -(phase_px.rem_euclid(period));
    while t < arc_len_px {
        let s = t.max(0.0);
        let e = (t + dash_len_px).min(arc_len_px);
        if e > s + 0.1 {
            let s_frac = (s / arc_len_px) as f64;
            let e_frac = (e / arc_len_px) as f64;
            let s_ang = start_ang + sweep * s_frac;
            let e_ang = start_ang + sweep * e_frac;
            // Subdivide each dash enough that the curvature reads as smooth.
            let subdiv = (((e - s) / 2.0).ceil() as usize).max(1);
            let mut pts = Vec::with_capacity(subdiv + 1);
            for i in 0..=subdiv {
                let f = i as f64 / subdiv as f64;
                let a = s_ang + (e_ang - s_ang) * f;
                let pw = Vec2::new(
                    center_w.x + radius_w * a.cos(),
                    center_w.y + radius_w * a.sin(),
                );
                pts.push(app.w2s(pw, rect));
            }
            p.add(egui::Shape::line(pts, stroke));
        }
        t += period;
    }
}

/// Filled grip handles on the selected dobject. The set of grip locations
/// follows the AutoCAD convention:
///   Line   — both endpoints + midpoint
///   Circle — centre + N/S/E/W quadrant points
///   Arc    — centre + both endpoints + midpoint
fn draw_grips(painter: &egui::Painter, rect: egui::Rect, app: &CadApp, g: &Geom) {
    let col   = egui::Color32::from_rgb(80, 170, 255);
    let outline = egui::Stroke::new(1.0, egui::Color32::WHITE);
    let s = 4.0;     // half-extent of grip square (screen px)
    let draw = |w: Vec2| {
        let sp = app.w2s(w, rect);
        let r = egui::Rect::from_min_max(
            egui::pos2(sp.x - s, sp.y - s),
            egui::pos2(sp.x + s, sp.y + s),
        );
        painter.rect(r, 1.0, col, outline);
    };
    match g {
        Geom::Line(l) => {
            draw(l.a);
            draw(l.b);
            draw((l.a + l.b) * 0.5);
        }
        Geom::Circle(c) => {
            draw(c.center);
            draw(c.center + Vec2::new( c.radius, 0.0));
            draw(c.center + Vec2::new(-c.radius, 0.0));
            draw(c.center + Vec2::new(0.0,  c.radius));
            draw(c.center + Vec2::new(0.0, -c.radius));
        }
        Geom::Arc(a) => {
            draw(a.center);
            let (p1, p2) = a.endpoints();
            draw(p1);
            draw(p2);
            let m = a.start_angle + a.sweep_angle * 0.5;
            draw(a.center + Vec2::new(a.radius * m.cos(), a.radius * m.sin()));
        }
        Geom::Ellipse(el) => {
            draw(el.center);
            // axis-end grips (the QUA points)
            for t in [0.0, std::f64::consts::FRAC_PI_2,
                      std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2] {
                draw(el.point_at(t));
            }
        }
        Geom::EllipseArc(ea) => {
            draw(ea.ellipse.center);
            let (p1, p2) = ea.endpoints();
            draw(p1);
            draw(p2);
            let m = ea.start_param + ea.sweep_param * 0.5;
            draw(ea.ellipse.point_at(m));
        }
        Geom::Point(pt) => {
            draw(pt.location);
        }
        Geom::Polyline(p) => {
            for v in &p.vertices { draw(v.pos); }
        }
        Geom::Hatch(_) => {
            // Hatch MVP exposes no grips — boundary vertices may become
            // PolyVertex grips later. Nothing to draw.
        }
        Geom::Spline(s) => {
            // Spline grips = every control point. Dragging one
            // reshapes the curve locally (see GripRole::SplineCtrlPt
            // in the kernel).
            for p in &s.control_points { draw(*p); }
        }
    }
}

/// Ellipse rubber-band: major guide line + perpendicular guide from major to
/// cursor + a live full-ellipse polyline using the cursor's distance from the
/// major-axis as the semi-minor.
fn draw_ellipse_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    app: &CadApp,
    centre: Vec2,
    major_end: Vec2,
    cursor_world: Vec2,
    dash: egui::Stroke,
    hint: egui::Stroke,
) {
    painter.line_segment(
        [app.w2s(centre, rect), app.w2s(major_end, rect)], hint);
    let major = major_end - centre;
    if major.len() < EPS { return; }
    let v_hat = major.normalized().perp();
    let semi_minor = (cursor_world - centre).dot(v_hat).abs();
    if let Some(el) = ellipse_center_major_minor(centre, major_end, semi_minor) {
        draw_polyline_full_ellipse(painter, rect, app, &el, dash);
    }
    let along = (cursor_world - centre).dot(major.normalized());
    let foot = centre + major.normalized() * along;
    painter.line_segment([app.w2s(foot, rect), app.w2s(cursor_world, rect)], hint);
}

fn draw_polyline_full_ellipse(
    painter: &egui::Painter,
    rect: egui::Rect,
    app: &CadApp,
    el: &Ellipse,
    stroke: egui::Stroke,
) {
    let n = 64;
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = (i as f64 / n as f64) * std::f64::consts::TAU;
        pts.push(app.w2s(el.point_at(t), rect));
    }
    painter.add(egui::Shape::line(pts, stroke));
}

fn draw_dobject(
    painter: &egui::Painter,
    rect: egui::Rect,
    app: &CadApp,
    g: &Geom,
    color: egui::Color32,
) {
    draw_dobject_thick(painter, rect, app, g, color, 1.6);
}

/// Same as `draw_dobject` with a parameterised stroke width. Used for the
/// trim-cutter / extend-boundary pulse overlay (Item 5) which draws a
/// thicker animated outline above the dobject's normal color.
fn draw_dobject_thick(
    painter: &egui::Painter,
    rect: egui::Rect,
    app: &CadApp,
    g: &Geom,
    color: egui::Color32,
    width: f32,
) {
    let stroke = egui::Stroke::new(width, color);
    match g {
        Geom::Line(l) => {
            painter.line_segment([app.w2s(l.a, rect), app.w2s(l.b, rect)], stroke);
        }
        Geom::Circle(c) => {
            let center = app.w2s(c.center, rect);
            let r_px = c.radius as f32 * app.scale;
            painter.circle_stroke(center, r_px, stroke);
        }
        Geom::Arc(a) => {
            let r_px = (a.radius as f32 * app.scale).max(1.0);
            let n = ((r_px * 0.5).clamp(8.0, 256.0)) as usize;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = a.start_angle + (i as f64 / n as f64) * a.sweep_angle;
                let p = Vec2::new(
                    a.center.x + a.radius * t.cos(),
                    a.center.y + a.radius * t.sin(),
                );
                pts.push(app.w2s(p, rect));
            }
            painter.add(egui::Shape::line(pts, stroke));
        }
        Geom::Ellipse(el) => {
            // Tessellation density grows with the visible size on screen so
            // small ellipses stay cheap and large ones stay smooth.
            let r_px = (el.semi_major() as f32 * app.scale).max(1.0);
            let n = ((r_px * 0.7).clamp(16.0, 512.0)) as usize;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = (i as f64 / n as f64) * std::f64::consts::TAU;
                pts.push(app.w2s(el.point_at(t), rect));
            }
            painter.add(egui::Shape::line(pts, stroke));
        }
        Geom::EllipseArc(ea) => {
            let r_px = (ea.ellipse.semi_major() as f32 * app.scale).max(1.0);
            let n = ((r_px * 0.7).clamp(12.0, 512.0)) as usize;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = ea.start_param + (i as f64 / n as f64) * ea.sweep_param;
                pts.push(app.w2s(ea.ellipse.point_at(t), rect));
            }
            painter.add(egui::Shape::line(pts, stroke));
        }
        Geom::Point(pt) => {
            // POINT renders as a small cross-hair glyph at the location.
            // PDMODE / PDSIZE will dispatch glyph variants when wired;
            // today every point draws the same '+'.
            let sp = app.w2s(pt.location, rect);
            let s = 4.0_f32;
            painter.line_segment(
                [egui::pos2(sp.x - s, sp.y), egui::pos2(sp.x + s, sp.y)], stroke);
            painter.line_segment(
                [egui::pos2(sp.x, sp.y - s), egui::pos2(sp.x, sp.y + s)], stroke);
        }
        Geom::Polyline(p) => {
            // Bulge-aware tessellation so arc segments inside a polyline
            // render as actual arcs, not chords.
            let pts = polyline_tessellated_screen_pts(p, app, rect);
            if !pts.is_empty() {
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
        Geom::Hatch(_) => {
            // Hatch render needs to resolve boundary handles against
            // the Document — done by `App::render_hatch_fill` which
            // the main render loop short-circuits to BEFORE reaching
            // this free renderer. Reaching here means a caller passed
            // a Hatch without going through the main loop; that's a
            // bug, so silently drop instead of crashing.
            let _ = (color, stroke);
        }
        Geom::Spline(s) => {
            // NURBS → screen polyline. Density scales with on-screen
            // size of the control polygon; pickbox-precision at most
            // typical zooms. Refine when someone zooms in past the
            // chord error of a 64-sample tessellation.
            let (min, max) = s.bbox();
            let bbox_diag_px = ((max - min).len() as f32) * app.scale;
            let n = (bbox_diag_px * 0.5).clamp(32.0, 512.0) as usize;
            let samples = s.tessellate(n);
            if samples.len() < 2 { return; }
            let pts: Vec<egui::Pos2> = samples.iter()
                .map(|w| app.w2s(*w, rect)).collect();
            painter.add(egui::Shape::line(pts, stroke));
        }
    }
}

/// Render a Dobject's geometry as a DASHED polyline (used today for the
/// pointer-mode selection look; see `feedback_rust_cad_pointer_is_selector`).
/// Reuses the same per-variant tessellation as `draw_dobject`, then passes
/// the resulting polyline through `egui::Shape::dashed_line`.
fn draw_dobject_dashed(
    painter: &egui::Painter,
    rect: egui::Rect,
    app: &CadApp,
    g: &Geom,
    color: egui::Color32,
    dash: f32,
    gap: f32,
) {
    let stroke = egui::Stroke::new(1.6, color);
    let push_dashed = |pts: Vec<egui::Pos2>| {
        for s in egui::Shape::dashed_line(&pts, stroke, dash, gap) {
            painter.add(s);
        }
    };
    match g {
        Geom::Line(l) => {
            push_dashed(vec![app.w2s(l.a, rect), app.w2s(l.b, rect)]);
        }
        Geom::Circle(c) => {
            // Tessellate the circle into a closed polygon and dash it.
            let r_px = (c.radius as f32 * app.scale).max(1.0);
            let n = ((r_px * 0.7).clamp(24.0, 256.0)) as usize;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = (i as f64 / n as f64) * std::f64::consts::TAU;
                let p = Vec2::new(
                    c.center.x + c.radius * t.cos(),
                    c.center.y + c.radius * t.sin(),
                );
                pts.push(app.w2s(p, rect));
            }
            push_dashed(pts);
        }
        Geom::Arc(a) => {
            let r_px = (a.radius as f32 * app.scale).max(1.0);
            let n = ((r_px * 0.5).clamp(8.0, 256.0)) as usize;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = a.start_angle + (i as f64 / n as f64) * a.sweep_angle;
                let p = Vec2::new(
                    a.center.x + a.radius * t.cos(),
                    a.center.y + a.radius * t.sin(),
                );
                pts.push(app.w2s(p, rect));
            }
            push_dashed(pts);
        }
        Geom::Ellipse(el) => {
            let r_px = (el.semi_major() as f32 * app.scale).max(1.0);
            let n = ((r_px * 0.7).clamp(16.0, 512.0)) as usize;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = (i as f64 / n as f64) * std::f64::consts::TAU;
                pts.push(app.w2s(el.point_at(t), rect));
            }
            push_dashed(pts);
        }
        Geom::EllipseArc(ea) => {
            let r_px = (ea.ellipse.semi_major() as f32 * app.scale).max(1.0);
            let n = ((r_px * 0.7).clamp(12.0, 512.0)) as usize;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = ea.start_param + (i as f64 / n as f64) * ea.sweep_param;
                pts.push(app.w2s(ea.ellipse.point_at(t), rect));
            }
            push_dashed(pts);
        }
        Geom::Point(pt) => {
            // Point glyph stays solid even when "selected" — dashing a
            // 4-pixel cross is meaningless.
            let sp = app.w2s(pt.location, rect);
            let s = 4.0_f32;
            painter.line_segment(
                [egui::pos2(sp.x - s, sp.y), egui::pos2(sp.x + s, sp.y)], stroke);
            painter.line_segment(
                [egui::pos2(sp.x, sp.y - s), egui::pos2(sp.x, sp.y + s)], stroke);
        }
        Geom::Polyline(p) => {
            // Same bulge-aware tessellation as the solid render — when
            // the user dash-highlights a polyline with arc segments
            // the dashes follow the arcs, not the chords.
            let pts = polyline_tessellated_screen_pts(p, app, rect);
            if !pts.is_empty() {
                push_dashed(pts);
            }
        }
        Geom::Hatch(_) => {
            // The boundary dobjects already render their OWN selection
            // outline when selected — and editing happens on the
            // boundary, not on the hatch itself. Hatch dashed-overlay
            // is a no-op until we have a selectable-hatch UI flow.
        }
        Geom::Spline(s) => {
            // Selection highlight — same tessellation as the solid
            // render, dashed.
            let (min, max) = s.bbox();
            let bbox_diag_px = ((max - min).len() as f32) * app.scale;
            let n = (bbox_diag_px * 0.5).clamp(32.0, 512.0) as usize;
            let samples = s.tessellate(n);
            if samples.len() < 2 { return; }
            let pts: Vec<egui::Pos2> = samples.iter()
                .map(|w| app.w2s(*w, rect)).collect();
            push_dashed(pts);
        }
    }
}
