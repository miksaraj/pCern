ENTRY(start)

SECTIONS
{
    /* begin sections at 1M, since below it is reserved for other x86-related stuff, so we can't use it */
    . = 1M;

    /* Multiboot header + executable code */
    .text BLOCK(4K) : ALIGN(4K)
    {
        *(.multiboot)
        *(.text)
    }

    /* Read-only data */
    .rodata BLOCK(4K) : ALIGN(4K)
    {
        *(.rodata)
    }

    /* Read-write data (initialized) */
    .data BLOCK(4K) : ALIGN(4K)
    {
        *(.data)
    }

    /* Read-write data (uninitialized) & stack */
    .bss BLOCK(4K) : ALIGN(4K)
    {
        *(COMMON)
        *(.bss)
    }
}
