//! Checkpoint F test fixture, half B: receives a transferred capability
//! from task_a, waits for the go-ahead, then tries to use it -- which must
//! fail once task_a has revoked its own (parent) copy. See task_a.rs for
//! the full protocol description.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

const MY_INBOX: u32 = 1;
const PEER_SLOT: u32 = 2;
const CONSOLE_SLOT: u32 = 3;
const OP_PUTCHAR: u32 = 0;

const MSG_GIFT: u32 = 1;
const MSG_ACK: u32 = 2;
const MSG_GO: u32 = 3;

fn print(s: &[u8]) {
    for &b in s {
        libpcern::send(CONSOLE_SLOT, OP_PUTCHAR, b as u32, 0, 0);
    }
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let gift = libpcern::recv(MY_INBOX);
    if gift.w0 != MSG_GIFT || gift.transferred_slot == 0 {
        print(b"cap_test_b: FAIL (no gift)\n");
        libpcern::exit(1);
    }
    let transferred_slot = gift.transferred_slot;

    libpcern::send(PEER_SLOT, MSG_ACK, 0, 0, 0);

    let go = libpcern::recv(MY_INBOX);
    if go.w0 != MSG_GO {
        print(b"cap_test_b: FAIL (bad go)\n");
        libpcern::exit(1);
    }

    // task_a revoked the badged capability this was derived from right
    // before sending MSG_GO -- this send must now fail.
    let result = libpcern::send(transferred_slot, 99, 99, 99, 0);
    if result == 0 {
        print(b"cap_test_b: FAIL (revoked cap still worked)\n");
        libpcern::exit(1);
    }

    print(b"cap_test_b: PASS\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
