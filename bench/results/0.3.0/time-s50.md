| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `agent list --limit 5` | 6.2 ± 0.2 |  |  |
| `agent list --json` | 19.2 ± 0.5 | +210.2% | (3.1x slower) |
| `agent show (1 session)` | 5.5 ± 0.4 | -11.6% |  |
| `ide list --limit 5` | 5.4 ± 0.1 | -12.2% |  |
| `ide list --json` | 14.6 ± 0.2 | +135.7% | (2.4x slower) |
| `ide show (1 session)` | 5.1 ± 0.1 | -18.6% |  |
