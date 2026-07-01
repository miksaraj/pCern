; Verification-only variant of ring3_test.asm: attempts a privileged
; instruction (cli) from ring 3, which must fault with #GP rather than
; actually disabling interrupts -- proof that ring-3 isolation is real and
; not just cosmetic. Not wired into the default build; swapped in manually
; via grub.cfg for a one-off check, then swapped back.
BITS 32
org 0x00400000

SYS_DEBUG_WRITE equ 5

_start:
    mov eax, SYS_DEBUG_WRITE
    mov ebx, msg
    mov ecx, msg_len
    int 0x80

    cli                 ; privileged instruction at CPL=3 -> must #GP

.hang:
    jmp .hang

msg:     db "About to attempt cli from ring 3...", 10
msg_len equ $ - msg
