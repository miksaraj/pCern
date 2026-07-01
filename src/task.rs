use alloc::boxed::Box;
use core::arch::global_asm;

use crate::mm::paging;
use crate::sync::Mutex;

global_asm!(include_str!("task_asm.s"));

pub type TaskId = usize;

const KERNEL_STACK_SIZE: usize = 16 * 1024;

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
    /// into `task_trampoline`: 4 saved registers, then `task_trampoline`'s
    /// address, then task_trampoline's own (unused) return slot, then its
    /// real cdecl argument.
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
        push(0); // ebp
        push(0); // ebx
        push(0); // esi
        push(0); // edi

        Task {
            id: alloc_task_id(),
            state: TaskState::Ready,
            esp: sp,
            page_dir_phys: paging::boot_page_directory_phys(),
            stack,
        }
    }

    /// Builds a new ring-3 task that starts executing at `entry_eip` (a
    /// virtual address in `page_dir_phys`'s address space) with `user_esp`
    /// as its initial user stack pointer. Same fabricated-stack trick as
    /// `new_kernel`, but `ret`s into `enter_ring3` (task_asm.s) instead,
    /// which loads user segments and `iret`s into ring 3.
    pub fn new_user(entry_eip: u32, user_esp: u32, page_dir_phys: usize) -> Task {
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
        push(0); // ebp
        push(0); // ebx
        push(0); // esi
        push(0); // edi

        Task {
            id: alloc_task_id(),
            state: TaskState::Ready,
            esp: sp,
            page_dir_phys,
            stack,
        }
    }
}
