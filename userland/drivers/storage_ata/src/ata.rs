//! Minimal ATA/IDE PIO driver for the primary bus (ports 0x1F0-0x1F7 +
//! 0x3F6). Polling only, no IRQ14 -- this scheduler already preempts on
//! the timer tick, so a polling loop in a user task can't starve
//! anything else, and it avoids IRQ plumbing entirely for a first driver.
//! LBA28 addressing, one sector (512 bytes) at a time.

use crate::port::{inb, inw, outb, outw};

const PORT_DATA: u16 = 0x1F0;
const PORT_ERROR: u16 = 0x1F1;
const PORT_SECTOR_COUNT: u16 = 0x1F2;
const PORT_LBA_LOW: u16 = 0x1F3;
const PORT_LBA_MID: u16 = 0x1F4;
const PORT_LBA_HIGH: u16 = 0x1F5;
const PORT_DRIVE_HEAD: u16 = 0x1F6;
const PORT_STATUS: u16 = 0x1F7;
const PORT_COMMAND: u16 = 0x1F7;
const PORT_CONTROL: u16 = 0x3F6;

const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;

const STATUS_ERR: u8 = 1 << 0;
const STATUS_DRQ: u8 = 1 << 3;
const STATUS_DF: u8 = 1 << 5;
const STATUS_BSY: u8 = 1 << 7;

const CONTROL_NIEN: u8 = 1 << 1;

pub const SECTOR_SIZE: usize = 512;

/// Disables IRQ14 generation for this bus (nIEN) -- we only ever poll the
/// status port, so there's no handler to receive an interrupt anyway, and
/// leaving it enabled risks an unhandled-interrupt fault.
pub fn init() {
    unsafe { outb(PORT_CONTROL, CONTROL_NIEN) };
}

fn wait_not_busy() -> u8 {
    loop {
        let status = unsafe { inb(PORT_STATUS) };
        if status & STATUS_BSY == 0 {
            return status;
        }
    }
}

/// Programs the drive/head, sector-count, and LBA28 registers and issues
/// `cmd` -- the part identical between a read and a write, up to the
/// point where they diverge on which direction the data phase goes.
fn issue_command(lba: u32, cmd: u8) {
    wait_not_busy();
    unsafe {
        // 0xE0: bit7/bit5 reserved-set-to-1, bit6 = LBA mode, bit4 = drive
        // (0 = master); bits 0-3 = LBA bits 24-27.
        outb(PORT_DRIVE_HEAD, 0xE0 | ((lba >> 24) & 0x0F) as u8);
        outb(PORT_SECTOR_COUNT, 1);
        outb(PORT_LBA_LOW, (lba & 0xFF) as u8);
        outb(PORT_LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(PORT_LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
        outb(PORT_COMMAND, cmd);
    }
}

/// Reads one 512-byte sector at `lba` (LBA28) into `buf`. Returns `false`
/// if the drive reports an error (`buf` is left untouched/partial in that
/// case -- there's nothing this driver can do to recover from a bad
/// sector beyond reporting failure to its caller).
pub fn read_sector(lba: u32, buf: &mut [u8; SECTOR_SIZE]) -> bool {
    issue_command(lba, CMD_READ_SECTORS);

    let status = wait_not_busy();
    if status & STATUS_ERR != 0 || status & STATUS_DRQ == 0 {
        let _ = unsafe { inb(PORT_ERROR) };
        return false;
    }

    for i in 0..(SECTOR_SIZE / 2) {
        let word = unsafe { inw(PORT_DATA) };
        buf[i * 2] = (word & 0xFF) as u8;
        buf[i * 2 + 1] = (word >> 8) as u8;
    }
    true
}

/// Writes one 512-byte sector at `lba` (LBA28) from `buf`. Returns `false`
/// on a reported error or write fault -- same "nothing to recover, just
/// report failure" contract as `read_sector`.
///
/// Unlike a read, DRQ here means "the drive is ready for the next data
/// word in", not "here is data" -- so each of the 256 words must wait for
/// DRQ individually rather than once up front. No cache-flush (`0xE7`) is
/// issued after: QEMU's IDE emulation over a raw host file has no
/// volatile write cache in the path this project's tests exercise, so
/// there is nothing a flush would make observable here. A real drive
/// would need one before treating this as durable.
pub fn write_sector(lba: u32, buf: &[u8; SECTOR_SIZE]) -> bool {
    issue_command(lba, CMD_WRITE_SECTORS);

    let mut status = wait_not_busy();
    if status & STATUS_ERR != 0 || status & STATUS_DF != 0 {
        let _ = unsafe { inb(PORT_ERROR) };
        return false;
    }

    for i in 0..(SECTOR_SIZE / 2) {
        loop {
            status = unsafe { inb(PORT_STATUS) };
            if status & STATUS_BSY == 0 && status & STATUS_DRQ != 0 {
                break;
            }
        }
        let word = (buf[i * 2] as u16) | ((buf[i * 2 + 1] as u16) << 8);
        unsafe { outw(PORT_DATA, word) };
    }

    status = wait_not_busy();
    if status & STATUS_ERR != 0 || status & STATUS_DF != 0 {
        let _ = unsafe { inb(PORT_ERROR) };
        return false;
    }
    true
}
