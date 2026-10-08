#!/usr/bin/env bash
# Benchmarks cursor-session with hyperfine on synthetic stores of growing size:
# wall time per command, and peak memory (median of all runs). The search rows
# use words every generated message holds, so every session is read and matches.
#
#   bench/run.sh [BINARY] [OUT_DIR]
#
# BINARY defaults to target/release/cursor-session, OUT_DIR to bench/results.
# Needs hyperfine 2.0+ (for memory) and python3.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
bin="$(realpath "${1:-$here/../target/release/cursor-session}")"
out="$(mkdir -p "${2:-$here/results}" && cd "${2:-$here/results}" && pwd)"
data="${BENCH_DATA:-$(mktemp -d)}"
sizes="${BENCH_SIZES:-50 300 1000}"
version="$("$bin" --version)"
echo "benchmarking $version -> $out (data in $data)"

memory="$out/memory.md"
printf '# Peak memory (%s)\n\n| sessions | store size | command | peak RSS (median) |\n|---:|---:|---|---:|\n' \
  "$version" > "$memory"

for n in $sizes; do
  dir="$data/s$n"
  if [ ! -f "$dir/ids" ]; then
    ids="$(python3 "$here/gen.py" "$dir" --sessions "$n" --messages 200 --size 1500)"
    printf '%s\n' "$ids" > "$dir/ids"
  fi
  agent_id="$(awk '$1=="agent"{print $2}' "$dir/ids")"
  ide_id="$(awk '$1=="ide"{print $2}' "$dir/ids")"
  agent="$dir/agent/.cursor"
  ide="$dir/ide/state.vscdb"
  size="$(du -sh "$dir/agent" | cut -f1) + $(du -sh "$ide" | cut -f1)"

  hyperfine -N --warmup 2 --min-runs 10 \
    --export-markdown "$out/time-s$n.md" --export-json "$out/time-s$n.json" \
    -n "agent list --limit 5"   "$bin --storage $agent list --limit 5" \
    -n "agent list --json"      "$bin --storage $agent list --json" \
    -n "agent show (1 session)" "$bin --storage $agent show ${agent_id:0:8}" \
    -n "ide list --limit 5"     "$bin --storage $ide list --limit 5" \
    -n "ide list --json"        "$bin --storage $ide list --json" \
    -n "ide show (1 session)"   "$bin --storage $ide show ${ide_id:0:8}" \
    -n "agent search (all match)"          "$bin --storage $agent search lorem dolor" \
    -n "agent handoff --stdout (1 session)" "$bin --storage $agent handoff ${agent_id:0:8} --stdout" \
    -n "ide search (all match)"            "$bin --storage $ide search lorem dolor" \
    -n "ide handoff --stdout (1 session)"   "$bin --storage $ide handoff ${ide_id:0:8} --stdout"
  python3 "$here/mem.py" "$out/time-s$n.json" "$n" "$size" >> "$memory"
done

echo; cat "$memory"
