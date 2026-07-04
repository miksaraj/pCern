#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

// At most one of these mutually exclusive boot-configuration features may
// be enabled at once -- each spawns a different binary at a fixed
// multiboot module index (see each `*_test_spawn` function's own doc
// comment) against its own dedicated grub config; building with more than
// one would have them race for the same module slot with no identity
// check to catch the mismatch. A single linear count, not a pairwise
// `compile_error!` per combination (which grew quadratically every time a
// new `*_test` feature was added -- ten pairs for five features before
// Checkpoint W's `nic_test` made it six), catches any two-or-more
// combination in one check, and adding a seventh feature later only means
// adding one more term here, not six more blocks.
const _: () = {
    let count = cfg!(feature = "test_harness") as u8
        + cfg!(feature = "keyboard_test") as u8
        + cfg!(feature = "raw_input_test") as u8
        + cfg!(feature = "editor_test") as u8
        + cfg!(feature = "reboot_test") as u8
        + cfg!(feature = "nic_test") as u8;
    assert!(
        count <= 1,
        "at most one *_test/test_harness feature may be enabled at a time; build one or the other, never several"
    );
};

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
mod pci;
mod pic;
mod port;
mod reboot;
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
    // A real (not debug-only) assert: nameservice's own registration
    // ALLOWLIST (userland/services/nameservice/src/main.rs) hardcodes this
    // exact task id to the "console" name, with nothing else tying the two
    // files together. A silent mismatch here (e.g. from a reordered spawn
    // sequence) would either wrongly reject console_server's own
    // registration, or let whatever task now holds id 2 register "console"
    // in its place -- panicking loudly at boot is far preferable to either.
    assert_eq!(console_id, 2, "nameservice's ALLOWLIST assumes console_server is task id 2");
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

    // Checkpoint I: the ATA/IDE storage driver. Port access is still
    // hand-wired here the same way console_server's is (there's no
    // capability for I/O ports, just the pre-existing allowed_ports/TSS
    // bitmap mechanism) -- everything else it needs (its own inbox, the
    // name service) comes from the same fixed CSlot convention as any
    // other task.
    const STORAGE_ATA_PORTS: [u16; 9] = [0x1F0, 0x1F1, 0x1F2, 0x1F3, 0x1F4, 0x1F5, 0x1F6, 0x1F7, 0x3F6];
    let storage_id =
        loader::spawn_from_module(2, &STORAGE_ATA_PORTS).expect("no multiboot module 2 found for 'storage_ata'");
    println!("[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task 'storage_ata' (id={})", storage_id);
    // See the identical assert on console_id above -- nameservice's
    // ALLOWLIST hardcodes this task id to "storage".
    assert_eq!(storage_id, 3, "nameservice's ALLOWLIST assumes storage_ata is task id 3");
    let storage_endpoint = ipc::create_endpoint(storage_id);
    grant_endpoint_cap(storage_id, storage_endpoint); // storage_ata's CSlot 2: its own inbox

    // Checkpoint J: the FAT32 filesystem server. No hardware ports of its
    // own -- it's purely an IPC client of storage_ata and (via the name
    // service) a server to whatever looks up "fs".
    let fs_id = loader::spawn_from_module(3, &[]).expect("no multiboot module 3 found for 'fs_fat32'");
    println!("[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task 'fs_fat32' (id={})", fs_id);
    // See the identical assert on console_id above -- nameservice's
    // ALLOWLIST hardcodes this task id to "fs".
    assert_eq!(fs_id, 4, "nameservice's ALLOWLIST assumes fs_fat32 is task id 4");
    let fs_endpoint = ipc::create_endpoint(fs_id);
    grant_endpoint_cap(fs_id, fs_endpoint); // fs_fat32's CSlot 2: its own inbox

    // Checkpoint N: the interactive shell -- purely an IPC client of
    // console_server/fs_fat32/name-service, no hardware ports or
    // capabilities of its own beyond the usual CSlot 1/2 convention.
    // Excluded from the test_harness/keyboard_test/raw_input_test/
    // editor_test/reboot_test/nic_test builds: it would be one more
    // concurrent console-input reader racing that build's own fixture for
    // the "single reader at a time" role, and would shift every fixture's
    // task id (used throughout run_tests.sh/console_input_test's own
    // script) in the test_harness build for no benefit, since nothing
    // there exercises it anyway. Module index 4, spawned *before*
    // spawn_net_rtl8139 below (module index 5) so shell's own id is
    // always deterministic (5) regardless of whether a NIC is attached --
    // see spawn_net_rtl8139's own doc comment for why the optional one
    // must always come last.
    #[cfg(not(any(
        feature = "test_harness",
        feature = "keyboard_test",
        feature = "raw_input_test",
        feature = "editor_test",
        feature = "reboot_test",
        feature = "nic_test"
    )))]
    {
        let shell_id = loader::spawn_from_module(4, &[]).expect("no multiboot module 4 found for 'shell'");
        println!("[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task 'shell' (id={})", shell_id);
        assert_eq!(shell_id, 5, "spawn_net_rtl8139 assumes shell is task id 5, so it always lands at id 6 when present");
        let shell_endpoint = ipc::create_endpoint(shell_id);
        grant_endpoint_cap(shell_id, shell_endpoint); // shell's CSlot 2: its own inbox
    }

    // Checkpoint W: the RTL8139 NIC driver, if PCI enumeration actually
    // finds one attached (QEMU only emulates one when told to via
    // `-device rtl8139`) -- module index 5, spawned *last* and only in
    // the production path (the standalone `nic_test` harness spawns it
    // separately, at the same relative position, via its own dedicated
    // module list). Deliberately last: `spawn_net_rtl8139` returns early
    // without consuming a task id when no card is found, so if anything
    // were spawned after it, that task would silently slide into the id
    // nameservice's ALLOWLIST hardcodes to "net" -- letting it register
    // that trusted name in the real driver's place. Spawning it last
    // means its absence just leaves that id never allocated to anyone.
    #[cfg(not(any(
        feature = "test_harness",
        feature = "keyboard_test",
        feature = "raw_input_test",
        feature = "editor_test",
        feature = "reboot_test",
        feature = "nic_test"
    )))]
    if spawn_net_rtl8139(5).is_none() {
        println!("[ \x1b[1;33mwarn\x1b[0m ] no RTL8139 NIC found via PCI enumeration -- networking unavailable this boot");
    }

    #[cfg(feature = "test_harness")]
    test_harness_spawn();

    #[cfg(feature = "keyboard_test")]
    keyboard_test_spawn();

    #[cfg(feature = "raw_input_test")]
    raw_input_test_spawn();

    #[cfg(feature = "editor_test")]
    editor_test_spawn();

    #[cfg(feature = "nic_test")]
    nic_test_spawn();

    #[cfg(feature = "reboot_test")]
    reboot_test_spawn();

    // Spawned last. Never blocks or exits, so block_current()/exit_current()
    // always have at least one task to fall back to instead of panicking
    // when every "real" task is blocked/exited.
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

/// Spawns the cap_test fixtures (see `userland/cap_test`) as ring-3
/// tasks, only present in a kernel built with `--features test_harness`
/// (see `make test`) -- `grub-test.cfg` is the one grub config whose
/// module list actually matches the indices below; production's
/// `grub.cfg` only has modules 0-4. Each fixture gets the same CSlot 1 =
/// name service / CSlot 2 = own inbox convention as every other task;
/// the two paired fixtures (which need to reach a specific peer, not
/// something discoverable by name) additionally get that peer's endpoint
/// hand-wired at CSlot 3, exactly the way console_server's hardware
/// capabilities are hand-wired above.
#[cfg(feature = "test_harness")]
fn test_harness_spawn() {
    let cap_test_a_id = loader::spawn_from_module(4, &[]).expect("no multiboot module 4 found for 'cap_test_a'");
    let cap_test_a_endpoint = ipc::create_endpoint(cap_test_a_id);
    grant_endpoint_cap(cap_test_a_id, cap_test_a_endpoint); // its CSlot 2: its own inbox

    let cap_test_b_id = loader::spawn_from_module(5, &[]).expect("no multiboot module 5 found for 'cap_test_b'");
    let cap_test_b_endpoint = ipc::create_endpoint(cap_test_b_id);
    grant_endpoint_cap(cap_test_b_id, cap_test_b_endpoint); // its CSlot 2: its own inbox

    grant_endpoint_cap(cap_test_a_id, cap_test_b_endpoint); // cap_test_a's CSlot 3: send to cap_test_b
    grant_endpoint_cap(cap_test_b_id, cap_test_a_endpoint); // cap_test_b's CSlot 3: send to cap_test_a

    let mem_test_a_id = loader::spawn_from_module(6, &[]).expect("no multiboot module 6 found for 'mem_test_a'");
    let mem_test_a_endpoint = ipc::create_endpoint(mem_test_a_id);
    grant_endpoint_cap(mem_test_a_id, mem_test_a_endpoint); // its CSlot 2: its own inbox

    let mem_test_b_id = loader::spawn_from_module(7, &[]).expect("no multiboot module 7 found for 'mem_test_b'");
    let mem_test_b_endpoint = ipc::create_endpoint(mem_test_b_id);
    grant_endpoint_cap(mem_test_b_id, mem_test_b_endpoint); // its CSlot 2: its own inbox

    grant_endpoint_cap(mem_test_a_id, mem_test_b_endpoint); // mem_test_a's CSlot 3: send to mem_test_b
    grant_endpoint_cap(mem_test_b_id, mem_test_a_endpoint); // mem_test_b's CSlot 3: send to mem_test_a

    // storage_client_test isn't included here: storage_ata only supports
    // one client at a time (a single reply_slot/buf_mapped pair, no
    // per-client state -- see its own doc comment), and fs_fat32 is
    // already permanently running as one. Running both concurrently
    // would have them clobber each other's connection. fs_fat32's own
    // client (fs_client_test, right below) already exercises storage_ata
    // thoroughly at one remove; storage_client_test is still there for
    // standalone verification (temporarily wired in with fs_fat32 absent,
    // the way Checkpoint I originally used it).
    // Checkpoint M: fs_client_test also exercises the new
    // SYS_SPAWN_FROM_MEMORY syscall (loads and runs LOADED.BIN) after its
    // own fs_fat32 checks, rather than a second fixture connecting to
    // fs_fat32 concurrently -- fs_fat32 only supports one client at a
    // time (same single-client scope as storage_ata), the same reason
    // storage_client_test can't run alongside it either. See
    // fs_client_test.rs and run_tests.sh's check for the exact task id
    // this produces.
    let fs_client_test_id =
        loader::spawn_from_module(8, &[]).expect("no multiboot module 8 found for 'fs_client_test'");
    let fs_client_test_endpoint = ipc::create_endpoint(fs_client_test_id);
    grant_endpoint_cap(fs_client_test_id, fs_client_test_endpoint); // its CSlot 2: its own inbox

    println!(
        "[ \x1b[1;32mok\x1b[0m ] test_harness: spawned cap_test fixtures (ids {}-{})",
        cap_test_a_id, fs_client_test_id
    );
}

/// Spawns `console_input_test` (see `userland/cap_test`) as a ring-3
/// task, only present in a kernel built with `--features keyboard_test`
/// (see `make iso-keytest`) -- `grub-keytest.cfg` is the one grub config
/// whose module list matches the index below; this is deliberately its
/// own standalone build/boot rather than one more fixture folded into
/// `test_harness_spawn`, since this is the one fixture that blocks on
/// real external keystrokes (see run_console_input_test.sh) rather than
/// completing on its own -- folded into the shared `iso-test` boot, it
/// would simply hang every `make test` run until that harness's own
/// timeout. Granted direct COM1 port access (0x3F8 data, 0x3FD line
/// status), the same allowed_ports mechanism storage_ata's ATA ports use,
/// so it can print its own readiness marker straight to serial -- see its
/// own doc comment for why.
#[cfg(feature = "keyboard_test")]
fn keyboard_test_spawn() {
    const COM1_PORTS: [u16; 2] = [0x3F8, 0x3FD];
    let console_input_test_id =
        loader::spawn_from_module(4, &COM1_PORTS).expect("no multiboot module 4 found for 'console_input_test'");
    let console_input_test_endpoint = ipc::create_endpoint(console_input_test_id);
    grant_endpoint_cap(console_input_test_id, console_input_test_endpoint); // its CSlot 2: its own inbox

    println!(
        "[ \x1b[1;32mok\x1b[0m ] keyboard_test: spawned console_input_test (id={})",
        console_input_test_id
    );
}

/// Spawns `raw_input_test` (see `userland/cap_test`), Phase 7 Checkpoint
/// R's regression fixture for console_server's new raw single-keystroke
/// mode, only present in a kernel built with `--features raw_input_test`
/// (see `make test-raw-input`) -- `grub-rawtest.cfg` is the one grub
/// config whose module list matches the index below. Its own standalone
/// build/boot for the same reason as `keyboard_test_spawn`: it blocks on
/// real external keystrokes rather than completing on its own, and it
/// would otherwise race console_input_test for the single reader_owner
/// role if folded into that build. Granted the same direct COM1 port
/// access as console_input_test, for the same readiness-marker reason.
#[cfg(feature = "raw_input_test")]
fn raw_input_test_spawn() {
    const COM1_PORTS: [u16; 2] = [0x3F8, 0x3FD];
    let raw_input_test_id =
        loader::spawn_from_module(4, &COM1_PORTS).expect("no multiboot module 4 found for 'raw_input_test'");
    let raw_input_test_endpoint = ipc::create_endpoint(raw_input_test_id);
    grant_endpoint_cap(raw_input_test_id, raw_input_test_endpoint); // its CSlot 2: its own inbox

    println!(
        "[ \x1b[1;32mok\x1b[0m ] raw_input_test: spawned raw_input_test (id={})",
        raw_input_test_id
    );
}

/// Spawns `editor_input_test` (see `userland/cap_test`), Phase 7
/// Checkpoint S's regression fixture for the full-screen editor, only
/// present in a kernel built with `--features editor_test` (see
/// `make test-editor`) -- `grub-editortest.cfg` is the one grub config
/// whose module list matches the index below. Its own standalone
/// build/boot for the same reason as `raw_input_test_spawn`: it blocks on
/// real external keystrokes. Granted the same direct COM1 port access as
/// console_input_test/raw_input_test, for the same readiness-marker
/// reason.
#[cfg(feature = "editor_test")]
fn editor_test_spawn() {
    const COM1_PORTS: [u16; 2] = [0x3F8, 0x3FD];
    let editor_input_test_id =
        loader::spawn_from_module(4, &COM1_PORTS).expect("no multiboot module 4 found for 'editor_input_test'");
    let editor_input_test_endpoint = ipc::create_endpoint(editor_input_test_id);
    grant_endpoint_cap(editor_input_test_id, editor_input_test_endpoint); // its CSlot 2: its own inbox

    println!(
        "[ \x1b[1;32mok\x1b[0m ] editor_test: spawned editor_input_test (id={})",
        editor_input_test_id
    );
}

/// Spawns `reboot_test` (see `userland/cap_test`), Checkpoint V's
/// regression fixture for the new `SYS_REBOOT` syscall, only present in a
/// kernel built with `--features reboot_test` (see `make test-reboot`) --
/// `grub-reboottest.cfg` is the one grub config whose module list matches
/// the index below. Its own standalone build/boot for the same reason as
/// every other `*_test_spawn` here: it deliberately resets the whole
/// machine, which would be indistinguishable from a crash to anything
/// else sharing this boot. Granted the same direct COM1 port access as
/// the other harnesses (to print a marker before the reset actually
/// lands), plus -- the whole point of this fixture -- a freshly minted
/// `RebootControl` capability at CSlot 3, following the same convention
/// every other hand-wired hardware capability in this file uses. The real
/// intended holder of a capability like this is a future update service
/// (ZephyrLite's own Checkpoint Z); for now, this fixture is the only
/// task in the whole system ever handed one.
#[cfg(feature = "reboot_test")]
fn reboot_test_spawn() {
    const COM1_PORTS: [u16; 2] = [0x3F8, 0x3FD];
    let reboot_test_id =
        loader::spawn_from_module(4, &COM1_PORTS).expect("no multiboot module 4 found for 'reboot_test'");
    let reboot_test_endpoint = ipc::create_endpoint(reboot_test_id);
    grant_endpoint_cap(reboot_test_id, reboot_test_endpoint); // its CSlot 2: its own inbox

    let reboot_control = cap::mint_root(cap::CapKind::RebootControl);
    let reboot_control_slot = scheduler::install_cap_for(reboot_test_id, reboot_control);
    debug_assert_eq!(reboot_control_slot, 3, "reboot_test's RebootControl must land at CSlot 3");

    println!(
        "[ \x1b[1;32mok\x1b[0m ] reboot_test: spawned reboot_test (id={})",
        reboot_test_id
    );
}

/// Checkpoint W: finds the RTL8139 NIC via PCI enumeration (kernel/src/
/// pci.rs) -- `None` gracefully if none is attached (QEMU only emulates
/// one when told to via `-device rtl8139`), the same "hardware not
/// present" tolerance fs_fat32 already has for "no disk" -- enables it,
/// and spawns its driver at `module_index` with exactly the capabilities
/// it needs:
///
/// - The discovered I/O-port range via the existing `allowed_ports`
///   mechanism (no new capability kind needed -- ports are still gated
///   the same way console_server's/storage_ata's fixed ones are; this
///   range just isn't known until runtime).
/// - An `IrqControl` for the discovered PCI interrupt line (CSlot 3),
///   the same capability kind console_server's fixed IRQ1 uses.
/// - A read-only `MemoryGrant` (CSlot 4) over a single physical page this
///   function writes the discovered I/O base into -- the mechanism the
///   driver needs to *learn* that runtime-discovered value at all, since
///   (unlike every other hand-wired hardware capability so far) it isn't
///   a fixed legacy address both sides can just agree on at compile time.
///   Exposing a value through a capability that already exists for a
///   different purpose (bulk memory sharing) rather than adding a new
///   syscall just to pass one integer at spawn time.
///
/// Called last (after every other deterministically-spawned task) in
/// both the production boot and the standalone `nic_test` harness, so it
/// always lands at the same task id -- 6 -- when it lands at all;
/// nameservice's own ALLOWLIST hardcodes that id to the name "net". This
/// function returning `None` without ever calling `spawn_from_module`
/// consumes no task id, so as long as nothing is spawned after it, its
/// absence simply leaves id 6 unallocated instead of letting the next
/// spawn silently slide into it -- see the call site's own comment for
/// why "spawned last" is the actual fix, not just a convention.
fn spawn_net_rtl8139(module_index: usize) -> Option<task::TaskId> {
    use alloc::vec::Vec;

    let nic = pci::find_device(0x10EC, 0x8139)?;
    nic.enable();
    let io_base = (nic.bar0() & 0xFFFC) as u16;
    let irq = nic.interrupt_line();

    let allowed_ports: Vec<u16> = (io_base..io_base.saturating_add(256)).collect();
    let nic_id = loader::spawn_from_module(module_index, &allowed_ports)?;
    // See the identical assert on console_id earlier in this file --
    // nameservice's ALLOWLIST hardcodes this task id to "net".
    assert_eq!(nic_id, 6, "nameservice's ALLOWLIST assumes net_rtl8139 is task id 6");
    println!(
        "[ \x1b[1;32mok\x1b[0m ] spawned ring-3 task 'net_rtl8139' (id={}, pci={:#06x}:{:#06x}, io_base={:#06x}, irq={})",
        nic_id, nic.vendor_id, nic.device_id, io_base, irq
    );
    let nic_endpoint = ipc::create_endpoint(nic_id);
    grant_endpoint_cap(nic_id, nic_endpoint); // its CSlot 2: its own inbox

    let irq_control = cap::mint_root(cap::CapKind::IrqControl { irq: irq as u32, endpoint: nic_endpoint });
    let irq_slot = scheduler::install_cap_for(nic_id, irq_control);
    debug_assert_eq!(irq_slot, 3, "net_rtl8139's IrqControl must land at CSlot 3");

    // A fresh physical page carrying nothing but the discovered I/O base
    // (as a little-endian u32 at offset 0) -- see this function's own
    // doc comment for why a MemoryGrant, not a new capability kind,
    // carries it across.
    let info_phys = mm::frame::alloc_frame().expect("out of memory for net_rtl8139's info page");
    let info_virt = mm::paging::phys_to_virt(info_phys) as *mut u32;
    unsafe { info_virt.write(io_base as u32) };
    let info_grant = cap::mint_root(cap::CapKind::MemoryGrant {
        phys_base: info_phys,
        len: mm::frame::FRAME_SIZE,
        writable: false,
    });
    let info_slot = scheduler::install_cap_for(nic_id, info_grant);
    debug_assert_eq!(info_slot, 4, "net_rtl8139's I/O-base info grant must land at CSlot 4");

    // No explicit `pic::unmask(irq)` here: `irq::register` was already
    // called by the driver's own startup code by the time it first calls
    // `recv`, and `ipc::recv` unmasks any IRQ registered to the endpoint
    // it's called on right then -- see its own doc comment for why that's
    // the right moment, not "as soon as this function hands out the
    // capability," to first let this line through.
    Some(nic_id)
}

/// Spawns `nic_test` (see `userland/cap_test`), Checkpoint W's regression
/// fixture for the RTL8139 driver, only present in a kernel built with
/// `--features nic_test` (see `make test-nic`) -- `grub-nictest.cfg` is
/// the one grub config whose module list matches the indices below.
/// Unlike every other `*_test_spawn` here, this one still needs the full
/// nameservice/console_server/storage_ata/fs_fat32 stack the shared code
/// above always spawns (module indices 0-3 in every build). `nic_test`
/// itself is spawned first (module index 4, task id 5, no hand-wired
/// capabilities beyond the usual CSlot 1/2 convention -- it reaches the
/// driver by looking up "net" through the name service, retrying until
/// it appears), then `spawn_net_rtl8139` last (module index 5, task id
/// 6): the same "optional thing spawned last" ordering the production
/// boot uses, just with `.expect()` instead of the graceful `None`
/// tolerance production boot uses, since this harness's whole point is
/// exercising the driver and a missing `-device rtl8139` here is a test
/// setup bug, not a real "no NIC" boot.
#[cfg(feature = "nic_test")]
fn nic_test_spawn() {
    let nic_test_id = loader::spawn_from_module(4, &[]).expect("no multiboot module 4 found for 'nic_test'");
    let nic_test_endpoint = ipc::create_endpoint(nic_test_id);
    grant_endpoint_cap(nic_test_id, nic_test_endpoint); // its CSlot 2: its own inbox

    spawn_net_rtl8139(5).expect("nic_test requires -device rtl8139 in this boot's QEMU invocation");

    println!(
        "[ \x1b[1;32mok\x1b[0m ] nic_test: spawned nic_test (id={})",
        nic_test_id
    );
}

extern "C" fn idle_task() -> ! {
    loop {
        unsafe { core::arch::asm!("hlt") };
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
