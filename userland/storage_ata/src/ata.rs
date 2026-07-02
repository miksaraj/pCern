//! Minimal ATA/IDE PIO driver for the primary bus (ports 0x1F0-0x1F7 +
//! 0x3F6). Polling only, no IRQ14 -- this scheduler already preempts on
//! the timer tick, so a polling loop in a user task can't starve
//! anything else, and it avoids IRQ plumbing entirely for a first driver.
//! LBA28 addressing, one sector (512 bytes) at a time.

use crate::port::{inb, inw, outb};

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

const STATUS_ERR: u8 = 1 << 0;
const STATUS_DRQ: u8 = 1 << 3;
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

/// Reads one 512-byte sector at `lba` (LBA28) into `buf`. Returns `false`
/// if the drive reports an error (`buf` is left untouched/partial in that
/// case -- there's nothing this driver can do to recover from a bad
/// sector beyond reporting failure to its caller).
pub fn read_sector(lba: u32, buf: &mut [u8; SECTOR_SIZE]) -> bool {
    wait_not_busy();

    unsafe {
        // 0xE0: bit7/bit5 reserved-set-to-1, bit6 = LBA mode, bit4 = drive
        // (0 = master); bits 0-3 = LBA bits 24-27.
        outb(PORT_DRIVE_HEAD, 0xE0 | ((lba >> 24) & 0x0F) as u8);
        outb(PORT_SECTOR_COUNT, 1);
        outb(PORT_LBA_LOW, (lba & 0xFF) as u8);
        outb(PORT_LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(PORT_LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
        outb(PORT_COMMAND, CMD_READ_SECTORS);
    }

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
