use crate::idt::InterruptStackFrame;
use crate::ipc;
use crate::irq;
use crate::pic;
use crate::port::inb;

/// IRQ1: acks (reads the raw scancode) and forwards it, via a non-blocking
/// IPC notification, to whichever userspace driver has registered for
/// IRQ1 (see irq.rs / the register_for_interrupt syscall) -- the console
/// server, spawned driver-flagged in main.rs. Scancode decoding and
/// echoing to the screen is entirely that task's job now; this ISR
/// touches nothing beyond the keyboard controller port and the PIC.
pub extern "x86-interrupt" fn handler(_frame: InterruptStackFrame) {
    let scancode = unsafe { inb(0x60) };

    if let Some(driver) = irq::handler_for(1) {
        ipc::notify_interrupt(driver, 1, scancode as u32);
    }

    pic::send_eoi(1);
}
