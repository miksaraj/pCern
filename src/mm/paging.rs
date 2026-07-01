use core::arch::asm;

use crate::mm::frame::{self, FRAME_SIZE};

extern "C" {
    static mut boot_page_directory: [u32; 1024];
}

const KERNEL_VMA: usize = 0xC000_0000;
const KERNEL_VMA_PDE_INDEX: usize = KERNEL_VMA >> 22; // 768

pub const PHYS_MAP_BASE: usize = 0xE000_0000;
const PHYS_MAP_PDE_INDEX: usize = PHYS_MAP_BASE >> 22; // 896
const PAGE_PRESENT_RW_4M: u32 = 0x83; // present | read-write | page-size(4M)

const PAGE_PRESENT: u32 = 1 << 0;
const PAGE_WRITABLE: u32 = 1 << 1;
const PAGE_USER: u32 = 1 << 2;
const PAGE_ADDR_MASK: u32 = 0xFFFF_F000;

/// Extends the boot bootstrap page directory with enough 4 MiB PSE entries
/// to linearly map all of physical memory at `PHYS_MAP_BASE`. This gives the
/// kernel a simple way to touch any physical frame (e.g. to zero a freshly
/// allocated page table) without a general per-frame temporary-mapping
/// mechanism: `phys_to_virt(p) = PHYS_MAP_BASE + p`.
pub fn init(total_memory_bytes: usize) {
    let entries_needed = total_memory_bytes.div_ceil(4 * 1024 * 1024);
    assert!(
        PHYS_MAP_PDE_INDEX + entries_needed <= 1024,
        "physical memory too large for the linear physmap window"
    );

    unsafe {
        for i in 0..entries_needed {
            let phys_base = (i as u32) * 4 * 1024 * 1024;
            boot_page_directory[PHYS_MAP_PDE_INDEX + i] = phys_base | PAGE_PRESENT_RW_4M;
        }
        flush_tlb();
    }
}

unsafe fn flush_tlb() {
    let cr3: u32;
    asm!("mov {0}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    asm!("mov cr3, {0}", in(reg) cr3, options(nostack, preserves_flags));
}

pub fn phys_to_virt(phys_addr: usize) -> usize {
    PHYS_MAP_BASE + phys_addr
}

/// Physical address of the boot bootstrap page directory (LMA == VMA for
/// the `.boot` section it lives in -- see linker.ld). Kernel-mode tasks
/// that don't need their own address space just run with this one active.
pub fn boot_page_directory_phys() -> usize {
    core::ptr::addr_of!(boot_page_directory) as usize
}

/// Loads `phys_addr` (a page directory's physical address) into CR3.
///
/// # Safety
/// The page directory must map whatever code/stack is currently executing,
/// or execution will fault the instant this returns.
pub unsafe fn activate_phys(phys_addr: usize) {
    asm!("mov cr3, {0}", in(reg) phys_addr as u32, options(nostack, preserves_flags));
}

fn zero_frame(phys_addr: usize) {
    let virt = phys_to_virt(phys_addr) as *mut u8;
    unsafe { core::ptr::write_bytes(virt, 0, FRAME_SIZE) };
}

/// A process address space: a physical frame holding 1024 page-directory
/// entries, plus separately allocated page-table frames for whichever
/// entries are actually mapped.
pub struct PageDirectory {
    phys_frame: usize,
}

impl PageDirectory {
    /// Allocates a fresh page directory that already contains the kernel's
    /// higher-half mapping (shared identically across every address space,
    /// so syscalls/interrupts work no matter which task's CR3 is loaded).
    pub fn new() -> Self {
        let phys_frame = frame::alloc_frame().expect("out of physical memory");
        zero_frame(phys_frame);

        let table = phys_to_virt(phys_frame) as *mut u32;
        unsafe {
            for i in KERNEL_VMA_PDE_INDEX..1024 {
                *table.add(i) = boot_page_directory[i];
            }
        }

        PageDirectory { phys_frame }
    }

    /// Maps a single 4 KiB page, allocating a page-table frame on demand if
    /// the covering PDE isn't present yet.
    pub fn map_page(&mut self, virt_addr: usize, phys_addr: usize, user: bool, writable: bool) {
        assert_eq!(virt_addr % FRAME_SIZE, 0);
        assert_eq!(phys_addr % FRAME_SIZE, 0);

        let pd_index = virt_addr >> 22;
        let pt_index = (virt_addr >> 12) & 0x3FF;

        let table = phys_to_virt(self.phys_frame) as *mut u32;
        unsafe {
            let pde = *table.add(pd_index);
            let pt_phys = if pde & PAGE_PRESENT != 0 {
                // The CPU ANDs the PDE's and PTE's user bits together, so a
                // PDE left at kernel-only (because an earlier mapping in
                // this same 4 MiB region didn't need user access) would
                // silently deny ring-3 access to this page regardless of
                // its own PTE bit. Upgrade it if this mapping needs it.
                if user && pde & PAGE_USER == 0 {
                    *table.add(pd_index) = pde | PAGE_USER;
                }
                (pde & PAGE_ADDR_MASK) as usize
            } else {
                let new_pt = frame::alloc_frame().expect("out of physical memory");
                zero_frame(new_pt);
                let mut flags = PAGE_PRESENT | PAGE_WRITABLE;
                if user {
                    flags |= PAGE_USER;
                }
                *table.add(pd_index) = (new_pt as u32) | flags;
                new_pt
            };

            let pt = phys_to_virt(pt_phys) as *mut u32;
            let mut flags = PAGE_PRESENT;
            if writable {
                flags |= PAGE_WRITABLE;
            }
            if user {
                flags |= PAGE_USER;
            }
            *pt.add(pt_index) = (phys_addr as u32) | flags;
        }
    }

    pub fn phys_addr(&self) -> usize {
        self.phys_frame
    }
}

/// Checks that every page covering `[vaddr, vaddr + len)` is present and
/// marked user-accessible (both PDE and PTE) in whichever page directory
/// CR3 currently points at. Used to validate a raw pointer/length a ring-3
/// task passes to a syscall before the kernel dereferences it directly --
/// without this, a syscall like debug_write would happily read out
/// whatever kernel memory a task pointed it at (the kernel's own mapping
/// is present in every address space) or crash the whole kernel by
/// dereferencing an unmapped address from kernel mode.
pub fn current_range_is_user_accessible(vaddr: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let end = match vaddr.checked_add(len) {
        Some(e) => e,
        None => return false,
    };

    let cr3 = unsafe {
        let value: u32;
        asm!("mov {0}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
        value as usize
    };
    let table = phys_to_virt(cr3) as *const u32;

    let mut page = vaddr & !(FRAME_SIZE - 1);
    let last_page = (end - 1) & !(FRAME_SIZE - 1);
    loop {
        let pd_index = page >> 22;
        let pt_index = (page >> 12) & 0x3FF;
        unsafe {
            let pde = *table.add(pd_index);
            // A present 4 MiB (PSE) PDE means this range falls in the
            // kernel/boot mapping, which is never marked user-accessible
            // (see boot.s / PageDirectory::new) -- reject it rather than
            // misreading the PDE's physical-base bits as a page-table.
            if pde & PAGE_PRESENT == 0 || pde & PAGE_USER == 0 || pde & 0x80 != 0 {
                return false;
            }
            let pt = phys_to_virt((pde & PAGE_ADDR_MASK) as usize) as *const u32;
            let pte = *pt.add(pt_index);
            if pte & PAGE_PRESENT == 0 || pte & PAGE_USER == 0 {
                return false;
            }
        }
        if page >= last_page {
            return true;
        }
        page += FRAME_SIZE;
    }
}
