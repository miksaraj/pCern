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
/// Same, but DPL=3: lets ring-3 code reach this vector directly via `int`,
/// used only for the syscall gate.
const GATE_INTERRUPT_32_DPL3: u8 = 0xEE;

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::MISSING; IDT_ENTRIES];

fn set_gate(vector: u8, handler_addr: u32, type_attr: u8) {
    unsafe {
        IDT[vector as usize] = IdtEntry::new(handler_addr, gdt::CODE_SEG, type_attr);
    }
}

/// Checkpoint W: generic stubs for every PIC line besides the timer
/// (IRQ0) and keyboard (IRQ1), which already have their own dedicated,
/// device-specific handlers. Unlike those two, a PCI-attached device's
/// IRQ line (the RTL8139's, for instance) is only known once `pci.rs`
/// reads it out of the device's config space at boot -- there's no fixed
/// IRQ number to hardcode a handler function against ahead of time the
/// way `keyboard::handler` hardcodes IRQ1. Registering all fourteen
/// unconditionally, always, is simpler than patching the IDT after the
/// fact once enumeration finds a specific number: an unregistered line
/// firing (nothing ever calls `irq::register` for it) just costs one
/// harmless no-op dispatch (`irq::dispatch` finds no endpoint, still
/// sends the EOI) instead of never being wired up at all.
macro_rules! define_irq_stub {
    ($name:ident, $irq:expr) => {
        extern "x86-interrupt" fn $name(_frame: InterruptStackFrame) {
            crate::irq::dispatch($irq, 0);
        }
    };
}

define_irq_stub!(irq2, 2);
define_irq_stub!(irq3, 3);
define_irq_stub!(irq4, 4);
define_irq_stub!(irq5, 5);
define_irq_stub!(irq6, 6);
define_irq_stub!(irq7, 7);
define_irq_stub!(irq8, 8);
define_irq_stub!(irq9, 9);
define_irq_stub!(irq10, 10);
define_irq_stub!(irq11, 11);
define_irq_stub!(irq12, 12);
define_irq_stub!(irq13, 13);
define_irq_stub!(irq14, 14);
define_irq_stub!(irq15, 15);

pub fn init() {
    use crate::exceptions;
    use crate::keyboard;
    use crate::syscall;
    use crate::timer;

    set_gate(0, exceptions::divide_by_zero as *const () as u32, GATE_INTERRUPT_32);
    set_gate(3, exceptions::breakpoint as *const () as u32, GATE_INTERRUPT_32);
    set_gate(6, exceptions::invalid_opcode as *const () as u32, GATE_INTERRUPT_32);
    set_gate(8, exceptions::double_fault as *const () as u32, GATE_INTERRUPT_32);
    set_gate(13, exceptions::general_protection_fault as *const () as u32, GATE_INTERRUPT_32);
    set_gate(14, exceptions::page_fault as *const () as u32, GATE_INTERRUPT_32);

    set_gate(32, timer::handler as *const () as u32, GATE_INTERRUPT_32); // IRQ0, remapped
    set_gate(33, keyboard::handler as *const () as u32, GATE_INTERRUPT_32); // IRQ1, remapped

    // IRQ2-15, remapped: vector = 32 + irq for both PICs (see pic.rs's
    // PIC1_OFFSET/PIC2_OFFSET -- 32 and 40 respectively, and 40 == 32 + 8
    // where the slave's lines pick up), so index 0 of this array (IRQ2)
    // lands at vector 34, matching its position in the loop below.
    let generic_irqs: [extern "x86-interrupt" fn(InterruptStackFrame); 14] =
        [irq2, irq3, irq4, irq5, irq6, irq7, irq8, irq9, irq10, irq11, irq12, irq13, irq14, irq15];
    for (i, handler) in generic_irqs.iter().enumerate() {
        set_gate(34 + i as u8, *handler as *const () as u32, GATE_INTERRUPT_32);
    }

    set_gate(0x80, syscall::syscall_isr as *const () as u32, GATE_INTERRUPT_32_DPL3);

    let ptr = IdtPointer {
        limit: (size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
        base: core::ptr::addr_of!(IDT) as u32,
    };
    unsafe {
        asm!("lidt [{0}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
}
