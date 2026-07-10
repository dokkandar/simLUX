//! EULUMDAT (`.ldt`) photometry parsing → the shared [`IesProfile`] C-γ table,
//! so the existing lux calc consumes LDT and IES identically.
//!
//! EULUMDAT is a positional, one-value-per-line ASCII format. Layout (1-based):
//! ```text
//!  1 company            13 luminaire length/Ø     26 number of lamp sets n
//!  2 Ityp               14 luminaire width         ── per set (×n), 6 lines:
//!  3 Isym  (symmetry)   15 luminaire height           num lamps / type / flux /
//!  4 Mc    (# C-planes) 16 lum-area length/Ø           colour temp / CRI / watts
//!  5 Dc                 17 lum-area width          ── then 10 direct-ratio values
//!  6 Ng    (# γ / plane)18 lum-area height C0      ── then Mc  C-plane angles
//!  7 Dg                 19..21 heights C90/180/270 ── then Ng  γ angles
//!  8 report no.         22 DFF %                   ── then intensities cd/1000lm
//!  9 luminaire name     23 LORL %                     (stored planes × Ng, then
//! 10 luminaire no.      24 conversion factor           expanded per Isym)
//! 11 file name          25 tilt
//! 12 date / user
//! ```
//! Intensities are candela per 1000 lm; absolute cd = value × lamp_flux/1000.
//! Symmetry stores only the unique C-planes and we mirror them out:
//! Isym 0 = none (Mc planes), 1 = rotational (1 plane), 2 = C0–C180 mirror,
//! 4 = quadrant (C0–C90). Isym 3 (C90–C270) is rejected for now.
use crate::ies::{IesProfile, PhotometryType};

/// Parse EULUMDAT `.ldt` text into an [`IesProfile`] (Type C, one C-γ table).
pub fn parse(contents: &str) -> Result<IesProfile, String> {
    let lines: Vec<&str> = contents.lines().map(|l| l.trim_end_matches('\r')).collect();

    let eof = |i: usize| format!("LDT: unexpected end of file at line {}", i + 1);
    let f = |i: usize| -> Result<f64, String> {
        lines
            .get(i)
            .map(|s| s.trim().parse::<f64>().unwrap_or(0.0))
            .ok_or_else(|| eof(i))
    };
    let int = |i: usize| -> Result<i64, String> {
        let s = lines.get(i).ok_or_else(|| eof(i))?;
        s.trim()
            .parse::<f64>()
            .map(|v| v as i64)
            .map_err(|_| format!("LDT: line {} expected a number, got '{}'", i + 1, s))
    };

    let isym = int(2)?;
    let mc = int(3)?.max(0) as usize;
    let ng = int(5)?.max(0) as usize;
    if mc == 0 || ng == 0 {
        return Err(format!("LDT: bad Mc={mc} / Ng={ng}"));
    }
    let name = lines.get(8).map(|s| s.trim().to_string()).unwrap_or_default();
    let conv = {
        let c = f(23)?;
        if c != 0.0 { c } else { 1.0 }
    };
    let n_sets = int(25)?.max(1) as usize;

    // Lamp sets: 6 lines each from line 27 (idx 26); flux is the 3rd (+2).
    let mut idx = 26usize;
    let first_num_lamps = int(idx)?;
    let mut total_flux = 0.0;
    for _ in 0..n_sets {
        total_flux += f(idx + 2)?.abs();
        idx += 6;
    }
    idx += 10; // 10 direct-ratio values

    // Mc C-plane angles, then Ng γ angles.
    let mut c_angles = Vec::with_capacity(mc);
    for _ in 0..mc {
        c_angles.push(f(idx)?);
        idx += 1;
    }
    let mut g_angles = Vec::with_capacity(ng);
    for _ in 0..ng {
        g_angles.push(f(idx)?);
        idx += 1;
    }

    // Number of intensity planes actually stored, per symmetry.
    let stored = match isym {
        0 => mc,
        1 => 1,
        2 => mc / 2 + 1,
        4 => mc / 4 + 1,
        3 => return Err("LDT: symmetry Isym=3 (C90–C270) not supported yet".into()),
        _ => return Err(format!("LDT: unknown symmetry Isym={isym}")),
    };
    let mut planes: Vec<Vec<f64>> = Vec::with_capacity(stored);
    for _ in 0..stored {
        let mut plane = Vec::with_capacity(ng);
        for _ in 0..ng {
            plane.push(f(idx)?);
            idx += 1;
        }
        planes.push(plane);
    }

    // cd/1000lm → cd. A NEGATIVE lamp count flags absolute candela (no flux scale).
    let mult = if first_num_lamps < 0 { conv } else { (total_flux / 1000.0) * conv };

    // Expand symmetry to a full C-plane table (candela[c_plane][gamma]).
    let (horizontal_angles, candela) = if isym == 1 {
        // Rotational: one plane serves every azimuth (calc ignores φ when there
        // is a single horizontal angle).
        (vec![0.0], vec![planes[0].clone()])
    } else {
        let last = planes.len() - 1;
        let stored_index = |c: usize| -> usize {
            match isym {
                0 => c,
                2 => {
                    if c <= mc / 2 { c } else { mc - c }
                }
                4 => {
                    let q = mc / 4;
                    if c <= q {
                        c
                    } else if c <= mc / 2 {
                        mc / 2 - c
                    } else if c <= 3 * q {
                        c - mc / 2
                    } else {
                        mc - c
                    }
                }
                _ => 0,
            }
        };
        let mut cand = Vec::with_capacity(mc);
        for c in 0..mc {
            cand.push(planes[stored_index(c).min(last)].clone());
        }
        (c_angles, cand)
    };

    Ok(IesProfile {
        name: if name.is_empty() { "LDT".into() } else { name },
        photometry: PhotometryType::C,
        lumens: total_flux,
        multiplier: mult,
        vertical_angles: g_angles,
        horizontal_angles,
        candela,
        watts: 0.0,
        width: 0.0,
        length: 0.0,
        height: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal EULUMDAT file: the fixed 26-line header, `n_sets` lamp
    /// blocks (6 lines each), 10 direct-ratios, then C-angles, γ-angles and
    /// intensity values appended by the caller.
    fn ldt(isym: i64, mc: usize, ng: usize, flux: f64, tail: &[&str]) -> String {
        let mut l: Vec<String> = Vec::new();
        l.push("TEST".into()); // 1 company
        l.push("1".into()); // 2 Ityp
        l.push(isym.to_string()); // 3 Isym
        l.push(mc.to_string()); // 4 Mc
        l.push("0".into()); // 5 Dc
        l.push(ng.to_string()); // 6 Ng
        l.push("0".into()); // 7 Dg
        l.push("".into()); // 8 report
        l.push("TestLum".into()); // 9 name
        for _ in 0..3 {
            l.push("".into());
        } // 10 no, 11 file, 12 date
        for _ in 0..9 {
            l.push("0".into());
        } // 13..21 geometry
        l.push("100".into()); // 22 DFF
        l.push("100".into()); // 23 LORL
        l.push("1.0".into()); // 24 conversion
        l.push("0".into()); // 25 tilt
        l.push("1".into()); // 26 n lamp sets
        l.push("1".into()); // 27 num lamps
        l.push("LED".into()); // 28 type
        l.push(flux.to_string()); // 29 flux
        l.push("3000".into()); // 30 colour temp
        l.push("80".into()); // 31 CRI
        l.push("10".into()); // 32 watts
        for _ in 0..10 {
            l.push("0".into());
        } // direct ratios
        for s in tail {
            l.push((*s).into());
        }
        l.join("\r\n")
    }

    #[test]
    fn rotational_scales_by_flux() {
        // Isym=1, one plane, 2 γ (0,90), cd/klm 100 & 0, flux 2000 → peak 200 cd.
        let src = ldt(1, 1, 2, 2000.0, &["0", "0", "90", "100", "0"]);
        let p = parse(&src).unwrap();
        assert_eq!(p.horizontal_angles.len(), 1); // axially symmetric
        assert_eq!(p.vertical_angles, vec![0.0, 90.0]);
        assert!((p.intensity(0.0, 0.0) - 200.0).abs() < 1e-6);
        assert!((p.intensity(0.0, 137.0) - 200.0).abs() < 1e-6); // φ ignored
        assert_eq!(p.intensity(90.0, 0.0), 0.0);
    }

    #[test]
    fn quadrant_mirrors_c_planes() {
        // Isym=4, Mc=4 (C0/90/180/270), 1 γ. stored = 4/4+1 = 2 planes:
        // plane0 = 200 cd/klm (C0), plane1 = 100 (C90). Quadrant symmetry must
        // yield C0=C180=200, C90=C270=100.
        let tail = ["0", "90", "180", "270", "0", "200", "100"];
        let src = ldt(4, 4, 1, 1000.0, &tail);
        let p = parse(&src).unwrap();
        assert_eq!(p.horizontal_angles, vec![0.0, 90.0, 180.0, 270.0]);
        assert!((p.intensity(0.0, 0.0) - 200.0).abs() < 1e-6); // C0
        assert!((p.intensity(0.0, 90.0) - 100.0).abs() < 1e-6); // C90
        assert!((p.intensity(0.0, 180.0) - 200.0).abs() < 1e-6); // C180 mirror
        assert!((p.intensity(0.0, 270.0) - 100.0).abs() < 1e-6); // C270 mirror
    }

    #[test]
    fn isym3_is_rejected_cleanly() {
        let src = ldt(3, 4, 1, 1000.0, &["0", "90", "180", "270", "0", "1", "2", "3"]);
        assert!(parse(&src).is_err());
    }
}
