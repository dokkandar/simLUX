//! Design tokens — the single source of every visual value in the app.
//!
//! This is the code side of `THEME_SYSTEM.md` §5 (the locked token registry).
//! **Components must read these tokens, never hard-code colours or sizes.**
//! When a value needs to change, change it *here* and it propagates app-wide.
//!
//! Dark theme only (teal-navy surfaces, cyan accent). A future `ThemeStore`
//! (THEME_SYSTEM §2) will make these runtime-editable; for now they are `const`.

use egui::Color32;

/// Colour tokens. See THEME_SYSTEM §5.3–5.6.
pub mod color {
    use super::Color32;

    // ── Surfaces (tonal elevation ladder) ──────────────────────────────
    pub const SURFACE_0: Color32 = Color32::from_rgb(0x14, 0x1c, 0x25); // canvas / page
    pub const SURFACE_1: Color32 = Color32::from_rgb(0x1a, 0x24, 0x30); // panel
    pub const SURFACE_2: Color32 = Color32::from_rgb(0x22, 0x2b, 0x34); // raised control
    pub const SURFACE_3: Color32 = Color32::from_rgb(0x2a, 0x37, 0x44); // popover / menu
    pub const CHROME:    Color32 = Color32::from_rgb(0x22, 0x30, 0x40); // header / footer band
    pub const FIELD:     Color32 = Color32::from_rgb(0x14, 0x1c, 0x25); // input fill (= surface-0)
    pub const BORDER:    Color32 = Color32::from_rgb(0x34, 0x41, 0x4b);

    // ── Accent ─────────────────────────────────────────────────────────
    pub const ACCENT:    Color32 = Color32::from_rgb(0x00, 0xe5, 0xff);
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0x06, 0x3b, 0x45); // text on the cyan fill

    // ── Text ───────────────────────────────────────────────────────────
    pub const TEXT_PRIMARY:   Color32 = Color32::from_rgb(0xda, 0xe3, 0xef);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xae, 0xb9, 0xc4);
    pub const TEXT_MUTED:     Color32 = Color32::from_rgb(0x93, 0xa1, 0xac); // labels, placeholders
    pub const TEXT_DISABLED:  Color32 = Color32::from_rgb(0x5c, 0x69, 0x75);

    // ── Semantic (no `info` — cyan carries it) ─────────────────────────
    pub const SUCCESS: Color32 = Color32::from_rgb(0x34, 0xd3, 0x99);
    pub const WARNING: Color32 = Color32::from_rgb(0xf2, 0xb5, 0x3d);
    pub const DANGER:  Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

    // ── State overlays (apply over any surface) ────────────────────────
    /// Hover lift — white at ~6%.
    pub const HOVER: Color32 = Color32::from_rgba_premultiplied(15, 15, 15, 15);
    /// Hover on cyan-highlight elements — accent at ~12%.
    pub const HOVER_ACCENT: Color32 = Color32::from_rgba_premultiplied(0, 27, 30, 31);
    /// Focus ring colour (2px, 2px offset — see the a11y rules).
    pub const FOCUS: Color32 = ACCENT;
}

/// Spacing scale (4px base). See THEME_SYSTEM §5.1.
pub mod space {
    pub const XXS: f32 = 2.0;
    pub const XS:  f32 = 4.0;
    pub const SM:  f32 = 8.0;
    pub const MD:  f32 = 12.0;
    pub const LG:  f32 = 16.0;
    pub const XL:  f32 = 24.0;
    pub const XXL: f32 = 32.0;

    /// Uniform control / field / row height.
    pub const CONTROL_H: f32 = 24.0;
    /// Vertical gap between property rows.
    pub const ROW_GAP: f32 = 8.0;
    /// Icon hit-box.
    pub const ICON_BOX: f32 = 24.0;
}

/// Corner-radius scale. See THEME_SYSTEM §5.2.
pub mod radius {
    pub const XS:   f32 = 2.0;  // swatches, micro chips
    pub const SM:   f32 = 4.0;  // inputs, value boxes
    pub const MD:   f32 = 8.0;  // buttons, icon buttons, dropdowns
    pub const LG:   f32 = 12.0; // cards, panels, menus
    pub const FULL: f32 = 9999.0; // pills, toggles
}
