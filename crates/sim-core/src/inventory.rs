use crate::event::{EventJournal, SimEvent};
use game_types::{
    EntityId, FactionId, GameError, GameResult, ReservationId, ResourceCategory, ResourceId,
    ResourceRegistry, SimTick,
};
use std::collections::BTreeMap;

/// Architectural classification of containers in the game world.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ContainerKind {
    /// Personal mobile backpack for players and combat bots (compact, lower volume)
    Backpack,
    /// Static logistics hub and general storage facility
    Depot,
    /// Specialized high-volume storage for bulk unrefined ores and minerals
    Silo,
    /// Fast input/output buffer attached to production machinery
    Hopper,
    /// Universal mobile cargo buffer for haulers, logistics drones, and modular drop pods
    CargoBuffer,
}

impl ContainerKind {
    pub const fn default_max_slots(&self) -> usize {
        match self {
            ContainerKind::Backpack => 10,
            ContainerKind::Depot => 50,
            ContainerKind::Silo => 8,
            ContainerKind::Hopper => 4,
            ContainerKind::CargoBuffer => 20,
        }
    }

    pub const fn default_max_volume_liters(&self) -> u32 {
        match self {
            ContainerKind::Backpack => 500,       // 0.5 m^3
            ContainerKind::Depot => 100_000,      // 100.0 m^3
            ContainerKind::Silo => 500_000,       // 500.0 m^3
            ContainerKind::Hopper => 10_000,      // 10.0 m^3
            ContainerKind::CargoBuffer => 20_000, // 20.0 m^3
        }
    }
}

/// A discrete storage slot within an inventory.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InventorySlot {
    pub resource_id: ResourceId,
    pub quantity: u32,
    pub reserved: u32,
}

impl InventorySlot {
    pub const fn new(resource_id: ResourceId, quantity: u32) -> Self {
        InventorySlot {
            resource_id,
            quantity,
            reserved: 0,
        }
    }

    /// Quantity available for consumption or reservation (unlocked balance).
    pub const fn available(&self) -> u32 {
        self.quantity.saturating_sub(self.reserved)
    }

    pub const fn is_empty(&self) -> bool {
        self.quantity == 0 && self.reserved == 0
    }
}

/// Represents an in-flight reservation locked against an inventory.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActiveReservation {
    pub reservation_id: ReservationId,
    pub resource_id: ResourceId,
    pub amount: u32,
    pub created_tick: SimTick,
    pub source_entity: EntityId,
    pub target_entity: Option<EntityId>,
}

/// Server-authoritative container component with capacity limits and transactional locking.
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
    pub owner: EntityId,
    /// Faction that owns this container.
    ///
    /// `FactionId::null()` is unowned world storage, open to every faction.
    /// Every container the authoritative world creates through
    /// `WorldState::create_container` is stamped with its entity's faction, so
    /// the transactional operations below can refuse a foreign actor.
    pub owner_faction: FactionId,
    pub kind: ContainerKind,
    pub max_slots: usize,
    pub max_volume_liters: u32,
    pub slots: Vec<InventorySlot>,
    pub reservations: BTreeMap<ReservationId, ActiveReservation>,
}

impl Inventory {
    pub fn new(owner: EntityId, kind: ContainerKind) -> Self {
        Inventory {
            owner,
            owner_faction: FactionId::null(),
            kind,
            max_slots: kind.default_max_slots(),
            max_volume_liters: kind.default_max_volume_liters(),
            slots: Vec::new(),
            reservations: BTreeMap::new(),
        }
    }

    pub fn with_custom_capacity(
        owner: EntityId,
        kind: ContainerKind,
        max_slots: usize,
        max_volume_liters: u32,
    ) -> Self {
        Inventory {
            owner,
            owner_faction: FactionId::null(),
            kind,
            max_slots,
            max_volume_liters,
            slots: Vec::new(),
            reservations: BTreeMap::new(),
        }
    }

    /// Bind this container to an owning faction.
    pub fn with_faction(mut self, faction: FactionId) -> Self {
        self.owner_faction = faction;
        self
    }

    /// Whether `actor_faction` may operate on this container.
    ///
    /// A null actor faction is server/internal authority; a null owner faction
    /// is unowned world storage.
    pub fn is_accessible_by(&self, actor_faction: FactionId) -> bool {
        actor_faction.is_null()
            || self.owner_faction.is_null()
            || self.owner_faction == actor_faction
    }

    /// Calculate total volume occupied in liters.
    pub fn current_volume_liters(&self) -> f32 {
        let mut total_vol = 0.0f32;
        for slot in &self.slots {
            if let Some(def) = ResourceRegistry::get(slot.resource_id) {
                // def.unit_volume is in m^3; 1 m^3 = 1000 liters
                total_vol += (slot.quantity as f32) * (def.unit_volume * 1000.0);
            }
        }
        total_vol
    }

    /// Calculate total mass occupied in kg.
    pub fn current_mass_kg(&self) -> f32 {
        let mut total_mass = 0.0f32;
        for slot in &self.slots {
            if let Some(def) = ResourceRegistry::get(slot.resource_id) {
                total_mass += (slot.quantity as f32) * def.unit_mass;
            }
        }
        total_mass
    }

    /// Total quantity of a resource (including reserved).
    pub fn total_quantity(&self, resource_id: ResourceId) -> u32 {
        self.slots
            .iter()
            .filter(|s| s.resource_id == resource_id)
            .map(|s| s.quantity)
            .sum()
    }

    /// Available (unreserved) quantity of a resource.
    pub fn available_quantity(&self, resource_id: ResourceId) -> u32 {
        self.slots
            .iter()
            .filter(|s| s.resource_id == resource_id)
            .map(|s| s.available())
            .sum()
    }

    /// Total reserved quantity of a resource.
    pub fn reserved_quantity(&self, resource_id: ResourceId) -> u32 {
        self.slots
            .iter()
            .filter(|s| s.resource_id == resource_id)
            .map(|s| s.reserved)
            .sum()
    }

    /// Check if container can receive an amount of resource without exceeding slot or volume limits.
    pub fn can_accept(&self, resource_id: ResourceId, amount: u32) -> bool {
        if amount == 0 {
            return true;
        }

        // Check silo mineral restriction if applicable
        if self.kind == ContainerKind::Silo
            && ResourceRegistry::get(resource_id)
                .is_some_and(|def| def.category != ResourceCategory::RawMineral)
        {
            return false;
        }

        // Volume check
        let def = match ResourceRegistry::get(resource_id) {
            Some(d) => d,
            None => return false,
        };
        let added_vol = (amount as f32) * (def.unit_volume * 1000.0);
        if self.current_volume_liters() + added_vol > (self.max_volume_liters as f32) + 0.001 {
            return false;
        }

        // Slot check
        let stack_limit = def.stack_limit;
        let mut remaining_to_place = amount;

        // Try placing in existing slots first
        for slot in &self.slots {
            if slot.resource_id == resource_id && slot.quantity < stack_limit {
                let space = stack_limit - slot.quantity;
                if remaining_to_place <= space {
                    return true;
                }
                remaining_to_place -= space;
            }
        }

        // Need new slots for remainder
        let slots_needed = (remaining_to_place as usize).div_ceil(stack_limit as usize);
        self.slots.len() + slots_needed <= self.max_slots
    }

    /// Add an amount of a resource directly into inventory.
    pub fn add(&mut self, resource_id: ResourceId, amount: u32) -> GameResult<()> {
        if amount == 0 {
            return Ok(());
        }

        if !self.can_accept(resource_id, amount) {
            return Err(GameError::InventoryFull {
                max_slots: self.max_slots,
                max_volume_liters: self.max_volume_liters,
            });
        }

        let stack_limit = ResourceRegistry::get(resource_id)
            .map(|d| d.stack_limit)
            .unwrap_or(1000);

        let mut remaining = amount;

        // Fill existing slots
        for slot in &mut self.slots {
            if slot.resource_id == resource_id && slot.quantity < stack_limit {
                let space = stack_limit - slot.quantity;
                let to_add = remaining.min(space);
                slot.quantity = slot
                    .quantity
                    .checked_add(to_add)
                    .ok_or(GameError::ResourceOverflow)?;
                remaining -= to_add;
                if remaining == 0 {
                    break;
                }
            }
        }

        // Allocate new slots
        while remaining > 0 {
            if self.slots.len() >= self.max_slots {
                return Err(GameError::InventoryFull {
                    max_slots: self.max_slots,
                    max_volume_liters: self.max_volume_liters,
                });
            }
            let to_add = remaining.min(stack_limit);
            self.slots.push(InventorySlot::new(resource_id, to_add));
            remaining -= to_add;
        }

        Ok(())
    }

    /// Remove an unreserved amount of resource directly from inventory.
    pub fn remove(&mut self, resource_id: ResourceId, amount: u32) -> GameResult<()> {
        if amount == 0 {
            return Ok(());
        }

        let available = self.available_quantity(resource_id);
        if available < amount {
            return Err(GameError::InsufficientUnreservedBalance {
                available,
                requested: amount,
            });
        }

        let mut remaining = amount;
        for slot in self.slots.iter_mut().rev() {
            if slot.resource_id == resource_id {
                let unreserved = slot.available();
                if unreserved > 0 {
                    let to_take = remaining.min(unreserved);
                    slot.quantity = slot
                        .quantity
                        .checked_sub(to_take)
                        .ok_or(GameError::ResourceUnderflow)?;
                    remaining -= to_take;
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }

        // Clean up empty slots
        self.slots.retain(|s| !s.is_empty());
        Ok(())
    }

    /// Phase 1: Atomically reserve an amount of resource under a unique reservation ID.
    pub fn reserve(
        &mut self,
        reservation_id: ReservationId,
        resource_id: ResourceId,
        amount: u32,
        tick: SimTick,
        target_entity: Option<EntityId>,
    ) -> GameResult<()> {
        if reservation_id.is_null() {
            return Err(GameError::InvalidId);
        }
        if amount == 0 {
            return Ok(());
        }
        if self.reservations.contains_key(&reservation_id) {
            return Err(GameError::ReservationAlreadyExists(reservation_id));
        }

        let available = self.available_quantity(resource_id);
        if available < amount {
            return Err(GameError::InsufficientUnreservedBalance {
                available,
                requested: amount,
            });
        }

        // Lock amount across matching slots
        let mut remaining = amount;
        for slot in &mut self.slots {
            if slot.resource_id == resource_id {
                let unreserved = slot.available();
                if unreserved > 0 {
                    let to_reserve = remaining.min(unreserved);
                    slot.reserved = slot
                        .reserved
                        .checked_add(to_reserve)
                        .ok_or(GameError::ResourceOverflow)?;
                    remaining -= to_reserve;
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }

        self.reservations.insert(
            reservation_id,
            ActiveReservation {
                reservation_id,
                resource_id,
                amount,
                created_tick: tick,
                source_entity: self.owner,
                target_entity,
            },
        );

        Ok(())
    }

    /// Phase 2 (Commit): Deduct the reserved items permanently upon successful delivery.
    pub fn commit_reservation(
        &mut self,
        reservation_id: ReservationId,
    ) -> GameResult<(ResourceId, u32)> {
        let reservation = self
            .reservations
            .get(&reservation_id)
            .cloned()
            .ok_or(GameError::ReservationNotFound(reservation_id))?;

        // Pre-validate that slots have sufficient reserved balance and quantity
        let available_reserved: u32 = self
            .slots
            .iter()
            .filter(|s| s.resource_id == reservation.resource_id)
            .map(|s| s.reserved.min(s.quantity))
            .sum();
        if available_reserved < reservation.amount {
            return Err(GameError::ResourceUnderflow);
        }

        // Reservation validated: remove it and perform atomic deduction
        self.reservations.remove(&reservation_id);
        let mut remaining = reservation.amount;
        for slot in &mut self.slots {
            if slot.resource_id == reservation.resource_id && slot.reserved > 0 {
                let to_deduct = remaining.min(slot.reserved);
                slot.reserved -= to_deduct;
                slot.quantity = slot
                    .quantity
                    .checked_sub(to_deduct)
                    .ok_or(GameError::ResourceUnderflow)?;
                remaining -= to_deduct;
                if remaining == 0 {
                    break;
                }
            }
        }
        debug_assert_eq!(remaining, 0);

        self.slots.retain(|s| !s.is_empty());
        Ok((reservation.resource_id, reservation.amount))
    }

    /// Phase 2 (Abort/Release): Unlock reserved items back to available pool.
    pub fn release_reservation(
        &mut self,
        reservation_id: ReservationId,
    ) -> GameResult<(ResourceId, u32)> {
        let reservation = self
            .reservations
            .get(&reservation_id)
            .cloned()
            .ok_or(GameError::ReservationNotFound(reservation_id))?;

        let available_reserved: u32 = self
            .slots
            .iter()
            .filter(|s| s.resource_id == reservation.resource_id)
            .map(|s| s.reserved)
            .sum();
        if available_reserved < reservation.amount {
            return Err(GameError::ResourceUnderflow);
        }

        self.reservations.remove(&reservation_id);
        let mut remaining = reservation.amount;
        for slot in &mut self.slots {
            if slot.resource_id == reservation.resource_id && slot.reserved > 0 {
                let to_unlock = remaining.min(slot.reserved);
                slot.reserved -= to_unlock;
                remaining -= to_unlock;
                if remaining == 0 {
                    break;
                }
            }
        }
        debug_assert_eq!(remaining, 0);

        Ok((reservation.resource_id, reservation.amount))
    }

    /// Comprehensive authoritative invariant validation and corruption audit.
    pub fn validate_invariants(&self) -> GameResult<()> {
        // Invariant 1: Slots within limit
        if self.slots.len() > self.max_slots {
            return Err(GameError::CorruptedState(format!(
                "Slot count {} exceeds maximum {}",
                self.slots.len(),
                self.max_slots
            )));
        }

        // Invariant 2: Volume within limit
        let current_vol = self.current_volume_liters();
        if current_vol > (self.max_volume_liters as f32) + 0.001 {
            return Err(GameError::CorruptedState(format!(
                "Current volume {:.1}L exceeds max volume {}L",
                current_vol, self.max_volume_liters
            )));
        }

        // Invariant 3: Each slot reserved <= quantity
        for (idx, slot) in self.slots.iter().enumerate() {
            if slot.reserved > slot.quantity {
                return Err(GameError::CorruptedState(format!(
                    "Slot {idx} has reserved ({}) greater than quantity ({})",
                    slot.reserved, slot.quantity
                )));
            }
        }

        // Invariant 4: Sum of slot.reserved matches sum of reservations per resource
        let mut slot_reserved_by_res = BTreeMap::new();
        for slot in &self.slots {
            *slot_reserved_by_res.entry(slot.resource_id).or_insert(0u32) += slot.reserved;
        }

        let mut map_reserved_by_res = BTreeMap::new();
        for res in self.reservations.values() {
            *map_reserved_by_res.entry(res.resource_id).or_insert(0u32) += res.amount;
        }

        for (res_id, slot_amt) in slot_reserved_by_res {
            let map_amt = map_reserved_by_res.get(&res_id).copied().unwrap_or(0);
            if slot_amt != map_amt {
                return Err(GameError::CorruptedState(format!(
                    "Resource {res_id}: slot reserved total ({slot_amt}) != active reservations total ({map_amt})"
                )));
            }
        }

        for (res_id, map_amt) in map_reserved_by_res {
            let slot_amt = self.reserved_quantity(res_id);
            if slot_amt != map_amt {
                return Err(GameError::CorruptedState(format!(
                    "Resource {res_id}: active reservation total ({map_amt}) has no matching slot reservations ({slot_amt})"
                )));
            }
        }

        Ok(())
    }
}

/// Strongly-typed request parameters for reserving resources in an inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReserveRequest {
    pub from: EntityId,
    pub reservation_id: ReservationId,
    pub resource_id: ResourceId,
    pub amount: u32,
    pub tick: SimTick,
    pub target_entity: Option<EntityId>,
}

impl ReserveRequest {
    pub fn new(
        from: EntityId,
        reservation_id: ReservationId,
        resource_id: ResourceId,
        amount: u32,
        tick: SimTick,
    ) -> Self {
        ReserveRequest {
            from,
            reservation_id,
            resource_id,
            amount,
            tick,
            target_entity: None,
        }
    }

    pub fn with_target(mut self, target: EntityId) -> Self {
        self.target_entity = Some(target);
        self
    }
}

/// Parameters for one direct atomic transfer between containers.
///
/// A struct rather than a positional argument list so the actor faction and the
/// two endpoints cannot be transposed at a call site.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct TransferRequest {
    /// Faction requesting the move. `FactionId::null()` is server authority.
    pub actor_faction: FactionId,
    pub from: EntityId,
    pub to: EntityId,
    pub resource_id: ResourceId,
    pub amount: u32,
    pub tick: SimTick,
}

/// Global registry managing container components and atomic multi-inventory transactions.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryRegistry {
    inventories: BTreeMap<EntityId, Inventory>,
    next_reservation_id: u64,
}

impl Default for InventoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InventoryRegistry {
    pub fn new() -> Self {
        InventoryRegistry {
            inventories: BTreeMap::new(),
            next_reservation_id: 1,
        }
    }

    pub fn register(&mut self, inventory: Inventory) {
        self.inventories.insert(inventory.owner, inventory);
    }

    pub fn remove(&mut self, owner: EntityId) -> Option<Inventory> {
        self.inventories.remove(&owner)
    }

    pub fn get(&self, owner: EntityId) -> Option<&Inventory> {
        self.inventories.get(&owner)
    }

    pub fn get_mut(&mut self, owner: EntityId) -> Option<&mut Inventory> {
        self.inventories.get_mut(&owner)
    }

    pub fn contains(&self, owner: EntityId) -> bool {
        self.inventories.contains_key(&owner)
    }

    /// Refuse an actor faction that does not own the container behind `owner`.
    ///
    /// An entity with no container at all passes: the caller's own lookup
    /// reports the specific `ContainerNotFound` error instead.
    pub fn authorize(&self, actor_faction: FactionId, owner: EntityId) -> GameResult<()> {
        match self.inventories.get(&owner) {
            None => Ok(()),
            Some(inv) if inv.is_accessible_by(actor_faction) => Ok(()),
            Some(_) => Err(GameError::PermissionDenied),
        }
    }

    /// Generate next unique reservation ID.
    pub fn next_reservation_id(&mut self) -> ReservationId {
        let id = ReservationId::new(self.next_reservation_id);
        self.next_reservation_id += 1;
        id
    }

    /// Direct atomic transfer between two inventories with zero item duplication or loss.
    pub fn atomic_transfer(
        &mut self,
        req: TransferRequest,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        let TransferRequest {
            actor_faction,
            from,
            to,
            resource_id,
            amount,
            tick,
        } = req;
        if amount == 0 {
            return Ok(());
        }
        if from == to {
            return Ok(());
        }

        // Validate both inventories exist
        if !self.inventories.contains_key(&from) {
            return Err(GameError::ContainerNotFound(from));
        }
        if !self.inventories.contains_key(&to) {
            return Err(GameError::ContainerNotFound(to));
        }
        self.authorize(actor_faction, from)?;
        self.authorize(actor_faction, to)?;

        // Check source balance
        let source_avail = self
            .inventories
            .get(&from)
            .unwrap()
            .available_quantity(resource_id);
        if source_avail < amount {
            return Err(GameError::InsufficientUnreservedBalance {
                available: source_avail,
                requested: amount,
            });
        }

        // Check target acceptance
        if !self
            .inventories
            .get(&to)
            .unwrap()
            .can_accept(resource_id, amount)
        {
            let to_inv = self.inventories.get(&to).unwrap();
            return Err(GameError::InventoryFull {
                max_slots: to_inv.max_slots,
                max_volume_liters: to_inv.max_volume_liters,
            });
        }

        // Perform atomic transfer
        self.inventories
            .get_mut(&from)
            .unwrap()
            .remove(resource_id, amount)?;
        if let Err(e) = self
            .inventories
            .get_mut(&to)
            .unwrap()
            .add(resource_id, amount)
        {
            // Rollback source if target addition failed
            let _ = self
                .inventories
                .get_mut(&from)
                .unwrap()
                .add(resource_id, amount);
            return Err(e);
        }

        journal.record(
            tick,
            SimEvent::ResourceTransferred {
                from,
                to,
                resource_id,
                amount,
            },
        );

        Ok(())
    }

    /// Phase 1: Atomically reserve resource in source inventory.
    pub fn two_phase_reserve(
        &mut self,
        actor_faction: FactionId,
        req: ReserveRequest,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        self.authorize(actor_faction, req.from)?;
        if let Some(target) = req.target_entity {
            self.authorize(actor_faction, target)?;
        }
        let inv = self
            .inventories
            .get_mut(&req.from)
            .ok_or(GameError::ContainerNotFound(req.from))?;

        inv.reserve(
            req.reservation_id,
            req.resource_id,
            req.amount,
            req.tick,
            req.target_entity,
        )?;

        journal.record(
            req.tick,
            SimEvent::ResourceReserved {
                entity: req.from,
                resource_id: req.resource_id,
                amount: req.amount,
                reservation_id: req.reservation_id,
            },
        );

        Ok(())
    }

    /// Phase 2 (Commit): Deliver reserved resources from source to destination.
    pub fn two_phase_commit(
        &mut self,
        actor_faction: FactionId,
        reservation_id: ReservationId,
        from: EntityId,
        to: EntityId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        // Ensure both containers exist
        if !self.inventories.contains_key(&from) {
            return Err(GameError::ContainerNotFound(from));
        }
        if !self.inventories.contains_key(&to) {
            return Err(GameError::ContainerNotFound(to));
        }
        self.authorize(actor_faction, from)?;
        self.authorize(actor_faction, to)?;

        // Verify reservation exists in source
        let res = self
            .inventories
            .get(&from)
            .unwrap()
            .reservations
            .get(&reservation_id)
            .cloned()
            .ok_or(GameError::ReservationNotFound(reservation_id))?;

        // Verify target can accept before deducting from source
        if !self
            .inventories
            .get(&to)
            .unwrap()
            .can_accept(res.resource_id, res.amount)
        {
            let to_inv = self.inventories.get(&to).unwrap();
            return Err(GameError::InventoryFull {
                max_slots: to_inv.max_slots,
                max_volume_liters: to_inv.max_volume_liters,
            });
        }

        // Commit on source
        let (resource_id, amount) = self
            .inventories
            .get_mut(&from)
            .unwrap()
            .commit_reservation(reservation_id)?;

        // Credit destination
        if let Err(e) = self
            .inventories
            .get_mut(&to)
            .unwrap()
            .add(resource_id, amount)
        {
            // Rollback on destination error
            let _ = self
                .inventories
                .get_mut(&from)
                .unwrap()
                .add(resource_id, amount);
            return Err(e);
        }

        journal.record(
            tick,
            SimEvent::ResourceCommitted {
                from,
                to,
                resource_id,
                amount,
                reservation_id,
            },
        );

        Ok(())
    }

    /// Phase 2 (Abort): Unlock reserved resources back to available balance on source.
    pub fn two_phase_cancel(
        &mut self,
        actor_faction: FactionId,
        reservation_id: ReservationId,
        from: EntityId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        self.authorize(actor_faction, from)?;
        let inv = self
            .inventories
            .get_mut(&from)
            .ok_or(GameError::ContainerNotFound(from))?;

        let (resource_id, amount) = inv.release_reservation(reservation_id)?;

        journal.record(
            tick,
            SimEvent::ResourceReleased {
                entity: from,
                resource_id,
                amount,
                reservation_id,
            },
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_types::{RES_IRON_ORE, RES_STEEL, RES_TUNGSTEN_COMPOSITE};

    #[test]
    fn test_inventory_capacity_and_limits() {
        let entity = EntityId::new(1);
        let mut inv = Inventory::new(entity, ContainerKind::Backpack);
        assert_eq!(inv.max_slots, 10);
        assert_eq!(inv.max_volume_liters, 500);

        // Add 500 iron ore (unit volume 0.005 m^3 = 5 L each, total 2500 L > 500 L limit!)
        // So 500 iron ore should be rejected due to volume limit!
        assert_eq!(
            inv.add(RES_IRON_ORE, 500),
            Err(GameError::InventoryFull {
                max_slots: 10,
                max_volume_liters: 500,
            })
        );

        // 50 iron ore = 250 L <= 500 L, should fit in 1 slot (stack limit 1000)
        assert!(inv.add(RES_IRON_ORE, 50).is_ok());
        assert_eq!(inv.total_quantity(RES_IRON_ORE), 50);
        assert_eq!(inv.available_quantity(RES_IRON_ORE), 50);
        assert_eq!(inv.slots.len(), 1);
        assert_eq!(inv.current_volume_liters(), 250.0);

        // Adding 60 more iron ore would be 300 L + 250 L = 550 L > 500 L, rejected
        assert_eq!(
            inv.add(RES_IRON_ORE, 60),
            Err(GameError::InventoryFull {
                max_slots: 10,
                max_volume_liters: 500,
            })
        );

        // Adding 50 iron ore = 250 L + 250 L = 500 L, exactly fills volume!
        assert!(inv.add(RES_IRON_ORE, 50).is_ok());
        assert_eq!(inv.total_quantity(RES_IRON_ORE), 100);
        assert_eq!(inv.current_volume_liters(), 500.0);
    }

    #[test]
    fn test_two_phase_reservation_commit_and_release() {
        let mut journal = EventJournal::new();
        let mut registry = InventoryRegistry::new();

        let source = EntityId::new(1);
        let target = EntityId::new(2);

        let mut src_inv = Inventory::new(source, ContainerKind::Depot);
        let target_inv = Inventory::new(target, ContainerKind::Depot);

        src_inv.add(RES_STEEL, 100).unwrap();
        registry.register(src_inv);
        registry.register(target_inv);

        let res_id = registry.next_reservation_id();

        // Phase 1: Reserve 40 Steel
        let mut req1 = ReserveRequest::new(source, res_id, RES_STEEL, 40, SimTick::new(1));
        req1.target_entity = Some(target);
        registry
            .two_phase_reserve(FactionId::null(), req1, &mut journal)
            .unwrap();

        // Verify balance split: 100 total, 60 available, 40 reserved
        let s = registry.get(source).unwrap();
        assert_eq!(s.total_quantity(RES_STEEL), 100);
        assert_eq!(s.available_quantity(RES_STEEL), 60);
        assert_eq!(s.reserved_quantity(RES_STEEL), 40);

        // Phase 2 (Commit): Deliver 40 Steel to target
        registry
            .two_phase_commit(
                FactionId::null(),
                res_id,
                source,
                target,
                SimTick::new(2),
                &mut journal,
            )
            .unwrap();

        let s = registry.get(source).unwrap();
        assert_eq!(s.total_quantity(RES_STEEL), 60);
        assert_eq!(s.available_quantity(RES_STEEL), 60);
        assert_eq!(s.reserved_quantity(RES_STEEL), 0);

        let t = registry.get(target).unwrap();
        assert_eq!(t.total_quantity(RES_STEEL), 40);
        assert_eq!(t.available_quantity(RES_STEEL), 40);

        // Now test release on a second reservation
        let res_id_2 = registry.next_reservation_id();
        registry
            .two_phase_reserve(
                FactionId::null(),
                ReserveRequest::new(source, res_id_2, RES_STEEL, 25, SimTick::new(3)),
                &mut journal,
            )
            .unwrap();

        assert_eq!(
            registry.get(source).unwrap().available_quantity(RES_STEEL),
            35
        );
        assert_eq!(
            registry.get(source).unwrap().reserved_quantity(RES_STEEL),
            25
        );

        // Cancel / abort reservation
        registry
            .two_phase_cancel(
                FactionId::null(),
                res_id_2,
                source,
                SimTick::new(4),
                &mut journal,
            )
            .unwrap();

        // Entire 60 is restored to available!
        let s = registry.get(source).unwrap();
        assert_eq!(s.total_quantity(RES_STEEL), 60);
        assert_eq!(s.available_quantity(RES_STEEL), 60);
        assert_eq!(s.reserved_quantity(RES_STEEL), 0);
    }

    #[test]
    fn test_concurrent_reservations_cannot_duplicate() {
        let mut journal = EventJournal::new();
        let mut registry = InventoryRegistry::new();

        let source = EntityId::new(10);
        let mut inv = Inventory::new(source, ContainerKind::Depot);
        inv.add(RES_TUNGSTEN_COMPOSITE, 50).unwrap();
        registry.register(inv);

        let res1 = registry.next_reservation_id();
        let res2 = registry.next_reservation_id();
        let res3 = registry.next_reservation_id();

        // Worker 1 reserves 30 items -> succeeds
        assert!(
            registry
                .two_phase_reserve(
                    FactionId::null(),
                    ReserveRequest::new(source, res1, RES_TUNGSTEN_COMPOSITE, 30, SimTick::new(1)),
                    &mut journal
                )
                .is_ok()
        );

        // Worker 2 attempts to reserve 30 items -> fails (only 20 unreserved remain!)
        let err2 = registry.two_phase_reserve(
            FactionId::null(),
            ReserveRequest::new(source, res2, RES_TUNGSTEN_COMPOSITE, 30, SimTick::new(1)),
            &mut journal,
        );
        assert_eq!(
            err2,
            Err(GameError::InsufficientUnreservedBalance {
                available: 20,
                requested: 30,
            })
        );

        // Worker 3 attempts to reserve 20 items -> succeeds
        assert!(
            registry
                .two_phase_reserve(
                    FactionId::null(),
                    ReserveRequest::new(source, res3, RES_TUNGSTEN_COMPOSITE, 20, SimTick::new(1)),
                    &mut journal
                )
                .is_ok()
        );

        // Now available is 0, reserved is 50, total is 50. Zero duplication!
        let s = registry.get(source).unwrap();
        assert_eq!(s.total_quantity(RES_TUNGSTEN_COMPOSITE), 50);
        assert_eq!(s.available_quantity(RES_TUNGSTEN_COMPOSITE), 0);
        assert_eq!(s.reserved_quantity(RES_TUNGSTEN_COMPOSITE), 50);
    }

    #[test]
    fn test_authoritative_corruption_detection() {
        let entity = EntityId::new(1);
        let mut inv = Inventory::new(entity, ContainerKind::Depot);
        inv.add(RES_STEEL, 100).unwrap();
        assert!(inv.validate_invariants().is_ok());

        // Corrupt slot: reserved > quantity
        inv.slots[0].reserved = 120;
        assert!(matches!(
            inv.validate_invariants(),
            Err(GameError::CorruptedState(_))
        ));

        // Restore and corrupt reservations map mismatch
        inv.slots[0].reserved = 40;
        // Invariant check will fail because slot reserved is 40 but reservations map is empty!
        assert!(matches!(
            inv.validate_invariants(),
            Err(GameError::CorruptedState(_))
        ));
    }

    #[test]
    fn test_b2_commit_reservation_fails_atomically_on_insufficient_reserved_balance() {
        let entity = EntityId::new(1);
        let mut inv = Inventory::new(entity, ContainerKind::Depot);
        inv.add(RES_STEEL, 100).unwrap();

        let res_id = ReservationId::new(10);
        inv.reserve(res_id, RES_STEEL, 50, SimTick::new(1), None)
            .unwrap();

        // Simulate inconsistency where reservation claims 50, but slot only has 30 reserved.
        inv.slots[0].reserved = 30;

        let res = inv.commit_reservation(res_id);
        assert_eq!(res, Err(GameError::ResourceUnderflow));

        // State must remain strictly untouched:
        // 1. Reservation is NOT removed.
        assert!(inv.reservations.contains_key(&res_id));
        // 2. Slot balance was not deducted.
        assert_eq!(inv.slots[0].quantity, 100);
        assert_eq!(inv.slots[0].reserved, 30);
    }

    #[test]
    fn test_b2_release_reservation_fails_atomically_on_corrupt_reservation() {
        let entity = EntityId::new(1);
        let mut inv = Inventory::new(entity, ContainerKind::Depot);
        inv.add(RES_STEEL, 100).unwrap();

        let res_id = ReservationId::new(10);
        inv.reserve(res_id, RES_STEEL, 50, SimTick::new(1), None)
            .unwrap();

        inv.slots[0].reserved = 20;

        let res = inv.release_reservation(res_id);
        assert_eq!(res, Err(GameError::ResourceUnderflow));

        assert!(inv.reservations.contains_key(&res_id));
        assert_eq!(inv.slots[0].reserved, 20);
    }
}
