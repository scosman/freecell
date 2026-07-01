//! Criterion micro/throughput benches for the IronCalc adapter (functional_spec
//! §5.3), mirroring the Formualizer bench so the two are directly comparable: viewport
//! read D1 vs D2 vs D3 (all per-cell under the hood for IronCalc — that's the finding),
//! and single vs batched writes.
//!
//! Run: `cargo bench` (from this crate). Sizes are modest; the spec-scale numbers come
//! from the `scenarios` bin.

use binding_common::binding::{read_under, BindingCache, Design};
use binding_common::{CellInput, EngineValue, SpreadsheetEngine, Viewport};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use ironcalc_bench::IronCalcEngine;

/// Builds an IronCalc engine seeded with a `rows × cols` numeric block.
fn seeded(rows: u32, cols: u32) -> IronCalcEngine {
    let mut e = IronCalcEngine::new_blank();
    let batch: Vec<(u32, u32, CellInput)> = (0..rows)
        .flat_map(|r| {
            (0..cols).map(move |c| {
                (
                    r,
                    c,
                    CellInput::Value(EngineValue::Number((r * cols + c) as f64)),
                )
            })
        })
        .collect();
    e.set_batch(&batch);
    e
}

fn bench_viewport_read(c: &mut Criterion) {
    // Smaller seed than Formualizer's bench: IronCalc's HashMap seeding + full
    // evaluate is slower, and the viewport read cost is what we're isolating.
    let engine = seeded(1_000, 100);
    let vp = Viewport::new(200, 20, 60, 30); // ~1,800 cells
    let mut group = c.benchmark_group("viewport_read");
    group.bench_function("D1_per_cell", |b| {
        b.iter(|| read_under(Design::NaivePerCell, &engine, vp))
    });
    group.bench_function("D2_range", |b| {
        b.iter(|| read_under(Design::BulkRange, &engine, vp))
    });
    group.bench_function("D3_cached_warm", |b| {
        let mut cache = BindingCache::new();
        cache.prime(&engine, vp);
        b.iter(|| cache.snapshot(&engine, vp))
    });
    group.finish();
}

fn bench_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("writes");
    let n = 200u32; // smaller than Formualizer: each single set_value pays a full evaluate
    group.bench_function("single_200", |b| {
        b.iter_batched(
            IronCalcEngine::new_blank,
            |mut e| {
                for i in 0..n {
                    e.set_value(0, i, EngineValue::Number(i as f64));
                    e.recompute();
                }
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("batched_200", |b| {
        let batch: Vec<(u32, u32, CellInput)> = (0..n)
            .map(|i| (0, i, CellInput::Value(EngineValue::Number(i as f64))))
            .collect();
        b.iter_batched(
            IronCalcEngine::new_blank,
            |mut e| e.set_batch(&batch),
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion_group!(benches, bench_viewport_read, bench_writes);
criterion_main!(benches);
