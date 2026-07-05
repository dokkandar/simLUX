//! IES LM-63 photometric file support (Type A/B/C, TILT=NONE).
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, EngineResult};

/// Goniometer geometry declared in the IES header (Type A/B/C photometry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhotometryType {
    A,
    B,
    C,
}

/// A parsed IES luminous-intensity distribution.
///
/// `candela[h][v]` is indexed by horizontal-angle row then vertical-angle
/// column, matching the LM-63 candela block layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IesProfile {
    pub name: String,
    pub photometry: PhotometryType,
    /// Rated lumens per lamp × number of lamps (−1 = absolute photometry).
    pub lumens: f64,
    /// Candela multiplier from the header (already excludes ballast factors).
    pub multiplier: f64,
    pub vertical_angles: Vec<f64>,
    pub horizontal_angles: Vec<f64>,
    pub candela: Vec<Vec<f64>>,
    /// Input power (watts) from the header.
    pub watts: f64,
    /// Luminous-opening dimensions (metres): width, length, height.
    pub width: f64,
    pub length: f64,
    pub height: f64,
}

impl IesProfile {
    /// Peak luminous intensity across the whole table (candela), multiplier applied.
    pub fn peak_candela(&self) -> f64 {
        self.candela
            .iter()
            .flat_map(|row| row.iter())
            .cloned()
            .fold(0.0, f64::max)
            * self.multiplier
    }

    /// Bilinearly-interpolated luminous intensity (candela) toward the given
    /// vertical/horizontal angle in degrees. Returns 0 outside the measured
    /// vertical range (e.g. a downlight emits nothing above its last angle).
    pub fn intensity(&self, vertical_deg: f64, horizontal_deg: f64) -> f64 {
        let (va, ha) = (&self.vertical_angles, &self.horizontal_angles);
        if va.is_empty() || ha.is_empty() || self.candela.is_empty() {
            return 0.0;
        }
        // No extrapolation past the measured vertical range.
        if vertical_deg < va[0] - 1e-6 || vertical_deg > va[va.len() - 1] + 1e-6 {
            return 0.0;
        }
        let (v0, v1, vt) = bracket(va, vertical_deg);
        let (h0, h1, ht) = if ha.len() == 1 {
            (0, 0, 0.0)
        } else {
            bracket(ha, horizontal_deg.rem_euclid(360.0))
        };
        let c00 = self.candela[h0][v0];
        let c01 = self.candela[h0][v1];
        let c10 = self.candela[h1][v0];
        let c11 = self.candela[h1][v1];
        let c0 = lerp(c00, c01, vt);
        let c1 = lerp(c10, c11, vt);
        lerp(c0, c1, ht) * self.multiplier
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Locate `x` in ascending `xs`; returns `(i, i+1, frac)` clamped to the ends.
fn bracket(xs: &[f64], x: f64) -> (usize, usize, f64) {
    if x <= xs[0] {
        return (0, 0, 0.0);
    }
    let last = xs.len() - 1;
    if x >= xs[last] {
        return (last, last, 0.0);
    }
    // Ascending scan is fine for the few-dozen angles a table carries.
    let mut i = 0;
    while i + 1 < xs.len() && xs[i + 1] < x {
        i += 1;
    }
    let t = (x - xs[i]) / (xs[i + 1] - xs[i]);
    (i, i + 1, t)
}

/// Parse the contents of an IES LM-63 file into an [`IesProfile`].
///
/// Handles the common `TILT=NONE` case (Type A/B/C). The numeric block is
/// tokenised free-form (values may wrap across lines, per the standard).
pub fn parse(contents: &str) -> EngineResult<IesProfile> {
    let err = |m: &str| EngineError::IesParse(m.to_string());

    // 1) Split keyword header from the numeric body at the TILT line.
    let mut name = String::new();
    let mut tilt_line: Option<&str> = None;
    let mut body_start = 0usize;
    let lines: Vec<&str> = contents.lines().collect();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("TILT=") {
            tilt_line = Some(rest.trim());
            body_start = i + 1;
            break;
        }
        // Prefer [LUMINAIRE] for the display name, else the first [TEST]-ish tag.
        if let Some(rest) = line.strip_prefix("[LUMINAIRE]") {
            name = rest.trim().to_string();
        }
    }
    let tilt = tilt_line.ok_or_else(|| err("missing TILT= line"))?;
    if !tilt.eq_ignore_ascii_case("NONE") {
        return Err(err("only TILT=NONE is supported for now"));
    }
    if name.is_empty() {
        name = "Luminaire".to_string();
    }

    // 2) Tokenise everything after TILT into a flat number stream.
    let nums: Vec<f64> = lines[body_start..]
        .iter()
        .flat_map(|l| l.split_whitespace())
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    let mut it = nums.into_iter();
    let mut next = |what: &str| it.next().ok_or_else(|| err(&format!("unexpected EOF reading {what}")));

    // 3) Two header records (13 values for TILT=NONE).
    let _num_lamps = next("num_lamps")?;
    let lumens = next("lumens_per_lamp")?;
    let multiplier = next("candela_multiplier")?;
    let n_vert = next("num_vertical_angles")? as usize;
    let n_horiz = next("num_horizontal_angles")? as usize;
    let photo = next("photometric_type")?;
    let units = next("units_type")?; // 1 = feet, 2 = metres
    let width = next("width")?;
    let length = next("length")?;
    let height = next("height")?;
    let _ballast = next("ballast_factor")?;
    let _future = next("future_use")?;
    let watts = next("input_watts")?;

    if n_vert == 0 || n_horiz == 0 {
        return Err(err("zero vertical or horizontal angles"));
    }

    // 4) Angle arrays then the candela block (horizontal-major).
    let mut vertical_angles = Vec::with_capacity(n_vert);
    for _ in 0..n_vert {
        vertical_angles.push(next("vertical angle")?);
    }
    let mut horizontal_angles = Vec::with_capacity(n_horiz);
    for _ in 0..n_horiz {
        horizontal_angles.push(next("horizontal angle")?);
    }
    let mut candela = Vec::with_capacity(n_horiz);
    for _ in 0..n_horiz {
        let mut row = Vec::with_capacity(n_vert);
        for _ in 0..n_vert {
            row.push(next("candela value")?);
        }
        candela.push(row);
    }

    let to_m = if units as i32 == 1 { 0.3048 } else { 1.0 };
    let photometry = match photo as i32 {
        3 => PhotometryType::A,
        2 => PhotometryType::B,
        _ => PhotometryType::C,
    };

    Ok(IesProfile {
        name,
        photometry,
        lumens,
        multiplier,
        vertical_angles,
        horizontal_angles,
        candela,
        watts,
        width: width * to_m,
        length: length * to_m,
        height: height * to_m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> IesProfile {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../samples/T1.ies");
        let contents = std::fs::read_to_string(path).expect("read T1.ies");
        parse(&contents).expect("parse T1.ies")
    }

    #[test]
    fn parses_header_and_table() {
        let p = sample();
        assert_eq!(p.photometry, PhotometryType::C);
        assert_eq!(p.vertical_angles.len(), 91);
        assert_eq!(p.horizontal_angles.len(), 1);
        assert_eq!(p.candela[0].len(), 91);
        assert_eq!(p.watts, 140.0);
        assert_eq!(p.vertical_angles[0], 0.0);
        assert_eq!(p.vertical_angles[90], 90.0);
    }

    #[test]
    fn interpolates_intensity() {
        let p = sample();
        // Nadir peak.
        assert!((p.intensity(0.0, 0.0) - 262637.3).abs() < 1.0);
        // Between 0° (262637.3) and 1° (252016.6): halfway ≈ mean.
        let mid = p.intensity(0.5, 0.0);
        assert!(mid < 262637.3 && mid > 252016.6);
        // Near grazing.
        assert!((p.intensity(90.0, 0.0) - 1.4).abs() < 0.1);
        // Above the measured range → no light.
        assert_eq!(p.intensity(95.0, 0.0), 0.0);
    }
}
