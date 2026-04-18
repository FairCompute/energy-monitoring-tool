use clap::Parser;
use emt::{collectors::Rapl, energy_group::EnergyGroup, utils};
use log::info;
use serde::Serialize;
use std::fs::File;
use std::io::Write;

#[derive(Parser, Debug)]
#[command(name = "energy-monitoring-tool")]
#[command(about = "Monitor energy consumption of processes using RAPL")]
struct Args {
    /// Process ID to monitor (if not specified, monitors current process tree)
    #[arg(short, long)]
    pid: Option<u32>,

    /// Duration to monitor in seconds
    #[arg(short, long, default_value = "10")]
    duration: u64,

    /// Collection rate in Hz
    #[arg(short, long, default_value = "10.0")]
    rate: f64,

    /// Output file for JSON results (optional)
    #[arg(short, long)]
    output: Option<String>,

    /// Quiet mode - only output JSON result
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Serialize)]
struct EnergyResult {
    pid: Option<u32>,
    duration_seconds: f64,
    total_energy: f64,
    energy_unit: String,
    devices: std::collections::HashMap<String, f64>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    
    if !args.quiet {
        utils::logger::setup_logger();
        info!("Energy Monitoring Tool started");
    }
    
    // Convert PID to the format expected by EnergyGroup
    let pids: Option<Vec<usize>> = args.pid.map(|p| vec![p as usize]);
    
    // Create a RAPL energy group collector with small batch size for CLI
    // Batch size of 10 means data is sent every 10 iterations (1 second at 10 Hz)
    let mut energy_group: EnergyGroup<Rapl> = match EnergyGroup::create_with_collector(
        Rapl::default(),
        args.rate,
        pids,
        Some(10),  // Small batch size for responsive CLI output
    ) {
        Ok(group) => group,
        Err(e) => {
            eprintln!("Failed to create energy group: {}", e);
            std::process::exit(1);
        }
    };
    
    if !args.quiet {
        info!("Tracked processes: {:?}", energy_group.processes());
        info!("Monitoring for {} seconds at {} Hz...", args.duration, args.rate);
    }
    
    // Start monitoring
    if let Err(e) = energy_group.commence().await {
        eprintln!("Failed to start monitoring: {}", e);
        std::process::exit(1);
    }
    
    // Monitor for the specified duration, polling data periodically
    let start = std::time::Instant::now();
    let poll_interval = tokio::time::Duration::from_millis(500);
    let mut poll_count = 0;
    
    while start.elapsed().as_secs() < args.duration {
        tokio::time::sleep(poll_interval).await;
        if let Err(e) = energy_group.poll_data() {
            eprintln!("Warning: Failed to poll data: {}", e);
        }
        poll_count += 1;
    }
    
    // Polling complete
    
    // Shutdown and get final data
    if let Err(e) = energy_group.shutdown() {
        eprintln!("Warning: Shutdown error: {}", e);
    }
    
    // Calculate total energy from trace
    let trace = energy_group.energy_trace();
    let actual_duration = start.elapsed().as_secs_f64();
    
    let mut total_energy = 0.0;
    let mut device_energy: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    
    if let Ok(energy_col) = trace.column("energy") {
        if let Ok(values) = energy_col.f64() {
            total_energy = values.iter().filter_map(|v| v).sum();
        }
    }
    
    // Group energy by device
    if let (Ok(device_col), Ok(energy_col)) = (trace.column("device"), trace.column("energy")) {
        if let (Ok(devices), Ok(energies)) = (device_col.str(), energy_col.f64()) {
            for (device, energy) in devices.iter().zip(energies.iter()) {
                if let (Some(d), Some(e)) = (device, energy) {
                    *device_energy.entry(d.to_string()).or_insert(0.0) += e;
                }
            }
        }
    }
    
    let result = EnergyResult {
        pid: args.pid,
        duration_seconds: actual_duration,
        total_energy,
        energy_unit: "Joules".to_string(),
        devices: device_energy,
    };
    
    // Output results
    let json_output = serde_json::to_string_pretty(&result).unwrap();
    
    if let Some(output_path) = &args.output {
        let mut file = File::create(output_path).expect("Failed to create output file");
        file.write_all(json_output.as_bytes()).expect("Failed to write output");
        if !args.quiet {
            info!("Results written to: {}", output_path);
        }
    }
    
    if args.quiet {
        println!("{}", json_output);
    } else {
        info!("Monitoring complete");
        println!("\n=== Energy Consumption Results ===");
        println!("PID: {:?}", args.pid.map(|p| p.to_string()).unwrap_or("all".to_string()));
        println!("Duration: {:.2} s", actual_duration);
        println!("Total Energy: {:.4} J", total_energy);
        println!("Average Power: {:.2} W", total_energy / actual_duration);
        println!("\nPer-device breakdown:");
        for (device, energy) in &result.devices {
            println!("  {}: {:.4} J", device, energy);
        }
    }
}
