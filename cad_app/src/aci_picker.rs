// Polar ACI color picker — the AutoRasm-style concentric wheel.
//
// 256 ACI colors laid out on a deterministic polar grid:
//   * center circle = brightest ACI (luminance pick from the full table)
//   * concentric rings outward, packed with as many fixed-radius circles
//     as fit at each radius (constant gap)
//   * per-ring colors angularly sorted by HSL hue so red sits near 0°,
//     green near 210°, blue near 120°, etc.
//
// The mapping (position → ACI index) is user-permutable via a "Swap"
// mode and can be persisted to JSON. Layout positions never change;
// only the ACI assignments at each slot do.
//
// Spec reference: ~/workspace/RUST_CAD/ACI_Picker_UI.html (HTML mockup).

use cad_kernel::color::aci_palette;
use eframe::egui;

// ---- layout constants — keep in lock-step with the HTML reference -------
const CIRCLE_RADIUS:   f32 = 8.0;
const RADIAL_GAP:      f32 = 3.0;
const TANGENTIAL_GAP:  f32 = 3.0;

#[derive(Clone, Copy)]
struct Slot {
    /// Position in widget-local coords (relative to the wheel center).
    dx: f32,
    dy: f32,
}

/// Shared state for the picker: precomputed slot positions + the live
/// permutation from slot index → ACI byte. Owned by `App`.
pub struct AciPickerState {
    /// Slot positions in widget-local coords. Length is exactly 256
    /// (one slot per ACI index). slot 0 = center.
    slots: Vec<Slot>,
    /// position → ACI byte. `mapping[slot_idx]` is the ACI shown at that
    /// slot. The factory default is computed once and re-used by Reset.
    pub mapping:           [u8; 256],
    default_mapping:       [u8; 256],
    /// Swap-mode lets the user click two slots to swap the ACI bytes at
    /// those positions (used to tune the wheel to taste; persisted via
    /// `save_mapping`).
    pub swap_mode:         bool,
    /// First slot picked in swap mode; None means awaiting the first.
    selected_for_swap:     Option<usize>,
    /// Hovered slot index, recomputed each frame for hover readout.
    pub hovered:           Option<usize>,
}

impl Default for AciPickerState {
    fn default() -> Self {
        let (slots, default_mapping) = build_layout_and_default_mapping();
        Self {
            slots,
            mapping: default_mapping,
            default_mapping,
            swap_mode: false,
            selected_for_swap: None,
            hovered: None,
        }
    }
}

impl AciPickerState {
    /// Try to replace the current mapping with one persisted to disk.
    /// Silent no-op if the file is missing or unparseable — the picker
    /// falls back to the deterministic default layout.
    pub fn try_load_mapping(&mut self, path: &std::path::Path) {
        let Ok(bytes) = std::fs::read(path) else { return };
        let Ok(text) = std::str::from_utf8(&bytes) else { return };
        // Minimal JSON-array-of-ints parser; the file is just `[n, n, ...]`.
        let trimmed = text.trim().trim_start_matches('[').trim_end_matches(']');
        let mut out = [0_u8; 256];
        let mut seen = 0_usize;
        for tok in trimmed.split(',') {
            if seen >= 256 { break; }
            let Ok(n) = tok.trim().parse::<u16>() else { return };
            if n > 255 { return; }
            out[seen] = n as u8;
            seen += 1;
        }
        if seen == 256 { self.mapping = out; }
    }

    /// Persist current mapping as a JSON array `[n, n, ...]`.
    pub fn save_mapping(&self, path: &std::path::Path) -> std::io::Result<()> {
        let mut s = String::with_capacity(256 * 4 + 4);
        s.push('[');
        for (i, v) in self.mapping.iter().enumerate() {
            if i > 0 { s.push(','); }
            s.push_str(&v.to_string());
        }
        s.push(']');
        std::fs::write(path, s)
    }

    pub fn reset_to_default(&mut self) {
        self.mapping = self.default_mapping;
        self.selected_for_swap = None;
    }

    /// Render the wheel inside the given UI. Returns `Some(aci)` only
    /// on the frame the user *commits* a pick (click in pick mode);
    /// returns `None` while swapping, hovering, or idle.
    pub fn wheel_ui(&mut self, ui: &mut egui::Ui) -> Option<u8> {
        // Allocate a square area large enough for all rings + padding.
        let size = self.required_size();
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(size, size), egui::Sense::click()
        );
        let painter = ui.painter_at(rect);
        let center = rect.center();

        // Backdrop — soft cream like the HTML mockup, but kept neutral
        // so it works on both light and dark themes.
        painter.rect_filled(rect, rect.width() * 0.5,
            egui::Color32::from_rgb(40, 44, 52));

        // Hover detection.
        self.hovered = None;
        if let Some(p) = resp.hover_pos() {
            let r2 = CIRCLE_RADIUS * CIRCLE_RADIUS;
            for (i, slot) in self.slots.iter().enumerate() {
                let cx = center.x + slot.dx;
                let cy = center.y + slot.dy;
                let dx = p.x - cx; let dy = p.y - cy;
                if dx * dx + dy * dy <= r2 {
                    self.hovered = Some(i);
                    break;
                }
            }
        }

        // Paint each slot.
        for (i, slot) in self.slots.iter().enumerate() {
            let (r, g, b) = aci_palette(self.mapping[i]);
            let pos = egui::pos2(center.x + slot.dx, center.y + slot.dy);
            painter.circle_filled(pos, CIRCLE_RADIUS,
                egui::Color32::from_rgb(r, g, b));
            painter.circle_stroke(pos, CIRCLE_RADIUS,
                egui::Stroke::new(0.6, egui::Color32::from_rgb(110, 120, 135)));
        }

        // Highlight selected (swap mode) — orange ring.
        if self.swap_mode {
            if let Some(i) = self.selected_for_swap {
                if let Some(slot) = self.slots.get(i) {
                    let pos = egui::pos2(center.x + slot.dx, center.y + slot.dy);
                    painter.circle_stroke(pos, CIRCLE_RADIUS + 3.5,
                        egui::Stroke::new(2.5, egui::Color32::from_rgb(245, 165, 35)));
                }
            }
        }
        // Highlight hover — pale ring.
        if let Some(i) = self.hovered {
            if let Some(slot) = self.slots.get(i) {
                let pos = egui::pos2(center.x + slot.dx, center.y + slot.dy);
                painter.circle_stroke(pos, CIRCLE_RADIUS + 2.0,
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(230, 235, 245)));
            }
        }

        // Click handling.
        if resp.clicked() {
            if let Some(i) = self.hovered {
                if self.swap_mode {
                    match self.selected_for_swap {
                        None    => { self.selected_for_swap = Some(i); }
                        Some(a) => {
                            self.mapping.swap(a, i);
                            self.selected_for_swap = None;
                        }
                    }
                    return None;
                } else {
                    return Some(self.mapping[i]);
                }
            }
        }
        None
    }

    /// Pixel-edge size of the wheel's bounding square. Derived once at
    /// construction from the slot table's outermost radius.
    fn required_size(&self) -> f32 {
        let mut max_r = 0.0_f32;
        for s in &self.slots {
            let r = (s.dx * s.dx + s.dy * s.dy).sqrt();
            if r > max_r { max_r = r; }
        }
        (max_r + CIRCLE_RADIUS) * 2.0 + 12.0
    }
}

// ---- deterministic layout + default mapping ----------------------------

fn build_layout_and_default_mapping() -> (Vec<Slot>, [u8; 256]) {
    // 1. Luminance-sort all 256 ACI indices. Brightest goes to center;
    //    rest fill rings in luminance-descending order, then per-ring
    //    they are re-sorted angularly by hue.
    let mut indices: Vec<u8> = (0..=255).collect();
    indices.sort_by(|a, b| {
        luminance(aci_palette(*b)).partial_cmp(&luminance(aci_palette(*a)))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let center_aci = indices.remove(0);

    // 2. Compute ring radii + per-ring capacity by walking outward.
    let diameter = CIRCLE_RADIUS * 2.0;
    let radial_step = diameter + RADIAL_GAP;
    let first_ring_radius = diameter + RADIAL_GAP;
    let mut rings: Vec<(f32, usize)> = Vec::new();   // (radius, capacity)
    let mut remaining = 255_usize;
    let mut ring_i = 0_usize;
    while remaining > 0 {
        let radius = first_ring_radius + (ring_i as f32) * radial_step;
        let circ = std::f32::consts::TAU * radius;
        let cap = ((circ / (diameter + TANGENTIAL_GAP)).floor() as usize).max(1);
        let take = cap.min(remaining);
        rings.push((radius, take));
        remaining = remaining.saturating_sub(take);
        ring_i += 1;
        if ring_i > 64 { break; }   // safety; loop converges in <15
    }

    // 3. Walk the luminance-sorted indices, slice into rings, then within
    //    each ring sort by ideal hue angle and assign to ring slots.
    let mut slots = Vec::with_capacity(256);
    let mut mapping = [0_u8; 256];

    // Center slot.
    slots.push(Slot { dx: 0.0, dy: 0.0 });
    mapping[0] = center_aci;

    let mut head = 0_usize;
    for (radius, cap) in &rings {
        let chunk = &indices[head .. head + *cap];
        head += *cap;

        // Per-ring: sort chunk by ideal-angle so neighbouring slots are
        // near-neighbours in hue.
        let mut by_angle: Vec<(u8, f32)> = chunk.iter()
            .map(|aci| (*aci, ideal_angle_for(aci_palette(*aci))))
            .collect();
        by_angle.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let angle_step = std::f32::consts::TAU / (*cap as f32);
        for (i, (aci, _)) in by_angle.into_iter().enumerate() {
            let theta = (i as f32) * angle_step;
            let dx = *radius * theta.cos();
            let dy = *radius * theta.sin();
            let slot_idx = slots.len();
            slots.push(Slot { dx, dy });
            mapping[slot_idx] = aci;
        }
    }

    (slots, mapping)
}

// ---- color math helpers -----------------------------------------------

fn luminance((r, g, b): (u8, u8, u8)) -> f32 {
    0.299 * (r as f32) + 0.587 * (g as f32) + 0.114 * (b as f32)
}

/// Map an RGB triple to its "ideal" angular slot on the picker wheel.
/// Hue values from HSL are remapped through `hue_map` so the visible
/// arrangement matches the reference (red ~0°, yellow ~315°, green
/// ~210°, blue ~120°, magenta ~45°).
fn ideal_angle_for(rgb: (u8, u8, u8)) -> f32 {
    let r = rgb.0 as f32; let g = rgb.1 as f32; let b = rgb.2 as f32;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let d = max - min;
    let s = if max < 1.0 { 0.0 } else { d / max };

    // Greys / very low-saturation: snap top or bottom based on luminance.
    if s < 0.12 {
        return if l > 160.0 { 270.0_f32.to_radians() } else { 90.0_f32.to_radians() };
    }

    let mut h = if max == min {
        0.0
    } else if max == r {
        let mut h = (g - b) / d;
        if g < b { h += 6.0; }
        h
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h /= 6.0;
    h *= 360.0;

    // Piece-wise remap (hue → ring angle) matching the HTML mockup.
    const HUE_MAP: &[(f32, f32)] = &[
        (0.0,   0.0),  (30.0,  315.0), (60.0,  240.0), (90.0,  210.0),
        (120.0, 180.0),(180.0, 150.0), (240.0, 120.0), (260.0, 100.0),
        (280.0,  80.0),(300.0,  45.0), (360.0,   0.0),
    ];
    let mut deg = 0.0_f32;
    for w in HUE_MAP.windows(2) {
        let (h1, a1) = w[0];
        let (h2, a2) = w[1];
        if h >= h1 && h <= h2 {
            let t = if h2 > h1 { (h - h1) / (h2 - h1) } else { 0.0 };
            // The hue ring wraps: at h=0 the mockup uses a1=360 so the
            // interpolation crosses the seam cleanly.
            let a1_eff = if a1 == 0.0 && a2 == 315.0 { 360.0 } else { a1 };
            deg = (a1_eff + t * (a2 - a1_eff)).rem_euclid(360.0);
            break;
        }
    }
    deg.to_radians()
}

// ---- tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_has_256_slots() {
        let (slots, mapping) = build_layout_and_default_mapping();
        assert_eq!(slots.len(), 256, "must place one slot per ACI index");
        // Each ACI index 0..=255 appears exactly once in the default mapping.
        let mut seen = [false; 256];
        for v in mapping.iter() { seen[*v as usize] = true; }
        assert!(seen.iter().all(|s| *s), "default mapping must be a permutation of 0..=255");
    }

    #[test]
    fn center_is_brightest() {
        let (_slots, mapping) = build_layout_and_default_mapping();
        let centre_lum = luminance(aci_palette(mapping[0]));
        for i in 1..256 {
            let l = luminance(aci_palette(mapping[i]));
            assert!(
                centre_lum + 0.5 >= l,
                "centre ACI {} (lum {}) must be brightest; slot {} ACI {} has lum {}",
                mapping[0], centre_lum, i, mapping[i], l
            );
        }
    }

    #[test]
    fn save_and_load_mapping_roundtrip() {
        let mut a = AciPickerState::default();
        a.mapping.swap(5, 200);
        a.mapping.swap(17, 42);

        let path = std::env::temp_dir().join("rust_cad_aci_picker_test.json");
        a.save_mapping(&path).unwrap();

        let mut b = AciPickerState::default();
        b.try_load_mapping(&path);
        assert_eq!(a.mapping, b.mapping);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn corrupt_file_is_silent_noop() {
        let mut a = AciPickerState::default();
        let orig = a.mapping;
        let path = std::env::temp_dir().join("rust_cad_aci_picker_bad.json");
        std::fs::write(&path, b"not json").unwrap();
        a.try_load_mapping(&path);
        assert_eq!(a.mapping, orig);
        let _ = std::fs::remove_file(path);
    }
}
