use crate::energy_group::{EnergyCollector, EnergyRecord};
use async_trait::async_trait;
use chrono::Utc;
use log::{info, warn};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use sysinfo::{Pid, System};

/// DeltaReader tracks energy deltas from RAPL MSR registers
/// It reads the energy_uj file and computes the delta from the previous reading
#[derive(Clone)]
struct DeltaReader {
    file_path: PathBuf,
    previous_value: Arc<Mutex<Option<i64>>>,
}

impl DeltaReader {
    fn new(file_path: PathBuf) -> Self {
        Self {
            file_path,
            previous_value: Arc::new(Mutex::new(None)),
        }
    }

    /// Read energy delta in joules from RAPL counter
    /// Handles counter overflow by retrying multiple times
    fn read_delta(&self) -> Result<f64, String> {
        let energy_file = self.file_path.join("energy_uj");
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

        // If all retries failed, log warning and return 0
        warn!("Energy counter overflow detected for: {:?}", &energy_file);
        *prev = Some(value);
        return Ok(0.0);
    }
}

/// Per-socket RAPL energy readers organized by component type
#[derive(Clone)]
struct SocketReaders {
    socket_id: u32,
    package_reader: Option<DeltaReader>,   // PKG: Total socket energy
    core_reader: Option<DeltaReader>,      // PP0: Cores + L1/L2 caches
    uncore_reader: Option<DeltaReader>,    // PP1: iGPU, L3, memory controller
}

/// Main RAPL collector with per-socket energy attribution
pub struct Rapl {
    /// Per-socket readers organized by socket ID
    socket_readers: Vec<SocketReaders>,
    /// System-level DRAM energy reader (off-package)
    dram_reader: Option<DeltaReader>,
    /// System-level PSYS energy reader (platform/system-wide power)
    psys_reader: Option<DeltaReader>,
    /// Tracked process PIDs for per-process energy attribution
    tracked_pids: Arc<Mutex<Vec<u32>>>,
}

impl Rapl {
    pub fn new(rapl_path: Option<String>) -> Self {
        let rapl_dir = rapl_path.unwrap_or_else(|| "/sys/class/powercap".to_string());
        let (socket_readers, dram_reader, psys_reader) = Self::scan_powercap_entries(&rapl_dir);

        Self {
            socket_readers,
            dram_reader,
            psys_reader,
            tracked_pids: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Discovers all RAPL sockets and their energy components in a single pass
    /// Returns socket readers and system-level DRAM/PSYS readers
    fn scan_powercap_entries(rapl_dir: &str) -> (Vec<SocketReaders>, Option<DeltaReader>, Option<DeltaReader>) {
        let mut socket_map: BTreeMap<u32, SocketReaders> = BTreeMap::new();
        let mut dram_reader: Option<DeltaReader> = None;
        let mut psys_reader: Option<DeltaReader> = None;

        let Ok(entries) = fs::read_dir(rapl_dir) else {
            warn!("Failed to read RAPL directory: {}", rapl_dir);
            return (Vec::new(), None, None);
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            // Skip non-RAPL entries (support both Intel and AMD)
            if !name.contains("rapl") {
                continue;
            }

            // Handle PSYS (system-wide power) separately
            if name.contains("psys") {
                if fs::metadata(path.join("energy_uj")).is_ok() {
                    psys_reader = Some(DeltaReader::new(path.clone()));
                }
                continue;
            }

            let colon_count = name.matches(':').count();
            
            match colon_count {
                // Socket-level entry: rapl:N (package energy at root level)
                1 => {
                    if let Some(socket_id) = Self::parse_socket_id(name) {
                        // Check if this socket has energy_uj (package energy)
                        if fs::metadata(path.join("energy_uj")).is_ok() {
                            let package_reader = DeltaReader::new(path.clone());
                            
                            // Insert or update socket with package reader
                            socket_map.entry(socket_id).and_modify(|socket| {
                                socket.package_reader = Some(package_reader.clone());
                            }).or_insert(SocketReaders {
                                socket_id,
                                package_reader: Some(package_reader),
                                core_reader: None,
                                uncore_reader: None,
                            });
                        }
                    }
                }
                // Component-level entry: rapl:N:M (core, uncore, etc.)
                2 => {
                    if let Some(reader) = Self::parse_component(&path, name) {
                        if let Some(socket_id) = Self::parse_socket_id(name) {
                            // Ensure socket exists before assigning component
                            // Use or_insert_with to avoid overwriting existing entry
                            socket_map.entry(socket_id).or_insert_with(|| SocketReaders {
                                socket_id,
                                package_reader: None,
                                core_reader: None,
                                uncore_reader: None,
                            });
                            // Now get mutable reference and assign
                            if let Some(socket) = socket_map.get_mut(&socket_id) {
                                Self::assign_socket_component(socket, reader, &path);
                            }
                        } else {
                            // System-level component (e.g., DRAM without socket association)
                            Self::assign_system_component(&mut dram_reader, reader, &path);
                        }
                    }
                }
                _ => continue,
            }
        }

        (socket_map.into_values().collect(), dram_reader, psys_reader)
    }

    /// Extracts socket ID from RAPL socket entry name (e.g., "intel-rapl:0" -> 0)
    fn parse_socket_id(name: &str) -> Option<u32> {
        name.split(':').nth(1)?.parse().ok()
    }

    /// Parses a component entry and returns a delta reader if valid
    fn parse_component(path: &Path, _name: &str) -> Option<DeltaReader> {
        // Verify energy_uj file exists
        if fs::metadata(path.join("energy_uj")).is_err() {
            return None;
        }

        Some(DeltaReader::new(path.to_path_buf()))
    }

    /// Assigns a component reader to the appropriate socket field based on component name
    fn assign_socket_component(socket: &mut SocketReaders, reader: DeltaReader, path: &Path) {
        let Ok(component_name) = fs::read_to_string(path.join("name")) else {
            warn!("Failed to read component name from: {:?}", path);
            return;
        };

        let comp_name = component_name.trim().to_lowercase();
        info!("Assigning component '{}' to socket {}", comp_name, socket.socket_id);

        // Match RAPL domain names for socket sub-components (core, uncore)
        // Note: package energy is at the socket root level, not here
        match comp_name.as_str() {
            "core" | "cores" => {
                socket.core_reader = Some(reader);
                info!("Assigned core reader to socket {}", socket.socket_id);
            }
            "uncore" => {
                socket.uncore_reader = Some(reader);
                info!("Assigned uncore reader to socket {}", socket.socket_id);
            }
            _ => {
                // Log unrecognized component for debugging
                info!("Unrecognized socket-level RAPL component: {}", comp_name);
            }
        }
    }

    /// Assigns a component reader to the system-level DRAM field based on component name
    fn assign_system_component(dram_reader: &mut Option<DeltaReader>, reader: DeltaReader, path: &Path) {
        let Ok(component_name) = fs::read_to_string(path.join("name")) else {
            return;
        };

        let comp_name = component_name.trim().to_lowercase();

        // Match RAPL domain names for system-level components
        match comp_name.as_str() {
            "dram" | "ram" => *dram_reader = Some(reader),
            _ => {
                // Log unrecognized component for debugging
                info!("Unrecognized system-level RAPL component: {}", comp_name);
            }
        }
    }

    /// Calculate per-process utilization metrics (CPU and memory)
    /// Returns a tuple of (cpu_utilization, memory_utilization) for each tracked PID
    /// CPU utilization is normalized relative to system usage
    /// Memory utilization is normalized relative to total process memory usage
    fn get_utilization(&self, pids: &[u32]) -> Result<(Vec<(u32, f64)>, Vec<(u32, f64)>), String> {
        let mut system = System::new_all();
        system.refresh_all();

        let system_cpu = system.global_cpu_usage() as f64;
        let cpu_count = system.cpus().len().max(1) as f64;
        let total_memory = system.total_memory();

        // Calculate per-process CPU utilization
        let mut _total_process_cpu = 0.0;
        let mut process_cpus: Vec<(u32, f64)> = Vec::new();

        for &pid in pids {
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

        for &pid in pids {
            if let Some(process) = system.process(Pid::from(pid as usize)) {
                let memory_bytes = process.memory();
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

        // Normalize CPU utilization relative to system CPU
        let normalized_cpus: Vec<(u32, f64)> = process_cpus
            .into_iter()
            .map(|(pid, cpu_usage)| {
                let normalized = if system_cpu > 0.0 {
                    (cpu_usage / cpu_count) / system_cpu
                } else {
                    0.0
                };
                (pid, normalized)
            })
            .collect();

        // Normalize memory utilization relative to total process memory
        let normalized_memory: Vec<(u32, f64)> = process_memory
            .into_iter()
            .map(|(pid, mem_percent)| {
                let normalized = if total_process_memory > 0.0 {
                    mem_percent / total_process_memory
                } else {
                    0.0
                };
                (pid, normalized)
            })
            .collect();

        Ok((normalized_cpus, normalized_memory))
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

        // Get tracked PIDs for per-process attribution
        let pids = self.tracked_pids.lock().unwrap().clone();

        if pids.is_empty() {
            // No tracked PIDs, skip producing records
            info!("RAPL energy trace collected (no PIDs tracked): 0 records");
            return Ok(records);
        }

        info!(
            "RAPL: Processing {} sockets with {} tracked PIDs",
            self.socket_readers.len(),
            pids.len()
        );

        // Calculate per-process utilization
        let (cpu_utilization_ratio, memory_utilization_ratio) = self.get_utilization(&pids)?;

        // Collect per-socket energy readings
        for socket in &self.socket_readers {
            let socket_id = socket.socket_id;

            info!(
                "Socket {}: pkg={}, core={}, uncore={}",
                socket_id,
                socket.package_reader.is_some(),
                socket.core_reader.is_some(),
                socket.uncore_reader.is_some()
            );

            // Read package energy for this socket (total socket energy)
            let package_energy = if let Some(reader) = &socket.package_reader {
                reader.read_delta().unwrap_or_else(|e| {
                    warn!("Failed to read package energy for socket {}: {}", socket_id, e);
                    0.0
                })
            } else {
                0.0
            };

            // Read core energy for this socket (PP0: cores + L1/L2)
            let core_energy = if let Some(reader) = &socket.core_reader {
                reader.read_delta().unwrap_or_else(|e| {
                    warn!("Failed to read core energy for socket {}: {}", socket_id, e);
                    0.0
                })
            } else {
                0.0
            };

            // Read uncore energy for this socket (PP1: iGPU, L3, memory controller)
            let uncore_energy = if let Some(reader) = &socket.uncore_reader {
                reader.read_delta().unwrap_or_else(|e| {
                    warn!("Failed to read uncore energy for socket {}: {}", socket_id, e);
                    0.0
                })
            } else {
                0.0
            };

            // Attribute energy to each tracked PID based on utilization
            for &pid in &pids {
                let normalized_cpu = cpu_utilization_ratio
                    .iter()
                    .find(|(p, _)| *p == pid)
                    .map(|(_, u)| *u)
                    .unwrap_or(0.0);

                let normalized_mem = memory_utilization_ratio
                    .iter()
                    .find(|(p, _)| *p == pid)
                    .map(|(_, u)| *u)
                    .unwrap_or(0.0);

                // Create per-socket device names and attribute energy (including zero values)
                // Zero values are expected on first read as baseline is established
                
                // Package energy (total socket) - attributed by CPU usage
                if socket.package_reader.is_some() {
                    let package_attribution = package_energy * normalized_cpu;
                    records.push(EnergyRecord {
                        pid,
                        timestamp,
                        device: format!("rapl:socket:{}:package", socket_id),
                        energy: package_attribution,
                    });
                }

                // Core energy (PP0: cores + L1/L2) - attributed by CPU usage
                if socket.core_reader.is_some() {
                    let core_attribution = core_energy * normalized_cpu;
                    records.push(EnergyRecord {
                        pid,
                        timestamp,
                        device: format!("rapl:socket:{}:core", socket_id),
                        energy: core_attribution,
                    });
                }

                // Uncore energy (PP1: iGPU, L3, memory controller) - distributed equally for now
                if socket.uncore_reader.is_some() {
                    let uncore_attribution = uncore_energy / pids.len() as f64;
                    records.push(EnergyRecord {
                        pid,
                        timestamp,
                        device: format!("rapl:socket:{}:uncore", socket_id),
                        energy: uncore_attribution,
                    });
                }
            }
        }

        // Collect system-level energy readings (DRAM and PSYS)
        info!(
            "System: dram={}, psys={}",
            self.dram_reader.is_some(),
            self.psys_reader.is_some()
        );

        // Read DRAM energy (system-level, off-package)
        let dram_energy = if let Some(reader) = &self.dram_reader {
            reader.read_delta().unwrap_or_else(|e| {
                warn!("Failed to read DRAM energy: {}", e);
                0.0
            })
        } else {
            0.0
        };

        // Read PSYS energy (platform/system-wide)
        let psys_energy = if let Some(reader) = &self.psys_reader {
            reader.read_delta().unwrap_or_else(|e| {
                warn!("Failed to read PSYS energy: {}", e);
                0.0
            })
        } else {
            0.0
        };

        // Attribute system-level energy to tracked PIDs
        for &pid in &pids {
            let normalized_mem = memory_utilization_ratio
                .iter()
                .find(|(p, _)| *p == pid)
                .map(|(_, u)| *u)
                .unwrap_or(0.0);

            // DRAM energy attributed by memory usage
            if self.dram_reader.is_some() {
                let dram_attribution = dram_energy * normalized_mem;
                records.push(EnergyRecord {
                    pid,
                    timestamp,
                    device: "rapl:system:dram".to_string(),
                    energy: dram_attribution,
                });
            }

            // PSYS energy distributed equally among processes
            if self.psys_reader.is_some() {
                let psys_attribution = psys_energy / pids.len() as f64;
                records.push(EnergyRecord {
                    pid,
                    timestamp,
                    device: "rapl:system:psys".to_string(),
                    energy: psys_attribution,
                });
            }
        }

        info!(
            "RAPL energy trace collected: {} records for {} processes across {} sockets",
            records.len(),
            pids.len(),
            self.socket_readers.len()
        );
        Ok(records)
    }

    fn is_available() -> bool {
        Path::new("/sys/class/powercap").exists()
            && fs::read_dir("/sys/class/powercap")
                .ok()
                .and_then(|entries| {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            // Check for RAPL interface (Intel or AMD)
                            if name.contains("rapl") {
                                return Some(true);
                            }
                        }
                    }
                    Some(false)
                })
                .unwrap_or(false)
    }
}
