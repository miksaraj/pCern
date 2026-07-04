use core::arch::global_asm;
use core::mem::size_of;

global_asm!(include_str!("gdt_asm.s"));

extern "C" {
    fn gdt_flush(ptr: *const GdtPointer);
    fn tss_flush();
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

/// Every port gets a real entry in the TSS I/O bitmap -- the full
/// architectural max of 8192 bytes (all 65536 ports). Checkpoint D
/// through Checkpoint I only ever needed a handful of low legacy ports
/// (keyboard, VGA CRTC, the ATA/IDE bus), so a much smaller backed
/// region covering just those, with the CPU's automatic "missing byte
/// reads as all-1s/blocked" behavior handling everything past it, used
/// to be enough. Checkpoint W's PCI-attached NIC breaks that: its I/O
/// BAR is assigned by firmware at boot to whatever address the platform
/// picks, discovered at runtime (see pci.rs) rather than known ahead of
/// time -- nothing here can assume it lands inside any particular
/// smaller window. The full bitmap costs a fixed ~8 KiB of `.bss`,
/// trivial next to this kernel's other static allocations.
const IO_BITMAP_PORTS: usize = 65536;
const IO_BITMAP_BYTES: usize = IO_BITMAP_PORTS / 8 + 1; // +1 trailing all-1s byte

/// A 32-bit TSS. This kernel never uses hardware task-switching -- the
/// fields that matter are `ss0`/`esp0` (which the CPU consults on any
/// ring3->ring0 transition to find the kernel stack to switch to,
/// refreshed every task switch by `set_kernel_stack`) and `io_bitmap`
/// (refreshed every switch by `set_io_permissions`, gating which ports a
/// ring-3 task can `in`/`out` directly without a #GP).
#[repr(C, packed)]
struct Tss {
    prev_task: u32,
    esp0: u32,
    ss0: u32,
    esp1: u32,
    ss1: u32,
    esp2: u32,
    ss2: u32,
    cr3: u32,
    eip: u32,
    eflags: u32,
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esp: u32,
    ebp: u32,
    esi: u32,
    edi: u32,
    es: u32,
    cs: u32,
    ss: u32,
    ds: u32,
    fs: u32,
    gs: u32,
    ldt: u32,
    trap: u16,
    iomap_base: u16,
    io_bitmap: [u8; IO_BITMAP_BYTES],
}

impl Tss {
    /// Every port denied by default (`io_bitmap` all 1s) -- ordinary tasks
    /// never get anything else, and driver tasks only get specific bits
    /// cleared via `set_io_permissions` when they're the one running.
    const fn initial() -> Self {
        Tss {
            prev_task: 0,
            esp0: 0,
            ss0: 0,
            esp1: 0,
            ss1: 0,
            esp2: 0,
            ss2: 0,
            cr3: 0,
            eip: 0,
            eflags: 0,
            eax: 0,
            ecx: 0,
            edx: 0,
            ebx: 0,
            esp: 0,
            ebp: 0,
            esi: 0,
            edi: 0,
            es: 0,
            cs: 0,
            ss: 0,
            ds: 0,
            fs: 0,
            gs: 0,
            ldt: 0,
            trap: 0,
            iomap_base: 0,
            io_bitmap: [0xFF; IO_BITMAP_BYTES],
        }
    }
}

pub const CODE_SEG: u16 = 0x08;
pub const DATA_SEG: u16 = 0x10;
/// RPL 3 already folded in -- these are the literal selector values loaded
/// into cs/ss (etc.) when entering ring 3. The actual loads happen directly
/// in enter_ring3 (task_asm.s, as 0x1B/0x23) since that's plain assembly and
/// can't reference these; kept here so the GDT layout is documented in one
/// place and to catch layout drift if the entries above ever move.
#[allow(dead_code)]
pub const USER_CODE_SEG: u16 = 0x18 | 3;
#[allow(dead_code)]
pub const USER_DATA_SEG: u16 = 0x20 | 3;
// The TSS descriptor's selector (0x28) is hardcoded directly in tss_flush
// (gdt_asm.s), which is the only place that needs it.

const ACCESS_CODE: u8 = 0x9A; // present, ring0, executable, readable
const ACCESS_DATA: u8 = 0x92; // present, ring0, writable
const ACCESS_USER_CODE: u8 = 0xFA; // present, ring3, executable, readable
const ACCESS_USER_DATA: u8 = 0xF2; // present, ring3, writable
const ACCESS_TSS: u8 = 0x89; // present, ring0, 32-bit TSS (available)
const FLAGS_32BIT_4K: u8 = 0xC; // 32-bit segment, 4 KiB granularity

const GDT_ENTRIES: usize = 6;

static mut GDT: [GdtEntry; GDT_ENTRIES] = [
    GdtEntry::new(0, 0, 0, 0),
    GdtEntry::new(0, 0xFFFFF, ACCESS_CODE, FLAGS_32BIT_4K),
    GdtEntry::new(0, 0xFFFFF, ACCESS_DATA, FLAGS_32BIT_4K),
    GdtEntry::new(0, 0xFFFFF, ACCESS_USER_CODE, FLAGS_32BIT_4K),
    GdtEntry::new(0, 0xFFFFF, ACCESS_USER_DATA, FLAGS_32BIT_4K),
    GdtEntry::new(0, 0, 0, 0), // TSS descriptor, filled in at init() (base = runtime address)
];

static mut TSS: Tss = Tss::initial();

extern "C" {
    /// The boot stack set up in boot.s, still valid for as long as the
    /// kernel runs. Used as a placeholder `esp0` until the scheduler's
    /// first `set_kernel_stack` call.
    static stack_top: u8;
}

pub fn init() {
    unsafe {
        let tss_base = core::ptr::addr_of!(TSS) as u32;
        let tss_limit = (size_of::<Tss>() - 1) as u32;
        GDT[5] = GdtEntry::new(tss_base, tss_limit, ACCESS_TSS, 0);
        TSS.ss0 = DATA_SEG as u32;
        // Never leave esp0 at 0: a ring3->ring0 transition before the
        // scheduler's first activate() call would otherwise load esp0=0 as
        // the kernel stack pointer, corrupting memory at address 0 instead
        // of failing in a diagnosable way. Not reachable today (no ring-3
        // code runs before scheduler::start()), but costs nothing to guard.
        TSS.esp0 = core::ptr::addr_of!(stack_top) as u32;
        // Always points at io_bitmap (which starts all-1s/blocked, see
        // Tss::initial): the bitmap itself, not this offset, is what
        // set_io_permissions toggles per task.
        TSS.iomap_base = core::mem::offset_of!(Tss, io_bitmap) as u16;
    }

    let ptr = GdtPointer {
        limit: (size_of::<[GdtEntry; GDT_ENTRIES]>() - 1) as u16,
        base: core::ptr::addr_of!(GDT) as u32,
    };
    unsafe {
        gdt_flush(&ptr);
        tss_flush();
    }
}

/// Points the TSS at the given ring0 stack, used the next time a
/// ring3->ring0 transition (interrupt, exception, or syscall) occurs.
/// Called by the scheduler on every task switch.
pub fn set_kernel_stack(esp0: u32) {
    unsafe { TSS.esp0 = esp0 };
}

/// How many leading bytes of `TSS.io_bitmap` could currently contain a
/// cleared (allowed) bit, tracked across calls to `set_io_permissions` --
/// see that function's own doc comment for why.
static mut DIRTY_BITMAP_BYTES: usize = 0;

/// Rebuilds the I/O permission bitmap to allow exactly `ports` (each must
/// be < IO_BITMAP_PORTS to take effect) and deny everything else. Called
/// by the scheduler on every task switch with the incoming task's
/// `allowed_ports` -- empty for ordinary tasks, which correctly just
/// resets to "everything denied".
///
/// Only resets the *union* of "bytes this call's own ports could set a
/// bit in" and "bytes the previous call could have left a bit cleared
/// in" (`DIRTY_BITMAP_BYTES`), not the full `IO_BITMAP_BYTES` every time:
/// with the bitmap now covering the full 65536-port range (see
/// IO_BITMAP_PORTS's own doc comment) but only one task (the NIC driver)
/// ever actually using a high port, resetting all ~8 KiB on every single
/// task switch -- the common case being ordinary tasks with few or no
/// allowed ports -- would burn far more of this hot path than the actual
/// port grants ever need. Correctness argument: after this call returns,
/// the only bytes that can contain a cleared bit are those covered by
/// `ports`, since every byte in `[0, reset_bytes)` was just reset to all-1
/// before any of `ports`' bits were cleared, and everything beyond
/// `reset_bytes` was already all-1 by the same argument applied to the
/// previous call (inductively, starting from `Tss::initial`'s all-1
/// array) -- so tracking just this call's own dirtied range as the next
/// call's `DIRTY_BITMAP_BYTES` is enough to preserve that invariant.
pub fn set_io_permissions(ports: &[u16]) {
    unsafe {
        let bitmap = core::ptr::addr_of_mut!(TSS.io_bitmap) as *mut u8;
        let needed_bytes = ports
            .iter()
            .filter(|&&port| (port as usize) < IO_BITMAP_PORTS)
            .map(|&port| port as usize / 8 + 1)
            .max()
            .unwrap_or(0);
        let reset_bytes = DIRTY_BITMAP_BYTES.max(needed_bytes).min(IO_BITMAP_BYTES);
        for i in 0..reset_bytes {
            bitmap.add(i).write(0xFF);
        }
        for &port in ports {
            let port = port as usize;
            if port < IO_BITMAP_PORTS {
                let byte = bitmap.add(port / 8);
                byte.write(byte.read() & !(1 << (port % 8)));
            }
        }
        DIRTY_BITMAP_BYTES = needed_bytes;
    }
}
