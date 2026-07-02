#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::global_asm;
use core::panic::PanicInfo;

global_asm!(include_str!("boot.s"));

mod ansi;
mod cap;
mod exceptions;
mod gdt;
mod idt;
mod ipc;
mod irq;
mod keyboard;
mod loader;
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

    loader::init(mb_info);
    loader::reserve_all_modules();
    println!("[ \x1b[1;32mok\x1b[0m ] frame allocator + physical memory map online");

    unsafe { core::arch::asm!("sti") };
    println!("[ \x1b[1;32mok\x1b[0m ] interrupts enabled");

    // Spawned first so it always gets task id 1 (id 0 is the reserved
    // KERNEL_TASK_ID pseudo-sender -- see ipc.rs). Every task spawned after
    // this automatically gets a capability to its endpoint installed at
    // CSlot 1 (see loader::set_nameservice_endpoint/spawn_from_module) --
    // the one piece of discovery infrastructure nothing has to be told
    // about individually, the way every other capability below still is.
    let nameservice_id = loader::spawn_from_module(0, &[]).expect("no multiboot module 0 found for 'nameservice'");
    println!("[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task 'nameservice' (id={})", nameservice_id);
    let nameservice_endpoint = ipc::create_endpoint(nameservice_id);
    let nameservice_inbox_slot = grant_endpoint_cap(nameservice_id, nameservice_endpoint);
    debug_assert_eq!(nameservice_inbox_slot, 1, "nameservice's own inbox must land at CSlot 1");
    loader::set_nameservice_endpoint(nameservice_endpoint);

    // Port access to the keyboard controller and CRTC, since console_server
    // owns both the keyboard and VGA/ANSI console (Checkpoint D). There's
    // still no *dynamic* discovery for capabilities beyond the name
    // service itself, so main.rs wires the rest up here by hand, right
    // after spawning -- this is trusted kernel code, so directly minting
    // capabilities into another task's own CSpace (via
    // scheduler::install_cap_for) is safe the same way allowed_ports
    // already is. Fixed convention every userland program below relies on:
    // CSlot 1 = name service (auto-granted), CSlot 2 = "my own inbox,"
    // CSlot 3+ = whatever else that task specifically needs.
    const CONSOLE_SERVER_PORTS: [u16; 4] = [0x60, 0x64, 0x3D4, 0x3D5];
    let console_id =
        loader::spawn_from_module(1, &CONSOLE_SERVER_PORTS).expect("no multiboot module 1 found for 'console_server'");
    println!("[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task 'console_server' (id={})", console_id);
    let console_endpoint = ipc::create_endpoint(console_id);
    let console_inbox_slot = grant_endpoint_cap(console_id, console_endpoint);
    debug_assert_eq!(console_inbox_slot, 2, "console_server's own inbox must land at CSlot 2");

    const VGA_BUFFER_PHYS: usize = 0xB8000;
    const VGA_BUFFER_LEN: usize = 0x1000;
    let vga_grant = cap::mint_root(cap::CapKind::MemoryGrant {
        phys_base: VGA_BUFFER_PHYS,
        len: VGA_BUFFER_LEN,
        writable: true,
    });
    let vga_slot = scheduler::install_cap_for(console_id, vga_grant);
    debug_assert_eq!(vga_slot, 3, "console_server's VGA grant must land at CSlot 3");

    let irq_control = cap::mint_root(cap::CapKind::IrqControl { irq: 1, endpoint: console_endpoint });
    let irq_slot = scheduler::install_cap_for(console_id, irq_control);
    debug_assert_eq!(irq_slot, 4, "console_server's IrqControl must land at CSlot 4");

    scheduler::spawn_kernel_task(task_a);
    scheduler::spawn_kernel_task(task_b);
    println!("[ \x1b[1;32mok\x1b[0m ] spawned 2 kernel tasks");

    let ping_id = loader::spawn_from_module(2, &[]).expect("no multiboot module 2 found for 'ping'");
    println!("[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task 'ping' (id={})", ping_id);
    let ping_endpoint = ipc::create_endpoint(ping_id);
    grant_endpoint_cap(ping_id, ping_endpoint); // ping's CSlot 2: its own inbox

    let pong_id = loader::spawn_from_module(3, &[]).expect("no multiboot module 3 found for 'pong'");
    println!("[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task 'pong' (id={})", pong_id);
    let pong_endpoint = ipc::create_endpoint(pong_id);
    grant_endpoint_cap(pong_id, pong_endpoint); // pong's CSlot 2: its own inbox

    grant_endpoint_cap(ping_id, pong_endpoint); // ping's CSlot 3: send to pong
    grant_endpoint_cap(pong_id, ping_endpoint); // pong's CSlot 3: send to ping
    // No pre-wired console capability for ping/pong anymore -- they look
    // up "console" through the name service themselves (see ping.asm/
    // pong.asm), the first real (non-main.rs-hardcoded) use of it.

    // Checkpoint I: the ATA/IDE storage driver. Port access is still
    // hand-wired here the same way console_server's is (there's no
    // capability for I/O ports, just the pre-existing allowed_ports/TSS
    // bitmap mechanism) -- everything else it needs (its own inbox, the
    // name service) comes from the same fixed CSlot convention as any
    // other task.
    const STORAGE_ATA_PORTS: [u16; 9] = [0x1F0, 0x1F1, 0x1F2, 0x1F3, 0x1F4, 0x1F5, 0x1F6, 0x1F7, 0x3F6];
    let storage_id =
        loader::spawn_from_module(4, &STORAGE_ATA_PORTS).expect("no multiboot module 4 found for 'storage_ata'");
    println!("[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task 'storage_ata' (id={})", storage_id);
    let storage_endpoint = ipc::create_endpoint(storage_id);
    grant_endpoint_cap(storage_id, storage_endpoint); // storage_ata's CSlot 2: its own inbox

    // Spawned last so it doesn't shift ping's/pong's task ids. Never blocks
    // or exits, so block_current()/exit_current() always have at least one
    // task to fall back to instead of panicking when every "real" task is
    // blocked/exited.
    scheduler::spawn_kernel_task(idle_task);
    println!("[ \x1b[1;32mok\x1b[0m ] spawned idle task");
    println!("[ \x1b[1;32mok\x1b[0m ] handing off to the scheduler");
    println!();

    scheduler::start();
}

/// Mints a fresh root capability for `endpoint` and installs it into
/// `task_id`'s own CSpace, returning the slot it landed in. A thin wrapper
/// around `cap::mint_root`+`scheduler::install_cap_for` just to keep the
/// boot-time wiring above readable.
fn grant_endpoint_cap(task_id: task::TaskId, endpoint: cap::EndpointId) -> cap::CSlot {
    let node = cap::mint_root(cap::CapKind::Endpoint { id: endpoint });
    scheduler::install_cap_for(task_id, node)
}

extern "C" fn idle_task() -> ! {
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
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
