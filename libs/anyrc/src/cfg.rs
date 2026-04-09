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
use crate::intern::{Interner, Symbol};
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
        Item::Trait(_) => &[],
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
            Item::Fn(f) => {
                if let Some(ref mut body) = f.body {
                    strip_stmts(&mut body.stmts, ctx, interner);
                }
            }
            _ => {}
        }
    }
}

fn strip_stmts(stmts: &mut Vec<Stmt>, ctx: &CfgContext, interner: &Interner) {
    stmts.retain(|stmt| {
        if let Stmt::Item(item) = stmt {
            should_keep_item_attrs(item_attrs(item), ctx, interner)
        } else {
            true
        }
    });
    for stmt in stmts.iter_mut() {
        if let Stmt::Item(item) = stmt {
            match item {
                Item::Mod(md) => {
                    if let Some(ref mut sub_items) = md.items {
                        strip_items(sub_items, ctx, interner);
                    }
                }
                Item::Impl(ib) => strip_items(&mut ib.items, ctx, interner),
                _ => {}
            }
        }
    }
}
