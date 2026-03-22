#!/usr/bin/env python3
import os
import sys
import time
import subprocess
import csv
from pathlib import Path
from emt.utils.config import load_config
from emt.utils.units import UnitConverter
from emt.utils import CSVRecorder

os.environ["EMT_RELOAD_PROCS"] = "1"

from emt import EnergyMonitor


def main():
    duration = int(sys.argv[1]) if len(sys.argv) > 1 else 30
    cpu_count = int(sys.argv[2]) if len(sys.argv) > 2 else 1

    trace_dir = Path(
        "/home/rameez-ismail/workspace/energy-monitoring-tool/logs/verification/emt_traces"
    )
    trace_dir.mkdir(parents=True, exist_ok=True)
    recorder = CSVRecorder(str(trace_dir), write_interval=duration + 5)

    with EnergyMonitor(
        name="verify_stress",
        trace_recorders=[recorder],
        startup_delay_s=0.0,
    ) as monitor:
        start = time.perf_counter()
        proc = subprocess.Popen(
            ["stress-ng", "--cpu", str(cpu_count), "--timeout", f"{duration}s"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        proc.wait()

    elapsed = time.perf_counter() - start
    trace_stats = {
        "samples": 0,
        "avg_norm_ps_util": 0.0,
        "avg_ps_util": 0.0,
        "avg_cpu_util": 0.0,
    }
    rapl_energy = 0.0
    consumed = monitor.consumed_energy
    if isinstance(consumed, dict):
        rapl_energy = float(consumed.get("RAPLSoC", 0.0))

    trace_csv = None
    rapl_traces = sorted(trace_dir.glob("RAPLSoC_*.csv"))
    if rapl_traces:
        trace_csv = rapl_traces[-1]
        with open(trace_csv, newline="") as file:
            reader = csv.DictReader(file)
            norm_vals = []
            ps_vals = []
            cpu_vals = []
            for row in reader:
                if "norm_ps_util" in row:
                    norm_vals.append(float(row["norm_ps_util"]))
                if "ps_util" in row:
                    ps_vals.append(float(row["ps_util"]))
                if "cpu_util" in row:
                    cpu_vals.append(float(row["cpu_util"]))
            if norm_vals:
                trace_stats["samples"] = len(norm_vals)
                trace_stats["avg_norm_ps_util"] = sum(norm_vals) / len(norm_vals)
            if ps_vals:
                trace_stats["avg_ps_util"] = sum(ps_vals) / len(ps_vals)
            if cpu_vals:
                trace_stats["avg_cpu_util"] = sum(cpu_vals) / len(cpu_vals)

    raw_energy = rapl_energy
    raw_unit = monitor.energy_unit
    config = load_config()
    config_unit = config.get("measurement_units", {}).get("energy", raw_unit)
    energy_joules = UnitConverter.convert_energy(raw_energy, config_unit, "Joules")

    print("method=python_emt")
    print(f"duration_seconds={elapsed:.3f}")
    print(f"energy_joules={energy_joules:.6f}")
    print("energy_unit=Joules")
    print(f"raw_energy_rapl={raw_energy}")
    print(f"raw_unit={raw_unit}")
    print(f"config_unit={config_unit}")
    print(f"trace_samples={trace_stats['samples']}")
    print(f"trace_avg_norm_ps_util={trace_stats['avg_norm_ps_util']:.6f}")
    print(f"trace_avg_ps_util={trace_stats['avg_ps_util']:.6f}")
    print(f"trace_avg_cpu_util={trace_stats['avg_cpu_util']:.6f}")
    if trace_csv is not None:
        print(f"trace_csv={trace_csv}")


if __name__ == "__main__":
    main()
