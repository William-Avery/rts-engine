mod report;
mod scenarios;

use report::{BenchmarkResult, BenchmarkSuiteReport};
use std::env;
use std::fs;
use std::time::Instant;

fn print_usage() {
    println!("sim-bench: RTS Engine Headless Simulation Benchmark Harness");
    println!("Usage:");
    println!("  cargo run -p sim-bench [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --json                Output results in JSON format to stdout");
    println!("  --output <path>       Write JSON report to specified file path");
    println!(
        "  --scenario <name>     Run specific scenario (10k_walls, 1k_idle_units, hot_vs_cold, scheduled_factories, event_queue_stress, power_grid_1k, production_chain_1k, logistics_jobs_1k, robots_1k, research_modifiers_1k)"
    );
    println!("  --help, -h            Show this help message");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut json_output = false;
    let mut output_file: Option<String> = None;
    let mut target_scenario: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json_output = true;
            }
            "--output" | "-o" => {
                if i + 1 < args.len() {
                    output_file = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    eprintln!("Error: --output requires a file path argument");
                    std::process::exit(1);
                }
            }
            "--scenario" | "-s" => {
                if i + 1 < args.len() {
                    target_scenario = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    eprintln!("Error: --scenario requires a scenario name");
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                print_usage();
                return;
            }
            unknown => {
                eprintln!("Unknown argument: {}", unknown);
                print_usage();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let overall_start = Instant::now();
    let mut results: Vec<BenchmarkResult> = Vec::new();

    let run_all = target_scenario.is_none();
    let target = target_scenario.as_deref().unwrap_or("");

    if run_all || target == "10k_walls" {
        results.push(scenarios::scenario_10k_walls());
    }
    if run_all || target == "1k_idle_units" {
        results.push(scenarios::scenario_1k_idle_units());
    }
    if run_all || target == "hot_vs_cold" {
        results.push(scenarios::scenario_hot_vs_cold());
    }
    if run_all || target == "scheduled_factories" {
        results.push(scenarios::scenario_scheduled_factories());
    }
    if run_all || target == "event_queue_stress" {
        results.push(scenarios::scenario_event_queue_stress());
    }
    if run_all || target == "power_grid_1k" {
        results.push(scenarios::scenario_power_grid_1k());
    }
    if run_all || target == "production_chain_1k" {
        results.push(scenarios::scenario_production_chain_1k());
    }
    if run_all || target == "logistics_jobs_1k" {
        results.push(scenarios::scenario_logistics_jobs_1k());
    }
    if run_all || target == "robots_1k" {
        results.push(scenarios::scenario_robots_1k());
    }
    if run_all || target == "research_modifiers_1k" {
        results.push(scenarios::scenario_research_modifiers_1k());
    }

    if results.is_empty() {
        eprintln!(
            "Error: Unknown scenario '{}'. Available: 10k_walls, 1k_idle_units, hot_vs_cold, scheduled_factories, event_queue_stress, power_grid_1k, production_chain_1k, logistics_jobs_1k, robots_1k, research_modifiers_1k",
            target
        );
        std::process::exit(1);
    }

    let total_duration_ms = overall_start.elapsed().as_secs_f64() * 1000.0;
    let report = BenchmarkSuiteReport::new(total_duration_ms, results);

    if json_output {
        println!("{}", report.to_json());
    } else {
        report.print_human_readable();
    }

    if let Some(file_path) = output_file {
        if let Err(e) = fs::write(&file_path, report.to_json()) {
            eprintln!("Failed to write report to {}: {}", file_path, e);
            std::process::exit(1);
        } else if !json_output {
            println!("Report saved to: {}", file_path);
        }
    }
}
