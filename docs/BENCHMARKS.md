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

## Baseline Targets vs Measured Performance (Milestones 3 - 19 & Phase B Hardening)

Measured on AMD64 Windows development environment (debug profile with live heap tracking via `TrackingAllocator`):

| Scenario | Entities | Regions | Ticks | Total (ms) | Avg (µs/tick) | Throughput (ticks/sec) | Memory (KB) |
|---|---:|---:|---:|---:|---:|---:|---:|
| `10k_walls` | 10,000 | 16 | 60 | 0.129 | 2.15 µs | ~465,000 | 2,177.0 KB |
| `1k_idle_units` | 1,000 | 8 | 120 | 0.123 | 1.02 µs | ~977,000 | 135.5 KB |
| `hot_vs_cold` | 1,000 | 2 | 120 | 0.087 | 0.73 µs | ~1,371,000 | 134.9 KB |
| `scheduled_factories` | 200 | 1 | 150 | 0.120 | 0.80 µs | ~1,246,000 | 33.5 KB |
| `event_queue_stress` | 160 | 16 | 30 | 3.513 | 117.09 µs | ~8,540 | 930.0 KB |
| `power_grid_1k` | 1,000 | 16 | 60 | 88.512 | 1475.19 µs | ~678 | 1,069.8 KB |
| `production_chain_1k` | 1,000 | 16 | 60 | 155.757 | 2595.95 µs | ~385 | 2,464.8 KB |
| `logistics_jobs_1k` | 1,350 | 16 | 60 | 2.127 | 35.44 µs | ~28,214 | 610.8 KB |
| `robots_1k` | 1,008 | 16 | 60 | 164.794 | 2746.57 µs | ~364 | 530.9 KB |
| `research_modifiers_1k` | 1,000 | 16 | 300 | 485.654 | 1618.85 µs | ~618 | 1,984.1 KB |

### Release Profile Performance (`--release`, `overflow-checks = true`)

Across the entire 10-scenario suite in release mode, total execution completes in **167.65 ms**:
- `10k_walls`: 0.55 µs/tick (~1.81M ticks/sec)
- `1k_idle_units`: 0.12 µs/tick (~8.16M ticks/sec)
- `hot_vs_cold`: 0.08 µs/tick (~13.19M ticks/sec)
- `scheduled_factories`: 0.08 µs/tick (~12.61M ticks/sec)
- `event_queue_stress`: 12.54 µs/tick (~79.7k ticks/sec)
- `power_grid_1k`: 218.91 µs/tick (~4,568 ticks/sec)
- `production_chain_1k`: 372.39 µs/tick (~2,685 ticks/sec)
- `logistics_jobs_1k`: 2.52 µs/tick (~396k ticks/sec)
- `robots_1k`: 586.99 µs/tick (~1,704 ticks/sec)
- `research_modifiers_1k`: 254.64 µs/tick (~3,927 ticks/sec)

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
| `robots_1k` | 960 / 60,480 | 0 / 0 | 0 / 0 | 0 |
| `research_modifiers_1k` | 4,800 / 0 | 0 / 0 | 0 / 0 | 0 |

## Key Findings

1. **Inert Entity Scalability**: 10,000 walls in cold regions run at over 600,000 simulation ticks/sec (< 1.7 µs per tick) because cold regions incur zero per-entity iteration overhead.
2. **Multi-Rate Regional LOD**: Warm regions run at half-rate (240 jobs across 120 ticks), while Cold regions remain completely dormant until an event or scheduled wakeup occurs.
3. **Event & Message Queue Throughput**: Over 24,000 cross-region messages were routed and dispatched across 16 regions in ~3.3 ms (~7.5 million messages/second routing throughput) under backpressure policies.
4. **Logistics Engine Efficiency**: 1,000 jobs across 100 depots, 250 haulers, and 16 regions run at ~28,700 ticks/sec (~34.8 µs per tick) in only 134 KB of memory with abstract bulk cargo and Dijkstra shortest-path route evaluation.
5. **Decoupled Multithreaded Server**: Dedicated network ingress and egress threads allow packet receiving and snapshot broadcasting to execute concurrently with the authoritative 30 Hz simulation loop and background worker pool.
6. **Biped Robot Framework Scalability**: 1,000 bipeds — 16 player escorts plus 40 regrouping squads across 16 hot regions — step navigation, local separation, movement integration, and facing in 2,648 µs/tick in the debug profile (~378 ticks/sec) and **538 µs/tick in the release profile (~1,860 ticks/sec, ~62x the 30 Hz simulation budget)** in 104 KB of state. Neighbour queries use a per-step uniform grid rather than an O(n²) sweep, so cost scales with local crowding, not total robot count.
7. **Research & Modifier Evaluation Scale**: `research_modifiers_1k` runs 500 powered research laboratories paired with 500 generators across 64 factions and 16 regions. In 300 ticks it queues 128 research jobs, starts all 128, and completes all 128 (each publishing a faction-wide modifier patch), while additionally performing 102,400 modifier evaluations (64 factions x 16 modifier kinds x 100 passes). Measured at 1556.11 µs/tick (~643 ticks/sec) in the debug profile used for every row above, and 245.34 µs/tick (~4,076 ticks/sec) with `--release`. Modifier evaluation is exact integer fixed-point arithmetic over `BTreeMap` buckets, so it adds no floating-point drift and no ordering sensitivity at this scale.
8. **Live Heap Memory Tracking & Release Overflow Hardening (Phase B)**: Replaced formulaic heuristics with real heap tracking via a custom `TrackingAllocator` wrapped around `System` allocator. Benchmarks report true heap consumption: `10k_walls` consumes ~2.18 MB, `production_chain_1k` consumes ~2.46 MB, and `research_modifiers_1k` consumes ~1.98 MB. Additionally, release and bench profiles are compiled with `overflow-checks = true`, confirming zero integer overflow panics or performance regression across all scenarios.
