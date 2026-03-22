#!/usr/bin/env python3
"""
Energy Monitoring Tool — Verification Script

Compares energy measurements from three independent methods, each run in ISOLATION:
  1. Python EMT (emt.EnergyMonitor) — runs workload.py as a subprocess
  2. Rust CLI   (energy-monitoring-tool) — runs workload.py as a subprocess
  3. Raw bash   (rapl_baseline.sh) — reads RAPL counters around workload.py

Isolation is critical: because idle energy is attributed proportionally to all
active processes, each method runs the workload alone so that the workload is
the dominant consumer and attribution is meaningful.
"""

import os
import sys
import time
import json
import subprocess
import statistics
from pathlib import Path
from dataclasses import dataclass, field, asdict
from typing import List

# Add parent directory to path for emt imports
sys.path.insert(0, str(Path(__file__).parent.parent))

PROJECT_ROOT = Path(__file__).parent.parent
WORKLOAD_SCRIPT = Path(__file__).parent / "workload.py"
RUST_BINARY = PROJECT_ROOT / "target" / "release" / "energy-monitoring-tool"
BASH_BASELINE = Path(__file__).parent / "rapl_baseline.sh"
PYTHON = sys.executable


@dataclass
class Result:
    method: str
    iteration: int
    duration: float
    total_energy_j: float
    raw_rapl_energy_j: float = 0.0
    process_fraction: float = 0.0
    details: dict = field(default_factory=dict)


# ── Method 1: Python EMT ────────────────────────────────────────────────────

def measure_python_emt(workload_duration: float, iteration: int) -> Result:
    """
    Run workload.py as a subprocess, monitored by Python EMT.
    EMT tracks the current process tree (this script + workload child).
    """
    # Import here so the module is only loaded when needed
    os.environ["EMT_RELOAD_PROCS"] = "1"
    from emt import EnergyMonitor

    start = time.perf_counter()

    with EnergyMonitor(name=f"verify_{iteration}") as monitor:
        proc = subprocess.Popen(
            [PYTHON, str(WORKLOAD_SCRIPT), str(workload_duration)],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        proc.wait()

    elapsed = time.perf_counter() - start

    total_energy = monitor.total_consumed_energy
    unit = monitor.energy_unit
    if unit.lower() == "kwh":
        total_energy *= 3_600_000  # kWh → J

    devices = {}
    if hasattr(monitor, "consumed_energy") and isinstance(monitor.consumed_energy, dict):
        devices = {k: v * 3_600_000 if unit.lower() == "kwh" else v
                   for k, v in monitor.consumed_energy.items()}

    return Result(
        method="python_emt",
        iteration=iteration,
        duration=elapsed,
        total_energy_j=total_energy,
        details={"devices": devices},
    )


# ── Method 2: Rust CLI ──────────────────────────────────────────────────────

def measure_rust_cli(workload_duration: float, iteration: int) -> Result:
    """
    Launch workload.py, then point the Rust binary at its PID.
    The Rust collector expands the PID to include children.
    """
    if not RUST_BINARY.exists():
        raise FileNotFoundError(
            f"Rust binary not found at {RUST_BINARY}. Run: cargo build --release"
        )

    output_file = f"/tmp/rust_verify_{iteration}_{os.getpid()}.json"
    rust_duration = int(workload_duration) + 2

    start = time.perf_counter()

    # Start workload first so we have a PID to monitor
    workload = subprocess.Popen(
        [PYTHON, str(WORKLOAD_SCRIPT), str(workload_duration)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    time.sleep(0.3)  # let it spin up

    rust_cmd = [
        "sudo", str(RUST_BINARY),
        "--pid", str(workload.pid),
        "--duration", str(rust_duration),
        "--rate", "10.0",
        "--output", output_file,
        "--quiet",
    ]

    rust = subprocess.Popen(
        rust_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )

    workload.wait()
    try:
        rust_stdout, rust_stderr = rust.communicate(timeout=rust_duration + 10)
    except subprocess.TimeoutExpired:
        rust.kill()
        rust_stdout, rust_stderr = rust.communicate()

    elapsed = time.perf_counter() - start

    # Parse result — prefer output file, fall back to stdout
    data = _parse_rust_output(output_file, rust_stdout)

    return Result(
        method="rust_cli",
        iteration=iteration,
        duration=elapsed,
        total_energy_j=data.get("total_energy", 0.0),
        details={"devices": data.get("devices", {}),
                 "workload_pid": workload.pid},
    )


def _parse_rust_output(output_file: str, stdout: str) -> dict:
    """Try to read JSON from the output file, then from stdout."""
    for source in [_read_json_file(output_file), _parse_json_str(stdout)]:
        if source:
            return source
    return {}


def _read_json_file(path: str) -> dict | None:
    try:
        with open(path) as f:
            data = json.load(f)
        os.unlink(path)
        return data
    except Exception:
        return None


def _parse_json_str(s: str) -> dict | None:
    try:
        return json.loads(s)
    except Exception:
        return None


# ── Method 3: Raw bash / RAPL baseline ──────────────────────────────────────

def measure_bash_baseline(workload_duration: float, iteration: int) -> Result:
    """
    Run rapl_baseline.sh which reads RAPL counters + /proc/stat around
    workload.py and computes attributed energy from first principles.
    """
    if not BASH_BASELINE.exists():
        raise FileNotFoundError(f"Bash baseline script not found: {BASH_BASELINE}")

    start = time.perf_counter()

    proc = subprocess.run(
        ["sudo", "bash", str(BASH_BASELINE), str(int(workload_duration)),
         str(WORKLOAD_SCRIPT), PYTHON],
        capture_output=True, text=True, timeout=int(workload_duration) * 3 + 30,
    )

    elapsed = time.perf_counter() - start

    if proc.returncode != 0:
        print(f"    bash stderr: {proc.stderr[:300]}")
        return Result(method="bash_baseline", iteration=iteration,
                      duration=elapsed, total_energy_j=0.0)

    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError:
        print(f"    Could not parse bash output: {proc.stdout[:300]}")
        return Result(method="bash_baseline", iteration=iteration,
                      duration=elapsed, total_energy_j=0.0)

    return Result(
        method="bash_baseline",
        iteration=iteration,
        duration=elapsed,
        total_energy_j=float(data.get("attributed_energy_j", 0.0)),
        raw_rapl_energy_j=float(data.get("rapl_total_energy_j", 0.0)),
        process_fraction=float(data.get("process_fraction", 0.0)),
        details=data,
    )


# ── Orchestrator ─────────────────────────────────────────────────────────────

METHODS = [
    ("Python EMT",      measure_python_emt),
    ("Rust CLI",        measure_rust_cli),
    ("Bash Baseline",   measure_bash_baseline),
]

SETTLE_SECONDS = 3  # pause between phases to let system idle


def run_verification(num_iterations: int, workload_duration: float):
    print("=" * 70)
    print("Energy Monitoring Tool — Verification")
    print(f"Methods: {', '.join(n for n, _ in METHODS)}")
    print(f"Iterations: {num_iterations}  |  Workload duration: {workload_duration}s")
    print("=" * 70)

    all_results: dict[str, List[Result]] = {name: [] for name, _ in METHODS}

    for name, fn in METHODS:
        print(f"\n{'─'*70}")
        print(f"Phase: {name}")
        print(f"{'─'*70}")

        for i in range(1, num_iterations + 1):
            print(f"\n  [{name}] Iteration {i}/{num_iterations} …")
            try:
                r = fn(workload_duration, i)
                all_results[name].append(r)
                print(f"    → {r.total_energy_j:.2f} J  (duration {r.duration:.1f}s)")
            except Exception as e:
                print(f"    ✗ Error: {e}")

            # pause between iterations
            time.sleep(1)

        # pause between phases
        print(f"\n  Settling for {SETTLE_SECONDS}s …")
        time.sleep(SETTLE_SECONDS)

    # ── Summary ──────────────────────────────────────────────────────────
    print()
    print("=" * 70)
    print("RESULTS")
    print("=" * 70)

    means = {}
    for name, results in all_results.items():
        if not results:
            continue
        energies = [r.total_energy_j for r in results]
        mean_e = statistics.mean(energies)
        means[name] = mean_e
        std_e = statistics.stdev(energies) if len(energies) > 1 else 0.0
        print(f"\n  {name} ({len(results)} runs):")
        print(f"    Mean:  {mean_e:>10.2f} J")
        print(f"    Stdev: {std_e:>10.2f} J")
        print(f"    Range: [{min(energies):.2f} – {max(energies):.2f}] J")

    # Pairwise comparison
    names = list(means.keys())
    if len(names) >= 2:
        print(f"\n  {'─'*50}")
        print(f"  Pairwise comparison:")
        for i in range(len(names)):
            for j in range(i + 1, len(names)):
                a, b = names[i], names[j]
                ref = means[a]
                if ref > 0:
                    diff = ((means[b] - ref) / ref) * 100
                    status = "✅" if abs(diff) < 20 else "⚠️"
                    print(f"    {status} {a} vs {b}: {diff:+.1f}%")

    # Per-iteration table
    print(f"\n  {'Iter':<5}", end="")
    for name in all_results:
        print(f"  {name:>15}", end="")
    print()
    print(f"  {'─'*5}", end="")
    for _ in all_results:
        print(f"  {'─'*15}", end="")
    print()

    max_iters = max(len(v) for v in all_results.values()) if all_results else 0
    for i in range(max_iters):
        print(f"  {i+1:<5}", end="")
        for name in all_results:
            results = all_results[name]
            if i < len(results):
                print(f"  {results[i].total_energy_j:>12.2f} J", end="")
            else:
                print(f"  {'—':>15}", end="")
        print()

    # Save JSON
    output_path = Path(__file__).parent / "verification_results.json"
    with open(output_path, "w") as f:
        json.dump(
            {name: [asdict(r) for r in results]
             for name, results in all_results.items()},
            f, indent=2, default=str,
        )
    print(f"\n  Results saved to {output_path}")


# ── CLI ──────────────────────────────────────────────────────────────────────

if __name__ == "__main__":
    import argparse
    p = argparse.ArgumentParser(description="Verify EMT energy measurements")
    p.add_argument("-n", "--iterations", type=int, default=5)
    p.add_argument("-d", "--duration", type=float, default=10.0)
    args = p.parse_args()
    run_verification(args.iterations, args.duration)
