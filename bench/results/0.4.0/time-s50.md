| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `agent list --limit 5` | 6.2 ± 0.3 |  |  |
| `agent list --json` | 19.0 ± 0.3 | +205.0% | (3.1x slower) |
| `agent show (1 session)` | 5.4 ± 0.1 | -13.7% |  |
| `ide list --limit 5` | 5.6 ± 0.1 | -10.7% |  |
| `ide list --json` | 14.5 ± 0.2 | +132.9% | (2.3x slower) |
| `ide show (1 session)` | 5.0 ± 0.1 | -20.4% |  |
| `agent search (all match)` | 43.6 ± 0.7 | +599.4% | (7.0x slower) |
| `agent handoff --stdout (1 session)` | 5.6 ± 0.1 | -9.5% |  |
| `ide search (all match)` | 31.6 ± 0.8 | +407.9% | (5.1x slower) |
| `ide handoff --stdout (1 session)` | 5.3 ± 0.3 | -15.4% |  |
