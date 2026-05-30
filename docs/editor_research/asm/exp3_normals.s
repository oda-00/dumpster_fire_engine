	.intel_syntax noprefix
	.file	"exp3_normals.1b1aa9d5efafc89f-cgu.0"
	.section	.text.face_normals_indexed,"ax",@progbits
	.globl	face_normals_indexed
	.p2align	4
	.type	face_normals_indexed,@function
face_normals_indexed:
	.cfi_startproc
	push	r15
	.cfi_def_cfa_offset 16
	push	r14
	.cfi_def_cfa_offset 24
	push	r12
	.cfi_def_cfa_offset 32
	push	rbx
	.cfi_def_cfa_offset 40
	push	rax
	.cfi_def_cfa_offset 48
	.cfi_offset rbx, -40
	.cfi_offset r12, -32
	.cfi_offset r14, -24
	.cfi_offset r15, -16
	test	r9, r9
	je	.LBB0_9
	mov	rax, rdx
	cmp	rcx, 3
	mov	edx, 2
	cmovae	rdx, rcx
	lea	r9, [r9 + 2*r9]
	movabs	r10, -6148914691236517205
	mulx	r10, r10, r10
	lea	rdx, [r8 + 4*r9]
	shr	r10
	lea	r14, [r10 + 2*r10]
	add	r14, 3
	xor	ebx, ebx
	.p2align	4
.LBB0_2:
	cmp	rbx, rcx
	jae	.LBB0_10
	mov	r9d, dword ptr [rax + 4*rbx]
	cmp	rsi, r9
	jbe	.LBB0_12
	lea	r10, [rbx + 1]
	cmp	r10, rcx
	jae	.LBB0_13
	mov	r10d, dword ptr [rax + 4*rbx + 4]
	cmp	rsi, r10
	jbe	.LBB0_14
	lea	r15, [rbx + 3]
	cmp	r14, r15
	je	.LBB0_15
	mov	r11d, dword ptr [rax + 4*rbx + 8]
	cmp	rsi, r11
	jbe	.LBB0_16
	lea	r12, [r8 + 4*rbx]
	lea	r9, [r9 + 2*r9]
	vmovsd	xmm0, qword ptr [rdi + 4*r9]
	vmovss	xmm1, dword ptr [rdi + 4*r9 + 8]
	lea	r9, [r10 + 2*r10]
	vmovsd	xmm2, qword ptr [rdi + 4*r9]
	vmovss	xmm3, dword ptr [rdi + 4*r9 + 8]
	lea	r9, [r11 + 2*r11]
	vbroadcastss	xmm4, xmm2
	vblendps	xmm3, xmm4, xmm3, 1
	vbroadcastss	xmm4, xmm0
	vblendps	xmm1, xmm4, xmm1, 1
	vsubps	xmm3, xmm3, xmm1
	vsubps	xmm2, xmm2, xmm0
	vmovsd	xmm4, qword ptr [rdi + 4*r9]
	vsubps	xmm0, xmm4, xmm0
	vbroadcastss	xmm4, xmm4
	vmovss	xmm5, dword ptr [rdi + 4*r9 + 8]
	vblendps	xmm4, xmm4, xmm5, 1
	vsubps	xmm1, xmm4, xmm1
	vmovshdup	xmm4, xmm0
	vmulss	xmm4, xmm3, xmm4
	vmovshdup	xmm5, xmm2
	vmulss	xmm5, xmm5, xmm1
	vsubss	xmm4, xmm5, xmm4
	vmulps	xmm0, xmm3, xmm0
	vmulps	xmm1, xmm2, xmm1
	vsubps	xmm0, xmm0, xmm1
	vmovss	dword ptr [r12], xmm4
	vmovlps	qword ptr [r12 + 4], xmm0
	add	r12, 12
	mov	rbx, r15
	cmp	r12, rdx
	jne	.LBB0_2
.LBB0_9:
	add	rsp, 8
	.cfi_def_cfa_offset 40
	pop	rbx
	.cfi_def_cfa_offset 32
	pop	r12
	.cfi_def_cfa_offset 24
	pop	r14
	.cfi_def_cfa_offset 16
	pop	r15
	.cfi_def_cfa_offset 8
	ret
.LBB0_15:
	.cfi_def_cfa_offset 48
	add	rbx, 2
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.5]
	mov	rdi, rbx
	mov	rsi, rcx
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_16:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.6]
	mov	rdi, r11
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_14:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.4]
	mov	rdi, r10
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_13:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.3]
	mov	rdi, r10
	mov	rsi, rcx
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_12:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.2]
	mov	rdi, r9
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_10:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.1]
	mov	rdi, rbx
	mov	rsi, rcx
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.Lfunc_end0:
	.size	face_normals_indexed, .Lfunc_end0-face_normals_indexed
	.cfi_endproc

	.section	.text.face_normals_soa,"ax",@progbits
	.globl	face_normals_soa
	.p2align	4
	.type	face_normals_soa,@function
face_normals_soa:
	.cfi_startproc
	push	rbp
	.cfi_def_cfa_offset 16
	push	r15
	.cfi_def_cfa_offset 24
	push	r14
	.cfi_def_cfa_offset 32
	push	r13
	.cfi_def_cfa_offset 40
	push	r12
	.cfi_def_cfa_offset 48
	push	rbx
	.cfi_def_cfa_offset 56
	sub	rsp, 248
	.cfi_def_cfa_offset 304
	.cfi_offset rbx, -56
	.cfi_offset r12, -48
	.cfi_offset r13, -40
	.cfi_offset r14, -32
	.cfi_offset r15, -24
	.cfi_offset rbp, -16
	mov	qword ptr [rsp + 16], r8
	mov	qword ptr [rsp + 160], rsi
	mov	qword ptr [rsp + 24], rdx
	test	rdx, rdx
	je	.LBB1_26
	mov	rdx, qword ptr [rsp + 304]
	mov	rsi, qword ptr [rdi + 88]
	mov	rax, qword ptr [rdi + 8]
	mov	qword ptr [rsp], rax
	mov	rbp, qword ptr [rdi + 16]
	mov	rax, qword ptr [rdi + 80]
	mov	qword ptr [rsp + 8], rax
	mov	r8, qword ptr [rdi + 112]
	mov	r12, qword ptr [rdi + 208]
	cmp	r12, rdx
	mov	r13, rdx
	cmovb	r13, r12
	mov	r10, qword ptr [rdi + 40]
	mov	rax, qword ptr [rsp + 16]
	cmp	r13, rax
	cmovae	r13, rax
	mov	rax, qword ptr [rsp + 24]
	dec	rax
	cmp	r13, rax
	cmovae	r13, rax
	mov	r11, qword ptr [rdi + 184]
	cmp	r13, r11
	cmovae	r13, r11
	mov	rbx, qword ptr [rdi + 160]
	cmp	r13, rbx
	cmovae	r13, rbx
	mov	r14, qword ptr [rdi + 64]
	cmp	r13, r14
	cmovae	r13, r14
	mov	r15, qword ptr [rdi + 136]
	cmp	r13, r15
	cmovae	r13, r15
	mov	rax, qword ptr [rdi + 32]
	mov	qword ptr [rsp + 144], rax
	cmp	r13, r10
	cmovae	r13, r10
	cmp	r13, r8
	cmovae	r13, r8
	mov	rax, qword ptr [rdi + 104]
	mov	qword ptr [rsp + 104], rax
	mov	rax, qword ptr [rdi + 56]
	mov	qword ptr [rsp + 136], rax
	mov	rax, qword ptr [rdi + 128]
	mov	qword ptr [rsp + 96], rax
	mov	rax, qword ptr [rdi + 152]
	mov	qword ptr [rsp + 120], rax
	mov	rax, qword ptr [rdi + 176]
	mov	qword ptr [rsp + 112], rax
	mov	rax, qword ptr [rdi + 200]
	mov	qword ptr [rsp + 128], rax
	cmp	r13, rbp
	cmovae	r13, rbp
	cmp	r13, rsi
	cmovae	r13, rsi
	cmp	r13, 7
	mov	qword ptr [rsp + 152], rcx
	mov	qword ptr [rsp + 88], rsi
	mov	qword ptr [rsp + 80], rbp
	mov	qword ptr [rsp + 72], r8
	mov	qword ptr [rsp + 64], r12
	mov	qword ptr [rsp + 56], r10
	mov	qword ptr [rsp + 48], r11
	mov	qword ptr [rsp + 40], rbx
	mov	qword ptr [rsp + 32], r14
	mov	qword ptr [rsp + 176], r9
	ja	.LBB1_12
	xor	r13d, r13d
	jmp	.LBB1_3
.LBB1_12:
	inc	r13
	mov	eax, r13d
	and	eax, 7
	mov	ecx, 8
	cmovne	rcx, rax
	sub	r13, rcx
	mov	rcx, qword ptr [rsp + 152]
	xor	eax, eax
	mov	rdi, qword ptr [rsp + 160]
	mov	rdx, qword ptr [rsp]
	mov	rsi, qword ptr [rsp + 8]
	mov	rbp, qword ptr [rsp + 144]
	mov	r8, qword ptr [rsp + 136]
	mov	r10, qword ptr [rsp + 128]
	mov	r12, qword ptr [rsp + 120]
	mov	r11, qword ptr [rsp + 112]
	mov	rbx, qword ptr [rsp + 104]
	mov	r14, qword ptr [rsp + 96]
	.p2align	4
.LBB1_13:
	vmovups	ymm0, ymmword ptr [rsi + 4*rax]
	vmovups	ymm1, ymmword ptr [rdx + 4*rax]
	vsubps	ymm0, ymm0, ymm1
	vmovups	ymm2, ymmword ptr [rbx + 4*rax]
	vmovups	ymm3, ymmword ptr [rbp + 4*rax]
	vsubps	ymm2, ymm2, ymm3
	vmovups	ymm4, ymmword ptr [r14 + 4*rax]
	vmovups	ymm5, ymmword ptr [r8 + 4*rax]
	vsubps	ymm4, ymm4, ymm5
	vmovups	ymm6, ymmword ptr [r12 + 4*rax]
	vsubps	ymm1, ymm6, ymm1
	vmovups	ymm6, ymmword ptr [r11 + 4*rax]
	vsubps	ymm3, ymm6, ymm3
	vmulps	ymm6, ymm4, ymm3
	vmovups	ymm7, ymmword ptr [r10 + 4*rax]
	vsubps	ymm5, ymm7, ymm5
	vmulps	ymm7, ymm2, ymm5
	vsubps	ymm6, ymm7, ymm6
	vmovups	ymmword ptr [rdi + 4*rax], ymm6
	vmulps	ymm5, ymm0, ymm5
	vmulps	ymm4, ymm4, ymm1
	vsubps	ymm4, ymm4, ymm5
	vmovups	ymmword ptr [rcx + 4*rax], ymm4
	vmulps	ymm1, ymm2, ymm1
	vmulps	ymm0, ymm0, ymm3
	vsubps	ymm0, ymm0, ymm1
	vmovups	ymmword ptr [r9 + 4*rax], ymm0
	add	rax, 8
	cmp	r13, rax
	jne	.LBB1_13
	mov	rsi, qword ptr [rsp + 88]
	mov	rdx, qword ptr [rsp + 304]
	mov	rbp, qword ptr [rsp + 80]
	mov	r8, qword ptr [rsp + 72]
	mov	r10, qword ptr [rsp + 56]
	mov	r12, qword ptr [rsp + 64]
	mov	r11, qword ptr [rsp + 48]
	mov	rbx, qword ptr [rsp + 40]
	mov	r14, qword ptr [rsp + 32]
.LBB1_3:
	sub	rdx, r13
	mov	qword ptr [rsp + 184], rdx
	mov	rax, qword ptr [rsp + 16]
	sub	rax, r13
	mov	qword ptr [rsp + 192], rax
	sub	r12, r13
	mov	qword ptr [rsp + 200], r12
	sub	r11, r13
	mov	qword ptr [rsp + 208], r11
	sub	rbx, r13
	mov	qword ptr [rsp + 216], rbx
	sub	r14, r13
	mov	qword ptr [rsp + 224], r14
	mov	qword ptr [rsp + 168], r15
	sub	r15, r13
	mov	qword ptr [rsp + 232], r15
	sub	r10, r13
	mov	qword ptr [rsp + 240], r10
	mov	r12, r8
	sub	r12, r13
	sub	rbp, r13
	mov	rcx, rsi
	sub	rcx, r13
	sub	qword ptr [rsp + 24], r13
	mov	rax, qword ptr [rsp + 8]
	lea	rax, [rax + 4*r13]
	mov	qword ptr [rsp + 8], rax
	mov	rax, qword ptr [rsp]
	lea	rax, [rax + 4*r13]
	mov	qword ptr [rsp], rax
	mov	rsi, qword ptr [rsp + 104]
	lea	rax, [rsi + 4*r13]
	mov	rsi, qword ptr [rsp + 144]
	lea	rdi, [rsi + 4*r13]
	mov	rsi, qword ptr [rsp + 96]
	lea	r8, [rsi + 4*r13]
	mov	rsi, qword ptr [rsp + 136]
	lea	r10, [rsi + 4*r13]
	mov	rsi, qword ptr [rsp + 120]
	lea	r11, [rsi + 4*r13]
	mov	rsi, qword ptr [rsp + 112]
	lea	rbx, [rsi + 4*r13]
	mov	rsi, qword ptr [rsp + 160]
	lea	r14, [rsi + 4*r13]
	mov	rsi, qword ptr [rsp + 128]
	lea	r9, [rsi + 4*r13]
	mov	rsi, qword ptr [rsp + 152]
	lea	r15, [rsi + 4*r13]
	mov	rsi, qword ptr [rsp + 176]
	lea	r13, [rsi + 4*r13]
	xor	esi, esi
	.p2align	4
.LBB1_4:
	cmp	rcx, rsi
	je	.LBB1_15
	cmp	rbp, rsi
	je	.LBB1_16
	cmp	r12, rsi
	je	.LBB1_17
	cmp	qword ptr [rsp + 240], rsi
	je	.LBB1_18
	cmp	qword ptr [rsp + 232], rsi
	je	.LBB1_19
	cmp	qword ptr [rsp + 224], rsi
	je	.LBB1_20
	cmp	qword ptr [rsp + 216], rsi
	je	.LBB1_11
	cmp	qword ptr [rsp + 208], rsi
	je	.LBB1_28
	cmp	qword ptr [rsp + 200], rsi
	je	.LBB1_29
	mov	rdx, qword ptr [rsp + 8]
	vmovss	xmm3, dword ptr [rdx + 4*rsi]
	mov	rdx, qword ptr [rsp]
	vmovss	xmm2, dword ptr [rdx + 4*rsi]
	vmovss	xmm0, dword ptr [rax + 4*rsi]
	vmovss	xmm1, dword ptr [rdi + 4*rsi]
	vsubss	xmm0, xmm0, xmm1
	vmovss	xmm4, dword ptr [r8 + 4*rsi]
	vmovss	xmm6, dword ptr [r10 + 4*rsi]
	vsubss	xmm4, xmm4, xmm6
	vmovss	xmm5, dword ptr [r11 + 4*rsi]
	vmovss	xmm7, dword ptr [rbx + 4*rsi]
	vsubss	xmm1, xmm7, xmm1
	vmulss	xmm7, xmm4, xmm1
	vmovss	xmm8, dword ptr [r9 + 4*rsi]
	vsubss	xmm6, xmm8, xmm6
	vmulss	xmm8, xmm0, xmm6
	vsubss	xmm7, xmm8, xmm7
	vmovss	dword ptr [r14 + 4*rsi], xmm7
	cmp	qword ptr [rsp + 192], rsi
	je	.LBB1_30
	vsubss	xmm3, xmm3, xmm2
	vsubss	xmm2, xmm5, xmm2
	vmulss	xmm5, xmm3, xmm6
	vmulss	xmm4, xmm4, xmm2
	vsubss	xmm4, xmm4, xmm5
	vmovss	dword ptr [r15 + 4*rsi], xmm4
	cmp	qword ptr [rsp + 184], rsi
	je	.LBB1_27
	vmulss	xmm0, xmm0, xmm2
	vmulss	xmm1, xmm3, xmm1
	vsubss	xmm0, xmm1, xmm0
	vmovss	dword ptr [r13 + 4*rsi], xmm0
	inc	rsi
	cmp	qword ptr [rsp + 24], rsi
	jne	.LBB1_4
.LBB1_26:
	add	rsp, 248
	.cfi_def_cfa_offset 56
	pop	rbx
	.cfi_def_cfa_offset 48
	pop	r12
	.cfi_def_cfa_offset 40
	pop	r13
	.cfi_def_cfa_offset 32
	pop	r14
	.cfi_def_cfa_offset 24
	pop	r15
	.cfi_def_cfa_offset 16
	pop	rbp
	.cfi_def_cfa_offset 8
	vzeroupper
	ret
.LBB1_15:
	.cfi_def_cfa_offset 304
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.7]
	mov	rdi, qword ptr [rsp + 88]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_16:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.8]
	mov	rdi, qword ptr [rsp + 80]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_17:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.9]
	mov	rdi, qword ptr [rsp + 72]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_18:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.10]
	mov	rdi, qword ptr [rsp + 56]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_19:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.11]
	mov	rdi, qword ptr [rsp + 168]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_20:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.12]
	mov	rdi, qword ptr [rsp + 32]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_11:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.13]
	mov	rdi, qword ptr [rsp + 40]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_28:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.14]
	mov	rdi, qword ptr [rsp + 48]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_29:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.15]
	mov	rdi, qword ptr [rsp + 64]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_30:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.16]
	mov	rdi, qword ptr [rsp + 16]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_27:
	lea	rdx, [rip + .Lanon.8a6002c7220e66987e0688fac3fedd59.17]
	mov	rdi, qword ptr [rsp + 304]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.Lfunc_end1:
	.size	face_normals_soa, .Lfunc_end1-face_normals_soa
	.cfi_endproc

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.0,@object
	.section	.rodata.str1.1,"aMS",@progbits,1
.Lanon.8a6002c7220e66987e0688fac3fedd59.0:
	.asciz	"exp3_normals.rs"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.0, 16

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.1,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.1,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.1:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000\027\000\000\000\025\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.1, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.2,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.2,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.2:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000\027\000\000\000\021\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.2, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.3,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.3,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.3:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000\030\000\000\000\025\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.3, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.4,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.4,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.4:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000\030\000\000\000\021\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.4, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.5,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.5,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.5:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000\031\000\000\000\025\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.5, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.6,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.6,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.6:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000\031\000\000\000\021\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.6, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.7,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.7,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.7:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000,\000\000\000\027\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.7, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.8,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.8,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.8:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000,\000\000\000!\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.8, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.9,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.9,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.9:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000-\000\000\000\027\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.9, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.10,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.10,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.10:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000-\000\000\000!\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.10, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.11,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.11,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.11:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000.\000\000\000\027\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.11, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.12,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.12,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.12:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000.\000\000\000!\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.12, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.13,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.13,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.13:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\000/\000\000\000\027\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.13, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.14,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.14,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.14:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\0000\000\000\000\027\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.14, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.15,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.15,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.15:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\0001\000\000\000\027\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.15, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.16,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.16,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.16:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\0003\000\000\000\t\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.16, 24

	.type	.Lanon.8a6002c7220e66987e0688fac3fedd59.17,@object
	.section	.data.rel.ro..Lanon.8a6002c7220e66987e0688fac3fedd59.17,"aw",@progbits
	.p2align	3, 0x0
.Lanon.8a6002c7220e66987e0688fac3fedd59.17:
	.quad	.Lanon.8a6002c7220e66987e0688fac3fedd59.0
	.asciz	"\017\000\000\000\000\000\000\0004\000\000\000\t\000\000"
	.size	.Lanon.8a6002c7220e66987e0688fac3fedd59.17, 24

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
