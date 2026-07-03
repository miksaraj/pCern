#!/usr/bin/env bash
# Boots the reboot-test ISO (see `make test-reboot` / `make iso-reboottest`)
# headlessly in QEMU with `-no-reboot` and waits for it to exit on its own.
#
# Checkpoint V's `reboot_test` fixture (userland/cap_test/src/bin/
# reboot_test.rs) prints a marker to serial, then immediately calls the new
# SYS_REBOOT syscall -- pulsing the 8042 keyboard controller's CPU-reset
# line. There's no exit code to check (the machine resets before the
# fixture could ever call `exit`), so this script's pass signal works the
# other way around: `-no-reboot` makes QEMU quit, rather than actually
# restart, the instant a real reset happens, so a *prompt, unforced* QEMU
# exit is itself the proof the pulse reached real (emulated) hardware --
# distinguished from a hang (this script's own `timeout` killing QEMU,
# exit code 124) by checking QEMU's own exit status.

set -uo pipefail

ISO="${1:?usage: $0 <reboottest-iso>}"
BOOT_TIMEOUT="${TEST_TIMEOUT:-30}"

SERIAL_LOG=$(mktemp)
INT_LOG=$(mktemp)
trap 'rm -f "$SERIAL_LOG" "$INT_LOG"' EXIT

timeout "$BOOT_TIMEOUT" qemu-system-i386 \
    -cdrom "$ISO" \
    -serial "file:$SERIAL_LOG" \
    -display none \
    -d int -D "$INT_LOG" \
    -no-reboot -monitor none
QEMU_EXIT=$?

FAILED=0

if [ "$QEMU_EXIT" -eq 124 ]; then
    echo "FAIL: QEMU never exited -- SYS_REBOOT's reset pulse never landed"
    FAILED=1
else
    echo "PASS: QEMU exited on its own (reset pulse landed, -no-reboot took effect)"
fi

if grep -q "reboot_test: about to reboot" "$SERIAL_LOG"; then
    echo "PASS: reboot_test printed its marker before resetting"
else
    echo "FAIL: reboot_test's marker never reached serial"
    FAILED=1
fi

if grep -q "reboot_test: FAIL" "$SERIAL_LOG"; then
    echo "FAIL: reboot_test reported the reboot syscall was rejected"
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
