mod utils {
    pub mod errors;
    pub mod logger;
    pub mod psutils;
}

// Collector modules
pub mod collectors;
pub mod energy_group;

use collectors::Rapl;
use energy_group::EnergyGroup;
use log::{info};

#[tokio::main]
async fn main() {
    utils::logger::setup_logger();
    info!("Application started");
    
    // Create a RAPL energy group collector
    let mut energy_group_rapl: EnergyGroup<Rapl> = EnergyGroup::new(1.0, None).unwrap();
    energy_group_rapl.commence().await.unwrap();

    // Print the tracked processes
    info!("Tracked processes: {:?}", energy_group_rapl.processes());
    
    // Wait for 10 seconds to see the RAPL collector working
    info!("Monitoring for 10 seconds...");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    
    // Shutdown the collector
    info!("Shutting down monitoring...");
    energy_group_rapl.shutdown().unwrap();
    
    // Print program end message
    info!("Program ended successfully.");
}