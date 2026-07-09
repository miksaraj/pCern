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
