use std::collections::HashMap;

use crate::ast::{BinOp, Literal, Mutability};
use crate::hir::*;
use crate::intern::{Interner, Symbol};
use crate::mir::*;
use crate::resolve::ResolveResult;
use crate::typeck::{TyKind, TypeckResult, IntTy, FloatTy};
use crate::diagnostics::Span;

pub struct MirBuilder<'a> {
    interner: &'a Interner,
    resolve: &'a ResolveResult,
    typeck: &'a TypeckResult,

    blocks: Vec<BasicBlock>,
    locals: Vec<LocalDecl>,
    current_block: BlockId,

    /// Map from DefId to Local index for local variables
    var_map: HashMap<DefId, Local>,

    /// Stack of (loop_header, loop_exit) block ids for break/continue
    loop_stack: Vec<(BlockId, BlockId)>,
}

impl<'a> MirBuilder<'a> {
    pub fn build_crate(
        interner: &'a Interner,
        resolve: &'a ResolveResult,
        typeck: &'a TypeckResult,
        hir: &HirCrate,
    ) -> Vec<MirBody> {
        let mut bodies = Vec::new();
        for item in &hir.items {
            Self::collect_fns(interner, resolve, typeck, item, &mut bodies);
        }
        bodies
    }

    fn collect_fns(
        interner: &'a Interner,
        resolve: &'a ResolveResult,
        typeck: &'a TypeckResult,
        item: &HirItem,
        bodies: &mut Vec<MirBody>,
    ) {
        match &item.kind {
            HirItemKind::Fn(f) => {
                if f.body.is_some() {
                    bodies.push(Self::build_fn(interner, resolve, typeck, f));
                }
            }
            HirItemKind::Impl(ib) => {
                for sub in &ib.items {
                    Self::collect_fns(interner, resolve, typeck, sub, bodies);
                }
            }
            HirItemKind::ExternBlock(eb) => {
                for sub in &eb.items {
                    Self::collect_fns(interner, resolve, typeck, sub, bodies);
                }
            }
            HirItemKind::Mod(m) => {
                if let Some(sub_items) = &m.items {
                    for sub in sub_items {
                        Self::collect_fns(interner, resolve, typeck, sub, bodies);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn build_fn(
        interner: &'a Interner,
        resolve: &'a ResolveResult,
        typeck: &'a TypeckResult,
        func: &HirFnDef,
    ) -> MirBody {
        let mut builder = MirBuilder {
            interner,
            resolve,
            typeck,
            blocks: Vec::new(),
            locals: Vec::new(),
            current_block: BlockId(0),
            var_map: HashMap::new(),
            loop_stack: Vec::new(),
        };

        // Create entry block
        builder.push_block();

        // Local _0 = return place
        let ret_ty = func.ret_ty.as_ref()
            .and_then(|_| typeck.expr_types.values().next()) // We'll derive from fn sig
            .cloned()
            .unwrap_or(TyKind::Unit);
        // Actually get return type properly - look at the fn sig via typeck
        let ret_ty = builder.get_fn_ret_ty(func);
        builder.alloc_local(ret_ty, None, Span::dummy());

        // Params: _1 .. _arg_count
        let arg_count = func.params.len();
        let param_tys = builder.typeck.fn_sigs.get(&func.def_id)
            .map(|(params, _)| params.clone())
            .unwrap_or_default();
        for (i, param) in func.params.iter().enumerate() {
            let ty = param_tys.get(i).cloned()
                .unwrap_or_else(|| builder.get_expr_ty_from_hir_ty(&param.ty));
            let name = match &param.pat {
                HirPattern::Ident(_, sym, _, _, _) => Some(*sym),
                _ => None,
            };
            let local = builder.alloc_local(ty, name, param.span);
            // Map the pattern's DefId to this local
            if let HirPattern::Ident(hir_id, _, _, _, _) = &param.pat {
                if let Some(&def_id) = resolve.resolutions.get(hir_id) {
                    builder.var_map.insert(def_id, local);
                }
            }
        }

        // Lower body
        if let Some(body) = &func.body {
            let result = builder.lower_block(body);
            // Assign result to _0 if not unit
            if let Some(result_op) = result {
                let ret_place = Place::local(Local(0));
                builder.emit_assign(ret_place, Rvalue::Use(result_op), body.span);
            }
            // Add return terminator if current block doesn't have one yet
            builder.terminate(Terminator::Return);
        }

        MirBody {
            basic_blocks: builder.blocks,
            locals: builder.locals,
            arg_count,
            name: func.name,
            span: Span::dummy(),
            no_mangle: func.no_mangle,
        }
    }

    fn get_fn_ret_ty(&self, func: &HirFnDef) -> TyKind {
        if let Some(ret_hir_ty) = &func.ret_ty {
            self.get_expr_ty_from_hir_ty(ret_hir_ty)
        } else {
            TyKind::Unit
        }
    }

    fn get_expr_ty_from_hir_ty(&self, _ty: &HirTy) -> TyKind {
        // Simplified: just return a generic type. The real type is in typeck.
        // For MIR construction purposes we mainly need it for local declarations.
        TyKind::Int(IntTy::I32) // placeholder
    }

    fn get_expr_ty(&self, expr: &HirExpr) -> TyKind {
        self.typeck.expr_types.get(&expr.id).cloned().unwrap_or(TyKind::Error)
    }

    fn alloc_local(&mut self, ty: TyKind, name: Option<Symbol>, span: Span) -> Local {
        let local = Local(self.locals.len());
        self.locals.push(LocalDecl {
            ty,
            mutability: Mutability::Immutable,
            name,
            span,
        });
        local
    }

    fn alloc_temp(&mut self, ty: TyKind, span: Span) -> Local {
        self.alloc_local(ty, None, span)
    }

    fn push_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len());
        self.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: Terminator::Unreachable, // placeholder
        });
        id
    }

    fn emit_stmt(&mut self, stmt: Statement) {
        self.blocks[self.current_block.0].statements.push(stmt);
    }

    fn emit_assign(&mut self, place: Place, rvalue: Rvalue, span: Span) {
        self.emit_stmt(Statement {
            kind: StatementKind::Assign(place, rvalue),
            span,
        });
    }

    fn terminate(&mut self, term: Terminator) {
        self.blocks[self.current_block.0].terminator = term;
    }

    fn goto_new_block(&mut self) -> BlockId {
        let next = self.push_block();
        self.terminate(Terminator::Goto(next));
        self.current_block = next;
        next
    }

    fn switch_to_block(&mut self, block: BlockId) {
        self.current_block = block;
    }

    // ── Lowering ──

    /// Lower a block, returning an operand for the block's value (if any)
    fn lower_block(&mut self, block: &HirBlock) -> Option<Operand> {
        let mut last_op = None;
        for (i, stmt) in block.stmts.iter().enumerate() {
            let is_last = i == block.stmts.len() - 1;
            match stmt {
                HirStmt::Let(hir_id, pat, _ty_ann, init, span) => {
                    // Allocate local for the binding
                    let ty = init.as_ref()
                        .map(|e| self.get_expr_ty(e))
                        .unwrap_or(TyKind::Int(IntTy::I32));
                    let name = match pat {
                        HirPattern::Ident(_, sym, _, _, _) => Some(*sym),
                        _ => None,
                    };
                    let local = self.alloc_local(ty, name, *span);

                    // Map DefId
                    if let HirPattern::Ident(pat_hir_id, _, _, _, _) = pat {
                        if let Some(&def_id) = self.resolve.resolutions.get(pat_hir_id) {
                            self.var_map.insert(def_id, local);
                        }
                    }

                    // Emit StorageLive
                    self.emit_stmt(Statement {
                        kind: StatementKind::StorageLive(local),
                        span: *span,
                    });

                    // Lower init
                    if let Some(init_expr) = init {
                        let op = self.lower_expr(init_expr);
                        self.emit_assign(Place::local(local), Rvalue::Use(op), *span);
                    }

                    last_op = None;
                }
                HirStmt::Expr(expr) => {
                    let op = self.lower_expr(expr);
                    last_op = if is_last { Some(op) } else { None };
                }
                HirStmt::Semi(expr, _span) => {
                    let _op = self.lower_expr(expr);
                    last_op = None;
                }
                HirStmt::Item(_) => {
                    last_op = None;
                }
            }
        }
        last_op
    }

    /// Lower an expression, returning an Operand for the result
    fn lower_expr(&mut self, expr: &HirExpr) -> Operand {
        match &expr.kind {
            HirExprKind::Lit(lit) => self.lower_literal(lit, expr),

            HirExprKind::Path(path) => self.lower_path(path, expr),

            HirExprKind::Binary(op, lhs, rhs) => {
                let l = self.lower_expr(lhs);
                let r = self.lower_expr(rhs);
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(
                    Place::local(tmp),
                    Rvalue::BinaryOp(*op, l, r),
                    expr.span,
                );
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Unary(op, inner) => {
                let operand = self.lower_expr(inner);
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(
                    Place::local(tmp),
                    Rvalue::UnaryOp(*op, operand),
                    expr.span,
                );
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Call(callee, args) => {
                // Check if this is an enum variant constructor call
                if let HirExprKind::Path(path) = &callee.kind {
                    if path.segments.len() == 2 {
                        let enum_name = path.segments[0].ident;
                        let variant_name = path.segments[1].ident;
                        if let Some(&variant_idx) = self.resolve.variant_indices.get(&(enum_name, variant_name)) {
                            // Check if this enum has data variants
                            let callee_ty = self.get_expr_ty(callee);
                            if let TyKind::Adt(enum_def_id, _) = &callee_ty {
                                if let Some(variants) = self.typeck.enum_variants.get(enum_def_id) {
                                    let max_fields = variants.iter().map(|(_, f)| f.len()).max().unwrap_or(0);
                                    if max_fields > 0 {
                                        // This is a data enum variant constructor
                                        let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                                        let ty = TyKind::Adt(*enum_def_id, vec![]);
                                        let tmp = self.alloc_temp(ty, expr.span);
                                        // Build aggregate: [discriminant, field0, field1, ...]
                                        let mut operands = vec![Operand::Constant(Constant {
                                            ty: TyKind::Int(IntTy::I64),
                                            value: ConstValue::Int(variant_idx as i128),
                                        })];
                                        operands.extend(arg_ops);
                                        // Pad with zeros up to max_fields
                                        let variant_fields = variants.get(variant_idx).map(|(_, f)| f.len()).unwrap_or(0);
                                        for _ in variant_fields..max_fields {
                                            operands.push(Operand::Constant(Constant {
                                                ty: TyKind::Int(IntTy::I64),
                                                value: ConstValue::Int(0),
                                            }));
                                        }
                                        self.emit_assign(
                                            Place::local(tmp),
                                            Rvalue::Aggregate(AggregateKind::Adt(*enum_def_id, variant_idx), operands),
                                            expr.span,
                                        );
                                        return Operand::Copy(Place::local(tmp));
                                    }
                                }
                            }
                        }
                    }
                }

                let func_op = self.lower_expr(callee);
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let ty = self.get_expr_ty(expr);
                let dest = self.alloc_temp(ty, expr.span);
                let next_block = self.push_block();
                self.terminate(Terminator::Call {
                    func: func_op,
                    args: arg_ops,
                    dest: Place::local(dest),
                    target: next_block,
                });
                self.current_block = next_block;
                Operand::Copy(Place::local(dest))
            }

            HirExprKind::If(cond, then_block, else_expr) => {
                let cond_op = self.lower_expr(cond);
                let ty = self.get_expr_ty(expr);
                let result = self.alloc_temp(ty, expr.span);

                let then_bb = self.push_block();
                let else_bb = self.push_block();
                let merge_bb = self.push_block();

                // SwitchInt: 0 -> else, otherwise -> then
                self.terminate(Terminator::SwitchInt {
                    operand: cond_op,
                    targets: vec![(0, else_bb)],
                    default: then_bb,
                });

                // Then
                self.current_block = then_bb;
                let then_op = self.lower_block(then_block);
                if let Some(op) = then_op {
                    self.emit_assign(Place::local(result), Rvalue::Use(op), expr.span);
                }
                self.terminate(Terminator::Goto(merge_bb));

                // Else
                self.current_block = else_bb;
                if let Some(else_e) = else_expr {
                    let else_op = self.lower_expr(else_e);
                    self.emit_assign(Place::local(result), Rvalue::Use(else_op), expr.span);
                }
                self.terminate(Terminator::Goto(merge_bb));

                self.current_block = merge_bb;
                Operand::Copy(Place::local(result))
            }

            HirExprKind::Block(block) => {
                self.lower_block(block).unwrap_or(Operand::Constant(Constant {
                    ty: TyKind::Unit,
                    value: ConstValue::Unit,
                }))
            }

            HirExprKind::Assign(lhs, rhs) => {
                let rhs_op = self.lower_expr(rhs);
                let place = self.lower_place(lhs);
                self.emit_assign(place, Rvalue::Use(rhs_op), expr.span);
                Operand::Constant(Constant { ty: TyKind::Unit, value: ConstValue::Unit })
            }

            HirExprKind::AssignOp(op, lhs, rhs) => {
                let lhs_op = self.lower_expr(lhs);
                let rhs_op = self.lower_expr(rhs);
                let place = self.lower_place(lhs);
                let ty = self.get_expr_ty(lhs);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(Place::local(tmp), Rvalue::BinaryOp(*op, lhs_op, rhs_op), expr.span);
                self.emit_assign(place, Rvalue::Use(Operand::Copy(Place::local(tmp))), expr.span);
                Operand::Constant(Constant { ty: TyKind::Unit, value: ConstValue::Unit })
            }

            HirExprKind::Return(val) => {
                let op = if let Some(v) = val {
                    self.lower_expr(v)
                } else {
                    Operand::Constant(Constant { ty: TyKind::Unit, value: ConstValue::Unit })
                };
                self.emit_assign(Place::local(Local(0)), Rvalue::Use(op), expr.span);
                self.terminate(Terminator::Return);
                // Create a new unreachable block for any subsequent code
                let unreachable_bb = self.push_block();
                self.current_block = unreachable_bb;
                Operand::Constant(Constant { ty: TyKind::Never, value: ConstValue::Unit })
            }

            HirExprKind::Loop(block, _label) => {
                let loop_header = self.push_block();
                let loop_exit = self.push_block();
                self.terminate(Terminator::Goto(loop_header));
                self.current_block = loop_header;

                self.loop_stack.push((loop_header, loop_exit));
                let _ = self.lower_block(block);
                self.loop_stack.pop();

                // Back edge
                self.terminate(Terminator::Goto(loop_header));
                self.current_block = loop_exit;
                Operand::Constant(Constant { ty: TyKind::Unit, value: ConstValue::Unit })
            }

            HirExprKind::Break(_label, val) => {
                if let Some(v) = val {
                    let _op = self.lower_expr(v);
                }
                if let Some(&(_, exit)) = self.loop_stack.last() {
                    self.terminate(Terminator::Goto(exit));
                }
                let unreachable_bb = self.push_block();
                self.current_block = unreachable_bb;
                Operand::Constant(Constant { ty: TyKind::Never, value: ConstValue::Unit })
            }

            HirExprKind::Continue(_label) => {
                if let Some(&(header, _)) = self.loop_stack.last() {
                    self.terminate(Terminator::Goto(header));
                }
                let unreachable_bb = self.push_block();
                self.current_block = unreachable_bb;
                Operand::Constant(Constant { ty: TyKind::Never, value: ConstValue::Unit })
            }

            HirExprKind::Ref(inner, mutability) => {
                let place = self.lower_place(inner);
                let borrow_kind = match mutability {
                    Mutability::Immutable => BorrowKind::Shared,
                    Mutability::Mut => BorrowKind::Mutable,
                };
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(Place::local(tmp), Rvalue::Ref(borrow_kind, place), expr.span);
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Deref(inner) => {
                let op = self.lower_expr(inner);
                // Put into a temp, then deref projection
                let inner_ty = self.get_expr_ty(inner);
                let tmp = self.alloc_temp(inner_ty, expr.span);
                self.emit_assign(Place::local(tmp), Rvalue::Use(op), expr.span);
                Operand::Copy(Place {
                    local: tmp,
                    projections: vec![Projection::Deref],
                })
            }

            HirExprKind::Struct(path, fields, _base) => {
                let field_ops: Vec<Operand> = fields.iter()
                    .map(|f| self.lower_expr(&f.value))
                    .collect();
                // Find struct DefId from path
                let def_id = if !path.segments.is_empty() {
                    // Try to resolve via the type checker's path
                    DefId(0) // placeholder
                } else {
                    DefId(0)
                };
                let ty = self.get_expr_ty(expr);
                // Extract actual DefId from ty if it's an Adt
                let actual_def_id = match &ty {
                    TyKind::Adt(did, _) => *did,
                    _ => def_id,
                };
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(
                    Place::local(tmp),
                    Rvalue::Aggregate(AggregateKind::Adt(actual_def_id, 0), field_ops),
                    expr.span,
                );
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Tuple(elems) => {
                let ops: Vec<Operand> = elems.iter().map(|e| self.lower_expr(e)).collect();
                if ops.is_empty() {
                    return Operand::Constant(Constant { ty: TyKind::Unit, value: ConstValue::Unit });
                }
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(
                    Place::local(tmp),
                    Rvalue::Aggregate(AggregateKind::Tuple, ops),
                    expr.span,
                );
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Array(elems) => {
                let ops: Vec<Operand> = elems.iter().map(|e| self.lower_expr(e)).collect();
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(
                    Place::local(tmp),
                    Rvalue::Aggregate(AggregateKind::Array, ops),
                    expr.span,
                );
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Field(base, field_name) => {
                let field_idx = self.resolve_field_index(base, *field_name);
                let base_ty = self.get_expr_ty(base);
                let needs_deref = matches!(&base_ty, TyKind::Ref(_, _));

                // Try to get a direct place for the base to avoid copying the whole struct
                let base_place = self.try_lower_to_place(base);
                if let Some(mut place) = base_place {
                    if needs_deref {
                        place.projections.push(Projection::Deref);
                    }
                    place.projections.push(Projection::Field(field_idx));
                    Operand::Copy(place)
                } else {
                    // Base is a complex expression; lower it into a temp
                    let base_op = self.lower_expr(base);
                    let tmp = self.alloc_temp(base_ty, expr.span);
                    self.emit_assign(Place::local(tmp), Rvalue::Use(base_op), expr.span);
                    let mut projs = Vec::new();
                    if needs_deref {
                        projs.push(Projection::Deref);
                    }
                    projs.push(Projection::Field(field_idx));
                    Operand::Copy(Place {
                        local: tmp,
                        projections: projs,
                    })
                }
            }

            HirExprKind::Index(base, idx) => {
                let base_op = self.lower_expr(base);
                let idx_op = self.lower_expr(idx);
                let base_ty = self.get_expr_ty(base);
                let base_tmp = self.alloc_temp(base_ty, expr.span);
                self.emit_assign(Place::local(base_tmp), Rvalue::Use(base_op), expr.span);
                let idx_ty = self.get_expr_ty(idx);
                let idx_tmp = self.alloc_temp(idx_ty, expr.span);
                self.emit_assign(Place::local(idx_tmp), Rvalue::Use(idx_op), expr.span);
                Operand::Copy(Place {
                    local: base_tmp,
                    projections: vec![Projection::Index(idx_tmp)],
                })
            }

            HirExprKind::Cast(inner, _ty) => {
                let op = self.lower_expr(inner);
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty.clone(), expr.span);
                self.emit_assign(Place::local(tmp), Rvalue::Cast(op, ty), expr.span);
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Paren(inner) => self.lower_expr(inner),
            HirExprKind::Unsafe(block) => {
                self.lower_block(block).unwrap_or(Operand::Constant(Constant {
                    ty: TyKind::Unit,
                    value: ConstValue::Unit,
                }))
            }

            HirExprKind::MethodCall(recv, method_name, _, args) => {
                // Resolve the method: find the impl method DefId by looking at receiver type
                let recv_ty = self.get_expr_ty(recv);
                let inner_ty = match &recv_ty {
                    TyKind::Ref(inner, _) => inner.as_ref().clone(),
                    other => other.clone(),
                };

                let method_def_id = if let TyKind::Adt(def_id, _) = &inner_ty {
                    // Find type name for this DefId
                    let type_name = self.typeck.struct_defs.keys()
                        .find(|&&did| did == *def_id)
                        .and_then(|_| {
                            // Search impl_methods by checking all type names
                            self.resolve.impl_methods.iter()
                                .find_map(|(ty_name, methods)| {
                                    methods.iter()
                                        .find(|(n, _)| *n == *method_name)
                                        .map(|(_, did)| *did)
                                })
                        });
                    type_name
                } else {
                    None
                };

                if let Some(method_did) = method_def_id {
                    // Get method name symbol
                    let fn_name = *method_name;

                    // Build receiver operand - auto-ref if needed
                    let recv_op = match &recv_ty {
                        TyKind::Ref(_, _) => {
                            // Already a reference, just lower it
                            self.lower_expr(recv)
                        }
                        _ => {
                            // Need to take a reference: &recv
                            let place = self.lower_place(recv);
                            let ref_ty = TyKind::Ref(Box::new(recv_ty.clone()), Mutability::Immutable);
                            let tmp = self.alloc_temp(ref_ty, expr.span);
                            self.emit_assign(Place::local(tmp), Rvalue::Ref(BorrowKind::Shared, place), expr.span);
                            Operand::Copy(Place::local(tmp))
                        }
                    };

                    // Build args: [receiver, ...user_args]
                    let mut all_args = vec![recv_op];
                    for a in args {
                        all_args.push(self.lower_expr(a));
                    }

                    let ty = self.get_expr_ty(expr);
                    let dest = self.alloc_temp(ty, expr.span);
                    let next_block = self.push_block();
                    self.terminate(Terminator::Call {
                        func: Operand::Constant(Constant {
                            ty: TyKind::FnDef(method_did, vec![]),
                            value: ConstValue::FnItem(fn_name),
                        }),
                        args: all_args,
                        dest: Place::local(dest),
                        target: next_block,
                    });
                    self.current_block = next_block;
                    Operand::Copy(Place::local(dest))
                } else {
                    // Fallback
                    Operand::Constant(Constant { ty: TyKind::Unit, value: ConstValue::Unit })
                }
            }

            HirExprKind::Match(scrutinee, arms) => {
                let scr_op = self.lower_expr(scrutinee);
                let scr_ty = self.get_expr_ty(scrutinee);
                let result_ty = self.get_expr_ty(expr);
                let result = self.alloc_temp(result_ty, expr.span);

                // For enum match: store scrutinee, get discriminant, then switch
                let (disc_op, scr_local) = if let TyKind::Adt(_, _) = &scr_ty {
                    let scr_tmp = self.alloc_temp(scr_ty.clone(), expr.span);
                    self.emit_assign(Place::local(scr_tmp), Rvalue::Use(scr_op), expr.span);
                    let disc_tmp = self.alloc_temp(TyKind::Int(IntTy::I64), expr.span);
                    self.emit_assign(Place::local(disc_tmp), Rvalue::Discriminant(Place::local(scr_tmp)), expr.span);
                    (Operand::Copy(Place::local(disc_tmp)), Some(scr_tmp))
                } else {
                    (scr_op, None)
                };

                // Save the block where discriminant was computed
                let switch_bb = self.current_block;

                let merge_bb = self.push_block();
                let mut targets: Vec<(u128, BlockId)> = Vec::new();
                let mut default_bb = merge_bb;
                let mut range_arms: Vec<(BlockId, BlockId)> = Vec::new(); // (check_bb, arm_bb)

                // Check if any arm has a range pattern
                let has_range = arms.iter().any(|a| self.is_range_pattern(&a.pat));

                for arm in arms {
                    let arm_bb = self.push_block();

                    if let HirPattern::Range(lo, hi, _inclusive, span) = &arm.pat {
                        // Range pattern: emit comparison chain
                        let check_bb = self.push_block();
                        range_arms.push((check_bb, arm_bb));

                        self.current_block = check_bb;

                        // Compare: disc_op >= lo && disc_op <= hi
                        let in_range = if let (Some(lo_expr), Some(hi_expr)) = (lo, hi) {
                            let lo_op = self.lower_expr(lo_expr);
                            let hi_op = self.lower_expr(hi_expr);
                            // ge_tmp = disc >= lo
                            let ge_tmp = self.alloc_temp(TyKind::Bool, *span);
                            self.emit_assign(Place::local(ge_tmp), Rvalue::BinaryOp(BinOp::Ge, disc_op.clone(), lo_op), *span);
                            // le_tmp = disc <= hi
                            let le_tmp = self.alloc_temp(TyKind::Bool, *span);
                            self.emit_assign(Place::local(le_tmp), Rvalue::BinaryOp(BinOp::Le, disc_op.clone(), hi_op), *span);
                            // in_range = ge && le
                            let in_range = self.alloc_temp(TyKind::Bool, *span);
                            self.emit_assign(Place::local(in_range), Rvalue::BinaryOp(BinOp::And, Operand::Copy(Place::local(ge_tmp)), Operand::Copy(Place::local(le_tmp))), *span);
                            Operand::Copy(Place::local(in_range))
                        } else {
                            Operand::Constant(Constant { ty: TyKind::Bool, value: ConstValue::Bool(true) })
                        };

                        // SwitchInt on in_range: 1 → arm_bb, default → next
                        // We'll fix up 'default' (next check or default_bb) later
                        let placeholder_bb = self.push_block(); // placeholder for "not in range"
                        self.blocks[check_bb.0].terminator = Terminator::SwitchInt {
                            operand: in_range,
                            targets: vec![(1, arm_bb)],
                            default: placeholder_bb,
                        };

                        // placeholder_bb will be patched to point to next range check or switch/default
                        // For now, store it -- we'll chain them below
                    } else {
                        let disc_val = self.pattern_discriminant(&arm.pat);
                        if let Some(val) = disc_val {
                            targets.push((val, arm_bb));
                        } else {
                            default_bb = arm_bb;
                        }
                    }

                    self.current_block = arm_bb;

                    // Bind inner pattern variables for TupleStruct patterns
                    if let HirPattern::TupleStruct(_, inner_pats, _) = &arm.pat {
                        if let Some(scr_local) = scr_local {
                            for (field_idx, inner_pat) in inner_pats.iter().enumerate() {
                                if let HirPattern::Ident(hir_id, name, _, _, _) = inner_pat {
                                    if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                                        let field_ty = TyKind::Int(IntTy::I64); // placeholder
                                        let local = self.alloc_local(field_ty, Some(*name), expr.span);
                                        self.var_map.insert(def_id, local);
                                        // Load from scrutinee's field (slot 1 + field_idx, since slot 0 is disc)
                                        let src_place = Place {
                                            local: scr_local,
                                            projections: vec![Projection::Field(1 + field_idx)],
                                        };
                                        self.emit_assign(
                                            Place::local(local),
                                            Rvalue::Use(Operand::Copy(src_place)),
                                            expr.span,
                                        );
                                    }
                                }
                            }
                        }
                    }

                    let arm_op = self.lower_expr(&arm.body);
                    self.emit_assign(Place::local(result), Rvalue::Use(arm_op), arm.span);
                    self.terminate(Terminator::Goto(merge_bb));
                }

                if default_bb == merge_bb && !targets.is_empty() {
                    let last = targets.pop().unwrap();
                    default_bb = last.1;
                }

                if has_range && !range_arms.is_empty() {
                    // Build the final target for non-range arms (SwitchInt or just goto default)
                    let non_range_target_bb = if targets.is_empty() {
                        default_bb
                    } else {
                        let sw_bb = self.push_block();
                        self.blocks[sw_bb.0].terminator = Terminator::SwitchInt {
                            operand: disc_op,
                            targets,
                            default: default_bb,
                        };
                        sw_bb
                    };

                    // Chain: switch_bb → first range check → second range check → ... → non_range_target_bb
                    self.blocks[switch_bb.0].terminator = Terminator::Goto(range_arms[0].0);

                    for i in 0..range_arms.len() {
                        let (check_bb, _arm_bb) = range_arms[i];
                        let next = if i + 1 < range_arms.len() {
                            range_arms[i + 1].0
                        } else {
                            non_range_target_bb
                        };
                        if let Terminator::SwitchInt { ref mut default, .. } = self.blocks[check_bb.0].terminator {
                            let placeholder = *default;
                            self.blocks[placeholder.0].terminator = Terminator::Goto(next);
                        }
                    }
                } else {
                    // No range patterns: use normal SwitchInt
                    self.blocks[switch_bb.0].terminator = Terminator::SwitchInt {
                        operand: disc_op,
                        targets,
                        default: default_bb,
                    };
                }

                self.current_block = merge_bb;
                Operand::Copy(Place::local(result))
            }

            HirExprKind::InlineAsm(asm) => {
                let mut mir_operands = Vec::new();
                for op in &asm.operands {
                    match op {
                        HirAsmOperand::In { reg, expr } => {
                            let operand = self.lower_expr(expr);
                            mir_operands.push(MirAsmOperand {
                                kind: MirAsmOperandKind::In(operand),
                                reg: match reg {
                                    HirAsmReg::Named(s) => MirAsmReg::Named(s.clone()),
                                    HirAsmReg::Class(s) => MirAsmReg::Class(s.clone()),
                                },
                            });
                        }
                        HirAsmOperand::Out { reg, expr } => {
                            let place = expr.as_ref().map(|e| {
                                self.try_lower_to_place(e)
                                    .unwrap_or_else(|| Place::local(self.alloc_temp(TyKind::Uint(crate::typeck::UintTy::U64), e.span)))
                            });
                            mir_operands.push(MirAsmOperand {
                                kind: MirAsmOperandKind::Out(place),
                                reg: match reg {
                                    HirAsmReg::Named(s) => MirAsmReg::Named(s.clone()),
                                    HirAsmReg::Class(s) => MirAsmReg::Class(s.clone()),
                                },
                            });
                        }
                        HirAsmOperand::InOut { reg, expr } => {
                            let operand = self.lower_expr(expr);
                            let place = self.try_lower_to_place(expr);
                            mir_operands.push(MirAsmOperand {
                                kind: MirAsmOperandKind::InOut(operand, place),
                                reg: match reg {
                                    HirAsmReg::Named(s) => MirAsmReg::Named(s.clone()),
                                    HirAsmReg::Class(s) => MirAsmReg::Class(s.clone()),
                                },
                            });
                        }
                    }
                }
                self.emit_stmt(Statement {
                    kind: StatementKind::InlineAsm {
                        template: asm.template.clone(),
                        operands: mir_operands,
                        options: asm.options.clone(),
                    },
                    span: expr.span,
                });
                Operand::Constant(Constant { ty: TyKind::Unit, value: ConstValue::Unit })
            }

            // Catch-all for unhandled cases
            HirExprKind::Closure(_, _, _, _) |
            HirExprKind::ArrayRepeat(_, _) |
            HirExprKind::Range(_, _, _) |
            HirExprKind::Try(_) |
            HirExprKind::For(_, _, _, _) => {
                Operand::Constant(Constant { ty: TyKind::Unit, value: ConstValue::Unit })
            }
        }
    }

    /// Try to get a Place for an expression without lowering it (no side effects).
    /// Returns None if the expression is not directly addressable.
    fn try_lower_to_place(&self, expr: &HirExpr) -> Option<Place> {
        match &expr.kind {
            HirExprKind::Path(path) => {
                self.resolve_path_to_local(path, expr.id)
                    .map(|local| Place::local(local))
            }
            HirExprKind::Field(base, field_name) => {
                let mut place = self.try_lower_to_place(base)?;
                let idx = self.resolve_field_index(base, *field_name);
                place.projections.push(Projection::Field(idx));
                Some(place)
            }
            HirExprKind::Paren(inner) => self.try_lower_to_place(inner),
            _ => None,
        }
    }

    /// Lower an expression to a Place (for assignment targets)
    fn lower_place(&mut self, expr: &HirExpr) -> Place {
        match &expr.kind {
            HirExprKind::Path(path) => {
                // Check if it's a static - allocate a temp with StaticRef
                if let Some(&def_id) = self.resolve.resolutions.get(&expr.id) {
                    if let Some((name, ty, _, _)) = self.typeck.static_defs.get(&def_id) {
                        // For assignment to a static, we create a temp that holds the
                        // static address via Projection::Static
                        let tmp = self.alloc_temp(TyKind::RawPtr(Box::new(ty.clone()), crate::ast::Mutability::Mut), expr.span);
                        self.emit_assign(
                            Place::local(tmp),
                            Rvalue::Use(Operand::Constant(Constant {
                                ty: TyKind::RawPtr(Box::new(ty.clone()), crate::ast::Mutability::Mut),
                                value: ConstValue::StaticRef(*name),
                            })),
                            expr.span,
                        );
                        return Place {
                            local: tmp,
                            projections: vec![Projection::Deref],
                        };
                    }
                }
                if let Some(local) = self.resolve_path_to_local(path, expr.id) {
                    Place::local(local)
                } else {
                    // Fallback: create a temp
                    let tmp = self.alloc_temp(TyKind::Error, expr.span);
                    Place::local(tmp)
                }
            }
            HirExprKind::Deref(inner) => {
                let mut place = self.lower_place(inner);
                place.projections.push(Projection::Deref);
                place
            }
            HirExprKind::Field(base, field_name) => {
                let mut place = self.lower_place(base);
                let idx = self.resolve_field_index(base, *field_name);
                place.projections.push(Projection::Field(idx));
                place
            }
            HirExprKind::Index(base, idx) => {
                let mut place = self.lower_place(base);
                let idx_op = self.lower_expr(idx);
                let idx_ty = self.get_expr_ty(idx);
                let idx_tmp = self.alloc_temp(idx_ty, expr.span);
                self.emit_assign(Place::local(idx_tmp), Rvalue::Use(idx_op), expr.span);
                place.projections.push(Projection::Index(idx_tmp));
                place
            }
            _ => {
                // Expression isn't directly a place, lower and put into temp
                let op = self.lower_expr(expr);
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(Place::local(tmp), Rvalue::Use(op), expr.span);
                Place::local(tmp)
            }
        }
    }

    fn lower_literal(&self, lit: &Literal, expr: &HirExpr) -> Operand {
        let (ty, value) = match lit {
            Literal::Int(v) => {
                let ty = self.get_expr_ty(expr);
                (ty, ConstValue::Int(*v as i128))
            }
            Literal::Float(v) => {
                let ty = self.get_expr_ty(expr);
                (ty, ConstValue::Float(*v))
            }
            Literal::Bool(v) => (TyKind::Bool, ConstValue::Bool(*v)),
            Literal::Char(v) => (TyKind::Char, ConstValue::Char(*v)),
            Literal::String(v) => (
                TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable),
                ConstValue::Str(v.clone()),
            ),
            Literal::ByteString(_) => (TyKind::Unit, ConstValue::Unit),
        };
        Operand::Constant(Constant { ty, value })
    }

    fn lower_path(&mut self, path: &HirPath, expr: &HirExpr) -> Operand {
        // Check if it resolves to a const
        if let Some(&def_id) = self.resolve.resolutions.get(&expr.id) {
            if let Some((cv, ty)) = self.typeck.const_values.get(&def_id) {
                let val = match cv {
                    crate::typeck::ConstVal::Int(v) => ConstValue::Int(*v),
                    crate::typeck::ConstVal::Bool(v) => ConstValue::Bool(*v),
                    crate::typeck::ConstVal::Char(v) => ConstValue::Char(*v),
                };
                return Operand::Constant(Constant { ty: ty.clone(), value: val });
            }
            // Check if it resolves to a static
            if let Some((name, ty, _, _)) = self.typeck.static_defs.get(&def_id) {
                let name = *name;
                let ty = ty.clone();
                // Load address of static into a temp (LEA)
                let ptr_ty = TyKind::RawPtr(Box::new(ty.clone()), crate::ast::Mutability::Mut);
                let addr_tmp = self.alloc_temp(ptr_ty, expr.span);
                self.emit_assign(
                    Place::local(addr_tmp),
                    Rvalue::Use(Operand::Constant(Constant {
                        ty: TyKind::RawPtr(Box::new(ty.clone()), crate::ast::Mutability::Mut),
                        value: ConstValue::StaticRef(name),
                    })),
                    expr.span,
                );
                // Dereference to load the value
                return Operand::Copy(Place {
                    local: addr_tmp,
                    projections: vec![Projection::Deref],
                });
            }
        }

        // Check if it resolves to a local variable
        if let Some(local) = self.resolve_path_to_local(path, expr.id) {
            let ty = self.get_expr_ty(expr);
            if self.is_copy_type(&ty) {
                Operand::Copy(Place::local(local))
            } else {
                Operand::Move(Place::local(local))
            }
        } else {
            let ty = self.get_expr_ty(expr);
            match &ty {
                TyKind::FnDef(_def_id, _) => {
                    let fn_name = path.segments.last().unwrap().ident;
                    Operand::Constant(Constant {
                        ty: ty.clone(),
                        value: ConstValue::FnItem(fn_name),
                    })
                }
                TyKind::Adt(ref enum_def_id, _) if path.segments.len() == 2 => {
                    let enum_def_id = *enum_def_id;
                    // Enum variant: look up discriminant
                    let enum_name = path.segments[0].ident;
                    let variant_name = path.segments[1].ident;
                    if let Some(&idx) = self.resolve.variant_indices.get(&(enum_name, variant_name)) {
                        // Check if this is a data enum (has variants with fields)
                        if let Some(variants) = self.typeck.enum_variants.get(&enum_def_id) {
                            let max_fields = variants.iter().map(|(_, f)| f.len()).max().unwrap_or(0);
                            if max_fields > 0 {
                                // Data enum unit variant: emit Aggregate with disc + zero padding
                                let tmp = self.alloc_temp(ty.clone(), expr.span);
                                let mut operands = vec![Operand::Constant(Constant {
                                    ty: TyKind::Int(IntTy::I64),
                                    value: ConstValue::Int(idx as i128),
                                })];
                                for _ in 0..max_fields {
                                    operands.push(Operand::Constant(Constant {
                                        ty: TyKind::Int(IntTy::I64),
                                        value: ConstValue::Int(0),
                                    }));
                                }
                                self.emit_assign(
                                    Place::local(tmp),
                                    Rvalue::Aggregate(AggregateKind::Adt(enum_def_id, idx), operands),
                                    expr.span,
                                );
                                return Operand::Copy(Place::local(tmp));
                            }
                        }
                        // C-like enum: just the discriminant value
                        Operand::Constant(Constant {
                            ty,
                            value: ConstValue::Int(idx as i128),
                        })
                    } else {
                        Operand::Constant(Constant { ty, value: ConstValue::Unit })
                    }
                }
                _ => Operand::Constant(Constant { ty, value: ConstValue::Unit }),
            }
        }
    }

    fn resolve_path_to_local(&self, path: &HirPath, hir_id: HirId) -> Option<Local> {
        if let Some(&def_id) = self.resolve.resolutions.get(&hir_id) {
            self.var_map.get(&def_id).copied()
        } else {
            None
        }
    }

    fn resolve_field_index(&self, base_expr: &HirExpr, field_name: Symbol) -> usize {
        // Try to parse as tuple index
        let name_str = self.interner.resolve(field_name);
        if let Ok(idx) = name_str.parse::<usize>() {
            return idx;
        }
        // Look up field order from struct definition
        let base_ty = self.get_expr_ty(base_expr);
        let base_ty = match &base_ty {
            TyKind::Ref(inner, _) => inner.as_ref(),
            other => other,
        };
        if let TyKind::Adt(def_id, _) = base_ty {
            if let Some(fields) = self.typeck.struct_defs.get(def_id) {
                for (i, (sym, _)) in fields.iter().enumerate() {
                    if *sym == field_name {
                        return i;
                    }
                }
            }
        }
        0
    }

    /// Extract discriminant value from a match arm pattern.
    fn pattern_discriminant(&self, pat: &HirPattern) -> Option<u128> {
        match pat {
            HirPattern::Path(path) if path.segments.len() == 2 => {
                let enum_name = path.segments[0].ident;
                let variant_name = path.segments[1].ident;
                self.resolve.variant_indices.get(&(enum_name, variant_name))
                    .map(|&idx| idx as u128)
            }
            HirPattern::TupleStruct(path, _, _) if path.segments.len() == 2 => {
                let enum_name = path.segments[0].ident;
                let variant_name = path.segments[1].ident;
                self.resolve.variant_indices.get(&(enum_name, variant_name))
                    .map(|&idx| idx as u128)
            }
            HirPattern::Literal(lit, _) => {
                match lit {
                    Literal::Int(v) => Some(*v as u128),
                    Literal::Bool(b) => Some(*b as u128),
                    Literal::Char(c) => Some(*c as u128),
                    _ => None,
                }
            }
            HirPattern::Wildcard(_) => None,
            // Range patterns are handled specially in match lowering
            HirPattern::Range(_, _, _, _) => None,
            _ => None,
        }
    }

    /// Check if a pattern is a range pattern
    fn is_range_pattern(&self, pat: &HirPattern) -> bool {
        matches!(pat, HirPattern::Range(_, _, _, _))
    }

    fn is_copy_type(&self, ty: &TyKind) -> bool {
        match ty {
            TyKind::Bool | TyKind::Char => true,
            TyKind::Int(_) | TyKind::Uint(_) | TyKind::Float(_) => true,
            TyKind::Ref(_, _) => true,
            TyKind::RawPtr(_, _) => true,
            TyKind::Unit | TyKind::Never | TyKind::Error => true,
            TyKind::FnDef(_, _) | TyKind::FnPtr(_, _) => true,
            _ => false,
        }
    }
}
