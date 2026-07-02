//! Port I/O helpers, mirroring the kernel's own src/port.rs. Only usable
//! because the console server is spawned driver-flagged with these exact
//! ports (0x3D4/0x3D5, the CRTC index/data pair) in its `allowed_ports` --
//! see loader::spawn_from_module in the kernel and main.rs's spawn call.

use core::arch::asm;

#[inline(always)]
pub unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}
