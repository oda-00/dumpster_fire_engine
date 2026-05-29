	.intel_syntax noprefix
	.file	"exp4_layout.379fde808f8e6c1a-cgu.0"
	.section	.rodata.cst4,"aM",@progbits,4
	.p2align	2, 0x0
.LCPI0_0:
	.long	0x3f800000
	.section	.text.distribute_branchless,"ax",@progbits
	.globl	distribute_branchless
	.p2align	4
	.type	distribute_branchless,@function
distribute_branchless:
	.cfi_startproc
	test	rsi, rsi
	je	.LBB0_1
	lea	rcx, [8*rsi]
	add	rcx, -8
	mov	eax, ecx
	not	eax
	test	al, 56
	jne	.LBB0_4
	vxorps	xmm1, xmm1, xmm1
	mov	rax, rdi
	jmp	.LBB0_6
.LBB0_1:
	vxorps	xmm1, xmm1, xmm1
	jmp	.LBB0_9
.LBB0_4:
	mov	edx, ecx
	shr	edx, 3
	inc	edx
	and	edx, 7
	neg	rdx
	vxorps	xmm1, xmm1, xmm1
	mov	rax, rdi
	.p2align	4
.LBB0_5:
	vmovsd	xmm2, qword ptr [rax]
	add	rax, 8
	vaddps	xmm1, xmm1, xmm2
	inc	rdx
	jne	.LBB0_5
.LBB0_6:
	cmp	rcx, 56
	jb	.LBB0_9
	lea	rcx, [rdi + 8*rsi]
	.p2align	4
.LBB0_8:
	vmovsd	xmm2, qword ptr [rax]
	vaddps	xmm1, xmm1, xmm2
	vmovsd	xmm2, qword ptr [rax + 8]
	vaddps	xmm1, xmm1, xmm2
	vmovsd	xmm2, qword ptr [rax + 16]
	vaddps	xmm1, xmm1, xmm2
	vmovsd	xmm2, qword ptr [rax + 24]
	vaddps	xmm1, xmm1, xmm2
	vmovsd	xmm2, qword ptr [rax + 32]
	vaddps	xmm1, xmm1, xmm2
	vmovsd	xmm2, qword ptr [rax + 40]
	vaddps	xmm1, xmm1, xmm2
	vmovsd	xmm2, qword ptr [rax + 48]
	vaddps	xmm1, xmm1, xmm2
	vmovsd	xmm2, qword ptr [rax + 56]
	add	rax, 64
	vaddps	xmm1, xmm1, xmm2
	cmp	rax, rcx
	jne	.LBB0_8
.LBB0_9:
	vsubss	xmm0, xmm0, xmm1
	vxorps	xmm2, xmm2, xmm2
	vmaxss	xmm0, xmm0, xmm2
	vmovshdup	xmm1, xmm1
	vcmpeqss	xmm2, xmm1, xmm2
	vbroadcastss	xmm3, dword ptr [rip + .LCPI0_0]
	vblendvps	xmm1, xmm1, xmm3, xmm2
	vdivss	xmm0, xmm0, xmm1
	ret
.Lfunc_end0:
	.size	distribute_branchless, .Lfunc_end0-distribute_branchless
	.cfi_endproc

	.section	.rodata.cst4,"aM",@progbits,4
	.p2align	2, 0x0
.LCPI1_0:
	.long	0x3f800000
	.section	.text.distribute_branchy,"ax",@progbits
	.globl	distribute_branchy
	.p2align	4
	.type	distribute_branchy,@function
distribute_branchy:
	.cfi_startproc
	push	rax
	.cfi_def_cfa_offset 16
	test	rsi, rsi
	je	.LBB1_15
	shl	rsi, 3
	xor	r8d, r8d
	vxorps	xmm1, xmm1, xmm1
	xor	eax, eax
	jmp	.LBB1_2
	.p2align	4
.LBB1_10:
	inc	r8d
.LBB1_11:
	inc	rax
	add	rsi, -8
	je	.LBB1_5
.LBB1_2:
	mov	r9d, dword ptr [rdi + 8*rax]
	test	r9d, r9d
	je	.LBB1_10
	cmp	r9d, 1
	jne	.LBB1_13
	vaddss	xmm1, xmm1, dword ptr [rdi + 8*rax + 4]
	jmp	.LBB1_11
	.p2align	4
.LBB1_13:
	cmp	rax, rcx
	jae	.LBB1_14
	vaddss	xmm1, xmm1, dword ptr [rdx + 4*rax]
	jmp	.LBB1_11
.LBB1_5:
	vsubss	xmm1, xmm0, xmm1
	vxorps	xmm2, xmm2, xmm2
	test	r8d, r8d
	je	.LBB1_6
	mov	eax, r8d
	vcvtsi2ss	xmm0, xmm15, rax
	vmaxss	xmm1, xmm1, xmm2
	vdivss	xmm0, xmm1, xmm0
	pop	rax
	.cfi_def_cfa_offset 8
	ret
.LBB1_15:
	.cfi_def_cfa_offset 16
	vxorps	xmm1, xmm1, xmm1
	vmaxss	xmm1, xmm0, xmm1
	vmovss	xmm0, dword ptr [rip + .LCPI1_0]
	vdivss	xmm0, xmm1, xmm0
	pop	rax
	.cfi_def_cfa_offset 8
	ret
.LBB1_6:
	.cfi_def_cfa_offset 16
	vmovss	xmm0, dword ptr [rip + .LCPI1_0]
	vmaxss	xmm1, xmm1, xmm2
	vdivss	xmm0, xmm1, xmm0
	pop	rax
	.cfi_def_cfa_offset 8
	ret
.LBB1_14:
	.cfi_def_cfa_offset 16
	lea	rdx, [rip + .Lanon.b9159734458d0383946c250a6aa35d71.1]
	mov	rdi, rax
	mov	rsi, rcx
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.Lfunc_end1:
	.size	distribute_branchy, .Lfunc_end1-distribute_branchy
	.cfi_endproc

	.type	.Lanon.b9159734458d0383946c250a6aa35d71.0,@object
	.section	.rodata.str1.1,"aMS",@progbits,1
.Lanon.b9159734458d0383946c250a6aa35d71.0:
	.asciz	"exp4_layout.rs"
	.size	.Lanon.b9159734458d0383946c250a6aa35d71.0, 15

	.type	.Lanon.b9159734458d0383946c250a6aa35d71.1,@object
	.section	.data.rel.ro..Lanon.b9159734458d0383946c250a6aa35d71.1,"aw",@progbits
	.p2align	3, 0x0
.Lanon.b9159734458d0383946c250a6aa35d71.1:
	.quad	.Lanon.b9159734458d0383946c250a6aa35d71.0
	.asciz	"\016\000\000\000\000\000\000\000\025\000\000\000$\000\000"
	.size	.Lanon.b9159734458d0383946c250a6aa35d71.1, 24

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
