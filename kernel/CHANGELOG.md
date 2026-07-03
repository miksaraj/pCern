# Changelog

All notable changes to the `pcern` kernel crate will be documented in this
file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each userland service has its own version and changelog under
`userland/<name>/CHANGELOG.md`. The OS this kernel and those services
combine into -- **ZephyrLite** -- has its own release history and its own,
separate versioning scheme at [../CHANGELOG.md](../CHANGELOG.md); a kernel
version bump here does not by itself mean a new ZephyrLite release, and
vice versa. Before the kernel/userland split into `kernel/`/`userland/`
subdirectories (see the root changelog's restructuring entry), this file
*was* the root changelog and covered "the kernel and the project as a
whole" -- entries below this point predate that split and may describe
userland changes made alongside a given kernel release, kept here for
historical context.

## [Unreleased]

## [0.4.0] - 2026-07-03

Read, write, and edit text files: a real full-screen editor, built on top
of write support all the way down the storage stack.

### Added

- ATA/IDE write support in `storage_ata`: a new `STORAGE_OP_WRITE_BLOCK`
  protocol op alongside the existing `READ_BLOCK`, backed by a real
  `write_sector` (`CMD_WRITE_SECTORS`, correct DRQ-wait-per-word
  sequencing, a `STATUS_DF` write-fault check).
- Write support in `fs_fat32`: overwrite, growth past a file's current
  cluster span (free-cluster allocation + FAT chain extension, mirrored
  to both FAT copies), and brand-new file creation, through a new
  `FS_OP_WRITE` op and a "create if missing" flag on the existing open
  op.
- A raw single-keystroke input mode in `console_server`:
  `CONSOLE_OP_SET_MODE`/`CONSOLE_OP_READ_KEY`, layered onto the existing
  line-input reader connection rather than a second one. `keyboard.rs`
  gained Ctrl-state tracking and `0xE0`-prefixed extended-key decoding
  (arrows, Home/End/Delete/PageUp/PageDown) to support it.
- `shell`'s `edit <file>` command: a full-screen text editor (arrow/Home/
  End/Delete/Backspace/insert, Ctrl-S to save, Ctrl-Q to discard) built on
  the three additions above. The editor's core logic lives in a new
  `libpcern::editor` module, shared with a `cap_test` regression fixture
  so the exact code that ships is the exact code that fixture exercises.
- `assert_eq!` checks (not `debug_assert_eq!` -- these must hold in the
  shipped release binary) right after spawning `console_server`/
  `storage_ata`/`fs_fat32` confirming each lands at the task id
  `nameservice`'s registration `ALLOWLIST` hardcodes it to. A spawn-order
  change that broke this correspondence would otherwise either silently
  break name registration or let the wrong task claim a trusted name; it
  now panics loudly at boot instead.

### Fixed

- `console_server`'s raw-mode key delivery initially held only one
  unclaimed keystroke, overwriting (dropping) any additional ones that
  arrived while a client was busy -- a real case for the editor, whose
  redraw cost scales with how much has been typed so far. Replaced with a
  32-deep queue.
- `shell`'s `edit` command originally switched the console into raw mode
  only after allocating the editor's buffer and opening/loading the file
  -- a user typing the instant `edit <file>` completed could have those
  keystrokes land while the connection was still in line mode, silently
  absorbed by its echo/accumulate path instead of reaching the editor.
  Fixed by switching to raw mode as the very first thing the command
  does.

### Security

- `sys_map_memory`/`sys_mem_alloc` accepted any caller-chosen `virt_addr`
  with no upper bound, and `PageDirectory::map_page` treated any present
  PDE it found there as a page-table pointer regardless of whether it was
  actually a 4 MiB PSE mapping. Since every task's page directory shares
  the kernel's own higher-half and physical-memory-linear-map PDEs
  verbatim (both PSE), an unprivileged task holding nothing more than a
  `MemoryGrant` it got for free via `SYS_MEM_ALLOC` could call
  `SYS_MAP_MEMORY` with a `virt_addr` landing in either range and flip
  `PAGE_USER` on the whole 4 MiB entry -- gaining ordinary read/write
  access to that much real physical memory, and by repeating this across
  the physmap's PDE range, to all of it. Fixed in two layers: both
  syscalls now reject any `virt_addr` (or range, for `sys_map_memory`) at
  or above `KERNEL_VMA`, and `map_page` itself now asserts (a real
  `assert!`, checked in release builds) that it's never asked to remap a
  PDE with the PS bit set, as a hard backstop against any future caller
  making the same mistake.

## [0.3.0] - 2026-07-03

The first interactive OS experience: type a command, something happens.

### Added

- A shared-memory buffered line-input protocol in `console_server`
  (`CONSOLE_OP_SET_BUFFER`/`SET_READER`/`READ_LINE`), mirroring
  `storage_ata`'s connect shape -- a client hands over a page and its own
  reader endpoint, then requests one typed line at a time. Verified
  against real PS/2 keystrokes injected through QEMU's monitor `sendkey`
  command (not a synthetic in-process byte), via a new standalone
  `keyboard_test` kernel feature/boot config and permanent fixture
  (`console_input_test`) synchronized on a serial readiness marker rather
  than a fixed sleep.
- A new syscall, `SYS_SPAWN_FROM_MEMORY` (13): loads and runs a program
  from up to 4 capability slots naming `MemoryGrant` pages the caller
  already filled with code (e.g. read from a file), the load-from-memory
  counterpart to the existing load-from-multiboot-module path
  (`SYS_CREATE_TASK`). Resolves capabilities the same way every other
  syscall argument does and always copies the bytes into freshly
  allocated frames, never mapping a resolved grant's physical pages
  directly into the new task. The new task gets no privilege beyond the
  universal name-service auto-grant.
- `userland/shell`: a minimal interactive shell reading lines from
  `console_server`'s new input protocol and dispatching `read <file>`/
  `run <file>` against `fs_fat32` and the new syscall -- the first thing
  in this project you can actually type a command into and watch happen.
- A `release.yml` GitHub Actions workflow: on publishing a GitHub release,
  builds the production ISO from the tagged commit (`make iso`) and
  attaches it as `pcern-<tag>-i386.iso`.

### Fixed

- `PageDirectory::new()` read its higher-half template page-directory
  entries by indexing the `boot_page_directory` static directly, which
  only works while that static's own low physical address happens to
  still be identity-mapped under the currently active page directory --
  true throughout every earlier caller (all of which ran during boot
  under `boot_page_directory` itself), but not once a page directory is
  built from inside a syscall running under some other task's own page
  directory, which `SYS_SPAWN_FROM_MEMORY` is the first thing to actually
  do. Fixed to read through the physical-memory map instead, which is
  present in every address space regardless of which one is active.

### Security

- `console_server`'s new line-input protocol had no check that a
  `CONSOLE_OP_SET_BUFFER`/`SET_READER`/`READ_LINE` message came from the
  task that owned the current reader connection -- any task, including
  one spawned with no privilege beyond the universal name-service
  auto-grant (e.g. via the new shell's `run` command), could re-point
  the connection at itself and receive every keystroke typed afterward
  instead of the legitimate reader. Fixed by latching the first
  successful `SET_BUFFER`'s kernel-attested sender id as the
  connection's owner and ignoring these ops from any other sender.

### Removed

- The two endless-print kernel smoke-test tasks (`task_a`/`task_b`,
  present since early on) from every boot configuration,
  including production -- their unthrottled console/serial spam has no
  place in a build meant to actually be typed into. Removing them also
  simplified every task-id-dependent build (`nameservice`'s registration
  allowlist, `run_tests.sh`) down to one consistent numbering instead of
  the production/test_harness builds needing +2 for their presence.

## [0.2.0] - 2026-07-02

Full rewrite of the original C stub into a Rust nanokernel with real
privilege separation, a capability-based security model, and a small
userspace ecosystem of drivers/services built on top of it.

### Added

- Higher-half boot with paging, a physical frame allocator, and a bump
  kernel heap (`GlobalAlloc`).
- Preemptive round-robin scheduler, ring-3 tasks, a TSS-based privilege
  transition, and a syscall gate (`int 0x80`).
- Rendezvous IPC (`send`/`recv`) and a multiboot-module task loader.
- A capability table: per-task capability spaces (CSpaces), capability
  derivation with badging, transfer over IPC, and revocation that cascades
  to every capability derived from the revoked one. IPC addressing moved
  from raw task IDs to capability slots; memory-mapped I/O and IRQ
  registration became capability-mediated (`MemoryGrant`, `IrqControl`)
  instead of a single `is_driver` flag and a hardcoded allowlist.
- A name service (`userland/nameservice`) as the one piece of discovery
  every task gets for free, replacing hand-wired capability grants for
  everything except a task's initial hardware/name-service capabilities.
- Userspace drivers/services: `console_server` (VGA/ANSI text console +
  keyboard, moved out of the kernel), `storage_ata` (polling ATA/IDE PIO
  driver serving block reads over a shared-memory grant), `fs_fat32`
  (read-only FAT32 filesystem server on top of `storage_ata`).
- `userland/libpcern`, a shared `no_std` syscall/protocol binding crate
  used by every userland program above.
- `userland/cap_test`, a set of regression fixtures covering capability
  transfer/badging/revocation, shared-memory grants, and the
  storage/filesystem protocols end to end.
- An automated test harness (`make test`): a second kernel build
  (`--features test_harness`) that spawns every `cap_test` fixture
  alongside the normal service set, boots headlessly in QEMU against a
  generated FAT32 test image, and checks every fixture's exit code and
  that no unexpected interrupt vectors fired.
- CI (GitHub Actions) running `make iso` and `make test` on every push/PR
  against `main`.

### Changed

- `sys_debug_write` (a syscall that let any ring-3 task write arbitrary
  kernel memory to serial) was retired once the console server could take
  over all userspace output.
- The kernel's own keyboard-echo path was retired once `console_server`
  owned the keyboard.

### Fixed

- `switch_to` wasn't saving/restoring `EFLAGS` across a context switch.
- `sys_debug_write` allowed an arbitrary kernel-memory read from ring 3
  (fixed by retiring the syscall entirely, see above).
- An unhandled exception on a ring-3 task would halt the entire kernel
  instead of just that task.
- `block_current`/`exit_current` could panic if they emptied the ready
  queue.
- `exit_current` didn't clean up a task's pending IPC entries, leaking
  state on every task exit.
- A `map_page` bug leaked the `PAGE_USER` bit across an unrelated shared
  4 MiB region.
- An alignment-padding sliver in the heap allocator could be smaller than
  the smallest trackable free block, corrupting the free list.
- `total_memory_bytes` silently returned `0` instead of failing loudly
  when multiboot didn't report `FLAG_MEM`.
- The TSS's `esp0` field started at `0`, a landmine for the first ring-3
  to ring-0 transition.
- `ping.asm`/`pong.asm` (an early IPC demo) silently completed only one
  round-trip instead of five, due to a clobbered register across a
  `call` -- caught while adding stricter round-by-round verification,
  moot once those demo programs were removed as redundant with
  `cap_test`'s fixtures.

### Removed

- `ping.asm`/`pong.asm`, the original two-task IPC demo, once
  `userland/cap_test`'s fixtures covered the same ground more thoroughly.
- A handful of standalone `.asm` verification fixtures at the userland
  root (`driver_test.asm`, `irq_test.asm`, `nondriver_test.asm`,
  `ring3_test.asm`, `ring3_cli_test.asm`) left over from early development;
  all of them exercised syscalls or mechanisms (`sys_debug_write`,
  `is_driver`) retired long before this release.

## [0.1.0] - 2023-01-31

Initial "pikokernel" exercise: a minimal multiboot-compliant kernel written
in C, booting to a hardcoded VGA text message under QEMU. Tagged
pre-release; superseded entirely by the 0.2.0 Rust rewrite.
