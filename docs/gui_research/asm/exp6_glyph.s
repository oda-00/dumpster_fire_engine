	.intel_syntax noprefix
	.file	"exp6_glyph.4970c63dc151bb11-cgu.0"
	.section	".text.unlikely._ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE","ax",@progbits
	.globl	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE
	.p2align	4
	.type	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE,@function
_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE:
	.cfi_startproc
	push	rax
	.cfi_def_cfa_offset 16
	mov	rsi, qword ptr [rdi]
	call	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$14grow_amortized17hd7fe622e01a70ac7E
	movabs	rcx, -9223372036854775807
	cmp	rax, rcx
	jne	.LBB0_2
	pop	rax
	.cfi_def_cfa_offset 8
	ret
.LBB0_2:
	.cfi_def_cfa_offset 16
	mov	rdi, rax
	mov	rsi, rdx
	call	qword ptr [rip + _ZN5alloc7raw_vec12handle_error17hfa86a3a4628bd209E@GOTPCREL]
.Lfunc_end0:
	.size	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE, .Lfunc_end0-_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE
	.cfi_endproc

	.section	".text.unlikely._ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h68aff3b93b17da97E","ax",@progbits
	.p2align	4
	.type	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h68aff3b93b17da97E,@function
_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h68aff3b93b17da97E:
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
	mov	rbx, rdi
	mov	r15d, 1
	movabs	rax, 461168601842738790
	cmp	rcx, rax
	jbe	.LBB1_2
	mov	eax, 8
	xor	r14d, r14d
	jmp	.LBB1_10
.LBB1_2:
	shl	rcx, 2
	lea	r14, [rcx + 4*rcx]
	test	rsi, rsi
	je	.LBB1_4
	shl	rsi, 2
	lea	rsi, [rsi + 4*rsi]
	mov	rdi, rdx
	mov	edx, 4
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
	.size	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h68aff3b93b17da97E, .Lfunc_end1-_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h68aff3b93b17da97E
	.cfi_endproc

	.section	".text.unlikely._ZN5alloc7raw_vec20RawVecInner$LT$A$GT$14grow_amortized17hd7fe622e01a70ac7E","ax",@progbits
	.p2align	4
	.type	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$14grow_amortized17hd7fe622e01a70ac7E,@function
_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$14grow_amortized17hd7fe622e01a70ac7E:
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
	inc	rsi
	mov	rax, qword ptr [rdi]
	lea	rcx, [rax + rax]
	cmp	rsi, rcx
	cmovbe	rsi, rcx
	cmp	rsi, 5
	mov	r14d, 4
	cmovae	r14, rsi
	mov	rdx, qword ptr [rdi + 8]
	mov	rdi, rsp
	mov	rsi, rax
	mov	rcx, r14
	call	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$11finish_grow17h68aff3b93b17da97E
	cmp	byte ptr [rsp], 0
	je	.LBB2_2
	mov	rax, qword ptr [rsp + 8]
	mov	rdx, qword ptr [rsp + 16]
	add	rsp, 24
	.cfi_def_cfa_offset 24
	pop	rbx
	.cfi_def_cfa_offset 16
	pop	r14
	.cfi_def_cfa_offset 8
	ret
.LBB2_2:
	.cfi_def_cfa_offset 48
	mov	rax, qword ptr [rsp + 8]
	mov	qword ptr [rbx + 8], rax
	mov	qword ptr [rbx], r14
	movabs	rax, -9223372036854775807
	add	rsp, 24
	.cfi_def_cfa_offset 24
	pop	rbx
	.cfi_def_cfa_offset 16
	pop	r14
	.cfi_def_cfa_offset 8
	ret
.Lfunc_end2:
	.size	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$14grow_amortized17hd7fe622e01a70ac7E, .Lfunc_end2-_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$14grow_amortized17hd7fe622e01a70ac7E
	.cfi_endproc

	.section	.text.push_text,"ax",@progbits
	.globl	push_text
	.p2align	4
	.type	push_text,@function
push_text:
	.cfi_startproc
	test	rdx, rdx
	je	.LBB3_19
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
	sub	rsp, 120
	.cfi_def_cfa_offset 176
	.cfi_offset rbx, -56
	.cfi_offset r12, -48
	.cfi_offset r13, -40
	.cfi_offset r14, -32
	.cfi_offset r15, -24
	.cfi_offset rbp, -16
	mov	ebx, r9d
	mov	r14, rcx
	mov	r12, rsi
	mov	r13, rdi
	add	rdx, rsi
	vmovd	xmm2, r8d
	vmovq	xmm3, r8
	mov	qword ptr [rsp + 8], rdx
	vmovdqa	xmmword ptr [rsp + 96], xmm3
	jmp	.LBB3_2
	.p2align	4
.LBB3_17:
	mov	rax, qword ptr [r13 + 8]
	lea	rcx, [r15 + 4*r15]
	vmovlhps	xmm0, xmm1, xmm5
	vmovlhps	xmm1, xmm6, xmm7
	vshufps	xmm0, xmm1, xmm0, 204
	vmovups	xmmword ptr [rax + 4*rcx], xmm0
	mov	dword ptr [rax + 4*rcx + 16], ebx
	add	rbp, 4
	mov	qword ptr [r13 + 16], rbp
	vaddss	xmm2, xmm2, xmm4
	cmp	r12, rdx
	je	.LBB3_18
.LBB3_2:
	movzx	edi, byte ptr [r12]
	test	dil, dil
	js	.LBB3_3
	inc	r12
	mov	eax, edi
	jmp	.LBB3_9
	.p2align	4
.LBB3_3:
	mov	eax, edi
	and	eax, 31
	movzx	esi, byte ptr [r12 + 1]
	and	esi, 63
	cmp	dil, -33
	jbe	.LBB3_4
	movzx	ecx, byte ptr [r12 + 2]
	shl	esi, 6
	and	ecx, 63
	or	ecx, esi
	cmp	dil, -16
	jb	.LBB3_7
	movzx	esi, byte ptr [r12 + 3]
	add	r12, 4
	and	eax, 7
	shl	eax, 18
	shl	ecx, 6
	and	esi, 63
	or	esi, ecx
	or	esi, eax
	mov	eax, esi
	jmp	.LBB3_9
.LBB3_4:
	add	r12, 2
	shl	eax, 6
	or	eax, esi
	jmp	.LBB3_9
.LBB3_7:
	add	r12, 3
	shl	eax, 12
	or	ecx, eax
	mov	eax, ecx
	.p2align	4
.LBB3_9:
	and	eax, 127
	lea	rax, [rax + 8*rax]
	vmovsd	xmm1, qword ptr [r14 + 4*rax]
	vmovsd	xmm5, qword ptr [r14 + 4*rax + 8]
	vmovsd	xmm7, qword ptr [r14 + 4*rax + 16]
	vmovss	xmm4, dword ptr [r14 + 4*rax + 24]
	vmovsd	xmm6, qword ptr [r14 + 4*rax + 28]
	mov	rax, qword ptr [r13]
	mov	rbp, qword ptr [r13 + 16]
	cmp	rbp, rax
	je	.LBB3_10
.LBB3_11:
	vpblendd	xmm0, xmm3, xmm2, 1
	vaddps	xmm6, xmm0, xmm6
	mov	rcx, qword ptr [r13 + 8]
	lea	rsi, [4*rbp]
	add	rsi, rbp
	vmovlhps	xmm0, xmm6, xmm1
	vmovups	xmmword ptr [rcx + 4*rsi], xmm0
	mov	dword ptr [rcx + 4*rsi + 16], ebx
	lea	r15, [rbp + 1]
	mov	qword ptr [r13 + 16], r15
	cmp	r15, rax
	je	.LBB3_12
.LBB3_13:
	vaddps	xmm7, xmm7, xmm6
	lea	rsi, [r15 + 4*r15]
	vblendps	xmm0, xmm7, xmm6, 2
	vmovlps	qword ptr [rcx + 4*rsi], xmm0
	vmovss	dword ptr [rcx + 4*rsi + 8], xmm5
	vextractps	dword ptr [rcx + 4*rsi + 12], xmm1, 1
	mov	dword ptr [rcx + 4*rsi + 16], ebx
	lea	r15, [rbp + 2]
	mov	qword ptr [r13 + 16], r15
	cmp	r15, rax
	je	.LBB3_14
.LBB3_15:
	lea	rax, [r15 + 4*r15]
	vmovlhps	xmm0, xmm7, xmm5
	vmovups	xmmword ptr [rcx + 4*rax], xmm0
	mov	dword ptr [rcx + 4*rax + 16], ebx
	lea	r15, [rbp + 3]
	mov	qword ptr [r13 + 16], r15
	cmp	r15, qword ptr [r13]
	jne	.LBB3_17
	mov	rdi, r13
	vmovdqa	xmmword ptr [rsp + 80], xmm2
	vmovss	dword ptr [rsp + 4], xmm4
	vmovaps	xmmword ptr [rsp + 64], xmm1
	vmovaps	xmmword ptr [rsp + 48], xmm5
	vmovaps	xmmword ptr [rsp + 32], xmm6
	vmovaps	xmmword ptr [rsp + 16], xmm7
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE@GOTPCREL]
	vmovaps	xmm7, xmmword ptr [rsp + 16]
	vmovaps	xmm6, xmmword ptr [rsp + 32]
	vmovaps	xmm5, xmmword ptr [rsp + 48]
	vmovaps	xmm1, xmmword ptr [rsp + 64]
	vmovss	xmm4, dword ptr [rsp + 4]
	vmovdqa	xmm3, xmmword ptr [rsp + 96]
	vmovdqa	xmm2, xmmword ptr [rsp + 80]
	mov	rdx, qword ptr [rsp + 8]
	jmp	.LBB3_17
.LBB3_10:
	mov	rdi, r13
	vmovdqa	xmmword ptr [rsp + 80], xmm2
	vmovss	dword ptr [rsp + 4], xmm4
	vmovaps	xmmword ptr [rsp + 64], xmm1
	vmovaps	xmmword ptr [rsp + 48], xmm5
	vmovaps	xmmword ptr [rsp + 16], xmm7
	vmovaps	xmmword ptr [rsp + 32], xmm6
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE@GOTPCREL]
	vmovaps	xmm6, xmmword ptr [rsp + 32]
	vmovaps	xmm7, xmmword ptr [rsp + 16]
	vmovaps	xmm5, xmmword ptr [rsp + 48]
	vmovaps	xmm1, xmmword ptr [rsp + 64]
	vmovss	xmm4, dword ptr [rsp + 4]
	vmovdqa	xmm3, xmmword ptr [rsp + 96]
	vmovdqa	xmm2, xmmword ptr [rsp + 80]
	mov	rdx, qword ptr [rsp + 8]
	mov	rax, qword ptr [r13]
	jmp	.LBB3_11
.LBB3_12:
	mov	rdi, r13
	vmovdqa	xmmword ptr [rsp + 80], xmm2
	vmovss	dword ptr [rsp + 4], xmm4
	vmovaps	xmmword ptr [rsp + 64], xmm1
	vmovaps	xmmword ptr [rsp + 48], xmm5
	vmovaps	xmmword ptr [rsp + 32], xmm6
	vmovaps	xmmword ptr [rsp + 16], xmm7
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE@GOTPCREL]
	vmovaps	xmm7, xmmword ptr [rsp + 16]
	vmovaps	xmm6, xmmword ptr [rsp + 32]
	vmovaps	xmm5, xmmword ptr [rsp + 48]
	vmovaps	xmm1, xmmword ptr [rsp + 64]
	vmovss	xmm4, dword ptr [rsp + 4]
	vmovdqa	xmm3, xmmword ptr [rsp + 96]
	vmovdqa	xmm2, xmmword ptr [rsp + 80]
	mov	rdx, qword ptr [rsp + 8]
	mov	rax, qword ptr [r13]
	mov	rcx, qword ptr [r13 + 8]
	jmp	.LBB3_13
.LBB3_14:
	mov	rdi, r13
	vmovdqa	xmmword ptr [rsp + 80], xmm2
	vmovss	dword ptr [rsp + 4], xmm4
	vmovaps	xmmword ptr [rsp + 64], xmm1
	vmovaps	xmmword ptr [rsp + 48], xmm5
	vmovaps	xmmword ptr [rsp + 32], xmm6
	vmovaps	xmmword ptr [rsp + 16], xmm7
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE@GOTPCREL]
	vmovaps	xmm7, xmmword ptr [rsp + 16]
	vmovaps	xmm6, xmmword ptr [rsp + 32]
	vmovaps	xmm5, xmmword ptr [rsp + 48]
	vmovaps	xmm1, xmmword ptr [rsp + 64]
	vmovss	xmm4, dword ptr [rsp + 4]
	vmovdqa	xmm3, xmmword ptr [rsp + 96]
	vmovdqa	xmm2, xmmword ptr [rsp + 80]
	mov	rdx, qword ptr [rsp + 8]
	mov	rcx, qword ptr [r13 + 8]
	jmp	.LBB3_15
.LBB3_18:
	add	rsp, 120
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
.LBB3_19:
	ret
.Lfunc_end3:
	.size	push_text, .Lfunc_end3-push_text
	.cfi_endproc

	.section	.text.push_text_ascii,"ax",@progbits
	.globl	push_text_ascii
	.p2align	4
	.type	push_text_ascii,@function
push_text_ascii:
	.cfi_startproc
	test	rdx, rdx
	je	.LBB4_12
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
	sub	rsp, 136
	.cfi_def_cfa_offset 192
	.cfi_offset rbx, -56
	.cfi_offset r12, -48
	.cfi_offset r13, -40
	.cfi_offset r14, -32
	.cfi_offset r15, -24
	.cfi_offset rbp, -16
	mov	ebx, r9d
	mov	r14, rcx
	mov	r13, rdi
	vmovd	xmm2, r8d
	mov	rbp, qword ptr [rdi + 16]
	vmovq	xmm3, r8
	lea	rax, [4*rbp]
	lea	r12, [rax + 4*rax]
	xor	r15d, r15d
	mov	qword ptr [rsp + 24], rdx
	mov	qword ptr [rsp + 16], rsi
	vmovdqa	xmmword ptr [rsp + 112], xmm3
	jmp	.LBB4_2
	.p2align	4
.LBB4_10:
	mov	rax, qword ptr [r13 + 8]
	vmovlhps	xmm0, xmm1, xmm5
	vmovlhps	xmm1, xmm6, xmm7
	vshufps	xmm0, xmm1, xmm0, 204
	vmovups	xmmword ptr [rax + r12 + 60], xmm0
	mov	dword ptr [rax + r12 + 76], ebx
	inc	rbp
	mov	qword ptr [r13 + 16], rbp
	vaddss	xmm2, xmm2, xmm4
	add	r12, 80
	inc	r15
	cmp	rdx, r15
	je	.LBB4_11
.LBB4_2:
	movzx	eax, byte ptr [rsi + r15]
	and	eax, 127
	lea	rax, [rax + 8*rax]
	vmovsd	xmm1, qword ptr [r14 + 4*rax]
	vmovsd	xmm5, qword ptr [r14 + 4*rax + 8]
	vmovsd	xmm7, qword ptr [r14 + 4*rax + 16]
	vmovss	xmm4, dword ptr [r14 + 4*rax + 24]
	vmovsd	xmm6, qword ptr [r14 + 4*rax + 28]
	mov	rax, qword ptr [r13]
	cmp	rbp, rax
	je	.LBB4_3
.LBB4_4:
	vpblendd	xmm0, xmm3, xmm2, 1
	vaddps	xmm6, xmm0, xmm6
	mov	rcx, qword ptr [r13 + 8]
	vmovlhps	xmm0, xmm6, xmm1
	vmovups	xmmword ptr [rcx + r12], xmm0
	mov	dword ptr [rcx + r12 + 16], ebx
	inc	rbp
	mov	qword ptr [r13 + 16], rbp
	cmp	rbp, rax
	je	.LBB4_5
.LBB4_6:
	vaddps	xmm7, xmm7, xmm6
	vblendps	xmm0, xmm7, xmm6, 2
	vmovlps	qword ptr [rcx + r12 + 20], xmm0
	vmovss	dword ptr [rcx + r12 + 28], xmm5
	vextractps	dword ptr [rcx + r12 + 32], xmm1, 1
	mov	dword ptr [rcx + r12 + 36], ebx
	inc	rbp
	mov	qword ptr [r13 + 16], rbp
	cmp	rbp, rax
	je	.LBB4_7
.LBB4_8:
	vmovlhps	xmm0, xmm7, xmm5
	vmovups	xmmword ptr [rcx + r12 + 40], xmm0
	mov	dword ptr [rcx + r12 + 56], ebx
	inc	rbp
	mov	qword ptr [r13 + 16], rbp
	cmp	rbp, qword ptr [r13]
	jne	.LBB4_10
	mov	rdi, r13
	vmovdqa	xmmword ptr [rsp + 96], xmm2
	vmovss	dword ptr [rsp + 12], xmm4
	vmovaps	xmmword ptr [rsp + 80], xmm1
	vmovaps	xmmword ptr [rsp + 64], xmm5
	vmovaps	xmmword ptr [rsp + 48], xmm6
	vmovaps	xmmword ptr [rsp + 32], xmm7
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE@GOTPCREL]
	vmovaps	xmm7, xmmword ptr [rsp + 32]
	vmovaps	xmm6, xmmword ptr [rsp + 48]
	vmovaps	xmm5, xmmword ptr [rsp + 64]
	vmovaps	xmm1, xmmword ptr [rsp + 80]
	vmovss	xmm4, dword ptr [rsp + 12]
	vmovdqa	xmm3, xmmword ptr [rsp + 112]
	vmovdqa	xmm2, xmmword ptr [rsp + 96]
	mov	rsi, qword ptr [rsp + 16]
	mov	rdx, qword ptr [rsp + 24]
	jmp	.LBB4_10
.LBB4_3:
	mov	rdi, r13
	vmovdqa	xmmword ptr [rsp + 96], xmm2
	vmovss	dword ptr [rsp + 12], xmm4
	vmovaps	xmmword ptr [rsp + 80], xmm1
	vmovaps	xmmword ptr [rsp + 64], xmm5
	vmovaps	xmmword ptr [rsp + 32], xmm7
	vmovaps	xmmword ptr [rsp + 48], xmm6
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE@GOTPCREL]
	vmovaps	xmm6, xmmword ptr [rsp + 48]
	vmovaps	xmm7, xmmword ptr [rsp + 32]
	vmovaps	xmm5, xmmword ptr [rsp + 64]
	vmovaps	xmm1, xmmword ptr [rsp + 80]
	vmovss	xmm4, dword ptr [rsp + 12]
	vmovdqa	xmm3, xmmword ptr [rsp + 112]
	vmovdqa	xmm2, xmmword ptr [rsp + 96]
	mov	rsi, qword ptr [rsp + 16]
	mov	rdx, qword ptr [rsp + 24]
	mov	rax, qword ptr [r13]
	jmp	.LBB4_4
.LBB4_5:
	mov	rdi, r13
	vmovdqa	xmmword ptr [rsp + 96], xmm2
	vmovss	dword ptr [rsp + 12], xmm4
	vmovaps	xmmword ptr [rsp + 80], xmm1
	vmovaps	xmmword ptr [rsp + 64], xmm5
	vmovaps	xmmword ptr [rsp + 48], xmm6
	vmovaps	xmmword ptr [rsp + 32], xmm7
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE@GOTPCREL]
	vmovaps	xmm7, xmmword ptr [rsp + 32]
	vmovaps	xmm6, xmmword ptr [rsp + 48]
	vmovaps	xmm5, xmmword ptr [rsp + 64]
	vmovaps	xmm1, xmmword ptr [rsp + 80]
	vmovss	xmm4, dword ptr [rsp + 12]
	vmovdqa	xmm3, xmmword ptr [rsp + 112]
	vmovdqa	xmm2, xmmword ptr [rsp + 96]
	mov	rsi, qword ptr [rsp + 16]
	mov	rdx, qword ptr [rsp + 24]
	mov	rax, qword ptr [r13]
	mov	rcx, qword ptr [r13 + 8]
	jmp	.LBB4_6
.LBB4_7:
	mov	rdi, r13
	vmovdqa	xmmword ptr [rsp + 96], xmm2
	vmovss	dword ptr [rsp + 12], xmm4
	vmovaps	xmmword ptr [rsp + 80], xmm1
	vmovaps	xmmword ptr [rsp + 64], xmm5
	vmovaps	xmmword ptr [rsp + 48], xmm6
	vmovaps	xmmword ptr [rsp + 32], xmm7
	call	qword ptr [rip + _ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hceaeaba2cc44a0baE@GOTPCREL]
	vmovaps	xmm7, xmmword ptr [rsp + 32]
	vmovaps	xmm6, xmmword ptr [rsp + 48]
	vmovaps	xmm5, xmmword ptr [rsp + 64]
	vmovaps	xmm1, xmmword ptr [rsp + 80]
	vmovss	xmm4, dword ptr [rsp + 12]
	vmovdqa	xmm3, xmmword ptr [rsp + 112]
	vmovdqa	xmm2, xmmword ptr [rsp + 96]
	mov	rsi, qword ptr [rsp + 16]
	mov	rdx, qword ptr [rsp + 24]
	mov	rcx, qword ptr [r13 + 8]
	jmp	.LBB4_8
.LBB4_11:
	add	rsp, 136
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
.LBB4_12:
	ret
.Lfunc_end4:
	.size	push_text_ascii, .Lfunc_end4-push_text_ascii
	.cfi_endproc

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
