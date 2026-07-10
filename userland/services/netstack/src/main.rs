//! A minimal ARP + IPv4 + ICMP responder, plus a minimal TCP client:
//! claims a static IP address, answers ARP/ICMP requests for it (the
//! first networking checkpoint's own scope, unchanged), and now also
//! opens outbound TCP connections on behalf of an external client task
//! over `netstack`'s own new `"tcp"` protocol (`libpcern`'s
//! `TCP_OP_*`/`tcp_open`/`tcp_write`/`tcp_read`/`tcp_close`) -- a fixed
//! advertised window, no congestion control, no retransmission timers
//! (this kernel exposes no clock to userland; see `MAX_ATTEMPTS`'s own
//! doc comment for the substitute). Packet-level parsing and frame
//! construction live in `proto.rs` (ARP/ICMP) and `tcp.rs` (TCP
//! segments); this file is the IPC glue and the TCP connection state
//! machine, which -- unlike Checkpoint X's pure responder -- needs
//! enough of its own logic (advancing a connection frame by frame,
//! deferring replies to whichever client operation is currently
//! outstanding) that it doesn't cleanly separate from the glue itself.
//!
//! ## Why this doesn't just keep blocking in `nic_recv`
//!
//! Checkpoint X's loop was `loop { let len = nic_recv(...); ...  }` --
//! one blocking round trip to `net_rtl8139` per iteration, forever.
//! That can't survive this checkpoint unchanged: this task must *also*
//! stay responsive to an external TCP client's requests, which arrive
//! independently of network traffic, and there's no way to block on
//! "whichever of two endpoints has something first" (this kernel's IPC
//! has no `select`). The tempting fix -- keep one `NIC_OP_RECV`
//! permanently outstanding, and dispatch whatever `recv` returns by
//! sender -- is *not* safe here: `net_rtl8139` only has one reply-to
//! slot, and delivering that deferred reply is itself a blocking `send`
//! on its end. If this task ever tries to send `net_rtl8139` anything
//! else (an ARP request, a TCP segment) while that `NIC_OP_RECV` is
//! still outstanding, and the timing lands wrong, both tasks end up
//! blocked in `send`, each waiting for the other's `recv` -- a genuine
//! deadlock, not a hypothetical one.
//!
//! The fix: never leave anything outstanding with `net_rtl8139` at all.
//! `NIC_OP_TRY_RECV` (see its own doc comment) replies immediately
//! either way, so polling it is always a short, bounded round trip.
//! This task's own inbox is polled the same way via `SYS_TRY_RECV`
//! (`libpcern::try_recv`) for the external client's requests. The main
//! loop below just alternates between the two, non-blockingly, yielding
//! once per round when neither has anything -- the one place in this
//! project where a service busy-polls instead of blocking when idle;
//! see `kernel/src/ipc.rs`'s `try_recv` for why that trade was made.

#![no_std]
#![no_main]

mod proto;
mod tcp;

use core::panic::PanicInfo;
use libpcern::RecvResult;

/// CSlot 1 is the name service (auto-granted); this is this task's own
/// inbox -- shared, by design, across three roles: the one-shot
/// name-lookup reply at startup, `net_rtl8139`'s replies, and an
/// external TCP client's requests. Sharing an inbox across roles is
/// normally the exact hazard CLAUDE.md's own "one inbox is not
/// automatically safe for two roles" section warns about -- what makes
/// it safe here specifically is that *every* message received on it is
/// dispatched by checking the kernel-attested `sender` first, never by
/// assuming "the next message is whatever I asked for" the way the
/// cap_test postmortem's bug did. See this module's own top doc comment
/// for why a second, dedicated endpoint wouldn't actually avoid needing
/// that same discipline anyway (this kernel has no way to block on two
/// endpoints jointly, so multiplexing has to happen on one inbox no
/// matter how many endpoints exist).
const MY_INBOX: u32 = 2;

const NIC_BUF_VIRT: u32 = 0x00B0_0000;
/// Where an external TCP client's shared data buffer gets mapped, once
/// it transfers a `MemoryGrant` via `TCP_OP_SET_BUFFER` -- distinct from
/// `NIC_BUF_VIRT` since both are mapped simultaneously in this task's
/// own address space (the raw frame buffer net_rtl8139 fills, and the
/// client-facing buffer this task fills/reads in turn).
const CLIENT_BUF_VIRT: u32 = 0x00C0_0000;

/// This host's own address -- see the identical constant's doc comment
/// from Checkpoint X for why it's hardcoded.
const STATIC_IP: [u8; 4] = [10, 0, 2, 15];

/// `net_rtl8139`'s task id is deterministic across every boot
/// configuration this service runs in -- production and every
/// standalone test harness alike spawn it 6th, exactly so its own
/// `assert_eq!(nic_id, 6, ...)` holds (nameservice's ALLOWLIST hardcodes
/// "net" to that id) -- see `kernel/src/main.rs`'s `spawn_net_rtl8139`.
/// This is the *other* half of the "safe to share one inbox" argument
/// above: telling `net_rtl8139`'s replies apart from a client's requests
/// is exactly a comparison against this constant.
const NIC_TASK_ID: u32 = 6;

/// Fixed for the lifetime of every connection, matching this checkpoint's
/// "fixed window, no congestion control" scope: this client sends and
/// advertises exactly this cap, never ramping it up or down based on
/// traffic, and (unlike a real TCP stack) never even reads the peer's
/// own advertised window -- it just never offers the peer more than
/// this many bytes in one `TCP_OP_SEND`, and expects a well-behaved
/// peer's own receive buffer to comfortably exceed it.
const WINDOW: u16 = 2048;
/// The largest payload one `TCP_OP_SEND` can actually push in a single
/// segment -- bounded by what fits in one Ethernet frame after this
/// client's own fixed-size headers (`tcp::build_segment` never sends
/// options), which is *not* the same bound as `WINDOW`: `WINDOW` also
/// serves as the advertised receive window (how much the peer may send
/// before waiting for more), and that side is fine well past one frame
/// since inbound data accumulates across many frames into
/// `TCP_MAX_TRANSFER`. Outbound data doesn't get that -- one
/// `TCP_OP_SEND` becomes exactly one frame -- so the send path needs its
/// own, smaller cap alongside `WINDOW`.
const MAX_SEGMENT_PAYLOAD: usize =
    libpcern::NIC_MAX_FRAME - proto::ETH_HEADER_LEN - proto::IPV4_HEADER_LEN - tcp::HEADER_LEN;
/// Also fixed, not randomized: this client only ever performs active
/// opens (no listen state to protect), so there's nothing here a
/// sequence-prediction attack would gain over guessing zero -- a
/// constant just keeps every trace reproducible.
const LOCAL_PORT: u16 = 51820;
const INITIAL_SEQ: u32 = 0x0001_0000;
/// Every blocking wait in this state machine (ARP resolution, the
/// handshake, waiting for data, the close handshake) is bounded by a
/// count of *outer-loop iterations*, not wall-clock time -- this kernel
/// exposes no timer/clock syscall to userland, so this is the
/// substitute, the same "bounded, not indefinite" idiom
/// `rtl8139::send`'s `MAX_HARDWARE_POLLS` already uses for an analogous
/// problem. Deliberately generous, since one iteration is often a
/// single non-blocking poll, not a real wait.
const MAX_ATTEMPTS: u32 = 2_000_000;
/// Cap on the idle-poll backoff in the main loop below: each fully idle
/// round (neither `net_rtl8139` nor the client had anything) doubles how
/// many times this task calls `yield_now()` before polling again, up to
/// this many -- otherwise this task would re-issue two syscalls (one to
/// `net_rtl8139`, one to its own inbox) on every single scheduler slot
/// forever even with no connection and no client, the one real cost of
/// this being the one service in this project that busy-polls instead of
/// blocking when idle (see this module's own top doc comment). Bounded,
/// not indefinite, the same idiom `MAX_ATTEMPTS` already uses; resets to
/// 1 the instant either poll finds anything, so a real event is never
/// delayed by more than one stale backoff step.
const MAX_IDLE_YIELDS: u32 = 16;

#[derive(Clone, Copy, PartialEq)]
enum ConnState {
    ArpPending,
    SynSent,
    Established,
    /// Peer's FIN already answered with our own FIN -- waiting for its
    /// final ACK (a passive close, initiated by the peer).
    LastAck,
    /// This client's own `TCP_OP_CLOSE` sent a FIN -- waiting for the
    /// peer's ACK and/or its own FIN (an active close).
    FinWait1,
    /// This client's FIN was ACKed -- waiting for the peer's own FIN.
    FinWait2,
}

struct Connection {
    peer_ip: [u8; 4],
    peer_port: u16,
    peer_mac: [u8; 6],
    state: ConnState,
    snd_nxt: u32,
    rcv_nxt: u32,
    attempts_left: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum PendingOp {
    None,
    Connect,
    Recv,
    Close,
}

/// What handling one frame did for the active connection, if any --
/// `advance_connection`'s return value.
enum Outcome {
    /// Not addressed to (or not relevant to) the active connection --
    /// caller should fall back to the standing ARP/ICMP responder.
    NotMine,
    /// Consumed by the connection; nothing further to do this frame.
    Handled,
    /// Consumed by the connection, and the connection is now finished
    /// (reset, or a close handshake completed) -- caller should drop it.
    Done,
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let nic_slot = match libpcern::lookup_name_retry(b"net", MY_INBOX, 1000) {
        Some(s) => s,
        None => libpcern::exit(1),
    };

    let nic_grant = libpcern::mem_alloc(NIC_BUF_VIRT);
    if nic_grant == 0 {
        libpcern::exit(1);
    }
    libpcern::nic_connect(nic_slot, nic_grant, MY_INBOX);
    let my_mac = libpcern::nic_get_mac(nic_slot, MY_INBOX);

    // Only succeeds where this task's id matches nameservice's ALLOWLIST
    // entry for "tcp" (task id 7 -- production numbering, and every
    // harness that exercises this protocol matches it); a harmless no-op
    // anywhere else (e.g. arp_icmp_test's harness, where this task's id
    // doesn't match and nothing there ever looks "tcp" up anyway).
    libpcern::register_name(b"tcp", MY_INBOX);

    let mut stash = Stash::new();
    let mut client_buf_mapped = false;
    let mut client_reply: u32 = 0;
    let mut conn: Option<Connection> = None;
    let mut pending_op = PendingOp::None;
    let mut buffered_len: usize = 0;
    let mut idle_yields: u32 = 1;

    loop {
        if pending_op != PendingOp::None {
            if let Some(c) = conn.as_mut() {
                if c.attempts_left == 0 {
                    fail_pending_op(&mut conn, &mut pending_op, &mut buffered_len, client_reply);
                    continue;
                }
                c.attempts_left -= 1;
            }
        }

        libpcern::send(nic_slot, libpcern::NIC_OP_TRY_RECV, 0, 0, 0);
        let nic_reply = recv_from_nic(&mut stash);
        let len = nic_reply.w0 as usize;
        if len > 0 {
            idle_yields = 1;
            let nic_buf = unsafe { core::slice::from_raw_parts_mut(NIC_BUF_VIRT as *mut u8, libpcern::NIC_MAX_FRAME) };
            handle_nic_frame(nic_buf, len, my_mac, nic_slot, &mut stash, &mut conn, &mut pending_op, &mut buffered_len, client_reply);
            continue;
        }

        if let Some(r) = next_client_message(&mut stash) {
            idle_yields = 1;
            handle_client_message(
                r,
                nic_slot,
                my_mac,
                &mut stash,
                &mut client_buf_mapped,
                &mut client_reply,
                &mut conn,
                &mut pending_op,
                &mut buffered_len,
            );
            continue;
        }

        for _ in 0..idle_yields {
            libpcern::yield_now();
        }
        idle_yields = (idle_yields * 2).min(MAX_IDLE_YIELDS);
    }
}

/// Bounded FIFO for client messages that arrive on this inbox while
/// `recv_from_nic`/`send_to_nic` are specifically waiting for
/// `net_rtl8139`'s reply. *Not* a single slot:
/// `TCP_OP_SET_BUFFER`/`SET_REPLY` are sent back-to-back with no reply
/// in between (see `libpcern::tcp_connect_setup`), so a client can
/// legitimately have both already queued here before `net_rtl8139` gets
/// around to replying -- a single `Option` overwrites (silently drops)
/// an earlier one the moment a second arrives, which is exactly the bug
/// this queue replaces. Sized at exactly 2, the true maximum: every
/// other client op in this protocol is strictly request-then-reply (the
/// caller waits for one reply before sending its next request -- see
/// `libpcern`'s own doc comment on the TCP protocol), so `SET_BUFFER` +
/// `SET_REPLY` is the only pair of sends this protocol ever produces
/// without a reply in between.
struct Stash {
    items: [Option<RecvResult>; 2],
    len: usize,
}

impl Stash {
    const fn new() -> Self {
        Stash { items: [None; 2], len: 0 }
    }

    /// Drops the message if the queue is somehow already full. Given the
    /// bound above, this can only happen if a caller violates the
    /// documented one-outstanding-request protocol -- still checked
    /// (never overflow the array), but not a case that needs recovering
    /// from gracefully: an external client that doesn't follow the
    /// protocol has no ordering guarantees to lose in the first place.
    fn push(&mut self, r: RecvResult) {
        if self.len < self.items.len() {
            self.items[self.len] = Some(r);
            self.len += 1;
        }
    }

    fn pop_front(&mut self) -> Option<RecvResult> {
        let r = self.items[0].take()?;
        for i in 1..self.len {
            self.items[i - 1] = self.items[i].take();
        }
        self.len -= 1;
        Some(r)
    }
}

/// After sending a request specifically to `net_rtl8139`, retrieves its
/// reply. `net_rtl8139` only ever replies to what this task most
/// recently asked it, and (via `NIC_OP_TRY_RECV`) always replies
/// immediately -- so if a client message arrives on this inbox first
/// (interleaved by the scheduler), it's queued in `stash` for
/// `next_client_message` to find, and this keeps waiting for
/// `net_rtl8139`'s own reply, which is always imminent.
fn recv_from_nic(stash: &mut Stash) -> RecvResult {
    loop {
        let r = libpcern::recv(MY_INBOX);
        if r.sender == NIC_TASK_ID {
            return r;
        }
        stash.push(r);
    }
}

/// Sends one frame to `net_rtl8139` and waits for its reply, same
/// contract as `libpcern::nic_send` -- but through `recv_from_nic`
/// instead of `libpcern::nic_send`'s own internal, unchecked `recv`.
/// `MY_INBOX` is genuinely shared with the external client role (see
/// this module's own top doc comment), so any blocking `recv` on it
/// that isn't sender-filtered can have an external client's message
/// delivered into it instead of `net_rtl8139`'s actual reply --
/// `libpcern::nic_send` does exactly that unfiltered `recv`, which is
/// safe for a task with no other role on its inbox but not safe here.
/// Every one of this task's own sends to `net_rtl8139` must go through
/// this function instead, so a client message that arrives in the
/// interim lands in `stash` like every other client message does.
fn send_to_nic(nic_slot: u32, stash: &mut Stash, len: u32) -> bool {
    libpcern::send(nic_slot, libpcern::NIC_OP_SEND, len, 0, 0);
    recv_from_nic(stash).w0 == 1
}

/// Returns the next external client message, if any, without blocking --
/// the oldest queued one from `recv_from_nic`, if there is one, else a
/// fresh `try_recv`.
fn next_client_message(stash: &mut Stash) -> Option<RecvResult> {
    if let Some(r) = stash.pop_front() {
        return Some(r);
    }
    let r = libpcern::try_recv(MY_INBOX)?;
    if r.sender == NIC_TASK_ID {
        // Shouldn't happen -- every net_rtl8139 request this task makes
        // is fully drained by recv_from_nic before this is ever called --
        // but never misinterpret a stray reply as a client request.
        return None;
    }
    Some(r)
}

fn fail_pending_op(conn: &mut Option<Connection>, pending_op: &mut PendingOp, buffered_len: &mut usize, client_reply: u32) {
    let op = *pending_op;
    *pending_op = PendingOp::None;
    let buffered = *buffered_len as u32;
    *buffered_len = 0;
    *conn = None;
    match op {
        PendingOp::Connect => {
            libpcern::send(client_reply, 0, 0, 0, 0);
        }
        PendingOp::Recv => {
            libpcern::send(client_reply, buffered, 0, 0, 0);
        }
        PendingOp::Close => {
            libpcern::send(client_reply, 1, 0, 0, 0);
        }
        PendingOp::None => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_client_message(
    r: RecvResult,
    nic_slot: u32,
    my_mac: [u8; 6],
    stash: &mut Stash,
    client_buf_mapped: &mut bool,
    client_reply: &mut u32,
    conn: &mut Option<Connection>,
    pending_op: &mut PendingOp,
    buffered_len: &mut usize,
) {
    match r.w0 {
        libpcern::TCP_OP_SET_BUFFER => {
            if r.transferred_slot != 0 && libpcern::map_memory(r.transferred_slot, CLIENT_BUF_VIRT) == 0 {
                *client_buf_mapped = true;
            }
        }
        libpcern::TCP_OP_SET_REPLY => {
            if r.transferred_slot != 0 {
                *client_reply = r.transferred_slot;
            }
        }
        libpcern::TCP_OP_CONNECT => {
            if *client_reply == 0 {
                return;
            }
            if conn.is_some() || !*client_buf_mapped {
                libpcern::send(*client_reply, 0, 0, 0, 0);
                return;
            }
            let peer_ip = r.w1.to_le_bytes();
            let peer_port = r.w2 as u16;
            let nic_buf = unsafe { core::slice::from_raw_parts_mut(NIC_BUF_VIRT as *mut u8, libpcern::NIC_MAX_FRAME) };
            let req_len = proto::build_arp_request(nic_buf, my_mac, STATIC_IP, peer_ip);
            send_to_nic(nic_slot, stash, req_len as u32);
            *conn = Some(Connection {
                peer_ip,
                peer_port,
                peer_mac: [0; 6],
                state: ConnState::ArpPending,
                snd_nxt: INITIAL_SEQ,
                rcv_nxt: 0,
                attempts_left: MAX_ATTEMPTS,
            });
            *pending_op = PendingOp::Connect;
        }
        libpcern::TCP_OP_SEND => {
            if *client_reply == 0 {
                return;
            }
            match conn.as_mut() {
                Some(c) if c.state == ConnState::Established && *client_buf_mapped => {
                    let want = (r.w1 as usize)
                        .min(WINDOW as usize)
                        .min(libpcern::TCP_MAX_TRANSFER)
                        .min(MAX_SEGMENT_PAYLOAD);
                    let client_buf = unsafe { core::slice::from_raw_parts(CLIENT_BUF_VIRT as *const u8, libpcern::TCP_MAX_TRANSFER) };
                    let nic_buf = unsafe { core::slice::from_raw_parts_mut(NIC_BUF_VIRT as *mut u8, libpcern::NIC_MAX_FRAME) };
                    let seg_len = tcp::build_segment(
                        nic_buf,
                        my_mac,
                        STATIC_IP,
                        c.peer_mac,
                        c.peer_ip,
                        LOCAL_PORT,
                        c.peer_port,
                        c.snd_nxt,
                        c.rcv_nxt,
                        tcp::FLAG_PSH | tcp::FLAG_ACK,
                        WINDOW,
                        &client_buf[..want],
                    );
                    send_to_nic(nic_slot, stash, seg_len as u32);
                    c.snd_nxt = c.snd_nxt.wrapping_add(want as u32);
                    libpcern::send(*client_reply, want as u32, 0, 0, 0);
                }
                _ => {
                    libpcern::send(*client_reply, 0, 0, 0, 0);
                }
            }
        }
        libpcern::TCP_OP_RECV => {
            if *client_reply == 0 {
                return;
            }
            let already_done = conn.as_ref().is_none_or(|c| matches!(c.state, ConnState::LastAck | ConnState::FinWait2));
            if *buffered_len > 0 || already_done {
                libpcern::send(*client_reply, *buffered_len as u32, 0, 0, 0);
                *buffered_len = 0;
            } else {
                *pending_op = PendingOp::Recv;
                if let Some(c) = conn.as_mut() {
                    c.attempts_left = MAX_ATTEMPTS;
                }
            }
        }
        libpcern::TCP_OP_CLOSE => {
            if *client_reply == 0 {
                return;
            }
            match conn.as_mut() {
                Some(c) if c.state == ConnState::Established => {
                    let nic_buf = unsafe { core::slice::from_raw_parts_mut(NIC_BUF_VIRT as *mut u8, libpcern::NIC_MAX_FRAME) };
                    let seg_len = tcp::build_segment(
                        nic_buf,
                        my_mac,
                        STATIC_IP,
                        c.peer_mac,
                        c.peer_ip,
                        LOCAL_PORT,
                        c.peer_port,
                        c.snd_nxt,
                        c.rcv_nxt,
                        tcp::FLAG_FIN | tcp::FLAG_ACK,
                        WINDOW,
                        &[],
                    );
                    send_to_nic(nic_slot, stash, seg_len as u32);
                    c.snd_nxt = c.snd_nxt.wrapping_add(1);
                    c.state = ConnState::FinWait1;
                    c.attempts_left = MAX_ATTEMPTS;
                    *pending_op = PendingOp::Close;
                }
                _ => {
                    *conn = None;
                    libpcern::send(*client_reply, 1, 0, 0, 0);
                }
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_nic_frame(
    buf: &mut [u8],
    len: usize,
    my_mac: [u8; 6],
    nic_slot: u32,
    stash: &mut Stash,
    conn: &mut Option<Connection>,
    pending_op: &mut PendingOp,
    buffered_len: &mut usize,
    client_reply: u32,
) {
    let outcome = match conn.as_mut() {
        Some(c) => advance_connection(buf, len, my_mac, nic_slot, stash, c, pending_op, buffered_len, client_reply),
        None => Outcome::NotMine,
    };

    match outcome {
        Outcome::Done => *conn = None,
        Outcome::Handled => {}
        Outcome::NotMine => {
            if let Some(reply_len) = proto::handle_frame(buf, len, my_mac, STATIC_IP) {
                send_to_nic(nic_slot, stash, reply_len as u32);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn advance_connection(
    buf: &mut [u8],
    len: usize,
    my_mac: [u8; 6],
    nic_slot: u32,
    stash: &mut Stash,
    c: &mut Connection,
    pending_op: &mut PendingOp,
    buffered_len: &mut usize,
    client_reply: u32,
) -> Outcome {
    if c.state == ConnState::ArpPending {
        let Some(mac) = proto::parse_arp_reply(buf, len, c.peer_ip) else {
            return Outcome::NotMine;
        };
        c.peer_mac = mac;
        c.state = ConnState::SynSent;
        c.attempts_left = MAX_ATTEMPTS;
        let seg_len = tcp::build_segment(
            buf, my_mac, STATIC_IP, c.peer_mac, c.peer_ip, LOCAL_PORT, c.peer_port, c.snd_nxt, 0, tcp::FLAG_SYN, WINDOW, &[],
        );
        send_to_nic(nic_slot, stash, seg_len as u32);
        return Outcome::Handled;
    }

    // Every other state is driven by an incoming TCP segment addressed
    // to this connection -- parse and validate it once here instead of
    // once per state below.
    let Some(seg) = tcp::parse_segment(buf, len, c.peer_ip) else {
        return Outcome::NotMine;
    };
    if seg.dst_port != LOCAL_PORT || seg.src_port != c.peer_port {
        return Outcome::NotMine;
    }

    match c.state {
        ConnState::ArpPending => unreachable!("handled above"),
        ConnState::SynSent => {
            if seg.flags & tcp::FLAG_RST != 0 {
                if *pending_op == PendingOp::Connect {
                    libpcern::send(client_reply, 0, 0, 0, 0);
                    *pending_op = PendingOp::None;
                }
                return Outcome::Done;
            }
            if seg.flags & tcp::FLAG_SYN != 0 && seg.flags & tcp::FLAG_ACK != 0 && seg.ack == c.snd_nxt.wrapping_add(1) {
                c.snd_nxt = c.snd_nxt.wrapping_add(1);
                c.rcv_nxt = seg.seq.wrapping_add(1);
                c.state = ConnState::Established;
                let ack_len = tcp::build_segment(
                    buf,
                    my_mac,
                    STATIC_IP,
                    c.peer_mac,
                    c.peer_ip,
                    LOCAL_PORT,
                    c.peer_port,
                    c.snd_nxt,
                    c.rcv_nxt,
                    tcp::FLAG_ACK,
                    WINDOW,
                    &[],
                );
                send_to_nic(nic_slot, stash, ack_len as u32);
                if *pending_op == PendingOp::Connect {
                    libpcern::send(client_reply, 1, 0, 0, 0);
                    *pending_op = PendingOp::None;
                }
                return Outcome::Handled;
            }
            Outcome::NotMine
        }
        ConnState::Established => {
            if seg.flags & tcp::FLAG_RST != 0 {
                match *pending_op {
                    PendingOp::Recv => {
                        libpcern::send(client_reply, 0, 0, 0, 0);
                    }
                    PendingOp::Close => {
                        libpcern::send(client_reply, 1, 0, 0, 0);
                    }
                    _ => {}
                }
                *pending_op = PendingOp::None;
                return Outcome::Done;
            }

            let mut advanced = false;
            if !seg.payload.is_empty() && seg.seq == c.rcv_nxt {
                let client_buf = unsafe { core::slice::from_raw_parts_mut(CLIENT_BUF_VIRT as *mut u8, libpcern::TCP_MAX_TRANSFER) };
                let room = libpcern::TCP_MAX_TRANSFER - *buffered_len;
                let n = seg.payload.len().min(room);
                client_buf[*buffered_len..*buffered_len + n].copy_from_slice(&seg.payload[..n]);
                *buffered_len += n;
                c.rcv_nxt = c.rcv_nxt.wrapping_add(n as u32);
                advanced = true;
            }
            let fin = seg.flags & tcp::FLAG_FIN != 0 && seg.seq.wrapping_add(seg.payload.len() as u32) == c.rcv_nxt;
            if !advanced && !fin {
                return Outcome::NotMine; // e.g. a bare ACK of our own earlier data -- nothing more to do
            }
            if fin {
                c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
            }
            let flags = if fin { tcp::FLAG_FIN | tcp::FLAG_ACK } else { tcp::FLAG_ACK };
            let seq = c.snd_nxt;
            if fin {
                c.snd_nxt = c.snd_nxt.wrapping_add(1);
            }
            let ack_len = tcp::build_segment(
                buf, my_mac, STATIC_IP, c.peer_mac, c.peer_ip, LOCAL_PORT, c.peer_port, seq, c.rcv_nxt, flags, WINDOW, &[],
            );
            send_to_nic(nic_slot, stash, ack_len as u32);
            if fin {
                c.state = ConnState::LastAck;
                c.attempts_left = MAX_ATTEMPTS;
            }
            if *pending_op == PendingOp::Recv {
                libpcern::send(client_reply, *buffered_len as u32, 0, 0, 0);
                *buffered_len = 0;
                *pending_op = PendingOp::None;
            }
            Outcome::Handled
        }
        ConnState::LastAck => {
            if seg.flags & tcp::FLAG_ACK != 0 && seg.ack == c.snd_nxt {
                if *pending_op == PendingOp::Close {
                    libpcern::send(client_reply, 1, 0, 0, 0);
                    *pending_op = PendingOp::None;
                }
                return Outcome::Done;
            }
            Outcome::NotMine
        }
        ConnState::FinWait1 => {
            if seg.flags & tcp::FLAG_FIN != 0 {
                c.rcv_nxt = seg.seq.wrapping_add(1);
                let ack_len = tcp::build_segment(
                    buf,
                    my_mac,
                    STATIC_IP,
                    c.peer_mac,
                    c.peer_ip,
                    LOCAL_PORT,
                    c.peer_port,
                    c.snd_nxt,
                    c.rcv_nxt,
                    tcp::FLAG_ACK,
                    WINDOW,
                    &[],
                );
                send_to_nic(nic_slot, stash, ack_len as u32);
                if *pending_op == PendingOp::Close {
                    libpcern::send(client_reply, 1, 0, 0, 0);
                    *pending_op = PendingOp::None;
                }
                return Outcome::Done;
            }
            if seg.flags & tcp::FLAG_ACK != 0 && seg.ack == c.snd_nxt {
                c.state = ConnState::FinWait2;
                c.attempts_left = MAX_ATTEMPTS;
                return Outcome::Handled;
            }
            Outcome::NotMine
        }
        ConnState::FinWait2 => {
            if seg.flags & tcp::FLAG_FIN == 0 {
                return Outcome::NotMine;
            }
            c.rcv_nxt = seg.seq.wrapping_add(1);
            let ack_len = tcp::build_segment(
                buf, my_mac, STATIC_IP, c.peer_mac, c.peer_ip, LOCAL_PORT, c.peer_port, c.snd_nxt, c.rcv_nxt, tcp::FLAG_ACK, WINDOW, &[],
            );
            send_to_nic(nic_slot, stash, ack_len as u32);
            if *pending_op == PendingOp::Close {
                libpcern::send(client_reply, 1, 0, 0, 0);
                *pending_op = PendingOp::None;
            }
            Outcome::Done
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
