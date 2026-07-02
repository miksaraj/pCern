//! Checkpoint D: the real console server. Owns the ANSI parser and VGA
//! writer that used to live in the kernel (src/ansi.rs, src/vga.rs) and the
//! keyboard scancode decoding that used to live in src/keyboard.rs -- the
//! kernel now only acks IRQ1 and forwards the raw scancode here (see
//! src/keyboard.rs), and other tasks reach the screen only by sending this
//! task one byte at a time (`OP_PUTCHAR`) rather than through a
//! kernel-mediated debug_write.
//!
//! Checkpoint E: addressing moved from raw task ids to capability slots
//! (see cap.rs's CSpace in the kernel).
//!
//! Checkpoint G: VGA/keyboard access are capability-mediated now instead
//! of the old is_driver bool + hardcoded MMIO allowlist -- main.rs grants
//! this task a MemoryGrant and an IrqControl at spawn.
//!
//! Checkpoint H: registers itself as "console" with the name service
//! (CSlot 1, auto-granted to every task -- see loader.rs in the kernel)
//! so any client can find it by name instead of main.rs pre-wiring a
//! capability to it by hand.
//!
//! Checkpoint L: gains a shared-memory buffered line-input protocol
//! (see libpcern's CONSOLE_OP_*/console_connect/console_read_line, and
//! that module's doc comment for the wire protocol and its one
//! documented hazard) mirroring storage_ata's SET_BUFFER/SET_REPLY
//! shape. Every keystroke is still echoed to the screen unconditionally;
//! additionally, once a reader has connected and armed a read via
//! `CONSOLE_OP_READ_LINE`, bytes are also accumulated into the reader's
//! shared page until Enter completes the line, at which point this task
//! replies with the line length and disarms until the next
//! `CONSOLE_OP_READ_LINE`. Backspace is tracked against this
//! accumulator's own length (bounded at 0), independently of vga.rs's
//! unrelated screen-cursor backspace handling; bytes typed once the
//! accumulator hits `CONSOLE_LINE_MAX` are dropped (not buffered), not an
//! error.

#![no_std]
#![no_main]

mod ansi;
mod keyboard;
mod port;
mod vga;

use core::panic::PanicInfo;

/// The arbitrary (but page-aligned, and clear of this task's own code/
/// stack range -- see loader.rs's USER_CODE_BASE/USER_STACK_TOP in the
/// kernel) virtual address this task asks the kernel to map the VGA
/// buffer to via `map_memory`.
const VGA_BUFFER_VIRT: u32 = 0x0090_0000;

/// Where a connected reader's shared input-buffer page gets mapped in
/// *this* task's own address space -- independent of whatever virtual
/// address the reader chose in its own space, since they're separate
/// page directories (same reasoning as storage_ata's BUF_VIRT).
const INPUT_BUF_VIRT: u32 = 0x0080_0000;

/// This task's own inbox endpoint -- see the module doc comment. CSlot 1
/// (the name service) is auto-granted, not listed here.
const MY_INBOX_SLOT: u32 = 2;
const VGA_GRANT_SLOT: u32 = 3;
const IRQ_CONTROL_SLOT: u32 = 4;

/// Protocol other tasks use to reach the screen: `send(CONSOLE_SLOT,
/// OP_PUTCHAR, byte, 0)`, one call per character.
const OP_PUTCHAR: u32 = 0;

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    if libpcern::map_memory(VGA_GRANT_SLOT, VGA_BUFFER_VIRT) != 0 {
        libpcern::exit(1);
    }
    libpcern::register_irq(IRQ_CONTROL_SLOT);
    libpcern::register_name(b"console", MY_INBOX_SLOT);

    let mut writer = vga::Writer::new(VGA_BUFFER_VIRT as *mut u16);
    writer.clear_screen();
    let mut ansi = ansi::AnsiState::new();
    let mut decoder = keyboard::Decoder::new();

    // Checkpoint L's line-input state. `reader_slot` is a capability slot
    // (0 = none connected yet) rather than a raw task id, same addressing
    // convention as everything else here.
    let mut input_buf_mapped = false;
    let mut reader_slot: u32 = 0;
    let mut armed = false;
    let mut line_len: usize = 0;

    loop {
        let r = libpcern::recv(MY_INBOX_SLOT);
        if r.sender == libpcern::KERNEL_TASK_ID {
            let scancode = r.w1 as u8;
            if let Some(ascii) = decoder.feed(scancode) {
                ansi.feed(ascii, &mut writer);
                writer.sync_hardware_cursor();

                if armed {
                    if ascii == 8 {
                        // Backspace: trim our own accumulator, independent
                        // of vga.rs's screen-cursor backspace above.
                        if line_len > 0 {
                            line_len -= 1;
                        }
                    } else if ascii == b'\n' {
                        armed = false;
                        if reader_slot != 0 {
                            libpcern::send(reader_slot, line_len as u32, 0, 0, 0);
                        }
                        line_len = 0;
                    } else if input_buf_mapped && line_len < libpcern::CONSOLE_LINE_MAX {
                        unsafe {
                            let buf = core::slice::from_raw_parts_mut(
                                INPUT_BUF_VIRT as *mut u8,
                                libpcern::CONSOLE_LINE_MAX,
                            );
                            buf[line_len] = ascii;
                        }
                        line_len += 1;
                    }
                    // else: buffer full -- already echoed to the screen
                    // above, just not accumulated (drop excess, don't
                    // overflow).
                }
            }
        } else {
            match r.w0 {
                OP_PUTCHAR => {
                    ansi.feed(r.w1 as u8, &mut writer);
                    writer.sync_hardware_cursor();
                }
                libpcern::CONSOLE_OP_SET_BUFFER => {
                    if r.transferred_slot != 0 && libpcern::map_memory(r.transferred_slot, INPUT_BUF_VIRT) == 0 {
                        input_buf_mapped = true;
                    }
                }
                libpcern::CONSOLE_OP_SET_READER => {
                    if r.transferred_slot != 0 {
                        reader_slot = r.transferred_slot;
                    }
                }
                libpcern::CONSOLE_OP_READ_LINE => {
                    if input_buf_mapped && reader_slot != 0 {
                        armed = true;
                        line_len = 0;
                    }
                }
                _ => {}
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
