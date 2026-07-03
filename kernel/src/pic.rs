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
