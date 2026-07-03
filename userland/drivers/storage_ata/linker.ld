/* Fixes the load address to USER_CODE_BASE (see src/loader.rs in the
   kernel) -- ring-3 programs here are flat, position-dependent binaries
   just like the hand-assembled ones in userland/*.asm, not relocatable
   ELF executables. */
ENTRY(_start)

SECTIONS
{
    . = 0x00400000;

    /* _start must land at byte 0 of the flat image: the kernel's loader
       always sets eip = the base load address for these flat binaries, it
       doesn't read an ELF entry point. */
    .text : { *(.text.start) *(.text .text.*) }
    .rodata : { *(.rodata .rodata.*) }
    .data : { *(.data .data.*) }
    .bss : { *(.bss .bss.*) *(COMMON) }

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
        *(.note.*)
    }
}
