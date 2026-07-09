#!/usr/bin/env bash
# Boots the TCP-client-test ISO (see `make test-tcp` / `make iso-tcptest`)
# headlessly in QEMU and verifies netstack's minimal TCP client
# (connect/send/recv/close) against a genuine external peer -- not a
# simulation: this script itself acts as a second host on the wire,
# hand-building and parsing real Ethernet+IPv4+TCP frames (ARP reply,
# SYN-ACK, data, FIN-ACK) the same way run_arp_icmp_test.sh already does
# for ARP/ICMP, just now playing TCP's *server* role opposite netstack's
# client. in-guest, http_client_test (see userland/cap_test) drives
# netstack's new "tcp" protocol to fetch a tiny fixed HTTP-shaped
# response and checks the exact bytes came back -- this script's own
# job is confirming that exchange actually crossed the wire as real
# frames, both by watching it happen live and by re-parsing an
# independent packet capture afterward, the same "don't just trust the
# in-guest report" pattern every prior driver/service test here uses.
#
# Same `-netdev socket` raw-Ethernet-over-TCP pipe as run_arp_icmp_test.sh
# (see its own doc comment for why, and for the 4-byte length-prefix
# framing) -- this test needs the same "genuinely a second host" shape,
# just with the roles of who calls out to whom reversed: here it's
# *netstack* that ARPs for this peer's address and opens the connection,
# not the other way around.
#
# Task ids in this build: nameservice=1, console_server=2, storage_ata=3,
# fs_fat32=4, http_client_test=5, net_rtl8139=6, netstack=7 -- the same
# relative order production boot uses (net_rtl8139 then netstack), so
# netstack's "tcp" name registration lands at the id nameservice's
# ALLOWLIST actually expects; see kernel/src/main.rs's tcp_test_spawn.
#
# The interrupt-vector check below allows any vector in 0x20-0x2f
# (IRQ0-15), the same widened allowance run_nic_test.sh/
# run_arp_icmp_test.sh already use.

set -uo pipefail

ISO="${1:?usage: $0 <tcptest-iso>}"
BOOT_TIMEOUT="${TEST_TIMEOUT:-60}"
PORT="${TCP_TEST_PORT:-17735}"

SERIAL_LOG=$(mktemp)
INT_LOG=$(mktemp)
PCAP=$(mktemp)
trap 'rm -f "$SERIAL_LOG" "$INT_LOG" "$PCAP"' EXIT

timeout "$BOOT_TIMEOUT" qemu-system-i386 \
    -cdrom "$ISO" \
    -netdev socket,id=n0,listen=:"$PORT" \
    -device rtl8139,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial "file:$SERIAL_LOG" \
    -display none \
    -d int -D "$INT_LOG" \
    -no-reboot -monitor none &
QEMU_BG_PID=$!

FAILED=0

python3 - "$PORT" <<'PYEOF'
import socket
import struct
import sys
import time

PORT = int(sys.argv[1])

PEER_MAC = bytes([0x02, 0x00, 0x00, 0x00, 0x00, 0x02])
PEER_IP = bytes([10, 0, 2, 1])
PEER_PORT = 8080
STATIC_IP = bytes([10, 0, 2, 15])
NETSTACK_PORT = 51820

ETHERTYPE_ARP = b"\x08\x06"
ETHERTYPE_IPV4 = b"\x08\x00"
IP_PROTO_TCP = 6

FLAG_FIN = 0x01
FLAG_SYN = 0x02
FLAG_RST = 0x04
FLAG_PSH = 0x08
FLAG_ACK = 0x10

REQUEST = b"GET / HTTP/1.0\r\nHost: 10.0.2.1\r\n\r\n"
RESPONSE = b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 13\r\n\r\nZephyrLite OK"

SERVER_ISN = 0x2000_0000


def checksum16(data: bytes) -> int:
    if len(data) % 2:
        data += b"\x00"
    total = sum(struct.unpack(f">{len(data) // 2}H", data))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return (~total) & 0xFFFF


def connect(port: int) -> socket.socket:
    for _ in range(200):
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=1)
        except OSError:
            time.sleep(0.1)
    print("FAIL: could not connect to QEMU's -netdev socket listener")
    sys.exit(1)


def send_frame(sock: socket.socket, frame: bytes) -> None:
    if len(frame) < 60:
        frame = frame + b"\x00" * (60 - len(frame))
    sock.sendall(struct.pack(">I", len(frame)) + frame)


class FrameStream:
    """See run_arp_icmp_test.sh's identical class for why this buffers
    partial reads across polling calls."""

    def __init__(self, sock: socket.socket):
        self.sock = sock
        self.buf = bytearray()

    def try_recv_frame(self, deadline: float):
        while True:
            if len(self.buf) >= 4:
                length = struct.unpack(">I", self.buf[:4])[0]
                if len(self.buf) >= 4 + length:
                    frame = bytes(self.buf[4:4 + length])
                    del self.buf[:4 + length]
                    return frame
            remaining = deadline - time.time()
            if remaining <= 0:
                return None
            self.sock.settimeout(min(remaining, 0.5))
            try:
                chunk = self.sock.recv(4096)
            except socket.timeout:
                continue
            if not chunk:
                raise ConnectionError("peer closed the connection")
            self.buf += chunk


def wait_for(stream: "FrameStream", predicate, deadline: float):
    while True:
        now = time.time()
        if now >= deadline:
            return None
        frame = stream.try_recv_frame(deadline)
        if frame is not None and predicate(frame):
            return frame


def is_arp_request_for_peer(frame: bytes) -> bool:
    if len(frame) < 42 or frame[12:14] != ETHERTYPE_ARP:
        return False
    arp = frame[14:42]
    return arp[6:8] == b"\x00\x01" and arp[24:28] == PEER_IP


def build_arp_reply(netstack_mac: bytes) -> bytes:
    frame = bytearray(42)
    frame[0:6] = netstack_mac
    frame[6:12] = PEER_MAC
    frame[12:14] = ETHERTYPE_ARP
    frame[14:16] = b"\x00\x01"  # HTYPE
    frame[16:18] = b"\x08\x00"  # PTYPE
    frame[18] = 6
    frame[19] = 4
    frame[20:22] = b"\x00\x02"  # OPER: reply
    frame[22:28] = PEER_MAC  # SHA
    frame[28:32] = PEER_IP  # SPA
    frame[32:38] = netstack_mac  # THA
    frame[38:42] = STATIC_IP  # TPA
    return bytes(frame)


def build_tcp_segment(netstack_mac: bytes, seq: int, ack: int, flags: int, payload: bytes) -> bytes:
    tcp = bytearray(20 + len(payload))
    tcp[0:2] = struct.pack(">H", PEER_PORT)
    tcp[2:4] = struct.pack(">H", NETSTACK_PORT)
    tcp[4:8] = struct.pack(">I", seq)
    tcp[8:12] = struct.pack(">I", ack)
    tcp[12] = 0x50
    tcp[13] = flags
    tcp[14:16] = struct.pack(">H", 2048)
    tcp[16:18] = b"\x00\x00"
    tcp[18:20] = b"\x00\x00"
    tcp[20:] = payload

    pseudo = PEER_IP + STATIC_IP + b"\x00" + bytes([IP_PROTO_TCP]) + struct.pack(">H", len(tcp))
    tcp_sum = checksum16(pseudo + bytes(tcp))
    tcp[16:18] = struct.pack(">H", tcp_sum)

    ip = bytearray(20)
    ip[0] = 0x45
    ip[1] = 0
    ip[2:4] = struct.pack(">H", 20 + len(tcp))
    ip[4:6] = b"\x00\x00"
    ip[6:8] = b"\x00\x00"
    ip[8] = 64
    ip[9] = IP_PROTO_TCP
    ip[10:12] = b"\x00\x00"
    ip[12:16] = PEER_IP
    ip[16:20] = STATIC_IP
    ip[10:12] = struct.pack(">H", checksum16(bytes(ip)))

    frame = bytearray(14)
    frame[0:6] = netstack_mac
    frame[6:12] = PEER_MAC
    frame[12:14] = ETHERTYPE_IPV4
    return bytes(frame) + bytes(ip) + bytes(tcp)


def parse_tcp_segment(frame: bytes):
    if len(frame) < 14 + 20 + 20 or frame[12:14] != ETHERTYPE_IPV4:
        return None
    ip = frame[14:34]
    if ip[0] != 0x45 or ip[9] != IP_PROTO_TCP:
        return None
    if ip[12:16] != STATIC_IP or ip[16:20] != PEER_IP:
        return None
    total_len = struct.unpack(">H", ip[2:4])[0]
    tcp = frame[34:14 + total_len]
    if len(tcp) < 20:
        return None
    data_offset = (tcp[12] >> 4) * 4
    return {
        "src_port": struct.unpack(">H", tcp[0:2])[0],
        "dst_port": struct.unpack(">H", tcp[2:4])[0],
        "seq": struct.unpack(">I", tcp[4:8])[0],
        "ack": struct.unpack(">I", tcp[8:12])[0],
        "flags": tcp[13],
        "payload": tcp[data_offset:],
    }


def is_tcp_for_us(frame: bytes) -> bool:
    seg = parse_tcp_segment(frame)
    return seg is not None and seg["src_port"] == NETSTACK_PORT and seg["dst_port"] == PEER_PORT


sock = connect(PORT)
stream = FrameStream(sock)

# --- ARP: netstack resolves this peer's MAC before it can open a
# connection to it. ---
deadline = time.time() + 30
req = wait_for(stream, is_arp_request_for_peer, deadline)
if req is None:
    print("FAIL: no ARP request for the test peer's IP seen from netstack")
    sys.exit(1)
netstack_mac = req[6:12]
send_frame(sock, build_arp_reply(netstack_mac))
print(f"PASS: answered netstack's ARP request (netstack MAC {netstack_mac.hex()})")

# --- Three-way handshake ---
deadline = time.time() + 20
syn = wait_for(stream, is_tcp_for_us, deadline)
if syn is None:
    print("FAIL: no TCP SYN received from netstack")
    sys.exit(1)
seg = parse_tcp_segment(syn)
if seg["flags"] & FLAG_SYN == 0:
    print(f"FAIL: first TCP segment from netstack wasn't a SYN (flags={seg['flags']:#04x})")
    sys.exit(1)
client_isn = seg["seq"]
server_seq = SERVER_ISN
client_next = (client_isn + 1) & 0xFFFFFFFF
send_frame(sock, build_tcp_segment(netstack_mac, server_seq, client_next, FLAG_SYN | FLAG_ACK, b""))
server_seq = (server_seq + 1) & 0xFFFFFFFF
print("PASS: sent SYN-ACK in response to netstack's SYN")

# --- Handshake ACK, then the request itself (possibly the same segment) ---
request_data = b""
deadline = time.time() + 20
while len(request_data) < len(REQUEST):
    seg_frame = wait_for(stream, is_tcp_for_us, deadline)
    if seg_frame is None:
        print("FAIL: handshake never completed / request never arrived")
        sys.exit(1)
    seg = parse_tcp_segment(seg_frame)
    if seg["flags"] & FLAG_RST:
        print("FAIL: netstack sent a RST")
        sys.exit(1)
    if seg["ack"] != server_seq:
        continue  # a stray/duplicate segment -- ignore, keep waiting
    if seg["payload"]:
        request_data += seg["payload"]
        client_next = (client_next + len(seg["payload"])) & 0xFFFFFFFF
        send_frame(sock, build_tcp_segment(netstack_mac, server_seq, client_next, FLAG_ACK, b""))

if request_data != REQUEST:
    print(f"FAIL: request bytes didn't match -- got {request_data!r}")
    sys.exit(1)
print("PASS: received netstack's HTTP request intact")

# --- Response, then close (PSH+ACK+FIN combined, as a real HTTP/1.0
# server closing right after its response commonly does) ---
send_frame(sock, build_tcp_segment(netstack_mac, server_seq, client_next, FLAG_PSH | FLAG_ACK | FLAG_FIN, RESPONSE))
server_seq = (server_seq + len(RESPONSE) + 1) & 0xFFFFFFFF
print("PASS: sent HTTP response + FIN")

# --- Final ACK from netstack, completing its passive close ---
deadline = time.time() + 20
final_ack = wait_for(
    stream,
    lambda f: is_tcp_for_us(f) and parse_tcp_segment(f)["flags"] & FLAG_ACK and parse_tcp_segment(f)["ack"] == server_seq,
    deadline,
)
if final_ack is None:
    print("FAIL: netstack never ACKed the close")
    sys.exit(1)
print("PASS: netstack completed the close handshake")
sys.exit(0)
PYEOF
PEER_STATUS=$?

# http_client_test (task id 5) still has its own remaining work to do
# after the wire-level close handshake the peer script just observed --
# comparing the accumulated response bytes and calling exit() -- purely
# in-guest CPU scheduling with no further wire traffic to watch for, so
# give it a brief grace period before killing QEMU rather than racing it.
sleep 1
kill "$QEMU_BG_PID" >/dev/null 2>&1
wait "$QEMU_BG_PID" 2>/dev/null

if [ "$PEER_STATUS" -ne 0 ]; then
    FAILED=1
fi

if grep -q "task 5 exited with code 0" "$SERIAL_LOG"; then
    echo "PASS: http_client_test (task 5 exited 0)"
else
    echo "FAIL: http_client_test (task 5 did not exit 0)"
    FAILED=1
fi

python3 - "$PCAP" <<'PYEOF'
import struct
import sys

path = sys.argv[1]
with open(path, "rb") as f:
    data = f.read()

if len(data) < 24:
    print("FAIL: pcap capture is empty or missing")
    sys.exit(1)

magic = struct.unpack("<I", data[0:4])[0]
endian = "<" if magic == 0xA1B2C3D4 else ">"

static_ip = bytes([10, 0, 2, 15])
peer_ip = bytes([10, 0, 2, 1])
offset = 24
saw_syn = False
saw_syn_ack = False
saw_data = False
saw_fin = False

while offset + 16 <= len(data):
    _ts_sec, _ts_usec, incl_len, _orig_len = struct.unpack(endian + "IIII", data[offset:offset + 16])
    offset += 16
    frame = data[offset:offset + incl_len]
    offset += incl_len

    if len(frame) < 34 or frame[12:14] != b"\x08\x00":
        continue
    ip = frame[14:34]
    if ip[9] != 6:
        continue
    total_len = struct.unpack(">H", ip[2:4])[0]
    tcp = frame[34:14 + total_len]
    if len(tcp) < 20:
        continue
    flags = tcp[13]
    src_ip, dst_ip = ip[12:16], ip[16:20]

    if src_ip == static_ip and dst_ip == peer_ip and flags == 0x02:
        saw_syn = True
    if src_ip == peer_ip and dst_ip == static_ip and flags & 0x12 == 0x12:
        saw_syn_ack = True
    if src_ip == static_ip and dst_ip == peer_ip and len(tcp) > 20:
        saw_data = True
    if src_ip == peer_ip and dst_ip == static_ip and flags & 0x01:
        saw_fin = True

checks = [
    (saw_syn, "real SYN from netstack observed on the wire"),
    (saw_syn_ack, "real SYN-ACK to netstack observed on the wire"),
    (saw_data, "real data segment from netstack observed on the wire"),
    (saw_fin, "real FIN from the test peer observed on the wire"),
]
ok = True
for passed, desc in checks:
    print(("PASS: " if passed else "FAIL: ") + desc)
    ok = ok and passed

sys.exit(0 if ok else 1)
PYEOF
PCAP_CHECK=$?
if [ "$PCAP_CHECK" -ne 0 ]; then
    FAILED=1
fi

UNEXPECTED=$(grep -o 'v=[0-9a-f]*' "$INT_LOG" | sort -u | grep -v -E '^v=(80|2[0-9a-f])$' || true)
if [ -n "$UNEXPECTED" ]; then
    echo "FAIL: unexpected interrupt vector(s): $(echo "$UNEXPECTED" | tr '\n' ' ')"
    FAILED=1
else
    echo "PASS: no unexpected interrupt vectors"
fi

if [ "$FAILED" -ne 0 ]; then
    echo
    echo "=== serial log ==="
    cat "$SERIAL_LOG"
    echo
    echo "Some tests FAILED."
    exit 1
fi

echo
echo "All tests passed."
