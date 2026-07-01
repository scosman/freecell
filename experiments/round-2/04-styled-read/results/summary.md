# SP4 — styled viewport read: recorded summary

IronCalc 0.7.1. Value + style (`get_style_for_cell`) per visible cell, at Excel-max (1048576x16384). Phase-1 value-only baseline p99 = 392.42 µs. Gate: p99 < 2.00 ms.

| window | cells | value+style p50 | value+style p99 | value-only p99 | added style p99 | GATE p99<2ms |
|--------|-------|-----------------|-----------------|----------------|-----------------|--------------|
| viewport | 1800 | 711.24 µs | 1.20 ms | 135.80 µs | 1.06 ms | PASS |
| overscan | 10000 | 3.80 ms | 6.97 ms | 558.09 µs | 6.41 ms | FAIL |

## Crossover sweep (largest window under 2 ms p99, at Excel-max)

| cells | value+style p99 | GATE p99<2ms |
|-------|-----------------|--------------|
| 1200 | 512.38 µs | PASS |
| 2000 | 914.59 µs | PASS |
| 2475 | 1.08 ms | PASS |
| 2750 | 1.57 ms | PASS |
| 3000 | 1.29 ms | PASS |
| 3500 | 1.51 ms | PASS |
| 4800 | 2.12 ms | FAIL |
| 7000 | 3.21 ms | FAIL |

**Crossover:** largest window under 2 ms p99 ≈ **3500 cells**; first failing ≈ **4800 cells**.
