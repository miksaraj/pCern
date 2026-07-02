//! Loads multiboot modules as ring-3 tasks. Used both by main.rs's initial
//! spawns and by the create_task syscall (see syscall.rs) -- the latter is
//! why this owns its own copy of the multiboot info (set once at boot via
//! `init`) rather than requiring every caller to thread it through.

use crate::cap::{self, EndpointId};
use crate::mm;
use crate::multiboot::MultibootInfo;
use crate::sync::Mutex;
use crate::task::{self, TaskId};

/// Where every ring-3 task's code gets mapped, and where its stack lives.
/// Arbitrary but page-aligned and clear of the kernel's own 0xC0000000+
/// range; every task gets its own fresh address space, so reusing the same
/// virtual layout for each is fine.
const USER_CODE_BASE: usize = 0x0040_0000;
const USER_STACK_TOP: usize = 0x0080_0000;
const USER_STACK_PAGES: usize = 4;

static MB_INFO: Mutex<Option<MultibootInfo>> = Mutex::new(None);

/// The name service's own endpoint, once it exists (see
/// `set_nameservice_endpoint`) -- `None` for the one spawn call (the name
/// service itself) that necessarily happens before this can be set.
/// Every spawn after that automatically gets a capability to it installed
/// at `libpcern::NAMESERVICE_SLOT`, whether spawned directly by main.rs or
/// later via the create_task syscall -- discovery shouldn't depend on
/// which path created a task.
static NAMESERVICE_ENDPOINT: Mutex<Option<EndpointId>> = Mutex::new(None);

pub fn init(mb_info: MultibootInfo) {
    *MB_INFO.lock() = Some(mb_info);
}

/// Called once by main.rs right after spawning and wiring up the name
/// service itself. Every `spawn_from_module` call from then on installs a
/// fresh capability to this same endpoint into the new task's CSpace.
pub fn set_nameservice_endpoint(endpoint: EndpointId) {
    *NAMESERVICE_ENDPOINT.lock() = Some(endpoint);
}

/// Reserves every module's physical range up front, before any task
/// allocation happens: modules are only a page apart in memory, so
/// allocating frames for one task while a not-yet-processed module's
/// bytes are still unreserved risks handing out (and clobbering) exactly
/// the frame the next module's own code is sitting in.
pub fn reserve_all_modules() {
    let mb_info = MB_INFO.lock();
    let mb_info = mb_info.as_ref().expect("loader::init not called yet");
    for i in 0..mb_info.module_count() {
        if let Some(m) = mb_info.module(i) {
            mm::frame::reserve_range(m.start, m.end);
        }
    }
}

/// Loads multiboot module `index` as a flat, position-dependent ring-3
/// program: maps it at `USER_CODE_BASE` in a fresh address space, gives it
/// a small stack, and spawns it. Returns `None` if there aren't that many
/// modules.
///
/// `allowed_ports` grants port access -- callers reachable from untrusted
/// code (the create_task syscall) must always pass `&[]`, so untrusted
/// code can never grant itself or a child task port access. Memory/IRQ
/// access aren't spawn-time flags at all -- see cap.rs -- so there's
/// nothing analogous to gate here for those.
pub fn spawn_from_module(index: usize, allowed_ports: &[u16]) -> Option<TaskId> {
    let module = {
        let mb_info = MB_INFO.lock();
        mb_info.as_ref().expect("loader::init not called yet").module(index)?
    };

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

    let task = task::Task::new_user(USER_CODE_BASE as u32, USER_STACK_TOP as u32, page_dir.phys_addr(), allowed_ports);
    let task_id = crate::scheduler::spawn(task);

    if let Some(ns_endpoint) = *NAMESERVICE_ENDPOINT.lock() {
        let node = cap::mint_root(cap::CapKind::Endpoint { id: ns_endpoint });
        let slot = crate::scheduler::install_cap_for(task_id, node);
        // Must match userland/libpcern's NAMESERVICE_SLOT constant -- this
        // is always the very first capability installed for a task other
        // than the name service itself, so it always lands here.
        debug_assert_eq!(slot, 1, "name service capability must land at CSlot 1");
    }

    Some(task_id)
}
