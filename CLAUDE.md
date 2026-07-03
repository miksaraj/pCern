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
`kernel/src/main.rs`'s `test_harness_spawn`, `kernel/grub-test.cfg`, and
`run_tests.sh`. Prefer extending that harness over hand-editing the
production boot sequence when adding a new regression test.

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

ZephyrLite is a monorepo: the pCern kernel (`kernel/`) and every userland
service (`userland/*`) live in one repository, each userland service as its
own Cargo crate with its own `Cargo.toml`/version/README/CHANGELOG. That's a
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
split preemptively. Note that this is about *separate repositories*, not
directory layout within this one -- the `kernel/` subdirectory move
(below) doesn't touch this reasoning at all, since it's still one repo,
one build, one set of PRs.

### The kernel moved into `kernel/`, and why

Through Phase 6 (0.3.0), the kernel's `Cargo.toml`/`src/`/build config lived
directly at the repo root, and the repo as a whole was still identified as
"pCern" -- reasonable while the only thing worth naming was the kernel
itself. Phase 7 (0.4.0) made the OS genuinely usable (read, write, and edit
files with a real full-screen editor) -- past that point, the repo root is
the OS/distro's identity, not the kernel's, and "pCern" (the kernel) and
whatever the OS as a whole is called are two different things that
shouldn't fight over the same root directory. So: `kernel/` now holds
everything specific to building/booting the `pcern` kernel crate
(`Cargo.toml`, `Cargo.lock`, `src/`, `.cargo/config.toml`,
`i686-pcern.json`, `linker.ld`, every `grub*.cfg`), and the repo root holds
the OS-level `README.md`/`CHANGELOG.md`/`VERSION` -- this file (CLAUDE.md,
development history for the whole monorepo) stays at the root since it's
never been kernel-specific.

One file deliberately did **not** move: `rust-toolchain.toml` stays at the
repo root. It was never a kernel-specific file -- every userland crate has
always relied on `rustup`'s upward directory search finding the *root*
copy (none of them ship their own), so moving it into `kernel/` would have
silently broken every userland build's ability to find the pinned nightly
channel while leaving the kernel's own build looking fine. (This is exactly
what happened during the migration itself before it was caught: builds run
from inside a userland crate's directory fell back to whatever `stable`
toolchain happened to be `rustup`'s default, which can't process the
`-Zbuild-std` flag every crate here needs -- a build failure specific to
the reorg, not a real regression in any crate.) The lesson generalizes:
before moving a config file that governs a build, check whether anything
*outside* the thing being moved was silently relying on it living where it
was.

The `Makefile` stays at the repo root too (it already orchestrates kernel
+ userland together, `cd`-ing into whichever crate's directory it needs
per target) -- only its `KERNEL_DIR`/`KERNEL_BIN`/`CFG*` variables needed
updating to point at `kernel/`.

### Driver vs. utility taxonomy in userland

Once the OS was something a user could actually type into and expect
persistent results from (Phase 7), it became worth asking plainly which
`userland/` crates are drivers (own real hardware), which are services
(no hardware, reachable only by name), and which are just programs a user
runs -- rather than leaving that distinction implicit in each crate's own
README. The audit, and what came of it:

- **`console_server`** owns real hardware (VGA MMIO via a `MemoryGrant`,
  the keyboard IRQ via `IrqControl`) *and* implements the line-discipline/
  echo state machine and the ANSI/VT100 escape parser (`ansi.rs`) on top,
  in the same crate. This looks at first glance like it should split into
  a hardware piece and a protocol piece -- but a real Unix tty driver
  combines exactly these two responsibilities (raw device I/O plus line
  discipline plus terminal emulation) in one layer for a reason: the
  policy of "what counts as a completed line", "what does Ctrl-C do", or
  here, "what does a redraw's CUP/ED/EL sequence do to the actual VGA
  buffer" is tightly coupled to the hardware it's driving, and splitting
  it into a separate crate would mean two crates renegotiating a private
  protocol for no external consumer -- nothing else in this project (or
  plausibly ever) needs raw scancode access without also wanting a line
  discipline on top. **Decision: `console_server` is not code-split.** It
  moved into `userland/drivers/` as-is; classified as a driver because
  hardware ownership is what makes it privileged, even though most of its
  line count is protocol logic, not device I/O.
- **`storage_ata`** owns hardware (port I/O via the `allowed_ports`
  mechanism) and nothing else -- a pure driver, moved to
  `userland/drivers/storage_ata`.
- **`nameservice`** and **`fs_fat32`** hold no hardware capabilities at
  all -- `fs_fat32` reaches disk only by being `storage_ata`'s client over
  ordinary IPC, the same access any other task could request. Both moved
  to `userland/services/`.
- **`shell`** (including its `edit` full-screen-editor command) never
  touches a scancode or a port directly -- it only ever calls
  `console_read_key`/`console_read_line` and gets back an already-decoded
  value. It's unambiguously a user-facing program, not a driver or a
  headless service, so it moved to `userland/bin/shell` on its own.
- **`libpcern`** (a library, never a task -- no `_start`) and
  **`cap_test`** (regression fixtures, never part of a normal boot) are
  neither drivers, services, nor programs, so neither moved under
  `drivers/`/`services/`/`bin/` -- they stayed where they were.

If a future userland crate mixes hardware ownership with a large amount of
non-hardware logic the way `console_server` does, apply the same test
before splitting it: would the split produce two crates with an actual
outside consumer for the boundary between them, or just two crates that
still only ever talk to each other?

### Versioning: ZephyrLite releases vs. every crate's own SemVer

Before Phase 7's restructuring, one version number (the kernel's,
`Cargo.toml`'s `pcern` version) stood in for "the project's version",
because the kernel and the shippable thing were the same identity. Once
the repo root became the OS's own identity (ZephyrLite) rather than the
kernel's, that stopped making sense -- a kernel SemVer bump and a
user-visible OS release are different events that don't have to coincide,
and forcing them to would mean either bumping the kernel's SemVer for
changes that are pure userland (most of Phase 7) or bumping some "project"
number that isn't really any single crate's.

**The fix: two completely separate versioning axes.**

- Every kernel and userland crate keeps its own SemVer
  (`Cargo.toml`/`CHANGELOG.md` per crate, unchanged from how this project
  has always done it) -- this tracks that one component's own API/ABI/
  protocol stability.
- **ZephyrLite**, the OS as a whole, is versioned separately:
  `YY.MM[-{alpha|beta}].N` or `YY.MM-rcN` (e.g. `26.07.1`, `26.08-beta.2`,
  `26.09-rc1`), tracked in the root `VERSION` file and `CHANGELOG.md`, and
  is what a GitHub release's tag actually names. This is deliberately not
  SemVer: SemVer's whole point is signaling breaking-vs-compatible changes
  across a versioned *interface*, and there isn't one at the OS level yet
  (one interactive user, one boot configuration, nothing external
  integrates against "the OS" as an API). `YY.MM` (Ubuntu's own scheme)
  gives every release an immediately readable sense of *when*, which is
  what actually matters at this project's pace -- and the trailing
  `.N`/`-alpha.N`/`-beta.N`/`-rcN` covers same-month, even same-day,
  multiple releases (a real possibility during this project's current
  rapid-iteration phase) without contorting SemVer's patch digit into
  meaning "the Nth release today", which isn't what it means anywhere
  else.

A ZephyrLite release bumping (say `26.07.1` -> `26.07.2`) does **not**
imply any crate's own version changed, and a crate bumping its own SemVer
does not by itself justify a new ZephyrLite release -- the two are
tracked, decided, and bumped independently. When both are changing in the
same PR (e.g. this restructuring), say so explicitly in each affected
CHANGELOG rather than letting a reader infer a relationship that isn't
there.

## Where to look next

- [README.md](README.md) -- what the OS is, how to build/run/test it.
- [CHANGELOG.md](CHANGELOG.md) -- ZephyrLite's OS-level release history,
  in Keep a Changelog format (see its own Versioning section for the
  scheme).
- [kernel/README.md](kernel/README.md) and
  [kernel/CHANGELOG.md](kernel/CHANGELOG.md) -- the pCern kernel crate's
  own docs and SemVer version history.
- `userland/<name>/README.md` and `CHANGELOG.md` -- per-service protocol
  and design notes, and per-service version history.
- Code comments at the top of each module/file -- these carry the
  checkpoint-by-checkpoint "why", not just the "what"; they were kept
  deliberately dense rather than moved into separate docs that would drift
  out of sync with the code.
