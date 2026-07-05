# ZephyrLite

ZephyrLite is a small, from-scratch x86 (i686) operating system: a
capability-secured microkernel (**pCern**) plus a minimal userspace built
directly on top of it -- a console, a keyboard driver, a name service, a
disk driver, a FAT32 filesystem with real read/write, and a shell with a
full-screen text editor. It's a learning/research project, not a production
OS -- see [Status](#status) below for exactly how far "OS" goes today.

If you're looking for the kernel specifically -- what's in ring 0, its
capability model, its syscall ABI -- see [kernel/README.md](kernel/README.md).
If you're looking for the story of *why* this project is built the way it
is, see [CLAUDE.md](CLAUDE.md).

## What runs in userspace (`userland/`)

Organized by what each piece actually *is*, not just what it's called --
see [CLAUDE.md](CLAUDE.md#driver-vs-utility-taxonomy-in-userland) for the
reasoning behind this split:

- **`userland/drivers/`** -- ring-3 tasks holding real hardware
  capabilities (`MemoryGrant`, `IrqControl`, or port I/O):
  - **console_server** -- owns the keyboard (IRQ) and VGA text console
    (MMIO). Also implements the line-discipline/ANSI-escape logic on top,
    the same way a Unix tty driver legitimately combines both rather than
    splitting them into separate layers.
  - **storage_ata** -- a polling-only ATA/IDE PIO driver (read and write)
    for the primary bus, registered as `"storage"`.
  - **net_rtl8139** -- a minimal RTL8139 Fast Ethernet driver, registered
    as `"net"`, discovered via PCI enumeration at boot. Raw Ethernet
    frames in and out only -- no ARP, no IP of its own; that's
    `netstack`'s job, built on top of it.
- **`userland/services/`** -- ring-3 tasks with *no* hardware capabilities
  of their own, reachable only by name:
  - **nameservice** -- the one piece of discovery every task gets for free
    (a capability to it is auto-granted at spawn, the way Unix processes
    inherit fds 0/1/2): a small registry other services register under a
    name with, and any task can look up by name.
  - **fs_fat32** -- a FAT32 filesystem server (read and write), registered
    as `"fs"`, that reads/writes sectors through `storage_ata` and serves
    files to its own clients.
  - **netstack** -- claims a static IP, answers ARP requests for it, and
    replies to ICMP echo (ping) requests, all by being `net_rtl8139`'s
    client over ordinary IPC the same way `fs_fat32` is `storage_ata`'s.
    No hardware capabilities of its own, and only spawned when a real
    RTL8139 NIC was actually found at boot.
- **`userland/bin/`** -- user-facing programs:
  - **shell** -- a minimal interactive shell: reads a line via
    `console_server`'s input protocol, then dispatches `read <file>`/
    `edit <file>`/`run <file>` against `fs_fat32`, the console's raw
    single-keystroke mode, and the `SYS_SPAWN_FROM_MEMORY` syscall --
    `edit` is a real full-screen text editor with arrow-key navigation.
- **`userland/libpcern/`** -- the shared `no_std` syscall/protocol bindings
  every program above links against. Not a driver, service, or program in
  its own right (it's never a task -- no `_start`), so it stays outside the
  three directories above.
- **`userland/cap_test/`** -- regression fixtures for the capability/IPC
  mechanisms (transfer, badging, revocation, shared-memory grants) and
  end-to-end clients that exercise the storage, filesystem, console-input
  (line and raw), and full-screen-editor protocols. Not part of a normal
  boot -- see [Testing](#testing) below. Also outside the driver/service/
  program split: these are test fixtures, not shipped code.

Each has its own README under `<path>/README.md` with the wire protocol
and design notes specific to it.

## Prerequisites

You'll need:

- **Rust nightly** with the `rust-src` and `llvm-tools` components. A
  `rust-toolchain.toml` at the repo root pins the exact channel for the
  kernel and every userland crate; if you have `rustup` installed, running
  any `cargo`/`rustc` command anywhere in this repo will install it
  automatically.
- **nasm** -- historically a build dependency for the userland toolchains;
  kept in the prerequisite list below for environments that still expect it.
- **grub-mkrescue** and **xorriso** -- build the bootable ISO
  (`grub-common` + `grub-pc-bin` on Debian/Ubuntu).
- **mtools** -- builds the FAT32 test image (`mformat`/`mcopy`) and the
  installed boot disk below; not required just to build and boot the ISO.
- **sfdisk** (`fdisk` on Debian/Ubuntu) and **mkfs.vfat** (`dosfstools`)
  -- only needed to build the installed FAT32 boot disk (`make disk`),
  not the ISO.
- **qemu-system-i386** -- to actually run it.

On Debian/Ubuntu:

```sh
sudo apt-get install nasm grub-common grub-pc-bin xorriso mtools qemu-system-x86 dosfstools fdisk
```

On macOS (Homebrew):

```sh
brew install nasm xorriso mtools qemu
brew install i686-elf-grub  # or another tap providing grub-mkrescue for i386-pc
```

(`grub-mkrescue` isn't part of upstream Homebrew's `grub` formula, which
targets EFI; you may need a third-party tap or a Linux VM/container for the
ISO-building step specifically.)

## Building and running

```sh
make iso   # builds the kernel + every default userland service, produces zephyrlite-i386.iso
make run   # builds (if needed) and boots it in QEMU with serial output on stdio
```

There's no supported `cargo run` for the kernel crate on its own -- the
kernel expects a set of multiboot modules (the userland binaries) to be
loaded alongside it, which only `make iso`/`make run` arrange, so a bare
`cargo build`/`cargo run` for `pcern` alone won't boot to anything useful.
See [kernel/README.md](kernel/README.md) if you want to build just the
kernel crate on its own (e.g. for a quick `cargo check`).

```sh
make disk       # builds a real, installed FAT32 boot disk: zephyrlite-i386.img
make run-disk   # builds (if needed) and boots it in QEMU, same as `make run` but from that disk
```

Unlike `make iso`'s read-only CD image, `zephyrlite-i386.img` is a real
disk GRUB itself boots from -- the kernel, every default userland
service, and GRUB's own bootstrap files sit on one partitioned FAT32 disk
as ordinary root-level files, through the exact same interface
`fs_fat32`'s own runtime read/write protocol supports. This is the
foundation an in-place update mechanism needs (not yet built -- see
[CHANGELOG.md](CHANGELOG.md)); `make iso`/`make run` remain the quicker
option for everyday development. Building the disk needs `sudo` for one
step (`grub-bios-setup` always needs root to install itself, the same as
a real `grub-install` on real hardware, even though the "disk" here is
just a file); everything else in `make disk` runs unprivileged.

`make clean` removes all build artifacts (including the generated `iso/`
tree, the ISO, and the installed disk image) across the kernel and every
userland crate.

## Testing

```sh
make test
```

builds a second kernel (`--features test_harness`) that additionally spawns
every fixture in `userland/cap_test`, assembles a separate ISO
(`kernel/grub-test.cfg`, kept apart from the production `kernel/grub.cfg`),
boots it headlessly in QEMU against a freshly generated FAT32 test image
(`make test-fat32-image`, built from the small tracked files in
`testdata/`), and checks that every fixture's task exited with code 0 and
that no unexpected interrupt vectors fired. See `scripts/test/run_tests.sh` for exactly
what's checked, and `userland/cap_test/README.md` for what each fixture
proves.

This is also what CI (`.github/workflows/ci.yml`) runs on every push and
pull request against `main`.

`make test` also runs `make test-keyboard`, `make test-raw-input`, and
`make test-editor`: three more standalone kernel builds
(`--features keyboard_test`/`raw_input_test`/`editor_test`, their own
`kernel/grub-*test.cfg`s) that each boot a single `cap_test` fixture
(`console_input_test`, `raw_input_test`, `editor_input_test`) in its own
isolated QEMU invocation with a monitor socket, driving it with *real*
PS/2 keystrokes via QEMU's monitor `sendkey` command rather than a
synthetic in-process byte -- see `scripts/test/run_console_input_test.sh`/
`scripts/test/run_raw_input_test.sh`/`scripts/test/run_editor_test.sh` and
`userland/cap_test/README.md` for how synchronization works and what
each one proves.

`make test` also runs `make test-reboot` (another standalone
`--features reboot_test` kernel build; its `reboot_test` fixture prints a
marker then calls the new `SYS_REBOOT` syscall -- `scripts/test/run_reboot_test.sh`
checks the marker reached serial *and* that QEMU, booted with
`-no-reboot`, exited on its own rather than hanging, since there's no
exit code to check once the machine actually resets), `make test-nic`
(another standalone `--features nic_test` kernel build; its `nic_test`
fixture hand-builds a real Ethernet+ARP request frame, sends it through
`net_rtl8139`, and blocks for QEMU usermode networking's real ARP reply
to come back through the same driver -- `scripts/test/run_nic_test.sh` checks both
`nic_test`'s own exit code *and* a real packet capture QEMU wrote to
disk during the boot, independent of anything `nic_test` itself
believes), `make test-arp` (another standalone `--features arp_icmp_test`
kernel build; boots `netstack` on top of `net_rtl8139` and, from outside
the VM entirely, sends a real ARP request and a real ICMP echo request
at it over QEMU's `-netdev socket` raw-Ethernet backend -- `scripts/test/run_arp_icmp_test.sh`
checks the ARP and ICMP echo replies it gets back, with their checksums
independently recomputed, against both what its own peer script saw
*and* what an independent pcap capture recorded, the same
don't-trust-a-single-witness pattern `scripts/test/run_nic_test.sh` established), and
`make test-disk-boot` (builds the installed FAT32 boot
disk via `make disk` and boots it headlessly, checking via
`scripts/test/run_disk_boot_test.sh` that every service's normal startup message
reached serial -- proof GRUB loaded every multiboot module from the disk
itself, not an ISO).

## Releases

Publishing a GitHub release (`.github/workflows/release.yml`, triggered on
`release: published`, not on tag push -- a tag push happens before release
notes are finalized) builds the production ISO from the tagged commit via
`make iso` and attaches it to the release as `zephyrlite-<tag>-i386.iso`.

A release's tag names a **ZephyrLite OS version** (`YY.MM[-{alpha|beta}].N`
or `YY.MM-rcN` -- see [CHANGELOG.md](CHANGELOG.md#versioning) for the full
scheme), not any single crate's own SemVer -- the kernel and every
userland crate keep versioning themselves independently in their own
`Cargo.toml`/`CHANGELOG.md`.

## Repository layout

```
kernel/                     the pCern nanokernel (ring 0) -- see kernel/README.md
userland/
  drivers/                  ring-3 tasks with real hardware capabilities
  services/                 ring-3 tasks with no hardware capabilities of their own
  bin/                      user-facing programs
  libpcern/                 shared no_std syscall/protocol bindings
  cap_test/                 regression fixtures (see Testing above)
testdata/                   small fixed input files for the FAT32 test image
scripts/test/
  run_tests.sh                the test harness's pass/fail checker
  run_console_input_test.sh   the keyboard-input test's pass/fail checker
  run_raw_input_test.sh       the raw-input test's pass/fail checker
  run_editor_test.sh          the editor test's pass/fail checker
  run_reboot_test.sh          the reboot-syscall test's pass/fail checker
  run_nic_test.sh             the NIC-driver test's pass/fail checker
  run_arp_icmp_test.sh        the ARP/ICMP responder test's pass/fail checker
  run_disk_boot_test.sh       the installed-disk-boot test's pass/fail checker
Makefile                    build orchestration -- see it for every target
CLAUDE.md                   development process and design history
CHANGELOG.md                ZephyrLite (OS-level) release history
VERSION                     the current ZephyrLite release version
rust-toolchain.toml         pins the nightly channel for every crate in this repo
```

## Status

ZephyrLite is a research/learning project, not a production OS. Its storage
driver and filesystem server only handle a single client at a time, its
full-screen editor caps a file at 64 KiB and has no scrolling viewport for
longer content, and there's no SMP, no persistence beyond what's
described above. Networking answers ARP and ICMP echo (ping) on one
hardcoded static IP (`netstack`, on top of the `net_rtl8139` driver) --
no DHCP, no TCP/UDP, no shell command to use it yet.
See [CHANGELOG.md](CHANGELOG.md) for OS-level release history, and
`kernel/CHANGELOG.md`/`userland/<name>/CHANGELOG.md` for what's actually
been built in each individual crate.

## License

MIT -- see [LICENSE](LICENSE).
