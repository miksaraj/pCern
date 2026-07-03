# Changelog

All notable changes to `cap_test` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-07-04

### Added

- `reboot_test`: exercises the new `SYS_REBOOT` syscall -- prints a
  marker directly to serial (the same technique `raw_input_test` uses),
  then triggers a real reset via a `RebootControl` capability hand-wired
  at CSlot 3. Its own standalone `reboot_test` kernel build/boot config,
  same reasoning as `raw_input_test`'s: it deliberately resets the whole
  machine, which nothing else can run alongside. See
  `run_reboot_test.sh`, which checks the marker reached serial *and* that
  QEMU (booted with `-no-reboot`) exited on its own rather than hanging --
  there's no exit code to check, since the fixture never gets to call
  `exit`.

## [0.3.0] - 2026-07-03

### Added

- `storage_client_test` extended to write a known pattern to the last
  LBA of the test image (far past anything `test-fat32-image` allocates)
  and read it back, proving `storage_ata`'s new write path round-trips --
  still run standalone (temporarily wired in for verification, then
  reverted), same as before.
- `fs_client_test` extended to create a new file, write enough to force
  a FAT chain-extension, overwrite a middle byte range, and read the
  whole thing back -- reusing its existing `fs_fat32` connection rather
  than a separate fixture, since `fs_fat32` only supports one client
  connection at a time.
- `raw_input_test`: exercises `console_server`'s raw single-keystroke
  mode -- a plain key, an extended (arrow) key, and a Ctrl-chord, each
  via real `sendkey` injection -- in its own standalone `raw_input_test`
  kernel build/boot config, same reasoning as `console_input_test`'s.
- `editor_input_test`: drives `libpcern::editor::Editor` directly (the
  exact type `shell`'s `edit` command uses) through a
  scripted real-keystroke edit session (type, navigate, insert,
  backspace, save with Ctrl-S), then reopens and reads the file back via
  `fs_fat32`'s normal read path to confirm the save reached disk. Its own
  standalone `editor_test` kernel build/boot config (needs the shared
  FAT32 test image attached, unlike `console_input_test`/
  `raw_input_test`).
- `mem_test_b` extended to cover two previously-untested denial paths a
  code review flagged: `SYS_MAP_MEMORY` with an invalid capability slot,
  `SYS_MAP_MEMORY` with a legitimate `MemoryGrant` but a `virt_addr`
  reaching the kernel's own higher half, and `SYS_REGISTER_IRQ` with an
  invalid slot -- all three must now be (and are) rejected. The second
  case is a direct regression test for a real privilege-escalation bug
  this same review found and the kernel fixed (see its CHANGELOG).

## [0.2.0] - 2026-07-03

### Added

- `console_input_test`: exercises `console_server`'s new line-input
  protocol against *real* PS/2 keystrokes injected via QEMU's monitor
  `sendkey` command, synchronized on a serial readiness marker. Only
  ever wired into the new standalone `keyboard_test` kernel build/boot
  config (see `run_console_input_test.sh`, `make test-keyboard`) --
  never the shared `iso-test` build every other fixture here runs under,
  since it would simply hang that harness waiting for keystrokes that
  never arrive.
- `loaded_program`: the tiniest possible ring-3 program (exits with a
  single distinctive code), dropped onto the test FAT32 image as
  `LOADED.BIN` rather than run as a multiboot module -- `fs_client_test`
  reads it via the real `fs_fat32` protocol and spawns it with the new
  `SYS_SPAWN_FROM_MEMORY` syscall, confirming (via that distinctive exit
  code) that the loaded program actually executed, not just that the
  syscall returned a task id.

## [0.1.0] - 2026-07-02

Initial release.

### Added

- `task_a`/`task_b`: capability derivation, transfer, and
  revocation-cascades-across-address-spaces regression fixture.
- `mem_test_a`/`mem_test_b`: shared-memory grant regression fixture.
- `storage_client_test`: direct `storage_ata` protocol client, asserting
  the FAT32 boot-sector signature.
- `fs_client_test`: direct `fs_fat32` protocol client, covering both a
  single-sector file and a multi-cluster file (exercising FAT
  chain-walking specifically).
- An automated harness wiring every fixture above (except
  `storage_client_test`, see the README) into a dedicated test-only kernel
  build and boot config, checked by `run_tests.sh` (see `make test` at the
  repo root).

### Fixed

- `task_a`/`task_b` and `mem_test_a`/`mem_test_b` originally reused their
  own inbox for both a one-shot name-service lookup reply and their
  peer-to-peer protocol messages; a peer's message could race the lookup
  reply and be consumed by the wrong `recv()` call. Fixed by giving each a
  dedicated endpoint for the lookup, separate from the inbox used for the
  peer protocol.
