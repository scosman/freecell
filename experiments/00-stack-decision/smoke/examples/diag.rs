// Temporary diagnostic — probes what survives the xlsx round trip. Not part of the
// committed deliverable; deleted after diagnosis.
fn main() -> anyhow::Result<()> {
    let mut wb = smoke::build_sum_workbook()?;
    wb.evaluate_all().map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("PRE-SAVE A3: value={:?} formula={:?}",
        wb.get_value("Sheet1", 3, 1), wb.get_formula("Sheet1", 3, 1));
    let bytes = smoke::xlsx_bytes_from(&wb)?;
    let re = smoke::load_xlsx_bytes(&bytes)?;
    for (r, name) in [(1, "A1"), (2, "A2"), (3, "A3")] {
        println!(
            "RELOAD {name}: value={:?} formula={:?}",
            re.get_value("Sheet1", r, 1),
            re.get_formula("Sheet1", r, 1)
        );
    }
    let mut re2 = smoke::load_xlsx_bytes(&bytes)?;
    re2.prepare_graph_all().ok();
    println!(
        "RELOAD A3 evaluated after reload: {:?}",
        re2.evaluate_cell("Sheet1", 3, 1).map(|v| format!("{v:?}"))
    );
    Ok(())
}
