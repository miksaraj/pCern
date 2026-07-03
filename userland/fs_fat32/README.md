# fs_fat32

A FAT32 filesystem server (read and write, Phase 7 Checkpoint Q). Looks
up `"storage"` and reads/writes sectors through `storage_ata`, then
registers as `"fs"` and serves the same kind of shared-memory-grant
protocol to its own clients that `storage_ata` serves to it.

## Capabilities it needs

Granted by `main.rs` at spawn:

| CSlot | What |
|-------|------|
| 1     | Name service (auto-granted) |
| 2     | Its own inbox (serves this task's *own* clients) |

No hardware capabilities of its own -- it never touches a port or a
physical address directly, only through `storage_ata`. Internally, it also
creates a second, private endpoint (via `SYS_ENDPOINT_CREATE`) just for
receiving `storage_ata`'s replies, kept separate from CSlot 2 so a
client's request can never race a pending storage reply on the same
inbox -- see [CLAUDE.md](../../CLAUDE.md)'s note on why one inbox isn't
automatically safe for two roles.

## Scope (deliberately narrow)

- Root-directory files only, no subdirectory traversal.
- 8.3 names only -- long-filename (`attr == 0x0F`) and volume-label
  entries are recognized and skipped, never matched.
- One client and one open file at a time.

The one FAT32-specific landmine worth knowing about if you're reading
`main.rs`: **the root directory is itself an ordinary cluster chain**
rooted at the BPB's `root_cluster` field, not a fixed region the way
FAT16's root directory is -- so it's walked with exactly the same
`next_cluster` FAT-chain-walking logic used for ordinary file data.

## Protocol

Setup mirrors `storage_ata`'s (`FS_OP_SET_BUFFER`/`FS_OP_SET_REPLY`, same
two-message reasoning). Opening a file needs an 11-byte fixed-width 8.3
name (`libpcern::fat_pack_name`, e.g. `b"HELLO.TXT"` -> the FAT-native
space-padded 8+3 form) -- one byte more than a single message's 3-word
budget can carry, so it's split the same way setup is:

- `FS_OP_OPEN_NAME1` (`w1`/`w2` = first 8 bytes of the packed name)
- `FS_OP_OPEN_NAME2` (`w1` = last 3 bytes; triggers the actual root-
  directory scan and a reply: `w0` = found flag, `w1` = file size)
- `FS_OP_READ` (`w1` = offset, `w2` = requested length; replies with
  `w0` = bytes actually placed in the shared buffer, `0` = EOF or no file
  open). A read never crosses a sector boundary -- callers loop,
  incrementing `offset` by whatever came back, same partial-read contract
  as `storage_ata`'s.

`FS_OP_OPEN_NAME2`'s `w2` (Phase 7, Checkpoint Q) is a "create if
missing" flag: `0` opens an existing file only (the original behavior,
unchanged for every read-only caller), `1` opens an existing file or
creates a fresh zero-length one. `FS_OP_WRITE` (`w1` = offset, `w2` =
length) mirrors `FS_OP_READ`'s shape and partial-transfer contract
exactly, writing from the shared buffer and growing the file (allocating
new clusters as needed) whenever the write's end offset exceeds the
current size; the reply's `w0` is the number of bytes actually written
(`0` = no file open, buffer not mapped, or the disk is out of free
clusters).

See `userland/libpcern`'s `fs_connect`/`fs_open`/`fs_open_for_write`/
`fs_read`/`fs_write` for the client-side helpers, and
`userland/cap_test/src/bin/fs_client_test.rs` for a complete example
client (including the write/overwrite/readback exercise).

## Testing it

`fs_fat32` needs an actual FAT32-formatted disk to do anything --
`make test-fat32-image` builds one on demand (via `mtools`) from the small
files tracked in `/testdata`, and `make test` boots against it
automatically. If no disk is attached, or LBA 0 doesn't look like a valid
FAT32 boot sector, this task stays alive but never registers `"fs"` (the
same graceful-idle behavior `storage_ata` has when nobody's asked it for
anything) rather than treating that as fatal.
