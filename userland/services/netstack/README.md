# netstack

A minimal network stack: claims a static IP address, answers ARP
requests and ICMP echo requests (ping) for it, and opens outbound TCP
connections on behalf of other tasks -- all over raw Ethernet frames
served by `net_rtl8139`. No DHCP, no ARP cache beyond a single active
connection's own peer, no IP forwarding, no fragmentation, no IPv4
options, no UDP.

## Capabilities it needs

None beyond the universal name-service auto-grant every task gets --
`netstack` reaches `net_rtl8139` the same way any other client would, by
looking up `"net"` and connecting to it (see
`userland/drivers/net_rtl8139/README.md`'s protocol section). It owns no
hardware, no ports, no IRQ.

## Address configuration

The static IP (`10.0.2.15`, matching the conventional guest address
QEMU's own usermode networking assumes) is a hardcoded constant in
`main.rs` -- there's no DHCP client, no configuration file, no way to
change it short of rebuilding. Address configuration is later work.

## What it serves

- **ARP/ICMP responder** -- answers any ARP request or ICMP echo request
  addressed to its own static IP, unconditionally, for as long as it
  runs. Every reply is built **in place** in the same shared buffer a
  request arrived in (swap source/destination, flip request to reply,
  recompute checksums) rather than assembling a fresh frame.
- **TCP client** -- registered as `"tcp"`, serving `libpcern`'s
  `TCP_OP_SET_BUFFER`/`SET_REPLY`/`CONNECT`/`SEND`/`RECV`/`CLOSE` to
  whichever other task looks it up (see `libpcern::tcp_connect_setup`/
  `tcp_open`/`tcp_write`/`tcp_read`/`tcp_close`). Only one connection at
  a time -- the same scope every driver/service in this project already
  has for its clients. `CONNECT` resolves the peer's MAC via ARP itself
  before addressing it at the Ethernet layer, the one ARP *client* role
  the pure responder never needed.

### TCP scope: fixed window, no congestion control, no retransmission

- **Fixed window**: this client advertises, and caps every `SEND` at, one
  constant (`WINDOW`, 2048 bytes) for the lifetime of a connection. It
  never grows or shrinks that cap based on traffic, and never even reads
  the *peer's* own advertised window -- a real flow-control
  implementation would need both; this doesn't claim to be one.
- **No congestion control**: nothing here tracks round-trip time, ramps a
  congestion window, or reacts to loss signals at all.
- **No retransmission**: a SYN, data segment, or FIN that's genuinely
  lost is never resent. Every wait (ARP resolution, the handshake, more
  data, the close handshake) is bounded only by a count of poll
  iterations (`MAX_ATTEMPTS`), not a real timeout -- this kernel exposes
  no clock/timer syscall to userland, so there's no wall-clock deadline
  to build one from. A connection whose peer never responds eventually
  fails outright rather than hanging forever, but it won't recover from
  a single dropped segment the way a real TCP stack would.
- **No options, either direction sent, but read on receipt**: every
  segment this client builds is a plain 20-byte header, no MSS/SACK/
  timestamps. A segment it *parses* still reads the peer's own data
  offset field rather than assuming 20 bytes, since a real TCP peer's
  SYN-ACK commonly carries options this client never sends itself --
  see `tcp.rs`'s own doc comment.

None of this is hidden inside otherwise-normal-looking TCP semantics --
each cut is a real, load-bearing decision to keep this client's
scope to "enough transport to speak HTTP" (a single small
request/response exchange), not a general-purpose sliding-window
implementation.

## Design

- `proto.rs` holds ARP/ICMP packet-level logic (parsing, in-place reply
  construction, the shared Internet checksum, plus `build_arp_request`/
  `parse_arp_reply` for the TCP client's own peer resolution) as plain
  functions over byte slices, independent of IPC.
- `tcp.rs` holds TCP segment building/parsing and the pseudo-header
  checksum, the same "packet-level logic only, no IPC" split as
  `proto.rs`.
- `main.rs` is the IPC glue *and* the TCP connection state machine
  (`Connection`/`ConnState`/`PendingOp`) -- unlike the pure ARP/ICMP
  responder, the state machine is advanced frame-by-frame from inside
  the same dispatch loop that talks to `net_rtl8139`, and a client
  operation's reply is deferred until whatever condition it's waiting
  for is actually met (the same "arm now, reply later" pattern
  `net_rtl8139`'s own `NIC_OP_RECV` already uses for its deferred
  reply), so it doesn't cleanly separate into its own IPC-free module.
- **Never blocks in `net_rtl8139`'s `NIC_OP_RECV`.** The pure ARP/ICMP
  responder did (`loop { let len = nic_recv(...); ... }`), which worked
  because it had nothing else to wait on. The TCP client requires
  `netstack` to *also* stay responsive to an external client's
  requests, arriving independently of network traffic -- and this
  kernel's IPC has no `select` to wait on both jointly. Leaving one
  `NIC_OP_RECV` permanently outstanding and dispatching by sender looks
  tempting, but isn't safe: `net_rtl8139` has only one reply-to slot,
  and delivering its deferred reply is itself a blocking `send` on its
  end -- if this task also tries to send it anything else (an ARP
  request, a TCP segment) while that `NIC_OP_RECV` is still outstanding,
  both tasks can end up blocked in `send`, each waiting for the other's
  `recv`, with neither about to call one. A genuine deadlock, confirmed
  by tracing `kernel/src/ipc.rs`'s actual `send`/`recv` semantics, not a
  hypothetical one. The fix: `NIC_OP_TRY_RECV` (`net_rtl8139`) and
  `SYS_TRY_RECV` (`try_recv`, the kernel) both reply/return immediately
  either way, never leaving anything outstanding -- the main loop polls
  both, non-blockingly, yielding once per round when neither has
  anything. This is the one service in this project that busy-polls
  instead of blocking when idle; see `kernel/src/ipc.rs`'s `try_recv`
  doc comment for the full trade-off.
- **A bounded FIFO, not a single slot, for messages that arrive while
  waiting on `net_rtl8139`.** A client's `TCP_OP_SET_BUFFER`/`SET_REPLY`
  are sent back-to-back with no reply in between, so more than one can
  legitimately be queued before `net_rtl8139` gets around to replying to
  whatever this task most recently asked it -- a single `Option` slot
  silently drops all but the last one the moment a second arrives (a
  real bug caught during this feature's own testing). See `main.rs`'s
  `Stash`.
- One inbox, three roles (the one-shot name-lookup reply at startup,
  `net_rtl8139`'s replies, and an external TCP client's requests) --
  every message received on it is dispatched by checking the
  kernel-attested sender first, never by assuming "the next message is
  whatever I asked for." That's what makes sharing one inbox safe here,
  unlike the cap_test postmortem CLAUDE.md documents (a peer's message
  consumed by a `recv` that assumed it knew what it was getting, with no
  sender check at all): see `main.rs`'s own top-of-file doc comment for
  the full argument, including why a second, dedicated endpoint
  wouldn't actually avoid needing this same discipline anyway.
- Only one client (this task) of `net_rtl8139`, and only one external
  TCP client of `netstack` itself, at a time -- the same scope every
  driver/service in this project already has.

## Testing

- `make test-arp` / `run_arp_icmp_test.sh` -- the ARP/ICMP responder,
  verified against real ARP and ICMP traffic from an external peer.
- `make test-tcp` / `run_tcp_test.sh` -- the TCP client, verified against
  a real three-way handshake, a real HTTP-shaped request/response
  exchange, and a real close handshake with an independent peer
  (a hand-built passive-open TCP responder in Python) on the wire, plus
  `userland/cap_test`'s `http_client_test` fixture's own exit code and an
  independent packet-capture re-check -- the same "don't just trust one
  witness" pattern every driver/service test in this project uses.
