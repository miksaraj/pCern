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
//! message. `edi` carries a capability slot to transfer on `send` (`0` =
//! none) and reports one that was transferred to you on `recv` (`0` =
//! none).

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
pub const SYS_CAP_MINT_BADGED: u32 = 10;
pub const SYS_CAP_REVOKE: u32 = 11;
pub const SYS_MEM_ALLOC: u32 = 12;

/// Reserved sender id `recv` reports for interrupts the kernel forwards
/// (see src/ipc.rs's KERNEL_TASK_ID in the kernel) -- never a real task.
pub const KERNEL_TASK_ID: u32 = 0;

/// Checkpoint H: every task spawned via `loader::spawn_from_module` in the
/// kernel (main.rs's own spawns, and any task created later via
/// `create_task`) automatically gets a capability to the name service's
/// public endpoint installed at this fixed slot -- the one piece of
/// discovery infrastructure every task can rely on existing without
/// having to be told about it individually, the same way Unix processes
/// implicitly inherit fds 0/1/2.
pub const NAMESERVICE_SLOT: u32 = 1;

/// Name-service wire protocol (see userland/nameservice): `w0`=op,
/// `w1`/`w2`=an 8-byte name packed via `pack_name`.
pub const NS_OP_REGISTER: u32 = 1;
pub const NS_OP_LOOKUP: u32 = 2;

/// Storage-service wire protocol (see userland/storage_ata). A client
/// connects once (`storage_connect`) -- handing over a shared page via
/// `SYS_MEM_ALLOC`/transfer for the driver to read sectors into, and its
/// own inbox as the reply-to address, since the 3-word/1-transfer budget
/// of a single message can't carry both at once -- then issues any number
/// of `STORAGE_OP_READ_BLOCK` requests against that connection. Only one
/// client is supported at a time (see storage_ata's `main.rs`): fine for
/// this phase, where `fs_fat32` is the only caller.
pub const STORAGE_OP_SET_BUFFER: u32 = 1;
pub const STORAGE_OP_SET_REPLY: u32 = 2;
pub const STORAGE_OP_READ_BLOCK: u32 = 3;
pub const STORAGE_SECTOR_SIZE: usize = 512;

/// Establishes a connection to the storage service: hands it the shared
/// page backing `buf_grant_slot` (already mapped locally by the caller,
/// typically via `mem_alloc`) to read sectors into, and `my_inbox_slot` as
/// where to send read replies. Must be called before `storage_read_block`.
#[allow(dead_code)]
pub fn storage_connect(storage_slot: u32, buf_grant_slot: u32, my_inbox_slot: u32) {
    send(storage_slot, STORAGE_OP_SET_BUFFER, 0, 0, buf_grant_slot);
    send(storage_slot, STORAGE_OP_SET_REPLY, 0, 0, my_inbox_slot);
}

/// Reads sector `lba` into the shared buffer previously established by
/// `storage_connect`. Returns `true` on success; the bytes are visible at
/// whatever local virtual address the caller mapped `buf_grant_slot` to.
#[allow(dead_code)]
pub fn storage_read_block(storage_slot: u32, my_inbox_slot: u32, lba: u32) -> bool {
    send(storage_slot, STORAGE_OP_READ_BLOCK, lba, 0, 0);
    recv(my_inbox_slot).w0 == 1
}

/// Packs up to 8 bytes of `name` into two little-endian u32 words,
/// zero-padded if shorter (longer names are truncated to 8 bytes). Used
/// by both sides of the name-service protocol so the encoding only lives
/// in one place.
pub fn pack_name(name: &[u8]) -> (u32, u32) {
    let mut buf = [0u8; 8];
    let n = name.len().min(8);
    buf[..n].copy_from_slice(&name[..n]);
    (
        u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
        u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
    )
}

/// Registers `my_endpoint_slot` (typically the caller's own inbox) under
/// `name` with the name service. Fire-and-forget -- the name service
/// checks the caller's kernel-attested task id against its own allowlist
/// and silently drops the request if it isn't authorized to claim that
/// name, so there's nothing meaningful to report back here.
pub fn register_name(name: &[u8], my_endpoint_slot: u32) {
    let (w0, w1) = pack_name(name);
    send(NAMESERVICE_SLOT, NS_OP_REGISTER, w0, w1, my_endpoint_slot);
}

/// Looks up `name` via the name service, blocking until it replies.
/// `my_inbox_slot` is transferred to the name service as the reply-to
/// address (it has no other way to reach the caller back). Returns a
/// freshly-installed capability slot in the caller's own CSpace if found,
/// `None` otherwise.
#[allow(dead_code)]
pub fn lookup_name(name: &[u8], my_inbox_slot: u32) -> Option<u32> {
    let (w0, w1) = pack_name(name);
    send(NAMESERVICE_SLOT, NS_OP_LOOKUP, w0, w1, my_inbox_slot);
    let r = recv(my_inbox_slot);
    if r.w0 == 1 {
        Some(r.transferred_slot)
    } else {
        None
    }
}

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
/// `transfer_slot` (`0` = none) optionally hands a capability from the
/// caller's own CSpace to whoever receives this message (see cap.rs's
/// mint_derived in the kernel) -- an invalid transfer slot doesn't fail
/// the send, the message just arrives without one.
#[allow(dead_code)]
pub fn send(dest_slot: u32, w0: u32, w1: u32, w2: u32, transfer_slot: u32) -> i32 {
    unsafe { syscall_raw(SYS_SEND, dest_slot, w0, w1, w2, transfer_slot) }.eax as i32
}

pub struct RecvResult {
    pub sender: u32,
    pub w0: u32,
    pub w1: u32,
    pub w2: u32,
    /// A capability slot in *this task's own* CSpace, freshly installed
    /// because the sender named a transfer -- `0` if none did.
    pub transferred_slot: u32,
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
        transferred_slot: r.edi,
    }
}

#[allow(dead_code)]
pub fn getpid() -> u32 {
    unsafe { syscall_raw(SYS_GETPID, 0, 0, 0, 0, 0) }.eax
}

/// Returns 0 on success, nonzero if `irq_control_slot` doesn't resolve to
/// a valid IrqControl capability (which itself bundles which irq and
/// which endpoint to target -- see cap.rs in the kernel).
#[allow(dead_code)]
pub fn register_irq(irq_control_slot: u32) -> i32 {
    unsafe { syscall_raw(SYS_REGISTER_IRQ, irq_control_slot, 0, 0, 0, 0) }.eax as i32
}

/// Maps the physical range described by `grant_slot` (a MemoryGrant
/// capability -- see cap.rs in the kernel) into the caller's own address
/// space at `virt_addr`. Returns 0 on success, nonzero if `grant_slot`
/// isn't a valid MemoryGrant.
pub fn map_memory(grant_slot: u32, virt_addr: u32) -> i32 {
    unsafe { syscall_raw(SYS_MAP_MEMORY, grant_slot, virt_addr, 0, 0, 0) }.eax as i32
}

/// Allocates one fresh physical page, maps it into the caller's own
/// address space at `virt_addr`, and returns a capability slot for a
/// MemoryGrant describing it (`0` on failure) -- which can then be handed
/// to a peer task (via `send`'s transfer slot) so it can map the *same*
/// physical page into its own space too.
#[allow(dead_code)]
pub fn mem_alloc(virt_addr: u32) -> u32 {
    unsafe { syscall_raw(SYS_MEM_ALLOC, virt_addr, 0, 0, 0, 0) }.eax
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

/// Derives a badged copy of the capability in `source_slot`, installed
/// into the *caller's own* CSpace (typically so it can then be handed to
/// someone else via `send`'s transfer slot). Returns the new slot, or `0`
/// if `source_slot` didn't resolve to anything (or was already revoked).
#[allow(dead_code)]
pub fn cap_mint_badged(source_slot: u32, badge: u32) -> u32 {
    unsafe { syscall_raw(SYS_CAP_MINT_BADGED, source_slot, badge, 0, 0, 0) }.eax
}

/// Revokes the capability in `slot` and everything derived from it --
/// after this, every copy (in any task's CSpace) stops working. A no-op
/// (not an error) if `slot` was already empty or invalid.
#[allow(dead_code)]
pub fn cap_revoke(slot: u32) {
    unsafe { syscall_raw(SYS_CAP_REVOKE, slot, 0, 0, 0, 0) };
}
