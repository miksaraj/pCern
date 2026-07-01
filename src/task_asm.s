# void switch_to(usize *old_esp, usize new_esp) -- cdecl
#
# Saves the four callee-saved GP registers onto the outgoing task's stack,
# stashes the resulting esp through `old_esp`, then loads `new_esp` and pops
# the same four registers back off -- resuming whatever task last suspended
# itself here (or, for a brand new task, "returning" into task_trampoline
# via the fabricated initial stack built in task.rs).
.global switch_to
.type switch_to, @function
switch_to:
    push ebp
    push ebx
    push esi
    push edi

    mov eax, [esp + 20]      # old_esp (4 pushed regs + return addr = +20)
    mov [eax], esp

    mov eax, [esp + 24]      # new_esp
    mov esp, eax

    pop edi
    pop esi
    pop ebx
    pop ebp
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
