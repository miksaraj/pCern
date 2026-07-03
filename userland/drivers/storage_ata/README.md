# storage_ata

A polling-only ATA/IDE PIO driver for the primary bus. Registers as
`"storage"` and serves block reads over a client-supplied shared-memory
grant.

## Capabilities it needs

Granted by `main.rs` at spawn:

| CSlot | What |
|-------|------|
| 1     | Name service (auto-granted) |
| 2     | Its own inbox |

Port access (`0x1F0`-`0x1F7`, `0x3F6`) is granted the same way any
driver's I/O ports are in this project -- through the pre-existing
`allowed_ports`/TSS-bitmap mechanism at spawn, not a capability; nothing
in this project needed a *transferable* capability for raw port access,
just a yes/no at spawn time.

## Design

Polling only, no IRQ 14: the scheduler already preempts on the timer tick,
so a polling loop in a user task can't starve anything else, and it avoids
plumbing a second IRQ path entirely. LBA28 addressing, one 512-byte sector
at a time.

## Protocol

A client connects once, then issues any number of reads and writes:

1. `STORAGE_OP_SET_BUFFER` (`transfer` = a `MemoryGrant` for a page the
   client already mapped locally) -- this driver maps the same physical
   page into its own address space and reads/writes sectors directly
   through it.
2. `STORAGE_OP_SET_REPLY` (`transfer` = a capability to the client's own
   inbox) -- where replies get sent.
3. Any number of `STORAGE_OP_READ_BLOCK` (`w1` = LBA), each replied to on
   the endpoint from step 2 with `w0 = 1` (success, sector now sitting in
   the shared page) or `w0 = 0` (failure).
4. Any number of `STORAGE_OP_WRITE_BLOCK` (`w1` = LBA) -- writes the
   shared page's current bytes to that sector, replying the same way.
   No cache-flush is issued after (see the CHANGELOG).

Two messages are needed for setup rather than one because a single
message can carry at most one capability transfer, and this handshake
needs to deliver two unrelated ones (the buffer, then the reply-to
address). See `userland/libpcern`'s `storage_connect`/`storage_read_block`
for the client-side helpers every consumer (`fs_fat32`,
`cap_test`'s `storage_client_test`) uses.

**Only one client at a time is supported** -- this driver keeps a single
global `reply_slot`/buffer-mapped pair, not per-client state, because this
project has exactly one standing client (`fs_fat32`). Running a second
client (e.g. `storage_client_test`) concurrently with `fs_fat32` would have
them silently clobber each other's connection; see
`kernel/src/main.rs`'s `test_harness_spawn` for where this is
called out, and `userland/cap_test/README.md` for how `storage_client_test`
is still exercised on its own instead.
