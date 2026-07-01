# SP3 coverage matrix — IronCalc 0.7.1 vs canonical Excel list

Overall: **345/506 = 68.2%** of the canonical Excel catalog is registered in IronCalc 0.7.1.

## By importance

| importance | supported | total | coverage |
|-----------|-----------|-------|----------|
| common | 154 | 189 | 81.5% |
| obscure | 191 | 317 | 60.3% |

## By category

| category | supported | total | coverage |
|----------|-----------|-------|----------|
| compatibility | 0 | 38 | 0.0% |
| cube | 0 | 7 | 0.0% |
| database | 12 | 12 | 100.0% |
| datetime | 25 | 25 | 100.0% |
| dynamic-array | 0 | 17 | 0.0% |
| engineering | 54 | 54 | 100.0% |
| financial | 28 | 55 | 50.9% |
| information | 20 | 21 | 95.2% |
| logical | 11 | 19 | 57.9% |
| lookup | 14 | 22 | 63.6% |
| math | 71 | 79 | 89.9% |
| stat | 88 | 111 | 79.3% |
| text | 22 | 43 | 51.2% |
| web | 0 | 3 | 0.0% |

## Missing COMMON functions (35) — the off-ramp set

- `ADDRESS` (lookup)
- `CHAR` (text)
- `CHOOSECOLS` (dynamic-array)
- `CHOOSEROWS` (dynamic-array)
- `CLEAN` (text)
- `CODE` (text)
- `DOLLAR` (text)
- `DROP` (dynamic-array)
- `FILTER` (dynamic-array)
- `HSTACK` (dynamic-array)
- `HYPERLINK` (lookup)
- `LET` (logical)
- `MODE` (compatibility)
- `MODE.SNGL` (stat)
- `PERCENTILE` (compatibility)
- `PERCENTILE.INC` (stat)
- `PROPER` (text)
- `QUARTILE` (compatibility)
- `QUARTILE.INC` (stat)
- `RANDARRAY` (dynamic-array)
- `RANK` (compatibility)
- `REPLACE` (text)
- `SORT` (dynamic-array)
- `SORTBY` (dynamic-array)
- `STDEV` (compatibility)
- `STDEVP` (compatibility)
- `SUMPRODUCT` (math)
- `TAKE` (dynamic-array)
- `TEXTSPLIT` (dynamic-array)
- `TRANSPOSE` (lookup)
- `UNIQUE` (dynamic-array)
- `VAR` (compatibility)
- `VARP` (compatibility)
- `VSTACK` (dynamic-array)
- `XMATCH` (lookup)
