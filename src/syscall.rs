use core::arch::global_asm;

use crate::ipc;
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
        _ => regs.eax = u32::MAX,
    }
}

/// Prints a user-supplied byte slice to the console. Deliberately temporary:
/// real output belongs in a userspace console server, not the kernel, so
/// this exists only to bring up/prove ring 3 before that server exists.
/// Validation is light (length cap, non-null) rather than a full page-table
/// walk proving the range is mapped and user-readable -- acceptable for a
/// syscall slated for removal, not for a permanent one.
fn sys_debug_write(ptr: *const u8, len: usize) {
    const MAX_LEN: usize = 512;
    if ptr.is_null() || len == 0 {
        return;
    }
    let len = len.min(MAX_LEN);
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    if let Ok(s) = core::str::from_utf8(bytes) {
        crate::print!("{}", s);
    }
}
