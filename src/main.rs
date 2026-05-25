use clap::Parser;
use emt::config::EmtConfig;
use emt::monitor::{DeviceEnergy, MetricsSnapshot, Monitor};
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

    /// Duration to monitor in seconds
    #[arg(short, long, default_value = "10")]
    duration: u64,

    /// Collection rate in Hz (overrides config file)
    #[arg(short, long)]
    rate: Option<f64>,

    /// Output file for JSON results (optional)
    #[arg(short, long)]
    output: Option<String>,

    /// Quiet mode - only output JSON result
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Serialize)]
struct CliOutput {
    pid: Option<u32>,
    duration_seconds: f64,
    total_energy_joules: f64,
    power_watts: f64,
    devices: DeviceBreakdown,
    workloads: Vec<WorkloadOutput>,
}

#[derive(Serialize)]
struct DeviceBreakdown {
    cpu_joules: f64,
    dram_joules: f64,
    gpu_joules: f64,
}

#[derive(Serialize)]
struct WorkloadOutput {
    root_pid: u32,
    name: String,
    user: String,
    energy_joules: f64,
    power_watts: f64,
}

impl From<&DeviceEnergy> for DeviceBreakdown {
    fn from(de: &DeviceEnergy) -> Self {
        Self {
            cpu_joules: de.cpu_joules,
            dram_joules: de.dram_joules,
            gpu_joules: de.gpu_joules,
        }
    }
}

fn build_cli_output(args: &Args, duration: f64, snapshot: &MetricsSnapshot) -> CliOutput {
    let total_energy = snapshot.system_total.total();
    let power_watts = if duration > 0.0 {
        total_energy / duration
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
            energy_joules: wl.energy.total(),
            power_watts: wl.power_watts,
        })
        .collect();

    CliOutput {
        pid: args.pid,
        duration_seconds: duration,
        total_energy_joules: total_energy,
        power_watts,
        devices: DeviceBreakdown::from(&snapshot.system_total),
        workloads,
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
    println!("Total Energy: {:.4} J", output.total_energy_joules);
    println!("Average Power: {:.2} W", output.power_watts);
    println!("Device breakdown:");
    println!("  CPU: {:.4} J", output.devices.cpu_joules);
    println!("  DRAM: {:.4} J", output.devices.dram_joules);
    println!("  GPU: {:.4} J", output.devices.gpu_joules);

    if !output.workloads.is_empty() {
        println!("Workloads:");
        for wl in &output.workloads {
            println!(
                "  PID {} ({}, {}): {:.4} J, {:.2} W",
                wl.root_pid, wl.name, wl.user, wl.energy_joules, wl.power_watts
            );
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    if !args.quiet {
        emt::utils::logger::setup_logger();
    }

    // Load config and apply CLI rate override
    let mut config = EmtConfig::load();
    if let Some(rate) = args.rate {
        config.collection.rate_hz = rate;
    }

    // Create Monitor with optional root PIDs
    let root_pids = args.pid.map(|p| vec![p]);
    let mut monitor = Monitor::new(config, root_pids);

    // Start monitoring
    let handle = match monitor.commence().await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to start monitoring: {e}");
            std::process::exit(1);
        }
    };

    // Sleep for the requested duration
    tokio::time::sleep(tokio::time::Duration::from_secs(args.duration)).await;

    // Capture final snapshot
    let snapshot = handle.snapshot();
    let duration = args.duration as f64;

    // Shutdown monitor
    if let Err(e) = monitor.shutdown().await {
        eprintln!("Warning: Shutdown error: {e}");
    }

    // Build output
    let output = build_cli_output(&args, duration, &snapshot);
    let json_output = serde_json::to_string_pretty(&output).expect("Failed to serialize output");

    // Write to file if requested
    if let Some(output_path) = &args.output {
        let mut file = File::create(output_path).expect("Failed to create output file");
        file.write_all(json_output.as_bytes())
            .expect("Failed to write output");
        if !args.quiet {
            eprintln!("Results written to: {output_path}");
        }
    }

    // Print output
    if args.quiet {
        println!("{json_output}");
    } else {
        print_human_readable(&output);
    }
}
