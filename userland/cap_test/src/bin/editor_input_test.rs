//! Phase 7, Checkpoint S test fixture: exercises the full-screen editor
//! end to end against *real* PS/2 keystrokes injected via QEMU's
//! `sendkey` monitor command (see run_editor_test.sh) -- the same
//! "prove it for real" approach every input-driven checkpoint in this
//! project uses. Uses `libpcern::editor::Editor` directly (the exact same
//! type `userland/shell`'s `edit` command drives), so this fixture proves
//! the actual shipping editor logic, not a re-implementation that could
//! quietly drift from it -- only the surrounding protocol glue (open/
//! load/save via fs_fat32, arm/read via console_server's raw mode) is
//! duplicated from `userland/shell/src/editor.rs`, and it's a thin,
//! direct call to the same libpcern helpers either way.
//!
//! Scripted edit: types "hello", moves the cursor left twice (to exercise
//! `KEY_LEFT`/extended-scancode decoding through a real full editing
//! session), inserts 'X', backspaces it back out (insert + backspace),
//! then saves with Ctrl-S -- expecting the file to end up containing
//! exactly "hello" again. Reopens and reads it back via fs_fat32's normal
//! read path (not anything the Editor itself still holds in memory) to
//! confirm the save actually round-tripped through the real write path
//! from Checkpoint Q.
//!
//! Only ever wired into the standalone `editor_test`-featured kernel
//! build (see main.rs's `editor_test_spawn` and `grub-editortest.cfg`) --
//! blocks on real external keystrokes, same reason console_input_test/
//! raw_input_test are never folded into the shared `iso-test` build.

#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;
use libpcern::editor::Editor;
use libpcern::print;

/// CSlot 1 is the name service (auto-granted). CSlot 2 is this task's own
/// inbox -- reused as the fs_fat32 reply endpoint (synchronous request/
/// reply, never contending with the separate console reader endpoint
/// below -- same reasoning as fs_client_test's MY_INBOX reuse).
const MY_INBOX: u32 = 2;

const CONSOLE_BUF_VIRT: u32 = 0x00B0_0000;
const FS_BUF_VIRT: u32 = 0x00B1_0000;
const EDITOR_BUF_VIRT: u32 = 0x00B2_0000;

const COM1: u16 = 0x3F8;

#[inline(always)]
unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

fn serial_print(s: &[u8]) {
    for &b in s {
        unsafe {
            while inb(COM1 + 5) & 0x20 == 0 {}
            outb(COM1, b);
        }
    }
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let console_slot = match libpcern::lookup_name(b"console", MY_INBOX) {
        Some(s) => s,
        None => libpcern::exit(1),
    };
    let fs_slot = match libpcern::lookup_name_retry(b"fs", MY_INBOX, 1000) {
        Some(s) => s,
        None => {
            print(console_slot, b"editor_input_test: FAIL (no fs)\n");
            libpcern::exit(1);
        }
    };

    let console_grant = libpcern::mem_alloc(CONSOLE_BUF_VIRT);
    let fs_grant = libpcern::mem_alloc(FS_BUF_VIRT);
    if console_grant == 0 || fs_grant == 0 {
        print(console_slot, b"editor_input_test: FAIL (alloc)\n");
        libpcern::exit(1);
    }

    let reader_slot = libpcern::endpoint_create();
    libpcern::console_connect(console_slot, console_grant, reader_slot);
    libpcern::fs_connect(fs_slot, fs_grant, MY_INBOX);

    let mut ed = match Editor::new(EDITOR_BUF_VIRT) {
        Some(e) => e,
        None => {
            print(console_slot, b"editor_input_test: FAIL (editor alloc)\n");
            libpcern::exit(1);
        }
    };

    if libpcern::fs_open_for_write(fs_slot, MY_INBOX, b"EDITTEST.TXT").is_none() {
        print(console_slot, b"editor_input_test: FAIL (create)\n");
        libpcern::exit(1);
    }

    libpcern::console_set_mode(console_slot, true);

    // Arm the first key request directly (rather than via
    // console_read_key, which would also block in recv before the
    // readiness marker goes out) -- same technique console_input_test/
    // raw_input_test use.
    libpcern::send(console_slot, libpcern::CONSOLE_OP_READ_KEY, 0, 0, 0);
    serial_print(b"editor_input_test: ready\n");
    let first_key = libpcern::recv(reader_slot).w0;

    // Redraws after every applied key, exactly mirroring
    // userland/shell/src/editor.rs's real usage pattern -- a redraw's
    // cost scales with content typed so far (see
    // libpcern::editor::Editor::redraw), so exercising that same
    // per-keystroke redraw here is what actually proves console_server's
    // key queue (Checkpoint R) holds up against it, not just against a
    // fixture that drains keys as fast as possible.
    let save = match ed.apply_key(first_key) {
        Some(save) => save,
        None => {
            ed.redraw(console_slot);
            loop {
                let key = libpcern::console_read_key(console_slot, reader_slot);
                if let Some(save) = ed.apply_key(key) {
                    break save;
                }
                ed.redraw(console_slot);
            }
        }
    };

    libpcern::console_set_mode(console_slot, false);

    if !save {
        print(console_slot, b"editor_input_test: FAIL (unexpected quit-without-save)\n");
        libpcern::exit(1);
    }

    let content = ed.content();
    if content != b"hello" {
        print(console_slot, b"editor_input_test: FAIL (in-memory content mismatch)\n");
        libpcern::exit(1);
    }

    let mut offset: usize = 0;
    while offset < content.len() {
        let want = (content.len() - offset).min(512);
        unsafe {
            let dst = core::slice::from_raw_parts_mut(FS_BUF_VIRT as *mut u8, want);
            dst.copy_from_slice(&content[offset..offset + want]);
        }
        let n = libpcern::fs_write(fs_slot, MY_INBOX, offset as u32, want as u32);
        if n == 0 {
            print(console_slot, b"editor_input_test: FAIL (save write)\n");
            libpcern::exit(1);
        }
        offset += n as usize;
    }

    // Reopen and read back via the normal read path -- independent of
    // anything the Editor itself still holds in memory -- to confirm the
    // save actually reached fs_fat32/storage_ata, not just this task's
    // own buffer.
    let size = match libpcern::fs_open(fs_slot, MY_INBOX, b"EDITTEST.TXT") {
        Some(s) => s,
        None => {
            print(console_slot, b"editor_input_test: FAIL (reopen)\n");
            libpcern::exit(1);
        }
    };
    if size != 5 {
        print(console_slot, b"editor_input_test: FAIL (saved size mismatch)\n");
        libpcern::exit(1);
    }
    let n = libpcern::fs_read(fs_slot, MY_INBOX, 0, 5);
    let readback = unsafe { core::slice::from_raw_parts(FS_BUF_VIRT as *const u8, n as usize) };
    if readback != b"hello" {
        print(console_slot, b"editor_input_test: FAIL (readback mismatch)\n");
        libpcern::exit(1);
    }

    print(console_slot, b"editor_input_test: PASS\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
