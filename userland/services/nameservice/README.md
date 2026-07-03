# nameservice

The one piece of service discovery every task gets for free. A capability
to this task's endpoint is auto-granted at CSlot 1 for every task spawned
after it (see `loader::spawn_from_module` in the kernel), the way Unix
processes implicitly inherit fds 0/1/2 -- nothing has to be told
individually how to reach it.

## Capabilities it needs

Just its own inbox, at CSlot 1 (it's the one task that never gets the
auto-granted name-service capability, for the obvious chicken-and-egg
reason: it doesn't exist yet when it would need to look itself up).

## Protocol

Two operations, `w1`/`w2` carrying an 8-byte name packed into two
little-endian words (`libpcern::pack_name`):

- **`NS_OP_REGISTER`** (`w1`/`w2` = name, `transfer` = the capability to
  register under that name): accepted only if the *kernel-attested* sender
  task ID is in a small compile-time allowlist (see `ALLOWLIST` in
  `main.rs`) mapping task IDs to the one name each is allowed to claim.
  There's no reply -- registration is fire-and-forget.
- **`NS_OP_LOOKUP`** (`w1`/`w2` = name, `transfer` = a capability to the
  caller's own inbox, used as the reply-to address since there's no other
  way to reach the caller back): open to any caller. Replies with
  `w0 = 1` and a freshly derived capability to the registered endpoint if
  found, `w0 = 0` otherwise.

A lookup is a synchronous, point-in-time check against whatever's
registered *right now* -- there's no queuing for "let me know when this
shows up". A client racing a service that needs its own setup time before
it registers (more than one IPC round trip of its own) should retry rather
than treat one failed lookup as final; see `libpcern::lookup_name_retry`.

## Why an allowlist instead of a capability kind

The alternative would be a dedicated `RegisterControl`-style capability
kind plus a syscall to let this task introspect "is this capability
actually the right kind" before trusting a registration. `main.rs`'s spawn
order already fixes which task ID each trusted service gets, and the
kernel-attested sender ID on every message is already unforgeable and
free -- so a compile-time `(task ID, name)` allowlist gets the same
security property (only the real storage driver can claim `"storage"`)
with one less kernel primitive. Adding a new trusted service means adding
one line to `ALLOWLIST` and keeping it in sync with `main.rs`'s spawn
order (documented in the comment above the list) -- `main.rs` asserts
(a real, not debug-only, assertion) that each trusted service actually
lands at the task id this table expects, right after spawning it, so a
future spawn-order change that would silently break this correspondence
panics loudly at boot instead.
