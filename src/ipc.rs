//! Synchronous rendezvous IPC: `send` blocks until a matching `recv` is
//! waiting (and vice versa), at which point the kernel copies a short,
//! fixed-size (4-word) message directly between the two calls -- no
//! kernel-side queues/buffers, no allocation on the hot path.
//!
//! The trick that keeps this simple: a blocked task's "return registers"
//! (`SavedRegs`, on its own kernel stack -- see syscall.rs/syscall_asm.s)
//! are mapped in every address space along with the rest of the kernel, so
//! whichever call resolves the rendezvous can write the other side's
//! result directly into them, no cross-address-space buffer copy needed.

use alloc::vec::Vec;

use crate::scheduler;
use crate::sync::Mutex;
use crate::syscall::SavedRegs;
use crate::task::TaskId;

/// Reserved sender id meaning "the kernel/hardware" -- never a real TaskId
/// (those start at 1). A driver calls `recv(filter=None or Some(0))` to
/// wait for its next forwarded interrupt (see notify_interrupt below,
/// called from IRQ handlers via irq.rs's registration table) the same way
/// it would wait for a message from any other task.
pub const KERNEL_TASK_ID: TaskId = 0;

struct PendingRecv {
    task_id: TaskId,
    filter: Option<TaskId>,
    regs: *mut SavedRegs,
}

struct PendingSend {
    task_id: TaskId,
    dest: TaskId,
    msg: [u32; 4],
    regs: *mut SavedRegs,
}

/// A forwarded interrupt notification a driver hasn't picked up yet (it
/// wasn't already blocked in a matching `recv` when the IRQ fired). Queued
/// rather than dropped so a driver that's briefly busy doesn't miss
/// events -- e.g. two keystrokes arriving before the console server gets
/// back around to its next `recv`.
struct PendingIrqEvent {
    target: TaskId,
    irq: u32,
    data: u32,
}

// Safety: these pointers only ever reference a task's own kernel stack,
// which is stable for that task's whole lifetime. All access goes through
// the Mutexes below, which (per sync.rs) disable interrupts for their
// duration, and this is a single core -- so there's never a moment where
// two different call sites are touching the same one concurrently.
unsafe impl Send for PendingRecv {}
unsafe impl Send for PendingSend {}

static PENDING_RECVS: Mutex<Vec<PendingRecv>> = Mutex::new(Vec::new());
static PENDING_SENDS: Mutex<Vec<PendingSend>> = Mutex::new(Vec::new());
static PENDING_IRQ_EVENTS: Mutex<Vec<PendingIrqEvent>> = Mutex::new(Vec::new());

/// Delivers `msg` to `dest`: immediately if `dest` is already blocked in a
/// matching `recv`, otherwise blocks the caller until one arrives.
pub fn send(self_id: TaskId, dest: TaskId, msg: [u32; 4], regs: *mut SavedRegs) {
    {
        let mut recvs = PENDING_RECVS.lock();
        if let Some(pos) = recvs
            .iter()
            .position(|r| r.task_id == dest && r.filter.map_or(true, |f| f == self_id))
        {
            let matched = recvs.remove(pos);
            drop(recvs);
            unsafe {
                (*matched.regs).eax = self_id as u32;
                (*matched.regs).ebx = msg[0];
                (*matched.regs).ecx = msg[1];
                (*matched.regs).edx = msg[2];
                (*matched.regs).esi = msg[3];
                (*regs).eax = 0;
            }
            scheduler::wake(matched.task_id);
            return;
        }
    }

    PENDING_SENDS.lock().push(PendingSend {
        task_id: self_id,
        dest,
        msg,
        regs,
    });
    scheduler::block_current();
    // Resumed only once a matching recv already wrote our result into
    // `regs` and woke us -- nothing left to do.
}

/// Takes a message addressed to `self_id`, optionally only from `filter`:
/// immediately if a matching `send` (or, when `filter` is `None` or
/// `Some(KERNEL_TASK_ID)`, a queued interrupt notification) is already
/// waiting, otherwise blocks the caller until one arrives.
pub fn recv(self_id: TaskId, filter: Option<TaskId>, regs: *mut SavedRegs) {
    {
        let mut sends = PENDING_SENDS.lock();
        if let Some(pos) = sends
            .iter()
            .position(|s| s.dest == self_id && filter.map_or(true, |f| f == s.task_id))
        {
            let matched = sends.remove(pos);
            drop(sends);
            unsafe {
                (*regs).eax = matched.task_id as u32;
                (*regs).ebx = matched.msg[0];
                (*regs).ecx = matched.msg[1];
                (*regs).edx = matched.msg[2];
                (*regs).esi = matched.msg[3];
                (*matched.regs).eax = 0;
            }
            scheduler::wake(matched.task_id);
            return;
        }
    }

    if filter.map_or(true, |f| f == KERNEL_TASK_ID) {
        let mut events = PENDING_IRQ_EVENTS.lock();
        if let Some(pos) = events.iter().position(|e| e.target == self_id) {
            let event = events.remove(pos);
            drop(events);
            unsafe {
                (*regs).eax = KERNEL_TASK_ID as u32;
                (*regs).ebx = event.irq;
                (*regs).ecx = event.data;
                (*regs).edx = 0;
                (*regs).esi = 0;
            }
            return;
        }
    }

    PENDING_RECVS.lock().push(PendingRecv {
        task_id: self_id,
        filter,
        regs,
    });
    scheduler::block_current();
    // Resumed only once a matching send/notify_interrupt already wrote our
    // result into `regs` and woke us -- nothing left to do.
}

/// Forwards a hardware interrupt to `target`: delivers immediately if it's
/// already blocked in a matching `recv`, otherwise queues the event so the
/// next matching `recv` sees it right away. Never blocks -- this is called
/// directly from interrupt context (see keyboard.rs), which must ack and
/// return promptly, not suspend waiting for a receiver.
pub fn notify_interrupt(target: TaskId, irq: u32, data: u32) {
    let mut recvs = PENDING_RECVS.lock();
    if let Some(pos) = recvs
        .iter()
        .position(|r| r.task_id == target && r.filter.map_or(true, |f| f == KERNEL_TASK_ID))
    {
        let matched = recvs.remove(pos);
        drop(recvs);
        unsafe {
            (*matched.regs).eax = KERNEL_TASK_ID as u32;
            (*matched.regs).ebx = irq;
            (*matched.regs).ecx = data;
            (*matched.regs).edx = 0;
            (*matched.regs).esi = 0;
        }
        scheduler::wake(matched.task_id);
        return;
    }
    drop(recvs);

    PENDING_IRQ_EVENTS.lock().push(PendingIrqEvent { target, irq, data });
}

/// Called when `task_id` exits: without this, a task blocked waiting to
/// send to or receive from `task_id` would never be matched (its partner
/// is gone) and would stay `Blocked` forever with no error and no wake, a
/// silent permanent hang. Wakes every such waiter with a failure
/// (`eax = u32::MAX`, matching the "unknown syscall" sentinel elsewhere)
/// instead, and drops the now-meaningless pending entries.
pub fn task_exited(task_id: TaskId) {
    let mut recvs = PENDING_RECVS.lock();
    recvs.retain(|r| {
        if r.filter == Some(task_id) {
            unsafe { (*r.regs).eax = u32::MAX };
            scheduler::wake(r.task_id);
            false
        } else {
            true
        }
    });
    drop(recvs);

    let mut sends = PENDING_SENDS.lock();
    sends.retain(|s| {
        if s.dest == task_id {
            unsafe { (*s.regs).eax = u32::MAX };
            scheduler::wake(s.task_id);
            false
        } else {
            true
        }
    });
    drop(sends);

    // No one to wake here (nothing was blocked on these, or they'd have
    // been delivered already) -- just drop them so they don't sit around
    // forever addressed to a task that no longer exists.
    PENDING_IRQ_EVENTS.lock().retain(|e| e.target != task_id);
}
