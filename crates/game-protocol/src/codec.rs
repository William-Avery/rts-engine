use crate::packet::{Packet, PacketHeader, PacketPayload};
use crate::snapshot::{DeltaEnvelope, EntitySnapshot, SnapshotEnvelope};
use crate::version::{HandshakeMessage, ProtocolError, ProtocolResult};
use game_types::{EntityId, FactionId, ItemId, RegionId, ResourceId, SessionId, SimTick};
use sim_core::command::{ActionType, Command, CommandEnvelope, RobotCommandType};

/// Encodes a protocol packet into a binary byte buffer.
pub fn encode_packet(packet: &Packet) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);

    // 1. Header (21 bytes)
    buf.extend_from_slice(&packet.header.protocol_version.to_le_bytes());
    buf.extend_from_slice(&packet.header.session_id.value().to_le_bytes());
    buf.extend_from_slice(&packet.header.sequence.to_le_bytes());
    buf.push(packet.header.packet_type);

    // 2. Payload
    match &packet.payload {
        PacketPayload::Handshake(msg) => {
            match msg {
                HandshakeMessage::ClientHello {
                    protocol_version,
                    client_name,
                } => {
                    buf.push(1); // sub-type 1
                    buf.extend_from_slice(&protocol_version.to_le_bytes());
                    encode_string(&mut buf, client_name);
                }
                HandshakeMessage::ServerHello {
                    accepted,
                    session_id,
                    server_tick,
                    reject_reason,
                } => {
                    buf.push(2); // sub-type 2
                    buf.push(if *accepted { 1 } else { 0 });
                    buf.extend_from_slice(&session_id.value().to_le_bytes());
                    buf.extend_from_slice(&server_tick.value().to_le_bytes());
                    if let Some(reason) = reject_reason {
                        buf.push(1);
                        encode_string(&mut buf, reason);
                    } else {
                        buf.push(0);
                    }
                }
                HandshakeMessage::Disconnect { session_id, reason } => {
                    buf.push(3); // sub-type 3
                    buf.extend_from_slice(&session_id.value().to_le_bytes());
                    encode_string(&mut buf, reason);
                }
            }
        }
        PacketPayload::Command(envelope) => {
            buf.extend_from_slice(&envelope.session_id.value().to_le_bytes());
            buf.extend_from_slice(&envelope.sequence.to_le_bytes());
            buf.extend_from_slice(&envelope.client_tick.value().to_le_bytes());
            encode_command(&mut buf, &envelope.command);
        }
        PacketPayload::Snapshot(snapshot) => {
            buf.extend_from_slice(&snapshot.server_tick.value().to_le_bytes());
            buf.extend_from_slice(&(snapshot.entities.len() as u32).to_le_bytes());
            for e in &snapshot.entities {
                encode_entity_snapshot(&mut buf, e);
            }
        }
        PacketPayload::Delta(delta) => {
            buf.extend_from_slice(&delta.base_tick.value().to_le_bytes());
            buf.extend_from_slice(&delta.target_tick.value().to_le_bytes());
            buf.extend_from_slice(&(delta.updated_entities.len() as u32).to_le_bytes());
            for e in &delta.updated_entities {
                encode_entity_snapshot(&mut buf, e);
            }
            buf.extend_from_slice(&(delta.removed_entities.len() as u32).to_le_bytes());
            for id in &delta.removed_entities {
                buf.extend_from_slice(&id.value().to_le_bytes());
            }
        }
        PacketPayload::Ping { timestamp } => {
            buf.extend_from_slice(&timestamp.to_le_bytes());
        }
        PacketPayload::Pong {
            timestamp,
            server_tick,
        } => {
            buf.extend_from_slice(&timestamp.to_le_bytes());
            buf.extend_from_slice(&server_tick.value().to_le_bytes());
        }
    }

    buf
}

/// Decodes a binary byte buffer into a protocol packet.
pub fn decode_packet(buf: &[u8]) -> ProtocolResult<Packet> {
    if buf.len() < PacketHeader::SIZE {
        return Err(ProtocolError::SerializationError(
            "Buffer smaller than packet header size".to_string(),
        ));
    }

    let mut cursor = 0;
    let version = read_u32(buf, &mut cursor)?;
    let session_id = SessionId::new(read_u64(buf, &mut cursor)?);
    let sequence = read_u64(buf, &mut cursor)?;
    let packet_type = read_u8(buf, &mut cursor)?;

    let header = PacketHeader::new(version, session_id, sequence, packet_type);

    let payload = match packet_type {
        1 => {
            // Handshake
            let sub_type = read_u8(buf, &mut cursor)?;
            match sub_type {
                1 => {
                    let proto = read_u32(buf, &mut cursor)?;
                    let name = decode_string(buf, &mut cursor)?;
                    PacketPayload::Handshake(HandshakeMessage::ClientHello {
                        protocol_version: proto,
                        client_name: name,
                    })
                }
                2 => {
                    let accepted = read_u8(buf, &mut cursor)? != 0;
                    let sid = SessionId::new(read_u64(buf, &mut cursor)?);
                    let tick = SimTick::new(read_u64(buf, &mut cursor)?);
                    let has_reason = read_u8(buf, &mut cursor)? != 0;
                    let reject_reason = if has_reason {
                        Some(decode_string(buf, &mut cursor)?)
                    } else {
                        None
                    };
                    PacketPayload::Handshake(HandshakeMessage::ServerHello {
                        accepted,
                        session_id: sid,
                        server_tick: tick,
                        reject_reason,
                    })
                }
                3 => {
                    let sid = SessionId::new(read_u64(buf, &mut cursor)?);
                    let reason = decode_string(buf, &mut cursor)?;
                    PacketPayload::Handshake(HandshakeMessage::Disconnect {
                        session_id: sid,
                        reason,
                    })
                }
                other => {
                    return Err(ProtocolError::SerializationError(format!(
                        "Unknown handshake sub-type: {other}"
                    )));
                }
            }
        }
        2 => {
            // Command
            let sid = SessionId::new(read_u64(buf, &mut cursor)?);
            let seq = read_u64(buf, &mut cursor)?;
            let client_tick = SimTick::new(read_u64(buf, &mut cursor)?);
            let cmd = decode_command(buf, &mut cursor)?;
            PacketPayload::Command(CommandEnvelope::new(sid, seq, client_tick, cmd))
        }
        3 => {
            // Snapshot
            let tick = SimTick::new(read_u64(buf, &mut cursor)?);
            let count = read_u32(buf, &mut cursor)? as usize;
            let mut entities = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                entities.push(decode_entity_snapshot(buf, &mut cursor)?);
            }
            PacketPayload::Snapshot(SnapshotEnvelope::new(tick, entities))
        }
        4 => {
            // Delta
            let base_tick = SimTick::new(read_u64(buf, &mut cursor)?);
            let target_tick = SimTick::new(read_u64(buf, &mut cursor)?);
            let updated_count = read_u32(buf, &mut cursor)? as usize;
            let mut updated = Vec::with_capacity(updated_count.min(1024));
            for _ in 0..updated_count {
                updated.push(decode_entity_snapshot(buf, &mut cursor)?);
            }
            let removed_count = read_u32(buf, &mut cursor)? as usize;
            let mut removed = Vec::with_capacity(removed_count.min(1024));
            for _ in 0..removed_count {
                removed.push(EntityId::new(read_u64(buf, &mut cursor)?));
            }
            PacketPayload::Delta(DeltaEnvelope::new(base_tick, target_tick, updated, removed))
        }
        5 => {
            // Ping
            let timestamp = read_u64(buf, &mut cursor)?;
            PacketPayload::Ping { timestamp }
        }
        6 => {
            // Pong
            let timestamp = read_u64(buf, &mut cursor)?;
            let server_tick = SimTick::new(read_u64(buf, &mut cursor)?);
            PacketPayload::Pong {
                timestamp,
                server_tick,
            }
        }
        other => {
            return Err(ProtocolError::SerializationError(format!(
                "Unknown packet type: {other}"
            )));
        }
    };

    Ok(Packet::new(header, payload))
}

fn encode_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(bytes);
}

fn decode_string(buf: &[u8], cursor: &mut usize) -> ProtocolResult<String> {
    let len = read_u16(buf, cursor)? as usize;
    if *cursor + len > buf.len() {
        return Err(ProtocolError::SerializationError(
            "String length exceeds buffer".to_string(),
        ));
    }
    let s = std::str::from_utf8(&buf[*cursor..*cursor + len])
        .map_err(|e| ProtocolError::SerializationError(e.to_string()))?
        .to_string();
    *cursor += len;
    Ok(s)
}

fn encode_entity_snapshot(buf: &mut Vec<u8>, e: &EntitySnapshot) {
    buf.extend_from_slice(&e.id.value().to_le_bytes());
    buf.extend_from_slice(&e.faction_id.value().to_le_bytes());
    buf.extend_from_slice(&e.region_id.value().to_le_bytes());
    buf.push(if e.active { 1 } else { 0 });
    buf.extend_from_slice(&e.flags.to_le_bytes());
}

fn decode_entity_snapshot(buf: &[u8], cursor: &mut usize) -> ProtocolResult<EntitySnapshot> {
    let id = EntityId::new(read_u64(buf, cursor)?);
    let faction_id = FactionId::new(read_u32(buf, cursor)?);
    let region_id = RegionId::new(read_u32(buf, cursor)?);
    let active = read_u8(buf, cursor)? != 0;
    let flags = read_u64(buf, cursor)?;
    Ok(EntitySnapshot::new(
        id, faction_id, region_id, active, flags,
    ))
}

fn encode_command(buf: &mut Vec<u8>, cmd: &Command) {
    match cmd {
        Command::Move { position, velocity } => {
            buf.push(1);
            buf.extend_from_slice(&position.0.to_le_bytes());
            buf.extend_from_slice(&position.1.to_le_bytes());
            buf.extend_from_slice(&position.2.to_le_bytes());
            buf.extend_from_slice(&velocity.0.to_le_bytes());
            buf.extend_from_slice(&velocity.1.to_le_bytes());
            buf.extend_from_slice(&velocity.2.to_le_bytes());
        }
        Command::Action {
            action_type,
            target,
        } => {
            buf.push(2);
            let at = match action_type {
                ActionType::FireWeapon => 1,
                ActionType::Reload => 2,
                ActionType::UseItem => 3,
                ActionType::Interact => 4,
                ActionType::Attack => 5,
                ActionType::MoveTo => 6,
                ActionType::HoldPosition => 7,
                ActionType::Return => 8,
            };
            buf.push(at);
            if let Some(t) = target {
                buf.push(1);
                buf.extend_from_slice(&t.value().to_le_bytes());
            } else {
                buf.push(0);
            }
        }
        Command::Build {
            position,
            structure_id,
        } => {
            buf.push(3);
            buf.extend_from_slice(&position.0.to_le_bytes());
            buf.extend_from_slice(&position.1.to_le_bytes());
            buf.extend_from_slice(&position.2.to_le_bytes());
            buf.extend_from_slice(&structure_id.value().to_le_bytes());
        }
        Command::RobotCommand {
            robot_id,
            command_type,
        } => {
            buf.push(4);
            buf.extend_from_slice(&robot_id.value().to_le_bytes());
            match command_type {
                RobotCommandType::Follow { target } => {
                    buf.push(1);
                    buf.extend_from_slice(&target.value().to_le_bytes());
                }
                RobotCommandType::Guard { position } => {
                    buf.push(2);
                    buf.extend_from_slice(&position.0.to_le_bytes());
                    buf.extend_from_slice(&position.1.to_le_bytes());
                    buf.extend_from_slice(&position.2.to_le_bytes());
                }
                RobotCommandType::Attack { target } => {
                    buf.push(3);
                    buf.extend_from_slice(&target.value().to_le_bytes());
                }
                RobotCommandType::Move { position } => {
                    buf.push(4);
                    buf.extend_from_slice(&position.0.to_le_bytes());
                    buf.extend_from_slice(&position.1.to_le_bytes());
                    buf.extend_from_slice(&position.2.to_le_bytes());
                }
                RobotCommandType::ReturnToBase => {
                    buf.push(5);
                }
            }
        }
        Command::RequestResource {
            resource_id,
            amount,
        } => {
            buf.push(5);
            buf.extend_from_slice(&resource_id.value().to_le_bytes());
            buf.extend_from_slice(&amount.to_le_bytes());
        }
        Command::Research { tech_id } => {
            buf.push(6);
            buf.extend_from_slice(&tech_id.value().to_le_bytes());
        }
        Command::TransferRegion {
            entity_id,
            destination_region,
        } => {
            buf.push(7);
            buf.extend_from_slice(&entity_id.value().to_le_bytes());
            buf.extend_from_slice(&destination_region.value().to_le_bytes());
        }
        Command::BuildStructure {
            kind,
            position,
            rotation_deg,
        } => {
            buf.push(8);
            let k_code = match kind {
                sim_core::structure::StructureKind::Wall(sim_core::wall::WallTier::Mk1Stone) => 1,
                sim_core::structure::StructureKind::Wall(sim_core::wall::WallTier::Mk2Steel) => 2,
                sim_core::structure::StructureKind::Wall(
                    sim_core::wall::WallTier::Mk3Composite,
                ) => 3,
                sim_core::structure::StructureKind::Pylon => 4,
                sim_core::structure::StructureKind::Depot => 5,
                sim_core::structure::StructureKind::Turret => 6,
                sim_core::structure::StructureKind::Fabricator => 7,
                sim_core::structure::StructureKind::Generator => 8,
                sim_core::structure::StructureKind::Battery => 9,
                sim_core::structure::StructureKind::MiningDrill => 10,
                sim_core::structure::StructureKind::Refinery => 11,
            };
            buf.push(k_code);
            buf.extend_from_slice(&position.0.to_le_bytes());
            buf.extend_from_slice(&position.1.to_le_bytes());
            buf.extend_from_slice(&position.2.to_le_bytes());
            buf.extend_from_slice(&rotation_deg.to_le_bytes());
        }
        Command::DismantleStructure { structure_id } => {
            buf.push(9);
            buf.extend_from_slice(&structure_id.value().to_le_bytes());
        }
        Command::RepairStructure {
            structure_id,
            actor_entity,
        } => {
            buf.push(10);
            buf.extend_from_slice(&structure_id.value().to_le_bytes());
            if let Some(actor) = actor_entity {
                buf.push(1);
                buf.extend_from_slice(&actor.value().to_le_bytes());
            } else {
                buf.push(0);
            }
        }
        Command::TransferResource {
            from_entity,
            to_entity,
            resource_id,
            amount,
        } => {
            buf.push(11);
            buf.extend_from_slice(&from_entity.value().to_le_bytes());
            buf.extend_from_slice(&to_entity.value().to_le_bytes());
            buf.extend_from_slice(&resource_id.value().to_le_bytes());
            buf.extend_from_slice(&amount.to_le_bytes());
        }
        Command::ReserveResource {
            entity,
            resource_id,
            amount,
            reservation_id,
        } => {
            buf.push(12);
            buf.extend_from_slice(&entity.value().to_le_bytes());
            buf.extend_from_slice(&resource_id.value().to_le_bytes());
            buf.extend_from_slice(&amount.to_le_bytes());
            buf.extend_from_slice(&reservation_id.value().to_le_bytes());
        }
        Command::CommitTransfer {
            reservation_id,
            from_entity,
            to_entity,
        } => {
            buf.push(13);
            buf.extend_from_slice(&reservation_id.value().to_le_bytes());
            buf.extend_from_slice(&from_entity.value().to_le_bytes());
            buf.extend_from_slice(&to_entity.value().to_le_bytes());
        }
        Command::CancelReservation {
            reservation_id,
            from_entity,
        } => {
            buf.push(14);
            buf.extend_from_slice(&reservation_id.value().to_le_bytes());
            buf.extend_from_slice(&from_entity.value().to_le_bytes());
        }
        Command::SetProductionRecipe {
            structure_id,
            recipe_id,
        } => {
            buf.push(15);
            buf.extend_from_slice(&structure_id.value().to_le_bytes());
            buf.extend_from_slice(&recipe_id.value().to_le_bytes());
        }
        Command::SetExtractionTarget {
            structure_id,
            deposit_id,
        } => {
            buf.push(16);
            buf.extend_from_slice(&structure_id.value().to_le_bytes());
            buf.extend_from_slice(&deposit_id.value().to_le_bytes());
        }
        Command::CreateLogisticsJob {
            source,
            destination,
            resource_id,
            amount,
            priority,
        } => {
            buf.push(17);
            buf.extend_from_slice(&source.value().to_le_bytes());
            buf.extend_from_slice(&destination.value().to_le_bytes());
            buf.extend_from_slice(&resource_id.value().to_le_bytes());
            buf.extend_from_slice(&amount.to_le_bytes());
            buf.push(*priority);
        }
        Command::CancelLogisticsJob { job_id } => {
            buf.push(18);
            buf.extend_from_slice(&job_id.value().to_le_bytes());
        }
        Command::ClaimLogisticsJob { job_id, worker_id } => {
            buf.push(19);
            buf.extend_from_slice(&job_id.value().to_le_bytes());
            buf.extend_from_slice(&worker_id.value().to_le_bytes());
        }
        Command::ExecuteLogisticsPickup { job_id, worker_id } => {
            buf.push(20);
            buf.extend_from_slice(&job_id.value().to_le_bytes());
            buf.extend_from_slice(&worker_id.value().to_le_bytes());
        }
        Command::ExecuteLogisticsDropoff { job_id, worker_id } => {
            buf.push(21);
            buf.extend_from_slice(&job_id.value().to_le_bytes());
            buf.extend_from_slice(&worker_id.value().to_le_bytes());
        }
    }
}

fn decode_command(buf: &[u8], cursor: &mut usize) -> ProtocolResult<Command> {
    let cmd_type = read_u8(buf, cursor)?;
    match cmd_type {
        1 => {
            let px = read_f32(buf, cursor)?;
            let py = read_f32(buf, cursor)?;
            let pz = read_f32(buf, cursor)?;
            let vx = read_f32(buf, cursor)?;
            let vy = read_f32(buf, cursor)?;
            let vz = read_f32(buf, cursor)?;
            Ok(Command::Move {
                position: (px, py, pz),
                velocity: (vx, vy, vz),
            })
        }
        2 => {
            let at_code = read_u8(buf, cursor)?;
            let action_type = match at_code {
                1 => ActionType::FireWeapon,
                2 => ActionType::Reload,
                3 => ActionType::UseItem,
                4 => ActionType::Interact,
                5 => ActionType::Attack,
                6 => ActionType::MoveTo,
                7 => ActionType::HoldPosition,
                8 => ActionType::Return,
                _ => {
                    return Err(ProtocolError::SerializationError(format!(
                        "Invalid action type: {at_code}"
                    )));
                }
            };
            let has_target = read_u8(buf, cursor)? != 0;
            let target = if has_target {
                Some(EntityId::new(read_u64(buf, cursor)?))
            } else {
                None
            };
            Ok(Command::Action {
                action_type,
                target,
            })
        }
        3 => {
            let px = read_f32(buf, cursor)?;
            let py = read_f32(buf, cursor)?;
            let pz = read_f32(buf, cursor)?;
            let structure_id = ItemId::new(read_u16(buf, cursor)?);
            Ok(Command::Build {
                position: (px, py, pz),
                structure_id,
            })
        }
        4 => {
            let robot_id = EntityId::new(read_u64(buf, cursor)?);
            let r_code = read_u8(buf, cursor)?;
            let command_type = match r_code {
                1 => RobotCommandType::Follow {
                    target: EntityId::new(read_u64(buf, cursor)?),
                },
                2 => {
                    let gx = read_f32(buf, cursor)?;
                    let gy = read_f32(buf, cursor)?;
                    let gz = read_f32(buf, cursor)?;
                    RobotCommandType::Guard {
                        position: (gx, gy, gz),
                    }
                }
                3 => RobotCommandType::Attack {
                    target: EntityId::new(read_u64(buf, cursor)?),
                },
                4 => {
                    let mx = read_f32(buf, cursor)?;
                    let my = read_f32(buf, cursor)?;
                    let mz = read_f32(buf, cursor)?;
                    RobotCommandType::Move {
                        position: (mx, my, mz),
                    }
                }
                5 => RobotCommandType::ReturnToBase,
                _ => {
                    return Err(ProtocolError::SerializationError(format!(
                        "Invalid robot command type: {r_code}"
                    )));
                }
            };
            Ok(Command::RobotCommand {
                robot_id,
                command_type,
            })
        }
        5 => {
            let resource_id = ResourceId::new(read_u16(buf, cursor)?);
            let amount = read_u32(buf, cursor)?;
            Ok(Command::RequestResource {
                resource_id,
                amount,
            })
        }
        6 => {
            let tech_id = ItemId::new(read_u16(buf, cursor)?);
            Ok(Command::Research { tech_id })
        }
        7 => {
            let entity_id = EntityId::new(read_u64(buf, cursor)?);
            let destination_region = RegionId::new(read_u32(buf, cursor)?);
            Ok(Command::TransferRegion {
                entity_id,
                destination_region,
            })
        }
        8 => {
            let k_code = read_u8(buf, cursor)?;
            let kind = match k_code {
                1 => sim_core::structure::StructureKind::Wall(sim_core::wall::WallTier::Mk1Stone),
                2 => sim_core::structure::StructureKind::Wall(sim_core::wall::WallTier::Mk2Steel),
                3 => {
                    sim_core::structure::StructureKind::Wall(sim_core::wall::WallTier::Mk3Composite)
                }
                4 => sim_core::structure::StructureKind::Pylon,
                5 => sim_core::structure::StructureKind::Depot,
                6 => sim_core::structure::StructureKind::Turret,
                7 => sim_core::structure::StructureKind::Fabricator,
                8 => sim_core::structure::StructureKind::Generator,
                9 => sim_core::structure::StructureKind::Battery,
                10 => sim_core::structure::StructureKind::MiningDrill,
                11 => sim_core::structure::StructureKind::Refinery,
                _ => {
                    return Err(ProtocolError::SerializationError(format!(
                        "Invalid structure kind: {k_code}"
                    )));
                }
            };
            let px = read_f32(buf, cursor)?;
            let py = read_f32(buf, cursor)?;
            let pz = read_f32(buf, cursor)?;
            let rotation_deg = read_f32(buf, cursor)?;
            Ok(Command::BuildStructure {
                kind,
                position: (px, py, pz),
                rotation_deg,
            })
        }
        9 => {
            let structure_id = game_types::StructureId::new(read_u64(buf, cursor)?);
            Ok(Command::DismantleStructure { structure_id })
        }
        10 => {
            let structure_id = game_types::StructureId::new(read_u64(buf, cursor)?);
            let has_actor = read_u8(buf, cursor)? != 0;
            let actor_entity = if has_actor {
                Some(EntityId::new(read_u64(buf, cursor)?))
            } else {
                None
            };
            Ok(Command::RepairStructure {
                structure_id,
                actor_entity,
            })
        }
        11 => {
            let from_entity = EntityId::new(read_u64(buf, cursor)?);
            let to_entity = EntityId::new(read_u64(buf, cursor)?);
            let resource_id = ResourceId::new(read_u16(buf, cursor)?);
            let amount = read_u32(buf, cursor)?;
            Ok(Command::TransferResource {
                from_entity,
                to_entity,
                resource_id,
                amount,
            })
        }
        12 => {
            let entity = EntityId::new(read_u64(buf, cursor)?);
            let resource_id = ResourceId::new(read_u16(buf, cursor)?);
            let amount = read_u32(buf, cursor)?;
            let reservation_id = game_types::ReservationId::new(read_u64(buf, cursor)?);
            Ok(Command::ReserveResource {
                entity,
                resource_id,
                amount,
                reservation_id,
            })
        }
        13 => {
            let reservation_id = game_types::ReservationId::new(read_u64(buf, cursor)?);
            let from_entity = EntityId::new(read_u64(buf, cursor)?);
            let to_entity = EntityId::new(read_u64(buf, cursor)?);
            Ok(Command::CommitTransfer {
                reservation_id,
                from_entity,
                to_entity,
            })
        }
        14 => {
            let reservation_id = game_types::ReservationId::new(read_u64(buf, cursor)?);
            let from_entity = EntityId::new(read_u64(buf, cursor)?);
            Ok(Command::CancelReservation {
                reservation_id,
                from_entity,
            })
        }
        15 => {
            let structure_id = game_types::StructureId::new(read_u64(buf, cursor)?);
            let recipe_id = game_types::RecipeId::new(read_u32(buf, cursor)?);
            Ok(Command::SetProductionRecipe {
                structure_id,
                recipe_id,
            })
        }
        16 => {
            let structure_id = game_types::StructureId::new(read_u64(buf, cursor)?);
            let deposit_id = game_types::DepositId::new(read_u64(buf, cursor)?);
            Ok(Command::SetExtractionTarget {
                structure_id,
                deposit_id,
            })
        }
        17 => {
            let source = EntityId::new(read_u64(buf, cursor)?);
            let destination = EntityId::new(read_u64(buf, cursor)?);
            let resource_id = ResourceId::new(read_u16(buf, cursor)?);
            let amount = read_u32(buf, cursor)?;
            let priority = read_u8(buf, cursor)?;
            Ok(Command::CreateLogisticsJob {
                source,
                destination,
                resource_id,
                amount,
                priority,
            })
        }
        18 => {
            let job_id = game_types::LogisticsJobId::new(read_u64(buf, cursor)?);
            Ok(Command::CancelLogisticsJob { job_id })
        }
        19 => {
            let job_id = game_types::LogisticsJobId::new(read_u64(buf, cursor)?);
            let worker_id = EntityId::new(read_u64(buf, cursor)?);
            Ok(Command::ClaimLogisticsJob { job_id, worker_id })
        }
        20 => {
            let job_id = game_types::LogisticsJobId::new(read_u64(buf, cursor)?);
            let worker_id = EntityId::new(read_u64(buf, cursor)?);
            Ok(Command::ExecuteLogisticsPickup { job_id, worker_id })
        }
        21 => {
            let job_id = game_types::LogisticsJobId::new(read_u64(buf, cursor)?);
            let worker_id = EntityId::new(read_u64(buf, cursor)?);
            Ok(Command::ExecuteLogisticsDropoff { job_id, worker_id })
        }
        other => Err(ProtocolError::SerializationError(format!(
            "Invalid command type code: {other}"
        ))),
    }
}

fn read_u8(buf: &[u8], cursor: &mut usize) -> ProtocolResult<u8> {
    if *cursor >= buf.len() {
        return Err(ProtocolError::SerializationError(
            "Unexpected EOF reading u8".to_string(),
        ));
    }
    let val = buf[*cursor];
    *cursor += 1;
    Ok(val)
}

fn read_u16(buf: &[u8], cursor: &mut usize) -> ProtocolResult<u16> {
    if *cursor + 2 > buf.len() {
        return Err(ProtocolError::SerializationError(
            "Unexpected EOF reading u16".to_string(),
        ));
    }
    let val = u16::from_le_bytes(buf[*cursor..*cursor + 2].try_into().unwrap());
    *cursor += 2;
    Ok(val)
}

fn read_u32(buf: &[u8], cursor: &mut usize) -> ProtocolResult<u32> {
    if *cursor + 4 > buf.len() {
        return Err(ProtocolError::SerializationError(
            "Unexpected EOF reading u32".to_string(),
        ));
    }
    let val = u32::from_le_bytes(buf[*cursor..*cursor + 4].try_into().unwrap());
    *cursor += 4;
    Ok(val)
}

fn read_u64(buf: &[u8], cursor: &mut usize) -> ProtocolResult<u64> {
    if *cursor + 8 > buf.len() {
        return Err(ProtocolError::SerializationError(
            "Unexpected EOF reading u64".to_string(),
        ));
    }
    let val = u64::from_le_bytes(buf[*cursor..*cursor + 8].try_into().unwrap());
    *cursor += 8;
    Ok(val)
}

fn read_f32(buf: &[u8], cursor: &mut usize) -> ProtocolResult<f32> {
    if *cursor + 4 > buf.len() {
        return Err(ProtocolError::SerializationError(
            "Unexpected EOF reading f32".to_string(),
        ));
    }
    let val = f32::from_le_bytes(buf[*cursor..*cursor + 4].try_into().unwrap());
    *cursor += 4;
    Ok(val)
}
