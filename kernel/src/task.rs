use alloc::boxed::Box;
use alloc::vec::Vec;
use core::arch::global_asm;

use crate::cap::{CSlot, CapNodeId};
use crate::mm::paging;
use crate::sync::Mutex;

global_asm!(include_str!("task_asm.s"));

pub type TaskId = usize;

/// A task's private capability table: `CSlot` (the handle syscalls take)
/// is just an index into this. Unforgeable because the kernel is the only
/// thing that ever pushes an entry -- userspace can hold a slot number but
/// can never manufacture one that resolves to something it wasn't actually
/// granted.
pub struct CSpace {
    slots: Vec<Option<CapNodeId>>,
}

impl CSpace {
    /// Slot 0 is always empty, mirroring the "0 is a reserved sentinel"
    /// convention already used for `ipc::KERNEL_TASK_ID`.
    pub fn new() -> CSpace {
        CSpace { slots: alloc::vec![None] }
    }

    /// Installs `node` into the next free slot and returns its index.
    pub fn install(&mut self, node: CapNodeId) -> CSlot {
        self.slots.push(Some(node));
        (self.slots.len() - 1) as CSlot
    }

    pub fn get(&self, slot: CSlot) -> Option<CapNodeId> {
        self.slots.get(slot as usize).copied().flatten()
    }
}

const KERNEL_STACK_SIZE: usize = 16 * 1024;
/// Reserved bit 1 (always 1) + IF: a freshly created task starts with
/// interrupts enabled, matching switch_to's popfd expecting an EFLAGS slot
/// in the fabricated initial stack (see task_asm.s).
const INITIAL_EFLAGS: usize = 0x202;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TaskState {
    Ready,
    Running,
    /// Waiting on something outside the scheduler's rotation (currently
    /// only IPC send/recv -- see ipc.rs, which tracks the specifics of
    /// what it's waiting for separately and calls `scheduler::wake` when
    /// that's resolved).
    Blocked,
    Zombie,
}

pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    /// Saved stack pointer; only meaningful while this task isn't the one
    /// currently executing (see scheduler::tick/start).
    pub esp: usize,
    /// Physical address of this task's page directory. Kernel-mode tasks
    /// all share the boot bootstrap directory (no isolation needed between
    /// them); ring-3 tasks get their own from `mm::paging::PageDirectory`.
    pub page_dir_phys: usize,
    /// I/O ports this task may access directly from ring 3; installed into
    /// the TSS I/O bitmap by `gdt::set_io_permissions` whenever the
    /// scheduler switches to this task. Empty for ordinary tasks.
    pub allowed_ports: Vec<u16>,
    /// This task's capability table -- see `CSpace` above.
    pub cspace: CSpace,
    // Kept only to own the stack allocation for the task's lifetime; also
    // doubles as this task's ring0 stack (TSS.esp0) once it's ring 3.
    #[allow(dead_code)]
    stack: Box<[u8]>,
}

extern "C" fn task_trampoline(entry: extern "C" fn() -> !) -> ! {
    entry()
}

extern "C" {
    fn enter_ring3(entry_eip: u32, user_esp: u32) -> !;
}

static NEXT_ID: Mutex<TaskId> = Mutex::new(1);

fn alloc_task_id() -> TaskId {
    let mut id = NEXT_ID.lock();
    let this = *id;
    *id += 1;
    this
}

/// Builds a fresh kernel stack and hands back both the boxed allocation and
/// its top address; shared by the kernel- and user-task constructors below.
fn new_stack() -> (Box<[u8]>, usize) {
    let mut stack = alloc::vec![0u8; KERNEL_STACK_SIZE].into_boxed_slice();
    let top = unsafe { stack.as_mut_ptr().add(KERNEL_STACK_SIZE) } as usize;
    (stack, top)
}

impl Task {
    pub fn kernel_stack_top(&self) -> usize {
        self.stack.as_ptr() as usize + self.stack.len()
    }

    /// Builds a new kernel-mode task ready to run `entry` (which must never
    /// return). The initial stack is hand-constructed to look exactly like
    /// a task that already called into `switch_to` and is about to `ret`
    /// into `task_trampoline`: 4 saved registers, then a saved EFLAGS, then
    /// `task_trampoline`'s address, then task_trampoline's own (unused)
    /// return slot, then its real cdecl argument.
    pub fn new_kernel(entry: extern "C" fn() -> !) -> Task {
        let (stack, stack_top) = new_stack();

        let mut sp = stack_top;
        let mut push = |value: usize| {
            sp -= 4;
            unsafe { (sp as *mut usize).write(value) };
        };
        // `ret` (in switch_to) consumes the task_trampoline slot as its jump
        // target rather than leaving it on the stack, so task_trampoline
        // needs its own (never used -- it never returns) dummy return
        // address underneath before its real cdecl argument.
        push(entry as usize); // task_trampoline's argument, ends up at [esp+4]
        push(0); // task_trampoline's own "return address" (unused)
        push(task_trampoline as *const () as usize); // switch_to's `ret` target
        push(INITIAL_EFLAGS); // popfd
        push(0); // ebp
        push(0); // ebx
        push(0); // esi
        push(0); // edi

        Task {
            id: alloc_task_id(),
            state: TaskState::Ready,
            esp: sp,
            page_dir_phys: paging::boot_page_directory_phys(),
            allowed_ports: Vec::new(),
            cspace: CSpace::new(),
            stack,
        }
    }

    /// Builds a new ring-3 task that starts executing at `entry_eip` (a
    /// virtual address in `page_dir_phys`'s address space) with `user_esp`
    /// as its initial user stack pointer. Same fabricated-stack trick as
    /// `new_kernel`, but `ret`s into `enter_ring3` (task_asm.s) instead,
    /// which loads user segments and `iret`s into ring 3.
    ///
    /// `allowed_ports` is only ever set by callers within the kernel
    /// itself (main.rs's own spawn calls) -- the create_task syscall
    /// always passes `&[]`, so untrusted code can never grant itself or a
    /// child task port access. Memory and IRQ access are gated separately,
    /// through capabilities (see cap.rs) rather than a spawn-time flag --
    /// holding a valid `MemoryGrant`/`IrqControl` capability is itself
    /// sufficient authorization, the same way holding a port doesn't need
    /// a second "and are you actually allowed to" check.
    pub fn new_user(entry_eip: u32, user_esp: u32, page_dir_phys: usize, allowed_ports: &[u16]) -> Task {
        let (stack, stack_top) = new_stack();

        let mut sp = stack_top;
        let mut push = |value: usize| {
            sp -= 4;
            unsafe { (sp as *mut usize).write(value) };
        };
        push(user_esp as usize); // enter_ring3's 2nd argument
        push(entry_eip as usize); // enter_ring3's 1st argument
        push(0); // enter_ring3's own (unused) return slot
        push(enter_ring3 as *const () as usize); // switch_to's `ret` target
        push(INITIAL_EFLAGS); // popfd
        push(0); // ebp
        push(0); // ebx
        push(0); // esi
        push(0); // edi

        Task {
            id: alloc_task_id(),
            state: TaskState::Ready,
            esp: sp,
            page_dir_phys,
            allowed_ports: allowed_ports.to_vec(),
            cspace: CSpace::new(),
            stack,
        }
    }
}
