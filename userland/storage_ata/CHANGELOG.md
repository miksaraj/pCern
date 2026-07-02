# Changelog

All notable changes to `storage_ata` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-02

Initial release (Checkpoint I).

### Added

- Polling-only ATA/IDE PIO driver for the primary bus, LBA28 addressing,
  one 512-byte sector at a time.
- Registers as `"storage"` with the name service.
- Serves block reads (`STORAGE_OP_SET_BUFFER`/`SET_REPLY`/`READ_BLOCK`)
  over a client-supplied shared-memory grant, single client at a time.

### Known limitations

- No IRQ14 support -- relies entirely on the scheduler's timer preemption
  instead.
- Exactly one client connection is supported; a second concurrent client
  will have its connection silently clobbered by the first (see the
  README).
