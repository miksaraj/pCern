# Syscall entry point for `int 0x80` (IDT vector 0x80, DPL 3 -- see idt.rs).
#
# Unlike the other interrupt handlers, this one is hand-written rather than
# an `extern "x86-interrupt" fn`: Rust's x86-interrupt calling convention
# doesn't expose the general-purpose registers to the handler, only the
# eip/cs/eflags trap frame, and the syscall ABI needs those (both to read
# the caller's arguments and, for recv, to write back multiple results).
#
# Fix: push all 7 GP registers, then pass syscall_dispatch a single pointer
# to that block (matching struct SavedRegs in syscall.rs field-for-field).
# It reads eax as the syscall number and ebx.. as arguments, and can freely
# overwrite any of them in place as return values -- the final pops send
# whatever it left there back to the caller.
.global syscall_isr
.type syscall_isr, @function
syscall_isr:
    push ebp
    push edi
    push esi
    push edx
    push ecx
    push ebx
    push eax

    push esp
    call syscall_dispatch
    add esp, 4

    pop eax
    pop ebx
    pop ecx
    pop edx
    pop esi
    pop edi
    pop ebp
    iretd
.size syscall_isr, . - syscall_isr
