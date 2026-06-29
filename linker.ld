ENTRY(_start)

SECTIONS
{
    /* begin sections at 1M; everything below is reserved for the BIOS/bootloader */
    . = 1M;

    .text ALIGN(4K) :
    {
        *(.multiboot)
        *(.text .text.*)
    }

    .rodata ALIGN(4K) :
    {
        *(.rodata .rodata.*)
    }

    .data ALIGN(4K) :
    {
        *(.data .data.*)
    }

    .bss ALIGN(4K) :
    {
        *(COMMON)
        *(.bss .bss.*)
    }

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
        *(.note.*)
        *(.got)
        *(.got.plt)
    }
}
