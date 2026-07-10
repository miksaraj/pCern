# Changelog

All notable changes to `libpcern` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0] - 2026-07-09

### Added

- `try_recv`/`SYS_TRY_RECV`: like `recv`, but returns `None` immediately
  instead of blocking when nothing is available. `netstack`'s TCP client
  needs it to poll both `net_rtl8139` and an external client's requests
  without ever leaving a request outstanding with either -- see
  `kernel/src/ipc.rs`'s own `try_recv` doc comment for the deadlock that
  would otherwise risk. `RecvResult` now derives `Copy`/`Clone` so a
  caller can queue results returned this way.
- TCP client protocol: `TCP_OP_SET_BUFFER`/`SET_REPLY`/`CONNECT`/`SEND`/
  `RECV`/`CLOSE` and `tcp_connect_setup`/`tcp_open`/`tcp_write`/
  `tcp_read`/`tcp_close`, mirroring the NIC protocol's own connect-then-
  request shape (see `userland/services/netstack`).
- `NIC_OP_TRY_RECV`: like `NIC_OP_RECV`, but replies immediately either
  way instead of deferring when no frame is waiting -- the non-blocking
  counterpart `netstack` needs for the same reason `try_recv` above
  exists at the syscall level.

### Fixed

- `try_recv` couldn't tell a legitimate empty poll apart from the
  kernel's generic "invalid capability" error -- both were reported as
  `u32::MAX`. The kernel now returns a distinct `TRY_RECV_ERR` sentinel
  for the latter (see `kernel/src/ipc.rs`'s own doc comment); `try_recv`
  treats it as fatal, since retrying can't fix a slot number that's
  simply wrong.

## [0.5.0] - 2026-07-04

### Added

- `mem_alloc_pages`: allocates multiple physically contiguous pages and
  returns both the capability slot and the range's physical base address
  (both `0` together on failure) -- the DMA-buffer counterpart to
  `mem_alloc` (still one page, slot only), which net_rtl8139 needs to
  hand its hardware a real physical address. `mem_alloc` itself now
  passes `page_count=1` explicitly rather than relying on the kernel's
  `0` default, since a caller can now request more.
- NIC wire protocol: `NIC_OP_SET_BUFFER`/`SET_REPLY`/`GET_MAC`/`SEND`/
  `RECV` and `nic_connect`/`nic_get_mac`/`nic_send`/`nic_recv`, mirroring
  storage's/fs's own connect-then-request shape (see
  userland/drivers/net_rtl8139).
- Port I/O helpers (`inb`/`outb`/`inw`/`outw`/`inl`/`outl`): net_rtl8139
  is the second driver (after storage_ata) to need raw port access, and
  the first to need all six widths, so these moved here to be shared by
  every current and future driver instead of each keeping its own
  byte-for-byte identical copy.

## [0.4.0] - 2026-07-04

### Added

- `reboot`/`SYS_REBOOT`: resets the machine via a `RebootControl`
  capability slot. Returns normally (rather than `-> !`) if the syscall is
  rejected, so a caller holding an invalid slot can detect and report the
  failure instead of the wrapper asserting unreachability that isn't
  actually guaranteed.

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
- `fs_truncate`/`FS_OP_TRUNCATE`: the only way a file's size shrinks --
  `fs_write` remains grow-or-overwrite-only by design (a single write's
  coverage is never a safe basis for inferring a shrink, since a write in
  the middle of a file must never truncate what comes after it). Refuses
  to grow past the file's current size, for the same reason `fs_write`
  refuses a write whose offset is past the current size (see Fixed below)
  -- neither op will expose a range of bytes nothing actually wrote.

### Fixed

- `fs_write` accepted an `offset` arbitrarily far past a file's current
  size, silently allocating (but never zero-filling) every intervening
  cluster and publishing the whole gap as valid content -- a client could
  read back stale, previously-deleted data through it. `fs_fat32` now
  refuses (`w0 = 0`) any `offset` beyond the file's current size; see its
  own CHANGELOG for the enforcement.
- `editor::Editor::append_loaded` silently truncated content past
  `EDITOR_MAX_BYTES` with no way for the caller to tell. Now returns
  `bool` (`false` the instant `data` doesn't fully fit) so a caller can
  detect and report truncation instead of silently proceeding as if the
  whole file had loaded.
- `editor::Editor` had no way to reuse an already-allocated instance --
  every construction called `mem_alloc` for a fresh 64 KiB buffer, and
  since this project has no syscall to free one, a caller reconstructing
  an `Editor` per use (as `shell`'s `edit` command did) leaked 64 KiB of
  physical frames every time. Added `Editor::reset()`, which clears the
  buffer's length/cursor without touching its already-mapped pages, so a
  long-lived `Editor` can be reused indefinitely instead.

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
