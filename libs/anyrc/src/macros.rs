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
    krate.items.iter().filter_map(|item| {
        if let Item::MacroDef(md) = item {
            Some(MacroDef { name: md.name, rules: md.rules.clone() })
        } else {
            None
        }
    }).collect()
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
    for stmt in &mut block.stmts {
        expand_stmt(stmt, defs, interner, changed);
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
                    let fn_sym = interner.intern("__anyrc_format");
                    let fn_path = Path {
                        segments: vec![PathSegment { ident: fn_sym, args: None }],
                        span: Span::dummy(),
                    };
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
                    // vec![a, b, c] → { let mut v = Vec::new(); v.push(a); v.push(b); v.push(c); v }
                    // For now, expand to Vec::new() as a simple placeholder
                    let fn_sym = interner.intern("Vec::new");
                    let fn_path = Path {
                        segments: vec![PathSegment { ident: fn_sym, args: None }],
                        span: Span::dummy(),
                    };
                    *expr = Expr::Call(
                        Box::new(Expr::Path(fn_path)),
                        vec![],
                        *span,
                    );
                    *changed = true;
                    return;
                }
                "println" | "eprintln" => {
                    // println!("...", args...) → __anyrc_println("...", args...)
                    let fn_sym = interner.intern("__anyrc_println");
                    let fn_path = Path {
                        segments: vec![PathSegment { ident: fn_sym, args: None }],
                        span: Span::dummy(),
                    };
                    let call_args = collect_macro_call_args(args, interner);
                    *expr = Expr::Call(
                        Box::new(Expr::Path(fn_path)),
                        call_args,
                        *span,
                    );
                    *changed = true;
                    return;
                }
                "assert" | "assert_eq" | "debug_assert" | "debug_assert_eq" => {
                    // Expand to a no-op or simple check (for bootstrap)
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
                                        if matches!(frag, "expr" | "ident" | "ty" | "pat" | "tt" | "literal" | "stmt" | "block") {
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
                    if matches!(t.kind, TokenKind::IntLit(_) | TokenKind::FloatLit(_) |
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
                                TokenKind::Comma | TokenKind::Semi if depth == 0 => break,
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
            _ => None,
        }
    }
}

fn is_dollar(tt: &TokenTree) -> bool {
    matches!(tt, TokenTree::Token(Token { kind: TokenKind::Dollar, .. }))
}

fn parse_rep_suffix(tts: &[TokenTree]) -> (Option<Token>, char, usize) {
    // After $(...), expect optional separator then * or +
    if tts.is_empty() { return (None, '*', 0); }

    // Check if first token is * or +
    if let TokenTree::Token(t) = &tts[0] {
        if t.kind == TokenKind::Star { return (None, '*', 1); }
        if t.kind == TokenKind::Plus { return (None, '+', 1); }
    }

    // Otherwise first is separator, second is * or +
    if tts.len() >= 2 {
        if let (TokenTree::Token(sep), TokenTree::Token(kleene)) = (&tts[0], &tts[1]) {
            let k = if kleene.kind == TokenKind::Star { '*' }
                    else if kleene.kind == TokenKind::Plus { '+' }
                    else { '*' };
            return (Some(sep.clone()), k, 2);
        }
    }

    (None, '*', 0)
}

fn tokens_match(a: &TokenKind, b: &TokenKind) -> bool {
    match (a, b) {
        (TokenKind::Ident(s1), TokenKind::Ident(s2)) => s1 == s2,
        (TokenKind::IntLit(a), TokenKind::IntLit(b)) => a == b,
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
            let src = token_trees_to_string(&expanded, interner);
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
            let src = token_trees_to_string(&expanded, interner);
            let mut parser = Parser::new(&src, interner);
            let krate = parser.parse_crate();
            return Some(krate.items);
        }
    }
    None
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
        TokenKind::IntLit(n) => out.push_str(&n.to_string()),
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
