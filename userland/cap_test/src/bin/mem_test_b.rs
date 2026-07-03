//! Checkpoint G test fixture, half B: receives a transferred MemoryGrant
//! capability from task_a, maps the *same* physical page into its own
//! (separate) address space, and verifies it can read back the exact
//! pattern task_a wrote before ever sending anything -- proving the
//! shared page is genuinely the same memory, not a coincidence. See
//! mem_test_a.rs for the full protocol description.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use libpcern::print;

/// CSlot 1 is the name service (auto-granted); this is this task's own
/// inbox. CSlot 3 is mem_test_a, hand-wired by main.rs -- see
/// task_a.rs's doc comment for why this test pairing isn't looked up by
/// name.
const MY_INBOX: u32 = 2;
const PEER_SLOT: u32 = 3;

const SHARED_VIRT: u32 = 0x0090_0000;
const PATTERN: u32 = 0xCAFE_BABE;

const MSG_SHARE: u32 = 1;
const MSG_DONE: u32 = 2;

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    // A dedicated endpoint for the lookup reply -- see cap_test's
    // task_a.rs doc comment: MY_INBOX also receives the peer protocol
    // below, and the two can race on a shared inbox.
    let console_reply = libpcern::endpoint_create();
    let console_slot = libpcern::lookup_name(b"console", console_reply).unwrap_or(0);

    let share = libpcern::recv(MY_INBOX);
    if share.w0 != MSG_SHARE || share.transferred_slot == 0 {
        print(console_slot, b"mem_test_b: FAIL (no grant)\n");
        libpcern::exit(1);
    }

    if libpcern::map_memory(share.transferred_slot, SHARED_VIRT) != 0 {
        print(console_slot, b"mem_test_b: FAIL (map)\n");
        libpcern::exit(1);
    }

    // Two denial-path regressions a code review found untested: no
    // fixture previously exercised SYS_MAP_MEMORY being refused at all
    // (only the success path above), which is exactly the gap that let a
    // real privilege-escalation bug (an unbounded virt_addr letting a task
    // flip PAGE_USER on the kernel's own higher-half/physmap PDEs) go
    // uncaught. An invalid capability slot must be rejected...
    const BOGUS_SLOT: u32 = 99;
    if libpcern::map_memory(BOGUS_SLOT, 0x0091_0000) == 0 {
        print(console_slot, b"mem_test_b: FAIL (bogus slot accepted)\n");
        libpcern::exit(1);
    }
    // ...and a legitimate MemoryGrant must still be refused the moment
    // virt_addr reaches the kernel's own higher half (every task's page
    // directory shares 0xC000_0000+ verbatim -- see
    // kernel/src/mm/paging.rs's KERNEL_VMA and sys_map_memory's bounds
    // check), even though the capability itself is perfectly valid.
    const KERNEL_VMA: u32 = 0xC000_0000;
    if libpcern::map_memory(share.transferred_slot, KERNEL_VMA) == 0 {
        print(console_slot, b"mem_test_b: FAIL (kernel-space map accepted)\n");
        libpcern::exit(1);
    }
    // Same denial-path gap existed for SYS_REGISTER_IRQ -- no fixture
    // exercised an invalid capability slot being rejected there either.
    if libpcern::register_irq(BOGUS_SLOT) == 0 {
        print(console_slot, b"mem_test_b: FAIL (bogus IRQ slot accepted)\n");
        libpcern::exit(1);
    }

    let observed = unsafe { (SHARED_VIRT as *const u32).read_volatile() };
    libpcern::send(PEER_SLOT, MSG_DONE, 0, 0, 0);

    if observed != PATTERN {
        print(console_slot, b"mem_test_b: FAIL (pattern mismatch)\n");
        libpcern::exit(1);
    }

    print(console_slot, b"mem_test_b: PASS\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
