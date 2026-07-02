//! Checkpoint I: the ATA/IDE PIO storage driver. Registers as "storage"
//! via the name service (Checkpoint H) and serves block reads over a
//! client-provided shared-memory grant (Checkpoint G), rather than
//! shipping bytes one word at a time -- proving the memory-grant
//! mechanism on a real protocol, not just the isolated mem_test fixture.
//!
//! Wire protocol (see libpcern's STORAGE_OP_* / storage_connect /
//! storage_read_block, shared with every client): a client connects once
//! by transferring a MemoryGrant for a page it already mapped locally
//! (`STORAGE_OP_SET_BUFFER`) and a capability to its own inbox to receive
//! replies on (`STORAGE_OP_SET_REPLY`) -- two messages, since a single
//! message can only carry one transfer -- then issues any number of
//! `STORAGE_OP_READ_BLOCK` requests. Only one client at a time is
//! supported: this phase has exactly one (`fs_fat32`), and the single
//! rendezvous inbox this driver reads from already serializes requests,
//! so there's nothing to arbitrate.
//!
//! The disk-plumbing spike this replaced (raw LBA0 hex dump straight to
//! the console, checked against a host-written pattern) proved the ATA
//! PIO protocol itself works before this real service went on top of it.

#![no_std]
#![no_main]

mod ata;
mod port;

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted -- see loader.rs in the
/// kernel); this is this task's own inbox.
const MY_INBOX: u32 = 2;

/// Where the client's shared page gets mapped in *this* task's own
/// address space -- independent of whatever virtual address the client
/// chose in its own space, since they're separate page directories.
const BUF_VIRT: u32 = 0x0080_0000;

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    ata::init();
    libpcern::register_name(b"storage", MY_INBOX);

    let mut buf_mapped = false;
    let mut reply_slot: u32 = 0;

    loop {
        let r = libpcern::recv(MY_INBOX);

        match r.w0 {
            libpcern::STORAGE_OP_SET_BUFFER => {
                if r.transferred_slot != 0 && libpcern::map_memory(r.transferred_slot, BUF_VIRT) == 0 {
                    buf_mapped = true;
                }
            }
            libpcern::STORAGE_OP_SET_REPLY => {
                if r.transferred_slot != 0 {
                    reply_slot = r.transferred_slot;
                }
            }
            libpcern::STORAGE_OP_READ_BLOCK => {
                if reply_slot == 0 {
                    continue;
                }
                if !buf_mapped {
                    libpcern::send(reply_slot, 0, 0, 0, 0);
                    continue;
                }
                let lba = r.w1;
                let buf = unsafe {
                    core::slice::from_raw_parts_mut(BUF_VIRT as *mut u8, ata::SECTOR_SIZE)
                };
                let sector: &mut [u8; ata::SECTOR_SIZE] = buf.try_into().unwrap();
                let ok = ata::read_sector(lba, sector);
                libpcern::send(reply_slot, if ok { 1 } else { 0 }, 0, 0, 0);
            }
            _ => {}
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
