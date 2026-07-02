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
//! Checkpoint F adds derivation (`mint_derived`, used for both transfer
//! over `send` and explicit re-badging) and revocation on top of
//! Checkpoint E's plain root-capability mint/resolve.

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
    /// A physical range the holder may `map_memory` into their own address
    /// space -- either the VGA buffer (minted once at console_server's
    /// spawn) or fresh anonymous RAM (minted by `SYS_MEM_ALLOC`, then
    /// optionally transferred to a peer as the bulk-data-sharing
    /// primitive later checkpoints use).
    MemoryGrant {
        phys_base: usize,
        len: usize,
        writable: bool,
    },
    /// Permission to register for a specific hardware irq, targeting a
    /// specific endpoint -- bundling both means holding the capability is
    /// itself sufficient authorization for `SYS_REGISTER_IRQ`, no separate
    /// "and are you allowed to pick this irq/endpoint" check needed.
    IrqControl {
        irq: u32,
        endpoint: EndpointId,
    },
}

struct CapNode {
    kind: CapKind,
    /// Opaque value set at derivation time (see `mint_derived`), reported
    /// alongside the resolved kind -- lets a single shared capability be
    /// handed out in distinguishable copies (e.g. one server endpoint,
    /// differently-badged per client). Not yet surfaced to userspace via
    /// IPC (nothing in this phase's protocols needs a client to *see* its
    /// own badge over the wire); it exists now so revocation and transfer
    /// have real values to carry, ready for whenever a protocol does.
    badge: u32,
    children: Vec<CapNodeId>,
    revoked: bool,
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
    nodes.push(CapNode {
        kind,
        badge: 0,
        children: Vec::new(),
        revoked: false,
    });
    id
}

/// Derives a child capability of the same kind from `parent`, tagged with
/// `badge`. Returns `None` if `parent` doesn't exist or has already been
/// revoked (a revoked capability can't be used to mint further children --
/// otherwise revocation could always be defeated by deriving one more copy
/// first). The new node's id still needs to be installed into some task's
/// `CSpace` via `CSpace::install` to actually be reachable.
pub fn mint_derived(parent: CapNodeId, badge: u32) -> Option<CapNodeId> {
    let mut nodes = CAP_NODES.lock();
    let kind = {
        let entry = nodes.get(parent as usize)?;
        if entry.revoked {
            return None;
        }
        entry.kind
    };
    let id = nodes.len() as CapNodeId;
    nodes.push(CapNode {
        kind,
        badge,
        children: Vec::new(),
        revoked: false,
    });
    nodes[parent as usize].children.push(id);
    Some(id)
}

/// Looks up what a capability node actually is, and its badge. Returns
/// `None` for an unknown or revoked id (there's no way to get an unknown
/// one without going through `mint_root`/`mint_derived`, but syscalls
/// resolve untrusted slot numbers, so every caller must treat this as
/// fallible).
pub fn resolve(node: CapNodeId) -> Option<(CapKind, u32)> {
    let nodes = CAP_NODES.lock();
    let entry = nodes.get(node as usize)?;
    if entry.revoked {
        None
    } else {
        Some((entry.kind, entry.badge))
    }
}

/// Revokes `node` and every capability derived from it (transitively),
/// marking them so `resolve`/`mint_derived` stop honoring them. Doesn't
/// reach into any task's `CSpace` to remove the now-dead slot entries --
/// a stale slot pointing at a revoked node just starts resolving to `None`
/// (indistinguishable from an empty slot) the next time it's used, which
/// is enough: nothing in this kernel ever iterates "everyone who might be
/// holding capability X" the way a slot-nulling approach would need to.
///
/// Iterative (an explicit work-stack), not recursive -- a derivation chain
/// is attacker-influenced-depth in principle (every `mint_derived` call
/// adds one more link), and this must never blow the kernel stack.
pub fn revoke(node: CapNodeId) {
    let mut nodes = CAP_NODES.lock();
    let mut stack = alloc::vec![node];
    while let Some(id) = stack.pop() {
        let Some(entry) = nodes.get_mut(id as usize) else {
            continue;
        };
        if entry.revoked {
            continue; // already processed -- also guards against any cycle
        }
        entry.revoked = true;
        stack.extend(entry.children.iter().copied());
    }
}
