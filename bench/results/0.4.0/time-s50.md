| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `agent list --limit 5` | 6.2 ± 0.3 |  |  |
| `agent list --json` | 19.1 ± 0.6 | +208.9% | (3.1x slower) |
| `agent show (1 session)` | 5.4 ± 0.1 | -13.3% |  |
| `ide list --limit 5` | 5.5 ± 0.1 | -11.1% |  |
| `ide list --json` | 14.5 ± 0.2 | +134.3% | (2.3x slower) |
| `ide show (1 session)` | 5.0 ± 0.2 | -19.1% |  |
| `agent search (all match)` | 45.0 ± 0.8 | +627.9% | (7.3x slower) |
| `agent handoff --stdout (1 session)` | 5.6 ± 0.1 | -9.1% |  |
| `ide search (all match)` | 31.4 ± 0.7 | +406.6% | (5.1x slower) |
| `ide handoff --stdout (1 session)` | 5.3 ± 0.3 | -14.2% |  |
