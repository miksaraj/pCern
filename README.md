# pCern

pCern is a small x86 (i686) nanokernel, written in Rust, built as a
learning/research project. The kernel itself does almost nothing: it brings
up paging, a scheduler, and a capability-based IPC mechanism, then gets out
of the way. Everything a traditional monolithic kernel would do in ring 0 --
the console/keyboard driver, a name service, a storage driver, a filesystem
-- runs as an ordinary ring-3 task here, talking to the kernel only through
a handful of syscalls and to each other only through capabilities it
mediates.

If you're looking for the story of *why* it's built this way and how it got
here checkpoint by checkpoint, see [CLAUDE.md](CLAUDE.md).

## What's actually in ring 0

- GDT/IDT/PIC bring-up, paging, a physical frame allocator, a small bump
  heap.
- A preemptive round-robin scheduler with ring-3 tasks (TSS-based privilege
  transitions, a syscall gate via `int 0x80`).
- A capability table: every task has its own capability space (CSpace); the
  only way to reach an IPC endpoint, a block of physical memory, or an IRQ
  is to already hold a capability naming it. Capabilities can be derived
  (with a badge), transferred between tasks over IPC, and revoked --
  revocation cascades to every capability derived from the revoked one.
- Rendezvous IPC (`send`/`recv`) addressed by capability slot, not by task
  ID.

Nothing else. No filesystem, no block device driver, no window into
physical memory beyond what a capability specifically grants.

## What runs in userspace (`userland/`)

- **console_server** -- owns the keyboard and VGA/ANSI text console.
- **nameservice** -- the one piece of discovery every task gets for free (a
  capability to it is auto-granted at spawn, the way Unix processes inherit
  fds 0/1/2): a small registry other services register under a name with,
  and any task can look up by name.
- **storage_ata** -- a polling-only ATA/IDE PIO driver for the primary bus,
  registered as `"storage"`.
- **fs_fat32** -- a read-only FAT32 filesystem server, registered as
  `"fs"`, that reads sectors through `storage_ata` and serves files to its
  own clients.
- **shell** -- a minimal interactive shell: reads a line via
  `console_server`'s input protocol, then dispatches `read <file>`/
  `run <file>` against `fs_fat32` and the `SYS_SPAWN_FROM_MEMORY` syscall
  -- the first thing in this project you can actually type a command
  into.
- **libpcern** -- the shared `no_std` syscall/protocol bindings every
  program above links against.
- **cap_test** -- regression fixtures for the capability/IPC mechanisms
  (transfer, badging, revocation, shared-memory grants) and end-to-end
  clients that exercise the storage, filesystem, and console-input
  protocols. Not part of a normal boot; see [Testing](#testing) below.

Each has its own README under `userland/<name>/README.md` with the wire
protocol and design notes specific to it.

## Prerequisites

You'll need:

- **Rust nightly** with the `rust-src` and `llvm-tools` components. A
  `rust-toolchain.toml` at the repo root pins the exact channel; if you
  have `rustup` installed, running any `cargo`/`rustc` command in this
  repo will install it automatically.
- **nasm** -- assembles the kernel's boot stub (`src/boot.s`).
- **grub-mkrescue** and **xorriso** -- build the bootable ISO
  (`grub-common` + `grub-pc-bin` on Debian/Ubuntu).
- **mtools** -- only needed to build the FAT32 test image (`mformat`/
  `mcopy`); not required just to build and boot the kernel.
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
make iso   # builds the kernel + every default userland service, produces pcern-i386.iso
make run   # builds (if needed) and boots it in QEMU with serial output on stdio
```

There's no supported `cargo run` for the kernel crate on its own -- the
kernel expects a set of multiboot modules (the userland binaries) to be
loaded alongside it, which only `make iso`/`make run` arrange, so a bare
`cargo build`/`cargo run` for `pcern` alone won't boot to anything useful.

`make clean` removes all build artifacts (including the generated `iso/`
tree and the ISO itself).

## Testing

```sh
make test
```

builds a second kernel (`--features test_harness`) that additionally spawns
every fixture in `userland/cap_test`, assembles a separate ISO
(`grub-test.cfg`, kept apart from the production `grub.cfg`), boots it
headlessly in QEMU against a freshly generated FAT32 test image
(`make test-fat32-image`, built from the small tracked files in
`testdata/`), and checks that every fixture's task exited with code 0 and
that no unexpected interrupt vectors fired. See `run_tests.sh` for exactly
what's checked, and `userland/cap_test/README.md` for what each fixture
proves.

This is also what CI (`.github/workflows/ci.yml`) runs on every push and
pull request against `main`.

`make test` also runs `make test-keyboard`: a third kernel build
(`--features keyboard_test`, `grub-keytest.cfg`) that boots just
`cap_test`'s `console_input_test` fixture in its own isolated QEMU
invocation with a monitor socket, and drives it with *real* PS/2
keystrokes via QEMU's monitor `sendkey` command rather than a synthetic
in-process byte -- see `run_console_input_test.sh` and
`userland/cap_test/README.md` for how synchronization works.

## Releases

Publishing a GitHub release (`.github/workflows/release.yml`, triggered on
`release: published`, not on tag push -- a tag push happens before release
notes are finalized) builds the production ISO from the tagged commit via
`make iso` and attaches it to the release as `pcern-<tag>-i386.iso`. This
only applies to releases published from now on; it does not retroactively
attach an ISO to earlier releases.

## Repository layout

```
src/                        the kernel (ring 0)
userland/                   ring-3 services and shared bindings (see above)
testdata/                   small fixed input files for the FAT32 test image
grub.cfg                    production boot config
grub-test.cfg               test-harness boot config (make test only)
grub-keytest.cfg            keyboard-input test boot config (make test-keyboard only)
run_tests.sh                the test harness's pass/fail checker
run_console_input_test.sh   the keyboard-input test's pass/fail checker
Makefile                    build orchestration -- see it for every target
CLAUDE.md                   development process and design history
CHANGELOG.md                release history (Keep a Changelog format)
```

## Status

pCern is a research/learning project, not a production OS. Its FAT32
support is read-only, its storage driver only handles a single client at a
time, and there's no networking, no SMP, no persistence beyond what's
described above. See [CHANGELOG.md](CHANGELOG.md) for what's actually been
built so far.

## License

MIT -- see [LICENSE](LICENSE).
