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
//!
//! The request is padded past `netstack::MAX_SEGMENT_PAYLOAD` (1464
//! bytes) on purpose: a single `TCP_OP_SEND` for more than one frame's
//! worth of payload used to make netstack index straight past its NIC
//! frame buffer and panic, taking the whole task (and the ARP/ICMP
//! responder sharing it) down with it. Sending the request in a real
//! write loop -- calling `tcp_write` again with whatever `tcp_write`
//! didn't send, exactly the partial-write handling any correct caller
//! of a capped `SEND` needs -- both proves the fix (no panic, no lost
//! bytes) and exercises the general case a fixed request smaller than
//! one frame never could.

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

/// A query-string pad, the same shape a real long URL or cookie would
/// take -- deliberately more than one frame's worth (see `PAD_LEN`)
/// but still comfortably under `TCP_MAX_TRANSFER` (4096), so this stays
/// a request `netstack` can legitimately buffer and forward whole, just
/// not in a single `TCP_OP_SEND`. `run_tcp_test.sh`'s peer builds the
/// exact same bytes (`REQUEST_PREFIX + b"A" * PAD_LEN + REQUEST_SUFFIX`)
/// to check against.
const REQUEST_PREFIX: &[u8] = b"GET /?pad=";
const REQUEST_SUFFIX: &[u8] = b" HTTP/1.0\r\nHost: 10.0.2.1\r\n\r\n";
const PAD_BYTE: u8 = b'A';
/// More than `netstack::MAX_SEGMENT_PAYLOAD` (1464 bytes, not
/// referenced directly here -- this fixture only needs to know it's
/// exceeding *some* single-frame limit, not netstack's own internal
/// constant), so the prefix alone already forces more than one
/// `tcp_write` call to deliver the whole request.
const PAD_LEN: usize = 1600;
const REQUEST_LEN: usize = REQUEST_PREFIX.len() + PAD_LEN + REQUEST_SUFFIX.len();

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

    let mut request = [0u8; REQUEST_LEN];
    request[..REQUEST_PREFIX.len()].copy_from_slice(REQUEST_PREFIX);
    request[REQUEST_PREFIX.len()..REQUEST_PREFIX.len() + PAD_LEN].fill(PAD_BYTE);
    request[REQUEST_PREFIX.len() + PAD_LEN..].copy_from_slice(REQUEST_SUFFIX);

    // `tcp_write` can (and here, must) send less than requested in one
    // call -- netstack caps a single TCP_OP_SEND at one frame's worth
    // of payload, so the request above needs more than one `tcp_write`
    // call to deliver in full. Requesting the *entire* remaining request
    // on every call and re-copying whatever's left to the front of the
    // shared buffer for the next one is exactly how a correct caller
    // handles a capped/partial send in general, not a special case for
    // this fixture's oversized request.
    let mut sent_total = 0usize;
    let mut first_sent: Option<usize> = None;
    while sent_total < request.len() {
        let remaining = &request[sent_total..];
        {
            let buf = unsafe { core::slice::from_raw_parts_mut(BUF_VIRT as *mut u8, libpcern::TCP_MAX_TRANSFER) };
            buf[..remaining.len()].copy_from_slice(remaining);
        }
        let sent = libpcern::tcp_write(tcp_slot, MY_INBOX, remaining.len() as u32) as usize;
        if sent == 0 {
            libpcern::exit(1);
        }
        first_sent.get_or_insert(sent);
        sent_total += sent;
    }
    // The actual proof: requesting the whole request in that first call
    // above did NOT send it all at once -- if it had, either the fix
    // regressed (this send is bigger than one frame can hold, so
    // sending it whole again would mean the OOB write is back) or this
    // fixture's own request stopped being big enough to exercise the
    // cap at all. Either way, that's a real regression worth failing
    // loudly on rather than passing by accident.
    if first_sent == Some(request.len()) {
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
