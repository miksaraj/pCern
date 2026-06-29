use core::arch::asm;
use core::mem::size_of;

use crate::gdt;

/// The register state the CPU pushes before invoking an interrupt handler.
/// Since this kernel never drops to ring 3, the CPU never performs a stack
/// switch on interrupt entry, so `esp`/`ss` are never pushed and are
/// intentionally absent here.
#[repr(C)]
pub struct InterruptStackFrame {
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    zero: u8,
    type_attr: u8,
    offset_high: u16,
}

impl IdtEntry {
    const MISSING: Self = IdtEntry {
        offset_low: 0,
        selector: 0,
        zero: 0,
        type_attr: 0,
        offset_high: 0,
    };

    fn new(handler_addr: u32, selector: u16, type_attr: u8) -> Self {
        IdtEntry {
            offset_low: (handler_addr & 0xFFFF) as u16,
            selector,
            zero: 0,
            type_attr,
            offset_high: ((handler_addr >> 16) & 0xFFFF) as u16,
        }
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u32,
}

const IDT_ENTRIES: usize = 256;
const GATE_INTERRUPT_32: u8 = 0x8E; // present, ring0, 32-bit interrupt gate

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::MISSING; IDT_ENTRIES];

fn set_gate(vector: u8, handler_addr: u32) {
    unsafe {
        IDT[vector as usize] = IdtEntry::new(handler_addr, gdt::CODE_SEG, GATE_INTERRUPT_32);
    }
}

pub fn init() {
    use crate::exceptions;
    use crate::keyboard;
    use crate::timer;

    set_gate(0, exceptions::divide_by_zero as *const () as u32);
    set_gate(3, exceptions::breakpoint as *const () as u32);
    set_gate(6, exceptions::invalid_opcode as *const () as u32);
    set_gate(8, exceptions::double_fault as *const () as u32);
    set_gate(13, exceptions::general_protection_fault as *const () as u32);
    set_gate(14, exceptions::page_fault as *const () as u32);

    set_gate(32, timer::handler as *const () as u32); // IRQ0, remapped
    set_gate(33, keyboard::handler as *const () as u32); // IRQ1, remapped

    let ptr = IdtPointer {
        limit: (size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
        base: core::ptr::addr_of!(IDT) as u32,
    };
    unsafe {
        asm!("lidt [{0}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
}
