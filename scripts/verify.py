#!/usr/bin/env python3
"""
Energy Monitoring Tool — Verification Script

Compares energy measurements from two independent methods, each run in ISOLATION:
    1. Python EMT (emt.EnergyMonitor) — runs verification_workload.py as a subprocess
    2. Rust CLI   (energy-monitoring-tool) — runs verification_workload.py as a subprocess

Isolation is critical: because idle energy is attributed proportionally to all
active processes, each method runs the workload alone so that the workload is
the dominant consumer and attribution is meaningful.
"""

import os
import json
import platform
import socket
import subprocess
import statistics
import sys
import time
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Any, Callable

# Add project root to path for emt imports
PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT))

WORKLOAD_SCRIPT = Path(__file__).parent / "verification_workload.py"
RUST_BINARY = PROJECT_ROOT / "target" / "release" / "energy-monitoring-tool"
DEFAULT_OUTPUT_PATH = PROJECT_ROOT / ".artifacts" / "verification_results.json"
PYTHON = sys.executable
RAPL_ROOT = Path("/sys/class/powercap")
ACCURACY_TOLERANCE_PERCENT = 2.0


@dataclass
class Result:
    method: str
    iteration: int
    duration: float
    total_energy_j: float
    raw_rapl_energy_j: float = 0.0
    process_fraction: float = 0.0
    details: dict = field(default_factory=dict)


def rapl_energy_entries(root: Path = RAPL_ROOT) -> list[Path]:
    """Return readable RAPL counter directories under *root*."""
    if not root.exists():
        return []

    return sorted(
        entry
        for entry in root.iterdir()
        if entry.is_dir()
        and entry.name.startswith(("intel-rapl", "amd-rapl"))
        and (entry / "energy_uj").exists()
    )


def assert_rapl_available(root: Path = RAPL_ROOT) -> list[Path]:
    """Fail fast when the host does not expose any readable RAPL counters."""
    entries = rapl_energy_entries(root)
    if entries:
        return entries

    raise RuntimeError(
        f"No readable RAPL energy counters were found under {root}. "
        "Run this verification on a physical host with populated "
        "/sys/class/powercap entries."
    )


def build_acceptance_analysis(
    all_results: dict[str, list[Result]],
    tolerance_percent: float = ACCURACY_TOLERANCE_PERCENT,
) -> dict[str, Any]:
    """Summarize whether the physical-host Python vs Rust comparison passed."""
    analysis = {
        "tolerance_percent": tolerance_percent,
        "python_vs_rust": None,
    }

    python_results = all_results.get("Python EMT", [])
    rust_results = all_results.get("Rust CLI", [])
    if not python_results or not rust_results:
        return analysis

    python_mean = statistics.mean(result.total_energy_j for result in python_results)
    rust_mean = statistics.mean(result.total_energy_j for result in rust_results)
    relative_diff_percent = None
    within_tolerance = False

    if python_mean > 0:
        # Use Python EMT as the reference denominator because the acceptance
        # criterion is defined as "Rust-measured total energy within ±2% of the
        # Python-measured total" before the Rust collector replaces it.
        relative_diff_percent = abs(rust_mean - python_mean) / python_mean * 100.0
        within_tolerance = relative_diff_percent <= tolerance_percent

    analysis["python_vs_rust"] = {
        "python_mean_j": python_mean,
        "rust_mean_j": rust_mean,
        "relative_diff_percent": relative_diff_percent,
        "within_tolerance": within_tolerance,
        "iterations_compared": min(len(python_results), len(rust_results)),
    }
    return analysis


def _read_first_matching_cpuinfo_value(key: str) -> str | None:
    cpuinfo = Path("/proc/cpuinfo")
    if not cpuinfo.exists():
        return None

    for line in cpuinfo.read_text(encoding="utf-8", errors="ignore").splitlines():
        if ":" not in line:
            continue
        name, value = line.split(":", 1)
        if name.strip().lower() == key.lower():
            return value.strip()
    return None


def _read_memtotal_kib() -> int | None:
    meminfo = Path("/proc/meminfo")
    if not meminfo.exists():
        return None

    for line in meminfo.read_text(encoding="utf-8", errors="ignore").splitlines():
        if line.startswith("MemTotal:"):
            parts = line.split()
            if len(parts) >= 2:
                try:
                    return int(parts[1])
                except ValueError:
                    return None
    return None


def _probe_nvidia_smi() -> dict[str, Any]:
    query = [
        "nvidia-smi",
        "--query-gpu=index,name,uuid,memory.total,driver_version",
        "--format=csv,noheader,nounits",
    ]
    try:
        result = subprocess.run(
            query,
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except FileNotFoundError:
        return {"available": False, "error": "nvidia-smi not found"}
    except subprocess.TimeoutExpired:
        return {"available": False, "error": "nvidia-smi timed out"}

    if result.returncode != 0:
        return {
            "available": False,
            "error": result.stderr.strip() or result.stdout.strip(),
        }

    gpus = []
    for line in result.stdout.splitlines():
        parts = [part.strip() for part in line.split(",")]
        if len(parts) >= 5:
            gpus.append(
                {
                    "index": parts[0],
                    "name": parts[1],
                    "uuid": parts[2],
                    "memory_total_mib": parts[3],
                    "driver_version": parts[4],
                }
            )

    return {"available": bool(gpus), "gpus": gpus}


def collect_run_metadata(
    rapl_entries: list[Path],
    num_iterations: int,
    workload_duration: float,
    output_path: Path,
    use_sudo: bool,
) -> dict[str, Any]:
    memtotal_kib = _read_memtotal_kib()
    return {
        "timestamp_unix": time.time(),
        "hostname": socket.gethostname(),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "python_version": platform.python_version(),
        "cpu": {
            "model": _read_first_matching_cpuinfo_value("model name"),
            "vendor": _read_first_matching_cpuinfo_value("vendor_id"),
            "family": _read_first_matching_cpuinfo_value("cpu family"),
            "stepping": _read_first_matching_cpuinfo_value("stepping"),
            "logical_cpus": os.cpu_count(),
        },
        "memory": {
            "total_kib": memtotal_kib,
            "total_gib": (
                round(memtotal_kib / 1024 / 1024, 2)
                if memtotal_kib is not None
                else None
            ),
        },
        "rapl": {
            "root": str(RAPL_ROOT),
            "zones": [entry.name for entry in rapl_entries],
        },
        "nvidia": _probe_nvidia_smi(),
        "verification": {
            "iterations": num_iterations,
            "duration_seconds": workload_duration,
            "output_path": str(output_path),
            "rust_cli_uses_sudo": use_sudo,
        },
    }


def print_metadata_summary(metadata: dict[str, Any]) -> None:
    cpu = metadata["cpu"]
    memory = metadata["memory"]
    nvidia = metadata["nvidia"]

    print(f"CPU: {cpu['model'] or 'unknown'}")
    print(f"Logical CPUs: {cpu['logical_cpus']}")
    print(f"Memory: {memory['total_gib']} GiB")
    if nvidia.get("available"):
        gpu_names = ", ".join(gpu["name"] for gpu in nvidia.get("gpus", []))
        print(f"NVIDIA: available ({gpu_names})")
    else:
        print(f"NVIDIA: unavailable ({nvidia.get('error', 'no GPUs reported')})")


# ── Method 1: Python EMT ────────────────────────────────────────────────────


def measure_python_emt(workload_duration: float, iteration: int) -> Result:
    """
    Run verification_workload.py as a subprocess, monitored by Python EMT.
    EMT tracks the current process tree (this script + workload child).
    """
    # Import here so the module is only loaded when needed
    os.environ["EMT_RELOAD_PROCS"] = "1"
    from emt import EnergyMonitor

    start = time.perf_counter()

    with EnergyMonitor(name=f"verify_{iteration}") as monitor:
        proc = subprocess.Popen(
            [PYTHON, str(WORKLOAD_SCRIPT), str(workload_duration)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        proc.wait()

    elapsed = time.perf_counter() - start

    total_energy = monitor.total_consumed_energy
    unit = monitor.energy_unit
    if unit.lower() == "kwh":
        total_energy *= 3_600_000  # kWh → J

    devices = {}
    if hasattr(monitor, "consumed_energy") and isinstance(
        monitor.consumed_energy, dict
    ):
        devices = {
            k: v * 3_600_000 if unit.lower() == "kwh" else v
            for k, v in monitor.consumed_energy.items()
        }

    return Result(
        method="python_emt",
        iteration=iteration,
        duration=elapsed,
        total_energy_j=total_energy,
        details={"devices": devices},
    )


# ── Method 2: Rust CLI ──────────────────────────────────────────────────────


def measure_rust_cli(
    workload_duration: float,
    iteration: int,
    use_sudo: bool = False,
) -> Result:
    """
    Launch verification_workload.py, then point the Rust binary at its PID.
    The Rust collector expands the PID to include children.
    """
    if not RUST_BINARY.exists():
        raise FileNotFoundError(
            f"Rust binary not found at {RUST_BINARY}. Run: cargo build --release"
        )

    output_file = Path(f"/tmp/rust_verify_{iteration}_{os.getpid()}.json")
    rust_duration = int(workload_duration) + 2

    start = time.perf_counter()

    # Start workload first so we have a PID to monitor
    workload = subprocess.Popen(
        [PYTHON, str(WORKLOAD_SCRIPT), str(workload_duration)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(0.3)  # let it spin up

    rust_cmd = [
        str(RUST_BINARY),
        "--pid",
        str(workload.pid),
        "--duration",
        str(rust_duration),
        "--rate",
        "10.0",
        "--output",
        str(output_file),
        "--quiet",
    ]
    if use_sudo:
        rust_cmd.insert(0, "sudo")

    rust = subprocess.Popen(
        rust_cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    workload.wait()
    try:
        rust_stdout, rust_stderr = rust.communicate(timeout=rust_duration + 10)
    except subprocess.TimeoutExpired:
        rust.kill()
        rust_stdout, rust_stderr = rust.communicate()

    elapsed = time.perf_counter() - start

    if rust.returncode != 0:
        message = rust_stderr.strip() or rust_stdout.strip() or "no output"
        raise RuntimeError(
            f"Rust CLI failed with exit code {rust.returncode}: {message}"
        )

    # Parse result — prefer output file, fall back to stdout
    data = _parse_rust_output(output_file, rust_stdout)

    return Result(
        method="rust_cli",
        iteration=iteration,
        duration=elapsed,
        total_energy_j=data.get("total_energy", 0.0),
        details={"devices": data.get("devices", {}), "workload_pid": workload.pid},
    )


def _parse_rust_output(output_file: Path, stdout: str) -> dict[str, Any]:
    """Try to read JSON from the output file, then from stdout."""
    for source in [_read_json_file(output_file), _parse_json_str(stdout)]:
        if source:
            return source
    return {}


def _read_json_file(path: Path) -> dict[str, Any] | None:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        path.unlink()
        return data
    except (OSError, json.JSONDecodeError):
        return None


def _parse_json_str(s: str) -> dict[str, Any] | None:
    try:
        return json.loads(s)
    except json.JSONDecodeError:
        return None


# ── Orchestrator ─────────────────────────────────────────────────────────────

SETTLE_SECONDS = 3  # pause between phases to let system idle


def run_verification(
    num_iterations: int,
    workload_duration: float,
    output_path: Path = DEFAULT_OUTPUT_PATH,
    use_sudo: bool = False,
) -> None:
    rapl_entries = assert_rapl_available()
    metadata = collect_run_metadata(
        rapl_entries,
        num_iterations,
        workload_duration,
        output_path,
        use_sudo,
    )
    methods: list[tuple[str, Callable[[float, int], Result]]] = [
        ("Python EMT", measure_python_emt),
        (
            "Rust CLI",
            lambda duration, iteration: measure_rust_cli(
                duration,
                iteration,
                use_sudo=use_sudo,
            ),
        ),
    ]

    print("=" * 70)
    print("Energy Monitoring Tool — Verification")
    print(f"Methods: {', '.join(n for n, _ in methods)}")
    print(f"Iterations: {num_iterations}  |  Workload duration: {workload_duration}s")
    print("RAPL zones: " + ", ".join(entry.name for entry in rapl_entries))
    print(f"Rust CLI sudo: {'enabled' if use_sudo else 'disabled'}")
    print_metadata_summary(metadata)
    print("=" * 70)

    all_results: dict[str, list[Result]] = {name: [] for name, _ in methods}

    for name, fn in methods:
        print(f"\n{'─'*70}")
        print(f"Phase: {name}")
        print(f"{'─'*70}")

        for i in range(1, num_iterations + 1):
            print(f"\n  [{name}] Iteration {i}/{num_iterations} …")
            try:
                r = fn(workload_duration, i)
                all_results[name].append(r)
                print(f"    → {r.total_energy_j:.2f} J  (duration {r.duration:.1f}s)")
            except (
                OSError,
                RuntimeError,
                ValueError,
                subprocess.SubprocessError,
            ) as error:
                print(f"    ✗ Error: {error}")

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

    analysis = build_acceptance_analysis(all_results)
    python_vs_rust = analysis.get("python_vs_rust")
    if python_vs_rust is not None:
        status = "✅ PASS" if python_vs_rust["within_tolerance"] else "⚠️ FAIL"
        relative_diff = python_vs_rust["relative_diff_percent"]
        rel_text = f"{relative_diff:.2f}%" if relative_diff is not None else "n/a"
        print(f"\n  {'─'*50}")
        print("  Acceptance criterion — Python EMT vs Rust CLI:")
        print(
            f"    {status} relative difference {rel_text} "
            f"(tolerance ±{analysis['tolerance_percent']:.2f}%)"
        )

    # Save JSON
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_payload = {
        "metadata": metadata,
        "analysis": analysis,
        **{name: [asdict(r) for r in results] for name, results in all_results.items()},
    }
    output_path.write_text(
        json.dumps(output_payload, indent=2, default=str),
        encoding="utf-8",
    )
    print(f"\n  Results saved to {output_path}")


# ── CLI ──────────────────────────────────────────────────────────────────────

if __name__ == "__main__":
    import argparse

    p = argparse.ArgumentParser(description="Verify EMT energy measurements")
    p.add_argument("-n", "--iterations", type=int, default=5)
    p.add_argument("-d", "--duration", type=float, default=10.0)
    p.add_argument(
        "-o",
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT_PATH,
        help=f"Path for result JSON output. Default: {DEFAULT_OUTPUT_PATH}",
    )
    p.add_argument(
        "--sudo",
        action="store_true",
        help="Run the Rust CLI through sudo. Default is direct execution.",
    )
    args = p.parse_args()
    run_verification(args.iterations, args.duration, args.output, args.sudo)
