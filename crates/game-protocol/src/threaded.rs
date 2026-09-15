use crate::packet::{Packet, PacketPayload};
use crate::session::Session;
use crate::snapshot::{EntitySnapshot, SnapshotEnvelope};
use crate::transport::{TransportRecv, TransportSend};
use crate::version::{HandshakeMessage, PROTOCOL_VERSION};
use game_types::{SessionId, SimTick};
use sim_core::test_harness::TestSimState;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A lightweight, thread-safe worker pool for parallel computations (snapshot encoding, route planning, background simulation tasks).
pub struct WorkerPool {
    workers: Vec<Option<JoinHandle<()>>>,
    sender: Option<Sender<Job>>,
    num_workers: usize,
}

impl WorkerPool {
    /// Create a new worker pool with the specified number of threads.
    pub fn new(num_workers: usize) -> Self {
        let actual_workers = num_workers.max(1);
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(actual_workers);
        for id in 0..actual_workers {
            let rx = Arc::clone(&receiver);
            let handle = thread::Builder::new()
                .name(format!("server-worker-{id}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let lock = rx.lock().unwrap();
                            lock.recv()
                        };
                        match job {
                            Ok(job) => job(),
                            Err(_) => break, // Channel disconnected
                        }
                    }
                })
                .expect("Failed to spawn server worker thread");

            workers.push(Some(handle));
        }

        WorkerPool {
            workers,
            sender: Some(sender),
            num_workers: actual_workers,
        }
    }

    /// Execute a job asynchronously across the worker thread pool.
    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Box::new(f));
        }
    }

    /// Number of active worker threads.
    pub fn worker_count(&self) -> usize {
        self.num_workers
    }

    /// Shutdown the worker pool and join all worker threads cleanly.
    pub fn shutdown(&mut self) {
        self.sender = None; // Dropping sender notifies workers to exit
        for worker in &mut self.workers {
            if let Some(handle) = worker.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Configuration parameters for the threaded authoritative server.
#[derive(Debug, Clone)]
pub struct ThreadedServerConfig {
    /// Simulation tick rate in Hertz (e.g. 30 Hz = 33.3ms / tick).
    pub tick_rate_hz: u32,
    /// Number of background worker threads for parallel jobs.
    pub worker_threads: usize,
    /// Timeout in simulation ticks before inactive sessions are disconnected (e.g. 150 ticks = 5s at 30Hz).
    pub timeout_ticks: u64,
}

impl Default for ThreadedServerConfig {
    fn default() -> Self {
        ThreadedServerConfig {
            tick_rate_hz: 30,
            worker_threads: 4,
            timeout_ticks: 150,
        }
    }
}

/// Telemetry metrics for the threaded authoritative server.
#[derive(Debug, Clone, Default)]
pub struct ThreadedServerMetrics {
    pub total_ticks: u64,
    pub packets_received: u64,
    pub packets_sent: u64,
    pub active_sessions: usize,
    pub avg_tick_duration_micros: u64,
    pub worker_threads: usize,
}

/// Thread-safe controller handle for managing and observing a running threaded server.
pub struct ServerHandle {
    running: Arc<AtomicBool>,
    current_tick: Arc<AtomicU64>,
    metrics: Arc<RwLock<ThreadedServerMetrics>>,
    worker_pool: Arc<WorkerPool>,
    sim_handle: Option<JoinHandle<()>>,
    rx_handle: Option<JoinHandle<()>>,
    tx_handle: Option<JoinHandle<()>>,
}

impl ServerHandle {
    /// Whether the server threads are actively running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Current authoritative simulation tick.
    pub fn current_tick(&self) -> SimTick {
        SimTick::new(self.current_tick.load(Ordering::Relaxed))
    }

    /// Retrieve the latest telemetry snapshot.
    pub fn metrics(&self) -> ThreadedServerMetrics {
        self.metrics.read().unwrap().clone()
    }

    /// Number of worker pool threads.
    pub fn worker_count(&self) -> usize {
        self.worker_pool.worker_count()
    }

    /// Execute a background closure across the server's worker pool.
    pub fn execute_worker_job<F>(&self, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.worker_pool.execute(job);
    }

    /// Signal all server threads to stop cleanly.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Wait for all server threads (network ingress, network egress, simulation loop) to exit cleanly.
    pub fn join(mut self) {
        self.stop();
        if let Some(handle) = self.rx_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.tx_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.sim_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

/// Multithreaded Authoritative Dedicated Server.
///
/// Architecture:
/// - **Network Ingress Thread**: Continuously polls transport receiver, non-blocking, queuing decoded packets.
/// - **Simulation Tick Thread**: Runs deterministic 30 Hz simulation loop, draining incoming packets, advancing state, generating snapshots.
/// - **Network Egress Thread**: Asynchronously transmits outgoing packets / broadcast snapshots without stalling simulation ticks.
/// - **Worker Thread Pool**: Parallel tasks (snapshot packaging, background logistics routing, parallel calculations).
pub struct ThreadedAuthoritativeServer;

impl ThreadedAuthoritativeServer {
    /// Start the multithreaded server engine with decoupled network I/O, simulation tick loop, and worker pool.
    pub fn start<S, R>(
        sender: S,
        receiver: R,
        config: ThreadedServerConfig,
        initial_sim_state: Option<TestSimState>,
    ) -> ServerHandle
    where
        S: TransportSend + 'static,
        R: TransportRecv + 'static,
    {
        let running = Arc::new(AtomicBool::new(true));
        let current_tick = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(RwLock::new(ThreadedServerMetrics {
            worker_threads: config.worker_threads,
            ..Default::default()
        }));

        let worker_pool = Arc::new(WorkerPool::new(config.worker_threads));

        let (inbound_tx, inbound_rx) = mpsc::channel::<Packet>();
        let (outbound_tx, outbound_rx) = mpsc::channel::<Packet>();

        // 1. Spawn Network Ingress Thread
        let rx_running = Arc::clone(&running);
        let rx_metrics = Arc::clone(&metrics);
        let rx_handle = thread::Builder::new()
            .name("server-net-rx".to_string())
            .spawn(move || {
                let mut recv = receiver;
                while rx_running.load(Ordering::Relaxed) {
                    let mut received_any = false;
                    while let Ok(Some(packet)) = recv.recv() {
                        received_any = true;
                        {
                            if let Ok(mut m) = rx_metrics.write() {
                                m.packets_received += 1;
                            }
                        }
                        if inbound_tx.send(packet).is_err() {
                            return; // Channel disconnected
                        }
                    }

                    if !received_any {
                        // Short sleep to prevent 100% CPU spinning on non-blocking poll
                        thread::sleep(Duration::from_millis(1));
                    }
                }
                recv.close();
            })
            .expect("Failed to spawn network receiver thread");

        // 2. Spawn Network Egress Thread
        let tx_running = Arc::clone(&running);
        let tx_metrics = Arc::clone(&metrics);
        let tx_handle = thread::Builder::new()
            .name("server-net-tx".to_string())
            .spawn(move || {
                let mut send = sender;
                while tx_running.load(Ordering::Relaxed) {
                    match outbound_rx.recv_timeout(Duration::from_millis(10)) {
                        Ok(packet) => {
                            if send.send(packet).is_ok()
                                && let Ok(mut m) = tx_metrics.write()
                            {
                                m.packets_sent += 1;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                send.close();
            })
            .expect("Failed to spawn network transmitter thread");

        // 3. Spawn Authoritative Simulation Tick Thread
        let sim_running = Arc::clone(&running);
        let sim_current_tick = Arc::clone(&current_tick);
        let sim_metrics = Arc::clone(&metrics);
        let tick_interval = Duration::from_nanos(1_000_000_000 / config.tick_rate_hz.max(1) as u64);

        let sim_handle = thread::Builder::new()
            .name("server-sim-tick".to_string())
            .spawn(move || {
                let mut sim_state = initial_sim_state.unwrap_or_default();
                let mut sessions: BTreeMap<SessionId, Session> = BTreeMap::new();
                let mut next_session_id = 1u64;
                let mut last_broadcast_seq = 0u64;
                let timeout_ticks = config.timeout_ticks;

                while sim_running.load(Ordering::Relaxed) {
                    let tick_start = Instant::now();

                    // Step A: Drain inbound packet queue
                    while let Ok(packet) = inbound_rx.try_recv() {
                        Self::process_inbound_packet(
                            packet,
                            &mut sim_state,
                            &mut sessions,
                            &mut next_session_id,
                            &mut last_broadcast_seq,
                            &outbound_tx,
                        );
                    }

                    // Step B: Advance simulation tick
                    sim_state.tick = sim_state.tick.next();
                    sim_current_tick.store(sim_state.tick.value(), Ordering::Relaxed);

                    // Step C: Apply simulation commands
                    Self::apply_commands(&mut sim_state);

                    // Step D: Step systems
                    sim_state
                        .structure_registry
                        .tick_with_journal(sim_state.tick, &mut sim_state.event_journal);

                    sim_state.structure_registry.logistics.step(
                        sim_state.tick,
                        &mut sim_state.inventory_registry,
                        &mut sim_state.event_journal,
                    );

                    sim_state.scheduler.tick(
                        sim_state.tick,
                        &mut sim_state.region_map,
                        &mut sim_state.router,
                    );

                    // Step E: Session timeout check
                    let current_tk = sim_state.tick;
                    for session in sessions.values_mut() {
                        if session.is_timed_out(current_tk, timeout_ticks) {
                            session.disconnect();
                        }
                    }

                    // Step F: Broadcast snapshots
                    if !sessions.is_empty() {
                        Self::broadcast_snapshots(
                            &sim_state,
                            &sessions,
                            &mut last_broadcast_seq,
                            &outbound_tx,
                        );
                    }

                    // Update metrics
                    let tick_duration = tick_start.elapsed();
                    let active_sessions = sessions.values().filter(|s| s.state.is_active()).count();
                    if let Ok(mut m) = sim_metrics.write() {
                        m.total_ticks += 1;
                        m.active_sessions = active_sessions;
                        m.avg_tick_duration_micros = tick_duration.as_micros() as u64;
                    }

                    // Sleep for remainder of tick interval
                    if tick_duration < tick_interval {
                        thread::sleep(tick_interval - tick_duration);
                    }
                }
            })
            .expect("Failed to spawn simulation tick thread");

        ServerHandle {
            running,
            current_tick,
            metrics,
            worker_pool,
            sim_handle: Some(sim_handle),
            rx_handle: Some(rx_handle),
            tx_handle: Some(tx_handle),
        }
    }

    fn process_inbound_packet(
        packet: Packet,
        sim_state: &mut TestSimState,
        sessions: &mut BTreeMap<SessionId, Session>,
        next_session_id: &mut u64,
        last_broadcast_seq: &mut u64,
        outbound_tx: &Sender<Packet>,
    ) {
        match packet.payload {
            PacketPayload::Handshake(handshake) => match handshake {
                HandshakeMessage::ClientHello {
                    protocol_version,
                    client_name,
                } => {
                    *last_broadcast_seq += 1;
                    if protocol_version != PROTOCOL_VERSION {
                        let response = Packet::new_handshake(
                            SessionId::null(),
                            *last_broadcast_seq,
                            HandshakeMessage::ServerHello {
                                accepted: false,
                                session_id: SessionId::null(),
                                server_tick: sim_state.tick,
                                reject_reason: Some(format!(
                                    "Incompatible protocol version. Server: {PROTOCOL_VERSION}, Client: {protocol_version}"
                                )),
                            },
                        );
                        let _ = outbound_tx.send(response);
                        return;
                    }

                    let session_id = SessionId::new(*next_session_id);
                    *next_session_id += 1;

                    let mut session = Session::new(session_id, client_name, sim_state.tick);
                    session.activate();
                    sessions.insert(session_id, session);

                    let response = Packet::new_handshake(
                        session_id,
                        *last_broadcast_seq,
                        HandshakeMessage::ServerHello {
                            accepted: true,
                            session_id,
                            server_tick: sim_state.tick,
                            reject_reason: None,
                        },
                    );
                    let _ = outbound_tx.send(response);
                }
                HandshakeMessage::Disconnect { session_id, .. } => {
                    if let Some(session) = sessions.get_mut(&session_id) {
                        session.disconnect();
                    }
                }
                _ => {}
            },
            PacketPayload::Command(envelope) => {
                let session_id = envelope.session_id;
                let sequence = envelope.sequence;

                if let Some(session) = sessions.get_mut(&session_id) {
                    session.update_heartbeat(sim_state.tick);
                    if session.validate_and_advance_sequence(sequence).is_ok() {
                        sim_state.add_command(envelope);
                    }
                }
            }
            PacketPayload::Ping { timestamp } => {
                let session_id = packet.header.session_id;
                if let Some(session) = sessions.get_mut(&session_id) {
                    session.update_heartbeat(sim_state.tick);
                }
                *last_broadcast_seq += 1;
                let pong =
                    Packet::new_pong(session_id, *last_broadcast_seq, timestamp, sim_state.tick);
                let _ = outbound_tx.send(pong);
            }
            _ => {}
        }
    }

    fn apply_commands(sim_state: &mut TestSimState) {
        while let Some(envelope) = sim_state.command_buffer.pop() {
            match envelope.command {
                sim_core::command::Command::TransferRegion {
                    entity_id,
                    destination_region,
                } => {
                    let _ = sim_state.transfer_entity(entity_id, destination_region);
                }
                sim_core::command::Command::BuildStructure {
                    kind,
                    position,
                    rotation_deg,
                } => {
                    let _ = sim_state.structure_registry.request_build(
                        sim_core::structure::BuildRequest {
                            player_pos: position,
                            requested_pos: position,
                            kind,
                            rotation_deg,
                            faction_id: game_types::FactionId::new(1),
                            region_id: game_types::RegionId::new(1),
                            creation_tick: sim_state.tick,
                            world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                        },
                        None,
                    );
                }
                sim_core::command::Command::DismantleStructure { structure_id } => {
                    let _ = sim_state
                        .structure_registry
                        .request_dismantle(structure_id, game_types::FactionId::new(1));
                }
                sim_core::command::Command::RepairStructure {
                    structure_id,
                    actor_entity: Some(actor),
                } => {
                    let _ = sim_state.repair_structure(structure_id, actor);
                }
                sim_core::command::Command::TransferResource {
                    from_entity,
                    to_entity,
                    resource_id,
                    amount,
                } => {
                    let _ =
                        sim_state.transfer_resources(from_entity, to_entity, resource_id, amount);
                }
                sim_core::command::Command::ReserveResource {
                    entity,
                    resource_id,
                    amount,
                    reservation_id,
                } => {
                    let _ = sim_state.reserve_resources(
                        entity,
                        reservation_id,
                        resource_id,
                        amount,
                        None,
                    );
                }
                sim_core::command::Command::CommitTransfer {
                    reservation_id,
                    from_entity,
                    to_entity,
                } => {
                    let _ =
                        sim_state.commit_resource_transfer(reservation_id, from_entity, to_entity);
                }
                sim_core::command::Command::CancelReservation {
                    reservation_id,
                    from_entity,
                } => {
                    let _ = sim_state.cancel_resource_reservation(reservation_id, from_entity);
                }
                sim_core::command::Command::SetProductionRecipe {
                    structure_id,
                    recipe_id,
                } => {
                    let _ = sim_state
                        .structure_registry
                        .set_production_recipe(structure_id, recipe_id);
                }
                sim_core::command::Command::SetExtractionTarget {
                    structure_id,
                    deposit_id,
                } => {
                    let _ = sim_state
                        .structure_registry
                        .set_extraction_target(structure_id, deposit_id);
                }
                sim_core::command::Command::CreateLogisticsJob {
                    source,
                    destination,
                    resource_id,
                    amount,
                    priority,
                } => {
                    let _ = sim_state.structure_registry.logistics.create_job(
                        source,
                        destination,
                        resource_id,
                        amount,
                        sim_core::logistics::JobPriority::from_u8(priority),
                        sim_state.tick,
                        &mut sim_state.event_journal,
                    );
                }
                sim_core::command::Command::CancelLogisticsJob { job_id } => {
                    let _ = sim_state.structure_registry.logistics.cancel_job(
                        job_id,
                        "Server command cancelled",
                        sim_state.tick,
                        &mut sim_state.inventory_registry,
                        &mut sim_state.event_journal,
                    );
                }
                sim_core::command::Command::ClaimLogisticsJob { job_id, worker_id } => {
                    let _ = sim_state.structure_registry.logistics.claim_job(
                        job_id,
                        worker_id,
                        sim_state.tick,
                        &mut sim_state.inventory_registry,
                        &mut sim_state.event_journal,
                    );
                }
                sim_core::command::Command::ExecuteLogisticsPickup { job_id, worker_id } => {
                    let _ = sim_state.structure_registry.logistics.execute_pickup(
                        job_id,
                        worker_id,
                        sim_state.tick,
                        &mut sim_state.inventory_registry,
                        &mut sim_state.event_journal,
                    );
                }
                sim_core::command::Command::ExecuteLogisticsDropoff { job_id, worker_id } => {
                    let _ = sim_state.structure_registry.logistics.execute_dropoff(
                        job_id,
                        worker_id,
                        sim_state.tick,
                        &mut sim_state.inventory_registry,
                        &mut sim_state.event_journal,
                    );
                }
                _ => {}
            }
        }
    }

    fn broadcast_snapshots(
        sim_state: &TestSimState,
        sessions: &BTreeMap<SessionId, Session>,
        last_broadcast_seq: &mut u64,
        outbound_tx: &Sender<Packet>,
    ) {
        let mut entity_snapshots = Vec::new();
        for entity in sim_state.entity_registry.iter() {
            entity_snapshots.push(EntitySnapshot::new(
                entity.id,
                entity.faction_id,
                entity.region_id,
                entity.active,
                entity.flags.0,
            ));
        }

        let snapshot = SnapshotEnvelope::new(sim_state.tick, entity_snapshots);
        *last_broadcast_seq += 1;
        let seq = *last_broadcast_seq;

        for (&session_id, session) in sessions {
            if session.state.is_active() {
                let packet = Packet::new_snapshot(session_id, seq, snapshot.clone());
                let _ = outbound_tx.send(packet);
            }
        }
    }
}
