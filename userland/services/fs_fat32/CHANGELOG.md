# Changelog

All notable changes to `fs_fat32` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-07-04

### Added

- `find_fat32_base`: the FAT32 volume this task reads and writes may now
  start at LBA 0 directly (the original "superfloppy" layout, still what
  `make test-fat32-image` builds) *or* wherever an MBR partition table's
  first FAT32 partition (type `0x0B`/`0x0C`) begins -- distinguished by
  FAT32's own `BS_FilSysType` field, not the 0x55AA boot signature both
  an MBR and a bare FAT32 boot sector carry at the identical offset.
  Needed for ZephyrLite's new installed boot disk (`make disk` at the
  repo root), which requires a real partition table so GRUB's own
  `i386-pc` BIOS install has a gap to embed its `core.img` in -- a bare
  FAT32 filesystem has none. Fully backward compatible: every existing
  unpartitioned disk/test image resolves to the same base-0 behavior as
  before.

## [0.2.0] - 2026-07-03

### Added

- Write support: overwrite, growth past a file's current cluster span
  (free-cluster allocation + FAT chain extension),
  and brand-new file creation, all through one `FS_OP_WRITE` op
  (mirroring `FS_OP_READ`'s shape/partial-transfer contract) plus a
  "create if missing" flag folded into `FS_OP_OPEN_NAME2`'s
  previously-unused `w2`. Every FAT32 entry update now mirrors both FAT
  copies (unlike the read side, which deliberately only ever consults the
  first); newly allocated file-data clusters are left unzeroed (bounded
  by the directory entry's own size field), but newly allocated
  root-directory clusters are zero-filled, since directory-walking scans
  raw bytes with no separate size field to bound it.
- `FS_OP_TRUNCATE`/`truncate_file`: sets the currently open file's size
  explicitly, but only ever shrinks it -- see `libpcern`'s changelog for
  the client-side rationale. The only way a file's size decreases;
  `write_file` remains grow-or-overwrite-only.

### Fixed

- `write_file` accepted any `offset`, even one far past the file's current
  size -- allocating (but never zero-filling) every intervening cluster
  and publishing that whole gap as valid content the moment `file.size`
  grew to cover it, exposing whatever stale data already occupied those
  clusters. Now refuses (`0`) any write whose `offset` exceeds the current
  size; a legitimate caller only ever overwrites within the existing
  range or appends immediately at the end, so this costs nothing for any
  real usage while closing an information-disclosure path. As a side
  effect this also bounds the cost of a single write request: a request
  can no longer force allocation of an unbounded number of clusters in
  one call by naming an arbitrarily large offset.
- `write_file` only ever grew `file.size`, never shrank it -- overwriting
  an existing file with shorter content left the old, larger size on
  disk, so a later read exposed stale trailing bytes past what was
  actually (re)written. Shrinking is deliberately not inferred from a
  write's own coverage (see `FS_OP_TRUNCATE`'s doc comment for why); a
  caller doing a full-file rewrite now calls the new `FS_OP_TRUNCATE`
  after writing to declare the file's true final size.

## [0.1.0] - 2026-07-02

Initial release.

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
