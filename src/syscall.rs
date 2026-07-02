use core::arch::global_asm;

use crate::cap::{self, CapKind};
use crate::ipc;
use crate::irq;
use crate::loader;
use crate::mm;
use crate::scheduler;

global_asm!(include_str!("syscall_asm.s"));

extern "C" {
    pub fn syscall_isr();
}

const SYS_EXIT: u32 = 0;
const SYS_YIELD: u32 = 1;
const SYS_SEND: u32 = 2;
const SYS_RECV: u32 = 3;
const SYS_GETPID: u32 = 4;
// 5 was SYS_DEBUG_WRITE, retired now that the console server (Checkpoint D)
// owns all real console output; left unassigned rather than renumbering
// everything after it.
const SYS_REGISTER_IRQ: u32 = 6;
const SYS_MAP_MEMORY: u32 = 7;
const SYS_CREATE_TASK: u32 = 8;
const SYS_ENDPOINT_CREATE: u32 = 9;
const SYS_CAP_MINT_BADGED: u32 = 10;
const SYS_CAP_REVOKE: u32 = 11;

/// Error sentinel returned in `eax` by the privileged syscalls
/// (register_for_interrupt, map_memory) when the caller isn't
/// driver-flagged or the request is otherwise invalid. Real task ids and
/// successful map_memory results (0) never collide with this.
const ERR: u32 = u32::MAX;

/// The GP registers syscall_isr (syscall_asm.s) saves, in the exact order
/// it pushes them (so this struct overlays that stack memory field-for-
/// field). `eax` carries the syscall number in, and every field doubles as
/// a return-value slot: whatever's here when syscall_dispatch returns is
/// what the caller sees restored in that register after `int 0x80`.
#[repr(C)]
pub struct SavedRegs {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
}

/// The capability node a slot in the *current* task's own `CSpace` refers
/// to, if any -- the raw node id, for callers that need to derive from or
/// revoke it rather than just use it (see `resolve_current_cap` for the
/// common "just tell me what it is" case).
fn current_cap_node(slot: u32) -> Option<cap::CapNodeId> {
    scheduler::current_cspace_get(slot)
}

/// Resolves a capability slot in the *current* task's own `CSpace` to its
/// kind and badge. Every syscall that takes a capability argument goes
/// through this -- an unrecognized, empty, or revoked slot just resolves
/// to `None`, never panics, since the slot number comes straight from
/// untrusted userspace.
fn resolve_current_cap(slot: u32) -> Option<(CapKind, u32)> {
    cap::resolve(current_cap_node(slot)?)
}

#[no_mangle]
extern "C" fn syscall_dispatch(regs: *mut SavedRegs) {
    let regs = unsafe { &mut *regs };
    let num = regs.eax;
    let self_id = scheduler::current_id().expect("syscall with no current task");

    match num {
        SYS_EXIT => scheduler::exit_current(regs.ebx as i32),
        SYS_YIELD => {
            scheduler::yield_now();
            regs.eax = 0;
        }
        SYS_SEND => match resolve_current_cap(regs.ebx) {
            Some((CapKind::Endpoint { id }, _badge)) => {
                let msg = [regs.ecx, regs.edx, regs.esi];
                // `edi` optionally names a capability (in the *caller's*
                // CSpace) to hand to whoever receives this message. `0`
                // (or anything that doesn't resolve/is revoked) just means
                // "no transfer" -- an invalid transfer slot doesn't abort
                // an otherwise-valid send, it only means the message
                // arrives without one. The derived child is minted now,
                // at send time, with the parent's own badge carried
                // forward unchanged (a plain hand-off, not a re-badge --
                // that's what SYS_CAP_MINT_BADGED is for, done locally
                // before transferring the result).
                let transfer = if regs.edi != 0 {
                    current_cap_node(regs.edi)
                        .and_then(|src| cap::resolve(src).map(|(_, badge)| (src, badge)))
                        .and_then(|(src, badge)| cap::mint_derived(src, badge))
                } else {
                    None
                };
                ipc::send(self_id, id, msg, transfer, regs as *mut SavedRegs);
            }
            _ => regs.eax = ERR,
        },
        SYS_RECV => match resolve_current_cap(regs.ebx) {
            Some((CapKind::Endpoint { id }, _badge)) => ipc::recv(self_id, id, regs as *mut SavedRegs),
            _ => regs.eax = ERR,
        },
        SYS_GETPID => regs.eax = self_id as u32,
        SYS_REGISTER_IRQ => {
            // `ebx`=irq number, `ecx`=capability slot for the endpoint to
            // target. Still gated by the same is_driver bool as before --
            // Checkpoint G tightens this to a real per-irq IrqControl
            // capability instead of "any driver can register for any irq".
            let irq_num = regs.ebx;
            regs.eax = match (scheduler::current_is_driver(), resolve_current_cap(regs.ecx)) {
                (true, Some((CapKind::Endpoint { id }, _badge))) if irq::register(irq_num, id) => 0,
                _ => ERR,
            };
        }
        SYS_MAP_MEMORY => {
            regs.eax = if scheduler::current_is_driver() {
                sys_map_memory(regs.ebx as usize, regs.ecx as usize, regs.edx as usize)
            } else {
                ERR
            };
        }
        SYS_CREATE_TASK => {
            let module_index = regs.ebx as usize;
            // Always spawned as an ordinary (non-driver) task, regardless
            // of whether the caller itself is one -- privilege is never
            // delegable through this syscall, only grantable by the
            // kernel's own internal spawn code (see loader.rs/main.rs).
            regs.eax = match loader::spawn_from_module(module_index, false, &[]) {
                Some(id) => id as u32,
                None => 0,
            };
        }
        SYS_ENDPOINT_CREATE => {
            let endpoint = ipc::create_endpoint(self_id);
            let node = cap::mint_root(CapKind::Endpoint { id: endpoint });
            regs.eax = scheduler::current_cspace_install(node);
        }
        SYS_CAP_MINT_BADGED => {
            // `ebx`=source capability slot, `ecx`=badge for the new copy.
            // Purely local -- installs the derived capability into the
            // *caller's own* CSpace (to be handed to someone else via a
            // send's transfer slot, if that's the point of re-badging it).
            regs.eax = match current_cap_node(regs.ebx).and_then(|src| cap::mint_derived(src, regs.ecx)) {
                Some(node) => scheduler::current_cspace_install(node),
                None => ERR,
            };
        }
        SYS_CAP_REVOKE => {
            // `ebx`=capability slot to revoke. Revokes that capability and
            // everything derived from it (see cap::revoke) -- an unknown
            // or already-empty slot is simply a no-op, not an error, since
            // the end state ("this slot doesn't grant anything") is the
            // same either way.
            if let Some(node) = current_cap_node(regs.ebx) {
                cap::revoke(node);
            }
            regs.eax = 0;
        }
        _ => regs.eax = ERR,
    }
}

/// Maps `len` bytes of physical memory at `phys_addr` into the calling
/// (already-verified driver) task's own address space at `virt_addr`.
/// Restricted to a small allowlist of known-safe MMIO ranges (just the VGA
/// text buffer for now) -- without that, "driver" would be equivalent to
/// unrestricted physical memory access, i.e. a full privilege escalation
/// for the one flag this kernel currently grants at all.
fn sys_map_memory(phys_addr: usize, virt_addr: usize, len: usize) -> u32 {
    const VGA_BUFFER_PHYS: usize = 0xB8000;
    const VGA_BUFFER_LEN: usize = 0x1000;

    if phys_addr % mm::frame::FRAME_SIZE != 0 || virt_addr % mm::frame::FRAME_SIZE != 0 || len == 0 {
        return ERR;
    }
    let Some(phys_end) = phys_addr.checked_add(len) else {
        return ERR;
    };
    let in_allowlist = phys_addr >= VGA_BUFFER_PHYS && phys_end <= VGA_BUFFER_PHYS + VGA_BUFFER_LEN;
    if !in_allowlist {
        return ERR;
    }

    let mut page_dir = mm::paging::PageDirectory::from_phys(scheduler::current_page_dir_phys());
    let pages = len.div_ceil(mm::frame::FRAME_SIZE);
    for i in 0..pages {
        let offset = i * mm::frame::FRAME_SIZE;
        page_dir.map_page(virt_addr + offset, phys_addr + offset, true, true);
    }
    0
}

