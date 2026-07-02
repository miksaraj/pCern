//! Checkpoint I test fixture: exercises the *real* storage_ata protocol
//! (name-service lookup + storage_connect + storage_read_block over a
//! shared-memory grant) end to end, as opposed to the earlier disk-
//! plumbing spike which called ata::read_sector directly in-process.
//! Asserts the FAT32 boot-sector signature (`0x55 0xAA` at the last two
//! bytes of LBA 0) rather than a bespoke byte pattern, so this can run
//! against the exact same `make test-fat32-image` disk fs_client_test
//! uses -- no second disk needed just to give this fixture something
//! deterministic to check. Not part of the default build; see
//! `make test`.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use libpcern::print;

/// CSlot 1 is the name service (auto-granted). CSlot 2 is this task's own
/// inbox (see main.rs's temporary wiring for this fixture).
const MY_INBOX: u32 = 2;

const BUF_VIRT: u32 = 0x0090_0000;

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

    let buf = unsafe { core::slice::from_raw_parts(BUF_VIRT as *const u8, 512) };
    print(console_slot, b"storage_client_test: LBA0 first 16 bytes: ");
    for &byte in &buf[..16] {
        print_hex_byte(console_slot, byte);
    }
    print(console_slot, b"\n");

    if buf[510] != 0x55 || buf[511] != 0xAA {
        print(console_slot, b"storage_client_test: FAIL (bad boot signature)\n");
        libpcern::exit(1);
    }

    print(console_slot, b"storage_client_test: PASS\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
