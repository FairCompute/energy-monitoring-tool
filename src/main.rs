use clap::Parser;
use emt::config::{EmtConfig, MeasurementUnitsConfig};
use emt::monitor::{DeviceEnergy, MetricsSnapshot, Monitor};
use emt::tui::{self, App};
use serde::Serialize;
use std::fs::File;
use std::io::Write;

#[derive(Parser, Debug)]
#[command(name = "emt")]
#[command(about = "Monitor energy consumption of processes")]
struct Args {
    /// Process ID to monitor (if not specified, monitors all root processes)
    #[arg(short, long)]
    pid: Option<u32>,

    /// Duration to monitor in seconds (headless mode only)
    #[arg(short, long, default_value = "10")]
    duration: u64,

    /// Collection rate in Hz (overrides config file)
    #[arg(short, long)]
    rate: Option<f64>,

    /// Output file for JSON results (optional, implies headless)
    #[arg(short, long)]
    output: Option<String>,

    /// Quiet mode - only output JSON result (implies headless)
    #[arg(short, long)]
    quiet: bool,

    /// Launch interactive TUI (default when no --output or --quiet)
    #[arg(long)]
    tui: bool,

    /// Run in headless mode (print results after duration)
    #[arg(long)]
    headless: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use emt::monitor::WorkloadSnapshot;

    #[test]
    fn cli_output_uses_configured_units_and_unit_neutral_fields() {
        let args = Args {
            pid: Some(123),
            duration: 10,
            rate: None,
            output: None,
            quiet: true,
            tui: false,
            headless: false,
        };
        let units = MeasurementUnitsConfig {
            energy: "kWh".to_string(),
            power: "mW".to_string(),
        };
        let snapshot = MetricsSnapshot {
            timestamp: 0,
            system_total: DeviceEnergy {
                cpu_joules: 2_700.0,
                dram_joules: 900.0,
                gpu_joules: 0.0,
            },
            workloads: vec![WorkloadSnapshot {
                root_pid: 123,
                name: "work".to_string(),
                user: "user".to_string(),
                energy: DeviceEnergy {
                    cpu_joules: 2_700.0,
                    dram_joules: 900.0,
                    gpu_joules: 0.0,
                },
                power_watts: 360.0,
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
        assert!((output.workloads[0].energy - 0.001).abs() < 1e-9);
        assert!((output.workloads[0].power - 360_000.0).abs() < 1e-9);
    }

    #[test]
    fn cli_rate_override_wins_over_loaded_config() {
        let args = Args {
            pid: None,
            duration: 1,
            rate: Some(5.0),
            output: None,
            quiet: true,
            tui: false,
            headless: false,
        };
        let mut config = EmtConfig::default();
        config.collection.rate_hz = 0.0;

        apply_cli_overrides(&mut config, &args);

        assert_eq!(config.collection.rate_hz, 5.0);
        config.validate().unwrap();
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
    name: String,
    user: String,
    energy: f64,
    power: f64,
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
            name: wl.name.clone(),
            user: wl.user.clone(),
            energy: units.convert_energy_from_joules(wl.energy.total()),
            power: units.convert_power_from_watts(if duration > 0.0 {
                wl.energy.total() / duration
            } else {
                wl.power_watts
            }),
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

fn print_human_readable(output: &CliOutput) {
    println!("\n=== Energy Consumption Results ===");
    println!(
        "PID: {}",
        output
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "all".to_string())
    );
    println!("Duration: {:.2} s", output.duration_seconds);
    println!(
        "Total Energy: {:.4} {}",
        output.total_energy, output.energy_unit
    );
    println!("Average Power: {:.2} {}", output.power, output.power_unit);
    println!("Device breakdown:");
    println!("  CPU: {:.4} {}", output.devices.cpu, output.energy_unit);
    println!("  DRAM: {:.4} {}", output.devices.dram, output.energy_unit);
    println!("  GPU: {:.4} {}", output.devices.gpu, output.energy_unit);

    if !output.workloads.is_empty() {
        println!("Workloads:");
        for wl in &output.workloads {
            println!(
                "  PID {} ({}, {}): {:.4} {}, {:.2} {}",
                wl.root_pid,
                wl.name,
                wl.user,
                wl.energy,
                output.energy_unit,
                wl.power,
                output.power_unit
            );
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Determine mode: TUI vs headless
    let use_tui = args.tui || (!args.headless && !args.quiet && args.output.is_none());

    if !args.quiet && !use_tui {
        emt::utils::logger::setup_logger();
    }

    // Load config and apply CLI rate override
    let mut config = EmtConfig::load();
    apply_cli_overrides(&mut config, &args);
    if let Err(e) = config.validate() {
        eprintln!("Invalid configuration: {e}");
        std::process::exit(2);
    }

    if use_tui {
        run_tui(config, args.pid).await;
    } else {
        run_headless(config, &args).await;
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
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen
        );
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

    while !app.should_quit {
        terminal
            .draw(|frame| tui::ui::render(frame, &app))
            .expect("Failed to draw");

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

async fn run_headless(config: EmtConfig, args: &Args) {
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

    tokio::time::sleep(tokio::time::Duration::from_secs(args.duration)).await;

    if let Err(e) = monitor.shutdown().await {
        eprintln!("Warning: Shutdown error: {e}");
    }

    let snapshot = handle.snapshot();
    let duration = args.duration as f64;

    let output = build_cli_output(args, duration, &snapshot, &measurement_units);
    let json_output = serde_json::to_string_pretty(&output).expect("Failed to serialize output");

    if let Some(output_path) = &args.output {
        let mut file = File::create(output_path).expect("Failed to create output file");
        file.write_all(json_output.as_bytes())
            .expect("Failed to write output");
        if !args.quiet {
            eprintln!("Results written to: {output_path}");
        }
    }

    if args.quiet {
        println!("{json_output}");
    } else {
        print_human_readable(&output);
    }
}
