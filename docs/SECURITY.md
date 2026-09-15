# Security

This document describes security considerations for the RTS Engine.

## Server Authority

The server is the ultimate authority on all game state. Clients cannot directly modify server state.

## Command Validation

All commands are validated on the server:

- Movement is within bounds
- Resources are sufficient
- Construction is valid
- Commands are in correct state

## Replay Protection

Commands include:

- Session ID
- Monotonic sequence number
- Client tick

Duplicate commands are rejected.

## Resource Integrity

Resource changes are transactional:

- Reservations must be committed
- Failed transactions leave state unchanged
- Overflow/underflow is detected and reported

## Future Work

This document will be expanded with:

- Anti-cheat measures (Milestone 25)
- Encryption requirements
- Rate limiting
- DDoS protection
