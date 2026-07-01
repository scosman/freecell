//! Writes the IronCalc formatting-capability matrix to
//! `experiments/03-formatting/results/ironcalc/capabilities.json`.
//!
//! Run from the crate directory: `cargo run --bin emit`. The matrix content is backed
//! by the passing probes in `tests/probe.rs`.

use std::path::Path;

fn main() -> std::io::Result<()> {
    let matrix = ironcalc_formatting::capability_matrix();
    let json = serde_json::to_string_pretty(&matrix).expect("serialize matrix");
    let out = Path::new("../results/ironcalc/capabilities.json");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, json + "\n")?;
    println!("wrote {}", out.display());
    Ok(())
}
