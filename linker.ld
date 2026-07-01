ENTRY(_start)

KERNEL_LMA = 0x00100000;   /* physical address GRUB loads the kernel at */
KERNEL_VMA = 0xC0000000;   /* virtual address the kernel is linked to run at */

SECTIONS
{
    /* Low, identity-linked boot code/data: multiboot header, the PSE
       bootstrap page directory, and _start. Runs before paging is enabled,
       so LMA == VMA here (no KERNEL_VMA offset). */
    . = KERNEL_LMA;

    .boot ALIGN(4K) :
    {
        *(.multiboot)
        *(.boot.text)
        *(.boot.data)
    }

    /* Everything from here on is linked at the high half; AT() pins each
       section's physical load address to the low, contiguous continuation
       of .boot so GRUB still loads a single flat low image. */
    . += KERNEL_VMA;

    .text ALIGN(4K) : AT(ADDR(.text) - KERNEL_VMA)
    {
        *(.text .text.*)
    }

    .rodata ALIGN(4K) : AT(ADDR(.rodata) - KERNEL_VMA)
    {
        *(.rodata .rodata.*)
    }

    .data ALIGN(4K) : AT(ADDR(.data) - KERNEL_VMA)
    {
        *(.data .data.*)
    }

    .bss ALIGN(4K) : AT(ADDR(.bss) - KERNEL_VMA)
    {
        *(COMMON)
        *(.bss .bss.*)
    }

    kernel_end = .;
    KERNEL_END_PHYS = kernel_end - KERNEL_VMA;

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
        *(.note.*)
        *(.got)
        *(.got.plt)
    }
}
