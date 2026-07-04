//! Checkpoint W: the RTL8139 Fast Ethernet driver. Registers as "net" via
//! the name service and serves raw Ethernet frames in and out over a
//! client-provided shared-memory grant, the same protocol shape every
//! other driver in this project uses (see libpcern's NIC_OP_*/
//! nic_connect/nic_send/nic_recv/nic_get_mac). Deliberately narrow scope,
//! matching this project's own precedent for every checkpoint: no ARP,
//! no IP, nothing above the Ethernet-frame layer -- that's later
//! checkpoints' job. Only one client is supported, the same scope
//! narrowing storage_ata/fs_fat32/console_server already use.
//!
//! The card's I/O-port range and PCI interrupt line are discovered at
//! boot by the kernel's own PCI enumeration (kernel/src/pci.rs), not
//! known ahead of time the way console_server's/storage_ata's fixed
//! legacy ports are -- there's no way to hardcode a value here that both
//! sides already agree on. main.rs hands the discovered I/O base to this
//! driver the same way it hands console_server the VGA buffer: a
//! read-only `MemoryGrant` capability (CSlot 4) over a single physical
//! page it wrote the value into, reusing that existing mechanism rather
//! than inventing a new one just to carry one integer.
//!
//! On the receive side, only the *most recently received* frame is ever
//! held for a client to claim -- not a queue. If a second frame arrives
//! before `NIC_OP_RECV` claims the first, the first is silently dropped.
//! A real queue would need bounding and backpressure this checkpoint's
//! one-fixture test doesn't exercise; see console_server's raw-mode key
//! queue (Checkpoint R) for what that would look like if a future client
//! ever needs it.

#![no_std]
#![no_main]

mod port;
mod rtl8139;

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted). CSlot 2 is this task's own
/// inbox. CSlot 3 is the IrqControl capability, and CSlot 4 the
/// I/O-base info grant, both hand-wired by main.rs at spawn time --
/// see this module's own doc comment for the latter.
const MY_INBOX: u32 = 2;
const IRQ_CONTROL_SLOT: u32 = 3;
const NIC_INFO_SLOT: u32 = 4;

/// Where this task's own receive-ring, transmit buffer, and the
/// hand-wired I/O-base info page get mapped -- arbitrary addresses,
/// distinct from where a connected client's shared buffer gets mapped
/// (CLIENT_BUF_VIRT), since all of them are mapped simultaneously in
/// this same address space.
const NIC_INFO_VIRT: u32 = 0x0070_0000;
const RX_BUF_VIRT: u32 = 0x0080_0000;
const TX_BUF_VIRT: u32 = 0x0090_0000;
const CLIENT_BUF_VIRT: u32 = 0x00A0_0000;

/// `rtl8139::RX_BUF_BYTES` rounded up to whole pages (4 KiB) --
/// `mem_alloc_pages` only ever hands out whole pages.
const RX_BUF_PAGES: u32 = (rtl8139::RX_BUF_BYTES as u32).div_ceil(4096);

fn client_buf() -> &'static mut [u8] {
    unsafe { core::slice::from_raw_parts_mut(CLIENT_BUF_VIRT as *mut u8, rtl8139::MAX_FRAME_SIZE) }
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    if libpcern::map_memory(NIC_INFO_SLOT, NIC_INFO_VIRT) != 0 {
        libpcern::exit(1);
    }
    // Written by main.rs at spawn time -- see this module's own doc
    // comment for why a MemoryGrant, not a new capability kind, carries
    // this discovered value across.
    let io_base = unsafe { core::ptr::read_volatile(NIC_INFO_VIRT as *const u32) } as u16;

    let (rx_grant, rx_buf_phys) = libpcern::mem_alloc_pages(RX_BUF_VIRT, RX_BUF_PAGES);
    let (tx_grant, tx_buf_phys) = libpcern::mem_alloc_pages(TX_BUF_VIRT, 1);
    if rx_grant == 0 || tx_grant == 0 {
        libpcern::exit(1);
    }

    let mac = rtl8139::init(io_base, rx_buf_phys);

    libpcern::register_irq(IRQ_CONTROL_SLOT);
    libpcern::register_name(b"net", MY_INBOX);

    let mut cur_rx: u16 = 0;
    let mut last_rx = [0u8; rtl8139::MAX_FRAME_SIZE];
    let mut last_rx_len: usize = 0;

    let mut client_buf_mapped = false;
    let mut client_reply: u32 = 0;
    let mut recv_armed = false;

    loop {
        let r = libpcern::recv(MY_INBOX);

        if r.sender == libpcern::KERNEL_TASK_ID {
            let isr = rtl8139::ack_interrupt(io_base);
            if isr & rtl8139::ISR_ROK != 0 {
                if let Some(len) = rtl8139::receive(io_base, RX_BUF_VIRT as usize, &mut cur_rx, &mut last_rx) {
                    last_rx_len = len;
                    if recv_armed && client_reply != 0 && client_buf_mapped {
                        client_buf()[..last_rx_len].copy_from_slice(&last_rx[..last_rx_len]);
                        libpcern::send(client_reply, last_rx_len as u32, 0, 0, 0);
                        recv_armed = false;
                        last_rx_len = 0;
                    }
                }
            }
            continue;
        }

        match r.w0 {
            libpcern::NIC_OP_SET_BUFFER => {
                if r.transferred_slot != 0 && libpcern::map_memory(r.transferred_slot, CLIENT_BUF_VIRT) == 0 {
                    client_buf_mapped = true;
                }
            }
            libpcern::NIC_OP_SET_REPLY => {
                if r.transferred_slot != 0 {
                    client_reply = r.transferred_slot;
                }
            }
            libpcern::NIC_OP_GET_MAC => {
                if client_reply == 0 {
                    continue;
                }
                let w0 = u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]);
                let w1 = u32::from_le_bytes([mac[4], mac[5], 0, 0]);
                libpcern::send(client_reply, w0, w1, 0, 0);
            }
            libpcern::NIC_OP_SEND => {
                if client_reply == 0 {
                    continue;
                }
                if !client_buf_mapped {
                    libpcern::send(client_reply, 0, 0, 0, 0);
                    continue;
                }
                let len = (r.w1 as usize).min(rtl8139::MAX_FRAME_SIZE);
                let ok = rtl8139::send(io_base, TX_BUF_VIRT as usize, tx_buf_phys, &client_buf()[..len]);
                libpcern::send(client_reply, if ok { 1 } else { 0 }, 0, 0, 0);
            }
            libpcern::NIC_OP_RECV => {
                if client_reply == 0 {
                    continue;
                }
                if !client_buf_mapped {
                    libpcern::send(client_reply, 0, 0, 0, 0);
                    continue;
                }
                if last_rx_len > 0 {
                    client_buf()[..last_rx_len].copy_from_slice(&last_rx[..last_rx_len]);
                    libpcern::send(client_reply, last_rx_len as u32, 0, 0, 0);
                    last_rx_len = 0;
                } else {
                    recv_armed = true;
                }
            }
            _ => {}
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
