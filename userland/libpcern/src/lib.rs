//! Shared syscall bindings for pCern userspace programs -- one copy
//! instead of copy-pasting this into every userland crate (console_server
//! was the first and only one through Checkpoint D; Checkpoint E onward
//! adds more). See src/syscall.rs, src/cap.rs and src/ipc.rs in the kernel
//! for the authoritative ABI this mirrors.
//!
//! `eax` carries the syscall number in and the primary result out.
//! `ebx`/`ecx`/`edx`/`esi` carry up to four more arguments in; `send`/
//! `recv` use `ebx` for a capability slot (not a raw task id -- see
//! cap.rs's CSpace in the kernel) and `ecx`/`edx`/`esi` for a 3-word
//! message. `edi` carries a capability slot to transfer on `send` and
//! reports one that was transferred to you on `recv` (`0` = none either
//! way; unused/always 0 until Checkpoint F actually implements transfer).

#![no_std]

use core::arch::global_asm;

global_asm!(include_str!("syscall_asm.s"));

extern "C" {
    fn syscall_raw_asm(num: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32, out: *mut RawResult);
}

pub const SYS_EXIT: u32 = 0;
pub const SYS_YIELD: u32 = 1;
pub const SYS_SEND: u32 = 2;
pub const SYS_RECV: u32 = 3;
pub const SYS_GETPID: u32 = 4;
pub const SYS_REGISTER_IRQ: u32 = 6;
pub const SYS_MAP_MEMORY: u32 = 7;
pub const SYS_CREATE_TASK: u32 = 8;
pub const SYS_ENDPOINT_CREATE: u32 = 9;

/// Reserved sender id `recv` reports for interrupts the kernel forwards
/// (see src/ipc.rs's KERNEL_TASK_ID in the kernel) -- never a real task.
pub const KERNEL_TASK_ID: u32 = 0;

/// Every register the kernel's syscall ABI might write on return, captured
/// unconditionally by the asm trampoline regardless of which ones a given
/// syscall actually uses.
#[repr(C)]
struct RawResult {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
    esi: u32,
    edi: u32,
}

/// The register-pinned `int 0x80` trampoline lives in `syscall_asm.s`
/// rather than here as a Rust `asm!` block -- see that file's header
/// comment for why (LLVM reserves `esi` in ordinary function bodies).
unsafe fn syscall_raw(num: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32) -> RawResult {
    let mut out = RawResult { eax: 0, ebx: 0, ecx: 0, edx: 0, esi: 0, edi: 0 };
    syscall_raw_asm(num, a1, a2, a3, a4, a5, &mut out);
    out
}

pub fn exit(code: i32) -> ! {
    unsafe { syscall_raw(SYS_EXIT, code as u32, 0, 0, 0, 0) };
    unreachable!("sys_exit returned")
}

#[allow(dead_code)]
pub fn yield_now() {
    unsafe { syscall_raw(SYS_YIELD, 0, 0, 0, 0, 0) };
}

/// Returns 0 on success. `dest_slot` is a capability slot (see cap.rs's
/// CSpace in the kernel), not a raw task id -- the kernel checks it
/// actually resolves to an Endpoint the caller holds before doing anything.
pub fn send(dest_slot: u32, w0: u32, w1: u32, w2: u32) -> i32 {
    unsafe { syscall_raw(SYS_SEND, dest_slot, w0, w1, w2, 0) }.eax as i32
}

pub struct RecvResult {
    pub sender: u32,
    pub w0: u32,
    pub w1: u32,
    pub w2: u32,
}

/// `endpoint_slot`: a capability slot resolving to the Endpoint to wait
/// on. There's no more "filter by sender" argument -- selectivity comes
/// entirely from which capability you were handed, not a runtime filter.
pub fn recv(endpoint_slot: u32) -> RecvResult {
    let r = unsafe { syscall_raw(SYS_RECV, endpoint_slot, 0, 0, 0, 0) };
    RecvResult {
        sender: r.eax,
        w0: r.ebx,
        w1: r.ecx,
        w2: r.edx,
    }
}

#[allow(dead_code)]
pub fn getpid() -> u32 {
    unsafe { syscall_raw(SYS_GETPID, 0, 0, 0, 0, 0) }.eax
}

/// Returns 0 on success, nonzero if the caller isn't driver-flagged or
/// `endpoint_slot` isn't a valid capability.
#[allow(dead_code)]
pub fn register_irq(irq: u32, endpoint_slot: u32) -> i32 {
    unsafe { syscall_raw(SYS_REGISTER_IRQ, irq, endpoint_slot, 0, 0, 0) }.eax as i32
}

/// Returns 0 on success, nonzero if the caller isn't driver-flagged or the
/// physical range isn't on the kernel's MMIO allowlist.
pub fn map_memory(phys_addr: u32, virt_addr: u32, len: u32) -> i32 {
    unsafe { syscall_raw(SYS_MAP_MEMORY, phys_addr, virt_addr, len, 0, 0) }.eax as i32
}

/// Returns the new task's id, or 0 if `module_index` doesn't exist.
#[allow(dead_code)]
pub fn create_task(module_index: u32) -> u32 {
    unsafe { syscall_raw(SYS_CREATE_TASK, module_index, 0, 0, 0, 0) }.eax
}

/// Mints a new endpoint owned by the caller and installs a capability to
/// it in the caller's own CSpace. Returns the slot it landed in (`0` on
/// failure, though this syscall never actually fails today).
#[allow(dead_code)]
pub fn endpoint_create() -> u32 {
    unsafe { syscall_raw(SYS_ENDPOINT_CREATE, 0, 0, 0, 0, 0) }.eax
}
