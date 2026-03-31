use crate::energy_group::{EnergyCollector, EnergyRecord};
use async_trait::async_trait;
use chrono::Utc;
use log::{debug, warn};
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tokio::task;

pub struct NvidiaGpu {
    pub device_ids: Vec<u32>,
    tracked_pids: Arc<Mutex<Vec<u32>>>,
    previous_energy_mj: Arc<Mutex<HashMap<u32, f64>>>,
}

#[derive(Debug, Clone)]
struct GpuSnapshot {
    index: u32,
    uuid: String,
    total_energy_mj: f64,
    used_memory_mib: f64,
}

impl NvidiaGpu {
    pub fn new(device_ids: Vec<u32>) -> Self {
        Self {
            device_ids,
            tracked_pids: Arc::new(Mutex::new(Vec::new())),
            previous_energy_mj: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn run_nvidia_smi(args: &[&str]) -> Result<String, String> {
        let args_vec: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let task_result = task::spawn_blocking(move || {
            let output = Command::new("nvidia-smi")
                .args(&args_vec)
                .output()
                .map_err(|e| format!("Failed to execute nvidia-smi with args {:?}: {}", args_vec, e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!(
                    "nvidia-smi command failed with args {:?}: {}",
                    args_vec,
                    stderr.trim()
                ));
            }

            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        })
        .await
        .map_err(|e| format!("Failed to join nvidia-smi task with args {:?}: {}", args, e))?;

        task_result
    }

    fn parse_gpu_snapshot_line(line: &str) -> Option<GpuSnapshot> {
        let fields: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if fields.len() != 4 {
            return None;
        }

        let index = fields[0].parse::<u32>().ok()?;
        let total_energy_mj = fields[2].parse::<f64>().ok()?;
        let used_memory_mib = fields[3].parse::<f64>().ok()?;

        Some(GpuSnapshot {
            index,
            uuid: fields[1].to_string(),
            total_energy_mj,
            used_memory_mib,
        })
    }

    fn parse_process_line(line: &str) -> Option<(String, u32, f64)> {
        let fields: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if fields.len() != 3 {
            return None;
        }

        let pid = fields[1].parse::<u32>().ok()?;
        let used_memory_mib = fields[2].parse::<f64>().ok()?;

        Some((fields[0].to_string(), pid, used_memory_mib))
    }

    async fn query_gpus(&self) -> Result<Vec<GpuSnapshot>, String> {
        let output = Self::run_nvidia_smi(&[
            "--query-gpu=index,uuid,total_energy_consumption,memory.used",
            "--format=csv,noheader,nounits",
        ])
        .await?;

        let wanted_devices: HashSet<u32> = self.device_ids.iter().cloned().collect();
        let snapshots = output
            .lines()
            .filter_map(Self::parse_gpu_snapshot_line)
            .filter(|gpu| wanted_devices.contains(&gpu.index))
            .collect::<Vec<_>>();

        Ok(snapshots)
    }

    async fn query_compute_processes(&self) -> HashMap<String, Vec<(u32, f64)>> {
        let output = Self::run_nvidia_smi(&[
            "--query-compute-apps=gpu_uuid,pid,used_gpu_memory",
            "--format=csv,noheader,nounits",
        ])
        .await;

        let output = match output {
            Ok(output) => output,
            Err(err) => {
                // Treat failures as no active processes, but log so outages/misconfigurations are visible.
                warn!(
                    "Failed to query active compute processes via nvidia-smi: {}",
                    err
                );
                return HashMap::new();
            }
        };

        let mut per_gpu_processes: HashMap<String, Vec<(u32, f64)>> = HashMap::new();
        for process in output.lines().filter_map(Self::parse_process_line) {
            let (uuid, pid, used_memory_mib) = process;
            per_gpu_processes
                .entry(uuid)
                .or_default()
                .push((pid, used_memory_mib));
        }

        per_gpu_processes
    }
}

impl Default for NvidiaGpu {
    fn default() -> Self {
        Self::new(vec![0]) // Default to GPU 0
    }
}

#[async_trait]
impl EnergyCollector for NvidiaGpu {
    fn set_tracked_pids(&mut self, pids: Vec<u32>) {
        *self.tracked_pids.lock().unwrap() = pids;
    }

    async fn get_energy_trace(&self) -> Result<Vec<EnergyRecord>, String> {
        let timestamp = Utc::now().timestamp_millis();
        let tracked_pids = self.tracked_pids.lock().unwrap().clone();
        if tracked_pids.is_empty() {
            return Ok(Vec::new());
        }

        let tracked_pid_set: HashSet<u32> = tracked_pids.into_iter().collect();
        let gpus = match self.query_gpus().await {
            Ok(gpus) => gpus,
            Err(e) => {
                warn!("Could not query NVIDIA GPU energy: {}", e);
                return Ok(Vec::new());
            }
        };

        let per_gpu_processes = self.query_compute_processes().await;
        let mut previous_energy_mj = self.previous_energy_mj.lock().unwrap();
        let mut records = Vec::new();

        for gpu in gpus {
            let previous = previous_energy_mj.get(&gpu.index).copied();
            previous_energy_mj.insert(gpu.index, gpu.total_energy_mj);
            let delta_joules = previous
                .map(|prev| ((gpu.total_energy_mj - prev) / 1000.0).max(0.0))
                .unwrap_or(0.0);

            if delta_joules <= 0.0 || gpu.used_memory_mib <= 0.0 {
                continue;
            }

            let process_memories = per_gpu_processes
                .get(&gpu.uuid)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|(pid, used_mem)| tracked_pid_set.contains(pid) && *used_mem > 0.0)
                .collect::<Vec<_>>();

            if process_memories.is_empty() {
                continue;
            }

            for (pid, process_memory_mib) in process_memories {
                // Match Python collector attribution: distribute zone energy by each tracked
                // process memory share on the GPU for this sampling interval.
                let energy = delta_joules * (process_memory_mib / gpu.used_memory_mib);
                records.push(EnergyRecord {
                    pid,
                    timestamp,
                    device: format!("nvidia:gpu:{}", gpu.index),
                    energy,
                });
            }
        }

        debug!("NVIDIA GPU energy trace collected: {} records", records.len());
        Ok(records)
    }

    fn is_available() -> bool {
        // Check if nvidia-smi command exists or NVIDIA drivers are loaded
        Command::new("nvidia-smi")
            .arg("--query-gpu=count")
            .arg("--format=csv,noheader,nounits")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gpu_snapshot_line() {
        let parsed =
            NvidiaGpu::parse_gpu_snapshot_line("0, GPU-1234, 1200, 2048").expect("must parse");
        assert_eq!(parsed.index, 0);
        assert_eq!(parsed.uuid, "GPU-1234");
        assert_eq!(parsed.total_energy_mj, 1200.0);
        assert_eq!(parsed.used_memory_mib, 2048.0);
    }

    #[test]
    fn rejects_gpu_snapshot_line_with_non_numeric_energy() {
        let parsed = NvidiaGpu::parse_gpu_snapshot_line("0, GPU-1234, N/A, 2048");
        assert!(parsed.is_none());
    }

    #[test]
    fn parses_process_line() {
        let parsed = NvidiaGpu::parse_process_line("GPU-1234, 4242, 512").expect("must parse");
        assert_eq!(parsed.0, "GPU-1234");
        assert_eq!(parsed.1, 4242);
        assert_eq!(parsed.2, 512.0);
    }

    #[test]
    fn rejects_process_line_with_n_a_memory() {
        let parsed = NvidiaGpu::parse_process_line("GPU-1234, 4242, N/A");
        assert!(parsed.is_none());
    }
}
