use core::sync::atomic::{AtomicU64, Ordering};

use crate::idt::InterruptStackFrame;
use crate::pic;
use crate::scheduler;

static TICKS: AtomicU64 = AtomicU64::new(0);

/// IRQ0: fires at the PIT's default rate (~18.2 Hz, since we don't reprogram it).
/// Drives preemptive round-robin scheduling: every tick hands the CPU to the
/// next ready task.
pub extern "x86-interrupt" fn handler(_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    pic::send_eoi(0);
    scheduler::tick();
}

#[allow(dead_code)]
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}
