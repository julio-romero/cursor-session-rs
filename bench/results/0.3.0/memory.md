# Peak memory (cursor-session 0.3.0)

| sessions | store size | command | peak RSS (median) |
|---:|---:|---|---:|
| 50 |  15M +  21M | `agent list --limit 5` | 7.7 MB |
| 50 |  15M +  21M | `agent list --json` | 7.8 MB |
| 50 |  15M +  21M | `agent show (1 session)` | 8.4 MB |
| 50 |  15M +  21M | `ide list --limit 5` | 11.6 MB |
| 50 |  15M +  21M | `ide list --json` | 11.8 MB |
| 50 |  15M +  21M | `ide show (1 session)` | 10.1 MB |
| 300 |  91M + 124M | `agent list --limit 5` | 8.5 MB |
| 300 |  91M + 124M | `agent list --json` | 9.0 MB |
| 300 |  91M + 124M | `agent show (1 session)` | 8.8 MB |
| 300 |  91M + 124M | `ide list --limit 5` | 12.0 MB |
| 300 |  91M + 124M | `ide list --json` | 12.5 MB |
| 300 |  91M + 124M | `ide show (1 session)` | 12.1 MB |
| 1000 | 305M + 416M | `agent list --limit 5` | 10.4 MB |
| 1000 | 305M + 416M | `agent list --json` | 10.9 MB |
| 1000 | 305M + 416M | `agent show (1 session)` | 10.7 MB |
| 1000 | 305M + 416M | `ide list --limit 5` | 13.0 MB |
| 1000 | 305M + 416M | `ide list --json` | 14.0 MB |
| 1000 | 305M + 416M | `ide show (1 session)` | 13.0 MB |
