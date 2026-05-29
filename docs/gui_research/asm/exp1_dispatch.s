	.intel_syntax noprefix
	.file	"exp1_dispatch.582f0eee8fa113af-cgu.0"
	.section	.text.dispatch_dyn,"ax",@progbits
	.globl	dispatch_dyn
	.p2align	4
	.type	dispatch_dyn,@function
dispatch_dyn:
	.cfi_startproc
	mov	rax, qword ptr [rsi + 24]
	mov	esi, edx
	jmp	rax
.Lfunc_end0:
	.size	dispatch_dyn, .Lfunc_end0-dispatch_dyn
	.cfi_endproc

	.section	.text.dispatch_enum,"ax",@progbits
	.globl	dispatch_enum
	.p2align	4
	.type	dispatch_enum,@function
dispatch_enum:
	.cfi_startproc
	mov	eax, dword ptr [rdi]
	vmovss	xmm4, dword ptr [rdi + 4]
	test	eax, eax
	je	.LBB1_4
	vxorps	xmm2, xmm2, xmm2
	vxorps	xmm3, xmm3, xmm3
	cmp	eax, 1
	jne	.LBB1_3
	mov	eax, esi
	vcvtsi2ss	xmm2, xmm15, rax
	xor	eax, eax
	sub	esi, 1
	cmovb	esi, eax
	vcvtsi2ss	xmm3, xmm15, rsi
	vmulss	xmm1, xmm1, xmm2
	vmulss	xmm2, xmm4, xmm3
	vaddss	xmm2, xmm1, xmm2
	vmovaps	xmm3, xmm0
.LBB1_3:
	vmovaps	xmm0, xmm3
	vmovaps	xmm1, xmm2
	ret
.LBB1_4:
	mov	eax, esi
	vcvtsi2ss	xmm2, xmm15, rax
	xor	eax, eax
	sub	esi, 1
	cmovb	esi, eax
	vcvtsi2ss	xmm3, xmm15, rsi
	vmulss	xmm0, xmm0, xmm2
	vmulss	xmm2, xmm4, xmm3
	vaddss	xmm3, xmm0, xmm2
	vmovaps	xmm2, xmm1
	vmovaps	xmm0, xmm3
	vmovaps	xmm1, xmm2
	ret
.Lfunc_end1:
	.size	dispatch_enum, .Lfunc_end1-dispatch_enum
	.cfi_endproc

	.section	.text.dispatch_fnptr,"ax",@progbits
	.globl	dispatch_fnptr
	.p2align	4
	.type	dispatch_fnptr,@function
dispatch_fnptr:
	.cfi_startproc
	mov	rax, rdi
	mov	edi, esi
	jmp	rax
.Lfunc_end2:
	.size	dispatch_fnptr, .Lfunc_end2-dispatch_fnptr
	.cfi_endproc

	.section	.text.solve_many_dyn,"ax",@progbits
	.globl	solve_many_dyn
	.p2align	4
	.type	solve_many_dyn,@function
solve_many_dyn:
	.cfi_startproc
	test	rsi, rsi
	je	.LBB3_1
	push	r14
	.cfi_def_cfa_offset 16
	push	rbx
	.cfi_def_cfa_offset 24
	sub	rsp, 24
	.cfi_def_cfa_offset 48
	.cfi_offset rbx, -24
	.cfi_offset r14, -16
	mov	rbx, rsi
	mov	r14, rdi
	shl	rbx, 4
	add	rbx, rdi
	vxorps	xmm2, xmm2, xmm2
	vmovss	dword ptr [rsp + 16], xmm1
	vmovss	dword ptr [rsp + 12], xmm0
	.p2align	4
.LBB3_4:
	vmovss	dword ptr [rsp + 20], xmm2
	mov	rdi, qword ptr [r14]
	mov	rax, qword ptr [r14 + 8]
	mov	esi, 8
	vmovss	xmm0, dword ptr [rsp + 12]
	vmovss	xmm1, dword ptr [rsp + 16]
	call	qword ptr [rax + 24]
	vmovss	xmm2, dword ptr [rsp + 20]
	vaddss	xmm2, xmm2, xmm0
	add	r14, 16
	cmp	r14, rbx
	jne	.LBB3_4
	add	rsp, 24
	.cfi_def_cfa_offset 24
	pop	rbx
	.cfi_def_cfa_offset 16
	pop	r14
	.cfi_def_cfa_offset 8
	.cfi_restore rbx
	.cfi_restore r14
	vmovaps	xmm0, xmm2
	ret
.LBB3_1:
	vxorps	xmm0, xmm0, xmm0
	ret
.Lfunc_end3:
	.size	solve_many_dyn, .Lfunc_end3-solve_many_dyn
	.cfi_endproc

	.section	.rodata.cst4,"aM",@progbits,4
	.p2align	2, 0x0
.LCPI4_0:
	.long	0x41000000
.LCPI4_1:
	.long	0x40e00000
	.section	.text.solve_many_enum,"ax",@progbits
	.globl	solve_many_enum
	.p2align	4
	.type	solve_many_enum,@function
solve_many_enum:
	.cfi_startproc
	test	rsi, rsi
	je	.LBB4_1
	vmulss	xmm2, xmm0, dword ptr [rip + .LCPI4_0]
	shl	rsi, 3
	vxorps	xmm1, xmm1, xmm1
	xor	eax, eax
	vmovss	xmm3, dword ptr [rip + .LCPI4_1]
	jmp	.LBB4_4
	.p2align	4
.LBB4_8:
	vmulss	xmm4, xmm3, dword ptr [rdi + rax + 4]
	vaddss	xmm4, xmm2, xmm4
.LBB4_7:
	vaddss	xmm1, xmm1, xmm4
	add	rax, 8
	cmp	rsi, rax
	je	.LBB4_2
.LBB4_4:
	mov	ecx, dword ptr [rdi + rax]
	test	ecx, ecx
	je	.LBB4_8
	vxorps	xmm4, xmm4, xmm4
	cmp	ecx, 1
	jne	.LBB4_7
	vmovaps	xmm4, xmm0
	jmp	.LBB4_7
.LBB4_1:
	vxorps	xmm1, xmm1, xmm1
.LBB4_2:
	vmovaps	xmm0, xmm1
	ret
.Lfunc_end4:
	.size	solve_many_enum, .Lfunc_end4-solve_many_enum
	.cfi_endproc

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
