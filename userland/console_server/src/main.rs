//! Checkpoint C: proves the Rust userspace toolchain builds, loads as a
//! multiboot module, and runs in ring 3 at all -- before Checkpoint D ports
//! any real console logic in. Also exercises `.bss` zero-initialization
//! (see the `COUNTER` check below) since the real console server will lean
//! on static mutable state for its ANSI parser and VGA writer.

#![no_std]
#![no_main]

mod syscall;

use core::panic::PanicInfo;

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    syscall::debug_write(b"hello from Rust ring 3!\n");

    // If objcopy's flat-binary conversion didn't zero-fill .bss, this
    // would start at garbage instead of 0 and the check below would fail.
    static mut COUNTER: u32 = 0;
    unsafe {
        COUNTER += 1;
        COUNTER += 1;
        if COUNTER == 2 {
            syscall::debug_write(b"bss zero-init ok\n");
        } else {
            syscall::debug_write(b"bss zero-init FAILED\n");
        }
    }

    syscall::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    syscall::debug_write(b"console_server panic\n");
    syscall::exit(1);
}
