; Checkpoint B verification only (not wired into the default build):
; registers for IRQ1 (keyboard) and prints every scancode it receives via
; the new interrupt-forwarding path, proving the kernel's minimal-ack ISR
; correctly notifies a registered userspace task -- alongside (not instead
; of) the kernel's own still-active keyboard echo, kept as a safety net
; until Checkpoint D.
BITS 32
org 0x00400000

SYS_REGISTER_IRQ equ 6
SYS_RECV         equ 3
SYS_DEBUG_WRITE  equ 5

_start:
    mov eax, SYS_REGISTER_IRQ
    mov ebx, 1              ; IRQ1 = keyboard
    int 0x80

.loop:
    mov eax, SYS_RECV
    mov ebx, 0               ; filter = 0 (kernel/any)
    int 0x80
    ; eax = sender (0 = kernel), ebx = irq number, ecx = scancode

    mov edx, ecx              ; scancode, saved across the hex_digit calls

    mov eax, edx
    shr eax, 4
    and eax, 0x0F
    call hex_digit
    mov [digit_hi], al

    mov eax, edx
    and eax, 0x0F
    call hex_digit
    mov [digit_lo], al

    mov eax, SYS_DEBUG_WRITE
    mov ebx, msg
    mov ecx, msg_len
    int 0x80

    jmp .loop

; al (0-15) -> ascii hex digit in al
hex_digit:
    cmp al, 10
    jl .digit
    add al, 'A' - 10
    ret
.digit:
    add al, '0'
    ret

msg:      db "[irq_test] scancode=0x"
digit_hi: db "0"
digit_lo: db "0"
          db 10
msg_end:
msg_len equ msg_end - msg
