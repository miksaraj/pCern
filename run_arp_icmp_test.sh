#!/usr/bin/env bash
# Boots the ARP/ICMP-test ISO (see `make test-arp` / `make iso-arptest`)
# headlessly in QEMU and verifies netstack's real replies against a
# genuine external peer -- not a simulation, and not QEMU's own usermode
# network stack answering on netstack's behalf (as run_nic_test.sh's
# gateway-replies-to-us case is): this script itself acts as a second
# host on the wire, sending real ARP and ICMP echo requests and checking
# netstack's real replies byte-for-byte, including recomputing every
# checksum independently rather than trusting whatever value netstack
# put there.
#
# Unlike every other `-device rtl8139` test in this project, this one
# uses `-netdev socket,listen=:PORT` instead of `-netdev user`: slirp
# (usermode networking) is NAT-outbound-only by design -- nothing can
# reach *into* the guest except via `hostfwd`'s TCP/UDP-only port
# forwarding, which can't carry ARP or ICMP at all. `-netdev socket`
# instead gives a plain raw-Ethernet pipe over an ordinary TCP socket
# (QEMU listens; this script connects as a client), with no NAT and no
# protocol restriction -- exactly the "this script is genuinely another
# host on the same wire" shape this test needs. Each frame on that pipe
# is a 4-byte big-endian length prefix followed by that many bytes of
# raw Ethernet frame -- QEMU's own simple framing for this netdev type,
# confirmed empirically against this project's own nic_test fixture
# before this script was written.
#
# netstack never exits (it's a long-running responder, not a one-shot
# fixture), so unlike every other driver test here there's no in-guest
# exit code to check at all -- this script's own pass/fail *is* the
# entire signal, checked while QEMU is still running rather than after
# the fact. QEMU is killed as soon as the peer script finishes rather
# than left to run out its full boot timeout.
#
# Also cross-checks the same round trips independently via a real packet
# capture (`-object filter-dump`), the same "don't just trust what this
# script's own socket believes" pattern run_nic_test.sh's pcap check
# already established.
#
# Task ids in this build: nameservice=1, console_server=2, storage_ata=3,
# fs_fat32=4, netstack=5, net_rtl8139=6 -- netstack spawns first here
# (module index 4) purely so net_rtl8139 still lands at task id 6, the
# same id nameservice's ALLOWLIST hardcodes to "net" in every other boot
# configuration; spawn order in main.rs doesn't affect which task's own
# code the scheduler actually runs first, so netstack still finds "net"
# registered by the time its own retry loop looks it up.
#
# The interrupt-vector check below additionally allows any vector in
# 0x20-0x2f (IRQ0-15, this kernel's full remapped PIC range), the same
# widened allowance run_nic_test.sh uses and for the same reason:
# whichever line PCI routed the RTL8139's interrupt to varies by
# chipset/BIOS and isn't worth hardcoding.

set -uo pipefail

ISO="${1:?usage: $0 <arptest-iso>}"
BOOT_TIMEOUT="${TEST_TIMEOUT:-60}"
PORT="${ARP_ICMP_TEST_PORT:-17734}"

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

PEER_MAC = bytes([0x02, 0x00, 0x00, 0x00, 0x00, 0x01])
PEER_IP = bytes([10, 0, 2, 20])
STATIC_IP = bytes([10, 0, 2, 15])
BROADCAST = b"\xff" * 6

ETHERTYPE_ARP = b"\x08\x06"
ETHERTYPE_IPV4 = b"\x08\x00"


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
    """Buffers partial reads across polling calls so a caller can
    interleave periodic re-sends with receive attempts -- needed because
    the guest's boot (several task spawns, a heap smoke test, PCI
    enumeration) takes real wall-clock time, and a frame sent before
    net_rtl8139 has enabled its receive ring is simply gone, never
    queued for later. A plain send-once-then-block approach would race
    that boot window; retrying periodically until the deadline does not.
    """

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


def wait_for_frame(stream: "FrameStream", predicate, deadline: float, resend, resend_interval: float = 1.0):
    next_send = 0.0
    while True:
        now = time.time()
        if now >= deadline:
            return None
        if now >= next_send:
            resend()
            next_send = now + resend_interval
        frame = stream.try_recv_frame(min(next_send, deadline))
        if frame is not None and predicate(frame):
            return frame


def build_arp_request() -> bytes:
    frame = bytearray(42)
    frame[0:6] = BROADCAST
    frame[6:12] = PEER_MAC
    frame[12:14] = ETHERTYPE_ARP
    frame[14:16] = b"\x00\x01"  # HTYPE
    frame[16:18] = b"\x08\x00"  # PTYPE
    frame[18] = 6
    frame[19] = 4
    frame[20:22] = b"\x00\x01"  # OPER: request
    frame[22:28] = PEER_MAC  # SHA
    frame[28:32] = PEER_IP  # SPA
    frame[32:38] = b"\x00" * 6  # THA: unknown
    frame[38:42] = STATIC_IP  # TPA
    return bytes(frame)


def is_arp_reply_for_us(frame: bytes) -> bool:
    if len(frame) < 42:
        return False
    if frame[0:6] != PEER_MAC or frame[12:14] != ETHERTYPE_ARP:
        return False
    return frame[20:22] == b"\x00\x02" and frame[28:32] == STATIC_IP and frame[38:42] == PEER_IP


def build_icmp_echo_request(target_mac: bytes, ident: int, seq: int, payload: bytes) -> bytes:
    icmp = bytearray(8 + len(payload))
    icmp[0] = 8  # type: echo request
    icmp[1] = 0  # code
    icmp[4:6] = struct.pack(">H", ident)
    icmp[6:8] = struct.pack(">H", seq)
    icmp[8:] = payload
    icmp[2:4] = struct.pack(">H", checksum16(bytes(icmp)))

    ip = bytearray(20)
    ip[0] = 0x45
    ip[1] = 0
    total_len = 20 + len(icmp)
    ip[2:4] = struct.pack(">H", total_len)
    ip[4:6] = struct.pack(">H", 0x1234)  # identification
    ip[6:8] = b"\x00\x00"  # flags/fragment
    ip[8] = 64  # TTL
    ip[9] = 1  # protocol: ICMP
    ip[10:12] = b"\x00\x00"  # checksum, filled below
    ip[12:16] = PEER_IP
    ip[16:20] = STATIC_IP
    ip[10:12] = struct.pack(">H", checksum16(bytes(ip)))

    frame = bytearray(14)
    frame[0:6] = target_mac
    frame[6:12] = PEER_MAC
    frame[12:14] = ETHERTYPE_IPV4
    return bytes(frame) + bytes(ip) + bytes(icmp)


def check_icmp_echo_reply(frame: bytes, ident: int, seq: int, payload: bytes) -> list:
    problems = []
    if len(frame) < 14 + 20 + 8:
        return ["reply frame too short"]
    if frame[0:6] != PEER_MAC:
        problems.append(f"Ethernet dest {frame[0:6].hex()} != peer MAC")
    if frame[12:14] != ETHERTYPE_IPV4:
        problems.append("not an IPv4 frame")
        return problems

    ip = frame[14:34]
    if ip[0] != 0x45:
        problems.append(f"unexpected IPv4 version/IHL byte {ip[0]:#04x}")
    if ip[9] != 1:
        problems.append(f"protocol {ip[9]} != ICMP")
    if ip[12:16] != STATIC_IP:
        problems.append(f"IP src {ip[12:16]} != netstack's static IP")
    if ip[16:20] != PEER_IP:
        problems.append(f"IP dst {ip[16:20]} != peer IP")
    ip_sum = checksum16(bytes(ip))
    if ip_sum != 0:
        problems.append(f"IPv4 header checksum invalid (residual {ip_sum:#06x})")

    total_len = struct.unpack(">H", ip[2:4])[0]
    icmp = frame[34:14 + total_len]
    if len(icmp) < 8:
        problems.append("ICMP message too short")
        return problems
    if icmp[0] != 0:
        problems.append(f"ICMP type {icmp[0]} != 0 (echo reply)")
    if icmp[1] != 0:
        problems.append(f"ICMP code {icmp[1]} != 0")
    if struct.unpack(">H", icmp[4:6])[0] != ident:
        problems.append("ICMP identifier not echoed back correctly")
    if struct.unpack(">H", icmp[6:8])[0] != seq:
        problems.append("ICMP sequence not echoed back correctly")
    if icmp[8:] != payload:
        problems.append("ICMP payload not echoed back correctly")
    icmp_sum = checksum16(bytes(icmp))
    if icmp_sum != 0:
        problems.append(f"ICMP checksum invalid (residual {icmp_sum:#06x})")
    return problems


sock = connect(PORT)
stream = FrameStream(sock)

# --- ARP --- (resent periodically: the guest is still booting when this
# script first connects, so the first attempt or two may arrive before
# net_rtl8139 has enabled its receive ring at all)
deadline = time.time() + 20
reply = wait_for_frame(
    stream,
    is_arp_reply_for_us,
    deadline,
    resend=lambda: send_frame(sock, build_arp_request()),
)

if reply is None:
    print("FAIL: no valid ARP reply received from netstack")
    sys.exit(1)
netstack_mac = reply[22:28]
print(f"PASS: netstack answered our ARP request (MAC {netstack_mac.hex()})")

# --- ICMP echo ---
ident, seq = 0xBEEF, 1
payload = b"ZephyrLite-Checkpoint-X-ping-payload!!"


def is_icmp_reply_candidate(frame: bytes) -> bool:
    return len(frame) >= 14 + 20 + 8 and frame[12:14] == ETHERTYPE_IPV4 and frame[14] == 0x45 and frame[23] == 1


deadline = time.time() + 20
reply_frame = wait_for_frame(
    stream,
    is_icmp_reply_candidate,
    deadline,
    resend=lambda: send_frame(sock, build_icmp_echo_request(netstack_mac, ident, seq, payload)),
)

if reply_frame is None:
    print("FAIL: no ICMP reply received from netstack")
    sys.exit(1)

problems = check_icmp_echo_reply(reply_frame, ident, seq, payload)
if problems:
    print("FAIL: ICMP echo reply did not validate:")
    for p in problems:
        print(f"  - {p}")
    sys.exit(1)

print("PASS: netstack answered our ICMP echo request with a correct reply")
sys.exit(0)
PYEOF
PEER_STATUS=$?

kill "$QEMU_BG_PID" >/dev/null 2>&1
wait "$QEMU_BG_PID" 2>/dev/null

if [ "$PEER_STATUS" -ne 0 ]; then
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
offset = 24
saw_arp_reply = False
saw_icmp_reply = False
while offset + 16 <= len(data):
    _ts_sec, _ts_usec, incl_len, _orig_len = struct.unpack(endian + "IIII", data[offset:offset + 16])
    offset += 16
    frame = data[offset:offset + incl_len]
    offset += incl_len

    if len(frame) < 14:
        continue
    ethertype = frame[12:14]
    if ethertype == b"\x08\x06" and len(frame) >= 42:
        arp = frame[14:42]
        if arp[6:8] == b"\x00\x02" and arp[14:18] == static_ip:
            saw_arp_reply = True
    elif ethertype == b"\x08\x00" and len(frame) >= 34:
        ip = frame[14:34]
        if ip[9] == 1 and ip[12:16] == static_ip and len(frame) >= 34 + 8 and frame[34] == 0:
            saw_icmp_reply = True

if saw_arp_reply:
    print("PASS: real ARP reply from netstack observed on the wire")
else:
    print("FAIL: no ARP reply from netstack observed on the wire")

if saw_icmp_reply:
    print("PASS: real ICMP echo reply from netstack observed on the wire")
else:
    print("FAIL: no ICMP echo reply from netstack observed on the wire")

sys.exit(0 if (saw_arp_reply and saw_icmp_reply) else 1)
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
