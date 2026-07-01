use core::alloc::{GlobalAlloc, Layout};
use core::mem;
use core::ptr::NonNull;

use crate::sync::Mutex;

struct FreeBlock {
    size: usize,
    next: Option<NonNull<FreeBlock>>,
}

/// Every free region's address and size are kept a multiple of this. As
/// long as every allocation's *size* is also rounded up to a multiple of
/// this (done in `alloc`/`dealloc` below) and its alignment doesn't exceed
/// it (true for everything this heap currently serves -- page-aligned
/// buffers go through the frame allocator instead), a split's front excess
/// is always exactly 0 and its back excess is always itself a multiple of
/// `MIN_BLOCK_ALIGN`, so it's either 0 or large enough to hold a
/// `FreeBlock` header. Without this, alignment padding could produce
/// leftover slivers smaller than a `FreeBlock` that `add_free_region`
/// can't track and so permanently leaks.
const MIN_BLOCK_ALIGN: usize = 8;

/// A first-fit free-list allocator: no coalescing of adjacent freed blocks,
/// which trades some fragmentation over long runs for a much smaller
/// implementation. Good enough for kernel-internal bookkeeping structures;
/// worth revisiting if/when long-lived allocation churn makes fragmentation
/// a real problem.
struct LinkedListHeap {
    head: Option<NonNull<FreeBlock>>,
}

unsafe impl Send for LinkedListHeap {}

impl LinkedListHeap {
    const fn empty() -> Self {
        LinkedListHeap { head: None }
    }

    unsafe fn add_free_region(&mut self, addr: usize, size: usize) {
        if size < mem::size_of::<FreeBlock>() {
            return;
        }
        let block = FreeBlock {
            size,
            next: self.head.take(),
        };
        let ptr = addr as *mut FreeBlock;
        ptr.write(block);
        self.head = Some(NonNull::new_unchecked(ptr));
    }

    fn align_up(addr: usize, align: usize) -> usize {
        (addr + align - 1) & !(align - 1)
    }

    /// Finds and unlinks a free block able to hold `size` bytes aligned to
    /// `align`, reinserting any leftover space in front of or behind the
    /// allocation as new (smaller) free blocks.
    unsafe fn find_region(&mut self, size: usize, align: usize) -> Option<usize> {
        let mut prev: Option<NonNull<FreeBlock>> = None;
        let mut current = self.head;

        while let Some(mut node) = current {
            let node_ref = node.as_mut();
            let region_start = node.as_ptr() as usize;
            let region_end = region_start + node_ref.size;
            let alloc_start = Self::align_up(region_start, align);

            if let Some(alloc_end) = alloc_start.checked_add(size) {
                if alloc_end <= region_end {
                    let excess_front = alloc_start - region_start;
                    let excess_back = region_end - alloc_end;
                    let next = node_ref.next;

                    match prev {
                        Some(mut p) => p.as_mut().next = next,
                        None => self.head = next,
                    }

                    // With size/address kept MIN_BLOCK_ALIGN-aligned
                    // throughout (see its doc comment), excess_front is 0
                    // and excess_back is a multiple of MIN_BLOCK_ALIGN for
                    // every request this heap actually serves today; the
                    // >0 checks are only a safety net for align >
                    // MIN_BLOCK_ALIGN, which nothing currently requests.
                    if excess_front > 0 {
                        self.add_free_region(region_start, excess_front);
                    }
                    if excess_back > 0 {
                        self.add_free_region(alloc_end, excess_back);
                    }

                    return Some(alloc_start);
                }
            }

            prev = current;
            current = node_ref.next;
        }
        None
    }
}

pub struct LockedHeap(Mutex<LinkedListHeap>);

impl LockedHeap {
    pub const fn empty() -> Self {
        LockedHeap(Mutex::new(LinkedListHeap::empty()))
    }

    /// # Safety
    /// `start` must point to at least `size` bytes of otherwise-unused,
    /// valid, writable, already-mapped memory, and this must be called
    /// exactly once before any allocation is attempted. `start` must be
    /// aligned to `MIN_BLOCK_ALIGN` (see its doc comment).
    pub unsafe fn init(&self, start: *mut u8, size: usize) {
        assert_eq!(start as usize % MIN_BLOCK_ALIGN, 0, "heap start must be MIN_BLOCK_ALIGN-aligned");
        let size = size & !(MIN_BLOCK_ALIGN - 1);
        self.0.lock().add_free_region(start as usize, size);
    }
}

/// Rounds `size` up to a multiple of `MIN_BLOCK_ALIGN`, matching the
/// invariant every free region in this heap is kept to.
fn round_up_min_block(size: usize) -> usize {
    (size + MIN_BLOCK_ALIGN - 1) & !(MIN_BLOCK_ALIGN - 1)
}

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = round_up_min_block(layout.size().max(mem::size_of::<FreeBlock>()));
        let align = layout.align().max(MIN_BLOCK_ALIGN);
        match self.0.lock().find_region(size, align) {
            Some(start) => start as *mut u8,
            None => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // Must recompute the exact same (rounded) size alloc() reserved --
        // handing back only layout.size() would understate what's actually
        // free and leak the rounding padding right back into the same bug
        // this rounding exists to close.
        let size = round_up_min_block(layout.size().max(mem::size_of::<FreeBlock>()));
        self.0.lock().add_free_region(ptr as usize, size);
    }
}
