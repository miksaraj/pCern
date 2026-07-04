//! Minimal RTL8139 Fast Ethernet driver: hardware init, one frame at a
//! time transmit, and receive-ring parsing. I/O-port based -- reachable
//! at all only because main.rs discovers the card's I/O-BAR range via
//! PCI enumeration (kernel/src/pci.rs) and grants exactly those ports
//! through the same `allowed_ports` mechanism every other driver in this
//! project uses, never a new capability kind.
//!
//! Scope is deliberately narrow, matching every other driver in this
//! project: raw Ethernet frames in and out, nothing above that layer --
//! no ARP, no IP, that's later checkpoints' job. One frame in flight at a
//! time in each direction: a single transmit descriptor (of the four the
//! hardware has), and no attempt to pipeline a second send before the
//! first completes.

use crate::port::{inb, inl, inw, outb, outl, outw};

// Register offsets, relative to the discovered I/O-BAR base (see
// main.rs).
const REG_MAC0: u16 = 0x00; // 6 bytes, IDR0-5 -- burned-in station address
const REG_TSD0: u16 = 0x10; // Transmit Status of Descriptor 0 (the only one this driver uses)
const REG_TSAD0: u16 = 0x20; // Transmit Start Address of Descriptor 0 (physical)
const REG_RBSTART: u16 = 0x30; // Receive Buffer Start (physical)
const REG_CR: u16 = 0x37; // Command Register
const REG_CAPR: u16 = 0x38; // Current Address of Packet Read
pub const REG_ISR: u16 = 0x3E; // Interrupt Status Register
const REG_IMR: u16 = 0x3C; // Interrupt Mask Register
const REG_RCR: u16 = 0x44; // Receive Config Register
const REG_CONFIG1: u16 = 0x52;
// TCR (offset 0x40) is deliberately left untouched -- its post-reset
// default is a valid, if conservative (smaller DMA burst size), config
// that's entirely correct for this driver's one-frame-at-a-time scope;
// only throughput, not correctness, would benefit from programming it.

const CR_BUFE: u8 = 1 << 0; // Rx buffer empty (status, read-only)
const CR_TE: u8 = 1 << 2; // Transmitter Enable
const CR_RE: u8 = 1 << 3; // Receiver Enable
const CR_RST: u8 = 1 << 4; // Reset; self-clears once complete

pub const ISR_ROK: u16 = 1 << 0; // Receive OK
const ISR_TOK: u16 = 1 << 2; // Transmit OK -- no driver-side action needed, `send`'s own busy-wait already observes completion directly; only ROK's bit is inspected by name here.

const RCR_APM: u32 = 1 << 1; // Accept Physical Match (frames addressed to our own MAC)
const RCR_AM: u32 = 1 << 2; // Accept Multicast
const RCR_AB: u32 = 1 << 3; // Accept Broadcast
const RCR_WRAP: u32 = 1 << 7; // card pads rather than splitting a packet across the ring's physical end

const TSD_OWN: u32 = 1 << 13; // set BY THE CARD once it's done with the descriptor (successfully or not)

/// The receive ring's *nominal* size: 8K+16, the smallest of the four
/// sizes the hardware supports (selected by RCR's RBLEN field, left at
/// its default 0 = this size). The buffer actually allocated for it (see
/// main.rs) is larger by another 1500 bytes: `RCR_WRAP` above promises
/// the card it never needs to split a packet's bytes across the ring's
/// physical end, in exchange for the driver reserving that much
/// additional headroom past the nominal boundary for it to spill into.
pub const RX_RING_SIZE: usize = 8192 + 16;
/// Total bytes the receive buffer must actually be, including the
/// `RCR_WRAP` overflow margin above.
pub const RX_BUF_BYTES: usize = RX_RING_SIZE + 1500;
/// The largest raw Ethernet frame (header + payload, excluding the
/// 4-byte CRC the card appends/strips) this driver will send or deliver.
pub const MAX_FRAME_SIZE: usize = 1518;

/// Initializes the card: resets it, points it at the (already allocated)
/// receive ring, enables RX/TX and the two interrupt conditions this
/// driver cares about, and returns the card's burned-in MAC address.
/// `rx_buf_phys` must be the physical address of a buffer at least
/// `RX_BUF_BYTES` long and physically contiguous -- the card's DMA
/// engine writes to it directly, with no notion of this task's own page
/// tables, so scattered pages that merely *look* contiguous in this
/// task's own virtual address space would not work.
pub fn init(io_base: u16, rx_buf_phys: u32) -> [u8; 6] {
    unsafe {
        // Power on (clear any sleep/power-down bits). A no-op under
        // QEMU's emulated card, which is already active on reset, but
        // this is what real hardware needs too and costs nothing here.
        outb(io_base + REG_CONFIG1, 0x00);

        outb(io_base + REG_CR, CR_RST);
        while inb(io_base + REG_CR) & CR_RST != 0 {}

        let mut mac = [0u8; 6];
        for (i, byte) in mac.iter_mut().enumerate() {
            *byte = inb(io_base + REG_MAC0 + i as u16);
        }

        outl(io_base + REG_RBSTART, rx_buf_phys);
        outw(io_base + REG_IMR, ISR_ROK | ISR_TOK);
        outl(io_base + REG_RCR, RCR_APM | RCR_AM | RCR_AB | RCR_WRAP);
        outb(io_base + REG_CR, CR_RE | CR_TE);

        mac
    }
}

/// Reads the Interrupt Status Register and immediately acknowledges
/// every bit just read (the card's documented write-1-to-clear
/// convention for this register), returning the bits that were set so
/// the caller can decide what to do about them.
pub fn ack_interrupt(io_base: u16) -> u16 {
    unsafe {
        let isr = inw(io_base + REG_ISR);
        outw(io_base + REG_ISR, isr);
        isr
    }
}

/// Transmits `frame` (must be non-empty and at most `MAX_FRAME_SIZE`
/// bytes): copies it into the driver's own transmit buffer (already
/// mapped at `tx_buf_virt`, physical address `tx_buf_phys`), kicks off
/// transmission via the one descriptor this driver uses, then blocks --
/// a plain busy-wait, the same polling-not-interrupt-driven approach
/// storage_ata's own PIO loop already uses for an analogous wait; this
/// scheduler preempts on the timer tick regardless, so a polling loop
/// here can't starve anything else -- until the card reports it's done
/// with the descriptor. Returns `false` if `frame` doesn't fit.
pub fn send(io_base: u16, tx_buf_virt: usize, tx_buf_phys: u32, frame: &[u8]) -> bool {
    if frame.is_empty() || frame.len() > MAX_FRAME_SIZE {
        return false;
    }
    unsafe {
        let dst = core::slice::from_raw_parts_mut(tx_buf_virt as *mut u8, frame.len());
        dst.copy_from_slice(frame);

        outl(io_base + REG_TSAD0, tx_buf_phys);
        // Writing just the length here (bit 13, OWN, stays 0) is what
        // actually kicks off the card's DMA read + transmit; it sets bit
        // 13 back to 1 once it's done with the descriptor, successfully
        // or not -- this driver's narrow scope only checks "done", not
        // "succeeded" (no TOK/TABT inspection), matching the same
        // best-effort level of care as the rest of this checkpoint.
        outl(io_base + REG_TSD0, frame.len() as u32);
        while inl(io_base + REG_TSD0) & TSD_OWN == 0 {}
    }
    true
}

/// Drains every packet currently sitting in the receive ring into `out`,
/// keeping only the last one (see main.rs's own doc comment for why a
/// single "most recently received frame" slot, not a queue, is enough
/// for this checkpoint) and returning its length if there was at least
/// one. Draining the *whole* ring every call -- not just the newest
/// entry -- matters regardless of how many of them get kept: leaving
/// unconsumed packets sitting in the ring is what the card's own BUFE
/// status bit uses to decide there's no room for more, so skipping this
/// would eventually stall the receive path entirely.
///
/// `cur_rx` is this driver's own read offset into the ring, persisted
/// across calls by the caller (main.rs) -- there's no hardware register
/// that tracks "how far the driver has consumed," only `CAPR`, which is
/// the driver's own way of *telling* the card that, not reading it back.
pub fn receive(io_base: u16, rx_buf_virt: usize, cur_rx: &mut u16, out: &mut [u8; MAX_FRAME_SIZE]) -> Option<usize> {
    // A hard ceiling on how many ring entries one call will walk, purely
    // as a last-resort guard against ever looping forever: a genuinely
    // malformed ring entry (this driver has already seen the card's own
    // BUFE status bit report "not empty" incorrectly persist during
    // bring-up) would otherwise leave the driver's whole recv loop stuck
    // inside this call permanently, unable to answer anything else --
    // including the very interrupts still arriving for it, which would
    // then queue up in the kernel's unbounded per-endpoint IRQ event list
    // forever. Far more than this checkpoint's own test ever needs in one
    // call; hitting it at all means something is already wrong.
    const MAX_PACKETS_PER_DRAIN: u32 = 64;

    let mut last_len = None;
    unsafe {
        let mut drained = 0u32;
        while inb(io_base + REG_CR) & CR_BUFE == 0 && drained < MAX_PACKETS_PER_DRAIN {
            drained += 1;
            let hdr_addr = rx_buf_virt + *cur_rx as usize;
            let status = core::ptr::read_unaligned(hdr_addr as *const u16);
            let length = core::ptr::read_unaligned((hdr_addr + 2) as *const u16);

            // Bit 0 of the per-packet status word is this packet's own
            // ROK bit -- checked (along with a sane length) before
            // trusting `length`, so a malformed ring entry can't be
            // copied out as if it were a real frame. The ring pointer
            // still advances past it either way below: skipping *that*
            // would desync this driver's bookkeeping from the card's own
            // idea of where the ring's live data ends.
            if status & 0x0001 != 0 && (4..=MAX_FRAME_SIZE + 4).contains(&(length as usize)) {
                let data_len = length as usize - 4; // exclude the trailing CRC
                let src = core::slice::from_raw_parts((hdr_addr + 4) as *const u8, data_len);
                out[..data_len].copy_from_slice(src);
                last_len = Some(data_len);
            }

            // Packets are stored dword-aligned; advance past this one's
            // 4-byte header + its (CRC-inclusive) length, rounded up.
            let advance = (length as u32 + 4 + 3) & !3;
            let mut next = *cur_rx as u32 + advance;
            if next > RX_RING_SIZE as u32 {
                // This packet's bytes spilled into the WRAP overflow
                // margin past the nominal ring boundary -- fold the
                // offset back into the nominal range, matching where the
                // card's own hardware pointer will actually be once it
                // truly wraps back to physical offset 0.
                next -= RX_RING_SIZE as u32;
            }
            *cur_rx = next as u16;
            // The card's read-pointer register is documented to sit 16
            // bytes behind the driver's own consumed-up-to offset.
            outw(io_base + REG_CAPR, cur_rx.wrapping_sub(16));
        }
    }
    last_len
}
