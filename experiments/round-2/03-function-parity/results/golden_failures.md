# SP3 golden-file results — IronCalc 0.7.1 vs known-correct Excel

**133/138 passed = 96.4%.** GATE (>=~100 cases): MET.

## Per-category pass rate

| category | passed | total | rate |
|----------|--------|-------|------|
| array-spill | 7 | 8 | 87.5% |
| coercion | 27 | 28 | 96.4% |
| common | 50 | 51 | 98.0% |
| dates | 18 | 20 | 90.0% |
| error-propagation | 31 | 31 | 100.0% |

## Itemized failures (5)

| id | formula | expected | actual | reason |
|----|---------|----------|--------|--------|
| `coerce-sumproduct-bools` | `=SUMPRODUCT((A1:A3>1)*1)` | 2 (±0) | error #NAME? | expected a value, got error #NAME? |
| `date-serial-1900-01-01` | `=DATE(1900,1,1)` | 1 (±0) | 2 | number outside tolerance |
| `date-serial-epoch-leapbug` | `=DATE(1900,2,28)` | 59 (±0) | 60 | number outside tolerance |
| `text-trim` | `=TRIM("  a  b  ")` | "a b" | "a  b" | expected text |
| `array-transpose-index` | `=INDEX(TRANSPOSE(A1:C1),2)` | 20 (±0) | error #NAME? | expected a value, got error #NAME? |
