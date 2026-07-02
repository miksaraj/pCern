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
; Checkpoint H: CSlot 1 is now the name service (auto-granted to every
; task -- see loader.rs in the kernel), pushing this task's own inbox to
; CSlot 2 and pong's endpoint to CSlot 3. There's no more pre-wired
; capability to the console server -- it's looked up by name at startup
; instead (see lookup_console below), the first real (non-main.rs-
; hardcoded) use of the name service.
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

NAMESERVICE_SLOT equ 1
MY_INBOX_SLOT    equ 2
PONG_SLOT        equ 3
OP_PUTCHAR       equ 0
NS_OP_LOOKUP     equ 2
ROUNDS           equ 5

_start:
    call lookup_console

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
    ; print_msg clobbers edx (it loads each byte being sent into edx, the
    ; wire format's w1) -- save/restore the round counter around the call,
    ; a real bug found while verifying Checkpoint H: without this, edx
    ; comes back holding the last byte sent (msg_part3's newline, 10),
    ; `inc edx` makes it 11, and `cmp edx, ROUNDS` (5) fails immediately,
    ; silently ending the loop after round 0 every time -- invisible
    ; before now since the exit code is 0 either way.
    push edx
    call print_msg
    pop edx

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
; name service replies -- see userland/nameservice/src/main.rs for the
; protocol this implements from a raw-asm client's side (libpcern's
; lookup_name is the same thing for Rust userland programs).
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
    ; eax=sender (the name service, unused), ebx=found flag (unused --
    ; console_server always registers before ping/pong ever run), edi=the
    ; granted capability slot
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

msg_part1:   db "[ping] round="
digit_round: db "0"
msg_part2:   db " reply="
digit_reply: db "0"
msg_part3:   db 10
msg_end:
msg_len equ msg_end - msg_part1
