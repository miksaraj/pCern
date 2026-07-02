//! Checkpoint L test fixture: exercises the real keyboard-input protocol
//! (console_server's CONSOLE_OP_SET_BUFFER/SET_READER/READ_LINE, see
//! libpcern's console_connect/console_read_line) against *real* PS/2
//! keystrokes injected via QEMU's `sendkey` monitor command (see
//! run_console_input_test.sh) rather than a synthetic in-process byte --
//! proving the actual IRQ1 -> kernel-forward -> console_server's
//! keyboard::Decoder -> line-buffer path end to end, the same signal path
//! a human types through. Only ever wired into the standalone
//! `keyboard_test`-featured kernel build (see main.rs's
//! keyboard_test_spawn and grub-keytest.cfg) -- never part of the shared
//! `iso-test` build every other cap_test fixture runs under, since no
//! keys are ever injected there and this fixture would simply hang until
//! that harness's own boot timeout.
//!
//! Synchronization: this fixture is granted direct port access to COM1
//! (0x3F8/0x3FD -- same allowed_ports mechanism as storage_ata's ATA
//! ports, see main.rs) so it can print one plain readiness line to serial
//! itself, once armed and about to block waiting for a line -- the test
//! script polls for that line before calling `sendkey`, rather than
//! sleeping a fixed amount (see CLAUDE.md's design notes on this
//! checkpoint). This is a synchronization *gate*, not the pass/fail
//! signal -- the exit code is still that, checked the same way as every
//! other fixture. Nothing else in the keyboard_test build writes to
//! serial from outside the kernel's own `println!`/serial module, so
//! there's no byte-interleaving risk with this fixture's own raw COM1
//! writes to worry about here.

#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted). CSlot 2 is this task's own
/// inbox -- reused as both the name-lookup reply and the console reader
/// endpoint, safe here because nothing else ever sends to it concurrently
/// with either (same reasoning as fs_client_test's MY_INBOX reuse, not
/// task_a/b's -- see CLAUDE.md's "one inbox is not automatically safe for
/// two roles").
const MY_INBOX: u32 = 2;
const OP_PUTCHAR: u32 = 0;

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

/// Writes straight to COM1, bypassing console_server entirely -- this
/// fixture is granted the port directly (see main.rs's
/// keyboard_test_spawn), the same way storage_ata's ports are, purely so
/// it can emit its own readiness marker independent of the console
/// protocol it's busy testing.
fn serial_print(s: &[u8]) {
    for &b in s {
        unsafe {
            while inb(COM1 + 5) & 0x20 == 0 {}
            outb(COM1, b);
        }
    }
}

fn print(console_slot: u32, s: &[u8]) {
    for &b in s {
        libpcern::send(console_slot, OP_PUTCHAR, b as u32, 0, 0);
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
        print(console_slot, b"console_input_test: FAIL (alloc)\n");
        libpcern::exit(1);
    }

    libpcern::console_connect(console_slot, grant_slot, MY_INBOX);

    // Arm the read directly (rather than via console_read_line, which
    // would also block in recv before we get a chance to print the
    // readiness marker below). send() only returns once console_server's
    // own recv() loop has matched it -- so by the time this call returns,
    // console_server has already processed CONSOLE_OP_READ_LINE and gone
    // back to waiting for the next event (see ipc.rs's rendezvous
    // semantics) -- it's armed before the marker line goes out.
    libpcern::send(console_slot, libpcern::CONSOLE_OP_READ_LINE, 0, 0, 0);

    serial_print(b"console_input_test: ready\n");

    let len = libpcern::recv(MY_INBOX).w0 as usize;
    let data = unsafe { core::slice::from_raw_parts(BUF_VIRT as *const u8, len) };

    const EXPECTED: &[u8] = b"hello";
    if data == EXPECTED {
        print(console_slot, b"console_input_test: PASS\n");
        libpcern::exit(0);
    } else {
        print(console_slot, b"console_input_test: FAIL (mismatch)\n");
        libpcern::exit(1);
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
