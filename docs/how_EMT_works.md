# EMT: Operational Principles

EMT monitors energy consumption by combining hardware energy counters with process-level resource utilisation data to attribute a fair share of the physical power draw to the workload being monitored.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│  User Application                                   │
│                                                     │
│  with EnergyMonitor(name="task") as monitor:        │
│      run_my_workload()                              │
└──────────────────────┬──────────────────────────────┘
                       │ spawns monitoring thread
┌──────────────────────▼──────────────────────────────┐
│  EnergyMonitor (context manager)                    │
│  ├── discovers available PowerGroups                │
│  ├── creates EnergyMonitorCore                      │
│  └── starts monitoring thread                       │
└──────────────────────┬──────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────┐
│  EnergyMonitorCore (orchestrator)                   │
│  ├── RAPLSoC PowerGroup (CPU energy)                │
│  ├── NvidiaGPU PowerGroup (GPU energy)              │
│  └── TraceRecorders (optional CSV / Prometheus)     │
└─────────────────────────────────────────────────────┘
```

---

## Energy Attribution

EMT reads **total hardware energy** from the available counters (RAPL for CPUs, NVML for NVIDIA GPUs) and then **attributes a proportional share** to the monitored process based on its resource utilisation:

### CPU Energy Attribution

```
process_energy = (socket_energy - dram_energy) × normalised_cpu_util
               + dram_energy × memory_util
```

where:

- `socket_energy` — RAPL package energy delta (Joules) since the last collection interval.
- `dram_energy` — RAPL DRAM subdomain energy delta (Joules).
- `normalised_cpu_util` — the monitored process's CPU utilisation divided by the total utilisation of all active processes on the socket (capped at 1.0).
- `memory_util` — the monitored process's resident memory divided by total used system memory.

This proportional attribution correctly handles scenarios where multiple processes share the same CPU; the process only receives the share of energy proportional to its workload.

### GPU Energy Attribution

```
gpu_process_energy = Σ (gpu_zone_energy × process_gpu_memory / total_gpu_memory)
```

GPU memory utilisation is used as a proxy for compute utilisation because NVML does not expose per-process GPU compute time directly. Memory footprint correlates well with compute intensity for most ML/AI workloads.

---

## Data Collection Loop

Each `PowerGroup` runs an async collection loop at a configurable rate (default: 10 Hz):

1. Read energy delta from hardware counter (RAPL / NVML).
2. Read CPU / GPU / memory utilisation for all processes and the monitored process.
3. Compute attributed energy share.
4. Append a row to the in-memory `RotatingTrace` (automatic cleanup of old data).
5. Accumulate total consumed energy.

The monitoring thread runs an `asyncio` event loop, gathering all `PowerGroup` collection tasks concurrently so that CPU and GPU measurements interleave without blocking each other.

---

## PowerGroup Discovery

`EnergyMonitor` discovers which `PowerGroup` implementations are available at runtime:

| Condition | PowerGroup Activated |
|---|---|
| `/sys/class/powercap/intel-rapl` exists and is readable | `RAPLSoC` |
| NVML initialises without error (NVIDIA driver present) | `NvidiaGPU` |
| Neither RAPL nor NVML available | `CPUEstimator` (model-based fallback) |

This automatic discovery means the same code works on bare metal, inside containers, and on cloud VMs without any configuration changes. See [Virtualization Strategies](virtualization_strategies.md) for details on each deployment scenario.

---

## Trace Recording

Detailed per-interval measurements are optionally written to trace recorders (CSV files, Prometheus endpoint). The `RotatingTrace` mechanism bounds memory usage during long-running jobs by discarding data older than a configurable retention window. See [Trace Rotation](trace_rotation.md) for the full design.
