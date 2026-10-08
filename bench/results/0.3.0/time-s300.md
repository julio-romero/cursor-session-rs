| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `agent list --limit 5` | 15.1 ± 0.3 |  |  |
| `agent list --json` | 97.8 ± 0.9 | +549.2% | (6.5x slower) |
| `agent show (1 session)` | 14.5 ± 0.6 | -3.8% |  |
| `ide list --limit 5` | 8.5 ± 0.2 | -43.9% | (1.8x faster) |
| `ide list --json` | 63.6 ± 0.7 | +322.0% | (4.2x slower) |
| `ide show (1 session)` | 8.1 ± 0.3 | -46.3% | (1.9x faster) |
