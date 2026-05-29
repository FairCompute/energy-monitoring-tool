# EMT: Operational Principles

EMT monitors energy consumption by combining hardware energy counters with process-level resource utilisation data to attribute a fair share of the physical power draw to the workload being monitored.

The **energy attribution formulas and the user-facing Python API do not change** across any architectural transition. What evolves is the layer that performs the actual hardware reads and process tracking — moving from Python to a high-performance Rust collector invoked via [PyO3](https://pyo3.rs/).

---

## Current Architecture (Python Collector)

The shipping implementation is entirely Python. The `EnergyMonitor` context manager discovers and drives Python `PowerGroup` objects, each of which reads hardware counters and attributes energy directly.

```
┌─────────────────────────────────────────────────────┐
│  User Application                                   │
│                                                     │
│  with EnergyMonitor(name="task") as monitor:        │
│      run_my_workload()                              │
└──────────────────────┬──────────────────────────────┘
                       │ spawns OS thread
┌──────────────────────▼──────────────────────────────┐
│  EnergyMonitor  (emt/energy_monitor.py)             │
│  ├── calls get_available_pgs() for auto-discovery   │
│  ├── creates EnergyMonitorCore                      │
│  └── starts EnergyMonitoringThread                  │
└──────────────────────┬──────────────────────────────┘
                       │ asyncio event loop in thread
┌──────────────────────▼──────────────────────────────┐
│  EnergyMonitorCore  (emt/energy_monitor.py)         │
│  ├── gathers asyncio tasks for each PowerGroup      │
│  ├── runs TraceRecorder tasks (CSV / Prometheus)    │
│  └── handles shutdown via asyncio.CancelledError    │
└──────────────────────┬──────────────────────────────┘
                       │ concurrent asyncio tasks
       ┌───────────────┴────────────────┐
       ▼                                ▼
┌──────────────┐               ┌────────────────┐
│  RAPLSoC     │               │  NvidiaGPU     │
│  PowerGroup  │               │  PowerGroup    │
│              │               │                │
│  reads RAPL  │               │  reads NVML    │
│  /sys/class/ │               │  per-process   │
│  powercap    │               │  GPU memory    │
└──────────────┘               └────────────────┘
```

### Python Collection Loop

Each `PowerGroup.commence()` coroutine runs the following steps on every tick (default: 10 Hz):

1. Read energy delta from the hardware counter (`/sys/class/powercap/…/energy_uj` for RAPL, NVML for GPU).
2. Read CPU utilisation from `psutil` for the tracked process tree and all other processes on the socket.
3. Read memory utilisation from `psutil`.
4. Compute attributed energy share (see [Energy Attribution](#energy-attribution) below).
5. Append a row to the in-memory `RotatingTrace`.
6. Accumulate total consumed energy in the `PowerGroup` instance.

The `asyncio` event loop gathers all `PowerGroup` coroutines so CPU and GPU measurements interleave without blocking each other. Because the main application thread may be CPU-bound, the monitoring loop runs in a **dedicated OS thread** with its own `asyncio.run()` call.

---

## Transition Architecture (Rust Collector via PyO3)

A Rust collector is in active development (`src/` in the repository). It now exposes PyO3 bindings as `emt._rust`; once the remaining integration work is complete, Python `EnergyMonitor` will delegate to those bindings instead of the Python `PowerGroup` implementations. The Python context manager API (`with EnergyMonitor(…) as monitor:`) and the energy attribution formulas **remain unchanged**. Only the internal data-collection layer changes.

The Rust collector is exposed to Python via [PyO3](https://pyo3.rs/) — a zero-overhead Rust↔Python FFI bridge. The next integration step is for the Python `EnergyMonitor` context manager to call into native Rust code without spawning a subprocess or using sockets.

```
┌─────────────────────────────────────────────────────┐
│  User Application  (unchanged)                      │
│                                                     │
│  with EnergyMonitor(name="task") as monitor:        │
│      run_my_workload()                              │
└──────────────────────┬──────────────────────────────┘
                       │ spawns OS thread (unchanged)
┌──────────────────────▼──────────────────────────────┐
│  EnergyMonitor  (Python, unchanged public API)      │
│  ├── auto-discovers available collectors            │
│  └── starts EnergyMonitoringThread                  │
└──────────────────────┬──────────────────────────────┘
                       │ calls via PyO3 FFI
┌──────────────────────▼──────────────────────────────┐
│  EnergyGroup<T>  (Rust, src/energy_group.rs)        │
│                                                     │
│  Generic over any EnergyCollector trait impl.       │
│  ├── commence()  → spawns Tokio async background    │
│  │               task at configurable Hz            │
│  ├── poll_data() → drains mpsc channel into         │
│  │               RotatingTrace (Polars DataFrame)   │
│  └── shutdown()  → signals background task to stop  │
└──────────────────────┬──────────────────────────────┘
                       │ Tokio async runtime
       ┌───────────────┴────────────────┐
       ▼                                ▼
┌──────────────┐               ┌────────────────┐
│  Rapl        │               │  NvidiaGpu     │
│  (Rust)      │               │  (Rust)        │
│              │               │                │
│  reads RAPL  │               │  reads NVML    │
│  /sys/class/ │               │  per-process   │
│  powercap    │               │  GPU memory    │
│  /proc/stat  │               │                │
│  for CPU%    │               │                │
└──────────────┘               └────────────────┘
```

### Rust Collection Loop

The Rust `EnergyGroup<T>::run_monitoring_loop()` runs at a configurable rate inside a Tokio async task:

1. Call `collector.get_energy_trace()` — reads hardware delta + process CPU times from `/proc/<pid>/stat` and `/proc/stat`.
2. Batch the resulting `EnergyRecord` structs until `batch_size` is reached (default: 1 000 records).
3. Send the batch over a bounded `mpsc` channel back to the main `EnergyGroup` (backpressure via bounded channel prevents unbounded memory growth).
4. On the Python side, `poll_data()` drains the channel and appends records to the in-memory `RotatingTrace` (a Polars `DataFrame`).

### How the Rust Collector Simplifies the Flow

The Rust implementation consolidates several concerns that are spread across multiple Python classes:

| Python (current) | Rust (planned) |
|---|---|
| `EnergyMonitor` + `EnergyMonitorCore` + `PowerGroup` (three layers) | `EnergyGroup<T>` — one generic struct owns the lifecycle |
| `asyncio` event loop inside a dedicated OS thread | Tokio async runtime — lighter, does not block the GIL |
| `psutil` Python calls for CPU utilisation | Direct `/proc/stat` and `/proc/<pid>/stat` reads in Rust |
| `psutil` process tree walk on every `EnergyMonitor.__enter__` | `collect_process_groups()` with recursive child expansion, refreshable on demand |
| Python `RotatingTrace` list of dicts | Polars `DataFrame` — columnar, zero-copy, easily exported to CSV / Arrow / Parquet |
| CLI written in Python (`python -m emt --pid …`) | Native binary (`emt --pid …`) compiled from `src/main.rs` |

### PyO3 Integration Points

The compiled `emt._rust` extension exposes the following Rust symbols to Python:

```python
from emt._rust import EnergyGroup, RaplCollector, NvidiaGpuCollector

# EnergyMonitor.__enter__ will internally call:
group = EnergyGroup.create(collector=RaplCollector(), rate=10.0, pids=[os.getpid()])
group.commence()          # starts Tokio background task

# EnergyMonitor.__exit__ will internally call:
group.poll_data()
group.shutdown()
energy_joules = group.total_energy()
```

The Python context manager (`with EnergyMonitor(…) as monitor:`) remains the public API. Users who already use EMT do not need to change their code.

### CLI Path (Rust Binary)

The Rust binary (`emt`) is built directly from `src/main.rs` and does not depend on Python. It provides a standalone CLI for monitoring a PID without writing Python code:

```bash
# Monitor PID 1234 for 30 seconds at 10 Hz, write JSON to results.json
emt --pid 1234 --duration 30 --rate 10 --json-out results.json
```

The Python CLI (`python -m emt …`) will delegate to the same Rust binary once the PyO3 bridge is complete.

---

## Energy Attribution

The attribution formulas are **identical in both the Python and Rust implementations**. EMT reads **total hardware energy** from the available counters and **attributes a proportional share** to the monitored process based on its resource utilisation.

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

**Python**: `normalised_cpu_util` is computed using `psutil.cpu_percent(percpu=True)`.  
**Rust**: `normalised_cpu_util` is computed by reading `/proc/<pid>/stat` (utime + stime) and `/proc/stat` (total CPU time) directly.

### GPU Energy Attribution

```
gpu_process_energy = Σ (gpu_zone_energy × process_gpu_memory / total_gpu_memory)
```

GPU memory utilisation is used as a proxy for compute utilisation because NVML does not expose per-process GPU compute time directly. Memory footprint correlates well with compute intensity for most ML/AI workloads.

---

## Collector Auto-Discovery

Both the Python and Rust paths use the same priority order for selecting the active collector(s):

| Condition | Collector Activated |
|---|---|
| `/sys/class/powercap/intel-rapl` exists and is readable | `RAPLSoC` (Python) / `Rapl` (Rust) |
| NVML initialises without error (NVIDIA driver present) | `NvidiaGPU` (Python) / `NvidiaGpu` (Rust) |
| Neither RAPL nor NVML available | `CPUEstimator` (model-based fallback — planned) |

This automatic discovery means the same code works on bare metal, inside containers, and on cloud VMs without any configuration changes. See [Virtualization Strategies](virtualization_strategies.md) for details on each deployment scenario.

---

## Trace Recording

Detailed per-interval measurements are optionally written to trace recorders (CSV files, Prometheus endpoint). The `RotatingTrace` mechanism bounds memory usage during long-running jobs by discarding data older than a configurable retention window. Both the Python and Rust implementations share the same `RotatingTrace` design. See [Trace Rotation](trace_rotation.md) for the full design.
