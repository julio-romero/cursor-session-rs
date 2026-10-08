# Peak memory (cursor-session 0.4.0)

| sessions | store size | command | peak RSS (median) |
|---:|---:|---|---:|
| 50 |  15M +  21M | `agent list --limit 5` | 8.4 MB |
| 50 |  15M +  21M | `agent list --json` | 8.4 MB |
| 50 |  15M +  21M | `agent show (1 session)` | 9.1 MB |
| 50 |  15M +  21M | `ide list --limit 5` | 12.4 MB |
| 50 |  15M +  21M | `ide list --json` | 12.5 MB |
| 50 |  15M +  21M | `ide show (1 session)` | 10.8 MB |
| 50 |  15M +  21M | `agent search (all match)` | 9.7 MB |
| 50 |  15M +  21M | `agent handoff --stdout (1 session)` | 8.9 MB |
| 50 |  15M +  21M | `ide search (all match)` | 13.8 MB |
| 50 |  15M +  21M | `ide handoff --stdout (1 session)` | 10.7 MB |
| 300 |  91M + 124M | `agent list --limit 5` | 9.2 MB |
| 300 |  91M + 124M | `agent list --json` | 9.6 MB |
| 300 |  91M + 124M | `agent show (1 session)` | 9.6 MB |
| 300 |  91M + 124M | `ide list --limit 5` | 12.7 MB |
| 300 |  91M + 124M | `ide list --json` | 13.2 MB |
| 300 |  91M + 124M | `ide show (1 session)` | 12.8 MB |
| 300 |  91M + 124M | `agent search (all match)` | 10.9 MB |
| 300 |  91M + 124M | `agent handoff --stdout (1 session)` | 9.7 MB |
| 300 |  91M + 124M | `ide search (all match)` | 14.6 MB |
| 300 |  91M + 124M | `ide handoff --stdout (1 session)` | 12.8 MB |
| 1000 | 305M + 413M | `agent list --limit 5` | 11.2 MB |
| 1000 | 305M + 413M | `agent list --json` | 11.8 MB |
| 1000 | 305M + 413M | `agent show (1 session)` | 11.6 MB |
| 1000 | 305M + 413M | `ide list --limit 5` | 13.8 MB |
| 1000 | 305M + 413M | `ide list --json` | 14.9 MB |
| 1000 | 305M + 413M | `ide show (1 session)` | 13.8 MB |
| 1000 | 305M + 413M | `agent search (all match)` | 13.5 MB |
| 1000 | 305M + 413M | `agent handoff --stdout (1 session)` | 11.6 MB |
| 1000 | 305M + 413M | `ide search (all match)` | 16.3 MB |
| 1000 | 305M + 413M | `ide handoff --stdout (1 session)` | 13.9 MB |
