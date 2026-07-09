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

pub(crate) const ETH_HEADER_LEN: usize = 14;
const ARP_LEN: usize = 28;
pub(crate) const IPV4_HEADER_LEN: usize = 20;
const ICMP_HEADER_LEN: usize = 8;
/// Real Ethernet's minimum frame size before the 4-byte FCS the
/// card/hardware appends on its own -- this driver stack doesn't pad
/// short frames itself (see net_rtl8139's own send()), so any reply
/// built shorter than this is zero-padded up to it before sending.
pub(crate) const MIN_FRAME_LEN: usize = 60;

pub(crate) const ETHERTYPE_ARP: [u8; 2] = [0x08, 0x06];
pub(crate) const ETHERTYPE_IPV4: [u8; 2] = [0x08, 0x00];

const ARP_HTYPE_ETHERNET: [u8; 2] = [0x00, 0x01];
const ARP_PTYPE_IPV4: [u8; 2] = [0x08, 0x00];
const ARP_OP_REQUEST: [u8; 2] = [0x00, 0x01];
const ARP_OP_REPLY: [u8; 2] = [0x00, 0x02];

/// IPv4 "version 4, header length 5 (x4 bytes) = 20 bytes, no options" --
/// the only value this narrow scope understands.
pub(crate) const IPV4_VER_IHL_NO_OPTIONS: u8 = 0x45;
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
/// Shared by IPv4's header checksum, ICMP's message checksum, and (via
/// `checksum16_accumulate`/`checksum16_finish` below) TCP's own
/// pseudo-header checksum in `tcp.rs` -- same algorithm throughout,
/// different byte ranges.
fn checksum16(data: &[u8]) -> u16 {
    checksum16_finish(checksum16_accumulate(0, data))
}

/// The accumulation half of `checksum16`, split out so a caller that
/// needs to sum more than one non-adjacent byte range (TCP's
/// pseudo-header, then the segment itself, without copying both into one
/// combined buffer first) can feed them through the same running sum
/// before folding and complementing.
pub(crate) fn checksum16_accumulate(sum: u32, data: &[u8]) -> u32 {
    let mut sum = sum;
    let mut chunks = data.chunks_exact(2);
    for word in &mut chunks {
        sum += u16::from_be_bytes([word[0], word[1]]) as u32;
    }
    if let [last] = *chunks.remainder() {
        sum += (last as u32) << 8;
    }
    sum
}

/// The fold-and-complement half of `checksum16` -- see
/// `checksum16_accumulate`.
pub(crate) fn checksum16_finish(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Inspects `buf[..len]` (a frame `net_rtl8139` just delivered) and, if
/// it's an ARP request or ICMP echo request addressed to `my_ip`,
/// rewrites it *in place* into the corresponding reply and returns the
/// reply's own length. `buf` itself is never truncated to `len`: a reply
/// shorter than `MIN_FRAME_LEN` is zero-padded into the bytes just past
/// the received frame, which requires write access to `buf`'s full
/// capacity, not just the `len` bytes that were actually received --
/// truncating `buf` to `len` up front (an earlier version of this
/// function did exactly that) made that padding step index past the end
/// of the resulting slice and panic on any request shorter than
/// `MIN_FRAME_LEN`. Every read of `buf` below is still bounds-checked
/// against `len`, never against `buf.len()`, so bytes past `len` (old
/// data from a previous frame) are never treated as this request's own.
pub fn handle_frame(buf: &mut [u8], len: usize, my_mac: [u8; 6], my_ip: [u8; 4]) -> Option<usize> {
    if len < ETH_HEADER_LEN || len > buf.len() {
        return None;
    }
    let ethertype = [buf[12], buf[13]];
    if ethertype == ETHERTYPE_ARP {
        handle_arp(buf, len, my_mac, my_ip)
    } else if ethertype == ETHERTYPE_IPV4 {
        handle_icmp_echo(buf, len, my_mac, my_ip)
    } else {
        None
    }
}

/// Zero-pads `buf[reply_len..]` up to `MIN_FRAME_LEN` if `reply_len` is
/// short of it, returning whichever length was actually sent. Requires
/// `buf.len() >= MIN_FRAME_LEN`, always true here since `handle_frame`
/// never truncates `buf` down to the received frame's own length.
pub(crate) fn pad_to_min_frame(buf: &mut [u8], reply_len: usize) -> usize {
    if reply_len < MIN_FRAME_LEN {
        buf[reply_len..MIN_FRAME_LEN].fill(0);
        MIN_FRAME_LEN
    } else {
        reply_len
    }
}

/// If `buf[..len]` is an ARP request asking for `my_ip`, rewrites it in
/// place into the matching reply (swapping every sender/target field)
/// and returns its length, zero-padded up to `MIN_FRAME_LEN` if the
/// request arrived shorter than that (an ARP reply is otherwise always
/// exactly as long as the request that prompted it).
fn handle_arp(buf: &mut [u8], len: usize, my_mac: [u8; 6], my_ip: [u8; 4]) -> Option<usize> {
    const ARP: usize = ETH_HEADER_LEN;
    if len < ARP + ARP_LEN {
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

    Some(pad_to_min_frame(buf, ARP + ARP_LEN))
}

/// If `buf[..len]` is an ICMP echo request addressed to `my_ip` (a plain
/// 20-byte IPv4 header, no options, protocol ICMP), rewrites it in place
/// into the matching echo reply -- identifier, sequence, and payload
/// bytes are left completely untouched, since a ping reply must echo
/// them back verbatim; only the type byte and the three checksums
/// (ICMP's own, then IPv4's, after the address swap) change. Returns the
/// reply's length, zero-padded up to `MIN_FRAME_LEN` if the echoed
/// payload was short enough to need it.
fn handle_icmp_echo(buf: &mut [u8], len: usize, my_mac: [u8; 6], my_ip: [u8; 4]) -> Option<usize> {
    const IP: usize = ETH_HEADER_LEN;
    if len < IP + IPV4_HEADER_LEN {
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
    if total_len < IPV4_HEADER_LEN || IP + total_len > len {
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

    Some(pad_to_min_frame(buf, IP + total_len))
}

/// Builds an ARP request asking who has `target_ip`, into `buf`,
/// zero-padded to `MIN_FRAME_LEN` like every other frame this module
/// sends. The mirror image of `handle_arp`'s reply-construction fields --
/// used by `netstack`'s TCP client to resolve a connection's peer before
/// it can address it at the Ethernet layer, the one ARP role Checkpoint
/// X's pure responder never needed.
pub fn build_arp_request(buf: &mut [u8], my_mac: [u8; 6], my_ip: [u8; 4], target_ip: [u8; 4]) -> usize {
    const ARP: usize = ETH_HEADER_LEN;
    buf[0..6].fill(0xff); // Ethernet dest: broadcast, nobody specific to address this to yet
    buf[6..12].copy_from_slice(&my_mac);
    buf[12..14].copy_from_slice(&ETHERTYPE_ARP);
    buf[ARP..ARP + 2].copy_from_slice(&ARP_HTYPE_ETHERNET);
    buf[ARP + 2..ARP + 4].copy_from_slice(&ARP_PTYPE_IPV4);
    buf[ARP + 4] = 6;
    buf[ARP + 5] = 4;
    buf[ARP + 6..ARP + 8].copy_from_slice(&ARP_OP_REQUEST);
    buf[ARP + 8..ARP + 14].copy_from_slice(&my_mac); // SHA
    buf[ARP + 14..ARP + 18].copy_from_slice(&my_ip); // SPA
    buf[ARP + 18..ARP + 24].fill(0); // THA: unknown -- that's what we're asking for
    buf[ARP + 24..ARP + 28].copy_from_slice(&target_ip); // TPA

    pad_to_min_frame(buf, ARP + ARP_LEN)
}

/// If `buf[..len]` is an ARP reply naming `target_ip` as its own sender
/// address, returns that sender's MAC -- the other half of
/// `build_arp_request`'s round trip. Every other ARP opcode (requests,
/// including ones this same responder might itself need to keep
/// answering while a resolution is in flight) returns `None`, so a
/// caller can safely try this first and fall back to `handle_frame` for
/// anything it doesn't recognize.
pub fn parse_arp_reply(buf: &[u8], len: usize, target_ip: [u8; 4]) -> Option<[u8; 6]> {
    const ARP: usize = ETH_HEADER_LEN;
    if len < ARP + ARP_LEN {
        return None;
    }
    if buf[12..14] != ETHERTYPE_ARP {
        return None;
    }
    if buf[ARP..ARP + 2] != ARP_HTYPE_ETHERNET || buf[ARP + 2..ARP + 4] != ARP_PTYPE_IPV4 {
        return None;
    }
    if buf[ARP + 6..ARP + 8] != ARP_OP_REPLY {
        return None;
    }
    if buf[ARP + 14..ARP + 18] != target_ip {
        return None; // not a reply about the peer we're resolving
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&buf[ARP + 8..ARP + 14]);
    Some(mac)
}
