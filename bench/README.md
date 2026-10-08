# Benchmarks

`run.sh` generates synthetic Agent CLI and IDE stores of 50, 300 and 1000
sessions (200 messages of ~1.5 KB each, about 300 KB per session) and runs
these commands against each with
[hyperfine](https://github.com/sharkdp/hyperfine), plus a peak-memory table read from hyperfine (`mem.py`):

| row | command |
|---|---|
| `list --limit 5`, `list --json` | list the newest 5, or every session as JSON |
| `show (1 session)` | show the newest session |
| `search (all match)` | `search lorem dolor`: words every generated message holds, so every session is read and matches (since 0.4.0) |
| `handoff --stdout (1 session)` | print the newest session's handoff transcript (since 0.4.0) |

Each row runs once per store (`agent ...` and `ide ...`). Rows keep their
names from one version to the next, so the tables of two versions compare row
by row; the 0.3.0 tables have no search or handoff rows.

```bash
cargo build --release
bench/run.sh                                   # results in bench/results
bench/run.sh target/release/cursor-session bench/results/0.3.0
BENCH_SIZES="50 300" bench/run.sh              # fewer sizes
BENCH_DATA=/tmp/cs-bench bench/run.sh          # reuse generated data between runs
```

Needs hyperfine 2.0+ (older versions do not record memory) and `python3`. Baselines live in `results/<version>/`.
Only their Markdown tables are committed: git ignores the hyperfine JSON and
anything written straight into `results/`.
