// egui front-end. Pure visualization + command dispatch + interactive draw tools.
// All geometry comes from cad_kernel — no math defined in this file.

use eframe::egui;

use std::sync::Mutex;
use std::sync::Arc as StdArc;

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
enum Tool { None, Line, Circle, Arc, Ellipse, EllipseArc, Point, Polyline }

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
    /// Open/closed state for each dockable Window panel. Default true
    /// for the most-used panels. Toggled from the Tools menu.
    cmd_window_open:     bool,
    layers_window_open:  bool,
    pens_window_open:    bool,
    info_window_open:    bool,
    dobjects_window_open: bool,
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
    chamfer_state:  ChamferState,

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
            chamfer_state:  ChamferState::Off,
            trim_state:     TrimState::Off,
            extend_state:   ExtendState::Off,
            pre_op_selection: Vec::new(),
            trim_debug_log:  Vec::new(),
            trim_debug_open: false,
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
        // Remember the line as the "last command" for Enter-on-empty
        // repeat — but only NOW, AFTER the rotate/scale/etc sub-command
        // intercepts have had their shot. Numbers / R / C typed inside
        // a rotate or scale session are sub-command input, not top-level
        // commands; they must not overwrite last_command. (Bug from
        // 2026-06-03 screenshot: typing `2` mid-scale stored "2", so
        // the next Enter tried to parse "2" as a global command.)
        if !trimmed.is_empty() {
            self.last_command = Some(trimmed.to_string());
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
        match parse(&effective) {
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
            Ok(Command::Offset(d)) => {
                if self.selection.is_empty() {
                    self.history.push("  ! offset: empty basket — `select` first".into());
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
            Ok(Command::Fillet(r_opt)) => {
                if let Some(r) = r_opt {
                    self.env.FltRad = r;
                    let _ = self.env.save();
                }
                let r = self.env.FltRad;
                self.fillet_state = FilletState::WaitingForFirst(r);
                self.set_prompt(format!(
                    "fillet (r={}): click FIRST line on the SIDE to KEEP  [Esc=cancel]", r));
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
                self.set_prompt(format!(
                    "chamfer (d1={}, d2={}): click FIRST line  [Esc=cancel]", d1, d2));
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
        }
        // self.selection persists so follow-up commands (move, list, …) can use it.
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
        // If the user explicitly armed window/crossing via typed `w` /
        // `c`, that mode wins; otherwise fall back to drag direction.
        let crossing = match self.armed_window_inside.take() {
            Some(true)  => false,            // armed window → inside-only
            Some(false) => true,             // armed crossing
            None        => p2.x < p1.x,      // direction-default
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

    /// Trim Debug floating window — instrumented log of every trim /
    /// extend state transition + canvas click. User pastes the log back
    /// when reporting a bug.
    fn render_trim_debug_window(&mut self, ctx: &egui::Context) {
        let mut open = self.trim_debug_open;
        egui::Window::new("Trim Debug Log")
            .open(&mut open)
            .default_width(640.0)
            .default_height(400.0)
            .resizable(true)
            .show(ctx, |ui| {
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
        self.trim_debug_open = open;
    }

    fn render_layer_panel(&mut self, ctx: &egui::Context) {
        let mut open = self.layers_window_open;
        egui::Window::new(format!("Layers ({})", self.doc.layers.len()))
            .open(&mut open)
            .default_pos(egui::pos2(10.0, 70.0))
            .default_size(egui::vec2(320.0, 480.0))
            .min_width(240.0)
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
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
        egui::Window::new(format!("Pens ({})", self.doc.pens.len()))
            .open(&mut open)
            .default_pos(egui::pos2(340.0, 70.0))
            .default_size(egui::vec2(280.0, 420.0))
            .min_width(220.0)
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
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
        egui::Window::new("Info / Properties")
            .open(&mut open)
            .default_pos(egui::pos2(640.0, 70.0))
            .default_size(egui::vec2(300.0, 520.0))
            .min_width(240.0)
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
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
        let (mut nl, mut nc, mut na, mut ne, mut nea, mut npt, mut npl) =
            (0, 0, 0, 0, 0, 0, 0);
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
                }
            }
        }
        ui.monospace(format!(
            "  lines: {}\n  circles: {}\n  arcs: {}\n  ellipses: {}\n  \
             ellipse-arcs: {}\n  points: {}\n  polylines: {}",
            nl, nc, na, ne, nea, npt, npl
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
        match cad_kernel::fillet_lines(&l1, pick1, &l2, pick2, r) {
            Ok(out) => {
                // Replace in place (preserve handles + styles) and append arc.
                if let Some(d) = self.doc.dobjects.get_mut(idx1) { d.geom = out.g1_new; }
                if let Some(d) = self.doc.dobjects.get_mut(idx2) { d.geom = out.g2_new; }
                if let Some(arc) = out.arc {
                    let mut d = DObject::new(arc);
                    // Arc inherits style from the FIRST clicked line — same
                    // convention AutoCAD uses for the fillet entity.
                    d.style = style1;
                    let _ = style2;
                    self.doc.push(d);
                }
                self.history.push(format!(
                    "  ⌐ fillet ✓ r={} between #{} and #{}", r, idx1, idx2));
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
        match cad_kernel::chamfer_lines(&l1, pick1, &l2, pick2, d1_dist, d2_dist) {
            Ok(out) => {
                if let Some(d) = self.doc.dobjects.get_mut(idx1) { d.geom = out.g1_new; }
                if let Some(d) = self.doc.dobjects.get_mut(idx2) { d.geom = out.g2_new; }
                let mut bridge = DObject::new(out.bridge);
                bridge.style = style1;
                self.doc.push(bridge);
                self.history.push(format!(
                    "  ⌐ chamfer ✓ d=({}, {}) between #{} and #{}",
                    d1_dist, d2_dist, idx1, idx2));
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
        let Some(src) = self.selected else {
            self.history.push("  ! select an dobject first (click it in the right panel)".into());
            return;
        };
        if src >= self.doc.dobjects.len() {
            self.history.push("  ! invalid selection".into());
            return;
        }
        let source = self.doc.dobjects[src].clone();
        let cols = self.array_cols.max(1);
        let rows = self.array_rows.max(1);
        let dx   = self.array_dx;
        let dy   = self.array_dy;
        let copies = cols * rows;

        // pre-reserve to avoid repeated allocs
        self.doc.dobjects.reserve(copies - 1);
        for r in 0..rows {
            for c in 0..cols {
                if r == 0 && c == 0 { continue; }   // skip the source itself
                let off = Vec2::new(c as f64 * dx, r as f64 * dy);
                self.doc.dobjects.push(source.translated(off));
            }
        }
        let new_total = self.doc.dobjects.len();
        self.intersections.clear();
        self.index_dirty = true;        // bulk add — invalidate the index
        self.gpu_dirty   = true;        // upload-on-next-render
        self.history.push(format!(
            "  + array: {} cols × {} rows = {} copies → {} dobjects. (press an ∩ button to query)",
            cols, rows, copies - 1, new_total,
        ));
        // Immediately rebuild the spatial index so render culling kicks in on
        // the very next frame instead of waiting for the user's first ∩ press.
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

        // global Esc: cancel any in-progress draw or pick / intersect / select mode
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending.clear();
            self.tool = Tool::None;
            self.picking_source = false;
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
                self.history.push("  fillet cancelled".into());
            }
            if self.chamfer_state != ChamferState::Off {
                self.chamfer_state = ChamferState::Off;
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
                let verts = self.pending.drain(..).map(|p| PolyVertex {
                    pos: p, bulge: 0.0,
                }).collect();
                self.add_dobject(Geom::Polyline(Polyline {
                    vertices: verts, closed: false,
                }), "canvas");
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
                    let verts = self.pending.drain(..).map(|p| PolyVertex {
                        pos: p, bulge: 0.0,
                    }).collect();
                    self.add_dobject(Geom::Polyline(Polyline {
                        vertices: verts, closed: true,
                    }), "canvas (closed)");
                    self.cmd.clear();
                } else {
                    self.history.push("  ! polyline needs at least 2 vertices".into());
                    self.pending.clear();
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
                    if ui.button("Trim Debug Log").clicked() {
                        self.trim_debug_open = !self.trim_debug_open;
                        ui.close_menu();
                    }
                    if ui.button("Toggle Grips").clicked() {
                        self.env.GrpEnb = !self.env.GrpEnb;
                        let _ = self.env.save();
                        ui.close_menu();
                    }
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
                tool_button(ui, &mut self.tool, Tool::Ellipse,    "ellipse");
                tool_button(ui, &mut self.tool, Tool::EllipseArc, "ell.arc");
                tool_button(ui, &mut self.tool, Tool::Point,    "point");
                tool_button(ui, &mut self.tool, Tool::Polyline, "pline");
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
                if ui.button("clear all").clicked() {
                    self.clear_all();
                    self.history.push("  cleared".into());
                }
                if ui.button("array…").clicked() {
                    self.array_open = true;
                }
                // Spatial index rebuild button (auto-runs on first ∩ if dirty).
                let idx_label = if self.index_dirty || self.index.is_none() {
                    "rebuild idx ⟲"
                } else {
                    "idx ✓"
                };
                if ui.button(idx_label).clicked() {
                    self.ensure_index();
                }
                if ui.button("debug…").on_hover_text("CPU/GPU toggle + render stats").clicked() {
                    self.debug_open = !self.debug_open;
                }
                // OSNAP settings (floating window) + a quick on/off badge
                // showing which kinds are currently active.
                let snap_btn = format!(
                    "snap… ({})",
                    active_snap_letters(self.snap_enabled),
                );
                if ui.button(snap_btn)
                    .on_hover_text("object snap settings — END, MID, CEN, INT, PER, TAN, NEA")
                    .clicked()
                {
                    self.snap_window_open = !self.snap_window_open;
                }
                // GRIPS toggle (also: typing `grips` on the command line, or
                // the GrpEnb checkbox in the settings window).
                let grips_btn = if self.env.GrpEnb { "grips: ON" } else { "grips: off" };
                if ui.button(grips_btn)
                    .on_hover_text("GrpEnb — show grip handles on the selected dobject")
                    .clicked()
                {
                    self.env.GrpEnb = !self.env.GrpEnb;
                }
                // Settings window
                if ui.button("settings…")
                    .on_hover_text("User-Environment Settings (UserEnv)")
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
                // Layer panel toggle
                let layer_btn = if self.layer_panel_open { "layers ▾" } else { "layers ▸" };
                if ui.button(layer_btn)
                    .on_hover_text("Layer panel — add / rename / delete / visibility / lock / freeze")
                    .clicked()
                {
                    self.layer_panel_open = !self.layer_panel_open;
                }
                // Pen palette toggle
                let pen_btn = if self.pen_panel_open { "pens ▾" } else { "pens ▸" };
                if ui.button(pen_btn)
                    .on_hover_text("Pen palette — preset (color + linetype + lineweight) bundles; click to apply to selection")
                    .clicked()
                {
                    self.pen_panel_open = !self.pen_panel_open;
                }
                // Entity Info panel toggle
                let info_btn = if self.info_panel_open { "info ▾" } else { "info ▸" };
                if ui.button(info_btn)
                    .on_hover_text("Entity Info — properties of current selection (read + edit layer/color/visibility)")
                    .clicked()
                {
                    self.info_panel_open = !self.info_panel_open;
                }
                // Trim Debug window toggle
                let tdbg_label = if self.trim_debug_open { "trim dbg ▾" } else { "trim dbg ▸" };
                if ui.button(tdbg_label)
                    .on_hover_text("Trim Debug — log every click + state transition; Copy Log button writes to clipboard")
                    .clicked()
                {
                    self.trim_debug_open = !self.trim_debug_open;
                }
                ui.add_space(20.0);
                ui.label("intersect:");
                // View-mode: intersect only dobjects visible in the current viewport.
                // The action is deferred to the canvas closure (which is where the
                // viewport rect actually becomes known); we just set a flag here.
                if ui.button("∩ view").clicked() {
                    self.intersect_view_pending = true;
                }
                // Click-mode: arm a one-shot, the next canvas click computes
                // intersections in a 50-pixel radius around the click point.
                let click_btn_text = if self.intersect_pending_click {
                    "∩ click (waiting…)"
                } else { "∩ click" };
                if ui.button(click_btn_text).clicked() {
                    self.intersect_pending_click = !self.intersect_pending_click;
                }
                if ui.button("clear ∩").clicked() {
                    self.intersections.clear();
                    self.last_intersect_label.clear();
                }
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
            egui::Window::new("DEBUG — render mode")
                .open(&mut keep)
                .resizable(true)
                .default_width(310.0)
                .default_pos(egui::pos2(20.0, 130.0))
                .show(ctx, |ui| {
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
                    ui.monospace(format!("FPS         {:>6.1}", fps));
                    ui.monospace(format!("dobjects    {:>6}", dobject_count));
                    ui.monospace(format!("  circles   {:>6}", circle_count));
                    ui.monospace(format!("  other     {:>6}",
                        dobject_count.saturating_sub(circle_count)));
                    ui.separator();
                    ui.label(egui::RichText::new("Notes").small());
                    ui.small("• GPU path: one PaintCallback, one glDrawArraysInstanced");
                    ui.small("• GPU renders Circles only this slice; Lines/Arcs stay CPU");
                    ui.small("• Switch back to CPU any time for comparison");
                });
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
        // Two states:
        //  - picking_source = true  → dialog HIDDEN, small banner shown, waiting
        //                              for the user to click an dobject in the right
        //                              panel; the click leaves pick-mode and the
        //                              dialog reappears.
        //  - picking_source = false → full dialog shown.
        if self.array_open {
            if self.picking_source {
                // Banner — small, doesn't obscure the right panel.
                egui::Window::new("Pick array source")
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.set_min_width(280.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 220, 100),
                            "→ Click an dobject in the right panel.",
                        );
                        ui.label("It will be highlighted and the array dialog will return.");
                        if ui.button("Cancel pick").clicked() {
                            self.picking_source = false;
                        }
                    });
            } else {
                let mut do_generate = false;
                let mut close_it    = false;
                let mut start_pick  = false;
                let selected_desc = self.selected.and_then(|i| self.doc.dobjects.get(i))
                    .map(|d| describe(&d.geom));
                egui::Window::new("Rectangular Array")
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.set_min_width(340.0);
                        ui.label("Duplicates the selected dobject into a grid.");
                        ui.separator();

                        // Source row: "Select source" button + current selection display
                        ui.horizontal(|ui| {
                            if ui.button("Select source ↓").clicked() {
                                start_pick = true;
                            }
                            match &selected_desc {
                                Some(d) => { ui.monospace(format!("#{} {}",
                                    self.selected.unwrap(), d)); }
                                None    => { ui.colored_label(
                                    egui::Color32::from_rgb(255, 140, 140),
                                    "no source selected"); }
                            }
                        });
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
                        let total = self.array_cols * self.array_rows;
                        let total_after = self.doc.dobjects.len() + total.saturating_sub(1);
                        ui.label(format!(
                            "{} copies → {} dobjects total after generation",
                            total - 1, total_after
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
                            if ui.add_enabled(self.selected.is_some(),
                                              egui::Button::new("Generate")).clicked() {
                                do_generate = true;
                            }
                            if ui.button("Close").clicked() {
                                close_it = true;
                            }
                        });
                    });
                if start_pick  { self.picking_source = true; }
                if do_generate { self.generate_array(); }
                if close_it    { self.array_open = false; }
            }
        }

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

        // ---- DObjects palette — floating Window -------------------------
        let mut dobjects_open = self.dobjects_window_open;
        let dobjects_count = self.doc.dobjects.len();
        egui::Window::new(format!("DObjects ({})", dobjects_count))
            .open(&mut dobjects_open)
            .default_pos(egui::pos2(
                ctx.screen_rect().right() - 320.0, 70.0))
            .default_size(egui::vec2(300.0, 520.0))
            .min_width(220.0)
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
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
        egui::Window::new("Command")
            .open(&mut cmd_open)
            .default_pos(cmd_default_pos)
            .default_size(egui::vec2(720.0, 180.0))
            .min_width(360.0)
            .min_height(120.0)
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
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
        self.cmd_window_open = cmd_open;

        // ---- central panel: canvas --------------------------------------
        egui::CentralPanel::default().show(ctx, |ui| {
            let avail = ui.available_size();
            let (resp, painter) =
                ui.allocate_painter(avail, egui::Sense::click_and_drag());
            let rect = resp.rect;

            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(18, 22, 28));

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
            let snap_candidates: Vec<SnapHit> = if !self.doc.dobjects.is_empty()
                && snap_phase_active
                && !self.picking_source && !self.intersect_pending_click
            {
                resp.hover_pos().map(|cur| {
                    let world = self.s2w(cur, rect);
                    let world_radius = self.env.SpTGSZ as f64 / self.scale as f64;
                    let grid = if self.index_dirty { None } else { self.index.as_ref() };
                    find_all_snaps(
                        world, world_radius,
                        self.snap_enabled, self.snap_override,
                        self.pending.last().copied(),
                        &self.doc.dobjects, grid,
                    )
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
            // Outside select mode + no Shift: drag_stopped → click.
            // No motion threshold; the user's gesture wins.
            let drag_intent_is_window =
                (in_select || (shift_held && !in_click_only_phase))
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
                        let tol = (self.env.GrpSz as f64 + 4.0) / self.scale as f64;
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
                    let world = self.s2w(pos, rect);
                    // Use the snap point if one is active — same convention
                    // as the draw tools, so move base / destination can land
                    // on END / MID / CEN / etc.
                    let click_world = snap_hit.map(|h| h.point).unwrap_or(world);

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
                                self.fillet_state = FilletState::Off;
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
                                self.chamfer_state = ChamferState::Off;
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
                        if let Some(i) = self.nearest_entity_under(world, tol_world) {
                            self.click_select(i, shift);
                            self.window_first = None;   // any half-started window is dropped
                        } else if let Some(first) = self.window_first.take() {
                            self.add_window_selection(first, world, shift);
                        } else {
                            self.window_first = Some(world);
                            self.history.push(
                                "    window: click opposite corner (L→R inside, R→L crossing — hold Shift to subtract)".into());
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
                        self.pending.push(click_world);
                        self.try_finalise();
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
            let candidate_iter: Box<dyn Iterator<Item = usize>> =
                if let (Some(g), false) = (self.index.as_ref(), self.index_dirty) {
                    Box::new(g.query_bbox(v_min, v_max).into_iter().map(|u| u as usize))
                } else {
                    Box::new(0..self.doc.dobjects.len())
                };

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
            let cutter_or_bound_active =
                trim_cutters.is_some() || extend_bounds.is_some();
            if cutter_or_bound_active {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            }
            let pulse_t = ctx.input(|i| i.time);
            // sin: -1..1  →  pulse: 0.15..0.85
            let pulse = 0.5 + 0.35 * (pulse_t * std::f64::consts::TAU * 1.4).sin();
            let pulse_alpha = (pulse.clamp(0.15, 0.85) * 255.0) as u8;

            let mut drawn   = 0usize;
            let mut skipped = 0usize;
            let mut gpu_circles_count = 0usize;
            match self.render_mode {
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
                        // Basket members render as DASHED GRAY (the pointer-
                        // mode selection look — does NOT modify style).
                        // self.selected (array-dialog single pick) stays
                        // yellow so it's distinguishable.
                        if in_selection {
                            let gray = egui::Color32::from_rgb(160, 165, 175);
                            draw_dobject_dashed(&painter, rect, self, &e.geom,
                                gray, 6.0, 4.0);
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
                        draw_dobject(&painter, rect, self, &e.geom, color);
                        drawn += 1;
                    }
                }
                RenderMode::Gpu => {
                    // GPU path: build instance buffer for circles only this slice.
                    // Non-circle dobjects still go through CPU (mixed rendering)
                    // so lines/arcs are visible. Future slices add their own
                    // instance kinds.
                    let mut circles: Vec<CircleInstance> = Vec::new();
                    // GPU path: dashed gray for basket isn't available yet
                    // (GPU dashing would need a shader change). Render solid
                    // gray instead so basket members are still distinguishable.
                    let sel_col  = 0xA0A5AFFFu32; // gray (basket / selected)
                    let snap_col = 0x78F0FFFFu32; // cyan
                    let def_col  = 0xAAC8E6FFu32; // light blue
                    for i in candidate_iter {
                        let e = &self.doc.dobjects[i];
                        let (emin, emax) = e.bbox();
                        if emax.x < v_min.x || emin.x > v_max.x
                        || emax.y < v_min.y || emin.y > v_max.y {
                            continue;
                        }
                        let in_selection = self.selection.contains(&i);
                        match &e.geom {
                            Geom::Circle(c) => {
                                let color = if self.selected == Some(i) || in_selection { sel_col }
                                    else if snap_source == Some(i) { snap_col }
                                    else { def_col };
                                circles.push(CircleInstance {
                                    x: c.center.x as f32,
                                    y: c.center.y as f32,
                                    r: c.radius as f32,
                                    color,
                                });
                                drawn += 1;
                            }
                            _ => {
                                // line / arc: still CPU
                                let color = if self.selected == Some(i) || in_selection {
                                    egui::Color32::from_rgb(255, 200, 80)
                                } else if snap_source == Some(i) {
                                    egui::Color32::from_rgb(120, 240, 255)
                                } else {
                                    egui::Color32::from_rgb(170, 200, 230)
                                };
                                draw_dobject(&painter, rect, self, &e.geom, color);
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
            }
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
            let mode_str = match self.render_mode {
                RenderMode::Cpu => format!("CPU"),
                RenderMode::Gpu => format!("GPU ({} circles instanced)", gpu_circles_count),
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
                    for (gp, _role) in geom_ref.grip_points() {
                        let sp = self.w2s(gp, rect);
                        let r = egui::Rect::from_center_size(
                            sp, egui::vec2(gsz * 2.0, gsz * 2.0));
                        let hot = self.grip_drag
                            .map(|gd| gd.dobject_idx == *idx
                                 && gd.grip_origin.dist(gp) < 1e-6)
                            .unwrap_or(false);
                        let col = if hot { grip_col_s } else { grip_col_u };
                        painter.rect_filled(r, 0.0, col);
                        painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0,
                            egui::Color32::from_rgb(20, 20, 20)));
                    }
                }
            }

            // Live rubber-band preview while a window-drag is in progress
            // (select mode active OR Shift held in any other phase except
            // edit-active phases). L→R draws BLUE (window — fully-inside);
            // R→L draws GREEN (crossing — anything touching).
            if resp.dragged()
                && (in_select || (shift_held && !in_click_only_phase))
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
                    let cur_world = snap_hit.map(|h| h.point)
                        .or_else(|| resp.hover_pos().map(|p| self.s2w(p, rect)));
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
                    let cur_world = snap_hit.map(|h| h.point)
                        .or_else(|| resp.hover_pos().map(|p| self.s2w(p, rect)));
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
            // effect — see the click pipeline override above. Drawn
            // last so it sits on top of dobjects and previews.
            if in_click_only_phase {
                if let Some(p) = resp.hover_pos() {
                    let half  = 7.0_f32;     // pickbox half-edge in px
                    let arm   = 14.0_f32;    // crosshair arm half-length
                    let col   = egui::Color32::from_rgb(235, 235, 245);
                    let stroke= egui::Stroke::new(1.0, col);
                    // square
                    let sq = egui::Rect::from_center_size(p, egui::vec2(half*2.0, half*2.0));
                    painter.rect_stroke(sq, 0.0, stroke);
                    // cross arms (extend a bit past the square edges)
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
            // Polyline = piecewise lines. Bulges deferred — straight only
            // for now (matches the kernel's distance/bbox approximations).
            if p.vertices.len() < 2 { return; }
            let n = p.vertices.len();
            let count = if p.closed { n + 1 } else { n };
            let mut pts: Vec<egui::Pos2> = (0..count).map(|i| {
                app.w2s(p.vertices[i % n].pos, rect)
            }).collect();
            // Tidy: if not closed, the +1 above never executed, but the
            // length stayed = n. Just pass to Shape::line.
            if !p.closed { pts.truncate(n); }
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
            if p.vertices.len() < 2 { return; }
            let n = p.vertices.len();
            let count = if p.closed { n + 1 } else { n };
            let mut pts: Vec<egui::Pos2> = (0..count).map(|i| {
                app.w2s(p.vertices[i % n].pos, rect)
            }).collect();
            if !p.closed { pts.truncate(n); }
            push_dashed(pts);
        }
    }
}
