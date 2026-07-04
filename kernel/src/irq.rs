//! Routes hardware IRQs (0-15, the legacy PIC lines) to whichever endpoint
//! a userspace driver registered for them via the register_for_interrupt
//! syscall (see syscall.rs). Actually notifying that endpoint when the IRQ
//! fires is ipc::notify_interrupt's job; this module only tracks what's
//! registered.

use crate::cap::EndpointId;
use crate::sync::Mutex;

const NUM_IRQS: usize = 16;

static HANDLERS: Mutex<[Option<EndpointId>; NUM_IRQS]> = Mutex::new([None; NUM_IRQS]);

/// Registers `endpoint` as the target for `irq` (0-15), replacing whatever
/// was registered before. Returns false for an out-of-range irq number.
pub fn register(irq: u32, endpoint: EndpointId) -> bool {
    match HANDLERS.lock().get_mut(irq as usize) {
        Some(slot) => {
            *slot = Some(endpoint);
            true
        }
        None => false,
    }
}

/// The endpoint currently registered for `irq`, if any.
pub fn handler_for(irq: u32) -> Option<EndpointId> {
    HANDLERS.lock().get(irq as usize).copied().flatten()
}

/// Every IRQ (0-15) currently registered against `endpoint`, if any --
/// the reverse of `handler_for`. Used by `ipc::recv` to unmask each one
/// right as its driver becomes ready for another notification (see
/// pic::mask's own doc comment for why that pairing matters). A plain
/// linear scan of 16 entries, called on every `recv` regardless of
/// whether `endpoint` has any IRQ registered at all -- cheap enough not
/// to matter next to an IPC rendezvous.
pub fn irqs_for_endpoint(endpoint: EndpointId) -> impl Iterator<Item = u32> {
    let handlers = *HANDLERS.lock();
    (0..NUM_IRQS as u32).filter(move |&irq| handlers[irq as usize] == Some(endpoint))
}

/// Looks up whatever userspace endpoint is registered for `irq` and
/// delivers a non-blocking interrupt notification carrying `data`, then
/// acknowledges the interrupt to the PIC. Shared by every IRQ ISR that
/// has no device-specific bytes of its own to read before notifying
/// (unlike keyboard.rs's handler, which reads the scancode itself first,
/// since that byte -- not just "an interrupt happened" -- is the whole
/// point of the notification) -- Checkpoint W's generic, runtime-assigned
/// IRQ2-15 stubs (idt.rs) all go through this.
///
/// Masks `irq` at the PIC *before* sending EOI: unlike the timer/
/// keyboard's own edge-triggered-in-practice lines, a PCI device's
/// interrupt is level-triggered and stays asserted until the device's
/// own condition is cleared -- which only the userland driver's own
/// register writes can do, not anything at this level. Without masking
/// first, EOI alone would have the PIC re-deliver the still-asserted
/// line the instant `iret` re-enables interrupts, faster than any task
/// could ever be scheduled to actually clear it -- an infinite storm,
/// not a single notification. `ipc::recv` unmasks it again once the
/// driver is back and ready for another.
pub fn dispatch(irq: u32, data: u32) {
    crate::pic::mask(irq as u8);
    if let Some(endpoint) = handler_for(irq) {
        crate::ipc::notify_interrupt(endpoint, irq, data);
    }
    crate::pic::send_eoi(irq as u8);
}
