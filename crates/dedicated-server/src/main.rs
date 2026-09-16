use anti_cheat::manifest::{BuildManifest, ContentManifest, ServerPolicy};
use anti_cheat::provider::AntiCheatMode;
use game_protocol::threaded::{ThreadedAuthoritativeServer, ThreadedServerConfig};
use game_protocol::transport::UdpTransport;
use game_protocol::version::PROTOCOL_VERSION;
use sim_core::world::WorldState;
use std::env;
use std::time::Duration;

/// Build identifier advertised in this server's manifest.
const BUILD_ID: &str = concat!("rts-engine-", env!("CARGO_PKG_VERSION"));

/// Build the server's own build/protocol/content manifest.
///
/// The content hash is rolled up from the content packs the server has loaded.
/// Milestone 29 (mod/data boundary and content pipeline) will register real
/// data-driven packs here; until then the built-in content set is registered
/// under a single well-known name so the hash is still meaningful and stable.
fn server_manifest(official: bool) -> BuildManifest {
    let mut content = ContentManifest::new();
    content.insert_bytes("builtin", BUILD_ID.as_bytes());
    let mut manifest = BuildManifest::from_content(BUILD_ID, PROTOCOL_VERSION, &content);
    manifest.official = official;
    manifest
}

/// Read the value following `--flag` / `-f` on the command line.
fn flag_value<'a>(args: &'a [String], long: &str, short: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == long || w[0] == short)
        .map(|w| w[1].as_str())
}

fn main() {
    println!("============================================================");
    println!("   RTS Engine Authoritative Multithreaded Server v0.1.0    ");
    println!("============================================================");

    let args: Vec<String> = env::args().collect();
    let port = args
        .windows(2)
        .find(|w| w[0] == "--port" || w[0] == "-p")
        .and_then(|w| w[1].parse::<u16>().ok())
        .unwrap_or(7777);

    let default_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let worker_threads = args
        .windows(2)
        .find(|w| w[0] == "--threads" || w[0] == "-t")
        .and_then(|w| w[1].parse::<usize>().ok())
        .unwrap_or(default_threads);

    let tick_rate_hz = args
        .windows(2)
        .find(|w| w[0] == "--tick-rate" || w[0] == "-r")
        .and_then(|w| w[1].parse::<u32>().ok())
        .unwrap_or(30);

    let dry_run = args.iter().any(|a| a == "--dry-run");

    // Anti-cheat is opt-in. Omitting the flag leaves it disabled, which is the
    // supported configuration for local development and single-player.
    let anti_cheat = flag_value(&args, "--anti-cheat", "-a")
        .map(|v| {
            AntiCheatMode::parse(v).unwrap_or_else(|| {
                eprintln!("Unknown --anti-cheat value '{v}'; expected 'off' or 'basic'.");
                AntiCheatMode::Disabled
            })
        })
        .unwrap_or_default();

    let server_policy = match flag_value(&args, "--server-policy", "-s").unwrap_or("local") {
        "official" => ServerPolicy::Official,
        "private" => ServerPolicy::PrivateCustom {
            accepted_manifest_hash: None,
        },
        "local" => ServerPolicy::LocalDev,
        other => {
            eprintln!(
                "Unknown --server-policy value '{other}'; expected 'local', 'private' or 'official'."
            );
            ServerPolicy::LocalDev
        }
    };

    let build_manifest = server_manifest(matches!(server_policy, ServerPolicy::Official));

    let bind_addr = format!("0.0.0.0:{port}");
    println!("Binding UDP network transport to {bind_addr}...");

    let transport = match UdpTransport::bind(&bind_addr) {
        Ok(t) => {
            println!("Network transport successfully bound to {bind_addr}");
            t
        }
        Err(e) => {
            eprintln!("Failed to bind to {bind_addr}: {e}");
            eprintln!("Falling back to dynamic port 0.0.0.0:0...");
            UdpTransport::bind("0.0.0.0:0").expect("Failed to bind UDP transport to dynamic port")
        }
    };

    let (tx, rx) = transport
        .split()
        .expect("Failed to split UDP transport for multithreaded I/O");

    // Bootstrap world regions
    let mut sim_state = WorldState::new();
    let grid_regions = sim_core::region::RegionMap::create_grid(
        -1000.0,
        -1000.0,
        500.0,
        500.0,
        4,
        4,
        sim_core::region::RegionState::Hot,
    )
    .expect("Failed to initialize world region grid");
    sim_state.region_map = grid_regions;

    let region_count = sim_state.region_map.region_count();

    let config = ThreadedServerConfig {
        tick_rate_hz,
        worker_threads,
        timeout_ticks: 150,
        anti_cheat,
        server_policy: server_policy.clone(),
        build_manifest: build_manifest.clone(),
    };

    println!("[Topology] Thread Architecture:");
    println!("  - Network Ingress Thread: Active (non-blocking UDP socket poller)");
    println!("  - Network Egress Thread:  Active (asynchronous snapshot broadcaster)");
    println!("  - Simulation Tick Loop:   Active ({tick_rate_hz} Hz deterministic cadence)");
    println!("  - Background Worker Pool: {worker_threads} worker threads");
    println!("  - World Regions:          {region_count} regions initialized");
    println!("[Security] Server Policy & Anti-Cheat:");
    println!(
        "  - Anti-Cheat Provider:    {} ({})",
        anti_cheat.as_str(),
        if anti_cheat == AntiCheatMode::Disabled {
            "server authority remains the primary defence"
        } else {
            "internal heuristics, no proprietary SDK required"
        }
    );
    println!("  - Server Policy:          {}", server_policy.as_str());
    println!("  - Build Manifest:         {build_manifest}");
    println!(
        "  - Manifest Hash:          {:#018x}",
        build_manifest.manifest_hash()
    );

    let handle = ThreadedAuthoritativeServer::start(tx, rx, config, Some(sim_state));
    println!("Multithreaded server online and ready for client connections.");

    if dry_run {
        println!("Dry run mode enabled: executing across threads and verifying shutdown...");
        std::thread::sleep(Duration::from_millis(200));

        let metrics = handle.metrics();
        println!(
            "Dry run verified: {} simulation ticks processed across threads, {} workers active.",
            handle.current_tick().value(),
            handle.worker_count()
        );
        println!(
            "Telemetry: {} inbound packets, {} outbound packets, avg tick: {} µs.",
            metrics.packets_received, metrics.packets_sent, metrics.avg_tick_duration_micros
        );

        handle.join();
        println!("Multithreaded server cleanly stopped and all threads joined.");
        return;
    }

    // Supervisor monitoring loop
    let monitor_interval = Duration::from_secs(5);
    loop {
        std::thread::sleep(monitor_interval);
        if !handle.is_running() {
            break;
        }
        let metrics = handle.metrics();
        println!(
            "[Tick {:>6}] Active Sessions: {:>2} | Inbound Packets: {:>5} | Outbound Packets: {:>5} | Avg Tick Latency: {:>4} µs",
            handle.current_tick().value(),
            metrics.active_sessions,
            metrics.packets_received,
            metrics.packets_sent,
            metrics.avg_tick_duration_micros
        );
    }

    handle.join();
}
