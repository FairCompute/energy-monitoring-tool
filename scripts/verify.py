#!/usr/bin/env python3
"""
Energy Monitoring Tool — Verification Script

Compares energy measurements from two independent methods, each run in ISOLATION:
    1. Python EMT (emt.EnergyMonitor) — monitors verification_workload.py by PID
    2. Rust CLI   (emt) — monitors verification_workload.py by PID

Isolation is critical: because idle energy is attributed proportionally to all
active processes, each method runs the workload alone so that the workload is
the dominant consumer and attribution is meaningful. The methods use the same
workload PID scope to avoid charging verifier or monitor overhead to only one
side of the comparison.
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
RUST_BINARY = PROJECT_ROOT / "target" / "release" / "emt"
DEFAULT_OUTPUT_PATH = PROJECT_ROOT / ".artifacts" / "verification_results.json"
RUST_VERIFY_TMP_DIR = PROJECT_ROOT / ".artifacts" / "tmp"
PYTHON = sys.executable
RAPL_ROOT = Path("/sys/class/powercap")
ACCURACY_TOLERANCE_PERCENT = 2.0
WORKLOAD_MONITOR_START_DELAY_SECONDS = 0.3
ENERGY_FACTORS_TO_JOULES = {
    "Joules": 1.0,
    "kJ": 1_000.0,
    "μJ": 1e-6,
    "uJ": 1e-6,
    "mJ": 1e-3,
    "Wh": 3_600.0,
    "kWh": 3_600_000.0,
}


@dataclass
class Result:
    method: str
    iteration: int
    duration: float
    total_energy_j: float
    raw_rapl_energy_j: float = 0.0
    process_fraction: float = 0.0
    details: dict = field(default_factory=dict)


def _can_read(path: Path) -> bool:
    try:
        with path.open("r", encoding="utf-8") as file:
            file.read(1)
        return True
    except OSError:
        return False


def rapl_energy_entries(root: Path = RAPL_ROOT) -> list[Path]:
    """Return readable RAPL counter directories under *root*."""
    if not root.exists():
        return []

    return sorted(
        entry
        for entry in root.iterdir()
        if entry.name.startswith(("intel-rapl", "amd-rapl"))
        and _can_read(entry / "name")
        and _can_read(entry / "energy_uj")
    )


def assert_rapl_available(root: Path = RAPL_ROOT) -> list[Path]:
    """Fail fast when the host does not expose any readable RAPL counters."""
    entries = rapl_energy_entries(root)
    if entries:
        return entries

    raise RuntimeError(
        f"No readable RAPL energy counters were found under {root}. "
        "Run this verification on a physical host with populated "
        "/sys/class/powercap entries and readable powercap permissions."
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
            "python_gpu_disabled": True,
            "rust_cli_gpu_disabled": True,
            "method_order": "interleaved_alternating",
            "monitor_start_delay_seconds": WORKLOAD_MONITOR_START_DELAY_SECONDS,
            "python_monitoring_scope": "workload_pid",
            "rust_monitoring_scope": "workload_pid",
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

    The monitor is scoped to the workload subprocess PID so this path matches
    the Rust CLI path, which also monitors the workload from outside instead of
    charging the verifier process tree.
    """
    previous_reload = os.environ.get("EMT_RELOAD_PROCS")
    previous_disable_gpu = os.environ.get("EMT_DISABLE_GPU")
    os.environ["EMT_RELOAD_PROCS"] = "1"
    os.environ["EMT_DISABLE_GPU"] = "1"

    # Import here so the module is only loaded when needed
    from emt import EnergyMonitor

    start = time.perf_counter()

    proc = subprocess.Popen(
        [PYTHON, str(WORKLOAD_SCRIPT), str(workload_duration)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        time.sleep(WORKLOAD_MONITOR_START_DELAY_SECONDS)
        with EnergyMonitor(
            name=f"verify_{iteration}",
            pid=proc.pid,
            startup_delay_s=0.0,
        ) as monitor:
            proc.wait()
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
        if previous_reload is None:
            os.environ.pop("EMT_RELOAD_PROCS", None)
        else:
            os.environ["EMT_RELOAD_PROCS"] = previous_reload
        if previous_disable_gpu is None:
            os.environ.pop("EMT_DISABLE_GPU", None)
        else:
            os.environ["EMT_DISABLE_GPU"] = previous_disable_gpu

    elapsed = time.perf_counter() - start

    total_energy = monitor.total_consumed_energy
    unit = monitor.energy_unit
    total_energy = energy_to_joules(total_energy, unit)

    devices = {}
    if hasattr(monitor, "consumed_energy") and isinstance(
        monitor.consumed_energy, dict
    ):
        devices = {
            k: energy_to_joules(v, unit) for k, v in monitor.consumed_energy.items()
        }

    return Result(
        method="python_emt",
        iteration=iteration,
        duration=elapsed,
        total_energy_j=total_energy,
        details={
            "devices": devices,
            "workload_pid": proc.pid,
            "monitor_start_delay_s": WORKLOAD_MONITOR_START_DELAY_SECONDS,
        },
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

    RUST_VERIFY_TMP_DIR.mkdir(parents=True, exist_ok=True)
    output_file = RUST_VERIFY_TMP_DIR / f"rust_verify_{iteration}_{os.getpid()}.json"
    rust_duration = int(workload_duration) + 2

    start = time.perf_counter()

    # Start workload first so we have a PID to monitor
    workload = subprocess.Popen(
        [PYTHON, str(WORKLOAD_SCRIPT), str(workload_duration)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(WORKLOAD_MONITOR_START_DELAY_SECONDS)  # let it spin up

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

    rust_env = os.environ.copy()
    rust_env["EMT_DISABLE_GPU"] = "1"

    rust = subprocess.Popen(
        rust_cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=rust_env,
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
        total_energy_j=rust_total_energy_joules(data),
        details={
            "devices": rust_devices_joules(data),
            "workload_pid": workload.pid,
            "monitor_start_delay_s": WORKLOAD_MONITOR_START_DELAY_SECONDS,
        },
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


def energy_to_joules(value: float, unit: str | None) -> float:
    """Normalize an energy value to Joules."""
    return value * ENERGY_FACTORS_TO_JOULES.get(unit or "Joules", 1.0)


def rust_total_energy_joules(data: dict[str, Any]) -> float:
    """Read Rust CLI energy and normalize it to Joules for parity checks."""
    unit = data.get("energy_unit", "Joules")
    if "total_energy" in data:
        return energy_to_joules(float(data["total_energy"]), unit)
    return float(data.get("total_energy_joules", 0.0))


def rust_devices_joules(data: dict[str, Any]) -> dict[str, float]:
    """Normalize Rust CLI device energy details to Joules."""
    unit = data.get("energy_unit", "Joules")
    devices = data.get("devices", {})
    if not isinstance(devices, dict):
        return {}

    normalized = {}
    for key, value in devices.items():
        if not isinstance(value, (int, float)):
            continue
        normalized[key] = energy_to_joules(float(value), unit)
    return normalized


# ── Orchestrator ─────────────────────────────────────────────────────────────

SETTLE_SECONDS = 3  # pause between phases to let system idle
Method = tuple[str, Callable[[float, int], Result]]


def build_verification_methods(use_sudo: bool) -> list[Method]:
    return [
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


def print_verification_header(
    methods: list[Method],
    num_iterations: int,
    workload_duration: float,
    rapl_entries: list[Path],
    metadata: dict[str, Any],
    use_sudo: bool,
) -> None:
    print("=" * 70)
    print("Energy Monitoring Tool — Verification")
    print(f"Methods: {', '.join(n for n, _ in methods)}")
    print(f"Iterations: {num_iterations}  |  Workload duration: {workload_duration}s")
    print("RAPL zones: " + ", ".join(entry.name for entry in rapl_entries))
    print(f"Rust CLI sudo: {'enabled' if use_sudo else 'disabled'}")
    print_metadata_summary(metadata)
    print("=" * 70)


def measure_method(
    name: str,
    fn: Callable[[float, int], Result],
    num_iterations: int,
    workload_duration: float,
) -> list[Result]:
    print(f"\n{'─'*70}")
    print(f"Phase: {name}")
    print(f"{'─'*70}")

    results = []
    for iteration in range(1, num_iterations + 1):
        print(f"\n  [{name}] Iteration {iteration}/{num_iterations} …")
        try:
            result = fn(workload_duration, iteration)
            results.append(result)
            print(
                f"    → {result.total_energy_j:.2f} J  "
                f"(duration {result.duration:.1f}s)"
            )
        except (
            OSError,
            RuntimeError,
            ValueError,
            subprocess.SubprocessError,
        ) as error:
            print(f"    ✗ Error: {error}")

        time.sleep(1)

    print(f"\n  Settling for {SETTLE_SECONDS}s …")
    time.sleep(SETTLE_SECONDS)
    return results


def run_methods(
    methods: list[Method],
    num_iterations: int,
    workload_duration: float,
) -> dict[str, list[Result]]:
    results: dict[str, list[Result]] = {name: [] for name, _ in methods}

    for iteration in range(1, num_iterations + 1):
        ordered_methods = methods if iteration % 2 == 1 else list(reversed(methods))
        print(f"\n{'─'*70}")
        print(f"Iteration {iteration}/{num_iterations}")
        print(f"{'─'*70}")

        for name, fn in ordered_methods:
            print(f"\n  [{name}] Iteration {iteration}/{num_iterations} …")
            try:
                result = fn(workload_duration, iteration)
                results[name].append(result)
                print(
                    f"    → {result.total_energy_j:.2f} J  "
                    f"(duration {result.duration:.1f}s)"
                )
            except (
                OSError,
                RuntimeError,
                ValueError,
                subprocess.SubprocessError,
            ) as error:
                print(f"    ✗ Error: {error}")

            print(f"    Settling for {SETTLE_SECONDS}s …")
            time.sleep(SETTLE_SECONDS)

    return results


def print_method_statistics(all_results: dict[str, list[Result]]) -> dict[str, float]:
    print()
    print("=" * 70)
    print("RESULTS")
    print("=" * 70)

    means: dict[str, float] = {}
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
    return means


def print_pairwise_comparison(means: dict[str, float]) -> None:
    names = list(means.keys())
    if len(names) < 2:
        return

    print("\n  " + "─" * 50)
    print("  Pairwise comparison:")
    for i in range(len(names)):
        for j in range(i + 1, len(names)):
            a, b = names[i], names[j]
            ref = means[a]
            if ref > 0:
                diff = ((means[b] - ref) / ref) * 100
                status = "✅" if abs(diff) < 20 else "⚠️"
                print(f"    {status} {a} vs {b}: {diff:+.1f}%")


def print_iteration_table(all_results: dict[str, list[Result]]) -> None:
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


def print_acceptance_summary(analysis: dict[str, Any]) -> None:
    python_vs_rust = analysis.get("python_vs_rust")
    if python_vs_rust is None:
        return

    status = "✅ PASS" if python_vs_rust["within_tolerance"] else "⚠️ FAIL"
    relative_diff = python_vs_rust["relative_diff_percent"]
    rel_text = f"{relative_diff:.2f}%" if relative_diff is not None else "n/a"
    print("\n  " + "─" * 50)
    print("  Acceptance criterion — Python EMT vs Rust CLI:")
    print(
        f"    {status} relative difference {rel_text} "
        f"(tolerance ±{analysis['tolerance_percent']:.2f}%)"
    )


def write_results(
    output_path: Path,
    metadata: dict[str, Any],
    analysis: dict[str, Any],
    all_results: dict[str, list[Result]],
) -> None:
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


def print_results(all_results: dict[str, list[Result]]) -> dict[str, Any]:
    means = print_method_statistics(all_results)
    print_pairwise_comparison(means)
    print_iteration_table(all_results)
    analysis = build_acceptance_analysis(all_results)
    print_acceptance_summary(analysis)
    return analysis


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
    methods = build_verification_methods(use_sudo)
    print_verification_header(
        methods,
        num_iterations,
        workload_duration,
        rapl_entries,
        metadata,
        use_sudo,
    )
    all_results = run_methods(methods, num_iterations, workload_duration)
    analysis = print_results(all_results)
    write_results(output_path, metadata, analysis, all_results)


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
