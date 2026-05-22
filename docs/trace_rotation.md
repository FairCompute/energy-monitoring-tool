# Trace Rotation Module

## Overview

The `trace_rotation` module implements an automated trace rotation mechanism that prevents unbounded memory growth in the energy and utilization traces. Similar to log rotation, it maintains a configurable time window (default: 1 hour) of recent data, automatically removing older entries.

## Problem Solved

Without trace rotation, the `energy_trace` and `utilization_trace` DataFrames grow indefinitely as data is appended. This can lead to:
- Excessive memory consumption over long-running monitoring sessions
- Degraded performance when querying large DataFrames
- Potential out-of-memory errors

## Solution

The `RotatingTrace` struct wraps a Polars DataFrame and provides:
- **Automatic cleanup**: Removes entries older than the retention window
- **Configurable retention**: Default 1 hour, adjustable per-trace
- **Efficient filtering**: Uses Polars' native filtering capabilities
- **Statistics**: Track data span, row count, and age of oldest entries

## Architecture

### RotatingTrace Struct

```rust
pub struct RotatingTrace {
    data: DataFrame,                      // The actual trace data
    config: RotationConfig,               // Retention configuration
    last_cleanup_time: i64,              // Throttles cleanup operations
    cleanup_interval_seconds: i64,       // Min interval between cleanups
}
```

### RotationConfig

```rust
pub struct RotationConfig {
    pub retention_seconds: i64,           // Time window to keep (default: 3600)
    pub auto_cleanup: bool,              // Auto-cleanup on append (default: true)
}
```

## Usage Examples

### Basic Usage in EnergyGroup

```rust
// Create rotating traces with 1-hour retention (automatic)
let energy_trace = RotatingTrace::new(3600);
let utilization_trace = RotatingTrace::new(3600);

// Data is automatically cleaned up when appended
energy_group.append_energy_records(records)?;  // Cleanup triggered if interval elapsed

// Change retention window at runtime
energy_group.set_energy_trace_retention(1800); // Switch to 30 minutes
```

### Configuration

```rust
// Create with custom configuration
let config = RotationConfig::new(7200)  // 2 hours
    .with_auto_cleanup(true);
let trace = RotatingTrace::with_config(config);

// Adjust cleanup throttling (avoid excessive cleanup operations)
let mut trace = RotatingTrace::new(3600);
trace.set_cleanup_interval_seconds(120); // Cleanup at most every 2 minutes
```

### Change Retention at Runtime

```rust
// This affects both energy and utilization traces
energy_group.set_trace_retention(1800); // Switch to 30 minutes
```

## Integration with EnergyGroup

The `EnergyGroup` struct now uses `RotatingTrace` for both traces:

```rust
pub struct EnergyGroup<T: EnergyCollector> {
    // ...
    energy_trace: RotatingTrace,        // Auto-rotating trace
    utilization_trace: RotatingTrace,   // Auto-rotating trace
    // ...
}
```

### New Methods

```rust
// Get DataFrame references (as before)
pub fn energy_trace(&self) -> &DataFrame { ... }
pub fn utilization_trace(&self) -> &DataFrame { ... }

// Get mutable access to RotatingTrace for advanced operations
pub fn energy_trace_mut(&mut self) -> &mut RotatingTrace { ... }
pub fn utilization_trace_mut(&mut self) -> &mut RotatingTrace { ... }

// Configure retention for all traces at once
pub fn set_trace_retention(&mut self, retention_seconds: i64) { ... }

// Monitor memory usage
pub fn trace_stats(&self) -> TraceMemoryStats { ... }
```

## Cleanup Mechanism

### Automatic Cleanup
- Triggered on each `append()` call if enabled
- Throttled to avoid excessive operations (default: every 60 seconds)
- Filters out entries where `timestamp <= (now - retention_seconds)`

### Manual Cleanup
```rust
trace.force_cleanup()?;  // Immediate cleanup regardless of throttling
```

### Cleanup Throttling
To prevent performance degradation from frequent cleanup:
```rust
trace.set_cleanup_interval_seconds(60);  // At most every 60 seconds
```

## Performance Considerations

1. **Memory Usage**: Bounded by retention window size and data collection rate
2. **Cleanup Cost**: O(n) operation on trace size, but throttled to reduce impact
3. **Query Performance**: Smaller DataFrames = faster queries on trace data
4. **CPU Overhead**: Minimal - cleanup uses efficient Polars filtering

## Configuration Strategies

### For Short-Term Monitoring (< 1 hour)
```rust
energy_group.set_energy_trace_retention(1800); // 30 minutes
```

### For Extended Sessions (< 24 hours)
```rust
energy_group.set_energy_trace_retention(86400); // 24 hours
```

### For High-Frequency Sampling
```rust
// Increase cleanup interval to reduce overhead
energy_trace_mut.set_cleanup_interval_seconds(300); // Every 5 minutes
energy_group.set_energy_trace_retention(3600);  // Keep 1 hour
```

## Testing

Unit tests verify:
- ✅ Creating rotating traces
- ✅ Appending data
- ✅ Automatic cleanup of old entries
- ✅ Statistics calculation
- ✅ Retention window enforcement

Run tests:
```bash
cargo test --bin emt trace_rotation
```

## API Stability

The trace rotation module is production-ready with:
- Comprehensive error handling
- Full documentation
- Unit test coverage
- Backward compatibility with existing trace access patterns

## Future Enhancements

Potential improvements:
- Compressed storage for archived data
- Export/archival of rotated data
- Configurable cleanup strategies (time-based, size-based, count-based)
- Trace snapshots at rotation boundaries
- Metrics on cleanup operations
