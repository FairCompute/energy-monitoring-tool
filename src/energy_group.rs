use crate::utils::errors::MonitoringError;
use crate::utils::psutils::collect_process_groups;
use async_trait::async_trait;
use itertools::multiunzip;
use polars::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;

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

#[derive(Debug, Clone)]
pub struct EnergyRecord {
    pub pid: u32,
    pub timestamp: i64,
    pub device: String,
    pub energy: f64,
}

#[derive(Debug, Clone)]
pub struct UtilizationRecord {
    pub pid: u32,
    pub timestamp: i64,
    pub device: String,
    pub utilization: f64,
}

/// Generic Energy Monitor
/// # Type Parameters
/// * `T` - An energy collector type that implements `EnergyCollector`
pub struct EnergyGroup<T: EnergyCollector> {
    rate: f64,
    /// DataFrame: user | task | pid
    tracked_processes: DataFrame,
    /// DataFrame: pid | timestamp | device | energy
    energy_trace: DataFrame,
    /// DataFrame: pid | timestamp | device | utilization
    utilization_trace: DataFrame,
    /// Underlying concrete energy group
    energy_collector: T,
    /// Track whether the collector is currently running
    is_running: Arc<AtomicBool>,
    /// Handle to the background monitoring task
    task_handle: Option<JoinHandle<()>>,
}

impl<T: EnergyCollector> EnergyGroup<T> {
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
        let energy_trace = df![
            "pid" => Vec::<u32>::new(),
            "device" => Vec::<String>::new(),
            "energy" => Vec::<f64>::new(),
            "timestamp" => Vec::<i64>::new(),
        ]
        .map_err(|e| {
            MonitoringError::Other(format!("Failed to create energy_traces DataFrame: {}", e))
        })?;

        // Create empty utilization_trace DataFrame: pid | timestamp | device | utilization
        let utilization_trace = df![
            "pid" => Vec::<u32>::new(),
            "device" => Vec::<String>::new(),
            "utilization" => Vec::<f64>::new(),
            "timestamp" => Vec::<i64>::new(),
        ]
        .map_err(|e| {
            MonitoringError::Other(format!("Failed to create utilization_trace DataFrame: {}", e))
        })?;

        Ok(Self {
            rate,
            tracked_processes,
            energy_trace,
            utilization_trace,
            energy_collector: collector,
            is_running: Arc::new(AtomicBool::new(false)),
            task_handle: None,
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

    /// Get a reference to the utilization trace DataFrame
    pub fn utilization_trace(&self) -> &DataFrame {
        &self.utilization_trace
    }

    /// Add energy records to the energy trace DataFrame
    pub fn append_energy_records(&mut self, records: Vec<EnergyRecord>) -> Result<(), MonitoringError> {
        if records.is_empty() {
            return Ok(());
        }

        let data = DataFrame::new(vec![
            Column::new("pid".into(), records.iter().map(|r| r.pid).collect::<Vec<_>>()),
            Column::new("device".into(), records.iter().map(|r| r.device.clone()).collect::<Vec<_>>()),
            Column::new("energy".into(), records.iter().map(|r| r.energy).collect::<Vec<_>>()),
            Column::new("timestamp".into(), records.iter().map(|r| r.timestamp).collect::<Vec<_>>()),
        ])
        .map_err(|err| MonitoringError::Other(err.to_string()))?;

        self.energy_trace = self.energy_trace
            .clone()
            .vstack(&data)
            .map_err(|err| MonitoringError::Other(err.to_string()))?;

        Ok(())
    }

    /// Add utilization records to the utilization trace DataFrame
    pub fn append_utilization_records(&mut self, records: Vec<UtilizationRecord>) -> Result<(), MonitoringError> {
        if records.is_empty() {
            return Ok(());
        }

        // Extract data from records
        let pids: Vec<u32> = records.iter().map(|r| r.pid).collect();
        let timestamps: Vec<i64> = records.iter().map(|r| r.timestamp).collect();
        let devices: Vec<String> = records.iter().map(|r| r.device.clone()).collect();
        let utilizations: Vec<f64> = records.iter().map(|r| r.utilization).collect();

        // Create new DataFrame from records
        let new_data = df![
            "pid" => pids,
            "device" => devices,
            "utilization" => utilizations,
            "timestamp" => timestamps,
        ]
        .map_err(|e| MonitoringError::Other(format!("Failed to create utilization DataFrame: {}", e)))?;

        // Append to existing utilization trace
        self.utilization_trace = self.utilization_trace
            .clone()
            .vstack(&new_data)
            .map_err(|e| MonitoringError::Other(format!("Failed to append utilization data: {}", e)))?;

        Ok(())
    }

    /// Check if the underlying collector is available on the system
    pub fn is_available() -> bool { 
        T::is_available()
    }

    /// Check if the collector is currently running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Relaxed)
    }

    pub async fn commence(&mut self) -> Result<(), MonitoringError> {
        // Check if collector is already running
        if self.is_running() {
            eprintln!("Warning: Energy collector is already running. Ignoring commence request.");
            return Ok(());
        }

        if !T::is_available() {
            return Err(MonitoringError::Other(format!(
                "Collector type is not available on this system"
            )));
        }
        
        // Set running state before starting
        self.is_running.store(true, Ordering::Relaxed);
        
        // Collect initial data concurrently using tokio::join!
        let (energy_result, utilization_result) = tokio::join!(
            self.energy_collector.get_energy_trace(),
            self.energy_collector.get_utilization_trace()
        );

        let energy_records = energy_result
            .map_err(|e| MonitoringError::Other(format!("Failed to get energy trace: {}", e)))?;
        let utilization_records = utilization_result
            .map_err(|e| MonitoringError::Other(format!("Failed to get utilization trace: {}", e)))?;
        
        // Append initial data
        self.append_energy_records(energy_records)?;
        self.append_utilization_records(utilization_records)?;
        
        // Spawn background task for continuous monitoring
        let rate = self.rate;
        let interval = tokio::time::Duration::from_secs_f64(1.0 / rate);
        let is_running = Arc::clone(&self.is_running);
        
        // For this simplified version, we'll just simulate data generation
        // In a real implementation, we'd move the collector to the background task
        let handle = tokio::spawn(async move {
            let mut iteration = 0;
            while is_running.load(Ordering::Relaxed) {
                iteration += 1;
                println!("Background monitoring iteration {}", iteration);
                
                // Simulate collecting data concurrently
                // In real implementation, we'd call collector methods here concurrently
                // and update the data structures
                
                tokio::time::sleep(interval).await;
            }
            println!("Background monitoring stopped after {} iterations", iteration);
        });
        
        // Store the task handle
        self.task_handle = Some(handle);
        
        println!("Monitoring started in background at {} Hz", rate);
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<(), MonitoringError> {
        // Reset running state before shutdown
        self.is_running.store(false, Ordering::Relaxed);
        
        // Cancel the background task if it exists
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
        
        Ok(())
    }

}

#[async_trait]
pub trait EnergyCollector {
    /// Get energy trace data
    async fn get_energy_trace(&self) -> Result<Vec<EnergyRecord>, String>;

    /// Get utilization trace data  
    async fn get_utilization_trace(&self) -> Result<Vec<UtilizationRecord>, String>;

    /// Check if this collector type is available on the system
    fn is_available() -> bool {
        unimplemented!()
    }
}

