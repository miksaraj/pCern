//! The kernel-side capability table: the single source of truth for what a
//! task's capability-table slot (`CSlot`) actually refers to. Every task
//! has its own `CSpace` (see task.rs) mapping small per-task slot indices
//! to entries in this global table -- the slot number itself is
//! meaningless outside the task that holds it, and the kernel is the only
//! thing that ever writes a `CSpace` entry, so slots are unforgeable the
//! same way file descriptors are: you can't turn an arbitrary integer into
//! a capability by guessing, you can only use one the kernel actually gave
//! you.
//!
//! Checkpoint E only mints root capabilities (no parent) and resolves
//! them; derivation (transfer/badging) and revocation are Checkpoint F.

use alloc::vec::Vec;

use crate::sync::Mutex;

/// Index into a task's own `CSpace` (see task.rs). `0` is always empty --
/// mirrors the "0 is a reserved sentinel" convention `ipc::KERNEL_TASK_ID`
/// already uses for task ids.
pub type CSlot = u32;

/// Index into the global `CAP_NODES` table below.
pub type CapNodeId = u32;

/// Index into `ipc.rs`'s endpoint table.
pub type EndpointId = usize;

#[derive(Clone, Copy)]
pub enum CapKind {
    Endpoint {
        id: EndpointId,
    },
    /// Wired up starting Checkpoint G (VGA buffer, shared bulk-transfer
    /// pages); the variant exists now so its shape doesn't need to change
    /// once something actually constructs one.
    #[allow(dead_code)]
    MemoryGrant {
        phys_base: usize,
        len: usize,
        writable: bool,
    },
    /// Wired up starting Checkpoint G.
    #[allow(dead_code)]
    IrqControl {
        irq: u32,
        endpoint: EndpointId,
    },
}

struct CapNode {
    kind: CapKind,
}

static CAP_NODES: Mutex<Vec<CapNode>> = Mutex::new(Vec::new());

/// Mints a fresh capability with no parent (i.e. not derived from another
/// capability -- only trusted kernel-side code calls this today: the
/// `SYS_ENDPOINT_CREATE` syscall handler, and main.rs's own boot-time
/// wiring). Returns the new node's id, to be installed into some task's
/// `CSpace` via `CSpace::install`.
pub fn mint_root(kind: CapKind) -> CapNodeId {
    let mut nodes = CAP_NODES.lock();
    let id = nodes.len() as CapNodeId;
    nodes.push(CapNode { kind });
    id
}

/// Looks up what a capability node actually is. Returns `None` for an
/// unknown id (there's no way to get one of those without going through
/// `mint_root`/`CSpace`, but syscalls resolve untrusted slot numbers, so
/// every caller must treat this as fallible).
pub fn resolve(node: CapNodeId) -> Option<CapKind> {
    CAP_NODES.lock().get(node as usize).map(|n| n.kind)
}
