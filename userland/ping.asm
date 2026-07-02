; Checkpoint 5 IPC test, half A: sends an incrementing counter to `pong`
; (task id 5 -- see main.rs, which spawns console_server, task_a, task_b,
; ping, then pong in that fixed order) and waits for its reply, 5 rounds,
; proving synchronous rendezvous send/recv works correctly across two
; separate address spaces.
;
; Checkpoint D: output goes through the userspace console server (task id
; 1) instead of the kernel's now-removed debug_write syscall -- see
; print_msg below.
;
; Note: only eax/ebx/ecx/edx have byte sub-registers (al/bl/cl/dl) in
; 32-bit mode -- esi/edi/ebp do not (that needs a REX prefix, x86-64 only)
; -- so the round counter is kept in edx/ecx rather than esi/ebp.
BITS 32
org 0x00400000

SYS_EXIT equ 0
SYS_SEND equ 2
SYS_RECV equ 3

CONSOLE_TASK_ID equ 1
OP_PUTCHAR      equ 0
PONG_TASK_ID    equ 5
ROUNDS          equ 5

_start:
    xor edx, edx            ; round counter

.loop:
    ; send(dest=PONG_TASK_ID, w0=round, 0, 0, 0); dispatcher only touches
    ; eax for a successful send, so edx (round) survives this untouched.
    mov eax, SYS_SEND
    mov ebx, PONG_TASK_ID
    mov ecx, edx
    xor esi, esi
    xor edi, edi
    int 0x80

    ; recv(filter=PONG_TASK_ID) -> ebx = reply word; recv overwrites
    ; ebx/ecx/edx/esi as return slots, so save the round counter first.
    push edx
    mov eax, SYS_RECV
    mov ebx, PONG_TASK_ID
    int 0x80
    mov ecx, ebx             ; ecx = reply value
    pop edx                  ; edx = round, restored

    mov al, cl
    add al, '0'
    mov [digit_reply], al
    mov al, dl
    add al, '0'
    mov [digit_round], al

    mov dword [print_ptr], msg_part1
    mov dword [print_len], msg_len
    call print_msg

    inc edx
    cmp edx, ROUNDS
    jl .loop

    mov eax, SYS_EXIT
    xor ebx, ebx
    int 0x80

.hang:
    jmp .hang

; Sends [print_len] bytes starting at [print_ptr] to the console server,
; one send() per byte (see console_server's OP_PUTCHAR protocol). send
; consumes all of ebx/ecx/edx/esi/edi as arguments/return slots, so the
; loop index has to live somewhere send() doesn't touch -- ebp is saved
; and restored whole across every syscall (see syscall_asm.s), so it's
; free to use here.
print_msg:
    xor ebp, ebp
.byte_loop:
    cmp ebp, [print_len]
    jge .done
    mov esi, [print_ptr]
    movzx edx, byte [esi + ebp]

    mov eax, SYS_SEND
    mov ebx, CONSOLE_TASK_ID
    mov ecx, OP_PUTCHAR
    xor esi, esi
    xor edi, edi
    int 0x80

    inc ebp
    jmp .byte_loop
.done:
    ret

print_ptr: dd 0
print_len: dd 0

msg_part1:   db "[ping] round="
digit_round: db "0"
msg_part2:   db " reply="
digit_reply: db "0"
msg_part3:   db 10
msg_end:
msg_len equ msg_end - msg_part1
