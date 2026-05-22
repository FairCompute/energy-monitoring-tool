# emt verification summary

## overview

this summary tracks the current verification path, which compares only two methods on the same workload:

- python emt (`emt.energymonitor`)
- rust cli (`energy-monitoring-tool`)

the active verification runner is `scripts/verify.py`.

## current scope

- bash baseline path removed
- `verification/rapl_baseline.sh` removed
- stress helper scripts removed:
- `verification/py_emt_stress.py`
- `verification/workload_stress.sh`

## methods used by verify

| method | workload source | status |
| -------- | -------- | -------- |
| python emt | `scripts/verification_workload.py` | active |
| rust cli | `scripts/verification_workload.py` | active |

both methods run in isolated phases with settling time between phases. Each
method monitors the `scripts/verification_workload.py` subprocess PID directly
so verifier-process overhead is not charged to only one side of the comparison.

## output

`scripts/verify.py` writes results to
`.artifacts/verification_results.json` by default. The
`.artifacts/` directory is ignored so local hardware verification output is not
committed accidentally. The output includes host metadata and hardware context
so local results can be interpreted without committing machine-specific files.

## usage

```bash
python scripts/verify.py -n 5 -d 10
python scripts/verify.py --iterations 3 --duration 30
```

The Rust CLI runs without `sudo` by default. Use `--sudo` only on hosts where
RAPL access requires elevated privileges.
