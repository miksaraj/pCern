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
const SYS_MEM_ALLOC: u32 = 12;
const SYS_SPAWN_FROM_MEMORY: u32 = 13;

/// Error sentinel returned in `eax` when a capability argument doesn't
/// resolve to what a syscall needed, or the request is otherwise invalid.
/// Real task ids and successful map_memory results (0) never collide with
/// this.
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
            // `ebx`=capability slot holding an IrqControl (bundles which
            // irq and which endpoint to target -- see cap.rs). Holding a
            // valid one is itself sufficient authorization, the same way
            // holding a MemoryGrant is for SYS_MAP_MEMORY below: only
            // trusted spawn code (main.rs) ever mints one.
            regs.eax = match resolve_current_cap(regs.ebx) {
                Some((CapKind::IrqControl { irq: irq_num, endpoint }, _badge)) if irq::register(irq_num, endpoint) => 0,
                _ => ERR,
            };
        }
        SYS_MAP_MEMORY => {
            // `ebx`=capability slot holding a MemoryGrant, `ecx`=virt_addr
            // to map it at in the caller's own address space.
            regs.eax = match resolve_current_cap(regs.ebx) {
                Some((CapKind::MemoryGrant { phys_base, len, writable }, _badge)) => {
                    sys_map_memory(phys_base, regs.ecx as usize, len, writable)
                }
                _ => ERR,
            };
        }
        SYS_CREATE_TASK => {
            let module_index = regs.ebx as usize;
            // Always spawned with no ports and no capabilities beyond
            // whatever it's granted later -- privilege is never delegable
            // through this syscall, only grantable by the kernel's own
            // internal spawn code (see loader.rs/main.rs).
            regs.eax = match loader::spawn_from_module(module_index, &[]) {
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
        SYS_MEM_ALLOC => {
            // `ebx`=virt_addr to map a freshly allocated page at in the
            // caller's own address space. Returns a capability slot for a
            // MemoryGrant describing that same page, which can then be
            // handed to another task (via send's transfer slot) so it can
            // map the *same* physical page into its own space too -- the
            // bulk data transfer primitive later checkpoints build on.
            // Capped at exactly one page: mm::frame only hands out single
            // frames today, no contiguous-multi-frame allocation exists.
            regs.eax = sys_mem_alloc(regs.ebx as usize);
        }
        SYS_SPAWN_FROM_MEMORY => regs.eax = sys_spawn_from_memory(regs),
        _ => regs.eax = ERR,
    }
}

/// Checkpoint M: loads and runs a program from up to 4 capability slots
/// (`ebx`/`ecx`/`edx`/`esi`, `0` = stop) naming `MemoryGrant` pages the
/// caller already assembled (typically via `SYS_MEM_ALLOC` + a filesystem
/// read), totaling `edi` bytes -- the same privilege ceiling as
/// `SYS_CREATE_TASK`'s existing module-loading path (no ports, no
/// capabilities beyond the universal name-service auto-grant), just with
/// the code coming from caller-supplied memory instead of a multiboot
/// module baked into the boot image.
///
/// The security boundary here is holding each named capability, resolved
/// through the same `resolve_current_cap` machinery every other syscall
/// argument goes through -- not a virt-addr/page-table walk (nothing in
/// this codebase does one, and "this happens to be present|user in my
/// own page tables" would be a weaker property than "I hold a capability
/// actually naming this page" anyway). `loader::spawn_from_memory` always
/// copies these pages' bytes into freshly allocated frames (never maps a
/// resolved grant's physical page directly into the new task), the same
/// way `spawn_from_module` copies a multiboot module's bytes -- so the
/// caller (or anyone else still holding a copy of that grant) can't keep
/// writing to the new task's "code" after it starts running.
///
/// This must stay fully synchronous -- never call anything that can
/// block (`ipc::send`/`recv`) here. `int 0x80` is a 32-bit *interrupt*
/// gate (see idt.rs), which clears `EFLAGS.IF` on entry and is never
/// re-set before this returns, so IRQ0 (the only thing that ever
/// preempts on this single CPU) cannot fire during this call -- a
/// TOCTOU-free window between resolving a grant and copying its bytes,
/// but only as long as this invariant holds.
fn sys_spawn_from_memory(regs: &SavedRegs) -> u32 {
    let slots = [regs.ebx, regs.ecx, regs.edx, regs.esi];
    let mut grants = [0usize; 4];
    let mut n = 0;
    for &slot in &slots {
        if slot == 0 {
            break;
        }
        match resolve_current_cap(slot) {
            Some((CapKind::MemoryGrant { phys_base, .. }, _)) => {
                grants[n] = phys_base;
                n += 1;
            }
            _ => return 0,
        }
    }
    let total_len = regs.edi as usize;
    match loader::spawn_from_memory(&grants[..n], total_len) {
        Some(id) => id as u32,
        None => 0,
    }
}

/// Maps the physical range described by a resolved MemoryGrant capability
/// into the calling task's own address space at `virt_addr`. Holding a
/// valid capability is itself sufficient authorization -- only trusted
/// code (main.rs's boot-time wiring, or this same syscall's own
/// SYS_MEM_ALLOC path) ever mints a MemoryGrant in the first place, so
/// there's no separate allowlist check needed here anymore.
fn sys_map_memory(phys_addr: usize, virt_addr: usize, len: usize, writable: bool) -> u32 {
    if phys_addr % mm::frame::FRAME_SIZE != 0 || virt_addr % mm::frame::FRAME_SIZE != 0 || len == 0 {
        return ERR;
    }
    // virt_addr is entirely caller-chosen -- reject anything that would
    // reach the kernel's own higher half (shared, verbatim, across every
    // task's page directory; see PageDirectory::new). Without this, a task
    // could target e.g. the physmap window and map_page would find an
    // existing 4 MiB PSE mapping already there, which is the caller's own
    // page directory but not a page range this call is allowed to touch.
    let Some(virt_end) = virt_addr.checked_add(len) else {
        return ERR;
    };
    if virt_end > mm::paging::KERNEL_VMA {
        return ERR;
    }
    let mut page_dir = mm::paging::PageDirectory::from_phys(scheduler::current_page_dir_phys());
    let pages = len.div_ceil(mm::frame::FRAME_SIZE);
    for i in 0..pages {
        let offset = i * mm::frame::FRAME_SIZE;
        page_dir.map_page(virt_addr + offset, phys_addr + offset, true, writable);
    }
    0
}

/// Allocates one fresh physical frame, maps it into the caller's own
/// address space at `virt_addr`, and mints a capability describing it.
/// Returns `0` (an empty/invalid slot, indistinguishable from failure --
/// there's nothing sensitive about "you're out of memory" worth a
/// separate error channel here) if `virt_addr` isn't page-aligned or
/// allocation fails.
fn sys_mem_alloc(virt_addr: usize) -> u32 {
    // Same reasoning as sys_map_memory: virt_addr is entirely caller-chosen
    // and must not reach the kernel's own higher half.
    if virt_addr % mm::frame::FRAME_SIZE != 0 || virt_addr >= mm::paging::KERNEL_VMA {
        return 0;
    }
    let Some(phys) = mm::frame::alloc_frame() else {
        return 0;
    };
    let mut page_dir = mm::paging::PageDirectory::from_phys(scheduler::current_page_dir_phys());
    page_dir.map_page(virt_addr, phys, true, true);
    let node = cap::mint_root(CapKind::MemoryGrant {
        phys_base: phys,
        len: mm::frame::FRAME_SIZE,
        writable: true,
    });
    scheduler::current_cspace_install(node)
}

