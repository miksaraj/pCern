//! TCP segment-level logic: header packing/parsing and the pseudo-header
//! checksum, as plain functions over byte slices -- no IPC, no
//! connection state, mirroring `proto.rs`'s own "packet-level logic
//! lives here, the state machine and IPC glue live in main.rs" split.
//! Deliberately narrow, matching this project's usual scope: a segment
//! this client builds is always a plain 20-byte TCP header (no options)
//! carried in a plain 20-byte IPv4 header (no options) -- but a segment
//! this client *parses* still respects the peer's own data-offset field
//! rather than assuming 20 bytes, since a real TCP stack's SYN-ACK
//! commonly carries options (MSS, SACK-permitted, timestamps) even
//! though this client never sends any itself; hardcoding the header
//! length on the read side would silently misparse the very first real
//! peer this client ever talks to.

use crate::proto::{
    checksum16_accumulate, checksum16_finish, ETH_HEADER_LEN, ETHERTYPE_IPV4, IPV4_HEADER_LEN,
    IPV4_VER_IHL_NO_OPTIONS,
};

pub const HEADER_LEN: usize = 20;
pub const FLAG_FIN: u8 = 0x01;
pub const FLAG_SYN: u8 = 0x02;
pub const FLAG_RST: u8 = 0x04;
pub const FLAG_PSH: u8 = 0x08;
pub const FLAG_ACK: u8 = 0x10;

const IP_PROTO_TCP: u8 = 6;

/// A parsed segment's fields, borrowing its payload directly out of the
/// frame buffer it was parsed from -- no copy.
pub struct Segment<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    /// Parsed for completeness, but never read anywhere in this crate --
    /// this client's fixed-window scope means it never adjusts what it
    /// sends based on the peer's own advertised window (see main.rs's
    /// `WINDOW` doc comment).
    #[allow(dead_code)]
    pub window: u16,
    pub payload: &'a [u8],
}

/// Builds one Ethernet+IPv4+TCP frame into `buf` (zero-padded to the
/// Ethernet minimum) and returns its length. `payload` is copied
/// immediately after the 20-byte header; empty for a pure control
/// segment (SYN, a bare ACK, FIN).
pub fn build_segment(
    buf: &mut [u8],
    my_mac: [u8; 6],
    my_ip: [u8; 4],
    peer_mac: [u8; 6],
    peer_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    payload: &[u8],
) -> usize {
    const IP: usize = ETH_HEADER_LEN;
    const TCP: usize = IP + IPV4_HEADER_LEN;
    let total_len = IPV4_HEADER_LEN + HEADER_LEN + payload.len();

    buf[0..6].copy_from_slice(&peer_mac);
    buf[6..12].copy_from_slice(&my_mac);
    buf[12..14].copy_from_slice(&ETHERTYPE_IPV4);

    buf[IP] = IPV4_VER_IHL_NO_OPTIONS;
    buf[IP + 1] = 0; // DSCP/ECN: unused
    buf[IP + 2..IP + 4].copy_from_slice(&(total_len as u16).to_be_bytes());
    buf[IP + 4..IP + 8].fill(0); // identification + flags/fragment offset: this client never fragments
    buf[IP + 8] = 64; // TTL: see proto.rs's DEFAULT_TTL doc comment -- same reasoning, this host doesn't route either
    buf[IP + 9] = IP_PROTO_TCP;
    buf[IP + 10..IP + 12].fill(0); // checksum, filled in below once the rest of the header is final
    buf[IP + 12..IP + 16].copy_from_slice(&my_ip);
    buf[IP + 16..IP + 20].copy_from_slice(&peer_ip);

    buf[TCP..TCP + 2].copy_from_slice(&src_port.to_be_bytes());
    buf[TCP + 2..TCP + 4].copy_from_slice(&dst_port.to_be_bytes());
    buf[TCP + 4..TCP + 8].copy_from_slice(&seq.to_be_bytes());
    buf[TCP + 8..TCP + 12].copy_from_slice(&ack.to_be_bytes());
    buf[TCP + 12] = 0x50; // data offset 5 (x4 bytes = 20), reserved bits 0 -- no options, ever, on the send side
    buf[TCP + 13] = flags;
    buf[TCP + 14..TCP + 16].copy_from_slice(&window.to_be_bytes());
    buf[TCP + 16..TCP + 18].fill(0); // checksum, filled in below
    buf[TCP + 18..TCP + 20].fill(0); // urgent pointer: unused, URG never set
    buf[TCP + HEADER_LEN..TCP + HEADER_LEN + payload.len()].copy_from_slice(payload);

    // TCP checksum covers a pseudo-header (RFC 793 3.1) the wire format
    // itself never carries, so it's summed separately rather than as
    // part of one contiguous slice -- see checksum16_accumulate's own
    // doc comment for why this split exists.
    let mut pseudo = [0u8; 12];
    pseudo[0..4].copy_from_slice(&my_ip);
    pseudo[4..8].copy_from_slice(&peer_ip);
    pseudo[8] = 0;
    pseudo[9] = IP_PROTO_TCP;
    pseudo[10..12].copy_from_slice(&((HEADER_LEN + payload.len()) as u16).to_be_bytes());
    let sum = checksum16_accumulate(0, &pseudo);
    let sum = checksum16_accumulate(sum, &buf[TCP..TCP + HEADER_LEN + payload.len()]);
    let tcp_sum = checksum16_finish(sum);
    buf[TCP + 16..TCP + 18].copy_from_slice(&tcp_sum.to_be_bytes());

    let ip_sum = checksum16_finish(checksum16_accumulate(0, &buf[IP..IP + IPV4_HEADER_LEN]));
    buf[IP + 10..IP + 12].copy_from_slice(&ip_sum.to_be_bytes());

    crate::proto::pad_to_min_frame(buf, IP + total_len)
}

/// Parses `buf[..len]` as a TCP segment from `expect_peer_ip`, or `None`
/// if it isn't one (wrong ethertype, wrong protocol, wrong source
/// address, or too short to hold what its own length/offset fields
/// claim). Like `proto.rs`'s ICMP path, only a plain 20-byte IPv4 header
/// is understood -- but see this module's own doc comment for why the
/// *TCP* header's length is still read from the wire, not assumed.
pub fn parse_segment(buf: &[u8], len: usize, expect_peer_ip: [u8; 4]) -> Option<Segment<'_>> {
    const IP: usize = ETH_HEADER_LEN;
    if len < IP + IPV4_HEADER_LEN {
        return None;
    }
    if buf[12..14] != ETHERTYPE_IPV4 {
        return None;
    }
    if buf[IP] != IPV4_VER_IHL_NO_OPTIONS {
        return None;
    }
    if buf[IP + 9] != IP_PROTO_TCP {
        return None;
    }
    if buf[IP + 12..IP + 16] != expect_peer_ip {
        return None;
    }

    let total_len = u16::from_be_bytes([buf[IP + 2], buf[IP + 3]]) as usize;
    if total_len < IPV4_HEADER_LEN || IP + total_len > len {
        return None;
    }

    let tcp = IP + IPV4_HEADER_LEN;
    let tcp_len = total_len - IPV4_HEADER_LEN;
    if tcp_len < HEADER_LEN {
        return None;
    }
    let data_offset = ((buf[tcp + 12] >> 4) as usize) * 4;
    if data_offset < HEADER_LEN || data_offset > tcp_len {
        return None; // claims a header shorter than the minimum, or longer than the segment actually is
    }

    Some(Segment {
        src_port: u16::from_be_bytes([buf[tcp], buf[tcp + 1]]),
        dst_port: u16::from_be_bytes([buf[tcp + 2], buf[tcp + 3]]),
        seq: u32::from_be_bytes([buf[tcp + 4], buf[tcp + 5], buf[tcp + 6], buf[tcp + 7]]),
        ack: u32::from_be_bytes([buf[tcp + 8], buf[tcp + 9], buf[tcp + 10], buf[tcp + 11]]),
        flags: buf[tcp + 13],
        window: u16::from_be_bytes([buf[tcp + 14], buf[tcp + 15]]),
        payload: &buf[tcp + data_offset..IP + total_len],
    })
}
