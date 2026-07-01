# Multiboot1 header constants
.set ALIGN,    1 << 0                   # align loaded modules on page boundaries
.set MEMINFO,  1 << 1                   # provide memory map
.set FLAGS,    ALIGN | MEMINFO
.set MAGIC,    0x1BADB002               # magic number the bootloader looks for
.set CHECKSUM, -(MAGIC + FLAGS)

# Higher-half layout: the kernel is linked to run at KERNEL_VMA but GRUB loads
# it at the low physical address KERNEL_LMA (see linker.ld). A single PSE 4MiB
# page directory bridges the gap until proper 4KiB paging takes over later.
.set KERNEL_VMA, 0xC0000000
.set KERNEL_PDE_INDEX, (KERNEL_VMA >> 22)   # PDE index covering KERNEL_VMA (768)
.set PAGE_PRESENT_RW_4M, 0x83               # present | read-write | page-size(4M)

.section .multiboot
.align 4
    .long MAGIC
    .long FLAGS
    .long CHECKSUM

# ---- Low, identity-linked boot code/data (see linker.ld's .boot output
# section). Runs with paging disabled, so nothing here may reference a
# symbol linked at the high KERNEL_VMA addresses except via an absolute
# (not rip/eip-relative) load. ----
.section .boot.data, "aw"
.align 4096
.global boot_page_directory
boot_page_directory:
    .skip 4096

.section .boot.text, "ax"
.global _start
.type _start, @function
_start:
    # GRUB leaves the multiboot magic/info pointer in eax/ebx; stash them
    # somewhere the page-table bootstrap below won't clobber.
    mov esi, eax
    mov edi, ebx

    # Identity-map the first 4 MiB (so execution can continue from the
    # current low EIP the instant paging turns on) and alias the same 4 MiB
    # at KERNEL_VMA (so the high-linked kernel becomes reachable).
    mov dword ptr [boot_page_directory + 0*4], PAGE_PRESENT_RW_4M
    mov dword ptr [boot_page_directory + KERNEL_PDE_INDEX*4], PAGE_PRESENT_RW_4M

    mov eax, offset boot_page_directory
    mov cr3, eax

    mov eax, cr4
    or eax, 0x00000010          # CR4.PSE
    mov cr4, eax

    mov eax, cr0
    or eax, 0x80000000          # CR0.PG
    mov cr0, eax

    # The gap between the low and high links is ~3 GiB, too large for a
    # relative jmp's signed 32-bit displacement, so load the high-linked
    # target address explicitly and jump through a register instead.
    lea eax, [higher_half_entry]
    jmp eax
.size _start, . - _start

# ---- High-linked kernel entry, running with paging enabled. ----
.section .bss
.align 16
stack_bottom:
    .skip 65536                  # 64 KiB stack
stack_top:

.section .text
.global higher_half_entry
.type higher_half_entry, @function
higher_half_entry:
    # The low identity mapping (entry 0) is deliberately left in place: the
    # multiboot info struct/memory map/modules GRUB left behind live in low
    # physical memory, and the kernel's own frame allocator wants a way to
    # touch arbitrary physical frames later too. It only affects the
    # kernel's own page directory -- every user task gets its own, built
    # fresh by mm::paging, so this doesn't leak into user address spaces.
    mov esp, offset stack_top
    mov ebp, esp
    push edi                     # multiboot info pointer (arg2)
    push esi                     # multiboot magic (arg1)
    call kernel_main
    cli
.hang:
    hlt
    jmp .hang
.size higher_half_entry, . - higher_half_entry
