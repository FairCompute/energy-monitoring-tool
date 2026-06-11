# Getting Started

## Mode 1: Per-Process Energy Estimation

- Combines hardware measurements with behavioral models
- Uses eBPF for kernel-level data collection
- Leverages RAPL and other hardware counters
- Focuses on granularity and accuracy

Unlike tools such as EnergyMeter, which are limited to bare-metal environments, EMT is engineered to provide accurate per-process energy estimation even in virtualized and containerized settings.

## Mode 2: Prometheus-based Telemetry

- Exposes energy metrics via HTTP endpoint in Prometheus format
- Follows best practices for metric naming and labeling
- Designed for integration with Grafana and other observability tools

While Kepler and Scaphandre also export Prometheus metrics, EMT emphasizes low-cardinality labeling and robust integration for both container and VM monitoring, reducing the risk of performance bottlenecks in large-scale deployments.

Before demos or exporter changes on a Linux host with readable RAPL
`/sys/class/powercap` counters, run the local power-cadence probe against a
fresh release binary:

```bash
cargo build --release
.venv/bin/python scripts/probe_prometheus_power_cadence.py
```

The probe starts a short CPU workload, runs the headless Prometheus exporter
against that workload PID at the demo cadence, samples `/metrics` faster than the
collection rate, and fails if either system or workload CPU energy increases
while `emt_power_watts` falls back to zero. If the probe fails before energy
increases, check host permissions and hardware counter availability before
treating it as a cadence regression.

*See [Virtualization Challenges](virtualization_challenges.md) for more details on technical hurdles.*
