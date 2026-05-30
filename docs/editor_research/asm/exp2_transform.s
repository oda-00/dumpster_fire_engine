	.intel_syntax noprefix
	.file	"exp2_transform.df372bb2cf324c0d-cgu.0"
	.section	.text.affine_row_soa,"ax",@progbits
	.globl	affine_row_soa
	.p2align	4
	.type	affine_row_soa,@function
affine_row_soa:
	.cfi_startproc
	push	r15
	.cfi_def_cfa_offset 16
	push	r14
	.cfi_def_cfa_offset 24
	push	rbx
	.cfi_def_cfa_offset 32
	.cfi_offset rbx, -32
	.cfi_offset r14, -24
	.cfi_offset r15, -16
	mov	rax, qword ptr [rsp + 40]
	test	rax, rax
	je	.LBB0_13
	mov	r10, qword ptr [rsp + 32]
	cmp	r9, rcx
	mov	r11, rcx
	cmovb	r11, r9
	cmp	r11, rsi
	cmovae	r11, rsi
	lea	rbx, [rax - 1]
	cmp	r11, rbx
	cmovae	r11, rbx
	cmp	r11, 31
	ja	.LBB0_8
	xor	r11d, r11d
	jmp	.LBB0_3
.LBB0_8:
	inc	r11
	mov	ebx, r11d
	and	ebx, 31
	mov	r14d, 32
	cmovne	r14, rbx
	sub	r11, r14
	vbroadcastss	ymm4, xmm0
	vbroadcastss	ymm5, xmm1
	vbroadcastss	ymm6, xmm2
	vbroadcastss	ymm7, xmm3
	xor	ebx, ebx
	.p2align	4
.LBB0_9:
	vmulps	ymm8, ymm4, ymmword ptr [rdi + 4*rbx]
	vmulps	ymm9, ymm4, ymmword ptr [rdi + 4*rbx + 32]
	vmulps	ymm10, ymm4, ymmword ptr [rdi + 4*rbx + 64]
	vmulps	ymm11, ymm4, ymmword ptr [rdi + 4*rbx + 96]
	vmulps	ymm12, ymm5, ymmword ptr [rdx + 4*rbx]
	vaddps	ymm8, ymm8, ymm12
	vmulps	ymm12, ymm5, ymmword ptr [rdx + 4*rbx + 32]
	vaddps	ymm9, ymm9, ymm12
	vmulps	ymm12, ymm5, ymmword ptr [rdx + 4*rbx + 64]
	vmulps	ymm13, ymm5, ymmword ptr [rdx + 4*rbx + 96]
	vaddps	ymm10, ymm10, ymm12
	vaddps	ymm11, ymm11, ymm13
	vmulps	ymm12, ymm6, ymmword ptr [r8 + 4*rbx]
	vaddps	ymm8, ymm8, ymm12
	vmulps	ymm12, ymm6, ymmword ptr [r8 + 4*rbx + 32]
	vaddps	ymm9, ymm9, ymm12
	vmulps	ymm12, ymm6, ymmword ptr [r8 + 4*rbx + 64]
	vmulps	ymm13, ymm6, ymmword ptr [r8 + 4*rbx + 96]
	vaddps	ymm10, ymm10, ymm12
	vaddps	ymm11, ymm11, ymm13
	vaddps	ymm8, ymm8, ymm7
	vaddps	ymm9, ymm9, ymm7
	vaddps	ymm10, ymm10, ymm7
	vaddps	ymm11, ymm11, ymm7
	vmovups	ymmword ptr [r10 + 4*rbx], ymm8
	vmovups	ymmword ptr [r10 + 4*rbx + 32], ymm9
	vmovups	ymmword ptr [r10 + 4*rbx + 64], ymm10
	vmovups	ymmword ptr [r10 + 4*rbx + 96], ymm11
	add	rbx, 32
	cmp	r11, rbx
	jne	.LBB0_9
.LBB0_3:
	mov	rbx, r9
	sub	rbx, r11
	mov	r14, rcx
	sub	r14, r11
	mov	r15, rsi
	sub	r15, r11
	sub	rax, r11
	lea	rdi, [rdi + 4*r11]
	lea	rdx, [rdx + 4*r11]
	lea	r10, [r10 + 4*r11]
	lea	r8, [r8 + 4*r11]
	xor	r11d, r11d
	.p2align	4
.LBB0_4:
	cmp	r15, r11
	je	.LBB0_10
	cmp	r14, r11
	je	.LBB0_11
	cmp	rbx, r11
	je	.LBB0_7
	vmulss	xmm4, xmm0, dword ptr [rdi + 4*r11]
	vmulss	xmm5, xmm1, dword ptr [rdx + 4*r11]
	vaddss	xmm4, xmm4, xmm5
	vmulss	xmm5, xmm2, dword ptr [r8 + 4*r11]
	vaddss	xmm4, xmm4, xmm5
	vaddss	xmm4, xmm3, xmm4
	vmovss	dword ptr [r10 + 4*r11], xmm4
	inc	r11
	cmp	rax, r11
	jne	.LBB0_4
.LBB0_13:
	pop	rbx
	.cfi_def_cfa_offset 24
	pop	r14
	.cfi_def_cfa_offset 16
	pop	r15
	.cfi_def_cfa_offset 8
	vzeroupper
	ret
.LBB0_10:
	.cfi_def_cfa_offset 32
	lea	rdx, [rip + .Lanon.3e617c0c78f3a4fbb978c968fbf35c95.1]
	mov	rdi, rsi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_11:
	lea	rdx, [rip + .Lanon.3e617c0c78f3a4fbb978c968fbf35c95.2]
	mov	rdi, rcx
	mov	rsi, rcx
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_7:
	lea	rdx, [rip + .Lanon.3e617c0c78f3a4fbb978c968fbf35c95.3]
	mov	rdi, r9
	mov	rsi, r9
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.Lfunc_end0:
	.size	affine_row_soa, .Lfunc_end0-affine_row_soa
	.cfi_endproc

	.section	.rodata.cst4,"aM",@progbits,4
	.p2align	2, 0x0
.LCPI1_0:
	.long	1
	.section	.rodata.cst32,"aM",@progbits,32
	.p2align	5, 0x0
.LCPI1_1:
	.long	0
	.long	3
	.long	6
	.long	1
	.long	4
	.long	7
	.long	2
	.long	5
.LCPI1_2:
	.long	1
	.long	4
	.long	7
	.long	2
	.long	5
	.long	0
	.long	3
	.long	6
.LCPI1_3:
	.long	2
	.long	5
	.long	0
	.long	3
	.long	6
	.long	1
	.long	4
	.long	7
.LCPI1_4:
	.long	5
	.zero	4
	.zero	4
	.long	6
	.zero	4
	.zero	4
	.long	7
	.zero	4
	.section	.rodata.cst16,"aM",@progbits,16
	.p2align	4, 0x0
.LCPI1_5:
	.long	1
	.long	0
	.long	2
	.long	2
.LCPI1_6:
	.long	5
	.long	0
	.long	7
	.long	6
	.section	.text.translate_aos,"ax",@progbits
	.globl	translate_aos
	.p2align	4
	.type	translate_aos,@function
translate_aos:
	.cfi_startproc
	test	rsi, rsi
	je	.LBB1_7
	shl	rsi, 2
	lea	rax, [rsi + 2*rsi]
	vmovsd	xmm0, qword ptr [rdx]
	vmovss	xmm1, dword ptr [rdx + 8]
	lea	rdx, [rax - 12]
	movabs	rcx, -6148914691236517205
	mulx	rsi, rsi, rcx
	mov	rcx, rdi
	cmp	rdx, 84
	jb	.LBB1_5
	shr	rsi, 3
	inc	rsi
	mov	rdx, rsi
	and	rdx, -8
	lea	rcx, [rdx + 2*rdx]
	lea	rcx, [rdi + 4*rcx]
	vbroadcastss	ymm2, xmm0
	vbroadcastss	ymm3, dword ptr [rip + .LCPI1_0]
	vpermps	ymm3, ymm3, ymm0
	vbroadcastss	ymm4, xmm1
	vmovaps	ymm5, ymmword ptr [rip + .LCPI1_1]
	vmovaps	ymm6, ymmword ptr [rip + .LCPI1_2]
	vmovaps	ymm7, ymmword ptr [rip + .LCPI1_3]
	vbroadcastf128	ymm8, xmmword ptr [rip + .LCPI1_6]
	vbroadcastf128	ymm9, xmmword ptr [rip + .LCPI1_5]
	mov	r8, rdi
	mov	r9, rdx
	.p2align	4
.LBB1_3:
	vmovups	ymm10, ymmword ptr [r8]
	vmovups	ymm11, ymmword ptr [r8 + 32]
	vmovups	ymm12, ymmword ptr [r8 + 64]
	vblendps	ymm13, ymm10, ymm11, 146
	vblendps	ymm13, ymm13, ymm12, 36
	vpermps	ymm13, ymm5, ymm13
	vaddps	ymm13, ymm13, ymm2
	vblendps	ymm14, ymm10, ymm11, 36
	vblendps	ymm14, ymm14, ymm12, 73
	vpermps	ymm14, ymm6, ymm14
	vaddps	ymm14, ymm14, ymm3
	vblendps	ymm10, ymm11, ymm10, 36
	vblendps	ymm10, ymm10, ymm12, 146
	vpermps	ymm10, ymm7, ymm10
	vaddps	ymm10, ymm10, ymm4
	vpermps	ymm11, ymm8, ymm14
	vpermpd	ymm12, ymm13, 255
	vblendps	ymm11, ymm11, ymm12, 36
	vpermpd	ymm12, ymm10, 246
	vblendps	ymm11, ymm11, ymm12, 146
	vshufps	ymm12, ymm14, ymm14, 240
	vshufpd	ymm15, ymm13, ymm13, 1
	vblendps	ymm12, ymm15, ymm12, 36
	vshufpd	ymm15, ymm10, ymm10, 3
	vblendps	ymm12, ymm12, ymm15, 73
	vpermps	ymm14, ymm9, ymm14
	vpermpd	ymm13, ymm13, 96
	vblendps	ymm13, ymm13, ymm14, 146
	vbroadcastsd	ymm10, xmm10
	vblendps	ymm10, ymm13, ymm10, 36
	vmovups	ymmword ptr [r8], ymm10
	vmovups	ymmword ptr [r8 + 32], ymm12
	vmovups	ymmword ptr [r8 + 64], ymm11
	add	r8, 96
	add	r9, -8
	jne	.LBB1_3
	cmp	rsi, rdx
	je	.LBB1_7
.LBB1_5:
	add	rdi, rax
	.p2align	4
.LBB1_6:
	vmovsd	xmm2, qword ptr [rcx]
	vaddps	xmm2, xmm0, xmm2
	vmovlps	qword ptr [rcx], xmm2
	vaddss	xmm2, xmm1, dword ptr [rcx + 8]
	vmovss	dword ptr [rcx + 8], xmm2
	add	rcx, 12
	cmp	rcx, rdi
	jne	.LBB1_6
.LBB1_7:
	vzeroupper
	ret
.Lfunc_end1:
	.size	translate_aos, .Lfunc_end1-translate_aos
	.cfi_endproc

	.section	.text.translate_selected_soa,"ax",@progbits
	.globl	translate_selected_soa
	.p2align	4
	.type	translate_selected_soa,@function
translate_selected_soa:
	.cfi_startproc
	push	rbx
	.cfi_def_cfa_offset 16
	.cfi_offset rbx, -16
	test	rdx, rdx
	je	.LBB2_6
	shl	rdx, 2
	mov	r10, qword ptr [rdi + 8]
	mov	rax, qword ptr [rdi + 16]
	vmovss	xmm0, dword ptr [rcx]
	vmovss	xmm1, dword ptr [rcx + 4]
	mov	r9, qword ptr [rdi + 40]
	mov	r11, qword ptr [rdi + 32]
	mov	r8, qword ptr [rdi + 64]
	mov	rbx, qword ptr [rdi + 56]
	vmovss	xmm2, dword ptr [rcx + 8]
	xor	ecx, ecx
	.p2align	4
.LBB2_2:
	mov	edi, dword ptr [rsi + rcx]
	cmp	rax, rdi
	jbe	.LBB2_8
	vaddss	xmm3, xmm0, dword ptr [r10 + 4*rdi]
	vmovss	dword ptr [r10 + 4*rdi], xmm3
	cmp	r9, rdi
	jbe	.LBB2_9
	vaddss	xmm3, xmm1, dword ptr [r11 + 4*rdi]
	vmovss	dword ptr [r11 + 4*rdi], xmm3
	cmp	r8, rdi
	jbe	.LBB2_7
	vaddss	xmm3, xmm2, dword ptr [rbx + 4*rdi]
	vmovss	dword ptr [rbx + 4*rdi], xmm3
	add	rcx, 4
	cmp	rdx, rcx
	jne	.LBB2_2
.LBB2_6:
	pop	rbx
	.cfi_def_cfa_offset 8
	ret
.LBB2_7:
	.cfi_def_cfa_offset 16
	lea	rdx, [rip + .Lanon.3e617c0c78f3a4fbb978c968fbf35c95.6]
	mov	rsi, r8
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB2_9:
	lea	rdx, [rip + .Lanon.3e617c0c78f3a4fbb978c968fbf35c95.5]
	mov	rsi, r9
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB2_8:
	lea	rdx, [rip + .Lanon.3e617c0c78f3a4fbb978c968fbf35c95.4]
	mov	rsi, rax
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.Lfunc_end2:
	.size	translate_selected_soa, .Lfunc_end2-translate_selected_soa
	.cfi_endproc

	.section	.text.translate_soa,"ax",@progbits
	.globl	translate_soa
	.p2align	4
	.type	translate_soa,@function
translate_soa:
	.cfi_startproc
	mov	rax, qword ptr [rdi + 16]
	test	rax, rax
	je	.LBB3_14
	mov	rcx, qword ptr [rdi + 8]
	lea	r9, [4*rax]
	vmovss	xmm0, dword ptr [rsi]
	add	r9, -4
	mov	r10, rcx
	cmp	r9, 12
	jb	.LBB3_12
	mov	rdx, r9
	shr	rdx, 2
	inc	rdx
	movabs	r8, 9223372036854775776
	cmp	r9, 124
	jae	.LBB3_7
	xor	r9d, r9d
	jmp	.LBB3_4
.LBB3_7:
	mov	r9, rdx
	and	r9, r8
	vbroadcastss	ymm1, xmm0
	xor	r10d, r10d
	.p2align	4
.LBB3_8:
	vaddps	ymm2, ymm1, ymmword ptr [rcx + 4*r10]
	vaddps	ymm3, ymm1, ymmword ptr [rcx + 4*r10 + 32]
	vaddps	ymm4, ymm1, ymmword ptr [rcx + 4*r10 + 64]
	vaddps	ymm5, ymm1, ymmword ptr [rcx + 4*r10 + 96]
	vmovups	ymmword ptr [rcx + 4*r10], ymm2
	vmovups	ymmword ptr [rcx + 4*r10 + 32], ymm3
	vmovups	ymmword ptr [rcx + 4*r10 + 64], ymm4
	vmovups	ymmword ptr [rcx + 4*r10 + 96], ymm5
	add	r10, 32
	cmp	r9, r10
	jne	.LBB3_8
	cmp	rdx, r9
	je	.LBB3_14
	test	dl, 28
	je	.LBB3_11
.LBB3_4:
	add	r8, 28
	and	r8, rdx
	lea	r10, [rcx + 4*r8]
	vbroadcastss	xmm1, xmm0
	.p2align	4
.LBB3_5:
	vaddps	xmm2, xmm1, xmmword ptr [rcx + 4*r9]
	vmovups	xmmword ptr [rcx + 4*r9], xmm2
	add	r9, 4
	cmp	r8, r9
	jne	.LBB3_5
	cmp	rdx, r8
	jne	.LBB3_12
	jmp	.LBB3_14
.LBB3_11:
	lea	r10, [rcx + 4*r9]
.LBB3_12:
	lea	rax, [rcx + 4*rax]
	.p2align	4
.LBB3_13:
	vaddss	xmm1, xmm0, dword ptr [r10]
	vmovss	dword ptr [r10], xmm1
	add	r10, 4
	cmp	r10, rax
	jne	.LBB3_13
.LBB3_14:
	mov	rax, qword ptr [rdi + 40]
	test	rax, rax
	je	.LBB3_28
	mov	rcx, qword ptr [rdi + 32]
	lea	r9, [4*rax]
	vmovss	xmm0, dword ptr [rsi + 4]
	add	r9, -4
	mov	r10, rcx
	cmp	r9, 12
	jb	.LBB3_26
	mov	rdx, r9
	shr	rdx, 2
	inc	rdx
	movabs	r8, 9223372036854775776
	cmp	r9, 124
	jae	.LBB3_21
	xor	r9d, r9d
	jmp	.LBB3_18
.LBB3_21:
	mov	r9, rdx
	and	r9, r8
	vbroadcastss	ymm1, xmm0
	xor	r10d, r10d
	.p2align	4
.LBB3_22:
	vaddps	ymm2, ymm1, ymmword ptr [rcx + 4*r10]
	vaddps	ymm3, ymm1, ymmword ptr [rcx + 4*r10 + 32]
	vaddps	ymm4, ymm1, ymmword ptr [rcx + 4*r10 + 64]
	vaddps	ymm5, ymm1, ymmword ptr [rcx + 4*r10 + 96]
	vmovups	ymmword ptr [rcx + 4*r10], ymm2
	vmovups	ymmword ptr [rcx + 4*r10 + 32], ymm3
	vmovups	ymmword ptr [rcx + 4*r10 + 64], ymm4
	vmovups	ymmword ptr [rcx + 4*r10 + 96], ymm5
	add	r10, 32
	cmp	r9, r10
	jne	.LBB3_22
	cmp	rdx, r9
	je	.LBB3_28
	test	dl, 28
	je	.LBB3_25
.LBB3_18:
	add	r8, 28
	and	r8, rdx
	lea	r10, [rcx + 4*r8]
	vbroadcastss	xmm1, xmm0
	.p2align	4
.LBB3_19:
	vaddps	xmm2, xmm1, xmmword ptr [rcx + 4*r9]
	vmovups	xmmword ptr [rcx + 4*r9], xmm2
	add	r9, 4
	cmp	r8, r9
	jne	.LBB3_19
	cmp	rdx, r8
	jne	.LBB3_26
	jmp	.LBB3_28
.LBB3_25:
	lea	r10, [rcx + 4*r9]
.LBB3_26:
	lea	rax, [rcx + 4*rax]
	.p2align	4
.LBB3_27:
	vaddss	xmm1, xmm0, dword ptr [r10]
	vmovss	dword ptr [r10], xmm1
	add	r10, 4
	cmp	r10, rax
	jne	.LBB3_27
.LBB3_28:
	mov	rax, qword ptr [rdi + 64]
	test	rax, rax
	je	.LBB3_42
	mov	rcx, qword ptr [rdi + 56]
	lea	rdi, [4*rax]
	vmovss	xmm0, dword ptr [rsi + 8]
	add	rdi, -4
	mov	r8, rcx
	cmp	rdi, 12
	jb	.LBB3_40
	mov	rdx, rdi
	shr	rdx, 2
	inc	rdx
	movabs	rsi, 9223372036854775776
	cmp	rdi, 124
	jae	.LBB3_35
	xor	edi, edi
	jmp	.LBB3_32
.LBB3_35:
	mov	rdi, rdx
	and	rdi, rsi
	vbroadcastss	ymm1, xmm0
	xor	r8d, r8d
	.p2align	4
.LBB3_36:
	vaddps	ymm2, ymm1, ymmword ptr [rcx + 4*r8]
	vaddps	ymm3, ymm1, ymmword ptr [rcx + 4*r8 + 32]
	vaddps	ymm4, ymm1, ymmword ptr [rcx + 4*r8 + 64]
	vaddps	ymm5, ymm1, ymmword ptr [rcx + 4*r8 + 96]
	vmovups	ymmword ptr [rcx + 4*r8], ymm2
	vmovups	ymmword ptr [rcx + 4*r8 + 32], ymm3
	vmovups	ymmword ptr [rcx + 4*r8 + 64], ymm4
	vmovups	ymmword ptr [rcx + 4*r8 + 96], ymm5
	add	r8, 32
	cmp	rdi, r8
	jne	.LBB3_36
	cmp	rdx, rdi
	je	.LBB3_42
	test	dl, 28
	je	.LBB3_39
.LBB3_32:
	add	rsi, 28
	and	rsi, rdx
	lea	r8, [rcx + 4*rsi]
	vbroadcastss	xmm1, xmm0
	.p2align	4
.LBB3_33:
	vaddps	xmm2, xmm1, xmmword ptr [rcx + 4*rdi]
	vmovups	xmmword ptr [rcx + 4*rdi], xmm2
	add	rdi, 4
	cmp	rsi, rdi
	jne	.LBB3_33
	cmp	rdx, rsi
	jne	.LBB3_40
	jmp	.LBB3_42
.LBB3_39:
	lea	r8, [rcx + 4*rdi]
.LBB3_40:
	lea	rax, [rcx + 4*rax]
	.p2align	4
.LBB3_41:
	vaddss	xmm1, xmm0, dword ptr [r8]
	vmovss	dword ptr [r8], xmm1
	add	r8, 4
	cmp	r8, rax
	jne	.LBB3_41
.LBB3_42:
	vzeroupper
	ret
.Lfunc_end3:
	.size	translate_soa, .Lfunc_end3-translate_soa
	.cfi_endproc

	.type	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.0,@object
	.section	.rodata.str1.1,"aMS",@progbits,1
.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.0:
	.asciz	"exp2_transform.rs"
	.size	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.0, 18

	.type	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.1,@object
	.section	.data.rel.ro..Lanon.3e617c0c78f3a4fbb978c968fbf35c95.1,"aw",@progbits
	.p2align	3, 0x0
.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.1:
	.quad	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.0
	.asciz	"\021\000\000\000\000\000\000\000/\000\000\000\026\000\000"
	.size	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.1, 24

	.type	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.2,@object
	.section	.data.rel.ro..Lanon.3e617c0c78f3a4fbb978c968fbf35c95.2,"aw",@progbits
	.p2align	3, 0x0
.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.2:
	.quad	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.0
	.asciz	"\021\000\000\000\000\000\000\000/\000\000\000!\000\000"
	.size	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.2, 24

	.type	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.3,@object
	.section	.data.rel.ro..Lanon.3e617c0c78f3a4fbb978c968fbf35c95.3,"aw",@progbits
	.p2align	3, 0x0
.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.3:
	.quad	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.0
	.asciz	"\021\000\000\000\000\000\000\000/\000\000\000,\000\000"
	.size	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.3, 24

	.type	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.4,@object
	.section	.data.rel.ro..Lanon.3e617c0c78f3a4fbb978c968fbf35c95.4,"aw",@progbits
	.p2align	3, 0x0
.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.4:
	.quad	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.0
	.asciz	"\021\000\000\000\000\000\000\0009\000\000\000\f\000\000"
	.size	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.4, 24

	.type	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.5,@object
	.section	.data.rel.ro..Lanon.3e617c0c78f3a4fbb978c968fbf35c95.5,"aw",@progbits
	.p2align	3, 0x0
.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.5:
	.quad	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.0
	.asciz	"\021\000\000\000\000\000\000\000:\000\000\000\f\000\000"
	.size	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.5, 24

	.type	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.6,@object
	.section	.data.rel.ro..Lanon.3e617c0c78f3a4fbb978c968fbf35c95.6,"aw",@progbits
	.p2align	3, 0x0
.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.6:
	.quad	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.0
	.asciz	"\021\000\000\000\000\000\000\000;\000\000\000\f\000\000"
	.size	.Lanon.3e617c0c78f3a4fbb978c968fbf35c95.6, 24

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
