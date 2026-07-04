use core::arch::asm;

#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

/// Checkpoint W: PCI configuration-space access (0xCF8/0xCFC) is
/// dword-wide -- the only reason this kernel needs 32-bit port I/O at
/// all, everything else so far has been byte-wide.
#[inline]
pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

#[inline]
pub unsafe fn outl(port: u16, value: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
}

/// Burns a small amount of time by writing to an unused port; used to give
/// slow legacy hardware (PIC, etc.) time to process the previous command.
#[inline]
pub unsafe fn io_wait() {
    outb(0x80, 0);
}
