use game_types::{EntityId, FactionId, GameError, GameResult, RegionId, SimTick};

/// Component flag mask for entity classification.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ComponentMask(pub u64);

impl ComponentMask {
    pub const fn empty() -> Self {
        ComponentMask(0)
    }

    pub const fn all() -> Self {
        ComponentMask(u64::MAX)
    }

    pub const fn has(&self, flag: u64) -> bool {
        (self.0 & flag) != 0
    }

    pub const fn with(self, flag: u64) -> Self {
        ComponentMask(self.0 | flag)
    }

    pub const fn without(self, flag: u64) -> Self {
        ComponentMask(self.0 & !flag)
    }
}

/// Entity state flags.
pub struct EntityFlags;

impl EntityFlags {
    pub const ACTIVE: u64 = 1 << 0;
    pub const REMOVED: u64 = 1 << 1;
    pub const DORMANT: u64 = 1 << 2;
}

/// Simulation entity with metadata.
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: EntityId,
    pub faction_id: FactionId,
    pub region_id: RegionId,
    pub creation_tick: SimTick,
    pub active: bool,
    pub flags: ComponentMask,
}

impl Entity {
    pub fn new(id: EntityId, faction_id: FactionId, region_id: RegionId) -> Self {
        Entity {
            id,
            faction_id,
            region_id,
            creation_tick: SimTick::zero(),
            active: true,
            flags: ComponentMask::empty(),
        }
    }

    pub fn with_flags(mut self, flags: ComponentMask) -> Self {
        self.flags = flags;
        self
    }

    pub fn deactivate(&mut self) {
        self.active = false;
    }

    pub fn reactivate(&mut self) {
        self.active = true;
    }

    pub fn set_region(&mut self, new_region: RegionId) {
        self.region_id = new_region;
    }
}

/// Entity registry for the simulation.
#[derive(Debug, Clone)]
pub struct EntityRegistry {
    entities: Vec<Option<Entity>>,
    next_id: u64,
}

impl Default for EntityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityRegistry {
    pub fn new() -> Self {
        EntityRegistry {
            // Reserve index 0 for EntityId::null()
            entities: vec![None],
            next_id: 1,
        }
    }

    pub fn create(&mut self, faction_id: FactionId, region_id: RegionId) -> EntityId {
        let id = EntityId(self.next_id);
        self.next_id += 1;

        let entity = Entity::new(id, faction_id, region_id);
        self.entities.push(Some(entity));

        id
    }

    pub fn contains(&self, id: EntityId) -> bool {
        if id.is_null() {
            return false;
        }
        self.get(id).is_some()
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        if id.is_null() {
            return None;
        }
        self.entities.get(id.0 as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        if id.is_null() {
            return None;
        }
        self.entities.get_mut(id.0 as usize)?.as_mut()
    }

    pub fn update_region(&mut self, id: EntityId, new_region: RegionId) -> GameResult<RegionId> {
        let entity = self.get_mut(id).ok_or(GameError::EntityNotFound(id))?;
        let old_region = entity.region_id;
        entity.region_id = new_region;
        Ok(old_region)
    }

    pub fn remove(&mut self, id: EntityId) -> Option<Entity> {
        if id.is_null() {
            return None;
        }
        let idx = id.0 as usize;
        if idx < self.entities.len() {
            self.entities[idx].take()
        } else {
            None
        }
    }

    pub fn count(&self) -> usize {
        self.entities.iter().filter(|e| e.is_some()).count()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.entities.iter().filter_map(|e| e.as_ref())
    }
}
