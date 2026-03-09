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
        for param in &func.params {
            let ty = builder.get_expr_ty_from_hir_ty(&param.ty);
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
                let base_op = self.lower_expr(base);
                // For field access, put base in a local and add Field projection
                let base_ty = self.get_expr_ty(base);
                let tmp = self.alloc_temp(base_ty, expr.span);
                self.emit_assign(Place::local(tmp), Rvalue::Use(base_op), expr.span);
                // Determine field index (simplified: use symbol to find index)
                let field_idx = self.resolve_field_index(base, *field_name);
                Operand::Copy(Place {
                    local: tmp,
                    projections: vec![Projection::Field(field_idx)],
                })
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

            // Catch-all for unhandled cases
            HirExprKind::Match(_, _, ..) |
            HirExprKind::MethodCall(_, _, _, _) |
            HirExprKind::Closure(_, _, _, _) |
            HirExprKind::ArrayRepeat(_, _) |
            HirExprKind::Range(_, _, _) |
            HirExprKind::Try(_) |
            HirExprKind::For(_, _, _, _) => {
                Operand::Constant(Constant { ty: TyKind::Unit, value: ConstValue::Unit })
            }
        }
    }

    /// Lower an expression to a Place (for assignment targets)
    fn lower_place(&mut self, expr: &HirExpr) -> Place {
        match &expr.kind {
            HirExprKind::Path(path) => {
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
        // Check if it resolves to a local variable
        if let Some(local) = self.resolve_path_to_local(path, expr.id) {
            let ty = self.get_expr_ty(expr);
            if self.is_copy_type(&ty) {
                Operand::Copy(Place::local(local))
            } else {
                Operand::Move(Place::local(local))
            }
        } else {
            // Could be a function reference
            let ty = self.get_expr_ty(expr);
            match &ty {
                TyKind::FnDef(_def_id, _) => {
                    let fn_name = path.segments.last().unwrap().ident;
                    Operand::Constant(Constant {
                        ty: ty.clone(),
                        value: ConstValue::FnItem(fn_name),
                    })
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
        // For struct fields, would need to look up field order. Simplified: use 0.
        0
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
