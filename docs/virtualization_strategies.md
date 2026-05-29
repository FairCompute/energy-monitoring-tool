# Scaling EMT: Strategies for Virtualized and Distributed Environments

This page describes how to deploy and configure the Energy Monitoring Tool (EMT) across four common infrastructure scenarios where direct hardware access is restricted or where workloads are distributed across multiple physical nodes.

---

## 1. Virtualization: Virtual Private Servers (VPS)

### Context

A VPS is a guest VM running on a shared cloud host. The guest OS has no direct access to RAPL MSRs or NVML because the hypervisor withholds those hardware interfaces. Energy must therefore be **estimated** rather than measured directly.

### Strategy: Utilisation-Based Energy Modelling

When RAPL is unavailable, EMT falls back to a CPU utilisation model. The approach combines:

1. **CPU utilisation sampling** via `psutil` (available in any Linux guest).
2. **CPU TDP (Thermal Design Power) database** — the Teads Cloud Carbon Coefficients dataset provides minimum, average, and maximum TDP values for thousands of CPU models.
3. **Linear interpolation**: `estimated_power ≈ TDP_idle + (TDP_max - TDP_idle) × cpu_utilisation`

The resulting energy estimate has an accuracy of ±15–25 % compared to ground-truth RAPL readings on bare metal, which is acceptable for workload-level carbon accounting.

### Deployment Requirements

| Requirement | Detail |
|---|---|
| Linux kernel | Any (no special drivers needed) |
| RAPL | Not required |
| `psutil` | Must be installed (included in EMT's dependencies) |
| CPU TDP DB | Bundled with EMT (`emt/data/teads_cpu_tdp.csv`) |

### Configuration

```python
from emt import EnergyMonitor

# EMT automatically detects that RAPL is unavailable and
# activates the utilisation-based estimator.
with EnergyMonitor(name="my_task") as monitor:
    run_my_workload()

print(f"Estimated energy: {monitor.consumed_energy}")
```

No special configuration flags are required. EMT's `EnergyMonitor.__enter__` discovers available `PowerGroup` implementations at runtime. When neither `RAPLSoC` nor `NvidiaGPU` initialise successfully, a `CPUEstimator` power group is activated automatically.

### Limitations and Accuracy Notes

- Shared physical CPUs mean that TDP is shared across tenants. EMT's estimate reflects the *virtual workload's share* of the modelled physical TDP, but cannot account for thermal throttling caused by other tenants.
- Memory DRAM energy is not estimated in VPS mode.
- GPU energy estimation is not supported unless the VPS has a dedicated GPU passthrough.

---

## 2. Hypervisors and Guest OS

### Context

In on-premises environments running KVM, VMware ESXi, or Hyper-V, energy monitoring can be approached from two directions: **host-side** (privileged, full visibility) or **guest-side** (restricted, estimation-based). Combining both yields the most accurate results.

### Strategy A: Host-Side Monitoring with Per-VM Attribution

Run EMT (or its Rust CLI) directly on the hypervisor host. The host has unrestricted RAPL access. Energy attribution to individual VMs is achieved by tracking the CPU time consumed by each VM's QEMU/KVM process:

```
VM energy ≈ total_socket_energy × (vm_vcpu_cpu_time / total_host_cpu_time)
```

**Deployment steps (KVM example):**

1. Install EMT on the hypervisor host:
   ```bash
   pip install emt
   emt_cfgup   # grants powercap group membership for RAPL access
   ```
2. Identify QEMU process IDs for each VM:
   ```bash
   ps aux | grep qemu
   ```
3. Start the Rust CLI with the QEMU PID to attribute energy to that VM:
   ```bash
   emt --pid <qemu_pid> --json-out vm-energy.json --duration 60
   ```
4. The CLI outputs per-socket RAPL energy weighted by the QEMU process's CPU utilisation share.

### Strategy B: Guest-Side Monitoring with RAPL MSR Passthrough

Some KVM configurations expose RAPL MSRs to the guest via the `msr-safe` kernel module. This allows the guest to read raw RAPL counters, but the values represent **total physical-package energy** (shared by all VMs on that host), so guest-side attribution requires an additional normalisation step.

**Host setup (KVM):**

```bash
# Load msr-safe with an allowlist that permits RAPL MSR reads
modprobe msr-safe
echo "0x00000606 0xFFFFFFFFFFFFFFFF" >> /etc/msr-safe/whitelist  # IA32_RAPL_POWER_UNIT
echo "0x00000611 0xFFFFFFFFFFFFFFFF" >> /etc/msr-safe/whitelist  # MSR_PKG_ENERGY_STATUS
```

**Guest usage:**

```python
from emt import EnergyMonitor
# Guest reads package RAPL energy; values are the full physical socket
# and must be scaled by the guest's vCPU share.
with EnergyMonitor(name="guest_task") as monitor:
    run_workload()
```

!!! warning "Shared RAPL Readings"
    RAPL readings inside a guest reflect the **entire physical socket**, not just the guest's workload. Without normalisation by vCPU allocation ratio, energy will be **overestimated**. Always apply the formula:
    `guest_energy = rapl_energy × (guest_vcpu_count / host_total_vcpu_count)`

### Strategy C: Dual-Layer Correlation (Recommended)

The most accurate approach correlates host-side RAPL readings with guest-side CPU utilisation:

1. **Host agent**: Reads per-socket RAPL energy every second; records timestamp and total CPU utilisation.
2. **Guest agent**: Records process-level CPU utilisation every second; forwards metrics to a shared time-series store (Prometheus/InfluxDB).
3. **Attribution engine**: Correlates timestamps and computes:
   ```
   process_energy = socket_energy × (process_cpu_time / socket_cpu_time)
   ```

This mirrors the approach taken by Kepler's host/guest dual deployment but without requiring a privileged DaemonSet inside the guest.

### Live Migration Handling

When a VM is live-migrated, the RAPL energy counter resets on the destination host. EMT's `RotatingTrace` mechanism detects discontinuities in the energy time series and inserts a breakpoint, preventing negative or wildly inflated energy deltas from corrupting the measurement.

---

## 3. Distributed Computing: Kubernetes Pods Across Physical Hosts

### Context

A Kubernetes cluster runs workloads as pods, potentially spread across dozens of heterogeneous physical nodes. Each node has its own RAPL-capable CPUs and optional GPUs. Accurate job-level energy attribution requires aggregating per-node measurements and correlating them with Kubernetes pod metadata.

### Recommended Architecture: DaemonSet + Prometheus

```
┌────────────────────────────────────────────────────────────────┐
│  Kubernetes Cluster                                            │
│                                                                │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐        │
│  │  Node A      │  │  Node B      │  │  Node C      │        │
│  │  EMT agent   │  │  EMT agent   │  │  EMT agent   │        │
│  │  (DaemonSet) │  │  (DaemonSet) │  │  (DaemonSet) │        │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘        │
│         │                 │                 │                 │
│         └────────────┬────┘                 │                 │
│                      │◄────────────────────┘                  │
│              ┌───────▼────────┐                               │
│              │  Prometheus    │                               │
│              │  (scrapes all  │                               │
│              │   node agents) │                               │
│              └───────┬────────┘                               │
│                      │                                         │
│              ┌───────▼────────┐                               │
│              │  Grafana       │                               │
│              │  (dashboards)  │                               │
│              └────────────────┘                               │
└────────────────────────────────────────────────────────────────┘
```

### Step 1: Deploy EMT as a DaemonSet

Create a Kubernetes DaemonSet that runs EMT's Prometheus exporter on every node. The DaemonSet pod must be **privileged** or granted the `SYS_RAWIO` capability to read RAPL.

```yaml
# emt-daemonset.yaml
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: emt-agent
  namespace: monitoring
spec:
  selector:
    matchLabels:
      app: emt-agent
  template:
    metadata:
      labels:
        app: emt-agent
    spec:
      hostPID: true          # Required to observe host process IDs
      hostNetwork: true      # Required for Prometheus scraping on host IP
      tolerations:
        - operator: Exists   # Run on all nodes including control-plane
      containers:
        - name: emt
          image: faircompute/emt:latest
          command: ["emt-exporter", "--port", "9101"]
          securityContext:
            privileged: true  # Required for RAPL MSR access
          volumeMounts:
            - name: powercap
              mountPath: /sys/class/powercap
              readOnly: true
            - name: proc
              mountPath: /proc
              readOnly: true
      volumes:
        - name: powercap
          hostPath:
            path: /sys/class/powercap
        - name: proc
          hostPath:
            path: /proc
```

### Step 2: Annotate Pods with Energy Labels

Add standard labels to workload pods so that energy metrics can be filtered by namespace, job name, or team:

```yaml
metadata:
  labels:
    app.kubernetes.io/name: my-training-job
    app.kubernetes.io/component: trainer
    team: ml-platform
```

The EMT node agent reads `/proc/<pid>/cgroup` for each container process, resolves the Kubernetes pod UID via the cgroup path, and then adds the pod's labels as Prometheus metric labels.

### Step 3: Aggregate Job Energy in Prometheus

Use PromQL to sum energy consumption across all nodes for a given job:

```promql
# Total energy consumed by all pods in the ml-training namespace (Joules)
sum by (namespace, pod) (
  emt_process_energy_joules_total{namespace="ml-training"}
)

# Energy rate (Watts) for a specific app
rate(emt_process_energy_joules_total{app_kubernetes_io_name="my-training-job"}[5m])
```

### Step 4: Handle Pod Ephemerality

EMT uses a **sliding window trace** (see [Trace Rotation](trace_rotation.md)) that automatically discards measurements older than a configurable retention window. When a pod is rescheduled or its PID changes:

1. The old PID's measurements are naturally expired from the trace.
2. The new PID is discovered at the next collection interval and begins accumulating a fresh energy counter.
3. The Prometheus `emt_process_energy_joules_total` counter resets for the new pod instance; downstream aggregation should use `increase()` or `rate()` rather than raw counter values.

### Heterogeneous Hardware Considerations

| Node type | RAPL availability | GPU availability | Recommended PowerGroup |
|---|---|---|---|
| Intel CPU, no GPU | ✅ intel-rapl | ❌ | `RAPLSoC` |
| AMD CPU, no GPU | ✅ amd_energy driver | ❌ | Future `AMDEnergy` collector |
| Intel CPU + NVIDIA GPU | ✅ intel-rapl | ✅ nvml | `RAPLSoC` + `NvidiaGPU` |
| ARM / Graviton | ❌ RAPL | ❌ | `CPUEstimator` (model-based) |

EMT's `PowerGroup` discovery mechanism automatically selects the available groups at startup, so the same DaemonSet image can be deployed on heterogeneous clusters.

---

## 4. Slurm Cluster Nodes: cgroups-Limited Visibility

### Context

HPC clusters managed by Slurm assign jobs to compute nodes and place those jobs inside cgroup subtrees for resource isolation. Standard users cannot read RAPL MSR files or `/sys/class/powercap/` entries without elevated privileges. Accurate per-job energy attribution requires either:

- A **privileged system daemon** running outside the job cgroup, or
- **Kernel capability delegation** to the Slurm job via `msr-safe`.

### Strategy A: Privileged EMT Daemon via Slurm Prolog/Epilog

Run EMT as a **root-privileged system service** on each compute node. Use Slurm's prolog and epilog scripts to register and deregister each job so that the daemon can attribute energy to the correct job IDs.

**System daemon setup (`/etc/systemd/system/emt.service`):**

```ini
[Unit]
Description=EMT Energy Monitoring Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/emt-daemon --config /etc/emt/config.json
Restart=always
User=root

[Install]
WantedBy=multi-user.target
```

**Slurm Prolog (`/etc/slurm/prolog.d/50-emt.sh`):**

```bash
#!/bin/bash
# Notify the EMT daemon that a new job is starting
curl -s -X POST http://localhost:9102/jobs \
  -H "Content-Type: application/json" \
  -d "{\"job_id\": \"${SLURM_JOB_ID}\", \"cgroup\": \"/sys/fs/cgroup/system.slice/slurmstepd.scope/job_${SLURM_JOB_ID}\"}"
```

**Slurm Epilog (`/etc/slurm/epilog.d/50-emt.sh`):**

```bash
#!/bin/bash
# Retrieve final energy reading and write to job stats directory
curl -s "http://localhost:9102/jobs/${SLURM_JOB_ID}/energy" \
  > "/var/log/slurm/energy/${SLURM_JOB_ID}.json"

# Deregister the job from the EMT daemon
curl -s -X DELETE "http://localhost:9102/jobs/${SLURM_JOB_ID}"
```

The daemon tracks per-job RAPL energy by:

1. Reading total-socket RAPL energy at a fixed rate (e.g. 10 Hz).
2. Reading CPU utilisation per cgroup subtree via `/sys/fs/cgroup/<job_cgroup>/cpu.stat`.
3. Computing each job's energy share:
   ```
   job_energy = socket_energy × (job_cpu_usage / total_cpu_usage)
   ```

### Strategy B: `msr-safe` Kernel Module for User-Space RAPL Access

In environments where per-job RAPL delegation is preferred, install the `msr-safe` kernel module and configure a whitelist of RAPL-related MSR addresses that Slurm job users are permitted to read:

```bash
# Install msr-safe (requires kernel headers)
git clone https://github.com/LLNL/msr-safe.git
cd msr-safe && make && sudo make install
sudo modprobe msr-safe

# Allow RAPL MSRs for all users
sudo bash -c 'cat >> /etc/msr-safe/whitelist <<EOF
0x00000606 0xFFFFFFFFFFFFFFFF  # IA32_RAPL_POWER_UNIT
0x00000611 0xFFFFFFFFFFFFFFFF  # MSR_PKG_ENERGY_STATUS
0x00000639 0xFFFFFFFFFFFFFFFF  # MSR_DRAM_ENERGY_STATUS
0x00000641 0xFFFFFFFFFFFFFFFF  # MSR_PP0_ENERGY_STATUS
EOF'

# Add the powercap group so emt_cfgup succeeds inside the job
sudo emt_cfgup
```

With `msr-safe` in place, EMT can be installed and used directly inside a Slurm job without root access:

```bash
# Inside a Slurm batch script
#SBATCH --job-name=my_ml_job
#SBATCH --ntasks=4
#SBATCH --cpus-per-task=8

module load python/3.11 emt
python -c "
from emt import EnergyMonitor
with EnergyMonitor(name='slurm_job_${SLURM_JOB_ID}') as m:
    import subprocess
    subprocess.run(['python', 'train.py'])
print(m.consumed_energy)
" > energy_report_${SLURM_JOB_ID}.json
```

### Shared-Node Attribution

When Slurm packs multiple jobs onto the same node, EMT uses **proportional energy attribution** based on CPU time shares — the same algorithm used in bare-metal multi-process scenarios:

```
job_energy ≈ socket_energy × (job_cpu_seconds / socket_total_cpu_seconds)
```

This is collected per-socket over the measurement window using EMT's `DeltaReader` mechanism, which reads RAPL energy deltas at a configurable interval.

### AMD EPYC Nodes

AMD EPYC processors expose per-core energy through the `amd_energy` kernel driver (kernel ≥ 5.8) rather than the `intel-rapl` powercap interface. On clusters with AMD nodes, verify the driver is loaded:

```bash
lsmod | grep amd_energy
# If not loaded:
sudo modprobe amd_energy
```

The `amd_energy` sysfs interface is located at `/sys/bus/platform/devices/amd_energy.*/energy*_input` and provides per-core and per-socket energy in micro-joules. A future EMT `AMDEnergy` PowerGroup will wrap this interface with the same API as `RAPLSoC`.

### Recommended Configuration for HPC Sites

| Configuration | Value |
|---|---|
| Collection rate | 1 Hz (reduce overhead on large nodes) |
| Trace retention | Job wall time + 5 minutes |
| Output format | JSON (compatible with Slurm accounting database) |
| Integration | `sacct --format=JobID,Energy` (requires Slurm energy plugin) |

---

## Summary Matrix

| Scenario | RAPL Available | GPU Metrics | Attribution Method | Accuracy |
|---|---|---|---|---|
| Bare metal | ✅ Direct | ✅ NVML | Process CPU/GPU utilisation | High (±5%) |
| VPS / Cloud VM | ❌ Blocked | ❌ (usually) | TDP model × CPU utilisation | Medium (±20%) |
| KVM Guest (MSR passthrough) | ⚠️ Shared socket | Conditional | Socket RAPL × vCPU share | Medium-High (±10%) |
| KVM Host (per-VM) | ✅ Host RAPL | ✅ If host GPU | QEMU process CPU share | High (±5%) |
| Kubernetes DaemonSet | ✅ Host RAPL | ✅ If GPU node | cgroup → pod metadata | High (±5%) |
| Slurm (privileged daemon) | ✅ Via daemon | ✅ If GPU node | cgroup CPU share | High (±5%) |
| Slurm (msr-safe) | ✅ User-space | ✅ If GPU node | Process CPU utilisation | High (±5%) |

---

*See [Virtualization Challenges](virtualization_challenges.md) for a description of the specific technical hurdles that motivate these strategies.*
