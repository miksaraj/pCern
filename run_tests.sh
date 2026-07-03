#!/usr/bin/env bash
# Boots the test-harness ISO (see `make test`) headlessly in QEMU and
# checks that every cap_test fixture ran to completion successfully.
#
# Task ids are fixed by main.rs's spawn order (nameservice=1,
# console_server=2, storage_ata=3, fs_fat32=4), then test_harness_spawn's
# fixtures: cap_test_a=5, cap_test_b=6, mem_test_a=7, mem_test_b=8,
# fs_client_test=9 (then idle_task=10, spawned right after
# test_harness_spawn returns). storage_client_test isn't spawned here --
# see main.rs's test_harness_spawn for why it can't coexist with
# fs_fat32. Exit codes are the authoritative pass/fail signal (see the
# project's established convention) -- console text is not, since
# multiple fixtures printing concurrently interleave byte-for-byte.
#
# fs_client_test additionally exercises the new SYS_SPAWN_FROM_MEMORY
# syscall (Checkpoint M) after its own fs_fat32 checks -- loading and
# running LOADED.BIN rather than a second fixture connecting to fs_fat32
# concurrently (which only supports one client at a time). Since task ids
# are a monotonic counter never reused, and every static spawn above
# happens before the scheduler ever runs anything, that dynamically
# spawned program deterministically lands at task id 11 (one past
# idle_task's 10).

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

check_exit 5 "cap_test_a  -- capability transfer/badging"
check_exit 6 "cap_test_b  -- capability revocation"
check_exit 7 "mem_test_a  -- shared memory grant (writer)"
check_exit 8 "mem_test_b  -- shared memory grant (reader)"
check_exit 9 "fs_client_test -- fs_fat32 protocol (exercises storage_ata too)"

if grep -q "task 11 exited with code 42" "$SERIAL_LOG"; then
    echo "PASS: LOADED.BIN -- dynamically spawned program actually executed (task 11 exited 42)"
else
    echo "FAIL: LOADED.BIN -- dynamically spawned program did not exit with its distinctive code"
    FAILED=1
fi

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
