//! Checkpoint J test fixture: exercises the real fs_fat32 protocol
//! (name-service lookup + shared-memory grant + open/read) against a
//! host-built FAT32 image (`mformat`/`mcopy`, see Makefile/grub.cfg's
//! temporary second `-drive`) containing a known file, HELLO.TXT. Prints
//! the file size and full contents over the console so the result can be
//! screendumped and compared against the host-side source text. Not part
//! of the default build; wired into main.rs only temporarily for this
//! checkpoint's verification.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted). CSlot 2 is this task's
/// own inbox (see main.rs's temporary wiring for this fixture).
const MY_INBOX: u32 = 2;
const OP_PUTCHAR: u32 = 0;

const BUF_VIRT: u32 = 0x00A0_0000;

fn print(console_slot: u32, s: &[u8]) {
    for &b in s {
        libpcern::send(console_slot, OP_PUTCHAR, b as u32, 0, 0);
    }
}

fn print_u32(console_slot: u32, mut n: u32) {
    if n == 0 {
        print(console_slot, b"0");
        return;
    }
    let mut digits = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    let mut buf = [0u8; 10];
    for j in 0..i {
        buf[j] = digits[i - 1 - j];
    }
    print(console_slot, &buf[..i]);
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let console_slot = libpcern::lookup_name(b"console", MY_INBOX).unwrap_or(0);
    // fs_fat32 needs several IPC round trips of its own (connecting to
    // storage_ata, reading the BPB) before it registers "fs" -- retry
    // rather than treating a too-early lookup as failure.
    let fs_slot = match libpcern::lookup_name_retry(b"fs", MY_INBOX, 1000) {
        Some(s) => s,
        None => {
            print(console_slot, b"fs_client_test: FAIL (no fs)\n");
            libpcern::exit(1);
        }
    };

    let grant_slot = libpcern::mem_alloc(BUF_VIRT);
    if grant_slot == 0 {
        print(console_slot, b"fs_client_test: FAIL (alloc)\n");
        libpcern::exit(1);
    }

    libpcern::fs_connect(fs_slot, grant_slot, MY_INBOX);

    let size = match libpcern::fs_open(fs_slot, MY_INBOX, b"HELLO.TXT") {
        Some(s) => s,
        None => {
            print(console_slot, b"fs_client_test: FAIL (open)\n");
            libpcern::exit(1);
        }
    };

    print(console_slot, b"fs_client_test: HELLO.TXT size=");
    print_u32(console_slot, size);
    print(console_slot, b" contents: [");

    let mut offset: u32 = 0;
    loop {
        let n = libpcern::fs_read(fs_slot, MY_INBOX, offset, 512);
        if n == 0 {
            break;
        }
        let data = unsafe { core::slice::from_raw_parts(BUF_VIRT as *const u8, n as usize) };
        print(console_slot, data);
        offset += n;
    }

    print(console_slot, b"]\n");

    // BIG.TXT spans multiple clusters (sectors_per_cluster=1 on this
    // image), exercising the FAT chain-walk in read_file/next_cluster
    // that HELLO.TXT's single-sector size never touches.
    let big_size = match libpcern::fs_open(fs_slot, MY_INBOX, b"BIG.TXT") {
        Some(s) => s,
        None => {
            print(console_slot, b"fs_client_test: FAIL (open BIG.TXT)\n");
            libpcern::exit(1);
        }
    };

    let mut total: u32 = 0;
    let mut checksum: u32 = 0;
    let mut offset: u32 = 0;
    loop {
        let n = libpcern::fs_read(fs_slot, MY_INBOX, offset, 512);
        if n == 0 {
            break;
        }
        let data = unsafe { core::slice::from_raw_parts(BUF_VIRT as *const u8, n as usize) };
        for &b in data {
            checksum = checksum.wrapping_add(b as u32);
        }
        total += n;
        offset += n;
    }

    print(console_slot, b"fs_client_test: BIG.TXT size=");
    print_u32(console_slot, big_size);
    print(console_slot, b" read=");
    print_u32(console_slot, total);
    print(console_slot, b" checksum=");
    print_u32(console_slot, checksum);
    print(console_slot, b"\n");

    if total != big_size || checksum != 0x4109b {
        print(console_slot, b"fs_client_test: FAIL (BIG.TXT mismatch)\n");
        libpcern::exit(1);
    }

    print(console_slot, b"fs_client_test: PASS\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
