use game_client::presentation::ClientPresentation;
use game_protocol::client::GameClientNet;
use game_protocol::transport::{LoopbackTransport, UdpTransport};
use std::env;

fn main() {
    println!("============================================================");
    println!("                   RTS Engine Client v0.1.0                 ");
    println!("============================================================");

    let args: Vec<String> = env::args().collect();
    let server_addr = args
        .windows(2)
        .find(|w| w[0] == "--connect" || w[0] == "-c")
        .map(|w| w[1].clone());

    let dry_run = args.iter().any(|a| a == "--dry-run");

    let mut presentation = ClientPresentation::new();
    println!("Client presentation foundation initialized:");
    println!(
        " - Camera Target: ({:.1}, {:.1}, {:.1})",
        presentation.camera.target.0, presentation.camera.target.1, presentation.camera.target.2
    );
    println!(" - Greybox Arena: 200m x 200m with static obstacles");
    println!(" - Local Prediction & Server Reconciliation: Enabled");

    if let Some(addr) = server_addr {
        println!("Configuring remote network transport to connect to {addr}...");
        let mut transport = match UdpTransport::bind("0.0.0.0:0") {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Failed to bind client transport socket: {e}");
                return;
            }
        };

        if let Err(e) = transport.connect_to(&addr) {
            eprintln!("Failed to configure destination address {addr}: {e}");
            return;
        }

        let mut client = GameClientNet::new(transport, "RemotePlayer".to_string());
        println!("Initiating connection handshake with {addr}...");
        if let Err(e) = client.connect() {
            eprintln!("Connection initiation error: {e}");
            return;
        }

        println!("Client connecting in remote mode.");
        if dry_run {
            run_dry_run(&mut presentation, &mut client);
            return;
        }
    } else {
        println!(
            "No remote server specified. Initializing local single-player loopback transport..."
        );
        let (client_transport, _server_transport) = LoopbackTransport::create_pair();
        let mut client = GameClientNet::new(client_transport, "LocalCommander".to_string());

        println!("Initiating local loopback handshake...");
        if let Err(e) = client.connect() {
            eprintln!("Loopback connection error: {e}");
            return;
        }

        println!("Client initialized in local loopback mode.");
        if dry_run {
            run_dry_run(&mut presentation, &mut client);
            return;
        }
    }

    println!("Client ready. Press Ctrl+C or close window to exit.");
}

fn run_dry_run<T: game_protocol::transport::Transport>(
    presentation: &mut ClientPresentation,
    client: &mut GameClientNet<T>,
) {
    println!("Dry run mode: executing 5 presentation & prediction frames...");

    // Simulate forward movement input and mouse camera orbit
    presentation.input.forward = 1.0;
    presentation.input.mouse_delta_x = 5.0;

    for frame_idx in 1..=5 {
        let current_time_ms = (frame_idx as u64) * 33;
        let frame = presentation.update(0.033, current_time_ms);
        println!(
            " Frame {:02}: Pos ({:.2}, {:.2}, {:.2}) | Camera Eye ({:.2}, {:.2}, {:.2}) | Locomotion: {:?}",
            frame_idx,
            frame.player_position.0,
            frame.player_position.1,
            frame.player_position.2,
            frame.camera_eye.0,
            frame.camera_eye.1,
            frame.camera_eye.2,
            frame.player_locomotion
        );
    }

    println!("\nTesting Interactive Construction Placement Ghost:");
    presentation.activate_placement(sim_core::structure::StructureKind::Turret);
    let preview_frame = presentation.update(0.033, 200);

    if let Some(ghost) = &preview_frame.placement_ghost {
        println!(
            " - Ghost Active: {:?} at Snapped Grid Pos ({:.1}, {:.1}, {:.1})",
            ghost.kind, ghost.snapped_pos.0, ghost.snapped_pos.1, ghost.snapped_pos.2
        );
        println!(" - Preview Status: {}", ghost.status.status_text());
        if let Some(build_cmd) = ghost.create_build_command() {
            println!(" - Created Authoritative Command: {:?}", build_cmd);
            if client.is_connected()
                && let Ok(seq) = client.send_command(build_cmd)
            {
                println!(
                    " - Dispatched build command to server with sequence #{}",
                    seq
                );
            }
        }
    }

    println!("\n{}", presentation.hud.render_ascii_card());
    println!("Dry run mode: disconnecting and exiting cleanly.");
    let _ = client.disconnect();
}
