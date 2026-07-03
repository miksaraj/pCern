#!/usr/bin/env bash
# Boots the editor test ISO (see `make test-editor` / `make
# iso-editortest`) headlessly in QEMU with its own monitor socket and the
# shared FAT32 test image attached (editor_input_test needs a real
# fs_fat32 behind it, unlike console_input_test/raw_input_test which never
# touch the filesystem), waits for editor_input_test's own readiness
# marker on serial, then injects a real scripted edit session via QEMU's
# monitor `sendkey` command -- type "hello", move the cursor left twice,
# insert 'x', backspace it back out, save with Ctrl-S (see
# userland/cap_test/src/bin/editor_input_test.rs).
#
# Pass/fail is still the fixture's own exit code, checked from the same
# serial log the readiness marker came from -- same convention as
# run_console_input_test.sh/run_raw_input_test.sh.
#
# Task ids in this build: nameservice=1, console_server=2, storage_ata=3,
# fs_fat32=4, editor_input_test=5.

set -uo pipefail

ISO="${1:?usage: $0 <editortest-iso> <fat32-test-image>}"
DISK="${2:?usage: $0 <editortest-iso> <fat32-test-image>}"
BOOT_TIMEOUT="${TEST_TIMEOUT:-60}"
READY_TIMEOUT="${READY_TIMEOUT:-30}"

SERIAL_LOG=$(mktemp)
MONITOR_SOCK=$(mktemp -u)
QEMU_PID=""
trap 'rm -f "$SERIAL_LOG" "$MONITOR_SOCK"; [ -n "$QEMU_PID" ] && kill "$QEMU_PID" 2>/dev/null' EXIT

qemu-system-i386 \
    -cdrom "$ISO" \
    -boot d \
    -drive "file=$DISK,if=ide,index=0,format=raw" \
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
    if grep -q "editor_input_test: ready" "$SERIAL_LOG" 2>/dev/null; then
        ready=1
        break
    fi
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        break
    fi
    sleep 0.1
done

if [ "$ready" -ne 1 ]; then
    echo "FAIL: editor_input_test never printed its readiness marker"
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
keys = ["h", "e", "l", "l", "o", "left", "left", "x", "backspace", "ctrl-s"]

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

# Same reasoning as run_console_input_test.sh/run_raw_input_test.sh: the
# kernel idles forever after the fixture exits, so poll serial for the
# exit line rather than waiting on process death.
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
    echo "FAIL: editor_input_test never exited"
    FAILED=1
elif grep -q "task 5 exited with code 0" "$SERIAL_LOG"; then
    echo "PASS: editor_input_test (task 5 exited 0)"
else
    echo "FAIL: editor_input_test (task 5 did not exit 0)"
    FAILED=1
fi

if [ "$FAILED" -ne 0 ]; then
    echo
    echo "=== serial log ==="
    cat "$SERIAL_LOG"
    exit 1
fi

echo "editor_input_test passed."
