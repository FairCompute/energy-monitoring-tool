use crate::power_groups::energy_group::AsyncEnergyCollector;
use async_trait::async_trait;
use log::info;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{sleep, Duration};

pub struct DummyEnergyGroup {
    running: Arc<AtomicBool>,
}

impl DummyEnergyGroup {
    pub fn new() -> Result<Self, crate::utils::errors::MonitoringError> {
        Ok(Self {
            running: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl Default for DummyEnergyGroup {
    fn default() -> Self {
        DummyEnergyGroup {
            running: Arc::new(AtomicBool::new(false)),
        }
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

    async fn commence(&self, rate: f64) -> Result<(), String> {
        info!("Dummy group commence called - starting monitoring loop");
        self.running.store(true, Ordering::Relaxed);
        let interval_secs = 1.0 / rate;
        let interval = Duration::from_secs_f64(interval_secs);
        let running = Arc::clone(&self.running);

        tokio::spawn(async move {
            let mut counter = 0;
            while running.load(Ordering::Relaxed) {
                counter += 1;
                info!("Dummy energy monitoring iteration {}", counter);       
                sleep(interval).await;
            }
            info!("Dummy monitoring loop stopped");
        });
        
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), String> {
        info!("Dummy group shutdown called - stopping monitoring loop");
        self.running.store(false, Ordering::Relaxed);
        Ok(())
    }
}
