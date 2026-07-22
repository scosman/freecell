---
status: complete
---

# Architecture: Scalar Functions Batch (+ TRIM fix)

Technical design for the 11 new scalar functions + the TRIM correctness fix defined in
`functional_spec.md`. **This is entirely fork work** (`scosman/ironcalc`); there is **no
FreeCell-side code** beyond bumping the pin. Each function (or tightly-coupled pair) is one
`fix/<slug>` branch off `main` → one focused upstream PR, integrated onto `freecell-fixes`,
which FreeCell's `[patch.crates-io]` already points at (`app/Cargo.toml` lines 110-112;
current lock rev `81feec4`).

**No new infrastructure.** IronCalc already implements 345 functions, so the function enum +
name parser + dispatch match, the argument-coercion helpers, the `CalcResult`/`Error` types,
the range/array iteration, the wildcard matcher (COUNTIF/MATCH), and the number formatter
(TEXT/FIXED) **all exist**. Every item here is **new registry entries + one impl fn (+ tests)**
per function — except TRIM, which is a **body edit** to an existing function.

**Fork symbols are best-inferred, not cloned.** Where an exact fork path/symbol is named it is
marked **[checkpoint]** — confirm it against the fork on the `fix/<slug>` branch before coding.
Clone/branch the fork through the container git-proxy (it routes `scosman/ironcalc`):
`git clone http://local_proxy@127.0.0.1:41729/git/scosman/ironcalc` (port = the `local_proxy@`
port in FreeCell's `git remote -v`; same credential as FreeCell's origin), per
`specs/projects/ironcalc-upstreaming/implementation_plan.md` §Operating model → "Proxy fallback".
`add_repo scosman/ironcalc` is the alternative but needs interactive approval up front.

---

## 0. Resolved decisions (all §6 open items closed here)

Every §6 open item is **decided** below so the coding agent designs nothing. Each carries a
one-line **[checkpoint]** only where it must agree with pre-existing fork behavior.

| # | Decision (locked) | Confirm-against-fork checkpoint |
|---|---|---|
| **O-1** CHAR/CODE 128–255 charset | **Windows-1252**; CODE is CHAR's exact inverse. 1–127 = ASCII/Unicode identity; 0xA0–0xFF = Unicode identity (Latin-1); 0x80–0x9F = the CP1252 table in §3.4; the 5 undefined CP1252 slots (129,141,143,144,157) map to the identity C1 code point (U+0081…). CODE of an unrepresentable first char → `63` (`?`). | The fork already has a **CHAR/UNICHAR/CODE/UNICODE family** (O-1). Confirm whether CHAR/CODE **already exist**; if they use raw Unicode for 128–255, this branch reconciles them to CP1252 (Excel-Windows fidelity) — flag to owner if that would change existing green tests. UNICHAR/UNICODE stay Unicode-based, untouched. |
| **O-2** CLEAN removal set | **ASCII control 0–31 only.** 127 (DEL) and all Unicode "extras" (129/141/143/144/157/160…) are **kept**. | none — CLEAN is new; no interaction. |
| **O-3** DOLLAR locale | **en-US hardwired** (`$`, `,` grouping, `.` decimal, parenthesized negatives). Full locale support is a later item. | Reuse the same formatter path TEXT/FIXED use; confirm it emits **no trailing alignment space** for positives (§3.7). |
| **O-4** ADDRESS `sheet_text` quoting | Quote (single quotes, internal `'`→`''`) when the name contains any char outside `[A-Za-z0-9_.]`, **or** starts with a digit; otherwise leave unquoted. An **empty** `sheet_text` still emits the `!` prefix (`ADDRESS(1,1,1,TRUE,"")` → `!$A$1`). | Confirm the fork has no existing sheet-name-quoting helper to reuse; if it does (used by reference serialization), reuse it. |
| **O-5** ADDRESS R1C1 (`a1=FALSE`) | **Implement FULL R1C1** per §3.8 (owner-resolved — do **not** degrade). A1 (default) must also be fully correct. | none — pure string assembly. |
| **O-6** XMATCH invalid-mode error | Numeric mode **outside** its valid set → `#N/A`; **non-numeric text** mode → `#VALUE!`. | none. |
| **O-7** REPLACE indexing + CHAR/CODE pairing | REPLACE indexes by **Unicode scalar (`char`)**, matching MID/LEFT/RIGHT. CHAR+CODE **ship as one branch** `fix/char-code` (§4). | Confirm MID/LEFT/RIGHT index by `char` in the fork (they do) so REPLACE is consistent. |
| **XMATCH scope** (§7) | **Implement ALL four `match_mode`s (0/−1/1/2) AND all four `search_mode`s (1/−1/2/−2)** incl. binary search + wildcard (owner-resolved). | Reuse MATCH's comparison + the fork's existing wildcard matcher (§3.11). |

---

## 1. Where it lives in the fork (best-inferred layout + [checkpoint]s)

Adding a function to IronCalc is a fixed 5-step edit; TRIM is step 4 only. The paths below are
best-inferred from IronCalc's known structure — **[checkpoint]** each on the branch.

- **Function enum + name mapping** — `base/src/functions/mod.rs` **[checkpoint]**:
  `pub enum Function { … }` (add a variant per new name), the name↔variant mapping used by the
  parser (`Function::from_name`/`to_string` or equivalent — the parser already accepts **dotted
  names** like `NORM.DIST`, so `PERCENTILE.INC`/`QUARTILE.INC` parse for free), and the central
  dispatch `impl Model { fn evaluate_function(&mut self, kind, args, cell) -> CalcResult { match … } }`
  (add one arm per variant → the impl fn).
- **Impl fns by category** — `impl Model { fn fn_<name>(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult }`:
  - text → `base/src/functions/text.rs` (PROPER, REPLACE, CLEAN, **TRIM already here**; CHAR/CODE
    likely here too **[checkpoint]**).
  - statistical → `base/src/functions/statistical.rs` (PERCENTILE.INC/QUARTILE.INC + legacy aliases).
  - math → `base/src/functions/mathematical.rs` (SUMPRODUCT — provides array context; may live in
    its own module **[checkpoint]**).
  - lookup → `base/src/functions/lookup_and_reference.rs` (ADDRESS, XMATCH).
  - DOLLAR → text or a `financial`/`text` module alongside TEXT/FIXED **[checkpoint]**.
- **Argument-coercion helpers** (already exist, reuse — do not add) **[checkpoint on exact names]**:
  `Model::get_number(node, cell) -> Result<f64, CalcResult>` (coerces logical + numeric text; non-numeric
  text → `Err(CalcResult::Error{VALUE…})`), `Model::get_string(node, cell) -> Result<String, CalcResult>`
  (General-format value→text coercion — the same one UPPER/LOWER/MID use, §2.1), `Model::get_boolean(…)`.
  Integer-valued args are `get_number` then `f64::trunc` (toward zero, §2.1).
- **Result + error types** — `CalcResult` (`base/src/calc_result.rs` **[checkpoint]**):
  `Number(f64) | String(String) | Boolean(bool) | Error{error, origin, message} | Range{left,right} |
  EmptyCell | EmptyArg | Array(…)`. `Error` enum (`base/src/expressions/token.rs` **[checkpoint]**):
  `VALUE | DIV | NA | NUM | REF | NAME | NULL | …`. Construct errors the way sibling fns do
  (`CalcResult::new_error(Error::VALUE, cell, msg)` or the direct struct — **[checkpoint]**).
- **Number formatter (DOLLAR)** — `base/src/formatter/format.rs::format_number(value, format_code,
  locale)` **[checkpoint]**, the path TEXT/FIXED already use; en-US `Locale`.
- **Wildcard matcher (XMATCH mode 2)** — the criteria/wildcard helper COUNTIF/SUMIF/MATCH use
  (`?`,`*`,`~`, case-insensitive) — **reuse, do not reimplement** **[checkpoint]**.
- **Volatility list** — wherever the fork enumerates volatile functions (recalc-on-every-change) —
  **do NOT add any of these** (§2.3; all are pure). **[checkpoint]**
- **Tests** — IronCalc's unit-test harness (`base/src/test/` **[checkpoint]**):
  `let mut model = new_empty_model(); model._set("A1", "=SUMPRODUCT({1,2,3},{4,5,6})"); model.evaluate();
  assert_eq!(model._get_text("A1"), "32");` — build a workbook, set a formula, evaluate, assert the
  formatted cell. Errors assert as `"#VALUE!"` etc. Confirm the exact helper names on the branch.

Registration is identical for every function; the rest of this doc specifies each impl fn's body.

---

## 2. The per-function implementation pattern (template = SUMPRODUCT)

SUMPRODUCT is the hardest (array-argument iteration + the boolean-multiply idiom) and is the
template the other 10 follow. Two shared helpers introduced here are reused by the array/stat fns.

### 2.1 Shared helper — read an argument as a value grid

Several functions (SUMPRODUCT; PERCENTILE/QUARTILE via a numeric-collection variant) must
materialize an argument that may be a range, an array constant, an array-valued expression, or a
scalar into a uniform 2-D grid. Define (in the same module, or reuse the fork's existing
range-materialization if present **[checkpoint]**):

```
struct ArgGrid { rows: usize, cols: usize, data: Vec<CalcResult> }  // row-major, len = rows*cols

// Err(CalcResult::Error) = an error element to propagate (error-first).
fn eval_arg_as_grid(&mut self, node: &Node, cell) -> Result<ArgGrid, CalcResult>
```

- **Range node** → dims from the range; `data[i]` = each cell's evaluated `CalcResult` in row-major
  order (`self.evaluate_cell(CellReferenceIndex{…})` **[checkpoint]**). An error cell → `Err`.
- **Array constant** → its declared shape + elements.
- **Array-valued expression** (e.g. `(A1:A3="x")*(B1:B3)`) → evaluate the node in **array context**
  so it yields the element-wise array (see §3.1 — the critical SUMPRODUCT case). **[checkpoint:
  confirm the fork's node-in-array-context evaluator — the same machinery `{=A1:A3*B1:B3}` array
  formulas use; if the fork lacks it, the single-expression idiom is the only at-risk path, and
  the multi-array form below still works.]**
- **Scalar node** → a 1×1 grid.

### 2.2 Shared helper — numeric coercion for the multiply

```
// Number → its value; Error → propagate; String/Boolean/EmptyCell/EmptyArg → 0.0
fn to_number_or_zero(v: &CalcResult) -> Result<f64, CalcResult>
```

This single helper is why **both** SUMPRODUCT forms are correct with one code path (§3.1): in the
multi-array form raw booleans/text arrive and become `0`; in the single-expression form the `*`
already coerced them to numbers *before* SUMPRODUCT saw them.

### 2.3 SUMPRODUCT body (`fn_sumproduct`)

```
if args.is_empty() { return CalcResult::Error(NA/arity …) }         // parser normally enforces ≥1
let g0 = self.eval_arg_as_grid(&args[0], cell)?;                     // ? propagates first error
let (rows, cols) = (g0.rows, g0.cols);
let mut grids = vec![g0];
for arg in &args[1..] {
    let g = self.eval_arg_as_grid(arg, cell)?;
    if g.rows != rows || g.cols != cols { return CalcResult::Error(VALUE, …) }  // dimension rule
    grids.push(g);
}
let mut sum = 0.0;
for idx in 0..rows*cols {
    let mut prod = 1.0;
    for g in &grids { prod *= to_number_or_zero(&g.data[idx])?; }   // error element → propagate
    sum += prod;
}
CalcResult::Number(sum)
```

- **Error-first ordering** (§2.2): args are materialized left-to-right, each grid row-major; the
  first `Err` returned by `?` is the first error under normal eval order. Dimension check on arg *k*
  happens after arg *k* materializes, so an error in an earlier arg still wins.
- **Single array** → `grids` has one grid; each `prod` = that element → returns the plain sum.
- **Scalar arg** = 1×1 grid; mixing a 1×1 with an N×M **is** a dimension mismatch → `#VALUE!`
  (Excel does not broadcast).
- **Return:** one `Number`.

**Test vectors** (functional_spec §3.1 — use verbatim): `{1,2,3}·{4,5,6}=32`; `{1,2,3}=6`;
`(A1:A3="x")*(B1:B3)=40` (single-expression idiom); `SUMPRODUCT(A,B)` with A=`{1,"text",3}`,
B=`{4,5,6}` → `22` (text→0); `SUMPRODUCT((A1:A3="x"))=0` (raw booleans→0); `--(A1:A3="x")=2`;
`{1,2,3},{4,5}` → `#VALUE!`; an error element anywhere → that error.

**Every other function follows the same skeleton:** coerce args via the existing helpers (`?` to
propagate errors), compute, return a `CalcResult`. The subsections below give only each body's
specifics + its test vectors.

---

## 3. Per-function contracts (impl notes + test vectors)

### 3.2 PROPER — `fn_proper` (text.rs)

- `let s = self.get_string(&args[0], cell)?;`
- Walk `s.chars()` with `prev_is_letter = false`. For each `c`: if `c.is_alphabetic()` → push
  `c.to_uppercase()` when `!prev_is_letter` else `c.to_lowercase()`, set `prev_is_letter = true`;
  else push `c` unchanged, `prev_is_letter = false`. Use `char::to_uppercase/to_lowercase` — the
  same Unicode case tables UPPER/LOWER use **[checkpoint]**. Return `String`.
- **Tests** (§3.2): `"john smith"`→`John Smith`; `"JOHN SMITH"`→`John Smith`; `"e-mail address"`→
  `E-Mail Address`; `"o'brien"`→`O'Brien`; `"2-way 76street"`→`2-Way 76Street`; `""`→`""`.

### 3.3 REPLACE — `fn_replace` (text.rs)

- `old = get_string(args[0])?; start = get_number(args[1])?.trunc() as i64; num = get_number(args[2])?.trunc() as i64; new = get_string(args[3])?;`
- `start < 1` → `#VALUE!`; `num < 0` → `#VALUE!`.
- `let cs: Vec<char> = old.chars().collect();` (Unicode-scalar indexing, O-7). `let s0 = min((start-1) as usize, cs.len());` (start past end → append). `let e = min(s0 + num as usize, cs.len());`
  `result = cs[..s0].iter().collect::<String>() + &new + &cs[e..].iter().collect::<String>();` → `String`.
- **Tests** (§3.3): `("abcdefg",3,2,"XY")`→`abXYefg`; `("2009",3,2,"10")`→`2010`; `("Hello",6,0," World")`→
  `Hello World`; `("Hello",1,0,">>")`→`>>Hello`; `("abc",2,10,"X")`→`aX`; `("abc",10,2,"XYZ")`→`abcXYZ`;
  `("abc",0,1,"X")`→`#VALUE!`; `("abc",2,-1,"X")`→`#VALUE!`.

### 3.4 / 3.5 CHAR + CODE — `fn_char`, `fn_code` (text.rs, **one branch** `fix/char-code`)

Shared CP1252 mapping for 0x80–0x9F (0xA0–0xFF = Unicode identity; undefined slots → identity):

| code | char | code | char | code | char | code | char |
|---|---|---|---|---|---|---|---|
|128|€ U+20AC|136|ˆ U+02C6|144|*(U+0090)*|152|˜ U+02DC|
|129|*(U+0081)*|137|‰ U+2030|145|' U+2018|153|™ U+2122|
|130|‚ U+201A|138|Š U+0160|146|' U+2019|154|š U+0161|
|131|ƒ U+0192|139|‹ U+2039|147|" U+201C|155|› U+203A|
|132|„ U+201E|140|Œ U+0152|148|" U+201D|156|œ U+0153|
|133|… U+2026|141|*(U+008D)*|149|• U+2022|157|*(U+009D)*|
|134|† U+2020|142|Ž U+017D|150|– U+2013|158|ž U+017E|
|135|‡ U+2021|143|*(U+008F)*|151|— U+2014|159|Ÿ U+0178|

- **CHAR** `fn_char`: `let n = get_number(args[0])?.trunc() as i64;` `n < 1 || n > 255` → `#VALUE!`.
  `1..=127` → `char::from_u32(n as u32)`; `128..=255` → CP1252 table (0x80–0x9F above, else identity).
  Return one-char `String`.
- **CODE** `fn_code`: `let s = get_string(args[0])?;` empty → `#VALUE!`. Take `c = s.chars().next()`.
  `c as u32 <= 127` → that code; else reverse-lookup CP1252 (inverse of CHAR's table); not found →
  `63` (`?`). Return `Number`.
- **Inverse invariant test:** for `n in 1..=255`, `CODE(CHAR(n)) == n` (round-trips including the
  identity-mapped undefined slots).
- **Tests** (§3.4/§3.5): `CHAR(65)`→`A`, `CHAR(97)`→`a`, `CHAR(33)`→`!`, `CHAR(9)`→tab,
  `CHAR(65.9)`→`A`, `CHAR(0)`/`CHAR(256)`→`#VALUE!`, `CHAR(128)`→`€`, `CHAR(169)`→`©`;
  `CODE("A")`→`65`, `CODE("abc")`→`97`, `CODE(" ")`→`32`, `CODE("")`→`#VALUE!`, `CODE(CHAR(200))`→`200`.

### 3.6 CLEAN — `fn_clean` (text.rs)

- `let s = get_string(args[0])?;` → `s.chars().filter(|c| (*c as u32) > 31).collect::<String>()` →
  `String`. Keeps 127, 160, and all Unicode ≥ 32 (O-2).
- **Tests** (§3.6): `CLEAN(CHAR(9)&"text"&CHAR(10))`→`text`; `"Hello"&CHAR(7)&"World"`→`HelloWorld`;
  `CHAR(31)&"x"&CHAR(0)`→`x`; `"normal text"`→unchanged; `"keep"&CHAR(127)`→`keep`+DEL (127 **kept**);
  `CHAR(160)&"y"`→NBSP kept.

### 3.7 DOLLAR — `fn_dollar` (text/financial module beside TEXT/FIXED)

- `let x = get_number(args[0])?;` `let d = if args.len()>1 { get_number(args[1])?.trunc() as i64 } else { 2 };`
- **Round explicitly** (Excel ROUND, half-away-from-zero — reuse the fork's ROUND helper **[checkpoint]**):
  - `d >= 0`: `v = round_half_away(x, d)`; format with `d` decimal places.
  - `d < 0`: `f = 10f64.powi(-d as i32)`; `v = round_half_away(x / f, 0) * f`; format with **0** decimals.
- **Format** `v` as currency text via the formatter TEXT/FIXED use **[checkpoint]**, en-US locale,
  format code `"$"#,##0[.0…d…0];("$"#,##0[.0…d…0])` (dp = `max(d,0)`). **No trailing alignment space**
  on positives — verify against §3.7 (drop any `_)` padding or assemble the string directly if the
  formatter injects it). Negatives → parenthesized, no minus. A rounded-to-zero value → `"$0"`/`"$0.00"`
  (never `-`/`()`; treat `v == 0.0` as non-negative). Return `String`.
- Non-numeric `number` → `#VALUE!`.
- **Tests** (§3.7): `DOLLAR(1234.567)`→`$1,234.57`; `(1234.567,1)`→`$1,234.6`; `(-1234.567,2)`→
  `($1,234.57)`; `(99.9,0)`→`$100`; `(12345.67,-2)`→`$12,300`; `(50,-3)`→`$0`; `(0)`→`$0.00`;
  **add** `(-0.001,2)`→`$0.00` (negative-zero guard).

### 3.8 ADDRESS — `fn_address` (lookup_and_reference.rs)

- `row = get_number(args[0])?.trunc() as i64; col = get_number(args[1])?.trunc() as i64;`
  `row` ∉ `1..=1_048_576` or `col` ∉ `1..=16_384` → `#VALUE!`.
- `abs = if args.len()>2 { get_number(args[2])?.trunc() as i64 } else { 1 };` ∉ `1..=4` → `#VALUE!`.
  `col_abs = abs==1||abs==3; row_abs = abs==1||abs==2;`
- `a1 = if args.len()>3 { get_boolean_or_nonzero(args[3])? } else { true };`
- **Column→letters** (bijective base-26): `while col>0 { r=(col-1)%26; push (b'A'+r) ; col=(col-1)/26 }`
  reversed. (`16384`→`XFD`.)
- **A1** (`a1==true`): `format!("{}{}{}{}", if col_abs {"$"} else {""}, letters, if row_abs {"$"} else {""}, row)`.
- **R1C1** (`a1==false`, **full**, O-5): row part = `R{row}` if `row_abs` else `R[{row}]`; col part =
  `C{col}` if `col_abs` else `C[{col}]`; concat.
- **sheet_text** (if `args.len()>4`): `let name = get_string(args[4])?;` quote per O-4 (single quotes,
  internal `'`→`''`) when `name` has a char outside `[A-Za-z0-9_.]` or starts with a digit; empty name
  still emits the `!`. Result = `format!("{}!{}", maybe_quoted, ref)`.
- **Tests** (§3.8): `(1,1)`→`$A$1`; `(2,3)`→`$C$2`; `(2,3,2)`→`C$2`; `(2,3,3)`→`$C2`; `(2,3,4)`→`C2`;
  `(2,3,1,FALSE)`→`R2C3`; `(2,3,4,FALSE)`→`R[2]C[3]`; `(1,1,1,TRUE,"Sheet1")`→`Sheet1!$A$1`;
  `(1,1,1,TRUE,"My Sheet")`→`'My Sheet'!$A$1`; `(1,16384)`→`$XFD$1`; `(0,1)`→`#VALUE!`;
  **add** `(1,1,1,TRUE,"")`→`!$A$1` (empty-sheet edge, O-4).

### 3.9 / 3.10 PERCENTILE.INC + QUARTILE.INC — statistical.rs (**one branch** `fix/percentile-quartile-inc`)

Shared numeric-collection + percentile core (both fns collect once, then call the core):

```
// collect numeric values from a range/array arg; non-numeric (text/blank/logical) ignored (§2.5);
// error element → Err(that error). Reuse the fork's AVERAGE/MEDIAN numeric iterator if present.
fn collect_numbers(&mut self, node: &Node, cell) -> Result<Vec<f64>, CalcResult>

fn percentile_inc_core(mut v: Vec<f64>, k: f64) -> CalcResult {
    if v.is_empty() { return Error(NUM) }               // no numeric values
    if !(0.0..=1.0).contains(&k) { return Error(NUM) }  // k out of range
    v.sort_by(|a,b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n == 1 { return Number(v[0]) }
    let idx = k * (n as f64 - 1.0);
    let lo = idx.floor() as usize;
    let frac = idx - lo as f64;
    Number(if frac == 0.0 || lo+1 >= n { v[lo] } else { v[lo] + frac*(v[lo+1]-v[lo]) })
}
```

- **PERCENTILE.INC** `fn_percentile_inc`: `let v = collect_numbers(args[0])?; let k = get_number(args[1])?;`
  → `percentile_inc_core(v, k)`. Registered for **both** `PERCENTILE.INC` and legacy `PERCENTILE`
  (same impl, §4).
- **QUARTILE.INC** `fn_quartile_inc`: `let v = collect_numbers(args[0])?; let q = get_number(args[1])?.trunc() as i64;`
  `q ∉ 0..=4` → `#NUM!`; `let k = q as f64 * 0.25;` → `percentile_inc_core(v, k)`. Registered for
  **both** `QUARTILE.INC` and legacy `QUARTILE`. QUARTILE reuses the **same** core (it does not
  re-parse args by calling PERCENTILE) — clean single-branch code.
- **Tests** — PERCENTILE (§3.9, `{1,2,3,4}`): `0`→`1`, `1`→`4`, `0.5`→`2.5`, `0.25`→`1.75`,
  `0.75`→`3.25`; `{5},0.3`→`5`; `1.1`/`-0.1`→`#NUM!`; empty/all-text array → `#NUM!`; error element
  propagates. QUARTILE (§3.10, `{1,2,4,7,8,9,10,12}`): `0`→`1`, `1`→`3.5`, `2`→`7.5`, `3`→`9.25`,
  `4`→`12`; `5`/`-1`→`#NUM!`.

### 3.11 XMATCH — `fn_xmatch` (lookup_and_reference.rs, `fix/xmatch`) — biggest

- `lookup = self.evaluate(&args[0], cell);` error → propagate; **keep its type** (Number/String/Boolean).
- `lookup_array` (args[1]) → materialize to a 1-D `Vec<CalcResult>` with 1-based positions; **2-D**
  (rows>1 AND cols>1) → `#VALUE!`. An error element → propagate.
- `match_mode = if args.len()>2 { get_number(args[2])?.trunc() as i64 } else { 0 };` ∉ `{-1,0,1,2}`
  → `#N/A` (O-6). Non-numeric text mode → `get_number` already returns `#VALUE!`.
- `search_mode = if args.len()>3 { get_number(args[3])?.trunc() as i64 } else { 1 };` ∉ `{-2,-1,1,2}`
  → `#N/A`. Non-numeric → `#VALUE!`.

**Building blocks** (reuse MATCH's + the fork wildcard matcher **[checkpoint]**):

- `cmp(a: &CalcResult, b: &CalcResult) -> Option<Ordering>`: numbers numeric; text
  **case-insensitive** (lowercase both); booleans FALSE<TRUE; **different types → `None`**
  (type-sensitive: `5` vs `"5"` never match). `equals = cmp == Some(Equal)`.
- `wildcard_eq(pattern_text, element_text)`: the fork's existing case-insensitive `?`/`*`/`~` matcher.

**Search strategies:**

- **Exact (match_mode 0)** / **Wildcard (2)** — scan in `search_mode` direction (1/2 = first→last,
  −1/−2 = last→first); first element with `equals` (mode 0) or `wildcard_eq` (mode 2, `lookup` as
  text pattern) → return its 1-based position. None → `#N/A`.
- **Approximate next-smaller (−1)** — scan all; among elements with `cmp(elem, lookup) ≤ Equal`
  (elem ≤ lookup, comparable only) keep the **largest** elem; ties broken by `search_mode` direction
  (first vs last). Return its position, else `#N/A`.
- **Approximate next-larger (1)** — symmetric: smallest elem ≥ lookup.
- **Binary (search_mode 2 asc / −2 desc)** — same match predicates over a **binary traversal** of
  the assumed-sorted array (asc vs desc flips comparisons); exact → find any equal; −1/1 → find the
  boundary neighbor. **No sort validation** (undefined on unsorted, matches Excel — do not add checks).
  **Implementation aid:** on sorted input a binary result must equal the linear result — unit-test
  that equivalence on sorted fixtures.

Return `Number(position)` or `#N/A`.

- **Tests** (§3.11, `arr={10,20,30,40,50}`): `XMATCH(30,arr)`→`3`; `(35,arr)`→`#N/A`; `(35,arr,-1)`→`3`;
  `(35,arr,1)`→`4`; `(20,{10,20,20,30},0,1)`→`2`; `(20,{10,20,20,30},0,-1)`→`3`;
  `("ban*",{"apple","banana","cherry"},2)`→`2`; `(30,arr,0,2)`→`3` (binary asc); `(99,arr,0)`→`#N/A`;
  **add** a 2-D `lookup_array` → `#VALUE!`; an out-of-set mode → `#N/A`; `("ABC",{"abc"})`→`1`
  (case-insensitive); a binary-desc case; the binary≡linear equivalence test.

### 4 (spec §4). TRIM fix — `fn_trim` body edit (text.rs, `fix/trim-internal-runs`)

- **Before** (buggy) **[checkpoint: read the current body]**: ends-only trim (e.g. `s.trim()` or
  `trim_matches(' ')`) — leaves internal runs uncollapsed, and if it uses generic `.trim()` it also
  wrongly touches non-0x20 whitespace at the ends.
- **After** (one-line, 0x20-only, collapses internal runs):
  ```
  let s = self.get_string(&args[0], cell)?;
  let out = s.split(' ').filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
  CalcResult::String(out)
  ```
  `split(' ')` splits on the single ASCII space **only** (not tabs/NBSP/other Unicode ws); dropping
  empty tokens removes leading/trailing/internal empty runs; `join(" ")` re-inserts single spaces.
  This corrects the missing internal-collapse **and** guarantees the 0x20-only scope (tabs/NBSP inside
  tokens pass through untouched).
- **Regression tests** (§4 before/after table — the load-bearing ones): `"a    b"`→`"a b"`,
  `"no  extra"`→`"no extra"`, `"  hello   world  "`→`"hello world"`, `"single"`→`"single"`,
  `"   "`→`""`; **0x20-only proof:** `"a"&CHAR(9)&CHAR(9)&"b"` unchanged (tabs not collapsed),
  `CHAR(160)&"x"&CHAR(160)` unchanged (NBSP not trimmed).

---

## 4. Shared-code across branches — the main architecture call

Two batch pairs share **new** code, which collides with "one fix = one branch, branches independent
(off `main`), never combined." Resolved:

**PERCENTILE.INC + QUARTILE.INC ship as ONE branch/PR: `fix/percentile-quartile-inc`.**
- **Rationale.** QUARTILE.INC is *definitionally* PERCENTILE.INC at fixed k (Excel documents it that
  way); they share one algorithmic core and identical edge/error semantics. The two alternatives are
  both strictly worse: (a) **duplicate** `percentile_inc_core` into a separate `fix/quartile-inc` off
  `main` → two copies that **conflict when both merge to `freecell-fixes`**, and a redundant PR;
  (b) branch `fix/quartile-inc` **off** `fix/percentile-inc` → a **stacked/dependent** PR upstream
  dislikes more than a cohesive family PR. A single "inclusive percentile/quartile family" PR is the
  most reviewable and has **no** duplication or conflict.
- **Not a policy violation.** CLAUDE.md forbids folding **unrelated** capabilities; these are one
  tightly-coupled feature. The functional-spec §5 branch list (`fix/percentile-inc` + `fix/quartile-inc`)
  is explicitly deferred to this step (§5 "granularity … confirmed in the architecture step — see O-7").
- **Alias wiring.** All four names — `PERCENTILE.INC`, `PERCENTILE`, `QUARTILE.INC`, `QUARTILE` — get an
  enum variant + name mapping + dispatch arm; `.INC` and legacy variants route to the same two impl fns.
  If the fork **already** has legacy `PERCENTILE`/`QUARTILE`, this branch adds the `.INC` names and
  reconciles the impl to the inclusive-interpolation contract (§3.9) rather than adding duplicates
  **[checkpoint via pre-branch git-grep, §5]**.

**CHAR + CODE ship as ONE branch/PR: `fix/char-code`** (same principle, O-7).
- They share the **CP1252 128–255 table** (new code) and require **inverse-consistency**
  (`CODE(CHAR(n))=n`, §3.4/§3.5) — one file, one source of truth for the table. Separate branches would
  duplicate the table and conflict on `freecell-fixes`, and could drift out of inverse-consistency.
  Upstream readily accepts an inverse pair as one PR. This overrides the "one function = one branch"
  default for the same reason as the percentile family: **a shared new helper + a definitional
  coupling.**

**Every other function shares no new code → strictly one function = one branch** (SUMPRODUCT, PROPER,
REPLACE, CLEAN, DOLLAR, ADDRESS, XMATCH, TRIM).

Net: **11 functions + TRIM → 10 branches → 10 upstream PRs.**

---

## 5. Delivery architecture

### 5.1 Branch / PR matrix (one row per function + TRIM)

| Function | Branch | Impl fn / module (best-inferred) | Notes |
|---|---|---|---|
| SUMPRODUCT | `fix/sumproduct` | `fn_sumproduct` (math) | array-context arg eval (§2/§3.1) |
| PROPER | `fix/proper` | `fn_proper` (text) | |
| REPLACE | `fix/replace` | `fn_replace` (text) | Unicode-scalar indexing |
| CHAR | `fix/char-code` | `fn_char` (text) | **paired** (§4) — shared CP1252 table |
| CODE | `fix/char-code` | `fn_code` (text) | **paired** — inverse of CHAR |
| CLEAN | `fix/clean` | `fn_clean` (text) | strip 0–31 only |
| DOLLAR | `fix/dollar` | `fn_dollar` (text/fin) | reuse TEXT/FIXED formatter, en-US |
| ADDRESS | `fix/address` | `fn_address` (lookup) | **full R1C1** (O-5) |
| PERCENTILE.INC (+ PERCENTILE) | `fix/percentile-quartile-inc` | `fn_percentile_inc` (stat) | **paired** (§4) — shared core |
| QUARTILE.INC (+ QUARTILE) | `fix/percentile-quartile-inc` | `fn_quartile_inc` (stat) | **paired** — quart→k over core |
| XMATCH | `fix/xmatch` | `fn_xmatch` (lookup) | all 4 match_modes × 4 search_modes |
| TRIM (fix) | `fix/trim-internal-runs` | `fn_trim` body (text) | collapse internal 0x20 runs |

Each `fix/<slug>` branches off fork **`main`**, carries upstream-style tests, and passes the fork's
own `cargo test` + `make lint` (fmt + strict clippy) **crate-scoped to `ironcalc_base`**. Author as
the owner: `Steve Cosman <848343+scosman@users.noreply.github.com>`; clean commit messages; **no
internal/session URLs** in commits bound for a public PR.

### 5.2 Pre-branch existence check (mandatory, before every branch)

Per the operating-model gotchas, before creating any `fix/*` confirm the capability isn't **already
present** (a stale gap note, or an upstream landing — as happened with hide/unhide in
`gaps_closing_7_15`):

- `git grep -i "sumproduct\|percentile\.inc\|xmatch\|…"` at the pinned rev / on `main`, and check the
  `Function` enum for the name.
- `git merge-base --is-ancestor <upstream-sha> origin/freecell-fixes` when a specific upstream commit
  is suspected.
- **CHAR/CODE and PERCENTILE/QUARTILE especially** may already exist in some form (common functions;
  O-1 references an existing CHAR/UNICHAR family). If a name already computes correctly → **skip the
  branch** (record "already present"). If it exists but is wrong (e.g. CHAR raw-Unicode for 128–255,
  or a legacy PERCENTILE using a non-inclusive method) → the branch becomes a **correctness fix** to
  the existing impl, not a new registration.

### 5.3 Integration onto `freecell-fixes` + FreeCell pickup

- Merge each ready `fix/<slug>` into **`freecell-fixes`** (the branch FreeCell's `[patch.crates-io]`
  pins, `app/Cargo.toml` L110-112). Push `freecell-fixes` (and each `fix/*`) via the git-proxy URL
  (§1). If push to `scosman/ironcalc` is **blocked** (as the CF fix hit — 403 on push), keep a durable
  `NNNN-<slug>.patch` under this project's `fork-fixes/` and record it in the tracker (mirrors
  `conditional-formatting/fork-fixes/`).
- In FreeCell: `cd app && cargo update -p ironcalc_base -p ironcalc` re-pins the lock onto the new
  `freecell-fixes` HEAD. **No other FreeCell change** — the functions simply start computing (§7).

### 5.4 Upstream PR prep (agent preps, **owner** opens)

The agent **cannot** open upstream `ironcalc/IronCalc` PRs. For each ready branch it prepares a
one-click PR for the owner (captured in `fork-fixes/README.md`):

- **Compare link:** `https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/<slug>`
- **Title:** e.g. `Add SUMPRODUCT`, `Add CHAR and CODE`, `Add PERCENTILE.INC / QUARTILE.INC (+ legacy
  PERCENTILE / QUARTILE)`, `Fix TRIM to collapse internal spaces`.
- **Body:** one-paragraph what/why + the Excel contract summary + a minimal repro (a formula → expected
  value) + "tests included" pointing at the added test module. Single-feature, self-contained,
  compiles against upstream `main`.

Owner shepherds the PRs (the human-in-loop step); when one merges upstream it returns via the next
`main` sync and its `fix/*` + `freecell-fixes` merge can be dropped.

---

## 6. Testing strategy

- **Fork, upstream-style unit tests per branch (the bulk).** Each `fix/*` adds a test module using
  IronCalc's harness (`new_empty_model()` → `model._set("A1","=…")` → `model.evaluate()` →
  `assert_eq!(model._get_text("A1"), …)` **[checkpoint]**), covering **every worked example** in the
  function's §3 subsection **verbatim** (they are the test vectors) + the added edge rows called out
  above (DOLLAR negative-zero, ADDRESS empty-sheet, XMATCH 2-D/case-insensitive/binary≡linear, CHAR/CODE
  inverse invariant, TRIM 0x20-only proof). Errors assert as their string form (`"#VALUE!"`, `"#NUM!"`,
  `"#N/A"`). Run **crate-scoped**: `cargo test -p ironcalc_base` + `make lint` (fmt + strict clippy) on
  each branch — not the whole fork workspace per iteration.
- **FreeCell-side smoke (once, after integration).** After `cargo update` re-pins `freecell-fixes`, a
  small `freecell-engine` test evaluates one formula per new function through `WorkbookDocument` and
  asserts it returns the computed value (not `#NAME?`), proving the pin resolves the new names
  end-to-end. Crate-scoped `cargo test -p freecell-engine --lib`.
- **No pixel/render suite** — this batch has **no UI surface** (functions compute values; nothing in the
  grid/cell/sheet/titlebar/chart baseline inventory moves). The CLAUDE.md render gate does not apply.
- **No benchmarks required** — pure scalar fns; XMATCH binary mode is the only perf-sensitive path and
  is validated for *correctness* (binary≡linear on sorted data), not timed.

---

## 7. Error handling, volatility, and effect on FreeCell

- **Error handling.** Uniform error-first propagation (§2.2): coercion helpers return
  `Err(CalcResult::Error{…})` which each body forwards via `?`; array/range error elements propagate
  out (SUMPRODUCT/PERCENTILE/QUARTILE/XMATCH). Bad coercion → `#VALUE!`; out-of-domain numeric → `#NUM!`
  (PERCENTILE k, QUARTILE quart); not-found lookup → `#N/A` (XMATCH). The **first** error under normal
  left-to-right / row-major eval order wins (do not reorder).
- **Volatility: none.** Every function is pure; **none** is added to the fork's volatile-function set
  (§2.3) — contrast RAND/NOW/TODAY/OFFSET/INDIRECT. **[checkpoint]** confirm the new variants are absent
  from that set.
- **Effect on FreeCell.** Zero code change beyond the pin bump: a formula calling one of these names
  currently errors (`#NAME?`/`#ERROR!`); after integration it computes. **Optional follow-up (not
  blocking):** add the 11 new names (+ arg templates) to FreeCell's authored autocomplete catalog
  `freecell-core/src/functions.rs` (`FUNCTIONS`, from `gaps_closing_7_15` §1) so they autocomplete and
  show signature hints — a small FreeCell-side data edit, done last or deferred to a GAPS follow-on.

## 8. One-file vs component-split decision

**Single `architecture.md`** (this doc). Although it exceeds ~300 lines, that is breadth (11 near-uniform
function subsections + delivery), not any single component's internal complexity — even XMATCH (the most
involved) fits one subsection. No `components/*.md` is warranted; the per-function subsections + the
branch matrix are the decomposition, and the implementation plan maps each 1:1 to a phase.

---

## Open decisions for the owner (few — all have a working default)

1. **CHAR/CODE may already exist in the fork** (common functions; O-1 references an existing
   CHAR/UNICHAR family). *Recommendation:* treat as decided — the pre-branch check (§5.2) resolves it
   mechanically: skip if already correct, or convert `fix/char-code` into a 128–255 CP1252
   **correctness** fix. **Only escalate** if reconciling an existing raw-Unicode CHAR to CP1252 would
   break currently-green fork tests (then owner chooses CP1252-fidelity vs preserving existing behavior).
2. **Legacy PERCENTILE/QUARTILE method reconciliation.** If the fork already ships legacy
   PERCENTILE/QUARTILE on a non-inclusive method, `fix/percentile-quartile-inc` aligns them to the
   inclusive/interpolation contract (§3.9). *Recommendation:* proceed (Excel's legacy PERCENTILE **is**
   inclusive, so this is a fidelity improvement); escalate only if it churns existing tests.

Everything else is locked in §0 and above.

---

## Owner decisions (2026-07-18)

- **F1 — family PRs confirmed.** `PERCENTILE.INC + QUARTILE.INC` ship as one branch/PR and
  `CHAR + CODE` ship as one branch/PR (each a coupled feature sharing a new helper; not the
  "unrelated fixes" the fork policy forbids bundling). Net **11 functions + TRIM → 10
  branches → 10 upstream PRs**.
- **F2 — proceed on the recommended defaults** for the conditional reconciliations (CHAR/CODE
  → CP1252 if they already exist; legacy `PERCENTILE`/`QUARTILE` → inclusive method). The
  implementer escalates to the owner **only** if a reconciliation would break currently-green
  fork tests.
