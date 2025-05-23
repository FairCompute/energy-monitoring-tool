use std::collections::HashMap;
use async_trait::async_trait;
use log::info;
use crate::power_groups::tracker::{PowerGroupTracker, AsyncEnergyCollector};

pub struct DummyEnergyGroup {
    pub tracker: PowerGroupTracker,
}

#[async_trait]
impl AsyncEnergyCollector for DummyEnergyGroup {
    fn get_trace(&self) -> Result<HashMap<u64, Vec<f64>>, String> {
        Ok(self.tracker.energy_trace())
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
