#!/usr/bin/env bash
# Boots the installed FAT32 disk (see `make disk`) headlessly in QEMU and
# checks that GRUB, reading straight off that disk's own MBR-embedded
# core.img (no CD/ISO involved at all -- see the root Makefile's `disk`
# target and kernel/grub-disk.cfg), successfully loaded every multiboot
# module and handed off into the normal production boot sequence.
#
# Unlike run_tests.sh's fixtures, a normal production boot has no exit
# code to check -- shell blocks forever on console input and the idle
# task loops forever, both correct/expected, not a bug -- so this checks
# the serial log for the same "spawned ring-3 task '<name>'" markers
# main.rs already prints for every service, bounded by a short timeout
# (nothing after "handing off to the scheduler" is relevant to whether
# the disk itself booted correctly). This is exactly the class of
# console-text check CLAUDE.md's testing philosophy calls out as
# reasonable when there's no exit-code alternative -- a normal
# interactive boot never exits by design.

set -uo pipefail

DISK="${1:?usage: $0 <disk-image>}"
BOOT_TIMEOUT="${TEST_TIMEOUT:-20}"

SERIAL_LOG=$(mktemp)
INT_LOG=$(mktemp)
trap 'rm -f "$SERIAL_LOG" "$INT_LOG"' EXIT

timeout "$BOOT_TIMEOUT" qemu-system-i386 \
    -drive "file=$DISK,if=ide,index=0,format=raw" \
    -boot c \
    -serial "file:$SERIAL_LOG" \
    -display none \
    -d int -D "$INT_LOG" \
    -no-reboot -monitor none

FAILED=0

check_line() {
    local pattern="$1" name="$2"
    if grep -qF "$pattern" "$SERIAL_LOG"; then
        echo "PASS: $name"
    else
        echo "FAIL: $name"
        FAILED=1
    fi
}

check_line "pCern" "kernel started"
check_line "spawned ring-3 task 'nameservice'" "nameservice loaded from the FAT32 disk"
check_line "spawned ring-3 task 'console_server'" "console_server loaded from the FAT32 disk"
check_line "spawned ring-3 task 'storage_ata'" "storage_ata loaded from the FAT32 disk"
check_line "spawned ring-3 task 'fs_fat32'" "fs_fat32 loaded from the FAT32 disk"
check_line "spawned ring-3 task 'shell'" "shell loaded from the FAT32 disk"
check_line "handing off to the scheduler" "reached scheduler handoff"

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
