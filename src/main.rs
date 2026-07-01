#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::global_asm;
use core::panic::PanicInfo;

global_asm!(include_str!("boot.s"));

mod ansi;
mod exceptions;
mod gdt;
mod idt;
mod ipc;
mod keyboard;
mod mm;
mod multiboot;
mod pic;
mod port;
mod scheduler;
mod serial;
mod sync;
mod syscall;
mod task;
mod timer;
mod vga;

/// Where each ring-3 test program (loaded as a multiboot module) gets
/// mapped, and where its stack lives. Arbitrary but page-aligned and clear
/// of the kernel's own 0xC0000000+ range; every module gets its own fresh
/// address space, so reusing the same virtual layout for each is fine.
const USER_CODE_BASE: usize = 0x0040_0000;
const USER_STACK_TOP: usize = 0x0080_0000;
const USER_STACK_PAGES: usize = 4;

const MULTIBOOT_MAGIC: u32 = 0x2BADB002;

/// Backing storage for the kernel heap. Comfortably inside the 4 MiB PSE
/// mapping the boot bootstrap sets up, so no real paging is needed yet;
/// the physical frame allocator/paging brought up later in kernel_main is
/// only used for task stacks, page tables, etc.
///
/// Explicitly 8-byte aligned: mm::heap::LockedHeap::init requires it (see
/// its MIN_BLOCK_ALIGN doc comment) to guarantee alignment-padding slivers
/// during allocation are never smaller than a trackable free block. A
/// plain `[u8; N]` has no inherent alignment beyond 1.
const HEAP_SIZE: usize = 256 * 1024;
#[repr(align(8))]
struct HeapMemory(#[allow(dead_code)] [u8; HEAP_SIZE]);
static mut HEAP_MEMORY: HeapMemory = HeapMemory([0; HEAP_SIZE]);

#[global_allocator]
static ALLOCATOR: mm::heap::LockedHeap = mm::heap::LockedHeap::empty();

#[no_mangle]
pub extern "C" fn kernel_main(magic: u32, multiboot_info_addr: u32) -> ! {
    vga::WRITER.lock().clear_screen();
    serial::init();

    println!("\x1b[1;36mpCern\x1b[0m nanokernel starting...");

    if magic != MULTIBOOT_MAGIC {
        println!("\x1b[1;33mwarning:\x1b[0m unexpected multiboot magic {:#010x}", magic);
    }

    gdt::init();
    println!("[ \x1b[1;32mok\x1b[0m ] GDT installed");

    idt::init();
    println!("[ \x1b[1;32mok\x1b[0m ] IDT installed");

    pic::init();
    println!("[ \x1b[1;32mok\x1b[0m ] PIC remapped, timer + keyboard unmasked");

    unsafe { ALLOCATOR.init(core::ptr::addr_of_mut!(HEAP_MEMORY) as *mut u8, HEAP_SIZE) };
    heap_smoke_test();
    println!("[ \x1b[1;32mok\x1b[0m ] kernel heap online ({} KiB)", HEAP_SIZE / 1024);

    let mb_info = unsafe { multiboot::MultibootInfo::from_addr(multiboot_info_addr) };
    let total_memory = mb_info
        .total_memory_bytes()
        .expect("multiboot did not report memory size (FLAG_MEM unset) -- not booted by a compliant multiboot loader?");
    println!(
        "[ \x1b[1;32mok\x1b[0m ] multiboot reports {} MiB usable memory",
        total_memory / (1024 * 1024)
    );

    mm::frame::init(total_memory);
    mm::paging::init(total_memory);
    frame_and_paging_smoke_test();

    // Reserve every module's physical range up front, before allocating any
    // task stacks/page tables: modules are only 1 page apart in memory, so
    // allocating frames for one task while a not-yet-processed module's
    // bytes are still unreserved risks handing out (and clobbering) exactly
    // the frame the next module's own code is sitting in.
    for i in 0..mb_info.module_count() {
        if let Some(m) = mb_info.module(i) {
            mm::frame::reserve_range(m.start, m.end);
        }
    }
    println!("[ \x1b[1;32mok\x1b[0m ] frame allocator + physical memory map online");

    unsafe { core::arch::asm!("sti") };
    println!("[ \x1b[1;32mok\x1b[0m ] interrupts enabled");

    scheduler::spawn_kernel_task(task_a);
    scheduler::spawn_kernel_task(task_b);
    println!("[ \x1b[1;32mok\x1b[0m ] spawned 2 kernel tasks");

    // Spawn order matters: ping.asm hardcodes pong's task id (4), which
    // depends on task_a/task_b (1, 2) and ping itself (3) being spawned
    // first in exactly this order -- see userland/ping.asm.
    for (index, name) in [(0, "ping"), (1, "pong")] {
        match spawn_ring3_task_from_module(&mb_info, index) {
            Some(id) => println!("[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task '{}' (id={})", name, id),
            None => println!(
                "[ \x1b[1;33mwarn\x1b[0m ] no multiboot module {} found, skipping '{}'",
                index, name
            ),
        }
    }

    // Spawned last so it doesn't shift ping's hardcoded PONG_TASK_ID. Never
    // blocks or exits, so block_current()/exit_current() always have at
    // least one task to fall back to instead of panicking when every "real"
    // task is blocked/exited.
    scheduler::spawn_kernel_task(idle_task);
    println!("[ \x1b[1;32mok\x1b[0m ] spawned idle task");
    println!("[ \x1b[1;32mok\x1b[0m ] handing off to the scheduler");
    println!();

    scheduler::start();
}

extern "C" fn idle_task() -> ! {
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}

/// Loads multiboot module `index` (see grub.cfg) as a flat, position-
/// dependent ring-3 program: maps it at `USER_CODE_BASE` in a fresh address
/// space, gives it a small stack, and spawns it. Returns `None` if GRUB
/// didn't hand us that many modules. Assumes every module's physical range
/// has already been reserved (see kernel_main) -- otherwise allocating
/// frames here risks clobbering a module this kernel hasn't loaded yet.
fn spawn_ring3_task_from_module(mb_info: &multiboot::MultibootInfo, index: usize) -> Option<task::TaskId> {
    let module = mb_info.module(index)?;

    let mut page_dir = mm::paging::PageDirectory::new();

    let module_len = module.end - module.start;
    let code_pages = module_len.div_ceil(mm::frame::FRAME_SIZE).max(1);
    for i in 0..code_pages {
        let phys = mm::frame::alloc_frame().expect("out of memory mapping user code");
        page_dir.map_page(USER_CODE_BASE + i * mm::frame::FRAME_SIZE, phys, true, true);

        let dst = mm::paging::phys_to_virt(phys) as *mut u8;
        let page_offset = i * mm::frame::FRAME_SIZE;
        let copy_len = module_len.saturating_sub(page_offset).min(mm::frame::FRAME_SIZE);
        unsafe {
            core::ptr::write_bytes(dst, 0, mm::frame::FRAME_SIZE);
            if copy_len > 0 {
                let src = mm::paging::phys_to_virt(module.start + page_offset) as *const u8;
                core::ptr::copy_nonoverlapping(src, dst, copy_len);
            }
        }
    }

    for i in 0..USER_STACK_PAGES {
        let phys = mm::frame::alloc_frame().expect("out of memory mapping user stack");
        let vaddr = USER_STACK_TOP - (i + 1) * mm::frame::FRAME_SIZE;
        page_dir.map_page(vaddr, phys, true, true);
    }

    let task = task::Task::new_user(USER_CODE_BASE as u32, USER_STACK_TOP as u32, page_dir.phys_addr());
    Some(scheduler::spawn(task))
}

extern "C" fn task_a() -> ! {
    let mut i: u32 = 0;
    loop {
        println!("\x1b[1;32m[task A]\x1b[0m iteration {}", i);
        i += 1;
        scheduler::yield_now();
    }
}

extern "C" fn task_b() -> ! {
    let mut i: u32 = 0;
    loop {
        println!("\x1b[1;35m[task B]\x1b[0m iteration {}", i);
        i += 1;
        scheduler::yield_now();
    }
}

/// Exercises the heap through `Vec` (growth -> reallocation -> dealloc of
/// the old backing store) and `Box`, proving alloc/dealloc both work rather
/// than just a bump allocator that never frees.
fn heap_smoke_test() {
    use alloc::boxed::Box;
    use alloc::vec::Vec;

    let mut v: Vec<u32> = Vec::new();
    for i in 0..64 {
        v.push(i * i);
    }
    let sum: u32 = v.iter().sum();
    assert_eq!(v.len(), 64);
    assert_eq!(sum, (0..64u32).map(|i| i * i).sum());

    let boxed = Box::new(0x1234_5678u32);
    assert_eq!(*boxed, 0x1234_5678);

    println!(
        "      Vec<u32> len={} sum={:#x}, Box<u32>={:#x}",
        v.len(),
        sum,
        *boxed
    );
}

/// Proves frame alloc/free works (including reuse of freed frames) and that
/// a fresh `PageDirectory` can be built (frame alloc + zero + copy-in the
/// kernel's higher-half PDEs) without crashing. Ring-3 tasks are what
/// actually activate/map through one of these -- that's Checkpoint 4.
fn frame_and_paging_smoke_test() {
    let f1 = mm::frame::alloc_frame().expect("frame alloc failed");
    let f2 = mm::frame::alloc_frame().expect("frame alloc failed");
    assert_ne!(f1, f2);
    mm::frame::free_frame(f1);
    let f3 = mm::frame::alloc_frame().expect("frame alloc failed");
    assert_eq!(f3, f1, "freed frame should be reused before untouched ones");
    mm::frame::free_frame(f2);
    mm::frame::free_frame(f3);

    let pd = mm::paging::PageDirectory::new();
    println!(
        "      frames: f1={:#x} f2={:#x} f3(reused)={:#x}, PageDirectory phys={:#x}",
        f1,
        f2,
        f3,
        pd.phys_addr()
    );
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("allocation error: {:?}", layout);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\x1b[1;41;37m KERNEL PANIC \x1b[0m {}", info);
    loop {
        unsafe { core::arch::asm!("cli", "hlt") };
    }
}
