//! Checkpoint J test fixture: exercises the real fs_fat32 protocol
//! (name-service lookup + shared-memory grant + open/read) against a
//! host-built FAT32 image (`mformat`/`mcopy`, see Makefile/grub.cfg's
//! temporary second `-drive`) containing a known file, HELLO.TXT. Prints
//! the file size and full contents over the console so the result can be
//! screendumped and compared against the host-side source text. Not part
//! of the default build; wired into main.rs only temporarily for this
//! checkpoint's verification.
//!
//! Checkpoint M also exercises the new `SYS_SPAWN_FROM_MEMORY` syscall
//! from here (load and run LOADED.BIN, see loaded_program.rs) rather than
//! from a separate fixture -- fs_fat32 only supports one client at a
//! time, so a second concurrent connection would clobber this one's.
//!
//! Phase 7, Checkpoint Q: for the same single-client reason, the new
//! write-support exercise (create a file, write enough to force a FAT
//! chain-extension, overwrite a middle range, reopen, read back
//! byte-for-byte) also runs here rather than as a separate fixture. Its
//! in-VM exit code is only half of what's actually checked -- see
//! `run_tests.sh`'s host-side `mtools` inspection of `test_fat32.img`
//! after QEMU exits, which independently confirms the written bytes
//! actually reached the disk image rather than this fixture's read-back
//! being satisfied by something fs_fat32 already held in memory.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use libpcern::{print, print_u32};

/// CSlot 1 is the name service (auto-granted). CSlot 2 is this task's
/// own inbox (see main.rs's temporary wiring for this fixture).
const MY_INBOX: u32 = 2;

const BUF_VIRT: u32 = 0x00A0_0000;

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

    // Checkpoint M: exercise SYS_SPAWN_FROM_MEMORY by loading and running
    // LOADED.BIN (see loaded_program.rs) through this exact connection --
    // a second fixture connecting to fs_fat32 concurrently would clobber
    // this one's (fs_fat32 only supports one client at a time, see its
    // own doc comment), so this reuses grant_slot/BUF_VIRT rather than
    // adding one. `send`'s capability transfer mints a derived child
    // rather than moving the original (see cap.rs/syscall.rs), so
    // grant_slot is still a valid MemoryGrant here even after fs_connect
    // already handed a copy to fs_fat32. Confirms the loaded program
    // actually *executed*, not just that the syscall returned a nonzero
    // task id, via its own distinctive exit code (see loaded_program.rs's
    // doc comment) -- checked from the kernel's own serial log by
    // run_tests.sh, since this fixture has no way to observe another
    // task's exit code directly.
    let loaded_size = match libpcern::fs_open(fs_slot, MY_INBOX, b"LOADED.BIN") {
        Some(s) => s,
        None => {
            print(console_slot, b"fs_client_test: FAIL (open LOADED.BIN)\n");
            libpcern::exit(1);
        }
    };

    if loaded_size == 0 || loaded_size > 512 {
        print(console_slot, b"fs_client_test: FAIL (LOADED.BIN unexpected size)\n");
        libpcern::exit(1);
    }

    let n = libpcern::fs_read(fs_slot, MY_INBOX, 0, loaded_size);
    if n != loaded_size {
        print(console_slot, b"fs_client_test: FAIL (LOADED.BIN short read)\n");
        libpcern::exit(1);
    }

    let loaded_task_id = libpcern::spawn_from_memory(&[grant_slot], loaded_size);
    if loaded_task_id == 0 {
        print(console_slot, b"fs_client_test: FAIL (spawn_from_memory)\n");
        libpcern::exit(1);
    }

    print(console_slot, b"fs_client_test: spawn_from_memory spawned task ");
    print_u32(console_slot, loaded_task_id);
    print(console_slot, b"\n");

    // Checkpoint Q: write support. Same connection as everything above
    // (grant_slot/BUF_VIRT) -- fs_fat32 still only supports one client
    // connection at a time, so a second fixture opening its own would
    // clobber this one's, exactly the same reason the spawn_from_memory
    // exercise above reuses this connection instead of adding one.
    const WRITE_LEN: u32 = 1500; // 3 clusters on this image (512 B/cluster)
    const OVERWRITE_OFFSET: u32 = 700;
    const OVERWRITE_LEN: u32 = 50;
    fn pattern_byte(i: u32) -> u8 {
        (i.wrapping_mul(17).wrapping_add(5) & 0xFF) as u8
    }
    fn overwrite_byte(i: u32) -> u8 {
        (i.wrapping_mul(53).wrapping_add(11) & 0xFF) as u8
    }
    fn expected_byte(i: u32) -> u8 {
        if i >= OVERWRITE_OFFSET && i < OVERWRITE_OFFSET + OVERWRITE_LEN {
            overwrite_byte(i - OVERWRITE_OFFSET)
        } else {
            pattern_byte(i)
        }
    }

    let initial_size = match libpcern::fs_open_for_write(fs_slot, MY_INBOX, b"WRTEST.TXT") {
        Some(s) => s,
        None => {
            print(console_slot, b"fs_client_test: FAIL (create WRTEST.TXT)\n");
            libpcern::exit(1);
        }
    };
    if initial_size != 0 {
        print(console_slot, b"fs_client_test: FAIL (new file not zero-length)\n");
        libpcern::exit(1);
    }

    let mut offset: u32 = 0;
    while offset < WRITE_LEN {
        let want = (WRITE_LEN - offset).min(512);
        let buf = unsafe { core::slice::from_raw_parts_mut(BUF_VIRT as *mut u8, want as usize) };
        for (i, b) in buf.iter_mut().enumerate() {
            *b = pattern_byte(offset + i as u32);
        }
        let n = libpcern::fs_write(fs_slot, MY_INBOX, offset, want);
        if n == 0 {
            print(console_slot, b"fs_client_test: FAIL (write stalled)\n");
            libpcern::exit(1);
        }
        offset += n;
    }

    {
        let buf = unsafe { core::slice::from_raw_parts_mut(BUF_VIRT as *mut u8, OVERWRITE_LEN as usize) };
        for (i, b) in buf.iter_mut().enumerate() {
            *b = overwrite_byte(i as u32);
        }
        let n = libpcern::fs_write(fs_slot, MY_INBOX, OVERWRITE_OFFSET, OVERWRITE_LEN);
        if n != OVERWRITE_LEN {
            print(console_slot, b"fs_client_test: FAIL (overwrite short)\n");
            libpcern::exit(1);
        }
    }

    let final_size = match libpcern::fs_open(fs_slot, MY_INBOX, b"WRTEST.TXT") {
        Some(s) => s,
        None => {
            print(console_slot, b"fs_client_test: FAIL (reopen WRTEST.TXT)\n");
            libpcern::exit(1);
        }
    };
    if final_size != WRITE_LEN {
        print(console_slot, b"fs_client_test: FAIL (WRTEST.TXT size mismatch)\n");
        libpcern::exit(1);
    }

    let mut read_offset: u32 = 0;
    let mut mismatch = false;
    loop {
        let n = libpcern::fs_read(fs_slot, MY_INBOX, read_offset, 512);
        if n == 0 {
            break;
        }
        let data = unsafe { core::slice::from_raw_parts(BUF_VIRT as *const u8, n as usize) };
        for (i, &b) in data.iter().enumerate() {
            if b != expected_byte(read_offset + i as u32) {
                mismatch = true;
            }
        }
        read_offset += n;
    }

    if mismatch || read_offset != WRITE_LEN {
        print(console_slot, b"fs_client_test: FAIL (WRTEST.TXT readback mismatch)\n");
        libpcern::exit(1);
    }

    print(console_slot, b"fs_client_test: WRTEST.TXT write/overwrite/readback OK\n");

    print(console_slot, b"fs_client_test: PASS\n");
    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
