# Changelog

All notable changes to `fs_fat32` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-03

### Added

- Write support (Phase 7, Checkpoint Q): overwrite, growth past a file's
  current cluster span (free-cluster allocation + FAT chain extension),
  and brand-new file creation, all through one `FS_OP_WRITE` op
  (mirroring `FS_OP_READ`'s shape/partial-transfer contract) plus a
  "create if missing" flag folded into `FS_OP_OPEN_NAME2`'s
  previously-unused `w2`. Every FAT32 entry update now mirrors both FAT
  copies (unlike the read side, which deliberately only ever consults the
  first); newly allocated file-data clusters are left unzeroed (bounded
  by the directory entry's own size field), but newly allocated
  root-directory clusters are zero-filled, since directory-walking scans
  raw bytes with no separate size field to bound it.

## [0.1.0] - 2026-07-02

Initial release (Checkpoint J).

### Added

- Read-only FAT32 support: BPB parsing, FAT cluster-chain walking
  (including the root directory, which is itself a cluster chain), 8.3
  name matching in the root directory.
- Registers as `"fs"` with the name service, reads sectors through
  `storage_ata`.
- Serves `FS_OP_SET_BUFFER`/`SET_REPLY`/`OPEN_NAME1`/`OPEN_NAME2`/`READ`
  over a shared-memory grant, mirroring `storage_ata`'s own protocol
  shape.
- Falls back to staying alive but unregistered (rather than exiting) if
  no usable disk is behind `storage_ata`.

### Known limitations

- Root-directory files only -- no subdirectory traversal.
- 8.3 names only; long-filename and volume-label entries are skipped.
- One client and one open file at a time.
