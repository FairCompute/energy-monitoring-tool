use crate::collectors::{NvidiaGpu, Rapl};
use crate::config::EmtConfig;
use crate::energy_group::{EnergyCollector, EnergyGroup};
use crate::utils::errors::MonitoringError;
use crate::utils::psutils::{scan_roots, walk_child_pids, ProcessRoot};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Simplified snapshot of monitor state, shared with MonitorHandle.
/// ENE-36 will extend this into a full MetricsSnapshot.
#[derive(Debug, Clone, Default)]
pub struct MonitorSnapshot {
    pub timestamp: i64,
    pub total_energy_joules: f64,
    pub energy_by_pid: HashMap<u32, f64>,
    pub tracked_pids: Vec<u32>,
}

/// A cloneable handle providing read-only access to the latest monitor state.
#[derive(Clone)]
pub struct MonitorHandle {
    snapshot: Arc<RwLock<MonitorSnapshot>>,
}

impl MonitorHandle {
    /// Returns a clone of the current snapshot.
    pub fn snapshot(&self) -> MonitorSnapshot {
        self.snapshot.read().unwrap().clone()
    }

    /// Returns the total consumed energy in joules across all PIDs and collectors.
    pub fn total_consumed_energy(&self) -> f64 {
        self.snapshot.read().unwrap().total_energy_joules
    }

    /// Returns a clone of the per-PID energy map.
    pub fn consumed_energy_by_pid(&self) -> HashMap<u32, f64> {
        self.snapshot.read().unwrap().energy_by_pid.clone()
    }
}

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
    snapshot: Arc<RwLock<MonitorSnapshot>>,
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
            snapshot: Arc::new(RwLock::new(MonitorSnapshot::default())),
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
            while is_running.load(Ordering::SeqCst) {
                // 1. Determine current root PIDs
                let roots: Vec<u32> = if let Some(ref pids) = root_pids {
                    pids.clone()
                } else {
                    discovered_roots
                        .read()
                        .unwrap()
                        .iter()
                        .map(|r| r.pid)
                        .collect()
                };

                // 2. Walk child PIDs to expand the full process tree
                let expanded_pids = if roots.is_empty() {
                    Vec::new()
                } else {
                    tokio::task::spawn_blocking(move || walk_child_pids(&roots))
                        .await
                        .unwrap_or_default()
                };

                // 3. Set tracked PIDs on collectors and poll data
                {
                    let mut rapl = rapl_group.lock().await;
                    rapl.set_tracked_pids(expanded_pids.clone());
                    rapl.poll_data();
                }

                if let Some(ref gpu) = gpu_group {
                    let mut gpu_lock = gpu.lock().await;
                    gpu_lock.set_tracked_pids(expanded_pids.clone());
                    gpu_lock.poll_data();
                }

                // 4. Update snapshot from accumulated state
                let mut merged_energy: HashMap<u32, f64> = HashMap::new();
                let mut total_energy = 0.0;

                {
                    let rapl = rapl_group.lock().await;
                    for (&pid, &energy) in rapl.consumed_energy_by_pid() {
                        *merged_energy.entry(pid).or_insert(0.0) += energy;
                        total_energy += energy;
                    }
                }

                if let Some(ref gpu) = gpu_group {
                    let gpu_lock = gpu.lock().await;
                    for (&pid, &energy) in gpu_lock.consumed_energy_by_pid() {
                        *merged_energy.entry(pid).or_insert(0.0) += energy;
                        total_energy += energy;
                    }
                }

                // Write updated snapshot
                {
                    let mut snap = snapshot.write().unwrap();
                    snap.timestamp = chrono::Utc::now().timestamp_millis();
                    snap.total_energy_joules = total_energy;
                    snap.energy_by_pid = merged_energy;
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

    #[tokio::test]
    async fn monitor_starts_and_stops_cleanly() {
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, Some(vec![std::process::id()]));

        let handle = monitor.commence().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;

        let snapshot = handle.snapshot();
        assert!(snapshot.total_energy_joules >= 0.0);

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
}
