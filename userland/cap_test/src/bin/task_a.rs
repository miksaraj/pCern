//! Checkpoint F test fixture, half A: mints a fresh endpoint, derives a
//! badged copy of it, and hands that copy to task_b over IPC (`send`'s
//! transfer slot). Once task_b confirms receipt, revokes the badged copy
//! and tells task_b to try using its transferred capability -- which must
//! now fail, proving revocation cascades to a capability that already
//! crossed into another task's address space.
//!
//! Not wired into grub.cfg/main.rs by default -- see driver_test.asm/
//! irq_test.asm for the established pattern this follows: a documented,
//! independently buildable fixture, temporarily wired in to verify, then
//! reverted.

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
    let gift_endpoint_slot = libpcern::endpoint_create();
    let badged_slot = libpcern::cap_mint_badged(gift_endpoint_slot, 42);

    libpcern::send(PEER_SLOT, MSG_GIFT, 0, 0, badged_slot);

    let ack = libpcern::recv(MY_INBOX);
    if ack.w0 != MSG_ACK {
        print(b"cap_test_a: FAIL (bad ack)\n");
        libpcern::exit(1);
    }

    // Revoke the badged copy we handed over. Cascades to task_b's derived
    // (transferred) copy without touching gift_endpoint_slot itself.
    libpcern::cap_revoke(badged_slot);

    libpcern::send(PEER_SLOT, MSG_GO, 0, 0, 0);

    print(b"cap_test_a: done\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
