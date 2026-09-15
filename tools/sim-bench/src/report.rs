use std::fmt::Write as _;

/// Performance measurement results for a single simulation benchmark scenario.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub scenario_name: String,
    pub description: String,
    pub entity_count: usize,
    pub region_count: usize,
    pub ticks_run: u64,
    pub total_duration_ms: f64,
    pub avg_tick_us: f64,
    pub ticks_per_sec: f64,
    pub jobs_executed_hot: u64,
    pub jobs_executed_warm: u64,
    pub jobs_executed_cold: u64,
    pub entities_ticked_hot: u64,
    pub entities_ticked_warm: u64,
    pub entities_ticked_cold: u64,
    pub messages_routed: u64,
    pub estimated_memory_bytes: usize,
}

impl BenchmarkResult {
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "    {{\n",
                "      \"scenario\": \"{}\",\n",
                "      \"description\": \"{}\",\n",
                "      \"entities\": {},\n",
                "      \"regions\": {},\n",
                "      \"ticks\": {},\n",
                "      \"total_duration_ms\": {:.3},\n",
                "      \"avg_tick_us\": {:.3},\n",
                "      \"ticks_per_second\": {:.1},\n",
                "      \"jobs\": {{\n",
                "        \"hot\": {},\n",
                "        \"warm\": {},\n",
                "        \"cold\": {}\n",
                "      }},\n",
                "      \"entity_ticks\": {{\n",
                "        \"hot\": {},\n",
                "        \"warm\": {},\n",
                "        \"cold\": {}\n",
                "      }},\n",
                "      \"messages_routed\": {},\n",
                "      \"estimated_memory_bytes\": {}\n",
                "    }}"
            ),
            self.scenario_name,
            self.description,
            self.entity_count,
            self.region_count,
            self.ticks_run,
            self.total_duration_ms,
            self.avg_tick_us,
            self.ticks_per_sec,
            self.jobs_executed_hot,
            self.jobs_executed_warm,
            self.jobs_executed_cold,
            self.entities_ticked_hot,
            self.entities_ticked_warm,
            self.entities_ticked_cold,
            self.messages_routed,
            self.estimated_memory_bytes,
        )
    }
}

/// Overall report containing results for all executed scenarios.
#[derive(Debug, Clone)]
pub struct BenchmarkSuiteReport {
    pub total_duration_ms: f64,
    pub results: Vec<BenchmarkResult>,
}

impl BenchmarkSuiteReport {
    pub fn new(total_duration_ms: f64, results: Vec<BenchmarkResult>) -> Self {
        BenchmarkSuiteReport {
            total_duration_ms,
            results,
        }
    }

    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n  \"benchmark_suite\": \"sim-bench\",\n");
        let _ = writeln!(
            out,
            "  \"total_duration_ms\": {:.3},",
            self.total_duration_ms
        );
        out.push_str("  \"scenarios\": [\n");
        for (i, res) in self.results.iter().enumerate() {
            out.push_str(&res.to_json());
            if i + 1 < self.results.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ]\n}");
        out
    }

    pub fn print_human_readable(&self) {
        println!(
            "========================================================================================================================"
        );
        println!(
            "                                        RTS ENGINE SIMULATION BENCHMARK REPORT                                         "
        );
        println!(
            "========================================================================================================================"
        );
        println!(
            "{:<24} | {:>8} | {:>7} | {:>6} | {:>10} | {:>11} | {:>10} | {:>12}",
            "Scenario",
            "Entities",
            "Regions",
            "Ticks",
            "Total (ms)",
            "Avg (us/tk)",
            "Ticks/sec",
            "Mem Est (KB)"
        );
        println!(
            "-------------------------+----------+---------+--------+------------+-------------+------------+--------------"
        );

        for r in &self.results {
            let mem_kb = r.estimated_memory_bytes as f64 / 1024.0;
            println!(
                "{:<24} | {:>8} | {:>7} | {:>6} | {:>10.3} | {:>11.3} | {:>10.1} | {:>12.1}",
                r.scenario_name,
                r.entity_count,
                r.region_count,
                r.ticks_run,
                r.total_duration_ms,
                r.avg_tick_us,
                r.ticks_per_sec,
                mem_kb
            );
        }

        println!(
            "========================================================================================================================"
        );
        println!("Detailed Regional Breakdown:");
        println!(
            "{:<24} | {:>15} | {:>15} | {:>15} | {:>10}",
            "Scenario", "Hot (Jobs/Ent)", "Warm (Jobs/Ent)", "Cold (Jobs/Ent)", "Msgs Routed"
        );
        println!(
            "-------------------------+-----------------+-----------------+-----------------+-----------"
        );

        for r in &self.results {
            let hot_str = format!("{}/{}", r.jobs_executed_hot, r.entities_ticked_hot);
            let warm_str = format!("{}/{}", r.jobs_executed_warm, r.entities_ticked_warm);
            let cold_str = format!("{}/{}", r.jobs_executed_cold, r.entities_ticked_cold);
            println!(
                "{:<24} | {:>15} | {:>15} | {:>15} | {:>10}",
                r.scenario_name, hot_str, warm_str, cold_str, r.messages_routed
            );
        }

        println!(
            "========================================================================================================================"
        );
        println!(
            "Total Benchmark Suite Runtime: {:.2} ms\n",
            self.total_duration_ms
        );
    }
}
