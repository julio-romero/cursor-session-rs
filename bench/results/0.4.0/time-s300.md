| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `agent list --limit 5` | 14.3 ± 0.2 |  |  |
| `agent list --json` | 98.5 ± 0.9 | +590.4% | (6.9x slower) |
| `agent show (1 session)` | 13.5 ± 0.2 | -5.4% |  |
| `ide list --limit 5` | 8.1 ± 0.1 | -43.3% | (1.8x faster) |
| `ide list --json` | 65.8 ± 1.4 | +360.7% | (4.6x slower) |
| `ide show (1 session)` | 7.8 ± 0.1 | -45.1% | (1.8x faster) |
| `agent search (all match)` | 244.3 ± 3.0 | +1611.7% | (17.1x slower) |
| `agent handoff --stdout (1 session)` | 13.8 ± 0.1 | -3.2% |  |
| `ide search (all match)` | 162.6 ± 3.3 | +1039.0% | (11.4x slower) |
| `ide handoff --stdout (1 session)` | 8.1 ± 0.1 | -43.3% | (1.8x faster) |
