#!/usr/bin/env bash
# Boots the raw-input test ISO (see `make test-raw-input` / `make
# iso-rawtest`) headlessly in QEMU with its own monitor socket, waits for
# raw_input_test's own readiness marker on serial, then injects real PS/2
# keystrokes via QEMU's monitor `sendkey` command -- a plain printable
# key, an extended (0xE0-prefixed) arrow key, and a Ctrl-chord, each
# exercising a different new decoding path added in Phase 7, Checkpoint R
# (see userland/cap_test/src/bin/raw_input_test.rs).
#
# Pass/fail is still the fixture's own exit code, checked from the same
# serial log the readiness marker came from -- same convention as
# run_console_input_test.sh.
#
# Task ids in this build: nameservice=1, console_server=2, storage_ata=3,
# fs_fat32=4, raw_input_test=5.

set -uo pipefail

ISO="${1:?usage: $0 <rawtest-iso>}"
BOOT_TIMEOUT="${TEST_TIMEOUT:-60}"
READY_TIMEOUT="${READY_TIMEOUT:-30}"

SERIAL_LOG=$(mktemp)
MONITOR_SOCK=$(mktemp -u)
QEMU_PID=""
trap 'rm -f "$SERIAL_LOG" "$MONITOR_SOCK"; [ -n "$QEMU_PID" ] && kill "$QEMU_PID" 2>/dev/null' EXIT

qemu-system-i386 \
    -cdrom "$ISO" \
    -serial "file:$SERIAL_LOG" \
    -display none \
    -no-reboot \
    -monitor "unix:$MONITOR_SOCK,server,nowait" &
QEMU_PID=$!

for _ in $(seq 1 100); do
    [ -S "$MONITOR_SOCK" ] && break
    sleep 0.1
done
if [ ! -S "$MONITOR_SOCK" ]; then
    echo "FAIL: QEMU monitor socket never appeared"
    exit 1
fi

ready=0
for _ in $(seq 1 $((READY_TIMEOUT * 10))); do
    if grep -q "raw_input_test: ready" "$SERIAL_LOG" 2>/dev/null; then
        ready=1
        break
    fi
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        break
    fi
    sleep 0.1
done

if [ "$ready" -ne 1 ]; then
    echo "FAIL: raw_input_test never printed its readiness marker"
    kill "$QEMU_PID" 2>/dev/null
    wait "$QEMU_PID" 2>/dev/null
    QEMU_PID=""
    echo
    echo "=== serial log ==="
    cat "$SERIAL_LOG"
    exit 1
fi

python3 - "$MONITOR_SOCK" <<'PYEOF'
import socket
import sys
import time

sock_path = sys.argv[1]
keys = ["a", "left", "ctrl-a"]

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sock_path)
time.sleep(0.2)
s.recv(4096)  # discard the monitor banner/prompt

for k in keys:
    s.sendall(("sendkey " + k + "\n").encode())
    time.sleep(0.05)
    s.recv(4096)

s.close()
PYEOF

# Same reasoning as run_console_input_test.sh: the kernel idles forever
# after the fixture exits, so poll serial for the exit line rather than
# waiting on process death.
done_exiting=0
for _ in $(seq 1 $((BOOT_TIMEOUT * 10))); do
    if grep -q "task 5 exited with code" "$SERIAL_LOG" 2>/dev/null; then
        done_exiting=1
        break
    fi
    sleep 0.1
done

kill "$QEMU_PID" 2>/dev/null
wait "$QEMU_PID" 2>/dev/null
QEMU_PID=""

FAILED=0
if [ "$done_exiting" -ne 1 ]; then
    echo "FAIL: raw_input_test never exited"
    FAILED=1
elif grep -q "task 5 exited with code 0" "$SERIAL_LOG"; then
    echo "PASS: raw_input_test (task 5 exited 0)"
else
    echo "FAIL: raw_input_test (task 5 did not exit 0)"
    FAILED=1
fi

if [ "$FAILED" -ne 0 ]; then
    echo
    echo "=== serial log ==="
    cat "$SERIAL_LOG"
    exit 1
fi

echo "raw_input_test passed."
