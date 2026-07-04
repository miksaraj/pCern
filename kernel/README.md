# pCern

pCern is a small x86 (i686) nanokernel, written in Rust, built as a
learning/research project. The kernel itself does almost nothing: it brings
up paging, a scheduler, and a capability-based IPC mechanism, then gets out
of the way. Everything a traditional monolithic kernel would do in ring 0 --
the console/keyboard driver, a name service, a storage driver, a filesystem
-- runs as an ordinary ring-3 task, talking to the kernel only through a
handful of syscalls and to each other only through capabilities it mediates.

pCern is the kernel underneath **ZephyrLite**, the OS built on top of it in
this same repository -- see the [repo root README](../README.md) for the OS
as a whole (what it's for, how the pieces fit together, release/versioning)
and [../CLAUDE.md](../CLAUDE.md) for the story of *why* this kernel is built
the way it is.

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
- A capability-gated reboot syscall (`SYS_REBOOT`), pulsing the 8042
  keyboard controller's reset line -- the one piece of infrastructure a
  future in-place update mechanism needs beyond a writable boot disk
  (see the root README's `make disk`).
- A minimal PCI configuration-space enumerator, used at boot to discover
  a NIC's I/O-port range and interrupt line and hand both to its driver
  the same way legacy hardware's fixed ports/IRQs are hand-wired --
  everything actually talking to PCI hardware still lives in userland,
  same as every other driver.

Nothing else. No filesystem, no block device driver, no window into
physical memory beyond what a capability specifically grants -- all of that
lives in `../userland/`, described in the root README.

## Prerequisites

You'll need:

- **Rust nightly** with the `rust-src` and `llvm-tools` components. A
  `rust-toolchain.toml` at the *repo root* pins the exact channel for the
  kernel and every userland crate alike; if you have `rustup` installed,
  running any `cargo`/`rustc` command anywhere in this repo will install it
  automatically.
- **grub-mkrescue** and **xorriso** -- build the bootable ISO
  (`grub-common` + `grub-pc-bin` on Debian/Ubuntu).
- **qemu-system-i386** -- to actually run it.

See the root README's [Prerequisites](../README.md#prerequisites) section
for the full list, including what userland needs (`mtools`, `nasm`).

## Building and running the kernel on its own

There's no supported `cargo run` for the `pcern` crate on its own -- the
kernel expects a set of multiboot modules (the userland binaries) to be
loaded alongside it, which only the root `Makefile`'s `make iso`/`make run`
arrange. From the repo root:

```sh
make iso   # builds this kernel + every default userland service, produces zephyrlite-i386.iso
make run   # builds (if needed) and boots it in QEMU with serial output on stdio
```

A bare `cd kernel && cargo build --release` will compile the kernel crate
by itself (useful for a quick `cargo check`/`clippy` pass) but produces an
ELF that's never booted directly -- always go through the root `Makefile`
to get something you can actually run.

## Directory layout

```
Cargo.toml, Cargo.lock      the pcern kernel crate
src/                        kernel source (ring 0)
.cargo/config.toml          build-std config + target selection
i686-pcern.json             custom bare-metal target spec
linker.ld                   higher-half linker script
grub.cfg                    production boot config (CD ISO)
grub-disk.cfg               installed-disk boot config, embedded into core.img (make disk)
grub-test.cfg               test-harness boot config (make test only)
grub-keytest.cfg            keyboard-input test boot config (make test-keyboard only)
grub-rawtest.cfg            raw-input test boot config (make test-raw-input only)
grub-editortest.cfg         editor test boot config (make test-editor only)
grub-reboottest.cfg         reboot-syscall test boot config (make test-reboot only)
grub-nictest.cfg            NIC-driver test boot config (make test-nic only)
grub-arptest.cfg            ARP/ICMP responder test boot config (make test-arp only)
CHANGELOG.md                this crate's release history (Keep a Changelog + SemVer)
```

## Status

See [../README.md](../README.md#status) and
[CHANGELOG.md](CHANGELOG.md) for what's actually been built so far.

## License

MIT -- see [../LICENSE](../LICENSE).
