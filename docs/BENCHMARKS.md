# Benchmarks

This document records performance benchmarks for the project.

## Benchmarking Process

Benchmarks are run using the headless `sim-bench` tool:

```powershell
# Run all baseline scenarios and print formatted table
cargo run -p sim-bench

# Output machine-readable JSON to stdout
cargo run -p sim-bench -- --json

# Save JSON report to a file
cargo run -p sim-bench -- --output benchmark_report.json

# Run a specific scenario
cargo run -p sim-bench -- --scenario 10k_walls
```

## Baseline Targets vs Measured Performance (Milestones 3 - 11)

Measured on AMD64 Windows development environment:

| Scenario | Entities | Regions | Ticks | Total (ms) | Avg (µs/tick) | Throughput (ticks/sec) | Memory Est (KB) |
|---|---:|---:|---:|---:|---:|---:|---:|
| `10k_walls` | 10,000 | 16 | 60 | 0.099 | 1.65 µs | ~606,000 | 877 KB |
| `1k_idle_units` | 1,000 | 8 | 120 | 0.088 | 0.74 µs | ~1,359,000 | 103 KB |
| `hot_vs_cold` | 1,000 | 2 | 120 | 0.049 | 0.41 µs | ~2,469,000 | 102 KB |
| `scheduled_factories` | 200 | 1 | 150 | 0.059 | 0.40 µs | ~2,530,000 | 33 KB |
| `event_queue_stress` | 160 | 16 | 30 | 3.334 | 111.14 µs | ~9,000 | 1,594 KB |
| `power_grid_1k` | 1,000 | 16 | 60 | 84.885 | 1414.76 µs | ~707 | 104 KB |
| `production_chain_1k` | 1,000 | 16 | 60 | 149.115 | 2485.25 µs | ~402 | 104 KB |
| `logistics_jobs_1k` | 1,350 | 16 | 60 | 2.089 | 34.81 µs | ~28,725 | 134 KB |

## Regional Activity Breakdown

| Scenario | Hot Activity (Jobs / Entity Ticks) | Warm Activity (Jobs / Entity Ticks) | Cold Activity (Jobs / Entity Ticks) | Messages Routed |
|---|---:|---:|---:|---:|
| `10k_walls` | 0 / 0 | 0 / 0 | 0 / 0 | 0 |
| `1k_idle_units` | 0 / 0 | 240 / 30,000 | 0 / 0 | 0 |
| `hot_vs_cold` | 120 / 60,000 | 0 / 0 | 0 / 0 | 0 |
| `scheduled_factories` | 0 / 0 | 0 / 0 | 5 / 1,000 | 0 |
| `event_queue_stress` | 240 / 2,400 | 0 / 0 | 240 / 2,400 | 24,990 |
| `power_grid_1k` | 960 / 0 | 0 / 0 | 0 / 0 | 0 |
| `production_chain_1k` | 960 / 0 | 0 / 0 | 0 / 0 | 0 |
| `logistics_jobs_1k` | 960 / 21,000 | 0 / 0 | 0 / 0 | 0 |

## Key Findings

1. **Inert Entity Scalability**: 10,000 walls in cold regions run at over 600,000 simulation ticks/sec (< 1.7 µs per tick) because cold regions incur zero per-entity iteration overhead.
2. **Multi-Rate Regional LOD**: Warm regions run at half-rate (240 jobs across 120 ticks), while Cold regions remain completely dormant until an event or scheduled wakeup occurs.
3. **Event & Message Queue Throughput**: Over 24,000 cross-region messages were routed and dispatched across 16 regions in ~3.3 ms (~7.5 million messages/second routing throughput) under backpressure policies.
4. **Logistics Engine Efficiency**: 1,000 jobs across 100 depots, 250 haulers, and 16 regions run at ~28,700 ticks/sec (~34.8 µs per tick) in only 134 KB of memory with abstract bulk cargo and Dijkstra shortest-path route evaluation.
5. **Decoupled Multithreaded Server**: Dedicated network ingress and egress threads allow packet receiving and snapshot broadcasting to execute concurrently with the authoritative 30 Hz simulation loop and background worker pool.
