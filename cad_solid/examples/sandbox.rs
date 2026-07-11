//! SIMLUX 3D Solid Sandbox — standalone modeler for parametric CSG solids.
//!
//! `cargo run -p cad_solid --example sandbox`
//!
//! Isolated window (no simLUX app, no persistence): pick a construction plane, add
//! parametric primitives, boolean them via csgrs, and modify selected solids with
//! the SAME select-first modifiers as the 2D app — Move / Copy / Rotate / Scale /
//! Mirror, base-point + second-point, with a CARD cardinal-lock. A corner view
//! navigator snaps to standard views. UI palette mirrors the app's `theme.rs`.

use std::f32::consts::{FRAC_PI_2, PI};
use std::sync::{Arc, Mutex};

use eframe::glow;
use eframe::glow::HasContext;
use glam::{Mat4, Vec3};

use cad_solid::modify::{Feed, Modify, ModifyOp};
use cad_solid::{BoolOp, Model, Placement, Plane, PlaneKind, Primitive, SolidMesh};

// ─────────────────────────────────────────────────────────────────────────────
// Design tokens — copied from cad_app/src/theme.rs so the sandbox reads like the
// app (teal-navy surfaces, cyan accent). Fonts are the app's one extra; deferred.
// ─────────────────────────────────────────────────────────────────────────────
mod theme {
    use egui::{Color32, Rounding, Stroke, Visuals};
    pub const SURFACE_0: Color32 = Color32::from_rgb(0x14, 0x1c, 0x25);
    pub const SURFACE_1: Color32 = Color32::from_rgb(0x1a, 0x24, 0x30);
    pub const SURFACE_2: Color32 = Color32::from_rgb(0x22, 0x2b, 0x34);
    pub const SURFACE_3: Color32 = Color32::from_rgb(0x2a, 0x37, 0x44);
    pub const BORDER: Color32 = Color32::from_rgb(0x34, 0x41, 0x4b);
    pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0xe5, 0xff);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xda, 0xe3, 0xef);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x93, 0xa1, 0xac);
    pub const WARNING: Color32 = Color32::from_rgb(0xf2, 0xb5, 0x3d);
    pub const DANGER: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

    pub fn apply(ctx: &egui::Context) {
        let mut v = Visuals::dark();
        v.window_fill = SURFACE_3;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.extreme_bg_color = SURFACE_0;
        v.faint_bg_color = SURFACE_2;
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = Color32::from_rgba_unmultiplied(0x00, 0xe5, 0xff, 60);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        let w = &mut v.widgets;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
        w.inactive.bg_fill = SURFACE_2;
        w.inactive.weak_bg_fill = SURFACE_2;
        w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
        w.inactive.rounding = Rounding::same(8.0);
        w.hovered.bg_fill = SURFACE_3;
        w.hovered.weak_bg_fill = SURFACE_3;
        w.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
        w.hovered.rounding = Rounding::same(8.0);
        w.active.bg_fill = SURFACE_3;
        w.active.weak_bg_fill = SURFACE_3;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.rounding = Rounding::same(8.0);
        ctx.set_visuals(v);
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_title("SIMLUX — 3D Solid Sandbox"),
        ..Default::default()
    };
    eframe::run_native("cad_solid_sandbox", options, Box::new(|_cc| Ok(Box::new(Sandbox::new()))))
}

/// Standard camera views for the corner navigator.
#[derive(Clone, Copy)]
enum ViewPreset {
    Top,
    Bottom,
    Front,
    Back,
    Left,
    Right,
    Iso,
}

impl ViewPreset {
    fn angles(self) -> (f32, f32) {
        let up = FRAC_PI_2 - 0.02;
        match self {
            ViewPreset::Top => (0.0, up),
            ViewPreset::Bottom => (0.0, -up),
            ViewPreset::Front => (-FRAC_PI_2, 0.0),
            ViewPreset::Back => (FRAC_PI_2, 0.0),
            ViewPreset::Right => (0.0, 0.0),
            ViewPreset::Left => (PI, 0.0),
            ViewPreset::Iso => (0.9, 0.62),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// App state
// ─────────────────────────────────────────────────────────────────────────────
struct Sandbox {
    model: Model,
    cached: SolidMesh,
    dirty: bool,

    // camera (orbit)
    yaw: f32,
    pitch: f32,
    dist: f32,

    // interaction
    card: bool,
    selection: Vec<u32>,
    modify: Option<Modify>,
    status: String,
    hover_plane_pt: Option<Vec3>,

    // authoring state for the NEXT primitive
    plane: Plane,
    place: Placement,
    op: BoolOp,
    prim_is_box: bool,
    box_wdh: [f32; 3],
    cyl_rh: [f32; 2],
    cyl_sides: u32,

    renderer: Arc<Mutex<SceneRenderer>>,
}

impl Sandbox {
    fn new() -> Self {
        let mut model = Model::default();
        model.push(BoolOp::Union, Plane::default(), Placement::default(), Primitive::Box { w: 2.0, d: 2.0, h: 1.0 });
        model.push(
            BoolOp::Difference,
            Plane::default(),
            Placement { u: 0.0, v: 0.0, lift: -0.3, spin_deg: 0.0 },
            Primitive::Cylinder { r: 0.55, h: 1.6, sides: 32 },
        );
        let cached = model.eval();
        Self {
            model,
            cached,
            dirty: false,
            yaw: 0.9,
            pitch: 0.62,
            dist: 6.5,
            card: false,
            selection: Vec::new(),
            modify: None,
            status: String::new(),
            hover_plane_pt: None,
            plane: Plane::default(),
            place: Placement::default(),
            op: BoolOp::Union,
            prim_is_box: true,
            box_wdh: [1.0, 1.0, 1.0],
            cyl_rh: [0.5, 1.0],
            cyl_sides: 32,
            renderer: Arc::new(Mutex::new(SceneRenderer::default())),
        }
    }

    fn recompute_if_dirty(&mut self) {
        if self.dirty {
            self.cached = self.model.eval();
            self.dirty = false;
        }
    }

    fn next_primitive(&self) -> Primitive {
        if self.prim_is_box {
            Primitive::Box { w: self.box_wdh[0], d: self.box_wdh[1], h: self.box_wdh[2] }
        } else {
            Primitive::Cylinder { r: self.cyl_rh[0], h: self.cyl_rh[1], sides: self.cyl_sides }
        }
    }

    fn frame(&mut self) {
        if let Some((mn, mx)) = self.cached.bounds() {
            let ext = ((mx[0] - mn[0]).max(mx[1] - mn[1]).max(mx[2] - mn[2])).max(0.5);
            self.dist = ext * 2.6;
        }
    }

    fn target(&self) -> Vec3 {
        match self.cached.bounds() {
            Some((mn, mx)) => Vec3::new((mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5),
            None => Vec3::ZERO,
        }
    }

    /// Start a select-first modifier (mirrors the 2D `run_command` arm: empty
    /// selection → prompt to select; else enter the base-point flow).
    fn start_modify(&mut self, op: ModifyOp) {
        if self.selection.is_empty() {
            self.status = format!("{}: select objects first", op.label().to_lowercase());
            return;
        }
        let m = Modify::new(op, self.selection.clone());
        self.status = m.prompt();
        self.modify = Some(m);
    }

    /// Unproject `cursor` (in `rect`) to a ray, intersect the active construction
    /// plane. `None` if the ray is parallel to or points away from the plane.
    fn cursor_on_plane(&self, cursor: egui::Pos2, rect: egui::Rect, mvp: &[f32; 16]) -> Option<Vec3> {
        let (near, dir) = self.ray(cursor, rect, mvp);
        let n = self.plane.normal();
        let denom = dir.dot(n);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (self.plane.origin() - near).dot(n) / denom;
        (t >= 0.0).then(|| near + dir * t)
    }

    /// Ray-pick the front-most feature under `cursor` via its world AABB.
    fn pick(&self, cursor: egui::Pos2, rect: egui::Rect, mvp: &[f32; 16]) -> Option<u32> {
        let (near, dir) = self.ray(cursor, rect, mvp);
        let mut best: Option<(f32, u32)> = None;
        for f in &self.model.features {
            let (mn, mx) = f.world_aabb();
            if let Some(t) = cad_solid::ray_aabb(near, dir, mn, mx) {
                if best.map_or(true, |(bt, _)| t < bt) {
                    best = Some((t, f.id));
                }
            }
        }
        best.map(|(_, id)| id)
    }

    /// World-space ray (origin, unit dir) through a screen cursor.
    fn ray(&self, cursor: egui::Pos2, rect: egui::Rect, mvp: &[f32; 16]) -> (Vec3, Vec3) {
        let ndc_x = 2.0 * (cursor.x - rect.left()) / rect.width().max(1.0) - 1.0;
        let ndc_y = 1.0 - 2.0 * (cursor.y - rect.top()) / rect.height().max(1.0);
        let inv = Mat4::from_cols_array(mvp).inverse();
        let near = inv.project_point3(Vec3::new(ndc_x, ndc_y, -1.0));
        let far = inv.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (near, (far - near).normalize_or_zero())
    }
}

impl eframe::App for Sandbox {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::apply(ctx);
        // Esc cancels an in-flight modifier; Del removes the selection.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.modify = None;
            self.status.clear();
        }
        if self.modify.is_none()
            && !self.selection.is_empty()
            && ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            for id in std::mem::take(&mut self.selection) {
                self.model.remove(id);
            }
            self.dirty = true;
        }
        self.controls_panel(ctx);
        self.viewport_panel(ctx);
        self.navigator(ctx);
        self.recompute_if_dirty();
    }
}

impl Sandbox {
    fn controls_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("controls")
            .exact_width(300.0)
            .frame(egui::Frame::none().fill(theme::SURFACE_1).inner_margin(14.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("3D Solid Sandbox");
                    ui.label(egui::RichText::new("parametric CSG · csgrs").color(theme::TEXT_MUTED).size(11.0));
                    ui.add_space(10.0);

                    // ── Modify (2D-identical: select-first, base + 2nd point) ──
                    section(ui, "MODIFY");
                    ui.horizontal_wrapped(|ui| {
                        for op in ModifyOp::ALL {
                            if ui.button(op.label()).clicked() {
                                self.start_modify(op);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        let card = ui.selectable_label(self.card, "CARD");
                        if card.clicked() {
                            self.card = !self.card;
                        }
                        card.on_hover_text("Cardinal lock: Move → single axis, Rotate → 90° snap");
                        if self.modify.is_some() && ui.button("Cancel (Esc)").clicked() {
                            self.modify = None;
                            self.status.clear();
                        }
                    });
                    if !self.status.is_empty() {
                        ui.label(egui::RichText::new(&self.status).color(theme::ACCENT).size(12.0));
                    } else {
                        ui.label(
                            egui::RichText::new(format!("{} selected", self.selection.len()))
                                .color(theme::TEXT_MUTED)
                                .size(11.0),
                        );
                    }
                    ui.add_space(12.0);

                    // ── Construction plane ──────────────────────────────────
                    section(ui, "CONSTRUCTION PLANE");
                    ui.horizontal(|ui| {
                        for k in PlaneKind::ALL {
                            if ui.selectable_label(self.plane.kind == k, k.label()).clicked() {
                                self.plane.kind = k;
                            }
                        }
                        ui.label("offset");
                        ui.add(egui::DragValue::new(&mut self.plane.offset).speed(0.05).suffix(" m"));
                    });
                    ui.add_space(12.0);

                    // ── Add primitive ───────────────────────────────────────
                    section(ui, "ADD PRIMITIVE");
                    ui.horizontal(|ui| {
                        if ui.selectable_label(self.prim_is_box, "Box").clicked() {
                            self.prim_is_box = true;
                        }
                        if ui.selectable_label(!self.prim_is_box, "Cylinder").clicked() {
                            self.prim_is_box = false;
                        }
                    });
                    if self.prim_is_box {
                        xyz_row(ui, "size", &mut self.box_wdh);
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("radius");
                            ui.add(egui::DragValue::new(&mut self.cyl_rh[0]).speed(0.02).range(0.02..=100.0));
                            ui.label("height");
                            ui.add(egui::DragValue::new(&mut self.cyl_rh[1]).speed(0.02).range(0.02..=100.0));
                            ui.label("sides");
                            ui.add(egui::DragValue::new(&mut self.cyl_sides).range(3..=128));
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("bool").color(theme::TEXT_MUTED).size(11.0));
                        for op in BoolOp::ALL {
                            if ui.selectable_label(self.op == op, format!("{} {}", op.glyph(), op.label())).clicked() {
                                self.op = op;
                            }
                        }
                    });
                    if ui
                        .add_sized([ui.available_width(), 26.0], egui::Button::new("＋ Add feature").fill(theme::SURFACE_3))
                        .clicked()
                    {
                        let prim = self.next_primitive();
                        let id = self.model.push(self.op, self.plane, self.place, prim);
                        self.selection = vec![id];
                        self.dirty = true;
                    }
                    ui.add_space(12.0);

                    // ── Selected feature (edit when exactly one) ────────────
                    self.selected_section(ui);

                    // ── Feature history ─────────────────────────────────────
                    section(ui, &format!("FEATURES ({})", self.model.features.len()));
                    let mut click_select: Option<u32> = None;
                    let add = ui.input(|i| i.modifiers.shift);
                    egui::ScrollArea::vertical().id_salt("feat_list").max_height(140.0).show(ui, |ui| {
                        for (i, f) in self.model.features.iter().enumerate() {
                            let tag = if i == 0 {
                                format!("#{}  base  {}", f.id, f.primitive.kind_label())
                            } else {
                                format!("#{}  {} {}", f.id, f.op.glyph(), f.primitive.kind_label())
                            };
                            if ui.selectable_label(self.selection.contains(&f.id), tag).clicked() {
                                click_select = Some(f.id);
                            }
                        }
                    });
                    if let Some(id) = click_select {
                        toggle_select(&mut self.selection, id, add);
                    }

                    ui.add_space(12.0);
                    section(ui, "VIEW");
                    ui.horizontal(|ui| {
                        if ui.button("⌖ Frame").clicked() {
                            self.frame();
                        }
                        ui.label(
                            egui::RichText::new(format!("{} tris", self.cached.tri_count()))
                                .color(theme::TEXT_MUTED)
                                .size(11.0),
                        );
                    });
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("click=select · shift=add · Del=delete · drag=orbit · scroll=zoom")
                            .color(theme::TEXT_MUTED)
                            .size(11.0),
                    );
                });
            });
    }

    fn selected_section(&mut self, ui: &mut egui::Ui) {
        if self.selection.len() != 1 {
            return;
        }
        let id = self.selection[0];
        let Some(idx) = self.model.features.iter().position(|f| f.id == id) else {
            return;
        };
        section(ui, &format!("SELECTED  #{}", id));
        let mut changed = false;
        let mut delete = false;
        {
            let f = &mut self.model.features[idx];
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("bool").color(theme::TEXT_MUTED).size(11.0));
                for op in BoolOp::ALL {
                    if ui.selectable_label(f.op == op, format!("{} {}", op.glyph(), op.label())).clicked() {
                        f.op = op;
                        changed = true;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("move u/v");
                changed |= ui.add(egui::DragValue::new(&mut f.placement.u).speed(0.05)).changed();
                changed |= ui.add(egui::DragValue::new(&mut f.placement.v).speed(0.05)).changed();
                ui.label("lift");
                changed |= ui.add(egui::DragValue::new(&mut f.placement.lift).speed(0.05)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("rotate");
                changed |= ui.add(egui::DragValue::new(&mut f.placement.spin_deg).speed(1.0).suffix("°")).changed();
            });
            match &mut f.primitive {
                Primitive::Box { w, d, h } => {
                    ui.horizontal(|ui| {
                        ui.label("size");
                        for c in [w, d, h] {
                            changed |= ui.add(egui::DragValue::new(c).speed(0.02).range(0.02..=1000.0)).changed();
                        }
                    });
                }
                Primitive::Cylinder { r, h, sides } => {
                    ui.horizontal(|ui| {
                        ui.label("r/h");
                        changed |= ui.add(egui::DragValue::new(r).speed(0.02).range(0.02..=1000.0)).changed();
                        changed |= ui.add(egui::DragValue::new(h).speed(0.02).range(0.02..=1000.0)).changed();
                        changed |= ui.add(egui::DragValue::new(sides).range(3..=128)).changed();
                    });
                }
            }
        }
        if ui
            .add_sized([ui.available_width(), 24.0], egui::Button::new(egui::RichText::new("🗑 Delete").color(theme::DANGER)))
            .clicked()
        {
            delete = true;
        }
        ui.add_space(12.0);
        if delete {
            self.model.remove(id);
            self.selection.clear();
            self.dirty = true;
        } else if changed {
            self.dirty = true;
        }
    }

    fn viewport_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::SURFACE_0))
            .show(ctx, |ui| {
                let size = ui.available_size();
                let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

                let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
                let mvp = mvp(self.yaw, self.pitch, self.dist, self.target().into(), aspect);

                self.hover_plane_pt = resp.hover_pos().and_then(|p| self.cursor_on_plane(p, rect, &mvp));

                // A click either feeds an in-flight modifier or (re)selects.
                if resp.clicked() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        if let Some(mut md) = self.modify.take() {
                            if let Some(w) = self.cursor_on_plane(pos, rect, &mvp) {
                                let plane = self.plane;
                                match md.feed(w, &plane, &mut self.model, self.card) {
                                    Feed::NeedMore => {
                                        self.status = md.prompt();
                                        self.modify = Some(md);
                                    }
                                    Feed::Applied => {
                                        self.dirty = true;
                                        self.status.clear();
                                    }
                                    Feed::AppliedContinue => {
                                        self.dirty = true;
                                        self.status = md.prompt();
                                        self.modify = Some(md);
                                    }
                                }
                            } else {
                                self.modify = Some(md); // click missed the plane; stay armed
                            }
                        } else {
                            let add = ui.input(|i| i.modifiers.shift);
                            match self.pick(pos, rect, &mvp) {
                                Some(id) => toggle_select(&mut self.selection, id, add),
                                None => {
                                    if !add {
                                        self.selection.clear();
                                    }
                                }
                            }
                        }
                    }
                }
                // Drag orbits (even mid-command, so you can reorient before a pick).
                if resp.dragged() {
                    let d = resp.drag_delta();
                    self.yaw -= d.x * 0.01;
                    self.pitch = (self.pitch + d.y * 0.01).clamp(-FRAC_PI_2 + 0.01, FRAC_PI_2 - 0.01);
                }
                if resp.hovered() {
                    let scroll = ui.input(|i| i.raw_scroll_delta.y);
                    if scroll != 0.0 {
                        self.dist = (self.dist * (1.0 - scroll * 0.0015)).clamp(0.4, 400.0);
                    }
                }

                self.recompute_if_dirty();
                let tris = mesh_verts(&self.cached);
                let mut lines = plane_grid(&self.plane);
                // selection highlights
                for id in &self.selection {
                    if let Some(f) = self.model.features.iter().find(|f| f.id == *id) {
                        let (mn, mx) = f.world_aabb();
                        aabb_lines(&mut lines, mn, mx, [0.0, 0.9, 1.0]);
                    }
                }
                // rubber-band from a gathered base/pivot to the cursor
                if let Some(md) = &self.modify {
                    if let (Some(a), Some(h)) = (md.anchor_world(&self.plane), self.hover_plane_pt) {
                        seg(&mut lines, a, h, [0.95, 0.71, 0.24]);
                    }
                }

                let renderer = self.renderer.clone();
                let cb = egui_glow::CallbackFn::new(move |info, painter| {
                    let gl = painter.gl();
                    let vp = info.viewport_in_pixels();
                    let screen = info.screen_size_px;
                    if let Ok(mut r) = renderer.lock() {
                        r.render(
                            gl, &tris, &lines, &mvp,
                            vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px,
                            screen[0] as i32, screen[1] as i32,
                        );
                    }
                });
                ui.painter().add(egui::PaintCallback { rect, callback: Arc::new(cb) });
            });
    }

    fn navigator(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("viewcube"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(112.0, 150.0), egui::Sense::click());
                let p = ui.painter();
                let c = rect.center_top() + egui::vec2(0.0, 56.0);
                let radius = 50.0;
                p.circle_filled(c, radius, egui::Color32::from_rgba_unmultiplied(0x1a, 0x24, 0x30, 220));
                p.circle_stroke(c, radius, egui::Stroke::new(1.0, theme::BORDER));
                let sq = egui::Rect::from_center_size(c, egui::vec2(34.0, 34.0));
                p.rect_filled(sq, egui::Rounding::same(6.0), theme::SURFACE_3);
                p.rect_stroke(sq, egui::Rounding::same(6.0), egui::Stroke::new(1.0, theme::BORDER));
                let f = egui::FontId::proportional(12.0);
                p.text(c, egui::Align2::CENTER_CENTER, "TOP", f.clone(), theme::TEXT_PRIMARY);
                for (t, dir) in [("N", egui::vec2(0.0, -1.0)), ("E", egui::vec2(1.0, 0.0)), ("S", egui::vec2(0.0, 1.0)), ("W", egui::vec2(-1.0, 0.0))] {
                    p.text(c + dir * (radius - 12.0), egui::Align2::CENTER_CENTER, t, f.clone(), theme::ACCENT);
                }
                let by = rect.top() + 122.0;
                let btm = egui::Rect::from_min_size(egui::pos2(c.x - 52.0, by), egui::vec2(50.0, 22.0));
                let iso = egui::Rect::from_min_size(egui::pos2(c.x + 2.0, by), egui::vec2(50.0, 22.0));
                for (r, t) in [(btm, "Bottom"), (iso, "Iso")] {
                    p.rect_filled(r, egui::Rounding::same(6.0), theme::SURFACE_2);
                    p.rect_stroke(r, egui::Rounding::same(6.0), egui::Stroke::new(1.0, theme::BORDER));
                    p.text(r.center(), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(11.0), theme::TEXT_PRIMARY);
                }
                if resp.clicked() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let preset = if btm.contains(pos) {
                            Some(ViewPreset::Bottom)
                        } else if iso.contains(pos) {
                            Some(ViewPreset::Iso)
                        } else if sq.contains(pos) {
                            Some(ViewPreset::Top)
                        } else if (pos - c).length() <= radius {
                            let d = pos - c;
                            if d.x.abs() > d.y.abs() {
                                Some(if d.x > 0.0 { ViewPreset::Right } else { ViewPreset::Left })
                            } else {
                                Some(if d.y > 0.0 { ViewPreset::Front } else { ViewPreset::Back })
                            }
                        } else {
                            None
                        };
                        if let Some(v) = preset {
                            let (yaw, pitch) = v.angles();
                            self.yaw = yaw;
                            self.pitch = pitch;
                        }
                    }
                }
                let _ = theme::WARNING;
            });
    }
}

/// Toggle/replace an id in the selection (shift = add/toggle, else replace).
fn toggle_select(sel: &mut Vec<u32>, id: u32, add: bool) {
    if add {
        if let Some(pos) = sel.iter().position(|x| *x == id) {
            sel.remove(pos);
        } else {
            sel.push(id);
        }
    } else {
        *sel = vec![id];
    }
}

fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::TEXT_MUTED).size(11.0).strong());
    ui.add_space(4.0);
}

fn xyz_row(ui: &mut egui::Ui, label: &str, v: &mut [f32; 3]) {
    ui.horizontal(|ui| {
        ui.label(label);
        for c in v.iter_mut() {
            ui.add(egui::DragValue::new(c).speed(0.02).range(0.02..=1000.0));
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Vertex assembly (CPU side)
// ─────────────────────────────────────────────────────────────────────────────
#[repr(C)]
#[derive(Clone, Copy)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
    r: f32,
    g: f32,
    b: f32,
}

fn light_dir() -> Vec3 {
    Vec3::new(0.35, 0.25, 0.9).normalize()
}

fn mesh_verts(m: &SolidMesh) -> Vec<V3> {
    let base = [0.72, 0.75, 0.80];
    let mut out = Vec::with_capacity(m.positions.len());
    for tri in m.positions.chunks_exact(3) {
        let (a, b, c) = (Vec3::from(tri[0]), Vec3::from(tri[1]), Vec3::from(tri[2]));
        let n = (b - a).cross(c - a).normalize_or_zero();
        let k = 0.35 + 0.65 * n.dot(light_dir()).abs();
        let col = [base[0] * k, base[1] * k, base[2] * k];
        for pt in [a, b, c] {
            out.push(V3 { x: pt.x, y: pt.y, z: pt.z, r: col[0], g: col[1], b: col[2] });
        }
    }
    out
}

fn plane_grid(plane: &Plane) -> Vec<V3> {
    let (u, v) = plane.axes();
    let o = plane.origin();
    let h = 6.0f32;
    let n = 12i32;
    let step = 2.0 * h / n as f32;
    let faint = [0.26, 0.30, 0.36];
    let axis_u = [0.58, 0.34, 0.34];
    let axis_v = [0.34, 0.52, 0.38];
    let mut out = Vec::new();
    for i in 0..=n {
        let t = -h + i as f32 * step;
        seg(&mut out, o + u * t - v * h, o + u * t + v * h, if t.abs() < 1e-4 { axis_v } else { faint });
        seg(&mut out, o + v * t - u * h, o + v * t + u * h, if t.abs() < 1e-4 { axis_u } else { faint });
    }
    out
}

fn aabb_lines(out: &mut Vec<V3>, mn: Vec3, mx: Vec3, c: [f32; 3]) {
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { mn.x } else { mx.x },
            if i & 2 == 0 { mn.y } else { mx.y },
            if i & 4 == 0 { mn.z } else { mx.z },
        )
    };
    for a in 0..8usize {
        for bit in [1usize, 2, 4] {
            let b = a ^ bit;
            if a < b {
                seg(out, corner(a), corner(b), c);
            }
        }
    }
}

fn seg(out: &mut Vec<V3>, a: Vec3, b: Vec3, c: [f32; 3]) {
    out.push(V3 { x: a.x, y: a.y, z: a.z, r: c[0], g: c[1], b: c[2] });
    out.push(V3 { x: b.x, y: b.y, z: b.z, r: c[0], g: c[1], b: c[2] });
}

fn mvp(yaw: f32, pitch: f32, dist: f32, target: [f32; 3], aspect: f32) -> [f32; 16] {
    let t = Vec3::from(target);
    let (cp, sp) = (pitch.cos(), pitch.sin());
    let (cy, sy) = (yaw.cos(), yaw.sin());
    let eye = t + Vec3::new(cp * cy, cp * sy, sp) * dist.max(0.1);
    let view = Mat4::look_at_rh(eye, t, Vec3::Z);
    let proj = Mat4::perspective_rh_gl(45f32.to_radians(), aspect.max(0.01), 0.05, (dist * 8.0).max(120.0));
    (proj * view).to_cols_array()
}

// ─────────────────────────────────────────────────────────────────────────────
// GL renderer — offscreen FBO, triangles + a line pass. Trimmed from
// cad_app/src/light3d.rs (proven), extended to also draw GL_LINES.
// ─────────────────────────────────────────────────────────────────────────────
const SCENE_VS: &str = r#"
    #version 330 core
    layout(location=0) in vec3 a_pos;
    layout(location=1) in vec3 a_col;
    uniform mat4 u_mvp;
    out vec3 v_col;
    void main() { gl_Position = u_mvp * vec4(a_pos, 1.0); v_col = a_col; }
"#;
const SCENE_FS: &str = r#"
    #version 330 core
    in vec3 v_col;
    out vec4 frag;
    void main() { frag = vec4(v_col, 1.0); }
"#;
const BLIT_VS: &str = r#"
    #version 330 core
    layout(location=0) in vec2 a_pos;
    layout(location=1) in vec2 a_uv;
    out vec2 v_uv;
    void main() { v_uv = a_uv; gl_Position = vec4(a_pos, 0.0, 1.0); }
"#;
const BLIT_FS: &str = r#"
    #version 330 core
    in vec2 v_uv;
    out vec4 frag;
    uniform sampler2D u_tex;
    void main() { frag = texture(u_tex, v_uv); }
"#;

#[derive(Default)]
struct SceneRenderer {
    inited: bool,
    scene_prog: Option<glow::Program>,
    u_mvp: Option<glow::UniformLocation>,
    scene_vao: Option<glow::VertexArray>,
    scene_vbo: Option<glow::Buffer>,
    blit_prog: Option<glow::Program>,
    u_tex: Option<glow::UniformLocation>,
    blit_vao: Option<glow::VertexArray>,
    blit_vbo: Option<glow::Buffer>,
    fbo: Option<glow::Framebuffer>,
    color: Option<glow::Texture>,
    depth: Option<glow::Renderbuffer>,
    fbo_w: i32,
    fbo_h: i32,
}

unsafe impl Send for SceneRenderer {}
unsafe impl Sync for SceneRenderer {}

impl SceneRenderer {
    fn ensure_init(&mut self, gl: &glow::Context) {
        if self.inited {
            return;
        }
        unsafe {
            let scene_prog = compile(gl, SCENE_VS, SCENE_FS);
            self.u_mvp = gl.get_uniform_location(scene_prog, "u_mvp");
            let svbo = gl.create_buffer().unwrap();
            let svao = gl.create_vertex_array().unwrap();
            gl.bind_vertex_array(Some(svao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(svbo));
            let stride = std::mem::size_of::<V3>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, 12);

            let blit_prog = compile(gl, BLIT_VS, BLIT_FS);
            self.u_tex = gl.get_uniform_location(blit_prog, "u_tex");
            let bvbo = gl.create_buffer().unwrap();
            let bvao = gl.create_vertex_array().unwrap();
            gl.bind_vertex_array(Some(bvao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(bvbo));
            let bstride = (4 * std::mem::size_of::<f32>()) as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, bstride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, bstride, 8);

            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            self.scene_prog = Some(scene_prog);
            self.scene_vao = Some(svao);
            self.scene_vbo = Some(svbo);
            self.blit_prog = Some(blit_prog);
            self.blit_vao = Some(bvao);
            self.blit_vbo = Some(bvbo);
            self.inited = true;
        }
    }

    unsafe fn ensure_fbo(&mut self, gl: &glow::Context, w: i32, h: i32) {
        if self.fbo.is_some() && self.fbo_w == w && self.fbo_h == h {
            return;
        }
        if let Some(f) = self.fbo.take() {
            gl.delete_framebuffer(f);
        }
        if let Some(t) = self.color.take() {
            gl.delete_texture(t);
        }
        if let Some(d) = self.depth.take() {
            gl.delete_renderbuffer(d);
        }
        let color = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(color));
        gl.tex_image_2d(
            glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w, h, 0,
            glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);

        let depth = gl.create_renderbuffer().unwrap();
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, w, h);

        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(color), 0);
        gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::RENDERBUFFER, Some(depth));

        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        self.fbo = Some(fbo);
        self.color = Some(color);
        self.depth = Some(depth);
        self.fbo_w = w;
        self.fbo_h = h;
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        gl: &glow::Context,
        tris: &[V3],
        lines: &[V3],
        mvp: &[f32; 16],
        vp_left: i32,
        vp_from_bottom: i32,
        vp_w: i32,
        vp_h: i32,
        screen_w: i32,
        screen_h: i32,
    ) {
        if vp_w <= 0 || vp_h <= 0 {
            return;
        }
        self.ensure_init(gl);
        unsafe {
            self.ensure_fbo(gl, vp_w, vp_h);
            gl.bind_framebuffer(glow::FRAMEBUFFER, self.fbo);
            gl.disable(glow::SCISSOR_TEST);
            gl.viewport(0, 0, vp_w, vp_h);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.disable(glow::BLEND);
            gl.clear_color(0.055, 0.07, 0.093, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            if let (Some(prog), Some(vao), Some(vbo)) = (self.scene_prog, self.scene_vao, self.scene_vbo) {
                gl.use_program(Some(prog));
                if let Some(loc) = &self.u_mvp {
                    gl.uniform_matrix_4_f32_slice(Some(loc), false, mvp);
                }
                gl.bind_vertex_array(Some(vao));
                if !tris.is_empty() {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(tris), glow::DYNAMIC_DRAW);
                    gl.draw_arrays(glow::TRIANGLES, 0, tris.len() as i32);
                }
                if !lines.is_empty() {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(lines), glow::DYNAMIC_DRAW);
                    gl.draw_arrays(glow::LINES, 0, lines.len() as i32);
                }
                gl.bind_vertex_array(None);
            }

            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, screen_w.max(1), screen_h.max(1));
            gl.enable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            let sw = screen_w.max(1) as f32;
            let sh = screen_h.max(1) as f32;
            let x0 = 2.0 * vp_left as f32 / sw - 1.0;
            let x1 = 2.0 * (vp_left + vp_w) as f32 / sw - 1.0;
            let y0 = 2.0 * vp_from_bottom as f32 / sh - 1.0;
            let y1 = 2.0 * (vp_from_bottom + vp_h) as f32 / sh - 1.0;
            let quad: [f32; 24] = [
                x0, y0, 0.0, 0.0, x1, y0, 1.0, 0.0, x1, y1, 1.0, 1.0,
                x0, y0, 0.0, 0.0, x1, y1, 1.0, 1.0, x0, y1, 0.0, 1.0,
            ];
            if let (Some(prog), Some(vao), Some(vbo), Some(color)) =
                (self.blit_prog, self.blit_vao, self.blit_vbo, self.color)
            {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&quad), glow::DYNAMIC_DRAW);
                gl.use_program(Some(prog));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(color));
                if let Some(loc) = &self.u_tex {
                    gl.uniform_1_i32(Some(loc), 0);
                }
                gl.bind_vertex_array(Some(vao));
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
                gl.bind_vertex_array(None);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            gl.enable(glow::BLEND);
            gl.use_program(None);
        }
    }
}

unsafe fn compile(gl: &glow::Context, vs: &str, fs: &str) -> glow::Program {
    let program = gl.create_program().expect("create_program");
    let one = |src: &str, kind: u32| -> glow::Shader {
        let s = gl.create_shader(kind).expect("create_shader");
        gl.shader_source(s, src);
        gl.compile_shader(s);
        if !gl.get_shader_compile_status(s) {
            panic!("sandbox shader compile failed:\n{}", gl.get_shader_info_log(s));
        }
        s
    };
    let v = one(vs, glow::VERTEX_SHADER);
    let f = one(fs, glow::FRAGMENT_SHADER);
    gl.attach_shader(program, v);
    gl.attach_shader(program, f);
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        panic!("sandbox program link failed:\n{}", gl.get_program_info_log(program));
    }
    gl.delete_shader(v);
    gl.delete_shader(f);
    program
}

fn bytes<T: Copy>(slice: &[T]) -> &[u8] {
    let len = std::mem::size_of_val(slice);
    unsafe { std::slice::from_raw_parts(slice.as_ptr() as *const u8, len) }
}
