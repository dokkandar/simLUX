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

#[derive(Clone, Copy, PartialEq, Eq)]
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
}

/// State machine for the interactive copy tool — same shape as MoveState.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CopyState {
    Off,
    WaitingForBase,
    WaitingForDest(Vec2),
}

/// State machine for the interactive rotate tool:
/// click pivot, then a reference point (the angle baseline), then the
/// target point — sweep = atan2(target - pivot) - atan2(ref - pivot).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RotateState {
    Off,
    WaitingForPivot,
    WaitingForRef(Vec2),
    WaitingForTarget(Vec2, Vec2),   // (pivot, ref)
}

/// State machine for the interactive scale tool — pivot + reference
/// distance + target distance; factor = |target| / |ref|.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ScaleState {
    Off,
    WaitingForPivot,
    WaitingForRef(Vec2),
    WaitingForTarget(Vec2, f64),    // (pivot, ref_dist)
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderMode {
    Cpu,
    Gpu,
}

impl Default for CadApp {
    fn default() -> Self {
        let mut s = Self {
            doc:           Document::default(),
            intersections: Vec::new(),
            cmd:           String::new(),
            history:       Vec::new(),
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
            layer_name_counter: 0,
            pen_panel_open:     true,
            info_panel_open:    true,
            copy_state:   CopyState::Off,
            rotate_state: RotateState::Off,
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
        };
        // Demo layers so the Layer panel has visible content at first launch.
        let walls = s.doc.layers.add(Layer {
            name: "WALLS".into(), color: Color::rgb(255, 90, 90),
            ..Layer::layer_zero()
        });
        let _hidden = s.doc.layers.add(Layer {
            name: "HIDDEN".into(), color: Color::rgb(120, 220, 160),
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

    fn run_command(&mut self, raw: &str) {
        self.history.push(format!("> {}", raw));
        match parse(raw) {
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
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Rotate;
                    self.history.push(
                        "  rotate — Select dobjects, Enter to continue (Esc cancels)".into());
                } else {
                    self.rotate_state = RotateState::WaitingForPivot;
                    self.history.push(format!(
                        "  rotate — {} dobject(s) selected. Click PIVOT point",
                        self.selection.len()));
                }
            }
            Ok(Command::Scale) => {
                if self.selection.is_empty() {
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Scale;
                    self.history.push(
                        "  scale — Select dobjects, Enter to continue (Esc cancels)".into());
                } else {
                    self.scale_state = ScaleState::WaitingForPivot;
                    self.history.push(format!(
                        "  scale — {} dobject(s) selected. Click PIVOT point",
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
                self.history.push(
                    "  stretch — Click FIRST corner of crossing window (Esc cancels)".into());
            }
            Ok(Command::Move) => {
                if self.selection.is_empty() {
                    // No basket yet — auto-enter a selection session that
                    // transitions into MOVE on Enter. User can use any
                    // selection method (single click, window, crossing,
                    // `before`, `all`) inside the same flow.
                    self.begin_selection(SelectMode::ForSelect);
                    self.queued_op = QueuedOp::Move;
                    self.history.push(
                        "  move — Select dobjects to move: click / window / crossing / `before` / `all`, Enter to continue (Esc cancels)".into());
                } else {
                    // Basket already populated by a prior `select` — go
                    // straight to base / destination.
                    self.move_state = MoveState::WaitingForBase;
                    self.history.push(format!(
                        "  move — {} dobject(s) already selected. Click BASE point (Esc cancels)",
                        self.selection.len()));
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
                self.history.push(format!(
                    "  move — {} dobject(s). Click BASE point (Esc cancels)",
                    self.selection.len()));
            }
            QueuedOp::Copy => {
                self.copy_state = CopyState::WaitingForBase;
                self.history.push(format!(
                    "  copy — {} dobject(s). Click BASE point",
                    self.selection.len()));
            }
            QueuedOp::Rotate => {
                self.rotate_state = RotateState::WaitingForPivot;
                self.history.push(format!(
                    "  rotate — {} dobject(s). Click PIVOT", self.selection.len()));
            }
            QueuedOp::Scale => {
                self.scale_state = ScaleState::WaitingForPivot;
                self.history.push(format!(
                    "  scale — {} dobject(s). Click PIVOT", self.selection.len()));
            }
            QueuedOp::Mirror => {
                self.mirror_state = MirrorState::WaitingForA;
                self.history.push(format!(
                    "  mirror — {} dobject(s). Click FIRST axis point",
                    self.selection.len()));
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
        let crossing    = p2.x < p1.x;
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

    fn render_layer_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("layers")
            .min_width(240.0)
            .default_width(280.0)
            .show(ctx, |ui| {
                ui.heading(format!("Layers ({})", self.doc.layers.len()));
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
                                    let mut rgb = match layer.color {
                                        Color::TrueColor(_) =>
                                            layer.color.rgb_bytes().unwrap_or((255, 255, 255)),
                                        Color::Aci(i) => aci_palette(i),
                                        _ => (255, 255, 255),
                                    };
                                    let mut arr = [rgb.0, rgb.1, rgb.2];
                                    if ui.color_edit_button_srgb(&mut arr).changed() {
                                        rgb = (arr[0], arr[1], arr[2]);
                                        layer.color = Color::rgb(rgb.0, rgb.1, rgb.2);
                                    }

                                    // ----- name (click to rename) ---------
                                    if self.layer_rename == Some(id) {
                                        let resp = ui.text_edit_singleline(&mut self.layer_rename_buf);
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
            });
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
        egui::SidePanel::left("pens")
            .min_width(220.0)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading(format!("Pens ({})", self.doc.pens.len()));
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
                                    Color::TrueColor(_) =>
                                        pen.color.rgb_bytes().unwrap_or((128, 128, 128)),
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
        egui::SidePanel::left("info")
            .min_width(240.0)
            .default_width(280.0)
            .show(ctx, |ui| {
                ui.heading("Entity Info");
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

                // Color (cycle: ByLayer → TrueColor picker → ByLayer)
                ui.label("Color");
                let color = self.doc.dobjects[idx].style.color;
                let (r, g, b) = match color {
                    Color::TrueColor(_) => color.rgb_bytes().unwrap_or((255, 255, 255)),
                    Color::Aci(i)       => aci_palette(i),
                    Color::ByLayer | Color::ByBlock =>
                        resolve_color(color, self.doc.dobjects[idx].style.layer, &self.doc.layers),
                };
                let mut arr = [r, g, b];
                let label = match color {
                    Color::ByLayer => "ByLayer",
                    Color::ByBlock => "ByBlock",
                    Color::Aci(_)  => "ACI",
                    Color::TrueColor(_) => "RGB",
                };
                ui.horizontal(|ui| {
                    if ui.color_edit_button_srgb(&mut arr).changed() {
                        self.doc.dobjects[idx].style.color =
                            Color::rgb(arr[0], arr[1], arr[2]);
                        self.gpu_dirty = true;
                    }
                    ui.small(label);
                    if ui.small_button("ByLayer").clicked() {
                        self.doc.dobjects[idx].style.color = Color::ByLayer;
                        self.gpu_dirty = true;
                    }
                });
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
        if src_len < EPS {
            self.history.push("  ! align: source points coincide".into());
            return;
        }
        self.snapshot_doc();
        // Translate: s1 → t1; then rotate around t1 so (s2 - s1) aligns with (t2 - t1).
        let v = t1 - s1;
        let src_dir = (s2 - s1).angle();
        let tgt_dir = (t2 - t1).angle();
        let dtheta = (tgt_dir - src_dir).rem_euclid(std::f64::consts::TAU);
        let dtheta = if dtheta > std::f64::consts::PI {
            dtheta - std::f64::consts::TAU
        } else { dtheta };
        let n = self.selection.len();
        for &i in &self.selection {
            if let Some(d) = self.doc.dobjects.get_mut(i) {
                let translated = d.geom.translated(v);
                d.geom = translated.rotated(t1, dtheta);
            }
        }
        self.history.push(format!(
            "  ⇲ align: {} dobject(s)  shifted ({:.2},{:.2})  rotated {:.2}° around ({:.2},{:.2})",
            n, v.x, v.y, dtheta.to_degrees(), t1.x, t1.y));
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

/// Single-token commands that should also submit when the user presses Space
/// (mimicking AutoCAD's "spacebar = enter" convention). Multi-arg commands
/// like `line 0,0 5,0` still need Enter — Space inside them adds a real
/// space so the arguments parse correctly.
fn is_complete_single_token_command(cmd: &str) -> bool {
    matches!(cmd.trim().to_ascii_lowercase().as_str(),
        // snap-kind one-shot overrides
        "end" | "endpoint"
        | "mid" | "midpoint"
        | "cen" | "center" | "centre"
        | "qua" | "quadrant"
        | "int" | "intersect" | "intersection"
        | "per" | "perp" | "perpendicular"
        | "tan" | "tangent"
        | "nea" | "near" | "nearest"
        // arg-less commands
        | "clear" | "help" | "?" | "grips" | "grip"
        | "list"  | "ls"   | "select" | "sel"
        | "all"   | "prev" | "previous" | "before" | "none" | "deselect"
        | "rem"   | "remove" | "addmode" | "amode"
        | "move"  | "m"
    )
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
                self.history.push("  rotate cancelled".into());
            }
            if self.scale_state != ScaleState::Off {
                self.scale_state = ScaleState::Off;
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
            if self.stretch_state != StretchState::Off {
                self.stretch_state = StretchState::Off;
                self.history.push("  stretch cancelled".into());
            }
        }

        // Enter (when the command line is empty) finalises an in-progress
        // selection — this is the LibreCAD / AutoCAD convention. The cmd
        // box's own Enter handler only fires when the text isn't empty, so
        // there's no double-handling.
        if self.select_mode != SelectMode::Off && self.cmd.trim().is_empty()
            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
        {
            self.finalise_selection();
        }

        // Polyline tool: Enter (with empty cmd line) finishes the open
        // polyline at its current vertex list. Typed "c" or "close"
        // followed by Enter finishes with closed=true.
        if self.tool == Tool::Polyline
            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
        {
            let trimmed = self.cmd.trim().to_ascii_lowercase();
            let close = trimmed == "c" || trimmed == "close" || trimmed == "closed";
            if trimmed.is_empty() || close {
                if self.pending.len() >= 2 {
                    let verts = self.pending.drain(..).map(|p| PolyVertex {
                        pos: p, bulge: 0.0,
                    }).collect();
                    self.add_dobject(Geom::Polyline(Polyline {
                        vertices: verts, closed: close,
                    }), if close { "canvas (closed)" } else { "canvas" });
                    if close { self.cmd.clear(); }
                } else {
                    self.history.push(
                        "  ! polyline needs at least 2 vertices".into()
                    );
                    self.pending.clear();
                }
            }
        }

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

        // ---- right panel: dobjects + intersection list ------------------
        egui::SidePanel::right("dobjects").min_width(280.0).show(ctx, |ui| {
            ui.heading(format!("DObjects ({})", self.doc.dobjects.len()));
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

        // ---- bottom: command input + history ----------------------------
        egui::TopBottomPanel::bottom("cmd")
            .resizable(true)
            .default_height(180.0)
            .min_height(120.0)
            .show(ctx, |ui| {
                ui.heading("Command");
                egui::ScrollArea::vertical()
                    .id_salt("hist_scroll")
                    .stick_to_bottom(true)
                    .max_height(ui.available_height() - 32.0)
                    .show(ui, |ui| {
                        for h in &self.history {
                            ui.monospace(h);
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label(">");
                    let btn_w = 56.0_f32;
                    let row_h = ui.spacing().interact_size.y;
                    let text_resp = ui.add_sized(
                        [(ui.available_width() - btn_w - 8.0).max(40.0), row_h],
                        egui::TextEdit::singleline(&mut self.cmd)
                            .hint_text("type a command (end / mid / per / line 0,0 5,0 / grips / clear / help …)"),
                    );
                    let run_clicked = ui.button("run").clicked();
                    // Enter is detected both via the lost-focus pattern AND
                    // by a global pressed-this-frame check while focused, so
                    // the input never silently drops.
                    let enter_pressed = (text_resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        || (text_resp.has_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    // AutoCAD-style "Space submits". The TextEdit has already
                    // added the space char to `self.cmd` by the time we see
                    // the key event; trim it before checking if the trimmed
                    // string is a complete single-token command. Multi-arg
                    // commands keep Space as a literal separator (the check
                    // returns false for them).
                    let space_pressed = text_resp.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Space));
                    let submit_via_space = space_pressed && {
                        let candidate = self.cmd.trim_end_matches(' ');
                        is_complete_single_token_command(candidate)
                    };
                    if submit_via_space {
                        // Strip the trailing space we just consumed.
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
                    if self.refocus_cmd && !other_focused {
                        text_resp.request_focus();
                        self.refocus_cmd = false;
                    } else if !other_focused && !text_resp.has_focus() {
                        text_resp.request_focus();
                    }
                });
            });

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
            let snap_candidates: Vec<SnapHit> = if !self.doc.dobjects.is_empty()
                && (self.tool != Tool::None || self.snap_override.is_some())
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
            if resp.clicked() {
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
                                self.rotate_state = RotateState::WaitingForRef(click_world);
                                self.history.push(format!(
                                    "    rotate: PIVOT = ({:.3}, {:.3}) — click REFERENCE direction",
                                    click_world.x, click_world.y));
                            }
                            RotateState::WaitingForRef(pivot) => {
                                self.rotate_state = RotateState::WaitingForTarget(pivot, click_world);
                                self.history.push(
                                    "    rotate: REFERENCE captured — click TARGET direction".into());
                            }
                            RotateState::WaitingForTarget(pivot, refpt) => {
                                let a0 = (refpt - pivot).angle();
                                let a1 = (click_world - pivot).angle();
                                let dtheta = (a1 - a0).rem_euclid(std::f64::consts::TAU);
                                // Take the shorter rotation (negative if past PI)
                                let signed = if dtheta > std::f64::consts::PI {
                                    dtheta - std::f64::consts::TAU
                                } else { dtheta };
                                self.apply_rotate(pivot, signed);
                                self.rotate_state = RotateState::Off;
                            }
                            RotateState::Off => unreachable!(),
                        }
                        self.refocus_cmd = true;
                    } else if self.scale_state != ScaleState::Off {
                        match self.scale_state {
                            ScaleState::WaitingForPivot => {
                                self.scale_state = ScaleState::WaitingForRef(click_world);
                                self.history.push(format!(
                                    "    scale: PIVOT = ({:.3}, {:.3}) — click REFERENCE distance",
                                    click_world.x, click_world.y));
                            }
                            ScaleState::WaitingForRef(pivot) => {
                                let d = pivot.dist(click_world);
                                if d < EPS {
                                    self.history.push("  ! reference too close to pivot".into());
                                    self.scale_state = ScaleState::Off;
                                } else {
                                    self.scale_state = ScaleState::WaitingForTarget(pivot, d);
                                    self.history.push(format!(
                                        "    scale: REFERENCE d = {:.3} — click TARGET distance", d));
                                }
                            }
                            ScaleState::WaitingForTarget(pivot, ref_d) => {
                                let target_d = pivot.dist(click_world);
                                if target_d < EPS {
                                    self.history.push("  ! target too close to pivot".into());
                                } else {
                                    self.apply_scale(pivot, target_d / ref_d);
                                }
                                self.scale_state = ScaleState::Off;
                            }
                            ScaleState::Off => unreachable!(),
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
                        let color = if self.selected == Some(i) || in_selection {
                            egui::Color32::from_rgb(255, 200, 80)
                        } else if snap_source == Some(i) {
                            egui::Color32::from_rgb(120, 240, 255)
                        } else {
                            // Resolve through ByLayer / ByBlock to a concrete RGB.
                            let (r, g, b) = resolve_color(
                                e.style.color, e.style.layer, &self.doc.layers,
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
                    let sel_col  = 0xFFC850FFu32; // yellow
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
                        // base→destination arrow
                        let base_s = self.w2s(base, rect);
                        let dest_s = self.w2s(cw, rect);
                        painter.line_segment([base_s, dest_s],
                            egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 200, 100)));
                        painter.circle_filled(base_s, 4.0,
                            egui::Color32::from_rgb(255, 200, 100));
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

            // ---- selection mode overlay --------------------------------
            //
            // Rubber-band rectangle from the first-corner click to the
            // current cursor. Left-to-right drag = "inside" window (solid
            // blue); right-to-left = "crossing" window (dashed green).
            if self.select_mode != SelectMode::Off {
                let label = match self.select_mode {
                    SelectMode::ForList   => "LIST: select dobjects, Enter when done (Esc cancels)",
                    SelectMode::ForSelect => "SELECT: pick dobjects, Enter when done (Esc cancels)",
                    SelectMode::Off       => unreachable!(),
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

            ctx.request_repaint();
        });
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
    let stroke = egui::Stroke::new(1.6, color);
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
