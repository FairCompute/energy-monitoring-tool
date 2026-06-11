#!/usr/bin/env python3
"""Probe headless Prometheus power cadence for ENE-53.

The probe starts `emt --headless --export prometheus`, samples raw `/metrics`
faster than the collection cadence, and fails if CPU package energy increases
while the corresponding power gauge repeatedly falls back to zero.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_EMT_BIN = PROJECT_ROOT / "target" / "release" / "emt"
ENERGY_EPSILON = 1e-9
POWER_EPSILON = 1e-6
METRIC_RE = re.compile(
    r"^(?P<name>[a-zA-Z_:][a-zA-Z0-9_:]*)(?:\{(?P<labels>[^}]*)\})?\s+"
    r"(?P<value>[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)$"
)
LABEL_RE = re.compile(r'([a-zA-Z_][a-zA-Z0-9_]*)="((?:\\.|[^"\\])*)"')


@dataclass(frozen=True)
class PowerReading:
    energy_joules: float = 0.0
    power_watts: float = 0.0
    name: str = ""


@dataclass(frozen=True)
class MetricsSample:
    index: int
    system_cpu: PowerReading
    workload_cpu: dict[str, PowerReading]


def parse_labels(text: str) -> dict[str, str]:
    return {
        key: value.replace(r"\"", '"').replace(r"\\", "\\")
        for key, value in LABEL_RE.findall(text)
    }


def read_metrics(url: str) -> tuple[PowerReading, dict[str, PowerReading]]:
    with urllib.request.urlopen(url, timeout=2.0) as response:
        text = response.read().decode("utf-8", errors="replace")

    energy: float | None = None
    power: float | None = None
    workloads: dict[str, PowerReading] = {}
    for raw_line in text.splitlines():
        match = METRIC_RE.match(raw_line.strip())
        if not match:
            continue
        labels = parse_labels(match.group("labels") or "")
        if labels.get("device") != "cpu":
            continue

        value = float(match.group("value"))
        name = match.group("name")
        scope = labels.get("scope")
        if scope == "system":
            if name == "emt_energy_joules_total":
                energy = value
            elif name == "emt_power_watts":
                power = value
        elif scope == "workload":
            workload_id = labels.get("workload")
            if not workload_id:
                continue
            current = workloads.get(workload_id, PowerReading())
            if name == "emt_energy_joules_total":
                workloads[workload_id] = PowerReading(
                    energy_joules=value,
                    power_watts=current.power_watts,
                    name=labels.get("workload_name", current.name),
                )
            elif name == "emt_power_watts":
                workloads[workload_id] = PowerReading(
                    energy_joules=current.energy_joules,
                    power_watts=value,
                    name=labels.get("workload_name", current.name),
                )

    if energy is None or power is None:
        raise RuntimeError("missing system CPU energy or power metric")
    return PowerReading(energy, power), workloads


def read_cpu_system_metrics(url: str) -> tuple[float, float]:
    system_cpu, _ = read_metrics(url)
    return system_cpu.energy_joules, system_cpu.power_watts


def process_failure(process: subprocess.Popen[str] | None) -> str | None:
    if process is None or process.poll() is None:
        return None
    stderr = ""
    if process.stderr is not None:
        stderr = process.stderr.read().strip()
    return f"exporter exited with code {process.returncode}: {stderr}"


def assert_endpoint_unused(url: str) -> None:
    try:
        with urllib.request.urlopen(url, timeout=0.3):
            pass
    except (OSError, urllib.error.URLError):
        return
    raise RuntimeError(
        f"{url} already responds before this probe starts; choose a free port "
        "so the probe cannot pass against a stale exporter"
    )


def wait_for_metrics(
    url: str,
    timeout_seconds: float,
    process: subprocess.Popen[str] | None,
) -> None:
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        failure = process_failure(process)
        if failure:
            raise RuntimeError(failure)
        try:
            read_cpu_system_metrics(url)
            return
        except (RuntimeError, OSError, urllib.error.URLError) as exc:
            last_error = exc
            time.sleep(0.2)
    raise RuntimeError(f"metrics endpoint did not become ready: {last_error}")


def workload_id_for_process(process: subprocess.Popen[str]) -> str:
    return f"pid:{process.pid}"


def start_exporter(
    args: argparse.Namespace,
    workload_pid: int | None,
) -> subprocess.Popen[str]:
    command = [
        str(args.emt_bin),
        "--headless",
        "--export",
        "prometheus",
        "--bind",
        args.host,
        "--port",
        str(args.port),
        "--rate",
        str(args.rate),
        "--scan-interval",
        str(args.scan_interval),
    ]
    if workload_pid is not None:
        command.extend(["--pid", str(workload_pid)])
    print("$ " + " ".join(command))
    return subprocess.Popen(
        command,
        cwd=PROJECT_ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )


def start_workload(duration_seconds: float) -> subprocess.Popen[str]:
    code = f"""
import math
import time

deadline = time.monotonic() + {duration_seconds!r}
value = 0.0
while time.monotonic() < deadline:
    for i in range(20_000):
        value += math.sqrt((i % 97) + 1.0)
print(value)
"""
    return subprocess.Popen(
        [sys.executable, "-c", code],
        cwd=PROJECT_ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )


def stop_exporter(process: subprocess.Popen[str] | None) -> None:
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def collect_samples(
    url: str,
    count: int,
    interval_seconds: float,
    process: subprocess.Popen[str] | None,
) -> list[MetricsSample]:
    samples: list[MetricsSample] = []
    for index in range(count):
        failure = process_failure(process)
        if failure:
            raise RuntimeError(failure)
        system_cpu, workload_cpu = read_metrics(url)
        samples.append(MetricsSample(index, system_cpu, workload_cpu))
        if index + 1 < count:
            time.sleep(interval_seconds)
    return samples


def select_workload(
    samples: list[MetricsSample],
    expected_workload_id: str | None,
) -> str | None:
    if expected_workload_id is not None:
        return expected_workload_id

    workload_ids = {
        workload_id for sample in samples for workload_id in sample.workload_cpu
    }
    best_id: str | None = None
    best_delta = 0.0
    for workload_id in workload_ids:
        series = [
            sample.workload_cpu.get(workload_id, PowerReading()) for sample in samples
        ]
        delta = series[-1].energy_joules - series[0].energy_joules
        if delta > best_delta:
            best_delta = delta
            best_id = workload_id
    return best_id


def print_samples(samples: list[MetricsSample], workload_id: str | None) -> None:
    print(
        "idx,"
        "system_cpu_energy_joules,system_cpu_power_watts,"
        "workload_id,workload_cpu_energy_joules,workload_cpu_power_watts"
    )
    for sample in samples:
        system_marker = " *" if sample.system_cpu.power_watts > POWER_EPSILON else ""
        workload = (
            sample.workload_cpu.get(workload_id, PowerReading())
            if workload_id is not None
            else PowerReading()
        )
        workload_marker = " *" if workload.power_watts > POWER_EPSILON else ""
        print(
            f"{sample.index:02d},"
            f"{sample.system_cpu.energy_joules:.9f},"
            f"{sample.system_cpu.power_watts:.9f}{system_marker},"
            f"{workload_id or '-'},"
            f"{workload.energy_joules:.9f},"
            f"{workload.power_watts:.9f}{workload_marker}"
        )


def evaluate_series(
    name: str,
    series: list[PowerReading],
    *,
    min_energy_changes: int,
    min_post_power_samples: int,
) -> tuple[bool, str]:
    energy_changes = sum(
        1
        for previous, current in zip(series, series[1:])
        if current.energy_joules > previous.energy_joules + ENERGY_EPSILON
    )
    total_energy_delta = series[-1].energy_joules - series[0].energy_joules
    first_nonzero_power = next(
        (
            index
            for index, sample in enumerate(series)
            if sample.power_watts > POWER_EPSILON
        ),
        None,
    )
    zero_after_nonzero = 0
    if first_nonzero_power is not None:
        zero_after_nonzero = sum(
            1
            for sample in series[first_nonzero_power + 1 :]
            if sample.power_watts <= POWER_EPSILON
        )
    post_power_samples = (
        0 if first_nonzero_power is None else len(series) - first_nonzero_power - 1
    )

    summary = (
        f"{name}: energy_changes={energy_changes} "
        f"total_energy_delta={total_energy_delta:.6f} "
        f"post_power_samples={post_power_samples} "
        f"zero_after_nonzero={zero_after_nonzero}"
    )
    if total_energy_delta <= ENERGY_EPSILON:
        return False, f"{name} energy did not increase; {summary}"
    if energy_changes < min_energy_changes:
        return False, f"{name} had too few energy-changing samples; {summary}"
    if first_nonzero_power is None:
        return (
            False,
            f"{name} power never became nonzero while energy increased; {summary}",
        )
    if post_power_samples < min_post_power_samples:
        return False, f"{name} had too few samples after first nonzero power; {summary}"
    if zero_after_nonzero > 0:
        return False, f"{name} power reset to zero after a nonzero sample; {summary}"
    return True, summary


def evaluate(
    samples: list[MetricsSample],
    workload_id: str | None,
    *,
    min_energy_changes: int,
    min_post_power_samples: int,
) -> tuple[bool, list[str]]:
    results: list[str] = []
    system_ok, system_summary = evaluate_series(
        "system_cpu",
        [sample.system_cpu for sample in samples],
        min_energy_changes=min_energy_changes,
        min_post_power_samples=min_post_power_samples,
    )
    results.append(system_summary)
    if not system_ok:
        return False, results

    if workload_id is None:
        results.append("workload_cpu: no workload CPU series was exported")
        return False, results

    workload_ok, workload_summary = evaluate_series(
        f"workload_cpu[{workload_id}]",
        [sample.workload_cpu.get(workload_id, PowerReading()) for sample in samples],
        min_energy_changes=min_energy_changes,
        min_post_power_samples=min_post_power_samples,
    )
    results.append(workload_summary)
    return workload_ok, results


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emt-bin", type=Path, default=DEFAULT_EMT_BIN)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=19104)
    parser.add_argument("--rate", type=float, default=4.0)
    parser.add_argument("--scan-interval", type=float, default=1.0)
    parser.add_argument("--samples", type=int, default=50)
    parser.add_argument("--interval", type=float, default=0.1)
    parser.add_argument("--warmup", type=float, default=1.5)
    parser.add_argument("--startup-timeout", type=float, default=8.0)
    parser.add_argument("--min-energy-changes", type=int, default=3)
    parser.add_argument("--min-post-power-samples", type=int, default=3)
    parser.add_argument(
        "--url",
        help="Probe an already-running exporter instead of starting one.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.samples < 3:
        raise SystemExit("--samples must be at least 3")
    if args.interval <= 0:
        raise SystemExit("--interval must be positive")
    if args.warmup < 0:
        raise SystemExit("--warmup must be non-negative")
    if args.min_energy_changes < 1:
        raise SystemExit("--min-energy-changes must be at least 1")
    if args.min_post_power_samples < 1:
        raise SystemExit("--min-post-power-samples must be at least 1")

    process: subprocess.Popen[str] | None = None
    workload: subprocess.Popen[str] | None = None
    url = args.url or f"http://{args.host}:{args.port}/metrics"
    try:
        workload_duration = (
            args.startup_timeout + args.warmup + args.samples * args.interval + 2.0
        )
        workload = start_workload(workload_duration)
        expected_workload_id: str | None = None
        if args.url is None:
            if not args.emt_bin.exists():
                raise SystemExit(f"EMT binary not found: {args.emt_bin}")
            assert_endpoint_unused(url)
            expected_workload_id = workload_id_for_process(workload)
            process = start_exporter(args, workload.pid)
        wait_for_metrics(url, args.startup_timeout, process)
        if args.warmup:
            time.sleep(args.warmup)
        samples = collect_samples(url, args.samples, args.interval, process)
        workload_id = select_workload(samples, expected_workload_id)
        print_samples(samples, workload_id)
        ok, summaries = evaluate(
            samples,
            workload_id,
            min_energy_changes=args.min_energy_changes,
            min_post_power_samples=args.min_post_power_samples,
        )
        for summary in summaries:
            print(summary)
        if not ok:
            return 1
        print("Prometheus power cadence probe passed.")
        return 0
    finally:
        stop_exporter(process)
        stop_exporter(workload)


if __name__ == "__main__":
    raise SystemExit(main())
