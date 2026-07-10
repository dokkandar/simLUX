//! Parse every `.ldt` / `.ies` file in a directory and report what came out, so
//! real manufacturer photometry can be validated end to end.
//!
//! Run:  cargo run -p cad_light --example ldt_check -- LTD-Ies
use std::path::Path;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "LTD-Ies".to_string());
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read dir '{dir}': {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            ext == "ldt" || ext == "ies"
        })
        .collect();
    entries.sort();

    println!(
        "{:<26} {:>6} {:>5} {:>6} {:>12} {:>10} {:>10}",
        "file", "planes", "gamma", "γ-range", "peak cd", "flux lm", "status"
    );
    println!("{}", "-".repeat(84));

    let (mut ok, mut fail) = (0, 0);
    for p in &entries {
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        let short: String = name.chars().take(25).collect();
        match std::fs::read_to_string(p).map_err(|e| e.to_string())
            .and_then(|txt| cad_light::parse_photometry(&txt))
        {
            Ok(prof) => {
                ok += 1;
                let g0 = prof.vertical_angles.first().copied().unwrap_or(0.0);
                let g1 = prof.vertical_angles.last().copied().unwrap_or(0.0);
                println!(
                    "{:<26} {:>6} {:>5} {:>5.0}-{:<4.0} {:>12.1} {:>10.0} {:>10}",
                    short,
                    prof.horizontal_angles.len(),
                    prof.vertical_angles.len(),
                    g0, g1,
                    prof.peak_candela(),
                    prof.lumens,
                    "OK",
                );
            }
            Err(e) => {
                fail += 1;
                println!("{short:<26} {:>6} {:>5} {:>10} {:>12} {:>10} {:>10}", "", "", "", "", "", "FAIL");
                println!("    └─ {e}");
            }
        }
    }
    let _ = Path::new(&dir);
    println!("\n{} parsed OK, {} failed, of {} files.", ok, fail, entries.len());
}
