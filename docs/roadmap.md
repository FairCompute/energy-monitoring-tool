# EMT Feature Roadmap and Blueprint

This document translates the challenges described in [Virtualization Challenges](virtualization_challenges.md) into a concrete, prioritised feature roadmap. Each tier builds on the previous one, progressing from the simplest single-host scenario to fully distributed, heterogeneous deployments.

For a detailed explanation of the current Python flow and the Rust/PyO3 transition path, see [How EMT Works](how_EMT_works.md).

---

## Tier 0 — Current State and In-Progress Work

### Today: Python Collector (Shipping)

EMT's shipping implementation is **entirely Python**. The process being monitored runs on a physical host with unrestricted access to hardware energy counters.

| Component | Status | Implementation |
|---|---|---|
| RAPL (Intel/AMD CPU energy) | ✅ Shipping | Python `RAPLSoC` PowerGroup |
| NVML (NVIDIA GPU energy) | ✅ Shipping | Python `NvidiaGPU` PowerGroup |
| Process-level CPU energy attribution | ✅ Shipping | proportional CPU utilisation via `psutil` |
| Process-level GPU energy attribution | ✅ Shipping | proportional GPU memory via NVML |
| Dynamic child-process tracking | ✅ Shipping | `psutil` process tree walk |
| Rotating in-memory trace | ✅ Shipping | Python `RotatingTrace` (list of dicts) |
| CSV trace export | ✅ Shipping | `TraceRecorder` |
| Python context manager API | ✅ Shipping | `with EnergyMonitor(…) as monitor:` |
| CLI monitor (`--pid`) | 🚧 In progress | Rust binary (`energy-monitoring-tool`) |

### In Progress: Rust Collector

A Rust collector is under active development in `src/`. It re-implements the same collection pipeline in native code and exposes PyO3 bindings; the remaining integration work is wiring the Python context manager to those bindings while preserving the Python fallback.

| Component | Status | Implementation |
|---|---|---|
| RAPL collector | ✅ Implemented | Rust `Rapl` struct — reads `/sys/class/powercap` and `/proc/stat` |
| NVIDIA GPU collector | ✅ Implemented | Rust `NvidiaGpu` struct |
| Process tree walk | ✅ Implemented | `collect_process_groups()` with recursive child expansion |
| CPU utilisation tracking | ✅ Implemented | Direct `/proc/<pid>/stat` reads (no `psutil` dependency) |
| Rotating in-memory trace | ✅ Implemented | Polars `DataFrame` backed `RotatingTrace` |
| Tokio async runtime | ✅ Implemented | `EnergyGroup<T>` with bounded `mpsc` channel for backpressure |
| CLI binary | ✅ Implemented | `energy-monitoring-tool --pid <PID> --duration <s>` |
| PyO3 Python bindings | ✅ Implemented | Exposes `EnergyGroup`, `RaplCollector`, and `NvidiaGpuCollector` to Python as `emt._rust` |
| Python context-manager delegation to Rust | 🔜 Planned | Will let `EnergyMonitor` use `emt._rust` internally with Python fallback |

### What Changes vs. What Stays the Same

| Aspect | Current (Python) | Planned (Rust via PyO3) |
|---|---|---|
| Public Python API | `with EnergyMonitor(name="task") as monitor:` | **Unchanged** |
| Energy attribution formulas | Proportional CPU + memory utilisation | **Unchanged** |
| Supported hardware | RAPL (Intel/AMD), NVML (NVIDIA) | **Unchanged** + AMD `amd_energy` driver |
| Hardware reads | Python file I/O + `psutil` | Rust direct `/proc` and `/sys` reads |
| Async runtime | Python `asyncio` in a dedicated OS thread | Tokio inside a Rust background task |
| CLI | `python -m emt --pid …` (delegates to Rust binary) | Native Rust `energy-monitoring-tool` binary |
| Trace format | Python list of dicts → CSV | Polars `DataFrame` → CSV / Arrow / Parquet |

All subsequent roadmap tiers are **additive** — each tier extends EMT without breaking the existing API, and each tier's features apply equally to the Python and Rust collector paths.

---

## Tier 1 — Process Tree Energy Attribution on a Physical Host

**Priority 1 — Completed baseline with full process-tree scope**

### Goal

Ensure that the existing host-level attribution correctly covers the **entire process tree** spawned by the monitored workload, including multi-process launchers (e.g. `torch.distributed`, `multiprocessing.Pool`, MPI rank 0).

### Current Gap

EMT today tracks processes by walking the psutil / sysinfo child tree from a root PID. If a child process is spawned after the `EnergyMonitor.__enter__` call, it may be missed during the first collection interval.

### Feature Plan

| Feature | Description |
|---|---|
| **Dynamic PID discovery** | Re-walk the process tree on every collection interval so newly spawned children are included automatically |
| **Orphan re-parenting** | Detect processes that were spawned by a tracked root but whose parent PID has since changed (re-parented to init) and continue tracking them |
| **Process exit accounting** | When a tracked process exits, accumulate its final energy share into the total before removing it from the tracking set |
| **Attribution metadata in trace** | Record which PIDs contributed to each energy sample, enabling post-hoc analysis of per-process shares |

### Architecture Change

```
EnergyGroup<T>  (Rust) / EnergyMonitorCore  (Python — current)
  └── PowerGroup.commence() / EnergyCollector.get_energy_trace()
        └── ProcessTracker
              ├── walk_process_tree(root_pid)     ← already exists (both paths)
              ├── refresh_children()              ← NEW: called every interval
              └── finalize_exited_processes()     ← NEW: drain exited PIDs
```

### Acceptance Criteria

- A workload that spawns 8 worker processes via `multiprocessing.Pool` within the monitored block has **all 8 workers** tracked within one collection interval.
- Energy accumulated by a worker process that exits mid-monitoring is **not lost**.

---

## Tier 2 — Container with Host-Level Hardware Counter Access

**Priority 2 — Containerised workload + privileged host-side tracker**

### Goal

Support the case where the **compute workload runs inside a container** (Docker, Podman, LXC) but the **host has full RAPL/NVML access**. A second EMT instance — or a lightweight host-level sidecar — runs on the host and attributes energy to the container's cgroup.

### Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│  Physical Host                                                   │
│                                                                  │
│  ┌────────────────────────────┐  ┌───────────────────────────┐  │
│  │  Container (workload)      │  │  Host-level EMT sidecar   │  │
│  │                            │  │  (privileged, RAPL access)│  │
│  │  EnergyMonitor (in-proc)   │  │                           │  │
│  │  ├── collects cgroup CPU   │  │  ├── reads RAPL delta     │  │
│  │  │   and memory stats      │  │  ├── reads /sys/fs/cgroup │  │
│  │  └── sends util to sidecar │  │  │   per container ID     │  │
│  │      via Unix socket       │  │  └── computes attribution │  │
│  └────────────────────────────┘  └───────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

The host sidecar will be implemented as the **Rust `energy-monitoring-tool` binary** (already available) deployed with appropriate host mounts, removing the need for a separate Python process.

### Feature Plan

| Feature | Description |
|---|---|
| **cgroup v2 reader** | Read `cpu.stat` and `memory.current` from the container's cgroup path (`/sys/fs/cgroup/…/<container_id>/`) to get precise CPU time without needing `CAP_SYS_RAWIO` inside the container |
| **Container-scoped `PowerGroup`** | New `ContainerCgroup` power group that reads cgroup accounting data and queries the host sidecar for RAPL energy via a local IPC endpoint |
| **Host sidecar daemon** | Lightweight daemon (`emt-host-agent`) that exposes a Unix socket or HTTP API; accepts cgroup paths and returns attributed energy shares |
| **Attribution formula** | `container_energy = socket_energy * (container_cgroup_cpu_ns / host_total_cpu_ns)` |
| **Docker label injection** | Automatically resolve container ID → Docker labels (image, compose service, stack) for rich trace annotation |

### Acceptance Criteria

- Energy attributed to the container matches within ±5 % of what the same workload reports in Tier 1 (bare-metal).
- Works without granting `CAP_SYS_RAWIO` or `--privileged` to the container.

---

## Tier 3 — Container with No Hardware Counter Access (TDP Model)

**Priority 3 — Containerised workload, host has only utilisation data**

### Goal

Support cloud-hosted containers (e.g. AWS ECS, Azure Container Instances, GCP Cloud Run) where **neither the container nor the host exposes RAPL**. Energy must be estimated using a CPU utilisation × TDP model.

### Architecture

```
Container / VPS / Serverless Environment
  └── EnergyMonitor (Python context manager — unchanged API)
        └── CPUEstimator PowerGroup  (NEW)
              ├── detect_cpu_model()         → reads /proc/cpuinfo
              ├── lookup_tdp(cpu_model)      → Teads TDP CSV lookup
              └── estimate_energy(util, tdp) → TDP_idle + (TDP_max - TDP_idle) × util
```

With the Rust path, `CPUEstimator` will be implemented as a Rust `EnergyCollector` trait implementation inside `EnergyGroup<CPUEstimator>` — the same generic pattern already used by `Rapl` and `NvidiaGpu`.

### Feature Plan

| Feature | Description |
|---|---|
| **`CPUEstimator` PowerGroup** | New power group that is activated automatically when RAPL and NVML are both unavailable |
| **Teads TDP database** | Bundle the [Teads Engineering CPU TDP dataset](https://github.com/teads/TeadsTV-CO2-Measurement) as a CSV file (`emt/data/teads_cpu_tdp.csv`); indexed by CPU model string |
| **Fuzzy model matching** | Use edit-distance matching to handle CPU model string variations between `/proc/cpuinfo` and the TDP database |
| **Idle power calibration** | Measure a 2-second idle baseline at `EnergyMonitor.__enter__` time to anchor the TDP model to the actual running system |
| **Accuracy disclaimer in trace** | Record `source: estimated` vs `source: measured` in every trace row so consumers know which rows come from models |
| **Memory energy estimation** | Use LPDDR/DDR memory bandwidth (via `/proc/vmstat`) as a proxy to estimate DRAM energy contribution |

### Acceptance Criteria

- On a bare-metal host where both RAPL and `CPUEstimator` are available, the estimated value is within ±20 % of the RAPL-measured value for CPU-bound workloads.
- On a VPS where RAPL is unavailable, `CPUEstimator` activates transparently without any user configuration.

---

## Tier 4 — Hypervisor / Multiple Guest OS

**Priority 4 — One guest monitors itself; host may or may not expose counters**

### Goal

Support on-premises virtualisation (KVM, VMware, Hyper-V) where the compute workload runs in a guest VM. The guest **may or may not** have access to hardware performance counters, but it **always lacks visibility of other guests** sharing the same physical host.

### Sub-scenarios

#### 4a — Guest with RAPL MSR passthrough

The hypervisor exposes RAPL MSRs to the guest (e.g. KVM with `msr-safe`). Raw readings represent the **entire physical socket** (all VMs combined), so a normalisation step is needed.

| Feature | Description |
|---|---|
| **vCPU share normalisation** | Detect the guest's vCPU allocation (`/sys/hypervisor/`, DMI, or `lscpu`) and scale RAPL readings by `guest_vcpu / host_total_vcpu` |
| **Hypervisor type detection** | Auto-detect virtualisation layer (KVM, VMware, Hyper-V, Xen) via CPUID flags or `/sys/class/dmi/id/product_name`; adjust normalisation strategy accordingly |
| **Live migration guard** | Detect RAPL counter reset (large negative delta) and insert a measurement breakpoint to avoid corrupted attributions |

#### 4b — Guest without RAPL access (model-based, as per Tier 3)

Fall back to `CPUEstimator` (Tier 3). No additional feature work required beyond enabling the estimator inside VMs.

#### 4c — Host-side per-VM attribution (dual-layer, optional)

When a privileged host agent is available, correlate host RAPL readings with per-VM vCPU time to produce **ground-truth guest energy** independently of what the guest can measure.

| Feature | Description |
|---|---|
| **VM discovery** | Enumerate active VMs via libvirt API, extracting vCPU thread PIDs for each domain |
| **Per-VM RAPL attribution** | Attribute socket energy to VMs using the same CPU-time proportional formula as Tier 2 |
| **Guest ↔ host correlation API** | Optional host agent endpoint that a guest can query to retrieve its attributed energy (requires shared network or virtio socket) |

The host agent will be implemented as the **Rust `energy-monitoring-tool` binary** running with `--pid` set to the vCPU thread PIDs of each guest domain.

### Architecture (Dual-Layer)

```
Physical Host
  ├── Host EMT agent (Rust binary, privileged)
  │     ├── reads RAPL
  │     ├── queries libvirt for VM vCPU PIDs
  │     ├── attributes energy per VM
  │     └── exposes /vm/<uuid>/energy via virtio/vsock
  │
  └── Guest VM
        └── EnergyMonitor (Python context manager — unchanged API)
              ├── tries Rapl collector (succeeds if MSR passthrough enabled)
              ├── falls back to CPUEstimator
              └── optionally queries host agent via vsock for corrected reading
```

### Acceptance Criteria

- Guest-reported energy (with normalisation) is within ±10 % of host-attributed energy for a CPU-bound workload.
- Live-migration discontinuity does not cause negative energy totals.

---

## Tier 5 — Combined Scenarios (Guest + Container)

**Priority 5 — Containers inside a VM (4+2 and 4+3)**

### Goal

Support the most common cloud deployment pattern: containers (Docker, Podman) running inside a VM, where the VM is one of many guests on a shared physical host. This combines the cgroup attribution of Tier 2 with the vCPU normalisation of Tier 4.

### Attribution Chain

```
Physical socket energy
  → attributed to VM (Tier 4: RAPL * vCPU share or model-based)
    → attributed to container cgroup (Tier 2: cgroup CPU time share)
      → attributed to process tree (Tier 1: process CPU utilisation)
```

### Feature Plan

| Feature | Description |
|---|---|
| **Chained attribution pipeline** | Compose Tier 1 + Tier 2 + Tier 4 attribution into a single `AttributionChain` class; each stage receives the energy budget from the stage above |
| **Automatic layer detection** | At startup, auto-detect the active layers (bare metal / VM / container) and build the appropriate attribution chain |
| **Normalised accuracy reporting** | Track the compounded uncertainty across all attribution stages and surface it as a confidence interval in the trace output |

### Acceptance Criteria

- End-to-end energy estimate for a container inside a VM is within ±15 % of a ground-truth measurement taken on equivalent bare metal.
- Attribution chain is assembled automatically without user configuration.

---

## Tier 6 — Kubernetes on a Single Host (k3s / k3d)

**Priority 6 — Single-node Kubernetes cluster**

### Goal

Support the case where a Kubernetes cluster (typically k3s or k3d for local or edge deployments) runs on a **single physical host**. Energy must be attributed to Kubernetes pods, not just raw process PIDs.

### Architecture

```
Single Host (k3s node)
  └── EMT DaemonSet pod (Rust binary or Python, privileged)
        ├── reads RAPL via /sys/class/powercap (host volume mount)
        ├── reads /proc on host (hostPID: true)
        ├── resolves PID → container ID → pod UID via /proc/<pid>/cgroup
        ├── queries Kubernetes API for pod metadata (labels, namespace, owner)
        └── exposes Prometheus metrics labelled by pod/namespace/label
```

The DaemonSet will preferably run the **Rust binary** for lower overhead and no Python/pip dependency inside the container image.

### Feature Plan

| Feature | Description |
|---|---|
| **cgroup → pod resolver** | Parse `/proc/<pid>/cgroup` to extract the Kubernetes-assigned container ID; resolve to pod UID via the local kubelet API (`/api/v1/pods`) |
| **Kubernetes metadata enrichment** | Annotate every energy sample with `namespace`, `pod_name`, `node_name`, `app.kubernetes.io/name`, and custom labels from the pod spec |
| **Prometheus exporter** | Expose `emt_pod_energy_joules_total`, `emt_pod_power_watts`, and `emt_node_energy_joules_total` metrics on a configurable port |
| **DaemonSet Helm chart** | Provide an official Helm chart for single-command deployment on k3s clusters |
| **Ephemeral pod handling** | Use Prometheus `increase()` semantics; reset counter label set when a pod UID changes so stale series do not accumulate |

### Acceptance Criteria

- Per-pod energy totals sum to within ±5 % of the node-level RAPL total (excluding system overhead).
- Metrics are available in Prometheus within 10 seconds of a new pod starting.

---

## Tier 7 — Distributed Multi-Host Kubernetes (k8s)

**Priority 7 — Workloads spanning multiple heterogeneous physical nodes**

### Goal

Scale EMT to production Kubernetes clusters where a single logical job (e.g. a distributed training run, a microservice application) may span dozens of nodes with heterogeneous hardware (Intel, AMD, NVIDIA, ARM).

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Kubernetes Cluster                                             │
│                                                                 │
│  Node A (Intel + NVIDIA)   Node B (AMD CPU)   Node C (ARM)     │
│  ┌──────────────────────┐  ┌───────────────┐  ┌─────────────┐  │
│  │ EMT DaemonSet        │  │ EMT DaemonSet │  │ EMT DaemonSet│  │
│  │ Rapl + NvidiaGpu     │  │ AMDEnergy     │  │ CPUEstimator│  │
│  │ (Rust binary)        │  │ (Rust binary) │  │ (Rust binary│  │
│  └──────────┬───────────┘  └───────┬───────┘  └──────┬──────┘  │
│             │                      │                 │         │
│             └──────────────────────┴─────────────────┘         │
│                                    │                           │
│                         ┌──────────▼──────────┐               │
│                         │  Prometheus          │               │
│                         │  (federation/remote  │               │
│                         │   write to Thanos)   │               │
│                         └──────────┬──────────┘               │
│                                    │                           │
│                         ┌──────────▼──────────┐               │
│                         │  Grafana dashboards  │               │
│                         │  + Job-level PromQL  │               │
│                         └──────────────────────┘               │
└─────────────────────────────────────────────────────────────────┘
```

### Feature Plan

| Feature | Description |
|---|---|
| **`AMDEnergy` collector** | New Rust `EnergyCollector` impl wrapping the `amd_energy` kernel driver (`/sys/bus/platform/devices/amd_energy.*/`) for AMD EPYC nodes |
| **ARM/Graviton estimation** | `CPUEstimator` extended with ARM TDP profiles (AWS Graviton 2/3, Ampere Altra) |
| **Job-level aggregation** | PromQL recording rules that sum `emt_pod_energy_joules_total` across all pods sharing a `batch.kubernetes.io/job-name` label |
| **Network energy estimation** | Attribute energy cost of network transfers using byte-count × energy-per-byte coefficients (based on published datacenter network energy studies) |
| **Multi-cluster federation** | Support Prometheus federation / Thanos remote-write for clusters that span multiple Kubernetes API servers |
| **Grafana dashboard bundle** | Ship a `dashboards/` folder with pre-built Grafana JSON dashboards for job-level, namespace-level, and node-level energy views |
| **Energy cost annotations** | Optional integration with electricity price APIs (Electricity Maps, WattTime) to convert Joules → CO₂ and currency cost |

### Acceptance Criteria

- Job-level energy aggregation across 10 nodes produces results within ±5 % of the sum of individual node attributions.
- DaemonSet image auto-selects the correct `EnergyCollector` on each node type without manual configuration.

---

## Summary: Priority vs. Effort vs. Coverage

| Tier | Priority | Effort | Compute Context Covered |
|---|---|---|---|
| **0** | ★★★★★ | — | Current: Python collector; In progress: Rust collector + PyO3 bridge |
| **1** | ★★★★★ | Low | Physical host, full process tree |
| **2** | ★★★★★ | Medium | Container on host with RAPL |
| **3** | ★★★★☆ | Medium | Container / VPS, no RAPL (model) |
| **4** | ★★★★☆ | High | Hypervisor guest (with/without RAPL) |
| **5** | ★★★☆☆ | Medium | Container inside VM |
| **6** | ★★★☆☆ | Medium | Single-node Kubernetes (k3s) |
| **7** | ★★☆☆☆ | High | Multi-host Kubernetes cluster |

---

## Cross-Cutting Concerns

The following features are needed by multiple tiers and should be implemented as shared infrastructure:

| Feature | Required By | Notes |
|---|---|---|
| **Rust → Python PyO3 bridge** | All tiers | Exposes `EnergyGroup<T>` to Python as `emt._rust`; replaces Python PowerGroups without changing the public API |
| **`AttributionChain` pipeline** | Tiers 5–7 | Compose attribution stages; each stage scales the energy budget from the stage above; implement in Rust for performance |
| **Hypervisor / container layer auto-detection** | Tiers 3–5 | Read CPUID, DMI, `/run/containerenv`, `/proc/1/cgroup` at startup — portable in Rust |
| **Prometheus exporter** | Tiers 6–7 | Reuse existing metric schema; add `source` label (`measured` / `estimated`) |
| **`AMDEnergy` collector** | Tiers 1, 7 | AMD EPYC is common in both HPC and Kubernetes clusters; implement as `EnergyCollector` trait in Rust |
| **Trace `source` metadata** | Tiers 3–5 | Distinguish measured from estimated values in every output row |
| **Helm chart** | Tiers 6–7 | Reduce deployment friction; configurable resource limits, RBAC, tolerations; use Rust binary image for small footprint |

---

*For the technical challenges that motivate this roadmap, see [Virtualization Challenges](virtualization_challenges.md). For the current Python and planned Rust flow in detail, see [How EMT Works](how_EMT_works.md). For current deployment patterns and configuration examples, see [Virtualization Strategies](virtualization_strategies.md).*
