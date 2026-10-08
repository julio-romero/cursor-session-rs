| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `agent list --limit 5` | 14.3 ± 0.2 |  |  |
| `agent list --json` | 98.1 ± 1.2 | +587.1% | (6.9x slower) |
| `agent show (1 session)` | 13.5 ± 0.2 | -5.6% |  |
| `ide list --limit 5` | 8.0 ± 0.1 | -43.7% | (1.8x faster) |
| `ide list --json` | 64.6 ± 0.5 | +352.1% | (4.5x slower) |
| `ide show (1 session)` | 7.8 ± 0.1 | -45.5% | (1.8x faster) |
| `agent search (all match)` | 247.0 ± 3.0 | +1629.6% | (17.3x slower) |
| `agent handoff --stdout (1 session)` | 13.8 ± 0.3 | -3.1% |  |
| `ide search (all match)` | 164.5 ± 3.1 | +1051.8% | (11.5x slower) |
| `ide handoff --stdout (1 session)` | 8.1 ± 0.1 | -43.5% | (1.8x faster) |
