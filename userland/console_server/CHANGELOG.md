# Changelog

All notable changes to `console_server` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-02

Initial release.

### Added

- Owns the keyboard and the VGA/ANSI text console, moved out of the
  kernel (Checkpoint D).
- `OP_PUTCHAR` byte-at-a-time IPC protocol for client output.
- ANSI/VT100 escape parsing: cursor movement, erase, and SGR (colors +
  bold).
- Registers as `"console"` with the name service (Checkpoint H).
- VGA and keyboard access via capability (`MemoryGrant`/`IrqControl`)
  instead of a hardcoded allowlist (Checkpoint G).
