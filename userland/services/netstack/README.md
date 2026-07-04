# netstack

A minimal ARP + IPv4 + ICMP responder. Claims a static IP address and
answers ARP requests and ICMP echo requests (ping) for it, over raw
Ethernet frames served by `net_rtl8139`. No outbound connections, no
DHCP, no ARP cache, no IP forwarding, no fragmentation, no IPv4 options
-- the first checkpoint where ZephyrLite is observable as "a host with an
IP address" from anywhere else on the wire, not just from inside its own
boot log.

## Capabilities it needs

None beyond the universal name-service auto-grant every task gets --
`netstack` reaches `net_rtl8139` the same way any other client would,
by looking up `"net"` and connecting to it (see
`userland/drivers/net_rtl8139/README.md`'s protocol section). It owns no
hardware, no ports, no IRQ.

## Address configuration

The static IP (`10.0.2.15`, matching the conventional guest address
QEMU's own usermode networking assumes) is a hardcoded constant in
`main.rs` -- there's no DHCP client, no configuration file, no way to
change it short of rebuilding. That's deliberately out of scope for this
checkpoint, whose whole point is proving the ARP/ICMP responder path
works at all against real traffic; address configuration is later work.

## Design

- `proto.rs` holds all packet-level logic (parsing, in-place reply
  construction, the Internet checksum) as plain functions over byte
  slices, independent of IPC -- `main.rs` is purely the glue connecting
  it to `net_rtl8139`'s protocol.
- Replies are built **in place** in the same shared buffer a request
  arrived in, mirroring the field layout back onto itself (swap
  source/destination, flip request to reply, recompute checksums) rather
  than assembling a fresh frame -- avoids a second buffer and keeps every
  untouched field (an ICMP echo's identifier/sequence/payload) exactly
  as received with no risk of a copy bug altering them.
- Incoming checksums are never verified before use: Ethernet's own CRC
  (checked by the NIC hardware; a frame that fails it is never handed up
  to any driver at all) is the integrity layer this narrow scope relies
  on, the same way every other protocol in this project trusts IPC's
  kernel-mediated delivery rather than re-checking it at every layer.
- IPv4 packets with any header length other than the plain 20-byte
  minimum (i.e. any options present) are silently ignored, matching
  every other checkpoint's "narrow the scope, don't half-implement the
  general case" precedent.
- Only one client (this task) of `net_rtl8139` at a time -- the same
  scope every driver in this project already has.

## What it does *not* do

No outbound ARP resolution (nothing to resolve -- replies just mirror
whatever MAC/IP a request already named), no ARP cache, no IP
forwarding/routing, no fragmentation or reassembly, no UDP or TCP, no
shell command to use any of this yet. All later checkpoints' job.

## Testing

See the root README's Testing section (`make test-arp`) and
`run_arp_icmp_test.sh` for how this is verified against *real* ARP and
ICMP traffic from an external peer, not a synthetic in-process byte.
