# Changelog

All notable changes to `console_server` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-03

### Added

- A shared-memory buffered line-input protocol (`CONSOLE_OP_SET_BUFFER`/
  `SET_READER`/`READ_LINE`), mirroring `storage_ata`'s connect shape: a
  reader hands over a `mem_alloc`'d page and its own dedicated endpoint,
  then requests one typed line at a time. Every keystroke is still
  echoed to the screen unconditionally; once armed, bytes are also
  accumulated into the reader's page until Enter, with backspace tracked
  against that accumulator's own length and excess bytes past
  `CONSOLE_LINE_MAX` dropped rather than overflowed. Only one reader is
  supported at a time -- see the crate's doc comment for why a
  misbehaving reader could, in principle, block the whole console (a
  known limitation, not yet needed against an untrusted client).

### Security

- `CONSOLE_OP_SET_BUFFER`/`SET_READER`/`READ_LINE` initially had no
  check that a message came from the task that owned the current reader
  connection -- any task (including one with no privilege beyond the
  universal name-service auto-grant) could re-point the connection at
  itself and receive every keystroke typed afterward instead of the
  legitimate reader. Fixed by latching the kernel-attested sender id of
  the first successful `SET_BUFFER` as the connection's owner and
  ignoring these three ops from any other sender.

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
