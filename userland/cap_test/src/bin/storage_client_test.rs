//! Checkpoint I test fixture: exercises the *real* storage_ata protocol
//! (name-service lookup + storage_connect + storage_read_block over a
//! shared-memory grant) end to end, as opposed to the earlier disk-
//! plumbing spike which called ata::read_sector directly in-process.
//! Verified against the same host-written test disk image as the spike
//! (see Makefile/grub.cfg's temporary second `-drive`) -- prints the
//! first 16 bytes of LBA 0 as hex over the console so the result can be
//! screendumped and compared byte-for-byte. Not part of the default
//! build; wired into main.rs only temporarily for this checkpoint's
//! verification (see cap_test's own doc comment for the convention).

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted). CSlot 2 is this task's own
/// inbox (see main.rs's temporary wiring for this fixture).
const MY_INBOX: u32 = 2;
const OP_PUTCHAR: u32 = 0;

const BUF_VIRT: u32 = 0x0090_0000;

fn print(console_slot: u32, s: &[u8]) {
    for &b in s {
        libpcern::send(console_slot, OP_PUTCHAR, b as u32, 0, 0);
    }
}

fn print_hex_byte(console_slot: u32, b: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    print(console_slot, &[HEX[(b >> 4) as usize], HEX[(b & 0xF) as usize]]);
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let console_slot = libpcern::lookup_name(b"console", MY_INBOX).unwrap_or(0);
    let storage_slot = match libpcern::lookup_name(b"storage", MY_INBOX) {
        Some(s) => s,
        None => {
            print(console_slot, b"storage_client_test: FAIL (no storage)\n");
            libpcern::exit(1);
        }
    };

    let grant_slot = libpcern::mem_alloc(BUF_VIRT);
    if grant_slot == 0 {
        print(console_slot, b"storage_client_test: FAIL (alloc)\n");
        libpcern::exit(1);
    }

    libpcern::storage_connect(storage_slot, grant_slot, MY_INBOX);

    if !libpcern::storage_read_block(storage_slot, MY_INBOX, 0) {
        print(console_slot, b"storage_client_test: FAIL (read)\n");
        libpcern::exit(1);
    }

    let buf = unsafe { core::slice::from_raw_parts(BUF_VIRT as *const u8, 16) };
    print(console_slot, b"storage_client_test: LBA0 first 16 bytes: ");
    for &byte in buf {
        print_hex_byte(console_slot, byte);
    }
    print(console_slot, b"\n");

    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
