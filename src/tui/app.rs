use crate::metrics_sink::MetricsSink;
use crate::monitor::{DeviceEnergy, MetricsSnapshot, MonitorHandle};
use crate::process_aggregation::percentage_of_system;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::Instant;

const POWER_HISTORY_WINDOW_SECS: f64 = 60.0;
const POWER_HISTORY_MAX_SAMPLES: usize = 120;

pub struct App {
    handle: MonitorHandle,
    start_time: Instant,
    sink: TuiSink,
    state: AppState,
    pub should_quit: bool,
}

impl App {
    pub fn new(handle: MonitorHandle) -> Self {
        Self {
            handle,
            start_time: Instant::now(),
            sink: TuiSink::default(),
            state: AppState::default(),
            should_quit: false,
        }
    }

    pub fn refresh(&mut self) {
        let snapshot = self.handle.snapshot();
        self.sink.update(&snapshot);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        self.sink.display_snapshot(self.state.sort_mode)
    }

    pub fn uptime_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    pub fn display_elapsed_secs(&self) -> f64 {
        self.sink.display_elapsed_secs(self.uptime_secs())
    }

    pub fn power_history(&self) -> PowerHistorySnapshot {
        self.sink.power_history()
    }

    pub fn sort_mode(&self) -> SortMode {
        self.state.sort_mode
    }

    pub fn cycle_sort_mode(&mut self) {
        self.state.cycle_sort_mode();
    }

    pub fn reset_display(&mut self) {
        let snapshot = self.handle.snapshot();
        self.sink.update(&snapshot);
        self.sink.reset();
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

#[derive(Debug, Clone, Default)]
struct AppState {
    sort_mode: SortMode,
}

impl AppState {
    fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.next();
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortMode {
    #[default]
    Energy,
    Power,
    Name,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            Self::Energy => Self::Power,
            Self::Power => Self::Name,
            Self::Name => Self::Energy,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Energy => "energy",
            Self::Power => "power",
            Self::Name => "name",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TuiSink {
    snapshot: MetricsSnapshot,
    baseline: Option<ResetBaseline>,
    power_history: RollingPowerHistory,
}

impl TuiSink {
    pub fn snapshot(&self) -> &MetricsSnapshot {
        &self.snapshot
    }

    pub fn display_snapshot(&self, sort_mode: SortMode) -> MetricsSnapshot {
        let mut snapshot = self.baseline_adjusted_snapshot();
        sort_workloads(&mut snapshot.workloads, sort_mode);
        snapshot
    }

    pub fn power_history(&self) -> PowerHistorySnapshot {
        self.power_history.snapshot()
    }

    pub fn reset(&mut self) {
        if self.snapshot.timestamp > 0 {
            self.power_history.reset(Some((
                timestamp_to_secs(self.snapshot.timestamp),
                self.snapshot.system_total.clone(),
            )));
            self.baseline = Some(ResetBaseline::from(&self.snapshot));
        } else {
            self.power_history.reset(None);
            self.baseline = None;
        }
    }

    pub fn display_elapsed_secs(&self, fallback_secs: f64) -> f64 {
        self.baseline
            .as_ref()
            .and_then(|baseline| baseline.elapsed_secs(self.snapshot.timestamp))
            .unwrap_or(fallback_secs)
    }

    fn baseline_adjusted_snapshot(&self) -> MetricsSnapshot {
        let mut snapshot = self.snapshot.clone();
        let Some(baseline) = &self.baseline else {
            return snapshot;
        };

        snapshot.system_total = snapshot.system_total.saturating_sub(&baseline.system_total);
        snapshot.unattributed = snapshot.unattributed.saturating_sub(&baseline.unattributed);

        let elapsed_secs = baseline.elapsed_secs(snapshot.timestamp).unwrap_or(0.0);
        for workload in &mut snapshot.workloads {
            let baseline_energy = baseline
                .workloads
                .get(&workload.group_id)
                .cloned()
                .unwrap_or_default();
            workload.energy = workload.energy.saturating_sub(&baseline_energy);
            workload.power_watts = if elapsed_secs > f64::EPSILON {
                workload.energy.total() / elapsed_secs
            } else {
                0.0
            };
            workload.percentage_of_system =
                percentage_of_system(&workload.energy, &snapshot.system_total);
        }

        snapshot
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

#[derive(Debug, Clone)]
struct ResetBaseline {
    timestamp: i64,
    system_total: DeviceEnergy,
    unattributed: DeviceEnergy,
    workloads: HashMap<String, DeviceEnergy>,
}

impl ResetBaseline {
    fn elapsed_secs(&self, timestamp: i64) -> Option<f64> {
        let elapsed_secs = timestamp_to_secs(timestamp) - timestamp_to_secs(self.timestamp);
        if elapsed_secs.is_finite() && elapsed_secs >= 0.0 {
            Some(elapsed_secs)
        } else {
            None
        }
    }
}

impl From<&MetricsSnapshot> for ResetBaseline {
    fn from(snapshot: &MetricsSnapshot) -> Self {
        Self {
            timestamp: snapshot.timestamp,
            system_total: snapshot.system_total.clone(),
            unattributed: snapshot.unattributed.clone(),
            workloads: snapshot
                .workloads
                .iter()
                .map(|workload| (workload.group_id.clone(), workload.energy.clone()))
                .collect(),
        }
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

    fn reset(&mut self, previous: Option<(f64, DeviceEnergy)>) {
        self.previous = previous
            .filter(|(at_secs, _)| at_secs.is_finite() && *at_secs >= 0.0)
            .map(|(at_secs, energy)| EnergySample { at_secs, energy });
        self.cpu.clear();
        self.dram.clear();
        self.gpu.clear();
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

fn timestamp_to_secs(timestamp: i64) -> f64 {
    timestamp as f64 / 1_000.0
}

fn sort_workloads(workloads: &mut [crate::monitor::WorkloadSnapshot], sort_mode: SortMode) {
    workloads.sort_by(|left, right| match sort_mode {
        SortMode::Energy => compare_desc(right.energy.total(), left.energy.total())
            .then_with(|| compare_names(left, right)),
        SortMode::Power => compare_desc(right.power_watts, left.power_watts)
            .then_with(|| compare_names(left, right)),
        SortMode::Name => compare_names(left, right),
    });
}

fn compare_desc(left: f64, right: f64) -> Ordering {
    left.total_cmp(&right)
}

fn compare_names(
    left: &crate::monitor::WorkloadSnapshot,
    right: &crate::monitor::WorkloadSnapshot,
) -> Ordering {
    left.name
        .cmp(&right.name)
        .then_with(|| left.group_id.cmp(&right.group_id))
        .then_with(|| left.root_pid.cmp(&right.root_pid))
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
    use crate::monitor::WorkloadSnapshot;

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

    fn workload(group_id: &str, name: &str, energy: DeviceEnergy) -> WorkloadSnapshot {
        WorkloadSnapshot {
            root_pid: 123,
            group_id: group_id.to_string(),
            name: name.to_string(),
            user: "user".to_string(),
            energy,
            power_watts: 0.0,
            percentage_of_system: 0.0,
        }
    }

    #[test]
    fn app_state_cycles_sort_modes_in_expected_order() {
        let mut state = AppState::default();

        assert_eq!(state.sort_mode, SortMode::Energy);
        state.cycle_sort_mode();
        assert_eq!(state.sort_mode, SortMode::Power);
        state.cycle_sort_mode();
        assert_eq!(state.sort_mode, SortMode::Name);
        state.cycle_sort_mode();
        assert_eq!(state.sort_mode, SortMode::Energy);
    }

    #[test]
    fn tui_sink_sorts_workloads_by_selected_mode() {
        let mut sink = TuiSink::default();
        let mut snapshot = snapshot(1_000, energy(8.0, 0.0, 0.0));
        snapshot.workloads = vec![
            WorkloadSnapshot {
                power_watts: 4.0,
                ..workload("group-b", "beta", energy(2.0, 0.0, 0.0))
            },
            WorkloadSnapshot {
                power_watts: 1.0,
                ..workload("group-a", "alpha", energy(5.0, 0.0, 0.0))
            },
            WorkloadSnapshot {
                power_watts: 9.0,
                ..workload("group-c", "gamma", energy(1.0, 0.0, 0.0))
            },
        ];
        sink.update(&snapshot);

        assert_eq!(
            workload_names(&sink.display_snapshot(SortMode::Energy)),
            vec!["alpha", "beta", "gamma"]
        );
        assert_eq!(
            workload_names(&sink.display_snapshot(SortMode::Power)),
            vec!["gamma", "beta", "alpha"]
        );
        assert_eq!(
            workload_names(&sink.display_snapshot(SortMode::Name)),
            vec!["alpha", "beta", "gamma"]
        );
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
    fn reset_zeroes_display_counters_and_preserves_workload_groups() {
        let mut sink = TuiSink::default();
        let mut before_reset = snapshot(2_000, energy(15.0, 6.0, 1.0));
        before_reset.workloads = vec![
            WorkloadSnapshot {
                root_pid: 101,
                power_watts: 5.0,
                percentage_of_system: 60.0,
                ..workload("pid:101", "compile", energy(10.0, 2.0, 1.0))
            },
            WorkloadSnapshot {
                root_pid: 202,
                power_watts: 3.0,
                percentage_of_system: 40.0,
                ..workload("pid:202", "test", energy(5.0, 4.0, 0.0))
            },
        ];
        sink.update(&snapshot(1_000, energy(10.0, 3.0, 0.0)));
        sink.update(&before_reset);

        assert!(sink.power_history().has_samples());
        sink.reset();

        let display = sink.display_snapshot(SortMode::Energy);
        assert_eq!(display.workloads.len(), 2);
        assert_eq!(
            display
                .workloads
                .iter()
                .map(|workload| workload.group_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pid:101", "pid:202"]
        );
        assert_eq!(display.system_total.total(), 0.0);
        assert!(display.workloads.iter().all(|workload| {
            workload.energy.total() == 0.0
                && workload.power_watts == 0.0
                && workload.percentage_of_system == 0.0
        }));
        assert!(!sink.power_history().has_samples());

        let mut after_reset = snapshot(4_000, energy(20.0, 7.0, 2.0));
        after_reset.workloads = vec![
            WorkloadSnapshot {
                root_pid: 101,
                power_watts: 6.0,
                ..workload("pid:101", "compile", energy(12.0, 4.0, 2.0))
            },
            WorkloadSnapshot {
                root_pid: 202,
                power_watts: 4.0,
                ..workload("pid:202", "test", energy(8.0, 3.0, 0.0))
            },
        ];
        sink.update(&after_reset);

        let display = sink.display_snapshot(SortMode::Name);
        assert_energy(&display.workloads[0].energy, &energy(2.0, 2.0, 1.0));
        assert_eq!(display.workloads[0].power_watts, 2.5);
        assert_energy(&display.workloads[1].energy, &energy(3.0, 0.0, 0.0));
        assert_eq!(display.workloads[1].power_watts, 1.5);
        assert_energy(&display.system_total, &energy(5.0, 1.0, 1.0));
        assert_eq!(sink.power_history().cpu, vec![2.5]);
    }

    #[test]
    fn reset_before_first_timestamp_does_not_use_epoch_sized_display_window() {
        let mut sink = TuiSink::default();

        sink.reset();
        sink.update(&snapshot(1_700_000_000_000, energy(10.0, 0.0, 0.0)));

        assert_eq!(sink.display_elapsed_secs(2.0), 2.0);
        assert_eq!(
            sink.display_snapshot(SortMode::Energy)
                .system_total
                .cpu_joules,
            10.0
        );
        assert!(sink.power_history().cpu.is_empty());
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

    fn workload_names(snapshot: &MetricsSnapshot) -> Vec<&str> {
        snapshot
            .workloads
            .iter()
            .map(|workload| workload.name.as_str())
            .collect()
    }

    fn assert_energy(actual: &DeviceEnergy, expected: &DeviceEnergy) {
        assert_eq!(actual.cpu_joules, expected.cpu_joules);
        assert_eq!(actual.dram_joules, expected.dram_joules);
        assert_eq!(actual.gpu_joules, expected.gpu_joules);
    }
}
