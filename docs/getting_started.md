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

*See [Virtualization Challenges](virtualization_challenges.md) for more details on technical hurdles.*
