#!/usr/bin/env python3
"""Find .unwrap() in non-test Rust code."""
import os, re

crate_dir = "crates"
results = []

for root, dirs, fnames in os.walk(crate_dir):
    if "/target/" in root:
        continue
    for f in fnames:
        if not f.endswith(".rs") or ".disabled" in f:
            continue
        fpath = os.path.join(root, f)
        with open(fpath) as fp:
            lines = fp.readlines()

        in_test = False
        brace_depth = 0
        test_start_depth = 0

        for i, line in enumerate(lines, 1):
            stripped = line.strip()
            if "#[cfg(test)]" in stripped or "#[test]" in stripped:
                in_test = True
                test_start_depth = brace_depth

            brace_depth += line.count("{") - line.count("}")

            if in_test and brace_depth <= test_start_depth:
                in_test = False

            if in_test:
                continue

            if stripped.startswith("//"):
                continue

            if ".unwrap()" in line and ".unwrap_or" not in line:
                results.append((fpath, i, stripped))

for fpath, line, code in sorted(results):
    print(f"{fpath}:{line}: {code}")

print(f"\nTotal non-test .unwrap() calls: {len(results)}")
