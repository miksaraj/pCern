//! Checkpoint M test fixture, the "loaded program" half: the tiniest
//! possible ring-3 program, built and objcopy'd the same way every other
//! userland binary is, then dropped onto the test FAT32 image as
//! LOADED.BIN (see Makefile's test-fat32-image target) -- not run
//! directly as a multiboot module. `spawn_from_memory_test.rs` reads
//! these bytes via the real fs_fat32 protocol and spawns them with the
//! new `SYS_SPAWN_FROM_MEMORY` syscall.
//!
//! Exits with a single distinctive, otherwise-impossible-to-get-by-
//! accident code (42) -- a crashing/faulted task exits with -1 (see
//! exceptions.rs in the kernel), so seeing this exact code in the serial
//! log's "task N exited with code 42" is proof this code actually ran to
//! completion, not just that SYS_SPAWN_FROM_MEMORY returned a task id.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    libpcern::exit(42);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
