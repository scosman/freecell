#!/usr/bin/env python3
"""Extract IronCalc 0.7.1's registered builtin-function list, deterministically.

WHY: `ironcalc_base`'s `functions` module is PRIVATE (`mod functions;`, not `pub`),
so the `Function` enum + its `into_iter()`/`to_localized_name()` are unreachable from a
downstream crate. We therefore read the *pinned* source directly and derive the exact
set of registered functions, mapped to their canonical (English) Excel names.

SOURCE (pinned, reproducible):
  - ironcalc_base 0.7.1 -> src/functions/mod.rs
      * the `impl_function_lookup! { field => Variant, ... }` macro body enumerates
        every *registered* function (the enum lookup used by the parser). 345 pairs.
  - ironcalc_base 0.7.1 -> src/language/language.json
      * `en.functions` maps each lookup field -> the uppercase Excel name.

The intersection (macro fields resolved through en.functions) is IronCalc's registered
Excel-function list. Functions that are commented out in the enum (Forecast, Linest,
Percentile*, Quartile*, Trend, ...) are NOT in the macro and are correctly absent.

Output: data/ironcalc_functions.csv  (single column `name`, sorted, 345 rows).

Run:  python3 scripts/extract_ironcalc_functions.py [path-to-ironcalc_base-0.7.1]
If the source path is omitted, the cargo registry cache is auto-located.
"""
import csv
import glob
import json
import os
import re
import sys

EXPECTED_COUNT = 345


def find_source_dir(explicit):
    if explicit:
        return explicit
    home = os.path.expanduser("~")
    pattern = os.path.join(
        home,
        ".cargo",
        "registry",
        "src",
        "*",
        "ironcalc_base-0.7.1",
    )
    matches = glob.glob(pattern)
    if not matches:
        sys.exit(
            "ironcalc_base-0.7.1 source not found in the cargo cache; "
            "pass its path as an argument."
        )
    return matches[0]


def extract(src_dir):
    mod_path = os.path.join(src_dir, "src", "functions", "mod.rs")
    lang_path = os.path.join(src_dir, "src", "language", "language.json")

    mod_src = open(mod_path, encoding="utf-8").read()
    macro = re.search(r"impl_function_lookup!\s*\{(.*?)\n\}", mod_src, re.S)
    if not macro:
        sys.exit("could not locate impl_function_lookup! macro in mod.rs")
    pairs = re.findall(r"([A-Za-z0-9_#]+)\s*=>\s*([A-Za-z0-9]+)\s*,", macro.group(1))
    fields = [f.replace("r#", "") for f, _ in pairs]
    if len(fields) != EXPECTED_COUNT:
        sys.exit(
            f"expected {EXPECTED_COUNT} registered functions, found {len(fields)} "
            "-- the pinned source may have drifted."
        )

    en_functions = json.load(open(lang_path, encoding="utf-8"))["en"]["functions"]

    names = set()
    for field in fields:
        excel_name = en_functions.get(field)
        if excel_name is None:
            sys.exit(f"lookup field '{field}' has no en.functions name")
        names.add(excel_name)

    return sorted(names)


def main():
    explicit = sys.argv[1] if len(sys.argv) > 1 else None
    src_dir = find_source_dir(explicit)
    names = extract(src_dir)

    out_path = os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "data",
        "ironcalc_functions.csv",
    )
    with open(out_path, "w", newline="", encoding="utf-8") as fh:
        writer = csv.writer(fh)
        writer.writerow(["name"])
        for name in names:
            writer.writerow([name])

    print(f"source:  {src_dir}")
    print(f"wrote:   {out_path}")
    print(f"count:   {len(names)} distinct registered functions")


if __name__ == "__main__":
    main()
