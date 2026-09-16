# Security

This document describes the security architecture of the RTS Engine: the threat
model, the layers of defence, the anti-cheat integration boundary, every detector
and its false-positive profile, and exactly how an EOS / Easy Anti-Cheat adapter
would be added.

Milestone 25 delivered the anti-cheat plumbing described here. Anything still
outstanding is listed explicitly in [Known gaps](#known-gaps).

---

## 1. Principles

1. **The authoritative server is the first anti-cheat layer.** Clients send
   intent; the server decides outcomes. A client can never assert a damage
   amount, a resource count or a position.
2. **Anti-cheat is defence in depth, never the only line of defence.** Removing
   the anti-cheat provider must not make the server exploitable. This is
   enforced by a test
   (`test_m25_acceptance_server_is_authoritative_with_anti_cheat_disabled`).
3. **Disabling anti-cheat must not change gameplay.** For legitimate input the
   simulation state is identical under the null and basic providers
   (`test_m25_acceptance_null_and_basic_providers_produce_identical_sim_state`).
4. **Do not instant-ban from weak heuristic evidence.** Low and medium severity
   findings record telemetry only. Conclusive evidence is required to remove a
   player.
5. **Gameplay crates know nothing about anti-cheat.** The dependency edge points
   one way only.
6. **Security telemetry is out-of-band.** It never enters the deterministic
   simulation journal, so it cannot perturb replays or desync a match.

---

## 2. Threat model

### In scope

| Threat | Mitigation |
|---|---|
| Modified client sends illegal commands | Server-side authoritative validation (primary), anti-cheat detectors (secondary) |
| Speed / teleport hacks | Server movement clamp + `impossible_movement` / `teleport_attempt` detectors |
| Rapid fire, infinite ammo | `fire_rate_violation` / `ammo_inconsistency` detectors (partly pending M13) |
| Resource duplication / economy exploits | Transactional inventory + `impossible_economy_delta` detector |
| Commanding units the player does not own | Faction ownership checks + `unauthorized_order` detector |
| Wallhack / acting on unseen entities | Replication interest (M14) + `hidden_target_attempt` detector hook |
| Replayed or malformed command envelopes | Monotonic per-session sequence validation + `future_dated_command` detector |
| Command flooding / input-rate abuse | `command_flood` detector |
| Unauthorized use of debug / admin cheats | `AdminRegistry` server-side authorization |
| Mismatched or modded builds on official servers | Build/protocol/content manifests + `ServerPolicy` |

### Explicitly out of scope for the internal layer

These require a platform anti-cheat product and are **not** defended against
today. Do not claim otherwise in marketing or release notes.

- Process memory tampering and code injection.
- DLL injection, hooking, and debugger attachment.
- Screen-reading and external overlay cheats.
- Kernel-level and hypervisor cheats.
- Input automation indistinguishable from a human at the command layer.

An internal, server-side layer can only compare *claims* against
*authoritative facts*. A cheat that only produces claims the server considers
legal is invisible to it by construction. That is exactly the gap an EAC-style
product fills, and why the adapter boundary in
[section 8](#8-eos--easy-anti-cheat-adapter-boundary) exists.

### Not yet addressed at all

Transport confidentiality and integrity (no encryption or MAC on the wire),
rate limiting at the socket layer, and DDoS mitigation. See
[Known gaps](#known-gaps).

---

## 3. Layers of defence

```text
  ┌────────────────────────────────────────────────────────────┐
  │ Layer 3  Platform anti-cheat (EOS/EAC)  — NOT PRESENT       │
  │          process + binary integrity, kernel-level detection │
  ├────────────────────────────────────────────────────────────┤
  │ Layer 2  Internal heuristics (BasicAntiCheat)               │
  │          claim-vs-authoritative-state comparison            │
  ├────────────────────────────────────────────────────────────┤
  │ Layer 1  Authoritative server  — ALWAYS ON                  │
  │          command validation, ownership, transactions        │
  └────────────────────────────────────────────────────────────┘
```

Layer 1 runs unconditionally. Layer 2 is opt-in (`--anti-cheat basic`). Layer 3
is a documented boundary with no implementation in this repository.

---

## 4. Dependency boundary

```text
  game-types  <--  sim-core  <--  anti-cheat  <--  game-protocol  <--  dedicated-server
```

The gameplay crates (`game-types`, `sim-core`) do **not** depend on `anti-cheat`
and therefore cannot depend, transitively, on any anti-cheat SDK. Verify with:

```
cargo tree -p sim-core --all-features
cargo tree -p game-types --all-features
```

Neither tree contains `anti-cheat`, even with `--all-features` (which enables
the `eos-eac` adapter feature).

The anti-cheat crate reads simulation state only through the
`anti_cheat::world_view::WorldView` trait, which is implemented **inside the
anti-cheat crate** for `sim_core::test_harness::TestSimState`. `sim-core` is
unaware the trait exists. The provider is handed an immutable `&dyn WorldView`
and can only answer with a `Verdict`; it can never mutate simulation state.

---

## 5. The `AntiCheatProvider` contract

`anti_cheat::provider::AntiCheatProvider` is the entire integration boundary.
Server code calls nothing else.

### Session lifecycle

| Phase | Method | Contract |
|---|---|---|
| Connecting | `on_client_connecting(session, client_name)` | May refuse the connection. Called before a `Session` exists. |
| Registering | `begin_session(player, session)` | Registers per-session state. Receives only the internal `PlayerId` — platform identity never crosses this boundary. |
| Manifest | `verify_client_manifest(session, manifest)` | Validates build/protocol/content identity against server policy. |
| Authenticated | `on_client_authenticated(session, player, faction, tick)` | Binds the faction and starts trust tracking. |
| Disconnected | `end_session(player)` / `end_session_by_id(session)` | Releases all per-session state. Must be idempotent. |

### Per-command inspection

```rust
fn inspect_command(&mut self, ctx: &InspectionContext<'_>, command: &Command) -> Verdict;
```

`InspectionContext` carries the session, player, faction, server tick, client
tick, sequence number, optional avatar entity, and `&dyn WorldView`.

Provider obligations:

- **Must not** mutate simulation state. It only has `&dyn WorldView`.
- **Must** be deterministic: identical input must produce an identical verdict
  and an identical telemetry sequence.
- **Must not** allocate on the clean path. A clean command returns an empty
  `Vec`, which does not allocate.
- **Must** tolerate unknown sessions by returning `Verdict::Allow` — the server
  will reject them anyway, and guessing about unknown state causes false
  positives.

### Verdicts

| Verdict | Command reaches the simulation? | Session kept? | Used for |
|---|---|---|---|
| `Allow` | yes | yes | nothing detected |
| `Observe(reason)` | yes | yes | weak evidence; telemetry only |
| `Reject(reason)` | no | yes | strong evidence |
| `Kick(reason)` | no | no | conclusive evidence |

`Allow` and `Observe` both hand the command on to the server's own
authoritative validation, which is what rejects it if it is actually illegal.

### Enforcement

The provider never disconnects anyone. It queues sessions via
`drain_pending_kicks()` and the **server** performs the disconnection, so
enforcement lives in exactly one place.

### Telemetry

`security_log() -> Option<&SecurityLog>`. `SecurityLog` is append-only and
bounded; eviction is counted in `dropped_count()` so a full log is visibly full
rather than silently lossy. Production deployments drain it with `iter_since()`.

---

## 6. Detectors

All detectors live in `anti_cheat::detectors` and are individually callable and
individually unit-tested. Thresholds are module constants.

| Detector | Catches | Severity | Wired into the server today | False-positive profile |
|---|---|---|---|---|
| `detect_impossible_movement` | speed hacks, teleports, coordinate corruption | Low / Medium (NaN and out-of-bounds: Critical) | yes (`Command::Move`) | **Low.** Speed clamp carries a 1.5× tolerance for prediction error, and a laggy client that batches displacement is rated `Low`. NaN/out-of-bounds has no benign cause. |
| `detect_fire_rate_violation` | rapid-fire cheats | Medium | yes (`Command::Action`, `RobotCommand::Attack`) | **Low today, by being conservative.** Uses a 4-tick floor no designed weapon may undercut. Becomes precise once M13 supplies real cycle times via `WorldView::weapon_cooldown_ticks`. |
| `detect_ammo_inconsistency` | infinite-ammo cheats | High | yes (`RobotCommand::Attack`) | **Very low.** Compares against the authoritative container balance. An actor with no container is never flagged. Partly pending M13 magazine state. |
| `detect_invalid_placement` | out-of-bounds or out-of-reach construction | Medium | yes (`BuildStructure`) | **Low.** Reach is only checked after the session has claimed a position at least once. Runs in addition to, not instead of, the structure registry's own validation. |
| `detect_impossible_economy_delta` | resource duplication, spending locked material | High | yes (transfer/reserve/job commands) | **Very low.** Compares against the *unreserved* authoritative balance. Race window is nil because inspection happens on the simulation thread. |
| `detect_unauthorized_entity_order` / `detect_unauthorized_structure_order` | commanding another faction's assets | High | yes (all entity/structure-scoped commands) | **Very low.** Unknown and unowned entities are deliberately not flagged. |
| `detect_hidden_target_attempt` | wallhacks — acting on entities the faction was never sent | High | hook wired, **inert** | **Zero today** (it reports nothing). Depends on Milestone 14; see below. |
| `detect_future_dated_command` | tick-stamp tampering, envelope replay | Low | yes (every command) | **Moderate but harmless.** Large clock skew produces this; rated `Low` for that reason. Duplicate/out-of-order sequences are already rejected by the session layer before inspection. |
| `detect_command_flood` | input-rate abuse, scripted spam | Medium | yes (every command) | **Low.** 240 commands per 30-tick window is far above human input rates. |

### Milestone hooks

Detectors that need systems built by later milestones call a `WorldView` method
with an inert default, and stay silent rather than guessing:

| Hook | Milestone | Default today | What M-n should do |
|---|---|---|---|
| `WorldView::faction_knows_entity` | **14** — sensors, faction knowledge, fog, replication interest | `KnowledgeQuery::Unavailable` → `detect_hidden_target_attempt` reports nothing | Override to consult the faction knowledge store and return `Known` / `Unknown`. The detector then works with no other change. |
| `WorldView::weapon_cooldown_ticks` | **13** — combat, weapons, damage, armor, projectiles | `None` → fire-rate falls back to a generous 4-tick floor | Override to return the equipped weapon's authoritative cycle time. |
| `WorldView::loaded_ammo` | **13** | `None` → ammo detector uses the `RES_AMMO` container balance | Override to return the loaded magazine count, catching a client firing on an empty magazine with a full reserve. |
| `InspectionContext::avatar_entity` | **12** — basic biped robot framework | `None` → actor-scoped detectors fall back to session-scoped tracking | Set to the entity the session directly controls, so ammo and fire-rate checks are per-avatar. |

---

## 7. Trust state machine

`anti_cheat::trust`. Transitions are driven by counted clean commands and an
accumulated suspicion score, never by wall-clock time, so they are fully
deterministic and reproducible from a command sequence alone.

```text
  Untrusted --8 clean--> Probationary --64 clean--> Trusted
       \                      ^   |                    |
        \        score decay  |   | suspicion >= 40    | any violation
         \                    +---+---> Flagged <------+
          \                             |
           +--- critical evidence ----> Banned (terminal)
```

- Suspicion weights: `Info` 0, `Low` 5, `Medium` 15, `High` 40, `Critical` 200.
- Flag threshold 40, ban threshold 200, decay 1 per clean command.
- **A single weak event never flags and never bans.** Eight low-severity events
  are needed to flag, and a flagged session is under observation, not punished —
  it keeps playing.
- A flagged session recovers to `Probationary` on a clean streak.
- `Banned` is terminal.
- `TrustPolicy::telemetry_only()` disables banning entirely, for community
  servers that only want the telemetry.

The externally visible `AntiCheatStatus` (`Unknown` / `Clean` / `UnderReview` /
`Banned`) deliberately exposes less than the internal trust level.

---

## 8. EOS / Easy Anti-Cheat adapter boundary

### Why there is no SDK code in this repository

Milestone 25 mandates creating an isolated FFI crate **only if the EAC/EOS SDK
is genuinely available and licensed in the build environment**. It is not: the
SDK is proprietary, is not vendored here, and cannot be redistributed. Writing
plausible `extern "C"` declarations against headers we do not have would produce
code that compiles, looks protective, and protects nothing — the worst possible
outcome for a security component.

Instead, `crates/anti-cheat/src/eos.rs` is a **compile-safe, feature-gated
adapter interface with no SDK calls**, behind the off-by-default `eos-eac`
cargo feature. It is included in `cargo clippy --all-features` so the boundary
cannot bit-rot.

### Behaviour without an SDK

```rust
EosEacAdapter::initialize()        // Err(AntiCheatError::SdkUnavailable { .. })
EosEacAdapter::on_client_connecting()  // Err(AntiCheatError::SdkUnavailable { .. })
EosEacAdapter::begin_session()     // Err(AntiCheatError::SdkUnavailable { .. })
EosEacAdapter::player_status()     // AntiCheatStatus::Unknown  — never "Clean"
EosEacAdapter::is_sdk_linked()     // false
```

The error text names the SDK, the `RTS_EOS_SDK_PATH` environment variable, the
steps to link a real SDK, and the `--anti-cheat basic` fallback that needs none.
An unlinked adapter never vouches for a player it could not check.

Gameplay-state inspection (`inspect_command`) still runs, because it is
SDK-independent: the adapter wraps a `BasicAntiCheat`, which is exactly how a
real integration is meant to work — EAC covers process and binary integrity
while the internal heuristics keep covering gameplay-state anomalies. The two
layers are complementary, not alternatives.

### Adding a real SDK

1. Obtain an EOS/EAC licence and product credentials from Epic.
2. Create a **separate** crate `crates/eos-sys` containing only the raw
   `extern "C"` bindings and the build script that links the SDK. Nothing else
   in the workspace may depend on it. Keeping it isolated means the `unsafe`
   surface is auditable in one place and the licence obligations attach to one
   crate.
3. Add `eos-sys` as an *optional* dependency of `crates/anti-cheat`, gated on
   the `eos-eac` feature:
   ```toml
   [features]
   eos-eac = ["dep:eos-sys"]

   [dependencies]
   eos-sys = { path = "../eos-sys", optional = true }
   ```
   The dependency edge is `anti-cheat -> eos-sys`. It must never become
   `sim-core -> eos-sys` or `game-types -> eos-sys`.
4. Replace the bodies of `initialize`, the peer register/unregister calls in
   `on_client_connecting` / `end_session`, and the callback drain in `poll`.
   Set `sdk_linked` from the real handshake result.
5. Map EAC client-violation callbacks onto `SecurityEvent`s and submit them
   through `report_event`, so they feed the same trust state machine and the
   same security log as the internal detectors.
6. Add `EosEac` to `AntiCheatMode` and its `--anti-cheat` parse arm.

Steps 4–6 touch only `crates/anti-cheat`. The `AntiCheatProvider` trait does not
change, so **no gameplay or protocol code is modified**. That is the property
the boundary exists to guarantee.

### Platform identity

Platform identity (Epic / Steam account) is kept strictly separate from the
internal `PlayerId`. `begin_session` receives only `PlayerId`; an SDK-backed
provider maintains its own platform-id mapping internally and never leaks it
into the simulation.

---

## 9. Build / protocol / content manifests

`anti_cheat::manifest`. A `BuildManifest` carries a build id, a protocol version
(mirroring `game_protocol::version::PROTOCOL_VERSION`), a rolled-up content hash
and an `official` flag. `ContentManifest` accumulates per-pack hashes in a
`BTreeMap`, so `content_hash()` is insertion-order independent and identical on
every machine.

Hashing is FNV-1a 64. This is an **accident and casual-tamper boundary, not an
integrity guarantee**: a client that controls its own process can report any
manifest it likes. Real binary integrity is EAC's job.

Clients advertise their manifest with `Command::SubmitClientManifest`, which is
handled at network ingress and never enters the simulation.

| Policy | Protocol version | Content | Modded clients |
|---|---|---|---|
| `LocalDev` (default) | not checked | not checked | accepted |
| `PrivateCustom { accepted_manifest_hash: None }` | must match | any | accepted |
| `PrivateCustom { accepted_manifest_hash: Some(h) }` | must match | must hash to `h` | accepted if approved |
| `Official` | must match | must match server exactly | **refused** |

`ServerPolicy::requires_manifest()` is true only for `Official`; such a server
refuses every gameplay command until a manifest has been accepted, and
disconnects a session whose manifest fails. **The server enforces the policy
itself**, independently of the anti-cheat provider — an official server with
anti-cheat disabled still refuses a mismatched manifest.

Select at runtime: `--server-policy local|private|official`.

---

## 10. Admin and debug command permissions

`anti_cheat::admin`. Debug and admin "cheats" are server-permissioned, never
client-asserted.

Roles form a strict containment ladder:

| Role | Permissions |
|---|---|
| `Player` (default) | none |
| `Moderator` | `KickSession`, `InspectSecurityLog` |
| `Host` | Moderator + `BanSession`, `SetTrustLevel`, `GrantResources`, `SetSessionRole`, `ReloadManifest` |
| `ServerOwner` | Host + `ToggleAntiCheat` |

Every session starts as `Player`, so a brand new connection can never execute a
privileged command. `required_admin_permission(&Command)` enumerates the
privileged command set in exactly one place; `AdminRegistry::authorize` is the
single authorization choke point, and it runs **before** the command is
buffered, independently of which anti-cheat provider is installed. A denied
attempt is recorded as a `High` severity `admin_permission_denied` security
event.

Privileged commands: `AdminKickSession`, `AdminSetTrustLevel`,
`AdminGrantResource`, `AdminSetSessionRole` (codec discriminants 161–164).

---

## 11. Command validation and replay protection

Command envelopes carry a session id, a monotonic sequence number, a client tick,
a server-issued capability token and the command.

**Session binding runs first.** `session_id` is a client-supplied routing field
and proves nothing on its own. The server issues a random 64-bit token at
handshake, returns it only to the peer that completed the handshake, and accepts
a command packet only when the token matches **and** the datagram arrived from
the address bound to that session (`Session::verify_binding`). A mismatch is
counted and reported as a `session_binding_mismatch` `SecurityEvent`
(`Critical`). Because this check precedes
`Session::validate_and_advance_sequence`, an unauthenticated datagram can no
longer latch a real player's `last_received_sequence` and mute them. Loopback
and in-process transports have no address to compare, so single-player and local
testing need no configuration.

`Session::validate_and_advance_sequence` then rejects duplicate and out-of-order
sequences before any inspection runs, so replay protection does not depend on
anti-cheat being enabled.

**Per-command entitlement.** Every command is dispatched through the single
`sim_core::dispatch::apply_command` against an `ActorContext` the server built
from its own session record: session id, player id, faction, avatar entity and
admin role. Faction is never read from a command payload. The dispatcher's match
is exhaustive with no catch-all, so a new command variant cannot reach production
without an explicit decision, and every refusal is a typed `GameError` returned
to the caller and counted.

Server-side validation independent of anti-cheat includes: authoritative movement
clamping against the simulation's own collision world
(`sim_core::terrain::validate_authoritative_movement`), world bounds and
placement overlap, site reservation, **construction cost deduction from the
builder's own container and a live 15 m build-reach check against the builder's
authoritative position**, faction ownership on dismantle, repair, production and
extraction configuration, faction ownership of both endpoints of a logistics job
and of the job itself at claim/pickup/dropoff, transactional inventory with
reservation accounting and container faction ownership, logistics job claim
exclusivity, and structure state machine transitions.

---

## 12. Resource integrity

Resource changes are transactional:

- Reservations must be committed.
- Failed transactions leave state completely unchanged.
- Overflow and underflow are detected and reported.
- The `impossible_economy_delta` detector compares requests against the
  *unreserved* balance, so it also catches attempts to spend material already
  locked by a logistics reservation.

---

## 13. Operating the security layer

```
# local development / single-player (default): anti-cheat off
cargo run -p dedicated-server

# private co-op server with internal heuristics
cargo run -p dedicated-server -- --anti-cheat basic --server-policy private

# official server: strict manifest matching, internal heuristics
cargo run -p dedicated-server -- --anti-cheat basic --server-policy official
```

Startup prints the active provider, policy, build manifest and manifest hash.

Recommended policy per the master spec:

| Deployment | Provider | Server policy |
|---|---|---|
| Local single-player | none | local |
| Private co-op | basic (host's choice) | private |
| Official co-op | basic (EAC when available) | official |
| Future ranked PvP | EAC required | official |

---

## Known gaps

Tracked honestly rather than implied to be solved:

- **Hidden-enemy replication filtering (Milestone 14).** Snapshots today
  replicate the existence of every entity — id, faction, region, active flag and
  a component mask — to every session. No position, health, inventory or order
  state crosses the wire, so no *actionable* hidden state leaks, but per-faction
  visibility filtering of the entity list itself is M14's replication-interest
  work. The `faction_knows_entity` hook that the wallhack detector consumes is
  wired and inert until then.
- **Combat detectors are partial (Milestone 13).** Fire-rate uses a conservative
  floor rather than real weapon cycle times; ammo uses the generic `RES_AMMO`
  container balance rather than magazine state. Invalid *damage claims* cannot
  be detected at all until a damage system exists — today no client can claim
  damage, because no damage command exists.
- **No avatar binding (Milestone 12).** Actor-scoped detectors fall back to
  session-scoped tracking.
- **Research validation.** `Command::Research` has no server-side validation to
  guard yet; the research system is Milestone 19.
- **Transport security.** No encryption, no message authentication, no
  socket-layer rate limiting, no DDoS mitigation. A network attacker can read
  and forge packets today.
- **No persistent ban list.** Bans are per-match and in-memory; a banned player
  can reconnect as a new session. Persistence is Milestone 23.
- **Platform anti-cheat is absent**, with the consequences listed in
  [section 2](#2-threat-model).
