use crate::utils::errors::MonitoringError;
use crate::utils::psutils::collect_process_groups;
use async_trait::async_trait;
use itertools::multiunzip;
use polars::prelude::*;
use std::collections::HashMap;

#[derive(Debug)]
pub enum EnergyCollectorType {
    Rapl,
    NvidiaGpu,
    Dummy,
}

#[derive(Debug)]
pub struct ProcessGroup {
    pub user: String,
    pub task: String,
    pub pids: Vec<usize>,
}

/// Generic Energy Monitor
/// # Type Parameters
/// * `T` - An energy collector type that implements `AsyncEnergyCollector`
pub struct EnergyGroup<T: AsyncEnergyCollector> {
    rate: f64,
    /// DataFrame: user | task | pid
    tracked_processes: DataFrame,
    /// DataFrame: pid | timestamp | device | energy
    energy_trace: DataFrame,
    /// DataFrame: pid | timestamp | device | utilization
    utilization_trace: DataFrame,
    /// Underlying concrete energy group
    energy_collector: T,
    /// Tokio runtime for async operations
    runtime: tokio::runtime::Runtime,
}

impl<T: AsyncEnergyCollector> EnergyGroup<T> {
    /// Create a new PowerGroup with explicit collector instance
    pub fn create_with_collector(
        collector: T,
        rate: f64,
        pids: Option<Vec<usize>>,
    ) -> Result<Self, MonitoringError> {
        let process_groups: Vec<ProcessGroup> = collect_process_groups(pids)?;
        if process_groups.is_empty() {
            return Err(MonitoringError::ProcessDiscoveryError(
                "No processes found".to_string(),
            ));
        }

        // Concise conversion to Polars DataFrame: user | task | pid
        let (users, tasks, pids_col): (Vec<String>, Vec<String>, Vec<u32>) =
            multiunzip(process_groups.iter().flat_map(|group| {
                group
                    .pids
                    .iter()
                    .map(move |&pid| (group.user.clone(), group.task.clone(), pid as u32))
            }));

        let tracked_processes = df![
            "user" => users,
            "task" => tasks,
            "pid" => pids_col,
        ]
        .map_err(|e| MonitoringError::Other(format!("Failed to create DataFrame: {}", e)))?;

        // Create empty energy_traces DataFrame: pid | timestamp | device | energy
        let mut energy_trace = df![
            "pid" => Vec::<u32>::new(),
            "device" => Vec::<String>::new(),
            "energy" => Vec::<f64>::new(),
            "timestamp" => Vec::<i64>::new(),
        ]
        .map_err(|e| {
            MonitoringError::Other(format!("Failed to create energy_traces DataFrame: {}", e))
        })?;

        // Cast the timestamp column to Datetime with milliseconds
        energy_trace
            .with_column(
                energy_trace
                    .column("timestamp")
                    .map_err(|e| MonitoringError::Other(e.to_string()))?
                    .cast(&DataType::Datetime(TimeUnit::Milliseconds, None))
                    .map_err(|e| MonitoringError::Other(e.to_string()))?,
            )
            .map_err(|e| MonitoringError::Other(e.to_string()))?;

        // Create empty utilization_trace DataFrame: pid | timestamp | device | utilization
        let mut utilization_trace = df![
            "pid" => Vec::<u32>::new(),
            "device" => Vec::<String>::new(),
            "utilization" => Vec::<f64>::new(),
            "timestamp" => Vec::<i64>::new(),
        ]
        .map_err(|e| {
            MonitoringError::Other(format!("Failed to create utilization_trace DataFrame: {}", e))
        })?;

        // Cast the timestamp column to Datetime with milliseconds
        utilization_trace
            .with_column(
                utilization_trace
                    .column("timestamp")
                    .map_err(|e| MonitoringError::Other(e.to_string()))?
                    .cast(&DataType::Datetime(TimeUnit::Milliseconds, None))
                    .map_err(|e| MonitoringError::Other(e.to_string()))?,
            )
            .map_err(|e| MonitoringError::Other(e.to_string()))?;

        Ok(Self {
            rate,
            tracked_processes,
            energy_trace,
            utilization_trace,
            energy_collector: collector,
            runtime: tokio::runtime::Runtime::new()
                .map_err(|e| MonitoringError::Other(format!("Failed to create Tokio runtime: {}", e)))?,
        })
    }

    /// Convenience constructor: only rate and pids. A default collector instance is created.
    /// for the collector type `T`, it must implement `Default`.
    pub fn new(rate: f64, pids: Option<Vec<usize>>) -> Result<Self, MonitoringError>
    where
        T: Default,
    {
        Self::create_with_collector(T::default(), rate, pids)
    }

    /// Get a reference to the tracked process DataFrame
    pub fn processes(&self) -> &DataFrame {
        &self.tracked_processes
    }

    /// Get a reference to the energy trace DataFrame
    pub fn energy_trace(&self) -> &DataFrame {
        &self.energy_trace
    }

    /// Check if the underlying collector is available on the system
    pub fn is_available() -> bool { 
        T::is_available()
    }

    pub fn commence(& self) -> Result<(), MonitoringError> {

        if !T::is_available() {
            return Err(MonitoringError::Other(format!(
                "Collector type is not available on this system"
            )));
        }
        
        self.runtime
            .block_on(self.energy_collector.commence(self.rate))
            .map_err(|e| MonitoringError::Other(format!("Failed to commence collector: {}", e)))?;
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<(), MonitoringError> {
        self.runtime
            .block_on(self.energy_collector.shutdown())
            .map_err(|e| MonitoringError::Other(format!("Failed to shutdown collector: {}", e)))?;
        Ok(())
    }

}

#[async_trait]
pub trait AsyncEnergyCollector {
    /// Get energy trace data
    fn get_trace(&self) -> Result<HashMap<u64, Vec<f64>>, String>;

    /// Check if this collector type is available on the system
    fn is_available() -> bool {
        unimplemented!()
    }

    /// Start monitoring at the specified rate (samples per second)
    async fn commence(&self, rate: f64) -> Result<(), String>;

    /// Stop monitoring
    async fn shutdown(&mut self) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power_groups::DummyEnergyGroup;

    #[test]
    fn test_tracker_empty() {
        // Empty process list should return an error, not succeed
        let result = EnergyGroup::<DummyEnergyGroup>::new(1.0, Some(vec![]));
        assert!(result.is_err());
        
        match result {
            Err(MonitoringError::ProcessDiscoveryError(msg)) => {
                assert!(msg.contains("No process"));
            }
            _ => panic!("Expected ProcessDiscoveryError"),
        }
    }

    #[test]
    fn test_tracker_with_nonexistent_pid() {
        // Use a realistic but likely non-existent PID, but the test should handle the case
        // where process discovery might not find any valid process groups
        let result = EnergyGroup::<DummyEnergyGroup>::new(1.0, Some(vec![999999]));
        
        match result {
            Ok(tracker) => {
                let df = tracker.processes();
                let user = df.column("user").unwrap().str().unwrap();
                let task = df.column("task").unwrap().str().unwrap();
                assert!(
                    user.into_iter()
                        .zip(task.into_iter())
                        .any(|(u, t)| u == Some("unknown") && t == Some("unknown"))
                );
            }
            Err(MonitoringError::ProcessDiscoveryError(_)) => {
                // This is also acceptable - no valid process groups found
                assert!(true);
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_tracker_with_pid_1() {
        let tracker: EnergyGroup<DummyEnergyGroup> = EnergyGroup::new(1.0, Some(vec![1])).unwrap();
        let df = tracker.processes();
        let pids = df.column("pid").unwrap().u32().unwrap();
        assert!(pids.into_iter().any(|pid| pid == Some(1)));
    }

    #[test]
    fn test_tracker_all_processes_not_empty() {
        let tracker: EnergyGroup<DummyEnergyGroup> = EnergyGroup::new(1.0, None).unwrap();
        assert!(tracker.processes().height() > 0);
    }

    #[test]
    fn test_tracker_pid_grouping() {
        let tracker: EnergyGroup<DummyEnergyGroup> = EnergyGroup::new(1.0, None).unwrap();
        let df = tracker.processes();
        let pids = df.column("pid").unwrap().u32().unwrap();
        let mut all_pids: Vec<u32> = pids.into_iter().flatten().collect();
        all_pids.sort();
        all_pids.dedup();
        assert_eq!(all_pids.len(), tracker.processes().height());
    }

    #[test]
    // Test to debug what users we actually see
    fn test_debug_users() {
        use crate::utils::psutils::resolve_username;
        use std::collections::HashMap;
        use sysinfo::System;
        use users::UsersCache;

        let system = System::new_all();
        let users_cache = UsersCache::new();
        let mut user_counts: HashMap<String, usize> = HashMap::new();

        // Count processes by user
        for (_pid, process) in system.processes().into_iter().take(100) {
            let user = process
                .user_id()
                .map(|uid| resolve_username(**uid, &users_cache))
                .unwrap_or_else(|| "unknown".to_string());

            *user_counts.entry(user).or_insert(0) += 1;
        }

        // Print results
        let mut users: Vec<(String, usize)> = user_counts.into_iter().collect();
        users.sort_by(|a, b| b.1.cmp(&a.1));

        eprintln!("Users found (first 10):");
        for (user, count) in users.iter().take(10) {
            eprintln!("  {}: {} processes", user, count);
        }

        // The test should pass - we just want to see the debug output
        assert!(!users.is_empty());
    }

    #[test]
    fn test_collector_availability_check() {
        // Test that dummy collector is available
        assert!(EnergyGroup::<DummyEnergyGroup>::is_available());
        
        // Create a tracker and test commence with availability check (use PID 1 which should exist)
        let mut tracker: EnergyGroup<DummyEnergyGroup> = EnergyGroup::new(1.0, Some(vec![1])).unwrap();
        
        // Since DummyEnergyGroup::is_available() returns true, commence should work
        assert!(tracker.commence().is_ok());
        
        // Test shutdown
        assert!(tracker.shutdown().is_ok());
    }

    #[test]
    // Test availability checks
    fn test_availability() {
        // Test dummy availability - should always be available since it's just for testing
        assert!(EnergyGroup::<DummyEnergyGroup>::is_available());
    }
}
