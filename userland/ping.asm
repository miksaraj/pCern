; Checkpoint 5 IPC test, half A: sends an incrementing counter to `pong`
; (task id 4 -- see main.rs, which spawns ping then pong in that fixed
; order) and waits for its reply, 5 rounds, proving synchronous rendezvous
; send/recv works correctly across two separate address spaces.
;
; Note: only eax/ebx/ecx/edx have byte sub-registers (al/bl/cl/dl) in
; 32-bit mode -- esi/edi/ebp do not (that needs a REX prefix, x86-64 only)
; -- so the round counter is kept in edx/ecx rather than esi/ebp.
BITS 32
org 0x00400000

SYS_EXIT        equ 0
SYS_SEND        equ 2
SYS_RECV        equ 3
SYS_DEBUG_WRITE equ 5

PONG_TASK_ID    equ 4
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

    mov eax, SYS_DEBUG_WRITE
    mov ebx, msg_part1
    mov ecx, msg_len
    int 0x80

    inc edx
    cmp edx, ROUNDS
    jl .loop

    mov eax, SYS_EXIT
    xor ebx, ebx
    int 0x80

.hang:
    jmp .hang

msg_part1:   db "[ping] round="
digit_round: db "0"
msg_part2:   db " reply="
digit_reply: db "0"
msg_part3:   db 10
msg_end:
msg_len equ msg_end - msg_part1
