//! Checkpoint G test fixture, half B: receives a transferred MemoryGrant
//! capability from task_a, maps the *same* physical page into its own
//! (separate) address space, and verifies it can read back the exact
//! pattern task_a wrote before ever sending anything -- proving the
//! shared page is genuinely the same memory, not a coincidence. See
//! mem_test_a.rs for the full protocol description.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

const MY_INBOX: u32 = 1;
const PEER_SLOT: u32 = 2;
const CONSOLE_SLOT: u32 = 3;
const OP_PUTCHAR: u32 = 0;

const SHARED_VIRT: u32 = 0x0090_0000;
const PATTERN: u32 = 0xCAFE_BABE;

const MSG_SHARE: u32 = 1;
const MSG_DONE: u32 = 2;

fn print(s: &[u8]) {
    for &b in s {
        libpcern::send(CONSOLE_SLOT, OP_PUTCHAR, b as u32, 0, 0);
    }
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let share = libpcern::recv(MY_INBOX);
    if share.w0 != MSG_SHARE || share.transferred_slot == 0 {
        print(b"mem_test_b: FAIL (no grant)\n");
        libpcern::exit(1);
    }

    if libpcern::map_memory(share.transferred_slot, SHARED_VIRT) != 0 {
        print(b"mem_test_b: FAIL (map)\n");
        libpcern::exit(1);
    }

    let observed = unsafe { (SHARED_VIRT as *const u32).read_volatile() };
    libpcern::send(PEER_SLOT, MSG_DONE, 0, 0, 0);

    if observed != PATTERN {
        print(b"mem_test_b: FAIL (pattern mismatch)\n");
        libpcern::exit(1);
    }

    print(b"mem_test_b: PASS\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
