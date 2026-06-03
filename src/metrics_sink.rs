use crate::monitor::{DeviceEnergy, MetricsSnapshot, WorkloadSnapshot};
use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use prometheus::core::{Collector, Desc};
use prometheus::proto::{Counter, Gauge, LabelPair, Metric, MetricFamily, MetricType};
use prometheus::{Encoder, Registry, TextEncoder};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

const SOCKET_LABEL: &str = "0";
const ENERGY_METRIC: &str = "emt_energy_joules_total";
const POWER_METRIC: &str = "emt_power_watts";
const ENERGY_HELP: &str = "Cumulative EMT energy attribution in joules.";
const POWER_HELP: &str = "EMT attributed power in watts.";

pub type SharedPrometheusSink = Arc<Mutex<PrometheusSink>>;

/// Receives point-in-time monitor snapshots and exports them to another system.
pub trait MetricsSink {
    fn update(&mut self, snapshot: &MetricsSnapshot);
}

/// Prometheus-backed sink for EMT monitor snapshots.
///
/// The sink owns a registry and registers one internal collector. It is ready to
/// be wired into a future HTTP handler by calling [`PrometheusSink::gather`] or
/// [`PrometheusSink::encode_text`].
pub struct PrometheusSink {
    registry: Registry,
    state: Arc<Mutex<PrometheusState>>,
}

impl PrometheusSink {
    pub fn new() -> Result<Self, prometheus::Error> {
        Self::with_registry(Registry::new())
    }

    pub fn with_registry(registry: Registry) -> Result<Self, prometheus::Error> {
        let state = Arc::new(Mutex::new(PrometheusState::default()));
        registry.register(Box::new(SnapshotCollector {
            state: Arc::clone(&state),
        }))?;

        Ok(Self { registry, state })
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn gather(&self) -> Vec<MetricFamily> {
        self.registry.gather()
    }

    pub fn encode_text(&self) -> Result<String, prometheus::Error> {
        TextEncoder::new().encode_to_string(&self.gather())
    }
}

impl MetricsSink for PrometheusSink {
    fn update(&mut self, snapshot: &MetricsSnapshot) {
        self.state.lock_unpoisoned().update(snapshot);
    }
}

pub fn prometheus_router(sink: SharedPrometheusSink) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/health", get(health_handler))
        .with_state(sink)
}

async fn health_handler() -> StatusCode {
    StatusCode::OK
}

async fn metrics_handler(State(sink): State<SharedPrometheusSink>) -> Response {
    let encoded = sink
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .encode_text();

    match encoded {
        Ok(body) => (
            [(
                header::CONTENT_TYPE,
                TextEncoder::new().format_type().to_string(),
            )],
            body,
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to encode metrics: {error}"),
        )
            .into_response(),
    }
}

#[derive(Debug, Default)]
struct PrometheusState {
    previous: Option<PreviousSnapshot>,
    energy_samples: Vec<MetricSample>,
    power_samples: Vec<MetricSample>,
}

impl PrometheusState {
    fn update(&mut self, snapshot: &MetricsSnapshot) {
        let previous = self.previous.as_ref();

        self.energy_samples = energy_samples(snapshot);
        self.power_samples = power_samples(snapshot, previous);
        self.previous = Some(PreviousSnapshot::from(snapshot));
    }

    fn collect(&self) -> Vec<MetricFamily> {
        vec![
            metric_family(
                ENERGY_METRIC,
                ENERGY_HELP,
                MetricType::COUNTER,
                &self.energy_samples,
            ),
            metric_family(
                POWER_METRIC,
                POWER_HELP,
                MetricType::GAUGE,
                &self.power_samples,
            ),
        ]
    }
}

#[derive(Debug, Clone)]
struct PreviousSnapshot {
    timestamp: i64,
    system_total: DeviceEnergy,
    workloads: HashMap<String, DeviceEnergy>,
}

impl From<&MetricsSnapshot> for PreviousSnapshot {
    fn from(snapshot: &MetricsSnapshot) -> Self {
        Self {
            timestamp: snapshot.timestamp,
            system_total: snapshot.system_total.clone(),
            workloads: snapshot
                .workloads
                .iter()
                .map(|workload| (workload_label(workload), workload.energy.clone()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct MetricSample {
    value: f64,
    labels: Vec<(&'static str, String)>,
}

#[derive(Clone)]
struct SnapshotCollector {
    state: Arc<Mutex<PrometheusState>>,
}

impl Collector for SnapshotCollector {
    fn desc(&self) -> Vec<&Desc> {
        Vec::new()
    }

    fn collect(&self) -> Vec<MetricFamily> {
        self.state.lock_unpoisoned().collect()
    }
}

trait LockUnpoisoned<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T>;
}

impl<T> LockUnpoisoned<T> for Mutex<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn energy_samples(snapshot: &MetricsSnapshot) -> Vec<MetricSample> {
    let mut samples = device_samples("system", None, &snapshot.system_total);

    for workload in &snapshot.workloads {
        let label = workload_label(workload);
        samples.extend(device_samples(
            "workload",
            Some(label.as_str()),
            &workload.energy,
        ));
    }

    samples
}

fn power_samples(
    snapshot: &MetricsSnapshot,
    previous: Option<&PreviousSnapshot>,
) -> Vec<MetricSample> {
    let Some(previous) = previous else {
        return zero_power_samples(snapshot);
    };

    let elapsed_seconds = (snapshot.timestamp - previous.timestamp) as f64 / 1_000.0;
    if elapsed_seconds <= f64::EPSILON {
        return zero_power_samples(snapshot);
    }

    let system_power = snapshot.system_total.saturating_sub(&previous.system_total);
    let mut samples = device_power_samples("system", None, &system_power, elapsed_seconds);

    for workload in &snapshot.workloads {
        let label = workload_label(workload);
        let power = previous
            .workloads
            .get(&label)
            .map(|previous_energy| workload.energy.saturating_sub(previous_energy))
            .unwrap_or_default();

        samples.extend(device_power_samples(
            "workload",
            Some(label.as_str()),
            &power,
            elapsed_seconds,
        ));
    }

    samples
}

fn zero_power_samples(snapshot: &MetricsSnapshot) -> Vec<MetricSample> {
    let zero = DeviceEnergy::default();
    let mut samples = device_power_samples("system", None, &zero, 1.0);

    for workload in &snapshot.workloads {
        let label = workload_label(workload);
        samples.extend(device_power_samples(
            "workload",
            Some(label.as_str()),
            &zero,
            1.0,
        ));
    }

    samples
}

fn device_power_samples(
    scope: &'static str,
    workload: Option<&str>,
    energy_delta: &DeviceEnergy,
    elapsed_seconds: f64,
) -> Vec<MetricSample> {
    device_samples(
        scope,
        workload,
        &DeviceEnergy {
            cpu_joules: energy_delta.cpu_joules / elapsed_seconds,
            dram_joules: energy_delta.dram_joules / elapsed_seconds,
            gpu_joules: energy_delta.gpu_joules / elapsed_seconds,
        },
    )
}

fn device_samples(
    scope: &'static str,
    workload: Option<&str>,
    energy: &DeviceEnergy,
) -> Vec<MetricSample> {
    vec![
        metric_sample(scope, workload, "cpu", energy.cpu_joules),
        metric_sample(scope, workload, "dram", energy.dram_joules),
        metric_sample(scope, workload, "gpu", energy.gpu_joules),
    ]
}

fn metric_sample(
    scope: &'static str,
    workload: Option<&str>,
    device: &'static str,
    value: f64,
) -> MetricSample {
    let mut labels = vec![
        ("scope", scope.to_string()),
        ("device", device.to_string()),
        ("socket", SOCKET_LABEL.to_string()),
    ];

    if let Some(workload) = workload {
        labels.push(("workload", workload.to_string()));
    }

    MetricSample { value, labels }
}

fn workload_label(workload: &WorkloadSnapshot) -> String {
    if workload.name.is_empty() {
        workload.group_id.clone()
    } else {
        workload.name.clone()
    }
}

fn metric_family(
    name: &str,
    help: &str,
    metric_type: MetricType,
    samples: &[MetricSample],
) -> MetricFamily {
    let mut family = MetricFamily::default();
    family.set_name(name.to_string());
    family.set_help(help.to_string());
    family.set_field_type(metric_type);
    family.set_metric(
        samples
            .iter()
            .map(|sample| metric(metric_type, sample))
            .collect(),
    );
    family
}

fn metric(metric_type: MetricType, sample: &MetricSample) -> Metric {
    let mut metric = Metric::from_label(label_pairs(&sample.labels));
    match metric_type {
        MetricType::COUNTER => {
            let mut counter = Counter::default();
            counter.set_value(sample.value);
            metric.set_counter(counter);
        }
        MetricType::GAUGE => {
            let mut gauge = Gauge::default();
            gauge.set_value(sample.value);
            metric.set_gauge(gauge);
        }
        _ => unreachable!("metrics sink only exports counters and gauges"),
    }
    metric
}

fn label_pairs(labels: &[(&'static str, String)]) -> Vec<LabelPair> {
    let mut pairs: Vec<LabelPair> = labels
        .iter()
        .map(|(name, value)| {
            let mut label_pair = LabelPair::default();
            label_pair.set_name((*name).to_string());
            label_pair.set_value(value.clone());
            label_pair
        })
        .collect();
    pairs.sort();
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    #[test]
    fn prometheus_sink_exports_energy_and_power_for_system_and_workloads() {
        let mut sink = PrometheusSink::new().unwrap();
        sink.update(&snapshot(
            1_000,
            DeviceEnergy {
                cpu_joules: 10.0,
                dram_joules: 4.0,
                gpu_joules: 2.0,
            },
            DeviceEnergy {
                cpu_joules: 2.0,
                dram_joules: 1.0,
                gpu_joules: 0.0,
            },
        ));
        sink.update(&snapshot(
            3_000,
            DeviceEnergy {
                cpu_joules: 16.0,
                dram_joules: 10.0,
                gpu_joules: 10.0,
            },
            DeviceEnergy {
                cpu_joules: 4.0,
                dram_joules: 3.0,
                gpu_joules: 6.0,
            },
        ));

        let exposition = sink.encode_text().unwrap();

        assert!(exposition.contains("# HELP emt_energy_joules_total"));
        assert!(
            exposition.contains(
                "emt_energy_joules_total{device=\"cpu\",scope=\"system\",socket=\"0\"} 16"
            ),
            "{exposition}"
        );
        assert!(
            exposition.contains(
                "emt_energy_joules_total{device=\"gpu\",scope=\"workload\",socket=\"0\",workload=\"render\"} 6"
            ),
            "{exposition}"
        );
        assert!(
            exposition.contains("emt_power_watts{device=\"dram\",scope=\"system\",socket=\"0\"} 3"),
            "{exposition}"
        );
        assert!(
            exposition.contains(
                "emt_power_watts{device=\"cpu\",scope=\"workload\",socket=\"0\",workload=\"render\"} 1"
            ),
            "{exposition}"
        );
        assert!(!exposition.contains("root_pid"));
        assert!(!exposition.contains("pid=\""));
    }

    #[test]
    fn prometheus_sink_exports_zero_power_on_first_snapshot() {
        let mut sink = PrometheusSink::new().unwrap();
        sink.update(&snapshot(
            1_000,
            DeviceEnergy {
                cpu_joules: 1.0,
                dram_joules: 0.0,
                gpu_joules: 0.0,
            },
            DeviceEnergy {
                cpu_joules: 0.5,
                dram_joules: 0.0,
                gpu_joules: 0.0,
            },
        ));

        let exposition = sink.encode_text().unwrap();

        assert!(
            exposition.contains("emt_power_watts{device=\"cpu\",scope=\"system\",socket=\"0\"} 0"),
            "{exposition}"
        );
        assert!(
            exposition.contains(
                "emt_power_watts{device=\"cpu\",scope=\"workload\",socket=\"0\",workload=\"render\"} 0"
            ),
            "{exposition}"
        );
    }

    #[tokio::test]
    async fn prometheus_router_serves_metrics_and_health() {
        let sink = Arc::new(Mutex::new(PrometheusSink::new().unwrap()));
        sink.lock().unwrap().update(&snapshot(
            1_000,
            energy(1.0, 0.0, 0.0),
            energy(0.5, 0.0, 0.0),
        ));
        let app = prometheus_router(sink);

        let metrics_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(metrics_response.status(), StatusCode::OK);
        assert_eq!(
            metrics_response
                .headers()
                .get(header::CONTENT_TYPE)
                .unwrap(),
            TextEncoder::new().format_type()
        );
        let metrics_body = to_bytes(metrics_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let metrics_text = String::from_utf8(metrics_body.to_vec()).unwrap();
        assert!(metrics_text.contains("# TYPE emt_energy_joules_total counter"));
        assert!(metrics_text.contains("emt_energy_joules_total"));

        let health_response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health_response.status(), StatusCode::OK);
    }

    fn energy(cpu: f64, dram: f64, gpu: f64) -> DeviceEnergy {
        DeviceEnergy {
            cpu_joules: cpu,
            dram_joules: dram,
            gpu_joules: gpu,
        }
    }

    fn snapshot(
        timestamp: i64,
        system_total: DeviceEnergy,
        workload_energy: DeviceEnergy,
    ) -> MetricsSnapshot {
        MetricsSnapshot {
            timestamp,
            system_total,
            workloads: vec![WorkloadSnapshot {
                root_pid: 123,
                group_id: "group-a".to_string(),
                name: "render".to_string(),
                user: "user".to_string(),
                energy: workload_energy,
                power_watts: 0.0,
                percentage_of_system: 0.0,
            }],
            unattributed: DeviceEnergy::default(),
            tracked_pids: vec![123],
        }
    }
}
