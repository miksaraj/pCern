//! Routes hardware IRQs (0-15, the legacy PIC lines) to whichever
//! userspace driver task registered for them via the register_for_
//! interrupt syscall (see syscall.rs). Actually notifying that task when
//! the IRQ fires is ipc::notify_interrupt's job; this module only tracks
//! who's registered.

use crate::sync::Mutex;
use crate::task::TaskId;

const NUM_IRQS: usize = 16;

static HANDLERS: Mutex<[Option<TaskId>; NUM_IRQS]> = Mutex::new([None; NUM_IRQS]);

/// Registers `task_id` as the driver for `irq` (0-15), replacing whatever
/// was registered before. Returns false for an out-of-range irq number.
pub fn register(irq: u32, task_id: TaskId) -> bool {
    match HANDLERS.lock().get_mut(irq as usize) {
        Some(slot) => {
            *slot = Some(task_id);
            true
        }
        None => false,
    }
}

/// The task currently registered for `irq`, if any. Unused until the next
/// checkpoint wires the keyboard ISR to call this instead of handling
/// keystrokes itself.
#[allow(dead_code)]
pub fn handler_for(irq: u32) -> Option<TaskId> {
    HANDLERS.lock().get(irq as usize).copied().flatten()
}
