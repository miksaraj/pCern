//! Multiboot1 info struct parsing. GRUB passes the physical address of this
//! structure in `ebx`; it (and everything it points to) lives in low
//! physical memory, which the kernel keeps identity-mapped forever
//! (see boot.s) specifically so this stays readable at any point.

const FLAG_MEM: u32 = 1 << 0;
const FLAG_MODS: u32 = 1 << 3;

#[repr(C)]
struct RawInfo {
    flags: u32,
    mem_lower: u32,
    mem_upper: u32,
    boot_device: u32,
    cmdline: u32,
    mods_count: u32,
    mods_addr: u32,
    _syms: [u32; 4],
    mmap_length: u32,
    mmap_addr: u32,
    // Remaining fields (drives, config table, boot loader name, APM/VBE
    // info) aren't needed by this kernel and are left unparsed.
}

#[repr(C)]
struct RawModule {
    mod_start: u32,
    mod_end: u32,
    _string: u32,
    _reserved: u32,
}

#[derive(Clone, Copy)]
pub struct Module {
    pub start: usize,
    pub end: usize,
}

pub struct MultibootInfo {
    raw: *const RawInfo,
}

impl MultibootInfo {
    /// # Safety
    /// `addr` must be the physical multiboot info pointer GRUB passed in
    /// `ebx`, and the low identity mapping covering it must still be active.
    pub unsafe fn from_addr(addr: u32) -> Self {
        MultibootInfo {
            raw: addr as *const RawInfo,
        }
    }

    /// Total usable RAM in bytes, approximated as 1 MiB + `mem_upper` KiB
    /// (the conventional lower/upper split multiboot1 reports); does not
    /// account for any holes reported in the full memory map.
    pub fn total_memory_bytes(&self) -> usize {
        let info = unsafe { &*self.raw };
        if info.flags & FLAG_MEM == 0 {
            return 0;
        }
        (1024 * 1024) + (info.mem_upper as usize * 1024)
    }

    #[allow(dead_code)]
    pub fn module_count(&self) -> usize {
        let info = unsafe { &*self.raw };
        if info.flags & FLAG_MODS == 0 {
            0
        } else {
            info.mods_count as usize
        }
    }

    pub fn module(&self, index: usize) -> Option<Module> {
        let info = unsafe { &*self.raw };
        if info.flags & FLAG_MODS == 0 || index >= info.mods_count as usize {
            return None;
        }
        let entry = unsafe {
            &*((info.mods_addr as usize + index * core::mem::size_of::<RawModule>())
                as *const RawModule)
        };
        Some(Module {
            start: entry.mod_start as usize,
            end: entry.mod_end as usize,
        })
    }
}
