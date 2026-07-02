# Development process and design notes

This file is for whoever (human or AI) picks up development on pCern next.
It documents *how* this project has been built and the judgment calls
behind its design, not just what the code does -- that's what the READMEs
and code comments are for.

## The checkpoint methodology

Every phase of this project (privilege separation, paging, the syscall ABI,
userspace drivers, the capability model, the name service, storage,
filesystem) was built as a sequence of small, independently verified
checkpoints, not as one large change landed at once. Each checkpoint:

1. Implements one coherent piece (e.g. "capability derivation via IPC
   transfer + badging + revocation", not "capabilities" as one giant step).
2. Gets a full clean rebuild (`make clean && make iso`) and a headless QEMU
   boot, every time -- never trust a build that reused stale artifacts.
3. Is checked against two logs: the serial console output, and a `-d int
   -D <file>` interrupt trace. The second one matters as much as the
   first -- a checkpoint isn't done until the interrupt log shows *only*
   the interrupt vectors you expect (the syscall gate, the timer, the IRQs
   you're deliberately testing) and nothing else. An unexpected `#GP`/`#PF`
   there is a real bug even if the serial log looks fine.
4. Re-verifies every previous checkpoint's behavior still holds (the
   existing services still boot, existing test fixtures still pass) --
   regressions get caught immediately, not at the end of a phase.

### Exit codes over console text

Early on, verification leaned on reading console text in a screendump.
That's necessary when a fixture's whole point is proving specific bytes
came back correctly (e.g. a filesystem read matching known file contents),
but it is *not* sufficient on its own: a userland program that ignores a
failed syscall and keeps going will still print something and still exit
0. One real bug shipped for an entire phase because of exactly this --
`ping.asm`/`pong.asm` silently completed only one round-trip instead of
five, invisible because the exit code was 0 either way (see the 0.2.0
CHANGELOG entry). The fix going forward: test fixtures assert their own
pass/fail condition in code and communicate it through the process exit
code (0 = pass, 1 = fail), and the automated harness (`make test`,
`run_tests.sh`) checks *that*, not console text. Console output is for a
human debugging a failure, not for the pass/fail decision itself -- which
also sidesteps a real limitation of this system: multiple tasks printing
through the console server at once interleave byte-for-byte, so console
text is unreliable as a signal the moment more than one thing is running.

### Temporary verification wiring

Multiboot module indices and task IDs are fixed by `main.rs`'s spawn order
and `grub.cfg`'s module list, both compiled/fixed ahead of time -- there's
no dynamic module loading. Historically, verifying a new fixture in
isolation meant temporarily editing `main.rs`/`grub.cfg`/`Makefile` to spawn
it, checking it, then reverting those edits before committing (backups
kept in a scratch directory in the meantime, never in the repo). `make
test` replaced most of that need for `userland/cap_test`'s fixtures with a
standing, permanent (if separate) boot configuration -- see
`src/main.rs`'s `test_harness_spawn`, `grub-test.cfg`, and `run_tests.sh`.
Prefer extending that harness over hand-editing the production boot
sequence when adding a new regression test.

## Design decisions worth knowing about

### Capabilities are the actual security boundary

The stated goal from early on was that security be *enforced*, not just
organized -- i.e. the kernel should refuse to let a task reach an IPC
endpoint, a physical memory range, or an IRQ unless it already holds an
unforgeable capability naming it, not "a few places trusted code remembers
to check". Endpoints are addressed by capability slot, not raw task ID;
`MemoryGrant`/`IrqControl` capabilities replaced a single `is_driver` bool
plus a hardcoded MMIO allowlist. Port I/O is the one exception -- it's
still gated by the pre-existing `allowed_ports`/TSS-bitmap mechanism set at
spawn time, not a capability, since nothing in this project's scope needed
a *transferable* capability for raw port access.

A few places deliberately narrow "full seL4-style capabilities" down to
what this project's actual protocols need, trading some textbook purity
for meaningfully less risk:

- **Revocation** splits into cheap "rights revocation" (a derivation tree,
  walked and flagged, checked lazily on every use -- this part is fully
  general) and "object teardown", which is *not* generalized: endpoints are
  simply retired when their owning task exits (`ipc::task_exited`). Nothing
  in this project's protocols needs more than that.
- **Name registration trust** uses the kernel-attested sender task ID IPC
  already exposes (unforgeable, free) plus a small compile-time allowlist,
  instead of a whole new capability kind and an introspection syscall just
  to let the name service check "is this the right kind of capability".
- **Memory grants** are capped at one page, since the physical frame
  allocator (`mm::frame`) only ever hands out single frames -- there's no
  contiguous multi-frame allocation, and nothing in this project's
  protocols (a 512-byte ATA sector comfortably fits in one page) needed
  one. If a future service needs a bigger shared buffer, that allocator
  needs to grow multi-frame support first.

### Wire protocols live inside a 3-word IPC budget

`SYS_SEND`/`SYS_RECV` carry a capability slot (destination/endpoint) plus
exactly three message words plus one optional capability transfer per
call -- there's no variable-length message. Every protocol in this project
(console `OP_PUTCHAR`, name-service register/lookup, storage/filesystem
block reads) was designed to fit that budget, sometimes by splitting a
logically-one operation into two or three messages (e.g. `fs_fat32`'s
`OPEN_NAME1`/`OPEN_NAME2` splits an 11-byte 8.3 filename across two
messages since it doesn't fit in one). If you're adding a new protocol,
check it against this budget before designing message contents around
something wider.

### One inbox is not automatically safe for two roles

A capability's owner receiving unrelated kinds of messages on the *same*
inbox is a real hazard if message arrival order isn't guaranteed -- which
it never is, in general. `fs_fat32` uses a separate endpoint for its
storage-client role versus the inbox it serves its own filesystem clients
on, specifically to avoid this. A real bug caught during this project's own
test-harness work: two `cap_test` fixtures reused their single inbox for
both a one-shot name-service lookup reply and an ongoing peer-to-peer
protocol; a peer's message could race the name service's reply and get
consumed by the wrong `recv()` call, since the two happened to share a
"success" code value by coincidence. The fix in every case is the same:
give any task that plays more than one such role a dedicated endpoint per
role.

### CSlot numbering convention

Every task spawned via `loader::spawn_from_module` gets CSlot 1 = a
capability to the name service (auto-granted, unconditional, the one piece
of discovery infrastructure nothing has to ask for individually -- see
`loader.rs`). By convention, CSlot 2 = "my own inbox" for tasks that need
one, and CSlot 3+ is whatever else that specific task needs, hand-wired by
`main.rs` at spawn time for anything not discoverable by name (hardware
capabilities, a fixed test-pairing's peer). If you're adding a new
service, follow this convention rather than inventing a new one -- every
existing service and test fixture assumes it.

### CI: pin GitHub Actions to a commit SHA, never a tag

Every `uses:` line in `.github/workflows/*.yml` pins a full 40-character
commit SHA, not a version tag (`@v4`) or branch. Tags are mutable -- if an
action's repository is ever compromised, a moved tag would pull malicious
code into every workflow still pinned to it without the workflow file
itself changing at all. A SHA pin doesn't move. Add a trailing comment
noting which released version the SHA corresponds to (e.g.
`# v4.3.1`) so it stays human-readable, but the SHA is what's actually
trusted. Bumping an action's version means fetching the new tag's commit
SHA from the action's own repository and updating both the SHA and the
comment -- never just editing the version number.

## Where things live, and why one repo

pCern is a monorepo: the kernel (`src/`) and every userland service
(`userland/*`) live in one repository, each userland service as its own
Cargo crate with its own `Cargo.toml`/version/README/CHANGELOG. That's a
deliberate choice, not just where things happened to end up:

- The syscall ABI and IPC wire formats are still moving (three ABI-shaping
  phases so far), and userland code has to track the kernel's ABI exactly.
  Coordinating that across separate repositories, with separate release
  cadences and separate PRs for what's really one change, would add
  process overhead with no corresponding benefit at this project's current
  size and pace.
- Each userland component is already independently buildable (its own
  `Cargo.toml`, its own target JSON, its own linker script) and only
  depends on `libpcern` and the kernel's compiled ABI, not on each other's
  internals. If a component ever needs an independent release cadence, or
  gains an external contributor who shouldn't need the whole kernel's
  history, splitting it out along that existing crate boundary is a
  mechanical `git subtree split`, not a redesign.

Revisit this if either of those conditions actually shows up -- don't
split preemptively.

## Where to look next

- [README.md](README.md) -- what the project is, how to build/run/test it.
- [CHANGELOG.md](CHANGELOG.md) -- what's shipped so far, in Keep a Changelog
  format.
- `userland/<name>/README.md` and `CHANGELOG.md` -- per-service protocol
  and design notes, and per-service version history.
- Code comments at the top of each module/file -- these carry the
  checkpoint-by-checkpoint "why", not just the "what"; they were kept
  deliberately dense rather than moved into separate docs that would drift
  out of sync with the code.
