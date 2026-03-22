# Challenges of Energy Monitoring in Virtualized Environments

Modern compute infrastructure rarely runs directly on bare-metal hardware. Energy monitoring tools must navigate a complex hierarchy of abstraction layers — from containers and virtual machines to distributed schedulers and cloud-hosted VPS instances — each introducing unique challenges for accurate energy attribution.

---

## 1. Containers and cgroups-based vCPUs

- **Ephemerality:** Containers are short-lived and dynamic, making real-time tracking difficult.
- **Shared Resources:** Containers share CPU, memory, and I/O with other containers on the same host, complicating energy attribution.
- **cgroups Accounting vs. Measurement:** Linux cgroups expose CPU and memory accounting data, but they do not provide direct energy counters for individual container workloads.
- **No Direct Hardware Access:** Containerised processes cannot access hardware interfaces like RAPL or NVML unless the container is granted elevated privileges.

Unlike PowerTOP or Intel Power Gadget, which focus on system-level or hardware-specific monitoring, EMT is designed to attribute energy at the container and cgroup level, even in highly dynamic environments.

---

## 2. Virtual Private Servers (VPS) and Virtualization

Running on a Virtual Private Server (VPS) introduces an additional layer of abstraction between the workload and the physical hardware:

- **No RAPL Access:** The guest OS on a VPS cannot read RAPL MSRs (Model-Specific Registers) because the hypervisor does not expose them. All CPU energy counter access is blocked.
- **No NVML Access:** NVIDIA GPU metrics via NVML are unavailable unless the VPS is provisioned with a GPU passthrough configuration (rare in most cloud offerings).
- **Shared Physical Resources:** The physical CPU and memory are shared across multiple VPS tenants, making it impossible to obtain a ground-truth measurement of the exact power consumed by one tenant's workload without host-level visibility.
- **Noisy Neighbour Effects:** Co-resident tenants consume power that influences thermal state and power delivery, introducing measurement noise even when a model-based approach is used.
- **Clock and Counter Drift:** vCPU time reported inside a VM can diverge from real wall-clock time, complicating utilisation-based energy estimations.

---

## 3. Hypervisors and Guest OS

In on-premises virtualisation environments (e.g. KVM, VMware ESXi, Hyper-V), a hypervisor mediates access between guest VMs and the physical hardware:

- **Hypervisor Overhead:** The hypervisor itself consumes CPU cycles and memory that are not visible to any single guest, creating an unattributed energy "dark pool".
- **Lack of Direct Hardware Access in the Guest:** Guests cannot read hardware energy counters (RAPL, ACPI HWMON) unless the hypervisor explicitly exposes them via virtual MSR passthrough or ACPI energy tables.
- **Partial RAPL Exposure:** Some hypervisors (e.g. KVM with the `msr` kernel module) can expose RAPL MSRs to a guest, but readings reflect physical-package energy shared across all VMs running on that host — not just the guest's own consumption.
- **Modeling vs. Measurement Discrepancies:** Reliance on CPU utilisation models (e.g. Teads TDP coefficients) to estimate energy inside a VM can diverge significantly from actual power draw, especially under bursty or memory-intensive workloads.
- **Live Migration:** Guests can be live-migrated between physical hosts mid-measurement, causing energy readings to jump discontinuously and attribution to break entirely.

Whereas Kepler and PowerAPI require deployment both on the host and inside the VM for full visibility, EMT aims to bridge the observability gap with a multi-layered approach, correlating host and guest data for more accurate attribution.

---

## 4. Distributed Computing: Kubernetes Pods Across Physical Hosts

Kubernetes orchestrates workloads across a fleet of nodes, introducing cross-host challenges:

- **Multi-Node Attribution:** A single logical job (e.g. a distributed training run) may span many physical nodes, each with different hardware, TDP envelopes, and energy costs. Energy must be aggregated across the entire job, not just a single node.
- **Pod Ephemerality and Rescheduling:** Pods can be evicted, rescheduled, or scaled horizontally at any time. A monitoring agent tied to a specific process ID or container ID will lose its measurement context.
- **Namespace and cgroup Isolation:** Kubernetes uses cgroups v1/v2 hierarchy for resource limits. Energy counters are not exposed natively per-namespace or per-pod; the monitoring agent must reconcile Kubernetes metadata (pod name, namespace, labels) with OS-level cgroup information.
- **Heterogeneous Hardware:** Different nodes in a cluster may have different CPUs (Intel vs. AMD) or different numbers of GPU models, making a uniform measurement approach difficult.
- **Network and Storage Energy:** CPU and GPU power capture the compute component, but distributed workloads also consume significant energy through network transfers and persistent storage I/O, which RAPL and NVML do not measure.
- **Control Plane Overhead:** The Kubernetes control plane (API server, etcd, scheduler, kubelet) consumes a non-trivial baseline power, which must be excluded or separately accounted when attributing energy to user workloads.

---

## 5. Slurm Cluster Nodes: cgroups-Limited Visibility

HPC clusters managed by Slurm submit jobs to dedicated compute nodes, but place those jobs inside cgroups that restrict what the job process can observe:

- **cgroups Visibility Limits:** When the `CgroupPlugin=cgroup/v2` and `TaskPlugin=task/cgroup` settings are active in Slurm, job processes run inside a dedicated cgroup subtree. RAPL MSR files (under `/sys/class/powercap/`) may not be readable from within the job's cgroup without additional kernel capabilities (`CAP_SYS_RAWIO`).
- **Shared Nodes and Backfill:** Slurm may pack multiple jobs onto the same node simultaneously. RAPL exposes total-package energy for the physical socket — there is no per-job RAPL partition. Any energy attribution must be weighted by the fraction of CPU resources consumed by each job.
- **Job Prologue/Epilogue Lifecycle:** Energy monitoring must align with the Slurm job lifecycle — starting at `PrologSlurmctld` time and ending at `EpilogSlurmctld` time — without relying on persistent daemons that survive across jobs.
- **Out-of-Band Access Restrictions:** In many HPC environments, security policy prevents unprivileged users from accessing `/dev/cpu/*/msr` directly. The `msr-safe` kernel module or `powercap` group membership (as configured by `emt_cfgup`) is required.
- **Node Heterogeneity:** HPC clusters often mix CPU generations and vendor architectures. AMD CPUs expose energy via the `amd_energy` driver rather than the `intel-rapl` powercap interface, requiring different collection paths.

---

*See [Virtualization Strategies](virtualization_strategies.md) for detailed deployment patterns and mitigation approaches for each of these scenarios.*
