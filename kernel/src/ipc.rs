//! Synchronous rendezvous IPC: `send` blocks until a matching `recv` is
//! waiting (and vice versa), at which point the kernel copies a short,
//! fixed-size (3-word) message directly between the two calls -- no
//! kernel-side queues/buffers, no allocation on the hot path.
//!
//! The trick that keeps this simple: a blocked task's "return registers"
//! (`SavedRegs`, on its own kernel stack -- see syscall.rs/syscall_asm.s)
//! are mapped in every address space along with the rest of the kernel, so
//! whichever call resolves the rendezvous can write the other side's
//! result directly into them, no cross-address-space buffer copy needed.
//!
//! Checkpoint E: messages are addressed to an `Endpoint` object (reached
//! through a capability -- see cap.rs/task.rs's CSpace) instead of a raw
//! `TaskId`. Whoever holds a capability for an endpoint can send/recv on
//! it; there's no more "filter by sender" argument on `recv`; selectivity
//! now comes entirely from who was handed which capability.

use alloc::vec::Vec;

use crate::cap::{CapNodeId, EndpointId};
use crate::scheduler;
use crate::sync::Mutex;
use crate::syscall::SavedRegs;
use crate::task::TaskId;

/// Reserved sender id meaning "the kernel/hardware" -- never a real TaskId
/// (those start at 1). Still reported via `eax` on `recv` (see
/// `notify_interrupt`) even though addressing itself is capability-based
/// now: a driver still needs to tell "this woke me up because of an
/// interrupt" apart from "this woke me up because of a message," and the
/// true sender's identity is cheap, kernel-attested information worth
/// keeping around regardless.
pub const KERNEL_TASK_ID: TaskId = 0;

/// Just enough bookkeeping to retire a task's endpoints when it exits (see
/// `task_exited`) -- nothing reads `owner` otherwise.
struct EndpointMeta {
    owner: TaskId,
}

struct PendingRecv {
    task_id: TaskId,
    endpoint: EndpointId,
    regs: *mut SavedRegs,
}

struct PendingSend {
    task_id: TaskId,
    endpoint: EndpointId,
    msg: [u32; 3],
    /// A capability (already derived from whatever the sender named, at
    /// send-call time -- see syscall.rs's SYS_SEND) waiting to be
    /// installed into whichever task's `CSpace` ends up receiving this
    /// message. `None` if this send didn't request a transfer.
    transfer: Option<CapNodeId>,
    regs: *mut SavedRegs,
}

/// A forwarded interrupt notification a driver hasn't picked up yet (it
/// wasn't already blocked in a matching `recv` when the IRQ fired). Queued
/// rather than dropped so a driver that's briefly busy doesn't miss
/// events -- e.g. two keystrokes arriving before the console server gets
/// back around to its next `recv`.
struct PendingIrqEvent {
    endpoint: EndpointId,
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

static ENDPOINTS: Mutex<Vec<EndpointMeta>> = Mutex::new(Vec::new());
static PENDING_RECVS: Mutex<Vec<PendingRecv>> = Mutex::new(Vec::new());
static PENDING_SENDS: Mutex<Vec<PendingSend>> = Mutex::new(Vec::new());
static PENDING_IRQ_EVENTS: Mutex<Vec<PendingIrqEvent>> = Mutex::new(Vec::new());

/// Mints a new endpoint object owned by `owner` (used only so
/// `task_exited` knows which endpoints to retire when that task exits) and
/// returns its id, to be wrapped in a `cap::CapKind::Endpoint` and
/// installed into some task's `CSpace`.
pub fn create_endpoint(owner: TaskId) -> EndpointId {
    let mut endpoints = ENDPOINTS.lock();
    let id = endpoints.len();
    endpoints.push(EndpointMeta { owner });
    id
}

/// Delivers `msg` to `endpoint`, optionally handing over `transfer` (a
/// capability already derived from whatever the caller named -- see
/// syscall.rs's SYS_SEND) to whoever receives it: immediately if some task
/// is already blocked in a matching `recv` on it, otherwise blocks the
/// caller until one arrives.
pub fn send(self_id: TaskId, endpoint: EndpointId, msg: [u32; 3], transfer: Option<CapNodeId>, regs: *mut SavedRegs) {
    {
        let mut recvs = PENDING_RECVS.lock();
        if let Some(pos) = recvs.iter().position(|r| r.endpoint == endpoint) {
            let matched = recvs.remove(pos);
            drop(recvs);
            // The receiver isn't the currently running task, so a
            // transferred capability has to be installed into *its*
            // CSpace explicitly rather than the current one's.
            let installed_slot = transfer.map(|node| scheduler::install_cap_for(matched.task_id, node)).unwrap_or(0);
            unsafe {
                (*matched.regs).eax = self_id as u32;
                (*matched.regs).ebx = msg[0];
                (*matched.regs).ecx = msg[1];
                (*matched.regs).edx = msg[2];
                (*matched.regs).edi = installed_slot;
                (*regs).eax = 0;
            }
            scheduler::wake(matched.task_id);
            return;
        }
    }

    PENDING_SENDS.lock().push(PendingSend {
        task_id: self_id,
        endpoint,
        msg,
        transfer,
        regs,
    });
    scheduler::block_current();
    // Resumed only once a matching recv already wrote our result into
    // `regs` and woke us -- nothing left to do.
}

/// Takes the next message addressed to `endpoint`: immediately if a
/// matching `send` (or a queued interrupt notification registered for this
/// same endpoint -- see `notify_interrupt`) is already waiting, otherwise
/// blocks the caller until one arrives.
pub fn recv(self_id: TaskId, endpoint: EndpointId, regs: *mut SavedRegs) {
    // Un-masks any IRQ registered to this endpoint (a no-op for the
    // ordinary case of an endpoint with none) -- calling `recv` again is
    // this task's own way of saying "I'm ready for another," including,
    // for a level-triggered PCI interrupt, having actually cleared the
    // device's own pending condition first. See `irq::dispatch`'s
    // masking for why this pairing exists at all.
    for irq in crate::irq::irqs_for_endpoint(endpoint) {
        crate::pic::unmask(irq as u8);
    }

    {
        let mut sends = PENDING_SENDS.lock();
        if let Some(pos) = sends.iter().position(|s| s.endpoint == endpoint) {
            let matched = sends.remove(pos);
            drop(sends);
            // Here, unlike in send's matching branch above, the receiver
            // *is* the currently running task (recv is what just found
            // this match), so a transferred capability lands directly in
            // its own CSpace.
            let installed_slot = matched.transfer.map(scheduler::current_cspace_install).unwrap_or(0);
            unsafe {
                (*regs).eax = matched.task_id as u32;
                (*regs).ebx = matched.msg[0];
                (*regs).ecx = matched.msg[1];
                (*regs).edx = matched.msg[2];
                (*regs).edi = installed_slot;
                (*matched.regs).eax = 0;
            }
            scheduler::wake(matched.task_id);
            return;
        }
    }

    {
        let mut events = PENDING_IRQ_EVENTS.lock();
        if let Some(pos) = events.iter().position(|e| e.endpoint == endpoint) {
            let event = events.remove(pos);
            drop(events);
            unsafe {
                (*regs).eax = KERNEL_TASK_ID as u32;
                (*regs).ebx = event.irq;
                (*regs).ecx = event.data;
                (*regs).edx = 0;
            }
            return;
        }
    }

    PENDING_RECVS.lock().push(PendingRecv {
        task_id: self_id,
        endpoint,
        regs,
    });
    scheduler::block_current();
    // Resumed only once a matching send/notify_interrupt already wrote our
    // result into `regs` and woke us -- nothing left to do.
}

/// Forwards a hardware interrupt to whoever holds `endpoint` (registered
/// via the register_for_interrupt syscall -- see irq.rs): delivers
/// immediately if a task is already blocked in a matching `recv`,
/// otherwise queues the event so the next matching `recv` sees it right
/// away. Never blocks -- this is called directly from interrupt context
/// (see keyboard.rs), which must ack and return promptly, not suspend
/// waiting for a receiver.
pub fn notify_interrupt(endpoint: EndpointId, irq: u32, data: u32) {
    let mut recvs = PENDING_RECVS.lock();
    if let Some(pos) = recvs.iter().position(|r| r.endpoint == endpoint) {
        let matched = recvs.remove(pos);
        drop(recvs);
        unsafe {
            (*matched.regs).eax = KERNEL_TASK_ID as u32;
            (*matched.regs).ebx = irq;
            (*matched.regs).ecx = data;
            (*matched.regs).edx = 0;
        }
        scheduler::wake(matched.task_id);
        return;
    }
    drop(recvs);

    PENDING_IRQ_EVENTS.lock().push(PendingIrqEvent { endpoint, irq, data });
}

/// Called when `task_id` exits: retires every endpoint it owned, waking
/// (with a failure, `eax = u32::MAX`, matching the "unknown syscall"
/// sentinel elsewhere) anything blocked send/recv-ing on one of them --
/// without this, a task waiting on an endpoint whose owner just vanished
/// would stay `Blocked` forever with no error and no wake, a silent
/// permanent hang. A task can't be both `Blocked` and exiting itself
/// (`exit_current` requires being the *running* task), so there's no need
/// to separately clean up entries where `task_id` itself is the blocked
/// party.
pub fn task_exited(task_id: TaskId) {
    let dead_endpoints: Vec<EndpointId> = {
        let endpoints = ENDPOINTS.lock();
        endpoints
            .iter()
            .enumerate()
            .filter(|(_, e)| e.owner == task_id)
            .map(|(id, _)| id)
            .collect()
    };

    let mut recvs = PENDING_RECVS.lock();
    recvs.retain(|r| {
        if dead_endpoints.contains(&r.endpoint) {
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
        if dead_endpoints.contains(&s.endpoint) {
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
    // forever addressed to an endpoint whose owner no longer exists.
    PENDING_IRQ_EVENTS.lock().retain(|e| !dead_endpoints.contains(&e.endpoint));
}
