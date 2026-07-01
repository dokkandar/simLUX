//! Unified docking abstraction — see WORKSPACE_SYSTEM.md.
//!
//! The application must **never depend on a specific docking engine**. Every
//! dockable panel (Inspector, command bar, future tool panels) is rendered
//! through [`DockHost`]. [`EguiDockHost`] is the hand-rolled egui implementation
//! used today; to replace the engine (e.g. with `egui_dock`), add another
//! `impl DockHost` and point [`HOST`] at it — the call sites don't change.
//!
//! A panel calls [`DockHost::show`] with its content closure and a `&mut
//! DockState`. The host draws the chrome header (title · close · drag-to-undock),
//! the frame, and handles docked↔floating transitions. Behaviour is therefore
//! identical for every panel.

use egui::{Align2, Color32, Context, CursorIcon, FontId, Id, Pos2, Rect, Sense,
           Stroke, Ui, Vec2};

/// Which edge a panel is docked against.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DockRegion { Left, Right, Bottom }

/// Whether a panel is docked (to a region) or floating (at a screen position).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DockState { Docked(DockRegion), Floating(Pos2) }

/// Per-panel docking configuration.
pub struct DockConfig<'a> {
    pub id: &'a str,
    pub title: &'a str,
    /// The ONE edge this panel may dock to. A panel never docks anywhere else,
    /// so dragging it toward any other edge just leaves it floating (the command
    /// bar docks only Bottom, the rails only Left, the Inspector only Right).
    pub dock_region: DockRegion,
    /// Docked size on the variable axis (width for L/R, height for Bottom).
    pub size: f32,
    pub min: f32,
    pub max: f32,
    pub resizable: bool,
    /// Content width when floating. For an L/R panel this usually equals
    /// `size`; a Bottom-docking panel (whose `size` is a height) floats wider.
    pub float_w: f32,
    /// Floating height cap as a fraction of the screen (e.g. 0.5).
    pub float_max_h_frac: f32,
}

/// The replaceable docking-engine boundary. Swap the implementation, not the
/// call sites.
pub trait DockHost {
    /// Render one dockable panel. `body(ui, scroll_cap)` fills the content;
    /// `scroll_cap` is `Some` when floating so the panel can cap its scroll area
    /// at ~`float_max_h_frac` of the screen. Mutates `state`/`open` for
    /// dock/undock/close. Returns the panel's outer rect.
    fn show(&self, ctx: &Context, cfg: &DockConfig, state: &mut DockState,
            open: &mut bool, body: impl FnOnce(&mut Ui, Option<f32>)) -> Rect;
}

// ── local palette (reads theme tokens; the panel bg keeps its slightly warmer
// tone until the full token migration lands) ───────────────────────────────
const BG:     Color32 = Color32::from_rgb(0x18, 0x20, 0x29);
fn border() -> Color32 { crate::theme::color::BORDER }
fn chrome() -> Color32 { crate::theme::color::CHROME }
const TEXT:  Color32 = Color32::from_rgb(0xda, 0xe3, 0xef);
const MUTED: Color32 = Color32::from_rgb(0xb4, 0xb5, 0xb7);

/// Chrome header for a DOCKED panel (Frame-based). Returns
/// `(close_clicked, undock_to)` where `undock_to` is `Some(pos)` when the title
/// area was dragged out.
fn docked_header(ui: &mut Ui, cfg: &DockConfig) -> (bool, Option<Pos2>) {
    let mut close = false;
    let hdr = egui::Frame::none().fill(chrome())
        .inner_margin(egui::Margin::symmetric(12.0, 9.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(cfg.title).size(15.0).color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Label::new(egui::RichText::new("×").size(18.0).color(MUTED))
                        .sense(Sense::click()))
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text("Close").clicked() { close = true; }
                });
            });
        });
    ui.painter().hline(hdr.response.rect.x_range(), hdr.response.rect.bottom(),
        Stroke::new(1.0, border()));
    let mut dr = hdr.response.rect; dr.max.x -= 34.0; // leave × clickable
    let hd = ui.interact(dr, Id::new((cfg.id, "hdrdrag")), Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::Grab);
    let undock = if hd.drag_started() {
        hd.interact_pointer_pos()
            .map(|p| egui::pos2((p.x - 130.0).max(0.0), (p.y - 12.0).max(48.0)))
    } else { None };
    (close, undock)
}

/// Chrome header for a FLOATING panel (painted; the whole band drags the
/// window). Returns `(close_clicked, drag_delta, drag_released)`. `drag_released`
/// is the single frame the drag ends on — the host docks only then, so a panel
/// follows the pointer during the drag and snaps only when let go on its edge.
fn float_header(ui: &mut Ui, cfg: &DockConfig) -> (bool, Vec2, bool) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 34.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, chrome());
    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, border()));
    p.text(egui::pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER,
        cfg.title, FontId::proportional(15.0), TEXT);
    let xr = Rect::from_center_size(
        egui::pos2(rect.right() - 16.0, rect.center().y), egui::vec2(16.0, 16.0));
    let xresp = ui.interact(xr, Id::new((cfg.id, "fx")), Sense::click());
    p.text(xr.center(), Align2::CENTER_CENTER, "×", FontId::proportional(18.0),
        if xresp.hovered() { TEXT } else { MUTED });
    let mut dr = rect; dr.max.x -= 34.0;
    let dresp = ui.interact(dr, Id::new((cfg.id, "fdrag")), Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::Grab);
    let delta = if dresp.dragged() { dresp.drag_delta() } else { Vec2::ZERO };
    (xresp.clicked(), delta, dresp.drag_stopped())
}

/// The hand-rolled egui docking engine.
pub struct EguiDockHost;

impl DockHost for EguiDockHost {
    fn show(&self, ctx: &Context, cfg: &DockConfig, state: &mut DockState,
            open: &mut bool, body: impl FnOnce(&mut Ui, Option<f32>)) -> Rect {
        if !*open { return Rect::NOTHING; }
        let frame = egui::Frame::none().fill(BG).stroke(Stroke::new(1.0, border()))
            .inner_margin(egui::Margin::ZERO);

        match *state {
            DockState::Docked(_) => {
                // Always dock to the panel's one allowed edge, regardless of what
                // the stored state says — a panel can't be docked anywhere else.
                let region = cfg.dock_region;
                let mut close = false;
                let mut undock: Option<Pos2> = None;
                let rect = match region {
                    DockRegion::Left | DockRegion::Right => {
                        let sp = if region == DockRegion::Right {
                            egui::SidePanel::right(Id::new((cfg.id, "dock")))
                        } else {
                            egui::SidePanel::left(Id::new((cfg.id, "dock")))
                        };
                        sp.resizable(cfg.resizable)
                            .default_width(cfg.size).min_width(cfg.min).max_width(cfg.max)
                            .frame(frame)
                            .show(ctx, |ui| {
                                let (c, u) = docked_header(ui, cfg);
                                close = c; undock = u;
                                egui::Frame::none()
                                    .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                                    .show(ui, |ui| body(ui, None));
                            }).response.rect
                    }
                    DockRegion::Bottom => {
                        egui::TopBottomPanel::bottom(Id::new((cfg.id, "dock")))
                            .resizable(cfg.resizable)
                            .default_height(cfg.size).min_height(cfg.min).max_height(cfg.max)
                            .frame(frame)
                            .show(ctx, |ui| {
                                let (c, u) = docked_header(ui, cfg);
                                close = c; undock = u;
                                egui::Frame::none()
                                    .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                                    .show(ui, |ui| body(ui, None));
                            }).response.rect
                    }
                };
                if close { *open = false; }
                if let Some(p) = undock {
                    // Lift the float clear of the edge it just left so it doesn't
                    // sit inside the re-dock zone (esp. Bottom, whose header is
                    // near the screen bottom). Keeps undock → float from snapping
                    // straight back.
                    let sr = ctx.screen_rect();
                    let fp = match region {
                        // Float to the LOWER-CENTRE of the canvas. Horizontally
                        // centred on the window; it may overlay the right bar
                        // (the float draws on top of the side panels). It won't
                        // re-dock on its own — docking only happens on an actual
                        // drag-release near the bottom edge.
                        DockRegion::Bottom =>
                            egui::pos2(sr.center().x - cfg.float_w * 0.5,
                                       (sr.bottom() - 220.0).max(sr.top() + 56.0)),
                        DockRegion::Right =>
                            egui::pos2(p.x.min(sr.right() - cfg.float_w - 60.0), p.y),
                        DockRegion::Left =>
                            egui::pos2(p.x.max(sr.left() + 60.0), p.y),
                    };
                    *state = DockState::Floating(fp);
                }
                rect
            }
            DockState::Floating(pos) => {
                let cap = (ctx.screen_rect().height() * cfg.float_max_h_frac).max(160.0);
                let mut close = false;
                let mut delta = Vec2::ZERO;
                let mut released = false;
                let area = egui::Area::new(Id::new((cfg.id, "float")))
                    .order(egui::Order::Middle)
                    .fixed_pos(pos)
                    .constrain(true)
                    .show(ctx, |ui| {
                        egui::Frame::none().fill(BG).stroke(Stroke::new(1.0, border()))
                            .shadow(egui::epaint::Shadow {
                                offset: egui::vec2(0.0, 6.0), blur: 18.0, spread: 0.0,
                                color: Color32::from_black_alpha(120) })
                            .show(ui, |ui| {
                                ui.set_width(cfg.float_w);
                                let (c, d, r) = float_header(ui, cfg);
                                close = c; delta = d; released = r;
                                egui::Frame::none()
                                    .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                                    .show(ui, |ui| body(ui, Some(cap)));
                            });
                    });
                let wr = area.response.rect;
                if close { *open = false; }
                // Follow the pointer while dragging; dock ONLY when the drag is
                // released with the panel's own edge inside the snap zone. A
                // panel can dock to its `dock_region` and nowhere else, so it
                // never grabs the wrong side and never re-docks mid-move.
                let np = egui::pos2((pos.x + delta.x).max(0.0), (pos.y + delta.y).max(44.0));
                let sr = ctx.screen_rect();
                let near_edge = match cfg.dock_region {
                    DockRegion::Right  => wr.right()  >= sr.right()  - 48.0,
                    DockRegion::Left   => wr.left()   <= sr.left()   + 48.0,
                    DockRegion::Bottom => wr.bottom() >= sr.bottom() - 48.0,
                };
                *state = if released && near_edge {
                    DockState::Docked(cfg.dock_region)
                } else {
                    DockState::Floating(np)
                };
                wr
            }
        }
    }
}

/// The active docking engine. Replace this (and add an `impl DockHost`) to swap
/// the underlying engine app-wide.
pub const HOST: EguiDockHost = EguiDockHost;
