use crate::idt::InterruptStackFrame;
use crate::println;

fn halt_loop() -> ! {
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}

pub extern "x86-interrupt" fn divide_by_zero(_frame: InterruptStackFrame) {
    println!("\x1b[1;31mEXCEPTION: divide by zero\x1b[0m");
    halt_loop();
}

pub extern "x86-interrupt" fn breakpoint(_frame: InterruptStackFrame) {
    println!("\x1b[1;33mEXCEPTION: breakpoint\x1b[0m");
}

pub extern "x86-interrupt" fn invalid_opcode(_frame: InterruptStackFrame) {
    println!("\x1b[1;31mEXCEPTION: invalid opcode\x1b[0m");
    halt_loop();
}

pub extern "x86-interrupt" fn double_fault(_frame: InterruptStackFrame, error_code: u32) -> ! {
    println!("\x1b[1;31mEXCEPTION: double fault (error={:#x})\x1b[0m", error_code);
    halt_loop();
}

pub extern "x86-interrupt" fn general_protection_fault(_frame: InterruptStackFrame, error_code: u32) {
    println!(
        "\x1b[1;31mEXCEPTION: general protection fault (error={:#x})\x1b[0m",
        error_code
    );
    halt_loop();
}

pub extern "x86-interrupt" fn page_fault(_frame: InterruptStackFrame, error_code: u32) {
    println!("\x1b[1;31mEXCEPTION: page fault (error={:#x})\x1b[0m", error_code);
    halt_loop();
}
