use crate::power_groups::{Rapl, NvidiaGpu, DummyEnergyGroup, AsyncEnergyCollector};
use crate::energy_monitor::EnergyMonitor;

pub async fn demonstrate_power_groups() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Power Group Tracker Demo ===\n");
    
    // Create different types of collectors
    println!("1. Creating different collector types:");
    
    // Check availability without creating unused variables
    println!("   ✓ RAPL collector - Available: {}", Rapl::is_available());
    println!("   ✓ NVIDIA GPU collector - Available: {}", NvidiaGpu::is_available());
    println!("   ✓ Dummy collector - Available: {}", DummyEnergyGroup::is_available());
    
    // Create energy monitors with a dummy collector for demonstration
    let dummy_collector = DummyEnergyGroup::new(1.0, None)?;
    let dummy_monitor = EnergyMonitor::new(1.0, dummy_collector, None)?;
    println!("   ✓ Energy monitor with dummy collector created");
    
    println!("\n2. Process groups found:");
    let processes = dummy_monitor.processes();
    println!("   Found {} process groups", processes.len());
    
    // Show first few process groups
    for (i, group) in processes.iter().take(5).enumerate() {
        println!("   {}: User '{}', App '{}', {} PIDs", 
            i + 1, group.user, group.application, group.pids.len());
    }
    
    println!("\n3. Creating additional power group implementations:");
    
    // Create additional collectors for testing
    let mut rapl_group = Rapl::new(1.0, None, None)?;
    println!("   ✓ RAPL power group created");
    
    let mut nvidia_group = NvidiaGpu::new(1.0, None, Some(vec![0]))?;
    println!("   ✓ NVIDIA GPU power group created");
    
    let mut dummy_group = DummyEnergyGroup::new(1.0, None)?;
    println!("   ✓ Dummy power group created");
    
    println!("\n4. Testing energy collection:");
    
    // Test dummy energy collection (should always work)
    match dummy_group.commence().await {
        Ok(_) => println!("   ✓ Dummy energy collection successful"),
        Err(e) => println!("   ✗ Dummy energy collection failed: {}", e),
    }
    
    // Test RAPL energy collection (may fail if not available)
    match rapl_group.commence().await {
        Ok(_) => println!("   ✓ RAPL energy collection successful"),
        Err(e) => println!("   ⚠ RAPL energy collection failed: {}", e),
    }
    
    // Test NVIDIA energy collection (may fail if not available)
    match nvidia_group.commence().await {
        Ok(_) => println!("   ✓ NVIDIA GPU energy collection successful"),
        Err(e) => println!("   ⚠ NVIDIA GPU energy collection failed: {}", e),
    }
    
    println!("\n5. Shutting down:");
    rapl_group.shutdown().await?;
    nvidia_group.shutdown().await?;
    dummy_group.shutdown().await?;
    println!("   ✓ All power groups shut down successfully");
    
    Ok(())
}
