//! Checkpoint N: the first interactive experience -- a minimal shell that
//! reads a line from console_server's input protocol (Checkpoint L),
//! parses it as `<command> <argument>`, and acts on it via fs_fat32
//! (open/read, Checkpoint J) and the new SYS_SPAWN_FROM_MEMORY syscall
//! (Checkpoint M). Two commands: `read <name>` prints a file's contents,
//! `run <name>` loads and runs it as a new task.
//!
//! Two endpoints, not one: `MY_INBOX` (CSlot 2) is used only for the
//! synchronous name-service/fs_fat32 request/reply round trips, and a
//! separately created `reader_slot` is used only for console_server's
//! asynchronous "line ready" notifications -- the exact hazard CLAUDE.md's
//! "one inbox is not automatically safe for two roles" documents: a
//! second typed-ahead line completing while this shell is still blocked
//! waiting on an `fs_open`/`fs_read` reply would otherwise race that
//! reply on a shared inbox, and the two happen to use overlapping-looking
//! success/length values, exactly the kind of coincidence that bug class
//! exploits.
//!
//! fs_fat32's `fs_read` only ever returns up to one 512-byte sector per
//! call, always written to the start of the shared buffer regardless of
//! file offset (see fs_fat32's own `read_file`) -- so loading a program
//! bigger than one sector means copying each chunk out to the right
//! offset in a second, separate page before handing that page to
//! `spawn_from_memory`. This shell caps `run` at one page (4096 bytes,
//! the same cap every `MemoryGrant` already has) rather than adding
//! multi-page assembly for what's still a "run a few small programs"
//! phase.

#![no_std]
#![no_main]

mod editor;

use core::panic::PanicInfo;
use libpcern::{print, print_u32};

/// CSlot 1 is the name service (auto-granted). CSlot 2 is this task's own
/// inbox -- see the module doc comment for why console line-ready
/// notifications deliberately do *not* share it.
const MY_INBOX: u32 = 2;

const CONSOLE_BUF_VIRT: u32 = 0x00D0_0000;
const FS_BUF_VIRT: u32 = 0x00D1_0000;
const RUN_BUF_VIRT: u32 = 0x00D2_0000;
/// Base of the editor's 16-page (64 KiB) buffer (Checkpoint S) -- a
/// separate, non-overlapping region from the three single-page ones
/// above, built from consecutive `mem_alloc` calls (see editor.rs).
const EDITOR_BUF_VIRT: u32 = 0x00D3_0000;
const SECTOR_SIZE: u32 = 512;
const PAGE_SIZE: u32 = 4096;

/// Splits `line` into `(command, argument)` on the first space; `argument`
/// is empty if there isn't one. Trailing/leading spaces in `argument`
/// aren't trimmed -- callers pass it straight to `fat_pack_name`, which
/// already caps at 8+3 bytes and ignores anything past that.
fn split_command(line: &[u8]) -> (&[u8], &[u8]) {
    match line.iter().position(|&b| b == b' ') {
        Some(i) => (&line[..i], &line[i + 1..]),
        None => (line, &[]),
    }
}

fn print_help(console_slot: u32) {
    print(console_slot, b"commands: read <FILE>, edit <FILE>, run <FILE>, help\n");
}

/// Prints a file's contents a sector at a time -- same read loop
/// fs_client_test already exercises, just against whatever name the user
/// typed instead of a fixed one.
fn cmd_read(console_slot: u32, fs_slot: u32, name: &[u8]) {
    let size = match libpcern::fs_open(fs_slot, MY_INBOX, name) {
        Some(s) => s,
        None => {
            print(console_slot, b"read: not found\n");
            return;
        }
    };

    let mut offset: u32 = 0;
    while offset < size {
        let n = libpcern::fs_read(fs_slot, MY_INBOX, offset, SECTOR_SIZE);
        if n == 0 {
            break;
        }
        let data = unsafe { core::slice::from_raw_parts(FS_BUF_VIRT as *const u8, n as usize) };
        print(console_slot, data);
        offset += n;
    }
    print(console_slot, b"\n");
}

/// Loads a file (capped at one page, see the module doc comment) into
/// `RUN_BUF_VIRT` and spawns it via `SYS_SPAWN_FROM_MEMORY`.
fn cmd_run(console_slot: u32, fs_slot: u32, run_grant: u32, name: &[u8]) {
    let size = match libpcern::fs_open(fs_slot, MY_INBOX, name) {
        Some(s) => s,
        None => {
            print(console_slot, b"run: not found\n");
            return;
        }
    };

    if size == 0 || size > PAGE_SIZE {
        print(console_slot, b"run: unsupported size\n");
        return;
    }

    let mut offset: u32 = 0;
    while offset < size {
        let want = (size - offset).min(SECTOR_SIZE);
        let n = libpcern::fs_read(fs_slot, MY_INBOX, offset, want);
        if n == 0 {
            print(console_slot, b"run: short read\n");
            return;
        }
        unsafe {
            let src = FS_BUF_VIRT as *const u8;
            let dst = (RUN_BUF_VIRT + offset) as *mut u8;
            core::ptr::copy_nonoverlapping(src, dst, n as usize);
        }
        offset += n;
    }

    let task_id = libpcern::spawn_from_memory(&[run_grant], size);
    if task_id == 0 {
        print(console_slot, b"run: spawn failed\n");
        return;
    }
    print(console_slot, b"run: spawned task ");
    print_u32(console_slot, task_id);
    print(console_slot, b"\n");
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let console_slot = match libpcern::lookup_name(b"console", MY_INBOX) {
        Some(s) => s,
        None => libpcern::exit(1),
    };

    // fs_fat32 needs several IPC round trips of its own (connecting to
    // storage_ata, reading the BPB) before it registers "fs" -- retry
    // rather than treating a too-early lookup as failure.
    let fs_slot = match libpcern::lookup_name_retry(b"fs", MY_INBOX, 1000) {
        Some(s) => s,
        None => {
            print(console_slot, b"shell: FAIL (no fs)\n");
            libpcern::exit(1);
        }
    };

    let fs_grant = libpcern::mem_alloc(FS_BUF_VIRT);
    let console_grant = libpcern::mem_alloc(CONSOLE_BUF_VIRT);
    let run_grant = libpcern::mem_alloc(RUN_BUF_VIRT);
    if fs_grant == 0 || console_grant == 0 || run_grant == 0 {
        print(console_slot, b"shell: FAIL (alloc)\n");
        libpcern::exit(1);
    }

    libpcern::fs_connect(fs_slot, fs_grant, MY_INBOX);

    // Own dedicated endpoint for console line-ready notifications -- see
    // the module doc comment for why this can't be MY_INBOX.
    let reader_slot = libpcern::endpoint_create();
    libpcern::console_connect(console_slot, console_grant, reader_slot);

    print(console_slot, b"pCern shell -- type 'help' for commands\n> ");

    loop {
        let len = libpcern::console_read_line(console_slot, reader_slot) as usize;
        let line = unsafe { core::slice::from_raw_parts(CONSOLE_BUF_VIRT as *const u8, len) };
        let (command, argument) = split_command(line);

        match command {
            b"help" => print_help(console_slot),
            b"read" => cmd_read(console_slot, fs_slot, argument),
            b"run" => cmd_run(console_slot, fs_slot, run_grant, argument),
            b"edit" => editor::run(
                console_slot,
                reader_slot,
                MY_INBOX,
                fs_slot,
                FS_BUF_VIRT,
                EDITOR_BUF_VIRT,
                argument,
            ),
            b"" => {}
            _ => {
                print(console_slot, b"unknown command: ");
                print(console_slot, command);
                print(console_slot, b"\n");
            }
        }

        print(console_slot, b"> ");
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
