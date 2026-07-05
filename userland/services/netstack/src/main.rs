//! A minimal ARP + IPv4 + ICMP responder: claims a static IP address and
//! answers ARP requests and ICMP echo requests (ping) for it, over raw
//! Ethernet frames served by `net_rtl8139` (see libpcern's
//! nic_connect/nic_get_mac/nic_send/nic_recv). No outbound connections
//! of its own, no DHCP, no ARP cache, no IP forwarding -- the first
//! genuinely externally-observable networking milestone, matching every
//! earlier checkpoint's own narrow-scope precedent. Packet-level parsing
//! and reply construction lives in `proto.rs`; this file is purely the
//! IPC glue (connect to "net", loop, hand each received frame to
//! `proto::handle_frame`, transmit whatever it builds).
//!
//! Only one client (this task) of `net_rtl8139` -- the same "one client
//! at a time" scope every driver in this project already has -- so
//! nothing else can be a NIC client simultaneously with this service.

#![no_std]
#![no_main]

mod proto;

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted); this is this task's own
/// inbox, reused as both the one-shot name-lookup reply and net_rtl8139's
/// ongoing reply-to address -- safe here for the same reason
/// nic_test's identical reuse is: nothing else ever sends to it.
const MY_INBOX: u32 = 2;

const BUF_VIRT: u32 = 0x00B0_0000;

/// This host's own address, claimed unconditionally at boot. Hardcoded,
/// not learned via DHCP or configured any other way -- there's no
/// mechanism for either yet, and this checkpoint's whole point is
/// proving the ARP/ICMP responder path works at all, not address
/// configuration. Matches the conventional guest address QEMU's own
/// usermode networking (`-netdev user`) assumes, so it works out of the
/// box under the same setup `net_rtl8139`'s own test harness already
/// uses, without colliding with slirp's fixed gateway (10.0.2.2) or DNS
/// (10.0.2.3) addresses.
const STATIC_IP: [u8; 4] = [10, 0, 2, 15];

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let nic_slot = match libpcern::lookup_name_retry(b"net", MY_INBOX, 1000) {
        Some(s) => s,
        None => libpcern::exit(1),
    };

    let grant_slot = libpcern::mem_alloc(BUF_VIRT);
    if grant_slot == 0 {
        libpcern::exit(1);
    }
    libpcern::nic_connect(nic_slot, grant_slot, MY_INBOX);

    let my_mac = libpcern::nic_get_mac(nic_slot, MY_INBOX);

    loop {
        let len = libpcern::nic_recv(nic_slot, MY_INBOX) as usize;
        if len == 0 {
            continue;
        }
        let buf = unsafe { core::slice::from_raw_parts_mut(BUF_VIRT as *mut u8, libpcern::NIC_MAX_FRAME) };
        if let Some(reply_len) = proto::handle_frame(buf, len, my_mac, STATIC_IP) {
            libpcern::nic_send(nic_slot, MY_INBOX, reply_len as u32);
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
