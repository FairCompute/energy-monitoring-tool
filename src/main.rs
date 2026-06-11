use clap::{Parser, ValueEnum};
use emt::config::{EmtConfig, MeasurementUnitsConfig};
use emt::metrics_sink::{MetricsSink, PrometheusSink, SharedPrometheusSink, prometheus_router};
use emt::monitor::{
    DeviceEnergy, DeviceSources, MetricsSnapshot, Monitor, MonitorDiagnostics, MonitorHandle,
};
use emt::tui::{self, App};
use serde::Serialize;
use std::fs::File;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DEFAULT_BATCH_DURATION_SECS: u64 = 10;
const DEFAULT_PROMETHEUS_PORT: u16 = 9101;
const TUI_INPUT_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Parser, Debug)]
#[command(name = "emt")]
#[command(about = "Monitor energy consumption of processes")]
struct Args {
    /// Process ID to monitor (if not specified, monitors all root processes)
    #[arg(short, long)]
    pid: Option<u32>,

    /// Duration to monitor in seconds (JSON output mode only)
    #[arg(short, long)]
    duration: Option<u64>,

    /// Collection rate in Hz (overrides config file)
    #[arg(short, long)]
    rate: Option<f64>,

    /// Process scan interval in seconds (overrides config file)
    #[arg(long = "scan-interval")]
    scan_interval: Option<f64>,

    /// Write the final raw metrics snapshot to PATH
    #[arg(long = "snapshot-out", value_name = "PATH")]
    snapshot_out: Option<String>,

    /// Launch interactive TUI (default mode)
    #[arg(long, conflicts_with_all = ["headless", "json_out"])]
    tui: bool,

    /// Reserved for future daemon export modes
    #[arg(long, conflicts_with_all = ["tui", "json_out"])]
    headless: bool,

    /// Export sink for headless daemon mode
    #[arg(long, value_enum, requires = "headless")]
    export: Option<ExportMode>,

    /// TCP port for headless Prometheus export
    #[arg(long, default_value_t = DEFAULT_PROMETHEUS_PORT)]
    port: u16,

    /// Bind address for headless Prometheus export
    #[arg(long, default_value = "0.0.0.0")]
    bind: IpAddr,

    /// Run once and write JSON results to PATH
    #[arg(long = "json-out", value_name = "PATH", conflicts_with_all = ["tui", "headless"])]
    json_out: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ExportMode {
    Prometheus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Tui,
    Headless,
    JsonOut,
}

fn selected_mode(args: &Args) -> Mode {
    if args.json_out.is_some() {
        Mode::JsonOut
    } else if args.headless {
        Mode::Headless
    } else {
        Mode::Tui
    }
}

fn validate_args(args: &Args) -> Result<(), &'static str> {
    if args.duration.is_some() && selected_mode(args) != Mode::JsonOut {
        return Err("--duration can only be used with --json-out");
    }
    if selected_mode(args) == Mode::Headless && args.export != Some(ExportMode::Prometheus) {
        return Err("--headless requires --export prometheus");
    }
    Ok(())
}

fn batch_duration_seconds(args: &Args) -> u64 {
    args.duration.unwrap_or(DEFAULT_BATCH_DURATION_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use emt::monitor::{
        DeviceEnergy, DeviceSource, DeviceSources, ProcessEnergySnapshot, WorkloadSnapshot,
    };

    #[test]
    fn cli_output_uses_configured_units_and_unit_neutral_fields() {
        let args = Args {
            pid: Some(123),
            duration: Some(10),
            rate: None,
            scan_interval: None,
            snapshot_out: None,
            tui: false,
            headless: false,
            export: None,
            port: DEFAULT_PROMETHEUS_PORT,
            bind: "0.0.0.0".parse().unwrap(),
            json_out: Some("results.json".to_string()),
        };
        let units = MeasurementUnitsConfig {
            energy: "kWh".to_string(),
            power: "mW".to_string(),
        };
        let snapshot = MetricsSnapshot {
            timestamp: 0,
            gpu_available: false,
            sources: DeviceSources {
                cpu: DeviceSource::MeasuredPackage,
                dram: DeviceSource::Measured,
                gpu: DeviceSource::Unavailable,
            },
            system_total: DeviceEnergy {
                cpu_joules: 2_700.0,
                dram_joules: 900.0,
                gpu_joules: 0.0,
            },
            workloads: vec![WorkloadSnapshot {
                root_pid: 123,
                group_id: "pid:123".to_string(),
                name: "work".to_string(),
                user: "user".to_string(),
                processes: Vec::new(),
                is_live: true,
                energy: DeviceEnergy {
                    cpu_joules: 2_700.0,
                    dram_joules: 900.0,
                    gpu_joules: 0.0,
                },
                power_watts: 360.0,
                percentage_of_system: 100.0,
            }],
            unattributed: DeviceEnergy::default(),
            tracked_pids: vec![123],
            ..MetricsSnapshot::default()
        };

        let output = build_cli_output(&args, 10.0, &snapshot, &units);

        assert_eq!(output.energy_unit, "kWh");
        assert_eq!(output.power_unit, "mW");
        assert!((output.total_energy - 0.001).abs() < 1e-9);
        assert!((output.power - 360_000.0).abs() < 1e-9);
        assert!((output.devices.cpu - 0.00075).abs() < 1e-9);
        assert!((output.devices.dram.unwrap() - 0.00025).abs() < 1e-9);
        assert_eq!(output.workloads[0].root_pid, 123);
        assert_eq!(output.workloads[0].group_id, "pid:123");
        assert_eq!(output.workloads[0].name, "work");
        assert_eq!(output.workloads[0].user, "user");
        assert!((output.workloads[0].energy - 0.001).abs() < 1e-9);
        assert!((output.workloads[0].power - 360_000.0).abs() < 1e-9);
        assert!((output.workloads[0].percentage_of_system - 100.0).abs() < 1e-9);
    }

    #[test]
    fn cli_output_omits_dram_device_when_dram_is_included_in_package() {
        let args = Args {
            pid: Some(123),
            duration: Some(10),
            rate: None,
            scan_interval: None,
            snapshot_out: None,
            tui: false,
            headless: false,
            export: None,
            port: DEFAULT_PROMETHEUS_PORT,
            bind: "0.0.0.0".parse().unwrap(),
            json_out: Some("results.json".to_string()),
        };
        let snapshot = MetricsSnapshot {
            sources: DeviceSources {
                cpu: DeviceSource::MeasuredPackage,
                dram: DeviceSource::IncludedInPackage,
                gpu: DeviceSource::Unavailable,
            },
            system_total: DeviceEnergy {
                cpu_joules: 42.0,
                dram_joules: 0.0,
                gpu_joules: 0.0,
            },
            ..MetricsSnapshot::default()
        };

        let output = build_cli_output(&args, 10.0, &snapshot, &MeasurementUnitsConfig::default());
        let json = serde_json::to_string(&output).unwrap();

        assert!(output.devices.dram.is_none());
        assert!(json.contains("\"cpu\""));
        assert!(!json.contains("\"dram\""));
    }

    #[test]
    fn snapshot_output_omits_dram_joules_when_dram_is_not_measured() {
        let snapshot = MetricsSnapshot {
            sources: DeviceSources {
                cpu: DeviceSource::MeasuredPackage,
                dram: DeviceSource::IncludedInPackage,
                gpu: DeviceSource::Unavailable,
            },
            system_total: DeviceEnergy {
                cpu_joules: 42.0,
                dram_joules: 99.0,
                gpu_joules: 0.0,
            },
            workloads: vec![WorkloadSnapshot {
                root_pid: 123,
                group_id: "pid:123".to_string(),
                name: "work".to_string(),
                user: "user".to_string(),
                processes: vec![ProcessEnergySnapshot {
                    pid: 123,
                    name: "work".to_string(),
                    energy: DeviceEnergy {
                        cpu_joules: 4.0,
                        dram_joules: 9.0,
                        gpu_joules: 0.0,
                    },
                    power_watts: 2.0,
                }],
                is_live: true,
                energy: DeviceEnergy {
                    cpu_joules: 4.0,
                    dram_joules: 9.0,
                    gpu_joules: 0.0,
                },
                power_watts: 2.0,
                percentage_of_system: 10.0,
            }],
            unattributed: DeviceEnergy {
                cpu_joules: 38.0,
                dram_joules: 90.0,
                gpu_joules: 0.0,
            },
            tracked_pids: vec![123],
            ..MetricsSnapshot::default()
        };

        let value = serde_json::to_value(build_snapshot_output(&snapshot)).unwrap();

        assert_eq!(value["sources"]["dram"], "included_in_package");
        assert!(value["system_total"].get("dram_joules").is_none());
        assert!(value["unattributed"].get("dram_joules").is_none());
        assert!(value["workloads"][0]["energy"].get("dram_joules").is_none());
        assert!(
            value["workloads"][0]["processes"][0]["energy"]
                .get("dram_joules")
                .is_none()
        );
    }

    #[test]
    fn snapshot_output_keeps_dram_joules_when_dram_is_measured() {
        let snapshot = MetricsSnapshot {
            sources: DeviceSources {
                cpu: DeviceSource::MeasuredPackage,
                dram: DeviceSource::Measured,
                gpu: DeviceSource::Unavailable,
            },
            system_total: DeviceEnergy {
                cpu_joules: 42.0,
                dram_joules: 9.0,
                gpu_joules: 0.0,
            },
            ..MetricsSnapshot::default()
        };

        let value = serde_json::to_value(build_snapshot_output(&snapshot)).unwrap();

        assert_eq!(value["sources"]["dram"], "measured");
        assert_eq!(value["system_total"]["dram_joules"], 9.0);
    }

    #[test]
    fn cli_rate_override_wins_over_loaded_config() {
        let args = Args {
            pid: None,
            duration: None,
            rate: Some(5.0),
            scan_interval: None,
            snapshot_out: None,
            tui: false,
            headless: false,
            export: None,
            port: DEFAULT_PROMETHEUS_PORT,
            bind: "0.0.0.0".parse().unwrap(),
            json_out: None,
        };
        let mut config = EmtConfig::default();
        config.collection.rate_hz = 0.0;

        apply_cli_overrides(&mut config, &args);

        assert_eq!(config.collection.rate_hz, 5.0);
        config.validate().unwrap();
    }

    #[test]
    fn cli_scan_interval_override_wins_over_loaded_config() {
        let args = Args::parse_from(["emt", "--scan-interval", "7.5"]);
        let mut config = EmtConfig::default();

        apply_cli_overrides(&mut config, &args);

        assert_eq!(config.discovery.scan_interval_secs, 7.5);
        config.validate().unwrap();
    }

    #[test]
    fn tui_monitor_all_uses_low_overhead_defaults_without_cli_overrides() {
        let args = Args::parse_from(["emt", "--tui"]);
        let mut config = EmtConfig::default();

        apply_mode_defaults(&mut config, &args);

        assert_eq!(config.collection.rate_hz, 0.1);
        assert_eq!(config.discovery.scan_interval_secs, 30.0);
        assert_eq!(tui_render_interval(&config), Duration::from_millis(2000));
    }

    #[test]
    fn tui_monitor_all_preserves_explicit_cli_rate_and_scan_interval() {
        let args = Args::parse_from(["emt", "--tui", "--rate", "8", "--scan-interval", "1.5"]);
        let mut config = EmtConfig::default();
        apply_cli_overrides(&mut config, &args);

        apply_mode_defaults(&mut config, &args);

        assert_eq!(config.collection.rate_hz, 8.0);
        assert_eq!(config.discovery.scan_interval_secs, 1.5);
    }

    #[test]
    fn tui_pid_mode_preserves_default_collection_cadence() {
        let args = Args::parse_from(["emt", "--tui", "--pid", "123"]);
        let mut config = EmtConfig::default();

        apply_mode_defaults(&mut config, &args);

        assert_eq!(config.collection.rate_hz, 10.0);
        assert_eq!(config.discovery.scan_interval_secs, 2.0);
    }

    #[test]
    fn cli_defaults_to_tui_mode() {
        let args = Args::parse_from(["emt"]);

        assert_eq!(selected_mode(&args), Mode::Tui);
    }

    #[test]
    fn cli_accepts_explicit_tui_mode() {
        let args = Args::parse_from(["emt", "--tui"]);

        assert_eq!(selected_mode(&args), Mode::Tui);
    }

    #[test]
    fn cli_reserves_headless_mode_for_future_exports() {
        let args = Args::parse_from(["emt", "--headless"]);

        assert_eq!(selected_mode(&args), Mode::Headless);
        assert!(validate_args(&args).is_err());
    }

    #[test]
    fn cli_accepts_headless_prometheus_export() {
        let args = Args::parse_from([
            "emt",
            "--headless",
            "--export",
            "prometheus",
            "--bind",
            "127.0.0.1",
            "--port",
            "9200",
        ]);

        assert_eq!(selected_mode(&args), Mode::Headless);
        assert_eq!(args.export, Some(ExportMode::Prometheus));
        assert_eq!(args.bind, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(args.port, 9200);
        assert!(validate_args(&args).is_ok());
    }

    #[test]
    fn cli_requires_headless_for_export() {
        let result = Args::try_parse_from(["emt", "--export", "prometheus"]);

        assert!(result.is_err());
    }

    #[test]
    fn cli_accepts_json_out_mode_with_duration() {
        let args = Args::parse_from(["emt", "--json-out", "results.json", "--duration", "30"]);

        assert_eq!(selected_mode(&args), Mode::JsonOut);
        assert_eq!(batch_duration_seconds(&args), 30);
    }

    #[test]
    fn cli_rejects_multiple_modes() {
        let result = Args::try_parse_from(["emt", "--tui", "--json-out", "results.json"]);

        assert!(result.is_err());
    }

    #[test]
    fn cli_rejects_duration_outside_json_out_mode() {
        let args = Args::parse_from(["emt", "--headless", "--duration", "30"]);

        assert!(validate_args(&args).is_err());
    }
}

#[derive(Serialize)]
struct CliOutput {
    pid: Option<u32>,
    duration_seconds: f64,
    total_energy: f64,
    energy_unit: String,
    power: f64,
    power_unit: String,
    devices: DeviceBreakdown,
    workloads: Vec<WorkloadOutput>,
}

#[derive(Serialize)]
struct DeviceBreakdown {
    cpu: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    dram: Option<f64>,
    gpu: f64,
}

#[derive(Serialize)]
struct WorkloadOutput {
    root_pid: u32,
    group_id: String,
    name: String,
    user: String,
    energy: f64,
    power: f64,
    percentage_of_system: f64,
}

#[derive(Serialize)]
struct SnapshotOutput<'a> {
    timestamp: i64,
    gpu_available: bool,
    sources: &'a DeviceSources,
    system_total: SnapshotDeviceEnergy,
    workloads: Vec<SnapshotWorkloadOutput<'a>>,
    unattributed: SnapshotDeviceEnergy,
    tracked_pids: &'a [u32],
    diagnostics: &'a MonitorDiagnostics,
}

#[derive(Serialize)]
struct SnapshotDeviceEnergy {
    cpu_joules: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    dram_joules: Option<f64>,
    gpu_joules: f64,
}

#[derive(Serialize)]
struct SnapshotWorkloadOutput<'a> {
    root_pid: u32,
    group_id: &'a str,
    name: &'a str,
    user: &'a str,
    processes: Vec<SnapshotProcessOutput<'a>>,
    is_live: bool,
    energy: SnapshotDeviceEnergy,
    power_watts: f64,
    percentage_of_system: f64,
}

#[derive(Serialize)]
struct SnapshotProcessOutput<'a> {
    pid: u32,
    name: &'a str,
    energy: SnapshotDeviceEnergy,
    power_watts: f64,
}

impl DeviceBreakdown {
    fn from_snapshot(snapshot: &MetricsSnapshot, units: &MeasurementUnitsConfig) -> Self {
        Self {
            cpu: units.convert_energy_from_joules(snapshot.system_total.cpu_joules),
            dram: snapshot
                .sources
                .reports_dram_energy()
                .then(|| units.convert_energy_from_joules(snapshot.system_total.dram_joules)),
            gpu: units.convert_energy_from_joules(snapshot.system_total.gpu_joules),
        }
    }
}

impl SnapshotDeviceEnergy {
    fn from_energy(energy: &DeviceEnergy, sources: &DeviceSources) -> Self {
        Self {
            cpu_joules: energy.cpu_joules,
            dram_joules: sources.reports_dram_energy().then_some(energy.dram_joules),
            gpu_joules: energy.gpu_joules,
        }
    }
}

fn build_snapshot_output(snapshot: &MetricsSnapshot) -> SnapshotOutput<'_> {
    let sources = &snapshot.sources;
    let workloads = snapshot
        .workloads
        .iter()
        .map(|workload| SnapshotWorkloadOutput {
            root_pid: workload.root_pid,
            group_id: workload.group_id.as_str(),
            name: workload.name.as_str(),
            user: workload.user.as_str(),
            processes: workload
                .processes
                .iter()
                .map(|process| SnapshotProcessOutput {
                    pid: process.pid,
                    name: process.name.as_str(),
                    energy: SnapshotDeviceEnergy::from_energy(&process.energy, sources),
                    power_watts: process.power_watts,
                })
                .collect(),
            is_live: workload.is_live,
            energy: SnapshotDeviceEnergy::from_energy(&workload.energy, sources),
            power_watts: workload.power_watts,
            percentage_of_system: workload.percentage_of_system,
        })
        .collect();

    SnapshotOutput {
        timestamp: snapshot.timestamp,
        gpu_available: snapshot.gpu_available,
        sources,
        system_total: SnapshotDeviceEnergy::from_energy(&snapshot.system_total, sources),
        workloads,
        unattributed: SnapshotDeviceEnergy::from_energy(&snapshot.unattributed, sources),
        tracked_pids: &snapshot.tracked_pids,
        diagnostics: &snapshot.diagnostics,
    }
}

fn build_cli_output(
    args: &Args,
    duration: f64,
    snapshot: &MetricsSnapshot,
    units: &MeasurementUnitsConfig,
) -> CliOutput {
    let total_energy_joules = snapshot.system_total.total();
    let power_watts = if duration > 0.0 {
        total_energy_joules / duration
    } else {
        0.0
    };

    let workloads: Vec<WorkloadOutput> = snapshot
        .workloads
        .iter()
        .map(|wl| WorkloadOutput {
            root_pid: wl.root_pid,
            group_id: wl.group_id.clone(),
            name: wl.name.clone(),
            user: wl.user.clone(),
            energy: units.convert_energy_from_joules(wl.energy.total()),
            power: units.convert_power_from_watts(if duration > 0.0 {
                wl.energy.total() / duration
            } else {
                wl.power_watts
            }),
            percentage_of_system: wl.percentage_of_system,
        })
        .collect();

    CliOutput {
        pid: args.pid,
        duration_seconds: duration,
        total_energy: units.convert_energy_from_joules(total_energy_joules),
        energy_unit: units.energy.clone(),
        power: units.convert_power_from_watts(power_watts),
        power_unit: units.power.clone(),
        devices: DeviceBreakdown::from_snapshot(snapshot, units),
        workloads,
    }
}

fn apply_cli_overrides(config: &mut EmtConfig, args: &Args) {
    if let Some(rate) = args.rate {
        config.collection.rate_hz = rate;
    }
    if let Some(scan_interval) = args.scan_interval {
        config.discovery.scan_interval_secs = scan_interval;
    }
}

fn apply_mode_defaults(config: &mut EmtConfig, args: &Args) {
    if selected_mode(args) != Mode::Tui || args.pid.is_some() {
        return;
    }

    if args.rate.is_none() {
        config.collection.rate_hz = config
            .collection
            .rate_hz
            .min(config.tui.monitor_all_rate_hz);
    }
    if args.scan_interval.is_none() {
        config.discovery.scan_interval_secs = config
            .discovery
            .scan_interval_secs
            .max(config.tui.monitor_all_scan_interval_secs);
    }
}

fn tui_render_interval(config: &EmtConfig) -> Duration {
    Duration::from_millis(config.tui.render_interval_millis)
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Err(message) = validate_args(&args) {
        eprintln!("{message}");
        std::process::exit(2);
    }
    let mode = selected_mode(&args);

    let mut config = EmtConfig::load();
    apply_cli_overrides(&mut config, &args);
    apply_mode_defaults(&mut config, &args);
    if let Err(e) = config.validate() {
        eprintln!("Invalid configuration: {e}");
        std::process::exit(2);
    }

    match mode {
        Mode::Tui => run_tui(config, args.pid, args.snapshot_out.as_deref()).await,
        Mode::Headless => {
            run_prometheus_export(
                config,
                args.pid,
                args.bind,
                args.port,
                args.snapshot_out.as_deref(),
            )
            .await
        }
        Mode::JsonOut => {
            let duration = batch_duration_seconds(&args);
            let path = args
                .json_out
                .as_deref()
                .expect("json_out is present in JsonOut mode");
            run_json_out(
                config,
                &args,
                duration,
                path.to_string(),
                args.snapshot_out.as_deref(),
            )
            .await;
        }
    }
}

async fn run_tui(config: EmtConfig, pid: Option<u32>, snapshot_out: Option<&str>) {
    let tick_rate = tui_render_interval(&config);
    let root_pids = pid.map(|p| vec![p]);
    let mut monitor = Monitor::new(config, root_pids);

    let handle = match monitor.commence().await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to start monitoring: {e}");
            std::process::exit(1);
        }
    };

    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        original_hook(info);
    }));

    // Enter TUI
    crossterm::terminal::enable_raw_mode().expect("Failed to enable raw mode");
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)
        .expect("Failed to enter alternate screen");
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend).expect("Failed to create terminal");

    let mut app = App::new(handle);
    let mut last_draw = std::time::Instant::now() - tick_rate;
    #[cfg(debug_assertions)]
    let force_panic_after_first_draw =
        std::env::var_os("EMT_TUI_FORCE_PANIC_AFTER_FIRST_DRAW").is_some();

    while !app.should_quit {
        let mut should_draw = last_draw.elapsed() >= tick_rate;

        if let Some(event) = tui::event::poll(TUI_INPUT_POLL_INTERVAL) {
            match event {
                tui::event::AppEvent::Quit => {
                    app.quit();
                    should_draw = true;
                }
                tui::event::AppEvent::CycleSortMode => {
                    app.cycle_sort_mode();
                    should_draw = true;
                }
                tui::event::AppEvent::ResetDisplay => {
                    app.reset_display();
                    should_draw = true;
                }
                tui::event::AppEvent::SelectPrevious => {
                    app.select_previous();
                    should_draw = true;
                }
                tui::event::AppEvent::SelectNext => {
                    app.select_next();
                    should_draw = true;
                }
                tui::event::AppEvent::ExpandSelected => {
                    app.expand_selected();
                    should_draw = true;
                }
                tui::event::AppEvent::CollapseSelected => {
                    app.collapse_selected();
                    should_draw = true;
                }
                tui::event::AppEvent::Tick => {}
            }
        }

        if should_draw {
            app.refresh();
            terminal
                .draw(|frame| tui::ui::render(frame, &app))
                .expect("Failed to draw");
            last_draw = std::time::Instant::now();

            #[cfg(debug_assertions)]
            if force_panic_after_first_draw {
                panic!("forced TUI panic smoke check");
            }
        }
    }

    // Restore terminal
    crossterm::terminal::disable_raw_mode().expect("Failed to disable raw mode");
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    )
    .expect("Failed to leave alternate screen");

    if let Err(e) = monitor.shutdown().await {
        eprintln!("Warning: Shutdown error: {e}");
    }
    app.refresh();
    write_snapshot_if_requested(snapshot_out, &app.snapshot());
}

async fn run_json_out(
    config: EmtConfig,
    args: &Args,
    duration_secs: u64,
    output_path: String,
    snapshot_out: Option<&str>,
) {
    let measurement_units = config.measurement_units.clone();
    let root_pids = args.pid.map(|p| vec![p]);
    let mut monitor = Monitor::new(config, root_pids);

    let handle = match monitor.commence().await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to start monitoring: {e}");
            std::process::exit(1);
        }
    };

    tokio::time::sleep(tokio::time::Duration::from_secs(duration_secs)).await;

    if let Err(e) = monitor.shutdown().await {
        eprintln!("Warning: Shutdown error: {e}");
    }

    let snapshot = handle.snapshot();
    write_snapshot_if_requested(snapshot_out, &snapshot);
    let duration = duration_secs as f64;
    let cli_output = build_cli_output(args, duration, &snapshot, &measurement_units);

    let json_output =
        serde_json::to_string_pretty(&cli_output).expect("Failed to serialize output");
    let mut file = File::create(&output_path).expect("Failed to create JSON output file");
    file.write_all(json_output.as_bytes())
        .expect("Failed to write JSON output");
    eprintln!("JSON results written to: {output_path}");
}

async fn run_prometheus_export(
    config: EmtConfig,
    pid: Option<u32>,
    bind: IpAddr,
    port: u16,
    snapshot_out: Option<&str>,
) {
    let update_interval = Duration::from_secs_f64((1.0 / config.collection.rate_hz).max(0.1));
    let root_pids = pid.map(|p| vec![p]);
    let mut monitor = Monitor::new(config, root_pids);

    let handle = match monitor.commence().await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to start monitoring: {e}");
            std::process::exit(1);
        }
    };

    let sink = Arc::new(Mutex::new(
        PrometheusSink::new().expect("Failed to create Prometheus sink"),
    ));
    update_prometheus_sink(&sink, &handle.snapshot());

    let app = prometheus_router(Arc::clone(&sink));
    let address = SocketAddr::new(bind, port);
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("Failed to bind Prometheus exporter on {address}: {e}");
            let _ = monitor.shutdown().await;
            std::process::exit(1);
        }
    };

    eprintln!("Prometheus exporter listening on http://{address}/metrics");

    let update_task = tokio::spawn(update_prometheus_sink_loop(
        Arc::clone(&sink),
        handle.clone(),
        update_interval,
    ));
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;

    update_task.abort();
    let _ = update_task.await;

    if let Err(e) = monitor.shutdown().await {
        eprintln!("Warning: Shutdown error: {e}");
    }
    write_snapshot_if_requested(snapshot_out, &handle.snapshot());

    if let Err(e) = serve_result {
        eprintln!("Prometheus exporter error: {e}");
        std::process::exit(1);
    }
}

async fn update_prometheus_sink_loop(
    sink: SharedPrometheusSink,
    handle: MonitorHandle,
    interval: Duration,
) {
    loop {
        update_prometheus_sink(&sink, &handle.snapshot());
        tokio::time::sleep(interval).await;
    }
}

fn update_prometheus_sink(sink: &SharedPrometheusSink, snapshot: &MetricsSnapshot) {
    sink.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .update(snapshot);
}

fn write_snapshot_if_requested(path: Option<&str>, snapshot: &MetricsSnapshot) {
    let Some(path) = path else {
        return;
    };

    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        let mut file = File::create(path)?;
        let output = build_snapshot_output(snapshot);
        serde_json::to_writer_pretty(&mut file, &output)?;
        file.write_all(b"\n")?;
        Ok(())
    })();

    match result {
        Ok(()) => eprintln!("Snapshot written to: {path}"),
        Err(e) => eprintln!("Warning: failed to write snapshot to {path}: {e}"),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    {
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
    }

    #[cfg(not(unix))]
    ctrl_c.await;
}
