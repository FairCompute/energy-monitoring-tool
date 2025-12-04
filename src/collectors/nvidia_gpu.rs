use crate::energy_group::{EnergyCollector, EnergyRecord, UtilizationRecord};
use async_trait::async_trait;
use log::info;

pub struct NvidiaGpu {
    pub device_ids: Vec<u32>,
}

impl NvidiaGpu {
    pub fn new(device_ids: Vec<u32>) -> Self {
        Self { device_ids }
    }
}

impl Default for NvidiaGpu {
    fn default() -> Self {
        Self {
            device_ids: vec![0],
        } // Default to GPU 0
    }
}

#[async_trait]
impl EnergyCollector for NvidiaGpu {
    fn set_tracked_pids(&mut self, _pids: Vec<u32>) {
        // GPU collector doesn't use PIDs for attribution yet
    }

    async fn get_energy_trace(&self) -> Result<Vec<EnergyRecord>, String> {
        info!("NVIDIA GPU get_energy_trace called for devices: {:?}", self.device_ids);
        // Return empty trace for now - would implement actual NVIDIA energy trace collection here
        Ok(Vec::new())
    }

    async fn get_utilization_trace(&self) -> Result<Vec<UtilizationRecord>, String> {
        info!("NVIDIA GPU get_utilization_trace called for devices: {:?}", self.device_ids);
        // Return empty trace for now - would implement actual NVIDIA utilization trace collection here
        Ok(Vec::new())
    }

    fn is_available() -> bool {
        // Check if nvidia-smi command exists or NVIDIA drivers are loaded
        std::process::Command::new("nvidia-smi")
            .arg("--query-gpu=count")
            .arg("--format=csv,noheader,nounits")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}
