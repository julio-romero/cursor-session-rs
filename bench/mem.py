#!/usr/bin/env python3
"""Prints memory-table rows from a hyperfine --export-json file.

    python3 bench/mem.py time-s300.json 300 "94M + 124M"

Peak memory comes from hyperfine 2.0+, which reads each run's own rusage.
Measuring through a wrapper process is wrong on Linux: a child started with
vfork/posix_spawn inherits its parent's peak RSS across exec.
"""

import json
import sys

path, sessions, size = sys.argv[1:4]
for r in json.load(open(path))["results"]:
    mem = r.get("summary", {}).get("memory_peak_resident")
    peak = f"{mem['median'] / 2**20:.1f} MB" if mem else "n/a (needs hyperfine 2.0+)"
    print(f"| {sessions} | {size} | `{r.get('name') or r['command']}` | {peak} |")
