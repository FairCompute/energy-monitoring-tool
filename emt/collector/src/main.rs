mod power_groups;
mod utils;

use power_groups::{DummyEnergyGroup};
use log::{info};

fn main() {
    utils::setup_logger();
    info!("Application started");
    // Create a dummy energy group tracker
    let dummy_energy_group = DummyEnergyGroup {
        tracker: power_groups::tracker::PowerGroupTracker::new(1.0, None).unwrap(),
    };

    // Print the tracked processes
    info!("Tracked processes: {:?}", dummy_energy_group.tracker.processes());
    // Print program end message
    info!("Program ended successfully.");
}