use crate::prelude::*;
use crate::ast::*;
use crate::diagnostics::Span;
use crate::intern::{Interner, Symbol};
use crate::lexer::{Token, TokenKind, Keyword};
use crate::parser::Parser;

use anyos_std::collections::HashMap;

/// A registered macro definition.
struct MacroDef {
    name: Symbol,
    rules: Vec<MacroRule>,
}

struct DllExportsSpec {
    lib_path: String,
    lib_struct: String,
    init_call: Option<String>,
    symbols: Vec<DllExportSymbol>,
}

struct DllExportSymbol {
    name: String,
    param_tys: Vec<String>,
    ret_ty: String,
}

/// A captured fragment from macro pattern matching.
#[derive(Clone, Debug)]
enum Capture {
    /// A single captured fragment (the token trees it matched).
    Single(Vec<TokenTree>),
    /// Repeated captures from a $(...)*
    Repeated(Vec<Vec<TokenTree>>),
}

/// Extract arguments from macro call token trees.
/// Returns each comma-separated group of tokens as a re-parsed expression.
fn collect_macro_call_args(tokens: &[TokenTree], interner: &mut Interner) -> Vec<Expr> {
    // Convert token trees back to source string, then parse as expressions
    let src = token_trees_to_string(tokens, interner);
    if src.trim().is_empty() {
        return vec![];
    }
    // Wrap in a function call context and parse the arguments
    let wrapped = format!("__f({})", src);
    let mut parser = Parser::new(&wrapped, interner);
    let expr = parser.parse_expr();
    // Extract the args from the Call expression
    if let Expr::Call(_, args, _) = expr {
        args
    } else {
        vec![]
    }
}

fn make_intrinsic_path(interner: &mut Interner, name: &str) -> Path {
    Path {
        segments: vec![PathSegment {
            ident: interner.intern(name),
            args: None,
        }],
        span: Span::dummy(),
    }
}

fn parse_vec_macro_expr(tokens: &[TokenTree], interner: &mut Interner) -> Option<Expr> {
    let src = token_trees_to_string(tokens, interner);
    let wrapped = format!("[{}]", src);
    let mut parser = Parser::new(&wrapped, interner);
    Some(parser.parse_expr())
}

/// Expand all macros in a crate, modifying the AST in place.
pub fn expand_macros(krate: &mut Crate, interner: &mut Interner) {
    let defs = collect_macro_defs(krate);
    for _ in 0..64 {
        let mut changed = false;
        expand_items(&mut krate.items, &defs, interner, &mut changed);
        if !changed { break; }
    }
}

fn collect_macro_defs(krate: &Crate) -> Vec<MacroDef> {
    let mut defs = Vec::new();
    collect_macro_defs_from_items(&krate.items, &mut defs);
    defs
}

fn collect_macro_defs_from_items(items: &[Item], defs: &mut Vec<MacroDef>) {
    for item in items {
        match item {
            Item::MacroDef(md) => defs.push(MacroDef { name: md.name, rules: md.rules.clone() }),
            Item::Mod(md) => {
                if let Some(items) = &md.items {
                    collect_macro_defs_from_items(items, defs);
                }
            }
            Item::Impl(ib) => collect_macro_defs_from_items(&ib.items, defs),
            Item::Trait(td) => collect_macro_defs_from_items(&td.items, defs),
            Item::ExternBlock(eb) => collect_macro_defs_from_items(&eb.items, defs),
            _ => {}
        }
    }
}

// ── AST walking and expansion ──

fn expand_items(items: &mut Vec<Item>, defs: &[MacroDef], interner: &mut Interner, changed: &mut bool) {
    let mut i = 0;
    while i < items.len() {
        // Try to expand macro calls at item position
        let should_expand = matches!(&items[i], Item::MacroCall(..));
        if should_expand {
            if let Item::MacroCall(path, args, span) = &items[i] {
                // Check for built-in item macros
                let macro_name = if !path.segments.is_empty() {
                    interner.resolve(path.segments.last().unwrap().ident).to_string()
                } else {
                    String::new()
                };

                if macro_name == "entry" {
                    // anyos_std::entry!(main) expands to:
                    //   #[no_mangle]
                    //   pub extern "C" fn _start() -> ! {
                    //       anyos_std::heap::init();
                    //       let code = main();
                    //       anyos_std::process::exit(code as u32);
                    //   }
                    let entry_src = concat!(
                        "#[no_mangle]\n",
                        "pub extern \"C\" fn _start() {\n",
                        "    main();\n",
                        "    exit(0);\n",
                        "}\n",
                    );
                    let mut parser = Parser::new(entry_src, interner);
                    let krate = parser.parse_crate();
                    items.splice(i..=i, krate.items);
                    *changed = true;
                    continue;
                }

                if macro_name == "dll_exports" {
                    if let Some(expanded) = expand_builtin_dll_exports(args, interner) {
                        items.splice(i..=i, expanded);
                        *changed = true;
                        continue;
                    }
                }

                if macro_name == "define_cast" {
                    if let Some(expanded) = expand_builtin_define_cast(args, interner) {
                        items.splice(i..=i, expanded);
                        *changed = true;
                        continue;
                    }
                }

                if macro_name == "cfg_if" {
                    if let Some(expanded) = expand_builtin_cfg_if(args, interner) {
                        items.splice(i..=i, expanded);
                        *changed = true;
                        continue;
                    }
                }

                if let Some(def) = find_macro(defs, path) {
                    if let Some(expanded) = try_expand_to_items(def, args, interner) {
                        items.splice(i..=i, expanded);
                        *changed = true;
                        continue;
                    }
                }
            }
            i += 1;
            continue;
        }

        // Recurse into item bodies
        take_and_modify(&mut items[i], |item| {
            match item {
                Item::Fn(ref mut f) => {
                    if let Some(ref mut body) = f.body {
                        expand_block(body, defs, interner, changed);
                    }
                }
                Item::Impl(ref mut ib) => expand_items(&mut ib.items, defs, interner, changed),
                Item::Trait(ref mut td) => expand_items(&mut td.items, defs, interner, changed),
                Item::Mod(ref mut md) => {
                    if let Some(ref mut items) = md.items {
                        expand_items(items, defs, interner, changed);
                    }
                }
                Item::Const(ref mut c) => {
                    if let Some(ref mut value) = c.value {
                        expand_expr(value, defs, interner, changed);
                    }
                }
                Item::Static(ref mut s) => {
                    if let Some(ref mut value) = s.value {
                        expand_expr(value, defs, interner, changed);
                    }
                }
                _ => {}
            }
        });
        i += 1;
    }
}

/// Helper to modify an item in place without needing to take ownership.
fn take_and_modify(item: &mut Item, f: impl FnOnce(&mut Item)) {
    f(item);
}

fn expand_block(block: &mut Block, defs: &[MacroDef], interner: &mut Interner, changed: &mut bool) {
    let mut i = 0;
    while i < block.stmts.len() {
        if let Stmt::Semi(Expr::MacroCall(path, args, _), _) = &block.stmts[i] {
            let macro_name = if !path.segments.is_empty() {
                interner.resolve(path.segments.last().unwrap().ident).to_string()
            } else {
                String::new()
            };
            if macro_name == "define_cast" {
                if let Some(items) = expand_builtin_define_cast(args, interner) {
                    block.stmts.splice(i..=i, items.into_iter().map(Stmt::Item));
                    *changed = true;
                    continue;
                }
            }
            if let Some(def) = find_macro(defs, path) {
                if let Some(items) = try_expand_to_items(def, args, interner) {
                    block.stmts.splice(i..=i, items.into_iter().map(Stmt::Item));
                    *changed = true;
                    continue;
                }
            }
        }

        expand_stmt(&mut block.stmts[i], defs, interner, changed);
        i += 1;
    }
}

fn expand_stmt(stmt: &mut Stmt, defs: &[MacroDef], interner: &mut Interner, changed: &mut bool) {
    match stmt {
        Stmt::Expr(expr) | Stmt::Semi(expr, _) => expand_expr(expr, defs, interner, changed),
        Stmt::Let(_, _, Some(init), _) => expand_expr(init, defs, interner, changed),
        Stmt::Item(item) => take_and_modify(item, |item| {
            if let Item::Fn(ref mut f) = item {
                if let Some(ref mut body) = f.body {
                    expand_block(body, defs, interner, changed);
                }
            }
        }),
        _ => {}
    }
}

fn expand_expr(expr: &mut Expr, defs: &[MacroDef], interner: &mut Interner, changed: &mut bool) {
    // Try macro expansion first
    let try_expand = matches!(expr, Expr::MacroCall(..));
    if try_expand {
        if let Expr::MacroCall(path, args, span) = expr {
            // Check for built-in macros first
            let macro_name = if !path.segments.is_empty() {
                interner.resolve(path.segments.last().unwrap().ident).to_string()
            } else {
                String::new()
            };

            match macro_name.as_str() {
                "format" => {
                    // format!("...", args...) → __anyrc_format("...", args...)
                    // Expand to a call to __anyrc_format intrinsic
                    let fn_path = make_intrinsic_path(interner, "__anyrc_format");
                    let call_args = collect_macro_call_args(args, interner);
                    *expr = Expr::Call(
                        Box::new(Expr::Path(fn_path)),
                        call_args,
                        *span,
                    );
                    *changed = true;
                    return;
                }
                "format_args" => {
                    let fn_path = make_intrinsic_path(interner, "__anyrc_format_args");
                    let call_args = collect_macro_call_args(args, interner);
                    *expr = Expr::Call(
                        Box::new(Expr::Path(fn_path)),
                        call_args,
                        *span,
                    );
                    *changed = true;
                    return;
                }
                "vec" => {
                    if let Some(parsed) = parse_vec_macro_expr(args, interner) {
                        let to_vec_sym = interner.intern("to_vec");
                        *expr = match parsed {
                            Expr::Array(_, _) | Expr::ArrayRepeat(_, _, _) => {
                                Expr::MethodCall(Box::new(parsed), to_vec_sym, vec![], vec![], *span)
                            }
                            _ => parsed,
                        };
                    } else {
                        let fn_path = make_intrinsic_path(interner, "Vec::new");
                        *expr = Expr::Call(
                            Box::new(Expr::Path(fn_path)),
                            vec![],
                            *span,
                        );
                    }
                    *changed = true;
                    return;
                }
                "println" | "eprintln" => {
                    // println!("...", args...) → __anyrc_println("...", args...)
                    let fn_path = make_intrinsic_path(interner, "__anyrc_println");
                    let call_args = collect_macro_call_args(args, interner);
                    *expr = Expr::Call(
                        Box::new(Expr::Path(fn_path)),
                        call_args,
                        *span,
                    );
                    *changed = true;
                    return;
                }
                "write" | "writeln" => {
                    let mut call_args = collect_macro_call_args(args, interner);
                    if call_args.is_empty() {
                        *expr = Expr::Tuple(vec![], *span);
                        *changed = true;
                        return;
                    }

                    let target = call_args.remove(0);
                    let fmt_expr = if call_args.is_empty() {
                        Expr::Tuple(vec![], *span)
                    } else {
                        let fn_path = make_intrinsic_path(interner, "__anyrc_format_args");
                        Expr::Call(Box::new(Expr::Path(fn_path)), call_args, *span)
                    };
                    let write_fmt_path = Path {
                        segments: vec![
                            PathSegment { ident: interner.intern("core"), args: None },
                            PathSegment { ident: interner.intern("fmt"), args: None },
                            PathSegment { ident: interner.intern("Write"), args: None },
                            PathSegment { ident: interner.intern("write_fmt"), args: None },
                        ],
                        span: Span::dummy(),
                    };
                    *expr = Expr::Call(
                        Box::new(Expr::Path(write_fmt_path)),
                        vec![
                            Expr::Ref(Box::new(target), Mutability::Mut, *span),
                            fmt_expr,
                        ],
                        *span,
                    );
                    *changed = true;
                    return;
                }
                "matches" => {
                    *expr = Expr::Lit(Literal::Bool(false), *span);
                    *changed = true;
                    return;
                }
                "assert" | "assert_eq" | "debug_assert" | "debug_assert_eq" => {
                    // Expand to a no-op or simple check (for bootstrap)
                    *expr = Expr::Tuple(vec![], *span);
                    *changed = true;
                    return;
                }
                "env" => {
                    // env!("VAR_NAME") → "value" (compile-time environment variable)
                    let src = token_trees_to_string(args, interner);
                    let var_name = src.trim().trim_matches('"');
                    // Look up in process environment (on anyOS: via anyos_std)
                    let value = lookup_env(var_name);
                    *expr = Expr::Lit(Literal::String(value), *span);
                    *changed = true;
                    return;
                }
                "option_env" => {
                    // option_env!("VAR_NAME") → Some("value") or None
                    // For simplicity, expand to empty string if not set
                    let src = token_trees_to_string(args, interner);
                    let var_name = src.trim().trim_matches('"');
                    let value = lookup_env(var_name);
                    *expr = Expr::Lit(Literal::String(value), *span);
                    *changed = true;
                    return;
                }
                "include_bytes" => {
                    // include_bytes!("path") → b"..." (compile-time file inclusion)
                    let src = token_trees_to_string(args, interner);
                    let path = src.trim().trim_matches('"');
                    let data = if let Some(bytes) = crate::loader::OsFileLoader::read_bytes(path) {
                        bytes
                    } else {
                        Vec::new()
                    };
                    *expr = Expr::Lit(Literal::ByteString(data), *span);
                    *changed = true;
                    return;
                }
                "include_str" => {
                    // include_str!("path") → "..." (compile-time file inclusion as string)
                    let src = token_trees_to_string(args, interner);
                    let path = src.trim().trim_matches('"');
                    let data = if let Some(bytes) = crate::loader::OsFileLoader::read_bytes(path) {
                        alloc::string::String::from_utf8(bytes).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    *expr = Expr::Lit(Literal::String(data), *span);
                    *changed = true;
                    return;
                }
                "cfg" => {
                    // cfg!(predicate) → true/false (compile-time cfg check)
                    // For now, always return false (without CfgContext access)
                    // The proper evaluation happens in the cfg stripping pass;
                    // cfg!() as an expression macro defaults to false.
                    *expr = Expr::Lit(Literal::Bool(false), *span);
                    *changed = true;
                    return;
                }
                "concat" => {
                    // concat!("a", "b", expr) → "ab..."
                    let call_args = collect_macro_call_args(args, interner);
                    let mut result = String::new();
                    for arg in &call_args {
                        match arg {
                            Expr::Lit(Literal::String(s), _) => result.push_str(s),
                            Expr::Lit(Literal::Int(n), _) => {
                                use core::fmt::Write;
                                let _ = write!(result, "{}", n);
                            }
                            Expr::Lit(Literal::Bool(b), _) => {
                                result.push_str(if *b { "true" } else { "false" });
                            }
                            _ => result.push_str("?"),
                        }
                    }
                    *expr = Expr::Lit(Literal::String(result), *span);
                    *changed = true;
                    return;
                }
                "stringify" => {
                    // stringify!(tokens) → "tokens" (stringified token stream)
                    let src = token_trees_to_string(args, interner);
                    *expr = Expr::Lit(Literal::String(src), *span);
                    *changed = true;
                    return;
                }
                "line" => {
                    *expr = Expr::Lit(Literal::Int(0), *span);
                    *changed = true;
                    return;
                }
                "column" => {
                    *expr = Expr::Lit(Literal::Int(0), *span);
                    *changed = true;
                    return;
                }
                "file" => {
                    *expr = Expr::Lit(Literal::String(String::from("")), *span);
                    *changed = true;
                    return;
                }
                "module_path" => {
                    *expr = Expr::Lit(Literal::String(String::from("")), *span);
                    *changed = true;
                    return;
                }
                "compile_error" => {
                    // compile_error!("msg") — for now, just produce empty tuple
                    *expr = Expr::Tuple(vec![], *span);
                    *changed = true;
                    return;
                }
                _ => {}
            }

            if let Some(def) = find_macro(defs, path) {
                if let Some(expanded) = try_expand_to_expr(def, args, interner) {
                    *expr = expanded;
                    *changed = true;
                    expand_expr(expr, defs, interner, changed);
                    return;
                }
            }
        }
    }

    // Recurse
    match expr {
        Expr::Binary(_, l, r, _) => { expand_expr(l, defs, interner, changed); expand_expr(r, defs, interner, changed); }
        Expr::Unary(_, e, _) | Expr::Paren(e, _) | Expr::Deref(e, _) => expand_expr(e, defs, interner, changed),
        Expr::Call(callee, args, _) => {
            expand_expr(callee, defs, interner, changed);
            for a in args { expand_expr(a, defs, interner, changed); }
        }
        Expr::MethodCall(recv, _, _, args, _) => {
            expand_expr(recv, defs, interner, changed);
            for a in args { expand_expr(a, defs, interner, changed); }
        }
        Expr::Block(b) | Expr::Unsafe(b, _) => expand_block(b, defs, interner, changed),
        Expr::If(cond, then, else_, _) => {
            expand_expr(cond, defs, interner, changed);
            expand_block(then, defs, interner, changed);
            if let Some(e) = else_ { expand_expr(e, defs, interner, changed); }
        }
        Expr::Match(scrutinee, arms, _) => {
            expand_expr(scrutinee, defs, interner, changed);
            for arm in arms { expand_expr(&mut arm.body, defs, interner, changed); }
        }
        Expr::Loop(b, _, _) => expand_block(b, defs, interner, changed),
        Expr::While(cond, b, _, _) => { expand_expr(cond, defs, interner, changed); expand_block(b, defs, interner, changed); }
        Expr::For(_, iter, b, _, _) => { expand_expr(iter, defs, interner, changed); expand_block(b, defs, interner, changed); }
        Expr::Return(Some(e), _) | Expr::Break(_, Some(e), _) => expand_expr(e, defs, interner, changed),
        Expr::Assign(l, r, _) | Expr::AssignOp(_, l, r, _) => { expand_expr(l, defs, interner, changed); expand_expr(r, defs, interner, changed); }
        Expr::Ref(e, _, _) | Expr::Cast(e, _, _) | Expr::Field(e, _, _) => expand_expr(e, defs, interner, changed),
        Expr::Index(a, b, _) => { expand_expr(a, defs, interner, changed); expand_expr(b, defs, interner, changed); }
        Expr::Tuple(es, _) | Expr::Array(es, _) => { for e in es { expand_expr(e, defs, interner, changed); } }
        Expr::Closure(_, _, body, _, _) => expand_expr(body, defs, interner, changed),
        Expr::Struct(_, fields, base, _) => {
            for f in fields { expand_expr(&mut f.value, defs, interner, changed); }
            if let Some(b) = base { expand_expr(b, defs, interner, changed); }
        }
        Expr::ArrayRepeat(a, b, _) => { expand_expr(a, defs, interner, changed); expand_expr(b, defs, interner, changed); }
        Expr::Range(a, b, _, _) => {
            if let Some(a) = a { expand_expr(a, defs, interner, changed); }
            if let Some(b) = b { expand_expr(b, defs, interner, changed); }
        }
        Expr::IfLet(_, scrutinee, then, else_, _) => {
            expand_expr(scrutinee, defs, interner, changed);
            expand_block(then, defs, interner, changed);
            if let Some(e) = else_ { expand_expr(e, defs, interner, changed); }
        }
        Expr::WhileLet(_, scrutinee, body, _, _) => {
            expand_expr(scrutinee, defs, interner, changed);
            expand_block(body, defs, interner, changed);
        }
        Expr::InlineAsm(asm) => {
            for op in &mut asm.operands {
                match op {
                    AsmOperand::In { expr, .. } | AsmOperand::InOut { expr, .. } | AsmOperand::Const { expr } => {
                        expand_expr(expr, defs, interner, changed);
                    }
                    AsmOperand::Out { expr: Some(expr), .. } => {
                        expand_expr(expr, defs, interner, changed);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn find_macro<'a>(defs: &'a [MacroDef], path: &Path) -> Option<&'a MacroDef> {
    if path.segments.len() == 1 {
        let name = path.segments[0].ident;
        defs.iter().find(|d| d.name == name)
    } else {
        None
    }
}

fn expand_builtin_dll_exports(args: &[TokenTree], interner: &mut Interner) -> Option<Vec<Item>> {
    let spec = parse_dll_exports_spec(args, interner)?;
    let src = render_dll_exports_source(&spec);
    let mut parser = Parser::new(&src, interner);
    Some(parser.parse_crate().items)
}

fn expand_builtin_cfg_if(args: &[TokenTree], interner: &mut Interner) -> Option<Vec<Item>> {
    let branches = parse_cfg_if_branches(args, interner)?;
    let mut expanded = Vec::new();
    let mut prev_conds: Vec<Vec<TokenTree>> = Vec::new();

    for branch in branches {
        let branch_cfg = cfg_if_branch_predicate(branch.cond.clone(), &prev_conds, interner);
        let src = token_trees_to_string(&branch.body, interner);
        let mut parser = Parser::new(&src, interner);
        let krate = parser.parse_crate();
        for mut item in krate.items {
            if let Some(pred) = &branch_cfg {
                prepend_cfg_attr(&mut item, cfg_attr(pred.clone(), interner));
            }
            expanded.push(item);
        }
        if let Some(cond) = branch.cond {
            prev_conds.push(cond);
        }
    }

    Some(expanded)
}

#[derive(Clone)]
struct CfgIfBranch {
    cond: Option<Vec<TokenTree>>,
    body: Vec<TokenTree>,
}

fn parse_cfg_if_branches(args: &[TokenTree], interner: &Interner) -> Option<Vec<CfgIfBranch>> {
    let mut branches = Vec::new();
    let mut idx = 0;
    let mut expect_if = true;

    loop {
        if expect_if {
            if !matches_token_kind(args.get(idx), |kind| *kind == TokenKind::Kw(Keyword::If)) {
                return None;
            }
            idx += 1;
            let cond = parse_cfg_if_condition(args, &mut idx, interner)?;
            let body = parse_cfg_if_body(args, &mut idx)?;
            branches.push(CfgIfBranch { cond: Some(cond), body });
            expect_if = false;
        }

        if idx >= args.len() {
            break;
        }

        if !matches_token_kind(args.get(idx), |kind| *kind == TokenKind::Kw(Keyword::Else)) {
            return None;
        }
        idx += 1;

        if matches_token_kind(args.get(idx), |kind| *kind == TokenKind::Kw(Keyword::If)) {
            expect_if = true;
            continue;
        }

        let body = parse_cfg_if_body(args, &mut idx)?;
        branches.push(CfgIfBranch { cond: None, body });
        break;
    }

    Some(branches)
}

fn parse_cfg_if_condition(
    args: &[TokenTree],
    idx: &mut usize,
    interner: &Interner,
) -> Option<Vec<TokenTree>> {
    if !matches_token_kind(args.get(*idx), |kind| *kind == TokenKind::Hash) {
        return None;
    }
    *idx += 1;
    let TokenTree::Delimited(Delimiter::Bracket, attr_tokens) = args.get(*idx)? else {
        return None;
    };
    *idx += 1;
    if attr_tokens.len() != 2 {
        return None;
    }
    let TokenTree::Token(name_tok) = &attr_tokens[0] else {
        return None;
    };
    let TokenKind::Ident(sym) = name_tok.kind else {
        return None;
    };
    if interner.resolve(sym) != "cfg" {
        return None;
    }
    let TokenTree::Delimited(Delimiter::Paren, cfg_tokens) = &attr_tokens[1] else {
        return None;
    };
    Some(cfg_tokens.clone())
}

fn parse_cfg_if_body(args: &[TokenTree], idx: &mut usize) -> Option<Vec<TokenTree>> {
    let TokenTree::Delimited(Delimiter::Brace, body) = args.get(*idx)? else {
        return None;
    };
    *idx += 1;
    Some(body.clone())
}

fn matches_token_kind<F>(tt: Option<&TokenTree>, pred: F) -> bool
where
    F: FnOnce(&TokenKind) -> bool,
{
    matches!(tt, Some(TokenTree::Token(tok)) if pred(&tok.kind))
}

fn cfg_if_branch_predicate(
    cond: Option<Vec<TokenTree>>,
    prev_conds: &[Vec<TokenTree>],
    interner: &mut Interner,
) -> Option<Vec<TokenTree>> {
    let pred = if prev_conds.is_empty() {
        cond?
    } else {
        let mut all_parts = Vec::new();
        for prev in prev_conds {
            if !all_parts.is_empty() {
                all_parts.push(comma_tt());
            }
            all_parts.extend(not_predicate(prev.clone(), interner));
        }
        if let Some(cond) = cond {
            if !all_parts.is_empty() {
                all_parts.push(comma_tt());
            }
            all_parts.extend(cond);
        }
        vec![
            ident_tt("all", interner),
            TokenTree::Delimited(Delimiter::Paren, all_parts),
        ]
    };

    Some(pred)
}

fn cfg_attr(pred: Vec<TokenTree>, interner: &mut Interner) -> Attribute {
    Attribute {
        path: Path {
            segments: vec![PathSegment {
                ident: interner.intern("cfg"),
                args: None,
            }],
            span: Span::dummy(),
        },
        args: AttrArgs::Delimited(pred),
        span: Span::dummy(),
    }
}

fn not_predicate(pred: Vec<TokenTree>, interner: &mut Interner) -> Vec<TokenTree> {
    vec![
        ident_tt("not", interner),
        TokenTree::Delimited(Delimiter::Paren, pred),
    ]
}

fn ident_tt(name: &str, interner: &mut Interner) -> TokenTree {
    TokenTree::Token(Token {
        kind: TokenKind::Ident(interner.intern(name)),
        span: Span::dummy(),
    })
}

fn comma_tt() -> TokenTree {
    TokenTree::Token(Token {
        kind: TokenKind::Comma,
        span: Span::dummy(),
    })
}

fn prepend_cfg_attr(item: &mut Item, attr: Attribute) {
    match item {
        Item::Fn(f) => f.attrs.insert(0, attr),
        Item::Struct(s) => s.attrs.insert(0, attr),
        Item::Enum(e) => e.attrs.insert(0, attr),
        Item::Impl(i) => i.attrs.insert(0, attr),
        Item::Trait(t) => t.attrs.insert(0, attr),
        Item::TypeAlias(t) => t.attrs.insert(0, attr),
        Item::Const(c) => c.attrs.insert(0, attr),
        Item::Static(s) => s.attrs.insert(0, attr),
        Item::Use(u) => u.attrs.insert(0, attr),
        Item::Mod(m) => m.attrs.insert(0, attr),
        Item::MacroDef(m) => m.attrs.insert(0, attr),
        Item::ExternBlock(e) => e.attrs.insert(0, attr),
        Item::ExternCrate(_) | Item::MacroCall(_, _, _) => {}
    }
}

fn expand_builtin_define_cast(args: &[TokenTree], interner: &mut Interner) -> Option<Vec<Item>> {
    let TokenTree::Token(Token { kind: TokenKind::Kw(Keyword::Unsafe), .. }) = args.first()? else {
        return None;
    };
    let TokenTree::Delimited(Delimiter::Brace, body) = args.get(1)? else {
        return None;
    };

    let mut is_pub = false;
    let mut name = None;
    for tt in body {
        match tt {
            TokenTree::Token(Token { kind: TokenKind::Kw(Keyword::Pub), .. }) => {
                is_pub = true;
            }
            TokenTree::Token(Token { kind: TokenKind::Ident(sym), .. }) => {
                name = Some(*sym);
                break;
            }
            _ => {}
        }
    }
    let name = name?;
    let vis = if is_pub { "pub " } else { "" };
    let src = format!("{}enum {} {{}}", vis, interner.resolve(name));
    let mut parser = Parser::new(&src, interner);
    Some(parser.parse_crate().items)
}

fn parse_dll_exports_spec(args: &[TokenTree], interner: &Interner) -> Option<DllExportsSpec> {
    let mut lib_path = None;
    let mut lib_struct = None;
    let mut init_call = None;
    let mut symbols = None;
    let mut i = 0;

    while i < args.len() {
        let key = token_tree_ident(&args[i], interner)?;
        i += 1;
        if !matches!(args.get(i), Some(TokenTree::Token(Token { kind: TokenKind::Colon, .. }))) {
            return None;
        }
        i += 1;

        match key {
            "lib_path" => {
                lib_path = Some(token_tree_string(args.get(i)?)?);
                i += 1;
            }
            "lib_struct" => {
                lib_struct = Some(token_tree_ident(args.get(i)?, interner)?.to_string());
                i += 1;
            }
            "init_call" => {
                init_call = Some(token_tree_string(args.get(i)?)?);
                i += 1;
            }
            "symbols" => {
                let TokenTree::Delimited(Delimiter::Brace, inner) = args.get(i)? else {
                    return None;
                };
                symbols = Some(parse_dll_export_symbols(inner, interner)?);
                i += 1;
            }
            _ => return None,
        }

        if matches!(args.get(i), Some(TokenTree::Token(Token { kind: TokenKind::Comma, .. }))) {
            i += 1;
        }
    }

    Some(DllExportsSpec {
        lib_path: lib_path?,
        lib_struct: lib_struct?,
        init_call,
        symbols: symbols?,
    })
}

fn parse_dll_export_symbols(tokens: &[TokenTree], interner: &Interner) -> Option<Vec<DllExportSymbol>> {
    let mut symbols = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        if matches!(tokens.get(i), Some(TokenTree::Token(Token { kind: TokenKind::Comma, .. }))) {
            i += 1;
            continue;
        }

        let name = token_tree_ident(tokens.get(i)?, interner)?.to_string();
        i += 1;

        let TokenTree::Delimited(Delimiter::Paren, params) = tokens.get(i)? else {
            return None;
        };
        i += 1;

        if !matches!(tokens.get(i), Some(TokenTree::Token(Token { kind: TokenKind::Arrow, .. }))) {
            return None;
        }
        i += 1;

        let ret_start = i;
        while i < tokens.len() {
            if matches!(tokens.get(i), Some(TokenTree::Token(Token { kind: TokenKind::Comma, .. }))) {
                break;
            }
            i += 1;
        }
        let ret_ty = token_trees_to_string(&tokens[ret_start..i], interner).trim().to_string();
        let param_tys = split_top_level_commas(params)
            .into_iter()
            .filter_map(|chunk| {
                let ty_tokens = strip_named_macro_param(chunk);
                let ty_src = token_trees_to_string(ty_tokens, interner);
                let ty_src = ty_src.trim();
                if ty_src.is_empty() {
                    None
                } else {
                    Some(ty_src.to_string())
                }
            })
            .collect();

        symbols.push(DllExportSymbol {
            name,
            param_tys,
            ret_ty,
        });

        if matches!(tokens.get(i), Some(TokenTree::Token(Token { kind: TokenKind::Comma, .. }))) {
            i += 1;
        }
    }

    Some(symbols)
}

fn split_top_level_commas(tokens: &[TokenTree]) -> Vec<&[TokenTree]> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, tt) in tokens.iter().enumerate() {
        if matches!(tt, TokenTree::Token(Token { kind: TokenKind::Comma, .. })) {
            parts.push(&tokens[start..i]);
            start = i + 1;
        }
    }
    if start < tokens.len() {
        parts.push(&tokens[start..]);
    }
    parts
}

fn strip_named_macro_param<'a>(tokens: &'a [TokenTree]) -> &'a [TokenTree] {
    for (i, tt) in tokens.iter().enumerate() {
        if matches!(tt, TokenTree::Token(Token { kind: TokenKind::Colon, .. })) {
            return &tokens[i + 1..];
        }
    }
    tokens
}

fn token_tree_ident<'a>(tt: &'a TokenTree, interner: &'a Interner) -> Option<&'a str> {
    let TokenTree::Token(Token { kind: TokenKind::Ident(sym), .. }) = tt else {
        return None;
    };
    Some(interner.resolve(*sym))
}

fn token_tree_string(tt: &TokenTree) -> Option<String> {
    let TokenTree::Token(Token { kind: TokenKind::StringLit(s), .. }) = tt else {
        return None;
    };
    Some(s.clone())
}

fn render_dll_exports_source(spec: &DllExportsSpec) -> String {
    let handle_ty = format!("__anyrc_dll_handle_{}", spec.lib_struct);
    let ehdr_ty = format!("__anyrc_elf64_ehdr_{}", spec.lib_struct);
    let phdr_ty = format!("__anyrc_elf64_phdr_{}", spec.lib_struct);
    let dyn_ty = format!("__anyrc_elf64_dyn_{}", spec.lib_struct);
    let sym_ty = format!("__anyrc_elf64_sym_{}", spec.lib_struct);
    let open_failed_fn = format!("__anyrc_log_open_failed_{}", spec.lib_struct);
    let missing_symbol_fn = format!("__anyrc_log_missing_symbol_{}", spec.lib_struct);
    let missing_init_fn = format!("__anyrc_log_missing_init_symbol_{}", spec.lib_struct);
    let hash_fn = format!("__anyrc_elf_hash_{}", spec.lib_struct);
    let cstr_eq_fn = format!("__anyrc_cstr_eq_{}", spec.lib_struct);
    let dl_open_fn = format!("__anyrc_dll_open_{}", spec.lib_struct);
    let dl_sym_fn = format!("__anyrc_dll_sym_{}", spec.lib_struct);

    let mut src = String::new();
    src.push_str("#[repr(C)]\n");
    src.push_str("struct ");
    src.push_str(&ehdr_ty);
    src.push_str(" {\n");
    src.push_str("    e_ident: [u8; 16],\n");
    src.push_str("    e_type: u16,\n");
    src.push_str("    e_machine: u16,\n");
    src.push_str("    e_version: u32,\n");
    src.push_str("    e_entry: u64,\n");
    src.push_str("    e_phoff: u64,\n");
    src.push_str("    e_shoff: u64,\n");
    src.push_str("    e_flags: u32,\n");
    src.push_str("    e_ehsize: u16,\n");
    src.push_str("    e_phentsize: u16,\n");
    src.push_str("    e_phnum: u16,\n");
    src.push_str("    e_shentsize: u16,\n");
    src.push_str("    e_shnum: u16,\n");
    src.push_str("    e_shstrndx: u16,\n");
    src.push_str("}\n\n");
    src.push_str("#[repr(C)]\n");
    src.push_str("struct ");
    src.push_str(&phdr_ty);
    src.push_str(" {\n");
    src.push_str("    p_type: u32,\n");
    src.push_str("    p_flags: u32,\n");
    src.push_str("    p_offset: u64,\n");
    src.push_str("    p_vaddr: u64,\n");
    src.push_str("    p_paddr: u64,\n");
    src.push_str("    p_filesz: u64,\n");
    src.push_str("    p_memsz: u64,\n");
    src.push_str("    p_align: u64,\n");
    src.push_str("}\n\n");
    src.push_str("#[repr(C)]\n");
    src.push_str("struct ");
    src.push_str(&dyn_ty);
    src.push_str(" {\n");
    src.push_str("    d_tag: i64,\n");
    src.push_str("    d_val: u64,\n");
    src.push_str("}\n\n");
    src.push_str("#[repr(C)]\n");
    src.push_str("struct ");
    src.push_str(&sym_ty);
    src.push_str(" {\n");
    src.push_str("    st_name: u32,\n");
    src.push_str("    st_info: u8,\n");
    src.push_str("    st_other: u8,\n");
    src.push_str("    st_shndx: u16,\n");
    src.push_str("    st_value: u64,\n");
    src.push_str("    st_size: u64,\n");
    src.push_str("}\n\n");
    src.push_str("struct ");
    src.push_str(&handle_ty);
    src.push_str(" {\n");
    src.push_str("    base: u64,\n");
    src.push_str("    symtab: *const ");
    src.push_str(&sym_ty);
    src.push_str(",\n");
    src.push_str("    strtab: *const u8,\n");
    src.push_str("    buckets: *const u32,\n");
    src.push_str("    chains: *const u32,\n");
    src.push_str("    nbuckets: u32,\n");
    src.push_str("}\n\n");
    src.push_str("fn ");
    src.push_str(&open_failed_fn);
    src.push_str("(path: &str) {\n");
    src.push_str("    anyos_std::println!(\"[dynlink] open failed: {}\", path);\n");
    src.push_str("}\n\n");
    src.push_str("fn ");
    src.push_str(&missing_symbol_fn);
    src.push_str("(path: &str, sym: &str) {\n");
    src.push_str("    anyos_std::println!(\"[dynlink] missing symbol '{}' in {}\", sym, path);\n");
    src.push_str("}\n\n");
    src.push_str("fn ");
    src.push_str(&missing_init_fn);
    src.push_str("(path: &str, sym: &str) {\n");
    src.push_str("    anyos_std::println!(\"[dynlink] missing init symbol '{}' in {}\", sym, path);\n");
    src.push_str("}\n\n");
    src.push_str("fn ");
    src.push_str(&hash_fn);
    src.push_str("(name: &[u8]) -> u32 {\n");
    src.push_str("    let mut h: u32 = 0;\n");
    src.push_str("    for &b in name {\n");
    src.push_str("        h = (h << 4).wrapping_add(b as u32);\n");
    src.push_str("        let g = h & 0xF000_0000;\n");
    src.push_str("        if g != 0 {\n");
    src.push_str("            h ^= g >> 24;\n");
    src.push_str("        }\n");
    src.push_str("        h &= !g;\n");
    src.push_str("    }\n");
    src.push_str("    h\n");
    src.push_str("}\n\n");
    src.push_str("unsafe fn ");
    src.push_str(&cstr_eq_fn);
    src.push_str("(cstr: *const u8, name: &[u8]) -> bool {\n");
    src.push_str("    for (i, &b) in name.iter().enumerate() {\n");
    src.push_str("        let c = unsafe { *cstr.add(i) };\n");
    src.push_str("        if c != b {\n");
    src.push_str("            return false;\n");
    src.push_str("        }\n");
    src.push_str("    }\n");
    src.push_str("    unsafe { *cstr.add(name.len()) == 0 }\n");
    src.push_str("}\n\n");
    src.push_str("fn ");
    src.push_str(&dl_open_fn);
    src.push_str("(path: &str) -> Option<");
    src.push_str(&handle_ty);
    src.push_str("> {\n");
    src.push_str("    let base = anyos_std::dll::dll_load(path) as u64;\n");
    src.push_str("    if base == 0 {\n");
    src.push_str("        return None;\n");
    src.push_str("    }\n");
    src.push_str("    let ehdr = unsafe { &*(base as *const ");
    src.push_str(&ehdr_ty);
    src.push_str(") };\n");
    src.push_str("    if ehdr.e_ident[0] != 0x7F || ehdr.e_ident[1] != b'E' || ehdr.e_ident[2] != b'L' || ehdr.e_ident[3] != b'F' {\n");
    src.push_str("        return None;\n");
    src.push_str("    }\n");
    src.push_str("    if ehdr.e_ident[4] != 2 || ehdr.e_type != 3 {\n");
    src.push_str("        return None;\n");
    src.push_str("    }\n");
    src.push_str("    let phdr_base = (base + ehdr.e_phoff) as *const ");
    src.push_str(&phdr_ty);
    src.push_str(";\n");
    src.push_str("    let mut dynamic_va: u64 = 0;\n");
    src.push_str("    let mut link_base: u64 = u64::MAX;\n");
    src.push_str("    for i in 0..ehdr.e_phnum as usize {\n");
    src.push_str("        let ph = unsafe { &*phdr_base.add(i) };\n");
    src.push_str("        if ph.p_type == 1 && ph.p_vaddr < link_base {\n");
    src.push_str("            link_base = ph.p_vaddr;\n");
    src.push_str("        }\n");
    src.push_str("        if ph.p_type == 2 {\n");
    src.push_str("            dynamic_va = ph.p_vaddr;\n");
    src.push_str("        }\n");
    src.push_str("    }\n");
    src.push_str("    if dynamic_va == 0 {\n");
    src.push_str("        return None;\n");
    src.push_str("    }\n");
    src.push_str("    let load_bias = if link_base != u64::MAX { base - link_base } else { 0 };\n");
    src.push_str("    dynamic_va += load_bias;\n");
    src.push_str("    let mut symtab_va: u64 = 0;\n");
    src.push_str("    let mut strtab_va: u64 = 0;\n");
    src.push_str("    let mut hash_va: u64 = 0;\n");
    src.push_str("    let dyn_ptr = dynamic_va as *const ");
    src.push_str(&dyn_ty);
    src.push_str(";\n");
    src.push_str("    for i in 0..128 {\n");
    src.push_str("        let d = unsafe { &*dyn_ptr.add(i) };\n");
    src.push_str("        match d.d_tag {\n");
    src.push_str("            6 => symtab_va = d.d_val,\n");
    src.push_str("            5 => strtab_va = d.d_val,\n");
    src.push_str("            4 => hash_va = d.d_val,\n");
    src.push_str("            0 => break,\n");
    src.push_str("            _ => {}\n");
    src.push_str("        }\n");
    src.push_str("    }\n");
    src.push_str("    if symtab_va == 0 || strtab_va == 0 || hash_va == 0 {\n");
    src.push_str("        return None;\n");
    src.push_str("    }\n");
    src.push_str("    let hash_ptr = hash_va as *const u32;\n");
    src.push_str("    let nbuckets = unsafe { *hash_ptr };\n");
    src.push_str("    let buckets = unsafe { hash_ptr.add(2) };\n");
    src.push_str("    let chains = unsafe { buckets.add(nbuckets as usize) };\n");
    src.push_str("    Some(");
    src.push_str(&handle_ty);
    src.push_str(" {\n");
    src.push_str("        base,\n");
    src.push_str("        symtab: symtab_va as *const ");
    src.push_str(&sym_ty);
    src.push_str(",\n");
    src.push_str("        strtab: strtab_va as *const u8,\n");
    src.push_str("        buckets,\n");
    src.push_str("        chains,\n");
    src.push_str("        nbuckets,\n");
    src.push_str("    })\n");
    src.push_str("}\n\n");
    src.push_str("fn ");
    src.push_str(&dl_sym_fn);
    src.push_str("(handle: &");
    src.push_str(&handle_ty);
    src.push_str(", name: &str) -> Option<*const ()> {\n");
    src.push_str("    let h = ");
    src.push_str(&hash_fn);
    src.push_str("(name.as_bytes());\n");
    src.push_str("    let bucket_idx = h % handle.nbuckets;\n");
    src.push_str("    let mut idx = unsafe { *handle.buckets.add(bucket_idx as usize) };\n");
    src.push_str("    while idx != 0 {\n");
    src.push_str("        let sym = unsafe { &*handle.symtab.add(idx as usize) };\n");
    src.push_str("        if sym.st_value != 0 {\n");
    src.push_str("            let sym_name = unsafe { ");
    src.push_str(&cstr_eq_fn);
    src.push_str("(handle.strtab.add(sym.st_name as usize), name.as_bytes()) };\n");
    src.push_str("            if sym_name {\n");
    src.push_str("                return Some(sym.st_value as *const ());\n");
    src.push_str("            }\n");
    src.push_str("        }\n");
    src.push_str("        idx = unsafe { *handle.chains.add(idx as usize) };\n");
    src.push_str("    }\n");
    src.push_str("    None\n");
    src.push_str("}\n\n");
    src.push_str("struct ");
    src.push_str(&spec.lib_struct);
    src.push_str(" {\n");
    src.push_str("    _handle: ");
    src.push_str(&handle_ty);
    src.push_str(",\n");
    for sym in &spec.symbols {
        src.push_str("    ");
        src.push_str(&sym.name);
        src.push_str(": extern \"C\" fn(");
        src.push_str(&sym.param_tys.join(", "));
        src.push_str(") -> ");
        src.push_str(&sym.ret_ty);
        src.push_str(",\n");
    }
    src.push_str("}\n\n");
    src.push_str("static mut LIB: Option<");
    src.push_str(&spec.lib_struct);
    src.push_str("> = None;\n\n");
    src.push_str("fn lib() -> &'static ");
    src.push_str(&spec.lib_struct);
    src.push_str(" {\n");
    src.push_str("    unsafe {\n");
    src.push_str("        match LIB {\n");
    src.push_str("            Some(ref lib) => lib,\n");
    src.push_str("            None => loop {},\n");
    src.push_str("        }\n");
    src.push_str("    }\n");
    src.push_str("}\n\n");
    src.push_str("pub fn init() -> bool {\n");
    src.push_str("    let handle = match ");
    src.push_str(&dl_open_fn);
    src.push_str("(\"");
    src.push_str(&spec.lib_path);
    src.push_str("\") {\n");
    src.push_str("        Some(h) => h,\n");
    src.push_str("        None => {\n");
    src.push_str("            ");
    src.push_str(&open_failed_fn);
    src.push_str("(\"");
    src.push_str(&spec.lib_path);
    src.push_str("\");\n");
    src.push_str("            return false;\n");
    src.push_str("        }\n");
    src.push_str("    };\n\n");
    src.push_str("    unsafe {\n");
    src.push_str("        let lib = ");
    src.push_str(&spec.lib_struct);
    src.push_str(" {\n");
    for sym in &spec.symbols {
        src.push_str("            ");
        src.push_str(&sym.name);
        src.push_str(": {\n");
        src.push_str("                let ptr = match ");
        src.push_str(&dl_sym_fn);
        src.push_str("(&handle, \"");
        src.push_str(&sym.name);
        src.push_str("\") {\n");
        src.push_str("                    Some(p) => p,\n");
        src.push_str("                    None => {\n");
        src.push_str("                        ");
        src.push_str(&missing_symbol_fn);
        src.push_str("(\"");
        src.push_str(&spec.lib_path);
        src.push_str("\", \"");
        src.push_str(&sym.name);
        src.push_str("\");\n");
        src.push_str("                        return false;\n");
        src.push_str("                    }\n");
        src.push_str("                };\n");
        src.push_str("                let func: extern \"C\" fn(");
        src.push_str(&sym.param_tys.join(", "));
        src.push_str(") -> ");
        src.push_str(&sym.ret_ty);
        src.push_str(" = core::mem::transmute_copy(&ptr);\n");
        src.push_str("                func\n");
        src.push_str("            },\n");
    }
    src.push_str("            _handle: handle,\n");
    src.push_str("        };\n");
    if let Some(init_call) = &spec.init_call {
        src.push_str("        if let Some(init_ptr) = ");
        src.push_str(&dl_sym_fn);
        src.push_str("(&lib._handle, \"");
        src.push_str(init_call);
        src.push_str("\") {\n");
        src.push_str("            let init_fn: extern \"C\" fn() = core::mem::transmute_copy(&init_ptr);\n");
        src.push_str("            init_fn();\n");
        src.push_str("        } else {\n");
        src.push_str("            ");
        src.push_str(&missing_init_fn);
        src.push_str("(\"");
        src.push_str(&spec.lib_path);
        src.push_str("\", \"");
        src.push_str(init_call);
        src.push_str("\");\n");
        src.push_str("            LIB = Some(lib);\n");
        src.push_str("            return true;\n");
        src.push_str("        }\n");
    }
    src.push_str("        LIB = Some(lib);\n");
    src.push_str("    }\n");
    src.push_str("    true\n");
    src.push_str("}\n");
    src
}

// ── Pattern matching ──

/// Match macro pattern against input tokens, returning captures.
fn match_pattern(
    pattern: &[TokenTree],
    input: &[TokenTree],
    interner: &Interner,
    captures: &mut HashMap<Symbol, Capture>,
) -> bool {
    let mut pm = PatternMatcher { interner, captures };
    let (p_rest, i_rest) = pm.match_seq(pattern, input);
    p_rest == 0 && i_rest == 0
}

struct PatternMatcher<'a> {
    interner: &'a Interner,
    captures: &'a mut HashMap<Symbol, Capture>,
}

impl<'a> PatternMatcher<'a> {
    /// Match a sequence of pattern TTs against input TTs.
    /// Returns (remaining pattern count, remaining input count).
    fn match_seq(&mut self, pattern: &[TokenTree], input: &[TokenTree]) -> (usize, usize) {
        let mut pi = 0;
        let mut ii = 0;

        while pi < pattern.len() {
            // Check for $( ... )SEP* or $( ... )SEP+
            if pi + 1 < pattern.len() && is_dollar(&pattern[pi]) {
                if let TokenTree::Delimited(Delimiter::Paren, rep_pattern) = &pattern[pi + 1] {
                    // After the $(...) we need separator and kleene op
                    let (sep, kleene, skip) = parse_rep_suffix(&pattern[pi + 2..]);
                    pi += 2 + skip;

                    // Match repetition
                    let mut rep_captures: HashMap<Symbol, Vec<Vec<TokenTree>>> = HashMap::new();
                    let mut count = 0;
                    loop {
                        // Try matching one iteration
                        let mut iter_captures = HashMap::new();
                        let mut iter_matcher = PatternMatcher {
                            interner: self.interner,
                            captures: &mut iter_captures,
                        };
                        let (pr, ir) = iter_matcher.match_seq(rep_pattern, &input[ii..]);
                        if pr != 0 {
                            break; // pattern didn't fully match
                        }
                        let consumed = input[ii..].len() - ir;
                        if consumed == 0 && pr == 0 {
                            // zero-width match, avoid infinite loop
                            break;
                        }
                        ii += consumed;
                        count += 1;

                        // Collect captures from this iteration
                        for (k, v) in iter_captures {
                            if let Capture::Single(tts) = v {
                                rep_captures.entry(k).or_default().push(tts);
                            }
                        }

                        if kleene == '?' {
                            break;
                        }

                        // Try to eat separator
                        if let Some(ref sep_tok) = sep {
                            if ii < input.len() {
                                if let TokenTree::Token(t) = &input[ii] {
                                    if tokens_match(&t.kind, &sep_tok.kind) {
                                        ii += 1;
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }

                    if kleene == '+' && count == 0 {
                        return (pattern.len() - pi, input.len() - ii);
                    }

                    // Store repeated captures
                    for (k, v) in rep_captures {
                        self.captures.insert(k, Capture::Repeated(v));
                    }
                    continue;
                }
            }

            // Check for $name:frag
            if pi + 2 < pattern.len() && is_dollar(&pattern[pi]) {
                if let (TokenTree::Token(name_tok), TokenTree::Token(colon_tok)) =
                    (&pattern[pi + 1], &pattern[pi + 2])
                {
                    if colon_tok.kind == TokenKind::Colon {
                        if let TokenKind::Ident(name_sym) = name_tok.kind {
                            if pi + 3 < pattern.len() {
                                if let TokenTree::Token(frag_tok) = &pattern[pi + 3] {
                                    if let TokenKind::Ident(frag_sym) = frag_tok.kind {
                                        let frag = self.interner.resolve(frag_sym);
                                        if matches!(frag, "expr" | "ident" | "ty" | "pat" | "tt" | "literal" | "stmt" | "block" | "vis") {
                                            pi += 4;
                                            // Capture based on fragment type
                                            let captured = self.capture_fragment(frag, &input[ii..]);
                                            if let Some((tts, consumed)) = captured {
                                                self.captures.insert(name_sym, Capture::Single(tts));
                                                ii += consumed;
                                                continue;
                                            } else {
                                                return (pattern.len() - pi + 4, input.len() - ii);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Check for lone $name (substitution variable in pattern without :frag - shouldn't happen but handle)
            if pi < pattern.len() && is_dollar(&pattern[pi]) {
                // If next is just an ident without colon, skip for now
                if pi + 1 < pattern.len() {
                    if let TokenTree::Token(Token { kind: TokenKind::Ident(_), .. }) = &pattern[pi + 1] {
                        // Check if followed by colon
                        if pi + 2 < pattern.len() {
                            if let TokenTree::Token(Token { kind: TokenKind::Colon, .. }) = &pattern[pi + 2] {
                                // Already handled above, shouldn't reach here
                            }
                        }
                    }
                }
            }

            // Regular token matching
            if ii >= input.len() { return (pattern.len() - pi, 0); }
            match (&pattern[pi], &input[ii]) {
                (TokenTree::Token(pt), TokenTree::Token(it)) => {
                    if !tokens_match(&pt.kind, &it.kind) {
                        return (pattern.len() - pi, input.len() - ii);
                    }
                }
                (TokenTree::Delimited(d1, p_inner), TokenTree::Delimited(d2, i_inner)) if d1 == d2 => {
                    let (pr, ir) = PatternMatcher {
                        interner: self.interner,
                        captures: self.captures,
                    }.match_seq(p_inner, i_inner);
                    if pr != 0 || ir != 0 {
                        return (pattern.len() - pi, input.len() - ii);
                    }
                }
                _ => return (pattern.len() - pi, input.len() - ii),
            }
            pi += 1;
            ii += 1;
        }

        (0, input.len() - ii)
    }

    fn capture_fragment(&self, frag: &str, input: &[TokenTree]) -> Option<(Vec<TokenTree>, usize)> {
        if input.is_empty() { return None; }
        match frag {
            "tt" => {
                Some((vec![input[0].clone()], 1))
            }
            "ident" => {
                if let TokenTree::Token(t) = &input[0] {
                    if matches!(t.kind, TokenKind::Ident(_)) {
                        return Some((vec![input[0].clone()], 1));
                    }
                }
                None
            }
            "literal" => {
                if let TokenTree::Token(t) = &input[0] {
                    if matches!(t.kind, TokenKind::IntLit(..) | TokenKind::FloatLit(_) |
                                TokenKind::StringLit(_) | TokenKind::CharLit(_) |
                                TokenKind::Kw(Keyword::True) | TokenKind::Kw(Keyword::False)) {
                        return Some((vec![input[0].clone()], 1));
                    }
                }
                None
            }
            "block" => {
                if let TokenTree::Delimited(Delimiter::Brace, _) = &input[0] {
                    return Some((vec![input[0].clone()], 1));
                }
                None
            }
            "expr" | "ty" | "pat" | "stmt" => {
                // Greedy: capture as many tokens as possible that form a valid unit.
                // Simple heuristic: take tokens until we hit a comma, semicolon,
                // or closing delimiter that isn't matched.
                let mut depth = 0i32;
                let mut end = 0;
                for (i, tt) in input.iter().enumerate() {
                    match tt {
                        TokenTree::Token(t) => {
                            match t.kind {
                                TokenKind::Comma | TokenKind::Semi | TokenKind::FatArrow if depth == 0 => break,
                                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket if depth == 0 => break,
                                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => depth -= 1,
                                _ => {}
                            }
                        }
                        TokenTree::Delimited(..) => {}
                    }
                    end = i + 1;
                }
                if end == 0 { return None; }
                Some((input[..end].to_vec(), end))
            }
            "vis" => {
                if let TokenTree::Token(t) = &input[0] {
                    if t.kind == TokenKind::Kw(Keyword::Pub) {
                        let mut end = 1;
                        if matches!(input.get(1), Some(TokenTree::Delimited(Delimiter::Paren, _))) {
                            end = 2;
                        }
                        return Some((input[..end].to_vec(), end));
                    }
                }
                Some((Vec::new(), 0))
            }
            _ => None,
        }
    }
}

fn is_dollar(tt: &TokenTree) -> bool {
    matches!(tt, TokenTree::Token(Token { kind: TokenKind::Dollar, .. }))
}

fn parse_rep_suffix(tts: &[TokenTree]) -> (Option<Token>, char, usize) {
    // After $(...), expect optional separator then *, +, or ?
    if tts.is_empty() { return (None, '*', 0); }

    // Check if first token is *, +, or ?
    if let TokenTree::Token(t) = &tts[0] {
        if t.kind == TokenKind::Star { return (None, '*', 1); }
        if t.kind == TokenKind::Plus { return (None, '+', 1); }
        if t.kind == TokenKind::Question { return (None, '?', 1); }
    }

    // Otherwise first is separator, second is *, +, or ?
    if tts.len() >= 2 {
        if let (TokenTree::Token(sep), TokenTree::Token(kleene)) = (&tts[0], &tts[1]) {
            let k = if kleene.kind == TokenKind::Star { '*' }
                    else if kleene.kind == TokenKind::Plus { '+' }
                    else if kleene.kind == TokenKind::Question { '?' }
                    else { '*' };
            return (Some(sep.clone()), k, 2);
        }
    }

    (None, '*', 0)
}

fn tokens_match(a: &TokenKind, b: &TokenKind) -> bool {
    match (a, b) {
        (TokenKind::Ident(s1), TokenKind::Ident(s2)) => s1 == s2,
        (TokenKind::IntLit(a, _), TokenKind::IntLit(b, _)) => a == b,
        (TokenKind::FloatLit(a), TokenKind::FloatLit(b)) => a == b,
        (TokenKind::Kw(a), TokenKind::Kw(b)) => a == b,
        _ => core::mem::discriminant(a) == core::mem::discriminant(b),
    }
}

// ── Substitution ──

fn substitute(body: &[TokenTree], captures: &HashMap<Symbol, Capture>, interner: &Interner) -> Vec<TokenTree> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        // Check for $( ... )SEP* repetition in body
        if i + 1 < body.len() && is_dollar(&body[i]) {
            if let TokenTree::Delimited(Delimiter::Paren, rep_body) = &body[i + 1] {
                let (sep, _kleene, skip) = parse_rep_suffix(&body[i + 2..]);
                i += 2 + skip;

                // Find the repetition count from captures
                let rep_count = find_rep_count(rep_body, captures, interner);
                for iter in 0..rep_count {
                    if iter > 0 {
                        if let Some(ref sep_tok) = sep {
                            out.push(TokenTree::Token(sep_tok.clone()));
                        }
                    }
                    // Substitute with iter-th element of repeated captures
                    let iter_tts = substitute_rep(rep_body, captures, interner, iter);
                    out.extend(iter_tts);
                }
                continue;
            }
            // Check for $name substitution
            if let TokenTree::Token(Token { kind: TokenKind::Ident(name), .. }) = &body[i + 1] {
                if let Some(cap) = captures.get(name) {
                    match cap {
                        Capture::Single(tts) => out.extend(tts.iter().cloned()),
                        Capture::Repeated(reps) => {
                            // In non-repetition context, concat all
                            for r in reps { out.extend(r.iter().cloned()); }
                        }
                    }
                    i += 2;
                    continue;
                }
            }
        }

        // Regular token
        match &body[i] {
            TokenTree::Delimited(d, inner) => {
                let expanded = substitute(inner, captures, interner);
                out.push(TokenTree::Delimited(*d, expanded));
            }
            other => out.push(other.clone()),
        }
        i += 1;
    }
    out
}

fn substitute_rep(body: &[TokenTree], captures: &HashMap<Symbol, Capture>, interner: &Interner, iter: usize) -> Vec<TokenTree> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if i + 1 < body.len() && is_dollar(&body[i]) {
            if let TokenTree::Delimited(Delimiter::Paren, rep_body) = &body[i + 1] {
                let (sep, _kleene, skip) = parse_rep_suffix(&body[i + 2..]);
                i += 2 + skip;

                let rep_count = find_rep_count(rep_body, captures, interner);
                for nested_iter in 0..rep_count {
                    if nested_iter > 0 {
                        if let Some(ref sep_tok) = sep {
                            out.push(TokenTree::Token(sep_tok.clone()));
                        }
                    }
                    out.extend(substitute_rep(rep_body, captures, interner, nested_iter));
                }
                continue;
            }
            if let TokenTree::Token(Token { kind: TokenKind::Ident(name), .. }) = &body[i + 1] {
                if let Some(cap) = captures.get(name) {
                    match cap {
                        Capture::Single(tts) => out.extend(tts.iter().cloned()),
                        Capture::Repeated(reps) => {
                            if iter < reps.len() {
                                out.extend(reps[iter].iter().cloned());
                            }
                        }
                    }
                    i += 2;
                    continue;
                }
            }
        }
        match &body[i] {
            TokenTree::Delimited(d, inner) => {
                let expanded = substitute_rep(inner, captures, interner, iter);
                out.push(TokenTree::Delimited(*d, expanded));
            }
            other => out.push(other.clone()),
        }
        i += 1;
    }
    out
}

fn find_rep_count(rep_body: &[TokenTree], captures: &HashMap<Symbol, Capture>, interner: &Interner) -> usize {
    // Look for $name references in the rep body and find their repetition count
    for i in 0..rep_body.len() {
        if is_dollar(&rep_body[i]) && i + 1 < rep_body.len() {
            if let TokenTree::Token(Token { kind: TokenKind::Ident(name), .. }) = &rep_body[i + 1] {
                if let Some(Capture::Repeated(reps)) = captures.get(name) {
                    return reps.len();
                }
            }
        }
        if let TokenTree::Delimited(_, inner) = &rep_body[i] {
            let n = find_rep_count(inner, captures, interner);
            if n > 0 { return n; }
        }
    }
    // If no repeated capture found, maybe all captures are single - just return 0
    // But for patterns like `$(+ 1)*` where there's no capture ref, use the first
    // repeated capture we can find in the captures map
    for cap in captures.values() {
        if let Capture::Repeated(reps) = cap {
            return reps.len();
        }
    }
    0
}

// ── Expansion entry points ──

fn try_expand_to_expr(def: &MacroDef, args: &[TokenTree], interner: &mut Interner) -> Option<Expr> {
    for rule in &def.rules {
        let mut captures = HashMap::new();
        if match_pattern(&rule.pattern, args, interner, &mut captures) {
            let expanded = substitute(&rule.body, &captures, interner);
            if has_unexpanded_dollar_tts(&expanded) {
                return None;
            }
            let src = token_trees_to_string(&expanded, interner);
            if has_unexpanded_dollar(&src) {
                return None;
            }
            let mut parser = Parser::new(&src, interner);
            return Some(parser.parse_expr());
        }
    }
    None
}

fn try_expand_to_items(def: &MacroDef, args: &[TokenTree], interner: &mut Interner) -> Option<Vec<Item>> {
    for rule in &def.rules {
        let mut captures = HashMap::new();
        if match_pattern(&rule.pattern, args, interner, &mut captures) {
            let expanded = substitute(&rule.body, &captures, interner);
            if has_unexpanded_dollar_tts(&expanded) {
                return None;
            }
            let src = token_trees_to_string(&expanded, interner);
            if has_unexpanded_dollar(&src) {
                return None;
            }
            let mut parser = Parser::new(&src, interner);
            let krate = parser.parse_crate();
            return Some(krate.items);
        }
    }
    None
}

fn has_unexpanded_dollar(src: &str) -> bool {
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            if src[i..].starts_with("$crate") {
                i += "$crate".len();
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

fn has_unexpanded_dollar_tts(tts: &[TokenTree]) -> bool {
    let mut i = 0;
    while i < tts.len() {
        match &tts[i] {
            TokenTree::Token(Token { kind: TokenKind::Dollar, .. }) => {
                if matches!(
                    tts.get(i + 1),
                    Some(TokenTree::Token(Token { kind: TokenKind::Kw(Keyword::Crate), .. }))
                ) {
                    i += 2;
                    continue;
                }
                return true;
            }
            TokenTree::Delimited(_, inner) => {
                if has_unexpanded_dollar_tts(inner) {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

// ── Token tree to string conversion ──

fn token_trees_to_string(tts: &[TokenTree], interner: &Interner) -> String {
    let mut out = String::new();
    for tt in tts {
        if !out.is_empty() { out.push(' '); }
        tt_to_string(tt, interner, &mut out);
    }
    out
}

fn tt_to_string(tt: &TokenTree, interner: &Interner, out: &mut String) {
    match tt {
        TokenTree::Token(tok) => token_to_string(&tok.kind, interner, out),
        TokenTree::Delimited(delim, inner) => {
            out.push(match delim { Delimiter::Paren => '(', Delimiter::Bracket => '[', Delimiter::Brace => '{' });
            for (i, tt) in inner.iter().enumerate() {
                if i > 0 { out.push(' '); }
                tt_to_string(tt, interner, out);
            }
            out.push(match delim { Delimiter::Paren => ')', Delimiter::Bracket => ']', Delimiter::Brace => '}' });
        }
    }
}

fn token_to_string(kind: &TokenKind, interner: &Interner, out: &mut String) {
    use crate::lexer::Keyword;
    match kind {
        TokenKind::Ident(sym) => out.push_str(interner.resolve(*sym)),
        TokenKind::IntLit(n, suffix) => {
            out.push_str(&n.to_string());
            if let Some(suffix) = suffix {
                let suffix_str = match suffix {
                    crate::lexer::IntSuffix::I8 => "i8",
                    crate::lexer::IntSuffix::I16 => "i16",
                    crate::lexer::IntSuffix::I32 => "i32",
                    crate::lexer::IntSuffix::I64 => "i64",
                    crate::lexer::IntSuffix::I128 => "i128",
                    crate::lexer::IntSuffix::Isize => "isize",
                    crate::lexer::IntSuffix::U8 => "u8",
                    crate::lexer::IntSuffix::U16 => "u16",
                    crate::lexer::IntSuffix::U32 => "u32",
                    crate::lexer::IntSuffix::U64 => "u64",
                    crate::lexer::IntSuffix::U128 => "u128",
                    crate::lexer::IntSuffix::Usize => "usize",
                };
                out.push_str(suffix_str);
            }
        }
        TokenKind::FloatLit(f) => out.push_str(&f.to_string()),
        TokenKind::StringLit(s) => { out.push('"'); out.push_str(s); out.push('"'); }
        TokenKind::CharLit(c) => { out.push('\''); out.push(*c); out.push('\''); }
        TokenKind::ByteStringLit(_) => out.push_str("b\"...\""),
        TokenKind::Lifetime(sym) => { out.push('\''); out.push_str(interner.resolve(*sym)); }
        TokenKind::Kw(kw) => out.push_str(match kw {
            Keyword::Fn => "fn", Keyword::Let => "let", Keyword::Mut => "mut",
            Keyword::Pub => "pub", Keyword::Struct => "struct", Keyword::Enum => "enum",
            Keyword::Impl => "impl", Keyword::Trait => "trait", Keyword::Type => "type",
            Keyword::Use => "use", Keyword::Mod => "mod", Keyword::Crate => "crate",
            Keyword::SelfValue => "self", Keyword::SelfType => "Self", Keyword::Super => "super",
            Keyword::As => "as", Keyword::In => "in", Keyword::For => "for",
            Keyword::While => "while", Keyword::Loop => "loop", Keyword::If => "if",
            Keyword::Else => "else", Keyword::Match => "match", Keyword::Return => "return",
            Keyword::Break => "break", Keyword::Continue => "continue", Keyword::Where => "where",
            Keyword::Const => "const", Keyword::Static => "static", Keyword::Unsafe => "unsafe",
            Keyword::Extern => "extern", Keyword::Ref => "ref", Keyword::Move => "move",
            Keyword::True => "true", Keyword::False => "false", Keyword::Dyn => "dyn",
        }),
        TokenKind::Plus => out.push('+'), TokenKind::Minus => out.push('-'),
        TokenKind::Star => out.push('*'), TokenKind::Slash => out.push('/'),
        TokenKind::Percent => out.push('%'), TokenKind::Amp => out.push('&'),
        TokenKind::Pipe => out.push('|'), TokenKind::Caret => out.push('^'),
        TokenKind::Tilde => out.push('~'), TokenKind::Not => out.push('!'),
        TokenKind::Eq => out.push('='), TokenKind::EqEq => out.push_str("=="),
        TokenKind::Ne => out.push_str("!="), TokenKind::Lt => out.push('<'),
        TokenKind::Le => out.push_str("<="), TokenKind::Gt => out.push('>'),
        TokenKind::Ge => out.push_str(">="), TokenKind::AndAnd => out.push_str("&&"),
        TokenKind::OrOr => out.push_str("||"), TokenKind::Shl => out.push_str("<<"),
        TokenKind::Shr => out.push_str(">>"),
        TokenKind::PlusEq => out.push_str("+="), TokenKind::MinusEq => out.push_str("-="),
        TokenKind::StarEq => out.push_str("*="), TokenKind::SlashEq => out.push_str("/="),
        TokenKind::PercentEq => out.push_str("%="), TokenKind::AmpEq => out.push_str("&="),
        TokenKind::PipeEq => out.push_str("|="), TokenKind::CaretEq => out.push_str("^="),
        TokenKind::ShlEq => out.push_str("<<="), TokenKind::ShrEq => out.push_str(">>="),
        TokenKind::Arrow => out.push_str("->"), TokenKind::FatArrow => out.push_str("=>"),
        TokenKind::ColonColon => out.push_str("::"), TokenKind::DotDot => out.push_str(".."),
        TokenKind::DotDotEq => out.push_str("..="),
        TokenKind::LParen => out.push('('), TokenKind::RParen => out.push(')'),
        TokenKind::LBrace => out.push('{'), TokenKind::RBrace => out.push('}'),
        TokenKind::LBracket => out.push('['), TokenKind::RBracket => out.push(']'),
        TokenKind::Semi => out.push(';'), TokenKind::Colon => out.push(':'),
        TokenKind::Comma => out.push(','), TokenKind::Dot => out.push('.'),
        TokenKind::At => out.push('@'), TokenKind::Hash => out.push('#'),
        TokenKind::Question => out.push('?'), TokenKind::Dollar => out.push('$'),
        TokenKind::Eof => {}
    }
}

/// Look up a compile-time environment variable.
/// On anyOS, reads from the process environment via anyos_std::env.
fn lookup_env(name: &str) -> String {
    let mut buf = [0u8; 512];
    let len = anyos_std::env::get(name, &mut buf);
    if len != u32::MAX && (len as usize) <= buf.len() {
        if let Ok(s) = core::str::from_utf8(&buf[..len as usize]) {
            return String::from(s);
        }
    }
    String::new()
}
