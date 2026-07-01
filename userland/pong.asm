; Checkpoint 5 IPC test, half B: waits for a message from any sender,
; replies with (received value + 1), 5 rounds. Uses filter=0 (ANY) in recv
; rather than hardcoding ping's task id, exercising that path of the
; rendezvous match too.
;
; sender_id/received_value are stashed in static memory rather than kept in
; registers across the debug_write call: recv/send both reuse ebx/ecx/edx/
; esi as return slots, and only eax/ebx/ecx/edx have byte sub-registers in
; 32-bit mode anyway (no REX-only esi/edi byte access here), so memory is
; simplest to reason about correctly.
BITS 32
org 0x00400000

SYS_EXIT        equ 0
SYS_SEND        equ 2
SYS_RECV        equ 3
SYS_DEBUG_WRITE equ 5

RECV_ANY equ 0
ROUNDS   equ 5

_start:
    xor edx, edx             ; round counter, preserved via the stack around recv

.loop:
    push edx
    mov eax, SYS_RECV
    mov ebx, RECV_ANY
    int 0x80
    ; eax = sender_id, ebx = received value
    pop edx                   ; round counter restored

    mov [saved_sender], eax
    mov [saved_recv], ebx

    mov al, bl
    add al, '0'
    mov [digit_recv], al
    mov al, dl
    add al, '0'
    mov [digit_round], al

    mov eax, SYS_DEBUG_WRITE
    mov ebx, msg_part1
    mov ecx, msg_len
    int 0x80

    ; send(dest=saved_sender, w0=saved_recv+1, 0, 0, 0); send only touches
    ; eax, so edx (round) needs no extra saving across this one.
    mov eax, SYS_SEND
    mov ebx, [saved_sender]
    mov ecx, [saved_recv]
    inc ecx
    xor esi, esi
    xor edi, edi
    int 0x80

    inc edx
    cmp edx, ROUNDS
    jl .loop

    mov eax, SYS_EXIT
    xor ebx, ebx
    int 0x80

.hang:
    jmp .hang

msg_part1:   db "[pong] round="
digit_round: db "0"
msg_part2:   db " recv="
digit_recv:  db "0"
msg_part3:   db 10
msg_end:
msg_len equ msg_end - msg_part1

saved_sender: dd 0
saved_recv:   dd 0
