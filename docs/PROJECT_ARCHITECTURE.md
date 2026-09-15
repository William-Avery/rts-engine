# Project Architecture

This document describes the high-level architecture of the RTS Engine.

## Overview

The RTS Engine is a Rust-based game engine for a tactical automation warfare game with 1-4 player co-op support. The architecture follows a server-authoritative model with client-side prediction and interpolation.

## Core Principles

1. **Server Authority**: The server owns all authoritative game state, including simulation, damage, resources, and construction.
2. **Simulation Independence**: Simulation code has no rendering dependencies and can run headless.
3. **Multi-Rate Simulation**: Different systems run at different frequencies based on their importance.
4. **Data-Oriented Design**: Use compact data structures, typed IDs, and SoA layouts where appropriate.
5. **No Client Trust**: Clients request actions; servers validate and apply them.

## Workspace Structure

```
crates/
  game-types/        # Shared types: IDs, time, errors
  sim-core/         # Core simulation: entities, commands, events
  sim-world/        # World representation (future)
  sim-combat/       # Combat systems (future)
  sim-industry/     # Industry systems (future)
  sim-logistics/    # Logistics systems (future)
  game-protocol/    # Network protocol (future)
  persistence/      # Save/load, replays (future)
dedicated-server/  # Headless server binary
game-client/       # Client presentation (future)
```

## Simulation Layers

### Game Types (`game-types`)
- Typed IDs: EntityId, PlayerId, FactionId, RegionId
- Time: SimTick, SimTime, SimDuration
- Errors: GameError, GameResult

### Core Simulation (`sim-core`)
- Entity Registry: Create, get, remove entities
- Commands: CommandEnvelope, CommandBuffer
- Events: SimEvent, EventJournal
- Scheduler: Multi-rate simulation buckets

## Network Model

```
Client <----> Network Transport <----> Authoritative Server
(Prediction)    (Protocol)           (Simulation)
```

## Rendering (Future)

Client presentation uses Bevy for rendering, input, and audio. The simulation is independent of rendering.

## Future Crates

- `sim-world`: World representation, terrain, regions
- `sim-combat`: Weapons, projectiles, damage
- `sim-industry`: Mining, refining, manufacturing
- `sim-logistics`: Transport, inventory, reservations
- `game-protocol`: Network protocol definitions
- `persistence`: Save/load, replays, snapshots

## Platform Support

- **Windows**: Primary target for client and server
- **Linux**: Server compatibility required

## Performance Targets

- 30 Hz base simulation tick
- 60-240 Hz render (independent)
- 100-300 high-fidelity agents in local battle
- 500-2000 match-wide autonomous units
