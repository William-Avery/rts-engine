# AGENT_START.md
## Deterministic startup instructions for PI / Qwen

Read `RUST_GAME_AGENT_MASTER_SPEC_v2.md` in full before modifying code.

Then follow this startup state machine exactly.

### 1. List the repository root once

Use the native filesystem/list-directory tool if available.

If using PowerShell:
```powershell
Get-ChildItem -LiteralPath . -Force
```

Do not use `dir /b` directly in PowerShell.

### 2. Detect a fresh repository

If all of these are true:

- `RUST_GAME_AGENT_MASTER_SPEC_v2.md` (or the original master spec) exists;
- `Cargo.toml` does not exist;
- there is no existing Rust workspace/implementation;
- the directory mostly contains the specification/template files;

then set:

```text
PROJECT_STATE = FRESH_REPOSITORY
ACTIVE_MILESTONE = 0
```

A missing `Cargo.toml`, `src/`, or `docs/` is expected in this state. It is not a blocker.

### 3. Fresh-repository behavior

When `PROJECT_STATE = FRESH_REPOSITORY`:

- Do not repeatedly read paths already confirmed absent.
- Do not keep calling `smart_recall`.
- If one memory lookup says no matching project memory, stop using recall for bootstrap.
- Do not search for a root `src/`; this project uses a virtual Cargo workspace.
- Begin Milestone 0 immediately.

Create:

```text
Cargo.toml
rust-toolchain.toml

crates/
  game-types/
  sim-core/
  dedicated-server/
  game-client/

docs/
  PROJECT_ARCHITECTURE.md
  MILESTONE_STATUS.md
  DECISIONS.md
  BENCHMARKS.md
  PROTOCOL.md
  SAVE_FORMAT.md
  SECURITY.md
```

Each crate must have the required `Cargo.toml` and `src` entry file.

The root `Cargo.toml` is a virtual workspace, not a root package.

Use Rust edition 2024 unless the installed toolchain proves incompatible.

The dedicated server must not depend on rendering.

Copy the milestone template into `docs/MILESTONE_STATUS.md` if present, then mark Milestone 0 `IN PROGRESS`.

### 4. Validate Milestone 0

Before marking Milestone 0 complete, run:

```powershell
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Launch both placeholder binaries long enough to prove they start successfully.

Only then mark Milestone 0 `COMPLETE`.

### 5. Continue

After Milestone 0:
- determine Milestone 1's exact subtasks from the master specification;
- implement them;
- validate them;
- update `docs/MILESTONE_STATUS.md`;
- continue sequentially.

Never mark a milestone complete without passing its acceptance criteria.

If genuinely blocked by an unavailable external dependency, mark it `BLOCKED`, record exact evidence and the smallest next action, and stop rather than inventing a successful implementation.
