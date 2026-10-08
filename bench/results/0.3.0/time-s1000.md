| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `agent list --limit 5` | 41.7 ± 2.0 |  |  |
| `agent list --json` | 314.2 ± 3.8 | +653.9% | (7.5x slower) |
| `agent show (1 session)` | 40.0 ± 0.4 | -4.1% |  |
| `ide list --limit 5` | 16.2 ± 0.2 | -61.1% | (2.6x faster) |
| `ide list --json` | 197.1 ± 2.0 | +372.9% | (4.7x slower) |
| `ide show (1 session)` | 16.1 ± 1.2 | -61.4% | (2.6x faster) |
