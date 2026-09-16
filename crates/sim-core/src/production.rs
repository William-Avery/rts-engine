use crate::event::{EventJournal, ProductionBlockedReason, SimEvent};
use crate::inventory::{ContainerKind, Inventory};
use crate::modifier::{ModifierKind, ModifierStore};
use crate::power::PowerStatus;
use game_types::{
    DepositId, EntityId, FactionId, GameError, GameResult, RES_AMMO, RES_BASIC_COMPONENTS,
    RES_CERAMIC_PLATE, RES_HARDENED_STEEL, RES_IRON_ORE, RES_REFINED_TUNGSTEN, RES_SILICATES,
    RES_STEEL, RES_TUNGSTEN_COMPOSITE, RES_TUNGSTEN_ORE, RecipeId, RegionId, ReservationId,
    ResourceId, SimTick, StructureId,
};
use std::collections::BTreeMap;

/// World resource deposit node representing harvestable raw minerals.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceDeposit {
    pub id: DepositId,
    pub resource_id: ResourceId,
    pub position: (f32, f32, f32),
    pub initial_quantity: u32,
    pub remaining_quantity: u32,
    pub purity: f32,
}

impl ResourceDeposit {
    pub fn new(
        id: DepositId,
        resource_id: ResourceId,
        position: (f32, f32, f32),
        quantity: u32,
        purity: f32,
    ) -> Self {
        ResourceDeposit {
            id,
            resource_id,
            position,
            initial_quantity: quantity,
            remaining_quantity: quantity,
            purity: purity.max(0.1),
        }
    }

    /// Extract an amount from the deposit, clamping to remaining balance.
    pub fn extract(&mut self, amount: u32) -> u32 {
        let to_extract = amount.min(self.remaining_quantity);
        self.remaining_quantity -= to_extract;
        to_extract
    }

    /// Returns true if the deposit has been completely exhausted.
    pub fn is_depleted(&self) -> bool {
        self.remaining_quantity == 0
    }
}

/// Facility role in the industrial production chain.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum FacilityKind {
    /// Stationary mechanical mining drill harvesting adjacent deposits
    MiningDrill,
    /// Smelting furnace and metallurgical refinery
    Refinery,
    /// Automated manufacturing fabricator for components and composite synthesis
    Fabricator,
}

// Canonical Recipe Constants
pub const RECIPE_SMELT_STEEL: RecipeId = RecipeId(1);
pub const RECIPE_SMELT_TUNGSTEN: RecipeId = RecipeId(2);
pub const RECIPE_SINTER_CERAMIC: RecipeId = RecipeId(3);
pub const RECIPE_HARDEN_STEEL: RecipeId = RecipeId(4);
pub const RECIPE_SYNTHESIZE_TUNGSTEN_COMPOSITE: RecipeId = RecipeId(5);
pub const RECIPE_FABRICATE_BASIC_COMPONENTS: RecipeId = RecipeId(6);
pub const RECIPE_FABRICATE_AMMO: RecipeId = RecipeId(7);

/// Recipe definition specifying inputs, outputs, duration, and target facility.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub id: RecipeId,
    pub name: &'static str,
    pub facility: FacilityKind,
    pub inputs: &'static [(ResourceId, u32)],
    pub outputs: &'static [(ResourceId, u32)],
    pub duration_ticks: u32,
}

const STATIC_RECIPES: &[Recipe] = &[
    // 1. Simple Steel Smelting: 2 Iron Ore -> 1 Steel Ingot (30 ticks)
    Recipe {
        id: RECIPE_SMELT_STEEL,
        name: "Steel Smelting",
        facility: FacilityKind::Refinery,
        inputs: &[(RES_IRON_ORE, 2)],
        outputs: &[(RES_STEEL, 1)],
        duration_ticks: 30,
    },
    // 2. Tungsten Smelting: 2 Tungsten Ore -> 1 Refined Tungsten (40 ticks)
    Recipe {
        id: RECIPE_SMELT_TUNGSTEN,
        name: "Tungsten Smelting",
        facility: FacilityKind::Refinery,
        inputs: &[(RES_TUNGSTEN_ORE, 2)],
        outputs: &[(RES_REFINED_TUNGSTEN, 1)],
        duration_ticks: 40,
    },
    // 3. Ceramic Sintering: 2 Silicates -> 1 Ceramic Plate (30 ticks)
    Recipe {
        id: RECIPE_SINTER_CERAMIC,
        name: "Ceramic Sintering",
        facility: FacilityKind::Refinery,
        inputs: &[(RES_SILICATES, 2)],
        outputs: &[(RES_CERAMIC_PLATE, 1)],
        duration_ticks: 30,
    },
    // 4. Steel Hardening: 2 Steel Ingot -> 1 Hardened Steel (35 ticks)
    Recipe {
        id: RECIPE_HARDEN_STEEL,
        name: "Steel Hardening",
        facility: FacilityKind::Refinery,
        inputs: &[(RES_STEEL, 2)],
        outputs: &[(RES_HARDENED_STEEL, 1)],
        duration_ticks: 35,
    },
    // 5. Tungsten Composite Synthesis: 1 Refined Tungsten + 1 Hardened Steel + 1 Ceramic Plate -> 1 Tungsten Composite (60 ticks)
    Recipe {
        id: RECIPE_SYNTHESIZE_TUNGSTEN_COMPOSITE,
        name: "Tungsten Composite Synthesis",
        facility: FacilityKind::Fabricator,
        inputs: &[
            (RES_REFINED_TUNGSTEN, 1),
            (RES_HARDENED_STEEL, 1),
            (RES_CERAMIC_PLATE, 1),
        ],
        outputs: &[(RES_TUNGSTEN_COMPOSITE, 1)],
        duration_ticks: 60,
    },
    // 6. Basic Components Assembly: 2 Steel Ingot -> 1 Basic Component (40 ticks)
    Recipe {
        id: RECIPE_FABRICATE_BASIC_COMPONENTS,
        name: "Basic Components Assembly",
        facility: FacilityKind::Fabricator,
        inputs: &[(RES_STEEL, 2)],
        outputs: &[(RES_BASIC_COMPONENTS, 1)],
        duration_ticks: 40,
    },
    // 7. Ballistic Ammo Manufacturing: 1 Steel Ingot -> 50 Ammo (20 ticks)
    Recipe {
        id: RECIPE_FABRICATE_AMMO,
        name: "Ballistic Ammo Manufacturing",
        facility: FacilityKind::Fabricator,
        inputs: &[(RES_STEEL, 1)],
        outputs: &[(RES_AMMO, 50)],
        duration_ticks: 20,
    },
];

/// Look up a recipe by its unique ID.
pub fn get_recipe(id: RecipeId) -> Option<&'static Recipe> {
    STATIC_RECIPES.iter().find(|r| r.id == id)
}

/// Retrieve all static recipes registered in the catalog.
pub fn all_recipes() -> &'static [Recipe] {
    STATIC_RECIPES
}

/// Research-derived multipliers applied to an industrial facility for one tick.
///
/// Resolved from the faction modifier network by the caller so the production
/// state machine stays a pure function of its inputs. All values are fixed-point
/// thousandths; `1000` is neutral.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ProductionModifiers {
    /// Ore recovered per extraction cycle.
    pub mining_yield_milli: i64,
    /// Mining cycle rate (higher shortens the interval between cycles).
    pub mining_speed_milli: i64,
    /// Refining and fabrication rate (higher shortens craft duration).
    pub refining_speed_milli: i64,
}

impl Default for ProductionModifiers {
    fn default() -> Self {
        ProductionModifiers {
            mining_yield_milli: crate::modifier::MODIFIER_SCALE,
            mining_speed_milli: crate::modifier::MODIFIER_SCALE,
            refining_speed_milli: crate::modifier::MODIFIER_SCALE,
        }
    }
}

impl ProductionModifiers {
    /// Neutral multipliers (no research applied).
    pub fn neutral() -> Self {
        ProductionModifiers::default()
    }

    /// Resolve the multipliers a faction's completed research grants.
    pub fn from_store(store: &ModifierStore, faction: FactionId) -> Self {
        ProductionModifiers {
            mining_yield_milli: store.multiplier_milli(faction, ModifierKind::MiningYield),
            mining_speed_milli: store.multiplier_milli(faction, ModifierKind::MiningSpeed),
            refining_speed_milli: store.multiplier_milli(faction, ModifierKind::RefiningSpeed),
        }
    }

    /// Scale an integer quantity by a milli multiplier, never below `floor`.
    fn scale(base: u32, milli: i64, floor: u32) -> u32 {
        let scaled = (base as i64).saturating_mul(milli) / crate::modifier::MODIFIER_SCALE;
        scaled.clamp(floor as i64, u32::MAX as i64) as u32
    }

    /// Divide a duration by a rate multiplier, never below one tick.
    fn shorten(base: u32, milli: i64) -> u32 {
        if milli <= 0 {
            return base.max(1);
        }
        let scaled = (base as i64).saturating_mul(crate::modifier::MODIFIER_SCALE) / milli;
        scaled.clamp(1, u32::MAX as i64) as u32
    }

    /// Effective ore yield for a cycle after research.
    pub fn effective_yield(&self, base: u32) -> u32 {
        Self::scale(base, self.mining_yield_milli, 1)
    }

    /// Effective ticks between mining cycles after research.
    pub fn effective_mining_interval(&self, base: u32) -> u32 {
        Self::shorten(base, self.mining_speed_milli)
    }

    /// Effective craft duration in ticks after research.
    pub fn effective_craft_ticks(&self, base: u32) -> u32 {
        Self::shorten(base, self.refining_speed_milli)
    }
}

/// State machine for manufacturing and refining facilities.
#[derive(Clone, PartialEq, Debug)]
pub enum ProductionState {
    /// Idle, awaiting active recipe assignment or input materials
    Idle,
    /// Active recipe assigned, but missing required input ingredients
    AwaitingInputs,
    /// Actively processing with inputs locked under two-phase reservations
    Crafting {
        recipe_id: RecipeId,
        reservation_ids: Vec<ReservationId>,
        start_tick: SimTick,
        progress_ticks: u32,
        total_ticks: u32,
    },
    /// Craft complete, but output hopper/buffer is full
    OutputBlocked {
        recipe_id: RecipeId,
        reservation_ids: Vec<ReservationId>,
    },
    /// Electrical deficit/blackout halted progress; preserved until power restoration
    Unpowered {
        recipe_id: RecipeId,
        reservation_ids: Vec<ReservationId>,
        start_tick: SimTick,
        progress_ticks: u32,
        total_ticks: u32,
    },
}

/// State machine for mining drill resource extraction.
#[derive(Clone, PartialEq, Debug)]
pub enum MiningDrillState {
    /// Idle, awaiting target deposit assignment or deposit is exhausted
    Idle,
    /// Actively extracting mineral batches from target deposit
    Mining {
        deposit_id: DepositId,
        progress_ticks: u32,
        interval_ticks: u32,
        yield_per_cycle: u32,
    },
    /// Extracted minerals ready, but output hopper is full
    OutputBlocked {
        deposit_id: DepositId,
        extracted_pending: u32,
    },
    /// Electrical blackout halted extraction
    Unpowered {
        deposit_id: DepositId,
        progress_ticks: u32,
        interval_ticks: u32,
        yield_per_cycle: u32,
    },
}

/// Authoritative production facility component attached to industrial structures.
#[derive(Clone, Debug, PartialEq)]
pub struct ProductionFacility {
    pub structure_id: StructureId,
    pub faction_id: FactionId,
    pub region_id: RegionId,
    pub kind: FacilityKind,
    pub active_recipe: Option<RecipeId>,
    pub target_deposit: Option<DepositId>,
    pub input_inventory: Inventory,
    pub output_inventory: Inventory,
    pub production_state: ProductionState,
    pub mining_state: MiningDrillState,
    pub total_cycles_completed: u64,
    pub scheduled_completion_tick: Option<SimTick>,
    next_reservation_seq: u64,
}

impl ProductionFacility {
    /// Create a new production facility component for an industrial structure.
    pub fn new(
        structure_id: StructureId,
        faction_id: FactionId,
        region_id: RegionId,
        kind: FacilityKind,
    ) -> Self {
        let owner_ent = EntityId::new(structure_id.0);

        let (in_slots, in_vol, out_slots, out_vol) = match kind {
            FacilityKind::MiningDrill => (0, 0, 10, 2000),
            FacilityKind::Refinery => (12, 2000, 12, 2000),
            FacilityKind::Fabricator => (16, 2500, 16, 2500),
        };

        let input_inventory =
            Inventory::with_custom_capacity(owner_ent, ContainerKind::Hopper, in_slots, in_vol);
        let output_inventory =
            Inventory::with_custom_capacity(owner_ent, ContainerKind::Hopper, out_slots, out_vol);

        ProductionFacility {
            structure_id,
            faction_id,
            region_id,
            kind,
            active_recipe: None,
            target_deposit: None,
            input_inventory,
            output_inventory,
            production_state: ProductionState::Idle,
            mining_state: MiningDrillState::Idle,
            total_cycles_completed: 0,
            scheduled_completion_tick: None,
            next_reservation_seq: 1,
        }
    }

    /// Set active recipe for refineries and fabricators.
    pub fn set_recipe(&mut self, recipe_id: RecipeId) -> GameResult<()> {
        let recipe = get_recipe(recipe_id).ok_or(GameError::InvalidId)?;
        if recipe.facility != self.kind {
            return Err(GameError::InvalidStructureState);
        }
        self.active_recipe = Some(recipe_id);
        if matches!(self.production_state, ProductionState::Idle) {
            self.production_state = ProductionState::AwaitingInputs;
        }
        Ok(())
    }

    /// Assign target deposit for mining drills.
    pub fn set_deposit(&mut self, deposit_id: DepositId) {
        self.target_deposit = Some(deposit_id);
        if matches!(self.mining_state, MiningDrillState::Idle) {
            self.mining_state = MiningDrillState::Mining {
                deposit_id,
                progress_ticks: 0,
                interval_ticks: 15,
                yield_per_cycle: 2,
            };
        }
    }

    /// Generate a unique reservation ID for transactional input locking.
    fn next_reservation_id(&mut self) -> ReservationId {
        let id = (self.structure_id.0 << 32) | self.next_reservation_seq;
        self.next_reservation_seq += 1;
        ReservationId::new(id)
    }

    /// Advance the facility simulation by a single tick.
    pub fn tick(
        &mut self,
        power_status: PowerStatus,
        current_tick: SimTick,
        deposits: &mut BTreeMap<DepositId, ResourceDeposit>,
        journal: &mut EventJournal,
        mods: ProductionModifiers,
    ) -> GameResult<()> {
        let is_powered = power_status.is_operational();

        match self.kind {
            FacilityKind::MiningDrill => {
                self.tick_mining(is_powered, current_tick, deposits, journal, mods)?;
            }
            FacilityKind::Refinery | FacilityKind::Fabricator => {
                self.tick_production(is_powered, current_tick, journal, mods)?;
            }
        }

        Ok(())
    }

    /// Step mining drill extraction logic.
    fn tick_mining(
        &mut self,
        is_powered: bool,
        current_tick: SimTick,
        deposits: &mut BTreeMap<DepositId, ResourceDeposit>,
        journal: &mut EventJournal,
        mods: ProductionModifiers,
    ) -> GameResult<()> {
        if !is_powered {
            if let MiningDrillState::Mining {
                deposit_id,
                progress_ticks,
                interval_ticks,
                yield_per_cycle,
            } = self.mining_state
            {
                self.mining_state = MiningDrillState::Unpowered {
                    deposit_id,
                    progress_ticks,
                    interval_ticks,
                    yield_per_cycle,
                };
                journal.record(
                    current_tick,
                    SimEvent::ProductionBlocked {
                        structure_id: self.structure_id,
                        reason: ProductionBlockedReason::Unpowered,
                    },
                );
            }
            return Ok(());
        }

        // Resume from unpowered if power restored
        if let MiningDrillState::Unpowered {
            deposit_id,
            progress_ticks,
            interval_ticks,
            yield_per_cycle,
        } = self.mining_state
        {
            self.mining_state = MiningDrillState::Mining {
                deposit_id,
                progress_ticks,
                interval_ticks,
                yield_per_cycle,
            };
        }

        // Attempt resolving blocked output
        if let MiningDrillState::OutputBlocked {
            deposit_id,
            extracted_pending,
        } = self.mining_state
        {
            if let Some(deposit) = deposits.get(&deposit_id)
                && self
                    .output_inventory
                    .can_accept(deposit.resource_id, extracted_pending)
            {
                self.output_inventory
                    .add(deposit.resource_id, extracted_pending)?;
                self.total_cycles_completed += 1;
                journal.record(
                    current_tick,
                    SimEvent::ResourceExtracted {
                        structure_id: self.structure_id,
                        deposit_id,
                        resource_id: deposit.resource_id,
                        amount: extracted_pending,
                    },
                );
                self.mining_state = MiningDrillState::Mining {
                    deposit_id,
                    progress_ticks: 0,
                    interval_ticks: 15,
                    yield_per_cycle: 2,
                };
            }
            return Ok(());
        }

        // Initialize mining if target deposit is set and idle
        if matches!(self.mining_state, MiningDrillState::Idle)
            && let Some(dep_id) = self.target_deposit
            && let Some(deposit) = deposits.get(&dep_id)
            && !deposit.is_depleted()
        {
            self.mining_state = MiningDrillState::Mining {
                deposit_id: dep_id,
                progress_ticks: 0,
                interval_ticks: 15,
                yield_per_cycle: 2,
            };
        }

        if let MiningDrillState::Mining {
            deposit_id,
            ref mut progress_ticks,
            interval_ticks,
            yield_per_cycle,
        } = self.mining_state
        {
            *progress_ticks += 1;
            let effective_interval = mods.effective_mining_interval(interval_ticks);
            if *progress_ticks >= effective_interval {
                let deposit = match deposits.get_mut(&deposit_id) {
                    Some(dep) if !dep.is_depleted() => dep,
                    _ => {
                        journal.record(current_tick, SimEvent::DepositDepleted { deposit_id });
                        self.mining_state = MiningDrillState::Idle;
                        return Ok(());
                    }
                };

                let purity_yield =
                    ((yield_per_cycle as f32 * deposit.purity).round() as u32).max(1);
                let effective_yield = mods.effective_yield(purity_yield);
                let can_store = self
                    .output_inventory
                    .can_accept(deposit.resource_id, effective_yield);

                if !can_store {
                    self.mining_state = MiningDrillState::OutputBlocked {
                        deposit_id,
                        extracted_pending: effective_yield,
                    };
                    journal.record(
                        current_tick,
                        SimEvent::ProductionBlocked {
                            structure_id: self.structure_id,
                            reason: ProductionBlockedReason::OutputFull,
                        },
                    );
                } else {
                    let extracted = deposit.extract(effective_yield);
                    self.output_inventory.add(deposit.resource_id, extracted)?;
                    self.total_cycles_completed += 1;
                    journal.record(
                        current_tick,
                        SimEvent::ResourceExtracted {
                            structure_id: self.structure_id,
                            deposit_id,
                            resource_id: deposit.resource_id,
                            amount: extracted,
                        },
                    );

                    if deposit.is_depleted() {
                        journal.record(current_tick, SimEvent::DepositDepleted { deposit_id });
                        self.mining_state = MiningDrillState::Idle;
                    } else {
                        self.mining_state = MiningDrillState::Mining {
                            deposit_id,
                            progress_ticks: 0,
                            interval_ticks,
                            yield_per_cycle,
                        };
                    }
                }
            }
        }

        Ok(())
    }

    /// Step refinery or fabricator production logic.
    fn tick_production(
        &mut self,
        is_powered: bool,
        current_tick: SimTick,
        journal: &mut EventJournal,
        mods: ProductionModifiers,
    ) -> GameResult<()> {
        if !is_powered {
            if let ProductionState::Crafting {
                recipe_id,
                reservation_ids,
                start_tick,
                progress_ticks,
                total_ticks,
            } = self.production_state.clone()
            {
                self.production_state = ProductionState::Unpowered {
                    recipe_id,
                    reservation_ids,
                    start_tick,
                    progress_ticks,
                    total_ticks,
                };
                journal.record(
                    current_tick,
                    SimEvent::ProductionBlocked {
                        structure_id: self.structure_id,
                        reason: ProductionBlockedReason::Unpowered,
                    },
                );
            }
            return Ok(());
        }

        // Resume from unpowered state upon power restoration
        if let ProductionState::Unpowered {
            recipe_id,
            reservation_ids,
            start_tick,
            progress_ticks,
            total_ticks,
        } = self.production_state.clone()
        {
            self.production_state = ProductionState::Crafting {
                recipe_id,
                reservation_ids,
                start_tick,
                progress_ticks,
                total_ticks,
            };
        }

        // Output blocked resolution
        if let ProductionState::OutputBlocked {
            recipe_id,
            reservation_ids,
        } = self.production_state.clone()
        {
            if let Some(recipe) = get_recipe(recipe_id) {
                let mut can_accept_all = true;
                for &(out_id, out_amt) in recipe.outputs {
                    if !self.output_inventory.can_accept(out_id, out_amt) {
                        can_accept_all = false;
                        break;
                    }
                }

                if can_accept_all {
                    // Commit reserved inputs
                    for res_id in reservation_ids {
                        self.input_inventory.commit_reservation(res_id)?;
                    }

                    // Deposit outputs
                    for &(out_id, out_amt) in recipe.outputs {
                        self.output_inventory.add(out_id, out_amt)?;
                    }

                    self.total_cycles_completed += 1;
                    self.scheduled_completion_tick = None;
                    journal.record(
                        current_tick,
                        SimEvent::ProductionJobCompleted {
                            structure_id: self.structure_id,
                            recipe_id,
                        },
                    );
                    self.production_state = ProductionState::Idle;
                }
            }
            return Ok(());
        }

        // Idle or awaiting inputs: attempt initiating new craft
        if matches!(
            self.production_state,
            ProductionState::Idle | ProductionState::AwaitingInputs
        ) && let Some(recipe_id) = self.active_recipe
            && let Some(recipe) = get_recipe(recipe_id)
        {
            // Check unreserved availability for all inputs
            let mut inputs_available = true;
            for &(in_id, in_amt) in recipe.inputs {
                if self.input_inventory.available_quantity(in_id) < in_amt {
                    inputs_available = false;
                    break;
                }
            }

            if !inputs_available {
                self.production_state = ProductionState::AwaitingInputs;
            } else {
                // Two-phase reservation lock for each input ingredient
                let mut reservations = Vec::new();
                let target_owner = Some(EntityId::new(self.structure_id.0));

                for &(in_id, in_amt) in recipe.inputs {
                    let res_id = self.next_reservation_id();
                    self.input_inventory.reserve(
                        res_id,
                        in_id,
                        in_amt,
                        current_tick,
                        target_owner,
                    )?;
                    reservations.push(res_id);
                }

                let craft_ticks = mods.effective_craft_ticks(recipe.duration_ticks);
                let finish_tick = current_tick + craft_ticks as u64;
                self.scheduled_completion_tick = Some(finish_tick);
                self.production_state = ProductionState::Crafting {
                    recipe_id,
                    reservation_ids: reservations,
                    start_tick: current_tick,
                    progress_ticks: 0,
                    total_ticks: craft_ticks,
                };

                journal.record(
                    current_tick,
                    SimEvent::ProductionJobStarted {
                        structure_id: self.structure_id,
                        recipe_id,
                        finish_tick,
                    },
                );
            }
        }

        // Progress ongoing craft
        if let ProductionState::Crafting {
            recipe_id,
            reservation_ids,
            start_tick,
            mut progress_ticks,
            total_ticks,
        } = self.production_state.clone()
        {
            progress_ticks += 1;
            if progress_ticks >= total_ticks {
                if let Some(recipe) = get_recipe(recipe_id) {
                    // Check output buffer capacity
                    let mut can_accept_all = true;
                    for &(out_id, out_amt) in recipe.outputs {
                        if !self.output_inventory.can_accept(out_id, out_amt) {
                            can_accept_all = false;
                            break;
                        }
                    }

                    if !can_accept_all {
                        self.production_state = ProductionState::OutputBlocked {
                            recipe_id,
                            reservation_ids,
                        };
                        journal.record(
                            current_tick,
                            SimEvent::ProductionBlocked {
                                structure_id: self.structure_id,
                                reason: ProductionBlockedReason::OutputFull,
                            },
                        );
                    } else {
                        // Commit all input reservations atomically
                        for res_id in reservation_ids {
                            self.input_inventory.commit_reservation(res_id)?;
                        }

                        // Add output goods
                        for &(out_id, out_amt) in recipe.outputs {
                            self.output_inventory.add(out_id, out_amt)?;
                        }

                        self.total_cycles_completed += 1;
                        self.scheduled_completion_tick = None;
                        journal.record(
                            current_tick,
                            SimEvent::ProductionJobCompleted {
                                structure_id: self.structure_id,
                                recipe_id,
                            },
                        );
                        self.production_state = ProductionState::Idle;
                    }
                }
            } else {
                self.production_state = ProductionState::Crafting {
                    recipe_id,
                    reservation_ids,
                    start_tick,
                    progress_ticks,
                    total_ticks,
                };
            }
        }

        Ok(())
    }

    /// Fast-forward production across multiple ticks for cold/distant event-driven scheduling.
    pub fn advance_ticks(
        &mut self,
        delta_ticks: u32,
        power_status: PowerStatus,
        current_tick: SimTick,
        deposits: &mut BTreeMap<DepositId, ResourceDeposit>,
        journal: &mut EventJournal,
        mods: ProductionModifiers,
    ) -> GameResult<()> {
        if delta_ticks == 0 {
            return Ok(());
        }

        // For small steps or active transitions, tick sequentially
        if delta_ticks <= 4 {
            for i in 0..delta_ticks {
                self.tick(
                    power_status,
                    current_tick + i as u64,
                    deposits,
                    journal,
                    mods,
                )?;
            }
            return Ok(());
        }

        // If unpowered, no progress occurs
        if !power_status.is_operational() {
            return self.tick(power_status, current_tick, deposits, journal, mods);
        }

        // Advance mining or crafting
        match self.kind {
            FacilityKind::MiningDrill => {
                if let MiningDrillState::Mining {
                    deposit_id,
                    progress_ticks,
                    interval_ticks,
                    yield_per_cycle,
                } = self.mining_state
                {
                    let effective_interval = mods.effective_mining_interval(interval_ticks);
                    let total_ticks = progress_ticks + delta_ticks;
                    let cycles = total_ticks / effective_interval;
                    let rem_ticks = total_ticks % effective_interval;

                    if cycles > 0
                        && let Some(deposit) = deposits.get_mut(&deposit_id)
                    {
                        let purity_yield =
                            ((yield_per_cycle as f32 * deposit.purity).round() as u32).max(1);
                        let effective_yield_per_cycle = mods.effective_yield(purity_yield);
                        let total_wanted = effective_yield_per_cycle * cycles;

                        let can_store = self
                            .output_inventory
                            .can_accept(deposit.resource_id, total_wanted);

                        if can_store {
                            let extracted = deposit.extract(total_wanted);
                            self.output_inventory.add(deposit.resource_id, extracted)?;
                            self.total_cycles_completed += cycles as u64;
                            journal.record(
                                current_tick + delta_ticks as u64,
                                SimEvent::ResourceExtracted {
                                    structure_id: self.structure_id,
                                    deposit_id,
                                    resource_id: deposit.resource_id,
                                    amount: extracted,
                                },
                            );

                            if deposit.is_depleted() {
                                journal.record(
                                    current_tick + delta_ticks as u64,
                                    SimEvent::DepositDepleted { deposit_id },
                                );
                                self.mining_state = MiningDrillState::Idle;
                            } else {
                                self.mining_state = MiningDrillState::Mining {
                                    deposit_id,
                                    progress_ticks: rem_ticks,
                                    interval_ticks,
                                    yield_per_cycle,
                                };
                            }
                            return Ok(());
                        }
                    }
                }
                // Fallback to iterative tick if buffer constraints apply
                for i in 0..delta_ticks {
                    self.tick(
                        power_status,
                        current_tick + i as u64,
                        deposits,
                        journal,
                        mods,
                    )?;
                }
            }
            FacilityKind::Refinery | FacilityKind::Fabricator => {
                // Step iteratively to properly manage state cycles and reservations
                for i in 0..delta_ticks {
                    self.tick(
                        power_status,
                        current_tick + i as u64,
                        deposits,
                        journal,
                        mods,
                    )?;
                }
            }
        }

        Ok(())
    }
}
