//! Raw `int 0x80` wrappers matching the kernel's syscall ABI (see
//! src/syscall.rs and src/ipc.rs in the kernel): `eax` carries the syscall
//! number in and the primary result out; `ebx`/`ecx`/`edx`/`esi` carry up
//! to four more arguments in and, for send/recv, the rest of the result
//! out. `edi` is write-only from userspace's side (nothing the kernel
//! returns today uses it).

use core::arch::global_asm;

global_asm!(include_str!("syscall_asm.s"));

extern "C" {
    fn syscall_raw_asm(num: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32, out: *mut RawResult);
}

const SYS_EXIT: u32 = 0;
const SYS_YIELD: u32 = 1;
const SYS_SEND: u32 = 2;
const SYS_RECV: u32 = 3;
const SYS_GETPID: u32 = 4;
const SYS_REGISTER_IRQ: u32 = 6;
const SYS_MAP_MEMORY: u32 = 7;
const SYS_CREATE_TASK: u32 = 8;

/// Reserved sender id `recv` reports for interrupts the kernel forwards
/// (see src/ipc.rs's KERNEL_TASK_ID in the kernel) -- never a real task.
pub const KERNEL_TASK_ID: u32 = 0;

#[repr(C)]
struct RawResult {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
    esi: u32,
}

/// The register-pinned `int 0x80` trampoline lives in `syscall_asm.s`
/// rather than here as a Rust `asm!` block -- see that file's header
/// comment for why (LLVM reserves `esi` in ordinary function bodies).
unsafe fn syscall_raw(num: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32) -> RawResult {
    let mut out = RawResult { eax: 0, ebx: 0, ecx: 0, edx: 0, esi: 0 };
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

/// Returns 0 on success.
#[allow(dead_code)]
pub fn send(dest: u32, w0: u32, w1: u32, w2: u32, w3: u32) -> i32 {
    unsafe { syscall_raw(SYS_SEND, dest, w0, w1, w2, w3) }.eax as i32
}

#[allow(dead_code)]
pub struct RecvResult {
    pub sender: u32,
    pub w0: u32,
    pub w1: u32,
    pub w2: u32,
    pub w3: u32,
}

/// `filter`: 0 means "any sender or the kernel" (the same reserved id
/// used for forwarded interrupts); a nonzero value waits only for that
/// specific task id.
pub fn recv(filter: u32) -> RecvResult {
    let r = unsafe { syscall_raw(SYS_RECV, filter, 0, 0, 0, 0) };
    RecvResult {
        sender: r.eax,
        w0: r.ebx,
        w1: r.ecx,
        w2: r.edx,
        w3: r.esi,
    }
}

#[allow(dead_code)]
pub fn getpid() -> u32 {
    unsafe { syscall_raw(SYS_GETPID, 0, 0, 0, 0, 0) }.eax
}

/// Returns 0 on success, nonzero if the caller isn't driver-flagged or
/// `irq` is out of range.
pub fn register_irq(irq: u32) -> i32 {
    unsafe { syscall_raw(SYS_REGISTER_IRQ, irq, 0, 0, 0, 0) }.eax as i32
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
