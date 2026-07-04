use alloc::vec;
use alloc::vec::Vec;

use crate::sync::Mutex;

pub const FRAME_SIZE: usize = 4096;

extern "C" {
    /// Linker-provided physical end address of the kernel image (see
    /// linker.ld); everything below it is reserved, never handed out.
    static KERNEL_END_PHYS: u8;
}

struct Bitmap {
    bits: Vec<u8>,
    frame_count: usize,
}

impl Bitmap {
    fn is_used(&self, frame: usize) -> bool {
        self.bits[frame / 8] & (1 << (frame % 8)) != 0
    }

    fn set_used(&mut self, frame: usize, used: bool) {
        if used {
            self.bits[frame / 8] |= 1 << (frame % 8);
        } else {
            self.bits[frame / 8] &= !(1 << (frame % 8));
        }
    }
}

static BITMAP: Mutex<Option<Bitmap>> = Mutex::new(None);

/// Builds the frame bitmap from the multiboot-reported memory size, marking
/// frame 0 (avoids a null physical address ever being "valid"), the BIOS/low
/// memory area below 1 MiB, and the kernel image itself as already used.
///
/// This trusts `mem_upper` as one contiguous usable region above 1 MiB; it
/// doesn't walk the full multiboot memory map, so it would over-allocate on
/// hardware with holes below 4 GiB. Fine for QEMU/bring-up, worth revisiting
/// before targeting real hardware.
pub fn init(total_memory_bytes: usize) {
    let frame_count = total_memory_bytes / FRAME_SIZE;
    let mut bits = vec![0u8; (frame_count + 7) / 8];

    let kernel_end_phys = core::ptr::addr_of!(KERNEL_END_PHYS) as usize;
    let reserved_up_to_frame = kernel_end_phys.div_ceil(FRAME_SIZE);

    for frame in 0..reserved_up_to_frame.min(frame_count) {
        bits[frame / 8] |= 1 << (frame % 8);
    }

    *BITMAP.lock() = Some(Bitmap { bits, frame_count });
}

/// Marks the physical range `[start, end)` as reserved (used), e.g. for
/// multiboot module payloads that must not be handed out as free frames.
pub fn reserve_range(start: usize, end: usize) {
    let mut guard = BITMAP.lock();
    let bitmap = guard.as_mut().expect("frame allocator not initialized");
    let first = start / FRAME_SIZE;
    let last = end.div_ceil(FRAME_SIZE).min(bitmap.frame_count);
    for frame in first..last {
        bitmap.set_used(frame, true);
    }
}

/// Allocates one physical frame, returning its physical address.
pub fn alloc_frame() -> Option<usize> {
    let mut guard = BITMAP.lock();
    let bitmap = guard.as_mut().expect("frame allocator not initialized");
    for frame in 0..bitmap.frame_count {
        if !bitmap.is_used(frame) {
            bitmap.set_used(frame, true);
            return Some(frame * FRAME_SIZE);
        }
    }
    None
}

pub fn free_frame(phys_addr: usize) {
    let mut guard = BITMAP.lock();
    let bitmap = guard.as_mut().expect("frame allocator not initialized");
    bitmap.set_used(phys_addr / FRAME_SIZE, false);
}

/// Allocates `count` *physically contiguous* frames, returning the
/// address of the first one. Checkpoint W needs this for a DMA-capable
/// device's ring buffer (the RTL8139's receive ring): the card's own DMA
/// engine writes directly to physical memory with no concept of page
/// tables, so a buffer backed by scattered single frames -- all this
/// allocator supported before, via `alloc_frame` -- wouldn't appear
/// contiguous to it the way it needs to.
///
/// A linear scan for the first run of `count` free frames -- no need for
/// anything fancier (a free-list keyed by run length, buddy allocation)
/// at this kernel's scale, and every existing caller of `alloc_frame`
/// keeps working unchanged since this is purely additive. Returns `None`
/// without marking anything used if no long-enough run exists; a
/// found run is marked used atomically (under the same lock the whole
/// scan holds), never partially on a later failure.
pub fn alloc_frames_contiguous(count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let mut guard = BITMAP.lock();
    let bitmap = guard.as_mut().expect("frame allocator not initialized");

    let mut run_start = 0usize;
    let mut run_len = 0usize;
    for frame in 0..bitmap.frame_count {
        if bitmap.is_used(frame) {
            run_len = 0;
            run_start = frame + 1;
        } else {
            run_len += 1;
            if run_len == count {
                for f in run_start..run_start + count {
                    bitmap.set_used(f, true);
                }
                return Some(run_start * FRAME_SIZE);
            }
        }
    }
    None
}

/// Frees `count` contiguous frames starting at `phys_addr`, the
/// counterpart to `alloc_frames_contiguous`.
#[allow(dead_code)]
pub fn free_frames_contiguous(phys_addr: usize, count: usize) {
    let mut guard = BITMAP.lock();
    let bitmap = guard.as_mut().expect("frame allocator not initialized");
    let first = phys_addr / FRAME_SIZE;
    for f in first..first + count {
        bitmap.set_used(f, false);
    }
}
