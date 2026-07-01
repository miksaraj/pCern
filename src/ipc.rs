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

// Safety: these pointers only ever reference a task's own kernel stack,
// which is stable for that task's whole lifetime. All access goes through
// the Mutexes below, which (per sync.rs) disable interrupts for their
// duration, and this is a single core -- so there's never a moment where
// two different call sites are touching the same one concurrently.
unsafe impl Send for PendingRecv {}
unsafe impl Send for PendingSend {}

static PENDING_RECVS: Mutex<Vec<PendingRecv>> = Mutex::new(Vec::new());
static PENDING_SENDS: Mutex<Vec<PendingSend>> = Mutex::new(Vec::new());

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
/// immediately if a matching `send` is already blocked, otherwise blocks
/// the caller until one arrives.
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

    PENDING_RECVS.lock().push(PendingRecv {
        task_id: self_id,
        filter,
        regs,
    });
    scheduler::block_current();
    // Resumed only once a matching send already wrote our result into
    // `regs` and woke us -- nothing left to do.
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
}
