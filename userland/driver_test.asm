; Checkpoint A verification only (not wired into the default build): a
; driver-flagged task should succeed at map_memory(VGA) and direct port
; I/O. Spawned temporarily with is_driver=true to prove the TSS I/O
; bitmap + map_memory allowlist work for a task that's actually allowed to
; use them.
BITS 32
org 0x00400000

SYS_EXIT        equ 0
SYS_MAP_MEMORY  equ 7
SYS_DEBUG_WRITE equ 5

VGA_VADDR equ 0x00900000

_start:
    mov eax, SYS_MAP_MEMORY
    mov ebx, 0xB8000
    mov ecx, VGA_VADDR
    mov edx, 0x1000
    int 0x80

    cmp eax, 0
    jne .map_failed
    mov byte [map_result], '0'
    jmp .map_done
.map_failed:
    mov byte [map_result], '1'
.map_done:

    mov eax, SYS_DEBUG_WRITE
    mov ebx, msg_map
    mov ecx, msg_map_len
    int 0x80

    ; Write 'D' (white-on-blue) through the freshly mapped pointer -- the
    ; real proof map_memory worked, visible in a VGA/physical-memory dump.
    mov edi, VGA_VADDR
    mov word [edi], 0x1F44

    ; Port I/O: read the keyboard controller status port. A driver task
    ; must NOT fault here.
    in al, 0x64

    mov eax, SYS_DEBUG_WRITE
    mov ebx, msg_io_ok
    mov ecx, msg_io_ok_len
    int 0x80

    mov eax, SYS_EXIT
    xor ebx, ebx
    int 0x80

.hang:
    jmp .hang

msg_map:    db "[driver_test] map_memory result="
map_result: db "0", 10
msg_map_end:
msg_map_len equ msg_map_end - msg_map

msg_io_ok:     db "[driver_test] port io ok", 10
msg_io_ok_len equ $ - msg_io_ok
