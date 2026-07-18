---
status: draft
---

# Functional Spec: Scalar Functions Batch (+ TRIM fix)

## 1. Purpose & scope

An **engine-coverage batch**: implement 11 missing common Excel functions in the IronCalc
fork (`scosman/ironcalc`) and fix one TRIM correctness bug. **No UI, no product design** —
this is pure calculation-engine coverage/correctness. It nevertheless gets the full spec
treatment so another agent can implement each function's exact **Excel-compatible contract**
with upstream-style tests, cold.

This document is a **behavior contract**. Its job is to pin down, for every function:
**signature → argument types & coercion → return value → edge cases & errors**, with worked
examples (including edge cases) so implementation and tests are unambiguous. It does **not**
specify *how* the fork registers/dispatches a function or which coercion helpers exist — that
is the architecture step's job. FreeCell's fork already implements **345 of ~506** common Excel
functions, so the dispatch, argument-marshalling, and error-propagation machinery exists; we are
specifying **new entries into an existing system**, not new infrastructure.

### 1.1 In scope — 11 functions + 1 bug fix

| # | Function | One-line contract |
|---|---|---|
| 1 | **SUMPRODUCT** | Sum of element-wise products of equally-shaped arrays |
| 2 | **PROPER** | Title-case: capitalize the first letter of each word, lowercase the rest |
| 3 | **REPLACE** | Replace `num_chars` characters of a string starting at a 1-based position |
| 4 | **CHAR** | Integer code 1–255 → its character |
| 5 | **CODE** | Numeric code of the first character of a string |
| 6 | **CLEAN** | Remove ASCII control characters (0–31) |
| 7 | **DOLLAR** | Format a number as currency **text** (`"$1,234.57"`) |
| 8 | **ADDRESS** | Build a reference string from row/column numbers (`"$A$1"`) |
| 9 | **PERCENTILE.INC** | Inclusive k-th percentile with linear interpolation (+ legacy `PERCENTILE`) |
| 10 | **QUARTILE.INC** | Quartile 0–4, delegating to PERCENTILE.INC (+ legacy `QUARTILE`) |
| 11 | **XMATCH** | Scalar 1-based position of a lookup value (exact / approximate / wildcard) |
| — | **TRIM bug fix** | TRIM must also collapse internal runs of ASCII spaces, not only trim the ends |

### 1.2 Explicitly out of scope (deferred to v1.0)

- **TRANSPOSE** — returns an **array**; it cannot behave correctly without the dynamic-array /
  **spill** capability (a v1.0 project). It is not a scalar function and is excluded here.
- **HYPERLINK (function)** — would compute/display a value but cannot be **clickable** until the
  v1.0 clickable-hyperlinks feature lands; deferred to travel with that feature.

Both are tracked in `project_overview.md` and GAPS.md; neither is implemented in this batch.

---

## 2. Shared contract concerns (apply to every function)

These are the cross-cutting rules. Each per-function section below assumes them and only calls
out deviations.

### 2.1 Argument coercion model

- **Text-consuming functions** (PROPER, REPLACE's `old_text`/`new_text`, CODE, CLEAN, DOLLAR
  never — see below, TRIM) coerce a **numeric or logical** argument to its text form using the
  **General-format** string: a number → its plain decimal text; `TRUE`/`FALSE` → the literals
  `"TRUE"`/`"FALSE"`; a **date/time value** → its **serial number's** text (dates are numbers;
  the *formatted* display date is not used). Text passes through verbatim. This must match the
  value→text coercion the sibling text functions (UPPER, LOWER, LEN, MID, …) already use.
- **Number-consuming functions/arguments** (SUMPRODUCT's arithmetic, REPLACE's `start_num`/
  `num_chars`, CHAR's `number`, DOLLAR's `number`/`decimals`, ADDRESS's `row_num`/`column_num`/
  `abs_num`, PERCENTILE/QUARTILE's `k`/`quart`, XMATCH's mode args) coerce logicals
  (`TRUE`=1, `FALSE`=0) and **numeric-looking text** (`"3"` → 3) to numbers. **Non-numeric text
  → `#VALUE!`.**
- **Integer-valued arguments** (`start_num`, `num_chars`, CHAR `number`, `abs_num`, `quart`,
  XMATCH `match_mode`/`search_mode`, DOLLAR `decimals`) are **truncated toward zero** to an
  integer before use (Excel `INT`-toward-zero on the already-numeric value; e.g. `65.9` → `65`).
- **Logical-valued arguments** (ADDRESS `a1`) treat `0`/`FALSE` as false and any nonzero/`TRUE`
  as true.

### 2.2 Error propagation

- If any argument **is** (or evaluates to) an Excel error value (`#VALUE!`, `#DIV/0!`, `#N/A`,
  `#NUM!`, `#REF!`, `#NAME?`, `#NULL!`), the function returns **that error** unchanged
  (error-first), matching sibling functions — **including** array elements: an error anywhere in
  a SUMPRODUCT/PERCENTILE/QUARTILE/XMATCH array propagates out.
- Bad type coercion yields **`#VALUE!`**; an out-of-domain numeric argument yields **`#NUM!`**;
  a lookup that finds nothing yields **`#N/A`**. Each function's section pins which applies.
- The **first** error encountered under the engine's normal argument-evaluation order propagates
  (do not reorder to prefer one error over another).

### 2.3 Volatility

**None** of these functions are volatile. Each is a pure function of its inputs and must **not**
be registered as recalc-on-every-change (contrast `RAND`, `NOW`, `TODAY`, `OFFSET`, `INDIRECT`).

### 2.4 Case-insensitivity

Excel text comparison is **case-insensitive**. This governs **XMATCH** text and wildcard
matching (`"ABC"` matches `"abc"`). PROPER performs case *mapping* (its up/lower-casing must use
the same Unicode case tables as the engine's existing UPPER/LOWER).

### 2.5 Array-argument acceptance & non-numeric handling

- **SUMPRODUCT, PERCENTILE.INC, QUARTILE.INC, XMATCH** accept **ranges** and **array constants**
  as their array arguments.
- In the **statistical** aggregation functions (PERCENTILE.INC, QUARTILE.INC): **non-numeric
  cells in a range are ignored** — text, blank/empty cells, and logicals in a *range* do not
  count toward `n`. (This matches AVERAGE/MEDIAN/etc.) If **no** numeric values remain → `#NUM!`.
- In **SUMPRODUCT**'s own element-wise multiply, non-numeric array entries (text, blank, logical)
  are treated as **0** (not ignored, not coerced) — see §3.1 for the important consequence.
- **Empty cell** semantics by context: `0` in arithmetic (SUMPRODUCT), *ignored* in statistical
  aggregation (PERCENTILE/QUARTILE), the empty string `""` in text contexts.

---

## 3. Per-function contracts

### 3.1 SUMPRODUCT — sum of element-wise products (highest-value item)

**Signature:** `SUMPRODUCT(array1, [array2], …, [array255])` (1–255 array arguments).

**Behavior.** Multiplies corresponding elements of the given arrays and returns the sum of those
products. A scalar argument is a 1×1 array. A single array argument returns the sum of that array
(each element × 1).

**Dimension rule.** **All arrays must have identical dimensions** (same row count *and* column
count). Excel does **not** broadcast a shorter array. Mismatched dimensions → **`#VALUE!`**.

**Coercion / non-numeric handling — the critical distinction:**

- **Multiple-array form** `SUMPRODUCT(A, B, …)`: SUMPRODUCT performs the element-wise multiply
  itself, and treats every **non-numeric** element (text, blank, **and logical `TRUE`/`FALSE`**)
  as **`0`**. This is why `SUMPRODUCT(range_of_TRUEs)` returns `0`, and why the double-unary
  `--(condition)` idiom is needed to *count* booleans.
- **Single-expression form** `SUMPRODUCT((A=x)*(B))`: here the `*` (array arithmetic) is
  evaluated **by the engine before SUMPRODUCT sees it**. Arithmetic operators **do** coerce
  logicals (`TRUE`×n = n, `FALSE`×n = 0), so the classic boolean idiom works. But because the `*`
  is a real multiply, a **text** operand hit by `*` yields `#VALUE!` (which then propagates).
  SUMPRODUCT then just sums the resulting numeric array.

**Errors.** Mismatched dimensions → `#VALUE!`. Any array element that is an error value →
that error. More than 255 arguments → the engine's standard arity error.

**Return:** a single number.

**Worked examples:**

| Expression | Result | Why |
|---|---|---|
| `SUMPRODUCT({1,2,3},{4,5,6})` | `32` | 1·4 + 2·5 + 3·6 |
| `SUMPRODUCT({1,2,3})` | `6` | single array = plain sum |
| `SUMPRODUCT((A1:A3="x")*(B1:B3))` with A=`{"x","y","x"}`, B=`{10,20,30}` | `40` | (T·10)+(F·20)+(T·30) |
| `SUMPRODUCT(A1:A3,B1:B3)` with A=`{1,"text",3}`, B=`{4,5,6}` | `22` | 1·4 + **0**·5 + 3·6 (text→0) |
| `SUMPRODUCT((A1:A3="x"))` with A=`{"x","y","x"}` | `0` | booleans passed *directly* → 0; must use `--` to count |
| `SUMPRODUCT(--(A1:A3="x"))` with A=`{"x","y","x"}` | `2` | `--` coerces T/F → 1/0 |
| `SUMPRODUCT({1,2,3},{4,5})` | `#VALUE!` | dimension mismatch |

Empty cells inside an array contribute `0` in the multiply form (same as text→0).

---

### 3.2 PROPER — title-case text

**Signature:** `PROPER(text)`.

**Behavior.** Returns `text` with the **first letter of each word capitalized** and **every other
letter lowercased**. A "word boundary" is **any non-letter character**: a letter is capitalized
iff it is the **first character of the string** or **immediately follows a non-letter** (space,
digit, punctuation, symbol). Non-letter characters pass through unchanged.

**Coercion.** `text` follows the §2.1 text-consuming rule (number/logical/date coerced to text
first). Empty string → empty string.

**Case mapping.** Upper/lower-casing must use the **same Unicode case tables as the engine's
UPPER/LOWER** (so accented letters case-fold consistently). "Letter" = a Unicode alphabetic
character.

**Errors.** An error argument propagates.

**Worked examples:**

| Expression | Result |
|---|---|
| `PROPER("john smith")` | `John Smith` |
| `PROPER("JOHN SMITH")` | `John Smith` |
| `PROPER("e-mail address")` | `E-Mail Address` (the `m` follows `-`) |
| `PROPER("o'brien")` | `O'Brien` (the `b` follows `'`) |
| `PROPER("2-way 76street")` | `2-Way 76Street` (letters after `-` and after digit are capitalized) |
| `PROPER("")` | `""` |

---

### 3.3 REPLACE — replace part of a string by position

**Signature:** `REPLACE(old_text, start_num, num_chars, new_text)`.

**Behavior.** Removes `num_chars` characters from `old_text` beginning at the **1-based** position
`start_num`, and inserts `new_text` in their place. The result is:
`(chars 1 … start_num-1) + new_text + (chars start_num+num_chars … end)`.

**Arguments & coercion:**

- `old_text`, `new_text` — text-consuming coercion (§2.1).
- `start_num` — number, truncated to integer. **Must be ≥ 1**; `< 1` → **`#VALUE!`**.
- `num_chars` — number, truncated to integer. **Must be ≥ 0**; `< 0` → **`#VALUE!`**.

**Boundary behaviors:**

- `num_chars = 0` → **pure insertion** (nothing removed; `new_text` inserted at `start_num`).
- `start_num` **beyond the length** of `old_text` → `new_text` is **appended** at the end.
- `num_chars` extending past the end of `old_text` → everything from `start_num` to the end is
  removed (no error).

**Character counting.** Counts by **Unicode scalar value (character)**. *(Excel counts UTF-16
code units, so astral-plane characters differ; see §6 open item O-7 — not expected to bite common
sheets.)*

**Errors.** `start_num < 1` or `num_chars < 0` → `#VALUE!`; non-numeric `start_num`/`num_chars`
text → `#VALUE!`; any error argument propagates.

**Return:** text.

**Worked examples:**

| Expression | Result | Why |
|---|---|---|
| `REPLACE("abcdefg",3,2,"XY")` | `abXYefg` | remove `cd`, insert `XY` |
| `REPLACE("2009",3,2,"10")` | `2010` | |
| `REPLACE("Hello",6,0," World")` | `Hello World` | insertion at end (start=len+1, num=0) |
| `REPLACE("Hello",1,0,">>")` | `>>Hello` | insertion at start |
| `REPLACE("abc",2,10,"X")` | `aX` | num_chars past end trims to end |
| `REPLACE("abc",10,2,"XYZ")` | `abcXYZ` | start past end → append |
| `REPLACE("abc",0,1,"X")` | `#VALUE!` | start_num < 1 |
| `REPLACE("abc",2,-1,"X")` | `#VALUE!` | num_chars < 0 |

---

### 3.4 CHAR — code → character

**Signature:** `CHAR(number)`.

**Behavior.** Returns the single character whose code is `number`, for `number` in **1–255**.

**Coercion.** `number` coerced to a number then **truncated to integer** (`CHAR(65.9)` → `A`).

**Character set (open item O-1).** Codes **1–127** map to **ASCII** (= the corresponding Unicode
code point). Codes **128–255**: Excel-Windows uses the **Windows-1252** code page. *Recommended:*
map 128–255 via **Windows-1252** for Excel fidelity (`CHAR(128)` → `€`, `CHAR(169)` → `©`), and
have **CODE** be its exact inverse. *Flagged* because the fork may currently use raw Unicode code
points for 128–255 — the architecture step must confirm which the fork's existing CHAR/UNICHAR
family uses and stay consistent.

**Errors.** After truncation, `number < 1` or `number > 255` → **`#VALUE!`** (`CHAR(0)` and
`CHAR(256)` both error). Non-numeric text → `#VALUE!`. Error argument propagates.

**Return:** a one-character string.

**Worked examples:**

| Expression | Result |
|---|---|
| `CHAR(65)` | `A` |
| `CHAR(97)` | `a` |
| `CHAR(33)` | `!` |
| `CHAR(9)` | tab (U+0009) |
| `CHAR(65.9)` | `A` (truncated to 65) |
| `CHAR(0)` | `#VALUE!` |
| `CHAR(256)` | `#VALUE!` |

---

### 3.5 CODE — first character → code

**Signature:** `CODE(text)`.

**Behavior.** Returns the numeric code of the **first character** of `text`. The **exact inverse
of CHAR** over 1–255 (`CODE(CHAR(n)) = n`). All characters after the first are ignored.

**Coercion.** `text` follows §2.1 (number/logical coerced to text first — `CODE(123)` reads the
first char `"1"` → 49).

**Character set (open item O-1).** Same code-page decision as CHAR: 1–127 = ASCII; 128–255 via
the same mapping CHAR uses (inverse consistency). For a first character **outside** the supported
set, follow Excel's fallback (Windows Excel returns `63` = `?` for characters it can't encode);
the architecture step confirms the fork's behavior.

**Errors.** **Empty string `""` → `#VALUE!`.** Error argument propagates.

**Return:** a number.

**Worked examples:**

| Expression | Result |
|---|---|
| `CODE("A")` | `65` |
| `CODE("abc")` | `97` (first char only) |
| `CODE(" ")` | `32` |
| `CODE("!")` | `33` |
| `CODE("")` | `#VALUE!` |
| `CODE(CHAR(200))` | `200` (round-trip) |

---

### 3.6 CLEAN — strip non-printable characters

**Signature:** `CLEAN(text)`.

**Behavior.** Removes every character whose code is in the **ASCII control range 0–31**
(`0x00`–`0x1F`) — inclusive of NUL(0), tab(9), line feed(10), carriage return(13), etc. **Every
character with code ≥ 32 is kept unchanged**, including DEL (127) and all high/Unicode characters.
*(This is the canonical/legacy CLEAN definition. It does not strip 127 or the Unicode "extra"
non-printables 129/141/143/144/157 — confirmed as the documented contract; see §6 O-2.)*

**Coercion.** `text` follows §2.1.

**Errors.** Error argument propagates. Empty → empty.

**Return:** text.

**Worked examples:**

| Expression | Result | Why |
|---|---|---|
| `CLEAN(CHAR(9)&"text"&CHAR(10))` | `text` | tab + LF removed |
| `CLEAN("Hello"&CHAR(7)&"World")` | `HelloWorld` | bell(7) removed |
| `CLEAN(CHAR(31)&"x"&CHAR(0))` | `x` | 31 and 0 removed |
| `CLEAN("normal text")` | `normal text` | nothing to remove |
| `CLEAN("keep"&CHAR(127))` | `keep` + DEL(127) | **127 is NOT removed** (≥ 32) |
| `CLEAN(CHAR(160)&"y")` | NBSP(160) + `y` | 160 kept (CLEAN ≠ TRIM) |

---

### 3.7 DOLLAR — number → currency text

**Signature:** `DOLLAR(number, [decimals])`; `decimals` default **2**.

**Behavior.** Returns `number` formatted as **currency text** using the currency number format:
the currency symbol, a comma thousands-separator, `decimals` fractional digits (rounded), and
**parentheses around negatives** (no minus sign) — i.e. the format
`"$"#,##0.00_);("$"#,##0.00)` at the default. The **result is text**, not a number.

**Arguments & coercion:**

- `number` — numeric coercion; non-numeric text → **`#VALUE!`**.
- `decimals` — numeric, **truncated to integer**.
  - `> 0` → round to that many decimal places.
  - `= 0` → integer dollars, no decimal point.
  - `< 0` → round to the **left** of the decimal point, i.e. to the nearest `10^|decimals|`
    (`decimals = -2` rounds to the nearest 100). If rounding zeroes out the whole magnitude the
    result is `"$0"`.

**Rounding.** Round **half away from zero** (Excel ROUND), then format.

**Locale / currency symbol (open item O-3).** *Recommended default:* **en-US** — `$` symbol,
`,` thousands separator, `.` decimal, parenthesized negatives. Full locale/currency-symbol
support is **flagged** as a later item; v0.5 should match the engine's current locale posture
(hardwired en-US if that is what siblings like TEXT/FIXED assume).

**Errors.** Non-numeric `number` → `#VALUE!`; error argument propagates.

**Return:** text.

**Worked examples (en-US):**

| Expression | Result |
|---|---|
| `DOLLAR(1234.567)` | `$1,234.57` |
| `DOLLAR(1234.567,1)` | `$1,234.6` |
| `DOLLAR(-1234.567,2)` | `($1,234.57)` |
| `DOLLAR(99.9,0)` | `$100` |
| `DOLLAR(12345.67,-2)` | `$12,300` |
| `DOLLAR(50,-3)` | `$0` (50 rounds to nearest 1000 = 0) |
| `DOLLAR(0)` | `$0.00` |

---

### 3.8 ADDRESS — build a reference string

**Signature:** `ADDRESS(row_num, column_num, [abs_num], [a1], [sheet_text])`.

**Behavior.** Returns a **text** cell reference built from the numeric `row_num` and
`column_num`. It does **not** validate that the cell/sheet exists — it only assembles a string.

**Arguments & coercion:**

- `row_num`, `column_num` — numeric, truncated to integer. Valid range: `row_num` **1…1,048,576**,
  `column_num` **1…16,384**. Out of range (`< 1` or above the max) → **`#VALUE!`**.
- `abs_num` — integer, **default 1**; controls absolute/relative markers:

  | `abs_num` | Meaning | A1 form of `ADDRESS(2,3,abs_num)` |
  |---|---|---|
  | 1 | absolute row, absolute column (default) | `$C$2` |
  | 2 | absolute row, relative column | `C$2` |
  | 3 | relative row, absolute column | `$C2` |
  | 4 | relative row, relative column | `C2` |

  `abs_num` outside 1–4 → **`#VALUE!`**.
- `a1` — logical, **default TRUE**. `TRUE`/omitted → **A1** style; `FALSE`/`0` → **R1C1** style.
- `sheet_text` — optional worksheet name to prefix. If present, the result is
  `sheet_text!<ref>`. **Quoting:** wrap the name in single quotes (doubling any internal `'`)
  when it is not a bare-legal name — i.e. it contains any character outside `[A-Za-z0-9_.]`,
  starts with a digit, or otherwise wouldn't parse as an unquoted sheet token. A simple name is
  left unquoted. *(Exact quoting predicate flagged O-4; the common cases below are firm.)*

**Column-number → letters.** `1→A, 26→Z, 27→AA, 702→ZZ, 703→AAA, 16384→XFD`.

**R1C1 style (open item O-5).** When `a1 = FALSE`, absolute markers become plain `R<row>C<col>`
and relative markers become bracketed offsets:

| `abs_num` | R1C1 form of `ADDRESS(2,3,abs_num,FALSE)` |
|---|---|
| 1 | `R2C3` |
| 2 | `R2C[3]` |
| 3 | `R[2]C3` |
| 4 | `R[2]C[3]` |

*Recommended:* implement R1C1 as specified (it is small and deterministic). *Flagged* because
FreeCell is A1-centric — the architecture step decides whether v0.5 ships full R1C1 or a
temporary degrade. **A1 (the default) must be fully correct regardless.**

**Errors.** `row_num`/`column_num` out of range, or `abs_num` outside 1–4 → `#VALUE!`; error
argument propagates.

**Return:** text.

**Worked examples:**

| Expression | Result |
|---|---|
| `ADDRESS(1,1)` | `$A$1` |
| `ADDRESS(2,3)` | `$C$2` |
| `ADDRESS(2,3,2)` | `C$2` |
| `ADDRESS(2,3,3)` | `$C2` |
| `ADDRESS(2,3,4)` | `C2` |
| `ADDRESS(2,3,1,FALSE)` | `R2C3` |
| `ADDRESS(2,3,4,FALSE)` | `R[2]C[3]` |
| `ADDRESS(1,1,1,TRUE,"Sheet1")` | `Sheet1!$A$1` |
| `ADDRESS(1,1,1,TRUE,"My Sheet")` | `'My Sheet'!$A$1` |
| `ADDRESS(1,16384)` | `$XFD$1` |
| `ADDRESS(0,1)` | `#VALUE!` |

*Edge:* an **empty-string** `sheet_text` still prefixes `!` in Excel (`ADDRESS(1,1,1,TRUE,"")` →
`!$A$1`); flagged O-4 for confirmation.

---

### 3.9 PERCENTILE.INC — inclusive percentile

**Signature:** `PERCENTILE.INC(array, k)`. Legacy alias: **`PERCENTILE(array, k)`** — identical
behavior (§4 covers alias wiring).

**Behavior.** Returns the value at the **k-th percentile** of the numeric values in `array`,
`k` **inclusive** in `[0, 1]`, using **linear interpolation between closest ranks** (the R-7 /
Excel `PERCENTILE.INC` method):

1. Collect the numeric values of `array` (ignore non-numeric per §2.5) and sort ascending into
   `v[0…n-1]`.
2. Let `idx = k · (n − 1)` (0-based). Let `lo = floor(idx)`, `frac = idx − lo`.
3. Result = `v[lo] + frac · (v[lo+1] − v[lo])` (and `= v[lo]` when `frac = 0` or `lo = n−1`).

So `k = 0` → min, `k = 1` → max, `k = 0.5` → median.

**Arguments & coercion:**

- `array` — a range or array constant; non-numeric entries ignored (§2.5).
- `k` — numeric; non-numeric text → `#VALUE!`.

**Errors.** `k < 0` or `k > 1` → **`#NUM!`**. **No numeric values** in `array` (empty/all
non-numeric) → **`#NUM!`**. Any error element propagates.

**Return:** a number.

**Worked examples** (`array = {1,2,3,4}`, `n = 4`):

| Expression | Result | Why |
|---|---|---|
| `PERCENTILE.INC({1,2,3,4},0)` | `1` | min |
| `PERCENTILE.INC({1,2,3,4},1)` | `4` | max |
| `PERCENTILE.INC({1,2,3,4},0.5)` | `2.5` | idx=1.5 → 2 + 0.5·(3−2) |
| `PERCENTILE.INC({1,2,3,4},0.25)` | `1.75` | idx=0.75 → 1 + 0.75·(2−1) |
| `PERCENTILE.INC({1,2,3,4},0.75)` | `3.25` | idx=2.25 → 3 + 0.25·(4−3) |
| `PERCENTILE.INC({5},0.3)` | `5` | n=1 → idx=0 always |
| `PERCENTILE.INC({1,2,3,4},1.1)` | `#NUM!` | k out of range |
| `PERCENTILE.INC({1,2,3,4},-0.1)` | `#NUM!` | k out of range |

---

### 3.10 QUARTILE.INC — quartile

**Signature:** `QUARTILE.INC(array, quart)`. Legacy alias: **`QUARTILE(array, quart)`** —
identical behavior.

**Behavior.** Returns a quartile of `array`. `quart` selects which, delegating to
**PERCENTILE.INC** at the mapped k:

| `quart` | Meaning | Equivalent |
|---|---|---|
| 0 | minimum | `PERCENTILE.INC(array, 0)` |
| 1 | first quartile (25th pct) | `PERCENTILE.INC(array, 0.25)` |
| 2 | median (50th pct) | `PERCENTILE.INC(array, 0.5)` |
| 3 | third quartile (75th pct) | `PERCENTILE.INC(array, 0.75)` |
| 4 | maximum | `PERCENTILE.INC(array, 1)` |

**Arguments & coercion:**

- `array` — range/array; non-numeric ignored (§2.5).
- `quart` — numeric, **truncated to integer**; must be **0–4** after truncation.

**Errors.** `quart < 0` or `quart > 4` (after truncation) → **`#NUM!`**. No numeric values →
**`#NUM!`**. Error element propagates. Because it delegates to PERCENTILE.INC, its interpolation
and edge behavior are identical.

**Return:** a number.

**Worked examples** (`array = {1,2,4,7,8,9,10,12}`, `n = 8`):

| Expression | Result | Why |
|---|---|---|
| `QUARTILE.INC(data,0)` | `1` | min |
| `QUARTILE.INC(data,1)` | `3.5` | k=0.25: idx=1.75 → 2 + 0.75·(4−2) |
| `QUARTILE.INC(data,2)` | `7.5` | k=0.5: idx=3.5 → 7 + 0.5·(8−7) |
| `QUARTILE.INC(data,3)` | `9.25` | k=0.75: idx=5.25 → 9 + 0.25·(10−9) |
| `QUARTILE.INC(data,4)` | `12` | max |
| `QUARTILE.INC(data,5)` | `#NUM!` | out of range |
| `QUARTILE.INC(data,-1)` | `#NUM!` | out of range |

---

### 3.11 XMATCH — position of a lookup value (scalar)

**Signature:** `XMATCH(lookup_value, lookup_array, [match_mode], [search_mode])`.
`match_mode` default **0**; `search_mode` default **1**.

**Behavior.** Returns the **1-based relative position** of `lookup_value` within `lookup_array`.
**It returns a single scalar number — no spill / dynamic array is needed** (unlike XLOOKUP's
return value). `lookup_array` must be **one-dimensional** (a single row or single column); a 2-D
array → **`#VALUE!`**.

**`match_mode`:**

| `match_mode` | Meaning |
|---|---|
| 0 (default) | **Exact** match; not found → `#N/A` |
| −1 | Exact, or the **next smaller** item (largest value ≤ `lookup_value`) |
| 1 | Exact, or the **next larger** item (smallest value ≥ `lookup_value`) |
| 2 | **Wildcard** match — `*`, `?`, `~` have special meaning in `lookup_value` |

Unlike legacy MATCH, `match_mode −1/1` with a **linear** search do **not** require the array to be
sorted — XMATCH scans and returns the best qualifying element.

**`search_mode`:**

| `search_mode` | Meaning |
|---|---|
| 1 (default) | First-to-last |
| −1 | Last-to-first (returns the **last** match on ties) |
| 2 | **Binary search**, array assumed sorted **ascending** |
| −2 | **Binary search**, array assumed sorted **descending** |

Binary modes assume the stated sort order; on an unsorted array the result is **undefined**
(Excel does not validate — it may return a wrong position or `#N/A`). This matches Excel; do not
add validation.

**Matching semantics:**

- **Type-sensitive:** a number `5` does not match the text `"5"`.
- **Text comparison is case-insensitive** (§2.4): `"ABC"` matches `"abc"`.
- **Ties:** when several elements match equally, `search_mode` direction picks first (`1`/`2`) or
  last (`−1`/`−2`).
- **Wildcard (`match_mode 2`):** `?` = any one character, `*` = any run (incl. empty), `~` escapes
  a following `?`/`*`/`~`. Case-insensitive. A `lookup_value` with no wildcard behaves as exact
  text match.

**Coercion.** `match_mode`, `search_mode` — numeric, truncated to integer. `lookup_value` is
compared as its own type.

**Errors.** Not found → **`#N/A`**. 2-D `lookup_array` → `#VALUE!`. Invalid mode: a `match_mode`/
`search_mode` that coerces to a number but is **outside its valid set** → **`#N/A`** *(recommended
— confirm against Excel, O-6)*; a mode argument that is **non-numeric text** → `#VALUE!`. Error
argument or error element propagates.

**Return:** a number (position), or `#N/A`.

**Worked examples** (`lookup_array = {10,20,30,40,50}` unless noted):

| Expression | Result | Why |
|---|---|---|
| `XMATCH(30, arr)` | `3` | exact |
| `XMATCH(35, arr)` | `#N/A` | exact, not found |
| `XMATCH(35, arr, -1)` | `3` | next smaller = 30 @ pos 3 |
| `XMATCH(35, arr, 1)` | `4` | next larger = 40 @ pos 4 |
| `XMATCH(20, {10,20,20,30}, 0, 1)` | `2` | first match |
| `XMATCH(20, {10,20,20,30}, 0, -1)` | `3` | last match (reverse) |
| `XMATCH("ban*", {"apple","banana","cherry"}, 2)` | `2` | wildcard |
| `XMATCH(30, arr, 0, 2)` | `3` | binary, ascending-sorted |
| `XMATCH(99, arr, 0)` | `#N/A` | not found |

---

## 4. TRIM bug fix

**Signature:** `TRIM(text)` (already exists in the fork; this is a **correctness fix**, not a new
function).

**Bug.** The fork's TRIM currently removes only **leading and trailing** spaces. It does **not**
collapse **internal** runs of multiple spaces to a single space.

**Corrected behavior (Excel-compatible).** TRIM removes leading and trailing **ASCII space
(0x20)** characters **and** replaces each internal run of **two or more** ASCII spaces with a
**single** ASCII space. Equivalently: split on runs of `0x20`, drop empty leading/trailing tokens,
join the remaining tokens with a single `0x20`.

**Precise whitespace scope — only 0x20.** TRIM operates **exclusively on the ASCII space
character `0x20`**. It does **not** trim or collapse tab (`0x09`), newline (`0x0A`/`0x0D`),
non-breaking space (`0xA0` / U+00A0), or any other Unicode whitespace — those pass through
untouched. This must be preserved by the fix (a naive "collapse all whitespace" implementation
would be **wrong** and would regress Excel compatibility).

**Coercion.** `text` follows §2.1. Error argument propagates. All-spaces input → `""`.

**Before/after test cases (these prove the fix):**

| Input | Buggy (current) output | Correct (fixed) output |
|---|---|---|
| `"  hello   world  "` | `"hello   world"` | `"hello world"` |
| `"a    b"` | `"a    b"` | `"a b"` |
| `"no  extra"` | `"no  extra"` | `"no extra"` |
| `"single"` | `"single"` | `"single"` |
| `"   "` (all spaces) | `""` | `""` |
| `"a"&CHAR(9)&CHAR(9)&"b"` (tabs) | `"a\t\tb"` | `"a\t\tb"` (tabs **not** collapsed) |
| `CHAR(160)&"x"&CHAR(160)` (NBSP) | unchanged | unchanged (NBSP **not** trimmed) |

The rows for `"a    b"` / `"no  extra"` are the load-bearing regression cases (ends already
correct, interior previously not collapsed). The tab and NBSP rows prove the **0x20-only** scope.

---

## 5. Delivery constraint — one fix = one branch = one PR

This batch is implemented in the **IronCalc fork** (`scosman/ironcalc`) per the standing fork
policy (`CLAUDE.md`; `specs/projects/ironcalc-upstreaming/implementation_plan.md` §Operating
model). It is **not** a single bundled change:

- **Each function ships as its own `fix/<slug>` branch off `main`**, with upstream-style tests,
  producing **one focused single-feature upstream PR** — e.g. `fix/sumproduct`, `fix/proper`,
  `fix/replace`, `fix/char`, `fix/code`, `fix/clean`, `fix/dollar`, `fix/address`,
  `fix/percentile-inc`, `fix/quartile-inc`, `fix/xmatch`, and `fix/trim-collapse`.
- **Legacy aliases ride with their `.INC` sibling in the same branch:** `PERCENTILE` is
  registered alongside `PERCENTILE.INC` in `fix/percentile-inc`; `QUARTILE` alongside
  `QUARTILE.INC` in `fix/quartile-inc` (one implementation, two registered names — the alias is
  part of the same single feature, not a separate PR).
- All branches are merged into the **`freecell-fixes`** integration branch, which FreeCell's
  `[patch.crates-io]` pins; FreeCell picks the functions up with no FreeCell-side code.
- The agent **cannot open upstream `ironcalc/IronCalc` PRs**. For each ready branch it **prepares**
  the PR for the owner to open in one click: a **compare link**
  `https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/<slug>` plus a
  suggested **title** and **body** (minimal repro + the tests). The **owner** opens the upstream
  PRs.
- Before branching any `fix/*`, confirm the function isn't **already present** in `freecell-fixes`
  or upstream (`git merge-base --is-ancestor` + a `git grep` for the name at the pinned rev), per
  the operating-model gotchas.

The **granularity** of branches (e.g. whether CHAR + CODE, being an inverse pair, share one branch)
is confirmed in the architecture step — see O-7. The default is **one function = one branch**.

---

## 6. Open items / decisions to confirm

These are the ambiguous points a next-round (architecture) decision must close. Each carries a
recommended default so implementation is never blocked.

- **O-1 — CHAR/CODE character set for 128–255.** *Recommended:* **Windows-1252** (Excel-Windows
  fidelity), CODE the exact inverse. Confirm against the fork's existing CHAR/UNICHAR family so the
  two agree; fallback is raw Unicode code points if that is what the fork already uses.
- **O-2 — CLEAN removal set.** *Recommended:* **ASCII 0–31 only** (the canonical/legacy contract;
  127 and Unicode extras kept). Only widen if a real file needs modern Excel's extended set.
- **O-3 — DOLLAR locale/currency symbol.** *Recommended:* **en-US `$`** hardwired for v0.5,
  matching sibling formatting functions; full locale support flagged as a later item.
- **O-4 — ADDRESS `sheet_text` quoting predicate + empty-string prefix.** *Recommended:* quote
  (single quotes, internal `'`→`''`) when the name is not `[A-Za-z0-9_.]`-only or starts with a
  digit; empty `sheet_text` still prefixes `!`. Confirm the exact predicate against Excel; the
  common cases in §3.8 are firm.
- **O-5 — ADDRESS R1C1 (`a1 = FALSE`).** *Recommended:* implement R1C1 per §3.8 (small,
  deterministic). Alternative: v0.5 ships A1 only and degrades R1C1 — decide in architecture. A1
  must be fully correct either way.
- **O-6 — XMATCH invalid-mode error code.** *Recommended:* out-of-set numeric mode → `#N/A`;
  non-numeric text mode → `#VALUE!`. Confirm the exact Excel code.
- **O-7 — REPLACE character indexing** (Unicode scalar vs UTF-16 code unit) and the **CHAR+CODE
  branch-pairing** question are minor; the architecture step confirms both. Default: index by
  Unicode character; one function = one branch.

---

## 7. Notes for the architecture step (non-binding pointers)

- FreeCell already has function dispatch + argument coercion (345 functions live); these are new
  registry entries plus (for TRIM) a body fix — no new infrastructure.
- **PERCENTILE.INC / QUARTILE.INC should share the percentile core** — QUARTILE.INC is a thin
  `quart → k` mapping over PERCENTILE.INC. Whether they live in one module and how the `.INC` name
  and the legacy alias are both registered are architecture decisions.
- **XMATCH scope for v0.5** — recommend implementing all four `match_mode`s and all four
  `search_mode`s (a partial XMATCH silently returns wrong positions, which is worse than a missing
  function); binary-search and wildcard scope is the main scoping call.
- **Per-function test-fixture pattern** — each `fix/*` branch must carry upstream-style tests
  (the fork's own `cargo test` + `make lint`). The architecture step should pin the fixture shape
  the upstream repo expects so the prepared PRs are review-ready.
