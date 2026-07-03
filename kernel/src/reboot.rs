//! Checkpoint V: a minimal reboot mechanism, gated by a capability
//! (`cap::CapKind::RebootControl`) rather than a bare syscall any task
//! could call. This kernel has no ACPI reset register support, so it uses
//! the standard, well-worn alternative on x86: pulse the 8042 keyboard
//! controller's CPU-reset output line.

use crate::port::{inb, outb};

const KBD_COMMAND_PORT: u16 = 0x64;
/// Set while the controller hasn't yet consumed the last byte written to
/// it -- must be clear before writing another command.
const KBD_STATUS_INPUT_FULL: u8 = 0x02;
/// "Pulse output line" controller command; the output port's bit 0 drives
/// the CPU's RESET pin, so pulsing it resets the machine.
const KBD_CMD_PULSE_RESET: u8 = 0xFE;

/// Resets the machine. Never returns: on real hardware and under QEMU the
/// pulse takes effect immediately. If the controller is somehow wedged
/// and the pulse never lands, this keeps retrying forever rather than
/// falling through to whatever called it -- a syscall handler that
/// "returns" from a reboot request would be a far more surprising failure
/// mode for a caller than this hanging.
pub fn reset() -> ! {
    loop {
        unsafe {
            while inb(KBD_COMMAND_PORT) & KBD_STATUS_INPUT_FULL != 0 {}
            outb(KBD_COMMAND_PORT, KBD_CMD_PULSE_RESET);
        }
    }
}
