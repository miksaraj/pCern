//! Packet-level logic: parsing and in-place reply construction for ARP
//! and ICMP echo, plus the Internet checksum both IPv4 and ICMP share.
//! Deliberately narrow, matching every other checkpoint's scope: answers
//! an ARP request for this host's own static IP, and an ICMP echo
//! request (ping) addressed to it -- nothing else. No outbound
//! connections, no ARP cache, no fragmentation, no IPv4 options (a
//! header with any length other than the plain 20-byte minimum is
//! ignored rather than parsed). Incoming checksums are not verified
//! before use -- Ethernet's own CRC (checked by the NIC hardware, never
//! handed up to us at all if it fails) is the integrity layer this
//! narrow scope relies on, the same way this project's other protocols
//! trust IPC's kernel-mediated delivery rather than re-checking it.

const ETH_HEADER_LEN: usize = 14;
const ARP_LEN: usize = 28;
const IPV4_HEADER_LEN: usize = 20;
const ICMP_HEADER_LEN: usize = 8;
/// Real Ethernet's minimum frame size before the 4-byte FCS the
/// card/hardware appends on its own -- this driver stack doesn't pad
/// short frames itself (see net_rtl8139's own send()), so any reply
/// built shorter than this is zero-padded up to it before sending.
const MIN_FRAME_LEN: usize = 60;

const ETHERTYPE_ARP: [u8; 2] = [0x08, 0x06];
const ETHERTYPE_IPV4: [u8; 2] = [0x08, 0x00];

const ARP_HTYPE_ETHERNET: [u8; 2] = [0x00, 0x01];
const ARP_PTYPE_IPV4: [u8; 2] = [0x08, 0x00];
const ARP_OP_REQUEST: [u8; 2] = [0x00, 0x01];
const ARP_OP_REPLY: [u8; 2] = [0x00, 0x02];

/// IPv4 "version 4, header length 5 (x4 bytes) = 20 bytes, no options" --
/// the only value this narrow scope understands.
const IPV4_VER_IHL_NO_OPTIONS: u8 = 0x45;
const IPV4_PROTO_ICMP: u8 = 1;
/// Not derived from anything -- this host doesn't route, so there's no
/// received TTL to decrement or forward; a plain, ordinary default for
/// packets it originates itself.
const DEFAULT_TTL: u8 = 64;

const ICMP_TYPE_ECHO_REQUEST: u8 = 8;
const ICMP_TYPE_ECHO_REPLY: u8 = 0;

/// The standard Internet checksum (RFC 1071): one's-complement sum of
/// 16-bit big-endian words (an odd trailing byte is treated as if
/// followed by a zero byte), carries folded back in, then complemented.
/// Shared by IPv4's header checksum and ICMP's message checksum -- same
/// algorithm, different byte ranges.
fn checksum16(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut chunks = data.chunks_exact(2);
    for word in &mut chunks {
        sum += u16::from_be_bytes([word[0], word[1]]) as u32;
    }
    if let [last] = *chunks.remainder() {
        sum += (last as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Inspects `buf[..len]` (a frame `net_rtl8139` just delivered) and, if
/// it's an ARP request or ICMP echo request addressed to `my_ip`,
/// rewrites it *in place* into the corresponding reply and returns the
/// reply's own length. `buf`'s capacity beyond `len` may still hold
/// bytes from a previous call -- callers must only ever transmit exactly
/// the returned length, never `buf.len()` itself.
pub fn handle_frame(buf: &mut [u8], len: usize, my_mac: [u8; 6], my_ip: [u8; 4]) -> Option<usize> {
    let buf = &mut buf[..len];
    if buf.len() < ETH_HEADER_LEN {
        return None;
    }
    let ethertype = [buf[12], buf[13]];
    if ethertype == ETHERTYPE_ARP {
        handle_arp(buf, my_mac, my_ip)
    } else if ethertype == ETHERTYPE_IPV4 {
        handle_icmp_echo(buf, my_mac, my_ip)
    } else {
        None
    }
}

/// If `buf` is an ARP request asking for `my_ip`, rewrites it in place
/// into the matching reply (swapping every sender/target field) and
/// returns its length -- always the same as the request's own, since an
/// ARP reply carries exactly the same fields as a request.
fn handle_arp(buf: &mut [u8], my_mac: [u8; 6], my_ip: [u8; 4]) -> Option<usize> {
    const ARP: usize = ETH_HEADER_LEN;
    if buf.len() < ARP + ARP_LEN {
        return None;
    }
    if buf[ARP..ARP + 2] != ARP_HTYPE_ETHERNET || buf[ARP + 2..ARP + 4] != ARP_PTYPE_IPV4 {
        return None;
    }
    if buf[ARP + 4] != 6 || buf[ARP + 5] != 4 {
        return None; // HLEN/PLEN: only plain Ethernet+IPv4 ARP understood
    }
    if buf[ARP + 6..ARP + 8] != ARP_OP_REQUEST {
        return None; // only answer requests -- replies/other opcodes ignored
    }
    if buf[ARP + 24..ARP + 28] != my_ip {
        return None; // not asking about us
    }

    let mut sender_mac = [0u8; 6];
    sender_mac.copy_from_slice(&buf[ARP + 8..ARP + 14]);
    let mut sender_ip = [0u8; 4];
    sender_ip.copy_from_slice(&buf[ARP + 14..ARP + 18]);

    buf[0..6].copy_from_slice(&sender_mac); // Ethernet dest: back to the requester
    buf[6..12].copy_from_slice(&my_mac); // Ethernet src: us
    buf[ARP + 6..ARP + 8].copy_from_slice(&ARP_OP_REPLY);
    buf[ARP + 8..ARP + 14].copy_from_slice(&my_mac); // SHA
    buf[ARP + 14..ARP + 18].copy_from_slice(&my_ip); // SPA
    buf[ARP + 18..ARP + 24].copy_from_slice(&sender_mac); // THA
    buf[ARP + 24..ARP + 28].copy_from_slice(&sender_ip); // TPA

    Some(buf.len())
}

/// If `buf` is an ICMP echo request addressed to `my_ip` (a plain
/// 20-byte IPv4 header, no options, protocol ICMP), rewrites it in place
/// into the matching echo reply -- identifier, sequence, and payload
/// bytes are left completely untouched, since a ping reply must echo
/// them back verbatim; only the type byte and the three checksums
/// (ICMP's own, then IPv4's, after the address swap) change. Returns the
/// reply's length, zero-padded up to `MIN_FRAME_LEN` if the echoed
/// payload was short enough to need it.
fn handle_icmp_echo(buf: &mut [u8], my_mac: [u8; 6], my_ip: [u8; 4]) -> Option<usize> {
    const IP: usize = ETH_HEADER_LEN;
    if buf.len() < IP + IPV4_HEADER_LEN {
        return None;
    }
    if buf[IP] != IPV4_VER_IHL_NO_OPTIONS {
        return None; // IHL != 5 (options present) -- out of scope, ignored
    }
    if buf[IP + 9] != IPV4_PROTO_ICMP {
        return None;
    }
    if buf[IP + 16..IP + 20] != my_ip {
        return None; // not addressed to us
    }

    let total_len = u16::from_be_bytes([buf[IP + 2], buf[IP + 3]]) as usize;
    if total_len < IPV4_HEADER_LEN || IP + total_len > buf.len() {
        return None; // malformed/truncated length field
    }

    let icmp = IP + IPV4_HEADER_LEN;
    let icmp_len = total_len - IPV4_HEADER_LEN;
    if icmp_len < ICMP_HEADER_LEN {
        return None;
    }
    if buf[icmp] != ICMP_TYPE_ECHO_REQUEST || buf[icmp + 1] != 0 {
        return None; // only plain echo request (code 0) is answered
    }

    // ICMP: flip type to echo reply, zero the checksum field, recompute
    // over the untouched identifier/sequence/payload.
    buf[icmp] = ICMP_TYPE_ECHO_REPLY;
    buf[icmp + 2] = 0;
    buf[icmp + 3] = 0;
    let icmp_sum = checksum16(&buf[icmp..icmp + icmp_len]);
    buf[icmp + 2..icmp + 4].copy_from_slice(&icmp_sum.to_be_bytes());

    // IPv4: swap source/destination, refresh TTL, recompute the header
    // checksum (the header's own length/protocol/id fields are already
    // correct for the reply -- only the addresses and TTL changed).
    let mut src_ip = [0u8; 4];
    src_ip.copy_from_slice(&buf[IP + 12..IP + 16]);
    buf.copy_within(IP + 16..IP + 20, IP + 12);
    buf[IP + 16..IP + 20].copy_from_slice(&src_ip);
    buf[IP + 8] = DEFAULT_TTL;
    buf[IP + 10] = 0;
    buf[IP + 11] = 0;
    let ip_sum = checksum16(&buf[IP..IP + IPV4_HEADER_LEN]);
    buf[IP + 10..IP + 12].copy_from_slice(&ip_sum.to_be_bytes());

    // Ethernet: swap source/destination.
    let mut sender_mac = [0u8; 6];
    sender_mac.copy_from_slice(&buf[6..12]);
    buf[0..6].copy_from_slice(&sender_mac);
    buf[6..12].copy_from_slice(&my_mac);

    let reply_len = IP + total_len;
    if reply_len < MIN_FRAME_LEN {
        buf[reply_len..MIN_FRAME_LEN].fill(0);
        Some(MIN_FRAME_LEN)
    } else {
        Some(reply_len)
    }
}
