# EMT Verification Summary

## Overview

A verification study was conducted to validate EMT energy measurements by comparing two independent implementations on the same controlled workload:

- Python EMT (`emt.EnergyMonitor`)
- Rust CLI (`energy-monitoring-tool`)

The objective is parity between Python and Rust attribution behavior before integrating the Rust collector more deeply into the Python path.

## Latest Verification Results (March 2026)

### Current Results After Fixes

| Method | Energy (J) | Duration | Variance | Status |
| -------- | ----------- | ---------- | ---------- | -------- |
| Python EMT | 73-78 J | ~3s | ±5% | ✅ Working |
| Rust CLI | 70-84 J | ~3s | ±10% | ✅ Working |

**Python and Rust agree within ~15%** and both read from the same package-level RAPL sources.

## Bugs Fixed

1. **Python EMT - RAPL Double Counting** (CRITICAL):
   - Was reading package + subdomain zones and overcounting
   - **Fix**: Read package-level zones only (`intel-rapl:N`)

2. **Python EMT - Child Process CPU Shows 0%**:
   - Fresh `psutil.Process` objects caused baseline loss each sample
   - **Fix**: Cache and reuse process objects

3. **Python EMT - Normalization Over 1.0**:
   - Timing mismatch could push normalized utilization above 1.0
   - **Fix**: Cap normalization at 1.0

4. **Rust CLI - Unstable CPU Measurements**:
   - `sysinfo` CPU values were inconsistent for this use case
   - **Fix**: Track CPU deltas directly from `/proc/stat` and `/proc/<pid>/stat`

5. **Rust CLI - Zero Energy Bug**:
   - Missing warmup between refreshes produced invalid initial CPU deltas
   - **Fix**: Add warmup delay and targeted process refreshes

6. **Rust CLI - Shutdown Race Condition**:
   - Final batch could be dropped during collector shutdown
   - **Fix**: Stop signal + grace wait before task abort

## Verification Methodology

### Method 1: Python EMT Instrumentation

**Files**: `verification/verify.py`, `verification/workload.py`

- Uses EMT's Python context manager (`EnergyMonitor`)
- Monitors subprocess execution of controlled workloads
- Tracks process tree with `EMT_RELOAD_PROCS=1`
- Collects energy attribution via RAPL-backed collectors

### Method 2: Rust CLI Tool

**Files**: `src/main.rs`, `verification/verify.py`

- Independent Rust implementation using the same data sources
- Monitors target process by PID with child expansion
- Runs as standalone binary and outputs JSON
- Compared directly against Python output over repeated runs

### Isolation Principle

Each method runs in a separate phase with settling time between phases. This prevents cross-interference and keeps workload attribution meaningful.

### Validation Metrics

- Total attributed energy (J)
- Duration consistency
- Multi-run statistics (mean, stdev, range)
- Pairwise Python vs Rust percentage difference

## Usage

```bash
# Full verification (Python vs Rust)
python verification/verify.py -n 5 -d 10

# Custom run
python verification/verify.py --iterations 3 --duration 30
```

Results are written to `verification/verification_results.json`.
