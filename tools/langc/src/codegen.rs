//! HIR → LLVM IR via inkwell.
//!
//! Emits one LLVM module per `.lang` script with the following exported symbols:
//!
//! * `df_state_size      () -> u32`
//! * `df_state_version   () -> u32`
//! * `df_init_state      (state: *mut u8)`
//! * `df_migrate_state   (old_version: u32, old: *const u8, new: *mut u8)`
//! * `df_create_scene_defs (api: *const EngineAPI, out: *mut SceneDefArray)`
//!
//! All per-scene `on_enter`/`on_exit`/`tick` functions are emitted with
//! `internal` linkage; only the descriptor table and the `df_*` entry points
//! are visible to the dynamic linker.
//!
//! Optimization pipeline runs at LLVM's standard `OptimizationLevel::Aggressive`
//! (equivalent to `-O3`).  LTO is unnecessary at this stage because each `.lang`
//! file compiles to a single module — every function is statically known to
//! the optimizer.

use thin_vec::ThinVec;

use inkwell::AddressSpace;
use inkwell::OptimizationLevel;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, StructType, FunctionType};
use inkwell::values::{
    BasicValueEnum, FunctionValue, IntValue, FloatValue, PointerValue,
};
use inkwell::IntPredicate;
use inkwell::FloatPredicate;

use lang_frontend::ast::{BinOp, CmpOp, Ty};
use lang_frontend::hir::{
    HirAssign, HirBtNode, HirCondition, HirEffect, HirExpr,
    HirScene, HirScript, IntrinsicEffect, IntrinsicPredicate,
    IntrinsicValue,
};

use crate::engine_api::*;

pub fn compile_to_object(
    hir: &HirScript,
    opt_level: OptimizationLevel,
    out_obj: &std::path::Path,
) -> Result<(), CodegenError> {
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| CodegenError::Llvm(std::sync::Arc::<str>::from(format!("target init: {e}").as_str())))?;

    let triple   = TargetMachine::get_default_triple();
    let cpu      = TargetMachine::get_host_cpu_name().to_string();
    let features = TargetMachine::get_host_cpu_features().to_string();

    let target = Target::from_triple(&triple)
        .map_err(|e| CodegenError::Llvm(std::sync::Arc::<str>::from(format!("target lookup: {e}").as_str())))?;
    let tm = target.create_target_machine(
        &triple, &cpu, &features,
        opt_level, RelocMode::PIC, CodeModel::Default,
    ).ok_or_else(|| CodegenError::Llvm("no target machine".into()))?;

    let ctx = Context::create();
    let mut cg = Codegen::new(&ctx, &hir.name);
    cg.module.set_triple(&triple);
    cg.module.set_data_layout(&tm.get_target_data().get_data_layout());

    cg.compile(hir)?;

    let passes = match opt_level {
        OptimizationLevel::None        => "default<O0>",
        OptimizationLevel::Less        => "default<O1>",
        OptimizationLevel::Default     => "default<O2>",
        OptimizationLevel::Aggressive  => "default<O3>",
    };
    cg.module.run_passes(passes, &tm, PassBuilderOptions::create())
        .map_err(|e| CodegenError::Llvm(std::sync::Arc::<str>::from(format!("opt: {e}").as_str())))?;

    cg.module.verify()
        .map_err(|e| CodegenError::Llvm(std::sync::Arc::<str>::from(format!("verify: {e}").as_str())))?;

    tm.write_to_file(&cg.module, FileType::Object, out_obj)
        .map_err(|e| CodegenError::Llvm(std::sync::Arc::<str>::from(format!("emit obj: {e}").as_str())))?;
    Ok(())
}

// ── Codegen context ──────────────────────────────────────────────────────────

struct Codegen<'ctx> {
    ctx: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,

    void_ty:      inkwell::types::VoidType<'ctx>,
    i1_ty:        inkwell::types::IntType<'ctx>,
    i8_ty:        inkwell::types::IntType<'ctx>,
    i32_ty:       inkwell::types::IntType<'ctx>,
    i64_ty:       inkwell::types::IntType<'ctx>,
    f64_ty:       inkwell::types::FloatType<'ctx>,
    f32_ty:       inkwell::types::FloatType<'ctx>,
    ptr_ty:       inkwell::types::PointerType<'ctx>,

    fn_void_api_state:  FunctionType<'ctx>,
    fn_i64_api_state:   FunctionType<'ctx>,
    #[allow(dead_code)]
    fn_bool_api_state:  FunctionType<'ctx>,

    effect_abi_ty:      StructType<'ctx>,
    scene_entry_ty:     StructType<'ctx>,
    #[allow(dead_code)]
    scene_def_array_ty: StructType<'ctx>,
}

impl<'ctx> Codegen<'ctx> {
    fn new(ctx: &'ctx Context, name: &str) -> Self {
        let module  = ctx.create_module(&format!("dfe.{name}"));
        let builder = ctx.create_builder();

        let void_ty = ctx.void_type();
        let i1_ty   = ctx.bool_type();
        let i8_ty   = ctx.i8_type();
        let i32_ty  = ctx.i32_type();
        let i64_ty  = ctx.i64_type();
        let f32_ty  = ctx.f32_type();
        let f64_ty  = ctx.f64_type();
        let ptr_ty  = ctx.ptr_type(AddressSpace::default());

        let api_arg: BasicMetadataTypeEnum = ptr_ty.into();
        let state_arg: BasicMetadataTypeEnum = ptr_ty.into();
        let fn_void_api_state = void_ty.fn_type(&[api_arg, state_arg], false);
        let fn_i64_api_state  = i64_ty.fn_type(&[api_arg, state_arg], false);
        let fn_bool_api_state = i1_ty.fn_type(&[api_arg, state_arg], false);

        // EffectAbi { kind u8, _pad [u8;7], arg0 i64, arg1 i64 }
        let effect_abi_ty = ctx.struct_type(&[
            i8_ty.into(),
            i8_ty.array_type(7).into(),
            i64_ty.into(),
            i64_ty.into(),
        ], false);

        // SceneEntry { raw_id i64, on_enter ptr, on_exit ptr, tick ptr }
        let scene_entry_ty = ctx.struct_type(&[
            i64_ty.into(),
            ptr_ty.into(),
            ptr_ty.into(),
            ptr_ty.into(),
        ], false);

        // SceneDefArray { scene_count u32, _pad u32, scenes ptr }
        let scene_def_array_ty = ctx.struct_type(&[
            i32_ty.into(),
            i32_ty.into(),
            ptr_ty.into(),
        ], false);

        Codegen {
            ctx, module, builder,
            void_ty, i1_ty, i8_ty, i32_ty, i64_ty, f64_ty, f32_ty, ptr_ty,
            fn_void_api_state, fn_i64_api_state, fn_bool_api_state,
            effect_abi_ty, scene_entry_ty, scene_def_array_ty,
        }
    }

    fn compile(&mut self, hir: &HirScript) -> Result<(), CodegenError> {
        self.emit_state_size_fn(hir.state_size);
        self.emit_state_version_fn(hir.state_version);
        self.emit_init_state_fn(hir)?;
        self.emit_migrate_state_fn(hir)?;

        // Per-scene functions.
        let mut entries: ThinVec<(i64, FunctionValue<'ctx>, FunctionValue<'ctx>, FunctionValue<'ctx>)>
            = ThinVec::new();
        for (idx, scene) in hir.scenes.iter().enumerate() {
            let on_enter = self.emit_on_enter_fn(idx, scene, hir)?;
            let on_exit  = self.emit_on_exit_fn(idx, scene, hir)?;
            let tick     = self.emit_tick_fn(idx, scene, hir)?;
            entries.push((scene.raw_id, on_enter, on_exit, tick));
        }

        self.emit_scene_def_table(&entries);
        self.emit_create_scene_defs_fn(entries.len() as u32);

        Ok(())
    }

    // ── df_state_size / df_state_version ─────────────────────────────────────

    fn emit_state_size_fn(&self, state_size: u32) {
        let ty = self.i32_ty.fn_type(&[], false);
        let f = self.module.add_function("df_state_size", ty, Some(Linkage::External));
        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        let v = self.i32_ty.const_int(state_size as u64, false);
        self.builder.build_return(Some(&v)).unwrap();
    }

    fn emit_state_version_fn(&self, state_version: u32) {
        let ty = self.i32_ty.fn_type(&[], false);
        let f = self.module.add_function("df_state_version", ty, Some(Linkage::External));
        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        let v = self.i32_ty.const_int(state_version as u64, false);
        self.builder.build_return(Some(&v)).unwrap();
    }

    // ── df_init_state(state) ─────────────────────────────────────────────────

    fn emit_init_state_fn(&self, hir: &HirScript) -> Result<(), CodegenError> {
        let ty = self.void_ty.fn_type(&[self.ptr_ty.into()], false);
        let f = self.module.add_function("df_init_state", ty, Some(Linkage::External));
        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        let state_ptr = f.get_nth_param(0).unwrap().into_pointer_value();

        // Zero the whole buffer using llvm.memset.p0.i64.
        let memset = self.module.get_function("llvm.memset.p0.i64")
            .unwrap_or_else(|| {
                let memset_ty = self.void_ty.fn_type(&[
                    self.ptr_ty.into(),
                    self.i8_ty.into(),
                    self.i64_ty.into(),
                    self.i1_ty.into(),
                ], false);
                self.module.add_function("llvm.memset.p0.i64", memset_ty, None)
            });
        let size = self.i64_ty.const_int(hir.state_size as u64, false);
        let zero = self.i8_ty.const_zero();
        let is_vol = self.i1_ty.const_zero();
        self.builder.build_call(memset, &[
            state_ptr.into(), zero.into(), size.into(), is_vol.into(),
        ], "").unwrap();

        // Apply each field default (if any).
        for field in hir.fields.iter() {
            if let Some(default) = field.default.as_ref() {
                let val = self.emit_expr(default, state_ptr, None, None, hir)?;
                self.store_field(state_ptr, field.offset, field.ty, val);
            }
        }

        self.builder.build_return(None).unwrap();
        Ok(())
    }

    // ── df_migrate_state(old_version, old, new) ──────────────────────────────

    fn emit_migrate_state_fn(&self, hir: &HirScript) -> Result<(), CodegenError> {
        let ty = self.void_ty.fn_type(&[
            self.i32_ty.into(),
            self.ptr_ty.into(),
            self.ptr_ty.into(),
        ], false);
        let f = self.module.add_function("df_migrate_state", ty, Some(Linkage::External));
        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        let old_version = f.get_nth_param(0).unwrap().into_int_value();
        let old_state   = f.get_nth_param(1).unwrap().into_pointer_value();
        let new_state   = f.get_nth_param(2).unwrap().into_pointer_value();

        // Always start by initialising new_state with defaults so any field the
        // user didn't migrate has a well-defined value.
        let init = self.module.get_function("df_init_state").unwrap();
        self.builder.build_call(init, &[new_state.into()], "").unwrap();

        // Build a switch over `old_version`.  Default block: leave defaults.
        let done_bb = self.ctx.append_basic_block(f, "done");

        if hir.migrations.is_empty() {
            self.builder.build_unconditional_branch(done_bb).unwrap();
        } else {
            let mut cases: ThinVec<(IntValue<'ctx>, BasicBlock<'ctx>)> = ThinVec::new();
            for m in hir.migrations.iter() {
                let bb = self.ctx.append_basic_block(f, &format!("mig_{}", m.from_version));
                cases.push((self.i32_ty.const_int(m.from_version as u64, false), bb));
            }
            // build_switch wants a `&[(IntValue, BasicBlock)]`.  We rebuild into
            // a `ThinVec` so the slice it expects can come from `.as_slice()`
            // without going through `std::Vec`.
            let mut case_pairs: ThinVec<(IntValue<'ctx>, BasicBlock<'ctx>)> =
                ThinVec::with_capacity(cases.len());
            for (v, b) in cases.iter() { case_pairs.push((*v, *b)); }
            self.builder.build_switch(old_version, done_bb, &case_pairs[..]).unwrap();

            for (m, (_, bb)) in hir.migrations.iter().zip(cases.iter()) {
                self.builder.position_at_end(*bb);
                for stmt in m.stmts.iter() {
                    self.emit_migrate_stmt(stmt, old_state, new_state, hir)?;
                }
                self.builder.build_unconditional_branch(done_bb).unwrap();
            }
        }

        self.builder.position_at_end(done_bb);
        self.builder.build_return(None).unwrap();
        Ok(())
    }

    fn emit_migrate_stmt(
        &self,
        stmt: &HirAssign,
        old_state: PointerValue<'ctx>,
        new_state: PointerValue<'ctx>,
        hir: &HirScript,
    ) -> Result<(), CodegenError> {
        let v = self.emit_expr(&stmt.value, new_state, Some(old_state), None, hir)?;
        self.store_field(new_state, stmt.new_offset, stmt.ty, v);
        Ok(())
    }

    // ── Per-scene functions ──────────────────────────────────────────────────

    fn emit_on_enter_fn(
        &self,
        idx: usize,
        scene: &HirScene,
        hir: &HirScript,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        let name = format!("scene_{idx}_on_enter");
        let f = self.module.add_function(&name, self.fn_void_api_state, Some(Linkage::Internal));
        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        let api   = f.get_nth_param(0).unwrap().into_pointer_value();
        let state = f.get_nth_param(1).unwrap().into_pointer_value();
        for e in scene.on_enter.iter() {
            self.emit_effect(e, api, state, hir)?;
        }
        self.builder.build_return(None).unwrap();
        Ok(f)
    }

    fn emit_on_exit_fn(
        &self,
        idx: usize,
        scene: &HirScene,
        hir: &HirScript,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        let name = format!("scene_{idx}_on_exit");
        let f = self.module.add_function(&name, self.fn_void_api_state, Some(Linkage::Internal));
        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        let api   = f.get_nth_param(0).unwrap().into_pointer_value();
        let state = f.get_nth_param(1).unwrap().into_pointer_value();
        for e in scene.on_exit.iter() {
            self.emit_effect(e, api, state, hir)?;
        }
        self.builder.build_return(None).unwrap();
        Ok(f)
    }

    /// The tick function:
    /// 1. evaluate each transition condition; on first true, return target's raw_id
    /// 2. otherwise run the behaviour tree (collecting effects via api)
    /// 3. return 0 to indicate "no transition"
    fn emit_tick_fn(
        &self,
        idx: usize,
        scene: &HirScene,
        hir: &HirScript,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        let name = format!("scene_{idx}_tick");
        let f = self.module.add_function(&name, self.fn_i64_api_state, Some(Linkage::Internal));
        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        let api   = f.get_nth_param(0).unwrap().into_pointer_value();
        let state = f.get_nth_param(1).unwrap().into_pointer_value();

        let bt_bb     = self.ctx.append_basic_block(f, "behavior");
        let return_bb = self.ctx.append_basic_block(f, "return_none");

        // Evaluate transitions in order.
        for (tidx, tr) in scene.transitions.iter().enumerate() {
            let cond = self.emit_condition(&tr.condition, api, state, hir)?;
            let take_bb = self.ctx.append_basic_block(f, &format!("take_t{tidx}"));
            let next_bb = self.ctx.append_basic_block(f, &format!("after_t{tidx}"));
            self.builder.build_conditional_branch(cond, take_bb, next_bb).unwrap();

            self.builder.position_at_end(take_bb);
            let target = self.i64_ty.const_int(tr.target_raw_id as u64, true);
            self.builder.build_return(Some(&target)).unwrap();

            self.builder.position_at_end(next_bb);
        }
        // After last transition's "next" block, jump into behaviour-tree eval.
        self.builder.build_unconditional_branch(bt_bb).unwrap();

        // Behavior tree.
        self.builder.position_at_end(bt_bb);
        if let Some(bt) = scene.behavior.as_ref() {
            let _status = self.emit_bt(bt, api, state, hir, f)?;
        }
        self.builder.build_unconditional_branch(return_bb).unwrap();

        // Default return = 0 (stay in this scene).
        self.builder.position_at_end(return_bb);
        let zero = self.i64_ty.const_zero();
        self.builder.build_return(Some(&zero)).unwrap();
        Ok(f)
    }

    // ── Behavior tree ────────────────────────────────────────────────────────

    /// Returns the IntValue<i32> BtStatus of this subtree.  Inserts blocks into
    /// the current function.  Caller is responsible for branching off the
    /// status if needed.
    fn emit_bt(
        &self,
        node: &HirBtNode,
        api: PointerValue<'ctx>,
        state: PointerValue<'ctx>,
        hir: &HirScript,
        f: FunctionValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        match node {
            HirBtNode::Leaf { condition, action } => {
                // status = Failure by default
                let status = self.builder.build_alloca(self.i32_ty, "leaf_status").unwrap();
                self.builder.build_store(status, self.i32_ty.const_int(BT_FAILURE as u64, true)).unwrap();

                let do_action_bb = self.ctx.append_basic_block(f, "leaf_action");
                let after_bb     = self.ctx.append_basic_block(f, "leaf_after");

                let cond_val = if let Some(c) = condition {
                    self.emit_condition(c, api, state, hir)?
                } else {
                    self.i1_ty.const_int(1, false)
                };
                self.builder.build_conditional_branch(cond_val, do_action_bb, after_bb).unwrap();

                self.builder.position_at_end(do_action_bb);
                if let Some(a) = action {
                    self.emit_effect(a, api, state, hir)?;
                }
                self.builder.build_store(status, self.i32_ty.const_int(BT_SUCCESS as u64, true)).unwrap();
                self.builder.build_unconditional_branch(after_bb).unwrap();

                self.builder.position_at_end(after_bb);
                let v = self.builder.build_load(self.i32_ty, status, "leaf_status_v").unwrap();
                Ok(v.into_int_value())
            }
            HirBtNode::Sequence(children) => {
                // Sequence: every child must succeed.  On Running or Failure, short-circuit.
                let result = self.builder.build_alloca(self.i32_ty, "seq_result").unwrap();
                self.builder.build_store(result, self.i32_ty.const_int(BT_SUCCESS as u64, true)).unwrap();
                let exit_bb = self.ctx.append_basic_block(f, "seq_exit");

                for (i, c) in children.iter().enumerate() {
                    let s = self.emit_bt(c, api, state, hir, f)?;
                    // store-and-test
                    self.builder.build_store(result, s).unwrap();
                    let is_success = self.builder.build_int_compare(
                        IntPredicate::EQ, s, self.i32_ty.const_int(BT_SUCCESS as u64, true), "is_succ"
                    ).unwrap();
                    let cont_bb = self.ctx.append_basic_block(f, &format!("seq_cont_{i}"));
                    self.builder.build_conditional_branch(is_success, cont_bb, exit_bb).unwrap();
                    self.builder.position_at_end(cont_bb);
                }
                self.builder.build_unconditional_branch(exit_bb).unwrap();

                self.builder.position_at_end(exit_bb);
                let v = self.builder.build_load(self.i32_ty, result, "seq_v").unwrap();
                Ok(v.into_int_value())
            }
            HirBtNode::Selector(children) => {
                // Selector: every child must fail.  On Running or Success, short-circuit.
                let result = self.builder.build_alloca(self.i32_ty, "sel_result").unwrap();
                self.builder.build_store(result, self.i32_ty.const_int(BT_FAILURE as u64, true)).unwrap();
                let exit_bb = self.ctx.append_basic_block(f, "sel_exit");

                for (i, c) in children.iter().enumerate() {
                    let s = self.emit_bt(c, api, state, hir, f)?;
                    self.builder.build_store(result, s).unwrap();
                    let is_failure = self.builder.build_int_compare(
                        IntPredicate::EQ, s, self.i32_ty.const_int(BT_FAILURE as u64, true), "is_fail"
                    ).unwrap();
                    let cont_bb = self.ctx.append_basic_block(f, &format!("sel_cont_{i}"));
                    self.builder.build_conditional_branch(is_failure, cont_bb, exit_bb).unwrap();
                    self.builder.position_at_end(cont_bb);
                }
                self.builder.build_unconditional_branch(exit_bb).unwrap();

                self.builder.position_at_end(exit_bb);
                let v = self.builder.build_load(self.i32_ty, result, "sel_v").unwrap();
                Ok(v.into_int_value())
            }
            HirBtNode::Parallel(children) => {
                // AllSucceed policy:
                //   if any child returns Failure → Failure (short-circuit)
                //   if all return Success → Success
                //   else Running
                let success_count = self.builder.build_alloca(self.i32_ty, "par_succ").unwrap();
                let failure_count = self.builder.build_alloca(self.i32_ty, "par_fail").unwrap();
                self.builder.build_store(success_count, self.i32_ty.const_zero()).unwrap();
                self.builder.build_store(failure_count, self.i32_ty.const_zero()).unwrap();

                for c in children.iter() {
                    let s = self.emit_bt(c, api, state, hir, f)?;
                    // success ++ if s == 1
                    let is_succ = self.builder.build_int_compare(
                        IntPredicate::EQ, s,
                        self.i32_ty.const_int(BT_SUCCESS as u64, true), "is_s"
                    ).unwrap();
                    let cur_s = self.builder.build_load(self.i32_ty, success_count, "cur_s")
                        .unwrap().into_int_value();
                    let inc_s = self.builder.build_int_add(
                        cur_s, self.i32_ty.const_int(1, false), "inc_s"
                    ).unwrap();
                    let next_s = self.builder.build_select(
                        is_succ, inc_s, cur_s, "next_s"
                    ).unwrap();
                    self.builder.build_store(success_count, next_s).unwrap();
                    // failure ++ if s == 2
                    let is_fail = self.builder.build_int_compare(
                        IntPredicate::EQ, s,
                        self.i32_ty.const_int(BT_FAILURE as u64, true), "is_f"
                    ).unwrap();
                    let cur_f = self.builder.build_load(self.i32_ty, failure_count, "cur_f")
                        .unwrap().into_int_value();
                    let inc_f = self.builder.build_int_add(
                        cur_f, self.i32_ty.const_int(1, false), "inc_f"
                    ).unwrap();
                    let next_f = self.builder.build_select(
                        is_fail, inc_f, cur_f, "next_f"
                    ).unwrap();
                    self.builder.build_store(failure_count, next_f).unwrap();
                }

                let total = self.i32_ty.const_int(children.len() as u64, false);
                let s_val = self.builder.build_load(self.i32_ty, success_count, "succ_v")
                    .unwrap().into_int_value();
                let f_val = self.builder.build_load(self.i32_ty, failure_count, "fail_v")
                    .unwrap().into_int_value();

                let all_succ = self.builder.build_int_compare(
                    IntPredicate::EQ, s_val, total, "all_succ"
                ).unwrap();
                let any_fail = self.builder.build_int_compare(
                    IntPredicate::UGT, f_val, self.i32_ty.const_zero(), "any_fail"
                ).unwrap();

                // result = any_fail ? FAILURE : (all_succ ? SUCCESS : RUNNING)
                let succ_or_run = self.builder.build_select(
                    all_succ,
                    self.i32_ty.const_int(BT_SUCCESS as u64, true),
                    self.i32_ty.const_int(BT_RUNNING as u64, true),
                    "succ_or_run",
                ).unwrap().into_int_value();
                let result = self.builder.build_select(
                    any_fail,
                    self.i32_ty.const_int(BT_FAILURE as u64, true),
                    succ_or_run,
                    "par_result",
                ).unwrap();
                Ok(result.into_int_value())
            }
            HirBtNode::Repeat { count, child } => {
                // Iterate up to `count` times (0 → infinite, capped at one iter for safety).
                let iter_count = (*count).max(1);
                let mut last = self.i32_ty.const_int(BT_SUCCESS as u64, true);
                for _ in 0..iter_count {
                    last = self.emit_bt(child, api, state, hir, f)?;
                }
                Ok(last)
            }
            HirBtNode::Inverter { child } => {
                let s = self.emit_bt(child, api, state, hir, f)?;
                // Invert: SUCCESS↔FAILURE, RUNNING unchanged.
                let is_succ = self.builder.build_int_compare(
                    IntPredicate::EQ, s, self.i32_ty.const_int(BT_SUCCESS as u64, true), "inv_succ"
                ).unwrap();
                let is_fail = self.builder.build_int_compare(
                    IntPredicate::EQ, s, self.i32_ty.const_int(BT_FAILURE as u64, true), "inv_fail"
                ).unwrap();
                // inv = is_succ ? FAILURE : (is_fail ? SUCCESS : s)
                let if_fail = self.builder.build_select(
                    is_fail, self.i32_ty.const_int(BT_SUCCESS as u64, true), s, "if_fail"
                ).unwrap().into_int_value();
                let r = self.builder.build_select(
                    is_succ,
                    self.i32_ty.const_int(BT_FAILURE as u64, true),
                    if_fail,
                    "inv_r",
                ).unwrap();
                Ok(r.into_int_value())
            }
            HirBtNode::Guard { cond, child } => {
                let result = self.builder.build_alloca(self.i32_ty, "guard_r").unwrap();
                self.builder.build_store(result, self.i32_ty.const_int(BT_SUCCESS as u64, true)).unwrap();
                let do_bb   = self.ctx.append_basic_block(f, "guard_run");
                let exit_bb = self.ctx.append_basic_block(f, "guard_exit");
                let c = self.emit_condition(cond, api, state, hir)?;
                self.builder.build_conditional_branch(c, do_bb, exit_bb).unwrap();

                self.builder.position_at_end(do_bb);
                let s = self.emit_bt(child, api, state, hir, f)?;
                self.builder.build_store(result, s).unwrap();
                self.builder.build_unconditional_branch(exit_bb).unwrap();

                self.builder.position_at_end(exit_bb);
                let v = self.builder.build_load(self.i32_ty, result, "guard_v")
                    .unwrap().into_int_value();
                Ok(v)
            }
            HirBtNode::Cooldown { duration: _, child } => {
                // Stateful cooldown needs scene-local state; v1 simply forwards the child.
                self.emit_bt(child, api, state, hir, f)
            }
        }
    }

    // ── Conditions ────────────────────────────────────────────────────────────

    fn emit_condition(
        &self,
        c: &HirCondition,
        api: PointerValue<'ctx>,
        state: PointerValue<'ctx>,
        hir: &HirScript,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        Ok(match c {
            HirCondition::Bool(b) => self.i1_ty.const_int(*b as u64, false),
            HirCondition::Not(a) => {
                let v = self.emit_condition(a, api, state, hir)?;
                self.builder.build_not(v, "not").unwrap()
            }
            HirCondition::And(a, b) => {
                let va = self.emit_condition(a, api, state, hir)?;
                let vb = self.emit_condition(b, api, state, hir)?;
                self.builder.build_and(va, vb, "and").unwrap()
            }
            HirCondition::Or(a, b) => {
                let va = self.emit_condition(a, api, state, hir)?;
                let vb = self.emit_condition(b, api, state, hir)?;
                self.builder.build_or(va, vb, "or").unwrap()
            }
            HirCondition::Cmp(a, op, b) => {
                let va = self.emit_expr_as_f64(a, state, None, hir, Some(api))?;
                let vb = self.emit_expr_as_f64(b, state, None, hir, Some(api))?;
                let pred = match op {
                    CmpOp::Eq => FloatPredicate::OEQ,
                    CmpOp::Ne => FloatPredicate::ONE,
                    CmpOp::Lt => FloatPredicate::OLT,
                    CmpOp::Le => FloatPredicate::OLE,
                    CmpOp::Gt => FloatPredicate::OGT,
                    CmpOp::Ge => FloatPredicate::OGE,
                };
                self.builder.build_float_compare(pred, va, vb, "cmp").unwrap()
            }
            HirCondition::Intrinsic(kind, args) => {
                self.emit_intrinsic_predicate(*kind, args, api, state, hir)?
            }
        })
    }

    fn emit_intrinsic_predicate(
        &self,
        kind: IntrinsicPredicate,
        args: &ThinVec<HirExpr>,
        api: PointerValue<'ctx>,
        state: PointerValue<'ctx>,
        hir: &HirScript,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        match kind {
            IntrinsicPredicate::EnemyInRange | IntrinsicPredicate::SeePlayer => {
                // For now: true iff actor_count > 0 AND first arg (radius) > 0.0.
                let actor_count = self.load_api_field(api, API_OFF_ACTOR_COUNT, self.i32_ty.into())
                    .into_int_value();
                let has_actors = self.builder.build_int_compare(
                    IntPredicate::UGT, actor_count, self.i32_ty.const_zero(), "has_actors"
                ).unwrap();
                let radius = if let Some(a) = args.first() {
                    self.emit_expr_as_f64(a, state, None, hir, Some(api))?
                } else { self.f64_ty.const_float(0.0) };
                let r_pos = self.builder.build_float_compare(
                    FloatPredicate::OGT, radius, self.f64_ty.const_float(0.0), "r_pos"
                ).unwrap();
                Ok(self.builder.build_and(has_actors, r_pos, "ipred").unwrap())
            }
            IntrinsicPredicate::ActorNear => {
                // Stub real semantics: true iff actor_count > 1.
                let actor_count = self.load_api_field(api, API_OFF_ACTOR_COUNT, self.i32_ty.into())
                    .into_int_value();
                Ok(self.builder.build_int_compare(
                    IntPredicate::UGT, actor_count, self.i32_ty.const_int(1, false), "near"
                ).unwrap())
            }
            IntrinsicPredicate::AfterSeconds => {
                let elapsed = self.load_api_field(api, API_OFF_ELAPSED, self.f32_ty.into())
                    .into_float_value();
                let elapsed64 = self.builder.build_float_ext(elapsed, self.f64_ty, "e64").unwrap();
                let t = if let Some(a) = args.first() {
                    self.emit_expr_as_f64(a, state, None, hir, Some(api))?
                } else { self.f64_ty.const_float(0.0) };
                Ok(self.builder.build_float_compare(FloatPredicate::OGE, elapsed64, t, "after").unwrap())
            }
            IntrinsicPredicate::EventFired => {
                // No event log yet exposed via the API.
                Ok(self.i1_ty.const_zero())
            }
            IntrinsicPredicate::Unknown => Ok(self.i1_ty.const_zero()),
        }
    }

    // ── Effects ──────────────────────────────────────────────────────────────

    fn emit_effect(
        &self,
        e: &HirEffect,
        api: PointerValue<'ctx>,
        state: PointerValue<'ctx>,
        hir: &HirScript,
    ) -> Result<(), CodegenError> {
        match e {
            HirEffect::CueTroupe(troupe_id) => {
                let cue_fn_ptr = self.load_api_field(api, API_OFF_CUE_TROUPE, self.ptr_ty.into())
                    .into_pointer_value();
                let cue_fn_ty = self.void_ty.fn_type(&[
                    self.ptr_ty.into(),
                    self.i64_ty.into(),
                ], false);
                let id = self.i64_ty.const_int(*troupe_id as u64, true);
                self.builder.build_indirect_call(
                    cue_fn_ty, cue_fn_ptr, &[api.into(), id.into()], ""
                ).unwrap();
            }
            HirEffect::Intrinsic(kind, args) => {
                // Build an EffectAbi on the stack and submit it via push_effect.
                let abi_alloca = self.builder.build_alloca(self.effect_abi_ty, "effect_abi").unwrap();

                // Zero via memset(8 bytes header) — easier than per-field stores.
                let memset = self.module.get_function("llvm.memset.p0.i64")
                    .unwrap_or_else(|| {
                        let memset_ty = self.void_ty.fn_type(&[
                            self.ptr_ty.into(),
                            self.i8_ty.into(),
                            self.i64_ty.into(),
                            self.i1_ty.into(),
                        ], false);
                        self.module.add_function("llvm.memset.p0.i64", memset_ty, None)
                    });
                self.builder.build_call(memset, &[
                    abi_alloca.into(),
                    self.i8_ty.const_zero().into(),
                    self.i64_ty.const_int(EFFECT_ABI_SIZE as u64, false).into(),
                    self.i1_ty.const_zero().into(),
                ], "").unwrap();

                let kind_byte = match kind {
                    IntrinsicEffect::EmitEvent   => EFFECT_KIND_EMIT_EVENT,
                    IntrinsicEffect::Attack      => EFFECT_KIND_ATTACK,
                    IntrinsicEffect::PatrolPath  => EFFECT_KIND_PATROL_PATH,
                    IntrinsicEffect::Unknown     => EFFECT_KIND_NOP,
                };
                // Store kind at offset 0.
                self.builder.build_store(abi_alloca, self.i8_ty.const_int(kind_byte as u64, false))
                    .unwrap();
                // Store first arg (truncated to i64) at offset 8 if present.
                if let Some(a) = args.first() {
                    let arg = self.emit_expr_as_i64(a, state, None, hir, Some(api))?;
                    let arg0_ptr = unsafe {
                        self.builder.build_in_bounds_gep(
                            self.i8_ty,
                            abi_alloca,
                            &[self.i64_ty.const_int(8, false)],
                            "arg0_ptr",
                        ).unwrap()
                    };
                    self.builder.build_store(arg0_ptr, arg).unwrap();
                }

                // Call push_effect(api, &abi).
                let push_fn_ptr = self.load_api_field(api, API_OFF_PUSH_EFFECT, self.ptr_ty.into())
                    .into_pointer_value();
                let push_fn_ty = self.void_ty.fn_type(&[
                    self.ptr_ty.into(),
                    self.ptr_ty.into(),
                ], false);
                self.builder.build_indirect_call(
                    push_fn_ty, push_fn_ptr,
                    &[api.into(), abi_alloca.into()],
                    "",
                ).unwrap();
            }
            HirEffect::AssignState { offset, ty, value } => {
                let v = self.emit_expr(value, state, None, Some(api), hir)?;
                self.store_field(state, *offset, *ty, v);
            }
        }
        Ok(())
    }

    // ── Expressions ──────────────────────────────────────────────────────────

    /// Evaluate an HIR expression, returning the LLVM value in its natural type.
    ///
    /// `api` is `None` only in contexts that cannot reach an API pointer
    /// (e.g. `df_init_state`); intrinsic value-returning calls
    /// (`elapsed()`, `tick_count()`) folded to 0 in that case.
    fn emit_expr(
        &self,
        e: &HirExpr,
        state: PointerValue<'ctx>,
        old_state: Option<PointerValue<'ctx>>,
        api: Option<PointerValue<'ctx>>,
        hir: &HirScript,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match e {
            HirExpr::Int(n)   => Ok(self.i32_ty.const_int(*n as u64, true).into()),
            HirExpr::Float(f) => Ok(self.f64_ty.const_float(*f).into()),
            HirExpr::Bool(b)  => Ok(self.i32_ty.const_int(*b as u64, false).into()),
            HirExpr::StateLoad { offset, ty } => Ok(self.load_field(state, *offset, *ty)),
            HirExpr::OldStateLoad { offset, ty } => {
                let p = old_state.ok_or_else(|| CodegenError::Llvm(
                    "old.<field> used outside migrate".into()
                ))?;
                Ok(self.load_field(p, *offset, *ty))
            }
            HirExpr::Neg(inner) => {
                let v = self.emit_expr_as_f64(inner, state, old_state, hir, api)?;
                Ok(self.builder.build_float_neg(v, "neg").unwrap().into())
            }
            HirExpr::Bin(a, op, b) => {
                let va = self.emit_expr_as_f64(a, state, old_state, hir, api)?;
                let vb = self.emit_expr_as_f64(b, state, old_state, hir, api)?;
                let v = match op {
                    BinOp::Add => self.builder.build_float_add(va, vb, "add").unwrap(),
                    BinOp::Sub => self.builder.build_float_sub(va, vb, "sub").unwrap(),
                    BinOp::Mul => self.builder.build_float_mul(va, vb, "mul").unwrap(),
                    BinOp::Div => self.builder.build_float_div(va, vb, "div").unwrap(),
                };
                Ok(v.into())
            }
            HirExpr::Intrinsic(kind, _args) => match (kind, api) {
                (IntrinsicValue::TickCount, Some(api)) => {
                    let tc = self.load_api_field(api, API_OFF_TICK_COUNT, self.i64_ty.into())
                        .into_int_value();
                    Ok(tc.into())
                }
                (IntrinsicValue::Elapsed, Some(api)) => {
                    let e = self.load_api_field(api, API_OFF_ELAPSED, self.f32_ty.into())
                        .into_float_value();
                    Ok(self.builder.build_float_ext(e, self.f64_ty, "el_f").unwrap().into())
                }
                _ => Ok(self.f64_ty.const_float(0.0).into()),
            }
        }
    }

    /// Coerce an HIR expression to `f64`.  Reads of `i32`/`bool` state fields
    /// are widened; intrinsic value-returning calls (`elapsed`, `tick_count`)
    /// are routed through the API here so they can read live values when one
    /// is available, otherwise fold to 0.
    fn emit_expr_as_f64(
        &self,
        e: &HirExpr,
        state: PointerValue<'ctx>,
        old_state: Option<PointerValue<'ctx>>,
        hir: &HirScript,
        api: Option<PointerValue<'ctx>>,
    ) -> Result<FloatValue<'ctx>, CodegenError> {
        match e {
            HirExpr::Int(n)   => Ok(self.f64_ty.const_float(*n as f64)),
            HirExpr::Float(f) => Ok(self.f64_ty.const_float(*f)),
            HirExpr::Bool(b)  => Ok(self.f64_ty.const_float(if *b { 1.0 } else { 0.0 })),
            HirExpr::StateLoad { offset, ty } => {
                let bv = self.load_field(state, *offset, *ty);
                Ok(self.to_f64(bv, *ty))
            }
            HirExpr::OldStateLoad { offset, ty } => {
                let p = old_state.ok_or_else(|| CodegenError::Llvm(
                    "old.<field> used outside migrate".into()
                ))?;
                let bv = self.load_field(p, *offset, *ty);
                Ok(self.to_f64(bv, *ty))
            }
            HirExpr::Neg(inner) => {
                let v = self.emit_expr_as_f64(inner, state, old_state, hir, api)?;
                Ok(self.builder.build_float_neg(v, "neg").unwrap())
            }
            HirExpr::Bin(a, op, b) => {
                let va = self.emit_expr_as_f64(a, state, old_state, hir, api)?;
                let vb = self.emit_expr_as_f64(b, state, old_state, hir, api)?;
                Ok(match op {
                    BinOp::Add => self.builder.build_float_add(va, vb, "add").unwrap(),
                    BinOp::Sub => self.builder.build_float_sub(va, vb, "sub").unwrap(),
                    BinOp::Mul => self.builder.build_float_mul(va, vb, "mul").unwrap(),
                    BinOp::Div => self.builder.build_float_div(va, vb, "div").unwrap(),
                })
            }
            HirExpr::Intrinsic(kind, _args) => Ok(match (kind, api) {
                (IntrinsicValue::TickCount, Some(api)) => {
                    let tc = self.load_api_field(api, API_OFF_TICK_COUNT, self.i64_ty.into())
                        .into_int_value();
                    self.builder.build_signed_int_to_float(tc, self.f64_ty, "tc_f").unwrap()
                }
                (IntrinsicValue::Elapsed, Some(api)) => {
                    let e = self.load_api_field(api, API_OFF_ELAPSED, self.f32_ty.into())
                        .into_float_value();
                    self.builder.build_float_ext(e, self.f64_ty, "el_f").unwrap()
                }
                _ => self.f64_ty.const_float(0.0),
            }),
        }
    }

    fn emit_expr_as_i64(
        &self,
        e: &HirExpr,
        state: PointerValue<'ctx>,
        old_state: Option<PointerValue<'ctx>>,
        hir: &HirScript,
        api: Option<PointerValue<'ctx>>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let f = self.emit_expr_as_f64(e, state, old_state, hir, api)?;
        Ok(self.builder.build_float_to_signed_int(f, self.i64_ty, "f2i").unwrap())
    }

    fn to_f64(&self, v: BasicValueEnum<'ctx>, ty: Ty) -> FloatValue<'ctx> {
        match ty {
            Ty::F64 => v.into_float_value(),
            Ty::I32 | Ty::Bool => self.builder.build_signed_int_to_float(
                v.into_int_value(), self.f64_ty, "i2f"
            ).unwrap(),
            Ty::ActorHandle | Ty::SceneId => self.builder.build_signed_int_to_float(
                v.into_int_value(), self.f64_ty, "i2f"
            ).unwrap(),
        }
    }

    // ── Field load / store ───────────────────────────────────────────────────

    fn store_field(&self, base: PointerValue<'ctx>, offset: u32, ty: Ty, val: BasicValueEnum<'ctx>) {
        let ptr = unsafe {
            self.builder.build_in_bounds_gep(
                self.i8_ty, base,
                &[self.i64_ty.const_int(offset as u64, false)],
                "fp",
            ).unwrap()
        };
        // Coerce val to the storage type if necessary.
        let stored: BasicValueEnum = match ty {
            Ty::I32 | Ty::Bool => {
                // Storage is i32.
                if let BasicValueEnum::FloatValue(f) = val {
                    self.builder.build_float_to_signed_int(f, self.i32_ty, "f2i").unwrap().into()
                } else if let BasicValueEnum::IntValue(i) = val {
                    if i.get_type().get_bit_width() != 32 {
                        self.builder.build_int_truncate_or_bit_cast(i, self.i32_ty, "trunc").unwrap().into()
                    } else { i.into() }
                } else { val }
            }
            Ty::F64 => {
                if let BasicValueEnum::IntValue(i) = val {
                    self.builder.build_signed_int_to_float(i, self.f64_ty, "i2f").unwrap().into()
                } else if let BasicValueEnum::FloatValue(f) = val {
                    if f.get_type() == self.f32_ty {
                        self.builder.build_float_ext(f, self.f64_ty, "f2f64").unwrap().into()
                    } else { f.into() }
                } else { val }
            }
            Ty::ActorHandle | Ty::SceneId => {
                // Storage is i64.
                if let BasicValueEnum::FloatValue(f) = val {
                    self.builder.build_float_to_signed_int(f, self.i64_ty, "f2i").unwrap().into()
                } else if let BasicValueEnum::IntValue(i) = val {
                    if i.get_type().get_bit_width() != 64 {
                        self.builder.build_int_s_extend_or_bit_cast(i, self.i64_ty, "sext").unwrap().into()
                    } else { i.into() }
                } else { val }
            }
        };
        self.builder.build_store(ptr, stored).unwrap();
    }

    fn load_field(&self, base: PointerValue<'ctx>, offset: u32, ty: Ty) -> BasicValueEnum<'ctx> {
        let ptr = unsafe {
            self.builder.build_in_bounds_gep(
                self.i8_ty, base,
                &[self.i64_ty.const_int(offset as u64, false)],
                "fp",
            ).unwrap()
        };
        let llvm_ty: BasicTypeEnum = match ty {
            Ty::I32 | Ty::Bool                            => self.i32_ty.into(),
            Ty::F64                                       => self.f64_ty.into(),
            Ty::ActorHandle | Ty::SceneId                 => self.i64_ty.into(),
        };
        self.builder.build_load(llvm_ty, ptr, "fv").unwrap()
    }

    fn load_api_field(
        &self,
        api: PointerValue<'ctx>,
        offset: u32,
        ty: BasicTypeEnum<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        let ptr = unsafe {
            self.builder.build_in_bounds_gep(
                self.i8_ty, api,
                &[self.i64_ty.const_int(offset as u64, false)],
                "api_fp",
            ).unwrap()
        };
        self.builder.build_load(ty, ptr, "api_fv").unwrap()
    }

    // ── Scene descriptor table + entry point ─────────────────────────────────

    fn emit_scene_def_table(
        &self,
        entries: &ThinVec<(i64, FunctionValue<'ctx>, FunctionValue<'ctx>, FunctionValue<'ctx>)>,
    ) {
        // Build a constant initializer for the `SceneEntry[]` table and a
        // private global referencing it.
        let mut consts: ThinVec<inkwell::values::StructValue<'ctx>> = ThinVec::new();
        for (raw_id, on_enter, on_exit, tick) in entries.iter() {
            let raw   = self.i64_ty.const_int(*raw_id as u64, true);
            let en_p  = on_enter.as_global_value().as_pointer_value();
            let ex_p  = on_exit .as_global_value().as_pointer_value();
            let tk_p  = tick    .as_global_value().as_pointer_value();
            consts.push(self.scene_entry_ty.const_named_struct(&[
                raw.into(), en_p.into(), ex_p.into(), tk_p.into(),
            ]));
        }
        let arr_ty = self.scene_entry_ty.array_type(consts.len() as u32);
        let arr_val = self.scene_entry_ty.const_array(&consts);
        let global = self.module.add_global(arr_ty, None, "scene_entries");
        global.set_initializer(&arr_val);
        global.set_linkage(Linkage::Private);
        global.set_constant(true);
    }

    fn emit_create_scene_defs_fn(&self, scene_count: u32) {
        let ty = self.void_ty.fn_type(&[
            self.ptr_ty.into(),  // api (unused; reserved for future intrinsics)
            self.ptr_ty.into(),  // out: *mut SceneDefArray
        ], false);
        let f = self.module.add_function("df_create_scene_defs", ty, Some(Linkage::External));
        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        let _api = f.get_nth_param(0).unwrap();
        let out  = f.get_nth_param(1).unwrap().into_pointer_value();

        // out.scene_count = N
        let count_ptr = out; // offset 0
        self.builder.build_store(count_ptr, self.i32_ty.const_int(scene_count as u64, false)).unwrap();
        // out.scenes = &scene_entries[0]
        let scenes_ptr_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                self.i8_ty, out,
                &[self.i64_ty.const_int(8, false)],
                "scenes_field",
            ).unwrap()
        };
        let table = self.module.get_global("scene_entries").expect("scene table missing");
        self.builder.build_store(scenes_ptr_ptr, table.as_pointer_value()).unwrap();

        self.builder.build_return(None).unwrap();
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CodegenError {
    Llvm(std::sync::Arc<str>),
}

impl core::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CodegenError::Llvm(s) => write!(f, "llvm: {s}"),
        }
    }
}

