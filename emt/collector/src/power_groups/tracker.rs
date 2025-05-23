use crate::power_groups::errors::TrackerError;
use async_trait::async_trait;
use std::collections::HashMap;
use sysinfo::{Pid, Process, System};

pub struct PowerGroupTracker {
    rate: f64,
    count_trace_calls: usize,
    tracked_processes: Vec<usize>,
    consumed_energy: Vec<f64>,
    energy_trace: HashMap<u64, Vec<f64>>,
}

impl PowerGroupTracker {
    pub fn new(rate: f64, provided_pids: Option<Vec<usize>>) -> Result<Self, TrackerError> {
        let system = System::new_all();

        let tracked_processes: Vec<usize> = match provided_pids {
            Some(pids) if !pids.is_empty() => pids
                .into_iter()
                .filter(|pid| system.process(Pid::from(*pid)).is_some())
                .collect(),
            _ => system
                .processes()
                .keys()
                .map(|pid| pid.as_u32() as usize)
                .collect(),
        };

        let consumed_energy = vec![0.0; tracked_processes.len()];

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
    pub fn processes(&self) -> &Vec<usize> {
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
