# AGENTS.md

This is the maintained project knowledge file for agents and contributors working on EMT. Keep this file concise and current. `CLAUDE.md` should only point here so guidance does not drift across files.

## Project Identity

EMT, the Energy Monitoring Tool, attributes hardware energy use to software workloads at process level. The core value is making energy visible enough for developers, researchers, and platform teams to compare workloads, identify hotspots, and report energy use in shared compute environments.

The shipping product is the Python package in `emt/`. The active strategic work is the Rust collector in `src/`, which should eventually provide the collection backend for the existing Python API through PyO3.

## Stable Product Contract

- Preserve the public Python API unless the task explicitly asks for a breaking change.
- The primary user workflow is still:

  ```python
  from emt import EnergyMonitor

  with EnergyMonitor() as monitor:
      run_workload()

  print(monitor.consumed_energy)
  ```

- `EnergyMonitor` must keep working as a context manager.
- `monitor.consumed_energy` and related total-energy properties must keep working.
- The Python API should be stable while performance-critical collection moves into Rust.

## Architecture Snapshot

### Python path, currently shipping

- `emt/energy_monitor.py` owns `EnergyMonitor`, `EnergyMonitorCore`, and the background monitoring thread.
- `EnergyMonitor` starts a dedicated OS thread so collection can continue while user code runs.
- `EnergyMonitorCore` runs an asyncio loop and gathers `PowerGroup.commence()` tasks.
- `PowerGroup` implementations read energy and utilization data:
  - RAPL CPU package and DRAM energy through `/sys/class/powercap`.
  - NVIDIA GPU energy and memory data through NVML.
- `TraceRecorder` writes traces, currently including CSV and related recorder paths.

### Rust path, actively developed

- `src/` contains the Rust collector and CLI.
- `EnergyGroup<T: EnergyCollector>` owns the generic monitor lifecycle.
- `EnergyCollector` implementations include `Rapl` and `NvidiaGpu`.
- Tokio runs the background monitoring task.
- Batched `EnergyRecord`s move over a bounded `mpsc` channel.
- `RotatingTrace` uses Polars DataFrames to bound trace memory over long sessions.
- The CLI is built from Rust and monitors a PID with arguments like `--pid`, `--duration`, and `--rate`.

### Intended integration

The target integration is PyO3:

- expose Rust collectors through an importable `emt._rust` module;
- let Python `EnergyMonitor` delegate to Rust internally;
- fall back to the Python `PowerGroup` path if the Rust extension is unavailable;
- avoid subprocess and socket overhead for the Python API path.

## Attribution Model

The formulas and semantics must remain equivalent between Python and Rust.

CPU energy attribution:

```text
process_energy = (socket_energy - dram_energy) * normalised_cpu_util
               + dram_energy * memory_util
```

Where:

- `socket_energy` is package RAPL energy delta.
- `dram_energy` is RAPL DRAM energy delta.
- `normalised_cpu_util` is the monitored process or process tree CPU share relative to active system CPU use.
- `memory_util` is the monitored process resident memory share relative to used system memory.

GPU energy attribution:

```text
gpu_process_energy = sum(gpu_zone_energy * process_gpu_memory / total_gpu_memory)
```

GPU memory is currently used as the per-process proxy because NVML does not expose reliable per-process compute energy.

## Verification Rule

Python and Rust must produce equivalent attribution for the same workload before Rust replaces Python collection.

The active parity runner is:

```bash
python scripts/verify.py
```

Useful variants:

```bash
python scripts/verify.py -n 5 -d 10
python scripts/verify.py --iterations 3 --duration 30
```

The active comparison is:

- Python EMT: `emt.EnergyMonitor`
- Rust CLI: `energy-monitoring-tool`
- Shared workload source: `scripts/verification_workload.py`
- Output: `.artifacts/verification_results.json`

Do not reintroduce the old bash baseline path. Historical notes say `verification/rapl_baseline.sh` and stress helper scripts were removed from the intended verification path.

## Commands

Python setup and checks:

```bash
pip install -e .[dev]
pytest
pytest tests/test_rapl_soc.py
pytest -k "test_name"
black .
coverage run -m pytest && coverage xml
```

Rust setup and checks:

```bash
cargo build
cargo build --release
cargo run -- --pid <PID> --duration 10 --rate 10
cargo test
```

Rust trace rotation tests:

```bash
cargo test --bin emt trace_rotation
```

## Active GitHub Backlog

Open issues as of 2026-05-21 are centered on Rust collector integration:

- #31 `feat: Integrate Rust collector with Python EnergyMonitor via PyO3`
- #32 `Verify Rust Rapl collector accuracy against Python RAPLSoC`
- #33 `Add PyO3 bindings: expose EnergyGroup<T> as emt._rust Python extension module`
- #34 `Verify Rust NvidiaGpu collector accuracy against Python NvidiaGPU`
- #35 `Update EnergyMonitor context manager to delegate to Rust EnergyGroup via PyO3`
- #36 `Implement dynamic PID refresh in Rust EnergyGroup`
- #37 `Implement process exit accounting in Rust EnergyGroup`
- #38 `End-to-end integration test: Python context manager backed by Rust collector`
- #42 `Verify Rust Rapl collector accuracy against Python RAPLSoC`
- #26 `Fix component/subcomponent extraction logic and factor it into a dedicated function`
- #7 `Using watt-wiser as a reference, expand platform/hardware support`

Treat #31 as the parent theme. The practical sequence is:

1. Verify Rust RAPL and NVIDIA collectors against Python.
2. Add PyO3 bindings.
3. Delegate `EnergyMonitor` to Rust while keeping Python fallback.
4. Add dynamic PID refresh and process exit accounting.
5. Add end-to-end integration coverage for the Python API backed by Rust.

Note that #32 and #42 appear to overlap by title. Check current GitHub state before creating new RAPL verification work.

## Issue Tracking

Feature work should be tracked in Linear. GitHub issues and PRs are still useful for repository history, review, CI, and merge state, but planning and feature status should use Linear as the source of truth when the Linear MCP server is available.

Linear is configured through the official remote MCP server:

```text
https://mcp.linear.app/mcp
```

Expected workflow:

- Start from the relevant Linear issue before implementing a feature.
- Use the Linear status flow `Backlog` -> `Todo` -> `In Review` -> `Done`.
- Use `Canceled` and `Duplicate` only as terminal exception states.
- Keep Linear issue status and comments updated when work starts, blocks, or completes.
- Reference Linear issue IDs in branch names, commits, and PR descriptions when available.
- Use GitHub PRs for code review and CI, then update/close the related Linear issue after merge.
- If Linear and GitHub disagree, verify Linear first for product/work status and GitHub for code/merge status.

## Process Tracking Rules

Process-tree attribution is central to EMT.

- Track the root PID and child processes.
- Newly spawned children should be discovered during monitoring, not only at `EnergyMonitor.__enter__`.
- Short-lived child processes should not lose their final energy contribution when they exit.
- `EMT_RELOAD_PROCS=1` enables dynamic child process discovery in the current Python workflow.
- Rust work should converge on dynamic PID refresh every collection interval plus explicit process exit accounting.

## Platform And Hardware Scope

Currently supported:

- Linux
- Intel and AMD x86 CPU package energy through RAPL-compatible powercap paths
- NVIDIA GPUs through NVML

Important constraints:

- RAPL requires readable `/sys/class/powercap`.
- `emt_cfgup` is the setup path for RAPL permissions.
- NVML support requires a working NVIDIA driver and visible GPU.
- GPU tests need suitable hardware and should be skipped or gated when unavailable.

Roadmap and issue context include broader support:

- AMD-specific energy paths such as `amd_energy`.
- Windows support through tools such as PCM or OpenHardwareMonitor.
- Model-based CPU estimation when hardware counters are unavailable.
- Platform expansion informed by watt-wiser.

## Virtualization Direction

EMT is explicitly aimed at difficult attribution environments, not only bare metal.

Key scenarios from the docs:

- Containers: use cgroup CPU and memory accounting, ideally with a host-side privileged agent for hardware counters.
- VPS/cloud guests: RAPL and NVML are usually unavailable, so use utilization plus TDP-style estimation.
- Hypervisors: host-side RAPL attribution to VM vCPU processes is more accurate than guest-only readings.
- Guest RAPL passthrough: readings may represent the physical package shared by all VMs and need normalization.
- Kubernetes: likely DaemonSet plus Prometheus, mapping process/cgroup data to pod metadata.
- Slurm/HPC: align with job lifecycle, cgroups, node heterogeneity, and restricted MSR/powercap access.

The roadmap is additive. Do not break the existing Python context-manager workflow while adding these deployment modes.

## Trace And Telemetry Direction

`RotatingTrace` exists to prevent unbounded trace growth.

Expected properties:

- bounded retention window, defaulting around one hour where configured;
- automatic cleanup on append, throttled to reduce overhead;
- Polars-backed DataFrame in Rust;
- future-friendly export paths such as CSV, Arrow, Parquet, and Prometheus-style telemetry.

Prometheus telemetry appears in docs as a target integration path, especially for platform and Kubernetes use cases. Keep metric labels low-cardinality.

## Documentation Notes

Docs are not fully synchronized. Prefer these stable facts:

- Python API is the stable user contract.
- Rust collector is the performance and integration direction.
- Python vs Rust parity verification is a hard gate.
- Virtualization support is a roadmap direction, with model-based fallback where counters are unavailable.

Known inconsistencies or stale references:

- Some docs mention eBPF or Prometheus as if already central; verify implementation before relying on those claims.
- Verification docs say bash baseline scripts were removed from the active path; do not revive that workflow without a deliberate decision.
- GitHub has overlapping RAPL verification issues by title.

## Working Conventions

- Keep changes scoped to the request.
- Use existing Python style in `emt/` and `tests/`.
- Use standard Rust formatting and idioms in `src/`.
- Prefer structured parsers and existing helper APIs over ad hoc string parsing.
- Add tests proportional to risk and public surface area.
- For public API behavior, add regression tests.
- For Rust collector parity, prefer verification fixtures or explicit hardware-gated tests.
- Do not edit generated or historical artifacts unless the task requires it.

## Code Quality

- Keep code simple, typed where useful, and easy to review.
- Avoid broad exception handling unless the fallback behavior is deliberate and documented in the surrounding code.
- Do not leave dead code, stale paths, conflict markers, debug prints, or generated local outputs in commits.
- Keep verification scripts deterministic apart from explicit hardware measurements and record host-specific evidence only under ignored artifact paths.
- Run Black on Python files before publishing changes.
- Treat SonarQube reliability, maintainability, and security findings as blocking unless the finding is clearly false-positive and documented in the PR.

## Quality Gates

- Black formatting for Python.
- Python tests across supported versions in CI.
- Rust tests for collector and trace behavior.
- SonarQube quality gate for reliability, maintainability, and security.
- PRs target `main`.

## Source Files Consulted

This file consolidates:

- `CLAUDE.md`
- `README.md`
- `docs/how_EMT_works.md`
- `docs/roadmap.md`
- `docs/rust_collector_issues.md`
- `docs/trace_rotation.md`
- `docs/virtualization_challenges.md`
- `docs/virtualization_strategies.md`
- `docs/executive_summary.md`
- `docs/getting_started.md`
- `docs/introduction.md`
- `verification/summary.md`
- GitHub open issue list for `FairCompute/energy-monitoring-tool`
