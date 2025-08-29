mod utils {
    pub mod errors;
    pub mod logger;
    pub mod psutils;
}

mod energy_monitor;

// Power group modules
pub mod power_groups {
    pub mod rapl;
    pub mod dummy;
    pub mod nvidia_gpu;
    
    pub use crate::energy_monitor::{AsyncEnergyCollector, PowerGroupType, ProcessGroup};
    pub use rapl::Rapl;
    pub use dummy::DummyEnergyGroup;
    pub use nvidia_gpu::NvidiaGpu;
}

use power_groups::DummyEnergyGroup;
use energy_monitor::EnergyMonitor;
use log::{info};

fn main() {
    utils::logger::setup_logger();
    info!("Application started");
    // Create a dummy energy group collector
    let dummy_collector: DummyEnergyGroup = DummyEnergyGroup::new(1.0, None).unwrap();
    
    // Create an energy monitor with the dummy collector
    let energy_monitor: EnergyMonitor<DummyEnergyGroup> = EnergyMonitor::new(1.0, dummy_collector, None).unwrap();

    // Print the tracked processes
    info!("Tracked processes: {:?}", energy_monitor.processes());
    // Print program end message
    info!("Program ended successfully.");
}