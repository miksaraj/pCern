# void gdt_flush(const GdtPointer *ptr) -- cdecl
.global gdt_flush
.type gdt_flush, @function
gdt_flush:
    mov eax, [esp + 4]
    lgdt [eax]

    mov ax, 0x10          # kernel data segment selector
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    push 0x08              # kernel code segment selector
    lea eax, [gdt_flush_ret]
    push eax
    retf                    # reload cs by "returning" into the new segment
gdt_flush_ret:
    ret
.size gdt_flush, . - gdt_flush

# void tss_flush() -- cdecl. Loads the task register with the TSS descriptor
# (GDT entry 5, selector 0x28); the CPU then knows where to find ss0/esp0 on
# a ring3->ring0 transition.
.global tss_flush
.type tss_flush, @function
tss_flush:
    mov ax, 0x28
    ltr ax
    ret
.size tss_flush, . - tss_flush
