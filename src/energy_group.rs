use crate::trace_recorder::TraceRecorder;
use crate::utils::errors::MonitoringError;
use crate::utils::trace_rotation::RotatingTrace;
use async_trait::async_trait;
use polars::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub enum EnergyCollectorType {
    Rapl,
    NvidiaGpu,
    Dummy,
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
    /// Collection rate in Hz
    rate: f64,
    /// Number of iterations to batch before sending data back from the collector
    batch_size: usize,
    /// Rotating trace: pid | timestamp | device | energy
    energy_trace: RotatingTrace,
    /// Underlying collector instance
    energy_collector: Arc<T>,
    /// Flag indicating if the collector is running
    is_running: Arc<AtomicBool>,
    /// Handle to the background monitoring task
    task_handle: Option<JoinHandle<()>>,
    /// Receiver for collected energy data from the background task
    data_receiver: Option<mpsc::Receiver<Vec<EnergyRecord>>>,
    /// Per-PID cumulative energy accumulator
    consumed_energy: HashMap<u32, f64>,
    /// Registered trace recorders for persistent storage
    recorders: Vec<Box<dyn TraceRecorder>>,
    /// Cadence for periodic trace recorder flushes.
    recorder_flush_interval: Duration,
    /// Last time registered trace recorders were flushed.
    last_recorder_flush: Instant,
}

impl<T: EnergyCollector> EnergyGroup<T> {
    /// Create a new EnergyGroup with an explicit collector instance
    pub fn new(collector: T, rate: f64, batch_size: Option<usize>) -> Self {
        // Create rotating trace with 1 hour default retention
        let energy_trace = RotatingTrace::new(3600);

        Self {
            rate,
            batch_size: batch_size.unwrap_or(1000),
            energy_trace,
            energy_collector: Arc::new(collector),
            is_running: Arc::new(AtomicBool::new(false)),
            task_handle: None,
            data_receiver: None,
            consumed_energy: HashMap::new(),
            recorders: Vec::new(),
            recorder_flush_interval: Duration::from_secs(5),
            last_recorder_flush: Instant::now(),
        }
    }

    /// Update the tracked PIDs by delegating to the collector.
    pub fn update_tracked_pids(&self, pids: Vec<u32>) {
        self.energy_collector.set_tracked_pids(pids);
    }

    /// Set the tracked PIDs by delegating to the collector.
    pub fn set_tracked_pids(&self, pids: Vec<u32>) {
        self.update_tracked_pids(pids);
    }

    /// Register a trace recorder for persistent storage of energy data.
    pub fn add_recorder(&mut self, recorder: Box<dyn TraceRecorder>) {
        self.recorders.push(recorder);
    }

    /// Set the cadence for periodic trace recorder flushes.
    pub fn set_recorder_flush_interval(&mut self, interval: Duration) {
        self.recorder_flush_interval = interval;
    }

    /// Get a reference to the energy trace data (as DataFrame)
    pub fn energy_trace(&self) -> &DataFrame {
        self.energy_trace.data()
    }

    /// Get a mutable reference to the energy trace for advanced operations
    pub fn energy_trace_mut(&mut self) -> &mut RotatingTrace {
        &mut self.energy_trace
    }

    /// Set the retention window for all traces (in seconds)
    pub fn set_trace_retention(&mut self, retention_seconds: i64) {
        self.energy_trace.set_retention_seconds(retention_seconds);
    }

    /// Get memory usage statistics for energy trace
    pub fn trace_stats(&self) -> TraceMemoryStats {
        TraceMemoryStats {
            energy_trace_rows: self.energy_trace.row_count(),
            energy_trace_stats: self.energy_trace.stats(),
        }
    }

    /// Get the per-PID cumulative energy accumulator
    pub fn consumed_energy_by_pid(&self) -> &HashMap<u32, f64> {
        &self.consumed_energy
    }

    /// Get total consumed energy across all tracked PIDs
    pub fn total_consumed_energy(&self) -> f64 {
        self.consumed_energy.values().sum()
    }

    /// Add energy records to the energy trace
    fn append_energy_records(&mut self, records: &[EnergyRecord]) -> Result<(), MonitoringError> {
        if records.is_empty() {
            return Ok(());
        }

        let data = DataFrame::new(vec![
            Column::new(
                "pid".into(),
                records.iter().map(|r| r.pid).collect::<Vec<_>>(),
            ),
            Column::new(
                "device".into(),
                records.iter().map(|r| r.device.clone()).collect::<Vec<_>>(),
            ),
            Column::new(
                "energy".into(),
                records.iter().map(|r| r.energy).collect::<Vec<_>>(),
            ),
            Column::new(
                "timestamp".into(),
                records.iter().map(|r| r.timestamp).collect::<Vec<_>>(),
            ),
        ])
        .map_err(|err| MonitoringError::Other(err.to_string()))?;

        self.energy_trace.append(&data)?;

        Ok(())
    }

    /// Accumulate energy records into the per-PID HashMap
    fn accumulate_energy(&mut self, records: &[EnergyRecord]) {
        for record in records {
            *self.consumed_energy.entry(record.pid).or_insert(0.0) += record.energy;
        }
    }

    fn flush_recorders(&mut self) {
        for recorder in &mut self.recorders {
            recorder.flush(&self.energy_trace);
        }
        self.last_recorder_flush = Instant::now();
    }

    fn flush_recorders_if_due(&mut self) {
        if self.recorders.is_empty() {
            return;
        }

        if self.last_recorder_flush.elapsed() >= self.recorder_flush_interval {
            self.flush_recorders();
        }
    }

    /// Check if the underlying collector is available on the system
    pub fn is_available() -> bool {
        T::is_available()
    }

    /// Check if the collector is currently running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Background monitoring task that collects data at a specified rate and sends batches
    async fn run_monitoring_loop<C: EnergyCollector>(
        collector: Arc<C>,
        tx: mpsc::Sender<Vec<EnergyRecord>>,
        is_monitoring_active: Arc<AtomicBool>,
        rate: f64,
        batch_size: usize,
    ) {
        let interval = tokio::time::Duration::from_secs_f64(1.0 / rate);
        let mut iteration = 0;
        let mut collected_energy_records = Vec::new();

        while is_monitoring_active.load(Ordering::SeqCst) {
            iteration += 1;
            log::trace!("Background monitoring iteration {}", iteration);

            match collector.get_energy_trace().await {
                Ok(energy_records) => {
                    log::debug!("Collected {} energy records", energy_records.len(),);

                    // Add to batch
                    collected_energy_records.extend(energy_records);

                    // Send batch when it reaches the batch size
                    if iteration % batch_size == 0 {
                        log::debug!(
                            "Sending batch of {} energy records",
                            collected_energy_records.len(),
                        );

                        // Use send().await for bounded channel (provides backpressure)
                        // This will wait if the channel is full, slowing down collection
                        let send_start = std::time::Instant::now();
                        match tx.send(collected_energy_records.clone()).await {
                            Ok(_) => {
                                let send_duration = send_start.elapsed();
                                if send_duration.as_millis() > 100 {
                                    log::warn!(
                                        "Channel send blocked for {:?} - receiver may be slow!",
                                        send_duration
                                    );
                                }
                            }
                            Err(_) => {
                                log::error!("Failed to send data - receiver dropped");
                                break;
                            }
                        }

                        // Clear the batch
                        collected_energy_records.clear();
                    }
                }
                Err(e) => {
                    log::error!("Error collecting data: {}", e);
                }
            }

            tokio::time::sleep(interval).await;
        }

        // Send any remaining records in the batch before stopping
        if !collected_energy_records.is_empty() {
            log::debug!(
                "Sending final batch of {} energy records",
                collected_energy_records.len(),
            );
            let _ = tx.send(collected_energy_records).await;
        }

        log::debug!(
            "Background monitoring stopped after {} iterations",
            iteration
        );
    }

    pub async fn commence(&mut self) -> Result<(), MonitoringError> {
        // Check if collector is already running
        if self.is_running() {
            eprintln!("Warning: Energy collector is already running. Ignoring commence request.");
            return Ok(());
        }

        if !T::is_available() {
            return Err(MonitoringError::Other(
                "Collector type is not available on this system".to_string(),
            ));
        }

        // Set running state before starting
        self.is_running.store(true, Ordering::SeqCst);

        // Collect initial energy data
        let energy_records = self
            .energy_collector
            .get_energy_trace()
            .await
            .map_err(|e| MonitoringError::Other(format!("Failed to get energy trace: {}", e)))?;

        // Append and accumulate initial data
        self.append_energy_records(&energy_records)?;
        self.accumulate_energy(&energy_records);

        // Create bounded channel for background task to send data back
        // Channel capacity: allow a reasonable buffer (e.g., 10 batches)
        // This provides backpressure if receiver is slow
        let channel_capacity = 10;
        let (tx, rx) = mpsc::channel(channel_capacity);
        self.data_receiver = Some(rx);

        // Spawn background task for continuous monitoring
        let rate = self.rate;
        let batch_size = self.batch_size;
        let is_running = Arc::clone(&self.is_running);
        let collector = Arc::clone(&self.energy_collector);

        let handle = tokio::spawn(Self::run_monitoring_loop(
            collector, tx, is_running, rate, batch_size,
        ));

        // Store the task handle
        self.task_handle = Some(handle);

        log::info!("Monitoring started in background at {} Hz", rate);
        Ok(())
    }

    /// Poll the channel, append received data to the energy trace, and accumulate per-PID energy.
    /// Returns all energy records drained from the channel.
    pub fn poll_data(&mut self) -> Vec<EnergyRecord> {
        // Collect all available messages first
        let mut all_energy_records = Vec::new();

        if let Some(rx) = &mut self.data_receiver {
            while let Ok(energy_records) = rx.try_recv() {
                all_energy_records.extend(energy_records);
            }
        }

        // Append to trace and accumulate
        if !all_energy_records.is_empty() {
            if let Err(e) = self.append_energy_records(&all_energy_records) {
                log::error!("Failed to append energy records to trace: {}", e);
            }
            self.accumulate_energy(&all_energy_records);
            self.flush_recorders_if_due();
        }

        all_energy_records
    }

    pub fn shutdown(&mut self) -> Result<(), MonitoringError> {
        self.shutdown_and_drain().map(|_| ())
    }

    /// Shut down the collector and return all final records drained from the channel.
    pub fn shutdown_and_drain(&mut self) -> Result<Vec<EnergyRecord>, MonitoringError> {
        log::info!("Shutdown requested");

        // if not running, nothing to do
        if !self.is_running() {
            log::info!("Collector is not running, nothing to shut down");
            return Ok(Vec::new());
        }

        // Signal the background task to stop
        self.is_running.store(false, Ordering::SeqCst);

        // Give the background task time to send its final batch
        // This is necessary because the task may be in the middle of collecting data
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Poll any remaining data from the channel
        let final_records = self.poll_data();

        // Final flush to all registered recorders
        self.flush_recorders();

        // Now abort the background task (it should already be stopped)
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }

        // Drop the receiver to signal completion
        self.data_receiver = None;
        Ok(final_records)
    }
}

#[async_trait]
pub trait EnergyCollector: Send + Sync + 'static {
    /// Set the list of tracked process PIDs for energy attribution
    fn set_tracked_pids(&self, pids: Vec<u32>);

    /// Get energy trace data
    async fn get_energy_trace(&self) -> Result<Vec<EnergyRecord>, String>;

    /// Check if this collector type is available on the system
    fn is_available() -> bool {
        unimplemented!()
    }
}

/// Statistics about trace memory usage
#[derive(Debug, Clone)]
pub struct TraceMemoryStats {
    /// Number of rows in energy trace
    pub energy_trace_rows: usize,
    /// Energy trace statistics
    pub energy_trace_stats: crate::utils::trace_rotation::TraceStats,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::trace_rotation::RotatingTrace;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingRecorder {
        flush_count: Arc<AtomicUsize>,
    }

    impl TraceRecorder for CountingRecorder {
        fn flush(&mut self, _trace: &RotatingTrace) {
            self.flush_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct TestCollector {
        pids: Mutex<Vec<u32>>,
        sequence: AtomicUsize,
    }

    impl TestCollector {
        fn new(pid: u32) -> Self {
            Self {
                pids: Mutex::new(vec![pid]),
                sequence: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl EnergyCollector for TestCollector {
        fn set_tracked_pids(&self, pids: Vec<u32>) {
            *self.pids.lock().unwrap() = pids;
        }

        async fn get_energy_trace(&self) -> Result<Vec<EnergyRecord>, String> {
            let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) as f64;
            let pids = self.pids.lock().unwrap().clone();
            Ok(pids
                .into_iter()
                .map(|pid| EnergyRecord {
                    pid,
                    timestamp: sequence as i64,
                    device: "test:device".to_string(),
                    energy: 1.0 + sequence,
                })
                .collect())
        }

        fn is_available() -> bool {
            true
        }
    }

    #[test]
    fn update_tracked_pids_delegates_to_collector() {
        let group = EnergyGroup::new(TestCollector::new(123), 50.0, Some(1));

        group.update_tracked_pids(vec![456, 789]);
        assert_eq!(*group.energy_collector.pids.lock().unwrap(), vec![456, 789]);

        group.set_tracked_pids(vec![321]);
        assert_eq!(*group.energy_collector.pids.lock().unwrap(), vec![321]);
    }

    #[tokio::test]
    async fn poll_data_flushes_recorders_when_cadence_is_due() {
        let flush_count = Arc::new(AtomicUsize::new(0));
        let mut group = EnergyGroup::new(TestCollector::new(123), 50.0, Some(1));
        group.set_recorder_flush_interval(Duration::from_secs(0));
        group.add_recorder(Box::new(CountingRecorder {
            flush_count: Arc::clone(&flush_count),
        }));

        group.commence().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let records = group.poll_data();

        assert!(!records.is_empty());
        assert!(flush_count.load(Ordering::SeqCst) >= 1);

        group.shutdown().unwrap();
    }

    #[tokio::test]
    async fn shutdown_and_drain_returns_final_records_and_flushes() {
        let flush_count = Arc::new(AtomicUsize::new(0));
        let mut group = EnergyGroup::new(TestCollector::new(456), 50.0, Some(1));
        group.add_recorder(Box::new(CountingRecorder {
            flush_count: Arc::clone(&flush_count),
        }));

        group.commence().await.unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        let final_records = group.shutdown_and_drain().unwrap();

        assert!(!final_records.is_empty());
        assert_eq!(flush_count.load(Ordering::SeqCst), 1);
    }
}
