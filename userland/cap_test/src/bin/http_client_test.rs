//! Checkpoint Y's http_client_test fixture: proves netstack's minimal TCP
//! client (connect/send/recv/close) works against *real* traffic -- a
//! genuine three-way handshake, a real HTTP-shaped request/response
//! exchange, and a real close handshake with an independent peer on the
//! wire (see `run_tcp_test.sh`'s Python TCP responder), not a
//! simulation. The same "prove it for real" bar nic_test/arp_icmp_test
//! were already held to.
//!
//! Speaks a deliberately tiny, fixed HTTP/1.0 exchange -- this fixture
//! and the peer script both hardcode the exact request/response bytes,
//! so this only proves the *transport* (TCP's own framing, sequencing,
//! and close handshake) carries arbitrary bytes correctly end to end,
//! not that netstack understands HTTP itself (it doesn't -- HTTP is
//! just what this checkpoint's traffic happens to look like, per
//! "enough transport to speak HTTP").

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted); this is this task's own
/// inbox, reused as both the one-shot name-lookup reply and netstack's
/// ongoing reply-to address -- safe here for the same reason
/// nic_test's identical reuse is: the lookup completes (a blocking
/// `recv`) before the TCP connection even starts, so there's no window
/// where a reply from each could race the other.
const MY_INBOX: u32 = 2;

const BUF_VIRT: u32 = 0x00B0_0000;

/// An address on the same virtual wire `run_tcp_test.sh`'s Python peer
/// answers ARP/TCP for -- distinct from netstack's own `STATIC_IP`
/// (10.0.2.15), and not any address QEMU's own slirp networking treats
/// specially, since this test uses the same raw `-netdev socket` frame
/// pipe Checkpoint X's test harness already proved out, not slirp.
const PEER_IP: [u8; 4] = [10, 0, 2, 1];
const PEER_PORT: u16 = 8080;

const REQUEST: &[u8] = b"GET / HTTP/1.0\r\nHost: 10.0.2.1\r\n\r\n";
const EXPECTED_RESPONSE: &[u8] =
    b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 13\r\n\r\nZephyrLite OK";

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let tcp_slot = match libpcern::lookup_name_retry(b"tcp", MY_INBOX, 1000) {
        Some(s) => s,
        None => libpcern::exit(1),
    };

    let grant_slot = libpcern::mem_alloc(BUF_VIRT);
    if grant_slot == 0 {
        libpcern::exit(1);
    }
    libpcern::tcp_connect_setup(tcp_slot, grant_slot, MY_INBOX);

    if !libpcern::tcp_open(tcp_slot, MY_INBOX, PEER_IP, PEER_PORT) {
        libpcern::exit(1);
    }

    {
        let buf = unsafe { core::slice::from_raw_parts_mut(BUF_VIRT as *mut u8, libpcern::TCP_MAX_TRANSFER) };
        buf[..REQUEST.len()].copy_from_slice(REQUEST);
    }
    let sent = libpcern::tcp_write(tcp_slot, MY_INBOX, REQUEST.len() as u32);
    if sent as usize != REQUEST.len() {
        libpcern::exit(1);
    }

    // The response may arrive as more than one TCP segment (and so more
    // than one TCP_OP_RECV reply) before the peer closes -- keep reading
    // until it does, accumulating into a local buffer since each reply
    // overwrites the same shared page.
    let mut response = [0u8; 256];
    let mut response_len = 0usize;
    loop {
        let n = libpcern::tcp_read(tcp_slot, MY_INBOX) as usize;
        if n == 0 {
            break; // peer closed -- no more data
        }
        if response_len + n > response.len() {
            libpcern::exit(1);
        }
        let buf = unsafe { core::slice::from_raw_parts(BUF_VIRT as *const u8, libpcern::TCP_MAX_TRANSFER) };
        response[response_len..response_len + n].copy_from_slice(&buf[..n]);
        response_len += n;
    }

    libpcern::tcp_close(tcp_slot, MY_INBOX);

    if response[..response_len] != *EXPECTED_RESPONSE {
        libpcern::exit(1);
    }

    libpcern::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
