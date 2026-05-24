use crate::energy_group::{EnergyCollector, EnergyRecord};
use async_trait::async_trait;
use chrono::Utc;
use log::{debug, warn};
use nvml_wrapper::enums::device::UsedGpuMemory;
use nvml_wrapper::Nvml;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::task;

/// NVIDIA GPU energy collector using direct NVML library bindings.
///
/// Replaces the previous `nvidia-smi` CLI-based approach with the `nvml-wrapper`
/// crate for significantly lower overhead (no process spawning per sample).
pub struct NvidiaGpu {
    /// NVML library handle. `None` when NVML is unavailable (graceful degradation).
    nvml: Option<Arc<Nvml>>,
    /// Number of GPU devices detected at construction time.
    device_count: u32,
    /// Optional device index filter. `None` means monitor all GPUs.
    device_filter: Option<Vec<u32>>,
    /// PIDs to attribute energy to.
    tracked_pids: Arc<Mutex<Vec<u32>>>,
    /// Previous cumulative energy reading (millijoules) per GPU index, used for delta computation.
    previous_energy_mj: Arc<Mutex<HashMap<u32, u64>>>,
}

impl NvidiaGpu {
    /// Construct a new collector that discovers all NVIDIA GPUs via NVML.
    pub fn new() -> Result<Self, String> {
        let nvml = Nvml::init().map_err(|e| format!("Failed to initialize NVML: {}", e))?;
        let device_count = nvml
            .device_count()
            .map_err(|e| format!("Failed to get device count: {}", e))?;
        Ok(Self {
            nvml: Some(Arc::new(nvml)),
            device_count,
            device_filter: None,
            tracked_pids: Arc::new(Mutex::new(Vec::new())),
            previous_energy_mj: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Construct a collector with an explicit device index filter.
    ///
    /// Only the GPUs whose indices appear in `device_ids` will be monitored.
    pub fn with_device_filter(device_ids: Vec<u32>) -> Result<Self, String> {
        let mut collector = Self::new()?;
        collector.device_filter = Some(device_ids);
        Ok(collector)
    }

    /// Compute the energy delta in joules from two consecutive millijoule readings.
    ///
    /// Returns 0.0 when there is no previous reading (first sample) or when the
    /// delta is negative (counter wrap or driver reset).
    fn compute_delta_joules(previous_mj: Option<u64>, current_mj: u64) -> f64 {
        previous_mj
            .map(|prev| {
                if current_mj >= prev {
                    (current_mj - prev) as f64 / 1000.0
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0)
    }

    /// Attribute a GPU energy delta to tracked processes by their GPU memory share.
    fn attribute_energy_for_processes(
        gpu_index: u32,
        delta_joules: f64,
        total_used_memory_bytes: u64,
        tracked_pid_set: &HashSet<u32>,
        process_memories: &[(u32, u64)],
        timestamp: i64,
    ) -> Vec<EnergyRecord> {
        if delta_joules <= 0.0 || total_used_memory_bytes == 0 {
            return Vec::new();
        }

        process_memories
            .iter()
            .filter(|(pid, mem)| tracked_pid_set.contains(pid) && *mem > 0)
            .map(|(pid, process_memory_bytes)| EnergyRecord {
                pid: *pid,
                timestamp,
                device: format!("nvidia:gpu:{}", gpu_index),
                energy: delta_joules * (*process_memory_bytes as f64 / total_used_memory_bytes as f64),
            })
            .collect()
    }

    /// Determine which device indices to iterate based on the optional filter.
    fn device_indices(&self) -> Vec<u32> {
        match &self.device_filter {
            Some(filter) => filter
                .iter()
                .filter(|&&idx| idx < self.device_count)
                .copied()
                .collect(),
            None => (0..self.device_count).collect(),
        }
    }
}

impl Default for NvidiaGpu {
    /// Creates a safe default that works even when no NVIDIA GPU is present.
    ///
    /// The `nvml` field will be `None` and all collection operations will
    /// gracefully return empty results.
    fn default() -> Self {
        match Self::new() {
            Ok(collector) => collector,
            Err(_) => Self {
                nvml: None,
                device_count: 0,
                device_filter: None,
                tracked_pids: Arc::new(Mutex::new(Vec::new())),
                previous_energy_mj: Arc::new(Mutex::new(HashMap::new())),
            },
        }
    }
}

#[async_trait]
impl EnergyCollector for NvidiaGpu {
    fn set_tracked_pids(&self, pids: Vec<u32>) {
        *self.tracked_pids.lock().unwrap() = pids;
    }

    async fn get_energy_trace(&self) -> Result<Vec<EnergyRecord>, String> {
        let nvml = match &self.nvml {
            Some(nvml) => Arc::clone(nvml),
            None => return Ok(Vec::new()),
        };

        let tracked_pids = self.tracked_pids.lock().unwrap().clone();
        if tracked_pids.is_empty() {
            return Ok(Vec::new());
        }

        let tracked_pid_set: HashSet<u32> = tracked_pids.into_iter().collect();
        let device_indices = self.device_indices();
        let previous_energy_mj = Arc::clone(&self.previous_energy_mj);

        // NVML calls are blocking; run them on a blocking thread to avoid
        // stalling the async runtime.
        let records = task::spawn_blocking(move || {
            let timestamp = Utc::now().timestamp_millis();
            let mut previous = previous_energy_mj.lock().unwrap();
            let mut records = Vec::new();

            for idx in device_indices {
                let device = match nvml.device_by_index(idx) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!("Failed to get NVIDIA device {}: {}", idx, e);
                        continue;
                    }
                };

                // Read cumulative energy consumption in millijoules.
                let current_energy_mj = match device.total_energy_consumption() {
                    Ok(mj) => mj,
                    Err(e) => {
                        warn!("Failed to read energy for GPU {}: {}", idx, e);
                        continue;
                    }
                };

                // Compute delta from previous sample.
                let prev = previous.get(&idx).copied();
                previous.insert(idx, current_energy_mj);
                let delta_joules = Self::compute_delta_joules(prev, current_energy_mj);

                // Get memory info for the total used memory on the device.
                let total_used_memory = match device.memory_info() {
                    Ok(info) => info.used,
                    Err(e) => {
                        warn!("Failed to read memory info for GPU {}: {}", idx, e);
                        continue;
                    }
                };

                // Get per-process GPU memory for compute processes.
                let process_memories: Vec<(u32, u64)> =
                    match device.running_compute_processes() {
                        Ok(procs) => procs
                            .iter()
                            .filter_map(|p| match p.used_gpu_memory {
                                UsedGpuMemory::Used(bytes) => Some((p.pid, bytes)),
                                UsedGpuMemory::Unavailable => None,
                            })
                            .collect(),
                        Err(e) => {
                            // No compute processes is a normal state; only warn
                            // on unexpected errors.
                            debug!(
                                "No compute processes on GPU {} ({}), skipping attribution",
                                idx, e
                            );
                            continue;
                        }
                    };

                if process_memories.is_empty() {
                    continue;
                }

                records.extend(Self::attribute_energy_for_processes(
                    idx,
                    delta_joules,
                    total_used_memory,
                    &tracked_pid_set,
                    &process_memories,
                    timestamp,
                ));
            }

            records
        })
        .await
        .map_err(|e| format!("Failed to join NVML collection task: {}", e))?;

        debug!(
            "NVIDIA GPU energy trace collected: {} records",
            records.len()
        );
        Ok(records)
    }

    fn is_available() -> bool {
        Nvml::init()
            .and_then(|nvml| nvml.device_count().map(|count| count > 0))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_delta_is_zero() {
        let delta = NvidiaGpu::compute_delta_joules(None, 1200);
        assert_eq!(delta, 0.0);
    }

    #[test]
    fn positive_delta_is_computed_correctly() {
        // 2200 mJ -> 3200 mJ = 1000 mJ = 1.0 J
        let delta = NvidiaGpu::compute_delta_joules(Some(2200), 3200);
        assert!((delta - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn negative_delta_is_clamped_to_zero() {
        let delta = NvidiaGpu::compute_delta_joules(Some(2200), 1200);
        assert_eq!(delta, 0.0);
    }

    #[test]
    fn zero_delta_when_no_change() {
        let delta = NvidiaGpu::compute_delta_joules(Some(5000), 5000);
        assert_eq!(delta, 0.0);
    }

    #[test]
    fn attributes_energy_by_process_memory_share() {
        let tracked: HashSet<u32> = HashSet::from([1001, 1002]);
        // Total used memory on GPU: 100 MB (in bytes)
        let total_used = 100 * 1024 * 1024;
        // Process memories in bytes
        let process_memories = vec![
            (1001, 40 * 1024 * 1024_u64),
            (1002, 60 * 1024 * 1024_u64),
            (9999, 30 * 1024 * 1024_u64),
        ];

        let records = NvidiaGpu::attribute_energy_for_processes(
            0,
            10.0,
            total_used,
            &tracked,
            &process_memories,
            42,
        );

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].pid, 1001);
        assert_eq!(records[1].pid, 1002);
        // 10.0 * (40/100) = 4.0
        assert!((records[0].energy - 4.0).abs() < f64::EPSILON);
        // 10.0 * (60/100) = 6.0
        assert!((records[1].energy - 6.0).abs() < f64::EPSILON);
    }

    #[test]
    fn attribution_returns_empty_on_zero_delta() {
        let tracked: HashSet<u32> = HashSet::from([1001]);
        let process_memories = vec![(1001, 1024_u64)];

        let records = NvidiaGpu::attribute_energy_for_processes(
            0,
            0.0, // zero delta
            4096,
            &tracked,
            &process_memories,
            42,
        );

        assert!(records.is_empty());
    }

    #[test]
    fn attribution_returns_empty_on_zero_memory() {
        let tracked: HashSet<u32> = HashSet::from([1001]);
        let process_memories = vec![(1001, 1024_u64)];

        let records = NvidiaGpu::attribute_energy_for_processes(
            0,
            10.0,
            0, // zero total used memory
            &tracked,
            &process_memories,
            42,
        );

        assert!(records.is_empty());
    }

    #[test]
    fn attribution_excludes_untracked_pids() {
        let tracked: HashSet<u32> = HashSet::from([1001]);
        let process_memories = vec![
            (1001, 50 * 1024 * 1024_u64),
            (9999, 50 * 1024 * 1024_u64), // not tracked
        ];
        let total_used = 100 * 1024 * 1024;

        let records = NvidiaGpu::attribute_energy_for_processes(
            0,
            10.0,
            total_used,
            &tracked,
            &process_memories,
            42,
        );

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].pid, 1001);
        // 10.0 * (50/100) = 5.0
        assert!((records[0].energy - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn default_without_gpu_is_safe() {
        // Default constructor should not panic regardless of GPU availability.
        let collector = NvidiaGpu::default();
        // device_count should be 0 if no GPU, >0 if GPU present
        // Either way, no panic is the success criterion.
        assert!(collector.device_count == 0 || collector.nvml.is_some());
    }

    #[test]
    fn device_indices_with_no_filter() {
        let collector = NvidiaGpu {
            nvml: None,
            device_count: 3,
            device_filter: None,
            tracked_pids: Arc::new(Mutex::new(Vec::new())),
            previous_energy_mj: Arc::new(Mutex::new(HashMap::new())),
        };
        assert_eq!(collector.device_indices(), vec![0, 1, 2]);
    }

    #[test]
    fn device_indices_with_filter() {
        let collector = NvidiaGpu {
            nvml: None,
            device_count: 4,
            device_filter: Some(vec![1, 3]),
            tracked_pids: Arc::new(Mutex::new(Vec::new())),
            previous_energy_mj: Arc::new(Mutex::new(HashMap::new())),
        };
        assert_eq!(collector.device_indices(), vec![1, 3]);
    }

    #[test]
    fn device_indices_filter_excludes_out_of_range() {
        let collector = NvidiaGpu {
            nvml: None,
            device_count: 2,
            device_filter: Some(vec![0, 1, 5, 10]),
            tracked_pids: Arc::new(Mutex::new(Vec::new())),
            previous_energy_mj: Arc::new(Mutex::new(HashMap::new())),
        };
        assert_eq!(collector.device_indices(), vec![0, 1]);
    }

    #[test]
    fn is_available_returns_false_without_gpu() {
        // On CI/test hosts without NVIDIA GPUs, this should return false (not panic).
        // On hosts with GPUs it returns true. Either is acceptable.
        let _ = NvidiaGpu::is_available();
    }

    #[tokio::test]
    async fn get_energy_trace_returns_empty_when_no_nvml() {
        let collector = NvidiaGpu {
            nvml: None,
            device_count: 0,
            device_filter: None,
            tracked_pids: Arc::new(Mutex::new(vec![1234])),
            previous_energy_mj: Arc::new(Mutex::new(HashMap::new())),
        };

        let result = collector.get_energy_trace().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_energy_trace_returns_empty_when_no_tracked_pids() {
        let collector = NvidiaGpu::default();
        // No tracked PIDs set
        let result = collector.get_energy_trace().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
