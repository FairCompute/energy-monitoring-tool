use crate::metrics_sink::MetricsSink;
use crate::monitor::{DeviceEnergy, MetricsSnapshot, MonitorHandle};
use std::collections::VecDeque;
use std::time::Instant;

const POWER_HISTORY_WINDOW_SECS: f64 = 60.0;
const POWER_HISTORY_MAX_SAMPLES: usize = 120;

pub struct App {
    handle: MonitorHandle,
    start_time: Instant,
    sink: TuiSink,
    pub should_quit: bool,
}

impl App {
    pub fn new(handle: MonitorHandle) -> Self {
        Self {
            handle,
            start_time: Instant::now(),
            sink: TuiSink::default(),
            should_quit: false,
        }
    }

    pub fn refresh(&mut self) {
        let snapshot = self.handle.snapshot();
        self.sink.update(&snapshot);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        self.sink.snapshot().clone()
    }

    pub fn uptime_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    pub fn power_history(&self) -> PowerHistorySnapshot {
        self.sink.power_history()
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

#[derive(Debug, Clone, Default)]
pub struct TuiSink {
    snapshot: MetricsSnapshot,
    power_history: RollingPowerHistory,
}

impl TuiSink {
    pub fn snapshot(&self) -> &MetricsSnapshot {
        &self.snapshot
    }

    pub fn power_history(&self) -> PowerHistorySnapshot {
        self.power_history.snapshot()
    }
}

impl MetricsSink for TuiSink {
    fn update(&mut self, snapshot: &MetricsSnapshot) {
        if snapshot.timestamp > 0 {
            self.power_history
                .record(snapshot.timestamp as f64 / 1_000.0, &snapshot.system_total);
        }
        self.snapshot = snapshot.clone();
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PowerHistorySnapshot {
    pub cpu: Vec<f64>,
    pub dram: Vec<f64>,
    pub gpu: Vec<f64>,
}

impl PowerHistorySnapshot {
    pub fn has_samples(&self) -> bool {
        !self.cpu.is_empty() || !self.dram.is_empty() || !self.gpu.is_empty()
    }

    pub fn latest_cpu(&self) -> Option<f64> {
        self.cpu.last().copied()
    }

    pub fn latest_dram(&self) -> Option<f64> {
        self.dram.last().copied()
    }

    pub fn latest_gpu(&self) -> Option<f64> {
        self.gpu.last().copied()
    }
}

#[derive(Debug, Clone)]
struct RollingPowerHistory {
    window_secs: f64,
    max_samples: usize,
    previous: Option<EnergySample>,
    cpu: VecDeque<PowerSample>,
    dram: VecDeque<PowerSample>,
    gpu: VecDeque<PowerSample>,
}

impl Default for RollingPowerHistory {
    fn default() -> Self {
        Self::new(POWER_HISTORY_WINDOW_SECS, POWER_HISTORY_MAX_SAMPLES)
    }
}

impl RollingPowerHistory {
    fn new(window_secs: f64, max_samples: usize) -> Self {
        Self {
            window_secs,
            max_samples,
            previous: None,
            cpu: VecDeque::new(),
            dram: VecDeque::new(),
            gpu: VecDeque::new(),
        }
    }

    fn record(&mut self, at_secs: f64, energy: &DeviceEnergy) {
        if !at_secs.is_finite() || at_secs < 0.0 {
            return;
        }

        let current = EnergySample {
            at_secs,
            energy: energy.clone(),
        };

        let Some(previous) = &self.previous else {
            self.previous = Some(current);
            return;
        };

        let elapsed_secs = at_secs - previous.at_secs;
        if elapsed_secs <= f64::EPSILON {
            return;
        }

        let delta = energy.saturating_sub(&previous.energy);
        Self::record_component(
            &mut self.cpu,
            at_secs,
            delta.cpu_joules,
            energy.cpu_joules,
            elapsed_secs,
        );
        Self::record_component(
            &mut self.dram,
            at_secs,
            delta.dram_joules,
            energy.dram_joules,
            elapsed_secs,
        );
        Self::record_component(
            &mut self.gpu,
            at_secs,
            delta.gpu_joules,
            energy.gpu_joules,
            elapsed_secs,
        );

        self.previous = Some(current);
        self.prune(at_secs);
    }

    fn snapshot(&self) -> PowerHistorySnapshot {
        PowerHistorySnapshot {
            cpu: Self::values(&self.cpu),
            dram: Self::values(&self.dram),
            gpu: Self::values(&self.gpu),
        }
    }

    fn record_component(
        samples: &mut VecDeque<PowerSample>,
        at_secs: f64,
        delta_joules: f64,
        cumulative_joules: f64,
        elapsed_secs: f64,
    ) {
        if cumulative_joules <= 0.0 && samples.is_empty() {
            return;
        }

        samples.push_back(PowerSample {
            at_secs,
            watts: (delta_joules / elapsed_secs).max(0.0),
        });
    }

    fn prune(&mut self, current_secs: f64) {
        Self::prune_component(
            &mut self.cpu,
            current_secs,
            self.window_secs,
            self.max_samples,
        );
        Self::prune_component(
            &mut self.dram,
            current_secs,
            self.window_secs,
            self.max_samples,
        );
        Self::prune_component(
            &mut self.gpu,
            current_secs,
            self.window_secs,
            self.max_samples,
        );
    }

    fn prune_component(
        samples: &mut VecDeque<PowerSample>,
        current_secs: f64,
        window_secs: f64,
        max_samples: usize,
    ) {
        while samples
            .front()
            .is_some_and(|sample| current_secs - sample.at_secs > window_secs)
        {
            samples.pop_front();
        }

        while samples.len() > max_samples {
            samples.pop_front();
        }
    }

    fn values(samples: &VecDeque<PowerSample>) -> Vec<f64> {
        samples.iter().map(|sample| sample.watts).collect()
    }
}

#[derive(Debug, Clone)]
struct EnergySample {
    at_secs: f64,
    energy: DeviceEnergy,
}

#[derive(Debug, Clone, Copy)]
struct PowerSample {
    at_secs: f64,
    watts: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(cpu: f64, dram: f64, gpu: f64) -> DeviceEnergy {
        DeviceEnergy {
            cpu_joules: cpu,
            dram_joules: dram,
            gpu_joules: gpu,
        }
    }

    fn snapshot(timestamp: i64, energy: DeviceEnergy) -> MetricsSnapshot {
        MetricsSnapshot {
            timestamp,
            system_total: energy,
            ..MetricsSnapshot::default()
        }
    }

    #[test]
    fn tui_sink_stores_latest_snapshot_and_power_history() {
        let mut sink = TuiSink::default();

        sink.update(&snapshot(1_000, energy(1.0, 0.0, 0.0)));
        sink.update(&snapshot(3_000, energy(11.0, 4.0, 0.0)));

        assert_eq!(sink.snapshot().system_total.cpu_joules, 11.0);
        assert_eq!(sink.power_history().cpu, vec![5.0]);
        assert_eq!(sink.power_history().dram, vec![2.0]);
    }

    #[test]
    fn records_power_from_cumulative_energy_deltas() {
        let mut history = RollingPowerHistory::new(60.0, 120);

        history.record(0.0, &energy(0.0, 0.0, 0.0));
        history.record(2.0, &energy(10.0, 2.0, 0.0));

        let snapshot = history.snapshot();
        assert_eq!(snapshot.cpu, vec![5.0]);
        assert_eq!(snapshot.dram, vec![1.0]);
        assert!(snapshot.gpu.is_empty());
    }

    #[test]
    fn records_zero_power_after_component_has_values() {
        let mut history = RollingPowerHistory::new(60.0, 120);

        history.record(0.0, &energy(0.0, 0.0, 0.0));
        history.record(1.0, &energy(2.0, 0.0, 0.0));
        history.record(2.0, &energy(2.0, 0.0, 0.0));

        assert_eq!(history.snapshot().cpu, vec![2.0, 0.0]);
    }

    #[test]
    fn keeps_component_history_bounded_by_sample_count() {
        let mut history = RollingPowerHistory::new(60.0, 3);

        history.record(0.0, &energy(0.0, 0.0, 0.0));
        for sample in 1..=5 {
            history.record(sample as f64, &energy(sample as f64, 0.0, 0.0));
        }

        assert_eq!(history.snapshot().cpu, vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn prunes_component_history_outside_window() {
        let mut history = RollingPowerHistory::new(2.0, 120);

        history.record(0.0, &energy(0.0, 0.0, 0.0));
        for sample in 1..=4 {
            history.record(sample as f64, &energy(sample as f64, 0.0, 0.0));
        }

        assert_eq!(history.snapshot().cpu, vec![1.0, 1.0, 1.0]);
        assert_eq!(history.cpu.front().map(|sample| sample.at_secs), Some(2.0));
    }

    #[test]
    fn ignores_non_advancing_timestamps() {
        let mut history = RollingPowerHistory::new(60.0, 120);

        history.record(1.0, &energy(0.0, 0.0, 0.0));
        history.record(1.0, &energy(5.0, 0.0, 0.0));
        history.record(0.5, &energy(10.0, 0.0, 0.0));

        assert!(history.snapshot().cpu.is_empty());
    }
}
