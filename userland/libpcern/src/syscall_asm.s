// Raw cdecl trampoline for `int 0x80`. This has to be hand-written
// assembly rather than a Rust `asm!` block: the kernel's syscall ABI pins
// arguments/results to specific registers (ebx/ecx/edx/esi/edi, see
// src/syscall.rs and src/cap.rs in the kernel), but LLVM's x86 codegen
// reserves esi as a base pointer for stack realignment in ordinary
// (non-naked) functions and refuses to let `asm!` bind it directly ("esi
// is used internally by LLVM"). A standalone assembly routine has no
// LLVM-generated prologue, so no such conflict exists -- the same reason
// the kernel's own context switch (switch_to) lives in global_asm rather
// than an `asm!` block.
//
// C signature: `fn syscall_raw_asm(num, a1, a2, a3, a4, a5, out: *mut RawResult)`
// cdecl pushes args right-to-left, so on entry:
//   [ebp+8]=num [ebp+12]=a1 [ebp+16]=a2 [ebp+20]=a3 [ebp+24]=a4 [ebp+28]=a5 [ebp+32]=out
// `out` receives eax/ebx/ecx/edx/esi/edi (in that order) as 6 packed u32s
// -- every register the kernel's syscall ABI ever writes on return, not
// just the ones a particular syscall happens to use, so this trampoline
// doesn't need to change again as new syscalls give new registers new
// meanings (e.g. `edi` carrying a transferred capability slot).
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

    // Stash all 6 result registers before repurposing ebp as the out-ptr
    // scratch -- every one of them (including edi) is a real, meaningful
    // return value now, so none can be clobbered before it's stored. The
    // original saved-ebp value stays sitting on the stack under these
    // pushes; `pop`/`push` address via esp, not ebp, so overwriting the
    // ebp *register* here doesn't disturb it.
    push eax
    push ebx
    push ecx
    push edx
    push esi
    push edi
    mov ebp, [ebp+32]
    // Pop order is the reverse of the push order above: edi, esi, edx,
    // ecx, ebx, eax.
    pop dword ptr [ebp+20]
    pop dword ptr [ebp+16]
    pop dword ptr [ebp+12]
    pop dword ptr [ebp+8]
    pop dword ptr [ebp+4]
    pop dword ptr [ebp+0]

    pop edi
    pop esi
    pop ebx
    pop ebp
    ret
