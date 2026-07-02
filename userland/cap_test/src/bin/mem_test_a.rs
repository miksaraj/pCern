//! Checkpoint G test fixture, half A: allocates a fresh page via
//! `SYS_MEM_ALLOC`, writes a known pattern into it, and transfers the
//! resulting MemoryGrant capability to task_b -- proving a capability
//! minted by an ordinary (non-driver) task for its own fresh memory can
//! still cross into another address space and be mapped there, the bulk
//! data transfer primitive later checkpoints (storage/filesystem) build
//! on. Not wired into the default build -- see cap_test/task_a.rs.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted); this is this task's own
/// inbox. CSlot 3 is mem_test_b, hand-wired by main.rs -- see
/// task_a.rs's doc comment for why this test pairing isn't looked up by
/// name.
const MY_INBOX: u32 = 2;
const PEER_SLOT: u32 = 3;
const OP_PUTCHAR: u32 = 0;

const SHARED_VIRT: u32 = 0x0090_0000;
const PATTERN: u32 = 0xCAFE_BABE;

const MSG_SHARE: u32 = 1;
const MSG_DONE: u32 = 2;

fn print(console_slot: u32, s: &[u8]) {
    for &b in s {
        libpcern::send(console_slot, OP_PUTCHAR, b as u32, 0, 0);
    }
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    // A dedicated endpoint for the lookup reply -- see cap_test's
    // task_a.rs doc comment: MY_INBOX also receives the peer protocol
    // below, and the two can race on a shared inbox.
    let console_reply = libpcern::endpoint_create();
    let console_slot = libpcern::lookup_name(b"console", console_reply).unwrap_or(0);

    let grant_slot = libpcern::mem_alloc(SHARED_VIRT);
    if grant_slot == 0 {
        print(console_slot, b"mem_test_a: FAIL (alloc)\n");
        libpcern::exit(1);
    }

    unsafe { (SHARED_VIRT as *mut u32).write_volatile(PATTERN) };

    libpcern::send(PEER_SLOT, MSG_SHARE, 0, 0, grant_slot);

    let done = libpcern::recv(MY_INBOX);
    if done.w0 != MSG_DONE {
        print(console_slot, b"mem_test_a: FAIL (bad done)\n");
        libpcern::exit(1);
    }

    print(console_slot, b"mem_test_a: done\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
