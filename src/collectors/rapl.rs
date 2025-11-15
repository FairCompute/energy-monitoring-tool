use crate::energy_group::{EnergyCollector, EnergyRecord, UtilizationRecord};
use async_trait::async_trait;
use log::info;

pub struct Rapl {
    pub rapl_path: String,
}

impl Rapl {
    pub fn new(rapl_path: Option<String>) -> Self {
        let rapl_path = rapl_path.unwrap_or_else(|| "/sys/class/powercap/intel-rapl".to_string());
        Self { rapl_path }
    }
}

impl Default for Rapl {
    fn default() -> Self {
        Self {
            rapl_path: "/sys/class/powercap/intel-rapl".to_string(),
        }
    }
}

#[async_trait]
impl EnergyCollector for Rapl {
    async fn get_energy_trace(&self) -> Result<Vec<EnergyRecord>, String> {
        info!("RAPL get_energy_trace called, reading from path: {}", self.rapl_path);
        // Return empty trace for now - would implement actual RAPL energy trace collection here
        Ok(Vec::new())
    }

    async fn get_utilization_trace(&self) -> Result<Vec<UtilizationRecord>, String> {
        info!("RAPL get_utilization_trace called, reading from path: {}", self.rapl_path);
        // Return empty trace for now - would implement actual RAPL utilization trace collection here
        Ok(Vec::new())
    }

    fn is_available() -> bool {
        std::path::Path::new("/sys/class/powercap/intel-rapl").exists()
    }
}
