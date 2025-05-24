use crate::power_groups::errors::TrackerError;
use async_trait::async_trait;
use std::collections::HashMap;
use sysinfo::{Pid, Process, System};

pub struct PowerGroupTracker {
    rate: f64,
    count_trace_calls: usize,
    // Grouped by application name, each with a vector of PIDs
    tracked_processes: HashMap<String, Vec<usize>>,
    consumed_energy: Vec<f64>,
    energy_trace: HashMap<u64, Vec<f64>>,
}

impl PowerGroupTracker {
    pub fn new(rate: f64, provided_pids: Option<Vec<usize>>) -> Result<Self, TrackerError> {
        let system = System::new_all();
        let mut tracked_processes: HashMap<String, Vec<usize>> = HashMap::new();
        match provided_pids {
            Some(pids) if !pids.is_empty() => {
                for pid in pids {
                    if let Some(process) = system.process(Pid::from(pid)) {
                        let name = process.name().to_string_lossy().to_string();
                        let key = if name.is_empty() { "unknown".to_string() } else { name };
                        tracked_processes.entry(key).or_default().push(pid);
                    } else {
                        tracked_processes.entry("unknown".to_string()).or_default().push(pid);
                    }
                }
            }
            _ => {
                for (pid, process) in system.processes() {
                    let name = process.name().to_string_lossy().to_string();
                    let key = if name.is_empty() { "unknown".to_string() } else { name };
                    tracked_processes.entry(key).or_default().push(pid.as_u32() as usize);
                }
            }
        }
        let total_pids = tracked_processes.values().map(|v| v.len()).sum();
        let consumed_energy = vec![0.0; total_pids];
        Ok(Self {
            rate,
            count_trace_calls: 0,
            tracked_processes,
            energy_trace: HashMap::new(),
            consumed_energy,
        })
    }

    pub fn sleep_interval(&self) -> f64 {
        1.0 / self.rate
    }
    pub fn processes(&self) -> &HashMap<String, Vec<usize>> {
        &self.tracked_processes
    }
    pub fn consumed_energy(&self) -> &Vec<f64> {
        &self.consumed_energy
    }
    pub fn energy_trace(&self) -> HashMap<u64, Vec<f64>> {
        self.energy_trace.clone()
    }
}

#[async_trait]
pub trait AsyncEnergyCollector {
    fn get_trace(&self) -> Result<HashMap<u64, Vec<f64>>, String>;
    fn is_available() -> bool {
        unimplemented!()
    }
    async fn commence(&mut self) -> Result<(), String>;
    async fn shutdown(&mut self) -> Result<(), String>;
}
