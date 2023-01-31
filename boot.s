# Constants needed by the multiboot header
.set ALIGN, 1 << 0                      # align on page boundaries
.set MEMINFO, 1 << 1                    # memory map
.set FLAGS, ALIGN | MEMINFO
.set MAGIC, 0x1BADB002                  # header location for the bootloader
.set CHECKSUM, -(MAGIC + FLAGS)

# multiboot header
.section .multiboot
        .align 4
        .long MAGIC
        .long FLAGS
        .long CHECKSUM

.section .bss
        .align 16
        stack_bottom:
                .skip 16384             # reserve a 16 KiB stack
        stack_top:

.section .text
        .global start
        .type start, @function
        start:
                mov $stack_top, %esp    # set up the stack for our kernel code
                call main
                cli                     # disable CPU interrupts
1:              hlt                     # halt the CPU
                jmp 1b                  # loop around and try again, if not hung

.size start, . - start
