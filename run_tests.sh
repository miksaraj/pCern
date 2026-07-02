#!/usr/bin/env bash
# Boots the test-harness ISO (see `make test`) headlessly in QEMU and
# checks that every cap_test fixture ran to completion successfully.
#
# Task ids are fixed by main.rs's spawn order (nameservice=1,
# console_server=2, the two background kernel tasks=3-4, storage_ata=5,
# fs_fat32=6), then test_harness_spawn's fixtures: cap_test_a=7,
# cap_test_b=8, mem_test_a=9, mem_test_b=10, fs_client_test=11.
# storage_client_test isn't spawned here -- see main.rs's
# test_harness_spawn for why it can't coexist with fs_fat32. Exit codes
# are the authoritative pass/fail signal (see the project's established
# convention) -- console text is not, since multiple fixtures printing
# concurrently interleave byte-for-byte.

set -uo pipefail

ISO="${1:?usage: $0 <test-iso> <fat32-test-image>}"
DISK="${2:?usage: $0 <test-iso> <fat32-test-image>}"
BOOT_TIMEOUT="${TEST_TIMEOUT:-120}"

SERIAL_LOG=$(mktemp)
INT_LOG=$(mktemp)
trap 'rm -f "$SERIAL_LOG" "$INT_LOG"' EXIT

timeout "$BOOT_TIMEOUT" qemu-system-i386 \
    -cdrom "$ISO" \
    -boot d \
    -drive "file=$DISK,if=ide,index=0,format=raw" \
    -serial "file:$SERIAL_LOG" \
    -display none \
    -d int -D "$INT_LOG" \
    -no-reboot -monitor none

FAILED=0

check_exit() {
    local task_id="$1" name="$2"
    if grep -q "task $task_id exited with code 0" "$SERIAL_LOG"; then
        echo "PASS: $name (task $task_id exited 0)"
    else
        echo "FAIL: $name (task $task_id did not exit 0)"
        FAILED=1
    fi
}

check_exit 7  "cap_test_a  -- capability transfer/badging"
check_exit 8  "cap_test_b  -- capability revocation"
check_exit 9  "mem_test_a  -- shared memory grant (writer)"
check_exit 10 "mem_test_b  -- shared memory grant (reader)"
check_exit 11 "fs_client_test -- fs_fat32 protocol (exercises storage_ata too)"

UNEXPECTED=$(grep -o 'v=[0-9a-f]*' "$INT_LOG" | sort -u | grep -v -e '^v=20$' -e '^v=80$' || true)
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
