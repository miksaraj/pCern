use core::arch::global_asm;
use core::mem::size_of;

global_asm!(include_str!("gdt_asm.s"));

extern "C" {
    fn gdt_flush(ptr: *const GdtPointer);
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

impl GdtEntry {
    const fn new(base: u32, limit: u32, access: u8, flags: u8) -> Self {
        GdtEntry {
            limit_low: (limit & 0xFFFF) as u16,
            base_low: (base & 0xFFFF) as u16,
            base_mid: ((base >> 16) & 0xFF) as u8,
            access,
            granularity: ((flags & 0x0F) << 4) | (((limit >> 16) & 0x0F) as u8),
            base_high: ((base >> 24) & 0xFF) as u8,
        }
    }
}

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u32,
}

pub const CODE_SEG: u16 = 0x08;
#[allow(dead_code)]
pub const DATA_SEG: u16 = 0x10;

const ACCESS_CODE: u8 = 0x9A; // present, ring0, executable, readable
const ACCESS_DATA: u8 = 0x92; // present, ring0, writable
const FLAGS_32BIT_4K: u8 = 0xC; // 32-bit segment, 4 KiB granularity

const GDT_ENTRIES: usize = 3;

static mut GDT: [GdtEntry; GDT_ENTRIES] = [
    GdtEntry::new(0, 0, 0, 0),
    GdtEntry::new(0, 0xFFFFF, ACCESS_CODE, FLAGS_32BIT_4K),
    GdtEntry::new(0, 0xFFFFF, ACCESS_DATA, FLAGS_32BIT_4K),
];

pub fn init() {
    let ptr = GdtPointer {
        limit: (size_of::<[GdtEntry; GDT_ENTRIES]>() - 1) as u16,
        base: core::ptr::addr_of!(GDT) as u32,
    };
    unsafe { gdt_flush(&ptr) };
}
