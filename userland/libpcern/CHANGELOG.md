# Changelog

All notable changes to `libpcern` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-07-03

### Added

- `storage_write_block`/`STORAGE_OP_WRITE_BLOCK`.
- `fs_open_for_write`/`fs_write`/`FS_OP_WRITE`, and a "create if missing"
  flag on `fs_open_impl`'s `OPEN_NAME2` call -- `fs_open`'s own behavior
  is unchanged.
- `console_set_mode`/`console_read_key`/`CONSOLE_OP_SET_MODE`/
  `CONSOLE_OP_READ_KEY`, and the tagged `KEY_UP`/`KEY_DOWN`/`KEY_LEFT`/
  `KEY_RIGHT`/`KEY_HOME`/`KEY_END`/`KEY_DELETE`/`KEY_PAGE_UP`/
  `KEY_PAGE_DOWN` constants, mirroring `console_server::keyboard`'s
  values exactly since they cross the wire.
- A new `editor` module: `Editor`, a full-screen text editor's core logic
  (cursor-tracked buffer, key application, ANSI redraw), shared between
  `userland/bin/shell`'s `edit` command and `userland/cap_test`'s
  `editor_input_test` regression fixture.

## [0.2.0] - 2026-07-03

### Added

- `spawn_from_memory`: client wrapper for the new `SYS_SPAWN_FROM_MEMORY`
  syscall (13) -- loads and runs a program from up to 4 `MemoryGrant`
  capability slots, the load-from-memory counterpart to `create_task`.
- Console-input client helpers: `console_connect`, `console_read_line`,
  and the `CONSOLE_OP_*`/`CONSOLE_LINE_MAX` constants for
  `console_server`'s new line-input protocol.

## [0.1.0] - 2026-07-02

Initial release. Extracted from `console_server`'s own syscall bindings
as the shared `no_std` crate every userland program now depends on, and
grown alongside the rest of the project since:

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
  a real output register (the transferred-capability slot). Fixed by
  using `ebp` as the post-syscall scratch pointer instead, since
  `ebp`'s original value is preserved on the stack across the call
  regardless of what's in the register in between.
