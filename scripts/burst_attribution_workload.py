#!/usr/bin/env python3
"""
Controlled bursty and short-lived process workload for ENE-44.

The script prints JSON lines describing every expected process. The benchmark
harness uses those events as ground truth when checking EMT's final snapshot.
"""

from __future__ import annotations

import argparse
import json
import multiprocessing as mp
import os
import time
from typing import Any


def emit(event: dict[str, Any]) -> None:
    print(json.dumps(event, sort_keys=True), flush=True)


def read_own_cpu_ticks() -> int | None:
    try:
        stat = open(
            f"/proc/{os.getpid()}/stat", encoding="utf-8", errors="ignore"
        ).read()
    except OSError:
        return None
    fields = stat[stat.rfind(")") + 2 :].split()
    if len(fields) < 13:
        return None
    try:
        return int(fields[11]) + int(fields[12])
    except ValueError:
        return None


def busy_for(seconds: float) -> None:
    deadline = time.perf_counter() + seconds
    value = 0.0
    while time.perf_counter() < deadline:
        for index in range(2_000):
            value += (index * 31 % 17) / 19.0
    if value < 0:
        print(value)


def child_worker(seconds: float, role: str) -> None:
    start_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_start",
            "role": role,
            "pid": os.getpid(),
            "timestamp": time.time(),
            "self_cpu_ticks": start_cpu_ticks,
        }
    )
    busy_for(seconds)
    end_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_end",
            "role": role,
            "pid": os.getpid(),
            "timestamp": time.time(),
            "runtime_seconds": seconds,
            "self_cpu_ticks": end_cpu_ticks,
            "self_cpu_ticks_delta": (
                None
                if start_cpu_ticks is None or end_cpu_ticks is None
                else max(0, end_cpu_ticks - start_cpu_ticks)
            ),
        }
    )


def long_lived_bursts(
    duration: float, burst_seconds: float, idle_seconds: float
) -> None:
    start_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_start",
            "role": "root",
            "pid": os.getpid(),
            "timestamp": time.time(),
            "self_cpu_ticks": start_cpu_ticks,
        }
    )
    deadline = time.perf_counter() + duration
    bursts = 0
    while time.perf_counter() < deadline:
        busy_for(min(burst_seconds, max(0.0, deadline - time.perf_counter())))
        bursts += 1
        time.sleep(min(idle_seconds, max(0.0, deadline - time.perf_counter())))
    end_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_end",
            "role": "root",
            "pid": os.getpid(),
            "timestamp": time.time(),
            "runtime_seconds": duration,
            "bursts": bursts,
            "self_cpu_ticks": end_cpu_ticks,
            "self_cpu_ticks_delta": (
                None
                if start_cpu_ticks is None or end_cpu_ticks is None
                else max(0, end_cpu_ticks - start_cpu_ticks)
            ),
        }
    )


def one_child(duration: float, child_runtime: float, role: str) -> None:
    start_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_start",
            "role": "root",
            "pid": os.getpid(),
            "timestamp": time.time(),
            "self_cpu_ticks": start_cpu_ticks,
        }
    )
    process = mp.Process(target=child_worker, args=(child_runtime, role))
    process.start()
    emit(
        {
            "event": "child_spawned",
            "role": role,
            "pid": process.pid,
            "timestamp": time.time(),
            "expected_runtime_seconds": child_runtime,
        }
    )
    process.join()
    time.sleep(max(0.0, duration - child_runtime))
    end_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_end",
            "role": "root",
            "pid": os.getpid(),
            "timestamp": time.time(),
            "runtime_seconds": duration,
            "self_cpu_ticks": end_cpu_ticks,
            "self_cpu_ticks_delta": (
                None
                if start_cpu_ticks is None or end_cpu_ticks is None
                else max(0, end_cpu_ticks - start_cpu_ticks)
            ),
        }
    )


def many_short_children(
    duration: float, child_runtime: float, spawn_interval: float
) -> None:
    start_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_start",
            "role": "root",
            "pid": os.getpid(),
            "timestamp": time.time(),
            "self_cpu_ticks": start_cpu_ticks,
        }
    )
    deadline = time.perf_counter() + duration
    processes: list[mp.Process] = []
    spawned = 0
    while time.perf_counter() < deadline:
        process = mp.Process(target=child_worker, args=(child_runtime, "short_child"))
        process.start()
        spawned += 1
        processes.append(process)
        emit(
            {
                "event": "child_spawned",
                "role": "short_child",
                "pid": process.pid,
                "timestamp": time.time(),
                "expected_runtime_seconds": child_runtime,
            }
        )
        time.sleep(min(spawn_interval, max(0.0, deadline - time.perf_counter())))
    for process in processes:
        process.join()
    end_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_end",
            "role": "root",
            "pid": os.getpid(),
            "timestamp": time.time(),
            "runtime_seconds": duration,
            "children_spawned": spawned,
            "self_cpu_ticks": end_cpu_ticks,
            "self_cpu_ticks_delta": (
                None
                if start_cpu_ticks is None or end_cpu_ticks is None
                else max(0, end_cpu_ticks - start_cpu_ticks)
            ),
        }
    )


def lifetime_sweep(
    duration: float,
    child_runtimes: list[float],
    repetitions: int,
    start_offsets: list[float],
) -> None:
    start_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_start",
            "role": "root",
            "pid": os.getpid(),
            "timestamp": time.time(),
            "self_cpu_ticks": start_cpu_ticks,
        }
    )
    processes: list[mp.Process] = []
    sweep_start = time.perf_counter()
    for repetition in range(repetitions):
        offset = start_offsets[repetition] if repetition < len(start_offsets) else 0.0
        while time.perf_counter() - sweep_start < offset:
            time.sleep(min(0.1, offset - (time.perf_counter() - sweep_start)))
        for runtime in child_runtimes:
            role = f"sweep_child_{runtime:.1f}s"
            process = mp.Process(target=child_worker, args=(runtime, role))
            process.start()
            processes.append(process)
            emit(
                {
                    "event": "child_spawned",
                    "role": role,
                    "pid": process.pid,
                    "timestamp": time.time(),
                    "expected_runtime_seconds": runtime,
                    "sweep_repetition": repetition + 1,
                    "sweep_start_offset_seconds": offset,
                }
            )
    for process in processes:
        process.join()
    elapsed = time.perf_counter() - sweep_start
    time.sleep(max(0.0, duration - elapsed))
    actual_runtime = time.perf_counter() - sweep_start
    end_cpu_ticks = read_own_cpu_ticks()
    emit(
        {
            "event": "process_end",
            "role": "root",
            "pid": os.getpid(),
            "timestamp": time.time(),
            "runtime_seconds": actual_runtime,
            "configured_duration_seconds": duration,
            "sweep_children_spawned": len(processes),
            "self_cpu_ticks": end_cpu_ticks,
            "self_cpu_ticks_delta": (
                None
                if start_cpu_ticks is None or end_cpu_ticks is None
                else max(0, end_cpu_ticks - start_cpu_ticks)
            ),
        }
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "pattern",
        choices=[
            "long_lived_bursts",
            "child_over_interval",
            "child_under_interval",
            "lifetime_sweep",
            "many_short",
        ],
    )
    parser.add_argument("--duration", type=float, default=20.0)
    parser.add_argument("--child-runtime", type=float, default=2.0)
    parser.add_argument("--burst-seconds", type=float, default=0.2)
    parser.add_argument("--idle-seconds", type=float, default=0.8)
    parser.add_argument("--spawn-interval", type=float, default=1.0)
    parser.add_argument("--child-runtimes", default="")
    parser.add_argument("--sweep-repetitions", type=int, default=2)
    parser.add_argument("--sweep-start-offsets", default="")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.pattern == "long_lived_bursts":
        long_lived_bursts(args.duration, args.burst_seconds, args.idle_seconds)
    elif args.pattern == "child_over_interval":
        one_child(args.duration, args.child_runtime, "child_over_interval")
    elif args.pattern == "child_under_interval":
        one_child(args.duration, args.child_runtime, "child_under_interval")
    elif args.pattern == "lifetime_sweep":
        runtimes = [
            float(value) for value in args.child_runtimes.split(",") if value.strip()
        ]
        offsets = [
            float(value)
            for value in args.sweep_start_offsets.split(",")
            if value.strip()
        ]
        lifetime_sweep(args.duration, runtimes, args.sweep_repetitions, offsets)
    elif args.pattern == "many_short":
        many_short_children(args.duration, args.child_runtime, args.spawn_interval)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
