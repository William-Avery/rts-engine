use crate::packet::{Packet, PacketPayload};
use crate::server::{
    BindingOutcome, DEFAULT_SESSION_FACTION, apply_buffered_commands, bind_command_packet,
};
use crate::session::{Session, TokenIssuer};
use crate::snapshot::{EntitySnapshot, SnapshotEnvelope};
use crate::transport::{TransportRecv, TransportSend};
use crate::version::{HandshakeMessage, PROTOCOL_VERSION};
use anti_cheat::admin::{AdminRegistry, AdminRole, required_admin_permission};
use anti_cheat::event::{SecurityEvent, SecurityEventKind};
use anti_cheat::manifest::{BuildManifest, ServerPolicy};
use anti_cheat::provider::{AntiCheatMode, AntiCheatProvider, InspectionContext, Verdict};
use anti_cheat::trust::TrustLevel;
use game_types::{PlayerId, SessionId, SimTick};
use sim_core::world::{SessionDirective, WorldState};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send + 'static>;

/// Security state owned by the simulation thread.
///
/// Bundled into one struct so the packet-processing function keeps a sane
/// parameter count and so the anti-cheat provider never crosses a thread
/// boundary.
struct ServerSecurity {
    anti_cheat: Box<dyn AntiCheatProvider>,
    admin_registry: AdminRegistry,
    server_policy: ServerPolicy,
    build_manifest: BuildManifest,
    commands_blocked: u64,
    /// Commands the simulation itself refused during dispatch.
    commands_rejected: u64,
    /// Issuer of per-session capability tokens. Deliberately not the
    /// simulation RNG: tokens must be unpredictable and must not perturb
    /// deterministic replay.
    tokens: TokenIssuer,
}

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
    /// Which anti-cheat provider to install. Defaults to disabled.
    pub anti_cheat: AntiCheatMode,
    /// Build/protocol/content manifest policy this server enforces.
    pub server_policy: ServerPolicy,
    /// The server's own build manifest, advertised to and compared against clients.
    pub build_manifest: BuildManifest,
}

impl Default for ThreadedServerConfig {
    fn default() -> Self {
        ThreadedServerConfig {
            tick_rate_hz: 30,
            worker_threads: 4,
            timeout_ticks: 150,
            anti_cheat: AntiCheatMode::Disabled,
            server_policy: ServerPolicy::LocalDev,
            build_manifest: BuildManifest::new("rts-engine-dev", PROTOCOL_VERSION, 0),
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
        initial_sim_state: Option<WorldState>,
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
                // The anti-cheat provider is owned by the simulation thread, so
                // inspection runs on the same thread as authoritative state and
                // needs no locking.
                let mut security = ServerSecurity {
                    anti_cheat: config.anti_cheat.create_provider(
                        config.server_policy.clone(),
                        config.build_manifest.clone(),
                    ),
                    admin_registry: AdminRegistry::new(),
                    server_policy: config.server_policy.clone(),
                    build_manifest: config.build_manifest.clone(),
                    commands_blocked: 0,
                    commands_rejected: 0,
                    tokens: TokenIssuer::new(),
                };
                let _ = security.anti_cheat.initialize();

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
                            &mut security,
                        );
                    }

                    // Step B: Advance simulation tick
                    sim_state.tick = sim_state.tick.next();
                    sim_current_tick.store(sim_state.tick.value(), Ordering::Relaxed);

                    // Step C: Apply simulation commands through the one
                    // dispatcher `AuthoritativeServer` and the test harness use.
                    let stats = apply_buffered_commands(
                        &mut sim_state,
                        &sessions,
                        &security.admin_registry,
                    );
                    security.commands_rejected += stats.rejected;

                    // Step C2: Carry out the session-layer actions the
                    // simulation authorized.
                    for directive in sim_state.drain_session_directives() {
                        Self::execute_session_directive(directive, &mut sessions, &mut security);
                    }

                    // Step D: Step systems
                    sim_state.step_systems();

                    // Step E: Session timeout check
                    let current_tk = sim_state.tick;
                    for session in sessions.values_mut() {
                        if session.is_timed_out(current_tk, timeout_ticks) {
                            session.disconnect();
                            security.anti_cheat.end_session(session.player_id);
                            security.admin_registry.remove_session(session.session_id);
                        }
                    }

                    // Step E2: Service anti-cheat and apply pending enforcement.
                    security.anti_cheat.poll();
                    for (session_id, _reason) in security.anti_cheat.drain_pending_kicks() {
                        if let Some(session) = sessions.get_mut(&session_id) {
                            session.disconnect();
                        }
                        security.admin_registry.remove_session(session_id);
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

    /// Carry out one session-layer action the simulation authorized.
    fn execute_session_directive(
        directive: SessionDirective,
        sessions: &mut BTreeMap<SessionId, Session>,
        security: &mut ServerSecurity,
    ) {
        match directive {
            SessionDirective::KickSession { target, .. } => {
                if let Some(session) = sessions.get_mut(&target) {
                    session.disconnect();
                    security.anti_cheat.end_session(session.player_id);
                }
                security.anti_cheat.end_session_by_id(target);
                security.admin_registry.remove_session(target);
            }
            SessionDirective::SetTrustLevel { target, trust_code } => {
                if let Some(level) = TrustLevel::from_code(trust_code) {
                    security
                        .anti_cheat
                        .apply_admin_trust_override(target, level);
                }
            }
            SessionDirective::SetSessionRole { target, role_code } => {
                if let Some(role) = AdminRole::from_code(role_code) {
                    security.admin_registry.set_role(target, role);
                }
            }
        }
    }

    fn process_inbound_packet(
        packet: Packet,
        sim_state: &mut WorldState,
        sessions: &mut BTreeMap<SessionId, Session>,
        next_session_id: &mut u64,
        last_broadcast_seq: &mut u64,
        outbound_tx: &Sender<Packet>,
        security: &mut ServerSecurity,
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
                                session_token: 0,
                            },
                        );
                        let _ = outbound_tx.send(response);
                        return;
                    }

                    let session_id = SessionId::new(*next_session_id);
                    let player_id = PlayerId::new(*next_session_id as u32);
                    *next_session_id += 1;
                    let source: Option<SocketAddr> = None;

                    if security
                        .anti_cheat
                        .on_client_connecting(session_id, &client_name)
                        .is_err()
                        || security
                            .anti_cheat
                            .begin_session(player_id, session_id)
                            .is_err()
                    {
                        let response = Packet::new_handshake(
                            SessionId::null(),
                            *last_broadcast_seq,
                            HandshakeMessage::ServerHello {
                                accepted: false,
                                session_id: SessionId::null(),
                                server_tick: sim_state.tick,
                                reject_reason: Some("Refused by anti-cheat provider".to_string()),
                                session_token: 0,
                            },
                        );
                        let _ = outbound_tx.send(response);
                        return;
                    }
                    security.anti_cheat.on_client_authenticated(
                        session_id,
                        player_id,
                        DEFAULT_SESSION_FACTION,
                        sim_state.tick,
                    );

                    let token = security.tokens.next_token();
                    let mut session = Session::new(session_id, client_name, sim_state.tick)
                        .with_identity(player_id, DEFAULT_SESSION_FACTION)
                        .with_binding(token, source);
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
                            session_token: token,
                        },
                    );
                    let _ = outbound_tx.send(response);
                }
                HandshakeMessage::Disconnect { session_id, .. } => {
                    if let Some(session) = sessions.get_mut(&session_id) {
                        session.disconnect();
                        security.anti_cheat.end_session(session.player_id);
                    }
                    security.admin_registry.remove_session(session_id);
                }
                _ => {}
            },
            PacketPayload::Command(envelope) => {
                let session_id = envelope.session_id;
                let sequence = envelope.sequence;

                // Session binding runs first: a packet that cannot prove it
                // owns the session must not be able to touch its heartbeat or
                // latch its sequence counter.
                if let BindingOutcome::Rejected(kind) =
                    bind_command_packet(sessions, &envelope, None)
                {
                    security.commands_blocked += 1;
                    security.anti_cheat.report_event(SecurityEvent::new(
                        session_id,
                        PlayerId::null(),
                        sim_state.tick,
                        kind,
                    ));
                    return;
                }

                let Some(session) = sessions.get_mut(&session_id) else {
                    return;
                };
                session.update_heartbeat(sim_state.tick);
                if session.validate_and_advance_sequence(sequence).is_err() {
                    return;
                }
                let player_id = session.player_id;
                let faction_id = session.faction_id;
                let manifest_verified = session.manifest_verified;

                if let sim_core::command::Command::SubmitClientManifest {
                    build_id,
                    protocol_version,
                    content_hash,
                    official_build,
                } = &envelope.command
                {
                    let client_manifest = BuildManifest {
                        build_id: build_id.clone(),
                        protocol_version: *protocol_version,
                        content_hash: *content_hash,
                        official: *official_build,
                    };
                    let accepted = security
                        .anti_cheat
                        .verify_client_manifest(session_id, &client_manifest)
                        .is_ok()
                        && security
                            .server_policy
                            .validate(&security.build_manifest, &client_manifest)
                            .is_ok();
                    if accepted {
                        session.manifest_verified = true;
                    } else {
                        session.disconnect();
                        security.commands_blocked += 1;
                        security.anti_cheat.report_event(SecurityEvent::new(
                            session_id,
                            player_id,
                            sim_state.tick,
                            SecurityEventKind::ManifestMismatch {
                                expected: security.build_manifest.manifest_hash(),
                                actual: client_manifest.manifest_hash(),
                            },
                        ));
                    }
                    return;
                }

                if let Some(permission) = required_admin_permission(&envelope.command)
                    && security
                        .admin_registry
                        .authorize(session_id, permission)
                        .is_err()
                {
                    security.commands_blocked += 1;
                    security.anti_cheat.report_event(SecurityEvent::new(
                        session_id,
                        player_id,
                        sim_state.tick,
                        SecurityEventKind::AdminPermissionDenied { permission },
                    ));
                    return;
                }

                if security.server_policy.requires_manifest() && !manifest_verified {
                    security.commands_blocked += 1;
                    return;
                }

                let verdict = {
                    let ctx = InspectionContext::new(
                        session_id,
                        player_id,
                        faction_id,
                        sim_state.tick,
                        envelope.client_tick,
                        sequence,
                        sim_state,
                    );
                    security.anti_cheat.inspect_command(&ctx, &envelope.command)
                };

                if verdict.allows_command() {
                    sim_state.add_command(envelope);
                } else {
                    security.commands_blocked += 1;
                }

                if matches!(verdict, Verdict::Kick(_))
                    && let Some(session) = sessions.get_mut(&session_id)
                {
                    session.disconnect();
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

    fn broadcast_snapshots(
        sim_state: &WorldState,
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
