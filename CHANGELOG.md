# Changelog

All notable changes to **ZephyrLite** -- the OS as a whole, not any single
crate inside it -- are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
but the versioning is not SemVer: see [Versioning](#versioning) below. Every
kernel and userland crate still keeps its own SemVer version and its own
changelog (`kernel/CHANGELOG.md`, `userland/<name>/CHANGELOG.md`) -- those
track a single component's own API/protocol/ABI stability. This file tracks
what a user of the ZephyrLite ISO can actually do that they couldn't
before, which usually spans several components at once.

## Versioning

ZephyrLite releases are versioned `YY.MM[-{alpha|beta}].N` or
`YY.MM-rcN`, not SemVer -- there is no meaningful "breaking change" axis at
the OS level yet (there's one interactive user, one boot configuration, no
external API), and this project ships multiple releases in a single day.
Ubuntu-style `YY.MM` gives every release an immediate, human-readable sense
of *when* without implying anything about compatibility; the trailing `.N`
covers same-month (even same-day) releases without contorting SemVer's
patch digit into meaning something it doesn't elsewhere. A release's tag is
exactly its version string (no leading `v`).

Every release carries `-alpha`, `-beta`, or `-rc` until ZephyrLite is
declared a stable release -- and that's a long way off: today it's
single-user, single-client-per-driver, has no networking or persistence
guarantees beyond a plain FAT32 write path, and no SMP. Expect the alpha/
beta/rc suffix on releases for a good while yet.

This scheme is orthogonal to every crate's own SemVer: a ZephyrLite release
bumping to `26.08.1` doesn't imply anything moved in `pcern`'s or any
userland crate's own version, and a crate bumping its own SemVer doesn't by
itself justify a new ZephyrLite release. See
[CLAUDE.md](CLAUDE.md#versioning-zephyrlite-releases-vs-every-crates-own-semver)
for the full rationale behind keeping these two axes separate.

## [Unreleased]

## [26.07-alpha.2] - 2026-07-04

### Added

- ZephyrLite now installs to, and boots from, its own writable FAT32 disk
  (`make disk` builds it; `make run-disk` boots it) instead of only ever
  running from a read-only CD ISO -- the foundation an in-place update
  mechanism needs, since a file written to the old CD-only setup's
  separate data disk was never something GRUB could load as a boot
  module. The kernel, every default userland service, and GRUB's own
  bootstrap files now live together on one real, partitioned disk image;
  `fs_fat32` gained the ability to find a FAT32 volume inside an MBR
  partition table (not just at the very start of the disk) to support
  this, while staying fully compatible with every existing unpartitioned
  disk/test image.
- A new `SYS_REBOOT` syscall resets the machine (via the 8042 keyboard
  controller's reset line), gated by a new capability so only a task
  explicitly handed one can trigger it -- the other piece an in-place
  update mechanism needs, to apply an update by simply restarting into
  it. Not yet wired up to anything user-facing; that's future work.

## [26.07-alpha.1] - 2026-07-03

The first ZephyrLite release.

### Added

- Read, write, and edit text files, with a real full-screen editor
  (arrow-key/Home/End/Delete/Backspace navigation, Ctrl-S to save, Ctrl-Q
  to discard) -- `shell`'s new `edit <file>` command, on top of new write
  support in the FAT32 filesystem server (overwrite, growth, and brand-new
  file creation) and in the ATA/IDE disk driver, and a new raw
  single-keystroke input mode in the console driver.

### Security

- A ring-3 task holding nothing more than a memory grant it could obtain
  for free could reach virtual addresses reserved for the kernel and gain
  ordinary read/write access to physical memory well beyond what it was
  ever granted, up to and including all of it -- a complete bypass of the
  capability model this OS is built around. Fixed at the syscall boundary
  (an out-of-range address is now rejected outright) with a second,
  independent check inside the kernel's own page-mapping code as a
  backstop. See `kernel/CHANGELOG.md` for the technical detail.

### Fixed

- Saving an edited file could leave stale old content on disk past the
  end of what was actually saved (most noticeably: deleting a file's
  text down to nothing and saving didn't actually empty it), and a file
  larger than the editor's 64 KiB limit was truncated on load with no
  warning. Both are now handled correctly and, in the second case,
  reported to the user.
- Repeatedly using `edit` in one session leaked memory (no way to free
  what a previous `edit` had allocated); it's now reused instead.
- Switching the console between typed-line and full-screen-editor input
  modes could leave keystrokes queued from one `edit` session bleeding
  into the next one.

### Changed

- The kernel moved from the repo root into `kernel/` (its own `Cargo.toml`,
  `src/`, target spec, linker script, and every `grub*.cfg`), freeing the
  repo root for the OS/distro level this file and the root README now
  occupy. `rust-toolchain.toml` stays at the repo root, since every crate
  in this monorepo -- kernel and userland alike -- has always relied on
  it via `rustup`'s upward directory search, not just the kernel.
- `userland/` reorganized to make the driver/service/program distinction
  visible in the directory tree itself, instead of only in prose:
  `userland/drivers/{console_server,storage_ata}` (own hardware
  capabilities -- `MemoryGrant`/`IrqControl`/port I/O),
  `userland/services/{nameservice,fs_fat32}` (no hardware capabilities of
  their own), `userland/bin/shell` (the one user-facing program). No code
  was split -- `console_server` still bundles VGA/keyboard I/O with its
  ANSI/line-discipline logic in one crate, the same way a Unix tty driver
  legitimately combines both; see
  [CLAUDE.md](CLAUDE.md#driver-vs-utility-taxonomy-in-userland) for the
  reasoning. `userland/libpcern` (shared library) and `userland/cap_test`
  (regression fixtures, never booted normally) were left where they were --
  neither is a driver, a service, or a user-facing program.
- The production ISO and its release pipeline (`.github/workflows/release.yml`)
  now build and publish `zephyrlite-<tag>-i386.iso` instead of
  `pcern-<tag>-i386.iso`; the kernel binary inside it is still named
  `pcern.elf` and the GRUB menu entry reads "ZephyrLite OS". Internal
  test-harness ISOs (`iso-test`/`iso-keytest`/`iso-rawtest`/
  `iso-editortest`) keep their existing `pcern-*` names -- they're build
  artifacts, never released.
- Adopted the versioning scheme documented above for OS-level releases,
  replacing the kernel's own SemVer as what a GitHub release tag names.

No kernel or userland crate's own version changed to produce this release
-- see `kernel/CHANGELOG.md`'s `[0.4.0]` entry and each touched crate's own
`CHANGELOG.md` for the underlying component-level changes.
