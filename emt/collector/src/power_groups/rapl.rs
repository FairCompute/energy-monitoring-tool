use std::collections::HashMap;
use async_trait::async_trait;
use crate::power_groups::tracker::{PowerGroupTracker, AsyncEnergyCollector};

pub struct RaplSocCpuGroup {
    pub tracker: PowerGroupTracker,
    pub rapl_path: String,
}

#[async_trait]
impl AsyncEnergyCollector for RaplSocCpuGroup {
    fn get_trace(&self) -> Result<HashMap<u64, Vec<f64>>, String> {
        Ok(self.tracker.energy_trace())
    }
    async fn commence(&mut self) -> Result<(), String> {
        log::info!("RAPL group commence called");
        let interval = self.tracker.sleep_interval();
        println!("Sleeping for {} seconds (yielding)...", interval);
        tokio::time::sleep(tokio::time::Duration::from_secs_f64(interval)).await;
        Ok(())
    }
    async fn shutdown(&mut self) -> Result<(), String> {
        log::info!("RAPL group shutdown called");
        Ok(())
    }
}
