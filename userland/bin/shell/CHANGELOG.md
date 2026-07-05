# Changelog

All notable changes to `shell` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-07-05

### Fixed

- The shell refused to start at all when `fs_fat32` hadn't registered
  `"fs"` with the name service -- which is the normal, documented outcome
  of booting with no FAT32 disk attached (e.g. `make run`'s plain
  `-cdrom`-only QEMU invocation, or a downloaded release ISO booted the
  same way), not a failure of anything. `read`/`edit`/`run <file>` do
  need `fs_fat32` and stay unavailable in that case, but `help` and the
  prompt itself don't, so hard-exiting on a missing `"fs"` made the
  entire shell inaccessible for no reason. The shell now starts
  regardless, prints a one-line notice that `fs`-dependent commands are
  unavailable, and only `read`/`edit`/`run` themselves report "no
  filesystem available" if actually invoked. (Issue #20)

## [0.2.0] - 2026-07-03

### Added

- `edit <file>`: a full-screen text editor -- opens or creates the file,
  loads any existing content into a 16-page (64 KiB) buffer, switches
  the console to raw single-keystroke mode, and supports arrow/Home/End/
  Delete/Backspace/insert editing with a live redraw. Ctrl-S saves via
  `fs_fat32`'s write support and returns to the prompt; Ctrl-Q discards.
  Switches to raw mode as the very first thing the command does, before
  any setup work, so keystrokes typed the instant `edit` completes can't
  land while the connection is still in line mode and be silently
  absorbed by its echo/accumulate path instead of reaching the editor.
  The actual editor logic lives in `libpcern::editor::Editor`, shared
  with a `cap_test` regression fixture.

### Fixed

- `edit <file>` allocated a fresh `libpcern::editor::Editor` (and its
  64 KiB backing buffer) on every invocation, permanently leaking it
  since this project has no syscall to free a `mem_alloc`'d page. The
  editor is now allocated exactly once at shell startup and reset
  (`Editor::reset()`) for each subsequent `edit` command instead.
- Saving never shrank the file to match the edited content -- deleting
  text (down to and including emptying the buffer entirely) and saving
  left the old, longer content's tail on disk, in the empty-buffer case
  because the save loop has nothing to write at all yet still reported
  "saved". `edit` now calls the new `fs_truncate` unconditionally after
  saving (even when nothing was written) to set the file's size to
  exactly what's now in the buffer.
- Loading a file larger than the editor's 64 KiB cap silently truncated
  it with no indication to the user; saving afterward would then persist
  only the truncated content with no warning. Now prints an explicit
  warning when a load is truncated.

## [0.1.0] - 2026-07-03

Initial release.

### Added

- Reads a line at a time via `console_server`'s input protocol and
  parses it as `<command> <argument>`.
- `read <file>`: opens and prints a file's contents via `fs_fat32`.
- `run <file>`: loads and runs a file (capped at one page) via the
  `SYS_SPAWN_FROM_MEMORY` syscall.
- Two endpoints, not one: a dedicated inbox for the synchronous
  name-service/`fs_fat32` request/reply round trips, and a separate one
  for `console_server`'s asynchronous "line ready" notifications -- see
  [CLAUDE.md](../../../CLAUDE.md)'s note on why one inbox isn't
  automatically safe for two roles.
