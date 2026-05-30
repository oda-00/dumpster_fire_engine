	.intel_syntax noprefix
	.file	"exp5_subdiv.c7c2d1f44777d00a-cgu.0"
	.section	.rodata.cst4,"aM",@progbits,4
	.p2align	2, 0x0
.LCPI0_0:
	.long	0x3e800000
	.section	.text.edge_points,"ax",@progbits
	.globl	edge_points
	.p2align	4
	.type	edge_points,@function
edge_points:
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
	push	rax
	.cfi_def_cfa_offset 64
	.cfi_offset rbx, -56
	.cfi_offset r12, -48
	.cfi_offset r13, -40
	.cfi_offset r14, -32
	.cfi_offset r15, -24
	.cfi_offset rbp, -16
	mov	r10, qword ptr [rsp + 88]
	test	r10, r10
	je	.LBB0_15
	mov	rax, rdi
	mov	r11, qword ptr [rsp + 80]
	mov	rdi, qword ptr [rsp + 72]
	cmp	rdi, r9
	mov	rbx, r9
	cmovb	rbx, rdi
	mov	r14, qword ptr [rsp + 64]
	cmp	rbx, rcx
	cmovae	rbx, rcx
	cmp	rbx, rsi
	cmovae	rbx, rsi
	lea	r15, [r10 - 1]
	cmp	rbx, r15
	cmovae	rbx, r15
	cmp	rbx, 31
	ja	.LBB0_9
	xor	ebx, ebx
	jmp	.LBB0_3
.LBB0_9:
	inc	rbx
	mov	r15d, ebx
	and	r15d, 31
	mov	r12d, 32
	cmovne	r12, r15
	sub	rbx, r12
	xor	r15d, r15d
	vbroadcastss	ymm0, dword ptr [rip + .LCPI0_0]
	.p2align	4
.LBB0_10:
	vmovups	ymm1, ymmword ptr [rax + 4*r15]
	vmovups	ymm2, ymmword ptr [rax + 4*r15 + 32]
	vmovups	ymm3, ymmword ptr [rax + 4*r15 + 64]
	vmovups	ymm4, ymmword ptr [rax + 4*r15 + 96]
	vaddps	ymm1, ymm1, ymmword ptr [rdx + 4*r15]
	vaddps	ymm2, ymm2, ymmword ptr [rdx + 4*r15 + 32]
	vaddps	ymm3, ymm3, ymmword ptr [rdx + 4*r15 + 64]
	vaddps	ymm4, ymm4, ymmword ptr [rdx + 4*r15 + 96]
	vaddps	ymm1, ymm1, ymmword ptr [r8 + 4*r15]
	vaddps	ymm2, ymm2, ymmword ptr [r8 + 4*r15 + 32]
	vaddps	ymm3, ymm3, ymmword ptr [r8 + 4*r15 + 64]
	vaddps	ymm4, ymm4, ymmword ptr [r8 + 4*r15 + 96]
	vaddps	ymm1, ymm1, ymmword ptr [r14 + 4*r15]
	vaddps	ymm2, ymm2, ymmword ptr [r14 + 4*r15 + 32]
	vaddps	ymm3, ymm3, ymmword ptr [r14 + 4*r15 + 64]
	vaddps	ymm4, ymm4, ymmword ptr [r14 + 4*r15 + 96]
	vmulps	ymm1, ymm1, ymm0
	vmulps	ymm2, ymm2, ymm0
	vmulps	ymm3, ymm3, ymm0
	vmulps	ymm4, ymm4, ymm0
	vmovups	ymmword ptr [r11 + 4*r15], ymm1
	vmovups	ymmword ptr [r11 + 4*r15 + 32], ymm2
	vmovups	ymmword ptr [r11 + 4*r15 + 64], ymm3
	vmovups	ymmword ptr [r11 + 4*r15 + 96], ymm4
	add	r15, 32
	cmp	rbx, r15
	jne	.LBB0_10
.LBB0_3:
	mov	r15, rdi
	sub	r15, rbx
	mov	r12, r9
	sub	r12, rbx
	mov	r13, rcx
	sub	r13, rbx
	mov	rbp, rsi
	sub	rbp, rbx
	sub	r10, rbx
	lea	rax, [rax + 4*rbx]
	lea	rdx, [rdx + 4*rbx]
	lea	r8, [r8 + 4*rbx]
	lea	r11, [r11 + 4*rbx]
	lea	rbx, [r14 + 4*rbx]
	xor	r14d, r14d
	vmovss	xmm0, dword ptr [rip + .LCPI0_0]
	.p2align	4
.LBB0_4:
	cmp	rbp, r14
	je	.LBB0_11
	cmp	r13, r14
	je	.LBB0_12
	cmp	r12, r14
	je	.LBB0_13
	cmp	r15, r14
	je	.LBB0_8
	vmovss	xmm1, dword ptr [rax + 4*r14]
	vaddss	xmm1, xmm1, dword ptr [rdx + 4*r14]
	vaddss	xmm1, xmm1, dword ptr [r8 + 4*r14]
	vaddss	xmm1, xmm1, dword ptr [rbx + 4*r14]
	vmulss	xmm1, xmm1, xmm0
	vmovss	dword ptr [r11 + 4*r14], xmm1
	inc	r14
	cmp	r10, r14
	jne	.LBB0_4
.LBB0_15:
	add	rsp, 8
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
.LBB0_11:
	.cfi_def_cfa_offset 64
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.1]
	mov	rdi, rsi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_12:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.2]
	mov	rdi, rcx
	mov	rsi, rcx
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_13:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.3]
	mov	rdi, r9
	mov	rsi, r9
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_8:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.4]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.Lfunc_end0:
	.size	edge_points, .Lfunc_end0-edge_points
	.cfi_endproc

	.section	.rodata.cst4,"aM",@progbits,4
	.p2align	2, 0x0
.LCPI1_0:
	.long	0x3e800000
	.section	.text.quad_face_points,"ax",@progbits
	.globl	quad_face_points
	.p2align	4
	.type	quad_face_points,@function
quad_face_points:
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
	sub	rsp, 184
	.cfi_def_cfa_offset 240
	.cfi_offset rbx, -56
	.cfi_offset r12, -48
	.cfi_offset r13, -40
	.cfi_offset r14, -32
	.cfi_offset r15, -24
	.cfi_offset rbp, -16
	mov	qword ptr [rsp + 24], r9
	mov	qword ptr [rsp + 40], r8
	mov	qword ptr [rsp + 16], rcx
	mov	qword ptr [rsp + 8], rsi
	mov	rax, qword ptr [rsp + 392]
	mov	qword ptr [rsp + 48], rax
	test	rax, rax
	je	.LBB1_26
	mov	rbx, rdi
	mov	qword ptr [rsp + 32], rdx
	mov	rdi, qword ptr [rsp + 424]
	mov	r11, qword ptr [rsp + 408]
	cmp	rdi, r11
	mov	rbp, r11
	cmovb	rbp, rdi
	mov	r14, qword ptr [rsp + 376]
	cmp	rbp, r14
	cmovae	rbp, r14
	mov	r15, qword ptr [rsp + 360]
	cmp	rbp, r15
	cmovae	rbp, r15
	mov	r12, qword ptr [rsp + 344]
	cmp	rbp, r12
	cmovae	rbp, r12
	mov	r13, qword ptr [rsp + 328]
	cmp	rbp, r13
	cmovae	rbp, r13
	mov	rdx, qword ptr [rsp + 312]
	cmp	rbp, rdx
	cmovae	rbp, rdx
	mov	rax, qword ptr [rsp + 296]
	cmp	rbp, rax
	cmovae	rbp, rax
	mov	rax, qword ptr [rsp + 280]
	cmp	rbp, rax
	cmovae	rbp, rax
	mov	rax, qword ptr [rsp + 264]
	cmp	rbp, rax
	cmovae	rbp, rax
	mov	rax, qword ptr [rsp + 248]
	cmp	rbp, rax
	cmovae	rbp, rax
	mov	rax, qword ptr [rsp + 24]
	cmp	rbp, rax
	cmovae	rbp, rax
	mov	rax, qword ptr [rsp + 16]
	cmp	rbp, rax
	cmovae	rbp, rax
	mov	rax, qword ptr [rsp + 8]
	cmp	rbp, rax
	cmovae	rbp, rax
	mov	rax, qword ptr [rsp + 392]
	dec	rax
	cmp	rbp, rax
	cmovae	rbp, rax
	mov	r9, qword ptr [rsp + 384]
	mov	rsi, qword ptr [rsp + 336]
	mov	rax, qword ptr [rsp + 304]
	mov	r8, qword ptr [rsp + 288]
	mov	rcx, qword ptr [rsp + 256]
	mov	r10, qword ptr [rsp + 240]
	cmp	rbp, 7
	ja	.LBB1_9
	xor	ebp, ebp
	jmp	.LBB1_3
.LBB1_9:
	inc	rbp
	mov	edx, ebp
	and	edx, 7
	mov	edi, 8
	cmovne	rdi, rdx
	sub	rbp, rdi
	xor	edx, edx
	vbroadcastss	ymm0, dword ptr [rip + .LCPI1_0]
	mov	rdi, qword ptr [rsp + 40]
	mov	r11, rbx
	mov	rbx, qword ptr [rsp + 32]
	mov	r12, qword ptr [rsp + 416]
	mov	r15, qword ptr [rsp + 368]
	mov	r14, qword ptr [rsp + 400]
	mov	r13, qword ptr [rsp + 320]
	mov	rcx, rsi
	mov	rsi, rax
	mov	rax, r9
	mov	r9, qword ptr [rsp + 352]
	mov	r10, qword ptr [rsp + 272]
	.p2align	4
.LBB1_10:
	vmovups	ymm1, ymmword ptr [r11 + 4*rdx]
	mov	r8, qword ptr [rsp + 240]
	vaddps	ymm1, ymm1, ymmword ptr [r8 + 4*rdx]
	mov	r8, qword ptr [rsp + 288]
	vaddps	ymm1, ymm1, ymmword ptr [r8 + 4*rdx]
	vaddps	ymm1, ymm1, ymmword ptr [rcx + 4*rdx]
	vmulps	ymm1, ymm1, ymm0
	vmovups	ymmword ptr [rax + 4*rdx], ymm1
	vmovups	ymm1, ymmword ptr [rbx + 4*rdx]
	mov	r8, qword ptr [rsp + 256]
	vaddps	ymm1, ymm1, ymmword ptr [r8 + 4*rdx]
	vaddps	ymm1, ymm1, ymmword ptr [rsi + 4*rdx]
	vaddps	ymm1, ymm1, ymmword ptr [r9 + 4*rdx]
	vmulps	ymm1, ymm1, ymm0
	vmovups	ymmword ptr [r14 + 4*rdx], ymm1
	vmovups	ymm1, ymmword ptr [rdi + 4*rdx]
	vaddps	ymm1, ymm1, ymmword ptr [r10 + 4*rdx]
	vaddps	ymm1, ymm1, ymmword ptr [r13 + 4*rdx]
	vaddps	ymm1, ymm1, ymmword ptr [r15 + 4*rdx]
	vmulps	ymm1, ymm1, ymm0
	vmovups	ymmword ptr [r12 + 4*rdx], ymm1
	add	rdx, 8
	cmp	rbp, rdx
	jne	.LBB1_10
	mov	rdx, qword ptr [rsp + 312]
	mov	rbx, r11
	mov	r11, qword ptr [rsp + 408]
	mov	rdi, qword ptr [rsp + 424]
	mov	r12, qword ptr [rsp + 344]
	mov	r15, qword ptr [rsp + 360]
	mov	r14, qword ptr [rsp + 376]
	mov	r13, qword ptr [rsp + 328]
	mov	r9, rax
	mov	rax, rsi
	mov	rsi, rcx
	mov	rcx, qword ptr [rsp + 256]
	mov	r8, qword ptr [rsp + 288]
	mov	r10, qword ptr [rsp + 240]
.LBB1_3:
	sub	rdi, rbp
	mov	qword ptr [rsp + 56], rdi
	sub	r14, rbp
	mov	qword ptr [rsp + 64], r14
	sub	r13, rbp
	mov	qword ptr [rsp + 72], r13
	mov	rdi, qword ptr [rsp + 280]
	sub	rdi, rbp
	mov	qword ptr [rsp + 80], rdi
	mov	rdi, qword ptr [rsp + 24]
	sub	rdi, rbp
	mov	qword ptr [rsp + 88], rdi
	sub	r11, rbp
	mov	qword ptr [rsp + 96], r11
	sub	r15, rbp
	mov	qword ptr [rsp + 104], r15
	sub	rdx, rbp
	mov	qword ptr [rsp + 112], rdx
	mov	rdx, qword ptr [rsp + 264]
	sub	rdx, rbp
	mov	qword ptr [rsp + 120], rdx
	mov	rdx, qword ptr [rsp + 16]
	sub	rdx, rbp
	mov	qword ptr [rsp + 160], rdx
	sub	r12, rbp
	mov	qword ptr [rsp + 168], r12
	mov	rdx, qword ptr [rsp + 296]
	sub	rdx, rbp
	mov	qword ptr [rsp + 176], rdx
	mov	r15, qword ptr [rsp + 248]
	sub	r15, rbp
	mov	rdi, qword ptr [rsp + 8]
	sub	rdi, rbp
	sub	qword ptr [rsp + 48], rbp
	lea	rdx, [rbx + 4*rbp]
	mov	qword ptr [rsp + 152], rdx
	lea	rdx, [r10 + 4*rbp]
	mov	qword ptr [rsp + 144], rdx
	lea	rdx, [r8 + 4*rbp]
	mov	qword ptr [rsp + 136], rdx
	lea	rdx, [r9 + 4*rbp]
	mov	qword ptr [rsp + 128], rdx
	lea	r9, [rsi + 4*rbp]
	mov	rsi, qword ptr [rsp + 32]
	lea	r12, [rsi + 4*rbp]
	lea	r13, [rcx + 4*rbp]
	lea	r14, [rax + 4*rbp]
	mov	rax, qword ptr [rsp + 352]
	lea	rax, [rax + 4*rbp]
	mov	rcx, qword ptr [rsp + 400]
	lea	rsi, [rcx + 4*rbp]
	mov	rcx, qword ptr [rsp + 40]
	lea	rcx, [rcx + 4*rbp]
	mov	rdx, qword ptr [rsp + 272]
	lea	r10, [rdx + 4*rbp]
	mov	rdx, qword ptr [rsp + 320]
	lea	r11, [rdx + 4*rbp]
	mov	r8, qword ptr [rsp + 368]
	lea	r8, [r8 + 4*rbp]
	mov	rbx, qword ptr [rsp + 416]
	lea	rbp, [rbx + 4*rbp]
	vmovss	xmm0, dword ptr [rip + .LCPI1_0]
	xor	ebx, ebx
	.p2align	4
.LBB1_4:
	cmp	rdi, rbx
	je	.LBB1_12
	cmp	r15, rbx
	je	.LBB1_13
	cmp	qword ptr [rsp + 176], rbx
	je	.LBB1_14
	cmp	qword ptr [rsp + 168], rbx
	je	.LBB1_8
	mov	rdx, qword ptr [rsp + 152]
	vmovss	xmm1, dword ptr [rdx + 4*rbx]
	mov	rdx, qword ptr [rsp + 144]
	vaddss	xmm1, xmm1, dword ptr [rdx + 4*rbx]
	mov	rdx, qword ptr [rsp + 136]
	vaddss	xmm1, xmm1, dword ptr [rdx + 4*rbx]
	vaddss	xmm1, xmm1, dword ptr [r9 + 4*rbx]
	vmulss	xmm1, xmm1, xmm0
	mov	rdx, qword ptr [rsp + 128]
	vmovss	dword ptr [rdx + 4*rbx], xmm1
	cmp	qword ptr [rsp + 160], rbx
	je	.LBB1_28
	cmp	qword ptr [rsp + 120], rbx
	je	.LBB1_29
	cmp	qword ptr [rsp + 112], rbx
	je	.LBB1_30
	cmp	qword ptr [rsp + 104], rbx
	je	.LBB1_31
	cmp	qword ptr [rsp + 96], rbx
	je	.LBB1_32
	vmovss	xmm1, dword ptr [r12 + 4*rbx]
	vaddss	xmm1, xmm1, dword ptr [r13 + 4*rbx]
	vaddss	xmm1, xmm1, dword ptr [r14 + 4*rbx]
	vaddss	xmm1, xmm1, dword ptr [rax + 4*rbx]
	vmulss	xmm1, xmm1, xmm0
	vmovss	dword ptr [rsi + 4*rbx], xmm1
	cmp	qword ptr [rsp + 88], rbx
	je	.LBB1_33
	cmp	qword ptr [rsp + 80], rbx
	je	.LBB1_34
	cmp	qword ptr [rsp + 72], rbx
	je	.LBB1_35
	cmp	qword ptr [rsp + 64], rbx
	je	.LBB1_36
	cmp	qword ptr [rsp + 56], rbx
	je	.LBB1_27
	vmovss	xmm1, dword ptr [rcx + 4*rbx]
	vaddss	xmm1, xmm1, dword ptr [r10 + 4*rbx]
	vaddss	xmm1, xmm1, dword ptr [r11 + 4*rbx]
	vaddss	xmm1, xmm1, dword ptr [r8 + 4*rbx]
	vmulss	xmm1, xmm1, xmm0
	vmovss	dword ptr [rbp + 4*rbx], xmm1
	inc	rbx
	cmp	qword ptr [rsp + 48], rbx
	jne	.LBB1_4
.LBB1_26:
	add	rsp, 184
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
.LBB1_12:
	.cfi_def_cfa_offset 240
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.5]
	mov	rdi, qword ptr [rsp + 8]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_13:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.6]
	mov	rdi, qword ptr [rsp + 248]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_14:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.7]
	mov	rdi, qword ptr [rsp + 296]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_8:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.8]
	mov	rdi, qword ptr [rsp + 344]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_28:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.9]
	mov	rdi, qword ptr [rsp + 16]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_29:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.10]
	mov	rdi, qword ptr [rsp + 264]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_30:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.11]
	mov	rdi, qword ptr [rsp + 312]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_31:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.12]
	mov	rdi, qword ptr [rsp + 360]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_32:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.13]
	mov	rdi, qword ptr [rsp + 408]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_33:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.14]
	mov	rdi, qword ptr [rsp + 24]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_34:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.15]
	mov	rdi, qword ptr [rsp + 280]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_35:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.16]
	mov	rdi, qword ptr [rsp + 328]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_36:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.17]
	mov	rdi, qword ptr [rsp + 376]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_27:
	lea	rdx, [rip + .Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.18]
	mov	rdi, qword ptr [rsp + 424]
	mov	rsi, rdi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.Lfunc_end1:
	.size	quad_face_points, .Lfunc_end1-quad_face_points
	.cfi_endproc

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0,@object
	.section	.rodata.str1.1,"aMS",@progbits,1
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0:
	.asciz	"exp5_subdiv.rs"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0, 15

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.1,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.1,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.1:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000%\000\000\000\023\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.1, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.2,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.2,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.2:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000%\000\000\000\033\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.2, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.3,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.3,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.3:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000%\000\000\000#\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.3, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.4,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.4,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.4:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000%\000\000\000+\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.4, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.5,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.5,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.5:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\027\000\000\000\022\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.5, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.6,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.6,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.6:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\027\000\000\000\032\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.6, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.7,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.7,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.7:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\027\000\000\000\"\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.7, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.8,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.8,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.8:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\027\000\000\000*\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.8, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.9,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.9,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.9:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\030\000\000\000\022\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.9, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.10,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.10,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.10:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\030\000\000\000\032\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.10, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.11,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.11,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.11:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\030\000\000\000\"\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.11, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.12,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.12,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.12:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\030\000\000\000*\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.12, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.13,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.13,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.13:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\030\000\000\000\t\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.13, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.14,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.14,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.14:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\031\000\000\000\022\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.14, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.15,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.15,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.15:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\031\000\000\000\032\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.15, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.16,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.16,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.16:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\031\000\000\000\"\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.16, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.17,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.17,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.17:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\031\000\000\000*\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.17, 24

	.type	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.18,@object
	.section	.data.rel.ro..Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.18,"aw",@progbits
	.p2align	3, 0x0
.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.18:
	.quad	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.0
	.asciz	"\016\000\000\000\000\000\000\000\031\000\000\000\t\000\000"
	.size	.Lanon.06089fe5e5edb9bfa2a58ec7ed53850f.18, 24

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
