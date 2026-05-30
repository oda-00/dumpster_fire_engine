	.intel_syntax noprefix
	.file	"exp4_select_upload.4d8d6ed1b9bed43d-cgu.0"
	.section	.rodata.cst4,"aM",@progbits,4
	.p2align	2, 0x0
.LCPI0_0:
	.long	1
.LCPI0_3:
	.short	1
	.short	1
	.section	.rodata.cst32,"aM",@progbits,32
	.p2align	5, 0x0
.LCPI0_1:
	.byte	0
	.byte	1
	.byte	4
	.byte	5
	.byte	8
	.byte	9
	.byte	12
	.byte	13
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.byte	16
	.byte	17
	.byte	20
	.byte	21
	.byte	24
	.byte	25
	.byte	28
	.byte	29
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.section	.rodata.cst16,"aM",@progbits,16
	.p2align	4, 0x0
.LCPI0_2:
	.short	1
	.short	1
	.short	1
	.short	1
	.short	1
	.short	1
	.short	1
	.short	1
	.section	.text.box_select_soa,"ax",@progbits
	.globl	box_select_soa
	.p2align	4
	.type	box_select_soa,@function
box_select_soa:
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
	mov	rax, qword ptr [rsp + 72]
	test	rax, rax
	je	.LBB0_13
	mov	r11, qword ptr [rsp + 88]
	mov	rbx, qword ptr [rsp + 80]
	mov	r10, qword ptr [rsp + 64]
	vmovss	xmm0, dword ptr [r11]
	vmovss	xmm1, dword ptr [rbx + 8]
	vmovsd	xmm3, qword ptr [rbx]
	vmovsd	xmm2, qword ptr [r11 + 4]
	cmp	r9, rcx
	mov	r11, rcx
	cmovb	r11, r9
	cmp	r11, rsi
	cmovae	r11, rsi
	lea	rbx, [rax - 1]
	cmp	r11, rbx
	cmovae	r11, rbx
	cmp	r11, 7
	ja	.LBB0_8
	xor	r11d, r11d
	jmp	.LBB0_3
.LBB0_8:
	inc	r11
	mov	ebx, r11d
	and	ebx, 7
	mov	r14d, 8
	cmovne	r14, rbx
	sub	r11, r14
	vbroadcastss	ymm4, xmm3
	vbroadcastss	ymm5, xmm0
	vbroadcastss	ymm9, dword ptr [rip + .LCPI0_0]
	vpermps	ymm6, ymm9, ymm3
	vbroadcastss	ymm7, xmm2
	vbroadcastss	ymm8, xmm1
	vpermps	ymm9, ymm9, ymm2
	xor	ebx, ebx
	vmovdqa	ymm10, ymmword ptr [rip + .LCPI0_1]
	vpbroadcastd	xmm11, dword ptr [rip + .LCPI0_3]
	.p2align	4
.LBB0_9:
	vmovups	ymm12, ymmword ptr [rdi + 4*rbx]
	vcmpleps	ymm13, ymm4, ymm12
	vcmpleps	ymm12, ymm12, ymm5
	vmovups	ymm14, ymmword ptr [rdx + 4*rbx]
	vcmpleps	ymm15, ymm6, ymm14
	vandps	ymm13, ymm13, ymm15
	vcmpleps	ymm14, ymm14, ymm7
	vandps	ymm12, ymm12, ymm14
	vandps	ymm12, ymm13, ymm12
	vmovups	ymm13, ymmword ptr [r8 + 4*rbx]
	vcmpleps	ymm14, ymm8, ymm13
	vcmpleps	ymm13, ymm13, ymm9
	vandps	ymm13, ymm14, ymm13
	vandps	ymm12, ymm12, ymm13
	vpshufb	ymm12, ymm12, ymm10
	vpermq	ymm12, ymm12, 232
	vpand	xmm12, xmm12, xmm11
	vpackuswb	xmm12, xmm12, xmm12
	vmovq	qword ptr [r10 + rbx], xmm12
	add	rbx, 8
	cmp	r11, rbx
	jne	.LBB0_9
.LBB0_3:
	vmovddup	xmm3, xmm3
	mov	rbx, r9
	sub	rbx, r11
	mov	r14, rcx
	sub	r14, r11
	mov	r15, rsi
	sub	r15, r11
	sub	rax, r11
	lea	rdi, [rdi + 4*r11]
	lea	rdx, [rdx + 4*r11]
	add	r10, r11
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
	vmovss	xmm4, dword ptr [rdi + 4*r11]
	vucomiss	xmm0, xmm4
	setae	bpl
	vmovss	xmm5, dword ptr [r8 + 4*r11]
	vucomiss	xmm5, xmm1
	setae	r12b
	vmovss	xmm6, dword ptr [rdx + 4*r11]
	vblendps	xmm7, xmm3, xmm6, 3
	vinsertps	xmm5, xmm7, xmm5, 16
	vinsertps	xmm4, xmm2, xmm4, 32
	vinsertps	xmm4, xmm4, xmm6, 48
	vcmpnleps	xmm4, xmm5, xmm4
	vtestps	xmm4, xmm4
	sete	r13b
	and	r12b, bpl
	and	r12b, r13b
	mov	byte ptr [r10 + r11], r12b
	inc	r11
	cmp	rax, r11
	jne	.LBB0_4
.LBB0_13:
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
.LBB0_10:
	.cfi_def_cfa_offset 64
	lea	rdx, [rip + .Lanon.1caba9cb7805f6637afb478735c2d9d5.1]
	mov	rdi, rsi
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_11:
	lea	rdx, [rip + .Lanon.1caba9cb7805f6637afb478735c2d9d5.2]
	mov	rdi, rcx
	mov	rsi, rcx
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB0_7:
	lea	rdx, [rip + .Lanon.1caba9cb7805f6637afb478735c2d9d5.3]
	mov	rdi, r9
	mov	rsi, r9
	vzeroupper
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.Lfunc_end0:
	.size	box_select_soa, .Lfunc_end0-box_select_soa
	.cfi_endproc

	.section	.rodata.cst16,"aM",@progbits,16
	.p2align	4, 0x0
.LCPI1_0:
	.byte	1
	.byte	1
	.byte	1
	.byte	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.zero	1
	.section	.rodata.cst4,"aM",@progbits,4
	.p2align	2, 0x0
.LCPI1_1:
	.zero	4,1
	.section	.text.count_selected,"ax",@progbits
	.globl	count_selected
	.p2align	4
	.type	count_selected,@function
count_selected:
	.cfi_startproc
	test	rsi, rsi
	je	.LBB1_1
	cmp	rsi, 4
	jae	.LBB1_5
	xor	eax, eax
	mov	r8, rdi
	jmp	.LBB1_15
.LBB1_1:
	xor	eax, eax
	ret
.LBB1_5:
	movabs	rcx, 9223372036854775792
	cmp	rsi, 16
	jae	.LBB1_10
	xor	edx, edx
	xor	eax, eax
	jmp	.LBB1_7
.LBB1_10:
	mov	rdx, rsi
	and	rdx, rcx
	vpxor	xmm0, xmm0, xmm0
	xor	eax, eax
	vpbroadcastd	xmm1, dword ptr [rip + .LCPI1_1]
	vpxor	xmm2, xmm2, xmm2
	vpxor	xmm3, xmm3, xmm3
	vpxor	xmm4, xmm4, xmm4
	.p2align	4
.LBB1_11:
	vmovd	xmm5, dword ptr [rdi + rax]
	vmovd	xmm6, dword ptr [rdi + rax + 4]
	vmovd	xmm7, dword ptr [rdi + rax + 8]
	vmovd	xmm8, dword ptr [rdi + rax + 12]
	vpand	xmm5, xmm5, xmm1
	vpand	xmm6, xmm6, xmm1
	vpand	xmm7, xmm7, xmm1
	vpand	xmm8, xmm8, xmm1
	vpmovzxbq	ymm5, xmm5
	vpaddq	ymm0, ymm0, ymm5
	vpmovzxbq	ymm5, xmm6
	vpaddq	ymm2, ymm2, ymm5
	vpmovzxbq	ymm5, xmm7
	vpaddq	ymm3, ymm3, ymm5
	vpmovzxbq	ymm5, xmm8
	vpaddq	ymm4, ymm4, ymm5
	add	rax, 16
	cmp	rdx, rax
	jne	.LBB1_11
	vpaddq	ymm0, ymm2, ymm0
	vpaddq	ymm0, ymm3, ymm0
	vpaddq	ymm0, ymm4, ymm0
	vextracti128	xmm1, ymm0, 1
	vpaddq	xmm0, xmm0, xmm1
	vpshufd	xmm1, xmm0, 238
	vpaddq	xmm0, xmm0, xmm1
	vmovq	rax, xmm0
	cmp	rsi, rdx
	je	.LBB1_2
	test	sil, 12
	je	.LBB1_14
.LBB1_7:
	add	rcx, 12
	and	rcx, rsi
	lea	r8, [rdi + rcx]
	vmovq	xmm0, rax
	vpbroadcastd	xmm1, dword ptr [rip + .LCPI1_1]
	.p2align	4
.LBB1_8:
	vmovd	xmm2, dword ptr [rdi + rdx]
	vpand	xmm2, xmm2, xmm1
	vpmovzxbq	ymm2, xmm2
	vpaddq	ymm0, ymm0, ymm2
	add	rdx, 4
	cmp	rcx, rdx
	jne	.LBB1_8
	vextracti128	xmm1, ymm0, 1
	vpaddq	xmm0, xmm0, xmm1
	vpshufd	xmm1, xmm0, 238
	vpaddq	xmm0, xmm0, xmm1
	vmovq	rax, xmm0
	cmp	rsi, rcx
	jne	.LBB1_15
.LBB1_2:
	vzeroupper
	ret
.LBB1_14:
	add	rdx, rdi
	mov	r8, rdx
.LBB1_15:
	add	rdi, rsi
	.p2align	4
.LBB1_16:
	movzx	ecx, byte ptr [r8]
	inc	r8
	and	ecx, 1
	add	rax, rcx
	cmp	r8, rdi
	jne	.LBB1_16
	jmp	.LBB1_2
.Lfunc_end1:
	.size	count_selected, .Lfunc_end1-count_selected
	.cfi_endproc

	.section	.text.upload_dirty_range,"ax",@progbits
	.globl	upload_dirty_range
	.p2align	4
	.type	upload_dirty_range,@function
upload_dirty_range:
	.cfi_startproc
	mov	rax, rsi
	lea	rsi, [r8 + rcx]
	cmp	rsi, rcx
	jb	.LBB2_3
	cmp	rsi, rdx
	ja	.LBB2_3
	lea	rsi, [rax + 4*rcx]
	lea	rdi, [rdi + 4*rcx]
	shl	r8, 2
	mov	rdx, r8
	jmp	qword ptr [rip + memcpy@GOTPCREL]
.LBB2_3:
	push	rax
	.cfi_def_cfa_offset 16
	lea	rax, [rip + .Lanon.1caba9cb7805f6637afb478735c2d9d5.4]
	mov	rdi, rcx
	mov	rcx, rax
	call	qword ptr [rip + _ZN4core5slice5index16slice_index_fail17hf1918ccaba3e9ba3E@GOTPCREL]
.Lfunc_end2:
	.size	upload_dirty_range, .Lfunc_end2-upload_dirty_range
	.cfi_endproc

	.type	.Lanon.1caba9cb7805f6637afb478735c2d9d5.0,@object
	.section	.rodata.str1.1,"aMS",@progbits,1
.Lanon.1caba9cb7805f6637afb478735c2d9d5.0:
	.asciz	"exp4_select_upload.rs"
	.size	.Lanon.1caba9cb7805f6637afb478735c2d9d5.0, 22

	.type	.Lanon.1caba9cb7805f6637afb478735c2d9d5.1,@object
	.section	.data.rel.ro..Lanon.1caba9cb7805f6637afb478735c2d9d5.1,"aw",@progbits
	.p2align	3, 0x0
.Lanon.1caba9cb7805f6637afb478735c2d9d5.1:
	.quad	.Lanon.1caba9cb7805f6637afb478735c2d9d5.0
	.asciz	"\025\000\000\000\000\000\000\000\034\000\000\000\027\000\000"
	.size	.Lanon.1caba9cb7805f6637afb478735c2d9d5.1, 24

	.type	.Lanon.1caba9cb7805f6637afb478735c2d9d5.2,@object
	.section	.data.rel.ro..Lanon.1caba9cb7805f6637afb478735c2d9d5.2,"aw",@progbits
	.p2align	3, 0x0
.Lanon.1caba9cb7805f6637afb478735c2d9d5.2:
	.quad	.Lanon.1caba9cb7805f6637afb478735c2d9d5.0
	.asciz	"\025\000\000\000\000\000\000\000\035\000\000\000\020\000\000"
	.size	.Lanon.1caba9cb7805f6637afb478735c2d9d5.2, 24

	.type	.Lanon.1caba9cb7805f6637afb478735c2d9d5.3,@object
	.section	.data.rel.ro..Lanon.1caba9cb7805f6637afb478735c2d9d5.3,"aw",@progbits
	.p2align	3, 0x0
.Lanon.1caba9cb7805f6637afb478735c2d9d5.3:
	.quad	.Lanon.1caba9cb7805f6637afb478735c2d9d5.0
	.asciz	"\025\000\000\000\000\000\000\000\036\000\000\000\020\000\000"
	.size	.Lanon.1caba9cb7805f6637afb478735c2d9d5.3, 24

	.type	.Lanon.1caba9cb7805f6637afb478735c2d9d5.4,@object
	.section	.data.rel.ro..Lanon.1caba9cb7805f6637afb478735c2d9d5.4,"aw",@progbits
	.p2align	3, 0x0
.Lanon.1caba9cb7805f6637afb478735c2d9d5.4:
	.quad	.Lanon.1caba9cb7805f6637afb478735c2d9d5.0
	.asciz	"\025\000\000\000\000\000\000\000*\000\000\000\021\000\000"
	.size	.Lanon.1caba9cb7805f6637afb478735c2d9d5.4, 24

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
