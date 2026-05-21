# Rust Collector: GitHub Issues

This file lists the parent issue and sub-issues to be created on GitHub for the in-progress Rust collector work. The hierarchy maps directly to the "In Progress: Rust Collector" table in [the roadmap](roadmap.md) and the two-path architecture described in [How EMT Works](how_EMT_works.md).

---

## Parent Issue

**Title:** `feat: Integrate Rust collector with Python EnergyMonitor via PyO3`

**Labels:** `enhancement`, `rust`, `in-progress`

**Body:**

> A native Rust collector (`src/`) re-implements the same data-collection pipeline as the Python `PowerGroup` classes, but with lower overhead, no `psutil` dependency, and a Polars `DataFrame`-backed rotating trace. Once accuracy is verified against the Python collector, it will be wired into the `EnergyMonitor` context manager via [PyO3](https://pyo3.rs/) — the public Python API stays unchanged.
>
> ### Current status
>
> | Component | Status |
> |---|---|
> | `Rapl` struct (RAPL collector) | ✅ Implemented |
> | `NvidiaGpu` struct (NVML collector) | ✅ Implemented |
> | `collect_process_groups()` (process tree walk) | ✅ Implemented |
> | `/proc/<pid>/stat` CPU utilisation tracking | ✅ Implemented |
> | Polars `RotatingTrace` | ✅ Implemented |
> | `EnergyGroup<T>` + Tokio async runtime | ✅ Implemented |
> | CLI binary (`energy-monitoring-tool`) | ✅ Implemented |
> | PyO3 Python bindings (`emt._rust`) | ✅ Implemented |
>
> ### Sub-issues
>
> - [ ] #N+1 Verify Rust `Rapl` collector accuracy against Python `RAPLSoC`
> - [ ] #N+2 Verify Rust `NvidiaGpu` collector accuracy against Python `NvidiaGPU`
> - [x] #N+3 Add PyO3 bindings: expose `EnergyGroup<T>` as `emt._rust` Python extension module
> - [ ] #N+4 Update `EnergyMonitor` context manager to delegate to Rust `EnergyGroup` via PyO3
> - [ ] #N+5 Implement dynamic PID refresh in Rust `EnergyGroup` (Tier 1)
> - [ ] #N+6 Implement process exit accounting in Rust `EnergyGroup` (Tier 1)
> - [ ] #N+7 End-to-end integration test: Python context manager backed by Rust collector

---

## Sub-issues

### Sub-issue 1 — Accuracy verification: `Rapl`

**Title:** `test(rust): Verify Rust Rapl collector accuracy against Python RAPLSoC`

**Labels:** `testing`, `rust`, `rapl`

**Body:**

> Before wiring the Rust collector into the Python context manager, we need confidence that `src/collectors/rapl.rs` produces the same energy readings as `emt/power_groups/rapl.py` for equivalent workloads.
>
> ### Acceptance criteria
>
> - For a CPU-bound benchmark (e.g. matrix multiply for 30 s) run on a physical host, the Rust-measured total energy is within **±2 %** of the Python-measured total.
> - Counter-overflow handling (`DeltaReader`) is verified by a unit test that injects a wrap-around value.
> - vCPU attribution matches on multi-socket systems.
>
> ### Suggested approach
>
> 1. Run the same workload twice — once measured by the Python `EnergyMonitor`, once by `energy-monitoring-tool --pid`.
> 2. Compare JSON output vs CSV trace.
> 3. Add a regression test (`tests/`) that reads the fixture outputs and asserts the tolerance.

---

### Sub-issue 2 — Accuracy verification: `NvidiaGpu`

**Title:** `test(rust): Verify Rust NvidiaGpu collector accuracy against Python NvidiaGPU`

**Labels:** `testing`, `rust`, `gpu`

**Body:**

> Verify that `src/collectors/nvidia_gpu.rs` produces the same per-process GPU energy readings as `emt/power_groups/nvidia_gpu.py`.
>
> ### Acceptance criteria
>
> - For a GPU-bound benchmark on a host with an NVIDIA GPU, the Rust-measured total GPU energy is within **±2 %** of the Python-measured total.
> - Verified on at least one NVIDIA architecture (Ampere or later).
>
> ### Notes
>
> - Requires an NVIDIA GPU; can be gated behind a CI label or run manually on a GPU runner.
> - Check that NVML initialisation failures are handled gracefully when no GPU is present.

---

### Sub-issue 3 — PyO3 bindings

**Title:** `feat(rust): Add PyO3 bindings — expose EnergyGroup<T> as emt._rust Python extension module`

**Labels:** `enhancement`, `rust`, `pyo3`

**Body:**

> Add a `pyo3` feature gate to `Cargo.toml` and implement Python-callable wrappers around `EnergyGroup<T>` so the Python context manager can drive the Rust collector without any subprocess or socket overhead.
>
> ### API surface
>
> ```python
> from emt._rust import EnergyGroup, RaplCollector, NvidiaGpuCollector
>
> group = EnergyGroup.create(collector=RaplCollector(), rate=10.0, pids=[os.getpid()])
> group.commence()       # starts Tokio background task
> group.poll_data()      # drains mpsc channel into RotatingTrace
> group.shutdown()       # signals task to stop
> energy_j = group.total_energy()   # → float (Joules)
> ```
>
> ### Acceptance criteria
>
> - `pip install emt` (or `maturin develop`) produces an `emt._rust` importable module.
> - Calling `EnergyGroup.create()` from Python does not block the GIL during the background collection loop.
> - `total_energy()` returns the same value as reading the `energy` column of the Polars trace.
>
> ### Implementation notes
>
> - Use `pyo3 = { version = "…", features = ["extension-module"] }` in `Cargo.toml`.
> - Use `maturin` for the build toolchain.
> - The existing `EnergyGroup<Rapl>` Rust code needs no changes — only thin `#[pyclass]` wrappers are required.

---

### Sub-issue 4 — Wire Rust into Python context manager

**Title:** `feat: Update EnergyMonitor context manager to delegate to Rust EnergyGroup via PyO3`

**Labels:** `enhancement`, `python`, `pyo3`

**Body:**

> Once the PyO3 bindings (sub-issue 3) are available, update `emt/energy_monitor.py` so that `EnergyMonitor.__enter__` and `__exit__` delegate to `emt._rust.EnergyGroup` instead of the Python `PowerGroup` classes.
>
> ### Constraints
>
> - The public `with EnergyMonitor(name="task") as monitor:` API must not change.
> - `monitor.total_consumed_energy` and `monitor.consumed_energy` properties must continue to work.
> - Fall back to Python `PowerGroup` classes if `emt._rust` is not importable (e.g. on Windows or in environments where the Rust extension was not compiled).
>
> ### Acceptance criteria
>
> - Existing tests in `tests/` pass unchanged after the migration.
> - A new smoke test confirms that `EnergyMonitor` uses the Rust path when `emt._rust` is importable.

---

### Sub-issue 5 — Dynamic PID refresh in Rust EnergyGroup

**Title:** `feat(rust): Dynamic PID refresh on every collection interval in EnergyGroup`

**Labels:** `enhancement`, `rust`, `process-tracking`

**Body:**

> Currently `collect_process_groups()` is called once at `EnergyGroup::create_with_collector()` time. If the monitored workload spawns child processes after collection starts, they are missed.
>
> ### Feature
>
> Add a `refresh_pids()` call at the start of every `run_monitoring_loop` iteration so that:
> - Newly spawned child processes of the root PID are picked up within one collection interval.
> - Processes that have exited are removed from the tracked set.
>
> ### Acceptance criteria
>
> - A workload that spawns 8 worker threads via `std::thread::spawn` after `commence()` has all workers attributed within one interval.
> - Covered by a unit test in `src/utils/psutils.rs`.

---

### Sub-issue 6 — Process exit accounting in Rust EnergyGroup

**Title:** `feat(rust): Process exit accounting — accumulate final energy share before removing PID`

**Labels:** `enhancement`, `rust`, `process-tracking`

**Body:**

> When a tracked process exits mid-monitoring, any energy accumulated since the last collection interval is currently lost (the process disappears from `/proc/` before the next `get_energy_trace()` call).
>
> ### Feature
>
> In `run_monitoring_loop`, detect when a previously-tracked PID is no longer present in `/proc/`, accumulate its final attributed energy into the trace, and remove it from the tracked set.
>
> ### Acceptance criteria
>
> - A workload that runs for 5 s inside the monitored window and exits mid-session contributes its energy to the total.
> - The total energy with exit accounting matches (within ±1 %) the total measured when the process stays alive for the full window.

---

### Sub-issue 7 — End-to-end integration test

**Title:** `test: End-to-end integration test — Python EnergyMonitor backed by Rust collector`

**Labels:** `testing`, `integration`

**Body:**

> Add an end-to-end integration test that exercises the full stack: Python `EnergyMonitor` context manager → PyO3 FFI → Rust `EnergyGroup<Rapl>` → Polars trace.
>
> ### Test scenarios
>
> 1. Single-process CPU-bound workload: total energy > 0 J, duration matches requested window.
> 2. Multi-process workload (spawned after `__enter__`): all child PIDs appear in the trace.
> 3. Short-lived child process: energy is non-zero and not lost after the child exits.
>
> ### Notes
>
> - Can run on CI without a GPU by skipping `NvidiaGpu` tests.
> - Should run on Linux only (RAPL requires Linux `/sys/class/powercap`).
