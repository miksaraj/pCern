# console_server

Owns the keyboard and the VGA text console. Every other task's console
output -- including the kernel's own boot log, still printed via `serial`
directly rather than through this server -- reaches the screen only by
sending `console_server` one byte at a time; there is no shared/mapped
console buffer any client writes into directly.

## Capabilities it needs

Granted by `main.rs` at spawn (not discoverable by name -- there's no
service to look these up from before this one exists):

| CSlot | What |
|-------|------|
| 1     | Name service (auto-granted to every task) |
| 2     | Its own inbox |
| 3     | A `MemoryGrant` for the VGA text buffer (`0xB8000`, 4 KiB) |
| 4     | An `IrqControl` for IRQ 1 (keyboard) |

It maps the VGA grant and registers for the keyboard IRQ itself at
startup, the same way any userland driver would.

## Protocol

Registers as `"console"` with the name service. Clients look that up, then
send:

```
send(console_slot, w0 = OP_PUTCHAR(0), w1 = byte, w2 = 0, transfer = none)
```

one call per character. There's no batching and no reply -- `OP_PUTCHAR`
is fire-and-forget. A client that wants confirmation output actually
reached the screen has no way to get one from this protocol; none of this
project's clients have needed it so far.

Bytes are fed through a small ANSI/VT100 parser (`ansi.rs`, moved here
unchanged from the kernel) supporting cursor movement (CUU/CUD/CUF/CUB/
CUP), erase (ED/EL), and SGR (colors + bold) -- enough for a colored shell
prompt or a simple full-screen terminal program, not a complete terminal
emulator.

Keyboard input arrives via the interrupt the kernel forwards for IRQ 1
(through the `IrqControl` capability above); scancodes are decoded here
(`keyboard.rs`) the same way they used to be decoded in the kernel before
Checkpoint D moved this whole responsibility out of ring 0.

## Why this is a userspace task at all

Before Checkpoint D, the kernel itself decoded scancodes and wrote
directly to VGA memory, and a privileged `sys_debug_write` syscall let any
ring-3 task print by handing the kernel a pointer to write to serial --
which also meant any ring-3 task could make the kernel read arbitrary
memory. Moving the console fully into userspace and retiring
`sys_debug_write` closed that off: the kernel no longer parses untrusted
pointers on behalf of a syscall whose only job was printing.
