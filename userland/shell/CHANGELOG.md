# Changelog

All notable changes to `shell` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-02

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
