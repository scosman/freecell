# Sub-project C — recorded benchmark summary

Machine-readable per-run JSON lives in `formualizer/` and `ironcalc/` alongside this file. Regenerate with each engine crate's `scenarios` binary (see `../findings.md`).

| engine | scenario | design | p50 | p99 | max | verdict |
|--------|----------|--------|-----|-----|-----|---------|
| ironcalc | scrolling-read | D1 | 86.19 µs | 168.91 µs | 168.91 µs | PASS |
| ironcalc | scrolling-read | D2 | 78.63 µs | 108.04 µs | 108.04 µs | PASS |
| ironcalc | scrolling-read | D3 | 154.50 µs | 192.31 µs | 192.31 µs | PASS |
| ironcalc | cascade-visible | D1 | 221.36 µs | 258.12 µs | 258.12 µs | PASS |
| ironcalc | cascade-visible | D2 | 221.48 µs | 248.42 µs | 248.42 µs | PASS |
| ironcalc | cascade-visible | D3 | 214.29 µs | 268.07 µs | 268.07 µs | PASS |
| ironcalc | cascade-1m-chain | — | 210.97 µs | 281.23 µs | 281.23 µs | PASS |
| ironcalc | cascade-fanout | — | 312.78 µs | 363.24 µs | 363.24 µs | PASS |
| ironcalc | writes-single | — | 479 ns | 2.85 µs | 16.22 µs | — |
| ironcalc | memory | — | — | — | — | — |
