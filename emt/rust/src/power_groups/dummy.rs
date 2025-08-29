use crate::energy_monitor::AsyncEnergyCollector;
use async_trait::async_trait;
use log::info;
use std::collections::HashMap;

pub struct DummyEnergyGroup;

impl DummyEnergyGroup {
    pub fn new() -> Result<Self, crate::utils::errors::MonitoringError> {
        Ok(Self {})
    }
}

impl Default for DummyEnergyGroup {
    fn default() -> Self {
        DummyEnergyGroup {}
    }
}

#[async_trait]
impl AsyncEnergyCollector for DummyEnergyGroup {
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
