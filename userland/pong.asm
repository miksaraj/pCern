; Checkpoint 5 IPC test, half B: waits for a message on its own inbox
; endpoint, replies with (received value + 1), 5 rounds.
;
; Checkpoint E: addressing moved from raw task ids to capability slots.
; recv used to take a `filter=0` (ANY sender) argument; now recv just
; waits on this task's own inbox endpoint and whoever holds a capability
; to it (only ping, per main.rs's boot-time wiring) can reach it -- same
; selectivity, now coming from capability possession rather than a
; runtime filter. The true sender's task id is still reported via `eax`
; on recv (kernel-attested, unspoofable), which is what lets this task
; reply to the right peer's endpoint without needing a dynamic capability
; transfer mechanism for that part (that's Checkpoint F, used here only
; for reaching the console server -- see lookup_console below).
;
; Checkpoint H: CSlot 1 is now the name service (auto-granted to every
; task -- see loader.rs in the kernel), pushing this task's own inbox to
; CSlot 2 and ping's endpoint to CSlot 3. There's no more pre-wired
; capability to the console server -- it's looked up by name at startup
; instead, the same as ping.asm.
;
; sender_id/received_value are stashed in static memory rather than kept in
; registers across syscalls: recv/send both reuse ebx/ecx/edx as
; argument/return slots, and only eax/ebx/ecx/edx have byte sub-registers in
; 32-bit mode anyway (no REX-only esi/edi byte access here), so memory is
; simplest to reason about correctly.
BITS 32
org 0x00400000

SYS_EXIT equ 0
SYS_SEND equ 2
SYS_RECV equ 3

NAMESERVICE_SLOT equ 1
MY_INBOX_SLOT    equ 2
PING_SLOT        equ 3
OP_PUTCHAR       equ 0
NS_OP_LOOKUP     equ 2
ROUNDS           equ 5

_start:
    call lookup_console

    xor edx, edx             ; round counter, preserved via the stack around recv

.loop:
    push edx
    mov eax, SYS_RECV
    mov ebx, MY_INBOX_SLOT
    int 0x80
    ; eax = sender task id (unused -- ping.asm's PING_SLOT capability is
    ; how the reply below reaches the right peer), ebx = received value
    pop edx                   ; round counter restored

    mov [saved_recv], ebx

    mov al, bl
    add al, '0'
    mov [digit_recv], al
    mov al, dl
    add al, '0'
    mov [digit_round], al

    mov dword [print_ptr], msg_part1
    mov dword [print_len], msg_len
    ; print_msg clobbers edx (see ping.asm's identical comment) -- save/
    ; restore the round counter around the call.
    push edx
    call print_msg
    pop edx

    ; send(dest=PING_SLOT, w0=saved_recv+1, w1=0, w2=0); send only touches
    ; eax, so edx (round) needs no extra saving across this one.
    mov eax, SYS_SEND
    mov ebx, PING_SLOT
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

; Looks up "console" via the name service, storing the granted capability
; slot (in this task's own CSpace) into [console_slot]. Blocks until the
; name service replies -- see ping.asm's identical subroutine and
; userland/nameservice/src/main.rs for the protocol this implements.
lookup_console:
    mov eax, SYS_SEND
    mov ebx, NAMESERVICE_SLOT
    mov ecx, NS_OP_LOOKUP
    mov edx, [console_name]
    mov esi, [console_name + 4]
    mov edi, MY_INBOX_SLOT     ; transfer: reply-to capability (our own inbox)
    int 0x80

    mov eax, SYS_RECV
    mov ebx, MY_INBOX_SLOT
    int 0x80
    mov [console_slot], edi
    ret

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
    mov ebx, [console_slot]
    mov ecx, OP_PUTCHAR
    xor esi, esi
    xor edi, edi
    int 0x80

    inc ebp
    jmp .byte_loop
.done:
    ret

console_name: db "console", 0
console_slot: dd 0

print_ptr: dd 0
print_len: dd 0

msg_part1:   db "[pong] round="
digit_round: db "0"
msg_part2:   db " recv="
digit_recv:  db "0"
msg_part3:   db 10
msg_end:
msg_len equ msg_end - msg_part1

saved_recv:   dd 0
