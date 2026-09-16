use game_types::{EntityId, GameError, GameResult, RegionId, ResourceId, SimTick};
use std::collections::{BTreeMap, VecDeque};

/// Payload types for cross-region messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrossRegionPayload {
    /// Request to transfer entity ownership to destination region
    TransferEntity { entity_id: EntityId },
    /// General numeric signal for inter-region coordination
    Signal { signal_id: u32, data: u64 },
    /// Event notification dispatched across regional boundaries
    EventNotification { event_type: u16, data: Vec<u8> },
    /// Inter-region resource request
    ResourceRequest {
        resource_id: ResourceId,
        amount: u32,
    },
    /// Generic binary payload
    Custom { message_type: u32, payload: Vec<u8> },
}

/// A message routed from one region to another with tick timestamp and unique ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossRegionMessage {
    pub id: u64,
    pub from_region: RegionId,
    pub to_region: RegionId,
    pub dispatch_tick: SimTick,
    pub payload: CrossRegionPayload,
}

impl CrossRegionMessage {
    pub fn new(
        id: u64,
        from_region: RegionId,
        to_region: RegionId,
        dispatch_tick: SimTick,
        payload: CrossRegionPayload,
    ) -> Self {
        CrossRegionMessage {
            id,
            from_region,
            to_region,
            dispatch_tick,
            payload,
        }
    }
}

/// Backpressure policy applied when a message queue reaches maximum capacity.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum BackpressurePolicy {
    /// Reject new messages with GameError::QueueFull
    #[default]
    Reject,
    /// Discard the oldest message to make room for the new message
    DropOldest,
}

/// Bounded FIFO queue for regional messages with backpressure management.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundedMessageQueue {
    queue: VecDeque<CrossRegionMessage>,
    max_capacity: usize,
    policy: BackpressurePolicy,
    total_enqueued: u64,
    total_dropped: u64,
    peak_depth: usize,
}

impl BoundedMessageQueue {
    pub fn new(max_capacity: usize, policy: BackpressurePolicy) -> Self {
        assert!(max_capacity > 0, "Queue capacity must be greater than zero");
        BoundedMessageQueue {
            queue: VecDeque::with_capacity(max_capacity.min(1024)),
            max_capacity,
            policy,
            total_enqueued: 0,
            total_dropped: 0,
            peak_depth: 0,
        }
    }

    pub fn with_default_capacity(max_capacity: usize) -> Self {
        Self::new(max_capacity, BackpressurePolicy::Reject)
    }

    pub fn enqueue(&mut self, message: CrossRegionMessage) -> GameResult<()> {
        if self.queue.len() >= self.max_capacity {
            match self.policy {
                BackpressurePolicy::Reject => {
                    self.total_dropped += 1;
                    return Err(GameError::QueueFull {
                        queue_name: format!("Region({})", message.to_region.value()),
                        capacity: self.max_capacity,
                    });
                }
                BackpressurePolicy::DropOldest => {
                    self.queue.pop_front();
                    self.total_dropped += 1;
                }
            }
        }

        self.queue.push_back(message);
        self.total_enqueued += 1;
        if self.queue.len() > self.peak_depth {
            self.peak_depth = self.queue.len();
        }
        Ok(())
    }

    pub fn dequeue(&mut self) -> Option<CrossRegionMessage> {
        self.queue.pop_front()
    }

    pub fn drain_all(&mut self) -> Vec<CrossRegionMessage> {
        self.queue.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn max_capacity(&self) -> usize {
        self.max_capacity
    }

    pub fn total_enqueued(&self) -> u64 {
        self.total_enqueued
    }

    pub fn total_dropped(&self) -> u64 {
        self.total_dropped
    }

    pub fn peak_depth(&self) -> usize {
        self.peak_depth
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

/// Central router for dispatching messages between world regions.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossRegionRouter {
    queues: BTreeMap<RegionId, BoundedMessageQueue>,
    default_capacity: usize,
    default_policy: BackpressurePolicy,
    next_message_id: u64,
    total_routed: u64,
}

impl Default for CrossRegionRouter {
    fn default() -> Self {
        Self::new(256, BackpressurePolicy::Reject)
    }
}

impl CrossRegionRouter {
    pub fn new(default_capacity: usize, default_policy: BackpressurePolicy) -> Self {
        CrossRegionRouter {
            queues: BTreeMap::new(),
            default_capacity,
            default_policy,
            next_message_id: 1,
            total_routed: 0,
        }
    }

    pub fn register_region(&mut self, region_id: RegionId) {
        self.queues.entry(region_id).or_insert_with(|| {
            BoundedMessageQueue::new(self.default_capacity, self.default_policy)
        });
    }

    pub fn set_region_queue_policy(
        &mut self,
        region_id: RegionId,
        capacity: usize,
        policy: BackpressurePolicy,
    ) {
        self.queues
            .insert(region_id, BoundedMessageQueue::new(capacity, policy));
    }

    pub fn send(
        &mut self,
        from_region: RegionId,
        to_region: RegionId,
        dispatch_tick: SimTick,
        payload: CrossRegionPayload,
    ) -> GameResult<u64> {
        if to_region.is_null() {
            return Err(GameError::InvalidId);
        }

        let message_id = self.next_message_id;
        self.next_message_id += 1;

        let message =
            CrossRegionMessage::new(message_id, from_region, to_region, dispatch_tick, payload);

        let default_capacity = self.default_capacity;
        let default_policy = self.default_policy;

        let queue = self
            .queues
            .entry(to_region)
            .or_insert_with(|| BoundedMessageQueue::new(default_capacity, default_policy));

        queue.enqueue(message)?;
        self.total_routed += 1;
        Ok(message_id)
    }

    /// Drains all pending messages for a destination region.
    pub fn drain_messages_for_region(&mut self, region_id: RegionId) -> Vec<CrossRegionMessage> {
        if let Some(queue) = self.queues.get_mut(&region_id) {
            queue.drain_all()
        } else {
            Vec::new()
        }
    }

    pub fn has_pending_messages(&self, region_id: RegionId) -> bool {
        self.queues
            .get(&region_id)
            .map(|q| !q.is_empty())
            .unwrap_or(false)
    }

    pub fn pending_count(&self, region_id: RegionId) -> usize {
        self.queues.get(&region_id).map(|q| q.len()).unwrap_or(0)
    }

    pub fn total_pending(&self) -> usize {
        self.queues.values().map(|q| q.len()).sum()
    }

    pub fn total_routed(&self) -> u64 {
        self.total_routed
    }

    pub fn total_dropped(&self) -> u64 {
        self.queues.values().map(|q| q.total_dropped()).sum()
    }

    pub fn clear(&mut self) {
        for q in self.queues.values_mut() {
            q.clear();
        }
    }
}
