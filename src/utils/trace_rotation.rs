/// Trace Rotation Module
/// 
/// This module implements a rotating trace buffer that maintains a limited history window.
/// Similar to log rotation, it keeps only recent data within a configurable time window
/// (default: 1 hour) to prevent unbounded memory growth.
///
/// # Examples
///
/// ```ignore
/// let mut rotating_trace = RotatingTrace::new(3600); // Keep last 1 hour
/// rotating_trace.append(&energy_records)?;
/// rotating_trace.cleanup()?; // Periodically remove old entries
/// ```

use crate::utils::errors::MonitoringError;
use polars::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};

/// Configuration for trace rotation behavior
#[derive(Debug, Clone)]
pub struct RotationConfig {
    /// Time window to maintain in seconds (default: 3600 = 1 hour)
    pub retention_seconds: i64,
    /// Automatically cleanup on append if true (default: true)
    pub auto_cleanup: bool,
}

impl Default for RotationConfig {
    fn default() -> Self {
        Self {
            retention_seconds: 3600, // 1 hour default
            auto_cleanup: true,
        }
    }
}

impl RotationConfig {
    pub fn new(retention_seconds: i64) -> Self {
        Self {
            retention_seconds,
            auto_cleanup: true,
        }
    }

    pub fn with_auto_cleanup(mut self, auto_cleanup: bool) -> Self {
        self.auto_cleanup = auto_cleanup;
        self
    }
}

/// A rotating trace buffer that maintains limited history
/// 
/// Automatically removes entries older than the configured retention window.
/// Works with any DataFrame containing a "timestamp" column.
pub struct RotatingTrace {
    /// The trace data DataFrame with columns: pid | timestamp | device | <metric>
    data: DataFrame,
    /// Rotation configuration
    config: RotationConfig,
    /// Last cleanup timestamp (to avoid excessive cleanup operations)
    last_cleanup_time: i64,
    /// Cleanup interval in seconds to throttle cleanup operations
    cleanup_interval_seconds: i64,
}

impl RotatingTrace {
    /// Create a new rotating trace with default configuration (1 hour retention)
    pub fn new(retention_seconds: i64) -> Self {
        Self::with_config(RotationConfig::new(retention_seconds))
    }

    /// Create a new rotating trace with custom configuration
    pub fn with_config(config: RotationConfig) -> Self {
        Self {
            data: DataFrame::default(),
            config,
            last_cleanup_time: current_timestamp_secs(),
            cleanup_interval_seconds: 60, // Cleanup at most every 60 seconds
        }
    }

    /// Get current timestamp in seconds since UNIX_EPOCH
    fn get_current_timestamp() -> i64 {
        current_timestamp_secs()
    }

    /// Get a reference to the trace data
    pub fn data(&self) -> &DataFrame {
        &self.data
    }

    /// Get a mutable reference to the trace data
    pub fn data_mut(&mut self) -> &mut DataFrame {
        &mut self.data
    }

    /// Get the retention window in seconds
    pub fn retention_seconds(&self) -> i64 {
        self.config.retention_seconds
    }

    /// Get the number of rows in the trace
    pub fn row_count(&self) -> usize {
        self.data.height()
    }

    /// Append new records to the trace
    /// 
    /// If auto_cleanup is enabled, this will also remove old entries outside the retention window.
    pub fn append(&mut self, new_data: &DataFrame) -> Result<(), MonitoringError> {
        if new_data.is_empty() {
            return Ok(());
        }

        // Validate that the new data has a timestamp column
        if !new_data.get_column_names().iter().any(|name| name.to_string() == "timestamp") {
            return Err(MonitoringError::Other(
                "DataFrame must contain a 'timestamp' column for rotation".to_string(),
            ));
        }

        // Append new data to existing trace
        if self.data.is_empty() {
            self.data = new_data.clone();
        } else {
            self.data = self.data.vstack(new_data).map_err(|e| {
                MonitoringError::Other(format!("Failed to append trace data: {}", e))
            })?;
        }

        // Auto cleanup if enabled
        if self.config.auto_cleanup {
            let now = Self::get_current_timestamp();
            if now - self.last_cleanup_time >= self.cleanup_interval_seconds {
                self.cleanup()?;
            }
        }

        Ok(())
    }

    /// Remove entries older than the retention window
    /// 
    /// This operation filters the DataFrame to keep only entries with timestamps
    /// within the last `retention_seconds`.
    pub fn cleanup(&mut self) -> Result<(), MonitoringError> {
        if self.data.is_empty() {
            self.last_cleanup_time = Self::get_current_timestamp();
            return Ok(());
        }

        let now = Self::get_current_timestamp();
        let cutoff_time = now - self.config.retention_seconds;

        // Get timestamp column
        let timestamp_col = self
            .data
            .column("timestamp")
            .map_err(|e| MonitoringError::Other(format!("Failed to access timestamp column: {}", e)))?;

        // Cast to i64 if needed
        let timestamps = timestamp_col.i64().map_err(|e| {
            MonitoringError::Other(format!("Timestamp column is not i64 type: {}", e))
        })?;

        // Create filter mask for rows within retention window
        let mask = timestamps
            .iter()
            .map(|opt_ts| opt_ts.map(|ts| ts > cutoff_time).unwrap_or(false))
            .collect::<Vec<_>>();

        // Convert mask to BooleanChunked
        let mask_series = Series::new("filter".into(), mask);
        let mask_bool = mask_series.bool().map_err(|e| {
            MonitoringError::Other(format!("Failed to create boolean mask: {}", e))
        })?;

        // Filter the DataFrame
        self.data = self
            .data
            .filter(&mask_bool)
            .map_err(|e| MonitoringError::Other(format!("Failed to filter trace data: {}", e)))?;

        self.last_cleanup_time = now;
        Ok(())
    }

    /// Force cleanup regardless of timing
    pub fn force_cleanup(&mut self) -> Result<(), MonitoringError> {
        self.cleanup()
    }

    /// Get statistics about the trace
    pub fn stats(&self) -> TraceStats {
        let row_count = self.data.height();
        let oldest_timestamp = if row_count > 0 {
            self.data
                .column("timestamp")
                .ok()
                .and_then(|col| col.i64().ok())
                .and_then(|s| s.iter().filter_map(|v| v).min())
        } else {
            None
        };

        let newest_timestamp = if row_count > 0 {
            self.data
                .column("timestamp")
                .ok()
                .and_then(|col| col.i64().ok())
                .and_then(|s| s.iter().filter_map(|v| v).max())
        } else {
            None
        };

        TraceStats {
            row_count,
            oldest_timestamp,
            newest_timestamp,
            retention_seconds: self.config.retention_seconds,
        }
    }

    /// Clear all data from the trace
    pub fn clear(&mut self) {
        self.data = DataFrame::default();
        self.last_cleanup_time = Self::get_current_timestamp();
    }

    /// Update the retention window (in seconds)
    pub fn set_retention_seconds(&mut self, seconds: i64) {
        self.config.retention_seconds = seconds;
    }

    /// Update cleanup interval (throttling mechanism)
    pub fn set_cleanup_interval_seconds(&mut self, seconds: i64) {
        self.cleanup_interval_seconds = seconds;
    }
}

/// Statistics about a rotating trace
#[derive(Debug, Clone)]
pub struct TraceStats {
    pub row_count: usize,
    pub oldest_timestamp: Option<i64>,
    pub newest_timestamp: Option<i64>,
    pub retention_seconds: i64,
}

impl TraceStats {
    /// Get the age of the oldest entry in seconds
    pub fn oldest_age_seconds(&self) -> Option<i64> {
        let now = current_timestamp_secs();
        self.oldest_timestamp.map(|ts| now - ts)
    }

    /// Get the span of data in seconds (newest - oldest)
    pub fn data_span_seconds(&self) -> Option<i64> {
        match (self.oldest_timestamp, self.newest_timestamp) {
            (Some(oldest), Some(newest)) => Some(newest - oldest),
            _ => None,
        }
    }
}

/// Get current timestamp in seconds since UNIX_EPOCH
fn current_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use polars::df;

    #[test]
    fn test_rotating_trace_creation() {
        let trace = RotatingTrace::new(3600);
        assert_eq!(trace.retention_seconds(), 3600);
        assert_eq!(trace.row_count(), 0);
    }

    #[test]
    fn test_append_data() {
        let mut trace = RotatingTrace::new(3600);
        let now = current_timestamp_secs();
        
        let data = df![
            "pid" => vec![1u32, 2u32],
            "timestamp" => vec![now, now],
            "device" => vec!["cpu".to_string(), "gpu".to_string()],
            "energy" => vec![10.5, 20.3],
        ]
        .unwrap();

        trace.append(&data).unwrap();
        assert_eq!(trace.row_count(), 2);
    }

    #[test]
    fn test_cleanup_old_entries() {
        let mut trace = RotatingTrace::new(100); // 100 second retention
        let now = current_timestamp_secs();
        
        let data = df![
            "pid" => vec![1u32, 1u32, 1u32],
            "timestamp" => vec![now - 200, now - 50, now], // one is too old
            "device" => vec!["cpu".to_string(), "cpu".to_string(), "cpu".to_string()],
            "energy" => vec![10.0, 20.0, 30.0],
        ]
        .unwrap();

        trace.append(&data).unwrap();
        assert_eq!(trace.row_count(), 3);

        // Force cleanup
        trace.force_cleanup().unwrap();
        
        // Should now have only 2 entries (the old one removed)
        assert_eq!(trace.row_count(), 2);
    }

    #[test]
    fn test_stats() {
        let mut trace = RotatingTrace::new(3600);
        let now = current_timestamp_secs();
        
        let data = df![
            "pid" => vec![1u32, 1u32],
            "timestamp" => vec![now - 100, now],
            "device" => vec!["cpu".to_string(), "cpu".to_string()],
            "energy" => vec![10.0, 20.0],
        ]
        .unwrap();

        trace.append(&data).unwrap();
        let stats = trace.stats();

        assert_eq!(stats.row_count, 2);
        assert!(stats.oldest_timestamp.is_some());
        assert!(stats.newest_timestamp.is_some());
        assert_eq!(stats.retention_seconds, 3600);
    }
}
