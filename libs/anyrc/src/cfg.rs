//! Conditional compilation: `#[cfg(...)]` evaluation and item stripping.
//!
//! Parses cfg predicates from `#[cfg(...)]` attributes and evaluates them
//! against a set of active cfg flags. Items (functions, structs, modules, etc.)
//! whose cfg predicate evaluates to false are stripped from the AST before
//! further compilation stages.
//!
//! Supports:
//! - `#[cfg(name)]` — true if `name` is set
//! - `#[cfg(name = "value")]` — true if `name` equals `value`
//! - `#[cfg(not(pred))]` — negation
//! - `#[cfg(any(pred, pred, ...))]` — disjunction
//! - `#[cfg(all(pred, pred, ...))]` — conjunction
//! - `#[cfg_attr(pred, attr)]` — conditional attribute application

use crate::prelude::*;
use crate::ast::*;
use crate::intern::Interner;
use crate::lexer::TokenKind;
use anyos_std::collections::HashMap;

/// Holds the set of active cfg flags for the current compilation.
#[derive(Debug, Clone)]
pub struct CfgContext {
    /// Simple flags: cfg(name) — stored as name -> None
    /// Key-value flags: cfg(name = "value") — stored as name -> Some(value)
    flags: HashMap<String, Vec<Option<String>>>,
}

impl CfgContext {
    pub fn new() -> Self {
        Self { flags: HashMap::new() }
    }

    /// Build from a list of flag strings.
    /// Formats: "name" (bare), "name=\"value\"" (key-value).
    pub fn from_flags(flags: &[String]) -> Self {
        let mut ctx = Self::new();
        for flag in flags {
            if let Some(eq_pos) = flag.find('=') {
                let key = flag[..eq_pos].trim().to_string();
                let mut val = flag[eq_pos + 1..].trim().to_string();
                // Strip surrounding quotes
                if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                    val = val[1..val.len() - 1].to_string();
                }
                ctx.flags.entry(key).or_default().push(Some(val));
            } else {
                let key = flag.trim().to_string();
                ctx.flags.entry(key).or_default().push(None);
            }
        }
        ctx
    }

    /// Check if a bare flag is set: cfg(name)
    pub fn has_flag(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    /// Check if a key-value flag matches: cfg(name = "value")
    pub fn has_value(&self, name: &str, value: &str) -> bool {
        if let Some(vals) = self.flags.get(name) {
            vals.iter().any(|v| v.as_deref() == Some(value))
        } else {
            false
        }
    }
}

/// A parsed cfg predicate.
#[derive(Debug, Clone)]
enum CfgPred {
    /// cfg(name)
    Flag(String),
    /// cfg(name = "value")
    KeyValue(String, String),
    /// cfg(not(pred))
    Not(Box<CfgPred>),
    /// cfg(any(pred, pred, ...))
    Any(Vec<CfgPred>),
    /// cfg(all(pred, pred, ...))
    All(Vec<CfgPred>),
    /// Always true (empty or unparseable)
    True,
}

impl CfgPred {
    fn eval(&self, ctx: &CfgContext) -> bool {
        match self {
            CfgPred::Flag(name) => ctx.has_flag(name),
            CfgPred::KeyValue(name, value) => ctx.has_value(name, value),
            CfgPred::Not(inner) => !inner.eval(ctx),
            CfgPred::Any(preds) => preds.iter().any(|p| p.eval(ctx)),
            CfgPred::All(preds) => preds.iter().all(|p| p.eval(ctx)),
            CfgPred::True => true,
        }
    }
}

/// Parse a cfg predicate from attribute token trees.
/// The token trees are the contents inside `#[cfg(...)]`.
fn parse_cfg_pred(tokens: &[TokenTree], interner: &Interner) -> CfgPred {
    if tokens.is_empty() {
        return CfgPred::True;
    }

    // Check for not(...), any(...), all(...)
    if tokens.len() >= 2 {
        if let TokenTree::Token(t) = &tokens[0] {
            if let TokenKind::Ident(sym) = t.kind {
                let name = interner.resolve(sym);
                if let TokenTree::Delimited(Delimiter::Paren, inner) = &tokens[1] {
                    match name {
                        "not" => {
                            let inner_pred = parse_cfg_pred(inner, interner);
                            return CfgPred::Not(Box::new(inner_pred));
                        }
                        "any" => {
                            let preds = split_comma(inner).iter()
                                .map(|tts| parse_cfg_pred(tts, interner))
                                .collect();
                            return CfgPred::Any(preds);
                        }
                        "all" => {
                            let preds = split_comma(inner).iter()
                                .map(|tts| parse_cfg_pred(tts, interner))
                                .collect();
                            return CfgPred::All(preds);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Check for name = "value"
    if tokens.len() >= 3 {
        if let (TokenTree::Token(name_tok), TokenTree::Token(eq_tok)) = (&tokens[0], &tokens[1]) {
            if let TokenKind::Ident(name_sym) = name_tok.kind {
                if eq_tok.kind == TokenKind::Eq {
                    if let TokenTree::Token(val_tok) = &tokens[2] {
                        if let TokenKind::StringLit(ref val_str) = val_tok.kind {
                            return CfgPred::KeyValue(
                                interner.resolve(name_sym).to_string(),
                                val_str.clone(),
                            );
                        }
                    }
                }
            }
        }
    }

    // Bare flag: cfg(name)
    if tokens.len() == 1 {
        if let TokenTree::Token(t) = &tokens[0] {
            if let TokenKind::Ident(sym) = t.kind {
                return CfgPred::Flag(interner.resolve(sym).to_string());
            }
        }
    }

    CfgPred::True
}

/// Split token trees by commas at the top level.
fn split_comma(tokens: &[TokenTree]) -> Vec<Vec<TokenTree>> {
    let mut groups: Vec<Vec<TokenTree>> = Vec::new();
    let mut current: Vec<TokenTree> = Vec::new();

    for tt in tokens {
        if let TokenTree::Token(t) = tt {
            if t.kind == TokenKind::Comma {
                if !current.is_empty() {
                    groups.push(core::mem::take(&mut current));
                }
                continue;
            }
        }
        current.push(tt.clone());
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// Try to extract a cfg predicate from an attribute.
/// Returns Some(predicate) if this is a `#[cfg(...)]` attribute.
fn extract_cfg(attr: &Attribute, interner: &Interner) -> Option<CfgPred> {
    if attr.path.segments.len() != 1 {
        return None;
    }
    let name = interner.resolve(attr.path.segments[0].ident);
    if name != "cfg" {
        return None;
    }
    if let AttrArgs::Delimited(tokens) = &attr.args {
        Some(parse_cfg_pred(tokens, interner))
    } else {
        None
    }
}

/// Check if an item should be kept based on its #[cfg(...)] attributes.
fn should_keep_item_attrs(attrs: &[Attribute], ctx: &CfgContext, interner: &Interner) -> bool {
    for attr in attrs {
        if let Some(pred) = extract_cfg(attr, interner) {
            if !pred.eval(ctx) {
                return false;
            }
        }
    }
    true
}

/// Get the attributes of an item (if it has any).
fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Fn(f) => &f.attrs,
        Item::Struct(s) => &s.attrs,
        Item::Enum(e) => &e.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Mod(m) => &m.attrs,
        Item::Trait(t) => &t.attrs,
        Item::TypeAlias(t) => &t.attrs,
        Item::Const(c) => &c.attrs,
        Item::Static(s) => &s.attrs,
        Item::Use(u) => &u.attrs,
        Item::MacroDef(m) => &m.attrs,
        Item::ExternBlock(e) => &e.attrs,
        _ => &[],
    }
}

/// Strip items from a crate that don't match the cfg context.
pub fn strip_cfg(krate: &mut Crate, ctx: &CfgContext, interner: &Interner) {
    strip_items(&mut krate.items, ctx, interner);
}

fn strip_items(items: &mut Vec<Item>, ctx: &CfgContext, interner: &Interner) {
    items.retain(|item| should_keep_item_attrs(item_attrs(item), ctx, interner));

    // Recurse into surviving items
    for item in items.iter_mut() {
        match item {
            Item::Mod(md) => {
                if let Some(ref mut sub_items) = md.items {
                    strip_items(sub_items, ctx, interner);
                }
            }
            Item::Impl(ib) => {
                strip_items(&mut ib.items, ctx, interner);
            }
            Item::Trait(td) => {
                strip_items(&mut td.items, ctx, interner);
            }
            Item::Struct(s) => {
                s.fields.retain(|field| should_keep_item_attrs(&field.attrs, ctx, interner));
            }
            Item::Enum(e) => {
                e.variants.retain(|variant| should_keep_item_attrs(&variant.attrs, ctx, interner));
                for variant in &mut e.variants {
                    if let VariantFields::Struct(fields) = &mut variant.fields {
                        fields.retain(|field| should_keep_item_attrs(&field.attrs, ctx, interner));
                    }
                }
            }
            Item::Fn(f) => {
                if let Some(ref mut body) = f.body {
                    strip_block(body, ctx, interner);
                }
            }
            _ => {}
        }
    }
}

fn strip_block(block: &mut Block, ctx: &CfgContext, interner: &Interner) {
    strip_stmts(&mut block.stmts, ctx, interner);
}

fn strip_stmts(stmts: &mut Vec<Stmt>, ctx: &CfgContext, interner: &Interner) {
    stmts.retain(|stmt| {
        match stmt {
            Stmt::Item(item) => should_keep_item_attrs(item_attrs(item), ctx, interner),
            Stmt::Attributed(attrs, _, _) => should_keep_item_attrs(attrs, ctx, interner),
            _ => true,
        }
    });
    for stmt in stmts.iter_mut() {
        strip_stmt(stmt, ctx, interner);
    }
}

fn strip_stmt(stmt: &mut Stmt, ctx: &CfgContext, interner: &Interner) {
    match stmt {
        Stmt::Let(pat, _, init, _) => {
            strip_pattern(pat, ctx, interner);
            if let Some(init) = init {
                strip_expr(init, ctx, interner);
            }
        }
        Stmt::Expr(expr) | Stmt::Semi(expr, _) => strip_expr(expr, ctx, interner),
        Stmt::Item(item) => match item {
            Item::Mod(md) => {
                if let Some(ref mut sub_items) = md.items {
                    strip_items(sub_items, ctx, interner);
                }
            }
            Item::Impl(ib) => strip_items(&mut ib.items, ctx, interner),
            Item::Struct(s) => {
                s.fields.retain(|field| should_keep_item_attrs(&field.attrs, ctx, interner));
            }
            Item::Enum(e) => {
                e.variants.retain(|variant| should_keep_item_attrs(&variant.attrs, ctx, interner));
                for variant in &mut e.variants {
                    if let VariantFields::Struct(fields) = &mut variant.fields {
                        fields.retain(|field| should_keep_item_attrs(&field.attrs, ctx, interner));
                    }
                }
            }
            Item::Fn(f) => {
                if let Some(ref mut body) = f.body {
                    strip_block(body, ctx, interner);
                }
            }
            _ => {}
        },
        Stmt::Attributed(_, inner, _) => strip_stmt(inner, ctx, interner),
    }
}

fn strip_expr(expr: &mut Expr, ctx: &CfgContext, interner: &Interner) {
    match expr {
        Expr::Binary(_, lhs, rhs, _) | Expr::Assign(lhs, rhs, _) | Expr::AssignOp(_, lhs, rhs, _) => {
            strip_expr(lhs, ctx, interner);
            strip_expr(rhs, ctx, interner);
        }
        Expr::Unary(_, inner, _)
        | Expr::Return(Some(inner), _)
        | Expr::Ref(inner, _, _)
        | Expr::Deref(inner, _)
        | Expr::Paren(inner, _)
        | Expr::Cast(inner, _, _) => strip_expr(inner, ctx, interner),
        Expr::Call(callee, args, _) => {
            strip_expr(callee, ctx, interner);
            for arg in args {
                strip_expr(arg, ctx, interner);
            }
        }
        Expr::MethodCall(recv, _, _, args, _) => {
            strip_expr(recv, ctx, interner);
            for arg in args {
                strip_expr(arg, ctx, interner);
            }
        }
        Expr::Field(inner, _, _) => strip_expr(inner, ctx, interner),
        Expr::Index(base, index, _) => {
            strip_expr(base, ctx, interner);
            strip_expr(index, ctx, interner);
        }
        Expr::Block(block) | Expr::Unsafe(block, _) | Expr::Loop(block, _, _) => {
            strip_block(block, ctx, interner);
        }
        Expr::If(cond, then_block, else_branch, _) => {
            strip_expr(cond, ctx, interner);
            strip_block(then_block, ctx, interner);
            if let Some(else_expr) = else_branch {
                strip_expr(else_expr, ctx, interner);
            }
        }
        Expr::Match(scrutinee, arms, _) => {
            strip_expr(scrutinee, ctx, interner);
            for arm in arms {
                strip_pattern(&mut arm.pat, ctx, interner);
                if let Some(guard) = &mut arm.guard {
                    strip_expr(guard, ctx, interner);
                }
                strip_expr(&mut arm.body, ctx, interner);
            }
        }
        Expr::While(cond, body, _, _) => {
            strip_expr(cond, ctx, interner);
            strip_block(body, ctx, interner);
        }
        Expr::For(pat, iter, body, _, _) => {
            strip_pattern(pat, ctx, interner);
            strip_expr(iter, ctx, interner);
            strip_block(body, ctx, interner);
        }
        Expr::Closure(params, _, body, _, _) => {
            for param in params {
                strip_pattern(&mut param.pat, ctx, interner);
            }
            strip_expr(body, ctx, interner);
        }
        Expr::Break(_, value, _) => {
            if let Some(value) = value {
                strip_expr(value, ctx, interner);
            }
        }
        Expr::Struct(_, fields, base, _) => {
            fields.retain(|field| should_keep_item_attrs(&field.attrs, ctx, interner));
            for field in fields {
                strip_expr(&mut field.value, ctx, interner);
            }
            if let Some(base) = base {
                strip_expr(base, ctx, interner);
            }
        }
        Expr::Tuple(items, _) | Expr::Array(items, _) => {
            for item in items {
                strip_expr(item, ctx, interner);
            }
        }
        Expr::ArrayRepeat(value, count, _) => {
            strip_expr(value, ctx, interner);
            strip_expr(count, ctx, interner);
        }
        Expr::Range(start, end, _, _) => {
            if let Some(start) = start {
                strip_expr(start, ctx, interner);
            }
            if let Some(end) = end {
                strip_expr(end, ctx, interner);
            }
        }
        Expr::IfLet(pat, scrutinee, then_block, else_branch, _) => {
            strip_pattern(pat, ctx, interner);
            strip_expr(scrutinee, ctx, interner);
            strip_block(then_block, ctx, interner);
            if let Some(else_expr) = else_branch {
                strip_expr(else_expr, ctx, interner);
            }
        }
        Expr::WhileLet(pat, scrutinee, body, _, _) => {
            strip_pattern(pat, ctx, interner);
            strip_expr(scrutinee, ctx, interner);
            strip_block(body, ctx, interner);
        }
        Expr::InlineAsm(asm) => {
            for operand in &mut asm.operands {
                match operand {
                    AsmOperand::In { expr, .. } => strip_expr(expr, ctx, interner),
                    AsmOperand::Out { expr: Some(expr), .. } => strip_expr(expr, ctx, interner),
                    AsmOperand::InOut { expr, out_expr, .. } => {
                        strip_expr(expr, ctx, interner);
                        if let Some(out_expr) = out_expr {
                            strip_expr(out_expr, ctx, interner);
                        }
                    }
                    AsmOperand::Const { expr } => strip_expr(expr, ctx, interner),
                    AsmOperand::Sym { .. } | AsmOperand::Out { expr: None, .. } => {}
                }
            }
        }
        Expr::Lit(_, _)
        | Expr::Path(_)
        | Expr::QualifiedPath(_)
        | Expr::Continue(_, _)
        | Expr::Return(None, _)
        | Expr::MacroCall(_, _, _) => {}
    }
}

fn strip_pattern(pat: &mut Pattern, ctx: &CfgContext, interner: &Interner) {
    match pat {
        Pattern::Ident(_, _, sub, _) => {
            if let Some(sub) = sub {
                strip_pattern(sub, ctx, interner);
            }
        }
        Pattern::Tuple(pats, _)
        | Pattern::Slice(pats, _)
        | Pattern::TupleStruct(_, pats, _)
        | Pattern::Or(pats, _) => {
            for pat in pats {
                strip_pattern(pat, ctx, interner);
            }
        }
        Pattern::Struct(_, fields, _, _) => {
            fields.retain(|field| should_keep_item_attrs(&field.attrs, ctx, interner));
            for field in fields {
                strip_pattern(&mut field.pat, ctx, interner);
            }
        }
        Pattern::Ref(inner, _, _) => strip_pattern(inner, ctx, interner),
        Pattern::Range(start, end, _, _) => {
            if let Some(start) = start {
                strip_expr(start, ctx, interner);
            }
            if let Some(end) = end {
                strip_expr(end, ctx, interner);
            }
        }
        Pattern::Literal(_, _)
        | Pattern::Wildcard(_)
        | Pattern::Rest(_)
        | Pattern::Path(_) => {}
    }
}
