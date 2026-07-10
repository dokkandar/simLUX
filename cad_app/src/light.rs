//! SIMLUX lighting integration for the CAD app.
//!
//! [`LightState`] holds the lighting scene (IES profiles, surface materials,
//! luminaires, room height, ray settings) and the last computed lux grid, and
//! draws the **Light** panel. It drives the pure-Rust `cad_light` engine on the
//! shared `cad_kernel::Document`; the app paints the resulting grid as a 2D
//! false-colour overlay on the plan (see `CadApp::paint_lux_overlay`).

use std::collections::{BTreeMap, HashMap};

use cad_light::{
    bbox, calculate_receiver, default_materials, extrude, extrude_handles_range, parse_photometry,
    CalcPlane, IesProfile, LuxGrid, Luminaire, Material, Mesh, PhotometryType, RaySettings,
    ReceiverNormal, Vertex,
};
use cad_kernel::{Document, Geom};

use crate::simlux_io::LuxBlock;

/// Human-readable name for a lux metric (receiver-normal rule), for the panel
/// readout and the `luxmetric` command echo.
pub fn metric_label(m: ReceiverNormal) -> String {
    match m {
        ReceiverNormal::Horizontal => "Horizontal (Eh)".to_string(),
        ReceiverNormal::Vertical { azimuth_deg } => format!("Vertical (Ev) @ {azimuth_deg:.0}°"),
        ReceiverNormal::Custom { x, y, z } => format!("Custom ({x:.1}, {y:.1}, {z:.1})"),
    }
}

/// Key for the always-available synthetic luminaire (works before any IES import).
pub const BUILTIN: &str = "Built-in downlight (1000 cd)";

/// A cosine (Lambertian) downlight: I(γ) = 1000·cos γ cd, axially symmetric.
fn builtin_downlight() -> IesProfile {
    let vertical_angles: Vec<f64> = (0..=18).map(|i| i as f64 * 5.0).collect();
    let candela: Vec<f64> = vertical_angles
        .iter()
        .map(|g| 1000.0 * g.to_radians().cos().max(0.0))
        .collect();
    IesProfile {
        name: BUILTIN.to_string(),
        photometry: PhotometryType::C,
        lumens: -1.0,
        multiplier: 1.0,
        vertical_angles,
        horizontal_angles: vec![0.0],
        candela: vec![candela],
        watts: 0.0,
        width: 0.0,
        length: 0.0,
        height: 0.0,
    }
}

/// Side effects the panel asks the app to run (they need `&Document` / the app's
/// insert machinery).
#[derive(Default)]
pub struct LightAction {
    pub calculate: bool,
    /// Import every dobject on this source-layer id into the room (Phase B).
    pub import_layer: Option<u32>,
    /// Drop this imported room layer.
    pub remove_layer: Option<u32>,
    /// Move the current selection onto the dedicated SIMLUX layer + use it for 3D.
    pub shift_to_simlux: bool,
    /// "Call" (insert) this luminaire block by name — arm the click-to-place flow.
    pub place_block: Option<String>,
    /// Import this IES path INTO the drawing (needs the Document, so the app does it).
    pub load_ies: Option<String>,
    /// An extrusion parameter changed — rebuild the 3D meshes (no lux recalc).
    pub rebuild_3d: bool,
}

/// One imported source layer of the room: the drafted dobjects on `layer_id`,
/// extruded to a per-layer `height` (SIMLUX layer-grouped room model — D1/D2).
/// Handle-based so the set survives redraws / re-ordering of the document.
#[derive(Clone)]
pub struct RoomLayer {
    pub layer_id: u32,
    pub name: String,
    /// Extrusion length (metres).
    pub height: f32,
    /// Base elevation the extrusion starts from (metres, Z-up). Default 0 (floor).
    pub base_z: f32,
    /// Extrude downward from `base_z` instead of up.
    pub dir_down: bool,
    pub handles: Vec<u64>,
}

impl RoomLayer {
    /// The elevation span (z0, z1) this layer extrudes between, from its base +
    /// height + direction. Passed straight to `extrude_handles_range`.
    pub fn z_range(&self) -> (f32, f32) {
        let z1 = if self.dir_down { self.base_z - self.height } else { self.base_z + self.height };
        (self.base_z, z1)
    }
}

/// All lighting UI + engine state, owned by `CadApp`.
pub struct LightState {
    /// Toggles the Light window (Tools ▸ SIMLUX Light).
    pub window_open: bool,
    /// Loaded IES profiles, keyed by name; always contains [`BUILTIN`].
    pub profiles: HashMap<String, IesProfile>,
    /// Profile used for auto-placed / new luminaires.
    pub active_profile: String,
    /// Surface materials [floor, wall, ceiling] — reflectances are editable.
    pub materials: Vec<Material>,
    /// Room (extrusion) height, metres — default height for newly imported
    /// layers and the fallback when no layer has been imported yet.
    pub room_height: f32,
    /// SIMLUX room (Phase B/C): imported source layers, each extruded to its
    /// own `height`. Empty ⇒ `calculate` falls back to extruding the whole doc.
    pub room: Vec<RoomLayer>,
    /// Work-plane height above the floor, metres (typ. 0.8 m desk height).
    pub plane_height: f32,
    /// Target grid cell size, metres (clamped to 8..64 cells per axis).
    pub cell_size: f32,
    /// Ray-tracer controls.
    pub settings: RaySettings,
    /// The illuminance metric: which receiver-normal rule the calc measures the
    /// (single) light field onto — horizontal work-plane, a vertical facing, or a
    /// custom direction. See `SIMLUX_CALC_ENGINE_PLAN` §4.
    pub metric: ReceiverNormal,
    /// LUX-block registry: block DEFINITION name → its luminaire descriptor
    /// (many IES linked, one active). A block is a luminaire iff it is a key
    /// here. Fixtures are then DERIVED from the block instances in the drawing.
    pub lux_blocks: BTreeMap<String, LuxBlock>,
    /// Manually placed luminaires (P4). Combined with the LUX-block-derived
    /// fixtures at calc time; if BOTH are empty ⇒ auto-place one at room centre.
    pub luminaires: Vec<Luminaire>,
    pub auto_center_light: bool,
    /// When set, canvas clicks drop a luminaire (P4 placement mode).
    pub place_mode: bool,
    /// Monotonic id source for placed luminaires.
    pub next_id: u32,
    /// Mounting height for newly placed fixtures (defaults to room height).
    pub mount_height: f32,
    /// Last computed grid + its plane + extruded scene.
    pub grid: Option<LuxGrid>,
    pub plane: Option<CalcPlane>,
    pub meshes: Vec<Mesh>,
    /// Paint the false-colour overlay on the 2D plan.
    pub show_overlay: bool,
    /// Fixed scale ceiling for the colour map (None ⇒ auto = grid max).
    pub scale_max: Option<f64>,
    /// IES file path typed into the panel.
    pub ies_path: String,
    /// Status / result line.
    pub last_msg: String,

    // ---- 3D viewport (P2) -------------------------------------------------
    /// Show the docked 3D viewport panel.
    pub view3d_open: bool,
    /// SIMLUX workspace mode — a persistent half-screen 2D | 3D split. The 3D
    /// panel is force-shown at ~half the window width and tracks the 2D drawing
    /// LIVE (extrudes the current room every frame, no Calculate needed).
    pub simlux_mode: bool,
    /// One-shot: fit the orbit camera the next time live meshes rebuild (set
    /// when the workspace is entered so the drawing is framed on arrival).
    pub simlux_fit_pending: bool,
    /// Orbit camera: yaw + pitch (radians), distance (m), target (world, Z-up).
    pub cam_yaw: f32,
    pub cam_pitch: f32,
    pub cam_dist: f32,
    pub cam_target: [f32; 3],
    /// Paint the lux heatmap on the 3D floor (P3) rather than the floor material.
    pub floor_heatmap: bool,
}

impl Default for LightState {
    fn default() -> Self {
        Self::new()
    }
}

impl LightState {
    pub fn new() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(BUILTIN.to_string(), builtin_downlight());
        Self {
            window_open: false,
            profiles,
            active_profile: BUILTIN.to_string(),
            materials: default_materials(),
            room_height: 3.0,
            room: Vec::new(),
            plane_height: 0.8,
            cell_size: 0.25,
            settings: RaySettings::default(),
            metric: ReceiverNormal::Horizontal,
            lux_blocks: BTreeMap::new(),
            luminaires: Vec::new(),
            auto_center_light: true,
            place_mode: false,
            next_id: 1,
            mount_height: 3.0,
            grid: None,
            plane: None,
            meshes: Vec::new(),
            show_overlay: true,
            scale_max: None,
            ies_path: String::new(),
            last_msg: "Draw a room, set the height, then Calculate.".to_string(),
            view3d_open: false,
            simlux_mode: false,
            simlux_fit_pending: false,
            cam_yaw: 0.7,
            cam_pitch: 0.6,
            cam_dist: 10.0,
            cam_target: [0.0, 0.0, 1.5],
            floor_heatmap: true,
        }
    }

    /// Colour-map ceiling: user override, else the current grid's max.
    pub fn scale_ceiling(&self) -> f64 {
        self.scale_max
            .or_else(|| self.grid.as_ref().map(|g| g.max))
            .unwrap_or(1.0)
            .max(1e-3)
    }

    // IES import is handled by `CadApp::import_ies_into_drawing` (it needs the
    // Document to embed the raw file). The panel signals it via `LightAction`.

    /// Drop a luminaire at plan position (x, y) on the mounting plane.
    pub fn add_luminaire_at(&mut self, x: f32, y: f32) {
        let id = self.next_id;
        self.next_id += 1;
        self.luminaires.push(Luminaire {
            id,
            profile: self.active_profile.clone(),
            position: Vertex::new(x, y, self.mount_height),
            rotation_deg: 0.0,
            dimming: 1.0,
        });
        self.last_msg = format!("Placed fixture #{id} at ({x:.2}, {y:.2}) — press Calculate.");
    }

    /// Import (Phase B) every drafted dobject on `layer_id` into the room, at
    /// the current default height. Re-importing the same layer refreshes its
    /// handle set and keeps its chosen height.
    pub fn import_layer(&mut self, doc: &Document, layer_id: u32) {
        let handles: Vec<u64> = doc.dobjects.iter()
            .filter(|d| d.style.layer == layer_id)
            .map(|d| d.handle)
            .collect();
        let name = doc.layers.get(layer_id)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| format!("layer {layer_id}"));
        let n = handles.len();
        if let Some(g) = self.room.iter_mut().find(|g| g.layer_id == layer_id) {
            g.handles = handles;
            g.name = name.clone();
        } else {
            self.room.push(RoomLayer { layer_id, name: name.clone(), height: self.room_height, base_z: 0.0, dir_down: false, handles });
        }
        self.last_msg =
            format!("Imported {n} object(s) from layer '{name}' — set height, then Calculate.");
    }

    /// Drop one imported room layer (Phase B).
    pub fn remove_room_layer(&mut self, layer_id: u32) {
        self.room.retain(|g| g.layer_id != layer_id);
    }

    /// Every handle across all imported room layers (for plan highlight / count).
    pub fn room_handles(&self) -> Vec<u64> {
        self.room.iter().flat_map(|g| g.handles.iter().copied()).collect()
    }

    /// Fixtures DERIVED from the drawing (Slice 4): every `BlockRef` whose
    /// definition is a registered LUX block contributes a `Luminaire`, posed at
    /// the instance and using that block's ACTIVE IES. Placing luminaire blocks
    /// on the plan IS placing the fixtures — no separate list to keep in sync.
    pub fn derived_luminaires(&self, doc: &Document) -> Vec<Luminaire> {
        let mut out = Vec::new();
        let mut id = 10_000; // keep clear of manually-placed fixture ids
        for d in &doc.dobjects {
            let Geom::BlockRef(br) = &d.geom else { continue };
            let Some(def) = doc.blocks.get(br.block) else { continue };
            let Some(lux) = self.lux_blocks.get(&def.name) else { continue };
            let Some(ies) = lux.active_ies() else { continue };
            id += 1;
            out.push(Luminaire {
                id,
                profile: ies.clone(),
                position: Vertex::new(br.insert.x as f32, br.insert.y as f32, self.mount_height),
                rotation_deg: br.rotation.to_degrees() as f32,
                dimming: 1.0,
            });
        }
        out
    }

    /// Count of LUX-block instances in the drawing that resolve to an active IES.
    pub fn derived_fixture_count(&self, doc: &Document) -> usize {
        self.derived_luminaires(doc).len()
    }

    /// Run the lux engine on `doc` and store the grid + plane + scene.
    pub fn calculate(&mut self, doc: &Document) {
        let Some((min_x, min_y, max_x, max_y)) = bbox(doc) else {
            self.grid = None;
            self.plane = None;
            self.last_msg = "No geometry — draw walls / a closed room first.".to_string();
            return;
        };
        let (w, d) = ((max_x - min_x).max(1e-3), (max_y - min_y).max(1e-3));
        let cols = ((w / self.cell_size).round() as u32).clamp(8, 64);
        let rows = ((d / self.cell_size).round() as u32).clamp(8, 64);
        let plane = CalcPlane {
            origin: Vertex::new(min_x, min_y, self.plane_height),
            width: w,
            depth: d,
            cols,
            rows,
        };
        // Phase C: build the room from imported per-layer groups (each at its
        // own height); fall back to extruding the whole document when nothing
        // has been imported yet, so the legacy one-click flow still works.
        let meshes = if self.room.is_empty() {
            extrude(doc, self.room_height)
        } else {
            let mut m = Vec::new();
            for g in &self.room {
                let (z0, z1) = g.z_range();
                m.extend(extrude_handles_range(doc, &g.handles, z0, z1));
            }
            m
        };
        // Fixtures come from LUX-block instances in the drawing (derived) PLUS
        // any manually placed luminaires; only when BOTH are empty do we
        // auto-place one at the room centre so the legacy one-click flow works.
        let mut lums = self.derived_luminaires(doc);
        lums.extend(self.luminaires.iter().cloned());
        if lums.is_empty() && self.auto_center_light {
            lums.push(Luminaire {
                id: 1,
                profile: self.active_profile.clone(),
                position: Vertex::new(0.5 * (min_x + max_x), 0.5 * (min_y + max_y), self.room_height),
                rotation_deg: 0.0,
                dimming: 1.0,
            });
        }
        let grid = calculate_receiver(
            &meshes,
            &lums,
            &self.profiles,
            &self.materials,
            &plane,
            &self.settings,
            self.metric,
        );
        self.last_msg = format!(
            "{} · {}×{} grid · avg {:.0} · min {:.0} · max {:.0} lx",
            metric_label(self.metric), cols, rows, grid.avg, grid.min, grid.max
        );
        self.grid = Some(grid);
        self.plane = Some(plane);
        self.meshes = meshes;
        self.show_overlay = true;

        // Fit the orbit camera to the room.
        self.cam_target = [0.5 * (min_x + max_x), 0.5 * (min_y + max_y), 0.5 * self.room_height];
        let diag = (w * w + d * d + self.room_height * self.room_height).sqrt();
        self.cam_dist = (diag * 1.3).max(3.0);
    }

    /// SIMLUX workspace live sync: extrude the current room (imported per-layer
    /// groups, else the whole document) into `meshes` WITHOUT running the lux
    /// calc, so the right-hand 3D view tracks whatever is drawn/imported on the
    /// left 2D plan. Cheap (geometry only). Fits the orbit camera ONCE, the
    /// first frame after the workspace is entered (`simlux_fit_pending`).
    pub fn rebuild_live_meshes(&mut self, doc: &Document) {
        self.meshes = if self.room.is_empty() {
            extrude(doc, self.room_height)
        } else {
            let mut m = Vec::new();
            for g in &self.room {
                let (z0, z1) = g.z_range();
                m.extend(extrude_handles_range(doc, &g.handles, z0, z1));
            }
            m
        };
        if self.simlux_fit_pending {
            if let Some((min_x, min_y, max_x, max_y)) = bbox(doc) {
                let (w, d) = ((max_x - min_x).max(1e-3), (max_y - min_y).max(1e-3));
                self.cam_target =
                    [0.5 * (min_x + max_x), 0.5 * (min_y + max_y), 0.5 * self.room_height];
                let diag = (w * w + d * d + self.room_height * self.room_height).sqrt();
                self.cam_dist = (diag * 1.3).max(3.0);
                self.simlux_fit_pending = false;
            }
        }
    }

    /// Snapshot the SIMLUX-side state into a serialisable sidecar config,
    /// keyed by STABLE NAMES (layer name, profile name) so it round-trips a
    /// save/reopen. The built-in synthetic downlight is NOT persisted (it is
    /// regenerated in `new`).
    pub fn to_config(&self, doc: &Document) -> crate::simlux_io::SimluxConfig {
        use std::collections::BTreeMap;
        let mut layers_3d = BTreeMap::new();
        let mut layers_3d_base = BTreeMap::new();
        let mut layers_3d_down = BTreeMap::new();
        for g in &self.room {
            let name = doc
                .layers
                .get(g.layer_id)
                .map(|l| l.name.clone())
                .unwrap_or_else(|| g.name.clone());
            layers_3d.insert(name.clone(), g.height);
            layers_3d_base.insert(name.clone(), g.base_z);
            layers_3d_down.insert(name, g.dir_down);
        }
        let mut ies_library = BTreeMap::new();
        for (k, v) in &self.profiles {
            if k != BUILTIN {
                ies_library.insert(k.clone(), v.clone());
            }
        }
        crate::simlux_io::SimluxConfig {
            layers_3d,
            layers_3d_base,
            layers_3d_down,
            ies_library,
            active_profile: self.active_profile.clone(),
            lux_blocks: self.lux_blocks.clone(),
            materials: self.materials.clone(),
            settings: self.settings,
            room_height: self.room_height,
            plane_height: self.plane_height,
            cell_size: self.cell_size,
        }
    }

    /// Apply a loaded sidecar config onto the current document — merge the IES
    /// library, restore materials/settings/defaults, and rebuild the room by
    /// resolving persisted layer NAMES back to ids + their current handles.
    pub fn apply_config(&mut self, cfg: crate::simlux_io::SimluxConfig, doc: &Document) {
        for (k, v) in cfg.ies_library {
            self.profiles.insert(k, v);
        }
        if self.profiles.contains_key(&cfg.active_profile) {
            self.active_profile = cfg.active_profile;
        }
        if !cfg.materials.is_empty() {
            self.materials = cfg.materials;
        }
        self.settings = cfg.settings;
        self.lux_blocks = cfg.lux_blocks;
        if cfg.room_height > 0.0 {
            self.room_height = cfg.room_height;
        }
        if cfg.plane_height > 0.0 {
            self.plane_height = cfg.plane_height;
        }
        if cfg.cell_size > 0.0 {
            self.cell_size = cfg.cell_size;
        }
        self.room.clear();
        for (name, height) in cfg.layers_3d {
            if let Some(lid) = doc.layers.find(&name) {
                let handles: Vec<u64> = doc
                    .dobjects
                    .iter()
                    .filter(|d| d.style.layer == lid)
                    .map(|d| d.handle)
                    .collect();
                let base_z = cfg.layers_3d_base.get(&name).copied().unwrap_or(0.0);
                let dir_down = cfg.layers_3d_down.get(&name).copied().unwrap_or(false);
                self.room.push(RoomLayer { layer_id: lid, name, height, base_z, dir_down, handles });
            }
        }
    }

    /// Populate the runtime IES library from the drawing's embedded IES files
    /// (RSM v8 — IES is kept in the drawing DB). Called after opening a drawing;
    /// these win over any legacy sidecar copy of the same name.
    pub fn load_embedded_ies(&mut self, doc: &Document) {
        for f in &doc.ies_files {
            match parse_photometry(&f.data) {
                Ok(mut prof) => {
                    prof.name = f.name.clone();
                    self.profiles.insert(f.name.clone(), prof);
                }
                Err(e) => self.last_msg = format!("embedded photometry '{}' parse error: {e}", f.name),
            }
        }
    }

    /// Draw the panel body. Returns actions the app must run (they need `&Document`).
    pub fn panel_ui(&mut self, ui: &mut egui::Ui, layers: &[(u32, String)]) -> LightAction {
        let mut action = LightAction::default();
        ui.set_min_width(260.0);

        // ---- ① Room — mark layers "use for 3D"; each extrudes to its height ----
        ui.label(egui::RichText::new("① Room  ·  use layers for 3D").strong());
        ui.label(
            egui::RichText::new("Tick the layers that form the room.")
                .small()
                .weak(),
        );
        if ui
            .button("⬚  Move selection → SIMLUX layer")
            .on_hover_text("Put the selected geometry on a dedicated SIMLUX layer and use it for 3D")
            .clicked()
        {
            action.shift_to_simlux = true;
        }
        egui::Grid::new("simlux_layer_use3d")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for (id, name) in layers {
                    let group = self.room.iter().find(|g| g.layer_id == *id);
                    let mut on = group.is_some();
                    let n = group.map(|g| g.handles.len()).unwrap_or(0);
                    if ui
                        .checkbox(&mut on, name.as_str())
                        .on_hover_text("Use this layer's geometry in the 3D model / lux calc")
                        .changed()
                    {
                        if on {
                            action.import_layer = Some(*id);
                        } else {
                            action.remove_layer = Some(*id);
                        }
                    }
                    ui.label(
                        egui::RichText::new(if on { format!("{n} obj") } else { String::new() })
                            .small()
                            .weak(),
                    );
                    ui.end_row();
                }
            });
        if self.room.is_empty() {
            ui.label(
                egui::RichText::new("No layers imported → Calculate extrudes the whole drawing.")
                    .small()
                    .weak(),
            );
        } else {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("② Extrude  ·  base + height + direction").strong());
            let mut remove_layer: Option<u32> = None;
            let mut changed = false;
            for g in &mut self.room {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&g.name).strong());
                        ui.label(egui::RichText::new(format!("{} obj", g.handles.len())).small().weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").on_hover_text("Remove from room").clicked() {
                                remove_layer = Some(g.layer_id);
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("base");
                        changed |= ui.add(egui::DragValue::new(&mut g.base_z)
                            .speed(0.05).suffix(" m").range(-50.0..=200.0)).changed();
                        ui.label("height");
                        changed |= ui.add(egui::DragValue::new(&mut g.height)
                            .speed(0.05).suffix(" m").range(0.01..=200.0)).changed();
                        let dir = if g.dir_down { "↓ down" } else { "↑ up" };
                        if ui.selectable_label(false, dir)
                            .on_hover_text("Toggle extrude direction (up / down from base)")
                            .clicked()
                        {
                            g.dir_down = !g.dir_down;
                            changed = true;
                        }
                    });
                    let (z0, z1) = g.z_range();
                    ui.label(egui::RichText::new(
                        format!("→ z {:.2} … {:.2} m", z0.min(z1), z0.max(z1))).small().weak());
                });
            }
            if let Some(id) = remove_layer { action.remove_layer = Some(id); }
            if changed { action.rebuild_3d = true; }
        }
        ui.separator();

        // ---- Luminaire / IES --------------------------------------------
        ui.label(egui::RichText::new("Luminaire").strong());
        let mut keys: Vec<String> = self.profiles.keys().cloned().collect();
        keys.sort();
        egui::ComboBox::from_label("Photometry")
            .selected_text(self.active_profile.clone())
            .show_ui(ui, |ui| {
                for k in &keys {
                    ui.selectable_value(&mut self.active_profile, k.clone(), k.as_str());
                }
            });
        ui.horizontal(|ui| {
            ui.label("IES:");
            ui.add(
                egui::TextEdit::singleline(&mut self.ies_path)
                    .desired_width(150.0)
                    .hint_text(r"C:\path\to\file.ies"),
            );
            if ui.button("Load").clicked() {
                action.load_ies = Some(self.ies_path.trim().to_string());
            }
        });
        ui.checkbox(&mut self.auto_center_light, "Auto-place one at room centre if none placed");

        ui.separator();

        // ---- Luminaire blocks (LUX blocks: many IES linked, one active) --
        ui.label(egui::RichText::new("Luminaire blocks").strong());
        ui.label(
            egui::RichText::new(
                "Blocks ticked ‘Luminaire (IES)’ in the Block dialog. Link IES options; the ◉ active one drives calc + render.",
            )
            .small()
            .weak(),
        );
        if self.lux_blocks.is_empty() {
            ui.label(
                egui::RichText::new("None yet — make a block and tick ‘Luminaire (IES)’.")
                    .small()
                    .weak(),
            );
        } else {
            let mut profile_keys: Vec<String> = self.profiles.keys().cloned().collect();
            profile_keys.sort();
            for (bname, lux) in self.lux_blocks.iter_mut() {
                let mut set_active: Option<String> = None;
                let mut remove_opt: Option<String> = None;
                let mut add_opt: Option<String> = None;
                let mut place_this = false;
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("💡 {bname}")).strong());
                        if ui
                            .small_button("＋ Place")
                            .on_hover_text("Insert this luminaire on the plan (click to place)")
                            .clicked()
                        {
                            place_this = true;
                        }
                    });
                    if lux.ies_options.is_empty() {
                        ui.label(egui::RichText::new("no IES linked").small().weak());
                    }
                    for opt in &lux.ies_options {
                        ui.horizontal(|ui| {
                            let active = lux.active.as_deref() == Some(opt.as_str());
                            if ui.radio(active, opt.as_str()).clicked() {
                                set_active = Some(opt.clone());
                            }
                            if ui.small_button("✕").on_hover_text("Unlink this IES").clicked() {
                                remove_opt = Some(opt.clone());
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("Link IES:");
                        egui::ComboBox::from_id_source(format!("luxlink_{bname}"))
                            .selected_text("add…")
                            .show_ui(ui, |ui| {
                                for k in &profile_keys {
                                    if !lux.ies_options.contains(k)
                                        && ui.selectable_label(false, k.as_str()).clicked()
                                    {
                                        add_opt = Some(k.clone());
                                    }
                                }
                            });
                    });
                });
                if place_this {
                    action.place_block = Some(bname.clone());
                }
                if let Some(o) = add_opt {
                    lux.ies_options.push(o.clone());
                    if lux.active.is_none() {
                        lux.active = Some(o); // first linked option becomes active
                    }
                }
                if let Some(o) = set_active {
                    lux.active = Some(o);
                }
                if let Some(o) = remove_opt {
                    lux.ies_options.retain(|x| x != &o);
                    if lux.active.as_deref() == Some(o.as_str()) {
                        lux.active = lux.ies_options.first().cloned();
                    }
                }
            }
        }

        ui.separator();

        // ---- Fixtures (P4 placement) ------------------------------------
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Fixtures").strong());
            ui.label(egui::RichText::new(format!("({})", self.luminaires.len())).weak());
        });
        let place_label = if self.place_mode { "◉ Placing… click the plan" } else { "＋ Place on plan" };
        if ui.selectable_label(self.place_mode, place_label)
            .on_hover_text("Toggle, then click points on the 2D plan to drop fixtures. Esc / untoggle to stop.")
            .clicked()
        {
            self.place_mode = !self.place_mode;
        }
        ui.add(egui::Slider::new(&mut self.mount_height, 0.0..=8.0).text("Mount height (m)"));
        if !self.luminaires.is_empty() {
            let mut remove: Option<usize> = None;
            egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                for (i, l) in self.luminaires.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("#{}  ({:.1}, {:.1}, {:.1})", l.id, l.position.x, l.position.y, l.position.z));
                        if ui.small_button("✕").clicked() {
                            remove = Some(i);
                        }
                        ui.add(egui::Slider::new(&mut l.dimming, 0.0..=1.0).text("dim"));
                    });
                }
            });
            if let Some(i) = remove {
                self.luminaires.remove(i);
            }
            if ui.button("Clear all fixtures").clicked() {
                self.luminaires.clear();
            }
        }

        ui.separator();

        // ---- Room -------------------------------------------------------
        ui.label(egui::RichText::new("Room").strong());
        ui.add(egui::Slider::new(&mut self.room_height, 2.0..=8.0).text("Height (m)"));
        ui.add(egui::Slider::new(&mut self.plane_height, 0.0..=2.0).text("Work plane (m)"));
        ui.add(egui::Slider::new(&mut self.cell_size, 0.1..=1.0).text("Grid cell (m)"));

        ui.separator();

        // ---- Materials --------------------------------------------------
        ui.label(egui::RichText::new("Reflectances").strong());
        for m in &mut self.materials {
            let name = m.name.clone();
            ui.add(egui::Slider::new(&mut m.reflectance, 0.0..=1.0).text(name));
        }

        ui.separator();

        // ---- Quality ----------------------------------------------------
        ui.collapsing("Quality", |ui| {
            ui.add(egui::Slider::new(&mut self.settings.max_bounces, 0..=3).text("Indirect bounces"));
            let mut rays = self.settings.rays_per_point as i32;
            if ui.add(egui::Slider::new(&mut rays, 8..=256).text("Rays / point")).changed() {
                self.settings.rays_per_point = rays.max(1) as u32;
            }
            ui.checkbox(&mut self.settings.shadows, "Cast shadows");
        });

        ui.separator();

        // ---- Metric (which receiver-normal rule the field is measured onto) ---
        ui.label(egui::RichText::new("Metric  ·  illuminance direction").strong());
        let metric_before = self.metric;
        egui::ComboBox::from_id_source("simlux_metric")
            .selected_text(metric_label(self.metric))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(matches!(self.metric, ReceiverNormal::Horizontal), "Horizontal (Eh) — work plane")
                    .clicked()
                {
                    self.metric = ReceiverNormal::Horizontal;
                }
                if ui
                    .selectable_label(matches!(self.metric, ReceiverNormal::Vertical { .. }), "Vertical (Ev) — walls / faces")
                    .clicked()
                    && !matches!(self.metric, ReceiverNormal::Vertical { .. })
                {
                    self.metric = ReceiverNormal::Vertical { azimuth_deg: 0.0 };
                }
                if ui
                    .selectable_label(matches!(self.metric, ReceiverNormal::Custom { .. }), "Custom direction")
                    .clicked()
                    && !matches!(self.metric, ReceiverNormal::Custom { .. })
                {
                    self.metric = ReceiverNormal::Custom { x: 1.0, y: 0.0, z: 1.0 };
                }
            });
        match &mut self.metric {
            ReceiverNormal::Vertical { azimuth_deg } => {
                ui.add(
                    egui::DragValue::new(azimuth_deg)
                        .speed(1.0)
                        .range(0.0..=360.0)
                        .suffix("°")
                        .prefix("azimuth "),
                );
            }
            ReceiverNormal::Custom { x, y, z } => {
                ui.horizontal(|ui| {
                    ui.label("normal");
                    ui.add(egui::DragValue::new(x).speed(0.1).prefix("x "));
                    ui.add(egui::DragValue::new(y).speed(0.1).prefix("y "));
                    ui.add(egui::DragValue::new(z).speed(0.1).prefix("z "));
                });
            }
            ReceiverNormal::Horizontal => {}
        }
        ui.label(
            egui::RichText::new("Also: type  luxmetric vertical 90  in the command line.")
                .small()
                .weak(),
        );
        // Switching metric with a grid already computed re-runs immediately so the
        // 2D overlay updates live (the calc is sub-second at these grid sizes).
        if metric_before != self.metric && self.grid.is_some() {
            action.calculate = true;
        }

        ui.separator();

        // ---- Calculate --------------------------------------------------
        if ui
            .add(egui::Button::new(egui::RichText::new("  Calculate  ").strong()))
            .clicked()
        {
            action.calculate = true;
        }
        ui.checkbox(&mut self.show_overlay, "Show lux overlay on 2D plan");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.view3d_open, "3D view");
            ui.checkbox(&mut self.floor_heatmap, "Heatmap floor");
        });

        // ---- Colour scale -----------------------------------------------
        ui.horizontal(|ui| {
            let mut auto = self.scale_max.is_none();
            if ui.checkbox(&mut auto, "Auto scale").changed() {
                self.scale_max = if auto {
                    None
                } else {
                    Some(self.grid.as_ref().map(|g| g.max).unwrap_or(500.0).max(1.0))
                };
            }
            if let Some(m) = &mut self.scale_max {
                ui.add(
                    egui::DragValue::new(m)
                        .speed(10.0)
                        .suffix(" lx")
                        .range(1.0..=100_000.0),
                );
            }
        });

        // ---- Results ----------------------------------------------------
        if let Some(g) = &self.grid {
            ui.separator();
            let uo = if g.avg > 0.0 { g.min / g.avg } else { 0.0 };
            ui.label(egui::RichText::new(metric_label(self.metric)).strong());
            ui.label(format!("Average   {:.0} lx", g.avg));
            ui.label(format!("Min / Max   {:.0} / {:.0} lx", g.min, g.max));
            ui.label(format!("Uniformity Uo (min/avg)   {:.2}", uo));
            legend_bar(ui, self.scale_ceiling());
        }

        ui.add_space(4.0);
        ui.label(egui::RichText::new(&self.last_msg).small().italics());
        action
    }
}

/// Five-stop false-colour ramp (low→high). `t` is clamped to 0..1.
pub fn lux_color(t: f32) -> egui::Color32 {
    const STOPS: [(f32, [u8; 3]); 5] = [
        (0.00, [20, 24, 82]),    // deep blue
        (0.25, [34, 116, 204]),  // blue
        (0.50, [40, 190, 120]),  // green
        (0.75, [240, 214, 72]),  // yellow
        (1.00, [226, 72, 46]),   // red
    ];
    let t = t.clamp(0.0, 1.0);
    let (mut lo, mut hi) = (STOPS[0], STOPS[STOPS.len() - 1]);
    for w in STOPS.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            lo = w[0];
            hi = w[1];
            break;
        }
    }
    let span = (hi.0 - lo.0).max(1e-6);
    let f = (t - lo.0) / span;
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
    egui::Color32::from_rgb(lerp(lo.1[0], hi.1[0]), lerp(lo.1[1], hi.1[1]), lerp(lo.1[2], hi.1[2]))
}

/// The same false-colour ramp as [`lux_color`], as float RGB (0..1) for the
/// 3D floor heatmap. `fn(f32) -> (f32, f32, f32)` so it can be passed as a
/// plain function pointer into the 3D vertex builder.
pub fn lux_rgb(t: f32) -> (f32, f32, f32) {
    let c = lux_color(t);
    (c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0)
}

/// A horizontal gradient legend from 0 to `max` lux.
pub fn legend_bar(ui: &mut egui::Ui, max: f64) {
    let (resp, painter) = ui.allocate_painter(egui::vec2(240.0, 16.0), egui::Sense::hover());
    let rect = resp.rect;
    let n = 64;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let x0 = rect.left() + rect.width() * (i as f32 / n as f32);
        let x1 = rect.left() + rect.width() * ((i + 1) as f32 / n as f32);
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom())),
            0.0,
            lux_color(t),
        );
    }
    ui.horizontal(|ui| {
        ui.label("0");
        ui.add_space(180.0);
        ui.label(format!("{max:.0} lx"));
    });
}
