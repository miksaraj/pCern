; Checkpoint A verification only (not wired into the default build): an
; ordinary (non-driver) task must NOT get map_memory or port I/O -- a
; regression check that the new privileged syscalls stay properly gated.
BITS 32
org 0x00400000

SYS_EXIT        equ 0
SYS_MAP_MEMORY  equ 7
SYS_DEBUG_WRITE equ 5

_start:
    mov eax, SYS_MAP_MEMORY
    mov ebx, 0xB8000
    mov ecx, 0x00900000
    mov edx, 0x1000
    int 0x80

    cmp eax, 0
    jne .map_denied
    mov byte [map_result], '0'   ; BUG if we ever see this: should be denied
    jmp .map_done
.map_denied:
    mov byte [map_result], '1'   ; expected: denied
.map_done:

    mov eax, SYS_DEBUG_WRITE
    mov ebx, msg_map
    mov ecx, msg_map_len
    int 0x80

    ; This must #GP -- an ordinary task has nothing allowed in the I/O
    ; bitmap, so it should never reach sys_exit below.
    in al, 0x64

    mov eax, SYS_DEBUG_WRITE
    mov ebx, msg_should_not_reach
    mov ecx, msg_should_not_reach_len
    int 0x80

    mov eax, SYS_EXIT
    mov ebx, 99
    int 0x80

.hang:
    jmp .hang

msg_map:    db "[nondriver_test] map_memory denied="
map_result: db "0", 10
msg_map_end:
msg_map_len equ msg_map_end - msg_map

msg_should_not_reach:     db "[nondriver_test] BUG: port io should have faulted", 10
msg_should_not_reach_len equ $ - msg_should_not_reach
