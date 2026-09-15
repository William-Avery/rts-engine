# Save Format

This document describes the save file format for the RTS Engine.

## File Format

Save files use a versioned binary format:

```
+-----------------+------------------+------------------+
| Magic String    | Version          | Checksum         |
| (4 bytes)       | (4 bytes)        | (32 bytes SHA-256)|
+-----------------+------------------+------------------+
| Compressed Data |                  |
| (variable)      |                  |
+-----------------+
```

## Magic String

```
"RTS0"
```

## Version

Current version: 1

## Checksum

SHA-256 of the compressed data for integrity verification.

## Compressed Data

The compressed data contains:

- World state (entities, regions)
- Resource quantities
- Player state
- Research progress
- Pending jobs
- Event journal (optional)

## Compression

Data is compressed using Zstandard (zstd) for fast compression/decompression.

## Future Work

This is a skeleton format. Full format details will be added in Milestone 23.
