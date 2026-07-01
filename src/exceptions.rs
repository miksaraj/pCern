use crate::idt::InterruptStackFrame;
use crate::println;
use crate::scheduler;

fn halt_loop() -> ! {
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}

/// A fault from ring-3 code shouldn't take the whole machine down with it:
/// this kills just the offending task instead, falling back to a full halt
/// for a ring-0 (kernel) fault -- which really is a kernel bug, not
/// something to recover from -- or if there's no scheduled task to kill
/// (e.g. a fault during early boot, before any task exists).
fn recover_or_halt(frame: &InterruptStackFrame) -> ! {
    if frame.cs & 3 == 3 && scheduler::current_id().is_some() {
        scheduler::exit_current(-1);
    }
    halt_loop();
}

pub extern "x86-interrupt" fn divide_by_zero(frame: InterruptStackFrame) {
    println!("\x1b[1;31mEXCEPTION: divide by zero\x1b[0m");
    recover_or_halt(&frame);
}

pub extern "x86-interrupt" fn breakpoint(_frame: InterruptStackFrame) {
    println!("\x1b[1;33mEXCEPTION: breakpoint\x1b[0m");
}

pub extern "x86-interrupt" fn invalid_opcode(frame: InterruptStackFrame) {
    println!("\x1b[1;31mEXCEPTION: invalid opcode\x1b[0m");
    recover_or_halt(&frame);
}

pub extern "x86-interrupt" fn double_fault(_frame: InterruptStackFrame, error_code: u32) -> ! {
    println!("\x1b[1;31mEXCEPTION: double fault (error={:#x})\x1b[0m", error_code);
    halt_loop();
}

pub extern "x86-interrupt" fn general_protection_fault(frame: InterruptStackFrame, error_code: u32) {
    println!(
        "\x1b[1;31mEXCEPTION: general protection fault (error={:#x})\x1b[0m",
        error_code
    );
    recover_or_halt(&frame);
}

pub extern "x86-interrupt" fn page_fault(frame: InterruptStackFrame, error_code: u32) {
    println!("\x1b[1;31mEXCEPTION: page fault (error={:#x})\x1b[0m", error_code);
    recover_or_halt(&frame);
}
