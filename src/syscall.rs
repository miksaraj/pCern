use core::arch::global_asm;

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
const SYS_DEBUG_WRITE: u32 = 5;
const SYS_REGISTER_IRQ: u32 = 6;
const SYS_MAP_MEMORY: u32 = 7;
const SYS_CREATE_TASK: u32 = 8;

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
        SYS_SEND => {
            let dest = regs.ebx as usize;
            let msg = [regs.ecx, regs.edx, regs.esi, regs.edi];
            ipc::send(self_id, dest, msg, regs as *mut SavedRegs);
        }
        SYS_RECV => {
            let filter = if regs.ebx == 0 { None } else { Some(regs.ebx as usize) };
            ipc::recv(self_id, filter, regs as *mut SavedRegs);
        }
        SYS_GETPID => regs.eax = self_id as u32,
        SYS_DEBUG_WRITE => {
            sys_debug_write(regs.ebx as *const u8, regs.ecx as usize);
            regs.eax = 0;
        }
        SYS_REGISTER_IRQ => {
            regs.eax = if scheduler::current_is_driver() && irq::register(regs.ebx, self_id) {
                0
            } else {
                ERR
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

/// Prints a user-supplied byte slice to the console. Deliberately temporary:
/// real output belongs in a userspace console server, not the kernel, so
/// this exists only to bring up/prove ring 3 before that server exists.
///
/// The pointer/length are validated against the calling task's own page
/// tables before being dereferenced: without this, a task could point ptr
/// at kernel memory (every address space maps the kernel identically) and
/// have the kernel print out arbitrary kernel heap/stack/GDT/IDT contents,
/// or point it at an unmapped address and crash the whole kernel via a
/// kernel-mode page fault.
fn sys_debug_write(ptr: *const u8, len: usize) {
    const MAX_LEN: usize = 512;
    if ptr.is_null() || len == 0 {
        return;
    }
    let len = len.min(MAX_LEN);
    if !crate::mm::paging::current_range_is_user_accessible(ptr as usize, len) {
        return;
    }
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    if let Ok(s) = core::str::from_utf8(bytes) {
        crate::print!("{}", s);
    }
}
