// Raw cdecl trampoline for `int 0x80`, called from syscall.rs's
// `syscall_raw`. This has to be hand-written assembly rather than a Rust
// `asm!` block: the kernel's syscall ABI pins arguments/results to
// specific registers (ebx/ecx/edx/esi/edi, see src/syscall.rs's SavedRegs
// in the kernel), but LLVM's x86 codegen reserves esi as a base pointer
// for stack realignment in ordinary (non-naked) functions and refuses to
// let `asm!` bind it directly ("esi is used internally by LLVM"). A
// standalone assembly routine has no LLVM-generated prologue, so no such
// conflict exists -- the same reason the kernel's own context switch
// (switch_to) lives in global_asm rather than an `asm!` block.
//
// C signature: `fn syscall_raw_asm(num, a1, a2, a3, a4, a5, out: *mut RawResult)`
// cdecl pushes args right-to-left, so on entry:
//   [ebp+8]=num [ebp+12]=a1 [ebp+16]=a2 [ebp+20]=a3 [ebp+24]=a4 [ebp+28]=a5 [ebp+32]=out
// `out` receives eax/ebx/ecx/edx/esi (in that order) as 5 packed u32s.
.global syscall_raw_asm
syscall_raw_asm:
    push ebp
    mov ebp, esp
    push ebx
    push esi
    push edi

    mov eax, [ebp+8]
    mov ebx, [ebp+12]
    mov ecx, [ebp+16]
    mov edx, [ebp+20]
    mov esi, [ebp+24]
    mov edi, [ebp+28]
    int 0x80

    mov edi, [ebp+32]
    mov [edi], eax
    mov [edi+4], ebx
    mov [edi+8], ecx
    mov [edi+12], edx
    mov [edi+16], esi

    pop edi
    pop esi
    pop ebx
    pop ebp
    ret
