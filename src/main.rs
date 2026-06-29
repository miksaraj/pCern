#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

use core::arch::global_asm;
use core::panic::PanicInfo;

global_asm!(include_str!("boot.s"));

mod ansi;
mod exceptions;
mod gdt;
mod idt;
mod keyboard;
mod pic;
mod port;
mod sync;
mod timer;
mod vga;

const MULTIBOOT_MAGIC: u32 = 0x2BADB002;

#[no_mangle]
pub extern "C" fn kernel_main(magic: u32, _multiboot_info: u32) -> ! {
    vga::WRITER.lock().clear_screen();

    println!("\x1b[1;36mpCern\x1b[0m nanokernel starting...");

    if magic != MULTIBOOT_MAGIC {
        println!("\x1b[1;33mwarning:\x1b[0m unexpected multiboot magic {:#010x}", magic);
    }

    gdt::init();
    println!("[ \x1b[1;32mok\x1b[0m ] GDT installed");

    idt::init();
    println!("[ \x1b[1;32mok\x1b[0m ] IDT installed");

    pic::init();
    println!("[ \x1b[1;32mok\x1b[0m ] PIC remapped, timer + keyboard unmasked");

    unsafe { core::arch::asm!("sti") };
    println!("[ \x1b[1;32mok\x1b[0m ] interrupts enabled");

    println!();
    println!("\x1b[1mType on the keyboard to test interrupt-driven input.\x1b[0m");

    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\x1b[1;41;37m KERNEL PANIC \x1b[0m {}", info);
    loop {
        unsafe { core::arch::asm!("cli", "hlt") };
    }
}
