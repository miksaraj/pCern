# Changelog

All notable changes to `shell` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-03

### Added

- `edit <file>`: a full-screen text editor (Phase 7, Checkpoint S) --
  opens or creates the file, loads any existing content into a 16-page
  (64 KiB) buffer, switches the console to raw single-keystroke mode
  (Checkpoint R), and supports arrow/Home/End/Delete/Backspace/insert
  editing with a live redraw. Ctrl-S saves via `fs_fat32`'s new write
  support (Checkpoint Q) and returns to the prompt; Ctrl-Q discards.
  Switches to raw mode as the very first thing the command does, before
  any setup work, so keystrokes typed the instant `edit` completes can't
  land while the connection is still in line mode and be silently
  absorbed by its echo/accumulate path instead of reaching the editor.
  The actual editor logic lives in `libpcern::editor::Editor`, shared
  with a `cap_test` regression fixture.

## [0.1.0] - 2026-07-03

Initial release.

### Added

- Reads a line at a time via `console_server`'s input protocol
  (Checkpoint L) and parses it as `<command> <argument>`.
- `read <file>`: opens and prints a file's contents via `fs_fat32`.
- `run <file>`: loads and runs a file (capped at one page) via the new
  `SYS_SPAWN_FROM_MEMORY` syscall (Checkpoint M).
- Two endpoints, not one: a dedicated inbox for the synchronous
  name-service/`fs_fat32` request/reply round trips, and a separate one
  for `console_server`'s asynchronous "line ready" notifications -- see
  [CLAUDE.md](../../CLAUDE.md)'s note on why one inbox isn't
  automatically safe for two roles.
