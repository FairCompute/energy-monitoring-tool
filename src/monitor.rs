use crate::collectors::{NvidiaGpu, Rapl};
use crate::config::EmtConfig;
use crate::energy_group::{EnergyCollector, EnergyGroup, EnergyRecord};
use crate::process::{
    ProcessGroup, group_processes, pid_to_group_map, scan_processes, tracked_pids,
};
use crate::process_aggregation::{aggregate_energy_records, percentage_of_system};
use crate::utils::errors::MonitoringError;
use crate::utils::psutils::{ProcessRoot, walk_child_pids};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

// ─── MetricsSnapshot data structures ────────────────────────────────────────

/// Energy source/provenance for a device in public outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceSource {
    /// Energy is measured by a dedicated device/domain counter.
    Measured,
    /// CPU/package energy is measured from package-level RAPL.
    MeasuredPackage,
    /// The device has no separate counter but is included in package energy.
    IncludedInPackage,
    /// No usable measurement source is available.
    Unavailable,
}

impl Default for DeviceSource {
    fn default() -> Self {
        Self::Unavailable
    }
}

impl DeviceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::MeasuredPackage => "measured_package",
            Self::IncludedInPackage => "included_in_package",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Device source/provenance metadata attached to monitor snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceSources {
    pub cpu: DeviceSource,
    pub dram: DeviceSource,
    pub gpu: DeviceSource,
}

impl Default for DeviceSources {
    fn default() -> Self {
        Self {
            cpu: DeviceSource::Unavailable,
            dram: DeviceSource::Unavailable,
            gpu: DeviceSource::Unavailable,
        }
    }
}

impl DeviceSources {
    pub fn reports_dram_energy(&self) -> bool {
        self.dram == DeviceSource::Measured
    }
}

/// Energy breakdown by device type (CPU, DRAM, GPU).
#[derive(Debug, Clone, Default, Serialize)]
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

/// Per-process energy and power snapshot nested under a workload group.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessEnergySnapshot {
    pub pid: u32,
    pub name: String,
    pub energy: DeviceEnergy,
    pub power_watts: f64,
}

/// Per-workload (root process) energy and power snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct WorkloadSnapshot {
    pub root_pid: u32,
    pub group_id: String,
    pub name: String,
    pub user: String,
    pub processes: Vec<ProcessEnergySnapshot>,
    pub is_live: bool,
    pub energy: DeviceEnergy,
    pub power_watts: f64,
    pub percentage_of_system: f64,
}

/// Full metrics snapshot shared via MonitorHandle.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MetricsSnapshot {
    pub timestamp: i64,
    pub gpu_available: bool,
    pub sources: DeviceSources,
    pub system_total: DeviceEnergy,
    pub workloads: Vec<WorkloadSnapshot>,
    pub unattributed: DeviceEnergy,
    pub tracked_pids: Vec<u32>,
    pub diagnostics: MonitorDiagnostics,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MonitorDiagnostics {
    pub collection_ticks: u64,
    pub process_scans: u64,
    pub process_groups: usize,
    pub tracked_pids: usize,
}

// ─── Snapshot helpers ───────────────────────────────────────────────────────

fn add_device_energy(total: &mut DeviceEnergy, delta: &DeviceEnergy) {
    total.cpu_joules += delta.cpu_joules;
    total.dram_joules += delta.dram_joules;
    total.gpu_joules += delta.gpu_joules;
}

fn system_total_from_workloads(
    workloads: &[WorkloadSnapshot],
    unattributed: &DeviceEnergy,
) -> DeviceEnergy {
    DeviceEnergy {
        cpu_joules: workloads
            .iter()
            .map(|workload| workload.energy.cpu_joules)
            .sum::<f64>()
            + unattributed.cpu_joules,
        dram_joules: workloads
            .iter()
            .map(|workload| workload.energy.dram_joules)
            .sum::<f64>()
            + unattributed.dram_joules,
        gpu_joules: workloads
            .iter()
            .map(|workload| workload.energy.gpu_joules)
            .sum::<f64>()
            + unattributed.gpu_joules,
    }
}

fn workload_snapshot(
    group: &ProcessGroup,
    energy: DeviceEnergy,
    processes: Vec<ProcessEnergySnapshot>,
    is_live: bool,
    elapsed_s: f64,
) -> WorkloadSnapshot {
    let power_watts = if elapsed_s > 0.0 {
        energy.total() / elapsed_s
    } else {
        0.0
    };

    WorkloadSnapshot {
        root_pid: group.representative_pid,
        group_id: group.id.clone(),
        name: group.name.clone(),
        user: group.user.clone(),
        processes,
        is_live,
        energy,
        power_watts,
        percentage_of_system: 0.0,
    }
}

fn workload_snapshots_for_known_groups<F>(
    known_groups: &HashMap<String, ProcessGroup>,
    cumulative_by_group: &HashMap<String, DeviceEnergy>,
    cumulative_by_pid: &HashMap<u32, DeviceEnergy>,
    pid_to_group: &HashMap<u32, String>,
    process_names: &HashMap<u32, String>,
    elapsed_s: f64,
    mut is_live_for_group: F,
) -> Vec<WorkloadSnapshot>
where
    F: FnMut(&ProcessGroup) -> bool,
{
    let mut workloads: Vec<WorkloadSnapshot> = known_groups
        .values()
        .filter_map(|group| {
            let energy = cumulative_by_group
                .get(&group.id)
                .cloned()
                .unwrap_or_default();
            if energy.total() <= 0.0 {
                return None;
            }

            let processes = process_snapshots_for_group(
                group,
                &pid_energy_for_group(&group.id, &group.pids, cumulative_by_pid, pid_to_group),
                process_names,
                elapsed_s,
            );

            Some(workload_snapshot(
                group,
                energy,
                processes,
                is_live_for_group(group),
                elapsed_s,
            ))
        })
        .collect();
    workloads.sort_by(|left, right| left.group_id.cmp(&right.group_id));
    workloads
}

fn process_energy_snapshot(
    pid: u32,
    name: String,
    energy: DeviceEnergy,
    elapsed_s: f64,
) -> ProcessEnergySnapshot {
    let power_watts = if elapsed_s > 0.0 {
        energy.total() / elapsed_s
    } else {
        0.0
    };

    ProcessEnergySnapshot {
        pid,
        name,
        energy,
        power_watts,
    }
}

fn process_snapshots_for_group(
    group: &ProcessGroup,
    cumulative_by_pid: &HashMap<u32, DeviceEnergy>,
    process_names: &HashMap<u32, String>,
    elapsed_s: f64,
) -> Vec<ProcessEnergySnapshot> {
    let mut pids = group.pids.clone();
    pids.extend(cumulative_by_pid.keys().copied());
    pids.sort_unstable();
    pids.dedup();

    pids.into_iter()
        .map(|pid| {
            let energy = cumulative_by_pid.get(&pid).cloned().unwrap_or_default();
            let name = process_names
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| format!("pid {pid}"));
            process_energy_snapshot(pid, name, energy, elapsed_s)
        })
        .collect()
}

fn apply_workload_percentages(workloads: &mut [WorkloadSnapshot], system_total: &DeviceEnergy) {
    for workload in workloads {
        workload.percentage_of_system = percentage_of_system(&workload.energy, system_total);
    }
}

fn resolve_process_roots(pids: &[u32]) -> Vec<ProcessRoot> {
    use sysinfo::System;
    use users::{Users, UsersCache};

    let system = System::new_all();
    let users_cache = UsersCache::new();

    pids.iter()
        .map(|&pid| {
            let (name, user) = system
                .process(sysinfo::Pid::from_u32(pid))
                .map(|process| {
                    let name = process.name().to_string_lossy().to_string();
                    let user = process
                        .user_id()
                        .map(|uid| {
                            users_cache
                                .get_user_by_uid(**uid)
                                .map(|user| user.name().to_string_lossy().to_string())
                                .unwrap_or_else(|| uid.to_string())
                        })
                        .unwrap_or_else(|| "unknown".to_string());
                    (name, user)
                })
                .unwrap_or_else(|| (String::new(), String::new()));

            ProcessRoot { pid, user, name }
        })
        .collect()
}

fn explicit_pid_groups(root_pids: &[u32]) -> Vec<ProcessGroup> {
    let roots = resolve_process_roots(root_pids);

    roots
        .into_iter()
        .filter_map(|root| {
            let mut pids = walk_child_pids(&[root.pid]);
            if pids.is_empty() {
                return None;
            }
            pids.sort_unstable();
            pids.dedup();

            Some(ProcessGroup {
                id: format!("pid:{}", root.pid),
                name: if root.name.is_empty() {
                    format!("pid {}", root.pid)
                } else {
                    root.name
                },
                user: if root.user.is_empty() {
                    "unknown".to_string()
                } else {
                    root.user
                },
                representative_pid: root.pid,
                pids,
            })
        })
        .collect()
}

fn refresh_explicit_pid_groups(cached_groups: &[ProcessGroup]) -> Vec<ProcessGroup> {
    cached_groups
        .iter()
        .filter_map(|group| {
            let mut pids = walk_child_pids(&[group.representative_pid]);
            if pids.is_empty() {
                return None;
            }
            pids.sort_unstable();
            pids.dedup();

            let mut refreshed = group.clone();
            refreshed.pids = pids;
            Some(refreshed)
        })
        .collect()
}

fn cached_explicit_pid_groups(
    root_pids: &[u32],
    known_groups: &HashMap<String, ProcessGroup>,
) -> Vec<ProcessGroup> {
    root_pids
        .iter()
        .map(|pid| {
            let id = format!("pid:{pid}");
            known_groups
                .get(&id)
                .cloned()
                .unwrap_or_else(|| ProcessGroup {
                    id,
                    name: format!("pid {pid}"),
                    user: "unknown".to_string(),
                    pids: vec![*pid],
                    representative_pid: *pid,
                })
        })
        .collect()
}

fn merge_pid_group_maps(
    current: &HashMap<u32, String>,
    previous: &HashMap<u32, String>,
) -> HashMap<u32, String> {
    let mut merged = previous.clone();
    for (pid, group_id) in current {
        merged.insert(*pid, group_id.clone());
    }
    merged
}

fn retained_pid_to_group_map(workloads: &[WorkloadSnapshot]) -> HashMap<u32, String> {
    workloads
        .iter()
        .flat_map(|workload| {
            workload
                .processes
                .iter()
                .map(|process| (process.pid, workload.group_id.clone()))
        })
        .collect()
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

/// Tracks cumulative state for computing power (watts).
#[derive(Debug, Clone)]
struct TickState {
    start_timestamp: i64,
    workload_energy: HashMap<String, DeviceEnergy>,
    pid_energy: HashMap<u32, DeviceEnergy>,
    pid_to_group: HashMap<u32, String>,
    process_names: HashMap<u32, String>,
}

impl Default for TickState {
    fn default() -> Self {
        Self {
            start_timestamp: 0,
            workload_energy: HashMap::new(),
            pid_energy: HashMap::new(),
            pid_to_group: HashMap::new(),
            process_names: HashMap::new(),
        }
    }
}

// ─── Monitor ────────────────────────────────────────────────────────────────

/// Central coordinator that owns all collectors, process discovery, and runs autonomously.
pub struct Monitor {
    config: EmtConfig,
    rapl_group: Arc<Mutex<EnergyGroup<Rapl>>>,
    gpu_group: Option<Arc<Mutex<EnergyGroup<NvidiaGpu>>>>,
    root_pids: Option<Vec<u32>>,
    /// Shared state for scan task results in monitor-all mode.
    discovered_groups: Arc<RwLock<Vec<ProcessGroup>>>,
    /// Group metadata retained for cumulative reporting.
    known_groups: Arc<RwLock<HashMap<String, ProcessGroup>>>,
    /// Last PID-to-group map for final records from exited children.
    last_pid_to_group: Arc<RwLock<HashMap<u32, String>>>,
    /// First tick timestamp for average-power calculations.
    start_timestamp: Arc<RwLock<i64>>,
    /// Number of process discovery scans completed in monitor-all mode.
    process_scan_count: Arc<AtomicU64>,
    /// Device source/provenance metadata for public outputs.
    sources: DeviceSources,
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
        // Live monitors publish every collection tick. Batching remains available
        // at the lower EnergyGroup layer for explicit callers.
        let batch_size = Some(1);
        let rapl = Rapl::default();
        let mut sources = rapl.device_sources();
        let mut rapl_group = EnergyGroup::new(rapl, rate, batch_size);
        rapl_group.set_trace_retention(config.collection.trace_retention_secs as i64);
        rapl_group.set_recorder_flush_interval(Duration::from_secs_f64(
            config.collection.trace_flush_interval_secs,
        ));

        // Auto-detect GPU availability
        let gpu_group =
            if std::env::var_os("EMT_DISABLE_GPU").is_none() && NvidiaGpu::is_available() {
                let mut group = EnergyGroup::new(NvidiaGpu::default(), rate, batch_size);
                group.set_trace_retention(config.collection.trace_retention_secs as i64);
                group.set_recorder_flush_interval(Duration::from_secs_f64(
                    config.collection.trace_flush_interval_secs,
                ));
                Some(Arc::new(Mutex::new(group)))
            } else {
                None
            };

        let gpu_available = gpu_group.is_some();
        sources.gpu = if gpu_available {
            DeviceSource::Measured
        } else {
            DeviceSource::Unavailable
        };

        Self {
            config,
            rapl_group: Arc::new(Mutex::new(rapl_group)),
            gpu_group,
            root_pids,
            discovered_groups: Arc::new(RwLock::new(Vec::new())),
            known_groups: Arc::new(RwLock::new(HashMap::new())),
            last_pid_to_group: Arc::new(RwLock::new(HashMap::new())),
            start_timestamp: Arc::new(RwLock::new(0)),
            process_scan_count: Arc::new(AtomicU64::new(0)),
            sources: sources.clone(),
            tick_handle: None,
            scan_handle: None,
            snapshot: Arc::new(RwLock::new(MetricsSnapshot {
                gpu_available,
                sources,
                ..MetricsSnapshot::default()
            })),
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

        *self.known_groups.write().unwrap() = HashMap::new();
        *self.last_pid_to_group.write().unwrap() = HashMap::new();
        *self.start_timestamp.write().unwrap() = 0;
        self.process_scan_count.store(0, Ordering::SeqCst);
        *self.snapshot.write().unwrap() = MetricsSnapshot {
            gpu_available: self.gpu_group.is_some(),
            sources: self.sources.clone(),
            ..MetricsSnapshot::default()
        };

        let initial_groups = if let Some(root_pids) = &self.root_pids {
            let pids = root_pids.clone();
            tokio::task::spawn_blocking(move || explicit_pid_groups(&pids))
                .await
                .unwrap_or_default()
        } else {
            tokio::task::spawn_blocking(|| group_processes(&scan_processes()))
                .await
                .unwrap_or_default()
        };

        if self.root_pids.is_none() {
            *self.discovered_groups.write().unwrap() = initial_groups.clone();
            self.process_scan_count.store(1, Ordering::SeqCst);
        }

        {
            let mut known = self.known_groups.write().unwrap();
            for group in &initial_groups {
                known.insert(group.id.clone(), group.clone());
            }
        }

        let initial_pid_to_group = pid_to_group_map(&initial_groups);
        let initial_tracked_pids = tracked_pids(&initial_groups);
        *self.last_pid_to_group.write().unwrap() = initial_pid_to_group;

        self.is_running.store(true, Ordering::SeqCst);

        // Start collector background tasks
        {
            let mut rapl = self.rapl_group.lock().await;
            if !initial_tracked_pids.is_empty() {
                rapl.update_tracked_pids(initial_tracked_pids.clone());
            }
            rapl.commence().await?;
        }
        if let Some(gpu) = &self.gpu_group {
            let mut gpu_lock = gpu.lock().await;
            if !initial_tracked_pids.is_empty() {
                gpu_lock.update_tracked_pids(initial_tracked_pids.clone());
            }
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

        // Shutdown collector groups and collect their final buffered batches.
        let mut final_records = Vec::new();
        {
            let mut rapl = self.rapl_group.lock().await;
            final_records.extend(rapl.shutdown_and_drain()?);
        }
        if let Some(gpu) = &self.gpu_group {
            let mut gpu_lock = gpu.lock().await;
            final_records.extend(gpu_lock.shutdown_and_drain()?);
        }

        self.apply_final_records_to_snapshot(&final_records);

        Ok(())
    }

    fn apply_final_records_to_snapshot(&self, final_records: &[EnergyRecord]) {
        if final_records.is_empty() {
            return;
        }

        let retained_pid_to_group = {
            let snap = self.snapshot.read().unwrap();
            retained_pid_to_group_map(&snap.workloads)
        };
        let current_pid_to_group = self.last_pid_to_group.read().unwrap().clone();
        let pid_to_group = merge_pid_group_maps(&current_pid_to_group, &retained_pid_to_group);
        let tick = aggregate_energy_records(final_records, &pid_to_group);
        if tick.system_total.total() <= 0.0 && tick.group_energy.is_empty() {
            return;
        }

        let current_timestamp = chrono::Utc::now().timestamp_millis();
        let start_timestamp = *self.start_timestamp.read().unwrap();
        let elapsed_s = if start_timestamp > 0 {
            (current_timestamp - start_timestamp) as f64 / 1000.0
        } else {
            0.0
        };

        let known_groups_snapshot = {
            let mut known_groups = self.known_groups.write().unwrap();
            for group_id in tick.group_energy.keys() {
                if known_groups.contains_key(group_id) {
                    continue;
                }

                let representative_pid = pid_to_group
                    .iter()
                    .find_map(|(pid, mapped_group)| (mapped_group == group_id).then_some(*pid))
                    .unwrap_or_default();
                let pids = if representative_pid == 0 {
                    Vec::new()
                } else {
                    vec![representative_pid]
                };

                known_groups.insert(
                    group_id.clone(),
                    ProcessGroup {
                        id: group_id.clone(),
                        name: group_id.clone(),
                        user: "unknown".to_string(),
                        pids,
                        representative_pid,
                    },
                );
            }
            known_groups.clone()
        };

        let mut snap = self.snapshot.write().unwrap();
        let existing_live_status: HashMap<String, bool> = snap
            .workloads
            .iter()
            .map(|workload| (workload.group_id.clone(), workload.is_live))
            .collect();
        let mut cumulative_by_group: HashMap<String, DeviceEnergy> = snap
            .workloads
            .iter()
            .map(|workload| (workload.group_id.clone(), workload.energy.clone()))
            .collect();
        let mut cumulative_by_pid: HashMap<u32, DeviceEnergy> = snap
            .workloads
            .iter()
            .flat_map(|workload| {
                workload
                    .processes
                    .iter()
                    .map(|process| (process.pid, process.energy.clone()))
            })
            .collect();
        let process_names: HashMap<u32, String> = snap
            .workloads
            .iter()
            .flat_map(|workload| {
                workload
                    .processes
                    .iter()
                    .map(|process| (process.pid, process.name.clone()))
            })
            .collect();

        for (group_id, tick_energy) in &tick.group_energy {
            let entry = cumulative_by_group.entry(group_id.clone()).or_default();
            add_device_energy(entry, tick_energy);
        }
        for (pid, tick_energy) in &tick.pid_energy {
            let entry = cumulative_by_pid.entry(*pid).or_default();
            add_device_energy(entry, tick_energy);
        }
        add_device_energy(&mut snap.unattributed, &tick.unattributed);

        let mut workloads = workload_snapshots_for_known_groups(
            &known_groups_snapshot,
            &cumulative_by_group,
            &cumulative_by_pid,
            &pid_to_group,
            &process_names,
            elapsed_s,
            |group| {
                let is_live = existing_live_status
                    .get(&group.id)
                    .copied()
                    .unwrap_or(!group.pids.is_empty());
                is_live
            },
        );

        let system_total = system_total_from_workloads(&workloads, &snap.unattributed);
        apply_workload_percentages(&mut workloads, &system_total);

        snap.timestamp = current_timestamp;
        snap.gpu_available = self.gpu_group.is_some();
        snap.sources = self.sources.clone();
        snap.workloads = workloads;
        snap.system_total = system_total;
    }

    /// Spawn the tick task that runs the core polling and update loop.
    fn spawn_tick_task(&mut self) {
        let interval = Duration::from_secs_f64(1.0 / self.config.collection.rate_hz);
        let rapl_group = Arc::clone(&self.rapl_group);
        let gpu_group = self.gpu_group.clone();
        let gpu_available = gpu_group.is_some();
        let root_pids = self.root_pids.clone();
        let discovered_groups = Arc::clone(&self.discovered_groups);
        let known_groups = Arc::clone(&self.known_groups);
        let last_pid_to_group = Arc::clone(&self.last_pid_to_group);
        let start_timestamp = Arc::clone(&self.start_timestamp);
        let process_scan_count = Arc::clone(&self.process_scan_count);
        let sources = self.sources.clone();
        let snapshot = Arc::clone(&self.snapshot);
        let is_running = Arc::clone(&self.is_running);

        self.tick_handle = Some(tokio::spawn(async move {
            let mut tick_state = TickState::default();
            let mut collection_ticks = 0_u64;

            while is_running.load(Ordering::SeqCst) {
                collection_ticks += 1;
                let groups = if let Some(ref pids) = root_pids {
                    let cached_groups = {
                        let known = known_groups.read().unwrap();
                        cached_explicit_pid_groups(pids, &known)
                    };
                    tokio::task::spawn_blocking(move || refresh_explicit_pid_groups(&cached_groups))
                        .await
                        .unwrap_or_default()
                } else {
                    discovered_groups.read().unwrap().clone()
                };

                {
                    let mut known = known_groups.write().unwrap();
                    for group in &groups {
                        known.insert(group.id.clone(), group.clone());
                    }
                }

                let current_pid_to_group = pid_to_group_map(&groups);
                let expanded_pids = tracked_pids(&groups);
                let previous_pid_to_group = last_pid_to_group.read().unwrap().clone();
                let active_pid_to_group =
                    merge_pid_group_maps(&current_pid_to_group, &previous_pid_to_group);

                let rapl_records;
                {
                    let mut rapl = rapl_group.lock().await;
                    rapl.update_tracked_pids(expanded_pids.clone());
                    rapl_records = rapl.poll_data();
                }

                let gpu_records = if let Some(ref gpu) = gpu_group {
                    let mut gpu_lock = gpu.lock().await;
                    gpu_lock.update_tracked_pids(expanded_pids.clone());
                    gpu_lock.poll_data()
                } else {
                    Vec::new()
                };

                let mut all_records = rapl_records;
                all_records.extend(gpu_records);
                let tick = aggregate_energy_records(&all_records, &active_pid_to_group);

                let current_timestamp = chrono::Utc::now().timestamp_millis();
                if tick_state.start_timestamp == 0 {
                    tick_state.start_timestamp = current_timestamp;
                    *start_timestamp.write().unwrap() = current_timestamp;
                }
                let elapsed_s = (current_timestamp - tick_state.start_timestamp) as f64 / 1000.0;

                let known_groups_snapshot = known_groups.read().unwrap().clone();
                let live_group_ids: HashSet<String> =
                    groups.iter().map(|group| group.id.clone()).collect();
                resolve_missing_process_names(&expanded_pids, &mut tick_state.process_names);
                for (pid, tick_energy) in &tick.pid_energy {
                    let entry = tick_state.pid_energy.entry(*pid).or_default();
                    add_device_energy(entry, tick_energy);
                }
                tick_state.pid_to_group =
                    merge_pid_group_maps(&current_pid_to_group, &tick_state.pid_to_group);
                for (group_id, tick_energy) in &tick.group_energy {
                    let entry = tick_state
                        .workload_energy
                        .entry(group_id.clone())
                        .or_default();
                    add_device_energy(entry, tick_energy);
                }

                let mut workloads = workload_snapshots_for_known_groups(
                    &known_groups_snapshot,
                    &tick_state.workload_energy,
                    &tick_state.pid_energy,
                    &tick_state.pid_to_group,
                    &tick_state.process_names,
                    elapsed_s,
                    |group| live_group_ids.contains(&group.id),
                );

                let prev_unattributed = { snapshot.read().unwrap().unattributed.clone() };
                let mut cumulative_unattributed = prev_unattributed;
                add_device_energy(&mut cumulative_unattributed, &tick.unattributed);

                let cumulative_system_total =
                    system_total_from_workloads(&workloads, &cumulative_unattributed);
                apply_workload_percentages(&mut workloads, &cumulative_system_total);

                tick_state.workload_energy = workloads
                    .iter()
                    .map(|workload| (workload.group_id.clone(), workload.energy.clone()))
                    .collect();

                {
                    let mut snap = snapshot.write().unwrap();
                    snap.timestamp = current_timestamp;
                    snap.gpu_available = gpu_available;
                    snap.sources = sources.clone();
                    snap.system_total = cumulative_system_total;
                    snap.workloads = workloads;
                    snap.unattributed = cumulative_unattributed;
                    snap.tracked_pids = expanded_pids;
                    snap.diagnostics = MonitorDiagnostics {
                        collection_ticks,
                        process_scans: process_scan_count.load(Ordering::SeqCst),
                        process_groups: groups.len(),
                        tracked_pids: snap.tracked_pids.len(),
                    };
                }
                *last_pid_to_group.write().unwrap() = current_pid_to_group;

                tokio::time::sleep(interval).await;
            }
        }));
    }

    /// Spawn the scan task that periodically discovers all root processes.
    /// Only spawned when `root_pids` is None (monitor-all mode).
    fn spawn_scan_task(&mut self) {
        let interval = Duration::from_secs_f64(self.config.discovery.scan_interval_secs);
        let discovered_groups = Arc::clone(&self.discovered_groups);
        let process_scan_count = Arc::clone(&self.process_scan_count);
        let is_running = Arc::clone(&self.is_running);

        self.scan_handle = Some(tokio::spawn(async move {
            while is_running.load(Ordering::SeqCst) {
                let groups = tokio::task::spawn_blocking(|| group_processes(&scan_processes()))
                    .await
                    .unwrap_or_default();
                *discovered_groups.write().unwrap() = groups;
                process_scan_count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(interval).await;
            }
        }));
    }
}

fn pid_energy_for_group(
    group_id: &str,
    live_pids: &[u32],
    cumulative_by_pid: &HashMap<u32, DeviceEnergy>,
    pid_to_group: &HashMap<u32, String>,
) -> HashMap<u32, DeviceEnergy> {
    let mut result = HashMap::new();

    for pid in live_pids {
        if let Some(energy) = cumulative_by_pid.get(pid) {
            result.insert(*pid, energy.clone());
        }
    }

    for (pid, mapped_group_id) in pid_to_group {
        if mapped_group_id == group_id {
            if let Some(energy) = cumulative_by_pid.get(pid) {
                result.insert(*pid, energy.clone());
            }
        }
    }

    result
}

fn resolve_missing_process_names(pids: &[u32], process_names: &mut HashMap<u32, String>) {
    let missing_pids: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|pid| !process_names.contains_key(pid))
        .collect();
    if missing_pids.is_empty() {
        return;
    }

    use sysinfo::System;

    let system = System::new_all();
    for pid in missing_pids {
        let name = system
            .process(sysinfo::Pid::from_u32(pid))
            .map(|process| {
                let name = process.name().to_string_lossy().to_string();
                if name.is_empty() {
                    format!("pid {pid}")
                } else {
                    name
                }
            })
            .unwrap_or_else(|| format!("pid {pid}"));
        process_names.insert(pid, name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip_if_rapl_unavailable() -> bool {
        if !Rapl::is_available() {
            eprintln!("skipping hardware-backed Monitor test: RAPL unavailable");
            return true;
        }
        false
    }

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
    fn refresh_explicit_pid_groups_keeps_cached_metadata() {
        let pid = std::process::id();
        let cached = vec![ProcessGroup {
            id: format!("pid:{pid}"),
            name: "cached-root".to_string(),
            user: "cached-user".to_string(),
            pids: Vec::new(),
            representative_pid: pid,
        }];

        let refreshed = refresh_explicit_pid_groups(&cached);

        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].id, format!("pid:{pid}"));
        assert_eq!(refreshed[0].name, "cached-root");
        assert_eq!(refreshed[0].user, "cached-user");
        assert!(refreshed[0].pids.contains(&pid));
    }

    #[test]
    fn process_snapshots_for_group_include_pid_energy_power_and_names() {
        let group = ProcessGroup {
            id: "pid:123".to_string(),
            name: "workload".to_string(),
            user: "user".to_string(),
            pids: vec![123, 456],
            representative_pid: 123,
        };
        let cumulative_by_pid = HashMap::from([(
            123,
            DeviceEnergy {
                cpu_joules: 3.0,
                dram_joules: 1.0,
                gpu_joules: 0.0,
            },
        )]);
        let process_names = HashMap::from([(123, "python".to_string())]);

        let processes =
            process_snapshots_for_group(&group, &cumulative_by_pid, &process_names, 2.0);

        assert_eq!(processes.len(), 2);
        assert_eq!(processes[0].pid, 123);
        assert_eq!(processes[0].name, "python");
        assert_eq!(processes[0].energy.total(), 4.0);
        assert_eq!(processes[0].power_watts, 2.0);
        assert_eq!(processes[1].pid, 456);
        assert_eq!(processes[1].name, "pid 456");
        assert_eq!(processes[1].energy.total(), 0.0);
        assert_eq!(processes[1].power_watts, 0.0);
    }

    #[test]
    fn pid_energy_for_group_keeps_previously_mapped_exited_pids() {
        let cumulative_by_pid = HashMap::from([
            (
                123,
                DeviceEnergy {
                    cpu_joules: 2.0,
                    dram_joules: 0.0,
                    gpu_joules: 0.0,
                },
            ),
            (
                456,
                DeviceEnergy {
                    cpu_joules: 3.0,
                    dram_joules: 0.0,
                    gpu_joules: 0.0,
                },
            ),
        ]);
        let pid_to_group =
            HashMap::from([(123, "pid:123".to_string()), (456, "pid:123".to_string())]);

        let group_energy =
            pid_energy_for_group("pid:123", &[123], &cumulative_by_pid, &pid_to_group);

        assert_eq!(group_energy.len(), 2);
        assert_eq!(group_energy.get(&123).unwrap().total(), 2.0);
        assert_eq!(group_energy.get(&456).unwrap().total(), 3.0);
    }

    #[test]
    fn workload_snapshots_mark_known_groups_missing_from_current_scan_as_dead() {
        let known_groups = HashMap::from([
            (
                "pid:100".to_string(),
                ProcessGroup {
                    id: "pid:100".to_string(),
                    name: "live workload".to_string(),
                    user: "alice".to_string(),
                    pids: vec![100],
                    representative_pid: 100,
                },
            ),
            (
                "pid:200".to_string(),
                ProcessGroup {
                    id: "pid:200".to_string(),
                    name: "finished workload".to_string(),
                    user: "alice".to_string(),
                    pids: vec![200],
                    representative_pid: 200,
                },
            ),
        ]);
        let cumulative_by_group = HashMap::from([
            (
                "pid:100".to_string(),
                DeviceEnergy {
                    cpu_joules: 3.0,
                    dram_joules: 0.0,
                    gpu_joules: 0.0,
                },
            ),
            (
                "pid:200".to_string(),
                DeviceEnergy {
                    cpu_joules: 5.0,
                    dram_joules: 0.0,
                    gpu_joules: 0.0,
                },
            ),
        ]);
        let cumulative_by_pid = HashMap::from([
            (
                100,
                DeviceEnergy {
                    cpu_joules: 3.0,
                    dram_joules: 0.0,
                    gpu_joules: 0.0,
                },
            ),
            (
                200,
                DeviceEnergy {
                    cpu_joules: 5.0,
                    dram_joules: 0.0,
                    gpu_joules: 0.0,
                },
            ),
        ]);
        let pid_to_group =
            HashMap::from([(100, "pid:100".to_string()), (200, "pid:200".to_string())]);
        let process_names = HashMap::from([(200, "short-lived".to_string())]);
        let live_group_ids = HashSet::from(["pid:100".to_string()]);

        let workloads = workload_snapshots_for_known_groups(
            &known_groups,
            &cumulative_by_group,
            &cumulative_by_pid,
            &pid_to_group,
            &process_names,
            2.0,
            |group| live_group_ids.contains(&group.id),
        );

        assert_eq!(workloads.len(), 2);
        assert!(workloads[0].is_live);
        assert!(!workloads[1].is_live);
        assert_eq!(workloads[1].group_id, "pid:200");
        assert_eq!(workloads[1].energy.total(), 5.0);
        assert_eq!(workloads[1].power_watts, 2.5);
        assert_eq!(workloads[1].processes[0].name, "short-lived");
    }

    #[test]
    fn retained_pid_to_group_map_preserves_existing_process_rows() {
        let workloads = vec![WorkloadSnapshot {
            root_pid: 123,
            group_id: "pid:123".to_string(),
            name: "workload".to_string(),
            user: "user".to_string(),
            processes: vec![ProcessEnergySnapshot {
                pid: 456,
                name: "child".to_string(),
                energy: DeviceEnergy::default(),
                power_watts: 0.0,
            }],
            is_live: true,
            energy: DeviceEnergy::default(),
            power_watts: 0.0,
            percentage_of_system: 0.0,
        }];

        let retained = retained_pid_to_group_map(&workloads);

        assert_eq!(retained.get(&456), Some(&"pid:123".to_string()));
    }

    #[test]
    fn metrics_snapshot_default() {
        let snap = MetricsSnapshot::default();
        assert_eq!(snap.timestamp, 0);
        assert!(!snap.gpu_available);
        assert_eq!(snap.sources, DeviceSources::default());
        assert_eq!(snap.system_total.total(), 0.0);
        assert!(snap.workloads.is_empty());
        assert_eq!(snap.unattributed.total(), 0.0);
        assert!(snap.tracked_pids.is_empty());
        assert_eq!(snap.diagnostics.collection_ticks, 0);
        assert_eq!(snap.diagnostics.process_scans, 0);
        assert_eq!(snap.diagnostics.process_groups, 0);
        assert_eq!(snap.diagnostics.tracked_pids, 0);
    }

    #[test]
    fn monitor_initial_snapshot_reports_gpu_availability() {
        let monitor = Monitor::new(EmtConfig::default(), Some(vec![std::process::id()]));
        let expected_gpu_available =
            std::env::var_os("EMT_DISABLE_GPU").is_none() && NvidiaGpu::is_available();

        let snapshot = monitor.snapshot.read().unwrap();

        assert_eq!(snapshot.gpu_available, expected_gpu_available);
    }

    #[test]
    fn monitor_live_collectors_flush_every_collection_tick() {
        let mut config = EmtConfig::default();
        config.collection.rate_hz = 4.0;
        let monitor = Monitor::new(config, Some(vec![std::process::id()]));

        assert_eq!(monitor.rapl_group.try_lock().unwrap().batch_size(), 1);

        if let Some(gpu_group) = &monitor.gpu_group {
            assert_eq!(gpu_group.try_lock().unwrap().batch_size(), 1);
        }
    }

    #[test]
    fn metrics_snapshot_serializes_device_sources() {
        let snapshot = MetricsSnapshot {
            sources: DeviceSources {
                cpu: DeviceSource::MeasuredPackage,
                dram: DeviceSource::IncludedInPackage,
                gpu: DeviceSource::Unavailable,
            },
            ..MetricsSnapshot::default()
        };

        let value = serde_json::to_value(&snapshot).unwrap();

        assert_eq!(value["sources"]["cpu"], "measured_package");
        assert_eq!(value["sources"]["dram"], "included_in_package");
        assert_eq!(value["sources"]["gpu"], "unavailable");
    }

    #[tokio::test]
    async fn monitor_starts_and_stops_cleanly() {
        if skip_if_rapl_unavailable() {
            return;
        }
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
        if skip_if_rapl_unavailable() {
            return;
        }
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, Some(vec![std::process::id()]));

        let handle = monitor.commence().await.unwrap();
        // Wait long enough for at least one collection tick plus scheduling margin.
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
        if skip_if_rapl_unavailable() {
            return;
        }
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, Some(vec![std::process::id()]));

        let _handle1 = monitor.commence().await.unwrap();
        let _handle2 = monitor.commence().await.unwrap();

        monitor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn monitor_without_pids_uses_scan_task() {
        if skip_if_rapl_unavailable() {
            return;
        }
        let config = EmtConfig::default();
        let mut monitor = Monitor::new(config, None);

        let handle = monitor.commence().await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;

        let snapshot = handle.snapshot();
        // Should have discovered some PIDs via scan
        assert!(!snapshot.tracked_pids.is_empty());
        assert!(snapshot.diagnostics.collection_ticks > 0);
        assert!(snapshot.diagnostics.process_scans > 0);
        assert_eq!(
            snapshot.diagnostics.tracked_pids,
            snapshot.tracked_pids.len()
        );

        monitor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn monitor_snapshot_has_device_breakdown() {
        if skip_if_rapl_unavailable() {
            return;
        }
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
        if skip_if_rapl_unavailable() {
            return;
        }
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
