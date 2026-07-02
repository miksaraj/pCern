//! Checkpoint F test fixture, half A: mints a fresh endpoint, derives a
//! badged copy of it, and hands that copy to task_b over IPC (`send`'s
//! transfer slot). Once task_b confirms receipt, revokes the badged copy
//! and tells task_b to try using its transferred capability -- which must
//! now fail, proving revocation cascades to a capability that already
//! crossed into another task's address space.
//!
//! Not wired into grub.cfg/main.rs by default -- built and run on demand
//! via `make test` (see the repo root's test harness), the same
//! documented, independently buildable fixture pattern as
//! driver_test.asm/irq_test.asm before it.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted -- see loader.rs in the
/// kernel); this is this task's own inbox. CSlot 3 is task_b, hand-wired
/// by main.rs the same way console_server's hardware capabilities are --
/// there's no name to look "the peer in this specific test pairing" up
/// under.
const MY_INBOX: u32 = 2;
const PEER_SLOT: u32 = 3;
const OP_PUTCHAR: u32 = 0;

const MSG_GIFT: u32 = 1;
const MSG_ACK: u32 = 2;
const MSG_GO: u32 = 3;

fn print(console_slot: u32, s: &[u8]) {
    for &b in s {
        libpcern::send(console_slot, OP_PUTCHAR, b as u32, 0, 0);
    }
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    // A dedicated endpoint for the lookup reply, *not* MY_INBOX: the peer
    // protocol below also receives on MY_INBOX, and message arrival isn't
    // ordered -- task_b's gift could arrive before the name service's
    // reply does, and `recv` would have no way to tell them apart on a
    // shared inbox (see fs_fat32's identically-motivated storage_reply).
    let console_reply = libpcern::endpoint_create();
    let console_slot = libpcern::lookup_name(b"console", console_reply).unwrap_or(0);

    let gift_endpoint_slot = libpcern::endpoint_create();
    let badged_slot = libpcern::cap_mint_badged(gift_endpoint_slot, 42);

    libpcern::send(PEER_SLOT, MSG_GIFT, 0, 0, badged_slot);

    let ack = libpcern::recv(MY_INBOX);
    if ack.w0 != MSG_ACK {
        print(console_slot, b"cap_test_a: FAIL (bad ack)\n");
        libpcern::exit(1);
    }

    // Revoke the badged copy we handed over. Cascades to task_b's derived
    // (transferred) copy without touching gift_endpoint_slot itself.
    libpcern::cap_revoke(badged_slot);

    libpcern::send(PEER_SLOT, MSG_GO, 0, 0, 0);

    print(console_slot, b"cap_test_a: done\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
