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

## [0.8.0] - 2026-07-09

### Added

- `SYS_TRY_RECV` (syscall 15): like `SYS_RECV`, but returns immediately
  with a `NO_MESSAGE` sentinel instead of blocking when nothing is
  available for the given endpoint. This kernel's IPC has no `select` --
  a blocking `recv` can only ever watch one endpoint -- and `netstack`'s
  new TCP client is the first task that genuinely needs
  to poll two independent event sources (its own inbox's ARP/ICMP
  traffic, and an external client's TCP requests) without risking a
  deadlock from leaving a request outstanding with `net_rtl8139` while
  it waits on the other. See `ipc::try_recv`'s own doc comment for the
  full reasoning, including the specific deadlock this avoids.
- Spawns `http_client_test` (see `userland/cap_test`), `net_rtl8139`,
  and `netstack` in a new standalone `tcp_test` boot configuration
  (`grub-tcptest.cfg`, see `make test-tcp`) -- the same relative spawn
  order as production, so `netstack`'s new "tcp" name registration
  lands at the id nameservice's ALLOWLIST expects.

## [0.7.0] - 2026-07-05

### Added

- Spawns `netstack` (see `userland/services/netstack`) right after
  `net_rtl8139`, and only when it was actually found -- both in
  production boot and in a new standalone `arp_icmp_test` boot
  configuration (`grub-arptest.cfg`, see `make test-arp`). No new
  capabilities or syscalls needed: netstack reaches `net_rtl8139` the
  same way any other client would, by looking up `"net"` through the
  name service, so main.rs's own role is unchanged from spawning
  `shell` -- an inbox and nothing else.

### Fixed

- Shell's spawn block wasn't excluded from the new `arp_icmp_test`
  feature's `#[cfg(not(any(...)))]` list, so building with that feature
  would have spawned shell's code under module index 4 -- which, in
  that boot configuration, is actually `net_rtl8139`'s own binary --
  running with none of the capabilities `net_rtl8139` expects. Caught
  before this ever shipped in a build anyone would run; added to the
  exclusion list alongside every other standalone test harness.

## [0.6.0] - 2026-07-04

A PCI-enumerated NIC driver, and everything the kernel needed to grow to
support one.

### Added

- A minimal PCI configuration-space enumerator (`pci.rs`): brute-force
  bus/device/function scanning via legacy port I/O (0xCF8/0xCFC), reading
  a matched device's BAR0, interrupt line, and enabling it (I/O space +
  bus master). 32-bit port I/O (`inl`/`outl`) added to `port.rs` to
  support it -- everything before this was byte-wide.
- Generic IRQ2-15 dispatch (`idt.rs`/`irq.rs`): unlike the timer's/
  keyboard's fixed IRQ0/IRQ1 handlers, a PCI device's interrupt line is
  only known once enumeration reads it out of the device's own config
  space at boot, so there's no fixed number to hardcode a handler
  against ahead of time. All fourteen remaining lines get a generic stub
  registered unconditionally; an unregistered line firing costs one
  harmless no-op dispatch.
- `pic::mask`/`pic::unmask`, and a fix for a real interrupt storm this
  surfaced during its own bring-up: a PCI interrupt is
  level-triggered, not edge-triggered like the keyboard's, so sending EOI
  alone (this kernel's only pattern until now) let the still-asserted
  line re-trigger the instant `iret` re-enabled interrupts -- faster than
  any task could ever be scheduled to actually clear the device's own
  condition, exhausting the kernel heap in an infinite loop. `irq::dispatch`
  now masks the line before EOI; `ipc::recv` unmasks it again once the
  registered task is back and ready for another, tying the mask/unmask
  lifecycle directly to the driver's own service loop with no new
  syscall needed.
- The TSS I/O permission bitmap (`gdt.rs`) now covers the full 65536-port
  architectural range instead of the first 1024: a PCI device's I/O-BAR
  is assigned by firmware at boot to whatever address the platform
  picks (QEMU's default chipset lands the RTL8139 around 0xC000), not a
  fixed low legacy address the old, smaller bitmap window was sized
  around.
- `mm::frame::alloc_frames_contiguous`/`free_frames_contiguous`: a linear
  scan for a run of physically contiguous free frames, needed for a
  DMA-capable device's ring buffer (the RTL8139's receive ring), which a
  device's own DMA engine requires but which this allocator's original
  single-frame-at-a-time `alloc_frame` can't guarantee.
- `SYS_MEM_ALLOC` now accepts a page count (`ecx`, `0` treated as `1` for
  every existing caller) and additionally returns the allocated range's
  physical base address (`ecx` on output) -- needed by a DMA-capable
  driver to tell its hardware where its buffers actually are, since the
  device operates on physical memory directly with no notion of the
  calling task's own page tables. Exposing it isn't a new privilege
  boundary: a caller already holding the resulting `MemoryGrant` can
  already read and write every byte that address names.
- Simplified the `*_test`/`test_harness` mutual-exclusion check from ten
  pairwise `compile_error!` blocks (which grew quadratically with every
  new feature) to one linear count-and-assert, ahead of adding the sixth
  (`nic_test`).

### Fixed

- `spawn_net_rtl8139` returning early (no card found) consumed no task
  id, so the production boot's next spawn (`shell`) silently slid into
  the id nameservice's registration allowlist hardcodes to the name
  "net" -- letting shell claim that trusted name in the real driver's
  place on any boot without a physical/emulated RTL8139 attached, which
  is the common case outside `make test-nic`. Fixed by always spawning
  the NIC driver *last*, after every task with a guaranteed, deterministic
  id (`main.rs`'s production and `nic_test` spawn orders both reordered
  accordingly; `net_rtl8139` now lands at task id 6, not 5): its absence
  now simply leaves that id unallocated instead of letting anything else
  take it.
- `irq::dispatch` masked an IRQ line even when no endpoint was registered
  for it, but only `ipc::recv`'s per-endpoint unmask could ever unmask it
  again -- an unregistered line firing even once (a spurious 8259
  interrupt, or a PCI line firing before its driver's own `register_irq`
  call has run) was masked permanently instead of the "harmless no-op"
  this generic dispatch path was meant to be for that case. Masking is
  now conditioned on a handler actually being registered.
- `ipc::recv` unmasked *every* IRQ registered to the endpoint being
  received on, regardless of which one actually fired -- harmless for
  today's single-IRQ-per-endpoint drivers, but a hypothetical future
  endpoint with two IRQs registered on it could have one's `recv` call
  prematurely unmask the other, still-unacknowledged line. Now tracks
  which specific IRQ each endpoint's last delivery was for and unmasks
  only that one on the next `recv` -- which also meant `irq::register`
  needed to start unmasking a freshly-registered line itself (nothing
  else ever had, once `recv`'s unmask stopped being a blanket "every IRQ
  this endpoint has ever registered" check), the one-time turn-on a line
  that's never fired yet still needs before it can fire at all.
- `sys_mem_alloc`'s failure paths cleared `eax` (the capability slot) but
  left `ecx` (the physical-base output) holding the caller's own
  requested page count, which a caller that checked `ecx` before `eax`
  could mistake for a real physical address. Both registers are now
  cleared together on every failure path.
- `gdt::set_io_permissions` reset the *entire* 8193-byte I/O bitmap on
  every task switch regardless of which task was switching in, a 64x
  cost increase once the bitmap grew to cover the full port range above
  (most tasks need only a handful of low ports, if any). Now only resets
  the union of the previous and current calls' actually-needed byte
  range, which stays small except for the one switch immediately after a
  high-port task (like the NIC driver) runs.
- `pci::find_device` scanned all 256 possible PCI buses even though this
  module's own design already assumes (and documents) that every device
  this project cares about sits on bus 0 with no bridges to recurse
  through. Now scans only bus 0.

## [0.5.0] - 2026-07-04

### Added

- `SYS_REBOOT` (syscall 14): resets the machine by pulsing the 8042
  keyboard controller's CPU-reset output line -- the standard software
  reset technique for x86 systems with no ACPI reset register support
  (this kernel has none). Gated by a new zero-data `CapKind::RebootControl`
  capability, the same "holding it is the whole check" pattern as
  `IrqControl`/`MemoryGrant`; today only a dedicated test fixture
  (`reboot_test`, its own standalone `--features reboot_test` boot
  configuration -- see `make test-reboot`) is ever handed one, since the
  real intended holder (an update service) doesn't exist yet.

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
