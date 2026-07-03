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
is, checkpoint by checkpoint, see [CLAUDE.md](CLAUDE.md).

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
- **`userland/services/`** -- ring-3 tasks with *no* hardware capabilities
  of their own, reachable only by name:
  - **nameservice** -- the one piece of discovery every task gets for free
    (a capability to it is auto-granted at spawn, the way Unix processes
    inherit fds 0/1/2): a small registry other services register under a
    name with, and any task can look up by name.
  - **fs_fat32** -- a FAT32 filesystem server (read and write), registered
    as `"fs"`, that reads/writes sectors through `storage_ata` and serves
    files to its own clients.
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
- **mtools** -- only needed to build the FAT32 test image (`mformat`/
  `mcopy`); not required just to build and boot the OS.
- **qemu-system-i386** -- to actually run it.

On Debian/Ubuntu:

```sh
sudo apt-get install nasm grub-common grub-pc-bin xorriso mtools qemu-system-x86
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

`make clean` removes all build artifacts (including the generated `iso/`
tree and the ISO itself) across the kernel and every userland crate.

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
that no unexpected interrupt vectors fired. See `run_tests.sh` for exactly
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
synthetic in-process byte -- see `run_console_input_test.sh`/
`run_raw_input_test.sh`/`run_editor_test.sh` and
`userland/cap_test/README.md` for how synchronization works and what
each one proves.

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
run_tests.sh                the test harness's pass/fail checker
run_console_input_test.sh   the keyboard-input test's pass/fail checker
run_raw_input_test.sh       the raw-input test's pass/fail checker
run_editor_test.sh          the editor test's pass/fail checker
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
longer content, and there's no networking, no SMP, no persistence beyond
what's described above. See [CHANGELOG.md](CHANGELOG.md) for OS-level
release history, and `kernel/CHANGELOG.md`/`userland/<name>/CHANGELOG.md`
for what's actually been built in each individual crate.

## License

MIT -- see [LICENSE](LICENSE).
