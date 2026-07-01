use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::gdt;
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
pub fn block_current() -> TaskId {
    let (blocked_id, old_esp_ptr, new_esp) = {
        let mut sched = SCHEDULER.lock();
        let current_id = sched.current.expect("block_current with no current task");
        let current_idx = sched.index_of(current_id);
        sched.tasks[current_idx].state = TaskState::Blocked;

        let next_id = sched
            .ready_queue
            .pop_front()
            .expect("no other tasks to run while blocking");
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
pub fn exit_current(exit_code: i32) -> ! {
    let (id, new_esp) = {
        let mut sched = SCHEDULER.lock();
        let current_id = sched.current.expect("exit with no current task");
        let current_idx = sched.index_of(current_id);
        sched.tasks[current_idx].state = TaskState::Zombie;

        let next_id = sched
            .ready_queue
            .pop_front()
            .expect("no other tasks to run after exit");
        let next_idx = sched.index_of(next_id);
        sched.tasks[next_idx].state = TaskState::Running;
        sched.current = Some(next_id);
        sched.activate(next_idx);

        (current_id, sched.tasks[next_idx].esp)
    };

    crate::println!("[ task {} exited with code {} ]", id, exit_code);

    let mut discard: usize = 0;
    unsafe { switch_to(&mut discard as *mut usize, new_esp) };
    unreachable!("a zombie task resumed");
}
