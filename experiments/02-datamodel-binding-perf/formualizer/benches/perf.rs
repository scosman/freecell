//! Criterion micro/throughput benches for the Formualizer adapter (functional_spec
//! §5.3: a real harness for the micro numbers, complementing the end-to-end
//! `scenarios` bin). Focuses on the two comparisons the binding-design question hinges
//! on: viewport read D1 vs D2 vs D3, and single vs batched writes.
//!
//! Run: `cargo bench` (from this crate). Sizes are modest so the bench completes
//! quickly; the spec-scale numbers come from the `scenarios` bin.

use binding_common::binding::{read_under, BindingCache, Design};
use binding_common::{EngineValue, SpreadsheetEngine, Viewport};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use formualizer_bench::FormualizerEngine;

/// Builds a Formualizer engine seeded with a `rows × cols` numeric block.
fn seeded(rows: u32, cols: u32) -> FormualizerEngine {
    let mut e = FormualizerEngine::new_blank();
    let batch: Vec<(u32, u32, binding_common::CellInput)> = (0..rows)
        .flat_map(|r| {
            (0..cols).map(move |c| {
                (
                    r,
                    c,
                    binding_common::CellInput::Value(EngineValue::Number((r * cols + c) as f64)),
                )
            })
        })
        .collect();
    e.set_batch(&batch);
    e
}

fn bench_viewport_read(c: &mut Criterion) {
    let engine = seeded(2_000, 200);
    let vp = Viewport::new(500, 20, 60, 30); // ~1,800 cells
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
    let n = 1_000u32;
    group.bench_function("single_1k", |b| {
        b.iter_batched(
            FormualizerEngine::new_blank,
            |mut e| {
                for i in 0..n {
                    e.set_value(0, i, EngineValue::Number(i as f64));
                }
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("batched_1k", |b| {
        let batch: Vec<(u32, u32, binding_common::CellInput)> = (0..n)
            .map(|i| {
                (
                    0,
                    i,
                    binding_common::CellInput::Value(EngineValue::Number(i as f64)),
                )
            })
            .collect();
        b.iter_batched(
            FormualizerEngine::new_blank,
            |mut e| e.set_batch(&batch),
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion_group!(benches, bench_viewport_read, bench_writes);
criterion_main!(benches);
