# Network Protocol

This document defines the network wire protocol, session lifecycle, and replication envelopes for the RTS Engine.

## Protocol Versioning

The network protocol is strictly versioned via `PROTOCOL_VERSION` (currently `1`). 
During connection handshake, the server verifies protocol version compatibility. If mismatched, the server rejects the connection with a `VersionMismatch` error and a human-readable reason.

## Packet Envelope (Wire Format)

Every packet begins with a fixed 21-byte `PacketHeader` followed by a typed binary payload:

```
+-----------------------------------+
| Protocol Version  (4 bytes, LE)   |
+-----------------------------------+
| Session ID        (8 bytes, LE)   |
+-----------------------------------+
| Sequence Number   (8 bytes, LE)   |
+-----------------------------------+
| Packet Type       (1 byte)        |
+-----------------------------------+
| Payload Data      (variable)      |
+-----------------------------------+
```

Packet Type Codes:
- `1`: Handshake (`ClientHello`, `ServerHello`, `Disconnect`)
- `2`: Command (`CommandEnvelope`)
- `3`: Snapshot (`SnapshotEnvelope`)
- `4`: Delta (`DeltaEnvelope`)
- `5`: Ping (`timestamp`)
- `6`: Pong (`timestamp`, `server_tick`)

## Connection & Handshake Lifecycle

```
Client                                      Server
  |                                           |
  |--- ClientHello(version, client_name) ---->| (validate version)
  |                                           |
  |<-- ServerHello(accepted, session_id, tk) -| (assigns session)
  |                                           |
  |--- Command(seq, tick, Move/Build/...) --->| (validates seq > last_seq)
  |                                           |
  |<-- Snapshot(tick, [EntitySnapshot]) ------| (broadcast state)
  |                                           |
  |--- Disconnect(session_id, reason) ------->| (cleans up session)
```

## Command Sequence & Monotonicity Guarantees

- Each command emitted by a client contains a monotonically increasing `sequence` number.
- The server maintains `last_received_sequence` for each active session.
- If a packet arrives with `sequence <= last_received_sequence`, the server increments its `commands_rejected` counter and silently drops the duplicate or out-of-order command.
- Client cannot directly mutate server simulation state; all mutations are mediated by validated commands applied to the authoritative server's simulation loop.

## Replication Envelopes

### Full World Snapshot (`SnapshotEnvelope`)
- `server_tick`: Authoritative simulation tick when snapshot was generated.
- `entities`: Vector of `EntitySnapshot` records:
  - `entity_id`: 8 bytes
  - `faction_id`: 4 bytes
  - `region_id`: 4 bytes
  - `active`: 1 byte
  - `flags`: 8 bytes

### Delta Snapshot (`DeltaEnvelope`)
- `base_tick`: Prior tick acknowledged by client.
- `target_tick`: Current authoritative tick.
- `updated_entities`: Entities modified since `base_tick`.
- `removed_entities`: Entity IDs deleted since `base_tick`.

## Transport Abstraction

All gameplay and replication logic depends only on the `Transport` trait:
- `LoopbackTransport`: In-memory thread-safe channels (`Sender`/`Receiver`) for local single-player games with zero network overhead.
- `UdpTransport`: Non-blocking UDP sockets for remote client-server multiplayer games.
