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
//!
//! Phase 7, Checkpoint P: also exercises `storage_write_block` against
//! the last LBA of the 64 MiB test image -- far past anything
//! `make test-fat32-image`'s tiny mcopy'd files or FAT32 metadata could
//! ever allocate, so this write can't corrupt the filesystem structure
//! `fs_client_test`/`fs_write_test` depend on. Writes a known pattern,
//! reads it back in a *second* `storage_read_block` call (proving the
//! bytes actually round-tripped through the driver, not just that the
//! shared buffer still holds what this task itself wrote into it), and
//! prints a hex dump so a human verifying this standalone run can also
//! check the disk image's bytes directly on the host afterward.

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

    // Last LBA of the 64 MiB test image (64*1024*1024/512 - 1) -- far past
    // anything mformat/mcopy could have allocated, so this write can't
    // touch the FAT32 structure or either test file's data.
    const SCRATCH_LBA: u32 = 131_071;
    let pattern: [u8; 512] = core::array::from_fn(|i| (i as u8).wrapping_mul(31).wrapping_add(7));

    {
        let write_buf =
            unsafe { core::slice::from_raw_parts_mut(BUF_VIRT as *mut u8, 512) };
        write_buf.copy_from_slice(&pattern);
    }
    if !libpcern::storage_write_block(storage_slot, MY_INBOX, SCRATCH_LBA) {
        print(console_slot, b"storage_client_test: FAIL (write)\n");
        libpcern::exit(1);
    }

    // Overwrite the shared buffer with something else first, so a
    // passing read-back genuinely proves the driver read fresh bytes off
    // "disk" rather than the buffer coincidentally still holding what was
    // just written.
    {
        let scratch_buf =
            unsafe { core::slice::from_raw_parts_mut(BUF_VIRT as *mut u8, 512) };
        scratch_buf.fill(0);
    }
    if !libpcern::storage_read_block(storage_slot, MY_INBOX, SCRATCH_LBA) {
        print(console_slot, b"storage_client_test: FAIL (read-back)\n");
        libpcern::exit(1);
    }

    let readback = unsafe { core::slice::from_raw_parts(BUF_VIRT as *const u8, 512) };
    print(console_slot, b"storage_client_test: scratch LBA first 16 bytes: ");
    for &byte in &readback[..16] {
        print_hex_byte(console_slot, byte);
    }
    print(console_slot, b"\n");

    if readback != &pattern[..] {
        print(console_slot, b"storage_client_test: FAIL (write/read-back mismatch)\n");
        libpcern::exit(1);
    }

    print(console_slot, b"storage_client_test: PASS\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
