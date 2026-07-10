# Changelog

All notable changes to `netstack` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-09

### Added

- A minimal TCP client: `connect`/`send`/`recv`/`close`, exposed to
  other tasks as a new `"tcp"` protocol (`libpcern`'s `TCP_OP_*`), with
  a fixed advertised window and no congestion control or retransmission
  timers (this kernel exposes no clock to userland; every wait is
  bounded by a count of poll iterations instead -- see `MAX_ATTEMPTS`'s
  own doc comment). Resolves a connection's peer via ARP itself before
  addressing it at the Ethernet layer -- the one ARP role the original
  pure ARP/ICMP responder never needed. `proto.rs` gained
  `build_arp_request`/`parse_arp_reply` for that; a new `tcp.rs` holds
  TCP segment packing/parsing and the pseudo-header checksum.
- The main loop no longer blocks in `net_rtl8139`'s `NIC_OP_RECV` --
  it now polls both `net_rtl8139` (via the new `NIC_OP_TRY_RECV`) and
  an external TCP client's requests (via the new `SYS_TRY_RECV`),
  non-blockingly, so it can stay responsive to both an ARP/ICMP request
  arriving at any time and a client's TCP request arriving at any time,
  without ever leaving a request outstanding with `net_rtl8139` (which
  would risk a real deadlock -- see this crate's own `main.rs` doc
  comment).

### Fixed

- `TCP_OP_SEND` could ask for up to `WINDOW` (2048) bytes in one
  segment, but `tcp::build_segment` copies straight into a single
  1518-byte NIC frame buffer -- any request over 1464 bytes (the actual
  per-frame capacity after this client's fixed headers) indexed past the
  buffer and panicked, killing the whole task. Added `MAX_SEGMENT_PAYLOAD`
  as an explicit additional cap on the send path; `WINDOW` still governs
  the advertised receive window on its own, since inbound data
  accumulates across frames and isn't bound by a single frame's size the
  way one `TCP_OP_SEND` is.
- Every one of this task's own sends to `net_rtl8139` (`ArpPending`'s
  ARP request, every SYN/ACK/FIN/data segment, the fallback ARP/ICMP
  reply) called `libpcern::nic_send`, whose internal `recv` has no
  sender check -- unsafe on `MY_INBOX`, which is genuinely shared with
  an external client's requests. A client's next request, delivered by
  ordinary scheduling while this task was blocked inside one of those
  calls, could land in that unchecked `recv` instead of `net_rtl8139`'s
  actual reply, silently dropping the client's request. Replaced every
  call with a new `send_to_nic`, which goes through the same
  sender-filtering `Stash` the main loop's own poll already used.
- `TCP_OP_CONNECT` never checked that a buffer had actually been mapped
  before starting a handshake (unlike `TCP_OP_SEND`, which does) -- a
  caller that sent `CONNECT` without a successful `SET_BUFFER` first
  could reach `Established` and fault on the first inbound data segment,
  which unconditionally writes into the unmapped buffer. `CONNECT` now
  fails the same way it already does for `conn.is_some()` if the buffer
  isn't mapped.
- `Stash` silently dropped a message once full, the same failure mode
  it was added to fix, just at a higher threshold -- shrunk from 8 to 2,
  a size now provably sufficient (not just generous) given every other
  client op in this protocol waits for its reply before sending the
  next.
- The main loop re-issued two syscalls every single scheduler slot even
  fully idle (no connection, no client) -- added a capped, doubling
  `yield_now` backoff that resets the instant either poll finds
  anything.

## [0.1.0] - 2026-07-05

Initial release.

### Added

- A minimal ARP + IPv4 + ICMP responder: claims a static IP address,
  answers ARP requests for it, and replies to ICMP echo requests
  (ping) addressed to it, over raw Ethernet frames served by
  `net_rtl8139`. No outbound connections, no DHCP, no ARP cache, no IP
  forwarding. Both reply paths zero-pad a reply shorter than the
  60-byte Ethernet minimum before sending it, without ever indexing
  past the bytes actually received to do so.
