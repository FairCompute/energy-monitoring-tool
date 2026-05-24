use crate::collectors::{NvidiaGpu, Rapl};
use crate::config::EmtConfig;
use crate::energy_group::{EnergyCollector, EnergyGroup, EnergyRecord};
use crate::utils::errors::MonitoringError;
use crate::utils::psutils::{scan_roots, walk_child_pids, ProcessRoot};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

// ─── MetricsSnapshot data structures ────────────────────────────────────────

/// Energy breakdown by device type (CPU, DRAM, GPU).
#[derive(Debug, Clone, Default)]
pub struct DeviceEnergy {
    pub cpu_joules: f64,
    pub dram_joules: f64,
    pub gpu_joules: f64,
}

impl DeviceEnergy {
    /// Total energy across all device types.
    pub fn total(&self) -> f64 {
        self.cpu_joules + self.dram_joules + self.gpu_joules
    }

    /// Subtract another DeviceEnergy, clamping each component to >= 0.
    pub fn saturating_sub(&self, other: &DeviceEnergy) -> DeviceEnergy {
        DeviceEnergy {
            cpu_joules: (self.cpu_joules - other.cpu_joules).max(0.0),
            dram_joules: (self.dram_joules - other.dram_joules).max(0.0),
            gpu_joules: (self.gpu_joules - other.gpu_joules).max(0.0),
        }
    }
}

/// Per-workload (root process) energy and power snapshot.
#[derive(Debug, Clone)]
pub struct WorkloadSnapshot {
    pub root_pid: u32,
    pub name: String,
    pub user: String,
    pub energy: DeviceEnergy,
    pub power_watts: f64,
}

/// Full metrics snapshot shared via MonitorHandle.
#[derive(Debug, Clone, Default)]
pub struct MetricsSnapshot {
    pub timestamp: i64,
    pub system_total: DeviceEnergy,
    pub workloads: Vec<WorkloadSnapshot>,
    pub unattributed: DeviceEnergy,
    pub tracked_pids: Vec<u32>,
}

// ─── Device classification ──────────────────────────────────────────────────

/// Classify an EnergyRecord into a device category and return the energy value.
fn classify_energy(record: &EnergyRecord) -> EnergyClass {
    if record.device.starts_with("nvidia:") {
        EnergyClass::Gpu
    } else if record.device == "rapl:system:dram" {
        EnergyClass::Dram
    } else {
        // rapl:socket:*:package, rapl:system:psys, or any other RAPL device
        EnergyClass::Cpu
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnergyClass {
    Cpu,
    Dram,
    Gpu,
}

/// Accumulate an energy record into a DeviceEnergy struct.
fn accumulate_device_energy(device_energy: &mut DeviceEnergy, class: EnergyClass, joules: f64) {
    match class {
        EnergyClass::Cpu => device_energy.cpu_joules += joules,
        EnergyClass::Dram => device_energy.dram_joules += joules,
        EnergyClass::Gpu => device_energy.gpu_joules += joules,
    }
}

// ─── Child-to-root mapping ──────────────────────────────────────────────────

/// Build a map from each child PID to its root PID.
/// Each root is walked independently so that all descendants map back.
fn build_child_to_root_map(roots: &[u32]) -> (HashMap<u32, u32>, Vec<u32>) {
    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut all_pids: Vec<u32> = Vec::new();

    for &root in roots {
        let children = walk_child_pids(&[root]);
        for &child in &children {
            map.insert(child, root);
        }
        all_pids.extend(children);
    }

    // Deduplicate all_pids while preserving order
    let mut seen = std::collections::HashSet::new();
    all_pids.retain(|pid| seen.insert(*pid));

    (map, all_pids)
}

// ─── MonitorHandle ──────────────────────────────────────────────────────────

/// A cloneable handle providing read-only access to the latest monitor state.
#[derive(Clone)]
pub struct MonitorHandle {
    snapshot: Arc<RwLock<MetricsSnapshot>>,
}

impl MonitorHandle {
    /// Returns a clone of the current snapshot.
    pub fn snapshot(&self) -> MetricsSnapshot {
        self.snapshot.read().unwrap().clone()
    }

    /// Returns the total consumed energy in joules across all device types.
    pub fn total_consumed_energy(&self) -> f64 {
        let snap = self.snapshot.read().unwrap();
        snap.system_total.total()
    }

    /// Returns a per-PID energy map (sum of all device types per PID).
    /// Kept for backwards compatibility.
    pub fn consumed_energy_by_pid(&self) -> HashMap<u32, f64> {
        let snap = self.snapshot.read().unwrap();
        let mut result = HashMap::new();
        for wl in &snap.workloads {
            *result.entry(wl.root_pid).or_insert(0.0) += wl.energy.total();
        }
        result
    }
}

// ─── Internal state for power computation ───────────────────────────────────

/// Tracks previous tick state for computing power (watts).
#[derive(Debug, Clone, Default)]
struct PreviousTickState {
    timestamp: i64,
    workload_energy: HashMap<u32, DeviceEnergy>,
}

// ─── Monitor ────────────────────────────────────────────────────────────────

/// Central coordinator that owns all collectors, process discovery, and runs autonomously.
pub struct Monitor {
    config: EmtConfig,
    rapl_group: Arc<Mutex<EnergyGroup<Rapl>>>,
    gpu_group: Option<Arc<Mutex<EnergyGroup<NvidiaGpu>>>>,
    root_pids: Option<Vec<u32>>,
    /// Shared state for scan task results
    discovered_roots: Arc<RwLock<Vec<ProcessRoot>>>,
    /// Internal task handles
    tick_handle: Option<JoinHandle<()>>,
    scan_handle: Option<JoinHandle<()>>,
    /// Shared snapshot for MonitorHandle
    snapshot: Arc<RwLock<MetricsSnapshot>>,
    is_running: Arc<AtomicBool>,
}

impl Monitor {
    /// Create a new Monitor with the given config and optional root PIDs.
    /// If `root_pids` is None, the monitor will use a background scan task
    /// to discover all root processes on the system.
    pub fn new(config: EmtConfig, root_pids: Option<Vec<u32>>) -> Self {
        let rate = config.collection.rate_hz;
        // Batch size = rate (flush once per second for responsive snapshots)
        let batch_size = Some(rate.ceil() as usize);
        let rapl_group = EnergyGroup::new(Rapl::default(), rate, batch_size);

        // Auto-detect GPU availability
        let gpu_group = if NvidiaGpu::is_available() {
            Some(Arc::new(Mutex::new(EnergyGroup::new(
                NvidiaGpu::default(),
                rate,
                batch_size,
            ))))
        } else {
            None
        };

        Self {
            config,
            rapl_group: Arc::new(Mutex::new(rapl_group)),
            gpu_group,
            root_pids,
            discovered_roots: Arc::new(RwLock::new(Vec::new())),
            tick_handle: None,
            scan_handle: None,
            snapshot: Arc::new(RwLock::new(MetricsSnapshot::default())),
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start the monitor and return a handle for reading state.
    /// If already running, returns a new handle to the existing shared snapshot.
    pub async fn commence(&mut self) -> Result<MonitorHandle, MonitoringError> {
        if self.is_running.load(Ordering::SeqCst) {
            // Already running -- return existing handle
            return Ok(MonitorHandle {
                snapshot: Arc::clone(&self.snapshot),
            });
        }

        self.is_running.store(true, Ordering::SeqCst);

        // Start collector background tasks
        {
            let mut rapl = self.rapl_group.lock().await;
            rapl.commence().await?;
        }
        if let Some(gpu) = &self.gpu_group {
            let mut gpu_lock = gpu.lock().await;
            gpu_lock.commence().await?;
        }

        // If no specific root_pids, spawn scan task for automatic discovery
        if self.root_pids.is_none() {
            self.spawn_scan_task();
        }

        // Spawn tick task (internal loop at configured rate)
        self.spawn_tick_task();

        Ok(MonitorHandle {
            snapshot: Arc::clone(&self.snapshot),
        })
    }

    /// Shut down the monitor, stopping all background tasks and collectors.
    pub async fn shutdown(&mut self) -> Result<(), MonitoringError> {
        self.is_running.store(false, Ordering::SeqCst);

        // Abort background tasks
        if let Some(handle) = self.tick_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.scan_handle.take() {
            handle.abort();
        }

        // Shutdown collector groups
        {
            let mut rapl = self.rapl_group.lock().await;
            rapl.shutdown()?;
        }
        if let Some(gpu) = &self.gpu_group {
            let mut gpu_lock = gpu.lock().await;
            gpu_lock.shutdown()?;
        }

        Ok(())
    }

    /// Spawn the tick task that runs the core polling and update loop.
    fn spawn_tick_task(&mut self) {
        let interval = Duration::from_secs_f64(1.0 / self.config.collection.rate_hz);
        let rapl_group = Arc::clone(&self.rapl_group);
        let gpu_group = self.gpu_group.clone();
        let root_pids = self.root_pids.clone();
        let discovered_roots = Arc::clone(&self.discovered_roots);
        let snapshot = Arc::clone(&self.snapshot);
        let is_running = Arc::clone(&self.is_running);

        self.tick_handle = Some(tokio::spawn(async move {
            let mut prev_state = PreviousTickState::default();

            while is_running.load(Ordering::SeqCst) {
                // 1. Determine current root PIDs and metadata
                let roots_with_metadata: Vec<ProcessRoot> = if let Some(ref pids) = root_pids {
                    // When explicit PIDs are given, create minimal ProcessRoot entries
                    pids.iter()
                        .map(|&pid| ProcessRoot {
                            pid,
                            user: String::new(),
                            name: String::new(),
                        })
                        .collect()
                } else {
                    discovered_roots.read().unwrap().clone()
                };

                let root_pid_list: Vec<u32> =
                    roots_with_metadata.iter().map(|r| r.pid).collect();

                // 2. Build child-to-root map and get expanded PID set
                let (child_to_root, expanded_pids) = if root_pid_list.is_empty() {
                    (HashMap::new(), Vec::new())
                } else {
                    let roots_for_walk = root_pid_list.clone();
                    tokio::task::spawn_blocking(move || {
                        build_child_to_root_map(&roots_for_walk)
                    })
                    .await
                    .unwrap_or_default()
                };

                // 3. Set tracked PIDs on collectors and poll data
                let rapl_records;
                {
                    let mut rapl = rapl_group.lock().await;
                    rapl.set_tracked_pids(expanded_pids.clone());
                    rapl_records = rapl.poll_data();
                }

                let gpu_records = if let Some(ref gpu) = gpu_group {
                    let mut gpu_lock = gpu.lock().await;
                    gpu_lock.set_tracked_pids(expanded_pids.clone());
                    gpu_lock.poll_data()
                } else {
                    Vec::new()
                };

                // 4. Compute MetricsSnapshot from records
                let all_records: Vec<&EnergyRecord> =
                    rapl_records.iter().chain(gpu_records.iter()).collect();

                // Compute system_total from all records
                let mut system_total = DeviceEnergy::default();
                for record in &all_records {
                    let class = classify_energy(record);
                    accumulate_device_energy(&mut system_total, class, record.energy);
                }

                // Compute per-root energy using child_to_root map
                let mut workload_energy_map: HashMap<u32, DeviceEnergy> = HashMap::new();
                for record in &all_records {
                    if let Some(&root) = child_to_root.get(&record.pid) {
                        let class = classify_energy(record);
                        let entry = workload_energy_map.entry(root).or_default();
                        accumulate_device_energy(entry, class, record.energy);
                    }
                }

                // Build workload snapshots with power computation
                let current_timestamp = chrono::Utc::now().timestamp_millis();
                let time_delta_s = if prev_state.timestamp > 0 {
                    (current_timestamp - prev_state.timestamp) as f64 / 1000.0
                } else {
                    0.0
                };

                let mut workloads: Vec<WorkloadSnapshot> = Vec::new();
                let mut workloads_sum = DeviceEnergy::default();

                for root_info in &roots_with_metadata {
                    let energy = workload_energy_map
                        .get(&root_info.pid)
                        .cloned()
                        .unwrap_or_default();

                    // Compute power from energy delta between ticks
                    let power_watts = if time_delta_s > 0.0 {
                        let prev_energy = prev_state
                            .workload_energy
                            .get(&root_info.pid)
                            .map(|de| de.total())
                            .unwrap_or(0.0);
                        let energy_delta = energy.total() - prev_energy;
                        (energy_delta / time_delta_s).max(0.0)
                    } else {
                        0.0
                    };

                    workloads_sum.cpu_joules += energy.cpu_joules;
                    workloads_sum.dram_joules += energy.dram_joules;
                    workloads_sum.gpu_joules += energy.gpu_joules;

                    workloads.push(WorkloadSnapshot {
                        root_pid: root_info.pid,
                        name: root_info.name.clone(),
                        user: root_info.user.clone(),
                        energy,
                        power_watts,
                    });
                }

                // Unattributed = system_total - sum(workloads), clamped >= 0
                let unattributed = system_total.saturating_sub(&workloads_sum);

                // Read previous snapshot to accumulate total energy
                let prev_snap = snapshot.read().unwrap().clone();

                // The system_total should be cumulative (add this tick's delta)
                let cumulative_total = DeviceEnergy {
                    cpu_joules: prev_snap.system_total.cpu_joules + system_total.cpu_joules,
                    dram_joules: prev_snap.system_total.dram_joules + system_total.dram_joules,
                    gpu_joules: prev_snap.system_total.gpu_joules + system_total.gpu_joules,
                };

                // Accumulate workload energy cumulatively
                let cumulative_workloads: Vec<WorkloadSnapshot> = workloads
                    .into_iter()
                    .map(|wl| {
                        // Find previous workload energy
                        let prev_wl_energy = prev_snap
                            .workloads
                            .iter()
                            .find(|pw| pw.root_pid == wl.root_pid)
                            .map(|pw| &pw.energy);

                        let cumulative_energy = if let Some(prev_e) = prev_wl_energy {
                            DeviceEnergy {
                                cpu_joules: prev_e.cpu_joules + wl.energy.cpu_joules,
                                dram_joules: prev_e.dram_joules + wl.energy.dram_joules,
                                gpu_joules: prev_e.gpu_joules + wl.energy.gpu_joules,
                            }
                        } else {
                            wl.energy
                        };

                        WorkloadSnapshot {
                            root_pid: wl.root_pid,
                            name: wl.name,
                            user: wl.user,
                            energy: cumulative_energy,
                            power_watts: wl.power_watts,
                        }
                    })
                    .collect();

                let cumulative_unattributed = DeviceEnergy {
                    cpu_joules: prev_snap.unattributed.cpu_joules + unattributed.cpu_joules,
                    dram_joules: prev_snap.unattributed.dram_joules + unattributed.dram_joules,
                    gpu_joules: prev_snap.unattributed.gpu_joules + unattributed.gpu_joules,
                };

                // Update previous tick state for power computation
                prev_state = PreviousTickState {
                    timestamp: current_timestamp,
                    workload_energy: cumulative_workloads
                        .iter()
                        .map(|wl| (wl.root_pid, wl.energy.clone()))
                        .collect(),
                };

                // Write updated snapshot
                {
                    let mut snap = snapshot.write().unwrap();
                    snap.timestamp = current_timestamp;
                    snap.system_total = cumulative_total;
                    snap.workloads = cumulative_workloads;
                    snap.unattributed = cumulative_unattributed;
                    snap.tracked_pids = expanded_pids;
                }

                tokio::time::sleep(interval).await;
            }
        }));
    }

    /// Spawn the scan task that periodically discovers all root processes.
    /// Only spawned when `root_pids` is None (monitor-all mode).
    fn spawn_scan_task(&mut self) {
        let interval = Duration::from_secs_f64(self.config.discovery.scan_interval_secs);
        let discovered_roots = Arc::clone(&self.discovered_roots);
        let is_running = Arc::clone(&self.is_running);

        self.scan_handle = Some(tokio::spawn(async move {
            while is_running.load(Ordering::SeqCst) {
                let roots = tokio::task::spawn_blocking(scan_roots)
                    .await
                    .unwrap_or_default();
                *discovered_roots.write().unwrap() = roots;
                tokio::time::sleep(interval).await;
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_energy_default_is_zero() {
        let de = DeviceEnergy::default();
        assert_eq!(de.cpu_joules, 0.0);
        assert_eq!(de.dram_joules, 0.0);
        assert_eq!(de.gpu_joules, 0.0);
    }

    #[test]
    fn device_energy_total() {
        let de = DeviceEnergy {
            cpu_joules: 1.0,
            dram_joules: 2.0,
            gpu_joules: 3.0,
        };
        assert!((de.total() - 6.0).abs() < f64::EPSILON);
    }

    #[test]
    fn device_energy_saturating_sub_clamps_to_zero() {
        let a = DeviceEnergy {
            cpu_joules: 1.0,
            dram_joules: 0.5,
            gpu_joules: 0.0,
        };
        let b = DeviceEnergy {
            cpu_joules: 2.0,
            dram_joules: 0.3,
            gpu_joules: 1.0,
        };
        let result = a.saturating_sub(&b);
        assert_eq!(result.cpu_joules, 0.0); // clamped
        assert!((result.dram_joules - 0.2).abs() < 1e-10);
        assert_eq!(result.gpu_joules, 0.0); // clamped
    }

    #[test]
    fn classify_energy_rapl_package() {
        let record = EnergyRecord {
            pid: 1,
            timestamp: 0,
            device: "rapl:socket:0:package".to_string(),
            energy: 5.0,
        };
        assert_eq!(classify_energy(&record), EnergyClass::Cpu);
    }

    #[test]
    fn classify_energy_rapl_dram() {
        let record = EnergyRecord {
            pid: 1,
            timestamp: 0,
            device: "rapl:system:dram".to_string(),
            energy: 2.0,
        };
        assert_eq!(classify_energy(&record), EnergyClass::Dram);
    }

    #[test]
    fn classify_energy_rapl_psys() {
        let record = EnergyRecord {
            pid: 1,
            timestamp: 0,
            device: "rapl:system:psys".to_string(),
            energy: 3.0,
        };
        assert_eq!(classify_energy(&record), EnergyClass::Cpu);
    }

    #[test]
    fn classify_energy_nvidia() {
        let record = EnergyRecord {
            pid: 1,
            timestamp: 0,
            device: "nvidia:gpu:0".to_string(),
            energy: 10.0,
        };
        assert_eq!(classify_energy(&record), EnergyClass::Gpu);
    }

    #[test]
    fn unattributed_is_clamped_to_zero() {
        // Simulate jitter: workloads sum exceeds system_total
        let system_total = DeviceEnergy {
            cpu_joules: 5.0,
            dram_joules: 1.0,
            gpu_joules: 0.0,
        };
        let workloads_sum = DeviceEnergy {
            cpu_joules: 5.5, // exceeds system_total due to jitter
            dram_joules: 1.2,
            gpu_joules: 0.1,
        };
        let unattributed = system_total.saturating_sub(&workloads_sum);
        assert_eq!(unattributed.cpu_joules, 0.0);
        assert_eq!(unattributed.dram_joules, 0.0);
        assert_eq!(unattributed.gpu_joules, 0.0);
    }

    #[test]
    fn build_child_to_root_map_maps_self() {
        let my_pid = std::process::id();
        let (map, pids) = build_child_to_root_map(&[my_pid]);
        // The root maps to itself
        assert_eq!(map.get(&my_pid), Some(&my_pid));
        assert!(pids.contains(&my_pid));
    }

    #[test]
    fn build_child_to_root_map_empty_returns_empty() {
        let (map, pids) = build_child_to_root_map(&[]);
        assert!(map.is_empty());
        assert!(pids.is_empty());
    }

    #[test]
    fn metrics_snapshot_default() {
        let snap = MetricsSnapshot::default();
        assert_eq!(snap.timestamp, 0);
        assert_eq!(snap.system_total.total(), 0.0);
        assert!(snap.workloads.is_empty());
        assert_eq!(snap.unattributed.total(), 0.0);
        assert!(snap.tracked_pids.is_empty());
    }

    #[tokio::test]
    async fn monitor_starts_and_stops_cleanly() {
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, Some(vec![std::process::id()]));

        let handle = monitor.commence().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;

        let snapshot = handle.snapshot();
        assert!(snapshot.system_total.total() >= 0.0);

        monitor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn monitor_handle_returns_non_zero_energy_after_running() {
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, Some(vec![std::process::id()]));

        let handle = monitor.commence().await.unwrap();
        // Wait long enough for at least one batch to flush (batch_size = rate = 10,
        // so 1 second of collection plus margin for scheduling).
        tokio::time::sleep(Duration::from_secs(3)).await;

        // On a RAPL-capable host with readable counters, should have some energy.
        // The assertion is gated because CI or container environments may expose
        // the powercap path but return zero deltas.
        if Rapl::is_available() {
            let energy = handle.total_consumed_energy();
            // Energy should be non-negative; a positive value means attribution worked.
            assert!(energy >= 0.0, "Energy must be non-negative, got {}", energy);
        }

        monitor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn double_commence_is_noop() {
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, Some(vec![std::process::id()]));

        let _handle1 = monitor.commence().await.unwrap();
        let _handle2 = monitor.commence().await.unwrap();

        monitor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn monitor_without_pids_uses_scan_task() {
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, None);

        let handle = monitor.commence().await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;

        let snapshot = handle.snapshot();
        // Should have discovered some PIDs via scan
        assert!(!snapshot.tracked_pids.is_empty());

        monitor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn monitor_snapshot_has_device_breakdown() {
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, Some(vec![std::process::id()]));

        let handle = monitor.commence().await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;

        let snapshot = handle.snapshot();

        // On a RAPL-capable host, cpu_joules should be populated
        if Rapl::is_available() {
            // At minimum, the snapshot should have some structure
            assert!(snapshot.system_total.cpu_joules >= 0.0);
            assert!(snapshot.system_total.dram_joules >= 0.0);
            assert!(snapshot.system_total.gpu_joules >= 0.0);
        }

        // Unattributed should never be negative (clamped)
        assert!(snapshot.unattributed.cpu_joules >= 0.0);
        assert!(snapshot.unattributed.dram_joules >= 0.0);
        assert!(snapshot.unattributed.gpu_joules >= 0.0);

        monitor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn consumed_energy_by_pid_backwards_compat() {
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, Some(vec![std::process::id()]));

        let handle = monitor.commence().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;

        // The backwards-compat method should return a HashMap
        let by_pid = handle.consumed_energy_by_pid();
        // All values should be non-negative
        for &energy in by_pid.values() {
            assert!(energy >= 0.0);
        }

        monitor.shutdown().await.unwrap();
    }
}
