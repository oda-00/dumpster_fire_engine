	.intel_syntax noprefix
	.file	"exp3_drawlist.b11fc999342add8-cgu.0"
	.section	".text.unlikely._ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E","ax",@progbits
	.globl	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E
	.p2align	4
	.type	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E,@function
_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E:
	.cfi_startproc
	push	r14
	.cfi_def_cfa_offset 16
	push	rbx
	.cfi_def_cfa_offset 24
	sub	rsp, 24
	.cfi_def_cfa_offset 48
	.cfi_offset rbx, -24
	.cfi_offset r14, -16
	mov	rbx, rdi
	mov	rsi, qword ptr [rdi]
	lea	rax, [rsi + rsi]
	cmp	rax, 5
	mov	r14d, 4
	cmovae	r14, rax
	mov	rdx, qword ptr [rdi + 8]
	mov	rdi, rsp
	mov	r8d, 20
	mov	rcx, r14
	call	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h7f1452d37d61853bE
	cmp	dword ptr [rsp], 1
	je	.LBB0_2
	mov	rax, qword ptr [rsp + 8]
	mov	qword ptr [rbx + 8], rax
	mov	qword ptr [rbx], r14
	add	rsp, 24
	.cfi_def_cfa_offset 24
	pop	rbx
	.cfi_def_cfa_offset 16
	pop	r14
	.cfi_def_cfa_offset 8
	ret
.LBB0_2:
	.cfi_def_cfa_offset 48
	mov	rdi, qword ptr [rsp + 8]
	mov	rsi, qword ptr [rsp + 16]
	call	qword ptr [rip + _ZN5alloc7raw_vec12handle_error17hfa86a3a4628bd209E@GOTPCREL]
.Lfunc_end0:
	.size	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E, .Lfunc_end0-_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E
	.cfi_endproc

	.section	".text.unlikely._ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h7f1452d37d61853bE","ax",@progbits
	.p2align	4
	.type	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h7f1452d37d61853bE,@function
_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h7f1452d37d61853bE:
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
	mov	r9, rdx
	lea	eax, [r8 + 3]
	and	eax, 60
	mul	rcx
	mov	r14, rax
	mov	rbx, rdi
	seto	al
	movabs	rcx, 9223372036854775804
	cmp	r14, rcx
	seta	cl
	or	cl, al
	mov	r15d, 1
	je	.LBB1_2
	mov	eax, 8
	xor	r14d, r14d
	jmp	.LBB1_10
.LBB1_2:
	test	rsi, rsi
	je	.LBB1_4
	imul	r8, rsi
	mov	edx, 4
	mov	rdi, r9
	mov	rsi, r8
	mov	rcx, r14
	call	qword ptr [rip + _RNvCs1Y7DaGC1cwg_7___rustc14___rust_realloc@GOTPCREL]
	test	rax, rax
	jne	.LBB1_6
	jmp	.LBB1_9
.LBB1_4:
	test	r14, r14
	je	.LBB1_5
	call	qword ptr [rip + _RNvCs1Y7DaGC1cwg_7___rustc35___rust_no_alloc_shim_is_unstable_v2@GOTPCREL]
	mov	esi, 4
	mov	rdi, r14
	call	qword ptr [rip + _RNvCs1Y7DaGC1cwg_7___rustc12___rust_alloc@GOTPCREL]
	test	rax, rax
	jne	.LBB1_6
.LBB1_9:
	mov	qword ptr [rbx + 8], 4
	mov	eax, 16
	jmp	.LBB1_10
.LBB1_5:
	mov	eax, 4
.LBB1_6:
	mov	qword ptr [rbx + 8], rax
	mov	eax, 16
	xor	r15d, r15d
.LBB1_10:
	mov	qword ptr [rbx + rax], r14
	mov	qword ptr [rbx], r15
	pop	rbx
	.cfi_def_cfa_offset 24
	pop	r14
	.cfi_def_cfa_offset 16
	pop	r15
	.cfi_def_cfa_offset 8
	ret
.Lfunc_end1:
	.size	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h7f1452d37d61853bE, .Lfunc_end1-_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h7f1452d37d61853bE
	.cfi_endproc

	.section	".text.unlikely._ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E","ax",@progbits
	.p2align	4
	.type	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E,@function
_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E:
	.cfi_startproc
	push	r14
	.cfi_def_cfa_offset 16
	push	rbx
	.cfi_def_cfa_offset 24
	sub	rsp, 24
	.cfi_def_cfa_offset 48
	.cfi_offset rbx, -24
	.cfi_offset r14, -16
	add	rsi, rdx
	jb	.LBB2_1
	mov	r8, rcx
	mov	rbx, rdi
	mov	rax, qword ptr [rdi]
	lea	rcx, [rax + rax]
	cmp	rsi, rcx
	cmova	rcx, rsi
	cmp	rcx, 5
	mov	r14d, 4
	cmovae	r14, rcx
	mov	rdx, qword ptr [rdi + 8]
	mov	rdi, rsp
	mov	rsi, rax
	mov	rcx, r14
	call	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h7f1452d37d61853bE
	cmp	dword ptr [rsp], 1
	je	.LBB2_3
	mov	rax, qword ptr [rsp + 8]
	mov	qword ptr [rbx + 8], rax
	mov	qword ptr [rbx], r14
	add	rsp, 24
	.cfi_def_cfa_offset 24
	pop	rbx
	.cfi_def_cfa_offset 16
	pop	r14
	.cfi_def_cfa_offset 8
	ret
.LBB2_1:
	.cfi_def_cfa_offset 48
	xor	edi, edi
	call	qword ptr [rip + _ZN5alloc7raw_vec12handle_error17hfa86a3a4628bd209E@GOTPCREL]
.LBB2_3:
	mov	rdi, qword ptr [rsp + 8]
	mov	rsi, qword ptr [rsp + 16]
	call	qword ptr [rip + _ZN5alloc7raw_vec12handle_error17hfa86a3a4628bd209E@GOTPCREL]
.Lfunc_end2:
	.size	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E, .Lfunc_end2-_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E
	.cfi_endproc

	.section	.text.push_rect_aos,"ax",@progbits
	.globl	push_rect_aos
	.p2align	4
	.type	push_rect_aos,@function
push_rect_aos:
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
	sub	rsp, 40
	.cfi_def_cfa_offset 96
	.cfi_offset rbx, -56
	.cfi_offset r12, -48
	.cfi_offset r13, -40
	.cfi_offset r14, -32
	.cfi_offset r15, -24
	.cfi_offset rbp, -16
	mov	rax, qword ptr [rdi]
	vmovsd	xmm3, qword ptr [rdx]
	vmovshdup	xmm0, xmm3
	vaddss	xmm1, xmm3, dword ptr [rdx + 8]
	vaddss	xmm2, xmm0, dword ptr [rdx + 12]
	mov	rbx, qword ptr [rdi + 16]
	vmovd	r15d, xmm1
	vmovd	r13d, xmm0
	vmovd	edx, xmm2
	vmovd	r12d, xmm3
	shl	r13, 32
	or	r13, r15
	shl	rdx, 32
	or	r15, rdx
	or	r12, rdx
	sub	rax, rbx
	mov	rdx, rbx
	cmp	rax, 3
	jbe	.LBB3_1
.LBB3_2:
	mov	rax, qword ptr [rdi + 8]
	lea	r8, [rdx + 4*rdx]
	vmovlpd	qword ptr [rax + 4*r8], xmm3
	mov	qword ptr [rax + 4*r8 + 8], 0
	mov	dword ptr [rax + 4*r8 + 16], ecx
	mov	qword ptr [rax + 4*r8 + 20], r13
	mov	qword ptr [rax + 4*r8 + 28], 0
	mov	dword ptr [rax + 4*r8 + 36], ecx
	mov	qword ptr [rax + 4*r8 + 40], r15
	mov	qword ptr [rax + 4*r8 + 48], 0
	mov	dword ptr [rax + 4*r8 + 56], ecx
	mov	qword ptr [rax + 4*r8 + 60], r12
	mov	qword ptr [rax + 4*r8 + 68], 0
	mov	dword ptr [rax + 4*r8 + 76], ecx
	add	rdx, 4
	mov	qword ptr [rdi + 16], rdx
	mov	rcx, qword ptr [rsi]
	mov	rax, qword ptr [rsi + 16]
	sub	rcx, rax
	cmp	rcx, 5
	jbe	.LBB3_3
.LBB3_4:
	lea	ecx, [rbx + 3]
	lea	edx, [rbx + 2]
	lea	edi, [rbx + 1]
	mov	r8, qword ptr [rsi + 8]
	mov	dword ptr [r8 + 4*rax], ebx
	mov	dword ptr [r8 + 4*rax + 4], edi
	mov	dword ptr [r8 + 4*rax + 8], edx
	mov	dword ptr [r8 + 4*rax + 12], ebx
	mov	dword ptr [r8 + 4*rax + 16], edx
	mov	dword ptr [r8 + 4*rax + 20], ecx
	add	rax, 6
	mov	qword ptr [rsi + 16], rax
	add	rsp, 40
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
	ret
.LBB3_1:
	.cfi_def_cfa_offset 96
	mov	edx, 4
	mov	dword ptr [rsp + 12], ecx
	mov	ecx, 20
	mov	r14, rdi
	mov	rbp, rsi
	mov	rsi, rbx
	vmovaps	xmmword ptr [rsp + 16], xmm3
	call	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E
	vmovaps	xmm3, xmmword ptr [rsp + 16]
	mov	ecx, dword ptr [rsp + 12]
	mov	rdi, r14
	mov	rsi, rbp
	mov	rdx, qword ptr [r14 + 16]
	jmp	.LBB3_2
.LBB3_3:
	mov	edx, 6
	mov	ecx, 4
	mov	rdi, rsi
	mov	r14, rsi
	mov	rsi, rax
	call	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E
	mov	rsi, r14
	mov	rax, qword ptr [r14 + 16]
	jmp	.LBB3_4
.Lfunc_end3:
	.size	push_rect_aos, .Lfunc_end3-push_rect_aos
	.cfi_endproc

	.section	.text.push_rect_soa,"ax",@progbits
	.globl	push_rect_soa
	.p2align	4
	.type	push_rect_soa,@function
push_rect_soa:
	.cfi_startproc
	push	rbp
	.cfi_def_cfa_offset 16
	push	r14
	.cfi_def_cfa_offset 24
	push	rbx
	.cfi_def_cfa_offset 32
	sub	rsp, 32
	.cfi_def_cfa_offset 64
	.cfi_offset rbx, -32
	.cfi_offset r14, -24
	.cfi_offset rbp, -16
	vmovsd	xmm1, qword ptr [rsi]
	vmovsd	xmm0, qword ptr [rsi + 8]
	vaddps	xmm0, xmm1, xmm0
	mov	rax, qword ptr [rdi]
	mov	rsi, qword ptr [rdi + 16]
	sub	rax, rsi
	cmp	rax, 7
	jbe	.LBB4_1
.LBB4_2:
	mov	rax, qword ptr [rdi + 8]
	vmovlps	qword ptr [rax + 4*rsi], xmm1
	vmovss	dword ptr [rax + 4*rsi + 8], xmm0
	vextractps	dword ptr [rax + 4*rsi + 12], xmm1, 1
	vmovlps	qword ptr [rax + 4*rsi + 16], xmm0
	vmovss	dword ptr [rax + 4*rsi + 24], xmm1
	vextractps	dword ptr [rax + 4*rsi + 28], xmm0, 1
	add	rsi, 8
	mov	qword ptr [rdi + 16], rsi
	mov	rax, qword ptr [rdi + 24]
	mov	rsi, qword ptr [rdi + 40]
	sub	rax, rsi
	cmp	rax, 7
	jbe	.LBB4_3
.LBB4_4:
	mov	rax, qword ptr [rdi + 32]
	vxorps	xmm0, xmm0, xmm0
	vmovups	ymmword ptr [rax + 4*rsi], ymm0
	add	rsi, 8
	mov	qword ptr [rdi + 40], rsi
	mov	rax, qword ptr [rdi + 48]
	mov	rsi, qword ptr [rdi + 64]
	sub	rax, rsi
	cmp	rax, 3
	jbe	.LBB4_5
.LBB4_6:
	mov	rax, qword ptr [rdi + 56]
	vmovd	xmm0, edx
	vpbroadcastd	xmm0, xmm0
	vmovdqu	xmmword ptr [rax + 4*rsi], xmm0
	add	rsi, 4
	mov	qword ptr [rdi + 64], rsi
	add	rsp, 32
	.cfi_def_cfa_offset 32
	pop	rbx
	.cfi_def_cfa_offset 24
	pop	r14
	.cfi_def_cfa_offset 16
	pop	rbp
	.cfi_def_cfa_offset 8
	vzeroupper
	ret
.LBB4_1:
	.cfi_def_cfa_offset 64
	mov	ebp, edx
	mov	edx, 8
	mov	ecx, 4
	mov	rbx, rdi
	vmovaps	xmmword ptr [rsp + 16], xmm1
	vmovaps	xmmword ptr [rsp], xmm0
	call	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E
	vmovaps	xmm0, xmmword ptr [rsp]
	vmovaps	xmm1, xmmword ptr [rsp + 16]
	mov	edx, ebp
	mov	rdi, rbx
	mov	rsi, qword ptr [rbx + 16]
	jmp	.LBB4_2
.LBB4_3:
	lea	rax, [rdi + 24]
	mov	ebx, edx
	mov	edx, 8
	mov	ecx, 4
	mov	r14, rdi
	mov	rdi, rax
	call	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E
	mov	edx, ebx
	mov	rdi, r14
	mov	rsi, qword ptr [r14 + 40]
	jmp	.LBB4_4
.LBB4_5:
	lea	rax, [rdi + 48]
	mov	ebx, edx
	mov	edx, 4
	mov	ecx, 4
	mov	r14, rdi
	mov	rdi, rax
	vzeroupper
	call	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17h0aa11323a4211e79E
	mov	edx, ebx
	mov	rdi, r14
	mov	rsi, qword ptr [r14 + 64]
	jmp	.LBB4_6
.Lfunc_end4:
	.size	push_rect_soa, .Lfunc_end4-push_rect_soa
	.cfi_endproc

	.section	.text.push_rects_bulk,"ax",@progbits
	.globl	push_rects_bulk
	.p2align	4
	.type	push_rects_bulk,@function
push_rects_bulk:
	.cfi_startproc
	test	rdx, rdx
	je	.LBB5_12
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
	sub	rsp, 40
	.cfi_def_cfa_offset 96
	.cfi_offset rbx, -56
	.cfi_offset r12, -48
	.cfi_offset r13, -40
	.cfi_offset r14, -32
	.cfi_offset r15, -24
	.cfi_offset rbp, -16
	mov	ebx, ecx
	mov	r14, rdx
	mov	r15, rsi
	mov	r12, rdi
	shl	r14, 4
	add	r14, rsi
	mov	r13, qword ptr [rdi + 16]
	lea	rax, [4*r13]
	lea	rbp, [rax + 4*rax]
	jmp	.LBB5_2
	.p2align	4
.LBB5_10:
	mov	rax, qword ptr [r12 + 8]
	vmovss	dword ptr [rax + rbp + 60], xmm1
	vextractps	dword ptr [rax + rbp + 64], xmm2, 1
	mov	qword ptr [rax + rbp + 68], 0
	mov	dword ptr [rax + rbp + 76], ebx
	inc	r13
	mov	qword ptr [r12 + 16], r13
	add	rbp, 80
	add	r15, 16
	cmp	r15, r14
	je	.LBB5_11
.LBB5_2:
	vmovq	xmm1, qword ptr [r15]
	vmovsd	xmm2, qword ptr [r15 + 8]
	mov	rax, qword ptr [r12]
	cmp	r13, rax
	je	.LBB5_3
.LBB5_4:
	mov	rcx, qword ptr [r12 + 8]
	vmovq	xmm0, xmm1
	vmovdqu	xmmword ptr [rcx + rbp], xmm0
	mov	dword ptr [rcx + rbp + 16], ebx
	inc	r13
	mov	qword ptr [r12 + 16], r13
	cmp	r13, rax
	je	.LBB5_5
.LBB5_6:
	vaddps	xmm2, xmm1, xmm2
	vinsertps	xmm0, xmm2, xmm1, 92
	vmovups	xmmword ptr [rcx + rbp + 20], xmm0
	mov	dword ptr [rcx + rbp + 36], ebx
	inc	r13
	mov	qword ptr [r12 + 16], r13
	cmp	r13, rax
	je	.LBB5_7
.LBB5_8:
	vmovq	xmm0, xmm2
	vmovdqu	xmmword ptr [rcx + rbp + 40], xmm0
	mov	dword ptr [rcx + rbp + 56], ebx
	inc	r13
	mov	qword ptr [r12 + 16], r13
	cmp	r13, qword ptr [r12]
	jne	.LBB5_10
	mov	rdi, r12
	vmovaps	xmmword ptr [rsp + 16], xmm1
	vmovaps	xmmword ptr [rsp], xmm2
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E@GOTPCREL]
	vmovaps	xmm2, xmmword ptr [rsp]
	vmovaps	xmm1, xmmword ptr [rsp + 16]
	jmp	.LBB5_10
.LBB5_3:
	mov	rdi, r12
	vmovdqa	xmmword ptr [rsp + 16], xmm1
	vmovaps	xmmword ptr [rsp], xmm2
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E@GOTPCREL]
	vmovaps	xmm2, xmmword ptr [rsp]
	vmovdqa	xmm1, xmmword ptr [rsp + 16]
	mov	rax, qword ptr [r12]
	jmp	.LBB5_4
.LBB5_5:
	mov	rdi, r12
	vmovdqa	xmmword ptr [rsp + 16], xmm1
	vmovaps	xmmword ptr [rsp], xmm2
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E@GOTPCREL]
	vmovaps	xmm2, xmmword ptr [rsp]
	vmovdqa	xmm1, xmmword ptr [rsp + 16]
	mov	rax, qword ptr [r12]
	mov	rcx, qword ptr [r12 + 8]
	jmp	.LBB5_6
.LBB5_7:
	mov	rdi, r12
	vmovaps	xmmword ptr [rsp + 16], xmm1
	vmovaps	xmmword ptr [rsp], xmm2
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h68edcb6632e11de6E@GOTPCREL]
	vmovaps	xmm2, xmmword ptr [rsp]
	vmovaps	xmm1, xmmword ptr [rsp + 16]
	mov	rcx, qword ptr [r12 + 8]
	jmp	.LBB5_8
.LBB5_11:
	add	rsp, 40
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
	.cfi_restore rbx
	.cfi_restore r12
	.cfi_restore r13
	.cfi_restore r14
	.cfi_restore r15
	.cfi_restore rbp
.LBB5_12:
	ret
.Lfunc_end5:
	.size	push_rects_bulk, .Lfunc_end5-push_rects_bulk
	.cfi_endproc

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
