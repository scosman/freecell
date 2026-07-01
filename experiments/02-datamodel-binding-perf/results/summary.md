# Sub-project C — recorded benchmark summary

Machine-readable per-run JSON lives in `formualizer/` and `ironcalc/` alongside this file. Regenerate with each engine crate's `scenarios` binary (see `../findings.md`).

| engine | scenario | design | p50 | p99 | max | verdict |
|--------|----------|--------|-----|-----|-----|---------|
| formualizer | cascade-1m-chain | — | 1.87 s | 3.24 s | 3.24 s | FAIL |
| formualizer | cascade-fanout | — | 3.51 s | 3.56 s | 3.56 s | FAIL |
| formualizer | cascade-visible | D1 | 132.39 ms | 196.63 ms | 196.63 ms | FAIL |
| formualizer | cascade-visible | D2 | 137.91 ms | 198.79 ms | 198.79 ms | FAIL |
| formualizer | cascade-visible | D3 | 123.33 ms | 194.31 ms | 194.31 ms | FAIL |
| formualizer | memory | — | — | — | — | — |
| formualizer | scrolling-read | D1 | 190.02 µs | 310.22 µs | 394.98 µs | PASS |
| formualizer | scrolling-read | D2 | 155.17 µs | 222.03 µs | 245.72 µs | PASS |
| formualizer | scrolling-read | D3 | 354.52 µs | 434.32 µs | 540.80 µs | PASS |
| formualizer | writes-single | — | 148.53 µs | 365.20 µs | 415.39 µs | — |
| ironcalc | cascade-1m-chain | — | 2.11 s | 2.15 s | 2.15 s | FAIL |
| ironcalc | cascade-fanout | — | 77.54 ms | 78.35 ms | 78.35 ms | FAIL |
| ironcalc | cascade-visible | D1 | 118.79 ms | 122.88 ms | 122.88 ms | FAIL |
| ironcalc | cascade-visible | D2 | 107.46 ms | 109.18 ms | 109.18 ms | FAIL |
| ironcalc | cascade-visible | D3 | 104.08 ms | 112.69 ms | 112.69 ms | FAIL |
| ironcalc | memory | — | — | — | — | — |
| ironcalc | scrolling-read | D1 | 336.76 µs | 418.72 µs | 518.00 µs | PASS |
| ironcalc | scrolling-read | D2 | 309.43 µs | 392.42 µs | 405.36 µs | PASS |
| ironcalc | scrolling-read | D3 | 485.14 µs | 585.29 µs | 714.37 µs | PASS |
| ironcalc | writes-single | — | 31.78 µs | 76.67 µs | 173.87 µs | — |
