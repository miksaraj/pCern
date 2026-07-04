# Changelog

All notable changes to `nameservice` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-04

### Added

- `(5, "net")` added to the registration allowlist for Checkpoint W's
  `net_rtl8139` NIC driver, which lands at task id 5 (right after
  fs_fat32) in both the production boot and the standalone `nic_test`
  harness.

## [0.1.0] - 2026-07-02

Initial release.

### Added

- A small in-memory name registry: `NS_OP_REGISTER`/`NS_OP_LOOKUP`, an
  8-byte packed name per entry, up to 8 entries.
- Registration gated by a compile-time `(kernel-attested task id, name)`
  allowlist; lookups open to any caller.
- Auto-granted to every task spawned after it, at a fixed capability slot,
  making it the one piece of service discovery nothing has to be told
  about individually.
