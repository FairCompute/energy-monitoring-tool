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

/// Tracks CPU times for a process to calculate CPU percentage accurately
/// Similar to how psutil tracks cpu_percent internally
#[derive(Clone, Default)]
struct ProcessCpuTracker {
    /// Last recorded user+system time in clock ticks
    last_cpu_time: u64,
    /// Last recorded timestamp in microseconds
    last_timestamp_us: u64,
}

impl ProcessCpuTracker {
    /// Read CPU time from /proc/<pid>/stat and calculate percentage since last call
    /// Returns (cpu_percent, is_valid) - is_valid is false if this is the first call
    fn update(&mut self, pid: u32) -> (f64, bool) {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        
        // Read /proc/<pid>/stat
        let stat_path = format!("/proc/{}/stat", pid);
        let Ok(stat_content) = fs::read_to_string(&stat_path) else {
            return (0.0, false);
        };
        
        // Parse utime and stime from stat file
        // Format: pid (comm) state ppid ... utime stime ...
        // Fields are space-separated, but comm can contain spaces, so find closing )
        let Some(comm_end) = stat_content.rfind(')') else {
            return (0.0, false);
        };
        let fields: Vec<&str> = stat_content[comm_end + 2..].split_whitespace().collect();
        if fields.len() < 13 {
            return (0.0, false);
        }
        
        // utime is field 11 (index 11 after the closing parenthesis)
        // stime is field 12
        let utime: u64 = fields[11].parse().unwrap_or(0);
        let stime: u64 = fields[12].parse().unwrap_or(0);
        let cpu_time = utime + stime;
        
        // Calculate CPU percentage
        let cpu_percent = if self.last_timestamp_us > 0 && now_us > self.last_timestamp_us {
            let time_delta_us = now_us - self.last_timestamp_us;
            let cpu_delta = cpu_time.saturating_sub(self.last_cpu_time);
            
            // Convert clock ticks to microseconds (USER_HZ is typically 100)
            // cpu_delta is in clock ticks, time_delta_us is in microseconds
            // cpu% = (cpu_delta_ticks / USER_HZ) / (time_delta_us / 1_000_000) * 100
            //      = (cpu_delta_ticks * 1_000_000 / USER_HZ) / time_delta_us * 100
            let user_hz = 100.0; // Standard Linux USER_HZ
            let cpu_time_us = (cpu_delta as f64 / user_hz) * 1_000_000.0;
            let percent = (cpu_time_us / time_delta_us as f64) * 100.0;
            percent
        } else {
            0.0
        };
        
        let is_first_call = self.last_timestamp_us == 0;
        self.last_cpu_time = cpu_time;
        self.last_timestamp_us = now_us;
        
        (cpu_percent, !is_first_call)
    }
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
    /// Persistent System instance for memory tracking
    system: Mutex<System>,
    /// Per-process CPU time trackers for accurate CPU percentage
    cpu_trackers: Mutex<std::collections::HashMap<u32, ProcessCpuTracker>>,
    /// System-wide CPU tracker
    system_cpu_tracker: Mutex<SystemCpuTracker>,
}

/// Tracks system-wide CPU times
#[derive(Clone, Default)]
struct SystemCpuTracker {
    last_total: u64,
    last_idle: u64,
    last_timestamp_us: u64,
}

impl SystemCpuTracker {
    /// Read system CPU usage from /proc/stat
    fn update(&mut self) -> (f64, bool) {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        
        let Ok(stat_content) = fs::read_to_string("/proc/stat") else {
            return (0.0, false);
        };
        
        // First line is "cpu  user nice system idle iowait irq softirq ..."
        let Some(cpu_line) = stat_content.lines().next() else {
            return (0.0, false);
        };
        
        let fields: Vec<&str> = cpu_line.split_whitespace().collect();
        if fields.len() < 5 || fields[0] != "cpu" {
            return (0.0, false);
        }
        
        let user: u64 = fields[1].parse().unwrap_or(0);
        let nice: u64 = fields[2].parse().unwrap_or(0);
        let system: u64 = fields[3].parse().unwrap_or(0);
        let idle: u64 = fields[4].parse().unwrap_or(0);
        let iowait: u64 = fields.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
        let irq: u64 = fields.get(6).and_then(|s| s.parse().ok()).unwrap_or(0);
        let softirq: u64 = fields.get(7).and_then(|s| s.parse().ok()).unwrap_or(0);
        
        let total = user + nice + system + idle + iowait + irq + softirq;
        
        let cpu_percent = if self.last_timestamp_us > 0 && total > self.last_total {
            let total_delta = total - self.last_total;
            let idle_delta = idle - self.last_idle;
            let active_delta = total_delta.saturating_sub(idle_delta);
            (active_delta as f64 / total_delta as f64) * 100.0
        } else {
            0.0
        };
        
        let is_first_call = self.last_timestamp_us == 0;
        self.last_total = total;
        self.last_idle = idle;
        self.last_timestamp_us = now_us;
        
        (cpu_percent, !is_first_call)
    }
}

impl Rapl {
    pub fn new(rapl_path: Option<String>) -> Self {
        let rapl_dir = rapl_path.unwrap_or_else(|| "/sys/class/powercap".to_string());
        let (socket_readers, dram_reader, psys_reader) = Self::scan_powercap_entries(&rapl_dir);

        // Initialize System for memory tracking (CPU tracking now uses /proc/stat directly)
        let system = System::new_all();
        
        // Initialize CPU trackers with a warmup call
        let mut system_cpu_tracker = SystemCpuTracker::default();
        system_cpu_tracker.update(); // First call establishes baseline

        Self {
            socket_readers,
            dram_reader,
            psys_reader,
            tracked_pids: Arc::new(Mutex::new(Vec::new())),
            system: Mutex::new(system),
            cpu_trackers: Mutex::new(std::collections::HashMap::new()),
            system_cpu_tracker: Mutex::new(system_cpu_tracker),
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
        log::debug!("Assigning component '{}' to socket {}", comp_name, socket.socket_id);

        // Match RAPL domain names for socket sub-components (core, uncore)
        // Note: package energy is at the socket root level, not here
        match comp_name.as_str() {
            "core" | "cores" => {
                socket.core_reader = Some(reader);
                log::debug!("Assigned core reader to socket {}", socket.socket_id);
            }
            "uncore" => {
                socket.uncore_reader = Some(reader);
                log::debug!("Assigned uncore reader to socket {}", socket.socket_id);
            }
            _ => {
                // Log unrecognized component for debugging
                log::debug!("Unrecognized socket-level RAPL component: {}", comp_name);
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
                log::debug!("Unrecognized system-level RAPL component: {}", comp_name);
            }
        }
    }

    /// Calculate per-process utilization metrics (CPU and memory)
    /// Returns a tuple of (cpu_utilization, memory_utilization) for each tracked PID
    /// CPU utilization is normalized relative to system usage (matching Python EMT formula)
    /// Memory utilization is normalized relative to total process memory usage
    fn get_utilization(&self, pids: &[u32]) -> Result<(Vec<(u32, f64)>, Vec<(u32, f64)>), String> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
        
        // Get system CPU using our custom tracker (reads from /proc/stat)
        let (system_cpu, sys_valid) = {
            let mut tracker = self.system_cpu_tracker.lock()
                .map_err(|e| format!("Failed to lock system CPU tracker: {}", e))?;
            tracker.update()
        };
        
        // Get per-process CPU using custom trackers (reads from /proc/<pid>/stat)
        let mut process_cpus: Vec<(u32, f64)> = Vec::new();
        {
            let mut trackers = self.cpu_trackers.lock()
                .map_err(|e| format!("Failed to lock CPU trackers: {}", e))?;
            
            for &pid in pids {
                let tracker = trackers.entry(pid).or_insert_with(ProcessCpuTracker::default);
                let (cpu_percent, is_valid) = tracker.update(pid);
                // Only use valid readings (not the first call which establishes baseline)
                let effective_cpu = if is_valid { cpu_percent } else { 0.0 };
                log::trace!("PID {} CPU: {:.2}% (valid: {})", pid, effective_cpu, is_valid);
                process_cpus.push((pid, effective_cpu));
            }
            
            // Clean up trackers for PIDs no longer being tracked
            let tracked_set: std::collections::HashSet<u32> = pids.iter().cloned().collect();
            trackers.retain(|pid, _| tracked_set.contains(pid));
        }
        
        log::trace!(
            "System CPU: {:.2}% (valid: {}), tracking {} processes",
            system_cpu, sys_valid, pids.len()
        );
        
        // Use sysinfo only for memory tracking
        let mut system = self.system.lock()
            .map_err(|e| format!("Failed to lock system: {}", e))?;
        
        // Refresh memory info for tracked processes
        let pids_to_update: Vec<Pid> = pids.iter().map(|&p| Pid::from(p as usize)).collect();
        let refresh_kind = ProcessRefreshKind::nothing().with_memory();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids_to_update),
            true,
            refresh_kind,
        );
        
        let cpu_count = system.cpus().len().max(1) as f64;
        let total_memory = system.total_memory();

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
        // Formula (matching Python EMT):
        //   ps_util = process_cpu_percent / cpu_count  (normalize to 0-100% range)
        //   norm_ps_util = ps_util / system_cpu_percent
        // 
        // This gives the fraction of system energy attributable to the process.
        let normalized_cpus: Vec<(u32, f64)> = process_cpus
            .into_iter()
            .map(|(pid, cpu_usage)| {
                // First normalize process CPU to 0-100% range (divide by cpu_count)
                // Then divide by system CPU to get the attribution fraction
                let ps_util = cpu_usage / cpu_count;
                let normalized = if system_cpu > 0.0 {
                    // Cap at 1.0 to prevent over-attribution due to timing differences
                    (ps_util / system_cpu).min(1.0)
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
            log::debug!("RAPL energy trace collected (no PIDs tracked): 0 records");
            return Ok(records);
        }

        log::debug!(
            "RAPL: Processing {} sockets with {} tracked PIDs",
            self.socket_readers.len(),
            pids.len()
        );

        // Calculate per-process utilization
        let (cpu_utilization_ratio, memory_utilization_ratio) = self.get_utilization(&pids)?;

        // Collect per-socket energy readings
        for socket in &self.socket_readers {
            let socket_id = socket.socket_id;

            log::debug!(
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
            // Currently unused but read for debugging purposes
            let _core_energy = if let Some(reader) = &socket.core_reader {
                reader.read_delta().unwrap_or_else(|e| {
                    warn!("Failed to read core energy for socket {}: {}", socket_id, e);
                    0.0
                })
            } else {
                0.0
            };

            // Read uncore energy for this socket (PP1: iGPU, L3, memory controller)
            // Currently unused but read for debugging purposes
            let _uncore_energy = if let Some(reader) = &socket.uncore_reader {
                reader.read_delta().unwrap_or_else(|e| {
                    warn!("Failed to read uncore energy for socket {}: {}", socket_id, e);
                    0.0
                })
            } else {
                0.0
            };

            // Attribute energy to each tracked PID based on utilization
            // NOTE: Package energy is the total socket energy and already includes core energy.
            // We only attribute package energy to avoid double counting.
            // Core and uncore are recorded separately for detailed breakdown but not summed into total.
            for &pid in &pids {
                let normalized_cpu = cpu_utilization_ratio
                    .iter()
                    .find(|(p, _)| *p == pid)
                    .map(|(_, u)| *u)
                    .unwrap_or(0.0);

                // Package energy (total socket) - this is the main energy attribution
                // Package = Core + Uncore, so we only count package to avoid double counting
                if socket.package_reader.is_some() {
                    let package_attribution = package_energy * normalized_cpu;
                    log::trace!(
                        "PID {} socket {}: package_energy={:.4}J × normalized_cpu={:.4} = {:.4}J",
                        pid, socket_id, package_energy, normalized_cpu, package_attribution
                    );
                    records.push(EnergyRecord {
                        pid,
                        timestamp,
                        device: format!("rapl:socket:{}:package", socket_id),
                        energy: package_attribution,
                    });
                }
            }
        }

        // Collect system-level energy readings (DRAM and PSYS)
        log::debug!(
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

        log::debug!(
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

#[cfg(test)]
mod tests {
    use super::DeltaReader;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_temp_energy_dir(initial_value: i64) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("emt_rapl_delta_reader_test_{}", unique));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("energy_uj"), initial_value.to_string()).unwrap();
        dir
    }

    #[test]
    fn delta_reader_handles_wrap_around() {
        let temp_dir = create_temp_energy_dir(3_000_000);
        let reader = DeltaReader::new(temp_dir.clone());

        {
            let mut previous = reader.previous_value.lock().unwrap();
            *previous = Some(4_000_000);
        }

        let delta = reader.read_delta().unwrap();
        assert_eq!(delta, 0.0);

        let previous = reader.previous_value.lock().unwrap();
        assert_eq!(*previous, Some(3_000_000));
        drop(previous);

        fs::write(temp_dir.join("energy_uj"), "8000000").unwrap();
        let recovered_delta = reader.read_delta().unwrap();
        assert_eq!(recovered_delta, 5.0);

        fs::remove_dir_all(temp_dir).unwrap();
    }
}
