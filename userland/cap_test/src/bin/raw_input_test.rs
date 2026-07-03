//! Phase 7, Checkpoint R test fixture: exercises console_server's new raw
//! single-keystroke mode (`CONSOLE_OP_SET_MODE`/`CONSOLE_OP_READ_KEY`) and
//! the new Ctrl-tracking/extended-scancode decoding in `keyboard.rs`
//! against *real* PS/2 keystrokes injected via QEMU's `sendkey` monitor
//! command (see run_raw_input_test.sh), the same "prove it for real"
//! approach Checkpoint L's console_input_test already established --
//! never a synthetic in-process byte. Checks a plain printable key, an
//! extended (0xE0-prefixed) arrow key, and a Ctrl-chord, one after the
//! other, since each exercises a different new decoding path
//! (`SCANCODE_ASCII` unchanged, `extended_key`'s new table, and the new
//! Ctrl-to-control-code remap).
//!
//! Only ever wired into the standalone `raw_input_test`-featured kernel
//! build (see main.rs's `raw_input_test_spawn` and `grub-rawtest.cfg`) --
//! never the shared `iso-test` build (this fixture would hang it waiting
//! for keystrokes that never arrive) nor `keyboard_test`'s build (it would
//! race `console_input_test` for the single `reader_owner` role).
//!
//! Synchronization: same pattern as `console_input_test` -- direct COM1
//! port access to print one readiness marker to serial once the first key
//! request is armed, gating when the test script may start calling
//! `sendkey`, not the pass/fail signal itself (that's still this
//! fixture's exit code).

#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;
use libpcern::print;

/// CSlot 1 is the name service (auto-granted). CSlot 2 is this task's own
/// inbox -- reused as both the name-lookup reply and the console reader
/// endpoint, same reasoning as console_input_test's MY_INBOX reuse.
const MY_INBOX: u32 = 2;

const BUF_VIRT: u32 = 0x00B0_0000;

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

    let grant_slot = libpcern::mem_alloc(BUF_VIRT);
    if grant_slot == 0 {
        print(console_slot, b"raw_input_test: FAIL (alloc)\n");
        libpcern::exit(1);
    }

    libpcern::console_connect(console_slot, grant_slot, MY_INBOX);
    libpcern::console_set_mode(console_slot, true);

    // Arm the first read directly (rather than via console_read_key,
    // which would also block in recv before we get a chance to print the
    // readiness marker) -- same technique console_input_test uses, and
    // for the same reason (send() only returns once console_server's own
    // recv() has matched it, so it's armed before the marker line goes
    // out).
    libpcern::send(console_slot, libpcern::CONSOLE_OP_READ_KEY, 0, 0, 0);
    serial_print(b"raw_input_test: ready\n");

    let key1 = libpcern::recv(MY_INBOX).w0;
    if key1 != b'a' as u32 {
        print(console_slot, b"raw_input_test: FAIL (plain key)\n");
        libpcern::exit(1);
    }

    let key2 = libpcern::console_read_key(console_slot, MY_INBOX);
    if key2 != libpcern::KEY_LEFT {
        print(console_slot, b"raw_input_test: FAIL (extended key)\n");
        libpcern::exit(1);
    }

    let key3 = libpcern::console_read_key(console_slot, MY_INBOX);
    if key3 != 0x01 {
        print(console_slot, b"raw_input_test: FAIL (ctrl chord)\n");
        libpcern::exit(1);
    }

    print(console_slot, b"raw_input_test: PASS\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
