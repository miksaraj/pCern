# Changelog

All notable changes to `nameservice` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-07-09

### Added

- `(7, "tcp")` added to the registration allowlist for `netstack`'s new
  TCP client protocol, which lands at task id 7 in production and the
  standalone `tcp_test` harness (spawned right after `net_rtl8139`'s id
  6) -- but *not* asserted the way `net_rtl8139`'s own id is, since
  `netstack` is deliberately spawned at a different id (5) in the
  standalone `arp_icmp_test` harness instead, where this registration
  attempt is a harmless no-op (nothing there ever looks "tcp" up). See
  the allowlist's own doc comment for the full reasoning.

## [0.2.0] - 2026-07-04

### Added

- `(6, "net")` added to the registration allowlist for the new
  `net_rtl8139` NIC driver, which lands at task id 6 -- spawned *last*,
  after every other task with a guaranteed id, so a boot with no card
  attached simply leaves that id unallocated instead of letting the next
  spawn slide into it -- in both the production boot and the standalone
  `nic_test` harness.

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
