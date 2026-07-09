# net_rtl8139

A minimal RTL8139 Fast Ethernet driver. Registers as `"net"` and serves
raw Ethernet frames in and out over a client-supplied shared-memory
grant. No ARP, no IP, nothing above the Ethernet-frame layer -- that's
later checkpoints' job.

## Capabilities it needs

Granted by `main.rs` at spawn (see `spawn_net_rtl8139`), once PCI
enumeration (`kernel/src/pci.rs`) actually finds an RTL8139 attached:

| CSlot | What |
|-------|------|
| 1     | Name service (auto-granted) |
| 2     | Its own inbox |
| 3     | `IrqControl` for the discovered PCI interrupt line |
| 4     | A read-only `MemoryGrant` over one page carrying the discovered I/O base (see below) |

Port access (the discovered I/O-BAR range, 256 ports starting at BAR0)
is granted the same way any driver's I/O ports are in this project --
through the pre-existing `allowed_ports`/TSS-bitmap mechanism at spawn,
not a capability.

### Why CSlot 4 exists

Every other hand-wired hardware capability in this project (console
server's VGA buffer and keyboard IRQ, storage_ata's ATA ports) targets a
fixed legacy address both the kernel and the driver's own source code
agree on at compile time. The RTL8139's I/O-port range isn't like that:
it's assigned by firmware at boot to whatever address the platform
picks (QEMU's default chipset happens to land it around `0xc000`), only
knowable once `kernel/src/main.rs` reads it out of the device's PCI
config space at runtime. CSlot 4 is how that one discovered value
crosses into this task at all: a `MemoryGrant` over a single physical
page `main.rs` wrote it into, reusing the capability kind that already
exists for sharing memory rather than adding a new one (or a new
syscall) just to pass one integer at spawn time.

## Design

- One frame in flight at a time -- no attempt to pipeline a second send
  before the first completes -- but rotating through all four transmit
  descriptors the hardware has, in order, not reusing descriptor 0 for
  every send: both real RTL8139 hardware and QEMU's emulation of it
  track an internal "next expected descriptor" pointer that advances
  after each completion, and rewriting the one just used instead of the
  next one in sequence is simply never picked up.
- Receive is interrupt-driven, but only the *most recently received*
  frame is ever held for a client to claim, not a queue: if a second
  frame arrives before `NIC_OP_RECV` claims the first, the first is
  silently dropped. See `main.rs`'s own doc comment for why that's
  enough for the current test fixture, and console_server's raw-mode
  key queue for what a real queue would look like if a future client
  needs one.
- The receive ring's nominal size is 8192 bytes (the smallest of the four
  sizes the hardware supports, and the modulus the card's own ring
  pointer wraps against), allocated with two kinds of padding on top:
  the "8K+16" RBLEN datasheet figure's 16 bytes of DMA slack, and a
  further 1500-byte overflow margin (`RCR`'s `WRAP` bit) the card is
  allowed to spill a packet's tail into rather than ever splitting one
  across the ring's physical end -- keeping this driver's own
  ring-parsing logic from needing to handle that case at all. Only the
  8192-byte figure is ever used as the wrap boundary; the allocation
  padding is not part of it.
- Reset and transmit-completion are both bounded busy-waits (a fixed
  poll count, not truly indefinite) on `CR`'s `RST` bit and the active
  descriptor's `OWN` bit respectively -- the same polling-not-interrupt-driven
  approach storage_ata's own PIO loop already uses for an analogous
  wait, since this scheduler preempts on the timer tick regardless. The
  bound exists purely as a last-resort guard against a stuck card
  leaving this driver's single-threaded service loop spinning forever
  instead of ever answering another client or interrupt again.

## Protocol

A client connects once, then issues any number of requests:

1. `NIC_OP_SET_BUFFER` (`transfer` = a `MemoryGrant` for a page the
   client already mapped locally) -- this driver maps the same physical
   page into its own address space and reads/writes frames directly
   through it.
2. `NIC_OP_SET_REPLY` (`transfer` = a capability to the client's own
   inbox) -- where replies get sent.
3. `NIC_OP_GET_MAC` -- replies with the card's burned-in MAC address,
   packed the same way `pack_name` packs an 8-byte name (`w0` = first 4
   bytes, `w1` = last 2, little-endian).
4. Any number of `NIC_OP_SEND` (`w1` = frame length in the shared
   buffer) -- transmits it as one raw Ethernet frame, replying `w0 = 1`
   (sent) or `w0 = 0` (failed: no buffer mapped, or the frame doesn't
   fit).
5. Any number of `NIC_OP_RECV` -- blocks until the next frame is
   received (or replies immediately if one was already waiting), with
   its length placed in the shared buffer and returned as `w0`.
6. Any number of `NIC_OP_TRY_RECV` -- like `NIC_OP_RECV`, but replies
   immediately either way (`w0` = frame length, `0` if none waiting)
   instead of deferring the reply when nothing's queued. `netstack`'s
   TCP client needs this: it can't safely leave a `NIC_OP_RECV`
   outstanding while it also has to poll an external client for
   requests (see `userland/services/netstack/README.md` for the
   deadlock that would otherwise risk).

See `userland/libpcern`'s `nic_connect`/`nic_get_mac`/`nic_send`/
`nic_recv` for the client-side helpers -- `cap_test`'s `nic_test` and
`userland/services/netstack` are this protocol's two clients so far.

**Only one client at a time is supported**, the same scope-narrowing
precedent as every other driver in this project.
