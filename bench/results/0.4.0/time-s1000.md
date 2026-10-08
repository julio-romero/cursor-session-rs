| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `agent list --limit 5` | 37.5 ± 0.4 |  |  |
| `agent list --json` | 319.2 ± 3.6 | +750.7% | (8.5x slower) |
| `agent show (1 session)` | 36.7 ± 0.3 | -2.3% |  |
| `ide list --limit 5` | 15.2 ± 0.1 | -59.4% | (2.5x faster) |
| `ide list --json` | 207.0 ± 1.8 | +451.8% | (5.5x slower) |
| `ide show (1 session)` | 14.9 ± 0.1 | -60.3% | (2.5x faster) |
| `agent search (all match)` | 807.9 ± 12.1 | +2053.5% | (21.5x slower) |
| `agent handoff --stdout (1 session)` | 37.6 ± 1.7 | +0.2% |  |
| `ide search (all match)` | 527.1 ± 8.8 | +1305.0% | (14.0x slower) |
| `ide handoff --stdout (1 session)` | 15.2 ± 0.1 | -59.5% | (2.5x faster) |
