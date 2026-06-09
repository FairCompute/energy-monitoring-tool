#!/usr/bin/env python3
"""
Benchmark release EMT self-overhead for ENE-43.

The primary signal is external: raw host package+DRAM RAPL energy over a fixed
window, plus EMT process-tree CPU time sampled from /proc. The TUI snapshot is
captured only as a diagnostic cross-check for attribution/grouping behavior.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import pty
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parents[1]
WORKLOAD_SCRIPT = PROJECT_ROOT / "scripts" / "verification_workload.py"
DEFAULT_BINARY = PROJECT_ROOT / "target" / "release" / "emt"
DEFAULT_OUTPUT = PROJECT_ROOT / ".artifacts" / "emt_overhead_benchmark.json"
RAPL_ROOT = Path("/sys/class/powercap")
CLOCK_TICKS = os.sysconf(os.sysconf_names.get("SC_CLK_TCK", "SC_CLK_TCK"))
PAGE_SIZE = os.sysconf(os.sysconf_names.get("SC_PAGE_SIZE", "SC_PAGE_SIZE"))
DEFAULT_COMPARISON_RATE_HZ = 0.1
DEFAULT_COMPARISON_SCAN_INTERVAL_SECS = 30.0
DISPLAY_EXTERNAL_AGREEMENT_PERCENT = 1.0
VISIBLE_TUI_MAX_PERCENT = 1.0
REQUIRED_NON_IDLE_RUNS = 3


@dataclass
class ProcSample:
    timestamp: float
    pids: list[int]
    cpu_ticks: int
    rss_bytes: int
    system_active_ticks: int


@dataclass
class RunResult:
    mode: str
    workload: str
    iteration: int
    duration_seconds: float
    emt_pid: int
    workload_pids: list[int]
    raw_package_dram_joules: float
    emt_cpu_seconds: float
    system_active_cpu_seconds: float
    emt_cpu_share: float
    external_estimated_emt_joules: float
    external_overhead_percent: float
    peak_rss_bytes: int
    rss_samples_bytes: list[int]
    process_sample_count: int
    min_process_count: int
    max_process_count: int
    pids_seen: list[int]
    tracked_pid_count: int | None
    collection_tick_count: int | None
    process_scan_count: int | None
    snapshot_process_group_count: int | None
    snapshot_emt_group_found: bool
    snapshot_emt_group_ids: list[str]
    snapshot_emt_percentage: float | None
    snapshot_emt_energy_joules: float | None
    snapshot_workload_groups_found: int
    snapshot_workload_group_ids: list[str]
    snapshot_groups_distinct: bool | None
    displayed_vs_external_delta_percent: float | None
    displayed_external_agree: bool | None
    displayed_attribution_diagnostic: str
    collection_rate_hz: float
    scan_interval_secs: float
    render_interval_millis: int | None
    rapl_zones: list[dict[str, Any]]


def readable_rapl_zones(root: Path = RAPL_ROOT) -> list[Path]:
    zones: list[Path] = []
    for entry in sorted(root.iterdir() if root.exists() else []):
        if not entry.name.startswith(("intel-rapl", "amd-rapl")):
            continue
        if (entry / "energy_uj").exists() and (entry / "name").exists():
            zones.append(entry)
    return zones


def zone_is_package_or_dram(zone: Path) -> bool:
    try:
        name = (zone / "name").read_text(encoding="utf-8").strip().lower()
    except OSError:
        return False
    return name.startswith("package") or "dram" in name


def read_rapl_snapshot(root: Path = RAPL_ROOT) -> dict[str, dict[str, Any]]:
    snapshot: dict[str, dict[str, Any]] = {}
    for zone in readable_rapl_zones(root):
        if not zone_is_package_or_dram(zone):
            continue
        try:
            energy_uj = int((zone / "energy_uj").read_text(encoding="utf-8").strip())
            name = (zone / "name").read_text(encoding="utf-8").strip()
        except (OSError, ValueError):
            continue
        max_path = zone / "max_energy_range_uj"
        try:
            max_energy_uj = int(max_path.read_text(encoding="utf-8").strip())
        except (OSError, ValueError):
            max_energy_uj = None
        snapshot[str(zone)] = {
            "name": name,
            "energy_uj": energy_uj,
            "max_energy_uj": max_energy_uj,
        }
    return snapshot


def rapl_delta_joules(
    before: dict[str, dict[str, Any]], after: dict[str, dict[str, Any]]
) -> tuple[float, list[dict[str, Any]]]:
    total_uj = 0
    zones: list[dict[str, Any]] = []
    for path, start in before.items():
        end = after.get(path)
        if end is None:
            continue
        start_uj = int(start["energy_uj"])
        end_uj = int(end["energy_uj"])
        max_energy_uj = start.get("max_energy_uj")
        if end_uj >= start_uj:
            delta_uj = end_uj - start_uj
        elif max_energy_uj:
            delta_uj = end_uj + int(max_energy_uj) - start_uj
        else:
            delta_uj = 0
        total_uj += delta_uj
        zones.append(
            {
                "path": path,
                "name": start["name"],
                "delta_joules": delta_uj / 1_000_000.0,
            }
        )
    return total_uj / 1_000_000.0, zones


def process_tree(root_pid: int) -> list[int]:
    children_by_parent: dict[int, list[int]] = {}
    live: set[int] = set()
    for proc_dir in Path("/proc").iterdir():
        if not proc_dir.name.isdigit():
            continue
        pid = int(proc_dir.name)
        try:
            stat = (proc_dir / "stat").read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        fields = stat[stat.rfind(")") + 2 :].split()
        if len(fields) < 2:
            continue
        try:
            ppid = int(fields[1])
        except ValueError:
            continue
        live.add(pid)
        children_by_parent.setdefault(ppid, []).append(pid)

    found: list[int] = []
    stack = [root_pid]
    seen: set[int] = set()
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        if pid in live:
            found.append(pid)
        stack.extend(children_by_parent.get(pid, []))
    return sorted(found)


def read_proc_cpu_ticks(pid: int) -> int:
    try:
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return 0
    fields = stat[stat.rfind(")") + 2 :].split()
    if len(fields) < 13:
        return 0
    try:
        return int(fields[11]) + int(fields[12])
    except ValueError:
        return 0


def read_proc_rss_bytes(pid: int) -> int:
    try:
        fields = Path(f"/proc/{pid}/statm").read_text(encoding="utf-8").split()
        return int(fields[1]) * PAGE_SIZE
    except (OSError, ValueError, IndexError):
        return 0


def read_system_active_ticks() -> int:
    try:
        line = Path("/proc/stat").read_text(encoding="utf-8").splitlines()[0]
    except (OSError, IndexError):
        return 0
    fields = [int(value) for value in line.split()[1:] if value.isdigit()]
    if len(fields) < 4:
        return 0
    idle = fields[3] + (fields[4] if len(fields) > 4 else 0)
    return sum(fields) - idle


def sample_process_tree(root_pid: int) -> ProcSample:
    pids = process_tree(root_pid)
    return ProcSample(
        timestamp=time.time(),
        pids=pids,
        cpu_ticks=sum(read_proc_cpu_ticks(pid) for pid in pids),
        rss_bytes=sum(read_proc_rss_bytes(pid) for pid in pids),
        system_active_ticks=read_system_active_ticks(),
    )


def start_workload(case: str, duration: float) -> list[subprocess.Popen[str]]:
    if case == "idle":
        return []
    count = 1
    if case == "multi_cpu":
        count = max(2, (os.cpu_count() or 2) // 2)
    return [
        subprocess.Popen(
            [sys.executable, str(WORKLOAD_SCRIPT), str(duration)],
            cwd=PROJECT_ROOT,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            text=True,
            start_new_session=True,
        )
        for _ in range(count)
    ]


def free_port() -> int:
    import socket

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def command_for_mode(
    mode: str,
    binary: Path,
    duration: float,
    snapshot_path: Path,
    cli_output_path: Path,
    rate: float | None,
    scan_interval: float | None,
) -> list[str]:
    cmd = [str(binary), "--snapshot-out", str(snapshot_path)]
    if rate is not None:
        cmd.extend(["--rate", str(rate)])
    if scan_interval is not None:
        cmd.extend(["--scan-interval", str(scan_interval)])
    if mode == "tui":
        cmd.append("--tui")
    elif mode == "headless":
        cmd.extend(
            [
                "--headless",
                "--export",
                "prometheus",
                "--bind",
                "127.0.0.1",
                "--port",
                str(free_port()),
            ]
        )
    elif mode == "json":
        cmd.extend(
            ["--json-out", str(cli_output_path), "--duration", str(math.ceil(duration))]
        )
    else:
        raise ValueError(f"unsupported mode: {mode}")
    return cmd


def drain_fd(fd: int, sink: list[bytes], stop: threading.Event) -> None:
    while not stop.is_set():
        try:
            chunk = os.read(fd, 4096)
        except OSError:
            break
        if not chunk:
            break
        sink.append(chunk)


def start_emt(
    cmd: list[str], mode: str
) -> tuple[subprocess.Popen[Any], int | None, threading.Event, list[bytes]]:
    if mode != "tui":
        proc = subprocess.Popen(
            cmd,
            cwd=PROJECT_ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        return proc, None, threading.Event(), []

    master_fd, slave_fd = pty.openpty()
    env = os.environ.copy()
    env.setdefault("TERM", "xterm")
    proc = subprocess.Popen(
        cmd,
        cwd=PROJECT_ROOT,
        stdin=slave_fd,
        stdout=slave_fd,
        stderr=slave_fd,
        env=env,
        start_new_session=True,
    )
    os.close(slave_fd)
    stop = threading.Event()
    output: list[bytes] = []
    thread = threading.Thread(
        target=drain_fd, args=(master_fd, output, stop), daemon=True
    )
    thread.start()
    return proc, master_fd, stop, output


def stop_emt(
    proc: subprocess.Popen[Any], mode: str, tty_fd: int | None, stop: threading.Event
) -> None:
    if proc.poll() is not None:
        stop.set()
        if tty_fd is not None:
            try:
                os.close(tty_fd)
            except OSError:
                pass
        return
    if mode == "tui" and tty_fd is not None:
        try:
            os.write(tty_fd, b"q")
        except OSError:
            pass
    else:
        proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)
    stop.set()
    if tty_fd is not None:
        try:
            os.close(tty_fd)
        except OSError:
            pass


def load_snapshot(path: Path) -> dict[str, Any] | None:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None


def group_contains_pid(group: dict[str, Any], pid: int) -> bool:
    if group.get("root_pid") == pid:
        return True
    return any(process.get("pid") == pid for process in group.get("processes", []))


def snapshot_diagnostics(
    snapshot: dict[str, Any] | None, emt_pids: set[int], workload_pids: list[int]
) -> dict[str, Any]:
    if snapshot is None:
        return {
            "emt_group_found": False,
            "emt_group_ids": [],
            "emt_percentage": None,
            "emt_energy_joules": None,
            "workload_group_ids": [],
            "workload_groups_found": 0,
            "groups_distinct": None,
            "tracked_pid_count": None,
            "collection_tick_count": None,
            "process_scan_count": None,
            "process_group_count": None,
        }
    workloads = snapshot.get("workloads", [])
    emt_groups = []
    for group in workloads:
        if any(group_contains_pid(group, pid) for pid in emt_pids):
            emt_groups.append(group)
    workload_groups = {
        group.get("group_id")
        for group in workloads
        if any(group_contains_pid(group, pid) for pid in workload_pids)
    }
    workload_groups = {group_id for group_id in workload_groups if group_id is not None}
    emt_group_ids = {
        group.get("group_id")
        for group in emt_groups
        if group.get("group_id") is not None
    }
    energy = None
    percentage = None
    if emt_groups:
        energy = 0.0
        percentage = 0.0
        for group in emt_groups:
            device_energy = group.get("energy", {})
            energy += sum(
                float(device_energy.get(key, 0.0))
                for key in ("cpu_joules", "dram_joules", "gpu_joules")
            )
            percentage += float(group.get("percentage_of_system", 0.0))
    groups_distinct = None
    if workload_pids:
        groups_distinct = (
            bool(emt_group_ids)
            and bool(workload_groups)
            and emt_group_ids.isdisjoint(workload_groups)
        )
    diagnostics = snapshot.get("diagnostics", {})
    return {
        "emt_group_found": bool(emt_groups),
        "emt_group_ids": sorted(emt_group_ids),
        "emt_percentage": percentage,
        "emt_energy_joules": energy,
        "workload_group_ids": sorted(workload_groups),
        "workload_groups_found": len(workload_groups),
        "groups_distinct": groups_distinct,
        "tracked_pid_count": len(snapshot.get("tracked_pids", [])),
        "collection_tick_count": diagnostics.get("collection_ticks"),
        "process_scan_count": diagnostics.get("process_scans"),
        "process_group_count": diagnostics.get("process_groups"),
    }


def displayed_attribution_diagnostic(
    displayed_percent: float | None,
    external_percent: float,
    groups_distinct: bool | None,
) -> tuple[float | None, bool | None, str]:
    if displayed_percent is None:
        return None, None, "snapshot_missing_emt_group"
    delta = abs(displayed_percent - external_percent)
    agree = delta <= DISPLAY_EXTERNAL_AGREEMENT_PERCENT
    if groups_distinct is False:
        return delta, agree, "grouping_not_distinct"
    if agree:
        return delta, True, "display_agrees_with_external_estimate"
    return delta, False, "display_appears_to_include_attribution_artifact"


def run_once(
    mode: str,
    workload: str,
    iteration: int,
    duration: float,
    binary: Path,
    rate: float | None,
    scan_interval: float | None,
) -> RunResult:
    (PROJECT_ROOT / ".artifacts").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / ".artifacts") as tmp:
        tmp_path = Path(tmp)
        snapshot_path = tmp_path / "snapshot.json"
        cli_output_path = tmp_path / "cli-output.json"
        cmd = command_for_mode(
            mode, binary, duration, snapshot_path, cli_output_path, rate, scan_interval
        )

        proc, tty_fd, stop, _output = start_emt(cmd, mode)
        try:
            time.sleep(1.0)
            before_rapl = read_rapl_snapshot()
            start_sample = sample_process_tree(proc.pid)
            workloads = start_workload(workload, duration)
            workload_pids = [process.pid for process in workloads]
            samples = [start_sample]
            deadline = time.time() + duration
            while time.time() < deadline and proc.poll() is None:
                time.sleep(min(1.0, max(0.1, deadline - time.time())))
                samples.append(sample_process_tree(proc.pid))
            for workload_proc in workloads:
                try:
                    workload_proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    workload_proc.terminate()
            samples.append(sample_process_tree(proc.pid))
            after_rapl = read_rapl_snapshot()
        finally:
            stop_emt(proc, mode, tty_fd, stop)

        snapshot = load_snapshot(snapshot_path)

    first = samples[0]
    live_samples = [sample for sample in samples if sample.pids]
    last = live_samples[-1] if live_samples else samples[-1]
    emt_cpu_ticks = max(0, last.cpu_ticks - first.cpu_ticks)
    system_active_ticks = max(0, last.system_active_ticks - first.system_active_ticks)
    emt_cpu_share = emt_cpu_ticks / system_active_ticks if system_active_ticks else 0.0
    raw_energy, zones = rapl_delta_joules(before_rapl, after_rapl)
    estimated_energy = raw_energy * emt_cpu_share
    overhead_percent = (
        (estimated_energy / raw_energy * 100.0) if raw_energy > 0 else 0.0
    )
    pids_seen = sorted({pid for sample in samples for pid in sample.pids})
    diagnostics = snapshot_diagnostics(
        snapshot,
        set(pids_seen),
        workload_pids,
    )
    displayed_delta, displayed_agree, displayed_diagnostic = (
        displayed_attribution_diagnostic(
            diagnostics["emt_percentage"],
            overhead_percent,
            diagnostics["groups_distinct"],
        )
    )

    effective_rate = rate if rate is not None else DEFAULT_COMPARISON_RATE_HZ
    effective_scan = (
        scan_interval
        if scan_interval is not None
        else DEFAULT_COMPARISON_SCAN_INTERVAL_SECS
    )
    process_counts = [len(sample.pids) for sample in samples]
    return RunResult(
        mode=mode,
        workload=workload,
        iteration=iteration,
        duration_seconds=duration,
        emt_pid=proc.pid,
        workload_pids=workload_pids,
        raw_package_dram_joules=raw_energy,
        emt_cpu_seconds=emt_cpu_ticks / CLOCK_TICKS,
        system_active_cpu_seconds=system_active_ticks / CLOCK_TICKS,
        emt_cpu_share=emt_cpu_share,
        external_estimated_emt_joules=estimated_energy,
        external_overhead_percent=overhead_percent,
        peak_rss_bytes=max(sample.rss_bytes for sample in samples),
        rss_samples_bytes=[sample.rss_bytes for sample in samples],
        process_sample_count=len(samples),
        min_process_count=min(process_counts) if process_counts else 0,
        max_process_count=max(process_counts) if process_counts else 0,
        pids_seen=pids_seen,
        tracked_pid_count=diagnostics["tracked_pid_count"],
        collection_tick_count=diagnostics["collection_tick_count"],
        process_scan_count=diagnostics["process_scan_count"],
        snapshot_process_group_count=diagnostics["process_group_count"],
        snapshot_emt_group_found=diagnostics["emt_group_found"],
        snapshot_emt_group_ids=diagnostics["emt_group_ids"],
        snapshot_emt_percentage=diagnostics["emt_percentage"],
        snapshot_emt_energy_joules=diagnostics["emt_energy_joules"],
        snapshot_workload_groups_found=diagnostics["workload_groups_found"],
        snapshot_workload_group_ids=diagnostics["workload_group_ids"],
        snapshot_groups_distinct=diagnostics["groups_distinct"],
        displayed_vs_external_delta_percent=displayed_delta,
        displayed_external_agree=displayed_agree,
        displayed_attribution_diagnostic=displayed_diagnostic,
        collection_rate_hz=effective_rate,
        scan_interval_secs=effective_scan,
        render_interval_millis=2000 if mode == "tui" else None,
        rapl_zones=zones,
    )


def summarize(results: list[RunResult]) -> dict[str, Any]:
    summary: dict[str, Any] = {}
    for mode in sorted({result.mode for result in results}):
        summary[mode] = {}
        for workload in sorted(
            {result.workload for result in results if result.mode == mode}
        ):
            subset = [r for r in results if r.mode == mode and r.workload == workload]
            overheads = [r.external_overhead_percent for r in subset]
            displayed_diagnostics = [r.displayed_attribution_diagnostic for r in subset]
            distinct_runs = [
                r.snapshot_groups_distinct
                for r in subset
                if r.snapshot_groups_distinct is not None
            ]
            visible_percentages = [
                r.snapshot_emt_percentage
                for r in subset
                if r.snapshot_emt_percentage is not None
            ]
            display_agreements = [
                r.displayed_external_agree
                for r in subset
                if r.displayed_external_agree is not None
            ]
            summary[mode][workload] = {
                "runs": len(subset),
                "median_external_overhead_percent": (
                    statistics.median(overheads) if overheads else None
                ),
                "max_external_overhead_percent": max(overheads) if overheads else None,
                "emt_group_found_runs": sum(
                    1 for result in subset if result.snapshot_emt_group_found
                ),
                "distinct_group_runs": sum(1 for distinct in distinct_runs if distinct),
                "grouping_distinct": (all(distinct_runs) if distinct_runs else None),
                "max_visible_emt_percent": (
                    max(visible_percentages) if visible_percentages else None
                ),
                "visible_emt_percent_under_1_runs": sum(
                    1
                    for visible_percent in visible_percentages
                    if visible_percent <= VISIBLE_TUI_MAX_PERCENT
                ),
                "display_agrees_with_external_runs": sum(
                    1 for agrees in display_agreements if agrees
                ),
                "display_agrees_with_external": (
                    all(display_agreements) if display_agreements else None
                ),
                "displayed_attribution_diagnostics": sorted(set(displayed_diagnostics)),
                "median_displayed_vs_external_delta_percent": (
                    statistics.median(
                        [
                            r.displayed_vs_external_delta_percent
                            for r in subset
                            if r.displayed_vs_external_delta_percent is not None
                        ]
                    )
                    if any(
                        r.displayed_vs_external_delta_percent is not None
                        for r in subset
                    )
                    else None
                ),
            }

    tui = summary.get("tui", {})
    passing = True
    for workload in ("single_cpu", "multi_cpu"):
        stats = tui.get(workload)
        if not stats:
            passing = False
            continue
        median = stats["median_external_overhead_percent"]
        maximum = stats["max_external_overhead_percent"]
        visible_max = stats["max_visible_emt_percent"]
        enough_runs = stats["runs"] >= REQUIRED_NON_IDLE_RUNS
        grouping_ok = stats["grouping_distinct"] is True
        visible_ok = visible_max is not None and visible_max <= VISIBLE_TUI_MAX_PERCENT
        display_agrees = stats["display_agrees_with_external"] is True
        passing = (
            passing
            and enough_runs
            and grouping_ok
            and visible_ok
            and display_agrees
            and median is not None
            and maximum is not None
            and median <= 0.5
            and maximum <= 1.0
        )
    summary["acceptance"] = {
        "tui_non_idle_overhead_passed": passing,
        "thresholds": {
            "median_percent": 0.5,
            "max_percent": 1.0,
            "required_non_idle_runs": REQUIRED_NON_IDLE_RUNS,
            "visible_tui_max_percent": VISIBLE_TUI_MAX_PERCENT,
        },
    }
    return summary


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--duration", type=float, default=60.0)
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--emt-binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--mode", action="append", choices=["tui", "headless", "json"])
    parser.add_argument(
        "--workload", action="append", choices=["idle", "single_cpu", "multi_cpu"]
    )
    parser.add_argument("--rate", type=float, default=DEFAULT_COMPARISON_RATE_HZ)
    parser.add_argument(
        "--scan-interval", type=float, default=DEFAULT_COMPARISON_SCAN_INTERVAL_SECS
    )
    parser.add_argument("--no-build", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release"], cwd=PROJECT_ROOT, check=True)
    if not args.emt_binary.exists():
        raise SystemExit(f"missing release binary: {args.emt_binary}")
    if not read_rapl_snapshot():
        raise SystemExit(
            "no readable package/DRAM RAPL zones found under /sys/class/powercap"
        )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    modes = args.mode or ["tui", "headless", "json"]
    workloads = args.workload or ["idle", "single_cpu", "multi_cpu"]
    results: list[RunResult] = []
    for iteration in range(1, args.iterations + 1):
        for mode in modes:
            for workload in workloads:
                print(f"[{iteration}] mode={mode} workload={workload}", flush=True)
                result = run_once(
                    mode,
                    workload,
                    iteration,
                    args.duration,
                    args.emt_binary,
                    args.rate,
                    args.scan_interval,
                )
                print(
                    f"  overhead={result.external_overhead_percent:.3f}% "
                    f"raw={result.raw_package_dram_joules:.3f}J "
                    f"emt_cpu={result.emt_cpu_seconds:.3f}s",
                    flush=True,
                )
                results.append(result)

    payload = {
        "metadata": {
            "duration_seconds": args.duration,
            "iterations": args.iterations,
            "modes": modes,
            "workloads": workloads,
            "emt_binary": str(args.emt_binary),
            "rate_hz": args.rate,
            "scan_interval_secs": args.scan_interval,
            "comparison_cadence": "same_rate_and_scan_interval_for_all_modes",
        },
        "summary": summarize(results),
        "results": [asdict(result) for result in results],
    }
    args.output.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote {args.output}")
    return 0 if payload["summary"]["acceptance"]["tui_non_idle_overhead_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
