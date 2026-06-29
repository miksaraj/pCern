# Multiboot1 header constants
.set ALIGN,    1 << 0                   # align loaded modules on page boundaries
.set MEMINFO,  1 << 1                   # provide memory map
.set FLAGS,    ALIGN | MEMINFO
.set MAGIC,    0x1BADB002               # magic number the bootloader looks for
.set CHECKSUM, -(MAGIC + FLAGS)

.section .multiboot
.align 4
    .long MAGIC
    .long FLAGS
    .long CHECKSUM

.section .bss
.align 16
stack_bottom:
    .skip 65536                          # 64 KiB stack
stack_top:

.section .text
.global _start
.type _start, @function
_start:
    mov esp, offset stack_top            # set up the stack
    mov ebp, esp
    push ebx                             # multiboot info pointer (arg2)
    push eax                             # multiboot magic (arg1)
    call kernel_main
    cli
.hang:
    hlt
    jmp .hang
.size _start, . - _start
