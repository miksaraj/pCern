; Minimal flat ring-3 test program for Checkpoint 4: proves user code can
; run and reach the kernel only through syscalls (int 0x80), not by doing
; privileged things directly. Loaded by the kernel as a GRUB multiboot
; module and mapped at USER_CODE_BASE (see main.rs).
;
; Assembled as a raw flat binary (no ELF headers) -- position-dependent,
; hence `org` matching where the kernel maps it.
BITS 32
org 0x00400000

SYS_EXIT        equ 0
SYS_DEBUG_WRITE equ 5

_start:
    mov eax, SYS_DEBUG_WRITE
    mov ebx, msg
    mov ecx, msg_len
    int 0x80

    mov eax, SYS_EXIT
    mov ebx, 0
    int 0x80

.hang:                  ; sys_exit never returns; only reached if it somehow did
    jmp .hang

msg:     db "Hello from ring 3! Syscalls work.", 10
msg_len equ $ - msg
