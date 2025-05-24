mod power_groups;

use power_groups::{DummyEnergyGroup};

fn main() {
    //simply print a dummy message
    println!("Hello, world!");
    // Create a dummy energy group tracker
    let dummy_energy_group = DummyEnergyGroup {
        tracker: power_groups::tracker::PowerGroupTracker::new(1.0, None).unwrap(),
    };

    // Print the tracked processes
    println!("Tracked processes: {:?}", dummy_energy_group.tracker.processes());
    // Print program end message
    println!("Program ended successfully.");
}