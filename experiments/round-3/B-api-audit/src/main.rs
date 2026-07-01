//! FreeCell Round-3 Investigation B (breadth) — needed-API audit runner.
//!
//! Runs every probe in the crate and prints the present / absent / workaround matrix,
//! env-stamped. Because each probe asserts the API it claims is present, `cargo run`
//! doubles as a liveness check of the "present" claims (a regression would panic here).
//! The authoritative, source-cited write-up is `findings.md`; this is the reproducible
//! demonstration.

use api_audit::{run_full_audit, Status};
use round2_harness::cpu_model;

fn main() {
    println!("# FreeCell Round-3 B — needed-API audit (IronCalc 0.7.1)");
    println!("# env: cpu = {}", cpu_model());
    println!("# ironcalc pin: 0.7.x (Cargo.lock -> 0.7.1, the round-2 harness pin)\n");

    let rows = run_full_audit();

    let mut present = 0;
    let mut absent = 0;
    let mut workaround = 0;
    for r in &rows {
        match r.status {
            Status::Present => present += 1,
            Status::Absent => absent += 1,
            Status::Workaround => workaround += 1,
        }
        println!(
            "[{:>10}] {}\n             {}",
            r.status.label(),
            r.capability,
            r.note
        );
    }

    println!(
        "\n# totals: {} present, {} workaround, {} absent ({} rows)",
        present,
        workaround,
        absent,
        rows.len()
    );
    println!("# HEADLINE: display formatting is engine-owned (PRESENT) — FreeCell does NOT");
    println!("#           implement number-format rendering. See findings.md §Findings.");
}
