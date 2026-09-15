# RTS Engine

A Rust-based tactical automation warfare game engine with 1-4 player co-op support.

## Architecture

This project uses a virtual Cargo workspace:

```
crates/
  game-types/        # Shared types: IDs, time, errors
  sim-core/         # Core simulation: entities, commands, events
  dedicated-server/ # Headless server binary
  game-client/      # Client presentation (future)
```

## Building

```bash
cargo build --release
```

## Running

```bash
# Dedicated server
cargo run --release --bin dedicated-server

# Game client (future)
cargo run --release --bin game-client
```

## Validation

```bash
# Format
cargo fmt --all -- --check

# Check
cargo check --workspace

# Clippy
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Tests
cargo test --workspace
```

## Milestones

See `docs/MILESTONE_STATUS.md` for current progress.

## License

MIT
