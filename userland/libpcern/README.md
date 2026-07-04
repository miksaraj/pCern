# libpcern

Shared `no_std` bindings for pCern userspace programs: the `int 0x80`
syscall trampoline, wrappers for every syscall, and the client-side
helpers for each service's IPC protocol (name-service register/lookup,
storage block reads, filesystem open/read). Every other crate under
`userland/` depends on this one so the syscall ABI and wire-format details
live in exactly one place instead of being copy-pasted into each program.

This is a library crate (`src/lib.rs`), not a binary -- it has no `_start`
of its own.

## What's in here

- **Syscall wrappers** (`send`, `recv`, `exit`, `yield_now`, `getpid`,
  `register_irq`, `map_memory`, `mem_alloc`, `mem_alloc_pages`,
  `create_task`, `endpoint_create`, `cap_mint_badged`, `cap_revoke`,
  `spawn_from_memory`, `reboot`) -- one function per syscall, matching the
  numbers/argument registers in `kernel/src/syscall.rs`.
- **Port I/O helpers** (`inb`/`outb`/`inw`/`outw`/`inl`/`outl`) -- shared
  by every driver that talks to hardware directly (`storage_ata`,
  `net_rtl8139`); only usable by a task actually granted the relevant
  ports at spawn (see each driver's own README).
- **Name-service helpers** (`lookup_name`, `lookup_name_retry`,
  `register_name`, `pack_name`) -- see `userland/services/nameservice/README.md`
  for the wire protocol these implement.
- **Storage-service helpers** (`storage_connect`, `storage_read_block`,
  `storage_write_block`) -- see `userland/drivers/storage_ata/README.md`.
- **Filesystem-service helpers** (`fs_connect`, `fs_open`,
  `fs_open_for_write`, `fs_read`, `fs_write`, `fs_truncate`,
  `fat_pack_name`) -- see `userland/services/fs_fat32/README.md`.
- **Console-input helpers** (`console_connect`, `console_read_line`,
  `console_set_mode`, `console_read_key`) -- see
  `userland/drivers/console_server/README.md`'s line-input and raw-mode protocol
  sections.
- **NIC helpers** (`nic_connect`, `nic_get_mac`, `nic_send`, `nic_recv`)
  -- see `userland/drivers/net_rtl8139/README.md`.
- **`editor` module** (`editor::Editor`) -- a full-screen text editor's
  core logic (cursor-tracked buffer, key application, ANSI redraw),
  shared between `userland/bin/shell`'s `edit` command and
  `userland/cap_test`'s `editor_input_test` regression fixture so the
  exact code that ships is the exact code that fixture exercises.

## The `int 0x80` trampoline lives in hand-written assembly

`syscall_asm.s` (assembled via `global_asm!`), not a Rust `asm!` block --
LLVM reserves `esi` as a base pointer inside ordinary (non-`naked`)
function bodies, which conflicts with this ABI's use of `esi` as a
register-pinned argument/return value. The trampoline captures every
register the kernel's syscall ABI might write on return (`eax`-`edi`)
unconditionally into a `RawResult` struct, regardless of which ones a
given syscall actually uses.

If you add a new syscall or change what a register carries, update
`syscall_asm.s`'s callers alongside the wrapper function -- there's no
compiler-enforced link between the two.

## Wire-format convention: a 3-word budget

`send`/`recv` carry a destination/endpoint capability slot, three message
words (`w0`/`w1`/`w2`), and one optional capability transfer. Every helper
in this crate designs its protocol around that fixed budget -- see
[CLAUDE.md](../../CLAUDE.md) for why, and for the pattern used when a
logical operation doesn't fit in one message (split it across two, as
`fs_open`'s `FS_OP_OPEN_NAME1`/`NAME2` does).
