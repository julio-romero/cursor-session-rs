| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `agent list --limit 5` | 37.4 ± 1.7 |  |  |
| `agent list --json` | 315.4 ± 2.7 | +742.3% | (8.4x slower) |
| `agent show (1 session)` | 36.2 ± 0.3 | -3.4% |  |
| `ide list --limit 5` | 15.1 ± 0.2 | -59.6% | (2.5x faster) |
| `ide list --json` | 205.5 ± 2.3 | +448.6% | (5.5x slower) |
| `ide show (1 session)` | 14.8 ± 0.1 | -60.5% | (2.5x faster) |
| `agent search (all match)` | 807.0 ± 6.2 | +2054.9% | (21.5x slower) |
| `agent handoff --stdout (1 session)` | 36.5 ± 0.4 | -2.6% |  |
| `ide search (all match)` | 535.1 ± 7.7 | +1328.9% | (14.3x slower) |
| `ide handoff --stdout (1 session)` | 15.1 ± 0.1 | -59.7% | (2.5x faster) |
