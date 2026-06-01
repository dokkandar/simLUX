// User-Environment Settings — RUST_CAD's analogue of AutoCAD SYSVARS.
//
// Cryptic short names (`SpTGSZ`, `GrpEnb`, …) follow a project-wide
// convention: no underscores, mixed case, 5–7 characters. See
// `feedback_rust_cad_settings_naming.md` in the memory store.
//
// Per-field doc comments are the source of truth for what each setting
// does. The settings window in `app.rs` mirrors them as plain-English
// labels next to the cryptic name.
//
// Persisted to `$HOME/.config/rust_cad/user_env.txt` — a one-line-per-
// setting format (`KEY = VALUE`). Survives app restarts; falls back to
// `Default::default()` if the file is missing or unparseable.

#![allow(non_snake_case)]

use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct UserEnv {
    // ---- snap & picking sizes (pixels / percent) ----
    /// Object-snap target height in pixels. Cursor must be within this many
    /// screen pixels of a candidate snap point (or entity, depending on the
    /// kind) for the snap to fire.
    pub SpTGSZ: u8,
    /// Pickbox height in pixels — the square inside which a click is
    /// considered "on" a dobject (selection / hit-test tolerance).
    /// TENTATIVE: `HitTolPx` (see Variables.md §13) may supersede this.
    /// Keep for now; revisit when selection hit-testing is centralized.
    pub PkBxSz: u8,
    /// Crosshair size as percentage of the viewport's shorter side.
    pub CrsHrS: u8,

    // ---- dialogs ----
    /// Attribute entry dialog on INSERT. true = popup dialog, false = prompt.
    /// TENTATIVE: no attribute system exists yet. Keep for forward-compat;
    /// revisit when (and if) blocks + attributes land.
    pub AtDlgM: bool,
    /// Attribute prompting during INSERT.
    /// TENTATIVE: same as `AtDlgM` — no attribute system yet.
    pub AtPrmM: bool,
    /// Dialog boxes for PLOT, etc. true = show, false = command-line only.
    pub CmDlgM: bool,
    /// Suppress file-navigation dialogs (true = use OS dialogs; false = type).
    pub FlDlgM: bool,

    // ---- editing behaviour ----
    /// Edge-mode for TRIM / EXTEND. ON = treat cutting / boundary edges as
    /// their infinite extensions for "imaginary intersection" cuts. OFF =
    /// use only intersections on the visible curve. Default ON (matches the
    /// AutoCAD `EDGEMODE 1` convention most CAD users expect today).
    pub EdgMod: bool,

    // ---- display ----
    /// Dragging display during MOVE/COPY. 0=off, 1=on, 2=auto.
    pub DrDspM: u8,
    /// Display the classic menu bar at the top of the window.
    pub MnuBar: bool,
    /// Show toolbar/ribbon tooltips on hover.
    pub TltEnb: bool,
    /// Tooltips on dobject rollover (hover over a dobject in the canvas).
    pub RllTp:  bool,
    /// Preview-highlight a dobject when the cursor is over it (before click).
    pub SelPrv: bool,
    /// Highlight selected dobjects with a distinct color.
    pub HltSel: bool,
    /// Frame display of wipeouts. 0=off, 1=on, 2=on for selection only.
    pub WpFrmM: u8,

    // ---- grips ----
    /// Enable / disable grip handles on selected dobjects.
    pub GrpEnb: bool,
    /// Show grips on dobjects that live inside blocks (when blocks land).
    pub GrpBlk: bool,
    /// Unselected grip colour (RGB packed: 0xRRGGBB).
    pub GrClrU: u32,
    /// Selected (hot) grip colour (RGB packed).
    pub GrClrS: u32,
    /// Grip size in pixels.
    pub GrpSz:  u8,

    // ---- external references ----
    /// External-reference demand-loading mode.
    /// 0 = off, 1 = on, 2 = on with copy (work on a temp duplicate).
    pub XrLdMd: u8,
    /// Path for temporary xref copies (empty → system temp dir).
    pub XrTmpP: String,
}

impl Default for UserEnv {
    fn default() -> Self {
        Self {
            SpTGSZ: 16,
            PkBxSz: 10,
            CrsHrS: 5,
            AtDlgM: true,
            AtPrmM: true,
            CmDlgM: true,
            FlDlgM: true,
            EdgMod: true,
            DrDspM: 2,
            MnuBar: false,
            TltEnb: true,
            RllTp:  true,
            SelPrv: true,
            HltSel: true,
            WpFrmM: 2,
            GrpEnb: true,
            GrpBlk: false,
            GrClrU: 0x4099FF,    // light blue
            GrClrS: 0xFF6464,    // red-pink
            GrpSz:  4,
            XrLdMd: 2,
            XrTmpP: String::new(),
        }
    }
}

impl UserEnv {
    fn config_path() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".config/rust_cad/user_env.txt"))
    }

    /// Load from disk, or fall back to `Default::default()` if the file
    /// is missing, unreadable, or malformed.
    pub fn load() -> Self {
        let mut env = Self::default();
        let Some(path) = Self::config_path() else { return env; };
        let Ok(text) = fs::read_to_string(&path) else { return env; };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let Some((k, v)) = line.split_once('=') else { continue; };
            env.set(k.trim(), v.trim());
        }
        env
    }

    /// Write to disk. Logs failures by returning Err; the caller decides
    /// whether to surface them to the user.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::config_path() else {
            return Err(std::io::Error::other("no HOME"));
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut s = String::new();
        s.push_str("# RUST_CAD User-Environment Settings\n");
        s.push_str("# Cryptic short names — see source / settings window for the plain-English description.\n\n");
        // Order matches the struct so the file reads top-down by section.
        let push_u8   = |s: &mut String, k: &str, v: u8| s.push_str(&format!("{} = {}\n", k, v));
        let push_u32  = |s: &mut String, k: &str, v: u32| s.push_str(&format!("{} = 0x{:06X}\n", k, v));
        let push_bool = |s: &mut String, k: &str, v: bool| s.push_str(&format!("{} = {}\n", k, v));
        let push_str  = |s: &mut String, k: &str, v: &str| s.push_str(&format!("{} = {}\n", k, v));
        push_u8(&mut s, "SpTGSZ", self.SpTGSZ);
        push_u8(&mut s, "PkBxSz", self.PkBxSz);
        push_u8(&mut s, "CrsHrS", self.CrsHrS);
        push_bool(&mut s, "AtDlgM", self.AtDlgM);
        push_bool(&mut s, "AtPrmM", self.AtPrmM);
        push_bool(&mut s, "CmDlgM", self.CmDlgM);
        push_bool(&mut s, "FlDlgM", self.FlDlgM);
        push_bool(&mut s, "EdgMod", self.EdgMod);
        push_u8(&mut s, "DrDspM", self.DrDspM);
        push_bool(&mut s, "MnuBar", self.MnuBar);
        push_bool(&mut s, "TltEnb", self.TltEnb);
        push_bool(&mut s, "RllTp",  self.RllTp);
        push_bool(&mut s, "SelPrv", self.SelPrv);
        push_bool(&mut s, "HltSel", self.HltSel);
        push_u8(&mut s, "WpFrmM", self.WpFrmM);
        push_bool(&mut s, "GrpEnb", self.GrpEnb);
        push_bool(&mut s, "GrpBlk", self.GrpBlk);
        push_u32(&mut s, "GrClrU", self.GrClrU);
        push_u32(&mut s, "GrClrS", self.GrClrS);
        push_u8(&mut s, "GrpSz",  self.GrpSz);
        push_u8(&mut s, "XrLdMd", self.XrLdMd);
        push_str(&mut s, "XrTmpP", &self.XrTmpP);
        fs::write(&path, s)
    }

    /// Assign by cryptic key. Unknown keys are ignored (forward-compatible
    /// with files written by a newer version). Malformed values are dropped
    /// silently — the field keeps its prior value.
    fn set(&mut self, key: &str, val: &str) {
        let parse_bool = |s: &str| -> Option<bool> {
            match s.to_ascii_lowercase().as_str() {
                "true" | "1" | "on" | "yes"  => Some(true),
                "false"| "0" | "off"| "no"   => Some(false),
                _ => None,
            }
        };
        let parse_u32 = |s: &str| -> Option<u32> {
            if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                s.parse().ok()
            }
        };
        match key {
            "SpTGSZ" => if let Ok(v) = val.parse() { self.SpTGSZ = v; }
            "PkBxSz" => if let Ok(v) = val.parse() { self.PkBxSz = v; }
            "CrsHrS" => if let Ok(v) = val.parse() { self.CrsHrS = v; }
            "AtDlgM" => if let Some(v) = parse_bool(val) { self.AtDlgM = v; }
            "AtPrmM" => if let Some(v) = parse_bool(val) { self.AtPrmM = v; }
            "CmDlgM" => if let Some(v) = parse_bool(val) { self.CmDlgM = v; }
            "FlDlgM" => if let Some(v) = parse_bool(val) { self.FlDlgM = v; }
            "EdgMod" => if let Some(v) = parse_bool(val) { self.EdgMod = v; }
            "DrDspM" => if let Ok(v) = val.parse() { self.DrDspM = v; }
            "MnuBar" => if let Some(v) = parse_bool(val) { self.MnuBar = v; }
            "TltEnb" => if let Some(v) = parse_bool(val) { self.TltEnb = v; }
            "RllTp"  => if let Some(v) = parse_bool(val) { self.RllTp  = v; }
            "SelPrv" => if let Some(v) = parse_bool(val) { self.SelPrv = v; }
            "HltSel" => if let Some(v) = parse_bool(val) { self.HltSel = v; }
            "WpFrmM" => if let Ok(v) = val.parse() { self.WpFrmM = v; }
            "GrpEnb" => if let Some(v) = parse_bool(val) { self.GrpEnb = v; }
            "GrpBlk" => if let Some(v) = parse_bool(val) { self.GrpBlk = v; }
            "GrClrU" => if let Some(v) = parse_u32(val) { self.GrClrU = v; }
            "GrClrS" => if let Some(v) = parse_u32(val) { self.GrClrS = v; }
            "GrpSz"  => if let Ok(v) = val.parse() { self.GrpSz = v; }
            "XrLdMd" => if let Ok(v) = val.parse() { self.XrLdMd = v; }
            "XrTmpP" => self.XrTmpP = val.to_string(),
            _ => {}     // unknown — forward-compatible
        }
    }
}
