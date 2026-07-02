# cap_test

Regression fixtures covering the capability/IPC mechanisms and the
storage/filesystem protocols end to end. Not part of a normal boot --
built on demand (`make cap_test`) and run through the automated harness
(`make test`, see the root README's [Testing](../../README.md#testing)
section) rather than the production `grub.cfg`.

Every fixture communicates pass/fail through its **process exit code**
(0 = pass, 1 = fail) -- that's what `run_tests.sh` actually checks, not
console text, since multiple fixtures printing through `console_server`
concurrently interleave byte-for-byte and can't be read reliably as a
signal. Console output (`print(console_slot, ...)`) is there to help a
human debug a failure, not to decide one.

## Fixtures

Each pair below is spawned together by `src/main.rs`'s
`test_harness_spawn` in the kernel, with a hand-wired capability to its
specific partner at CSlot 3 (there's no name to look a specific test
pairing up under -- see [CLAUDE.md](../../CLAUDE.md)'s CSlot convention).

- **`task_a` / `task_b`** (built as `cap_test_a`/`cap_test_b`) -- `task_a`
  mints an endpoint, derives a badged copy, and transfers it to `task_b`
  over IPC. Once `task_b` confirms receipt, `task_a` revokes the badged
  copy and tells `task_b` to try using its transferred copy anyway --
  which must now fail. Proves capability derivation, transfer across an
  address-space boundary, and that revocation actually cascades to a
  capability that already crossed into another task.
- **`mem_test_a` / `mem_test_b`** -- `mem_test_a` allocates a fresh page
  (`SYS_MEM_ALLOC`), writes a known pattern, and transfers the
  `MemoryGrant` to `mem_test_b`, which maps the *same* physical page into
  its own (separate) address space and verifies the pattern reads back
  correctly before ever sending anything itself. Proves the shared-memory
  primitive `storage_ata`/`fs_fat32` build on.
- **`storage_client_test`** -- exercises `storage_ata`'s real protocol
  (name lookup, `mem_alloc`, `storage_connect`, `storage_read_block`)
  directly, asserting the FAT32 boot-sector signature (`0x55 0xAA` at the
  end of LBA 0) so it can run against the same test image `fs_client_test`
  uses without needing a second disk. **Not spawned by `make test`** --
  `storage_ata` only supports one client at a time, and `fs_fat32` is
  already a standing one; run this fixture standalone (temporarily wire it
  into `main.rs`/`grub.cfg` in place of `fs_fat32`, the way earlier
  checkpoints verified it) if you need to test `storage_ata` in isolation.
- **`fs_client_test`** -- exercises `fs_fat32`'s real protocol end to end:
  looks up `"fs"`, opens a small single-sector file and a larger
  multi-cluster one from the generated test image (`/testdata`), and
  checksums the multi-cluster read to prove the FAT chain-walking path
  specifically (the small file fits in one cluster and never exercises
  it).

## Adding a new fixture

- Give it its own `src/bin/<name>.rs`, add the matching `objcopy` line to
  the Makefile's `cap_test` target, and (if it should run under `make
  test`) a spawn call in `test_harness_spawn` plus a module line in
  `grub-test.cfg`.
- Follow the CSlot convention: CSlot 1 is always the name-service
  capability, CSlot 2 is a fixture's own inbox if it needs one.
- If a fixture both looks something up by name *and* runs its own
  peer-to-peer protocol on the same inbox, give the lookup its own
  dedicated endpoint (`libpcern::endpoint_create()`) rather than reusing
  the inbox for both -- see [CLAUDE.md](../../CLAUDE.md)'s note on why one
  inbox isn't automatically safe for two roles; this bit two of these
  fixtures for real before being fixed.
- Assert your fixture's actual pass/fail condition and communicate it
  through `libpcern::exit(0)`/`exit(1)`, then add a `check_exit` line for
  its task ID to `run_tests.sh`.
