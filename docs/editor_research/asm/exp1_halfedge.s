	.intel_syntax noprefix
	.file	"exp1_halfedge.e01208e8165e4b52-cgu.0"
	.section	.text.one_ring_sum_ptr,"ax",@progbits
	.globl	one_ring_sum_ptr
	.p2align	4
	.type	one_ring_sum_ptr,@function
one_ring_sum_ptr:
	.cfi_startproc
	xor	eax, eax
	test	rdi, rdi
	je	.LBB0_4
	mov	rcx, rdi
	.p2align	4
.LBB0_2:
	mov	rdx, rax
	mov	rcx, qword ptr [rcx + 8]
	mov	eax, dword ptr [rcx + 16]
	add	rax, rdx
	mov	rcx, qword ptr [rcx]
	cmp	rcx, rdi
	je	.LBB0_4
	test	rcx, rcx
	jne	.LBB0_2
.LBB0_4:
	ret
.Lfunc_end0:
	.size	one_ring_sum_ptr, .Lfunc_end0-one_ring_sum_ptr
	.cfi_endproc

	.section	.text.one_ring_sum_soa,"ax",@progbits
	.globl	one_ring_sum_soa
	.p2align	4
	.type	one_ring_sum_soa,@function
one_ring_sum_soa:
	.cfi_startproc
	push	rbx
	.cfi_def_cfa_offset 16
	.cfi_offset rbx, -16
	mov	eax, esi
	mov	rsi, qword ptr [rdi + 88]
	cmp	rsi, rax
	jbe	.LBB1_3
	mov	rcx, qword ptr [rdi + 80]
	mov	edx, dword ptr [rcx + 4*rax]
	cmp	edx, -1
	je	.LBB1_2
	mov	rsi, qword ptr [rdi + 40]
	mov	r9, qword ptr [rdi + 32]
	mov	rcx, qword ptr [rdi + 64]
	mov	r10, qword ptr [rdi + 8]
	mov	r8, qword ptr [rdi + 16]
	mov	r11, qword ptr [rdi + 56]
	xor	eax, eax
	mov	edi, edx
	.p2align	4
.LBB1_5:
	mov	edi, edi
	cmp	rsi, rdi
	jbe	.LBB1_11
	mov	edi, dword ptr [r9 + 4*rdi]
	cmp	rcx, rdi
	jbe	.LBB1_12
	cmp	r8, rdi
	jbe	.LBB1_13
	mov	ebx, dword ptr [r11 + 4*rdi]
	add	rax, rbx
	mov	edi, dword ptr [r10 + 4*rdi]
	cmp	edi, edx
	je	.LBB1_10
	cmp	edi, -1
	jne	.LBB1_5
.LBB1_10:
	pop	rbx
	.cfi_def_cfa_offset 8
	ret
.LBB1_2:
	.cfi_def_cfa_offset 16
	xor	eax, eax
	pop	rbx
	.cfi_def_cfa_offset 8
	ret
.LBB1_13:
	.cfi_def_cfa_offset 16
	lea	rdx, [rip + .Lanon.bf482ab5970ef14f01b2519f5075e1c2.4]
	mov	rsi, r8
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_12:
	lea	rdx, [rip + .Lanon.bf482ab5970ef14f01b2519f5075e1c2.3]
	mov	rsi, rcx
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_11:
	lea	rdx, [rip + .Lanon.bf482ab5970ef14f01b2519f5075e1c2.2]
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.LBB1_3:
	lea	rdx, [rip + .Lanon.bf482ab5970ef14f01b2519f5075e1c2.1]
	mov	rdi, rax
	call	qword ptr [rip + _ZN4core9panicking18panic_bounds_check17h9ae613628793029fE@GOTPCREL]
.Lfunc_end1:
	.size	one_ring_sum_soa, .Lfunc_end1-one_ring_sum_soa
	.cfi_endproc

	.type	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.0,@object
	.section	.rodata.str1.1,"aMS",@progbits,1
.Lanon.bf482ab5970ef14f01b2519f5075e1c2.0:
	.asciz	"exp1_halfedge.rs"
	.size	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.0, 17

	.type	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.1,@object
	.section	.data.rel.ro..Lanon.bf482ab5970ef14f01b2519f5075e1c2.1,"aw",@progbits
	.p2align	3, 0x0
.Lanon.bf482ab5970ef14f01b2519f5075e1c2.1:
	.quad	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.0
	.asciz	"\020\000\000\000\000\000\000\000\034\000\000\000\032\000\000"
	.size	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.1, 24

	.type	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.2,@object
	.section	.data.rel.ro..Lanon.bf482ab5970ef14f01b2519f5075e1c2.2,"aw",@progbits
	.p2align	3, 0x0
.Lanon.bf482ab5970ef14f01b2519f5075e1c2.2:
	.quad	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.0
	.asciz	"\020\000\000\000\000\000\000\000$\000\000\000\027\000\000"
	.size	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.2, 24

	.type	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.3,@object
	.section	.data.rel.ro..Lanon.bf482ab5970ef14f01b2519f5075e1c2.3,"aw",@progbits
	.p2align	3, 0x0
.Lanon.bf482ab5970ef14f01b2519f5075e1c2.3:
	.quad	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.0
	.asciz	"\020\000\000\000\000\000\000\000%\000\000\000\026\000\000"
	.size	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.3, 24

	.type	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.4,@object
	.section	.data.rel.ro..Lanon.bf482ab5970ef14f01b2519f5075e1c2.4,"aw",@progbits
	.p2align	3, 0x0
.Lanon.bf482ab5970ef14f01b2519f5075e1c2.4:
	.quad	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.0
	.asciz	"\020\000\000\000\000\000\000\000'\000\000\000\024\000\000"
	.size	.Lanon.bf482ab5970ef14f01b2519f5075e1c2.4, 24

	.ident	"rustc version 1.94.1 (e408947bf 2026-03-25)"
	.section	".note.GNU-stack","",@progbits
