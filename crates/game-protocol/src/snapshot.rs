use game_types::{EntityId, FactionId, RegionId, SimTick};

/// Compact replicated snapshot of an entity's authoritative state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitySnapshot {
    pub id: EntityId,
    pub faction_id: FactionId,
    pub region_id: RegionId,
    pub active: bool,
    pub flags: u64,
}

impl EntitySnapshot {
    pub fn new(
        id: EntityId,
        faction_id: FactionId,
        region_id: RegionId,
        active: bool,
        flags: u64,
    ) -> Self {
        EntitySnapshot {
            id,
            faction_id,
            region_id,
            active,
            flags,
        }
    }
}

/// Full world state snapshot broadcast by authoritative server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEnvelope {
    pub server_tick: SimTick,
    pub entities: Vec<EntitySnapshot>,
}

impl SnapshotEnvelope {
    pub fn new(server_tick: SimTick, entities: Vec<EntitySnapshot>) -> Self {
        SnapshotEnvelope {
            server_tick,
            entities,
        }
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }
}

/// Delta compression envelope containing changes between a base tick and target tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaEnvelope {
    pub base_tick: SimTick,
    pub target_tick: SimTick,
    pub updated_entities: Vec<EntitySnapshot>,
    pub removed_entities: Vec<EntityId>,
}

impl DeltaEnvelope {
    pub fn new(
        base_tick: SimTick,
        target_tick: SimTick,
        updated_entities: Vec<EntitySnapshot>,
        removed_entities: Vec<EntityId>,
    ) -> Self {
        DeltaEnvelope {
            base_tick,
            target_tick,
            updated_entities,
            removed_entities,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.updated_entities.is_empty() && self.removed_entities.is_empty()
    }
}
