use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use log::{info, Logger};
use once_cell::sync::OnceCell;
use psutil::process::{Process, ProcessResult};

pub struct PowerGroup {
    count_trace_calls: usize,
    processes: Vec<Process>,
    consumed_energy: Vec<f64>,
    rate: f64,
    logger: Logger,
    energy_trace:  HashMap<u64, Vec<f64>> ,
    tracked_process: Option<Process>,
}

impl PowerGroup {
    pub fn new(rate: f64, pids: Option<Vec<i32>>) -> ProcessResult<Self> {
        let processes = match pids {
            Some(pids) => {
                let mut vec = Vec::new();
                for pid in pids {
                    vec.push(Process::new(pid)?);
                }
                vec
            }
            None => {
                let mut vec = Vec::new();
                for proc in psutil::process::all()? {
                    vec.push(proc);
                }
                vec
            }
        };

        # Initialize the consumed_energy vector with zeros
        let consumed_energy = vec![0.0; processes.len()];

        Ok(Self {
            count_trace_calls: 0,
            processes,
            consumed_energy,
            rate,
            logger: log::logger().clone(),
            energy_trace: HashMap::new(),
            tracked_process: None,
        })
    }

    pub fn sleep_interval(&self) -> f64 {
        1.0 / self.rate
    }

    pub fn processes(&self) -> &Vec<Process> {
        &self.processes
    }

    pub fn tracked_process(&self) -> &Process {
        self.tracked_process.as_ref().unwrap_or_else(|| {
            // Return the first process as a fallback
            &self.processes[0]
        })
    }

    pub fn consumed_energy(&self) -> &Vec<f64> {
        &self.consumed_energy
    }

    pub fn energy_trace(&mut self) -> HashMap<u64, Vec<f64>> {
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



