.extern kernel_main                             ; main function in kernel.c

.global start                                   ; declare start label as global for linker

.set MB_MAGIC, 0x1BADB002                       ; kernel location for GRUB
.set MB_FLAGS, (1 << 0) | (1 << 1)
.set MB_CHECKSUM, (0 - (MB_MAGIC + MB_FLAGS))

.section .multiboot                             ; multiboot header
        .align 4
        .long MB_MAGIC
        .long MB_FLAGS
        .long MB_CHECKSUM

.section .bss
        .align 16
        stack_bottom:
                .skip 4096                      ; reserve a 4K stack
        stack_top:

.section .text
        start:
                mov $stack_top, %esp            ; set up the stack for our kernel code
                call kernel_main
                hang:                           ; if kernel code ever returns, we hang CPU
                        cli                     ; disable CPU interrupts
                        hlt                     ; halt the CPU
                        jmp hang                ; loop around and try again, if not hung
