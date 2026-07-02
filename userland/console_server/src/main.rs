//! Checkpoint D: the real console server. Owns the ANSI parser and VGA
//! writer that used to live in the kernel (src/ansi.rs, src/vga.rs) and the
//! keyboard scancode decoding that used to live in src/keyboard.rs -- the
//! kernel now only acks IRQ1 and forwards the raw scancode here (see
//! src/keyboard.rs), and other tasks reach the screen only by sending this
//! task one byte at a time (`OP_PUTCHAR`) rather than through a
//! kernel-mediated debug_write.
//!
//! Checkpoint E: addressing moved from raw task ids to capability slots
//! (see cap.rs's CSpace in the kernel). There's no name service yet (that's
//! Checkpoint H), so main.rs wires every task's capabilities by hand right
//! after spawning, following one fixed convention every userland program
//! shares: CSlot 1 is always "my own inbox" endpoint, used for both
//! `register_irq` and `recv`.

#![no_std]
#![no_main]

mod ansi;
mod keyboard;
mod port;
mod vga;

use core::panic::PanicInfo;

/// Physical VGA text buffer, and the arbitrary (but page-aligned, and clear
/// of this task's own code/stack range -- see loader.rs's USER_CODE_BASE/
/// USER_STACK_TOP in the kernel) virtual address this task asks the kernel
/// to map it to via `map_memory`.
const VGA_BUFFER_PHYS: u32 = 0xB8000;
const VGA_BUFFER_VIRT: u32 = 0x0090_0000;
const VGA_BUFFER_LEN: u32 = 0x1000;

const IRQ_KEYBOARD: u32 = 1;

/// This task's own inbox endpoint -- see the module doc comment.
const MY_INBOX_SLOT: u32 = 1;

/// Protocol other tasks use to reach the screen: `send(CONSOLE_SLOT,
/// OP_PUTCHAR, byte, 0)`, one call per character.
const OP_PUTCHAR: u32 = 0;

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    if libpcern::map_memory(VGA_BUFFER_PHYS, VGA_BUFFER_VIRT, VGA_BUFFER_LEN) != 0 {
        libpcern::exit(1);
    }
    libpcern::register_irq(IRQ_KEYBOARD, MY_INBOX_SLOT);

    let mut writer = vga::Writer::new(VGA_BUFFER_VIRT as *mut u16);
    writer.clear_screen();
    let mut ansi = ansi::AnsiState::new();
    let mut decoder = keyboard::Decoder::new();

    loop {
        let r = libpcern::recv(MY_INBOX_SLOT);
        if r.sender == libpcern::KERNEL_TASK_ID {
            let scancode = r.w1 as u8;
            if let Some(ascii) = decoder.feed(scancode) {
                ansi.feed(ascii, &mut writer);
                writer.sync_hardware_cursor();
            }
        } else if r.w0 == OP_PUTCHAR {
            ansi.feed(r.w1 as u8, &mut writer);
            writer.sync_hardware_cursor();
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
