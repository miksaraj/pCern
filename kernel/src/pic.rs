use crate::port::{inb, io_wait, outb};

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

const ICW1_ICW4: u8 = 0x01;
const ICW1_INIT: u8 = 0x10;
const ICW4_8086: u8 = 0x01;

/// Vector offset the master PIC's IRQs are remapped to (IRQ0 -> 32).
pub const PIC1_OFFSET: u8 = 32;
/// Vector offset the slave PIC's IRQs are remapped to (IRQ8 -> 40).
pub const PIC2_OFFSET: u8 = 40;

/// Remaps the 8259 PIC away from vectors 0-15 (which collide with CPU
/// exceptions) and unmasks only the timer (IRQ0) and keyboard (IRQ1).
pub fn init() {
    unsafe {
        outb(PIC1_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();
        outb(PIC2_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();

        outb(PIC1_DATA, PIC1_OFFSET);
        io_wait();
        outb(PIC2_DATA, PIC2_OFFSET);
        io_wait();

        outb(PIC1_DATA, 4); // master: slave PIC lives on IRQ2
        io_wait();
        outb(PIC2_DATA, 2); // slave: its cascade identity
        io_wait();

        outb(PIC1_DATA, ICW4_8086);
        io_wait();
        outb(PIC2_DATA, ICW4_8086);
        io_wait();

        outb(PIC1_DATA, 0xFC); // unmask IRQ0 (timer) and IRQ1 (keyboard)
        outb(PIC2_DATA, 0xFF); // mask everything on the slave
    }
}

/// Sends an End Of Interrupt for the given IRQ (0-15) so the PIC delivers
/// further interrupts.
pub fn send_eoi(irq: u8) {
    unsafe {
        if irq >= 8 {
            outb(PIC2_CMD, 0x20);
        }
        outb(PIC1_CMD, 0x20);
    }
}

#[allow(dead_code)]
pub fn read_mask() -> (u8, u8) {
    unsafe { (inb(PIC1_DATA), inb(PIC2_DATA)) }
}

/// Unmasks a single IRQ line (0-15) without touching any other line's
/// mask bit -- for hardware discovered after boot (Checkpoint W's
/// PCI-attached NIC), unlike `init`'s own fixed timer/keyboard unmask.
/// Transparently also unmasks IRQ2 (the master's cascade line to the
/// slave PIC) whenever `irq` is 8-15: `init` leaves it masked since
/// nothing needed the slave PIC before, but nothing routed through it
/// ever reaches the CPU while that line stays masked.
///
/// Never called for IRQ2 itself once any 8-15 line is in active use (see
/// `ipc::recv`'s per-endpoint unmask) -- re-masking IRQ2 on `mask` below
/// while a *different* slave line still needs it would cut off every
/// slave IRQ, not just the one being serviced, so `mask` deliberately
/// leaves IRQ2 alone; only this function ever touches it.
pub fn unmask(irq: u8) {
    unsafe {
        if irq >= 8 {
            let mask = inb(PIC2_DATA);
            outb(PIC2_DATA, mask & !(1 << (irq - 8)));
            let master_mask = inb(PIC1_DATA);
            outb(PIC1_DATA, master_mask & !(1 << 2));
        } else {
            let mask = inb(PIC1_DATA);
            outb(PIC1_DATA, mask & !(1 << irq));
        }
    }
}

/// Masks a single IRQ line (0-15) without touching any other line's mask
/// bit, including IRQ2 (see `unmask`'s own doc comment for why that one's
/// asymmetric). Checkpoint W: a PCI interrupt is level-triggered, not
/// edge-triggered like the keyboard's -- the line stays asserted for as
/// long as the *device's own* interrupt-pending condition does, which
/// nothing at the kernel level clears (only the userland driver's own
/// register writes do, once it actually services the condition). Sending
/// EOI alone, the way the timer/keyboard ISRs always have, would tell the
/// PIC "ready for another" while the still-asserted line immediately
/// re-triggers it -- an infinite storm the instant interrupts are
/// re-enabled on `iret`, faster than any userland task could ever be
/// scheduled to clear it. Masking the line before EOI (see
/// `irq::dispatch`) breaks that: the PIC won't reconsider a masked line
/// regardless of whether the device is still asserting it, until
/// `ipc::recv` unmasks it again on the driver's own schedule -- once it's
/// actually ready for another.
pub fn mask(irq: u8) {
    unsafe {
        if irq >= 8 {
            let mask = inb(PIC2_DATA);
            outb(PIC2_DATA, mask | (1 << (irq - 8)));
        } else {
            let mask = inb(PIC1_DATA);
            outb(PIC1_DATA, mask | (1 << irq));
        }
    }
}
