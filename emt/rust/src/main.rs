mod utils {
    pub mod errors;
    pub mod logger;
    pub mod psutils;
}

// Power group modules
pub mod power_groups;

use power_groups::{DummyEnergyGroup, EnergyGroup};
use log::{info};
use std::thread;
use std::time::Duration;

fn main() {
    utils::logger::setup_logger();
    info!("Application started");
    // Create a dummy energy group collector
    // Create an energy monitor with the dummy collector (optional argument)
    let mut energy_group_dummy: EnergyGroup<DummyEnergyGroup> = EnergyGroup::new(1.0, None).unwrap();
    energy_group_dummy.commence().unwrap();

    // Print the tracked processes
    info!("Tracked processes: {:?}", energy_group_dummy.processes());
    
    // Wait for 10 seconds to see the dummy collector working
    info!("Monitoring for 10 seconds...");
    thread::sleep(Duration::from_secs(10));
    
    // Shutdown the collector
    info!("Shutting down monitoring...");
    energy_group_dummy.shutdown().unwrap();
    
    // Print program end message
    info!("Program ended successfully.");
}