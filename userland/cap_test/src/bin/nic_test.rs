//! Checkpoint W's nic_test fixture: proves the RTL8139 driver's raw
//! Ethernet frame in/out path works against *real* traffic -- QEMU's
//! usermode network stack (`-netdev user`), not a simulation -- the same
//! "prove it for real" approach this project's console-input/raw-input
//! fixtures already established for keystrokes.
//!
//! Hand-builds a complete Ethernet+ARP request frame (broadcast,
//! requesting the hardware address of 10.0.2.2 -- the fixed gateway
//! address QEMU's usermode network stack always answers ARP for, with no
//! DHCP or other setup needed first), sends it via `NIC_OP_SEND`, then
//! blocks on `NIC_OP_RECV` expecting the gateway's real ARP reply to
//! come back through the same driver. Checks only the fields this
//! checkpoint's scope actually covers (Ethernet framing + raw ARP bytes)
//! -- nothing about IP itself, that's a later checkpoint's job.
//! run_nic_test.sh separately verifies the same round trip by inspecting
//! a real packet capture QEMU wrote to disk, independent of anything
//! this fixture itself believes -- the same "don't just trust the in-VM
//! report" pattern run_tests.sh's own WRTEST.TXT check already
//! established.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted); this is this task's own
/// inbox, reused as both the one-shot name-lookup reply and the NIC
/// driver's ongoing reply-to address -- safe here for the same reason
/// storage_client_test/fs_client_test's identical reuse is: the lookup
/// completes (a blocking `recv`) before the NIC connection even starts,
/// so there's no window where a reply from each could race the other.
const MY_INBOX: u32 = 2;

const BUF_VIRT: u32 = 0x00B0_0000;

const ETHERTYPE_ARP: [u8; 2] = [0x08, 0x06];
const ARP_REQUEST: [u8; 2] = [0x00, 0x01];
const ARP_REPLY: [u8; 2] = [0x00, 0x02];
/// QEMU usermode networking's fixed virtual gateway address -- always
/// answers ARP for itself, no DHCP or other setup needed first.
const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];
/// The conventional guest address under QEMU usermode networking; not
/// actually configured on any interface here (this checkpoint's driver
/// has no concept of IP at all) -- just a plausible sender address to
/// put in the ARP request, which slirp doesn't validate against a lease.
const GUEST_IP: [u8; 4] = [10, 0, 2, 15];

/// Writes a complete Ethernet+ARP request frame into `out`, returning its
/// length (always 60: real Ethernet's minimum frame size before the
/// 4-byte FCS, which the driver/card handle on their own -- zero-padded
/// up to it, matching what real hardware would otherwise do for us).
fn build_arp_request(src_mac: [u8; 6], out: &mut [u8]) -> usize {
    out[0..6].fill(0xFF); // dest: broadcast
    out[6..12].copy_from_slice(&src_mac);
    out[12..14].copy_from_slice(&ETHERTYPE_ARP);

    let arp = &mut out[14..14 + 28];
    arp[0..2].copy_from_slice(&[0x00, 0x01]); // HTYPE: Ethernet
    arp[2..4].copy_from_slice(&[0x08, 0x00]); // PTYPE: IPv4
    arp[4] = 6; // HLEN
    arp[5] = 4; // PLEN
    arp[6..8].copy_from_slice(&ARP_REQUEST);
    arp[8..14].copy_from_slice(&src_mac); // SHA
    arp[14..18].copy_from_slice(&GUEST_IP); // SPA
    arp[18..24].fill(0x00); // THA: unknown, that's the whole point of asking
    arp[24..28].copy_from_slice(&GATEWAY_IP); // TPA

    let payload_len = 14 + 28;
    out[payload_len..60].fill(0);
    60
}

/// Checks `frame` is an ARP reply from `GATEWAY_IP`, addressed to us.
fn is_valid_arp_reply(frame: &[u8], my_mac: [u8; 6]) -> bool {
    if frame.len() < 14 + 28 {
        return false;
    }
    if frame[0..6] != my_mac[..] {
        return false; // not addressed to us
    }
    if frame[12..14] != ETHERTYPE_ARP[..] {
        return false;
    }
    let arp = &frame[14..14 + 28];
    arp[6..8] == ARP_REPLY[..] && arp[14..18] == GATEWAY_IP[..]
}

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

    let frame_len = {
        let buf = unsafe { core::slice::from_raw_parts_mut(BUF_VIRT as *mut u8, libpcern::NIC_MAX_FRAME) };
        build_arp_request(my_mac, buf)
    };

    if !libpcern::nic_send(nic_slot, MY_INBOX, frame_len as u32) {
        libpcern::exit(1);
    }

    let reply_len = libpcern::nic_recv(nic_slot, MY_INBOX) as usize;
    let buf = unsafe { core::slice::from_raw_parts(BUF_VIRT as *const u8, libpcern::NIC_MAX_FRAME) };
    if reply_len == 0 || !is_valid_arp_reply(&buf[..reply_len], my_mac) {
        libpcern::exit(1);
    }

    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
