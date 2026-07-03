//! Checkpoint V test fixture: exercises the new `SYS_REBOOT` syscall.
//!
//! Only ever wired into the standalone `reboot_test`-featured kernel build
//! (see main.rs's `reboot_test_spawn` and `grub-reboottest.cfg`) -- never
//! any other harness, since a real reset would tear down whatever else
//! was running alongside it.
//!
//! There's no exit code to check here (the whole point is that the
//! machine resets before this task ever gets to call `exit`), so
//! verification works the other way around: print a marker to serial
//! directly (same direct-COM1-port technique `raw_input_test`/
//! `console_input_test` use, for the same reason -- this needs to be
//! visible before anything as heavyweight as `console_server`'s own
//! protocol could plausibly matter) right before triggering the reset,
//! then trigger it. `run_reboot_test.sh` checks that the marker made it
//! to the serial log *and* that QEMU (booted with `-no-reboot`) exited on
//! its own well before the harness's timeout -- the machine actually
//! resetting is what makes a `-no-reboot` QEMU quit, so a clean, prompt
//! exit is the pass signal, not a hang.
#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted, unused by this fixture).
/// CSlot 2 is this task's own inbox (also unused -- this fixture never
/// receives). CSlot 3 is the `RebootControl` capability main.rs hand-wires
/// specifically for this fixture (see `reboot_test_spawn`).
const REBOOT_CONTROL: u32 = 3;

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
    serial_print(b"reboot_test: about to reboot\n");
    libpcern::reboot(REBOOT_CONTROL);

    // Only reached if SYS_REBOOT was rejected (e.g. CSlot 3 didn't resolve
    // to a RebootControl -- a real bug in main.rs's wiring, since this
    // fixture is the only task ever handed one). Report it distinctly
    // from "the marker never printed" so a failure here doesn't look like
    // a QEMU/harness problem.
    serial_print(b"reboot_test: FAIL (reboot syscall rejected)\n");
    libpcern::exit(1);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
