# Changelog

All notable changes to `libpcern` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-02

Initial release. Extracted from `console_server`'s own syscall bindings
(Checkpoint E) as the shared `no_std` crate every userland program now
depends on, and grown alongside every phase since:

### Added

- The `int 0x80` syscall trampoline (hand-written assembly, see the
  README for why) and a typed wrapper for every syscall: `send`, `recv`,
  `exit`, `yield_now`, `getpid`, `register_irq`, `map_memory`,
  `mem_alloc`, `create_task`, `endpoint_create`, `cap_mint_badged`,
  `cap_revoke`.
- Name-service client helpers: `pack_name`, `register_name`,
  `lookup_name`, `lookup_name_retry`.
- Storage-service client helpers: `storage_connect`, `storage_read_block`.
- Filesystem-service client helpers: `fat_pack_name`, `fs_connect`,
  `fs_open`, `fs_read`.

### Fixed

- The original trampoline wrote the syscall result through `edi` *after*
  `int 0x80`, clobbering `edi` before it could be stored once `edi` became
  a real output register (the transferred-capability slot, Checkpoint E).
  Fixed by using `ebp` as the post-syscall scratch pointer instead, since
  `ebp`'s original value is preserved on the stack across the call
  regardless of what's in the register in between.
