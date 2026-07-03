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
  it). Also loads and runs `LOADED.BIN` (see `loaded_program` below)
  through this same connection, exercising `SYS_SPAWN_FROM_MEMORY`
  end to end -- a *second* fixture connecting to `fs_fat32` concurrently
  would clobber this one's, since it only supports one client at a time.
  Phase 7, Checkpoint Q also exercises write support here, for the same
  single-client reason: creates a new file, writes enough to force a FAT
  chain-extension, overwrites a middle byte range, and reads the whole
  thing back byte-for-byte -- `run_tests.sh` additionally reads that file
  back out of the disk image directly via `mtools` after QEMU exits,
  independent of anything `fs_fat32` itself believes.
- **`loaded_program`** -- not spawned directly; the tiniest possible
  ring-3 program (it just calls `exit(42)`), objcopy'd and dropped onto
  the test FAT32 image as `LOADED.BIN` by `make test-fat32-image`.
  `fs_client_test` reads its bytes via the real filesystem protocol and
  spawns it with `SYS_SPAWN_FROM_MEMORY`; seeing task 11 exit with code
  `42` specifically (not the `-1` a crashing/faulted task would exit
  with) in `run_tests.sh` is what actually proves the loaded code ran,
  not just that the syscall returned a task id.
- **`console_input_test`** -- exercises `console_server`'s line-input
  protocol against *real* PS/2 keystrokes injected via QEMU's monitor
  `sendkey` command, not a synthetic in-process byte. Runs in its own
  standalone kernel build/boot config (`--features keyboard_test`,
  `grub-keytest.cfg`) rather than the shared `iso-test` every fixture
  above runs under, since it's the one fixture that blocks on real
  external input -- folded into `iso-test`, it would just hang that
  harness until its boot timeout. `run_console_input_test.sh` (`make
  test-keyboard`) boots it with its own QEMU monitor socket, waits for
  this fixture's own readiness marker on serial (a synchronization
  *gate*, not the pass/fail signal -- that's still the exit code) before
  calling `sendkey`, and checks the result the same way as every other
  fixture.
- **`raw_input_test`** (Phase 7, Checkpoint R) -- exercises
  `console_server`'s new raw single-keystroke mode
  (`CONSOLE_OP_SET_MODE`/`READ_KEY`) the same way `console_input_test`
  exercises line mode: real `sendkey` injection of a plain key, an
  extended (arrow) key, and a Ctrl-chord, asserting each decodes to the
  expected tagged value. Its own standalone build (`--features
  raw_input_test`, `grub-rawtest.cfg`, `make test-raw-input`) -- would
  otherwise race `console_input_test` for the single `reader_owner` role.
- **`editor_input_test`** (Checkpoint S) -- drives
  `libpcern::editor::Editor` directly (the exact type `shell`'s `edit`
  command uses, not a re-implementation) through a scripted real-keystroke
  edit session via `sendkey`: type a string, move the cursor with arrows,
  insert and backspace, save with Ctrl-S. Reopens and reads the file back
  through `fs_fat32`'s normal read path afterward, independent of
  anything the `Editor` itself still holds in memory, to confirm the save
  actually reached disk. Its own standalone build (`--features
  editor_test`, `grub-editortest.cfg`, `make test-editor`) -- needs the
  shared FAT32 test image attached, unlike `console_input_test`/
  `raw_input_test`, since it exercises real `fs_fat32` writes.

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
