use clap::{Parser, ValueEnum};
use emt::config::{EmtConfig, MeasurementUnitsConfig};
use emt::metrics_sink::{MetricsSink, PrometheusSink, SharedPrometheusSink, prometheus_router};
use emt::monitor::{DeviceEnergy, MetricsSnapshot, Monitor, MonitorHandle};
use emt::tui::{self, App};
use serde::Serialize;
use std::fs::File;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DEFAULT_BATCH_DURATION_SECS: u64 = 10;
const DEFAULT_PROMETHEUS_PORT: u16 = 9101;

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
    use emt::monitor::WorkloadSnapshot;

    #[test]
    fn cli_output_uses_configured_units_and_unit_neutral_fields() {
        let args = Args {
            pid: Some(123),
            duration: Some(10),
            rate: None,
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
        };

        let output = build_cli_output(&args, 10.0, &snapshot, &units);

        assert_eq!(output.energy_unit, "kWh");
        assert_eq!(output.power_unit, "mW");
        assert!((output.total_energy - 0.001).abs() < 1e-9);
        assert!((output.power - 360_000.0).abs() < 1e-9);
        assert!((output.devices.cpu - 0.00075).abs() < 1e-9);
        assert!((output.devices.dram - 0.00025).abs() < 1e-9);
        assert_eq!(output.workloads[0].root_pid, 123);
        assert_eq!(output.workloads[0].group_id, "pid:123");
        assert_eq!(output.workloads[0].name, "work");
        assert_eq!(output.workloads[0].user, "user");
        assert!((output.workloads[0].energy - 0.001).abs() < 1e-9);
        assert!((output.workloads[0].power - 360_000.0).abs() < 1e-9);
        assert!((output.workloads[0].percentage_of_system - 100.0).abs() < 1e-9);
    }

    #[test]
    fn cli_rate_override_wins_over_loaded_config() {
        let args = Args {
            pid: None,
            duration: None,
            rate: Some(5.0),
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
    dram: f64,
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

impl DeviceBreakdown {
    fn from_energy(de: &DeviceEnergy, units: &MeasurementUnitsConfig) -> Self {
        Self {
            cpu: units.convert_energy_from_joules(de.cpu_joules),
            dram: units.convert_energy_from_joules(de.dram_joules),
            gpu: units.convert_energy_from_joules(de.gpu_joules),
        }
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
        devices: DeviceBreakdown::from_energy(&snapshot.system_total, units),
        workloads,
    }
}

fn apply_cli_overrides(config: &mut EmtConfig, args: &Args) {
    if let Some(rate) = args.rate {
        config.collection.rate_hz = rate;
    }
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
    if let Err(e) = config.validate() {
        eprintln!("Invalid configuration: {e}");
        std::process::exit(2);
    }

    match mode {
        Mode::Tui => run_tui(config, args.pid).await,
        Mode::Headless => run_prometheus_export(config, args.pid, args.bind, args.port).await,
        Mode::JsonOut => {
            let duration = batch_duration_seconds(&args);
            let path = args
                .json_out
                .as_deref()
                .expect("json_out is present in JsonOut mode");
            run_json_out(config, &args, duration, path.to_string()).await;
        }
    }
}

async fn run_tui(config: EmtConfig, pid: Option<u32>) {
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
    let tick_rate = std::time::Duration::from_millis(500);
    #[cfg(debug_assertions)]
    let force_panic_after_first_draw =
        std::env::var_os("EMT_TUI_FORCE_PANIC_AFTER_FIRST_DRAW").is_some();

    while !app.should_quit {
        app.refresh();
        terminal
            .draw(|frame| tui::ui::render(frame, &app))
            .expect("Failed to draw");

        #[cfg(debug_assertions)]
        if force_panic_after_first_draw {
            panic!("forced TUI panic smoke check");
        }

        if let Some(event) = tui::event::poll(tick_rate) {
            match event {
                tui::event::AppEvent::Quit => app.quit(),
                tui::event::AppEvent::Tick => {}
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
}

async fn run_json_out(config: EmtConfig, args: &Args, duration_secs: u64, output_path: String) {
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
    let duration = duration_secs as f64;
    let cli_output = build_cli_output(args, duration, &snapshot, &measurement_units);

    let json_output =
        serde_json::to_string_pretty(&cli_output).expect("Failed to serialize output");
    let mut file = File::create(&output_path).expect("Failed to create JSON output file");
    file.write_all(json_output.as_bytes())
        .expect("Failed to write JSON output");
    eprintln!("JSON results written to: {output_path}");
}

async fn run_prometheus_export(config: EmtConfig, pid: Option<u32>, bind: IpAddr, port: u16) {
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
        handle,
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
