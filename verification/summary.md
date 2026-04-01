# emt verification summary

## overview

this summary tracks the current verification path, which compares only two methods on the same workload:

- python emt (`emt.energymonitor`)
- rust cli (`energy-monitoring-tool`)

the active verification runner is `verification/verify.py`.

## current scope

- bash baseline path removed
- `verification/rapl_baseline.sh` removed
- stress helper scripts removed:
- `verification/py_emt_stress.py`
- `verification/workload_stress.sh`

## methods used by verify

| method | workload source | status |
| -------- | -------- | -------- |
| python emt | `verification/workload.py` | active |
| rust cli | `verification/workload.py` | active |

both methods run in isolated phases with settling time between phases.

## output

`verification/verify.py` writes results to `verification/verification_results.json`.

## usage

```bash
python verification/verify.py -n 5 -d 10
python verification/verify.py --iterations 3 --duration 30
```
