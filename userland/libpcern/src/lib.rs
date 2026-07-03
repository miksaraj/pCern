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

pub mod editor;

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
pub const SYS_SPAWN_FROM_MEMORY: u32 = 13;

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
/// Writes the sector at `w1` from the shared buffer established by
/// `STORAGE_OP_SET_BUFFER` (Phase 7, Checkpoint P). Same reply shape as
/// `STORAGE_OP_READ_BLOCK`: `w0` = 1 ok / 0 failed.
pub const STORAGE_OP_WRITE_BLOCK: u32 = 4;
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

/// Writes sector `lba` from the shared buffer previously established by
/// `storage_connect` (the caller fills the buffer's bytes locally first).
/// Returns `true` on success.
#[allow(dead_code)]
pub fn storage_write_block(storage_slot: u32, my_inbox_slot: u32, lba: u32) -> bool {
    send(storage_slot, STORAGE_OP_WRITE_BLOCK, lba, 0, 0);
    recv(my_inbox_slot).w0 == 1
}

/// Filesystem-service wire protocol (see userland/fs_fat32). Setup mirrors
/// storage's (`fs_connect`, same SET_BUFFER/SET_REPLY two-message pattern
/// for the same reason -- one transfer per message). Opening a file needs
/// an 11-byte fixed-width 8.3 name (see `fat_pack_name`), one byte more
/// than the 3-word/1-op budget can carry in a single message, so it's
/// split across two ops the same way: `OPEN_NAME1` carries the first 8
/// bytes, `OPEN_NAME2` carries the last 3 and triggers the actual lookup
/// + reply (`w0`=found flag, `w1`=file size). Only one client and one
/// open file at a time, same scope as storage_ata.
pub const FS_OP_SET_BUFFER: u32 = 1;
pub const FS_OP_SET_REPLY: u32 = 2;
pub const FS_OP_OPEN_NAME1: u32 = 3;
/// `w2` (Phase 7, Checkpoint Q) is a "create if missing" flag: 0 = open
/// existing only (the original, unchanged behavior -- every existing
/// caller passes 0 via `fs_open`), 1 = open existing or create a fresh
/// zero-length file. Reply is unchanged: `w0`=opened flag, `w1`=size (0
/// for a brand-new file).
pub const FS_OP_OPEN_NAME2: u32 = 4;
pub const FS_OP_READ: u32 = 5;
/// Writes `w2` bytes at offset `w1` from the shared buffer into the
/// currently open file (Phase 7, Checkpoint Q) -- same partial-transfer
/// contract as `FS_OP_READ` (never crosses a sector boundary, caller
/// loops). Reply `w0` = bytes actually written (0 = no file open, buffer
/// not mapped, or the disk is out of free clusters).
pub const FS_OP_WRITE: u32 = 6;

/// Packs `name` (e.g. `b"HELLO.TXT"`) into FAT's fixed 11-byte 8.3 form:
/// up to 8 bytes before the `.` uppercased and space-padded, then up to 3
/// bytes after it likewise -- directly comparable against the raw 11
/// name bytes of a FAT directory entry.
pub fn fat_pack_name(name: &[u8]) -> [u8; 11] {
    let mut out = [b' '; 11];
    let dot = name.iter().position(|&b| b == b'.');
    let (base, ext) = match dot {
        Some(i) => (&name[..i], &name[i + 1..]),
        None => (name, &[][..]),
    };
    let base_len = base.len().min(8);
    for i in 0..base_len {
        out[i] = base[i].to_ascii_uppercase();
    }
    let ext_len = ext.len().min(3);
    for i in 0..ext_len {
        out[8 + i] = ext[i].to_ascii_uppercase();
    }
    out
}

/// Establishes a connection to the filesystem service, same shape as
/// `storage_connect`.
#[allow(dead_code)]
pub fn fs_connect(fs_slot: u32, buf_grant_slot: u32, my_inbox_slot: u32) {
    send(fs_slot, FS_OP_SET_BUFFER, 0, 0, buf_grant_slot);
    send(fs_slot, FS_OP_SET_REPLY, 0, 0, my_inbox_slot);
}

fn fs_open_impl(fs_slot: u32, my_inbox_slot: u32, name: &[u8], create: bool) -> Option<u32> {
    let packed = fat_pack_name(name);
    let w1 = u32::from_le_bytes([packed[0], packed[1], packed[2], packed[3]]);
    let w2 = u32::from_le_bytes([packed[4], packed[5], packed[6], packed[7]]);
    send(fs_slot, FS_OP_OPEN_NAME1, w1, w2, 0);
    let w1b = u32::from_le_bytes([packed[8], packed[9], packed[10], 0]);
    send(fs_slot, FS_OP_OPEN_NAME2, w1b, if create { 1 } else { 0 }, 0);
    let r = recv(my_inbox_slot);
    if r.w0 == 1 {
        Some(r.w1)
    } else {
        None
    }
}

/// Opens `name` (e.g. `b"HELLO.TXT"`) as the filesystem service's one
/// current file. Returns the file's size in bytes if found.
#[allow(dead_code)]
pub fn fs_open(fs_slot: u32, my_inbox_slot: u32, name: &[u8]) -> Option<u32> {
    fs_open_impl(fs_slot, my_inbox_slot, name, false)
}

/// Opens `name` for writing: same as `fs_open`, but creates a fresh
/// zero-length file if it doesn't already exist (Phase 7, Checkpoint Q).
/// Returns the file's current size (0 for a brand-new file).
#[allow(dead_code)]
pub fn fs_open_for_write(fs_slot: u32, my_inbox_slot: u32, name: &[u8]) -> Option<u32> {
    fs_open_impl(fs_slot, my_inbox_slot, name, true)
}

/// Reads up to `len` bytes at `offset` from the currently open file into
/// the shared buffer established by `fs_connect`. Returns the number of
/// bytes actually placed there (`0` = EOF or no file open) -- may be less
/// than `len`, same partial-read contract as `storage_read_block`'s
/// sector-at-a-time behavior (a read never crosses a sector boundary).
#[allow(dead_code)]
pub fn fs_read(fs_slot: u32, my_inbox_slot: u32, offset: u32, len: u32) -> u32 {
    send(fs_slot, FS_OP_READ, offset, len, 0);
    recv(my_inbox_slot).w0
}

/// Writes `len` bytes at `offset` from the shared buffer (the caller
/// fills it locally first) into the currently open file, growing it if
/// `offset + len` exceeds its current size. Returns the number of bytes
/// actually written (0 = no file open, buffer not mapped, or the disk is
/// out of free clusters) -- same partial-transfer contract as `fs_read`.
#[allow(dead_code)]
pub fn fs_write(fs_slot: u32, my_inbox_slot: u32, offset: u32, len: u32) -> u32 {
    send(fs_slot, FS_OP_WRITE, offset, len, 0);
    recv(my_inbox_slot).w0
}

/// Console *input* wire protocol (see userland/console_server). A reader
/// connects once (`console_connect`) -- handing over a shared page via
/// `SYS_MEM_ALLOC`/transfer for console_server to place a completed
/// line's bytes into, and its own dedicated line-ready endpoint (never
/// shared with any other role -- see CLAUDE.md's "one inbox is not
/// automatically safe for two roles") as the reply-to address -- then
/// issues one `CONSOLE_OP_READ_LINE` request per line it wants to read.
/// Only one reader is supported at a time, the same scope-narrowing
/// precedent as storage_ata's single client -- but worth calling out
/// specifically here: unlike storage_ata's client (fs_fat32, always
/// already blocked in its own `recv` by the time a reply is due),
/// console_server is a system-wide, always-running service. `send`
/// blocks the *sender* until a matching `recv` arrives (see ipc.rs's
/// rendezvous design), so a reader that requests a line and then never
/// calls `recv` would block console_server's entire main loop --
/// starving every other task's `OP_PUTCHAR` and all further keystroke
/// echo, too -- until it does. Acceptable for this phase's single
/// trusted shell client; would need revisiting (e.g. a bounded queue or
/// timeout) if an untrusted reader is ever introduced.
///
/// The first sender to successfully complete `CONSOLE_OP_SET_BUFFER`
/// becomes console_server's one reader for the rest of this boot --
/// every later `CONSOLE_OP_SET_BUFFER`/`SET_READER`/`READ_LINE` from any
/// *other* sender is silently ignored (checked against the
/// kernel-attested sender id, not anything the caller provides). Without
/// this, any task -- including one with no privilege beyond the
/// universal name-service auto-grant, since `console` lookups are open
/// to any caller -- could re-point the connection at itself and receive
/// every keystroke typed afterward instead of the legitimate reader.
pub const CONSOLE_OP_SET_BUFFER: u32 = 1;
pub const CONSOLE_OP_SET_READER: u32 = 2;
pub const CONSOLE_OP_READ_LINE: u32 = 3;
/// Switches the connection between line mode (`w1`=0, the default -- a
/// complete Enter-terminated line at a time, unchanged from Checkpoint L)
/// and raw mode (`w1`=1: every decoded key delivered immediately via
/// `CONSOLE_OP_READ_KEY`, no echo, no line accumulation). Phase 7,
/// Checkpoint R, for the full-screen editor -- gated by the same
/// `reader_owner` ownership check as every other `CONSOLE_OP_*`.
pub const CONSOLE_OP_SET_MODE: u32 = 4;
/// Requests the next decoded key while in raw mode; the reply's `w0` is a
/// tagged value (see `console_server::keyboard`'s `KEY_*` constants for
/// the `>= 256` non-ASCII ones, plain ASCII otherwise). Keys decoded
/// before this request arrives are queued (32 deep) rather than dropped
/// -- a raw-mode redraw's cost scales with how much has been typed so far
/// (see `editor::Editor::redraw`), so several keystrokes arriving while
/// one redraw is still in flight is an expected case, not a rare race.
pub const CONSOLE_OP_READ_KEY: u32 = 5;

/// Bytes typed before Enter beyond this are dropped (not buffered, and
/// not an error) -- bounded well under the shared page's 4096-byte
/// capacity for headroom; see console_server's own accumulator.
pub const CONSOLE_LINE_MAX: usize = 256;

/// Tagged non-ASCII key values a `CONSOLE_OP_READ_KEY` reply's `w0` can
/// carry -- must match `console_server::keyboard`'s `KEY_*` constants
/// exactly, since they're the same values crossing the wire.
pub const KEY_UP: u32 = 256;
pub const KEY_DOWN: u32 = 257;
pub const KEY_LEFT: u32 = 258;
pub const KEY_RIGHT: u32 = 259;
pub const KEY_HOME: u32 = 260;
pub const KEY_END: u32 = 261;
pub const KEY_DELETE: u32 = 262;
pub const KEY_PAGE_UP: u32 = 263;
pub const KEY_PAGE_DOWN: u32 = 264;

/// Establishes a connection to console_server's line-input protocol, same
/// shape as `storage_connect`/`fs_connect`.
#[allow(dead_code)]
pub fn console_connect(console_slot: u32, buf_grant_slot: u32, reader_slot: u32) {
    send(console_slot, CONSOLE_OP_SET_BUFFER, 0, 0, buf_grant_slot);
    send(console_slot, CONSOLE_OP_SET_READER, 0, 0, reader_slot);
}

/// Requests the next typed line and blocks until Enter is pressed.
/// Returns the number of bytes placed in the shared buffer established by
/// `console_connect` (not including the trailing newline) -- may be `0`
/// for an empty line.
#[allow(dead_code)]
pub fn console_read_line(console_slot: u32, reader_slot: u32) -> u32 {
    send(console_slot, CONSOLE_OP_READ_LINE, 0, 0, 0);
    recv(reader_slot).w0
}

/// Switches the connection's input mode (`raw` = true selects raw
/// single-keystroke mode). Must already be connected via
/// `console_connect`.
#[allow(dead_code)]
pub fn console_set_mode(console_slot: u32, raw: bool) {
    send(console_slot, CONSOLE_OP_SET_MODE, if raw { 1 } else { 0 }, 0, 0);
}

/// Requests and blocks for the next decoded key while in raw mode.
/// Returns the tagged key value (plain ASCII `0..=255`, or one of the
/// `KEY_*` constants for a non-ASCII key).
#[allow(dead_code)]
pub fn console_read_key(console_slot: u32, reader_slot: u32) -> u32 {
    send(console_slot, CONSOLE_OP_READ_KEY, 0, 0, 0);
    recv(reader_slot).w0
}

/// Wire protocol other tasks use to reach the screen: `send(console_slot,
/// OP_PUTCHAR, byte, 0, 0)`, one call per character -- see
/// userland/console_server's own README for the full protocol.
pub const OP_PUTCHAR: u32 = 0;

/// Sends `s` to `console_slot` one byte at a time via `OP_PUTCHAR`. Used
/// by every userland program that prints diagnostics directly (rather
/// than through a higher-level protocol), so this one copy is shared
/// instead of being hand-duplicated per crate.
#[allow(dead_code)]
pub fn print(console_slot: u32, s: &[u8]) {
    for &b in s {
        send(console_slot, OP_PUTCHAR, b as u32, 0, 0);
    }
}

/// Prints `n` in decimal via `print`. No sign, no padding -- just enough
/// for the small diagnostic counters/sizes this project's fixtures and
/// shell print (task ids, file sizes, checksums).
#[allow(dead_code)]
pub fn print_u32(console_slot: u32, mut n: u32) {
    if n == 0 {
        print(console_slot, b"0");
        return;
    }
    let mut digits = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    let mut buf = [0u8; 10];
    for j in 0..i {
        buf[j] = digits[i - 1 - j];
    }
    print(console_slot, &buf[..i]);
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

/// Like `lookup_name`, but retries (yielding in between) up to
/// `max_tries` times before giving up. A lookup is a synchronous,
/// point-in-time check against whatever's registered *right now* --
/// there's no queuing for "let me know when this shows up" -- so a
/// client racing a service that needs its own setup time (several IPC
/// round trips of its own, e.g. fs_fat32 connecting to storage_ata
/// before it can register "fs") needs to retry rather than look up once.
#[allow(dead_code)]
pub fn lookup_name_retry(name: &[u8], my_inbox_slot: u32, max_tries: u32) -> Option<u32> {
    for _ in 0..max_tries {
        if let Some(slot) = lookup_name(name, my_inbox_slot) {
            return Some(slot);
        }
        yield_now();
    }
    None
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

/// Checkpoint M: loads and runs a program from up to 4 `MemoryGrant`
/// capability slots (see `SYS_MEM_ALLOC`) the caller has already filled
/// with code bytes (typically read from a file via `fs_read`), totaling
/// `total_len` bytes -- the load-from-memory counterpart to
/// `create_task`'s load-from-module-index. `grant_slots` must have
/// between 1 and 4 entries; the new task gets no privilege beyond the
/// universal name-service auto-grant, the same ceiling `create_task`
/// already enforces. Returns the new task's id, or `0` on failure (an
/// invalid grant slot, or `total_len` not fitting in the pages supplied --
/// each `MemoryGrant` is capped at one page, see cap.rs in the kernel).
#[allow(dead_code)]
pub fn spawn_from_memory(grant_slots: &[u32], total_len: u32) -> u32 {
    let mut slots = [0u32; 4];
    let n = grant_slots.len().min(4);
    slots[..n].copy_from_slice(&grant_slots[..n]);
    unsafe { syscall_raw(SYS_SPAWN_FROM_MEMORY, slots[0], slots[1], slots[2], slots[3], total_len) }.eax
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
