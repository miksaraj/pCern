"""Shared helpers for this project's real-traffic test scripts
(run_arp_icmp_test.sh, run_tcp_test.sh) -- the raw-Ethernet-over-TCP
frame pipe (`connect`/`send_frame`/`FrameStream`), the Internet checksum
(`checksum16`), and walking a libpcap capture's frame records
(`iter_pcap_frames`) were byte-for-byte duplicated between the two
scripts' inline Python before this module existed. Each script's own
per-frame ARP/ICMP/TCP predicates and frame-building logic stay where
they are -- only the framing/transport/capture-format plumbing that
doesn't depend on which protocol is being tested lives here.
"""

import socket
import struct
import sys
import time


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


def iter_pcap_frames(path: str):
    """Yields each captured frame's raw bytes from a libpcap capture
    file, handling the length check, magic-number endianness detection,
    and 16-byte-record-header walk common to every consumer -- callers
    supply their own per-frame predicates. Calls `sys.exit(1)` itself if
    the capture is empty or missing, since every caller wants that exact
    failure regardless of what it's actually looking for in the frames.
    """
    with open(path, "rb") as f:
        data = f.read()

    if len(data) < 24:
        print("FAIL: pcap capture is empty or missing")
        sys.exit(1)

    magic = struct.unpack("<I", data[0:4])[0]
    endian = "<" if magic == 0xA1B2C3D4 else ">"

    offset = 24
    while offset + 16 <= len(data):
        _ts_sec, _ts_usec, incl_len, _orig_len = struct.unpack(endian + "IIII", data[offset:offset + 16])
        offset += 16
        yield data[offset:offset + incl_len]
        offset += incl_len
