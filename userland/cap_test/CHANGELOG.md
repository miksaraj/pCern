# Changelog

All notable changes to `cap_test` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

- `task_a`/`task_b` (Checkpoint F): capability derivation, transfer, and
  revocation-cascades-across-address-spaces regression fixture.
- `mem_test_a`/`mem_test_b` (Checkpoint G): shared-memory grant regression
  fixture.
- `storage_client_test` (Checkpoint I): direct `storage_ata` protocol
  client, asserting the FAT32 boot-sector signature.
- `fs_client_test` (Checkpoint J/K): direct `fs_fat32` protocol client,
  covering both a single-sector file and a multi-cluster file (exercising
  FAT chain-walking specifically).
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
