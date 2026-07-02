use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::cap::{CSlot, CapNodeId};
use crate::gdt;
use crate::ipc;
use crate::mm::paging;
use crate::sync::Mutex;
use crate::task::{Task, TaskId, TaskState};

extern "C" {
    fn switch_to(old_esp: *mut usize, new_esp: usize);
}

struct Scheduler {
    tasks: Vec<Task>,
    ready_queue: VecDeque<TaskId>,
    current: Option<TaskId>,
}

impl Scheduler {
    fn index_of(&self, id: TaskId) -> usize {
        self.tasks.iter().position(|t| t.id == id).expect("unknown task id")
    }

    /// Points the TSS and CR3 at whichever task is about to run. Must run
    /// before `switch_to` -- both stay valid on the *current* stack/code
    /// (every page directory maps the kernel identically), so this is safe
    /// even though the actual stack switch hasn't happened yet.
    fn activate(&self, idx: usize) {
        gdt::set_kernel_stack(self.tasks[idx].kernel_stack_top() as u32);
        gdt::set_io_permissions(&self.tasks[idx].allowed_ports);
        unsafe { paging::activate_phys(self.tasks[idx].page_dir_phys) };
    }
}

static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler {
    tasks: Vec::new(),
    ready_queue: VecDeque::new(),
    current: None,
});

/// Registers a new kernel-mode task; it starts out `Ready` and joins the
/// round-robin rotation the next time `tick()`/`start()` picks a task.
pub fn spawn_kernel_task(entry: extern "C" fn() -> !) -> TaskId {
    spawn(Task::new_kernel(entry))
}

/// Registers an already-built task (e.g. a ring-3 task from
/// `Task::new_user`); same rotation as `spawn_kernel_task`.
pub fn spawn(task: Task) -> TaskId {
    let id = task.id;
    let mut sched = SCHEDULER.lock();
    sched.ready_queue.push_back(id);
    sched.tasks.push(task);
    id
}

pub fn current_id() -> Option<TaskId> {
    SCHEDULER.lock().current
}

/// Whether the currently running task is kernel-flagged as a driver (see
/// task.rs) -- gates the privileged syscalls (map_memory,
/// register_for_interrupt) in syscall.rs.
pub fn current_is_driver() -> bool {
    let sched = SCHEDULER.lock();
    match sched.current {
        Some(id) => sched.tasks[sched.index_of(id)].is_driver,
        None => false,
    }
}

/// Physical address of the currently running task's own page directory --
/// needed so a syscall (e.g. map_memory) can map more pages into that same
/// still-active address space via `mm::paging::PageDirectory::from_phys`.
pub fn current_page_dir_phys() -> usize {
    let sched = SCHEDULER.lock();
    let id = sched.current.expect("syscall with no current task");
    sched.tasks[sched.index_of(id)].page_dir_phys
}

/// Installs `node` into the currently running task's own capability table
/// and returns the slot it landed in -- used by syscalls that mint a new
/// capability for the caller (e.g. `SYS_ENDPOINT_CREATE`).
pub fn current_cspace_install(node: CapNodeId) -> CSlot {
    let mut sched = SCHEDULER.lock();
    let id = sched.current.expect("syscall with no current task");
    let idx = sched.index_of(id);
    sched.tasks[idx].cspace.install(node)
}

/// Resolves a capability slot in the currently running task's own table --
/// used by syscalls that take a capability as an argument (e.g. `SYS_SEND`).
pub fn current_cspace_get(slot: CSlot) -> Option<CapNodeId> {
    let sched = SCHEDULER.lock();
    let id = sched.current.expect("syscall with no current task");
    sched.tasks[sched.index_of(id)].cspace.get(slot)
}

/// Installs `node` into an arbitrary (not necessarily current) task's
/// capability table. Only meant for trusted kernel-side code doing initial
/// capability wiring at boot (see main.rs) -- there's no syscall exposing
/// this, since a task granting itself or others arbitrary capabilities
/// would defeat the entire point of the capability system.
pub fn install_cap_for(task_id: TaskId, node: CapNodeId) -> CSlot {
    let mut sched = SCHEDULER.lock();
    let idx = sched.index_of(task_id);
    sched.tasks[idx].cspace.install(node)
}

/// Hands control to the scheduler: picks the first ready task and switches
/// onto it. Never returns -- the caller's own stack/registers are discarded,
/// so anything that must keep running has to be spawned as a task first.
pub fn start() -> ! {
    let new_esp = {
        let mut sched = SCHEDULER.lock();
        let id = sched.ready_queue.pop_front().expect("no tasks to run");
        let idx = sched.index_of(id);
        sched.tasks[idx].state = TaskState::Running;
        sched.current = Some(id);
        sched.activate(idx);
        sched.tasks[idx].esp
    };

    let mut discard: usize = 0;
    unsafe { switch_to(&mut discard as *mut usize, new_esp) };
    unreachable!("a task's entry function returned");
}

/// Called from the timer IRQ handler. Rotates the currently running task to
/// the back of the ready queue and switches to the next one; a no-op if
/// scheduling hasn't started yet or there's nothing else ready to run.
pub fn tick() {
    let switch = {
        let mut sched = SCHEDULER.lock();
        let (Some(current_id), false) = (sched.current, sched.ready_queue.is_empty()) else {
            return;
        };

        let current_idx = sched.index_of(current_id);
        sched.ready_queue.push_back(current_id);
        sched.tasks[current_idx].state = TaskState::Ready;

        let next_id = sched.ready_queue.pop_front().unwrap();
        let next_idx = sched.index_of(next_id);
        sched.tasks[next_idx].state = TaskState::Running;
        sched.current = Some(next_id);
        sched.activate(next_idx);

        // Guard is dropped right after this block: switch_to below will not
        // return until this task is scheduled again, possibly much later,
        // and holding the lock across that would deadlock every other task
        // and interrupt that needs the scheduler in the meantime.
        let old_esp_ptr = &mut sched.tasks[current_idx].esp as *mut usize;
        let new_esp = sched.tasks[next_idx].esp;
        (old_esp_ptr, new_esp)
    };

    let (old_esp_ptr, new_esp) = switch;
    unsafe { switch_to(old_esp_ptr, new_esp) };
}

/// Voluntarily gives up the CPU to the next ready task. Shares `tick()`'s
/// rotation logic -- rotating "the currently running task" makes just as
/// much sense whether the caller is the timer IRQ or ordinary task code.
pub fn yield_now() {
    tick();
}

/// Removes the current task from the ready rotation (marking it `Blocked`,
/// e.g. waiting on an IPC rendezvous -- see ipc.rs) and switches to the
/// next ready task. Returns the id of the task that just blocked (i.e. the
/// caller) once something calls `wake` on it and it runs again.
///
/// Relies on `main.rs` always keeping a permanent idle task in the
/// rotation (one that never itself blocks or exits) so the ready queue is
/// never empty here -- see the `.expect()` below.
pub fn block_current() -> TaskId {
    let (blocked_id, old_esp_ptr, new_esp) = {
        let mut sched = SCHEDULER.lock();
        let current_id = sched.current.expect("block_current with no current task");
        let current_idx = sched.index_of(current_id);
        sched.tasks[current_idx].state = TaskState::Blocked;

        let next_id = sched
            .ready_queue
            .pop_front()
            .expect("no other tasks to run while blocking (missing idle task?)");
        let next_idx = sched.index_of(next_id);
        sched.tasks[next_idx].state = TaskState::Running;
        sched.current = Some(next_id);
        sched.activate(next_idx);

        let old_esp_ptr = &mut sched.tasks[current_idx].esp as *mut usize;
        let new_esp = sched.tasks[next_idx].esp;
        (current_id, old_esp_ptr, new_esp)
    };

    unsafe { switch_to(old_esp_ptr, new_esp) };
    blocked_id
}

/// Moves a `Blocked` task back into the ready rotation. Does not itself
/// switch to it -- the caller keeps running until it next yields/is
/// preempted, same as any other reschedule point.
pub fn wake(task_id: TaskId) {
    let mut sched = SCHEDULER.lock();
    let idx = sched.index_of(task_id);
    sched.tasks[idx].state = TaskState::Ready;
    sched.ready_queue.push_back(task_id);
}

/// Ends the current task for good: marks it `Zombie` (its slot is simply
/// never scheduled again -- no cleanup of its stack/page directory yet,
/// acceptable for the small, short-lived test tasks this kernel runs today)
/// and switches to whatever's next. Never returns.
///
/// Relies on `main.rs` always keeping a permanent idle task in the
/// rotation (one that never itself blocks or exits) so the ready queue is
/// never empty here -- see the `.expect()` below.
pub fn exit_current(exit_code: i32) -> ! {
    let (id, new_esp) = {
        let mut sched = SCHEDULER.lock();
        let current_id = sched.current.expect("exit with no current task");
        let current_idx = sched.index_of(current_id);
        sched.tasks[current_idx].state = TaskState::Zombie;

        let next_id = sched
            .ready_queue
            .pop_front()
            .expect("no other tasks to run after exit (missing idle task?)");
        let next_idx = sched.index_of(next_id);
        sched.tasks[next_idx].state = TaskState::Running;
        sched.current = Some(next_id);
        sched.activate(next_idx);

        (current_id, sched.tasks[next_idx].esp)
    };

    // Wake (with a failure indication) anything blocked send/recv-ing with
    // this task, before it's gone for good -- otherwise a partner waiting
    // on a message from `id` would stay Blocked forever. Must run after the
    // SCHEDULER guard above has dropped: task_exited calls wake(), which
    // locks SCHEDULER itself.
    ipc::task_exited(id);
    crate::println!("[ task {} exited with code {} ]", id, exit_code);

    let mut discard: usize = 0;
    unsafe { switch_to(&mut discard as *mut usize, new_esp) };
    unreachable!("a zombie task resumed");
}
