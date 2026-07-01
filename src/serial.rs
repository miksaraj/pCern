//! Minimal 16550 UART driver for COM1 (0x3F8), used purely as a debug
//! console: unlike the VGA buffer, it never scrolls out of reach and is
//! easy to capture headlessly (`qemu -serial file:...`). Every byte the
//! VGA writer prints is mirrored here too (see vga.rs's `_print`).

use crate::port::{inb, outb};

const COM1: u16 = 0x3F8;

pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00); // disable interrupts
        outb(COM1 + 3, 0x80); // enable DLAB to set the baud rate divisor
        outb(COM1 + 0, 0x03); // divisor low byte: 115200 / 3 = 38400 baud
        outb(COM1 + 1, 0x00); // divisor high byte
        outb(COM1 + 3, 0x03); // 8 bits, no parity, one stop bit; DLAB off
        outb(COM1 + 2, 0xC7); // enable + clear FIFOs, 14-byte threshold
        outb(COM1 + 4, 0x0B); // IRQs disabled, RTS/DSR set
    }
}

fn transmit_empty() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

pub fn write_byte(byte: u8) {
    while !transmit_empty() {}
    unsafe { outb(COM1, byte) };
}
