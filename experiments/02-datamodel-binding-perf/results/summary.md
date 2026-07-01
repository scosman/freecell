# Sub-project C — recorded benchmark summary

Machine-readable per-run JSON lives in `formualizer/` and `ironcalc/` alongside this file. Regenerate with each engine crate's `scenarios` binary (see `../findings.md`).

| engine | scenario | design | p50 | p99 | max | verdict |
|--------|----------|--------|-----|-----|-----|---------|
| formualizer | cascade-1m-chain | — | 1.77 s | 3.20 s | 3.20 s | FAIL |
| formualizer | cascade-fanout | — | 3.47 s | 3.53 s | 3.53 s | FAIL |
| formualizer | cascade-visible | D1 | 122.33 ms | 174.68 ms | 174.68 ms | FAIL |
| formualizer | cascade-visible | D2 | 134.70 ms | 188.50 ms | 188.50 ms | FAIL |
| formualizer | cascade-visible | D3 | 119.86 ms | 183.06 ms | 183.06 ms | FAIL |
| formualizer | memory | — | — | — | — | — |
| formualizer | scrolling-read | D1 | 289.60 µs | 481.31 µs | 562.35 µs | PASS |
| formualizer | scrolling-read | D2 | 253.87 µs | 452.51 µs | 515.76 µs | PASS |
| formualizer | scrolling-read | D3 | 511.29 µs | 704.61 µs | 747.23 µs | PASS |
| formualizer | writes-single | — | 182.41 µs | 458.90 µs | 527.42 µs | — |
| ironcalc | cascade-1m-chain | — | 2.02 s | 2.12 s | 2.12 s | FAIL |
| ironcalc | cascade-fanout | — | 83.21 ms | 83.66 ms | 83.66 ms | FAIL |
| ironcalc | cascade-visible | D1 | 103.52 ms | 109.25 ms | 109.25 ms | FAIL |
| ironcalc | cascade-visible | D2 | 106.74 ms | 114.81 ms | 114.81 ms | FAIL |
| ironcalc | cascade-visible | D3 | 106.20 ms | 115.52 ms | 115.52 ms | FAIL |
| ironcalc | memory | — | — | — | — | — |
| ironcalc | scrolling-read | D1 | 298.03 µs | 391.08 µs | 416.79 µs | PASS |
| ironcalc | scrolling-read | D2 | 255.37 µs | 326.07 µs | 363.85 µs | PASS |
| ironcalc | scrolling-read | D3 | 441.34 µs | 562.44 µs | 640.54 µs | PASS |
| ironcalc | writes-single | — | 32.03 µs | 78.26 µs | 197.27 µs | — |
