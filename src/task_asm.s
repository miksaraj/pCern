# void switch_to(usize *old_esp, usize new_esp) -- cdecl
#
# Saves EFLAGS and the four callee-saved GP registers onto the outgoing
# task's stack, stashes the resulting esp through `old_esp`, then loads
# `new_esp` and pops the same values back off -- resuming whatever task
# last suspended itself here (or, for a brand new task, "returning" into
# task_trampoline/enter_ring3 via the fabricated initial stack built in
# task.rs).
#
# EFLAGS (specifically IF) MUST be part of the saved context here: IF is a
# single machine-wide flag, not something the CPU tracks per stack. A task
# switch driven by the timer IRQ always runs with IF=0 (interrupt gates
# clear it on entry), and a switch away from a task always leaves whatever
# IF value was current at that moment; without pushfd/popfd here, resuming
# a task that was last suspended via a *voluntary* yield/IPC block (a plain
# call/ret chain, not an interrupt, so there's no iretd to restore eflags)
# would silently inherit IF=0 from an unrelated preempting context and
# never re-enable interrupts again -- the timer and keyboard would go dead
# kernel-wide the first time that combination of switches occurred.
.global switch_to
.type switch_to, @function
switch_to:
    pushfd
    push ebp
    push ebx
    push esi
    push edi

    mov eax, [esp + 24]      # old_esp (5 pushed words + return addr = +24)
    mov [eax], esp

    mov eax, [esp + 28]      # new_esp
    mov esp, eax

    pop edi
    pop esi
    pop ebx
    pop ebp
    popfd
    ret
.size switch_to, . - switch_to

# void enter_ring3(u32 entry_eip, u32 user_esp) -- cdecl, never returns.
#
# Only ever reached the same way task_trampoline is: switch_to's `ret`
# jumping into a task's fabricated initial stack the very first time it
# runs (task.rs's new_user). Loads the ring-3 data segment selectors and
# `iret`s into user code -- the CR3 switch to this task's own page
# directory already happened in the scheduler before switch_to was called.
.global enter_ring3
.type enter_ring3, @function
enter_ring3:
    mov eax, [esp + 4]      # entry_eip
    mov ecx, [esp + 8]      # user_esp

    mov bx, 0x23             # USER_DATA_SEG (0x20 | RPL 3)
    mov ds, bx
    mov es, bx
    mov fs, bx
    mov gs, bx

    push 0x23                # ss
    push ecx                 # esp
    push 0x202                # eflags, IF set
    push 0x1B                 # cs (USER_CODE_SEG = 0x18 | RPL 3)
    push eax                  # eip
    iretd
.size enter_ring3, . - enter_ring3
