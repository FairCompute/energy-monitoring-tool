# Strategies for Reliable Virtualization Support

## Advanced Data Collection

- Uses eBPF for kernel-level insights
- Correlates host power data with guest/container activity

Unlike Scaphandre, which may encounter issues with container labeling in Kubernetes, EMT is actively developed to provide robust, accurate labeling and correlation for both containers and VMs.

## Attribution Models

- Employs advanced models (potentially ML-driven)
- Distinguishes idle vs. dynamic power

EMT's attribution models are designed to overcome the limitations seen in tools like Kepler, where accuracy discrepancies and high-cardinality metrics can impact monitoring quality.

## Orchestration Integration

- Prometheus-based telemetry for Kubernetes and VM managers
- Enables energy-aware scheduling and resource allocation

## cgroup-based vCPU Attribution

- Correlates cgroup metrics with RAPL data
- Enriches metrics with cgroup-specific labels

## Roadmap

- Ongoing model refinement
- Broader hardware support
- Deeper orchestration integration
- Validation and benchmarking
- User-friendly dashboards

---

*See [Conclusion](conclusion.md) for recommendations and next steps.*
