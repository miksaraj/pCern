; Checkpoint 5 IPC test, half A: sends an incrementing counter to `pong`
; and waits for its reply, 5 rounds, proving synchronous rendezvous
; send/recv works correctly across two separate address spaces.
;
; Checkpoint E: addressing moved from raw task ids to capability slots.
; There's no name service yet (Checkpoint H), so main.rs wires every
; task's capabilities by hand right after spawning: CSlot 1 is always
; this task's own inbox endpoint (for recv), CSlot 2/3 are peer endpoints
; it was granted a capability to send to. Selectivity that used to come
; from recv's `filter` argument now comes from capability possession
; instead -- only pong holds a capability to *this* task's inbox, so
; recv(MY_INBOX_SLOT) only ever wakes for a message from pong.
;
; Checkpoint D+: output goes through the userspace console server instead
; of the kernel's debug_write syscall -- see print_msg below.
;
; Note: only eax/ebx/ecx/edx have byte sub-registers (al/bl/cl/dl) in
; 32-bit mode -- esi/edi/ebp do not (that needs a REX prefix, x86-64 only)
; -- so the round counter is kept in edx/ecx rather than esi/ebp.
BITS 32
org 0x00400000

SYS_EXIT equ 0
SYS_SEND equ 2
SYS_RECV equ 3

MY_INBOX_SLOT equ 1
PONG_SLOT     equ 2
CONSOLE_SLOT  equ 3
OP_PUTCHAR    equ 0
ROUNDS        equ 5

_start:
    xor edx, edx            ; round counter

.loop:
    ; send(dest=PONG_SLOT, w0=round, w1=whatever's in edx (harmless), w2=0,
    ; transfer=none). Dispatcher only touches eax for a successful send, so
    ; edx (round) survives this untouched.
    mov eax, SYS_SEND
    mov ebx, PONG_SLOT
    mov ecx, edx
    xor esi, esi
    xor edi, edi
    int 0x80

    ; recv(MY_INBOX_SLOT) -> ebx = reply value (w0); recv overwrites
    ; ebx/ecx/edx as return slots, so save the round counter first.
    push edx
    mov eax, SYS_RECV
    mov ebx, MY_INBOX_SLOT
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
    mov ebx, CONSOLE_SLOT
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
