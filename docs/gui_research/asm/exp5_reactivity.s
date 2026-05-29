	.intel_syntax noprefix
	.file	"exp5_reactivity.6972f85b0925729c-cgu.0"
	.section	.text.epoch_changed,"ax",@progbits
	.globl	epoch_changed
	.p2align	4
	.type	epoch_changed,@function
epoch_changed:
	.cfi_startproc
	cmp	qword ptr [rdi], rsi
	setne	al
	ret
.Lfunc_end0:
	.size	epoch_changed, .Lfunc_end0-epoch_changed
	.cfi_endproc

	.section	.text.epoch_read,"ax",@progbits
	.globl	epoch_read
	.p2align	4
	.type	epoch_read,@function
epoch_read:
	.cfi_startproc
	vmovss	xmm0, dword ptr [rdi + 8]
	ret
.Lfunc_end1:
	.size	epoch_read, .Lfunc_end1-epoch_read
	.cfi_endproc

	.section	.text.scan_dirty_epoch,"ax",@progbits
	.globl	scan_dirty_epoch
	.p2align	4
	.type	scan_dirty_epoch,@function
scan_dirty_epoch:
	.cfi_startproc
	cmp	rcx, rsi
	cmovb	rsi, rcx
	test	rsi, rsi
	je	.LBB2_1
	cmp	rsi, 4
	jae	.LBB2_5
	xor	eax, eax
	xor	ecx, ecx
	jmp	.LBB2_4
.LBB2_1:
	xor	eax, eax
	ret
.LBB2_5:
	movabs	r8, 1152921504606846960
	cmp	rsi, 16
	jae	.LBB2_7
	xor	ecx, ecx
	xor	eax, eax
	jmp	.LBB2_11
.LBB2_7:
	mov	rcx, rsi
	and	rcx, r8
	lea	rax, [8*rsi]
	and	rax, -128
	vpxor	xmm0, xmm0, xmm0
	xor	r9d, r9d
	vpcmpeqd	ymm1, ymm1, ymm1
	vpxor	xmm2, xmm2, xmm2
	vpxor	xmm3, xmm3, xmm3
	vpxor	xmm4, xmm4, xmm4
	.p2align	4
.LBB2_8:
	vmovdqu	ymm5, ymmword ptr [rdi + r9]
	vmovdqu	ymm6, ymmword ptr [rdi + r9 + 32]
	vmovdqu	ymm7, ymmword ptr [rdi + r9 + 64]
	vmovdqu	ymm8, ymmword ptr [rdi + r9 + 96]
	vpcmpeqq	ymm5, ymm5, ymmword ptr [rdx + r9]
	vpxor	ymm5, ymm5, ymm1
	vextracti128	xmm9, ymm5, 1
	vpackssdw	xmm5, xmm5, xmm9
	vpsubd	xmm0, xmm0, xmm5
	vpcmpeqq	ymm5, ymm6, ymmword ptr [rdx + r9 + 32]
	vpxor	ymm5, ymm5, ymm1
	vextracti128	xmm6, ymm5, 1
	vpackssdw	xmm5, xmm5, xmm6
	vpsubd	xmm2, xmm2, xmm5
	vpcmpeqq	ymm5, ymm7, ymmword ptr [rdx + r9 + 64]
	vpxor	ymm5, ymm5, ymm1
	vextracti128	xmm6, ymm5, 1
	vpackssdw	xmm5, xmm5, xmm6
	vpsubd	xmm3, xmm3, xmm5
	vpcmpeqq	ymm5, ymm8, ymmword ptr [rdx + r9 + 96]
	vpxor	ymm5, ymm5, ymm1
	vextracti128	xmm6, ymm5, 1
	vpackssdw	xmm5, xmm5, xmm6
	vpsubd	xmm4, xmm4, xmm5
	sub	r9, -128
	cmp	rax, r9
	jne	.LBB2_8
	vpaddd	xmm0, xmm2, xmm0
	vpaddd	xmm1, xmm4, xmm3
	vpaddd	xmm0, xmm1, xmm0
	vpshufd	xmm1, xmm0, 238
	vpaddd	xmm0, xmm0, xmm1
	vpshufd	xmm1, xmm0, 85
	vpaddd	xmm0, xmm0, xmm1
	vmovd	eax, xmm0
	cmp	rsi, rcx
	je	.LBB2_15
	test	sil, 12
	je	.LBB2_4
.LBB2_11:
	mov	r9, rcx
	add	r8, 12
	mov	rcx, r8
	and	rcx, rsi
	vmovd	xmm0, eax
	vpcmpeqd	ymm1, ymm1, ymm1
	.p2align	4
.LBB2_12:
	vmovdqu	ymm2, ymmword ptr [rdi + 8*r9]
	vpcmpeqq	ymm2, ymm2, ymmword ptr [rdx + 8*r9]
	vpxor	ymm2, ymm2, ymm1
	vextracti128	xmm3, ymm2, 1
	vpackssdw	xmm2, xmm2, xmm3
	vpsubd	xmm0, xmm0, xmm2
	add	r9, 4
	cmp	rcx, r9
	jne	.LBB2_12
	vpshufd	xmm1, xmm0, 238
	vpaddd	xmm0, xmm0, xmm1
	vpshufd	xmm1, xmm0, 85
	vpaddd	xmm0, xmm0, xmm1
	vmovd	eax, xmm0
	jmp	.LBB2_14
.LBB2_4:
	mov	r8, qword ptr [rdi + 8*rcx]
	xor	r9d, r9d
	cmp	r8, qword ptr [rdx + 8*rcx]
	lea	rcx, [rcx + 1]
	setne	r9b
	add	eax, r9d
.LBB2_14:
	cmp	rsi, rcx
	jne	.LBB2_4
.LBB2_15:
	vzeroupper
	ret
.Lfunc_end2:
	.size	scan_dirty_epoch, .Lfunc_end2-scan_dirty_epoch
	.cfi_endproc

	.section	.text.signal_get_f32,"ax",@progbits
	.globl	signal_get_f32
	.p2align	4
	.type	signal_get_f32,@function
signal_get_f32:
	.cfi_startproc
	mov	rax, qword ptr [rdi]
	movabs	rcx, 9223372036854775807
	cmp	qword ptr [rax + 16], rcx
	jae	.LBB3_2
	vmovss	xmm0, dword ptr [rax + 24]
	ret
.LBB3_2:
	push	rax
	.cfi_def_cfa_offset 16
	lea	rdi, [rip + .Lanon.3611e29733d1d84b2d312d389361f904.1]
	call	qword ptr [rip + _ZN4core4cell30panic_already_mutably_borrowed17ha7d6289770adc8acE@GOTPCREL]
.Lfunc_end3:
	.size	signal_get_f32, .Lfunc_end3-signal_get_f32
	.cfi_endproc

	.type	.Lanon.3611e29733d1d84b2d312d389361f904.0,@object
	.section	.rodata.str1.1,"aMS",@progbits,1
.Lanon.3611e29733d1d84b2d312d389361f904.0:
	.asciz	"exp5_reactivity.rs"
	.size	.Lanon.3611e29733d1d84b2d312d389361f904.0, 19

	.type	.Lanon.3611e29733d1d84b2d312d389361f904.1,@object
	.section	.data.rel.ro..Lanon.3611e29733d1d84b2d312d389361f904.1,"aw",@progbits
	.p2align	3, 0x0
.Lanon.3611e29733d1d84b2d312d389361f904.1:
	.quad	.Lanon.3611e29733d1d84b2d312d389361f904.0
	.asciz	"\022\000\000\000\000\000\000\000\020\000\000\000\r\000\000"
	.size	.Lanon.3611e29733d1d84b2d312d389361f904.1, 24

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
