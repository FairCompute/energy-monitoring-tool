use crate::energy_group::{EnergyCollector, EnergyRecord, UtilizationRecord};
use async_trait::async_trait;
use chrono::Utc;
use log::{info, warn, debug};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use sysinfo::System;

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

/// RAPL zone information
#[derive(Clone, Debug)]
struct RaplZone {
    path: PathBuf,
    name: String,
}

/// Main RAPL collector
pub struct Rapl {
    zones: Vec<RaplZone>,
    zone_readers: Vec<DeltaReader>,
    core_readers: Vec<DeltaReader>,
    dram_readers: Vec<DeltaReader>,
    igpu_readers: Vec<DeltaReader>,
}

impl Rapl {
    pub fn new(rapl_path: Option<String>) -> Self {
        let rapl_dir = rapl_path.unwrap_or_else(|| "/sys/class/powercap".to_string());
        let mut zones = Vec::new();

        // Discover RAPL zones
        if let Ok(entries) = fs::read_dir(&rapl_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                // Filter for intel-rapl zones (but not psys)
                if name.contains("intel-rapl") && !name.contains("psys") && name.contains(":") {
                    // Check if this is a main zone or subcomponent
                    let is_component = name.matches(":").count() > 1;
                    
                    if !is_component {
                        // This is a main zone
                        if let Ok(name_content) = fs::read_to_string(path.join("name")) {
                            let zone_name = name_content.trim().to_string();
                            zones.push(RaplZone {
                                path: path.clone(),
                                name: zone_name,
                            });
                        }
                    }
                }
            }
        }

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

        // Create zone readers
        let zone_readers = zones
            .iter()
            .map(|zone| DeltaReader::new(zone.path.clone(), 3))
            .collect();

        Self {
            zones,
            zone_readers,
            core_readers,
            dram_readers,
            igpu_readers,
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
    async fn get_energy_trace(&self) -> Result<Vec<EnergyRecord>, String> {
        let timestamp = Utc::now().timestamp_millis();
        let mut records = Vec::new();

        // Read energy from all zones
        for (i, reader) in self.zone_readers.iter().enumerate() {
            match reader.read_delta() {
                Ok(delta) => {
                    if let Some(zone) = self.zones.get(i) {
                        records.push(EnergyRecord {
                            pid: 0,
                            timestamp,
                            device: format!("rapl_zone:{}", zone.name),
                            energy: delta,
                        });
                    }
                }
                Err(e) => warn!("Failed to read zone energy: {}", e),
            }
        }

        // Read energy from cores
        let mut core_energy = 0.0;
        for reader in &self.core_readers {
            match reader.read_delta() {
                Ok(delta) => {
                    core_energy += delta;
                }
                Err(e) => warn!("Failed to read core energy: {}", e),
            }
        }

        if !self.core_readers.is_empty() && core_energy > 0.0 {
            records.push(EnergyRecord {
                pid: 0,
                timestamp,
                device: "rapl:cores".to_string(),
                energy: core_energy,
            });
        }

        // Read energy from DRAM
        let mut dram_energy = 0.0;
        for reader in &self.dram_readers {
            match reader.read_delta() {
                Ok(delta) => {
                    dram_energy += delta;
                }
                Err(e) => warn!("Failed to read DRAM energy: {}", e),
            }
        }

        if !self.dram_readers.is_empty() && dram_energy > 0.0 {
            records.push(EnergyRecord {
                pid: 0,
                timestamp,
                device: "rapl:dram".to_string(),
                energy: dram_energy,
            });
        }

        // Read energy from iGPU
        let mut igpu_energy = 0.0;
        for reader in &self.igpu_readers {
            match reader.read_delta() {
                Ok(delta) => {
                    igpu_energy += delta;
                }
                Err(e) => warn!("Failed to read iGPU energy: {}", e),
            }
        }

        if !self.igpu_readers.is_empty() && igpu_energy > 0.0 {
            records.push(EnergyRecord {
                pid: 0,
                timestamp,
                device: "rapl:igpu".to_string(),
                energy: igpu_energy,
            });
        }

        info!("RAPL energy trace collected: {} records", records.len());
        Ok(records)
    }

    async fn get_utilization_trace(&self) -> Result<Vec<UtilizationRecord>, String> {
        let timestamp = Utc::now().timestamp_millis();

        // Get system CPU utilization
        let mut system = System::new_all();
        system.refresh_all();

        let cpu_util = system.global_cpu_usage();
        
        // Get global memory info
        let total_memory = system.total_memory();
        let used_memory = system.used_memory();
        let memory_percent = if total_memory > 0 {
            (used_memory as f64 / total_memory as f64) * 100.0
        } else {
            0.0
        };

        // Return single utilization record with average of CPU and memory utilization
        let avg_utilization = (cpu_util as f64 + memory_percent) / 2.0;
        let record = UtilizationRecord {
            pid: 0,
            timestamp,
            device: "rapl".to_string(),
            utilization: avg_utilization,
        };

        info!("RAPL utilization trace collected: CPU={:.2}%, Memory={:.2}%", cpu_util, memory_percent);
        Ok(vec![record])
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
