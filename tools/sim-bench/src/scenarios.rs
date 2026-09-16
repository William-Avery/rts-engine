use crate::report::BenchmarkResult;
use game_types::{FactionId, RegionId, SimTick};
use sim_core::message_queue::{BackpressurePolicy, CrossRegionPayload};
use sim_core::region::{Region, RegionBounds, RegionState};
use sim_core::scheduler::WakeupReason;
use sim_core::test_harness::TestHarness;
use std::time::Instant;

fn sample_memory_bytes(mem_before: usize) -> usize {
    let mem_after = crate::current_allocated_bytes();
    let delta = mem_after.saturating_sub(mem_before);
    if delta > 0 { delta } else { mem_after }
}

/// Scenario 1: 10,000 inert static structures (walls) in cold regions.
pub fn scenario_10k_walls() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let num_walls = 10_000;
    let num_regions = 16;
    let ticks_to_run = 60; // 2 seconds of 30 Hz simulation

    // Setup 16 cold regions in a 4x4 grid
    for r in 1..=num_regions {
        let reg_id = RegionId::new(r as u32);
        let col = ((r - 1) % 4) as f32;
        let row = ((r - 1) / 4) as f32;
        let bounds = RegionBounds::new(
            col * 250.0,
            row * 250.0,
            (col + 1.0) * 250.0,
            (row + 1.0) * 250.0,
        )
        .unwrap();
        harness
            .state_mut()
            .region_map
            .add_region(Region::new(reg_id, bounds, RegionState::Cold))
            .unwrap();
    }

    // Distribute 10,000 wall entities evenly across cold regions and compact wall grid with mixed tiers
    for i in 0..num_walls {
        let reg_id = RegionId::new(((i % num_regions) + 1) as u32);
        harness.create_entity(FactionId::new(1), reg_id);
        let gx = (i % 100) as i32;
        let gz = (i / 100) as i32;
        let tier = ((i % 3) + 1) as u8; // Mixed Mk1, Mk2, Mk3 tiers
        harness
            .state_mut()
            .structure_registry
            .wall_grid
            .insert_wall(gx, gz, tier, FactionId::new(1));
    }

    let start = Instant::now();
    harness.run_for_ticks(ticks_to_run);
    let elapsed = start.elapsed();

    let metrics = harness.state().scheduler.metrics();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "10k_walls".to_string(),
        description: "10,000 inert walls in 16 cold regions".to_string(),
        entity_count: num_walls,
        region_count: num_regions,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}

/// Scenario 2: 1,000 idle units across warm and cold regions.
pub fn scenario_1k_idle_units() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let num_units = 1_000;
    let num_regions = 8;
    let ticks_to_run = 120; // 4 seconds at 30 Hz

    // 4 Warm regions and 4 Cold regions
    for r in 1..=num_regions {
        let reg_id = RegionId::new(r as u32);
        let state = if r <= 4 {
            RegionState::Warm
        } else {
            RegionState::Cold
        };
        let bounds =
            RegionBounds::new((r as f32) * 100.0, 0.0, ((r + 1) as f32) * 100.0, 100.0).unwrap();
        harness
            .state_mut()
            .region_map
            .add_region(Region::new(reg_id, bounds, state))
            .unwrap();
    }

    for i in 0..num_units {
        let reg_id = RegionId::new(((i % num_regions) + 1) as u32);
        harness.create_entity(FactionId::new(1), reg_id);
    }

    let start = Instant::now();
    harness.run_for_ticks(ticks_to_run);
    let elapsed = start.elapsed();

    let metrics = harness.state().scheduler.metrics();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "1k_idle_units".to_string(),
        description: "1,000 idle units split across 4 warm and 4 cold regions".to_string(),
        entity_count: num_units,
        region_count: num_regions,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}

/// Scenario 3: Hot vs Cold regions comparison with identical entity counts.
pub fn scenario_hot_vs_cold() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let entities_per_region = 500;
    let num_entities = entities_per_region * 2;
    let ticks_to_run = 120; // 4 seconds at 30 Hz

    let reg_hot = RegionId::new(1);
    let reg_cold = RegionId::new(2);

    harness
        .state_mut()
        .region_map
        .add_region(Region::new(
            reg_hot,
            RegionBounds::new(0.0, 0.0, 500.0, 500.0).unwrap(),
            RegionState::Hot,
        ))
        .unwrap();

    harness
        .state_mut()
        .region_map
        .add_region(Region::new(
            reg_cold,
            RegionBounds::new(500.0, 0.0, 1000.0, 500.0).unwrap(),
            RegionState::Cold,
        ))
        .unwrap();

    for _ in 0..entities_per_region {
        harness.create_entity(FactionId::new(1), reg_hot);
        harness.create_entity(FactionId::new(1), reg_cold);
    }

    let start = Instant::now();
    harness.run_for_ticks(ticks_to_run);
    let elapsed = start.elapsed();

    let metrics = harness.state().scheduler.metrics();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "hot_vs_cold".to_string(),
        description: "500 entities in Hot vs 500 entities in Cold region".to_string(),
        entity_count: num_entities,
        region_count: 2,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}

/// Scenario 4: Scheduled factories in cold regions waking up periodically.
pub fn scenario_scheduled_factories() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let num_factories = 200;
    let ticks_to_run = 150; // 5 seconds at 30 Hz

    let reg_factory = RegionId::new(10);
    harness
        .state_mut()
        .region_map
        .add_region(Region::new(
            reg_factory,
            RegionBounds::new(0.0, 0.0, 1000.0, 1000.0).unwrap(),
            RegionState::Cold,
        ))
        .unwrap();

    for _ in 0..num_factories {
        harness.create_entity(FactionId::new(1), reg_factory);
    }

    // Schedule production wakeups every 30 ticks (at tick 30, 60, 90, 120, 150)
    for cycle in 1..=5 {
        let tick = SimTick::new(cycle * 30);
        harness
            .state_mut()
            .scheduler
            .schedule_wakeup(tick, reg_factory, WakeupReason::PeriodicTimer)
            .unwrap();
    }

    let start = Instant::now();
    harness.run_for_ticks(ticks_to_run);
    let elapsed = start.elapsed();

    let metrics = harness.state().scheduler.metrics();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "scheduled_factories".to_string(),
        description: "200 factories waking up every 30 ticks in cold region".to_string(),
        entity_count: num_factories,
        region_count: 1,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}

/// Scenario 5: High-volume cross-region message routing under backpressure.
pub fn scenario_event_queue_stress() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let num_regions = 16;
    let total_messages = 25_000;
    let ticks_to_run = 30; // 1 second

    // Configure 16 regions (half Hot, half Cold) with high queue capacity and DropOldest backpressure
    for r in 1..=num_regions {
        let reg_id = RegionId::new(r as u32);
        let state = if r % 2 == 0 {
            RegionState::Hot
        } else {
            RegionState::Cold
        };
        let bounds =
            RegionBounds::new((r as f32) * 50.0, 0.0, ((r + 1) as f32) * 50.0, 50.0).unwrap();
        harness
            .state_mut()
            .region_map
            .add_region(Region::new(reg_id, bounds, state))
            .unwrap();

        harness.state_mut().router.set_region_queue_policy(
            reg_id,
            2048,
            BackpressurePolicy::DropOldest,
        );

        // Add 10 entities per region
        for _ in 0..10 {
            harness.create_entity(FactionId::new(1), reg_id);
        }
    }

    let start = Instant::now();

    // Pump 25,000 cross-region messages across the 30 ticks
    let msgs_per_tick = total_messages / (ticks_to_run as usize);
    for t in 1..=ticks_to_run {
        let current_tick = SimTick::new(t);
        for m in 0..msgs_per_tick {
            let from = RegionId::new(((m % num_regions) + 1) as u32);
            let to = RegionId::new((((m + 1) % num_regions) + 1) as u32);
            let _ = harness.state_mut().router.send(
                from,
                to,
                current_tick,
                CrossRegionPayload::Signal {
                    signal_id: m as u32,
                    data: current_tick.value(),
                },
            );
        }
        harness.step_tick();
    }

    let elapsed = start.elapsed();
    let metrics = harness.state().scheduler.metrics();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "event_queue_stress".to_string(),
        description: "25,000 cross-region messages routed across 16 regions".to_string(),
        entity_count: num_regions * 10,
        region_count: num_regions,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}

/// Scenario 6: 1,000 power network structures (generators, pylons, batteries, turrets, fabricators) across multi-island grids.
pub fn scenario_power_grid_1k() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let num_structures = 1_000;
    let num_regions = 16;
    let ticks_to_run = 60; // 2 seconds of 30 Hz simulation

    // Setup 16 regions
    for r in 1..=num_regions {
        let reg_id = RegionId::new(r as u32);
        let col = ((r - 1) % 4) as f32;
        let row = ((r - 1) / 4) as f32;
        let bounds = RegionBounds::new(
            col * 250.0,
            row * 250.0,
            (col + 1.0) * 250.0,
            (row + 1.0) * 250.0,
        )
        .unwrap();
        harness
            .state_mut()
            .region_map
            .add_region(Region::new(reg_id, bounds, RegionState::Hot))
            .unwrap();
    }

    // Place 1,000 structures in a distributed power network
    for i in 0..num_structures {
        let reg_id = RegionId::new(((i % num_regions) + 1) as u32);
        let col = (i % 25) as f32 * 10.0;
        let row = (i / 25) as f32 * 10.0;
        let pos = (col, 0.0, row);

        let kind = match i % 10 {
            0 => sim_core::structure::StructureKind::Generator,
            1..=4 => sim_core::structure::StructureKind::Pylon,
            5..=7 => sim_core::structure::StructureKind::Turret,
            8 => sim_core::structure::StructureKind::Fabricator,
            _ => sim_core::structure::StructureKind::Battery,
        };

        let id = harness
            .state_mut()
            .structure_registry
            .request_build(
                sim_core::structure::BuildRequest {
                    player_pos: pos,
                    requested_pos: pos,
                    kind,
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1),
                    region_id: reg_id,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-1000.0, 1000.0, -1000.0, 1000.0),
                },
                None,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .complete_construction(id)
            .unwrap();
    }

    let start = Instant::now();
    harness.run_for_ticks(ticks_to_run);
    let elapsed = start.elapsed();

    let metrics = harness.state().scheduler.metrics();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "power_grid_1k".to_string(),
        description: "1,000 power structures across multi-island grids with battery buffering"
            .to_string(),
        entity_count: num_structures,
        region_count: num_regions,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}

/// Scenario 7: 1,000 industrial production structures (miners, refineries, fabricators, generators).
pub fn scenario_production_chain_1k() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let num_structures = 1_000;
    let num_regions = 16;
    let ticks_to_run = 60;

    for r in 1..=num_regions {
        let reg_id = RegionId::new(r as u32);
        let col = ((r - 1) % 4) as f32;
        let row = ((r - 1) / 4) as f32;
        let bounds = RegionBounds::new(
            col * 500.0,
            row * 500.0,
            (col + 1.0) * 500.0,
            (row + 1.0) * 500.0,
        )
        .unwrap();
        harness
            .state_mut()
            .region_map
            .add_region(Region::new(reg_id, bounds, RegionState::Hot))
            .unwrap();
    }

    // Register 100 deposits
    for d in 1..=100 {
        let dep_id = game_types::DepositId::new(d as u64);
        let res_id = if d % 2 == 0 {
            game_types::RES_IRON_ORE
        } else {
            game_types::RES_TUNGSTEN_ORE
        };
        harness.state_mut().structure_registry.register_deposit(
            sim_core::production::ResourceDeposit::new(
                dep_id,
                res_id,
                ((d % 10) as f32 * 50.0, 0.0, (d / 10) as f32 * 50.0),
                100_000,
                1.0,
            ),
        );
    }

    for i in 0..num_structures {
        let reg_id = RegionId::new(((i % num_regions) + 1) as u32);
        let col = (i % 25) as f32 * 15.0;
        let row = (i / 25) as f32 * 15.0;
        let pos = (col, 0.0, row);

        let kind = match i % 5 {
            0 => sim_core::structure::StructureKind::Generator,
            1 => sim_core::structure::StructureKind::MiningDrill,
            2 => sim_core::structure::StructureKind::Refinery,
            3 => sim_core::structure::StructureKind::Fabricator,
            _ => sim_core::structure::StructureKind::Pylon,
        };

        let id = harness
            .state_mut()
            .structure_registry
            .request_build(
                sim_core::structure::BuildRequest {
                    player_pos: pos,
                    requested_pos: pos,
                    kind,
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1),
                    region_id: reg_id,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-2000.0, 2000.0, -2000.0, 2000.0),
                },
                None,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .complete_construction(id)
            .unwrap();

        // Configure facility tasks and initial inputs
        if let Some(fac) = harness.state_mut().structure_registry.get_facility_mut(id) {
            match fac.kind {
                sim_core::production::FacilityKind::MiningDrill => {
                    let dep_id = game_types::DepositId::new(((i % 100) + 1) as u64);
                    fac.set_deposit(dep_id);
                }
                sim_core::production::FacilityKind::Refinery => {
                    fac.set_recipe(sim_core::production::RECIPE_SMELT_STEEL)
                        .unwrap();
                    let _ = fac.input_inventory.add(game_types::RES_IRON_ORE, 500);
                }
                sim_core::production::FacilityKind::Fabricator => {
                    fac.set_recipe(sim_core::production::RECIPE_FABRICATE_BASIC_COMPONENTS)
                        .unwrap();
                    let _ = fac.input_inventory.add(game_types::RES_STEEL, 500);
                }
            }
        }
    }

    let start = Instant::now();
    harness.run_for_ticks(ticks_to_run);
    let elapsed = start.elapsed();

    let metrics = harness.state().scheduler.metrics();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "production_chain_1k".to_string(),
        description:
            "1,000 industrial facilities (miners, refineries, fabricators) executing concurrent production cycles"
                .to_string(),
        entity_count: num_structures,
        region_count: num_regions,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}

/// Scenario 8: 1,000 concurrent logistics jobs across 16 regions with depots, docks, and route graph.
pub fn scenario_logistics_jobs_1k() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let num_jobs = 1000;
    let num_regions = 16;
    let num_depots = 100;
    let num_haulers = 250;
    let ticks_to_run = 60; // 2 seconds of 30 Hz simulation

    // Setup 16 regions in 4x4 grid
    for r in 1..=num_regions {
        let reg_id = RegionId::new(r as u32);
        let col = ((r - 1) % 4) as f32;
        let row = ((r - 1) / 4) as f32;
        let bounds = RegionBounds::new(
            col * 250.0,
            row * 250.0,
            (col + 1.0) * 250.0,
            (row + 1.0) * 250.0,
        )
        .unwrap();
        harness
            .state_mut()
            .region_map
            .add_region(Region::new(reg_id, bounds, RegionState::Hot))
            .unwrap();
    }

    // Setup 100 depots with inventories and docks
    let mut depot_entities = Vec::with_capacity(num_depots);
    for i in 0..num_depots {
        let reg_id = RegionId::new(((i % num_regions) + 1) as u32);
        let ent = harness.create_entity(FactionId::new(1), reg_id);
        depot_entities.push(ent);

        let mut inv =
            sim_core::inventory::Inventory::new(ent, sim_core::inventory::ContainerKind::Depot);
        let _ = inv.add(game_types::RES_IRON_ORE, 10_000);
        let _ = inv.add(game_types::RES_STEEL, 10_000);
        harness.state_mut().inventory_registry.register(inv);

        let dock = sim_core::logistics::LogisticsDock::new(ent, 4, 25);
        harness
            .state_mut()
            .structure_registry
            .logistics
            .register_dock(dock);
        let depot_logistics = sim_core::logistics::DepotLogistics::new(ent, 50.0);
        harness
            .state_mut()
            .structure_registry
            .logistics
            .register_depot(depot_logistics);

        // Add to route graph
        let gx = ((i % 10) as f32) * 100.0;
        let gz = ((i / 10) as f32) * 100.0;
        harness
            .state_mut()
            .structure_registry
            .logistics
            .route_graph
            .add_node(sim_core::logistics::RouteNode {
                id: game_types::RouteNodeId::new((i + 1) as u32),
                position: (gx, 0.0, gz),
                associated_entity: Some(ent),
            });
    }

    // Connect adjacent route graph nodes
    for i in 0..num_depots {
        let curr = game_types::RouteNodeId::new((i + 1) as u32);
        if (i % 10) < 9 {
            let next_e = game_types::RouteNodeId::new((i + 2) as u32);
            harness
                .state_mut()
                .structure_registry
                .logistics
                .route_graph
                .add_edge(sim_core::logistics::RouteEdge {
                    from: curr,
                    to: next_e,
                    distance: 100.0,
                    max_active_haulers: 8,
                    current_haulers: 0,
                    traversal_speed: 1.0,
                });
        }
    }

    // Create 250 mobile hauler entities with buffers
    let mut hauler_entities = Vec::with_capacity(num_haulers);
    for i in 0..num_haulers {
        let reg_id = RegionId::new(((i % num_regions) + 1) as u32);
        let hauler = harness.create_entity(FactionId::new(1), reg_id);
        hauler_entities.push(hauler);
        harness
            .state_mut()
            .inventory_registry
            .register(sim_core::inventory::Inventory::new(
                hauler,
                sim_core::inventory::ContainerKind::CargoBuffer,
            ));
    }

    // Submit 1,000 logistics jobs with mixed priorities
    let mut job_ids = Vec::with_capacity(num_jobs);
    for i in 0..num_jobs {
        let src = depot_entities[i % num_depots];
        let dst = depot_entities[(i + 17) % num_depots];
        let priority = match i % 4 {
            0 => sim_core::logistics::JobPriority::Critical,
            1 => sim_core::logistics::JobPriority::High,
            2 => sim_core::logistics::JobPriority::Normal,
            _ => sim_core::logistics::JobPriority::Low,
        };
        let res = if i % 2 == 0 {
            game_types::RES_IRON_ORE
        } else {
            game_types::RES_STEEL
        };

        let jid = harness
            .create_logistics_job(FactionId::null(), src, dst, res, 25, priority)
            .unwrap();
        job_ids.push(jid);
    }

    // Initial assignment of the first 250 jobs to haulers
    for (idx, &hauler) in hauler_entities.iter().enumerate() {
        let jid = job_ids[idx];
        let _ = harness.claim_logistics_job(FactionId::null(), jid, hauler);
    }

    let start = Instant::now();
    harness.run_for_ticks(ticks_to_run);
    let elapsed = start.elapsed();

    let metrics = harness.state().scheduler.metrics();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "logistics_jobs_1k".to_string(),
        description:
            "1,000 concurrent logistics jobs across 16 regions with multi-tier depots and route graph routing"
                .to_string(),
        entity_count: num_jobs + num_depots + num_haulers,
        region_count: num_regions,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}

/// Scenario 9: 1,000 biped robots navigating, separating, escorting, and holding formation.
pub fn scenario_robots_1k() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let num_robots = 1_000;
    let num_regions = 16;
    let num_players = 8;
    let num_squads = 40;
    let ticks_to_run = 60; // 2 seconds of 30 Hz simulation
    let faction = FactionId::new(1);

    // Setup 16 hot regions in a 4x4 grid
    for r in 1..=num_regions {
        let reg_id = RegionId::new(r as u32);
        let col = ((r - 1) % 4) as f32;
        let row = ((r - 1) / 4) as f32;
        let bounds = RegionBounds::new(
            col * 250.0,
            row * 250.0,
            (col + 1.0) * 250.0,
            (row + 1.0) * 250.0,
        )
        .unwrap();
        harness
            .state_mut()
            .region_map
            .add_region(Region::new(reg_id, bounds, RegionState::Hot))
            .unwrap();
    }

    // 8 commanders, each raised to the 2-escort progression ceiling.
    for p in 1..=num_players {
        let player = game_types::PlayerId::new(p as u32);
        let reg_id = RegionId::new(((p % num_regions) + 1) as u32);
        let px = (p as f32) * 60.0;
        harness
            .state_mut()
            .register_player(player, faction, reg_id, (px, 0.0, 30.0))
            .unwrap();
        harness
            .state_mut()
            .robot_registry
            .config
            .grant_escort_cap(player, 2);
    }

    // 40 squads receiving the bulk of the robots.
    let squads: Vec<game_types::SquadId> = (0..num_squads)
        .map(|_| harness.state_mut().create_squad(faction))
        .collect();

    // 1,000 guardsman/rifleman bipeds distributed across the grid.
    let mut robots = Vec::with_capacity(num_robots);
    for i in 0..num_robots {
        let reg_id = RegionId::new(((i % num_regions) + 1) as u32);
        let chassis = if i % 4 == 0 {
            sim_core::chassis::RobotChassis::Guardsman
        } else {
            sim_core::chassis::RobotChassis::Rifleman
        };
        let x = ((i % 50) as f32) * 6.0;
        let z = ((i / 50) as f32) * 6.0;
        let robot = harness
            .state_mut()
            .spawn_robot(chassis, faction, reg_id, (x, 0.0, z))
            .unwrap();
        robots.push(robot);
    }

    // Assign 16 escorts (2 per commander) and place everything else into squads.
    let mut next = 0usize;
    for p in 1..=num_players {
        let player = game_types::PlayerId::new(p as u32);
        for _ in 0..2 {
            let robot = robots[next];
            next += 1;
            harness
                .state_mut()
                .assign_escort(player, player, robot)
                .unwrap();
        }
    }
    let commander = game_types::PlayerId::new(1);
    let squad_members: Vec<game_types::EntityId> = robots[next..].to_vec();
    {
        let state = harness.state_mut();
        for (idx, &robot) in squad_members.iter().enumerate() {
            let squad = squads[idx % num_squads];
            let _ = state.robot_registry.assign_squad_member(
                commander,
                squad,
                robot,
                SimTick::zero(),
                &mut state.event_journal,
            );
        }

        // Every squad reforms on its own rally point, so all members are actively pathing.
        for (idx, &squad) in squads.iter().enumerate() {
            let rally = (((idx % 8) as f32) * 80.0, 0.0, ((idx / 8) as f32) * 80.0);
            let _ = state.robot_registry.regroup_squad(
                commander,
                squad,
                rally,
                SimTick::zero(),
                &mut state.event_journal,
            );
        }
    }

    let start = Instant::now();
    harness.run_for_ticks(ticks_to_run);
    let elapsed = start.elapsed();

    let metrics = harness.state().scheduler.metrics();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "robots_1k".to_string(),
        description:
            "1,000 biped robots across 16 hot regions with navigation, flocking separation, escort, and formation"
                .to_string(),
        entity_count: num_robots + num_players,
        region_count: num_regions,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}

/// Scenario 10: 1,000 research structures across 64 factions, with the full
/// tech tree queued and a large faction-wide modifier evaluation sweep.
pub fn scenario_research_modifiers_1k() -> BenchmarkResult {
    let mem_before = crate::current_allocated_bytes();
    let mut harness = TestHarness::new();
    let num_labs = 500;
    let num_structures = num_labs * 2; // paired lab + generator
    let num_factions = 64;
    let num_regions = 16;
    // 10 seconds of 30 Hz simulation: long enough for tier-1 and tier-2
    // technologies (90-180 ticks) to actually complete and publish patches.
    let ticks_to_run = 300;
    let modifier_eval_passes = 100;

    // Setup 16 regions in a 4x4 grid
    for r in 1..=num_regions {
        let reg_id = RegionId::new(r as u32);
        let col = ((r - 1) % 4) as f32;
        let row = ((r - 1) / 4) as f32;
        let bounds = RegionBounds::new(
            col * 250.0,
            row * 250.0,
            (col + 1.0) * 250.0,
            (row + 1.0) * 250.0,
        )
        .unwrap();
        harness
            .state_mut()
            .region_map
            .add_region(Region::new(reg_id, bounds, RegionState::Hot))
            .unwrap();
    }

    // Place 500 powered research laboratories, each paired with its own generator
    let mut lab_ids = Vec::with_capacity(num_labs);
    for i in 0..num_labs {
        let reg_id = RegionId::new(((i % num_regions) + 1) as u32);
        let faction = FactionId::new(((i % num_factions) + 1) as u32);
        let lab_pos = ((i % 25) as f32 * 20.0, 0.0, (i / 25) as f32 * 20.0);
        let gen_pos = (lab_pos.0 + 8.0, 0.0, lab_pos.2);

        for (kind, pos) in [
            (
                sim_core::structure::StructureKind::ResearchFacility,
                lab_pos,
            ),
            (sim_core::structure::StructureKind::Generator, gen_pos),
        ] {
            let id = harness
                .state_mut()
                .structure_registry
                .request_build(
                    sim_core::structure::BuildRequest {
                        player_pos: pos,
                        requested_pos: pos,
                        kind,
                        rotation_deg: 0.0,
                        faction_id: faction,
                        region_id: reg_id,
                        creation_tick: SimTick::zero(),
                        world_bounds_xz: (-1000.0, 1000.0, -1000.0, 1000.0),
                    },
                    None,
                )
                .unwrap();
            harness
                .state_mut()
                .structure_registry
                .complete_construction(id)
                .unwrap();
            if kind == sim_core::structure::StructureKind::ResearchFacility {
                lab_ids.push(id);
            }
        }
    }

    // Discover the laboratories and stock their hoppers with research materials
    {
        let state = harness.state_mut();
        state
            .research_manager
            .sync_facilities(&state.structure_registry);
    }
    for lab in &lab_ids {
        if let Some(facility) = harness.state_mut().research_manager.facility_mut(*lab) {
            let _ = facility.input_inventory.add(game_types::RES_STEEL, 80);
            let _ = facility
                .input_inventory
                .add(game_types::RES_BASIC_COMPONENTS, 30);
            let _ = facility
                .input_inventory
                .add(game_types::RES_ENERGY_CELL, 30);
        }
    }

    // Queue the entire available prerequisite frontier for every faction
    let mut queued_jobs = 0usize;
    for f in 1..=num_factions {
        let faction = FactionId::new(f as u32);
        let available = harness
            .state()
            .research_manager
            .available_techs(faction)
            .to_vec();
        for tech in available {
            let state = harness.state_mut();
            let tick = state.tick;
            if state
                .research_manager
                .queue_research(faction, tech, tick, &mut state.event_journal)
                .is_ok()
            {
                queued_jobs += 1;
            }
        }
    }

    let start = Instant::now();
    harness.run_for_ticks(ticks_to_run);

    // Faction-wide modifier evaluation sweep: every kind, every faction, many passes.
    let mut checksum: i64 = 0;
    for _ in 0..modifier_eval_passes {
        for f in 1..=num_factions {
            let faction = FactionId::new(f as u32);
            for kind in sim_core::modifier::ALL_MODIFIER_KINDS {
                checksum = checksum.wrapping_add(
                    harness
                        .state()
                        .structure_registry
                        .modifiers
                        .value_for_milli(faction, *kind, 1_000),
                );
            }
        }
    }
    let elapsed = start.elapsed();
    std::hint::black_box(checksum);

    let metrics = harness.state().scheduler.metrics();
    let research_metrics = harness.state().research_manager.metrics.clone();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = if ticks_to_run > 0 {
        (total_ms * 1000.0) / (ticks_to_run as f64)
    } else {
        0.0
    };
    let tps = if total_ms > 0.0 {
        (ticks_to_run as f64) / (total_ms / 1000.0)
    } else {
        0.0
    };
    let modifier_evals =
        modifier_eval_passes * num_factions * sim_core::modifier::ALL_MODIFIER_KINDS.len();

    let memory_bytes = sample_memory_bytes(mem_before);
    BenchmarkResult {
        scenario_name: "research_modifiers_1k".to_string(),
        description: format!(
            "{num_structures} research structures ({num_labs} labs) across {num_factions} factions and {num_regions} regions; {queued_jobs} queued research jobs, {} started, {} completed, {modifier_evals} modifier evaluations",
            research_metrics.jobs_started, research_metrics.jobs_completed
        ),
        entity_count: num_structures,
        region_count: num_regions,
        ticks_run: ticks_to_run,
        total_duration_ms: total_ms,
        avg_tick_us: avg_us,
        ticks_per_sec: tps,
        jobs_executed_hot: metrics.jobs_executed_hot,
        jobs_executed_warm: metrics.jobs_executed_warm,
        jobs_executed_cold: metrics.jobs_executed_cold,
        entities_ticked_hot: metrics.entities_ticked_hot,
        entities_ticked_warm: metrics.entities_ticked_warm,
        entities_ticked_cold: metrics.entities_ticked_cold,
        messages_routed: metrics.cross_region_messages_processed,
        measured_memory_bytes: memory_bytes,
        estimated_memory_bytes: memory_bytes,
    }
}
