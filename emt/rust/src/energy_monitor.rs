use async_trait::async_trait;
use std::collections::HashMap;
use itertools::multiunzip;
use polars::prelude::*;
use crate::utils::psutils::collect_process_groups;
use crate::utils::errors::MonitoringError;


#[derive(Debug)]
pub enum PowerGroupType {
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

/// Generic energy monitor that works with any energy collector implementation
/// 
/// # Type Parameters
/// * `T` - An energy collector type that implements `AsyncEnergyCollector`
pub struct EnergyMonitor<T: AsyncEnergyCollector> {
    rate: f64,
    count_trace_calls: usize,
    /// DataFrame: user | task | pid
    tracked_processes: DataFrame,
    /// DataFrame: pid | timestamp | device | energy
    energy_trace: DataFrame,
    /// Energy collector implementation that handles energy measurement
    collector: T,
}


impl<T: AsyncEnergyCollector> EnergyMonitor<T> {
    /// Create a new EnergyMonitor with explicit collector
    pub fn new_with_collector(collector: T, rate: f64,  pids: Option<Vec<usize>>) -> Result<Self, MonitoringError> {
        let process_groups: Vec<ProcessGroup> = collect_process_groups(pids)?;
        if process_groups.is_empty() {
            return Err(MonitoringError::ProcessDiscoveryError("No processes found".to_string()));
        }

        // Concise conversion to Polars DataFrame: user | task | pid

        let (users, tasks, pids_col): (Vec<String>, Vec<String>, Vec<u32>) = multiunzip(
            process_groups.iter().flat_map(|group|
                group.pids.iter().map(move |&pid| (group.user.clone(), group.task.clone(), pid as u32))
            )
        );

        let tracked_processes = df![
            "user" => users,
            "task" => tasks,
            "pid" => pids_col,
        ].map_err(|e| MonitoringError::Other(format!("Failed to create DataFrame: {}", e)))?;

        // Create empty energy_traces DataFrame: pid | timestamp | device | energy
        let mut energy_trace = df![
            "pid" => Vec::<u32>::new(),
            "device" => Vec::<String>::new(),
            "energy" => Vec::<f64>::new(),
            "timestamp" => Vec::<i64>::new(),
        ].map_err(|e| MonitoringError::Other(format!("Failed to create energy_traces DataFrame: {}", e)))?;

        // Cast the timestamp column to Datetime with milliseconds
        energy_trace.with_column(
                energy_trace.column("timestamp").map_err(|e| MonitoringError::Other(e.to_string()))?
                    .cast(&DataType::Datetime(TimeUnit::Milliseconds, None)).map_err(|e| MonitoringError::Other(e.to_string()))?
            ).map_err(|e| MonitoringError::Other(e.to_string()))?;  

                    
        Ok(Self {
            rate,
            count_trace_calls: 0,
            tracked_processes,
            energy_trace,
            collector: collector,
        })
    }

    /// Convenience constructor: only rate and pids, collector defaults to None (uses Default)
    pub fn new(rate: f64, pids: Option<Vec<usize>>) -> Result<Self, MonitoringError>
    where
        T: Default,
    {
    Self::new_with_collector(T::default(), rate, pids)
    
    }

    pub fn sleep_interval(&self) -> f64 {
        1.0 / self.rate
    }
    
    
    /// Get a reference to the tracked process DataFrame
    pub fn processes(&self) -> &DataFrame {
        &self.tracked_processes
    }

    /// Get a reference to the energy trace DataFrame
    pub fn energy_trace(&self) -> &DataFrame {
        &self.energy_trace
    }

    /// Get a mutable reference to the energy trace DataFrame
    pub fn energy_trace_mut(&mut self) -> &mut DataFrame {
        &mut self.energy_trace
    }

    /// Get a reference to the energy collector implementation
    pub fn collector(&self) -> &T {
        &self.collector
    }

    /// Check if the energy collector type is available on this system
    pub fn is_available() -> bool where T: AsyncEnergyCollector {
        T::is_available()
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
    
    /// Start monitoring
    async fn commence(&mut self) -> Result<(), String>;
    
    /// Stop monitoring
    async fn shutdown(&mut self) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power_groups::DummyEnergyGroup;

    #[test]
    fn test_tracker_empty() {
    let tracker: EnergyMonitor<DummyEnergyGroup> = EnergyMonitor::new(1.0, Some(vec![])).unwrap();
        assert_eq!(tracker.processes().height(), 0);
    }

    #[test]
    fn test_tracker_with_nonexistent_pid() {
    let tracker: EnergyMonitor<DummyEnergyGroup> = EnergyMonitor::new(1.0, Some(vec![999999])).unwrap();
        let df = tracker.processes();
        let user = df.column("user").unwrap().str().unwrap();
        let task = df.column("task").unwrap().str().unwrap();
        assert!(user.into_iter().zip(task.into_iter()).any(|(u, t)| u == Some("unknown") && t == Some("unknown")));
    }

    #[test]
    fn test_tracker_with_pid_1() {
    let tracker: EnergyMonitor<DummyEnergyGroup> = EnergyMonitor::new(1.0, Some(vec![1])).unwrap();
        let df = tracker.processes();
        let pids = df.column("pid").unwrap().u32().unwrap();
        assert!(pids.into_iter().any(|pid| pid == Some(1)));
    }

    #[test]
    fn test_tracker_all_processes_not_empty() {
    let tracker: EnergyMonitor<DummyEnergyGroup> = EnergyMonitor::new(1.0, None).unwrap();
        assert!(tracker.processes().height() > 0);
    }

    #[test]
    fn test_tracker_pid_grouping() {
    let tracker: EnergyMonitor<DummyEnergyGroup> = EnergyMonitor::new(1.0, None).unwrap();
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
        use std::collections::HashMap;
        use sysinfo::{System};
        use users::{UsersCache};
        use crate::utils::psutils::resolve_username;
        
        let system = System::new_all();
        let users_cache = UsersCache::new();
        let mut user_counts: HashMap<String, usize> = HashMap::new();
        
        // Count processes by user
        for (_pid, process) in system.processes().into_iter().take(100) {
            let user = process.user_id()
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
    // Test different power group types
    fn test_power_group_types() {
    let tracker: EnergyMonitor<DummyEnergyGroup> = EnergyMonitor::new(1.0, Some(vec![])).unwrap();
        
        // Test that we can access the collector
        assert!(tracker.collector().get_trace().is_ok());
    }
    
    #[test]
    // Test availability checks
    fn test_availability() {
        // Test dummy availability - should always be available since it's just for testing
        assert!(EnergyMonitor::<DummyEnergyGroup>::is_available());
    }
}
