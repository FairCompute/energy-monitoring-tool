/*
=== Energy Monitor Architecture Example ===

This demonstrates how the refactored architecture allows different collectors
to implement custom process discovery strategies.

## Key Benefits

1. **Delegation to Collectors**: Process discovery is now handled by each collector type
2. **Custom Filtering**: Each collector can implement specialized process filtering
3. **GPU-Specific Logic**: NVIDIA collectors can filter for GPU-using processes
4. **RAPL Optimization**: RAPL collectors can focus on CPU-intensive processes
5. **Extensibility**: New collector types can implement their own discovery logic

## Example Usage:

```rust
// Basic usage with different collectors
let dummy_collector = DummyEnergyGroup::new(1.0, None)?;
let monitor = EnergyMonitor::new(1.0, dummy_collector, None)?;

// GPU collector with custom process filtering
let nvidia_collector = NvidiaGpu::new(1.0, None, Some(vec![0]))?;
let gpu_monitor = EnergyMonitor::new(1.0, nvidia_collector, None)?;
// ^ This could internally filter to only GPU-using processes

// RAPL collector 
let rapl_collector = Rapl::new(1.0, None, None)?;
let rapl_monitor = EnergyMonitor::new(1.0, rapl_collector, None)?;
// ^ This could filter to CPU-intensive processes
```

## Implementation Details

Each collector implements the `discover_processes` method from `AsyncEnergyCollector`:

```rust
trait AsyncEnergyCollector {
    fn discover_processes(&self, provided_pids: Option<Vec<usize>>) 
        -> Result<Vec<ProcessGroup>, String>;
    // ... other methods
}
```

This allows each collector to:
- Filter processes based on hardware usage (GPU, CPU, etc.)
- Implement hardware-specific discovery logic
- Optimize for their specific monitoring requirements
- Maintain separation of concerns

## GPU Example Implementation

For NVIDIA GPU collectors, the `discover_processes` method could:
1. Query `nvidia-smi --query-compute-apps=pid,process_name` to find GPU-using processes
2. Filter the provided PIDs to only include those using GPU resources
3. Return only processes that are actually utilizing GPU hardware

This ensures that GPU energy monitoring only tracks relevant processes.
*/

fn main() {
    println!("See the comments above for architecture examples!");
    println!("This refactoring allows collectors to implement custom process discovery logic.");
}
