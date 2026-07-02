use core::sync::atomic::{AtomicBool, Ordering};

use crate::idt::InterruptStackFrame;
use crate::ipc;
use crate::irq;
use crate::pic;
use crate::port::inb;
use crate::print;

static SHIFT: AtomicBool = AtomicBool::new(false);

const LEFT_SHIFT: u8 = 0x2A;
const RIGHT_SHIFT: u8 = 0x36;

// PS/2 scancode set 1, unshifted/shifted ASCII for the make codes we handle.
const SCANCODE_ASCII: [u8; 58] = [
    0, 27, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'=', 8, b'\t', b'q',
    b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', b'\n', 0, b'a', b's', b'd',
    b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', 0, b'\\', b'z', b'x', b'c', b'v', b'b',
    b'n', b'm', b',', b'.', b'/', 0, b'*', 0, b' ',
];

const SCANCODE_ASCII_SHIFTED: [u8; 58] = [
    0, 27, b'!', b'@', b'#', b'$', b'%', b'^', b'&', b'*', b'(', b')', b'_', b'+', 8, b'\t', b'Q',
    b'W', b'E', b'R', b'T', b'Y', b'U', b'I', b'O', b'P', b'{', b'}', b'\n', 0, b'A', b'S', b'D',
    b'F', b'G', b'H', b'J', b'K', b'L', b':', b'"', b'~', 0, b'|', b'Z', b'X', b'C', b'V', b'B',
    b'N', b'M', b'<', b'>', b'?', 0, b'*', 0, b' ',
];

/// IRQ1: acks (reads the raw scancode) and, if a userspace driver has
/// registered for IRQ1 (see irq.rs / the register_for_interrupt syscall),
/// forwards it there via a non-blocking IPC notification. Also still
/// echoes printable ASCII to the kernel's own VGA console directly -- a
/// safety net kept until Checkpoint D's console server takes over this
/// job entirely; only basic keys and shift are handled here, no caps
/// lock/numpad.
pub extern "x86-interrupt" fn handler(_frame: InterruptStackFrame) {
    let scancode = unsafe { inb(0x60) };

    if let Some(driver) = irq::handler_for(1) {
        ipc::notify_interrupt(driver, 1, scancode as u32);
    }

    let released = scancode & 0x80 != 0;
    let code = (scancode & 0x7F) as usize;

    if code == LEFT_SHIFT as usize || code == RIGHT_SHIFT as usize {
        SHIFT.store(!released, Ordering::Relaxed);
    } else if !released && code < SCANCODE_ASCII.len() {
        let ascii = if SHIFT.load(Ordering::Relaxed) {
            SCANCODE_ASCII_SHIFTED[code]
        } else {
            SCANCODE_ASCII[code]
        };
        if ascii != 0 {
            print!("{}", ascii as char);
        }
    }

    pic::send_eoi(1);
}
