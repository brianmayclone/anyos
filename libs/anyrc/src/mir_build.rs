use crate::prelude::*;
use anyos_std::collections::HashMap;

use crate::ast::{BinOp, Literal, Mutability};
use crate::diagnostics::Span;
use crate::hir::*;
use crate::intern::{Interner, Symbol};
use crate::mir::*;
use crate::resolve::ResolveResult;
use crate::typeck::{FloatTy, IntTy, TyKind, TypeckResult, UintTy};

pub struct MirBuilder<'a> {
    interner: &'a mut Interner,
    resolve: &'a ResolveResult,
    typeck: &'a TypeckResult,

    blocks: Vec<BasicBlock>,
    locals: Vec<LocalDecl>,
    current_block: BlockId,

    /// Map from DefId to Local index for local variables
    var_map: HashMap<DefId, Local>,

    /// Stack of (loop_header, loop_exit) block ids for break/continue
    loop_stack: Vec<(BlockId, BlockId)>,

    /// Extra MirBodies generated from closure expressions
    extra_bodies: Vec<MirBody>,

    /// Counter for generating unique closure names
    closure_counter: u32,

    /// Map from closure DefId to its symbol name
    closure_symbols: HashMap<DefId, Symbol>,
}

impl<'a> MirBuilder<'a> {
    pub fn build_crate(
        interner: &mut Interner,
        resolve: &ResolveResult,
        typeck: &TypeckResult,
        hir: &HirCrate,
    ) -> Vec<MirBody> {
        // First collect all function defs (no mut borrow needed)
        let mut fn_defs = Vec::new();
        for item in &hir.items {
            Self::collect_fn_defs(item, &mut fn_defs);
        }
        // Then build MIR for each (sequential, so &mut interner is fine)
        let mut bodies = Vec::new();
        for f in &fn_defs {
            bodies.extend(Self::build_fn(interner, resolve, typeck, f));
        }
        bodies
    }

    fn collect_fn_defs<'b>(item: &'b HirItem, out: &mut Vec<&'b HirFnDef>) {
        match &item.kind {
            HirItemKind::Fn(f) => {
                if f.body.is_some() {
                    out.push(f);
                    if let Some(body) = &f.body {
                        Self::collect_fn_defs_from_block(body, out);
                    }
                }
            }
            HirItemKind::Impl(ib) => {
                for sub in &ib.items {
                    Self::collect_fn_defs(sub, out);
                }
            }
            HirItemKind::Trait(t) => {
                // Collect trait methods that have default bodies — these are
                // codegen'd so that impls which don't override them can call them.
                for sub in &t.items {
                    if let HirItemKind::Fn(f) = &sub.kind {
                        if f.body.is_some() {
                            out.push(f);
                        }
                    }
                }
            }
            HirItemKind::ExternBlock(eb) => {
                for sub in &eb.items {
                    Self::collect_fn_defs(sub, out);
                }
            }
            HirItemKind::Mod(m) => {
                if let Some(sub_items) = &m.items {
                    for sub in sub_items {
                        Self::collect_fn_defs(sub, out);
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_fn_defs_from_block<'b>(block: &'b HirBlock, out: &mut Vec<&'b HirFnDef>) {
        for stmt in &block.stmts {
            Self::collect_fn_defs_from_stmt(stmt, out);
        }
    }

    fn collect_fn_defs_from_stmt<'b>(stmt: &'b HirStmt, out: &mut Vec<&'b HirFnDef>) {
        match stmt {
            HirStmt::Let(_, _, _, init, _) => {
                if let Some(init) = init {
                    Self::collect_fn_defs_from_expr(init, out);
                }
            }
            HirStmt::Expr(expr) | HirStmt::Semi(expr, _) => {
                Self::collect_fn_defs_from_expr(expr, out);
            }
            HirStmt::Item(item) => {
                Self::collect_fn_defs(item, out);
            }
        }
    }

    fn collect_fn_defs_from_expr<'b>(expr: &'b HirExpr, out: &mut Vec<&'b HirFnDef>) {
        match &expr.kind {
            HirExprKind::Call(callee, args) => {
                Self::collect_fn_defs_from_expr(callee, out);
                for arg in args {
                    Self::collect_fn_defs_from_expr(arg, out);
                }
            }
            HirExprKind::MethodCall(receiver, _, _, args) => {
                Self::collect_fn_defs_from_expr(receiver, out);
                for arg in args {
                    Self::collect_fn_defs_from_expr(arg, out);
                }
            }
            HirExprKind::Field(base, _)
            | HirExprKind::Deref(base)
            | HirExprKind::Paren(base)
            | HirExprKind::Try(base)
            | HirExprKind::Return(Some(base))
            | HirExprKind::Break(_, Some(base)) => {
                Self::collect_fn_defs_from_expr(base, out);
            }
            HirExprKind::Index(base, index)
            | HirExprKind::Binary(_, base, index)
            | HirExprKind::Assign(base, index)
            | HirExprKind::AssignOp(_, base, index) => {
                Self::collect_fn_defs_from_expr(base, out);
                Self::collect_fn_defs_from_expr(index, out);
            }
            HirExprKind::Unary(_, base)
            | HirExprKind::Ref(base, _)
            | HirExprKind::RawRef(base, _)
            | HirExprKind::Cast(base, _) => {
                Self::collect_fn_defs_from_expr(base, out);
            }
            HirExprKind::Block(block)
            | HirExprKind::Unsafe(block)
            | HirExprKind::Loop(block, _) => {
                Self::collect_fn_defs_from_block(block, out);
            }
            HirExprKind::If(cond, then_block, else_expr) => {
                Self::collect_fn_defs_from_expr(cond, out);
                Self::collect_fn_defs_from_block(then_block, out);
                if let Some(else_expr) = else_expr {
                    Self::collect_fn_defs_from_expr(else_expr, out);
                }
            }
            HirExprKind::Match(scrutinee, arms) => {
                Self::collect_fn_defs_from_expr(scrutinee, out);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        Self::collect_fn_defs_from_expr(guard, out);
                    }
                    Self::collect_fn_defs_from_expr(&arm.body, out);
                }
            }
            HirExprKind::Closure(_, _, body, _) => {
                Self::collect_fn_defs_from_expr(body, out);
            }
            HirExprKind::Struct(_, fields, base) => {
                for field in fields {
                    Self::collect_fn_defs_from_expr(&field.value, out);
                }
                if let Some(base) = base {
                    Self::collect_fn_defs_from_expr(base, out);
                }
            }
            HirExprKind::Tuple(elems) | HirExprKind::Array(elems) => {
                for elem in elems {
                    Self::collect_fn_defs_from_expr(elem, out);
                }
            }
            HirExprKind::ArrayRepeat(value, len) => {
                Self::collect_fn_defs_from_expr(value, out);
                Self::collect_fn_defs_from_expr(len, out);
            }
            HirExprKind::Range(start, end, _) => {
                if let Some(start) = start {
                    Self::collect_fn_defs_from_expr(start, out);
                }
                if let Some(end) = end {
                    Self::collect_fn_defs_from_expr(end, out);
                }
            }
            HirExprKind::For(_, iter, body, _) => {
                Self::collect_fn_defs_from_expr(iter, out);
                Self::collect_fn_defs_from_block(body, out);
            }
            HirExprKind::InlineAsm(asm) => {
                for operand in &asm.operands {
                    match operand {
                        HirAsmOperand::In { expr, .. } | HirAsmOperand::InOut { expr, .. } => {
                            Self::collect_fn_defs_from_expr(expr, out);
                        }
                        HirAsmOperand::Out { expr, .. } => {
                            if let Some(expr) = expr {
                                Self::collect_fn_defs_from_expr(expr, out);
                            }
                        }
                    }
                }
            }
            HirExprKind::Lit(_)
            | HirExprKind::Path(_)
            | HirExprKind::QualifiedPath(_)
            | HirExprKind::Return(None)
            | HirExprKind::Break(_, None)
            | HirExprKind::Continue(_) => {}
        }
    }

    pub fn build_fn(
        interner: &mut Interner,
        resolve: &ResolveResult,
        typeck: &TypeckResult,
        func: &HirFnDef,
    ) -> Vec<MirBody> {
        let mut builder = MirBuilder {
            interner,
            resolve,
            typeck,
            blocks: Vec::new(),
            locals: Vec::new(),
            current_block: BlockId(0),
            var_map: HashMap::new(),
            loop_stack: Vec::new(),
            extra_bodies: Vec::new(),
            closure_counter: 0,
            closure_symbols: HashMap::new(),
        };

        // Create entry block
        builder.push_block();

        // Local _0 = return place
        let ret_ty = func
            .ret_ty
            .as_ref()
            .and_then(|_| typeck.expr_types.values().next()) // We'll derive from fn sig
            .cloned()
            .unwrap_or(TyKind::Unit);
        // Actually get return type properly - look at the fn sig via typeck
        let ret_ty = builder.get_fn_ret_ty(func);
        builder.alloc_local(ret_ty, None, Span::dummy());

        // Params: _1 .. _arg_count
        let arg_count = func.params.len();
        let param_tys = builder
            .typeck
            .fn_sigs
            .get(&func.def_id)
            .map(|(params, _)| params.clone())
            .unwrap_or_default();
        for (i, param) in func.params.iter().enumerate() {
            let ty = param_tys
                .get(i)
                .cloned()
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

        let mut result = builder.extra_bodies;
        result.push(MirBody {
            basic_blocks: builder.blocks,
            locals: builder.locals,
            arg_count,
            name: func.name,
            span: Span::dummy(),
            no_mangle: func.no_mangle,
        });
        result
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
        self.typeck
            .expr_types
            .get(&expr.id)
            .cloned()
            .unwrap_or(TyKind::Error)
    }

    fn enum_variant_constructor_info(
        &self,
        callee: &HirExpr,
        path: &HirPath,
    ) -> Option<(DefId, usize)> {
        let variant_name = path.segments.last()?.ident;

        if let Some(&variant_def_id) = self.resolve.resolutions.get(&callee.id) {
            if let Some(&enum_def_id) = self.resolve.variant_to_enum.get(&variant_def_id) {
                if let Some(idx) =
                    self.typeck
                        .enum_variants
                        .get(&enum_def_id)
                        .and_then(|variants| {
                            variants.iter().position(|(name, _)| *name == variant_name)
                        })
                {
                    return Some((enum_def_id, idx));
                }
            }
        }

        let enum_name = path
            .segments
            .get(path.segments.len().checked_sub(2)?)?
            .ident;
        let variant_idx = self
            .resolve
            .variant_indices
            .get(&(enum_name, variant_name))
            .copied()?;
        match self.get_expr_ty(callee) {
            TyKind::Adt(enum_def_id, _) => Some((enum_def_id, variant_idx)),
            TyKind::FnDef(variant_def_id, _) => self
                .resolve
                .variant_to_enum
                .get(&variant_def_id)
                .copied()
                .map(|enum_def_id| (enum_def_id, variant_idx)),
            _ => None,
        }
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
                    let ty = init
                        .as_ref()
                        .map(|e| self.get_expr_ty(e))
                        .unwrap_or(TyKind::Int(IntTy::I32));
                    if let HirPattern::Ident(pat_hir_id, _, _, _, _) = pat {
                        let name = match pat {
                            HirPattern::Ident(_, sym, _, _, _) => Some(*sym),
                            _ => None,
                        };
                        let local = self.alloc_local(ty, name, *span);
                        if let Some(&def_id) = self.resolve.resolutions.get(pat_hir_id) {
                            self.var_map.insert(def_id, local);
                        }
                        self.emit_stmt(Statement {
                            kind: StatementKind::StorageLive(local),
                            span: *span,
                        });
                        if let Some(init_expr) = init {
                            let op = self.lower_expr(init_expr);
                            self.emit_assign(Place::local(local), Rvalue::Use(op), *span);
                        }
                    } else {
                        let temp = self.alloc_local(ty.clone(), None, *span);
                        self.emit_stmt(Statement {
                            kind: StatementKind::StorageLive(temp),
                            span: *span,
                        });
                        if let Some(init_expr) = init {
                            let op = self.lower_expr(init_expr);
                            self.emit_assign(Place::local(temp), Rvalue::Use(op), *span);
                        }
                        self.bind_let_pattern(pat, Place::local(temp), &ty, *span);
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

    fn bind_let_pattern(&mut self, pat: &HirPattern, source: Place, ty: &TyKind, span: Span) {
        match pat {
            HirPattern::Ident(hir_id, name, _, _, _) => {
                let local = self.alloc_local(ty.clone(), Some(*name), span);
                if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                    self.var_map.insert(def_id, local);
                }
                self.emit_stmt(Statement {
                    kind: StatementKind::StorageLive(local),
                    span,
                });
                self.emit_assign(
                    Place::local(local),
                    Rvalue::Use(Operand::Copy(source)),
                    span,
                );
            }
            HirPattern::Tuple(pats, _) => {
                if let TyKind::Tuple(elem_tys) = ty {
                    for (idx, pat) in pats.iter().enumerate() {
                        let elem_ty = elem_tys
                            .get(idx)
                            .cloned()
                            .unwrap_or(TyKind::Int(IntTy::I32));
                        let mut projections = source.projections.clone();
                        projections.push(Projection::Field(idx));
                        self.bind_let_pattern(
                            pat,
                            Place {
                                local: source.local,
                                projections,
                            },
                            &elem_ty,
                            span,
                        );
                    }
                }
            }
            HirPattern::Slice(pats, _) => {
                let elem_ty = match ty {
                    TyKind::Array(elem_ty, _) | TyKind::Slice(elem_ty) => Some(elem_ty.as_ref()),
                    _ => None,
                };
                if let Some(elem_ty) = elem_ty {
                    for (idx, pat) in pats
                        .iter()
                        .filter(|p| !matches!(p, HirPattern::Rest(_)))
                        .enumerate()
                    {
                        let mut projections = source.projections.clone();
                        projections.push(Projection::Field(idx));
                        self.bind_let_pattern(
                            pat,
                            Place {
                                local: source.local,
                                projections,
                            },
                            elem_ty,
                            span,
                        );
                    }
                }
            }
            HirPattern::Ref(inner, _, _) => {
                if let TyKind::Ref(inner_ty, _) = ty {
                    self.bind_let_pattern(inner, source, inner_ty, span);
                }
            }
            HirPattern::RefBinding(inner, mutability, _) => {
                let ref_ty = TyKind::Ref(Box::new(ty.clone()), *mutability);
                if let HirPattern::Ident(hir_id, name, _, _, _) = inner.as_ref() {
                    let local = self.alloc_local(ref_ty, Some(*name), span);
                    if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                        self.var_map.insert(def_id, local);
                    }
                    self.emit_stmt(Statement {
                        kind: StatementKind::StorageLive(local),
                        span,
                    });
                    let borrow_kind = match mutability {
                        Mutability::Immutable => BorrowKind::Shared,
                        Mutability::Mut => BorrowKind::Mutable,
                    };
                    self.emit_assign(Place::local(local), Rvalue::Ref(borrow_kind, source), span);
                } else {
                    self.bind_let_pattern(inner, source, ty, span);
                }
            }
            HirPattern::Wildcard(_)
            | HirPattern::Rest(_)
            | HirPattern::Literal(_, _)
            | HirPattern::Struct(_, _, _, _)
            | HirPattern::TupleStruct(_, _, _)
            | HirPattern::Or(_, _)
            | HirPattern::Range(_, _, _, _)
            | HirPattern::Path(_) => {}
        }
    }

    /// Lower an expression, returning an Operand for the result
    fn lower_expr(&mut self, expr: &HirExpr) -> Operand {
        match &expr.kind {
            HirExprKind::Lit(lit) => self.lower_literal(lit, expr),

            HirExprKind::Path(path) => self.lower_path(path, expr),

            HirExprKind::QualifiedPath(qpath) => self.lower_path(&qpath.path, expr),

            HirExprKind::Binary(op, lhs, rhs) => {
                let l = self.lower_expr(lhs);
                let r = self.lower_expr(rhs);
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(Place::local(tmp), Rvalue::BinaryOp(*op, l, r), expr.span);
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Unary(op, inner) => {
                let operand = self.lower_expr(inner);
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(Place::local(tmp), Rvalue::UnaryOp(*op, operand), expr.span);
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Call(callee, args) => {
                // Check if this is an enum variant constructor call
                if let HirExprKind::Path(path) = &callee.kind {
                    if let Some((enum_def_id, variant_idx)) =
                        self.enum_variant_constructor_info(callee, path)
                    {
                        if let Some(variants) = self.typeck.enum_variants.get(&enum_def_id) {
                            let max_fields =
                                variants.iter().map(|(_, f)| f.len()).max().unwrap_or(0);
                            if max_fields > 0 {
                                // This is a data enum variant constructor.
                                let arg_ops: Vec<Operand> =
                                    args.iter().map(|a| self.lower_expr(a)).collect();
                                let expr_ty = self.get_expr_ty(expr);
                                let ty = match expr_ty {
                                    TyKind::Adt(_, _) => expr_ty,
                                    _ => TyKind::Adt(enum_def_id, vec![]),
                                };
                                let tmp = self.alloc_temp(ty, expr.span);
                                // Build aggregate: [discriminant, field0, field1, ...]
                                let mut operands = vec![Operand::Constant(Constant {
                                    ty: TyKind::Int(IntTy::I64),
                                    value: ConstValue::Int(variant_idx as i128),
                                })];
                                operands.extend(arg_ops);
                                // Pad with zeros up to max_fields.
                                let variant_fields =
                                    variants.get(variant_idx).map(|(_, f)| f.len()).unwrap_or(0);
                                for _ in variant_fields..max_fields {
                                    operands.push(Operand::Constant(Constant {
                                        ty: TyKind::Int(IntTy::I64),
                                        value: ConstValue::Int(0),
                                    }));
                                }
                                self.emit_assign(
                                    Place::local(tmp),
                                    Rvalue::Aggregate(
                                        AggregateKind::Adt(enum_def_id, variant_idx),
                                        operands,
                                    ),
                                    expr.span,
                                );
                                return Operand::Copy(Place::local(tmp));
                            }
                        }
                    }
                }

                // Check if this is a tuple struct constructor call: PhysAddr(val)
                if let HirExprKind::Path(path) = &callee.kind {
                    let struct_name = path
                        .segments
                        .last()
                        .map(|s| s.ident)
                        .unwrap_or(path.segments[0].ident);
                    if let Some(&struct_def_id) = self
                        .typeck
                        .type_def_to_name
                        .iter()
                        .find(|(_, name)| **name == struct_name)
                        .map(|(did, _)| did)
                    {
                        if let Some(fields) = self.typeck.struct_defs.get(&struct_def_id) {
                            // It's a struct — treat as tuple struct constructor
                            let arg_ops: Vec<Operand> =
                                args.iter().map(|a| self.lower_expr(a)).collect();
                            let ty = TyKind::Adt(struct_def_id, vec![]);
                            let tmp = self.alloc_temp(ty, expr.span);
                            let _ = fields;
                            self.emit_assign(
                                Place::local(tmp),
                                Rvalue::Aggregate(AggregateKind::Adt(struct_def_id, 0), arg_ops),
                                expr.span,
                            );
                            return Operand::Copy(Place::local(tmp));
                        }
                    }
                }

                let func_op = self.lower_expr(callee);
                let expected_param_tys = match &func_op {
                    Operand::Constant(c) => {
                        if let TyKind::FnDef(fn_def_id, _) = &c.ty {
                            self.typeck
                                .fn_sigs
                                .get(fn_def_id)
                                .map(|(params, _)| params.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                let mut arg_ops: Vec<Operand> = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let expected_param =
                        expected_param_tys.as_ref().and_then(|params| params.get(i));
                    let arg_ty = self.get_expr_ty(arg);
                    let arg_op = match expected_param {
                        Some(TyKind::Ref(expected_inner, expected_mut)) => match &arg_ty {
                            TyKind::Ref(actual_inner, actual_mut) => {
                                let place = self.lower_place(arg);
                                if *expected_mut == Mutability::Immutable
                                    && *actual_mut == Mutability::Mut
                                {
                                    let local = match &place {
                                        Place { local, projections } if projections.is_empty() => {
                                            *local
                                        }
                                        _ => {
                                            let tmp = self.alloc_temp(arg_ty.clone(), arg.span);
                                            self.emit_assign(
                                                Place::local(tmp),
                                                Rvalue::Use(Operand::Copy(place)),
                                                arg.span,
                                            );
                                            tmp
                                        }
                                    };
                                    let deref_place = Place {
                                        local,
                                        projections: vec![Projection::Deref],
                                    };
                                    let ref_ty =
                                        TyKind::Ref(expected_inner.clone(), Mutability::Immutable);
                                    let tmp = self.alloc_temp(ref_ty, arg.span);
                                    self.emit_assign(
                                        Place::local(tmp),
                                        Rvalue::Ref(BorrowKind::Shared, deref_place),
                                        arg.span,
                                    );
                                    Operand::Copy(Place::local(tmp))
                                } else {
                                    let local = match &place {
                                        Place { local, projections } if projections.is_empty() => {
                                            *local
                                        }
                                        _ => {
                                            let tmp = self.alloc_temp(arg_ty.clone(), arg.span);
                                            self.emit_assign(
                                                Place::local(tmp),
                                                Rvalue::Use(Operand::Copy(place)),
                                                arg.span,
                                            );
                                            tmp
                                        }
                                    };
                                    Operand::Copy(Place::local(local))
                                }
                            }
                            _ => {
                                let place = self.lower_place(arg);
                                let borrow_kind = if *expected_mut == Mutability::Mut {
                                    BorrowKind::Mutable
                                } else {
                                    BorrowKind::Shared
                                };
                                let tmp = self.alloc_temp(
                                    TyKind::Ref(expected_inner.clone(), *expected_mut),
                                    arg.span,
                                );
                                self.emit_assign(
                                    Place::local(tmp),
                                    Rvalue::Ref(borrow_kind, place),
                                    arg.span,
                                );
                                Operand::Copy(Place::local(tmp))
                            }
                        },
                        _ => self.lower_expr(arg),
                    };
                    arg_ops.push(arg_op);
                }

                // Check for &T -> &dyn Trait coercion at call site
                if let Operand::Constant(c) = &func_op {
                    if let TyKind::FnDef(fn_def_id, _) = &c.ty {
                        if let Some((param_tys, _)) = self.typeck.fn_sigs.get(fn_def_id).cloned() {
                            for (i, pty) in param_tys.iter().enumerate() {
                                if i >= arg_ops.len() {
                                    break;
                                }
                                if let TyKind::Ref(inner, _) = pty {
                                    if let TyKind::DynTrait(trait_def_id) = inner.as_ref() {
                                        let arg_op = arg_ops[i].clone();
                                        let arg_expr_ty = self.get_expr_ty(&args[i]);
                                        let concrete_ty_name = match &arg_expr_ty {
                                            TyKind::Ref(inner, _) => {
                                                if let TyKind::Adt(_, _) = inner.as_ref() {
                                                    self.resolve
                                                        .impl_methods
                                                        .keys()
                                                        .find(|ty_name| {
                                                            self.typeck.trait_impls.contains_key(&(
                                                                **ty_name,
                                                                *trait_def_id,
                                                            ))
                                                        })
                                                        .copied()
                                                } else {
                                                    None
                                                }
                                            }
                                            _ => None,
                                        };
                                        if let Some(type_name) = concrete_ty_name {
                                            // Build vtable: get impl method names in trait method order
                                            let trait_methods = self
                                                .typeck
                                                .trait_methods
                                                .get(trait_def_id)
                                                .cloned()
                                                .unwrap_or_default();
                                            let impl_methods = self
                                                .typeck
                                                .trait_impls
                                                .get(&(type_name, *trait_def_id))
                                                .cloned()
                                                .unwrap_or_default();

                                            let mut vtable_fn_names = Vec::new();
                                            for (method_name, _) in &trait_methods {
                                                // Find matching impl method
                                                if let Some((_, impl_def_id)) = impl_methods
                                                    .iter()
                                                    .find(|(n, _)| n == method_name)
                                                {
                                                    // Get the function symbol name from fn_sigs
                                                    let _ = impl_def_id;
                                                    vtable_fn_names.push(*method_name);
                                                }
                                            }

                                            // Create vtable on stack
                                            let vtable_ty = TyKind::Array(
                                                Box::new(TyKind::RawPtr(
                                                    Box::new(TyKind::Unit),
                                                    Mutability::Immutable,
                                                )),
                                                vtable_fn_names.len(),
                                            );
                                            let vtable_local =
                                                self.alloc_temp(vtable_ty, expr.span);
                                            self.emit_assign(
                                                Place::local(vtable_local),
                                                Rvalue::MakeVtable(vtable_fn_names),
                                                expr.span,
                                            );

                                            // Get pointer to vtable
                                            let vtable_ptr_ty = TyKind::RawPtr(
                                                Box::new(TyKind::Unit),
                                                Mutability::Immutable,
                                            );
                                            let vtable_ptr_local =
                                                self.alloc_temp(vtable_ptr_ty, expr.span);
                                            self.emit_assign(
                                                Place::local(vtable_ptr_local),
                                                Rvalue::Ref(
                                                    BorrowKind::Shared,
                                                    Place::local(vtable_local),
                                                ),
                                                expr.span,
                                            );

                                            // Build fat pointer: (data_ptr, vtable_ptr)
                                            let fat_ptr_ty = TyKind::Ref(
                                                Box::new(TyKind::DynTrait(*trait_def_id)),
                                                Mutability::Immutable,
                                            );
                                            let fat_ptr_local =
                                                self.alloc_temp(fat_ptr_ty, expr.span);
                                            self.emit_assign(
                                                Place::local(fat_ptr_local),
                                                Rvalue::Aggregate(
                                                    AggregateKind::Tuple,
                                                    vec![
                                                        arg_op,
                                                        Operand::Copy(Place::local(
                                                            vtable_ptr_local,
                                                        )),
                                                    ],
                                                ),
                                                expr.span,
                                            );
                                            arg_ops[i] = Operand::Copy(Place::local(fat_ptr_local));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

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
                self.lower_block(block)
                    .unwrap_or(Operand::Constant(Constant {
                        ty: TyKind::Unit,
                        value: ConstValue::Unit,
                    }))
            }

            HirExprKind::Assign(lhs, rhs) => {
                let rhs_op = self.lower_expr(rhs);
                let place = self.lower_place(lhs);
                self.emit_assign(place, Rvalue::Use(rhs_op), expr.span);
                Operand::Constant(Constant {
                    ty: TyKind::Unit,
                    value: ConstValue::Unit,
                })
            }

            HirExprKind::AssignOp(op, lhs, rhs) => {
                let lhs_op = self.lower_expr(lhs);
                let rhs_op = self.lower_expr(rhs);
                let place = self.lower_place(lhs);
                let ty = self.get_expr_ty(lhs);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(
                    Place::local(tmp),
                    Rvalue::BinaryOp(*op, lhs_op, rhs_op),
                    expr.span,
                );
                self.emit_assign(
                    place,
                    Rvalue::Use(Operand::Copy(Place::local(tmp))),
                    expr.span,
                );
                Operand::Constant(Constant {
                    ty: TyKind::Unit,
                    value: ConstValue::Unit,
                })
            }

            HirExprKind::Return(val) => {
                let op = if let Some(v) = val {
                    self.lower_expr(v)
                } else {
                    Operand::Constant(Constant {
                        ty: TyKind::Unit,
                        value: ConstValue::Unit,
                    })
                };
                self.emit_assign(Place::local(Local(0)), Rvalue::Use(op), expr.span);
                self.terminate(Terminator::Return);
                // Create a new unreachable block for any subsequent code
                let unreachable_bb = self.push_block();
                self.current_block = unreachable_bb;
                Operand::Constant(Constant {
                    ty: TyKind::Never,
                    value: ConstValue::Unit,
                })
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
                Operand::Constant(Constant {
                    ty: TyKind::Unit,
                    value: ConstValue::Unit,
                })
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
                Operand::Constant(Constant {
                    ty: TyKind::Never,
                    value: ConstValue::Unit,
                })
            }

            HirExprKind::Continue(_label) => {
                if let Some(&(header, _)) = self.loop_stack.last() {
                    self.terminate(Terminator::Goto(header));
                }
                let unreachable_bb = self.push_block();
                self.current_block = unreachable_bb;
                Operand::Constant(Constant {
                    ty: TyKind::Never,
                    value: ConstValue::Unit,
                })
            }

            HirExprKind::Ref(inner, mutability) => {
                let place = self.lower_place(inner);
                let borrow_kind = match mutability {
                    Mutability::Immutable => BorrowKind::Shared,
                    Mutability::Mut => BorrowKind::Mutable,
                };
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(
                    Place::local(tmp),
                    Rvalue::Ref(borrow_kind, place),
                    expr.span,
                );
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::RawRef(inner, mutability) => {
                let place = self.lower_place(inner);
                let borrow_kind = match mutability {
                    Mutability::Immutable => BorrowKind::Shared,
                    Mutability::Mut => BorrowKind::Mutable,
                };
                let ty = self.get_expr_ty(expr);
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(
                    Place::local(tmp),
                    Rvalue::Ref(borrow_kind, place),
                    expr.span,
                );
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Deref(inner) => {
                let mut place = self.lower_place(inner);
                place.projections.push(Projection::Deref);
                Operand::Copy(place)
            }

            HirExprKind::Struct(path, fields, _base) => {
                let field_ops: Vec<Operand> =
                    fields.iter().map(|f| self.lower_expr(&f.value)).collect();
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
                    return Operand::Constant(Constant {
                        ty: TyKind::Unit,
                        value: ConstValue::Unit,
                    });
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
                let idx_op = self.lower_expr(idx);
                let idx_ty = self.get_expr_ty(idx);
                let idx_tmp = self.alloc_temp(idx_ty, expr.span);
                self.emit_assign(Place::local(idx_tmp), Rvalue::Use(idx_op), expr.span);

                // Try to get a direct place for the base to avoid copying the whole array
                let base_place = self.try_lower_to_place(base);
                if let Some(mut place) = base_place {
                    place.projections.push(Projection::Index(idx_tmp));
                    Operand::Copy(place)
                } else {
                    let base_op = self.lower_expr(base);
                    let base_ty = self.get_expr_ty(base);
                    let base_tmp = self.alloc_temp(base_ty, expr.span);
                    self.emit_assign(Place::local(base_tmp), Rvalue::Use(base_op), expr.span);
                    Operand::Copy(Place {
                        local: base_tmp,
                        projections: vec![Projection::Index(idx_tmp)],
                    })
                }
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
                self.lower_block(block)
                    .unwrap_or(Operand::Constant(Constant {
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

                // Handle .len() on arrays
                let method_str = self.interner.resolve(*method_name);
                if let TyKind::RawPtr(inner, _) = &inner_ty {
                    if method_str == "is_null" && args.is_empty() {
                        let recv_op = self.lower_expr(recv);
                        let ty = self.get_expr_ty(expr);
                        let dest = self.alloc_temp(ty, expr.span);
                        let next_block = self.push_block();
                        let sym = self.interner.intern("is_null");
                        self.terminate(Terminator::Call {
                            func: Operand::Constant(Constant {
                                ty: TyKind::FnDef(DefId(0), vec![]),
                                value: ConstValue::FnItem(sym),
                            }),
                            args: vec![recv_op],
                            dest: Place::local(dest),
                            target: next_block,
                        });
                        self.current_block = next_block;
                        return Operand::Copy(Place::local(dest));
                    }

                    if method_str == "add" && args.len() == 1 {
                        let ptr_op = self.lower_expr(recv);
                        let count_op = self.lower_expr(&args[0]);
                        let usize_ty = TyKind::Uint(UintTy::Usize);

                        let ptr_usize = self.alloc_temp(usize_ty.clone(), expr.span);
                        self.emit_assign(
                            Place::local(ptr_usize),
                            Rvalue::Cast(ptr_op, usize_ty.clone()),
                            expr.span,
                        );

                        let elem_size = self.estimate_ty_size(inner.as_ref()) as u128;
                        let scaled_local = if elem_size > 1 {
                            let count_scaled = self.alloc_temp(usize_ty.clone(), expr.span);
                            self.emit_assign(
                                Place::local(count_scaled),
                                Rvalue::BinaryOp(
                                    BinOp::Mul,
                                    count_op,
                                    Operand::Constant(Constant {
                                        ty: usize_ty.clone(),
                                        value: ConstValue::Uint(elem_size),
                                    }),
                                ),
                                expr.span,
                            );
                            count_scaled
                        } else {
                            let count_local = self.alloc_temp(usize_ty.clone(), expr.span);
                            self.emit_assign(
                                Place::local(count_local),
                                Rvalue::Use(count_op),
                                expr.span,
                            );
                            count_local
                        };

                        let sum_local = self.alloc_temp(usize_ty.clone(), expr.span);
                        self.emit_assign(
                            Place::local(sum_local),
                            Rvalue::BinaryOp(
                                BinOp::Add,
                                Operand::Copy(Place::local(ptr_usize)),
                                Operand::Copy(Place::local(scaled_local)),
                            ),
                            expr.span,
                        );

                        let result_ty = self.get_expr_ty(expr);
                        let result = self.alloc_temp(result_ty.clone(), expr.span);
                        self.emit_assign(
                            Place::local(result),
                            Rvalue::Cast(Operand::Copy(Place::local(sum_local)), result_ty),
                            expr.span,
                        );
                        return Operand::Copy(Place::local(result));
                    }
                }

                if method_str == "len" && args.is_empty() {
                    if let TyKind::Array(_, n) = &inner_ty {
                        let ty = self.get_expr_ty(expr);
                        let tmp = self.alloc_temp(ty, expr.span);
                        self.emit_assign(
                            Place::local(tmp),
                            Rvalue::Use(Operand::Constant(Constant {
                                ty: TyKind::Uint(crate::typeck::UintTy::Usize),
                                value: ConstValue::Uint(*n as u128),
                            })),
                            expr.span,
                        );
                        // Still need to evaluate receiver for side effects
                        let _ = self.lower_expr(recv);
                        return Operand::Copy(Place::local(tmp));
                    }
                }

                // Handle dyn Trait virtual dispatch
                if let TyKind::DynTrait(trait_def_id) = &inner_ty {
                    if let Some(trait_methods) = self.typeck.trait_methods.get(trait_def_id) {
                        let method_index = trait_methods
                            .iter()
                            .position(|(n, _)| *n == *method_name)
                            .unwrap_or(0);

                        // Lower receiver (already a &dyn Trait = fat pointer)
                        let recv_op = self.lower_expr(recv);
                        // Store fat pointer to a local so we can project into it
                        let fat_ptr_ty =
                            TyKind::Ref(Box::new(inner_ty.clone()), Mutability::Immutable);
                        let fat_ptr_local = self.alloc_temp(fat_ptr_ty, expr.span);
                        self.emit_assign(
                            Place::local(fat_ptr_local),
                            Rvalue::Use(recv_op),
                            expr.span,
                        );

                        // Extract data_ptr (field 0) and vtable_ptr (field 1)
                        let data_ptr_local = self.alloc_temp(
                            TyKind::RawPtr(Box::new(TyKind::Unit), Mutability::Immutable),
                            expr.span,
                        );
                        self.emit_assign(
                            Place::local(data_ptr_local),
                            Rvalue::Use(Operand::Copy(Place {
                                local: fat_ptr_local,
                                projections: vec![Projection::Field(0)],
                            })),
                            expr.span,
                        );
                        let vtable_ptr_local = self.alloc_temp(
                            TyKind::RawPtr(Box::new(TyKind::Unit), Mutability::Immutable),
                            expr.span,
                        );
                        self.emit_assign(
                            Place::local(vtable_ptr_local),
                            Rvalue::Use(Operand::Copy(Place {
                                local: fat_ptr_local,
                                projections: vec![Projection::Field(1)],
                            })),
                            expr.span,
                        );

                        // Load fn_ptr from vtable[method_index]
                        // vtable_ptr points to an array of fn ptrs; fn_ptr = *(vtable_ptr + method_index * 8)
                        let fn_ptr_local = self
                            .alloc_temp(TyKind::FnPtr(vec![], Box::new(TyKind::Unit)), expr.span);
                        self.emit_assign(
                            Place::local(fn_ptr_local),
                            Rvalue::Use(Operand::Copy(Place {
                                local: vtable_ptr_local,
                                projections: vec![
                                    Projection::Deref,
                                    Projection::Field(method_index),
                                ],
                            })),
                            expr.span,
                        );

                        // Build args: [data_ptr (as &self), ...user_args]
                        let mut all_args = vec![Operand::Copy(Place::local(data_ptr_local))];
                        for a in args {
                            all_args.push(self.lower_expr(a));
                        }

                        let ty = self.get_expr_ty(expr);
                        let dest = self.alloc_temp(ty, expr.span);
                        let next_block = self.push_block();
                        self.terminate(Terminator::Call {
                            func: Operand::Copy(Place::local(fn_ptr_local)),
                            args: all_args,
                            dest: Place::local(dest),
                            target: next_block,
                        });
                        self.current_block = next_block;
                        return Operand::Copy(Place::local(dest));
                    }
                }

                let method_def_id = if let TyKind::Adt(def_id, _) = &inner_ty {
                    // Look up type name from DefId, then find method in impl_methods
                    self.typeck
                        .type_def_to_name
                        .get(def_id)
                        .and_then(|type_name| self.resolve.impl_methods.get(type_name))
                        .and_then(|methods| {
                            methods
                                .iter()
                                .find(|(n, _)| *n == *method_name)
                                .map(|(_, did)| *did)
                        })
                } else {
                    None
                };

                if let Some(method_did) = method_def_id {
                    // Get method name symbol
                    let fn_name = *method_name;

                    // Check what self parameter the method expects
                    let method_self_param = self
                        .typeck
                        .fn_sigs
                        .get(&method_did)
                        .and_then(|(params, _)| params.first().cloned());
                    let self_is_ref = method_self_param
                        .as_ref()
                        .map(|p| matches!(p, TyKind::Ref(_, _)))
                        .unwrap_or(true);
                    let method_wants_mut = match &method_self_param {
                        Some(TyKind::Ref(_, Mutability::Mut)) => true,
                        _ => false,
                    };

                    // Build receiver operand
                    let recv_op = if self_is_ref {
                        match &recv_ty {
                            TyKind::Ref(inner, recv_mut) => {
                                // Receiver is already a reference
                                let place = self.lower_place(recv);
                                if method_wants_mut || *recv_mut == Mutability::Immutable {
                                    // Same mutability or method wants &mut and we have &mut — just copy the ref
                                    let local = match &place {
                                        Place { local, projections } if projections.is_empty() => {
                                            *local
                                        }
                                        _ => {
                                            let tmp = self.alloc_temp(recv_ty.clone(), expr.span);
                                            self.emit_assign(
                                                Place::local(tmp),
                                                Rvalue::Use(Operand::Copy(place)),
                                                expr.span,
                                            );
                                            tmp
                                        }
                                    };
                                    Operand::Copy(Place::local(local))
                                } else {
                                    // Method wants &self but we have &mut self — reborrow as shared
                                    let deref_place = Place {
                                        local: match &place {
                                            Place { local, projections }
                                                if projections.is_empty() =>
                                            {
                                                *local
                                            }
                                            _ => {
                                                let tmp =
                                                    self.alloc_temp(recv_ty.clone(), expr.span);
                                                self.emit_assign(
                                                    Place::local(tmp),
                                                    Rvalue::Use(Operand::Copy(place)),
                                                    expr.span,
                                                );
                                                tmp
                                            }
                                        },
                                        projections: vec![Projection::Deref],
                                    };
                                    let ref_ty = TyKind::Ref(inner.clone(), Mutability::Immutable);
                                    let tmp = self.alloc_temp(ref_ty, expr.span);
                                    self.emit_assign(
                                        Place::local(tmp),
                                        Rvalue::Ref(BorrowKind::Shared, deref_place),
                                        expr.span,
                                    );
                                    Operand::Copy(Place::local(tmp))
                                }
                            }
                            _ => {
                                // Need to take a reference: &recv or &mut recv
                                let place = self.lower_place(recv);
                                let mutbl = if method_wants_mut {
                                    Mutability::Mut
                                } else {
                                    Mutability::Immutable
                                };
                                let bk = if method_wants_mut {
                                    BorrowKind::Mutable
                                } else {
                                    BorrowKind::Shared
                                };
                                let ref_ty = TyKind::Ref(Box::new(recv_ty.clone()), mutbl);
                                let tmp = self.alloc_temp(ref_ty, expr.span);
                                self.emit_assign(
                                    Place::local(tmp),
                                    Rvalue::Ref(bk, place),
                                    expr.span,
                                );
                                Operand::Copy(Place::local(tmp))
                            }
                        }
                    } else {
                        // self by value - pass the value directly
                        self.lower_expr(recv)
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
                } else if let Some((fn_name, method_wants_mut)) =
                    self.known_alloc_method_call(&inner_ty, *method_name)
                {
                    let recv_op = match &recv_ty {
                        TyKind::Ref(_, _) => {
                            let place = self.lower_place(recv);
                            let local = match &place {
                                Place { local, projections } if projections.is_empty() => *local,
                                _ => {
                                    let tmp = self.alloc_temp(recv_ty.clone(), expr.span);
                                    self.emit_assign(
                                        Place::local(tmp),
                                        Rvalue::Use(Operand::Copy(place)),
                                        expr.span,
                                    );
                                    tmp
                                }
                            };
                            Operand::Copy(Place::local(local))
                        }
                        _ => {
                            let place = self.lower_place(recv);
                            let mutbl = if method_wants_mut {
                                Mutability::Mut
                            } else {
                                Mutability::Immutable
                            };
                            let borrow = if method_wants_mut {
                                BorrowKind::Mutable
                            } else {
                                BorrowKind::Shared
                            };
                            let ref_ty = TyKind::Ref(Box::new(recv_ty.clone()), mutbl);
                            let tmp = self.alloc_temp(ref_ty, expr.span);
                            self.emit_assign(
                                Place::local(tmp),
                                Rvalue::Ref(borrow, place),
                                expr.span,
                            );
                            Operand::Copy(Place::local(tmp))
                        }
                    };

                    let mut all_args = vec![recv_op];
                    for a in args {
                        all_args.push(self.lower_expr(a));
                    }

                    let ty = self.get_expr_ty(expr);
                    let dest = self.alloc_temp(ty, expr.span);
                    let next_block = self.push_block();
                    let fn_sym = self.interner.intern(&fn_name);
                    self.terminate(Terminator::Call {
                        func: Operand::Constant(Constant {
                            ty: TyKind::FnDef(DefId(0), vec![]),
                            value: ConstValue::FnItem(fn_sym),
                        }),
                        args: all_args,
                        dest: Place::local(dest),
                        target: next_block,
                    });
                    self.current_block = next_block;
                    Operand::Copy(Place::local(dest))
                } else if let Some((fn_name, method_wants_mut)) =
                    self.fallback_alloc_method_call(method_str, args)
                {
                    let recv_op = match &recv_ty {
                        TyKind::Ref(_, _) => {
                            let place = self.lower_place(recv);
                            let local = match &place {
                                Place { local, projections } if projections.is_empty() => *local,
                                _ => {
                                    let tmp = self.alloc_temp(recv_ty.clone(), expr.span);
                                    self.emit_assign(
                                        Place::local(tmp),
                                        Rvalue::Use(Operand::Copy(place)),
                                        expr.span,
                                    );
                                    tmp
                                }
                            };
                            Operand::Copy(Place::local(local))
                        }
                        _ => {
                            let place = self.lower_place(recv);
                            let mutbl = if method_wants_mut {
                                Mutability::Mut
                            } else {
                                Mutability::Immutable
                            };
                            let borrow = if method_wants_mut {
                                BorrowKind::Mutable
                            } else {
                                BorrowKind::Shared
                            };
                            let ref_ty = TyKind::Ref(Box::new(recv_ty.clone()), mutbl);
                            let tmp = self.alloc_temp(ref_ty, expr.span);
                            self.emit_assign(
                                Place::local(tmp),
                                Rvalue::Ref(borrow, place),
                                expr.span,
                            );
                            Operand::Copy(Place::local(tmp))
                        }
                    };

                    let mut all_args = vec![recv_op];
                    for a in args {
                        all_args.push(self.lower_expr(a));
                    }

                    let ty = self.get_expr_ty(expr);
                    let dest = self.alloc_temp(ty, expr.span);
                    let next_block = self.push_block();
                    let fn_sym = self.interner.intern(&fn_name);
                    self.terminate(Terminator::Call {
                        func: Operand::Constant(Constant {
                            ty: TyKind::FnDef(DefId(0), vec![]),
                            value: ConstValue::FnItem(fn_sym),
                        }),
                        args: all_args,
                        dest: Place::local(dest),
                        target: next_block,
                    });
                    self.current_block = next_block;
                    Operand::Copy(Place::local(dest))
                } else if let TyKind::Adt(def_id, _) = &inner_ty {
                    // Check if receiver is an intrinsic type (AtomicBool, etc.)
                    if self.resolve.intrinsic_fns.contains_key(def_id) {
                        let method_str = self.interner.resolve(*method_name).to_string();
                        // Find the intrinsic type name from its path
                        let type_path = self.resolve.intrinsic_fns.get(def_id).unwrap();
                        let type_name = type_path.rsplit("::").next().unwrap_or(type_path);
                        // Create a descriptive intrinsic name like "AtomicBool::load"
                        let intrinsic_name = format!("{}::{}", type_name, method_str);
                        let sym = self.interner.intern(&intrinsic_name);

                        // Build receiver as pointer (take address of self)
                        // Use Shared borrow to avoid borrow checker conflicts on atomics
                        let recv_op = {
                            let place = self.lower_place(recv);
                            let ref_ty =
                                TyKind::Ref(Box::new(recv_ty.clone()), Mutability::Immutable);
                            let tmp = self.alloc_temp(ref_ty, expr.span);
                            self.emit_assign(
                                Place::local(tmp),
                                Rvalue::Ref(BorrowKind::Shared, place),
                                expr.span,
                            );
                            Operand::Copy(Place::local(tmp))
                        };

                        let mut all_args = vec![recv_op];
                        for a in args {
                            all_args.push(self.lower_expr(a));
                        }

                        let ty = self.get_expr_ty(expr);
                        let dest = self.alloc_temp(ty, expr.span);
                        let next_block = self.push_block();
                        let fn_def_id = *def_id; // use the type's DefId; codegen uses sym name
                        self.terminate(Terminator::Call {
                            func: Operand::Constant(Constant {
                                ty: TyKind::FnDef(fn_def_id, vec![]),
                                value: ConstValue::FnItem(sym),
                            }),
                            args: all_args,
                            dest: Place::local(dest),
                            target: next_block,
                        });
                        self.current_block = next_block;
                        Operand::Copy(Place::local(dest))
                    } else {
                        // Fallback
                        Operand::Constant(Constant {
                            ty: TyKind::Unit,
                            value: ConstValue::Unit,
                        })
                    }
                } else {
                    // Fallback
                    Operand::Constant(Constant {
                        ty: TyKind::Unit,
                        value: ConstValue::Unit,
                    })
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
                    self.emit_assign(
                        Place::local(disc_tmp),
                        Rvalue::Discriminant(Place::local(scr_tmp)),
                        expr.span,
                    );
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
                            self.emit_assign(
                                Place::local(ge_tmp),
                                Rvalue::BinaryOp(BinOp::Ge, disc_op.clone(), lo_op),
                                *span,
                            );
                            // le_tmp = disc <= hi
                            let le_tmp = self.alloc_temp(TyKind::Bool, *span);
                            self.emit_assign(
                                Place::local(le_tmp),
                                Rvalue::BinaryOp(BinOp::Le, disc_op.clone(), hi_op),
                                *span,
                            );
                            // in_range = ge && le
                            let in_range = self.alloc_temp(TyKind::Bool, *span);
                            self.emit_assign(
                                Place::local(in_range),
                                Rvalue::BinaryOp(
                                    BinOp::And,
                                    Operand::Copy(Place::local(ge_tmp)),
                                    Operand::Copy(Place::local(le_tmp)),
                                ),
                                *span,
                            );
                            Operand::Copy(Place::local(in_range))
                        } else {
                            Operand::Constant(Constant {
                                ty: TyKind::Bool,
                                value: ConstValue::Bool(true),
                            })
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
                                        let local =
                                            self.alloc_local(field_ty, Some(*name), expr.span);
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

                    // Bind inner pattern variables for Struct patterns: `Foo { x, y }`
                    if let HirPattern::Struct(path, field_pats, _, _) = &arm.pat {
                        if let Some(scr_local) = scr_local {
                            let scrutinee_ty = self.get_expr_ty(scrutinee);
                            // Look up the struct's field ordering
                            let field_order = self.resolve_struct_fields(&scrutinee_ty);
                            for field_pat in field_pats {
                                if let HirPattern::Ident(hir_id, name, _, _, _) = &field_pat.pat {
                                    if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                                        // Find this field's index in the struct
                                        let field_idx = field_order
                                            .iter()
                                            .position(|(fname, _)| *fname == field_pat.name)
                                            .unwrap_or(0);
                                        let field_ty = field_order
                                            .get(field_idx)
                                            .map(|(_, ty)| ty.clone())
                                            .unwrap_or(TyKind::Int(IntTy::I64));
                                        let local =
                                            self.alloc_local(field_ty, Some(*name), expr.span);
                                        self.var_map.insert(def_id, local);
                                        let src_place = Place {
                                            local: scr_local,
                                            projections: vec![Projection::Field(field_idx)],
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

                    // Bind Ident pattern (simple binding of whole scrutinee)
                    if let HirPattern::Ident(hir_id, name, _, _, _) = &arm.pat {
                        if let Some(scr_local) = scr_local {
                            if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                                let scrutinee_ty = self.get_expr_ty(scrutinee);
                                let local = self.alloc_local(scrutinee_ty, Some(*name), expr.span);
                                self.var_map.insert(def_id, local);
                                self.emit_assign(
                                    Place::local(local),
                                    Rvalue::Use(Operand::Copy(Place::local(scr_local))),
                                    expr.span,
                                );
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
                        if let Terminator::SwitchInt {
                            ref mut default, ..
                        } = self.blocks[check_bb.0].terminator
                        {
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
                                self.try_lower_to_place(e).unwrap_or_else(|| {
                                    Place::local(self.alloc_temp(
                                        TyKind::Uint(crate::typeck::UintTy::U64),
                                        e.span,
                                    ))
                                })
                            });
                            mir_operands.push(MirAsmOperand {
                                kind: MirAsmOperandKind::Out(place),
                                reg: match reg {
                                    HirAsmReg::Named(s) => MirAsmReg::Named(s.clone()),
                                    HirAsmReg::Class(s) => MirAsmReg::Class(s.clone()),
                                },
                            });
                        }
                        HirAsmOperand::InOut {
                            reg,
                            expr,
                            out_expr,
                        } => {
                            let operand = self.lower_expr(expr);
                            let place = out_expr
                                .as_ref()
                                .and_then(|expr| self.try_lower_to_place(expr))
                                .or_else(|| self.try_lower_to_place(expr));
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
                Operand::Constant(Constant {
                    ty: TyKind::Unit,
                    value: ConstValue::Unit,
                })
            }

            // Catch-all for unhandled cases
            HirExprKind::Closure(params, ret_ty, body, _) => {
                // Look up the closure's synthetic DefId from typeck
                let closure_def_id = self.typeck.closure_defs.get(&expr.id).copied();

                // Generate unique closure function name
                let closure_name_str = format!("__closure_{}", self.closure_counter);
                self.closure_counter += 1;
                let closure_sym = self.interner.intern(&closure_name_str);

                // Get the function signature from typeck
                let (param_tys, ret_type) = if let Some(def_id) = closure_def_id {
                    self.typeck
                        .fn_sigs
                        .get(&def_id)
                        .cloned()
                        .unwrap_or_else(|| (vec![], TyKind::Unit))
                } else {
                    (vec![], TyKind::Unit)
                };

                // Build a separate MirBody for the closure
                let mut closure_blocks: Vec<BasicBlock> = Vec::new();
                let mut closure_locals: Vec<LocalDecl> = Vec::new();

                // _0 = return place
                closure_locals.push(LocalDecl {
                    ty: ret_type,
                    mutability: Mutability::Immutable,
                    name: None,
                    span: Span::dummy(),
                });

                // Save current builder state
                let saved_blocks = core::mem::take(&mut self.blocks);
                let saved_locals = core::mem::take(&mut self.locals);
                let saved_current_block = self.current_block;
                let saved_var_map = core::mem::take(&mut self.var_map);

                self.blocks = Vec::new();
                self.locals = Vec::new();
                let entry_block = self.push_block();
                self.current_block = entry_block;

                // _0 = return place
                let ret_ty_resolved = if let Some(def_id) = closure_def_id {
                    self.typeck
                        .fn_sigs
                        .get(&def_id)
                        .map(|(_, ret)| ret.clone())
                        .unwrap_or(TyKind::Unit)
                } else {
                    TyKind::Unit
                };
                self.alloc_local(ret_ty_resolved, None, Span::dummy());

                // Params
                let arg_count = params.len();
                for (i, param) in params.iter().enumerate() {
                    let ty = param_tys.get(i).cloned().unwrap_or(TyKind::Int(IntTy::I32));
                    let name = match &param.pat {
                        HirPattern::Ident(_, sym, _, _, _) => Some(*sym),
                        _ => None,
                    };
                    let local = self.alloc_local(ty, name, param.span);
                    if let HirPattern::Ident(hir_id, _, _, _, _) = &param.pat {
                        if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                            self.var_map.insert(def_id, local);
                        }
                    }
                }

                // Lower the closure body
                let result = self.lower_expr(body);
                let ret_place = Place::local(Local(0));
                self.emit_assign(ret_place, Rvalue::Use(result), expr.span);
                self.terminate(Terminator::Return);

                // Extract the built closure body
                closure_blocks = core::mem::take(&mut self.blocks);
                closure_locals = core::mem::take(&mut self.locals);

                // Restore builder state
                self.blocks = saved_blocks;
                self.locals = saved_locals;
                self.current_block = saved_current_block;
                self.var_map = saved_var_map;

                let closure_body = MirBody {
                    basic_blocks: closure_blocks,
                    locals: closure_locals,
                    arg_count,
                    name: closure_sym,
                    span: expr.span,
                    no_mangle: false,
                };
                self.extra_bodies.push(closure_body);
                if let Some(def_id) = closure_def_id {
                    self.closure_symbols.insert(def_id, closure_sym);
                }

                // Return a FnItem constant pointing to the closure function
                let fn_ty = if let Some(def_id) = closure_def_id {
                    TyKind::FnDef(def_id, vec![])
                } else {
                    TyKind::Unit
                };
                Operand::Constant(Constant {
                    ty: fn_ty,
                    value: ConstValue::FnItem(closure_sym),
                })
            }

            HirExprKind::For(pat, iter, body, _label) => {
                // Desugar `for pat in start..end { body }` to:
                //   let mut __i = start;
                //   loop { if __i >= end { break; } let pat = __i; body; __i += 1; }
                match &iter.kind {
                    HirExprKind::Range(Some(start), Some(end), _inclusive) => {
                        let start_op = self.lower_expr(start);
                        let end_op = self.lower_expr(end);

                        // Determine element type from start expression
                        let elem_ty = self.get_expr_ty(start);

                        // Allocate the loop counter variable
                        let counter_name = match pat {
                            HirPattern::Ident(_, sym, _, _, _) => Some(*sym),
                            _ => None,
                        };
                        let counter = self.alloc_local(elem_ty.clone(), counter_name, expr.span);
                        self.locals[counter.0].mutability = Mutability::Mut;

                        // Map the pattern's DefId to this local
                        if let HirPattern::Ident(hir_id, _, _, _, _) = pat {
                            if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                                self.var_map.insert(def_id, counter);
                            }
                        }

                        // Initialize counter = start
                        self.emit_assign(Place::local(counter), Rvalue::Use(start_op), expr.span);

                        // Store end value in a temp
                        let end_tmp = self.alloc_temp(elem_ty.clone(), expr.span);
                        self.emit_assign(Place::local(end_tmp), Rvalue::Use(end_op), expr.span);

                        // Loop header: compare counter < end
                        let loop_header = self.push_block();
                        let loop_body = self.push_block();
                        let loop_exit = self.push_block();
                        self.terminate(Terminator::Goto(loop_header));
                        self.current_block = loop_header;

                        // cond = counter < end
                        let cond_tmp = self.alloc_temp(TyKind::Bool, expr.span);
                        self.emit_assign(
                            Place::local(cond_tmp),
                            Rvalue::BinaryOp(
                                BinOp::Lt,
                                Operand::Copy(Place::local(counter)),
                                Operand::Copy(Place::local(end_tmp)),
                            ),
                            expr.span,
                        );

                        // SwitchInt: if cond == 0 goto exit, else goto body
                        self.terminate(Terminator::SwitchInt {
                            operand: Operand::Copy(Place::local(cond_tmp)),
                            targets: vec![(0, loop_exit)],
                            default: loop_body,
                        });

                        // Body block
                        self.current_block = loop_body;
                        self.loop_stack.push((loop_header, loop_exit));
                        let _ = self.lower_block(body);
                        self.loop_stack.pop();

                        // Increment counter: counter = counter + 1
                        let one = self.alloc_temp(elem_ty.clone(), expr.span);
                        self.emit_assign(
                            Place::local(one),
                            Rvalue::Use(Operand::Constant(Constant {
                                ty: elem_ty,
                                value: ConstValue::Int(1),
                            })),
                            expr.span,
                        );
                        let inc_tmp = self.alloc_temp(self.locals[counter.0].ty.clone(), expr.span);
                        self.emit_assign(
                            Place::local(inc_tmp),
                            Rvalue::BinaryOp(
                                BinOp::Add,
                                Operand::Copy(Place::local(counter)),
                                Operand::Copy(Place::local(one)),
                            ),
                            expr.span,
                        );
                        self.emit_assign(
                            Place::local(counter),
                            Rvalue::Use(Operand::Copy(Place::local(inc_tmp))),
                            expr.span,
                        );

                        // Back edge
                        self.terminate(Terminator::Goto(loop_header));

                        self.current_block = loop_exit;
                    }
                    _ => {
                        // Iterator-based for loop:
                        // Desugar `for pat in iterable { body }` to:
                        //   let mut __iter = iterable.into_iter();  // or just iterable
                        //   loop {
                        //     match __iter.next() {
                        //       Some(val) => { let pat = val; body; }
                        //       None => break;
                        //     }
                        //   }
                        let iter_op = self.lower_expr(iter);
                        let iter_ty = self.get_expr_ty(iter);

                        // Store iterator in a local
                        let iter_local = self.alloc_temp(iter_ty.clone(), expr.span);
                        self.locals[iter_local.0].mutability = Mutability::Mut;
                        self.emit_assign(Place::local(iter_local), Rvalue::Use(iter_op), expr.span);

                        // Determine element type from the pattern or infer as i64 fallback
                        let elem_ty = self.get_expr_ty(expr);

                        let loop_header = self.push_block();
                        let loop_body = self.push_block();
                        let loop_exit = self.push_block();
                        self.terminate(Terminator::Goto(loop_header));

                        // Loop header: call .next() on iterator
                        self.current_block = loop_header;

                        // Call next() — result is an Option-like enum (discriminant + value)
                        let next_result_ty =
                            TyKind::Tuple(vec![TyKind::Int(IntTy::I64), elem_ty.clone()]);
                        let next_result = self.alloc_temp(next_result_ty.clone(), expr.span);

                        // Emit method call: __iter.next()
                        let next_sym = self.interner.intern("next");
                        let next_block = self.push_block();
                        self.terminate(Terminator::Call {
                            func: Operand::Constant(Constant {
                                ty: TyKind::FnDef(DefId(0), vec![]),
                                value: ConstValue::MethodRef(next_sym),
                            }),
                            args: vec![Operand::Ref(Place::local(iter_local), Mutability::Mut)],
                            dest: Place::local(next_result),
                            target: next_block,
                        });
                        self.current_block = next_block;

                        // Read discriminant of the Option result
                        let disc = self.alloc_temp(TyKind::Int(IntTy::I64), expr.span);
                        self.emit_assign(
                            Place::local(disc),
                            Rvalue::Discriminant(Place::local(next_result)),
                            expr.span,
                        );

                        // SwitchInt: 0 = Some (has value), 1 = None (done)
                        self.terminate(Terminator::SwitchInt {
                            operand: Operand::Copy(Place::local(disc)),
                            targets: vec![(0, loop_body)],
                            default: loop_exit,
                        });

                        // Body block: extract value from Some, bind to pattern
                        self.current_block = loop_body;
                        self.loop_stack.push((loop_header, loop_exit));

                        // Extract field 1 (the value inside Some)
                        if let HirPattern::Ident(hir_id, sym, _, _, _) = pat {
                            let local = self.alloc_local(elem_ty.clone(), Some(*sym), expr.span);
                            if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                                self.var_map.insert(def_id, local);
                            }
                            self.emit_assign(
                                Place::local(local),
                                Rvalue::Use(Operand::Copy(Place {
                                    local: next_result,
                                    projections: vec![Projection::Field(1)],
                                })),
                                expr.span,
                            );
                        }

                        let _ = self.lower_block(body);
                        self.loop_stack.pop();

                        // Back edge
                        self.terminate(Terminator::Goto(loop_header));
                        self.current_block = loop_exit;
                    }
                }
                Operand::Constant(Constant {
                    ty: TyKind::Unit,
                    value: ConstValue::Unit,
                })
            }

            HirExprKind::ArrayRepeat(val, _count) => {
                let ty = self.get_expr_ty(expr);
                let n = if let TyKind::Array(_, n) = &ty {
                    if *n > (1usize << 30) {
                        0
                    } else {
                        *n
                    }
                } else {
                    0
                };
                let val_op = self.lower_expr(val);
                let ops: Vec<Operand> = (0..n).map(|_| val_op.clone()).collect();
                let tmp = self.alloc_temp(ty, expr.span);
                self.emit_assign(
                    Place::local(tmp),
                    Rvalue::Aggregate(AggregateKind::Array, ops),
                    expr.span,
                );
                Operand::Copy(Place::local(tmp))
            }

            HirExprKind::Range(_, _, _) => Operand::Constant(Constant {
                ty: TyKind::Unit,
                value: ConstValue::Unit,
            }),

            HirExprKind::Try(inner) => {
                // Desugar `expr?` to:
                //   match expr {
                //     Ok(val) / Some(val) => val,
                //     Err(e) => return Err(e),  / None => return None,
                //   }
                let inner_op = self.lower_expr(inner);
                let inner_ty = self.get_expr_ty(inner);

                // Store the scrutinee in a temp
                let scrutinee = self.alloc_temp(inner_ty.clone(), expr.span);
                self.emit_assign(Place::local(scrutinee), Rvalue::Use(inner_op), expr.span);

                // Read discriminant (field 0 of the enum)
                let disc = self.alloc_temp(TyKind::Int(IntTy::I64), expr.span);
                self.emit_assign(
                    Place::local(disc),
                    Rvalue::Discriminant(Place::local(scrutinee)),
                    expr.span,
                );

                let ok_block = self.push_block();
                let err_block = self.push_block();
                let after_block = self.push_block();

                // SwitchInt: discriminant 0 = first variant (Ok/Some), 1 = second (Err/None)
                self.terminate(Terminator::SwitchInt {
                    operand: Operand::Copy(Place::local(disc)),
                    targets: vec![(0, ok_block)],
                    default: err_block,
                });

                // Ok/Some block: extract value (field 1)
                self.current_block = ok_block;
                let result_ty = self.get_expr_ty(expr);
                let val = self.alloc_temp(result_ty.clone(), expr.span);
                self.emit_assign(
                    Place::local(val),
                    Rvalue::Use(Operand::Copy(Place {
                        local: scrutinee,
                        projections: vec![Projection::Field(1)],
                    })),
                    expr.span,
                );
                self.terminate(Terminator::Goto(after_block));

                // Err/None block: early return with the error variant
                self.current_block = err_block;
                // Return the scrutinee as-is (it's already the Err/None variant)
                let ret_place = Place::local(Local(0)); // _0 = return place
                self.emit_assign(
                    ret_place,
                    Rvalue::Use(Operand::Copy(Place::local(scrutinee))),
                    expr.span,
                );
                self.terminate(Terminator::Return);

                self.current_block = after_block;
                Operand::Copy(Place::local(val))
            }
        }
    }

    /// Try to get a Place for an expression without lowering it (no side effects).
    /// Returns None if the expression is not directly addressable.
    fn try_lower_to_place(&self, expr: &HirExpr) -> Option<Place> {
        match &expr.kind {
            HirExprKind::Path(path) => self
                .resolve_path_to_local(path, expr.id)
                .map(|local| Place::local(local)),
            HirExprKind::QualifiedPath(qpath) => self
                .resolve_path_to_local(&qpath.path, expr.id)
                .map(|local| Place::local(local)),
            HirExprKind::Field(base, field_name) => {
                let mut place = self.try_lower_to_place(base)?;
                let base_ty = self.get_expr_ty(base);
                if matches!(&base_ty, TyKind::Ref(_, _) | TyKind::RawPtr(_, _)) {
                    place.projections.push(Projection::Deref);
                }
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
                        let tmp = self.alloc_temp(
                            TyKind::RawPtr(Box::new(ty.clone()), crate::ast::Mutability::Mut),
                            expr.span,
                        );
                        self.emit_assign(
                            Place::local(tmp),
                            Rvalue::Use(Operand::Constant(Constant {
                                ty: TyKind::RawPtr(
                                    Box::new(ty.clone()),
                                    crate::ast::Mutability::Mut,
                                ),
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
                let base_ty = self.get_expr_ty(base);
                if matches!(&base_ty, TyKind::Ref(_, _) | TyKind::RawPtr(_, _)) {
                    place.projections.push(Projection::Deref);
                }
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
                return Operand::Constant(Constant {
                    ty: ty.clone(),
                    value: val,
                });
            }
            // Check if it resolves to a primitive associated constant (e.g. u32::MAX)
            if let Some(path_str) = self.resolve.intrinsic_fns.get(&def_id).cloned() {
                if let Some((val, ty)) = Self::primitive_assoc_const_value(&path_str) {
                    return Operand::Constant(Constant { ty, value: val });
                }
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
            // If the local holds a FnDef (e.g., a closure), return the FnItem constant
            // so that codegen can emit a direct call.
            if let TyKind::FnDef(def_id, _) = &ty {
                if let Some(&sym) = self.closure_symbols.get(def_id) {
                    return Operand::Constant(Constant {
                        ty: ty.clone(),
                        value: ConstValue::FnItem(sym),
                    });
                }
            }
            if self.is_copy_type(&ty) {
                Operand::Copy(Place::local(local))
            } else {
                Operand::Move(Place::local(local))
            }
        } else {
            let ty = self.get_expr_ty(expr);
            match &ty {
                TyKind::FnDef(def_id, _) => {
                    // For intrinsic fns, use the full path as the symbol name
                    // so codegen can distinguish e.g. "AtomicBool::new" from a regular "new"
                    if let Some(intrinsic_path) = self.resolve.intrinsic_fns.get(def_id).cloned() {
                        // Ordering::* variants → integer constants (ignored on x86)
                        if intrinsic_path.starts_with("Ordering::") {
                            let val = match intrinsic_path.as_str() {
                                "Ordering::Relaxed" => 0i128,
                                "Ordering::Release" => 1,
                                "Ordering::Acquire" => 2,
                                "Ordering::AcqRel" => 3,
                                "Ordering::SeqCst" => 4,
                                _ => 0,
                            };
                            return Operand::Constant(Constant {
                                ty: TyKind::Int(IntTy::I32),
                                value: ConstValue::Int(val),
                            });
                        }
                        // Keep full names where the final segment is too generic
                        // or where codegen needs the owning primitive/ADT.
                        let fn_name = if intrinsic_path.contains("Atomic")
                            || intrinsic_path.starts_with("anyos_std::")
                            || Self::is_primitive_assoc_fn_path(&intrinsic_path)
                        {
                            self.interner.intern(&intrinsic_path)
                        } else {
                            path.segments.last().unwrap().ident
                        };
                        Operand::Constant(Constant {
                            ty: ty.clone(),
                            value: ConstValue::FnItem(fn_name),
                        })
                    } else {
                        let fn_name = path.segments.last().unwrap().ident;
                        Operand::Constant(Constant {
                            ty: ty.clone(),
                            value: ConstValue::FnItem(fn_name),
                        })
                    }
                }
                TyKind::Adt(ref enum_def_id, _) if path.segments.len() == 2 => {
                    let enum_def_id = *enum_def_id;
                    // Enum variant: look up discriminant
                    let enum_name = path.segments[0].ident;
                    let variant_name = path.segments[1].ident;
                    if let Some(&idx) = self.resolve.variant_indices.get(&(enum_name, variant_name))
                    {
                        // Check if this is a data enum (has variants with fields)
                        if let Some(variants) = self.typeck.enum_variants.get(&enum_def_id) {
                            let max_fields =
                                variants.iter().map(|(_, f)| f.len()).max().unwrap_or(0);
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
                                    Rvalue::Aggregate(
                                        AggregateKind::Adt(enum_def_id, idx),
                                        operands,
                                    ),
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
                        Operand::Constant(Constant {
                            ty,
                            value: ConstValue::Unit,
                        })
                    }
                }
                _ => Operand::Constant(Constant {
                    ty,
                    value: ConstValue::Unit,
                }),
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
            TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => inner.as_ref(),
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

    fn known_alloc_method_call(
        &self,
        receiver_ty: &TyKind,
        method_name: Symbol,
    ) -> Option<(String, bool)> {
        let TyKind::Adt(def_id, _) = receiver_ty else {
            return None;
        };
        let type_sym = self
            .typeck
            .type_def_to_name
            .get(def_id)
            .copied()
            .or_else(|| {
                self.typeck
                    .struct_defs
                    .get(def_id)
                    .and_then(|fields| fields.first().map(|(name, _)| *name))
            })?;
        let type_name = self.interner.resolve(type_sym);
        let method = self.interner.resolve(method_name);
        let full_name = match type_name {
            "String" => match method {
                "len" | "is_empty" | "as_str" | "as_bytes" | "clear" | "push_str" | "push" => {
                    format!("String::{}", method)
                }
                _ => return None,
            },
            "Vec" => match method {
                "new" | "with_capacity" | "len" | "capacity" | "is_empty" | "as_ptr"
                | "as_slice" | "as_ref" | "push" | "pop" | "clear" => {
                    format!("Vec::{}", method)
                }
                _ => return None,
            },
            "VecDeque" => match method {
                "push_back" => String::from("VecDeque::push_back"),
                _ => return None,
            },
            _ => return None,
        };
        let wants_mut = matches!(
            method,
            "clear" | "push_str" | "push" | "pop" | "push_back" | "truncate"
        );
        Some((full_name, wants_mut))
    }

    fn fallback_alloc_method_call(&self, method: &str, args: &[HirExpr]) -> Option<(String, bool)> {
        let name = match method {
            "len" => String::from("len"),
            "push_str" => String::from("String::push_str"),
            "push" => {
                if args
                    .first()
                    .is_some_and(|arg| matches!(self.get_expr_ty(arg), TyKind::Char))
                {
                    String::from("String::push")
                } else {
                    String::from("Vec::push")
                }
            }
            "pop" => String::from("Vec::pop"),
            "push_back" => String::from("VecDeque::push_back"),
            "clear" => String::from("Vec::clear"),
            _ => return None,
        };
        let wants_mut = matches!(method, "push_str" | "push" | "pop" | "push_back" | "clear");
        Some((name, wants_mut))
    }

    fn estimate_ty_size(&self, ty: &TyKind) -> usize {
        match ty {
            TyKind::Bool => 1,
            TyKind::Char => 4,
            TyKind::Int(IntTy::I8) | TyKind::Uint(UintTy::U8) => 1,
            TyKind::Int(IntTy::I16) | TyKind::Uint(UintTy::U16) => 2,
            TyKind::Int(IntTy::I32) | TyKind::Uint(UintTy::U32) | TyKind::Float(FloatTy::F32) => 4,
            TyKind::Int(IntTy::I64)
            | TyKind::Int(IntTy::Isize)
            | TyKind::Uint(UintTy::U64)
            | TyKind::Uint(UintTy::Usize)
            | TyKind::Float(FloatTy::F64)
            | TyKind::FnDef(_, _)
            | TyKind::FnPtr(_, _)
            | TyKind::DynTrait(_) => 8,
            TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _)
                if matches!(
                    inner.as_ref(),
                    TyKind::Slice(_) | TyKind::Str | TyKind::DynTrait(_)
                ) =>
            {
                16
            }
            TyKind::Ref(_, _) | TyKind::RawPtr(_, _) => 8,
            TyKind::Int(IntTy::I128) | TyKind::Uint(UintTy::U128) => 16,
            TyKind::Array(inner, n) if *n > (1usize << 30) => self.estimate_ty_size(inner.as_ref()),
            TyKind::Array(inner, n) => self.estimate_ty_size(inner.as_ref()) * *n,
            TyKind::Slice(_) | TyKind::Str => 16,
            TyKind::Projection(_, _, _) => 8,
            TyKind::Tuple(tys) => tys.iter().map(|ty| self.estimate_ty_size(ty)).sum(),
            TyKind::Adt(def_id, _) => self
                .typeck
                .struct_defs
                .get(def_id)
                .map(|fields| fields.iter().map(|(_, ty)| self.estimate_ty_size(ty)).sum())
                .unwrap_or(8),
            TyKind::Unit | TyKind::Never | TyKind::Error | TyKind::Infer(_) | TyKind::Param(_) => 0,
        }
    }

    /// Get the field list for a struct type.
    fn resolve_struct_fields(&self, ty: &TyKind) -> Vec<(Symbol, TyKind)> {
        let inner = match ty {
            TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => inner.as_ref(),
            other => other,
        };
        if let TyKind::Adt(def_id, _) = inner {
            if let Some(fields) = self.typeck.struct_defs.get(def_id) {
                return fields.clone();
            }
        }
        Vec::new()
    }

    /// Extract discriminant value from a match arm pattern.
    fn pattern_discriminant(&self, pat: &HirPattern) -> Option<u128> {
        match pat {
            HirPattern::Path(path) if path.segments.len() == 2 => {
                let enum_name = path.segments[0].ident;
                let variant_name = path.segments[1].ident;
                self.resolve
                    .variant_indices
                    .get(&(enum_name, variant_name))
                    .map(|&idx| idx as u128)
            }
            HirPattern::TupleStruct(path, _, _) if path.segments.len() == 2 => {
                let enum_name = path.segments[0].ident;
                let variant_name = path.segments[1].ident;
                self.resolve
                    .variant_indices
                    .get(&(enum_name, variant_name))
                    .map(|&idx| idx as u128)
            }
            HirPattern::Literal(lit, _) => match lit {
                Literal::Int(v) => Some(*v as u128),
                Literal::Bool(b) => Some(*b as u128),
                Literal::Char(c) => Some(*c as u128),
                _ => None,
            },
            HirPattern::Struct(_, _, _, _) => None, // Struct patterns always match (like wildcard)
            HirPattern::Ident(_, _, _, _, _) => None, // Ident patterns are catch-all bindings
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

    fn primitive_assoc_const_value(path: &str) -> Option<(ConstValue, TyKind)> {
        use crate::typeck::{FloatTy, IntTy, UintTy};
        match path {
            "u8::MAX" => Some((ConstValue::Uint(u8::MAX as u128), TyKind::Uint(UintTy::U8))),
            "u16::MAX" => Some((
                ConstValue::Uint(u16::MAX as u128),
                TyKind::Uint(UintTy::U16),
            )),
            "u32::MAX" => Some((
                ConstValue::Uint(u32::MAX as u128),
                TyKind::Uint(UintTy::U32),
            )),
            "u64::MAX" => Some((
                ConstValue::Uint(u64::MAX as u128),
                TyKind::Uint(UintTy::U64),
            )),
            "u128::MAX" => Some((ConstValue::Uint(u128::MAX), TyKind::Uint(UintTy::U128))),
            "usize::MAX" => Some((
                ConstValue::Uint(usize::MAX as u128),
                TyKind::Uint(UintTy::Usize),
            )),
            "i8::MAX" => Some((ConstValue::Int(i8::MAX as i128), TyKind::Int(IntTy::I8))),
            "i16::MAX" => Some((ConstValue::Int(i16::MAX as i128), TyKind::Int(IntTy::I16))),
            "i32::MAX" => Some((ConstValue::Int(i32::MAX as i128), TyKind::Int(IntTy::I32))),
            "i64::MAX" => Some((ConstValue::Int(i64::MAX as i128), TyKind::Int(IntTy::I64))),
            "i128::MAX" => Some((ConstValue::Int(i128::MAX), TyKind::Int(IntTy::I128))),
            "isize::MAX" => Some((
                ConstValue::Int(isize::MAX as i128),
                TyKind::Int(IntTy::Isize),
            )),
            "i8::MIN" => Some((ConstValue::Int(i8::MIN as i128), TyKind::Int(IntTy::I8))),
            "i16::MIN" => Some((ConstValue::Int(i16::MIN as i128), TyKind::Int(IntTy::I16))),
            "i32::MIN" => Some((ConstValue::Int(i32::MIN as i128), TyKind::Int(IntTy::I32))),
            "i64::MIN" => Some((ConstValue::Int(i64::MIN as i128), TyKind::Int(IntTy::I64))),
            "i128::MIN" => Some((ConstValue::Int(i128::MIN), TyKind::Int(IntTy::I128))),
            "isize::MIN" => Some((
                ConstValue::Int(isize::MIN as i128),
                TyKind::Int(IntTy::Isize),
            )),
            "f32::MAX" => Some((
                ConstValue::Float(f32::MAX as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "f32::MIN" => Some((
                ConstValue::Float(f32::MIN as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "f32::MIN_POSITIVE" => Some((
                ConstValue::Float(f32::MIN_POSITIVE as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "f32::INFINITY" => Some((
                ConstValue::Float(f32::INFINITY as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "f32::NEG_INFINITY" => Some((
                ConstValue::Float(f32::NEG_INFINITY as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "f32::NAN" => Some((
                ConstValue::Float(f32::NAN as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "f32::EPSILON" => Some((
                ConstValue::Float(f32::EPSILON as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "f64::MAX" => Some((ConstValue::Float(f64::MAX), TyKind::Float(FloatTy::F64))),
            "f64::MIN" => Some((ConstValue::Float(f64::MIN), TyKind::Float(FloatTy::F64))),
            "f64::MIN_POSITIVE" => Some((
                ConstValue::Float(f64::MIN_POSITIVE),
                TyKind::Float(FloatTy::F64),
            )),
            "f64::INFINITY" => Some((
                ConstValue::Float(f64::INFINITY),
                TyKind::Float(FloatTy::F64),
            )),
            "f64::NEG_INFINITY" => Some((
                ConstValue::Float(f64::NEG_INFINITY),
                TyKind::Float(FloatTy::F64),
            )),
            "f64::NAN" => Some((ConstValue::Float(f64::NAN), TyKind::Float(FloatTy::F64))),
            "f64::EPSILON" => Some((ConstValue::Float(f64::EPSILON), TyKind::Float(FloatTy::F64))),
            "core::f64::consts::E" => Some((
                ConstValue::Float(core::f64::consts::E),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::FRAC_1_PI" => Some((
                ConstValue::Float(core::f64::consts::FRAC_1_PI),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::FRAC_1_SQRT_2" => Some((
                ConstValue::Float(core::f64::consts::FRAC_1_SQRT_2),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::FRAC_2_PI" => Some((
                ConstValue::Float(core::f64::consts::FRAC_2_PI),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::FRAC_2_SQRT_PI" => Some((
                ConstValue::Float(core::f64::consts::FRAC_2_SQRT_PI),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::FRAC_PI_2" => Some((
                ConstValue::Float(core::f64::consts::FRAC_PI_2),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::FRAC_PI_3" => Some((
                ConstValue::Float(core::f64::consts::FRAC_PI_3),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::FRAC_PI_4" => Some((
                ConstValue::Float(core::f64::consts::FRAC_PI_4),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::FRAC_PI_6" => Some((
                ConstValue::Float(core::f64::consts::FRAC_PI_6),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::FRAC_PI_8" => Some((
                ConstValue::Float(core::f64::consts::FRAC_PI_8),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::LN_2" => Some((
                ConstValue::Float(core::f64::consts::LN_2),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::LN_10" => Some((
                ConstValue::Float(core::f64::consts::LN_10),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::LOG2_10" => Some((
                ConstValue::Float(core::f64::consts::LOG2_10),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::LOG2_E" => Some((
                ConstValue::Float(core::f64::consts::LOG2_E),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::LOG10_2" => Some((
                ConstValue::Float(core::f64::consts::LOG10_2),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::LOG10_E" => Some((
                ConstValue::Float(core::f64::consts::LOG10_E),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::PI" => Some((
                ConstValue::Float(core::f64::consts::PI),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f64::consts::SQRT_2" => Some((
                ConstValue::Float(core::f64::consts::SQRT_2),
                TyKind::Float(FloatTy::F64),
            )),
            "core::f32::consts::E" => Some((
                ConstValue::Float(core::f32::consts::E as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::FRAC_1_PI" => Some((
                ConstValue::Float(core::f32::consts::FRAC_1_PI as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::FRAC_1_SQRT_2" => Some((
                ConstValue::Float(core::f32::consts::FRAC_1_SQRT_2 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::FRAC_2_PI" => Some((
                ConstValue::Float(core::f32::consts::FRAC_2_PI as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::FRAC_2_SQRT_PI" => Some((
                ConstValue::Float(core::f32::consts::FRAC_2_SQRT_PI as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::FRAC_PI_2" => Some((
                ConstValue::Float(core::f32::consts::FRAC_PI_2 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::FRAC_PI_3" => Some((
                ConstValue::Float(core::f32::consts::FRAC_PI_3 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::FRAC_PI_4" => Some((
                ConstValue::Float(core::f32::consts::FRAC_PI_4 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::FRAC_PI_6" => Some((
                ConstValue::Float(core::f32::consts::FRAC_PI_6 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::FRAC_PI_8" => Some((
                ConstValue::Float(core::f32::consts::FRAC_PI_8 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::LN_2" => Some((
                ConstValue::Float(core::f32::consts::LN_2 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::LN_10" => Some((
                ConstValue::Float(core::f32::consts::LN_10 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::LOG2_10" => Some((
                ConstValue::Float(core::f32::consts::LOG2_10 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::LOG2_E" => Some((
                ConstValue::Float(core::f32::consts::LOG2_E as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::LOG10_2" => Some((
                ConstValue::Float(core::f32::consts::LOG10_2 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::LOG10_E" => Some((
                ConstValue::Float(core::f32::consts::LOG10_E as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::PI" => Some((
                ConstValue::Float(core::f32::consts::PI as f64),
                TyKind::Float(FloatTy::F32),
            )),
            "core::f32::consts::SQRT_2" => Some((
                ConstValue::Float(core::f32::consts::SQRT_2 as f64),
                TyKind::Float(FloatTy::F32),
            )),
            s if s.ends_with("::MIN") && s.starts_with('u') => {
                Some((ConstValue::Uint(0), TyKind::Uint(UintTy::U8)))
            } // all unsigned MIN = 0
            _ => None,
        }
    }

    fn is_primitive_assoc_fn_path(path: &str) -> bool {
        let Some((base, assoc)) = path.split_once("::") else {
            return false;
        };
        let is_primitive = matches!(
            base,
            "u8" | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "f32"
                | "f64"
                | "bool"
                | "char"
        );
        is_primitive
            && matches!(
                assoc,
                "from"
                    | "try_from"
                    | "from_le"
                    | "from_be"
                    | "to_le"
                    | "to_be"
                    | "from_le_bytes"
                    | "from_be_bytes"
                    | "from_ne_bytes"
                    | "from_str_radix"
                    | "from_u32"
                    | "is_whitespace"
                    | "min"
                    | "max"
                    | "to_string"
            )
    }

    fn is_copy_type(&self, ty: &TyKind) -> bool {
        match ty {
            TyKind::Bool | TyKind::Char => true,
            TyKind::Int(_) | TyKind::Uint(_) | TyKind::Float(_) => true,
            TyKind::Ref(_, Mutability::Immutable) => true,
            TyKind::Ref(_, Mutability::Mut) => false,
            TyKind::RawPtr(_, _) => true,
            TyKind::Unit | TyKind::Never | TyKind::Error => true,
            TyKind::FnDef(_, _) | TyKind::FnPtr(_, _) => true,
            TyKind::Adt(def_id, _) => {
                if let Some(fields) = self.typeck.struct_defs.get(def_id) {
                    fields.iter().all(|(_, fty)| self.is_copy_type(fty))
                } else {
                    false
                }
            }
            TyKind::Tuple(elems) => elems.iter().all(|e| self.is_copy_type(e)),
            TyKind::Array(inner, _) => self.is_copy_type(inner),
            _ => false,
        }
    }
}
