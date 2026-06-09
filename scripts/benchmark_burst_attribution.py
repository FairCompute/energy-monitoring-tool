#!/usr/bin/env python3
"""
Investigate bursty and short-lived attribution behavior for ENE-44.

This harness runs controlled process patterns while release EMT monitors all
processes with configurable low-overhead cadences. It records process ground
truth from the workload, independent /proc CPU-time samples, and compact EMT
group evidence so short-lived failures can be separated into discovery,
grouping, and exit-accounting gaps.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import threading
import time
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BINARY = PROJECT_ROOT / "target" / "release" / "emt"
DEFAULT_OUTPUT = PROJECT_ROOT / ".artifacts" / "burst_attribution_results.json"
WORKLOAD_SCRIPT = PROJECT_ROOT / "scripts" / "burst_attribution_workload.py"
DEFAULT_SWEEP_RUNTIMES = [2.0, 10.0, 20.0, 35.0, 52.0]
DEFAULT_SWEEP_START_OFFSETS = [0.0, 10.0, 20.0]
TOP_GROUP_EVIDENCE_LIMIT = 25


@dataclass
class ExpectedProcess:
    pid: int
    role: str
    expected_runtime_seconds: float | None
    sweep_repetition: int | None
    sweep_start_offset_seconds: float | None
    self_reported_cpu_ticks_delta: int | None


@dataclass
class ProcCpuSample:
    timestamp: float
    pid: int
    cpu_ticks: int


@dataclass
class ProcCpuEvidence:
    pid: int
    sample_count: int
    first_cpu_ticks: int | None
    last_cpu_ticks: int | None
    observed_cpu_ticks: int
    externally_observed: bool


@dataclass
class ProcessFinding:
    pid: int
    role: str
    expected_runtime_seconds: float | None
    sweep_repetition: int | None
    sweep_start_offset_seconds: float | None
    discovered: bool
    attributed: bool
    energy_joules: float
    group_id: str | None
    group_energy_joules: float
    proc_cpu_ticks_observed: int
    proc_cpu_sample_count: int
    self_reported_cpu_ticks_delta: int | None
    failure_mode: str


@dataclass
class GroupProcessEnergy:
    pid: int
    energy_joules: float


@dataclass
class GroupEvidence:
    group_id: str | None
    root_pid: int | None
    energy_joules: float
    process_pids: list[int]
    process_energy_joules: list[GroupProcessEnergy]
    evidence_reason: str


@dataclass
class PatternResult:
    pattern: str
    monitor_duration_seconds: float
    workload_duration_seconds: float
    collection_rate_hz: float
    scan_interval_secs: float
    attribution_refresh_secs: float
    expected_processes: list[ExpectedProcess]
    findings: list[ProcessFinding]
    proc_cpu_evidence: list[ProcCpuEvidence]
    group_evidence: list[GroupEvidence]
    snapshot_group_count: int
    snapshot_group_energy_joules: float
    workload_events: list[dict[str, Any]]
    unattributed_joules: float
    system_total_joules: float
    recommendation: str


def parse_event_line(line: str) -> dict[str, Any] | None:
    line = line.strip()
    if not line:
        return None
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        return None
    return event if isinstance(event, dict) else None


def parse_workload_events(stdout: str) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    for line in stdout.splitlines():
        event = parse_event_line(line)
        if event is not None:
            events.append(event)
    return events


def expected_processes(events: list[dict[str, Any]]) -> list[ExpectedProcess]:
    runtimes: dict[int, float | None] = {}
    roles: dict[int, str] = {}
    repetitions: dict[int, int | None] = {}
    start_offsets: dict[int, float | None] = {}
    self_ticks: dict[int, int | None] = {}
    for event in events:
        pid = event.get("pid")
        if not isinstance(pid, int):
            continue
        roles[pid] = str(event.get("role", "unknown"))
        if event.get("event") == "child_spawned":
            runtimes[pid] = float(event.get("expected_runtime_seconds", 0.0))
            repetition = event.get("sweep_repetition")
            repetitions[pid] = repetition if isinstance(repetition, int) else None
            offset = event.get("sweep_start_offset_seconds")
            start_offsets[pid] = (
                float(offset) if isinstance(offset, (int, float)) else None
            )
        elif event.get("event") == "process_end":
            if "runtime_seconds" in event:
                runtimes[pid] = float(event["runtime_seconds"])
            ticks_delta = event.get("self_cpu_ticks_delta")
            self_ticks[pid] = ticks_delta if isinstance(ticks_delta, int) else None
        else:
            runtimes.setdefault(pid, None)
    return [
        ExpectedProcess(
            pid=pid,
            role=roles.get(pid, "unknown"),
            expected_runtime_seconds=runtimes.get(pid),
            sweep_repetition=repetitions.get(pid),
            sweep_start_offset_seconds=start_offsets.get(pid),
            self_reported_cpu_ticks_delta=self_ticks.get(pid),
        )
        for pid in sorted(roles)
    ]


def load_snapshot(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def device_energy_total(energy: dict[str, Any]) -> float:
    return sum(
        float(energy.get(key, 0.0))
        for key in ("cpu_joules", "dram_joules", "gpu_joules")
    )


def read_proc_cpu_ticks(pid: int) -> int | None:
    try:
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return None
    fields = stat[stat.rfind(")") + 2 :].split()
    if len(fields) < 13:
        return None
    try:
        return int(fields[11]) + int(fields[12])
    except ValueError:
        return None


def read_workload_stdout(
    proc: subprocess.Popen[str],
    events: list[dict[str, Any]],
    known_pids: set[int],
    lock: threading.Lock,
) -> None:
    if proc.stdout is None:
        return
    for line in proc.stdout:
        event = parse_event_line(line)
        if event is None:
            continue
        with lock:
            events.append(event)
            pid = event.get("pid")
            if isinstance(pid, int):
                known_pids.add(pid)


def sample_known_pids(
    known_pids: set[int], lock: threading.Lock, samples: list[ProcCpuSample]
) -> None:
    with lock:
        pids = sorted(known_pids)
    timestamp = time.time()
    for pid in pids:
        ticks = read_proc_cpu_ticks(pid)
        if ticks is not None:
            samples.append(ProcCpuSample(timestamp, pid, ticks))


def run_workload_collecting_proc_evidence(
    cmd: list[str], timeout: float, sample_interval: float
) -> tuple[list[dict[str, Any]], list[ProcCpuSample], int, str]:
    events: list[dict[str, Any]] = []
    samples: list[ProcCpuSample] = []
    known_pids: set[int] = set()
    lock = threading.Lock()
    proc = subprocess.Popen(
        cmd,
        cwd=PROJECT_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
        start_new_session=True,
    )
    with lock:
        known_pids.add(proc.pid)
    reader = threading.Thread(
        target=read_workload_stdout,
        args=(proc, events, known_pids, lock),
        daemon=True,
    )
    reader.start()

    deadline = time.time() + timeout
    while proc.poll() is None:
        if time.time() > deadline:
            proc.kill()
            proc.wait(timeout=5)
            raise RuntimeError(f"workload timed out after {timeout:.1f}s")
        sample_known_pids(known_pids, lock, samples)
        time.sleep(sample_interval)
    sample_known_pids(known_pids, lock, samples)
    reader.join(timeout=2)
    stderr = proc.stderr.read() if proc.stderr is not None else ""
    with lock:
        final_events = list(events)
    return final_events, samples, int(proc.returncode or 0), stderr


def cpu_evidence_by_pid(
    samples: list[ProcCpuSample], expected: list[ExpectedProcess]
) -> dict[int, ProcCpuEvidence]:
    samples_by_pid: dict[int, list[ProcCpuSample]] = defaultdict(list)
    for sample in samples:
        samples_by_pid[sample.pid].append(sample)

    evidence: dict[int, ProcCpuEvidence] = {}
    for process in expected:
        pid_samples = sorted(
            samples_by_pid.get(process.pid, []), key=lambda sample: sample.timestamp
        )
        if pid_samples:
            first = pid_samples[0].cpu_ticks
            last = pid_samples[-1].cpu_ticks
            observed = max(0, last - first)
            evidence[process.pid] = ProcCpuEvidence(
                pid=process.pid,
                sample_count=len(pid_samples),
                first_cpu_ticks=first,
                last_cpu_ticks=last,
                observed_cpu_ticks=observed,
                externally_observed=True,
            )
        else:
            evidence[process.pid] = ProcCpuEvidence(
                pid=process.pid,
                sample_count=0,
                first_cpu_ticks=None,
                last_cpu_ticks=None,
                observed_cpu_ticks=0,
                externally_observed=False,
            )
    return evidence


def find_process(
    snapshot: dict[str, Any], pid: int
) -> tuple[bool, float, str | None, float]:
    for workload in snapshot.get("workloads", []):
        group_energy = device_energy_total(workload.get("energy", {}))
        if workload.get("root_pid") == pid:
            return True, group_energy, workload.get("group_id"), group_energy
        for process in workload.get("processes", []):
            if process.get("pid") == pid:
                return (
                    True,
                    device_energy_total(process.get("energy", {})),
                    workload.get("group_id"),
                    group_energy,
                )
    return False, 0.0, None, 0.0


def process_has_cpu_evidence(
    proc_ticks_observed: int,
    proc_sample_count: int,
    self_reported_ticks: int | None,
) -> bool:
    return (
        proc_ticks_observed > 0
        or proc_sample_count > 0
        or (self_reported_ticks is not None and self_reported_ticks > 0)
    )


def classify_failure_mode(
    discovered: bool,
    attributed: bool,
    proc_ticks_observed: int,
    proc_sample_count: int,
    self_reported_ticks: int | None,
) -> str:
    if attributed:
        return "attributed"
    if not process_has_cpu_evidence(
        proc_ticks_observed, proc_sample_count, self_reported_ticks
    ):
        return "no_cpu_time_evidence"
    if not discovered:
        return "not_discovered_before_exit_or_grouped_elsewhere"
    return "discovered_without_energy_exit_accounting_or_sample_gap"


def build_findings(
    snapshot: dict[str, Any],
    expected: list[ExpectedProcess],
    cpu_evidence: dict[int, ProcCpuEvidence] | None = None,
) -> list[ProcessFinding]:
    findings = []
    cpu_evidence = cpu_evidence or {}
    for process in expected:
        discovered, energy, group_id, group_energy = find_process(snapshot, process.pid)
        proc_evidence = cpu_evidence.get(
            process.pid,
            ProcCpuEvidence(
                pid=process.pid,
                sample_count=0,
                first_cpu_ticks=None,
                last_cpu_ticks=None,
                observed_cpu_ticks=0,
                externally_observed=False,
            ),
        )
        failure_mode = classify_failure_mode(
            discovered,
            energy > 0.0,
            proc_evidence.observed_cpu_ticks,
            proc_evidence.sample_count,
            process.self_reported_cpu_ticks_delta,
        )
        findings.append(
            ProcessFinding(
                pid=process.pid,
                role=process.role,
                expected_runtime_seconds=process.expected_runtime_seconds,
                sweep_repetition=process.sweep_repetition,
                sweep_start_offset_seconds=process.sweep_start_offset_seconds,
                discovered=discovered,
                attributed=energy > 0.0,
                energy_joules=energy,
                group_id=group_id,
                group_energy_joules=group_energy,
                proc_cpu_ticks_observed=proc_evidence.observed_cpu_ticks,
                proc_cpu_sample_count=proc_evidence.sample_count,
                self_reported_cpu_ticks_delta=process.self_reported_cpu_ticks_delta,
                failure_mode=failure_mode,
            )
        )
    return findings


def group_evidence_from_workload(
    workload: dict[str, Any], reason: str
) -> GroupEvidence:
    process_pids = []
    process_energies = []
    root_pid = workload.get("root_pid")
    if isinstance(root_pid, int):
        process_pids.append(root_pid)
    for process in workload.get("processes", []):
        pid = process.get("pid")
        if not isinstance(pid, int):
            continue
        process_pids.append(pid)
        process_energies.append(
            GroupProcessEnergy(
                pid=pid,
                energy_joules=device_energy_total(process.get("energy", {})),
            )
        )
    return GroupEvidence(
        group_id=workload.get("group_id"),
        root_pid=root_pid if isinstance(root_pid, int) else None,
        energy_joules=device_energy_total(workload.get("energy", {})),
        process_pids=sorted(set(process_pids)),
        process_energy_joules=process_energies,
        evidence_reason=reason,
    )


def compact_group_evidence(
    snapshot: dict[str, Any],
    expected_pids: set[int],
    top_limit: int = TOP_GROUP_EVIDENCE_LIMIT,
) -> list[GroupEvidence]:
    evidence_by_group_id: dict[str, GroupEvidence] = {}
    for workload in snapshot.get("workloads", []):
        evidence = group_evidence_from_workload(workload, "expected_pid_intersection")
        if expected_pids.intersection(evidence.process_pids):
            evidence_by_group_id[str(evidence.group_id)] = evidence

    top_groups = sorted(
        snapshot.get("workloads", []),
        key=lambda workload: device_energy_total(workload.get("energy", {})),
        reverse=True,
    )[:top_limit]
    for workload in top_groups:
        evidence = group_evidence_from_workload(workload, "top_energy_group")
        group_key = str(evidence.group_id)
        if group_key in evidence_by_group_id:
            evidence_by_group_id[group_key].evidence_reason = (
                "expected_pid_intersection,top_energy_group"
            )
        else:
            evidence_by_group_id[group_key] = evidence

    return sorted(
        evidence_by_group_id.values(),
        key=lambda evidence: evidence.energy_joules,
        reverse=True,
    )


def recommendation_for(
    pattern: str, findings: list[ProcessFinding], refresh_secs: float
) -> str:
    non_root = [finding for finding in findings if finding.role != "root"]
    targets = non_root or findings
    if not targets:
        return "No expected processes were emitted by the workload."
    if all(finding.discovered and finding.attributed for finding in targets):
        return "Observed processes were discovered and attributed under this cadence."
    failure_modes = Counter(finding.failure_mode for finding in targets)
    if pattern in {"child_under_interval", "many_short"}:
        return (
            "Sub-interval processes were missed or partially attributed; this is a "
            f"cadence/discovery limit at {refresh_secs:.2f}s refresh. Prefer "
            "event-based child discovery or retained exit accounting; only lower "
            "the cadence if ENE-43 overhead is re-benchmarked and still passes. "
            f"failure_modes={dict(failure_modes)}"
        )
    return (
        "Expected processes were not fully attributed; investigate event-based "
        "child discovery, retained PID maps, and exit accounting before reducing "
        f"cadence. failure_modes={dict(failure_modes)}"
    )


def run_pattern(
    pattern: str,
    binary: Path,
    monitor_duration: float,
    workload_duration: float,
    rate: float,
    scan_interval: float,
    sweep_runtimes: list[float],
    sweep_repetitions: int,
    sweep_start_offsets: list[float],
    proc_sample_interval: float,
) -> PatternResult:
    refresh_secs = 1.0 / rate
    minimum_reliable_runtime = scan_interval + (2.0 * refresh_secs)
    child_over_runtime = minimum_reliable_runtime + 2.0
    child_under_runtime = max(0.05, min(refresh_secs, scan_interval) / 2.0)
    (PROJECT_ROOT / ".artifacts").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / ".artifacts") as tmp:
        tmp_path = Path(tmp)
        snapshot_path = tmp_path / "snapshot.json"
        cli_output_path = tmp_path / "emt.json"
        emt = subprocess.Popen(
            [
                str(binary),
                "--json-out",
                str(cli_output_path),
                "--duration",
                str(round(monitor_duration)),
                "--rate",
                str(rate),
                "--scan-interval",
                str(scan_interval),
                "--snapshot-out",
                str(snapshot_path),
            ],
            cwd=PROJECT_ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        try:
            time.sleep(1.0)
            workload_cmd = [
                sys.executable,
                str(WORKLOAD_SCRIPT),
                pattern,
                "--duration",
                str(workload_duration),
            ]
            if pattern == "child_over_interval":
                workload_cmd.extend(["--child-runtime", str(child_over_runtime)])
            elif pattern in {"child_under_interval", "many_short"}:
                workload_cmd.extend(
                    [
                        "--child-runtime",
                        str(child_under_runtime),
                        "--spawn-interval",
                        str(max(child_under_runtime, 0.1)),
                    ]
                )
            elif pattern == "lifetime_sweep":
                workload_cmd.extend(
                    [
                        "--child-runtimes",
                        ",".join(str(runtime) for runtime in sweep_runtimes),
                        "--sweep-repetitions",
                        str(sweep_repetitions),
                        "--sweep-start-offsets",
                        ",".join(str(offset) for offset in sweep_start_offsets),
                    ]
                )
            events, proc_samples, workload_returncode, workload_stderr = (
                run_workload_collecting_proc_evidence(
                    workload_cmd,
                    timeout=monitor_duration + 5,
                    sample_interval=proc_sample_interval,
                )
            )
            stdout, stderr = emt.communicate(timeout=monitor_duration + 10)
        finally:
            if emt.poll() is None:
                emt.terminate()
                try:
                    emt.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    emt.kill()
                    emt.wait(timeout=5)

        if emt.returncode != 0:
            raise RuntimeError(f"emt failed with {emt.returncode}: {stderr or stdout}")
        if workload_returncode != 0:
            raise RuntimeError(
                f"workload failed with {workload_returncode}: {workload_stderr}"
            )
        snapshot = load_snapshot(snapshot_path)

    expected = expected_processes(events)
    evidence_by_pid = cpu_evidence_by_pid(proc_samples, expected)
    findings = build_findings(snapshot, expected, evidence_by_pid)
    group_evidence = compact_group_evidence(
        snapshot, {process.pid for process in expected}
    )
    snapshot_workloads = snapshot.get("workloads", [])
    unattributed = device_energy_total(snapshot.get("unattributed", {}))
    system_total = device_energy_total(snapshot.get("system_total", {}))
    return PatternResult(
        pattern=pattern,
        monitor_duration_seconds=monitor_duration,
        workload_duration_seconds=workload_duration,
        collection_rate_hz=rate,
        scan_interval_secs=scan_interval,
        attribution_refresh_secs=refresh_secs,
        expected_processes=expected,
        findings=findings,
        proc_cpu_evidence=list(evidence_by_pid.values()),
        group_evidence=group_evidence,
        snapshot_group_count=len(snapshot_workloads),
        snapshot_group_energy_joules=sum(
            device_energy_total(workload.get("energy", {}))
            for workload in snapshot_workloads
        ),
        workload_events=events,
        unattributed_joules=unattributed,
        system_total_joules=system_total,
        recommendation=recommendation_for(pattern, findings, refresh_secs),
    )


def has_cpu_evidence(finding: ProcessFinding) -> bool:
    return process_has_cpu_evidence(
        finding.proc_cpu_ticks_observed,
        finding.proc_cpu_sample_count,
        finding.self_reported_cpu_ticks_delta,
    )


def lifetime_sweep_summary(results: list[PatternResult]) -> list[dict[str, Any]]:
    grouped: dict[float, list[ProcessFinding]] = defaultdict(list)
    for result in results:
        if result.pattern != "lifetime_sweep":
            continue
        for finding in result.findings:
            if finding.role == "root" or finding.expected_runtime_seconds is None:
                continue
            grouped[finding.expected_runtime_seconds].append(finding)

    summary = []
    for runtime, findings in sorted(grouped.items()):
        expected = len(findings)
        discovered = sum(1 for finding in findings if finding.discovered)
        attributed = sum(1 for finding in findings if finding.attributed)
        cpu_evidence = sum(1 for finding in findings if has_cpu_evidence(finding))
        start_offsets = sorted(
            {
                finding.sweep_start_offset_seconds
                for finding in findings
                if finding.sweep_start_offset_seconds is not None
            }
        )
        summary.append(
            {
                "runtime_seconds": runtime,
                "start_offsets_seconds": start_offsets,
                "expected": expected,
                "discovered": discovered,
                "attributed": attributed,
                "cpu_evidence_processes": cpu_evidence,
                "reliable": (
                    expected > 0
                    and discovered == expected
                    and attributed == expected
                    and cpu_evidence == expected
                ),
                "failure_modes": dict(
                    Counter(finding.failure_mode for finding in findings)
                ),
            }
        )
    return summary


def exit_accounting_assessment(results: list[PatternResult]) -> str:
    if not results:
        return "No benchmark results were collected."
    target_runtime = results[0].scan_interval_secs + (
        2.0 * results[0].attribution_refresh_secs
    )
    findings_with_cpu = [
        finding
        for result in results
        for finding in result.findings
        if finding.role != "root" and has_cpu_evidence(finding)
    ]
    long_failures = [
        finding
        for finding in findings_with_cpu
        if finding.expected_runtime_seconds is not None
        and finding.expected_runtime_seconds >= target_runtime
        and not finding.attributed
    ]
    if long_failures:
        return (
            f"Exit accounting/discovery is insufficient: {len(long_failures)} "
            f"processes at or above the {target_runtime:.1f}s scan+sample target "
            "had CPU-time evidence but no process-level attribution."
        )
    short_failures = [
        finding for finding in findings_with_cpu if not finding.attributed
    ]
    if short_failures:
        return (
            "Long-lived processes at the scan+sample target were attributed, but "
            f"{len(short_failures)} shorter CPU-active processes were missed or "
            "partially attributed. Event-based child discovery or retained exit "
            "accounting is needed if those lifetimes must be reported."
        )
    return (
        "All CPU-active non-root processes observed in this run were discovered and "
        "attributed under the tested cadence."
    )


def summarize(results: list[PatternResult]) -> dict[str, Any]:
    minimum_observed_attributed_lifetime = None
    for result in results:
        attributed_lifetimes = [
            finding.expected_runtime_seconds
            for finding in result.findings
            if finding.role != "root"
            and finding.expected_runtime_seconds is not None
            and finding.discovered
            and finding.attributed
        ]
        if attributed_lifetimes:
            candidate = min(attributed_lifetimes)
            minimum_observed_attributed_lifetime = (
                candidate
                if minimum_observed_attributed_lifetime is None
                else min(minimum_observed_attributed_lifetime, candidate)
            )

    sweep_summary = lifetime_sweep_summary(results)
    reliable_runtimes = [
        row["runtime_seconds"] for row in sweep_summary if row["reliable"]
    ]

    by_pattern = {}
    for result in results:
        failure_modes = Counter(finding.failure_mode for finding in result.findings)
        non_root_findings = [
            finding for finding in result.findings if finding.role != "root"
        ]
        non_root_failure_modes = Counter(
            finding.failure_mode for finding in non_root_findings
        )
        by_pattern[result.pattern] = {
            "expected": len(result.findings),
            "discovered": sum(1 for finding in result.findings if finding.discovered),
            "attributed": sum(1 for finding in result.findings if finding.attributed),
            "cpu_evidence_processes": sum(
                1 for finding in result.findings if has_cpu_evidence(finding)
            ),
            "non_root_expected": len(non_root_findings),
            "non_root_discovered": sum(
                1 for finding in non_root_findings if finding.discovered
            ),
            "non_root_attributed": sum(
                1 for finding in non_root_findings if finding.attributed
            ),
            "non_root_cpu_evidence_processes": sum(
                1 for finding in non_root_findings if has_cpu_evidence(finding)
            ),
            "group_evidence_groups": len(result.group_evidence),
            "snapshot_group_count": result.snapshot_group_count,
            "snapshot_group_energy_joules": result.snapshot_group_energy_joules,
            "unattributed_joules": result.unattributed_joules,
            "system_total_joules": result.system_total_joules,
            "failure_modes": dict(failure_modes),
            "non_root_failure_modes": dict(non_root_failure_modes),
            "recommendation": result.recommendation,
        }
    return {
        "minimum_observed_attributed_lifetime_seconds": (
            minimum_observed_attributed_lifetime
        ),
        "minimum_reliable_lifetime_seconds": (
            min(reliable_runtimes) if reliable_runtimes else None
        ),
        "lifetime_sweep": sweep_summary,
        "exit_accounting_assessment": exit_accounting_assessment(results),
        "patterns": by_pattern,
    }


def parse_sweep_runtimes(value: str) -> list[float]:
    runtimes = [float(item) for item in value.split(",") if item.strip()]
    if not runtimes:
        raise argparse.ArgumentTypeError("at least one sweep runtime is required")
    if any(runtime <= 0 for runtime in runtimes):
        raise argparse.ArgumentTypeError("sweep runtimes must be positive")
    return runtimes


def parse_sweep_offsets(value: str) -> list[float]:
    offsets = [float(item) for item in value.split(",") if item.strip()]
    if not offsets:
        raise argparse.ArgumentTypeError("at least one sweep offset is required")
    if any(offset < 0 for offset in offsets):
        raise argparse.ArgumentTypeError("sweep offsets must be non-negative")
    return offsets


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emt-binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--monitor-duration", type=float, default=90.0)
    parser.add_argument("--workload-duration", type=float, default=60.0)
    parser.add_argument("--rate", type=float, default=0.1)
    parser.add_argument("--scan-interval", type=float, default=30.0)
    parser.add_argument(
        "--pattern",
        action="append",
        choices=[
            "long_lived_bursts",
            "child_over_interval",
            "child_under_interval",
            "lifetime_sweep",
            "many_short",
        ],
    )
    parser.add_argument(
        "--sweep-runtimes",
        type=parse_sweep_runtimes,
        default=DEFAULT_SWEEP_RUNTIMES,
    )
    parser.add_argument("--sweep-repetitions", type=int, default=3)
    parser.add_argument(
        "--sweep-start-offsets",
        type=parse_sweep_offsets,
        default=DEFAULT_SWEEP_START_OFFSETS,
    )
    parser.add_argument("--proc-sample-interval", type=float, default=0.25)
    parser.add_argument("--no-build", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release"], cwd=PROJECT_ROOT, check=True)
    if not args.emt_binary.exists():
        raise SystemExit(f"missing release binary: {args.emt_binary}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    (PROJECT_ROOT / ".artifacts").mkdir(exist_ok=True)
    patterns = args.pattern or [
        "long_lived_bursts",
        "child_over_interval",
        "child_under_interval",
        "lifetime_sweep",
        "many_short",
    ]
    results = []
    for pattern in patterns:
        print(f"pattern={pattern}", flush=True)
        results.append(
            run_pattern(
                pattern,
                args.emt_binary,
                args.monitor_duration,
                args.workload_duration,
                args.rate,
                args.scan_interval,
                args.sweep_runtimes,
                args.sweep_repetitions,
                args.sweep_start_offsets,
                args.proc_sample_interval,
            )
        )
    payload = {
        "metadata": {
            "monitor_duration_seconds": args.monitor_duration,
            "workload_duration_seconds": args.workload_duration,
            "rate_hz": args.rate,
            "scan_interval_secs": args.scan_interval,
            "sweep_runtimes": args.sweep_runtimes,
            "sweep_repetitions": args.sweep_repetitions,
            "sweep_start_offsets": args.sweep_start_offsets,
            "proc_sample_interval_seconds": args.proc_sample_interval,
            "top_group_evidence_limit": TOP_GROUP_EVIDENCE_LIMIT,
        },
        "summary": summarize(results),
        "results": [asdict(result) for result in results],
    }
    args.output.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
