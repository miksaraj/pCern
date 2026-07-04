#!/usr/bin/env bash
# Boots the NIC-test ISO (see `make test-nic` / `make iso-nictest`)
# headlessly in QEMU with real usermode networking (-netdev user) and a
# real emulated card (-device rtl8139), and checks two independent
# things:
#
# 1. nic_test's own exit code (see userland/cap_test/src/bin/
#    nic_test.rs) -- it hand-builds a real Ethernet+ARP request frame,
#    sends it via the RTL8139 driver, and checks that QEMU's usermode
#    network stack's real ARP reply comes back through the same driver.
# 2. A real packet capture QEMU wrote to disk during the boot
#    (-object filter-dump on the netdev), inspected independently of
#    anything nic_test itself believes -- the same "don't just trust the
#    in-VM report" pattern run_tests.sh's own WRTEST.TXT check already
#    established. A small inline Python script parses the raw pcap
#    format directly (no dependency beyond the python3 already used
#    elsewhere in this project's test scripts) and confirms both a real
#    ARP request left the (virtual) wire and a real ARP reply from the
#    gateway came back.
#
# Task ids in this build: nameservice=1, console_server=2, storage_ata=3,
# fs_fat32=4, net_rtl8139=5, nic_test=6.
#
# The interrupt-vector check below additionally allows any vector in
# 0x20-0x2f (IRQ0-15, this kernel's full remapped PIC range) rather than
# only the timer's/syscall gate's fixed v=20/v=80: unlike every other
# harness, this one legitimately fires a *third* vector too -- whichever
# line PCI routed the RTL8139's interrupt to, which varies by chipset/BIOS
# and isn't worth hardcoding. CPU exceptions (page fault, GPF, etc.) all
# use vectors below 0x20 and are still caught as unexpected.

set -uo pipefail

ISO="${1:?usage: $0 <nictest-iso>}"
BOOT_TIMEOUT="${TEST_TIMEOUT:-60}"

SERIAL_LOG=$(mktemp)
INT_LOG=$(mktemp)
PCAP=$(mktemp)
trap 'rm -f "$SERIAL_LOG" "$INT_LOG" "$PCAP"' EXIT

timeout "$BOOT_TIMEOUT" qemu-system-i386 \
    -cdrom "$ISO" \
    -netdev user,id=n0 \
    -device rtl8139,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial "file:$SERIAL_LOG" \
    -display none \
    -d int -D "$INT_LOG" \
    -no-reboot -monitor none

FAILED=0

if grep -q "task 6 exited with code 0" "$SERIAL_LOG"; then
    echo "PASS: nic_test (task 6 exited 0)"
else
    echo "FAIL: nic_test (task 6 did not exit 0)"
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

offset = 24
saw_request = False
saw_reply = False
while offset + 16 <= len(data):
    _ts_sec, _ts_usec, incl_len, _orig_len = struct.unpack(endian + "IIII", data[offset:offset + 16])
    offset += 16
    frame = data[offset:offset + incl_len]
    offset += incl_len

    if len(frame) < 14 + 28:
        continue
    if frame[12:14] != b"\x08\x06":  # ethertype: ARP
        continue
    arp = frame[14:14 + 28]
    opcode = arp[6:8]
    spa = arp[14:18]
    tpa = arp[24:28]
    gateway = bytes([10, 0, 2, 2])
    if opcode == b"\x00\x01" and tpa == gateway:
        saw_request = True
    if opcode == b"\x00\x02" and spa == gateway:
        saw_reply = True

if saw_request:
    print("PASS: real ARP request observed on the wire (nic_test -> gateway)")
else:
    print("FAIL: no ARP request for the gateway observed on the wire")

if saw_reply:
    print("PASS: real ARP reply observed on the wire (gateway -> nic_test)")
else:
    print("FAIL: no ARP reply from the gateway observed on the wire")

sys.exit(0 if (saw_request and saw_reply) else 1)
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
