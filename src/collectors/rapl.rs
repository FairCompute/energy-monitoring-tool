use crate::energy_group::{EnergyCollector, EnergyRecord, UtilizationRecord};
use async_trait::async_trait;
use chrono::Utc;
use log::{info, warn, debug};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use sysinfo::{System, Pid};

/// DeltaReader tracks energy deltas from RAPL MSR registers
/// It reads the energy_uj file and computes the delta from the previous reading
/// Must use Arc<Mutex<>> because EnergyCollector trait requires Sync bound for thread safety
struct DeltaReader {
    file_path: PathBuf,
    previous_value: Arc<Mutex<Option<i64>>>,
    num_trials: usize,
}

impl DeltaReader {
    fn new(file_path: PathBuf, num_trials: usize) -> Self {
        Self {
            file_path,
            previous_value: Arc::new(Mutex::new(None)),
            num_trials,
        }
    }

    /// Read energy delta in joules from RAPL counter
    /// Handles counter overflow by retrying multiple times
    fn read_delta(&self) -> Result<f64, String> {
        let energy_file = self.file_path.join("energy_uj");
        
        for attempt in 0..self.num_trials {
            let content = fs::read_to_string(&energy_file)
                .map_err(|e| format!("Failed to read energy file: {}", e))?;
            
            let value: i64 = content
                .trim()
                .parse()
                .map_err(|e| format!("Failed to parse energy value: {}", e))?;

            let mut prev = self.previous_value.lock().unwrap();
            
            // First read, just store the value
            if prev.is_none() {
                *prev = Some(value);
                return Ok(0.0);
            }

            let previous = prev.unwrap();
            let delta = value - previous;

            // Check if delta is positive (no overflow)
            if delta >= 0 {
                *prev = Some(value);
                // Convert from micro-joules to joules
                return Ok(delta as f64 * 1e-6);
            }

            // Counter overflow detected, retry
            if attempt < self.num_trials - 1 {
                debug!("RAPL counter overflow detected at {:?}, retrying...", &energy_file);
                continue;
            }

            // If all retries failed, log warning and return 0
            warn!("Energy counter overflow detected for: {:?} after {} attempts", &energy_file, self.num_trials);
            *prev = Some(value);
            return Ok(0.0);
        }

        Err("Failed to read valid energy delta after all attempts".to_string())
    }
}

/// Main RAPL collector with per-process energy attribution
pub struct Rapl {
    zone_readers: Vec<DeltaReader>,
    core_readers: Vec<DeltaReader>,
    dram_readers: Vec<DeltaReader>,
    igpu_readers: Vec<DeltaReader>,
    /// Tracked process PIDs for per-process energy attribution
    tracked_pids: Arc<Mutex<Vec<u32>>>,
}

impl Rapl {
    pub fn new(rapl_path: Option<String>) -> Self {
        let rapl_dir = rapl_path.unwrap_or_else(|| "/sys/class/powercap".to_string());

        // Discover components (cores, dram, igpu)
        let mut core_readers = Vec::new();
        let mut dram_readers = Vec::new();
        let mut igpu_readers = Vec::new();

        if let Ok(entries) = fs::read_dir(&rapl_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                // Find components (entries with 2 colons: intel-rapl:0:0)
                if name.contains("intel-rapl") && name.matches(":").count() == 2 {
                    if fs::metadata(path.join("energy_uj")).is_ok() {
                        if let Ok(name_content) = fs::read_to_string(path.join("name")) {
                            let component_name = name_content.trim().to_string();
                            
                            if component_name.contains("cores") || component_name.contains("cpu") {
                                core_readers.push(DeltaReader::new(path, 3));
                            } else if component_name.contains("dram") || component_name.contains("ram") {
                                dram_readers.push(DeltaReader::new(path, 3));
                            } else if component_name.contains("gpu") {
                                igpu_readers.push(DeltaReader::new(path, 3));
                            }
                        }
                    }
                }
            }
        }

        // Discover and create zone readers - find main RAPL zones
        let mut zone_readers = Vec::new();
        if let Ok(entries) = fs::read_dir(&rapl_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                // Filter for intel-rapl main zones (single colon, but not psys)
                if name.contains("intel-rapl") && !name.contains("psys") && name.matches(":").count() == 1 {
                    if fs::metadata(path.join("energy_uj")).is_ok() {
                        zone_readers.push(DeltaReader::new(path, 3));
                    }
                }
            }
        }

        Self {
            zone_readers,
            core_readers,
            dram_readers,
            igpu_readers,
            tracked_pids: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Default for Rapl {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait]
impl EnergyCollector for Rapl {
    fn set_tracked_pids(&mut self, pids: Vec<u32>) {
        self.tracked_pids = Arc::new(Mutex::new(pids));
    }

    async fn get_energy_trace(&self) -> Result<Vec<EnergyRecord>, String> {
        let timestamp = Utc::now().timestamp_millis();
        let mut records = Vec::new();

        // Read total system energy from zones
        let mut total_zone_energy = 0.0;
        for reader in &self.zone_readers {
            match reader.read_delta() {
                Ok(delta) => total_zone_energy += delta,
                Err(e) => warn!("Failed to read zone energy: {}", e),
            }
        }

        // Read energy from core components
        let mut total_core_energy = 0.0;
        for reader in &self.core_readers {
            match reader.read_delta() {
                Ok(delta) => total_core_energy += delta,
                Err(e) => warn!("Failed to read core energy: {}", e),
            }
        }

        // Read energy from DRAM components
        let mut total_dram_energy = 0.0;
        for reader in &self.dram_readers {
            match reader.read_delta() {
                Ok(delta) => total_dram_energy += delta,
                Err(e) => warn!("Failed to read DRAM energy: {}", e),
            }
        }

        // Read energy from iGPU components
        let mut total_igpu_energy = 0.0;
        for reader in &self.igpu_readers {
            match reader.read_delta() {
                Ok(delta) => total_igpu_energy += delta,
                Err(e) => warn!("Failed to read iGPU energy: {}", e),
            }
        }

        // Get tracked PIDs for per-process attribution
        let pids = self.tracked_pids.lock().unwrap().clone();
        
        if pids.is_empty() {
            // No tracked PIDs, return system-wide records
            if total_zone_energy > 0.0 {
                records.push(EnergyRecord {
                    pid: 0,
                    timestamp,
                    device: "rapl:zones".to_string(),
                    energy: total_zone_energy,
                });
            }
            if total_core_energy > 0.0 {
                records.push(EnergyRecord {
                    pid: 0,
                    timestamp,
                    device: "rapl:cores".to_string(),
                    energy: total_core_energy,
                });
            }
            if total_dram_energy > 0.0 {
                records.push(EnergyRecord {
                    pid: 0,
                    timestamp,
                    device: "rapl:dram".to_string(),
                    energy: total_dram_energy,
                });
            }
            if total_igpu_energy > 0.0 {
                records.push(EnergyRecord {
                    pid: 0,
                    timestamp,
                    device: "rapl:igpu".to_string(),
                    energy: total_igpu_energy,
                });
            }
            info!("RAPL energy trace collected (system-wide): {} records", records.len());
            return Ok(records);
        }

        // Per-process energy attribution based on CPU utilization
        let mut system = System::new_all();
        system.refresh_all();

        // Calculate per-process CPU utilization
        let mut _total_process_cpu = 0.0;
        let mut process_cpus: Vec<(u32, f64)> = Vec::new();

        for &pid in &pids {
            if let Some(process) = system.process(Pid::from(pid as usize)) {
                let cpu_usage = process.cpu_usage() as f64;
                _total_process_cpu += cpu_usage;
                process_cpus.push((pid, cpu_usage));
            } else {
                process_cpus.push((pid, 0.0));
            }
        }

        // Calculate per-process memory utilization
        let mut total_process_memory = 0.0;
        let mut process_memory: Vec<(u32, f64)> = Vec::new();

        for &pid in &pids {
            if let Some(process) = system.process(Pid::from(pid as usize)) {
                let memory_bytes = process.memory();
                let total_memory = system.total_memory();
                let memory_percent = if total_memory > 0 {
                    (memory_bytes as f64 / total_memory as f64) * 100.0
                } else {
                    0.0
                };
                total_process_memory += memory_percent;
                process_memory.push((pid, memory_percent));
            } else {
                process_memory.push((pid, 0.0));
            }
        }

        // Get global system CPU utilization
        let system_cpu = system.global_cpu_usage() as f64;
        let cpu_count = system.cpus().len().max(1) as f64;

        // Attribute energy to each tracked PID
        for &pid in &pids {
            let cpu_util = process_cpus
                .iter()
                .find(|(p, _)| *p == pid)
                .map(|(_, u)| u)
                .unwrap_or(&0.0);
            
            let memory_util = process_memory
                .iter()
                .find(|(p, _)| *p == pid)
                .map(|(_, u)| u)
                .unwrap_or(&0.0);

            // Calculate normalized CPU utilization: (process_cpu / cpu_count) / system_cpu
            let normalized_cpu = if system_cpu > 0.0 {
                (cpu_util / cpu_count) / system_cpu
            } else {
                0.0
            };

            // Calculate normalized memory utilization
            let normalized_memory = if total_process_memory > 0.0 {
                memory_util / total_process_memory
            } else {
                0.0
            };

            // Attribute CPU energy based on CPU utilization
            if total_core_energy > 0.0 && normalized_cpu > 0.0 {
                let cpu_energy = total_core_energy * normalized_cpu;
                records.push(EnergyRecord {
                    pid,
                    timestamp,
                    device: "rapl:cores".to_string(),
                    energy: cpu_energy,
                });
            }

            // Attribute DRAM energy based on memory utilization
            if total_dram_energy > 0.0 && normalized_memory > 0.0 {
                let dram_energy = total_dram_energy * normalized_memory;
                records.push(EnergyRecord {
                    pid,
                    timestamp,
                    device: "rapl:dram".to_string(),
                    energy: dram_energy,
                });
            }
        }

        info!("RAPL energy trace collected: {} records for {} processes", 
              records.len(), pids.len());
        Ok(records)
    }

    async fn get_utilization_trace(&self) -> Result<Vec<UtilizationRecord>, String> {
        let timestamp = Utc::now().timestamp_millis();
        let mut records = Vec::new();

        // Get tracked PIDs for per-process attribution
        let pids = self.tracked_pids.lock().unwrap().clone();

        // Get system CPU and memory information
        let mut system = System::new_all();
        system.refresh_all();

        let system_cpu = system.global_cpu_usage() as f64;
        let cpu_count = system.cpus().len().max(1) as f64;
        let total_memory = system.total_memory();

        if pids.is_empty() {
            // No tracked PIDs, return system-wide utilization
            let memory_percent = if total_memory > 0 {
                (system.used_memory() as f64 / total_memory as f64) * 100.0
            } else {
                0.0
            };

            let record = UtilizationRecord {
                pid: 0,
                timestamp,
                device: "rapl".to_string(),
                utilization: system_cpu,
            };
            records.push(record);

            info!("RAPL utilization trace collected (system-wide): CPU={:.2}%, Memory={:.2}%", 
                  system_cpu, memory_percent);
            return Ok(records);
        }

        // Per-process utilization attribution
        // Calculate normalized utilization for each process
        for &pid in &pids {
            let cpu_usage = if let Some(process) = system.process(Pid::from(pid as usize)) {
                process.cpu_usage() as f64
            } else {
                0.0
            };

            // Normalize CPU utilization: (process_cpu / cpu_count) / system_cpu
            let normalized_cpu = if system_cpu > 0.0 {
                (cpu_usage / cpu_count) / system_cpu
            } else {
                0.0
            };

            // Convert to percentage (normalized_cpu is already a ratio, multiply by 100 for percentage)
            let utilization_percent = normalized_cpu * 100.0;

            records.push(UtilizationRecord {
                pid,
                timestamp,
                device: "rapl".to_string(),
                utilization: utilization_percent,
            });
        }

        let memory_percent = if total_memory > 0 {
            (system.used_memory() as f64 / total_memory as f64) * 100.0
        } else {
            0.0
        };

        info!("RAPL utilization trace collected: {} records, system CPU={:.2}%, process memory={:.2}%", 
              records.len(), system_cpu, memory_percent);
        Ok(records)
    }

    fn is_available() -> bool {
        Path::new("/sys/class/powercap").exists()
            && fs::read_dir("/sys/class/powercap")
                .ok()
                .and_then(|entries| {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.contains("intel-rapl") {
                                return Some(true);
                            }
                        }
                    }
                    Some(false)
                })
                .unwrap_or(false)
    }
}
