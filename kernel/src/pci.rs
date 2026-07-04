//! Checkpoint W: a minimal PCI configuration-space enumerator -- legacy
//! port I/O via 0xCF8 (CONFIG_ADDRESS)/0xCFC (CONFIG_DATA), brute-force
//! scanning every bus/device/function rather than walking bridge
//! topology. This kernel boots under QEMU's i440fx/q35 chipset, and every
//! device this project cares about sits on bus 0 with no bridges of its
//! own to recurse through -- a flat scan is simpler and, at a few tens of
//! thousands of dword reads once at boot, cheap enough not to matter.
//!
//! Resource assignment (which I/O ports/IRQ line a device actually gets)
//! is trusted to already be correct by the time this runs -- done by
//! firmware (SeaBIOS/OVMF) before the multiboot kernel ever starts, the
//! same assumption every real OS's own PCI driver makes. This module only
//! *reads* what firmware already assigned and enables the device; it
//! never allocates a BAR or routes an interrupt itself.

use crate::port::{inl, outl};

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

/// A PCI function found during enumeration: its bus/device/function
/// address (needed to read more of its config space later) and the two
/// identifying fields callers match specific drivers against.
#[derive(Clone, Copy)]
pub struct PciDevice {
    bus: u8,
    device: u8,
    function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
}

fn config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | (bus as u32) << 16
        | (device as u32) << 11
        | (function as u32) << 8
        | (offset as u32 & 0xFC)
}

fn read_config_dword(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    unsafe {
        outl(CONFIG_ADDRESS, config_address(bus, device, function, offset));
        inl(CONFIG_DATA)
    }
}

fn write_config_dword(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    unsafe {
        outl(CONFIG_ADDRESS, config_address(bus, device, function, offset));
        outl(CONFIG_DATA, value);
    }
}

impl PciDevice {
    /// Base Address Register 0 (offset 0x10), raw -- the caller is
    /// expected to know its own device's BAR layout (I/O- vs
    /// memory-space, which bits are address vs flags). The only BAR any
    /// driver needs today: the RTL8139 (Checkpoint W's one PCI-attached
    /// device) has just the one, I/O-space BAR.
    pub fn bar0(&self) -> u32 {
        read_config_dword(self.bus, self.device, self.function, 0x10)
    }

    /// The Interrupt Line register (offset 0x3C, low byte): the legacy
    /// PIC IRQ line firmware routed this device's interrupt pin to.
    pub fn interrupt_line(&self) -> u8 {
        (read_config_dword(self.bus, self.device, self.function, 0x3C) & 0xFF) as u8
    }

    /// Sets the I/O Space Enable (bit 0) and Bus Master Enable (bit 2)
    /// bits in the Command register (offset 0x04) -- needed respectively
    /// for a driver to reach the device through its I/O-space BAR at all,
    /// and for the device's own DMA engine to read/write system memory.
    /// Firmware doesn't reliably leave either bit set for us.
    pub fn enable(&self) {
        let command_status = read_config_dword(self.bus, self.device, self.function, 0x04);
        write_config_dword(self.bus, self.device, self.function, 0x04, command_status | 0x0005);
    }
}

/// Scans every device/function on bus 0 for the first one matching
/// `vendor_id`/`device_id`. `None` if no such device is attached. Bus 0
/// only, not every possible bus (0-255): this module's own doc comment
/// already asserts every device this project cares about sits there with
/// no bridges to recurse through, so scanning further buses would only
/// cost more boot-time port I/O for a case that can't occur under this
/// kernel's target chipset.
pub fn find_device(vendor_id: u16, device_id: u16) -> Option<PciDevice> {
    const BUS: u8 = 0;
    for device in 0..32u8 {
        let vendor_device0 = read_config_dword(BUS, device, 0, 0x00);
        let vendor0 = (vendor_device0 & 0xFFFF) as u16;
        if vendor0 == 0xFFFF {
            continue; // nothing at this bus/device
        }
        if vendor0 == vendor_id && (vendor_device0 >> 16) as u16 == device_id {
            return Some(PciDevice { bus: BUS, device, function: 0, vendor_id, device_id });
        }

        // Bit 7 of the header-type byte (offset 0x0E) marks a
        // multi-function device -- only then is it worth probing
        // functions 1-7 too (function 0 is always implicitly probed
        // above, present or not).
        let header_type = (read_config_dword(BUS, device, 0, 0x0C) >> 16) as u8;
        if header_type & 0x80 == 0 {
            continue;
        }
        for function in 1..8u8 {
            let vendor_device = read_config_dword(BUS, device, function, 0x00);
            let f_vendor = (vendor_device & 0xFFFF) as u16;
            if f_vendor == 0xFFFF {
                continue;
            }
            if f_vendor == vendor_id && (vendor_device >> 16) as u16 == device_id {
                return Some(PciDevice { bus: BUS, device, function, vendor_id, device_id });
            }
        }
    }
    None
}
