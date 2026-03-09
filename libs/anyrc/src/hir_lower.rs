use crate::ast;
use crate::hir::*;
use crate::diagnostics::Span;
use crate::ast::UnOp;
use crate::intern::{Interner, Symbol};

pub struct LoweringContext<'a> {
    next_hir_id: u32,
    next_def_id: u32,
    interner: &'a mut Interner,
}

impl<'a> LoweringContext<'a> {
    pub fn new(interner: &'a mut Interner) -> Self {
        Self { next_hir_id: 0, next_def_id: 0, interner }
    }

    fn alloc_hir_id(&mut self) -> HirId {
        let id = HirId(self.next_hir_id);
        self.next_hir_id += 1;
        id
    }

    fn alloc_def_id(&mut self) -> DefId {
        let id = DefId(self.next_def_id);
        self.next_def_id += 1;
        id
    }

    pub fn lower_crate(&mut self, krate: &ast::Crate) -> HirCrate {
        let items = krate.items.iter().filter_map(|i| self.lower_item(i)).collect();
        HirCrate { items }
    }

    fn lower_item(&mut self, item: &ast::Item) -> Option<HirItem> {
        let (kind, span) = match item {
            ast::Item::Fn(f) => (HirItemKind::Fn(self.lower_fn_def(f)), f.span),
            ast::Item::Struct(s) => (HirItemKind::Struct(self.lower_struct_def(s)), s.span),
            ast::Item::Enum(e) => (HirItemKind::Enum(self.lower_enum_def(e)), e.span),
            ast::Item::Impl(ib) => (HirItemKind::Impl(self.lower_impl_block(ib)), ib.span),
            ast::Item::Trait(td) => (HirItemKind::Trait(self.lower_trait_def(td)), td.span),
            ast::Item::TypeAlias(ta) => (HirItemKind::TypeAlias(self.lower_type_alias(ta)), ta.span),
            ast::Item::Const(c) => (HirItemKind::Const(self.lower_const_def(c)), c.span),
            ast::Item::Static(s) => (HirItemKind::Static(self.lower_static_def(s)), s.span),
            ast::Item::Use(u) => (HirItemKind::Use(self.lower_use_tree(u)), u.span),
            ast::Item::Mod(m) => (HirItemKind::Mod(self.lower_mod_def(m)), m.span),
            ast::Item::ExternBlock(eb) => (HirItemKind::ExternBlock(self.lower_extern_block(eb)), eb.span),
            ast::Item::MacroDef(_) => return None, // macro defs are consumed by expansion
            ast::Item::MacroCall(_, _, _span) => return None, // unexpanded macros are dropped
        };
        Some(HirItem { id: self.alloc_hir_id(), kind, span })
    }

    fn lower_fn_def(&mut self, f: &ast::FnDef) -> HirFnDef {
        HirFnDef {
            def_id: self.alloc_def_id(),
            name: f.name,
            generics: self.lower_generics(&f.generics),
            params: f.params.iter().map(|p| self.lower_param(p)).collect(),
            ret_ty: f.ret_ty.as_ref().map(|t| Box::new(self.lower_ty(t))),
            body: f.body.as_ref().map(|b| self.lower_block(b)),
            vis: f.vis,
            is_unsafe: f.is_unsafe,
            is_const: f.is_const,
            abi: f.abi.clone(),
            no_mangle: self.has_attr(&f.attrs, "no_mangle"),
            is_panic_handler: self.has_attr(&f.attrs, "panic_handler"),
        }
    }

    fn has_attr(&mut self, attrs: &[ast::Attribute], name: &str) -> bool {
        let sym = self.interner.intern(name);
        attrs.iter().any(|a| a.path.segments.len() == 1 && a.path.segments[0].ident == sym)
    }

    fn lower_param(&mut self, p: &ast::Param) -> HirParam {
        HirParam {
            id: self.alloc_hir_id(),
            pat: self.lower_pattern(&p.pat),
            ty: self.lower_ty(&p.ty),
            span: p.span,
        }
    }

    fn lower_struct_def(&mut self, s: &ast::StructDef) -> HirStructDef {
        HirStructDef {
            def_id: self.alloc_def_id(),
            name: s.name,
            generics: self.lower_generics(&s.generics),
            fields: s.fields.iter().map(|f| self.lower_field_def(f)).collect(),
            vis: s.vis,
        }
    }

    fn lower_field_def(&mut self, f: &ast::FieldDef) -> HirFieldDef {
        HirFieldDef {
            id: self.alloc_hir_id(),
            name: f.name,
            ty: self.lower_ty(&f.ty),
            vis: f.vis,
            span: f.span,
        }
    }

    fn lower_enum_def(&mut self, e: &ast::EnumDef) -> HirEnumDef {
        HirEnumDef {
            def_id: self.alloc_def_id(),
            name: e.name,
            generics: self.lower_generics(&e.generics),
            variants: e.variants.iter().map(|v| self.lower_variant(v)).collect(),
            vis: e.vis,
        }
    }

    fn lower_variant(&mut self, v: &ast::Variant) -> HirVariant {
        HirVariant {
            id: self.alloc_hir_id(),
            name: v.name,
            fields: match &v.fields {
                ast::VariantFields::Unit => HirVariantFields::Unit,
                ast::VariantFields::Tuple(tys) => HirVariantFields::Tuple(tys.iter().map(|t| self.lower_ty(t)).collect()),
                ast::VariantFields::Struct(fs) => HirVariantFields::Struct(fs.iter().map(|f| self.lower_field_def(f)).collect()),
            },
            discriminant: v.discriminant.as_ref().map(|e| Box::new(self.lower_expr(e))),
            span: v.span,
        }
    }

    fn lower_impl_block(&mut self, ib: &ast::ImplBlock) -> HirImplBlock {
        HirImplBlock {
            id: self.alloc_hir_id(),
            generics: self.lower_generics(&ib.generics),
            trait_ref: ib.trait_ref.as_ref().map(|p| self.lower_path(p)),
            self_ty: self.lower_ty(&ib.self_ty),
            items: ib.items.iter().filter_map(|i| self.lower_item(i)).collect(),
            is_unsafe: ib.is_unsafe,
        }
    }

    fn lower_trait_def(&mut self, td: &ast::TraitDef) -> HirTraitDef {
        HirTraitDef {
            def_id: self.alloc_def_id(),
            name: td.name,
            generics: self.lower_generics(&td.generics),
            supertraits: td.supertraits.iter().map(|tb| self.lower_trait_bound(tb)).collect(),
            items: td.items.iter().filter_map(|i| self.lower_item(i)).collect(),
            vis: td.vis,
            is_unsafe: td.is_unsafe,
        }
    }

    fn lower_type_alias(&mut self, ta: &ast::TypeAliasDef) -> HirTypeAlias {
        HirTypeAlias {
            def_id: self.alloc_def_id(),
            name: ta.name,
            generics: self.lower_generics(&ta.generics),
            ty: ta.ty.as_ref().map(|t| Box::new(self.lower_ty(t))),
            vis: ta.vis,
        }
    }

    fn lower_const_def(&mut self, c: &ast::ConstDef) -> HirConstDef {
        HirConstDef {
            def_id: self.alloc_def_id(),
            name: c.name,
            ty: self.lower_ty(&c.ty),
            value: c.value.as_ref().map(|e| Box::new(self.lower_expr(e))),
            vis: c.vis,
        }
    }

    fn lower_static_def(&mut self, s: &ast::StaticDef) -> HirStaticDef {
        HirStaticDef {
            def_id: self.alloc_def_id(),
            name: s.name,
            ty: self.lower_ty(&s.ty),
            value: s.value.as_ref().map(|e| Box::new(self.lower_expr(e))),
            vis: s.vis,
            is_mut: s.is_mut,
        }
    }

    fn lower_use_tree(&mut self, u: &ast::UseTree) -> HirUseTree {
        HirUseTree {
            id: self.alloc_hir_id(),
            path: u.path.clone(),
            kind: match &u.kind {
                ast::UseTreeKind::Simple(alias) => HirUseTreeKind::Simple(*alias),
                ast::UseTreeKind::Glob => HirUseTreeKind::Glob,
                ast::UseTreeKind::Nested(trees) => HirUseTreeKind::Nested(trees.iter().map(|t| self.lower_use_tree(t)).collect()),
            },
            span: u.span,
        }
    }

    fn lower_mod_def(&mut self, m: &ast::ModDef) -> HirModDef {
        HirModDef {
            def_id: self.alloc_def_id(),
            name: m.name,
            items: m.items.as_ref().map(|items| items.iter().filter_map(|i| self.lower_item(i)).collect()),
            vis: m.vis,
        }
    }

    fn lower_extern_block(&mut self, eb: &ast::ExternBlockDef) -> HirExternBlock {
        HirExternBlock {
            id: self.alloc_hir_id(),
            abi: eb.abi.clone(),
            items: eb.items.iter().filter_map(|i| self.lower_item(i)).collect(),
        }
    }

    // ── Expressions ──

    fn lower_expr(&mut self, expr: &ast::Expr) -> HirExpr {
        let (kind, span) = match expr {
            ast::Expr::Lit(lit, span) => (HirExprKind::Lit(lit.clone()), *span),
            ast::Expr::Path(p) => (HirExprKind::Path(self.lower_path(p)), p.span),
            ast::Expr::Binary(op, l, r, span) => (
                HirExprKind::Binary(*op, Box::new(self.lower_expr(l)), Box::new(self.lower_expr(r))),
                *span,
            ),
            ast::Expr::Unary(op, e, span) => (HirExprKind::Unary(*op, Box::new(self.lower_expr(e))), *span),
            ast::Expr::Call(callee, args, span) => (
                HirExprKind::Call(Box::new(self.lower_expr(callee)), args.iter().map(|a| self.lower_expr(a)).collect()),
                *span,
            ),
            ast::Expr::MethodCall(recv, name, tys, args, span) => (
                HirExprKind::MethodCall(
                    Box::new(self.lower_expr(recv)),
                    *name,
                    tys.iter().map(|t| self.lower_ty(t)).collect(),
                    args.iter().map(|a| self.lower_expr(a)).collect(),
                ),
                *span,
            ),
            ast::Expr::Field(e, name, span) => (HirExprKind::Field(Box::new(self.lower_expr(e)), *name), *span),
            ast::Expr::Index(a, b, span) => (HirExprKind::Index(Box::new(self.lower_expr(a)), Box::new(self.lower_expr(b))), *span),
            ast::Expr::Block(b) => (HirExprKind::Block(self.lower_block(b)), b.span),
            ast::Expr::If(cond, then, else_, span) => (
                HirExprKind::If(
                    Box::new(self.lower_expr(cond)),
                    self.lower_block(then),
                    else_.as_ref().map(|e| Box::new(self.lower_expr(e))),
                ),
                *span,
            ),
            ast::Expr::Match(scrutinee, arms, span) => (
                HirExprKind::Match(
                    Box::new(self.lower_expr(scrutinee)),
                    arms.iter().map(|a| self.lower_match_arm(a)).collect(),
                ),
                *span,
            ),
            ast::Expr::Loop(b, label, span) => (HirExprKind::Loop(self.lower_block(b), *label), *span),
            // Desugar while → loop { if !cond { break; } body }
            ast::Expr::While(cond, body, label, span) => {
                let hir_cond = self.lower_expr(cond);
                let hir_body = self.lower_block(body);

                // Build: if !cond { break; }
                let not_cond = HirExpr {
                    id: self.alloc_hir_id(),
                    kind: HirExprKind::Unary(UnOp::Not, Box::new(hir_cond)),
                    span: *span,
                };
                let break_expr = HirExpr {
                    id: self.alloc_hir_id(),
                    kind: HirExprKind::Break(None, None),
                    span: *span,
                };
                let break_block = HirBlock {
                    id: self.alloc_hir_id(),
                    stmts: vec![HirStmt::Semi(break_expr, *span)],
                    span: *span,
                };
                let if_break = HirExpr {
                    id: self.alloc_hir_id(),
                    kind: HirExprKind::If(Box::new(not_cond), break_block, None),
                    span: *span,
                };

                // Build: loop { if_break; body_stmts... }
                let mut loop_stmts = vec![HirStmt::Semi(if_break, *span)];
                loop_stmts.extend(hir_body.stmts);
                let loop_block = HirBlock {
                    id: self.alloc_hir_id(),
                    stmts: loop_stmts,
                    span: *span,
                };
                (HirExprKind::Loop(loop_block, *label), *span)
            }
            // Keep for as-is for now (needs trait resolution)
            ast::Expr::For(pat, iter, body, label, span) => (
                HirExprKind::For(self.lower_pattern(pat), Box::new(self.lower_expr(iter)), self.lower_block(body), *label),
                *span,
            ),
            ast::Expr::Closure(params, ret_ty, body, is_move, span) => (
                HirExprKind::Closure(
                    params.iter().map(|p| self.lower_param(p)).collect(),
                    ret_ty.as_ref().map(|t| Box::new(self.lower_ty(t))),
                    Box::new(self.lower_expr(body)),
                    *is_move,
                ),
                *span,
            ),
            ast::Expr::Return(e, span) => (HirExprKind::Return(e.as_ref().map(|e| Box::new(self.lower_expr(e)))), *span),
            ast::Expr::Break(label, e, span) => (HirExprKind::Break(*label, e.as_ref().map(|e| Box::new(self.lower_expr(e)))), *span),
            ast::Expr::Continue(label, span) => (HirExprKind::Continue(*label), *span),
            ast::Expr::Assign(l, r, span) => (HirExprKind::Assign(Box::new(self.lower_expr(l)), Box::new(self.lower_expr(r))), *span),
            ast::Expr::AssignOp(op, l, r, span) => (HirExprKind::AssignOp(*op, Box::new(self.lower_expr(l)), Box::new(self.lower_expr(r))), *span),
            ast::Expr::Ref(e, m, span) => (HirExprKind::Ref(Box::new(self.lower_expr(e)), *m), *span),
            ast::Expr::Deref(e, span) => (HirExprKind::Deref(Box::new(self.lower_expr(e))), *span),
            ast::Expr::Cast(e, ty, span) => (HirExprKind::Cast(Box::new(self.lower_expr(e)), self.lower_ty(ty)), *span),
            ast::Expr::Struct(path, fields, base, span) => (
                HirExprKind::Struct(
                    self.lower_path(path),
                    fields.iter().map(|f| HirFieldExpr { name: f.name, value: self.lower_expr(&f.value), span: f.span }).collect(),
                    base.as_ref().map(|e| Box::new(self.lower_expr(e))),
                ),
                *span,
            ),
            ast::Expr::Tuple(es, span) => (HirExprKind::Tuple(es.iter().map(|e| self.lower_expr(e)).collect()), *span),
            ast::Expr::Array(es, span) => (HirExprKind::Array(es.iter().map(|e| self.lower_expr(e)).collect()), *span),
            ast::Expr::ArrayRepeat(a, b, span) => (HirExprKind::ArrayRepeat(Box::new(self.lower_expr(a)), Box::new(self.lower_expr(b))), *span),
            ast::Expr::Range(a, b, inclusive, span) => (
                HirExprKind::Range(
                    a.as_ref().map(|e| Box::new(self.lower_expr(e))),
                    b.as_ref().map(|e| Box::new(self.lower_expr(e))),
                    *inclusive,
                ),
                *span,
            ),
            ast::Expr::MacroCall(_, _, span) => {
                // Unexpanded macro call - shouldn't happen after macro expansion
                (HirExprKind::Lit(ast::Literal::Int(0)), *span)
            }
            ast::Expr::Unsafe(b, span) => (HirExprKind::Unsafe(self.lower_block(b)), *span),
            ast::Expr::Paren(e, span) => (HirExprKind::Paren(Box::new(self.lower_expr(e))), *span),
        };
        HirExpr { id: self.alloc_hir_id(), kind, span }
    }

    fn lower_match_arm(&mut self, arm: &ast::MatchArm) -> HirMatchArm {
        HirMatchArm {
            id: self.alloc_hir_id(),
            pat: self.lower_pattern(&arm.pat),
            guard: arm.guard.as_ref().map(|e| Box::new(self.lower_expr(e))),
            body: Box::new(self.lower_expr(&arm.body)),
            span: arm.span,
        }
    }

    // ── Blocks and Statements ──

    fn lower_block(&mut self, block: &ast::Block) -> HirBlock {
        HirBlock {
            id: self.alloc_hir_id(),
            stmts: block.stmts.iter().map(|s| self.lower_stmt(s)).collect(),
            span: block.span,
        }
    }

    fn lower_stmt(&mut self, stmt: &ast::Stmt) -> HirStmt {
        match stmt {
            ast::Stmt::Let(pat, ty, init, span) => HirStmt::Let(
                self.alloc_hir_id(),
                self.lower_pattern(pat),
                ty.as_ref().map(|t| self.lower_ty(t)),
                init.as_ref().map(|e| Box::new(self.lower_expr(e))),
                *span,
            ),
            ast::Stmt::Expr(e) => HirStmt::Expr(self.lower_expr(e)),
            ast::Stmt::Semi(e, span) => HirStmt::Semi(self.lower_expr(e), *span),
            ast::Stmt::Item(item) => {
                if let Some(hir_item) = self.lower_item(item) {
                    HirStmt::Item(hir_item)
                } else {
                    // Drop macro defs etc - replace with unit expr
                    HirStmt::Expr(HirExpr {
                        id: self.alloc_hir_id(),
                        kind: HirExprKind::Tuple(vec![]),
                        span: Span::dummy(),
                    })
                }
            }
        }
    }

    // ── Types ──

    fn lower_ty(&mut self, ty: &ast::Ty) -> HirTy {
        match ty {
            ast::Ty::Path(p) => HirTy::Path(self.lower_path(p)),
            ast::Ty::Reference(lt, t, m, span) => HirTy::Reference(*lt, Box::new(self.lower_ty(t)), *m, *span),
            ast::Ty::RawPtr(t, m, span) => HirTy::RawPtr(Box::new(self.lower_ty(t)), *m, *span),
            ast::Ty::Tuple(ts, span) => HirTy::Tuple(ts.iter().map(|t| self.lower_ty(t)).collect(), *span),
            ast::Ty::Array(t, len, span) => HirTy::Array(Box::new(self.lower_ty(t)), Box::new(self.lower_expr(len)), *span),
            ast::Ty::Slice(t, span) => HirTy::Slice(Box::new(self.lower_ty(t)), *span),
            ast::Ty::FnPtr(params, ret, span) => HirTy::FnPtr(
                params.iter().map(|t| self.lower_ty(t)).collect(),
                ret.as_ref().map(|t| Box::new(self.lower_ty(t))),
                *span,
            ),
            ast::Ty::Infer(span) => HirTy::Infer(*span),
            ast::Ty::Never(span) => HirTy::Never(*span),
        }
    }

    // ── Patterns ──

    fn lower_pattern(&mut self, pat: &ast::Pattern) -> HirPattern {
        match pat {
            ast::Pattern::Ident(name, m, sub, span) => HirPattern::Ident(
                self.alloc_hir_id(), *name, *m,
                sub.as_ref().map(|p| Box::new(self.lower_pattern(p))), *span,
            ),
            ast::Pattern::Literal(lit, span) => HirPattern::Literal(lit.clone(), *span),
            ast::Pattern::Tuple(ps, span) => HirPattern::Tuple(ps.iter().map(|p| self.lower_pattern(p)).collect(), *span),
            ast::Pattern::Struct(path, fps, dots, span) => HirPattern::Struct(
                self.lower_path(path),
                fps.iter().map(|fp| HirFieldPat { name: fp.name, pat: self.lower_pattern(&fp.pat), span: fp.span }).collect(),
                *dots, *span,
            ),
            ast::Pattern::TupleStruct(path, ps, span) => HirPattern::TupleStruct(
                self.lower_path(path), ps.iter().map(|p| self.lower_pattern(p)).collect(), *span,
            ),
            ast::Pattern::Wildcard(span) => HirPattern::Wildcard(*span),
            ast::Pattern::Ref(p, m, span) => HirPattern::Ref(Box::new(self.lower_pattern(p)), *m, *span),
            ast::Pattern::Or(ps, span) => HirPattern::Or(ps.iter().map(|p| self.lower_pattern(p)).collect(), *span),
            ast::Pattern::Range(a, b, inc, span) => HirPattern::Range(
                a.as_ref().map(|e| Box::new(self.lower_expr(e))),
                b.as_ref().map(|e| Box::new(self.lower_expr(e))),
                *inc, *span,
            ),
            ast::Pattern::Path(p) => HirPattern::Path(self.lower_path(p)),
        }
    }

    // ── Paths and Generics ──

    fn lower_path(&mut self, path: &ast::Path) -> HirPath {
        HirPath {
            segments: path.segments.iter().map(|s| HirPathSegment {
                ident: s.ident,
                args: s.args.as_ref().map(|a| HirGenericArgs {
                    args: a.args.iter().map(|arg| match arg {
                        ast::GenericArg::Type(t) => HirGenericArg::Type(self.lower_ty(t)),
                        ast::GenericArg::Lifetime(lt) => HirGenericArg::Lifetime(*lt),
                        ast::GenericArg::Const(e) => HirGenericArg::Const(self.lower_expr(e)),
                    }).collect(),
                    span: a.span,
                }),
            }).collect(),
            span: path.span,
        }
    }

    fn lower_generics(&mut self, g: &ast::Generics) -> HirGenerics {
        HirGenerics {
            params: g.params.iter().map(|p| match p {
                ast::GenericParam::Type(name, bounds, default, span) => HirGenericParam::Type(
                    *name,
                    bounds.iter().map(|tb| self.lower_trait_bound(tb)).collect(),
                    default.as_ref().map(|t| self.lower_ty(t)),
                    *span,
                ),
                ast::GenericParam::Lifetime(name, bounds, span) => HirGenericParam::Lifetime(*name, bounds.clone(), *span),
                ast::GenericParam::Const(name, ty, span) => HirGenericParam::Const(*name, self.lower_ty(ty), *span),
            }).collect(),
            span: g.span,
        }
    }

    fn lower_trait_bound(&mut self, tb: &ast::TraitBound) -> HirTraitBound {
        HirTraitBound { path: self.lower_path(&tb.path), span: tb.span }
    }
}
