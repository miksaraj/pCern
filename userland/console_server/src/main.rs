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
//! shape. Every keystroke is still echoed to the screen unconditionally.
//! Bytes are accumulated into the reader's shared page as soon as a
//! buffer is connected -- independent of whether a `CONSOLE_OP_READ_LINE`
//! request is currently outstanding, so a client that's busy (e.g. a
//! shell mid-command) doesn't silently lose keystrokes typed ahead of its
//! next request. If Enter completes a line before the client has asked
//! for one, the completed line just waits (`line_ready`) for the next
//! `CONSOLE_OP_READ_LINE`, which is answered immediately in that case;
//! only one completed-but-unclaimed line is kept at a time (further
//! typing is echoed but not accumulated until it's claimed), the same
//! one-result-in-flight scope `storage_ata`/`fs_fat32` already have.
//! Backspace is tracked against this accumulator's own length (bounded at
//! 0), independently of vga.rs's unrelated screen-cursor backspace
//! handling; bytes typed once the accumulator hits `CONSOLE_LINE_MAX` are
//! dropped (not buffered), not an error. A `CONSOLE_OP_READ_LINE` that
//! arrives before a buffer was ever successfully connected (e.g. a
//! rejected/invalid grant) is answered immediately with length `0`
//! rather than left to hang the caller forever with no buffer to satisfy
//! it.
//!
//! The three `CONSOLE_OP_*` ops above are only honored from the task
//! that first successfully establishes a reader connection: the first
//! `CONSOLE_OP_SET_BUFFER`/`SET_READER` received (from whichever task
//! sends it first) latches that sender's kernel-attested task id
//! (`r.sender` -- unforgeable, the same value `nameservice`'s own
//! registration allowlist trusts) as `reader_owner`, and every
//! subsequent message on these three ops is silently ignored unless it
//! comes from that same task id. Without this, any task -- including
//! one spawned through the shell's own `run` command with no privilege
//! beyond the universal name-service auto-grant, since `console`
//! lookups are open to any caller and `SYS_MEM_ALLOC`/
//! `SYS_ENDPOINT_CREATE`/`SYS_SEND` need no capability at all -- could
//! silently re-point `reader_slot`/the mapped buffer at itself and
//! receive every keystroke typed afterward (a confidentiality break, not
//! just disruption of the legitimate reader). `reader_owner` is latched
//! for the rest of this boot, the same permanent-single-client scope
//! `storage_ata`/`fs_fat32` already have -- there's no handoff/release
//! operation, since only one task (the shell) is ever expected to hold
//! this role for the phase this project is at.

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
/// page directories. Deliberately *not* `loader.rs`'s `USER_STACK_TOP`
/// (0x0080_0000, the address storage_ata's/fs_fat32's own buffers happen
/// to reuse): mapping a page there would sit exactly at this task's own
/// stack-overflow guard boundary, in this task's own address space,
/// turning a would-be clean page fault on overflow into silent
/// corruption of the connected reader's buffer instead.
const INPUT_BUF_VIRT: u32 = 0x00A0_0000;

/// This task's own inbox endpoint -- see the module doc comment. CSlot 1
/// (the name service) is auto-granted, not listed here.
const MY_INBOX_SLOT: u32 = 2;
const VGA_GRANT_SLOT: u32 = 3;
const IRQ_CONTROL_SLOT: u32 = 4;

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
    // convention as everything else here. `armed` = a CONSOLE_OP_READ_LINE
    // request is outstanding, waiting for a line to complete. `line_ready`
    // = a line already completed (Enter was pressed) before the client
    // asked for one, and is waiting in the buffer to be claimed -- the two
    // are never true at the same time (see the keystroke handling below).
    // `reader_owner` (0 = none yet) is the kernel-attested task id that
    // first connected -- see the module doc comment for why every
    // CONSOLE_OP_* connection message is rejected from any other sender.
    let mut input_buf_mapped = false;
    let mut reader_slot: u32 = 0;
    let mut reader_owner: u32 = 0;
    let mut armed = false;
    let mut line_ready = false;
    let mut line_len: usize = 0;

    // Phase 7, Checkpoint R: raw single-keystroke mode for a full-screen
    // editor, layered onto the exact same reader connection/ownership
    // latch above rather than a second one -- see the module doc comment.
    // `raw_mode` off (the default) leaves every line-mode behavior above
    // completely unchanged. `key_armed` mirrors `armed`'s role but for
    // `CONSOLE_OP_READ_KEY`. `key_queue` mirrors `line_ready`'s role too --
    // a key decoded before the reader's next `CONSOLE_OP_READ_KEY` request
    // is held rather than silently dropped, the same reasoning line
    // mode's typed-ahead accumulation already documents. This one needs a
    // real queue rather than line mode's single `line_ready` slot: a raw
    // reader's redraw cost scales with how much has been typed so far
    // (the whole buffer is reprinted after every key -- see
    // libpcern::editor::Editor::redraw), so several keystrokes typed in
    // the time one redraw takes are an expected case, not a rare race, and
    // a single-slot buffer would overwrite (silently drop) all but the
    // most recent of them. 32 entries is comfortably more than a human
    // can type between two IPC round trips even under this project's
    // slowest realistic redraw.
    const KEY_QUEUE_CAP: usize = 32;
    let mut raw_mode = false;
    let mut key_armed = false;
    let mut key_queue: [u32; KEY_QUEUE_CAP] = [0; KEY_QUEUE_CAP];
    let mut key_queue_head: usize = 0;
    let mut key_queue_len: usize = 0;

    loop {
        let r = libpcern::recv(MY_INBOX_SLOT);
        if r.sender == libpcern::KERNEL_TASK_ID {
            let scancode = r.w1 as u8;
            if let Some(key) = decoder.feed(scancode) {
                if raw_mode {
                    // No echo, no line accumulation -- a raw-mode client
                    // (the editor) owns the screen and redraws itself via
                    // ansi.rs's cursor-addressing escapes.
                    if key_armed && reader_slot != 0 {
                        libpcern::send(reader_slot, key, 0, 0, 0);
                        key_armed = false;
                    } else if key_queue_len < KEY_QUEUE_CAP {
                        let idx = (key_queue_head + key_queue_len) % KEY_QUEUE_CAP;
                        key_queue[idx] = key;
                        key_queue_len += 1;
                    }
                    // else: queue full (32 unclaimed keystrokes) -- drop,
                    // same "don't buffer forever" bound as
                    // CONSOLE_LINE_MAX has for line mode.
                    continue;
                }
                // Line mode only ever dealt in plain ASCII -- codes >= 256
                // (arrows etc.) have no ASCII form and are silently
                // dropped here, the same as they always were before this
                // checkpoint (previously undecoded scancodes returned
                // `None` and never reached this point at all).
                if key >= 256 {
                    continue;
                }
                let ascii = key as u8;
                ansi.feed(ascii, &mut writer);
                writer.sync_hardware_cursor();

                // Accumulate whenever a buffer is connected and there
                // isn't already a completed, unclaimed line sitting in
                // it -- deliberately *not* gated on `armed`, so keystrokes
                // typed before the client's next CONSOLE_OP_READ_LINE
                // request (e.g. while a shell is still busy with its
                // previous command) are still captured instead of only
                // being echoed and dropped.
                if input_buf_mapped && !line_ready {
                    if ascii == 8 {
                        // Backspace: trim our own accumulator, independent
                        // of vga.rs's screen-cursor backspace above.
                        if line_len > 0 {
                            line_len -= 1;
                        }
                    } else if ascii == b'\n' {
                        if armed && reader_slot != 0 {
                            libpcern::send(reader_slot, line_len as u32, 0, 0, 0);
                            armed = false;
                            line_len = 0;
                        } else {
                            // No outstanding request yet -- hold the
                            // completed line for the next READ_LINE
                            // rather than discarding it.
                            line_ready = true;
                        }
                    } else if line_len < libpcern::CONSOLE_LINE_MAX {
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
                libpcern::OP_PUTCHAR => {
                    ansi.feed(r.w1 as u8, &mut writer);
                    writer.sync_hardware_cursor();
                }
                libpcern::CONSOLE_OP_SET_BUFFER => {
                    // Latches `reader_owner` on the first sender to
                    // successfully provide a valid buffer grant; any
                    // other sender's SET_BUFFER is ignored from then on
                    // -- see the module doc comment for why.
                    if (reader_owner == 0 || reader_owner == r.sender)
                        && r.transferred_slot != 0
                        && libpcern::map_memory(r.transferred_slot, INPUT_BUF_VIRT) == 0
                    {
                        reader_owner = r.sender;
                        input_buf_mapped = true;
                    }
                }
                libpcern::CONSOLE_OP_SET_READER => {
                    if reader_owner == r.sender && r.transferred_slot != 0 {
                        reader_slot = r.transferred_slot;
                    }
                }
                libpcern::CONSOLE_OP_READ_LINE => {
                    if reader_owner == r.sender && reader_slot != 0 {
                        if !input_buf_mapped {
                            // No working buffer was ever connected (e.g.
                            // an invalid/rejected grant) -- reply right
                            // away rather than leaving the caller blocked
                            // in recv() forever with nothing that could
                            // ever satisfy it.
                            libpcern::send(reader_slot, 0, 0, 0, 0);
                        } else if line_ready {
                            libpcern::send(reader_slot, line_len as u32, 0, 0, 0);
                            line_ready = false;
                            line_len = 0;
                        } else {
                            armed = true;
                        }
                    }
                }
                libpcern::CONSOLE_OP_SET_MODE => {
                    // Same ownership check as every other CONSOLE_OP_* --
                    // only the latched reader can flip modes.
                    if reader_owner == r.sender {
                        raw_mode = r.w1 != 0;
                    }
                }
                libpcern::CONSOLE_OP_READ_KEY => {
                    if reader_owner == r.sender && reader_slot != 0 {
                        if key_queue_len > 0 {
                            let key = key_queue[key_queue_head];
                            key_queue_head = (key_queue_head + 1) % KEY_QUEUE_CAP;
                            key_queue_len -= 1;
                            libpcern::send(reader_slot, key, 0, 0, 0);
                        } else {
                            key_armed = true;
                        }
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
