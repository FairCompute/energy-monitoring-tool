use std::collections::HashMap;
use async_trait::async_trait;
use log::info;
use crate::energy_monitor::AsyncEnergyCollector;
use crate::utils::gather_process_groups;

pub struct DummyEnergyGroup;

impl DummyEnergyGroup {
    pub fn new(rate: f64, provided_pids: Option<Vec<usize>>) -> Result<Self, crate::utils::errors::TrackerError> {
        // For dummy, we don't need to store the rate or pids since it's just for testing
        let _ = (rate, provided_pids);
        Ok(Self)
    }
}

#[async_trait]
impl AsyncEnergyCollector for DummyEnergyGroup {
    fn discover_processes(&self, provided_pids: Option<Vec<usize>>) -> Result<Vec<crate::energy_monitor::ProcessGroup>, String> {
        // For dummy, use the default process discovery behavior
        gather_process_groups(provided_pids)
    }
    
    fn get_trace(&self) -> Result<HashMap<u64, Vec<f64>>, String> {
        // Return empty trace for dummy
        Ok(HashMap::new())
    }
    
    fn is_available() -> bool {
        true // Dummy is always available
    }
    
    async fn commence(&mut self) -> Result<(), String> {
        info!("Dummy group commence called");
        Ok(())
    }
    
    async fn shutdown(&mut self) -> Result<(), String> {
        info!("Dummy group shutdown called");
        Ok(())
    }
}
