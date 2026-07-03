# Changelog

All notable changes to **ZephyrLite** -- the OS as a whole, not any single
crate inside it -- are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
but the versioning is not SemVer: see [Versioning](#versioning) below. Every
kernel and userland crate still keeps its own SemVer version and its own
changelog (`kernel/CHANGELOG.md`, `userland/<name>/CHANGELOG.md`) -- those
track a single component's API/protocol/ABI stability. This file tracks
"what can a user of the ZephyrLite ISO actually do that they couldn't
before", which usually spans several crates at once and doesn't move in
lockstep with any one of them.

## Versioning

ZephyrLite releases are versioned `YY.MM[-{alpha|beta}].N` or
`YY.MM-rcN`, not SemVer -- there is no meaningful "breaking change" axis at
the OS level yet (there's one interactive user, one boot configuration, no
external API), and this project ships multiple releases in a single day
during this early rapid-development phase. Ubuntu-style `YY.MM` gives every
release an immediate, human-readable sense of *when* without implying
anything about compatibility; the trailing `.N` (or `-alpha.N`/`-beta.N`/
`-rcN` for a release still being shaken out before it's called done) covers
the "more than one release this month" case SemVer's own patch digit isn't
quite shaped for here, since nothing about these releases is a "patch" to a
previous minor/major. A release's tag is exactly its version string (no
leading `v`) -- see [CLAUDE.md](CLAUDE.md#versioning-zephyrlite-releases-vs-every-crates-own-semver)
for the full rationale.

This scheme is orthogonal to every crate's own SemVer: a ZephyrLite release
bumping to `26.08.1` doesn't imply anything moved in `pcern`'s or any
userland crate's own version, and a crate bumping its own SemVer doesn't by
itself justify a new ZephyrLite release.

## [Unreleased]

## [26.07-alpha.1] - 2026-07-03

The first release under the ZephyrLite name -- and the first release of
any kind to include Phase 7's work (read, write, and edit text files),
since the 0.4.0 kernel/userland set that phase produced was never itself
tagged/released (the last actual release was `v0.3.0`). Versioned `alpha`
rather than a plain `26.07.1`: a first cut that bundles a new feature set
with a simultaneous rename/restructuring is exactly the "still being
shaken out" case the scheme's `-alpha`/`-beta` suffixes exist for, not a
release that's already been through its own settled cycle.

### Added

- Everything Phase 7 (0.4.0) added, now actually reaching a release for
  the first time: real ATA/IDE write support in `storage_ata`; full
  `fs_fat32` write support (overwrite, growth, brand-new file creation);
  a raw single-keystroke console input mode with Ctrl/extended-key
  decoding; and `shell`'s `edit <file>` full-screen text editor built on
  top of all three. See `kernel/CHANGELOG.md`'s `[0.4.0]` entry and each
  touched crate's own `CHANGELOG.md` for the full detail -- this file
  covers the user-visible summary, not the per-crate mechanics.

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

No kernel or userland crate's own version changed to produce this
release -- every crate keeps the version it reached in Phase 7 (0.4.0),
unaffected by both the restructuring above and by finally reaching a
ZephyrLite release. See `kernel/CHANGELOG.md` for the kernel's version
history through 0.4.0, prior to this split.
