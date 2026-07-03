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
