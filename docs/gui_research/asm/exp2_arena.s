	.intel_syntax noprefix
	.file	"exp2_arena.39207a1412d288f-cgu.0"
	.section	.text.alloc_many_strings,"ax",@progbits
	.globl	alloc_many_strings
	.p2align	4
	.type	alloc_many_strings,@function
alloc_many_strings:
	.cfi_startproc
	test	rsi, rsi
	je	.LBB0_1
	shl	rsi, 4
	add	rsi, rdi
	xor	eax, eax
	lea	rcx, [rsp - 16]
	lea	rdx, [rsp - 24]
	.p2align	4
.LBB0_4:
	mov	r8, qword ptr [rdi]
	mov	r9, qword ptr [rdi + 8]
	mov	qword ptr [rsp - 16], r8
	mov	qword ptr [rsp - 8], r9
	add	rax, r9
	mov	qword ptr [rsp - 24], rcx
	#APP
	#NO_APP
	add	rdi, 16
	cmp	rdi, rsi
	jne	.LBB0_4
	ret
.LBB0_1:
	xor	eax, eax
	ret
.Lfunc_end0:
	.size	alloc_many_strings, .Lfunc_end0-alloc_many_strings
	.cfi_endproc

	.section	.text.alloc_string,"ax",@progbits
	.globl	alloc_string
	.p2align	4
	.type	alloc_string,@function
alloc_string:
	.cfi_startproc
	push	r15
	.cfi_def_cfa_offset 16
	push	r14
	.cfi_def_cfa_offset 24
	push	r13
	.cfi_def_cfa_offset 32
	push	r12
	.cfi_def_cfa_offset 40
	push	rbx
	.cfi_def_cfa_offset 48
	.cfi_offset rbx, -48
	.cfi_offset r12, -40
	.cfi_offset r13, -32
	.cfi_offset r14, -24
	.cfi_offset r15, -16
	mov	rbx, rdx
	test	rdx, rdx
	jns	.LBB1_3
	xor	r12d, r12d
.LBB1_2:
	mov	rdi, r12
	mov	rsi, rbx
	call	qword ptr [rip + _ZN5alloc7raw_vec12handle_error17hfa86a3a4628bd209E@GOTPCREL]
.LBB1_3:
	mov	r14, rdi
	je	.LBB1_4
	mov	r13, rsi
	call	qword ptr [rip + _RNvCs1Y7DaGC1cwg_7___rustc35___rust_no_alloc_shim_is_unstable_v2@GOTPCREL]
	mov	r12d, 1
	mov	esi, 1
	mov	rdi, rbx
	call	qword ptr [rip + _RNvCs1Y7DaGC1cwg_7___rustc12___rust_alloc@GOTPCREL]
	test	rax, rax
	je	.LBB1_2
	mov	r15, rax
	mov	rsi, r13
	jmp	.LBB1_7
.LBB1_4:
	mov	r15d, 1
.LBB1_7:
	mov	rdi, r15
	mov	rdx, rbx
	call	qword ptr [rip + memcpy@GOTPCREL]
	mov	qword ptr [r14], rbx
	mov	qword ptr [r14 + 8], r15
	mov	qword ptr [r14 + 16], rbx
	mov	rax, r14
	pop	rbx
	.cfi_def_cfa_offset 40
	pop	r12
	.cfi_def_cfa_offset 32
	pop	r13
	.cfi_def_cfa_offset 24
	pop	r14
	.cfi_def_cfa_offset 16
	pop	r15
	.cfi_def_cfa_offset 8
	ret
.Lfunc_end1:
	.size	alloc_string, .Lfunc_end1-alloc_string
	.cfi_endproc

	.section	.text.bump_alloc16,"ax",@progbits
	.globl	bump_alloc16
	.p2align	4
	.type	bump_alloc16,@function
bump_alloc16:
	.cfi_startproc
	mov	rax, qword ptr [rdi + 8]
	add	rax, 7
	and	rax, -8
	lea	rcx, [rax + 16]
	cmp	rcx, qword ptr [rdi + 16]
	jbe	.LBB2_2
	xor	eax, eax
	ret
.LBB2_2:
	mov	qword ptr [rdi + 8], rcx
	add	rax, qword ptr [rdi]
	ret
.Lfunc_end2:
	.size	bump_alloc16, .Lfunc_end2-bump_alloc16
	.cfi_endproc

	.section	.text.bump_reset,"ax",@progbits
	.globl	bump_reset
	.p2align	4
	.type	bump_reset,@function
bump_reset:
	.cfi_startproc
	mov	qword ptr [rdi + 8], 0
	ret
.Lfunc_end3:
	.size	bump_reset, .Lfunc_end3-bump_reset
	.cfi_endproc

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
