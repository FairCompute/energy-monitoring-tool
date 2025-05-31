# Challenges of Energy Monitoring in Virtualized Environments

## Containers and cgroups-based vCPUs

- **Ephemerality:** Containers are short-lived and dynamic, making real-time tracking difficult.
- **Shared Resources:** Containers share CPU, memory, and I/O, complicating energy attribution.
- **cgroups:** Provide resource accounting, but not direct energy measurement for vCPUs.

Unlike PowerTOP or Intel Power Gadget, which focus on system-level or hardware-specific monitoring, EMT is designed to attribute energy at the container and cgroup level, even in highly dynamic environments.

## Virtual Machines (VMs) and Hypervisors

- **Hypervisor Overhead:** Adds complexity to energy attribution.
- **Lack of Direct Hardware Access:** VMs can't access hardware counters like RAPL directly.
- **Modeling vs. Measurement:** Reliance on models can introduce discrepancies.

Whereas Kepler and PowerAPI require deployment both on the host and inside the VM for full visibility, EMT aims to bridge the observability gap with a multi-layered approach, correlating host and guest data for more accurate attribution.

---

*See [Virtualization Strategies](virtualization_strategies.md) for solutions and approaches.*
