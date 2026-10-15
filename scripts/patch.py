#!/usr/bin/env python3
"""patch.py FILE  — reads pairs of OLD/NEW blocks from stdin separated by lines '=====' and
'-----' (OLD ----- NEW ===== OLD ----- NEW ...). Fails if any OLD block is missing."""
import sys
path = sys.argv[1]
src = open(path).read()
blocks = sys.stdin.read().split("\n=====\n")
for b in blocks:
    if not b.strip():
        continue
    old, new = b.split("\n-----\n")
    if old not in src:
        sys.exit(f"patch.py: anchor not found in {path}:\n{old[:200]}")
    src = src.replace(old, new, 1)
open(path, "w").write(src)
