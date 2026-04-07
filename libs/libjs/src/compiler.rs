//! Compiles JavaScript AST into bytecode.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use crate::ast::*;
use crate::bytecode::{Chunk, Constant, Op, UpvalueRef};
use crate::lexer::Lexer;
use crate::parser::Parser;

/// How a name was resolved during compilation.
enum NameLookup {
    Local(u16),
    Upvalue(u16),
    Global,
}

/// Descriptor for a variable captured from an enclosing function scope.
struct UpvalueDesc {
    name: String,
    /// If true, captures from the enclosing function's local slot `index`.
    /// If false, captures from the enclosing function's upvalue slot `index`.
    is_local: bool,
    index: u16,
}

/// Entry on the label stack, tracking forward jumps for `break label`
/// and `continue label` (ES2023 §14.13 Labelled Statements).
struct LabelEntry {
    name: String,
    /// Forward-jump indices for `break label` — patched to after the labeled stmt.
    break_jumps: Vec<usize>,
    /// Forward-jump indices for `continue label` — patched to the loop head.
    continue_jumps: Vec<usize>,
    /// If the labeled statement wraps a loop, this is the continue target
    /// (loop head offset) once known.  `continue label` on a non-loop label
    /// is a syntax error per spec, but we gracefully ignore it at runtime.
    continue_target: Option<usize>,
    /// True if the labeled body is an iteration statement (while/do-while/for/for-in/for-of).
    is_iteration: bool,
    /// Set to true when the entry needs its continue target patched by the
    /// direct child for-loop.  Cleared after the first patch to prevent
    /// inner nested for-loops from overwriting the target.
    needs_continue_patch: bool,
}

/// Compiler state for a single scope/function.
struct Scope {
    chunk: Chunk,
    locals: Vec<Local>,
    /// Upvalues captured by this function from enclosing function scopes.
    upvalues: Vec<UpvalueDesc>,
    /// Break target offsets to patch (forward jumps).
    break_jumps: Vec<usize>,
    /// Continue forward-jump instruction indices to patch (for `for` loops
    /// where the update position is unknown until after the body).
    continue_jumps: Vec<usize>,
    /// Continue target offset for loops where the target is known before the
    /// body is compiled (while, do-while, for-in, for-of).
    continue_target: Option<usize>,
    scope_depth: u32,
    /// Stack of finally-block statement lists that must run before any `return`
    /// inside the try body (innermost last).
    pending_finallies: Vec<Vec<Stmt>>,
    /// Stack of active labels (ES2023 §14.13).  Pushed on entry to a
    /// `Labeled` statement, popped on exit.  `break label` / `continue label`
    /// search this stack to find the right jump target.
    label_stack: Vec<LabelEntry>,
    /// Set by for-loop compilation to the update-step offset so that
    /// `compile_labeled` can patch `continue label` forward-jumps.
    last_for_continue_pos: Option<usize>,
}

struct Local {
    name: String,
    depth: u32,
}

impl Scope {
    fn new() -> Self {
        Scope {
            chunk: Chunk::new(),
            locals: Vec::new(),
            upvalues: Vec::new(),
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            continue_target: None,
            scope_depth: 0,
            pending_finallies: Vec::new(),
            label_stack: Vec::new(),
            last_for_continue_pos: None,
        }
    }

    fn resolve_local(&self, name: &str) -> Option<u16> {
        for (i, local) in self.locals.iter().enumerate().rev() {
            if local.name == name {
                return Some(i as u16);
            }
        }
        None
    }

    fn add_local(&mut self, name: String) -> u16 {
        let idx = self.locals.len() as u16;
        self.locals.push(Local {
            name,
            depth: self.scope_depth,
        });
        if idx + 1 > self.chunk.local_count {
            self.chunk.local_count = idx + 1;
        }
        idx
    }
}

pub struct Compiler {
    scopes: Vec<Scope>,
    /// Set to `true` while compiling a top-level `var` binding so that
    /// `bind_ident` and `compile_pattern_binding` emit StoreGlobal instead
    /// of StoreLocal.  Mirrors JavaScript's var-hoisting-to-global behaviour.
    binding_is_global: bool,
    /// True if the current compilation is in strict mode (`"use strict"` directive).
    pub is_strict: bool,
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            scopes: Vec::new(),
            is_strict: false,
            binding_is_global: false,
        }
    }

    /// Returns true when we are at the outermost (global) function scope,
    /// i.e. not inside any compiled function body.
    fn is_global_scope(&self) -> bool {
        self.scopes.len() == 1
    }

    /// Set the continue target for the current loop AND propagate it to
    /// any enclosing label entry that wraps this loop (for `continue label`).
    fn set_continue_target(&mut self, target: usize) {
        self.scope_mut().continue_target = Some(target);
        // If the top of the label stack is an iteration label, set its target too.
        if let Some(entry) = self.scope_mut().label_stack.last_mut() {
            if entry.is_iteration {
                entry.continue_target = Some(target);
            }
        }
    }

    /// Emit a variable binding for `name`.  If `self.binding_is_global` the
    /// variable is stored in the global environment (StoreGlobal); otherwise
    /// it is allocated as a stack local (StoreLocal).  Either way the value
    /// is popped from the stack afterwards.
    fn bind_ident(&mut self, name: &str) {
        if self.binding_is_global {
            let ci = self.add_const(Constant::String(name.to_string()));
            self.emit(Op::StoreGlobal(ci));
            self.emit(Op::Pop);
        } else {
            let slot = self.scope_mut().add_local(name.to_string());
            self.emit(Op::StoreLocal(slot));
            self.emit(Op::Pop);
        }
    }

    /// Like `bind_ident`, but reuses an existing local slot if available.
    /// Used for `var` declarations which may have been pre-allocated by
    /// var-hoisting.  `let`/`const` must NOT use this (they need fresh slots
    /// for block scoping and TDZ).
    fn bind_ident_var(&mut self, name: &str) {
        if self.binding_is_global {
            let ci = self.add_const(Constant::String(name.to_string()));
            self.emit(Op::StoreGlobal(ci));
            self.emit(Op::Pop);
        } else {
            let slot = self
                .scope()
                .resolve_local(name)
                .unwrap_or_else(|| self.scope_mut().add_local(name.to_string()));
            self.emit(Op::StoreLocal(slot));
            self.emit(Op::Pop);
        }
    }

    /// Compile a program into a top-level chunk.
    pub fn compile(&mut self, program: &Program) -> Chunk {
        self.compile_program(program, false)
    }

    /// Compile for eval(): the last expression statement's value is returned
    /// instead of being discarded (ES2023 §19.2.1.1 PerformEval).
    pub fn compile_eval(&mut self, program: &Program) -> Chunk {
        self.compile_program(program, true)
    }

    fn compile_program(&mut self, program: &Program, is_eval: bool) -> Chunk {
        self.scopes.push(Scope::new());

        // Detect "use strict" directive at the beginning of the program
        if let Some(Stmt::Expr(Expr::String(ref s))) = program.body.first() {
            if s == "use strict" {
                self.is_strict = true;
            }
        }

        // ES2023 §10.2.1 — hoist function declarations: compile all top-level
        // FunctionDecl statements first so they are available before any other
        // code executes (function hoisting in the global scope).
        for stmt in &program.body {
            if let Stmt::FunctionDecl { .. } = stmt {
                self.compile_stmt(stmt);
            }
        }

        let body_len = program.body.len();
        for (i, stmt) in program.body.iter().enumerate() {
            // Skip FunctionDecl — already compiled above.
            if let Stmt::FunctionDecl { .. } = stmt {
                continue;
            }

            let is_last = i == body_len - 1;
            if is_eval && is_last {
                // For eval: if the last statement is an expression statement,
                // compile it without popping so the value is returned.
                if let Stmt::Expr(expr) = stmt {
                    self.compile_expr(expr);
                    self.emit(Op::Return);
                    let mut chunk = self.scopes.pop().unwrap().chunk;
                    chunk.strict = self.is_strict;
                    return chunk;
                }
            }
            self.compile_stmt(stmt);
        }
        // Implicit return undefined
        self.emit(Op::LoadUndefined);
        self.emit(Op::Return);
        let mut chunk = self.scopes.pop().unwrap().chunk;
        chunk.strict = self.is_strict;
        chunk
    }

    fn scope(&self) -> &Scope {
        self.scopes.last().unwrap()
    }

    fn scope_mut(&mut self) -> &mut Scope {
        self.scopes.last_mut().unwrap()
    }

    fn emit(&mut self, op: Op) -> usize {
        self.scope_mut().chunk.emit(op)
    }

    fn add_const(&mut self, c: Constant) -> u16 {
        self.scope_mut().chunk.add_const(c)
    }

    fn offset(&self) -> usize {
        self.scope().chunk.offset()
    }

    fn patch_jump(&mut self, idx: usize) {
        self.scope_mut().chunk.patch_jump(idx);
    }

    fn patch_jump_to_pos(&mut self, idx: usize, pos: usize) {
        self.scope_mut().chunk.patch_jump_to_pos(idx, pos);
    }

    // ── Upvalue resolution ──

    /// Walk outer function scopes starting at `scope_idx - 1` looking for `name`.
    /// Returns the upvalue index in `scopes[scope_idx]` if found, None otherwise.
    /// Does not look into `scopes[0]` (the top-level script scope) as a local
    /// capture target — its `var` bindings are globals, but `let`/`const` there
    /// can still be captured.
    /// Collect all `var`-declared names from a statement list (for hoisting).
    /// Recurses into blocks, if/else, for, while, switch, try/catch etc.
    /// Does NOT recurse into nested function bodies (they have their own scope).
    fn collect_var_names(stmts: &[Stmt], out: &mut Vec<String>) {
        for stmt in stmts {
            Self::collect_var_names_stmt(stmt, out);
        }
    }

    fn collect_var_names_stmt(stmt: &Stmt, out: &mut Vec<String>) {
        match stmt {
            Stmt::VarDecl { kind, decls } if *kind == VarKind::Var => {
                for decl in decls {
                    Self::collect_pattern_names(&decl.name, out);
                }
            }
            // Function declarations are also hoisted (ES2023 §10.2.11 step 28).
            Stmt::FunctionDecl { name, .. } => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
            Stmt::Block(stmts) => {
                Self::collect_var_names(stmts, out);
            }
            Stmt::If {
                consequent,
                alternate,
                ..
            } => {
                Self::collect_var_names_stmt(consequent, out);
                if let Some(alt) = alternate {
                    Self::collect_var_names_stmt(alt, out);
                }
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } => {
                Self::collect_var_names_stmt(body, out);
            }
            Stmt::For { init, body, .. } => {
                if let Some(init) = init {
                    if let ForInit::VarDecl { kind, decls } = init.as_ref() {
                        if *kind == VarKind::Var {
                            for decl in decls {
                                Self::collect_pattern_names(&decl.name, out);
                            }
                        }
                    }
                }
                Self::collect_var_names_stmt(body, out);
            }
            Stmt::ForIn { left, body, .. } | Stmt::ForOf { left, body, .. } => {
                if let ForInit::VarDecl { kind, decls } = left.as_ref() {
                    if *kind == VarKind::Var {
                        for decl in decls {
                            Self::collect_pattern_names(&decl.name, out);
                        }
                    }
                }
                Self::collect_var_names_stmt(body, out);
            }
            Stmt::Switch { cases, .. } => {
                for case in cases {
                    Self::collect_var_names(&case.consequent, out);
                }
            }
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                Self::collect_var_names(block, out);
                if let Some(c) = catch {
                    Self::collect_var_names(&c.body, out);
                }
                if let Some(f) = finally {
                    Self::collect_var_names(f, out);
                }
            }
            Stmt::Labeled { body, .. } => {
                Self::collect_var_names_stmt(body, out);
            }
            // Function declarations are hoisted separately (already handled).
            // Do NOT recurse into function bodies.
            _ => {}
        }
    }

    fn collect_pattern_names(pattern: &Pattern, out: &mut Vec<String>) {
        match pattern {
            Pattern::Ident(name) => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
            Pattern::Array(elements) => {
                for elem in elements {
                    if let Some(p) = elem {
                        Self::collect_pattern_names(p, out);
                    }
                }
            }
            Pattern::Object(props) => {
                for prop in props {
                    Self::collect_pattern_names(&prop.value, out);
                }
            }
            Pattern::Assign(inner, _) => {
                Self::collect_pattern_names(inner, out);
            }
            Pattern::Rest(inner) => {
                Self::collect_pattern_names(inner, out);
            }
        }
    }

    fn resolve_upvalue_in_scope(&mut self, scope_idx: usize, name: &str) -> Option<u16> {
        if scope_idx == 0 {
            return None; // nothing above global scope
        }
        // Try as a direct local in the immediately enclosing scope.
        if let Some(local_slot) = self.scopes[scope_idx - 1].resolve_local(name) {
            return Some(self.add_upvalue(scope_idx, name, true, local_slot));
        }
        // Recurse: try as an upvalue of the immediately enclosing scope.
        if let Some(outer_uv) = self.resolve_upvalue_in_scope(scope_idx - 1, name) {
            return Some(self.add_upvalue(scope_idx, name, false, outer_uv));
        }
        None
    }

    /// Add (or deduplicate) an upvalue descriptor in `scopes[scope_idx]`.
    fn add_upvalue(&mut self, scope_idx: usize, name: &str, is_local: bool, index: u16) -> u16 {
        for (i, uv) in self.scopes[scope_idx].upvalues.iter().enumerate() {
            if uv.name == name {
                return i as u16;
            }
        }
        let idx = self.scopes[scope_idx].upvalues.len() as u16;
        self.scopes[scope_idx].upvalues.push(UpvalueDesc {
            name: String::from(name),
            is_local,
            index,
        });
        idx
    }

    /// Resolve `name` from the current (innermost) scope, returning how to access it.
    fn resolve_name(&mut self, name: &str) -> NameLookup {
        if let Some(slot) = self.scopes.last().unwrap().resolve_local(name) {
            return NameLookup::Local(slot);
        }
        let current = self.scopes.len() - 1;
        if current >= 1 {
            if let Some(uv_idx) = self.resolve_upvalue_in_scope(current, name) {
                return NameLookup::Upvalue(uv_idx);
            }
        }
        NameLookup::Global
    }

    /// Emit a load for `name` using the appropriate opcode.
    fn emit_load_name(&mut self, name: &str) {
        match self.resolve_name(name) {
            NameLookup::Local(slot) => {
                self.emit(Op::LoadLocal(slot));
            }
            NameLookup::Upvalue(idx) => {
                self.emit(Op::LoadUpvalue(idx));
            }
            NameLookup::Global => {
                let ci = self.add_const(Constant::String(name.to_string()));
                self.emit(Op::LoadGlobal(ci));
            }
        }
    }

    /// Emit a store for `name` (leaves value on stack, used for assignment expressions).
    fn emit_store_name(&mut self, name: &str) {
        match self.resolve_name(name) {
            NameLookup::Local(slot) => {
                self.emit(Op::Dup);
                self.emit(Op::StoreLocal(slot));
                self.emit(Op::Pop);
            }
            NameLookup::Upvalue(idx) => {
                self.emit(Op::Dup);
                self.emit(Op::StoreUpvalue(idx));
                self.emit(Op::Pop);
            }
            NameLookup::Global => {
                let ci = self.add_const(Constant::String(name.to_string()));
                self.emit(Op::Dup);
                self.emit(Op::StoreGlobal(ci));
                self.emit(Op::Pop);
            }
        }
    }

    // ── Statements ──

    fn compile_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Expr(expr) => {
                self.compile_expr(expr);
                self.emit(Op::Pop);
            }
            Stmt::VarDecl { kind, decls } => {
                // In a browser, top-level var/let/const all create bindings
                // visible to subsequent <script> tags.  We mirror this by
                // storing ALL top-level declarations as globals so that
                // separate eval() calls (one per <script>) share them.
                let is_global = self.is_global_scope();
                let is_var = *kind == VarKind::Var;
                for decl in decls {
                    self.compile_var_decl(decl, is_global, is_var);
                }
            }
            Stmt::Block(stmts) => {
                self.begin_scope();
                for s in stmts {
                    self.compile_stmt(s);
                }
                self.end_scope();
            }
            Stmt::If {
                condition,
                consequent,
                alternate,
            } => {
                self.compile_expr(condition);
                let else_jump = self.emit(Op::JumpIfFalse(0));
                self.compile_stmt(consequent);
                if let Some(alt) = alternate {
                    let end_jump = self.emit(Op::Jump(0));
                    self.patch_jump(else_jump);
                    self.compile_stmt(alt);
                    self.patch_jump(end_jump);
                } else {
                    self.patch_jump(else_jump);
                }
            }
            Stmt::While { condition, body } => {
                let loop_start = self.offset();
                let old_continue = self.scope_mut().continue_target.take();
                self.set_continue_target(loop_start);
                let old_breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);

                self.compile_expr(condition);
                let exit_jump = self.emit(Op::JumpIfFalse(0));
                self.compile_stmt(body);
                let back = loop_start as i32 - self.offset() as i32 - 1;
                self.emit(Op::Jump(back));
                self.patch_jump(exit_jump);

                // Patch breaks
                let breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);
                for b in breaks {
                    self.patch_jump(b);
                }
                self.scope_mut().break_jumps = old_breaks;
                self.scope_mut().continue_target = old_continue;
            }
            Stmt::DoWhile { body, condition } => {
                let loop_start = self.offset();
                let old_continue = self.scope_mut().continue_target.take();
                let old_breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);

                let cond_target = self.offset(); // will be updated
                self.set_continue_target(cond_target);

                self.compile_stmt(body);

                // Update continue target to point here (condition)
                let cond_pos = self.offset();
                self.set_continue_target(cond_pos);

                self.compile_expr(condition);
                let back = loop_start as i32 - self.offset() as i32 - 1;
                self.emit(Op::JumpIfTrue(back));

                let breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);
                for b in breaks {
                    self.patch_jump(b);
                }
                self.scope_mut().break_jumps = old_breaks;
                self.scope_mut().continue_target = old_continue;
            }
            Stmt::For {
                init,
                test,
                update,
                body,
            } => {
                self.begin_scope();
                // Track which locals are `let`/`const` from the for-init so we can
                // clone them each iteration (per-iteration let binding for closures).
                let mut let_slots: Vec<u16> = Vec::new();
                if let Some(init) = init {
                    match init.as_ref() {
                        ForInit::VarDecl { kind, decls } => {
                            let is_global = *kind == VarKind::Var && self.is_global_scope();
                            let for_is_var = *kind == VarKind::Var;
                            if *kind != VarKind::Var && !is_global {
                                // Record slot indices before and after to find new let/const bindings.
                                let before = self.scope().locals.len() as u16;
                                for d in decls {
                                    self.compile_var_decl(d, false, false);
                                }
                                let after = self.scope().locals.len() as u16;
                                for slot in before..after {
                                    let_slots.push(slot);
                                }
                            } else {
                                for d in decls {
                                    self.compile_var_decl(d, is_global, for_is_var);
                                }
                            }
                        }
                        ForInit::Expr(e) => {
                            self.compile_expr(e);
                            self.emit(Op::Pop);
                        }
                    }
                }

                let loop_start = self.offset();
                let old_continue = self.scope_mut().continue_target.take();
                let old_continue_jumps = core::mem::take(&mut self.scope_mut().continue_jumps);
                let old_breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);
                // continue_target stays None during body so Continue emits forward jumps.

                let exit_jump = if let Some(test) = test {
                    self.compile_expr(test);
                    Some(self.emit(Op::JumpIfFalse(0)))
                } else {
                    None
                };

                self.compile_stmt(body);

                // Patch all forward continue-jumps to here (before update).
                let continue_pos = self.offset();
                let cont_jumps = core::mem::take(&mut self.scope_mut().continue_jumps);
                for cj in &cont_jumps {
                    self.patch_jump_to_pos(*cj, continue_pos);
                }
                // Save the continue position so that `compile_labeled` can
                // patch `continue label` forward-jumps for labeled for-loops.
                self.scope_mut().last_for_continue_pos = Some(continue_pos);

                // Clone let/const bindings AFTER body, BEFORE update so that each
                // iteration's closures capture a pre-increment snapshot of the variable.
                // The clone creates a fresh cell for the NEXT iteration; the update
                // modifies this fresh cell, leaving the captured (body-era) cell intact.
                for slot in &let_slots {
                    self.emit(Op::CloneLocal(*slot));
                }

                if let Some(update) = update {
                    self.compile_expr(update);
                    self.emit(Op::Pop);
                }

                let back = loop_start as i32 - self.offset() as i32 - 1;
                self.emit(Op::Jump(back));

                if let Some(ej) = exit_jump {
                    self.patch_jump(ej);
                }

                let breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);
                for b in breaks {
                    self.patch_jump(b);
                }
                self.scope_mut().break_jumps = old_breaks;
                self.scope_mut().continue_jumps = old_continue_jumps;
                self.scope_mut().continue_target = old_continue;
                self.end_scope();
            }
            Stmt::ForIn { left, right, body } => {
                self.compile_for_in_of(left, right, body, false);
            }
            Stmt::ForOf { left, right, body } => {
                self.compile_for_in_of(left, right, body, true);
            }
            Stmt::Return(val) => {
                if let Some(v) = val {
                    self.compile_expr(v);
                } else {
                    self.emit(Op::LoadUndefined);
                }
                // Inline any pending finally blocks before returning (innermost first).
                // Temporarily clear pending_finallies to prevent infinite recursion
                // when the finally block itself contains a `return` statement.
                let finallies = self.scope().pending_finallies.clone();
                self.scope_mut().pending_finallies.clear();
                for fin in finallies.iter().rev() {
                    for s in fin {
                        self.compile_stmt(s);
                    }
                }
                self.emit(Op::Return);
            }
            Stmt::Break(ref label) => {
                if let Some(label_name) = label {
                    // ES2023 §14.9.1 — `break label;` targets the LabelledStatement.
                    // Search the label stack for the matching label.
                    let idx = self.emit(Op::Jump(0));
                    let stack = &mut self.scope_mut().label_stack;
                    if let Some(entry) = stack.iter_mut().rev().find(|e| &e.name == label_name) {
                        entry.break_jumps.push(idx);
                    } else {
                        // Label not found on label_stack — fall back to enclosing
                        // loop/switch (handles `break label` where the label wraps
                        // a loop that directly manages break_jumps).
                        self.scope_mut().break_jumps.push(idx);
                    }
                } else {
                    // Unlabeled `break` — targets nearest loop/switch.
                    let idx = self.emit(Op::Jump(0));
                    self.scope_mut().break_jumps.push(idx);
                }
            }
            Stmt::Continue(ref label) => {
                if let Some(label_name) = label {
                    // ES2023 §14.8.1 — `continue label;` targets the IterationStatement
                    // labeled by `label`.  Search the label stack.
                    let stack = &self.scope().label_stack;
                    let found = stack.iter().rev().find(|e| &e.name == label_name);
                    if let Some(entry) = found {
                        if let Some(target) = entry.continue_target {
                            let back = target as i32 - self.offset() as i32 - 1;
                            self.emit(Op::Jump(back));
                        } else {
                            // Continue target not yet known (e.g., for-loop update).
                            // Emit forward jump; it will be patched by the label entry.
                            let idx = self.emit(Op::Jump(0));
                            let stack = &mut self.scope_mut().label_stack;
                            if let Some(entry) =
                                stack.iter_mut().rev().find(|e| &e.name == label_name)
                            {
                                entry.continue_jumps.push(idx);
                            }
                        }
                    } else {
                        // Fallback: treat as unlabeled continue.
                        if let Some(target) = self.scope().continue_target {
                            let back = target as i32 - self.offset() as i32 - 1;
                            self.emit(Op::Jump(back));
                        } else {
                            let idx = self.emit(Op::Jump(0));
                            self.scope_mut().continue_jumps.push(idx);
                        }
                    }
                } else {
                    // Unlabeled `continue` — targets nearest iteration statement.
                    if let Some(target) = self.scope().continue_target {
                        let back = target as i32 - self.offset() as i32 - 1;
                        self.emit(Op::Jump(back));
                    } else {
                        let idx = self.emit(Op::Jump(0));
                        self.scope_mut().continue_jumps.push(idx);
                    }
                }
            }
            Stmt::Switch {
                discriminant,
                cases,
            } => {
                self.compile_switch(discriminant, cases);
            }
            Stmt::Throw(expr) => {
                self.compile_expr(expr);
                self.emit(Op::Throw);
            }
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                self.compile_try(block, catch, finally);
            }
            Stmt::FunctionDecl {
                name,
                params,
                body,
                is_async,
                is_generator,
            } => {
                if self.is_global_scope() {
                    // At the global scope function declarations are global bindings.
                    self.compile_function_gen(Some(name), params, body, *is_async, *is_generator);
                    let ci = self.add_const(Constant::String(name.clone()));
                    self.emit(Op::StoreGlobal(ci));
                    self.emit(Op::Pop);
                } else {
                    // In non-global scopes, function declarations are fully hoisted:
                    // the closure was already compiled and stored during the function
                    // value hoisting pass in compile_function_impl.  Emit a no-op here.
                    self.emit(Op::Nop);
                }
            }
            Stmt::ClassDecl {
                name,
                super_class,
                body,
            } => {
                self.compile_class(Some(name), super_class, body);
                if self.is_global_scope() {
                    let ci = self.add_const(Constant::String(name.clone()));
                    self.emit(Op::StoreGlobal(ci));
                    self.emit(Op::Pop);
                } else {
                    let slot = self
                        .scope()
                        .resolve_local(name)
                        .unwrap_or_else(|| self.scope_mut().add_local(name.clone()));
                    self.emit(Op::StoreLocal(slot));
                    self.emit(Op::Pop);
                }
            }
            Stmt::Labeled { label, body } => {
                self.compile_labeled(label, body);
            }
            Stmt::Empty | Stmt::Debugger => {
                self.emit(Op::Nop);
            }
            Stmt::Import { specifiers, source } => {
                // Import: load module from registry, bind specifiers as globals.
                // We compile as: __import__('source') → module namespace object,
                // then extract each specifier and bind to a global.
                let src_ci = self.add_const(Constant::String(source.clone()));
                let import_ci = self.add_const(Constant::String(String::from("__import__")));
                self.emit(Op::LoadGlobal(import_ci));
                self.emit(Op::LoadConst(src_ci));
                self.emit(Op::Call(1));
                // Stack: [module_ns]
                for spec in specifiers {
                    match spec {
                        ImportSpecifier::Default(local) => {
                            self.emit(Op::Dup);
                            let key = self.add_const(Constant::String(String::from("default")));
                            self.emit(Op::GetPropNamed(key));
                            let ci = self.add_const(Constant::String(local.clone()));
                            self.emit(Op::StoreGlobal(ci));
                            self.emit(Op::Pop);
                        }
                        ImportSpecifier::Named { imported, local } => {
                            self.emit(Op::Dup);
                            let key = self.add_const(Constant::String(imported.clone()));
                            self.emit(Op::GetPropNamed(key));
                            let ci = self.add_const(Constant::String(local.clone()));
                            self.emit(Op::StoreGlobal(ci));
                            self.emit(Op::Pop);
                        }
                        ImportSpecifier::Namespace(local) => {
                            self.emit(Op::Dup);
                            let ci = self.add_const(Constant::String(local.clone()));
                            self.emit(Op::StoreGlobal(ci));
                            self.emit(Op::Pop);
                        }
                    }
                }
                self.emit(Op::Pop); // pop module_ns
            }
            Stmt::Export(decl) => {
                match decl {
                    ExportDecl::Default(expr) => {
                        // export default expr → __exports__.default = expr
                        let exports_ci =
                            self.add_const(Constant::String(String::from("__exports__")));
                        self.emit(Op::LoadGlobal(exports_ci));
                        self.compile_expr(expr);
                        let default_ci = self.add_const(Constant::String(String::from("default")));
                        self.emit(Op::SetPropNamed(default_ci));
                        self.emit(Op::Pop); // pop assigned value
                        self.emit(Op::Pop); // pop exports obj
                    }
                    ExportDecl::Decl(stmt) => {
                        // export function/class/var — compile the declaration normally.
                        // The exported names become available via the global scope.
                        self.compile_stmt(stmt);
                    }
                    ExportDecl::Named(specifiers) => {
                        // export { a, b as c } — alias existing globals into __exports__
                        for spec in specifiers {
                            let exports_ci =
                                self.add_const(Constant::String(String::from("__exports__")));
                            self.emit(Op::LoadGlobal(exports_ci));
                            let local_ci = self.add_const(Constant::String(spec.local.clone()));
                            self.emit(Op::LoadGlobal(local_ci));
                            let exported_ci =
                                self.add_const(Constant::String(spec.exported.clone()));
                            self.emit(Op::SetPropNamed(exported_ci));
                            self.emit(Op::Pop);
                            self.emit(Op::Pop);
                        }
                    }
                    ExportDecl::ReExport {
                        specifiers: _,
                        source: _,
                    } => {
                        // Re-exports are resolved at module-linking time, not compilation.
                        self.emit(Op::Nop);
                    }
                }
            }
        }
    }

    fn compile_var_decl(&mut self, decl: &VarDeclarator, is_global_var: bool, is_var: bool) {
        let prev = self.binding_is_global;
        self.binding_is_global = is_global_var;
        match &decl.name {
            Pattern::Ident(name) => {
                // For let/const (non-var, non-global): ensure the local slot exists
                // BEFORE compiling the initializer.  This is essential so that the
                // variable name is visible during the initializer compilation — e.g.
                //   const r = function(x) { return r(x-1); };
                // Without this, the inner function can't capture `r` as an upvalue
                // because the local doesn't exist yet when the function expr is compiled.
                // (For `var`, hoisting already handles this in compile_function_impl.)
                // The slot may already exist if pre-allocated by function-level
                // let/const name scanning (for function declaration hoisting).
                let prealloc_slot = if !is_var && !is_global_var {
                    let slot = self
                        .scope()
                        .resolve_local(name)
                        .unwrap_or_else(|| self.scope_mut().add_local(name.clone()));
                    Some(slot)
                } else {
                    None
                };
                if let Some(init) = &decl.init {
                    self.compile_expr_with_name(init, name);
                } else {
                    self.emit(Op::LoadUndefined);
                }
                if let Some(slot) = prealloc_slot {
                    // Local was pre-allocated; just store into it.
                    self.emit(Op::StoreLocal(slot));
                    self.emit(Op::Pop);
                } else {
                    let name_clone = name.clone();
                    if is_var {
                        self.bind_ident_var(&name_clone);
                    } else {
                        self.bind_ident(&name_clone);
                    }
                }
            }
            Pattern::Array(elements) => {
                if let Some(init) = &decl.init {
                    self.compile_expr(init);
                } else {
                    self.emit(Op::LoadUndefined);
                }
                self.compile_array_destructure(elements);
            }
            Pattern::Object(props) => {
                if let Some(init) = &decl.init {
                    self.compile_expr(init);
                } else {
                    self.emit(Op::LoadUndefined);
                }
                self.compile_object_destructure(props);
            }
            Pattern::Assign(pat, def) => {
                if let Some(init) = &decl.init {
                    self.compile_expr(init);
                } else {
                    self.compile_expr(def);
                }
                self.compile_pattern_binding(pat);
            }
            Pattern::Rest(_) => {
                // Bare rest in var decl — just load undefined (degenerate case).
                self.emit(Op::LoadUndefined);
                self.emit(Op::Pop);
            }
        }
        self.binding_is_global = prev;
    }

    fn compile_array_destructure(&mut self, elements: &[Option<Pattern>]) {
        // Stack: [..., iterable]
        // Convert to iterator using GetIterator (calls Symbol.iterator).
        self.emit(Op::GetIterator);
        // Stack: [..., iterator]

        for elem in elements.iter() {
            match elem {
                None => {
                    // Elision: [,] — advance iterator without binding
                    self.emit(Op::Dup); // [..., iter, iter]
                    self.emit(Op::IterNext); // [..., iter, value, has_more]
                    self.emit(Op::Pop); // [..., iter, value]
                    self.emit(Op::Pop); // [..., iter]
                }
                Some(Pattern::Rest(inner)) => {
                    // Rest element: [...rest] — collect remaining into array
                    // Stack: [..., iterator]
                    self.emit(Op::NewArray(0)); // [..., iter, result_arr]
                    let loop_start = self.offset();
                    // Dup iterator (under the array): need [iter, arr, iter]
                    // Use: load iter from below arr
                    // Simpler: swap iter and arr, dup iter, swap back
                    // Actually: just emit a loop that calls IterNext
                    self.emit(Op::Dup); // [..., iter, arr, arr] -- wrong, need iter
                                        // This is tricky with stack. Let me use a different approach:
                                        // Store the array in a local, iterate, push elements.
                                        // Actually, simplest: just use IterCollectRest or inline loop.
                    self.emit(Op::Pop); // undo the Dup
                                        // Approach: swap(iter, arr), dup iter, IterNext, if done break, else push to arr
                                        // But we don't have Swap. Let me use a simpler approach:
                                        // Pop the empty array, use iterator directly, collect into new array.
                    self.emit(Op::Pop); // pop empty array
                                        // Stack: [..., iterator]
                                        // Emit: collect remaining from iterator into array
                                        // We'll use a loop: dup iter, IterNext, check has_more, push to array
                    let result_arr = self.scope_mut().add_local(String::from("__rest_arr__"));
                    self.emit(Op::NewArray(0));
                    self.emit(Op::StoreLocal(result_arr));
                    self.emit(Op::Pop);
                    // Loop
                    let loop_top = self.offset();
                    self.emit(Op::Dup); // dup iterator
                    self.emit(Op::IterNext); // value, has_more
                    let exit = self.emit(Op::JumpIfFalse(0)); // if !has_more, exit
                                                              // Push value into array
                    self.emit(Op::LoadLocal(result_arr));
                    // Stack: [..., iter, value, arr]
                    // Need to swap value and arr... no Swap op.
                    // Use StoreLocal to save value temporarily
                    let tmp = self.scope_mut().add_local(String::from("__rest_tmp__"));
                    // Stack: [..., iter, value, arr]
                    self.emit(Op::Pop); // pop arr
                    self.emit(Op::StoreLocal(tmp)); // save value
                    self.emit(Op::Pop); // pop value (StoreLocal peeks)
                    self.emit(Op::LoadLocal(result_arr)); // load arr
                    self.emit(Op::LoadLocal(tmp)); // load value
                    self.emit(Op::ArrayPush); // arr.push(value)
                    self.emit(Op::Pop); // pop arr
                    self.emit(Op::Jump(loop_top as i32 - self.offset() as i32 - 1));
                    self.patch_jump(exit);
                    self.emit(Op::Pop); // pop the value (undefined when done)
                                        // Stack: [..., iterator]
                    self.emit(Op::LoadLocal(result_arr));
                    // Stack: [..., iterator, rest_array]
                    self.compile_pattern_binding(inner);
                    // Stack: [..., iterator]
                }
                Some(pat) => {
                    // Normal element: get next value from iterator
                    self.emit(Op::Dup); // [..., iter, iter]
                    self.emit(Op::IterNext); // [..., iter, value, has_more]
                    self.emit(Op::Pop); // [..., iter, value]  (drop has_more flag)
                    self.compile_pattern_binding(pat);
                    // Stack: [..., iter]
                }
            }
        }
        self.emit(Op::Pop); // pop the iterator
    }

    fn compile_object_destructure(&mut self, props: &[ObjPatProp]) {
        // ES2023 §13.3.3.5: RequireObjectCoercible — throw TypeError for null/undefined
        self.emit(Op::Dup);
        self.emit(Op::RequireObjectCoercible);
        self.emit(Op::Pop);
        // Collect the non-rest keys (for building the exclusion list for the rest element).
        let mut excluded_keys: Vec<String> = Vec::new();
        let mut has_rest = false;
        for prop in props {
            if matches!(&prop.value, Pattern::Rest(_)) {
                has_rest = true;
            } else {
                excluded_keys.push(prop.key.clone());
            }
        }

        // Emit normal property extractions first.
        for prop in props {
            if matches!(&prop.value, Pattern::Rest(_)) {
                continue;
            }
            self.emit(Op::Dup);
            let name_idx = self.add_const(Constant::String(prop.key.clone()));
            self.emit(Op::GetPropNamed(name_idx));
            self.compile_pattern_binding(&prop.value);
        }

        // Emit rest object creation if present.
        if has_rest {
            for prop in props {
                if let Pattern::Rest(inner) = &prop.value {
                    // Stack: [..., src_obj] — Dup it, push excluded keys, emit ObjectRest.
                    self.emit(Op::Dup);
                    for key in &excluded_keys {
                        let ki = self.add_const(Constant::String(key.clone()));
                        self.emit(Op::LoadConst(ki));
                    }
                    let n = excluded_keys.len() as u8;
                    self.emit(Op::ObjectRest(n));
                    self.compile_pattern_binding(inner);
                    break;
                }
            }
        }

        self.emit(Op::Pop); // pop the source object
    }

    fn compile_pattern_binding(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Ident(name) => {
                let name_clone = name.clone();
                self.bind_ident(&name_clone);
            }
            Pattern::Assign(inner, default) => {
                // If value is undefined, use default
                let idx = self.emit(Op::Dup);
                let _ = idx;
                self.emit(Op::LoadUndefined);
                self.emit(Op::StrictEq);
                let skip = self.emit(Op::JumpIfFalse(0));
                self.emit(Op::Pop); // pop undefined
                                    // ES2023 §14.1.20: infer function name from binding identifier
                if let Pattern::Ident(ref name) = **inner {
                    self.compile_expr_with_name(default, name);
                } else {
                    self.compile_expr(default);
                }
                self.patch_jump(skip);
                self.compile_pattern_binding(inner);
            }
            Pattern::Array(elems) => {
                self.compile_array_destructure(elems);
            }
            Pattern::Object(props) => {
                self.compile_object_destructure(props);
            }
            Pattern::Rest(inner) => {
                // A bare rest pattern (e.g. as a function parameter) — just bind the value.
                self.compile_pattern_binding(inner);
            }
        }
    }

    fn compile_for_in_of(&mut self, left: &ForInit, right: &Expr, body: &Stmt, is_of: bool) {
        self.begin_scope();
        self.compile_expr(right);

        if is_of {
            self.emit(Op::GetIterator);
        } else {
            // For-in: null/undefined → skip entirely (ES2023 §14.7.5.6 step 7a)
            self.emit(Op::Dup);
            self.emit(Op::LoadNull);
            self.emit(Op::StrictEq);
            let skip_null = self.emit(Op::JumpIfTrue(0));
            self.emit(Op::Dup);
            self.emit(Op::LoadUndefined);
            self.emit(Op::StrictEq);
            let skip_undef = self.emit(Op::JumpIfTrue(0));
            self.emit(Op::GetIterator);
            let skip_over = self.emit(Op::Jump(0));
            // Null/undefined path: pop value, push empty iterator
            self.patch_jump(skip_null);
            self.patch_jump(skip_undef);
            self.emit(Op::Pop); // pop null/undefined
            self.emit(Op::NewArray(0)); // empty "iterator"
            self.emit(Op::GetIterator);
            self.patch_jump(skip_over);
        }

        let loop_start = self.offset();
        let old_breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);

        self.emit(Op::Dup); // dup iterator
        self.emit(Op::IterNext);
        // Stack: [..., iterator, {value, done}]
        // We need to check `done` — simplified: IterNext pushes value or Undefined if done
        // Actually let's simplify: IterNext pushes value and a bool (done)
        // For our VM: IterNext pops iterator, pushes (value, done_bool)
        // Let's just push the value; the VM's IterNext returns Undefined when done
        let exit_jump = self.emit(Op::JumpIfFalse(0)); // done flag

        // Bind the iteration value to the loop variable.
        // The current iteration value is on top of the stack.
        match left {
            ForInit::VarDecl { kind, decls } => {
                if let Some(decl) = decls.first() {
                    let is_global = *kind == VarKind::Var && self.is_global_scope();
                    let prev = self.binding_is_global;
                    self.binding_is_global = is_global;
                    match &decl.name {
                        Pattern::Ident(name) => {
                            let name_clone = name.clone();
                            self.bind_ident(&name_clone);
                        }
                        Pattern::Array(elems) => {
                            self.compile_array_destructure(elems);
                        }
                        Pattern::Object(props) => {
                            self.compile_object_destructure(props);
                        }
                        Pattern::Assign(inner, _) => {
                            self.compile_pattern_binding(inner);
                        }
                        Pattern::Rest(inner) => {
                            self.compile_pattern_binding(inner);
                        }
                    }
                    self.binding_is_global = prev;
                } else {
                    self.emit(Op::Pop);
                }
            }
            ForInit::Expr(Expr::Ident(name)) => {
                if let Some(slot) = self.scope().resolve_local(name.as_str()) {
                    self.emit(Op::StoreLocal(slot));
                    self.emit(Op::Pop);
                } else {
                    let ci = self.add_const(Constant::String(name.clone()));
                    self.emit(Op::StoreGlobal(ci));
                    self.emit(Op::Pop);
                }
            }
            _ => {
                self.emit(Op::Pop);
            }
        }

        self.set_continue_target(loop_start);
        self.compile_stmt(body);

        let back = loop_start as i32 - self.offset() as i32 - 1;
        self.emit(Op::Jump(back));
        self.patch_jump(exit_jump);

        self.emit(Op::Pop); // pop iterator

        let breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);
        for b in breaks {
            self.patch_jump(b);
        }
        self.scope_mut().break_jumps = old_breaks;
        self.end_scope();
    }

    fn compile_switch(&mut self, discriminant: &Expr, cases: &[SwitchCase]) {
        self.compile_expr(discriminant);
        let old_breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);

        // First pass: emit Dup+compare+JumpIfTrue for each non-default case.
        // Collect the instruction index of each JumpIfTrue for later patching.
        let mut case_jumps: Vec<Option<usize>> = Vec::new(); // None for default
        let mut default_idx: Option<usize> = None;

        for (i, case) in cases.iter().enumerate() {
            if let Some(ref test) = case.test {
                self.emit(Op::Dup);
                self.compile_expr(test);
                self.emit(Op::StrictEq);
                let j = self.emit(Op::JumpIfTrue(0));
                case_jumps.push(Some(j));
            } else {
                default_idx = Some(i);
                case_jumps.push(None);
            }
        }

        // Emit the "no match" jump: goes to default body (if any) or past all bodies.
        let no_match_jump = self.emit(Op::Jump(0));

        // Second pass: emit bodies; patch each case's JumpIfTrue to its body start.
        let mut body_positions: Vec<usize> = Vec::new();
        for (i, case) in cases.iter().enumerate() {
            body_positions.push(self.offset());
            if let Some(j) = case_jumps[i] {
                self.patch_jump(j); // patch JumpIfTrue to this case's body
            }
            for s in &case.consequent {
                self.compile_stmt(s);
            }
        }

        // Patch the "no match" jump to the default body or past all bodies.
        if let Some(di) = default_idx {
            self.patch_jump_to_pos(no_match_jump, body_positions[di]);
        } else {
            self.patch_jump(no_match_jump);
        }

        self.emit(Op::Pop); // pop discriminant

        // Patch all break jumps to after the Pop.
        let breaks: Vec<usize> = core::mem::take(&mut self.scope_mut().break_jumps);
        for b in breaks {
            self.patch_jump(b);
        }
        self.scope_mut().break_jumps = old_breaks;
    }

    /// Compile a labeled statement (ES2023 §14.13).
    ///
    /// `break label` jumps past the labeled statement.
    /// `continue label` is valid only when the labeled body is an iteration
    /// statement — it jumps to the loop's continue point.
    ///
    /// Implementation: push a LabelEntry onto the label stack, compile the body,
    /// then patch all break/continue jumps collected by the entry.
    fn compile_labeled(&mut self, label: &str, body: &Stmt) {
        // Determine if the body is an iteration statement (ES2023 §14.1.1).
        let is_iteration = matches!(
            body,
            Stmt::While { .. }
                | Stmt::DoWhile { .. }
                | Stmt::For { .. }
                | Stmt::ForIn { .. }
                | Stmt::ForOf { .. }
        );

        self.scope_mut().label_stack.push(LabelEntry {
            name: String::from(label),
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            continue_target: None,
            is_iteration,
            needs_continue_patch: is_iteration,
        });

        // Clear last_for_continue_pos so we can detect if the body sets it.
        self.scope_mut().last_for_continue_pos = None;

        // Compile the body.
        self.compile_stmt(body);

        // Capture the for-loop's continue position (set by the for-loop compiler).
        let for_continue_pos = self.scope().last_for_continue_pos;

        // Pop the label entry and patch jumps.
        let entry = self.scope_mut().label_stack.pop().unwrap();

        // Patch `break label` jumps to here (after the labeled statement).
        let here = self.offset();
        for b in &entry.break_jumps {
            self.patch_jump_to_pos(*b, here);
        }

        // Patch `continue label` forward-jumps.  The continue target is:
        // - For while/do-while/for-in/for-of: entry.continue_target (set by
        //   set_continue_target during loop compilation).
        // - For for-loops: last_for_continue_pos (the update-step offset).
        if is_iteration && !entry.continue_jumps.is_empty() {
            let ct = entry.continue_target.or(for_continue_pos).unwrap_or(here);
            for cj in &entry.continue_jumps {
                self.patch_jump_to_pos(*cj, ct);
            }
        }
    }

    fn compile_try(
        &mut self,
        block: &[Stmt],
        catch: &Option<CatchClause>,
        finally: &Option<Vec<Stmt>>,
    ) {
        let catch_offset_slot = self.emit(Op::TryCatch(0, 0));

        // Push finally stmts so that any `return` inside the try body inlines them.
        if let Some(fin) = finally {
            self.scope_mut().pending_finallies.push(fin.clone());
        }

        // Try block
        for s in block {
            self.compile_stmt(s);
        }
        let try_end_jump = self.emit(Op::Jump(0));

        // Pop pending finally (we're leaving the try body).
        if finally.is_some() {
            self.scope_mut().pending_finallies.pop();
        }

        // Patch catch offset
        let catch_pos = self.offset();
        let catch_off = catch_pos as i32 - catch_offset_slot as i32 - 1;
        if let Op::TryCatch(ref mut co, _) = self.scope_mut().chunk.code[catch_offset_slot] {
            *co = catch_off;
        }

        // Catch block
        if let Some(cc) = catch {
            self.begin_scope();
            if let Some(ref param) = cc.param {
                self.compile_pattern_binding(param);
            } else {
                self.emit(Op::Pop); // pop exception
            }
            for s in &cc.body {
                self.compile_stmt(s);
            }
            self.end_scope();
        } else {
            self.emit(Op::Pop); // pop exception
        }

        self.patch_jump(try_end_jump);
        self.emit(Op::TryEnd);

        // Finally block (normal path — no exception, no early return).
        if let Some(fin) = finally {
            for s in fin {
                self.compile_stmt(s);
            }
        }
    }

    /// Compile an expression, inferring a name for anonymous functions/arrows/classes.
    /// ES2023 §14.1.20 — function name inference from variable or property assignment.
    fn compile_expr_with_name(&mut self, expr: &Expr, inferred_name: &str) {
        match expr {
            Expr::FunctionExpr {
                name: None,
                params,
                body,
                is_async,
                is_generator,
            } => {
                let n = String::from(inferred_name);
                self.compile_function_gen(Some(&n), params, body, *is_async, *is_generator);
            }
            Expr::Arrow {
                params,
                body,
                is_async,
            } => {
                let n = String::from(inferred_name);
                match body {
                    ArrowBody::Block(stmts) => {
                        self.compile_function_impl(
                            Some(&n),
                            params,
                            stmts,
                            *is_async,
                            false,
                            false,
                            true,
                        );
                    }
                    ArrowBody::Expr(e) => {
                        let return_stmt = Stmt::Return(Some(e.as_ref().clone()));
                        self.compile_function_impl(
                            Some(&n),
                            params,
                            &[return_stmt],
                            *is_async,
                            false,
                            false,
                            true,
                        );
                    }
                }
            }
            Expr::ClassExpr {
                name: None,
                super_class,
                body,
            } => {
                let n = String::from(inferred_name);
                let sc = super_class.as_ref().map(|b| *b.clone());
                self.compile_class(Some(&n), &sc, body);
            }
            _ => {
                self.compile_expr(expr);
            }
        }
    }

    fn compile_function(
        &mut self,
        name: Option<&String>,
        params: &[Param],
        body: &[Stmt],
        is_async: bool,
    ) {
        self.compile_function_full(name, params, body, is_async, false, false);
    }

    fn compile_function_gen(
        &mut self,
        name: Option<&String>,
        params: &[Param],
        body: &[Stmt],
        is_async: bool,
        is_generator: bool,
    ) {
        self.compile_function_full(name, params, body, is_async, false, is_generator);
    }

    fn compile_function_named_expr(
        &mut self,
        name: Option<&String>,
        params: &[Param],
        body: &[Stmt],
        is_async: bool,
    ) {
        self.compile_function_full(name, params, body, is_async, true, false);
    }

    fn compile_function_named_expr_gen(
        &mut self,
        name: Option<&String>,
        params: &[Param],
        body: &[Stmt],
        is_async: bool,
        is_generator: bool,
    ) {
        self.compile_function_full(name, params, body, is_async, true, is_generator);
    }

    fn compile_function_full(
        &mut self,
        name: Option<&String>,
        params: &[Param],
        body: &[Stmt],
        is_async: bool,
        named_expr: bool,
        is_generator: bool,
    ) {
        self.compile_function_impl(
            name,
            params,
            body,
            is_async,
            named_expr,
            is_generator,
            false,
        );
    }

    fn compile_arrow(&mut self, params: &[Param], body: &[Stmt], is_async: bool) {
        self.compile_function_impl(None, params, body, is_async, false, false, true);
    }

    fn compile_function_impl(
        &mut self,
        name: Option<&String>,
        params: &[Param],
        body: &[Stmt],
        is_async: bool,
        named_expr: bool,
        is_generator: bool,
        is_arrow: bool,
    ) {
        // Detect strict mode early (before param validation) — check if body
        // starts with "use strict" directive (ES2023 §10.2.1 step 4).
        let fn_strict = self.is_strict
            || matches!(
                body.first(),
                Some(Stmt::Expr(Expr::String(ref s))) if s == "use strict"
            );

        // ES2023 §14.1.2 Static Semantics: Early Errors — strict mode param checks.
        // These are Early Errors: the script must fail at parse/compile time.
        if fn_strict {
            let mut seen_params: Vec<String> = Vec::new();
            for param in params {
                let names = Self::collect_param_names(&param.pattern);
                for pname in &names {
                    if seen_params.contains(pname) {
                        // Emit throw in the OUTER scope (Early Error).
                        let msg = alloc::format!(
                            "Duplicate parameter name '{}' not allowed in strict mode",
                            pname
                        );
                        self.emit_throw_syntax_error(&msg);
                        // Still need to push something on the stack for the Closure slot.
                        self.emit(Op::LoadUndefined);
                        return;
                    }
                    if pname == "eval" || pname == "arguments" {
                        let msg = alloc::format!(
                            "'{}' cannot be used as a parameter name in strict mode",
                            pname
                        );
                        self.emit_throw_syntax_error(&msg);
                        self.emit(Op::LoadUndefined);
                        return;
                    }
                    seen_params.push(pname.clone());
                }
            }
        }

        // Save and reset binding_is_global — inside a function body,
        // all bindings (params, var decls, destructuring) must be local,
        // not global.  Without this, destructuring params like ([e,t,n,r])
        // in a top-level arrow would emit StoreGlobal instead of StoreLocal.
        let prev_binding_is_global = self.binding_is_global;
        self.binding_is_global = false;

        let mut func_scope = Scope::new();
        func_scope.chunk.name = name.cloned();
        func_scope.chunk.is_generator = is_generator;
        func_scope.chunk.is_arrow = is_arrow;
        func_scope.chunk.is_async = is_async;
        // ES2023 §10.2.8: function.length = number of params before the first
        // one with a default value, excluding rest parameters.
        let formal_length = params
            .iter()
            .take_while(|p| !p.is_rest && p.default.is_none())
            .count();
        func_scope.chunk.param_count = formal_length as u16;

        // Find the rest parameter (last param with is_rest=true), if any.
        let rest_param_idx = params.iter().position(|p| p.is_rest);

        // Add regular (non-rest) params as locals.
        // For destructuring params (Array/Object patterns), add a synthetic local
        // that receives the argument value; destructuring happens in the prologue.
        let mut destr_params: Vec<(u16, Pattern)> = Vec::new();
        for param in params {
            if param.is_rest {
                continue;
            }
            match &param.pattern {
                Pattern::Ident(ref n) => {
                    func_scope.add_local(n.clone());
                }
                pat @ (Pattern::Array(_) | Pattern::Object(_) | Pattern::Assign(_, _)) => {
                    let slot = func_scope.add_local(format!("__destr_{}", destr_params.len()));
                    destr_params.push((slot, pat.clone()));
                }
                _ => {
                    func_scope.add_local(format!("__destr_{}", destr_params.len()));
                }
            }
        }

        // Reserve a local for the rest param (holds an array of trailing args).
        let rest_slot: Option<u16> = if let Some(ri) = rest_param_idx {
            if let Pattern::Ident(ref n) = params[ri].pattern {
                let slot = func_scope.add_local(n.clone());
                Some(slot)
            } else {
                None
            }
        } else {
            None
        };

        // Reserve a local for `arguments` (array of all call args).
        let arguments_slot = func_scope.add_local(String::from("arguments"));

        // Reserve a local for the function's own name (named function expressions).
        let self_name_slot: Option<u16> = if named_expr {
            name.map(|n| func_scope.add_local(n.clone()))
        } else {
            None
        };

        self.scopes.push(func_scope);

        // ── Function prologue ──

        // 1. Build `arguments` from all call args.
        let rest_start = rest_param_idx.unwrap_or(params.len()) as u16;
        self.emit(Op::LoadArgsArray(0));
        self.emit(Op::StoreLocal(arguments_slot));
        // StoreLocal peeks; we still have the array on stack — pop it.
        self.emit(Op::Pop);

        // 2. Build rest array from args[rest_start..] if there's a rest param.
        if let Some(slot) = rest_slot {
            self.emit(Op::LoadArgsArray(rest_start));
            self.emit(Op::StoreLocal(slot));
            self.emit(Op::Pop);
        }

        // 3. For named function expressions, store self-reference in the name local.
        if let Some(slot) = self_name_slot {
            self.emit(Op::LoadSelf);
            self.emit(Op::StoreLocal(slot));
            self.emit(Op::Pop);
        }

        // 4. Compile default parameter values for regular params.
        let named_param_count = params.iter().filter(|p| !p.is_rest).count();
        for (i, param) in params.iter().filter(|p| !p.is_rest).enumerate() {
            if let Some(ref default) = param.default {
                self.emit(Op::LoadLocal(i as u16));
                self.emit(Op::LoadUndefined);
                self.emit(Op::StrictEq);
                let skip = self.emit(Op::JumpIfFalse(0));
                // Infer function name from parameter name
                if let Pattern::Ident(ref pname) = param.pattern {
                    self.compile_expr_with_name(default, pname);
                } else {
                    self.compile_expr(default);
                }
                self.emit(Op::StoreLocal(i as u16));
                self.emit(Op::Pop);
                self.patch_jump(skip);
            }
            let _ = named_param_count; // silence unused warning
        }

        // 5. Destructure non-ident params (array/object patterns).
        for (slot, pat) in &destr_params {
            self.emit(Op::LoadLocal(*slot));
            match pat {
                Pattern::Array(elements) => {
                    self.compile_array_destructure(elements);
                }
                Pattern::Object(props) => {
                    self.compile_object_destructure(props);
                }
                Pattern::Assign(inner, default) => {
                    // param with default: if undefined, use default
                    self.emit(Op::Dup);
                    self.emit(Op::LoadUndefined);
                    self.emit(Op::StrictEq);
                    let skip = self.emit(Op::JumpIfFalse(0));
                    self.emit(Op::Pop); // pop undefined value
                    if let Pattern::Ident(ref name) = **inner {
                        self.compile_expr_with_name(default, name);
                    } else {
                        self.compile_expr(default);
                    }
                    self.patch_jump(skip);
                    self.compile_pattern_binding(inner);
                }
                _ => {
                    self.emit(Op::Pop);
                }
            }
        }

        // Detect "use strict" at the beginning of function body
        let prev_strict = self.is_strict;
        if let Some(Stmt::Expr(Expr::String(ref s))) = body.first() {
            if s == "use strict" {
                self.is_strict = true;
            }
        }

        // ── Var hoisting (ES2023 §10.2.11) ──
        // Pre-scan the body for `var` declarations and register their names as
        // locals BEFORE compiling the body.  This ensures that nested functions
        // (which may reference these variables before the `var` statement is
        // reached) resolve them as upvalues instead of globals.
        // `let`/`const` are NOT hoisted this way (they use TDZ semantics).
        {
            let mut hoisted: Vec<String> = Vec::new();
            Self::collect_var_names(body, &mut hoisted);
            for name in &hoisted {
                // Only add if not already a local (e.g. param with the same name).
                if self.scope().resolve_local(name).is_none() {
                    self.scope_mut().add_local(name.clone());
                }
            }
        }

        // ── Lexical name pre-allocation ──
        // Pre-allocate locals for top-level lexical bindings in this function
        // body. While `let`/`const`/`class` are NOT value-hoisted (TDZ semantics),
        // their NAMES must be registered before nested function/arrow/class
        // compilation so closures can capture them as upvalues.
        // Example: `Object.defineProperty(exports, "V", { get: () => Logger }); class Logger {}`
        // Without this, the getter closure resolves `Logger` as a global and
        // later returns `undefined`.
        // Only top-level declarations (not nested in blocks) are pre-allocated,
        // matching function declaration hoisting scope.
        for s in body.iter() {
            match s {
                Stmt::VarDecl { kind, decls } => {
                    if *kind == VarKind::Let || *kind == VarKind::Const {
                        for decl in decls {
                            let mut names: Vec<String> = Vec::new();
                            Self::collect_pattern_names(&decl.name, &mut names);
                            for name in names {
                                if self.scope().resolve_local(&name).is_none() {
                                    self.scope_mut().add_local(name);
                                }
                            }
                        }
                    }
                }
                Stmt::ClassDecl { name, .. } => {
                    if self.scope().resolve_local(name).is_none() {
                        self.scope_mut().add_local(name.clone());
                    }
                }
                _ => {}
            }
        }

        // ── Function declaration value hoisting (ES2023 §10.2.11 step 28) ──
        // Function declarations are fully hoisted: both name AND value are
        // available from the start of the scope.  Compile them first so that
        // code appearing before the declaration can call the function.
        for s in body.iter() {
            if let Stmt::FunctionDecl {
                name,
                params,
                body: fn_body,
                is_async,
                is_generator,
            } = s
            {
                self.compile_function_gen(Some(name), params, fn_body, *is_async, *is_generator);
                // Store into the pre-allocated hoisted slot.
                let slot = self
                    .scope()
                    .resolve_local(name)
                    .unwrap_or_else(|| self.scope_mut().add_local(name.clone()));
                self.emit(Op::StoreLocal(slot));
                self.emit(Op::Pop);
            }
        }

        for s in body {
            self.compile_stmt(s);
        }

        // Restore strict mode state
        self.is_strict = prev_strict;

        if is_async {
            // Async functions: wrap implicit return undefined in Promise.resolve().
            self.emit(Op::LoadUndefined);
            self.emit(Op::Await);
            self.emit(Op::Return);
        } else {
            // Implicit return undefined
            self.emit(Op::LoadUndefined);
            self.emit(Op::Return);
        }

        let func_scope = self.scopes.pop().unwrap();
        let mut func_chunk = func_scope.chunk;
        func_chunk.strict = self.is_strict;
        // Copy upvalue descriptors into the chunk so the VM knows how to capture them.
        func_chunk.upvalues = func_scope
            .upvalues
            .iter()
            .map(|uv| UpvalueRef {
                is_local: uv.is_local,
                index: uv.index,
            })
            .collect();
        let ci = self.add_const(Constant::Function(func_chunk));
        self.emit(Op::Closure(ci));

        // For named function expressions: also make the function accessible by its
        // name as a global (simplified — avoids needing a scope wrapper object).
        if named_expr {
            if let Some(n) = name {
                let ni = self.add_const(Constant::String(n.clone()));
                self.emit(Op::Dup);
                self.emit(Op::StoreGlobal(ni));
                self.emit(Op::Pop);
            }
        }

        // Restore binding_is_global to the value before entering this function.
        self.binding_is_global = prev_binding_is_global;
    }

    fn compile_class(
        &mut self,
        name: Option<&String>,
        super_class: &Option<Expr>,
        body: &[ClassMember],
    ) {
        // Step 0: If there's a super class, evaluate it and stash it in a local named
        // "$$super$$" so that constructor/method closures can capture it as an upvalue
        // and emit correct `super()` / `super.method()` calls.
        let super_local: Option<u16> = if let Some(ref super_expr) = super_class {
            self.compile_expr(super_expr); // → [SuperClass]
            let slot = self.scope_mut().add_local(String::from("$$super$$"));
            self.emit(Op::StoreLocal(slot)); // peek — leaves [SuperClass] on stack
            self.emit(Op::Pop); // → []
            Some(slot)
        } else {
            None
        };

        // Step 1: Collect instance properties (public + private fields).
        // These need to be initialized in the constructor on `this`, NOT on prototype.
        let mut instance_inits: Vec<Stmt> = Vec::new();
        for member in body {
            if member.is_static {
                continue;
            }
            if let ClassMemberKind::Property { ref value } = member.kind {
                let rhs = Box::new(value.clone().unwrap_or(Expr::Undefined));
                let init_expr = match &member.key {
                    PropKey::Ident(s) | PropKey::String(s) => {
                        // Generate: this.<key> = <value>;
                        Expr::Assign {
                            op: AssignOp::Assign,
                            left: Box::new(Expr::Member {
                                object: Box::new(Expr::This),
                                property: s.clone(),
                                computed: false,
                            }),
                            right: rhs,
                        }
                    }
                    PropKey::Number(n) => {
                        // Generate: this[<number>] = <value>;
                        Expr::Assign {
                            op: AssignOp::Assign,
                            left: Box::new(Expr::Index {
                                object: Box::new(Expr::This),
                                index: Box::new(Expr::Number(*n)),
                            }),
                            right: rhs,
                        }
                    }
                    PropKey::Computed(expr) => {
                        // Generate: this[<expr>] = <value>;
                        Expr::Assign {
                            op: AssignOp::Assign,
                            left: Box::new(Expr::Index {
                                object: Box::new(Expr::This),
                                index: expr.clone(),
                            }),
                            right: rhs,
                        }
                    }
                };
                instance_inits.push(Stmt::Expr(init_expr));
            }
        }

        // Step 2: compile the constructor (or a default one), with instance field
        // initializers prepended to the body.
        let ctor = body
            .iter()
            .find(|m| matches!(m.kind, ClassMemberKind::Constructor { .. }));
        if let Some(ctor_member) = ctor {
            if let ClassMemberKind::Constructor {
                ref params,
                ref body,
            } = ctor_member.kind
            {
                let mut full_body = if super_class.is_some() {
                    Self::insert_instance_initializers_after_super(body, &instance_inits)
                } else {
                    let mut body_with_inits = instance_inits.clone();
                    body_with_inits.extend(body.iter().cloned());
                    body_with_inits
                };
                self.compile_function(name, params, &full_body, false);
            }
        } else {
            if super_class.is_some() {
                let mut default_body = vec![Stmt::Expr(Expr::Call {
                    callee: Box::new(Expr::Ident(String::from("super"))),
                    arguments: vec![Expr::Spread(Box::new(Expr::Ident(String::from("args"))))],
                })];
                default_body.extend(instance_inits.iter().cloned());
                let params = vec![Param {
                    pattern: Pattern::Ident(String::from("args")),
                    default: None,
                    is_rest: true,
                }];
                self.compile_function(name, &params, &default_body, false);
            } else if instance_inits.is_empty() {
                self.compile_function(name, &[], &[], false);
            } else {
                self.compile_function(name, &[], &instance_inits, false);
            }
        }
        // Stack: [..., Constructor]

        // Step 3: if there's a super class, set up the prototype chain.
        if let Some(super_slot) = super_local {
            // Store super class directly on the constructor function
            // Stack: [..., Constructor]
            self.emit(Op::Dup); // [..., Ctor, Ctor]
            self.emit(Op::LoadLocal(super_slot)); // [..., Ctor, Ctor, SuperClass]
            self.emit(Op::SetSuperClass); // [..., Ctor, Ctor]  (pops SuperClass, sets ctor.super_class)
            self.emit(Op::Pop); // [..., Ctor]

            // Set up prototype chain
            self.emit(Op::Dup);
            let proto_idx = self.add_const(Constant::String(String::from("prototype")));
            self.emit(Op::GetPropNamed(proto_idx)); // [..., Constructor, Constructor.prototype]
            self.emit(Op::LoadLocal(super_slot)); // [..., Constructor, Constructor.prototype, SuperClass]
            let proto_idx2 = self.add_const(Constant::String(String::from("prototype")));
            self.emit(Op::GetPropNamed(proto_idx2)); // [..., Constructor, Constructor.prototype, SuperClass.prototype]
                                                     // Set Constructor.prototype.__proto__ = SuperClass.prototype
            let proto_key_idx = self.add_const(Constant::String(String::from("__proto__")));
            self.emit(Op::SetPropNamed(proto_key_idx)); // [..., Constructor, SuperClass.prototype]
            self.emit(Op::Pop); // [..., Constructor]
        }

        // Step 3b: If the class has a name and contains static blocks, temporarily
        // bind the constructor to a local so that static blocks can reference the class.
        let class_name_slot: Option<u16> = if let Some(n) = name {
            if body
                .iter()
                .any(|m| matches!(m.kind, ClassMemberKind::StaticBlock { .. }))
            {
                self.emit(Op::Dup);
                let slot = self.scope_mut().add_local(n.clone());
                self.emit(Op::StoreLocal(slot));
                self.emit(Op::Pop);
                Some(slot)
            } else {
                None
            }
        } else {
            None
        };

        // Step 4: add instance methods and static members to Constructor/prototype.
        for member in body {
            if matches!(member.kind, ClassMemberKind::Constructor { .. }) {
                continue;
            }
            // Static block: compile body statements inline (ES2022 §14.7)
            if let ClassMemberKind::StaticBlock { ref body } = member.kind {
                for stmt in body {
                    self.compile_stmt(stmt);
                }
                continue;
            }
            // Skip non-static properties — they are initialized in the constructor.
            if !member.is_static {
                if let ClassMemberKind::Property { .. } = member.kind {
                    continue;
                }
            }
            // Resolve the member key: named keys get a string, computed keys
            // are evaluated at runtime via SetProp.
            let key_name = match &member.key {
                PropKey::Ident(s) | PropKey::String(s) => Some(s.clone()),
                PropKey::Number(n) => {
                    let mut s = alloc::format!("{}", n);
                    // Strip trailing ".0" for integer numbers (e.g. 10.0 → "10")
                    if s.ends_with(".0") {
                        s.truncate(s.len() - 2);
                    }
                    Some(s)
                }
                PropKey::Computed(_) => None,
            };
            // Helper: for computed keys, push the computed key expression onto
            // the stack BEFORE pushing the value.  SetProp expects [obj, key, val].
            // For named keys, we push key after value via SetPropNamed(idx).
            let is_computed = matches!(member.key, PropKey::Computed(_));
            let fn_name = key_name.clone().unwrap_or_else(|| String::new());
            if member.is_static {
                // Static methods/properties/accessors: set directly on Constructor.
                match &member.kind {
                    ClassMemberKind::Method {
                        params,
                        body,
                        is_generator,
                        is_async,
                    } => {
                        self.emit(Op::Dup); // [Ctor, Ctor]
                        if is_computed {
                            if let PropKey::Computed(ref e) = member.key {
                                self.compile_expr(e);
                            }
                        }
                        self.compile_function_gen(
                            Some(&fn_name),
                            params,
                            body,
                            *is_async,
                            *is_generator,
                        );
                        if is_computed {
                            self.emit(Op::SetProp);
                        } else {
                            let ki = self.add_const(Constant::String(fn_name.clone()));
                            self.emit(Op::DefineMethod(ki));
                        }
                        self.emit(Op::Pop);
                    }
                    ClassMemberKind::Property { value } => {
                        self.emit(Op::Dup);
                        if is_computed {
                            if let PropKey::Computed(ref e) = member.key {
                                self.compile_expr(e);
                            }
                        }
                        if let Some(v) = value {
                            self.compile_expr(v);
                        } else {
                            self.emit(Op::LoadUndefined);
                        }
                        if is_computed {
                            self.emit(Op::SetProp);
                        } else {
                            let ki = self.add_const(Constant::String(fn_name.clone()));
                            self.emit(Op::SetPropNamed(ki));
                        }
                        self.emit(Op::Pop);
                    }
                    ClassMemberKind::Getter { body } => {
                        self.emit(Op::Dup);
                        if is_computed {
                            if let PropKey::Computed(ref e) = member.key {
                                self.compile_expr(e);
                            }
                        }
                        let getter_name = alloc::format!("get {}", fn_name);
                        self.compile_function(Some(&getter_name), &[], body, false);
                        if is_computed {
                            self.emit(Op::DefineGetterComputed);
                        } else if let Some(ref kn) = key_name {
                            let ki = self.add_const(Constant::String(kn.clone()));
                            self.emit(Op::DefineGetter(ki));
                        }
                    }
                    ClassMemberKind::Setter { param, body } => {
                        self.emit(Op::Dup);
                        if is_computed {
                            if let PropKey::Computed(ref e) = member.key {
                                self.compile_expr(e);
                            }
                        }
                        let setter_name = alloc::format!("set {}", fn_name);
                        let p = vec![Param {
                            pattern: Pattern::Ident(param.clone()),
                            default: None,
                            is_rest: false,
                        }];
                        self.compile_function(Some(&setter_name), &p, body, false);
                        if is_computed {
                            self.emit(Op::DefineSetterComputed);
                        } else if let Some(ref kn) = key_name {
                            let ki = self.add_const(Constant::String(kn.clone()));
                            self.emit(Op::DefineSetter(ki));
                        }
                    }
                    _ => {}
                }
            } else {
                // Instance methods/accessors: set on Constructor.prototype.
                match &member.kind {
                    ClassMemberKind::Method {
                        params,
                        body,
                        is_generator,
                        is_async,
                    } => {
                        self.emit(Op::Dup);
                        let proto_idx = self.add_const(Constant::String(String::from("prototype")));
                        self.emit(Op::GetPropNamed(proto_idx));
                        // Stack: [..., Constructor, Constructor.prototype]
                        if is_computed {
                            if let PropKey::Computed(ref e) = member.key {
                                self.compile_expr(e);
                            }
                        }
                        self.compile_function_gen(
                            Some(&fn_name),
                            params,
                            body,
                            *is_async,
                            *is_generator,
                        );
                        if is_computed {
                            self.emit(Op::SetProp); // TODO: non-enum for computed methods
                        } else {
                            let ki = self.add_const(Constant::String(fn_name.clone()));
                            self.emit(Op::DefineMethod(ki));
                        }
                        self.emit(Op::Pop);
                    }
                    ClassMemberKind::Getter { body } => {
                        self.emit(Op::Dup);
                        let proto_idx = self.add_const(Constant::String(String::from("prototype")));
                        self.emit(Op::GetPropNamed(proto_idx));
                        if is_computed {
                            if let PropKey::Computed(ref e) = member.key {
                                self.compile_expr(e);
                            }
                        }
                        let getter_name = alloc::format!("get {}", fn_name);
                        self.compile_function(Some(&getter_name), &[], body, false);
                        if is_computed {
                            self.emit(Op::DefineGetterComputed);
                        } else if let Some(ref kn) = key_name {
                            let ki = self.add_const(Constant::String(kn.clone()));
                            self.emit(Op::DefineGetter(ki));
                        }
                        self.emit(Op::Pop);
                    }
                    ClassMemberKind::Setter { param, body } => {
                        self.emit(Op::Dup);
                        let proto_idx = self.add_const(Constant::String(String::from("prototype")));
                        self.emit(Op::GetPropNamed(proto_idx));
                        if is_computed {
                            if let PropKey::Computed(ref e) = member.key {
                                self.compile_expr(e);
                            }
                        }
                        let setter_name = alloc::format!("set {}", fn_name);
                        let p = vec![Param {
                            pattern: Pattern::Ident(param.clone()),
                            default: None,
                            is_rest: false,
                        }];
                        self.compile_function(Some(&setter_name), &p, body, false);
                        if is_computed {
                            self.emit(Op::DefineSetterComputed);
                        } else if let Some(ref kn) = key_name {
                            let ki = self.add_const(Constant::String(kn.clone()));
                            self.emit(Op::DefineSetter(ki));
                        }
                        self.emit(Op::Pop);
                    }
                    _ => {}
                }
            }
        }
        // Stack: [..., Constructor]
    }

    /// Check if an expression is a direct super() call.
    fn expr_is_super_call(expr: &Expr) -> bool {
        matches!(expr, Expr::Call { callee, .. } if matches!(callee.as_ref(), Expr::Ident(name) if name == "super"))
    }

    /// Check if an expression contains a super() call (at any nesting depth).
    fn expr_contains_super_call(expr: &Expr) -> bool {
        match expr {
            Expr::Call { callee, .. } => {
                matches!(callee.as_ref(), Expr::Ident(name) if name == "super")
            }
            // Comma expression / sequence: super(), this.init()
            Expr::Sequence(exprs) => exprs.iter().any(Self::expr_contains_super_call),
            // Assignment: const x = super()  or  this.x = super()
            Expr::Assign { right, .. } => Self::expr_contains_super_call(right),
            // Binary op with comma: (super(), expr)
            Expr::Binary { left, right, .. } => {
                Self::expr_contains_super_call(left) || Self::expr_contains_super_call(right)
            }
            _ => false,
        }
    }

    fn is_top_level_super_call(stmt: &Stmt) -> bool {
        match stmt {
            // Direct: super(args);
            Stmt::Expr(expr) => Self::expr_contains_super_call(expr),
            // return super(...arguments);
            Stmt::Return(Some(expr)) => Self::expr_contains_super_call(expr),
            // const x = super();  /  let x = super();
            Stmt::VarDecl { decls, .. } => {
                decls.iter().any(|d| d.init.as_ref().map_or(false, Self::expr_contains_super_call))
            }
            _ => false,
        }
    }

    /// Check if an expression is a sequence (comma expr) with super() as first element.
    fn expr_is_sequence_with_super(expr: &Expr) -> bool {
        match expr {
            Expr::Sequence(exprs) => exprs.first().map_or(false, Self::expr_is_super_call),
            _ => false,
        }
    }

    /// Split a sequence expression at the super() call:
    /// `super(), a, b` → (super(), Sequence([a, b]))
    fn split_sequence_at_super(expr: &Expr) -> (Expr, Option<Expr>) {
        if let Expr::Sequence(exprs) = expr {
            let super_expr = exprs[0].clone();
            let rest: Vec<Expr> = exprs[1..].to_vec();
            let rest_expr = if rest.is_empty() {
                None
            } else if rest.len() == 1 {
                Some(rest.into_iter().next().unwrap())
            } else {
                Some(Expr::Sequence(rest))
            };
            (super_expr, rest_expr)
        } else {
            (expr.clone(), None)
        }
    }

    fn insert_instance_initializers_after_super(
        body: &[Stmt],
        instance_inits: &[Stmt],
    ) -> Vec<Stmt> {
        if instance_inits.is_empty() {
            return body.to_vec();
        }

        let mut full_body = Vec::with_capacity(body.len() + instance_inits.len());
        let mut inserted = false;
        for stmt in body.iter().cloned() {
            if !inserted && Self::is_top_level_super_call(&stmt) {
                match &stmt {
                    // `return super(...args);` → `super(...args); <inits>; return this;`
                    Stmt::Return(Some(expr)) if Self::expr_contains_super_call(expr) => {
                        full_body.push(Stmt::Expr(expr.clone()));
                        full_body.extend(instance_inits.iter().cloned());
                        full_body.push(Stmt::Return(Some(Expr::This)));
                    }
                    // `super(), rest;` — split sequence: `super(); <inits>; rest;`
                    Stmt::Expr(expr) if Self::expr_is_sequence_with_super(expr) => {
                        let (super_part, rest) = Self::split_sequence_at_super(expr);
                        full_body.push(Stmt::Expr(super_part));
                        full_body.extend(instance_inits.iter().cloned());
                        if let Some(rest_expr) = rest {
                            full_body.push(Stmt::Expr(rest_expr));
                        }
                    }
                    // Simple: `super(args);` → `super(args); <inits>;`
                    _ => {
                        full_body.push(stmt);
                        full_body.extend(instance_inits.iter().cloned());
                    }
                }
                inserted = true;
            } else {
                full_body.push(stmt);
            }
        }

        if !inserted {
            let mut fallback = instance_inits.to_vec();
            fallback.extend(body.iter().cloned());
            return fallback;
        }

        full_body
    }

    // ── Template literal interpolation ──

    /// Compile a template literal string that may contain `${...}` expressions.
    /// The lexer stores the entire template (static parts + raw expression
    /// source) as a single string.  This method re-parses the expression
    /// segments and emits string-concatenation bytecode.
    fn compile_template_literal(&mut self, s: &str) {
        // Split the template string into alternating static/expression parts.
        let mut parts: Vec<Result<String, String>> = Vec::new(); // Ok = static, Err = expr src
        let bytes = s.as_bytes();
        let mut i = 0;
        let mut current = String::new();
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                parts.push(Ok(current.clone()));
                current.clear();
                i += 2; // skip ${
                let mut depth = 1u32;
                let mut expr_src = String::new();
                while i < bytes.len() && depth > 0 {
                    match bytes[i] {
                        b'{' => {
                            depth += 1;
                            expr_src.push(bytes[i] as char);
                            i += 1;
                        }
                        b'}' => {
                            depth -= 1;
                            if depth > 0 {
                                expr_src.push(b'}' as char);
                            }
                            i += 1;
                        }
                        _ => {
                            // Handle multi-byte UTF-8 safely
                            if bytes[i] < 0x80 {
                                expr_src.push(bytes[i] as char);
                                i += 1;
                            } else {
                                // Find the end of this UTF-8 sequence
                                let start = i;
                                i += 1;
                                while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 {
                                    i += 1;
                                }
                                if let Ok(ch_str) = core::str::from_utf8(&bytes[start..i]) {
                                    expr_src.push_str(ch_str);
                                }
                            }
                        }
                    }
                }
                parts.push(Err(expr_src));
            } else if bytes[i] < 0x80 {
                current.push(bytes[i] as char);
                i += 1;
            } else {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 {
                    i += 1;
                }
                if let Ok(ch_str) = core::str::from_utf8(&bytes[start..i]) {
                    current.push_str(ch_str);
                }
            }
        }
        parts.push(Ok(current));

        // Emit code: start with empty string, then + each part
        let empty_ci = self.add_const(Constant::String(String::new()));
        self.emit(Op::LoadConst(empty_ci));

        for part in &parts {
            match part {
                Ok(text) => {
                    if !text.is_empty() {
                        let ci = self.add_const(Constant::String(text.clone()));
                        self.emit(Op::LoadConst(ci));
                        self.emit(Op::Add);
                    }
                }
                Err(expr_src) => {
                    // Re-parse and compile the expression by wrapping it in a
                    // minimal program and extracting the first statement.
                    let tokens = Lexer::tokenize(expr_src);
                    let mut p = Parser::new(tokens);
                    let prog = p.parse_program();
                    if let Some(Stmt::Expr(inner)) = prog.body.into_iter().next() {
                        self.compile_expr(&inner);
                    } else {
                        self.emit(Op::LoadUndefined);
                    }
                    self.emit(Op::Add);
                }
            }
        }
    }

    // ── Expressions ──

    fn compile_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Number(n) => {
                let ci = self.add_const(Constant::Number(*n));
                self.emit(Op::LoadConst(ci));
            }
            Expr::String(s) => {
                let ci = self.add_const(Constant::String(s.clone()));
                self.emit(Op::LoadConst(ci));
            }
            Expr::Template(s) => {
                self.compile_template_literal(s);
            }
            Expr::Bool(true) => {
                self.emit(Op::LoadTrue);
            }
            Expr::Bool(false) => {
                self.emit(Op::LoadFalse);
            }
            Expr::Null => {
                self.emit(Op::LoadNull);
            }
            Expr::Undefined => {
                self.emit(Op::LoadUndefined);
            }
            Expr::This => {
                self.emit(Op::LoadThis);
            }
            Expr::Ident(name) => {
                let name = name.clone();
                self.emit_load_name(&name);
            }
            Expr::Array(elements) => {
                let has_spread = elements.iter().any(|e| matches!(e, Some(Expr::Spread(_))));
                if has_spread {
                    // Build incrementally: start with empty array, then push/spread each element.
                    // Spread/ArrayPush use pop-modify-push semantics: no Dup needed.
                    self.emit(Op::NewArray(0));
                    for elem in elements {
                        if let Some(Expr::Spread(inner)) = elem {
                            self.compile_expr(inner);
                            self.emit(Op::Spread);
                        } else if let Some(e) = elem {
                            self.compile_expr(e);
                            self.emit(Op::ArrayPush);
                        } else {
                            self.emit(Op::LoadUndefined);
                            self.emit(Op::ArrayPush);
                        }
                    }
                } else {
                    for elem in elements {
                        if let Some(e) = elem {
                            self.compile_expr(e);
                        } else {
                            self.emit(Op::LoadUndefined);
                        }
                    }
                    self.emit(Op::NewArray(elements.len() as u16));
                }
            }
            Expr::Object(props) => {
                self.emit(Op::NewObject);
                for prop in props {
                    // Spread property: { ...source }
                    if let PropKey::Ident(k) = &prop.key {
                        if k == "..." {
                            self.compile_expr(&prop.value); // [obj, source]
                            self.emit(Op::ObjectSpread); // [obj]  (copies source → obj, peeks target)
                            continue;
                        }
                    }

                    // Getter/Setter: { get prop() { }, set prop(v) { } }
                    if prop.kind == PropKind::Get || prop.kind == PropKind::Set {
                        if let PropKey::Ident(name) | PropKey::String(name) = &prop.key {
                            self.emit(Op::Dup); // [obj, obj]
                            let accessor_name = if prop.kind == PropKind::Get {
                                alloc::format!("get {}", name)
                            } else {
                                alloc::format!("set {}", name)
                            };
                            self.compile_expr_with_name(&prop.value, &accessor_name); // [obj, obj, fn]
                            let ci = self.add_const(Constant::String(name.clone()));
                            if prop.kind == PropKind::Get {
                                self.emit(Op::DefineGetter(ci)); // [obj, obj] (pops fn, peeks obj)
                            } else {
                                self.emit(Op::DefineSetter(ci)); // [obj, obj]
                            }
                            self.emit(Op::Pop); // [obj] — pop the Dup'd reference
                        }
                        continue;
                    }

                    match &prop.key {
                        PropKey::Ident(name) | PropKey::String(name) => {
                            self.emit(Op::Dup); // [obj, obj]
                            self.compile_expr_with_name(&prop.value, name); // [obj, obj, val]
                            let ci = self.add_const(Constant::String(name.clone()));
                            self.emit(Op::SetPropNamed(ci)); // [obj, val]
                            self.emit(Op::Pop); // [obj]
                        }
                        PropKey::Number(n) => {
                            // [obj] → [obj, obj] → [obj, obj, key] → [obj, obj, key, val] → [obj, val] → [obj]
                            self.emit(Op::Dup);
                            let key_ci = self.add_const(Constant::Number(*n));
                            self.emit(Op::LoadConst(key_ci));
                            self.compile_expr(&prop.value);
                            self.emit(Op::SetProp);
                            self.emit(Op::Pop);
                        }
                        PropKey::Computed(key) => {
                            // [obj] → [obj, obj] → [obj, obj, key] → [obj, obj, key, val] → [obj, val] → [obj]
                            self.emit(Op::Dup);
                            self.compile_expr(key);
                            self.compile_expr(&prop.value);
                            self.emit(Op::SetProp);
                            self.emit(Op::Pop);
                        }
                    }
                }
            }
            Expr::Member {
                object, property, ..
            } => {
                self.compile_expr(object);
                let ci = self.add_const(Constant::String(property.clone()));
                self.emit(Op::GetPropNamed(ci));
            }
            Expr::Index { object, index } => {
                self.compile_expr(object);
                self.compile_expr(index);
                self.emit(Op::GetProp);
            }
            Expr::Call { callee, arguments } => {
                // Check for super() and super.method() before other patterns.
                match callee.as_ref() {
                    Expr::Ident(name) if name == "super" => {
                        // super(args) — call parent constructor with current `this`.
                        self.emit_load_name("$$super$$");
                        if Self::args_have_spread(arguments) {
                            // super(...args) — spread arguments into array, then SuperCallSpread.
                            // Stack layout: [..., SuperClass, args_array]
                            self.compile_args_as_array(arguments);
                            self.emit(Op::SuperCallSpread);
                        } else {
                            // Stack layout: [..., SuperClass, arg1..argN]
                            for arg in arguments {
                                self.compile_expr(arg);
                            }
                            self.emit(Op::SuperCall(arguments.len() as u8));
                        }
                    }
                    Expr::Member {
                        object, property, ..
                    } if matches!(object.as_ref(), Expr::Ident(n) if n == "super") => {
                        // super.method(args) — call parent prototype method with current `this`.
                        // Stack layout: [..., this, SuperClass.prototype.method, arg1..argN]
                        self.emit(Op::LoadThis);
                        self.emit_load_name("$$super$$");
                        let proto_ci = self.add_const(Constant::String(String::from("prototype")));
                        self.emit(Op::GetPropNamed(proto_ci));
                        let method_ci = self.add_const(Constant::String(property.clone()));
                        self.emit(Op::GetPropNamed(method_ci));
                        for arg in arguments {
                            self.compile_expr(arg);
                        }
                        self.emit(Op::CallMethod(arguments.len() as u8));
                    }
                    Expr::Member {
                        object, property, ..
                    } => {
                        // Stack: [..., this_obj, method_fn, arg1, ..., argN]
                        self.compile_expr(object); // push this
                        self.emit(Op::Dup); // dup for GetPropNamed
                        let ci = self.add_const(Constant::String(property.clone()));
                        self.emit(Op::GetPropNamed(ci)); // pop dup, push method
                        if Self::args_have_spread(arguments) {
                            self.compile_args_as_array(arguments);
                            self.emit(Op::CallMethodSpread);
                        } else {
                            for arg in arguments {
                                self.compile_expr(arg);
                            }
                            self.emit(Op::CallMethod(arguments.len() as u8));
                        }
                    }
                    Expr::Index { object, index } => {
                        // Computed method call: obj[expr](args)
                        self.compile_expr(object);
                        self.emit(Op::Dup);
                        self.compile_expr(index);
                        self.emit(Op::GetProp);
                        if Self::args_have_spread(arguments) {
                            self.compile_args_as_array(arguments);
                            self.emit(Op::CallMethodSpread);
                        } else {
                            for arg in arguments {
                                self.compile_expr(arg);
                            }
                            self.emit(Op::CallMethod(arguments.len() as u8));
                        }
                    }
                    _ => {
                        self.compile_expr(callee);
                        if Self::args_have_spread(arguments) {
                            self.compile_args_as_array(arguments);
                            self.emit(Op::CallSpread);
                        } else {
                            for arg in arguments {
                                self.compile_expr(arg);
                            }
                            self.emit(Op::Call(arguments.len() as u8));
                        }
                    }
                }
            }
            Expr::New { callee, arguments } => {
                self.compile_expr(callee);
                for arg in arguments {
                    self.compile_expr(arg);
                }
                self.emit(Op::New(arguments.len() as u8));
            }
            Expr::Unary { op, argument, .. } => {
                self.compile_expr(argument);
                match op {
                    UnaryOp::Neg => {
                        self.emit(Op::Neg);
                    }
                    UnaryOp::Pos => {
                        self.emit(Op::Pos);
                    }
                    UnaryOp::Not => {
                        self.emit(Op::Not);
                    }
                    UnaryOp::BitNot => {
                        self.emit(Op::BitNot);
                    }
                    UnaryOp::Typeof => {
                        self.emit(Op::Typeof);
                    }
                    UnaryOp::Void => {
                        self.emit(Op::Void);
                    }
                    UnaryOp::Delete => {
                        self.emit(Op::Pop);
                        self.emit(Op::LoadTrue);
                    }
                }
            }
            Expr::Update {
                op,
                argument,
                prefix,
            } => {
                self.compile_update(op, argument, *prefix);
            }
            Expr::Binary { op, left, right } => {
                self.compile_expr(left);
                self.compile_expr(right);
                match op {
                    BinaryOp::Add => {
                        self.emit(Op::Add);
                    }
                    BinaryOp::Sub => {
                        self.emit(Op::Sub);
                    }
                    BinaryOp::Mul => {
                        self.emit(Op::Mul);
                    }
                    BinaryOp::Div => {
                        self.emit(Op::Div);
                    }
                    BinaryOp::Mod => {
                        self.emit(Op::Mod);
                    }
                    BinaryOp::Exp => {
                        self.emit(Op::Exp);
                    }
                    BinaryOp::Eq => {
                        self.emit(Op::Eq);
                    }
                    BinaryOp::Ne => {
                        self.emit(Op::Ne);
                    }
                    BinaryOp::StrictEq => {
                        self.emit(Op::StrictEq);
                    }
                    BinaryOp::StrictNe => {
                        self.emit(Op::StrictNe);
                    }
                    BinaryOp::Lt => {
                        self.emit(Op::Lt);
                    }
                    BinaryOp::Le => {
                        self.emit(Op::Le);
                    }
                    BinaryOp::Gt => {
                        self.emit(Op::Gt);
                    }
                    BinaryOp::Ge => {
                        self.emit(Op::Ge);
                    }
                    BinaryOp::BitAnd => {
                        self.emit(Op::BitAnd);
                    }
                    BinaryOp::BitOr => {
                        self.emit(Op::BitOr);
                    }
                    BinaryOp::BitXor => {
                        self.emit(Op::BitXor);
                    }
                    BinaryOp::Shl => {
                        self.emit(Op::Shl);
                    }
                    BinaryOp::Shr => {
                        self.emit(Op::Shr);
                    }
                    BinaryOp::UShr => {
                        self.emit(Op::UShr);
                    }
                    BinaryOp::In => {
                        self.emit(Op::In);
                    }
                    BinaryOp::InstanceOf => {
                        self.emit(Op::InstanceOf);
                    }
                }
            }
            Expr::Logical { op, left, right } => {
                self.compile_expr(left);
                match op {
                    LogicalOp::And => {
                        self.emit(Op::Dup);
                        let skip = self.emit(Op::JumpIfFalse(0));
                        self.emit(Op::Pop);
                        self.compile_expr(right);
                        self.patch_jump(skip);
                    }
                    LogicalOp::Or => {
                        self.emit(Op::Dup);
                        let skip = self.emit(Op::JumpIfTrue(0));
                        self.emit(Op::Pop);
                        self.compile_expr(right);
                        self.patch_jump(skip);
                    }
                    LogicalOp::NullishCoalesce => {
                        let skip = self.emit(Op::JumpIfNullish(0));
                        let end = self.emit(Op::Jump(0));
                        self.patch_jump(skip);
                        self.emit(Op::Pop);
                        self.compile_expr(right);
                        self.patch_jump(end);
                    }
                }
            }
            Expr::Assign { op, left, right } => {
                self.compile_assignment(op, left, right);
            }
            Expr::Conditional {
                test,
                consequent,
                alternate,
            } => {
                self.compile_expr(test);
                let else_jump = self.emit(Op::JumpIfFalse(0));
                self.compile_expr(consequent);
                let end_jump = self.emit(Op::Jump(0));
                self.patch_jump(else_jump);
                self.compile_expr(alternate);
                self.patch_jump(end_jump);
            }
            Expr::Sequence(exprs) => {
                for (i, e) in exprs.iter().enumerate() {
                    self.compile_expr(e);
                    if i + 1 < exprs.len() {
                        self.emit(Op::Pop);
                    }
                }
            }
            Expr::FunctionExpr {
                name,
                params,
                body,
                is_async,
                is_generator,
            } => {
                if name.is_some() {
                    self.compile_function_named_expr_gen(
                        name.as_ref(),
                        params,
                        body,
                        *is_async,
                        *is_generator,
                    );
                } else {
                    self.compile_function_gen(
                        name.as_ref(),
                        params,
                        body,
                        *is_async,
                        *is_generator,
                    );
                }
            }
            Expr::Arrow {
                params,
                body,
                is_async,
            } => match body {
                ArrowBody::Block(stmts) => {
                    self.compile_arrow(params, stmts, *is_async);
                }
                ArrowBody::Expr(expr) => {
                    let return_stmt = Stmt::Return(Some(expr.as_ref().clone()));
                    self.compile_arrow(params, &[return_stmt], *is_async);
                }
            },
            Expr::Spread(inner) => {
                self.compile_expr(inner);
                self.emit(Op::Spread);
            }
            Expr::Typeof(inner) => {
                // typeof on an unresolvable global must return "undefined", not throw.
                if let Expr::Ident(name) = inner.as_ref() {
                    match self.resolve_name(name) {
                        NameLookup::Local(slot) => {
                            self.emit(Op::LoadLocal(slot));
                        }
                        NameLookup::Upvalue(idx) => {
                            self.emit(Op::LoadUpvalue(idx));
                        }
                        NameLookup::Global => {
                            let ci = self.add_const(Constant::String(name.to_string()));
                            self.emit(Op::LoadGlobalSafe(ci));
                        }
                    }
                } else {
                    self.compile_expr(inner);
                }
                self.emit(Op::Typeof);
            }
            Expr::Void(inner) => {
                self.compile_expr(inner);
                self.emit(Op::Void);
            }
            Expr::Delete(inner) => {
                match inner.as_ref() {
                    Expr::Member {
                        object, property, ..
                    } => {
                        self.compile_expr(object);
                        let ci = self.add_const(Constant::String(property.clone()));
                        self.emit(Op::LoadConst(ci));
                        self.emit(Op::Delete);
                    }
                    Expr::Index { object, index } => {
                        self.compile_expr(object);
                        self.compile_expr(index);
                        self.emit(Op::Delete);
                    }
                    Expr::Ident(ref name) if self.is_strict => {
                        // ES2023 §13.5.1.1: delete of plain identifier in strict mode is SyntaxError.
                        self.emit_throw_syntax_error(&alloc::format!(
                            "Delete of an unqualified identifier '{}' in strict mode",
                            name
                        ));
                    }
                    _ => {
                        self.emit(Op::LoadTrue);
                    }
                }
            }
            Expr::Yield(val) => {
                if let Some(v) = val {
                    self.compile_expr(v);
                } else {
                    self.emit(Op::LoadUndefined);
                }
                self.emit(Op::Yield);
            }
            Expr::YieldDelegate(inner) => {
                self.compile_expr(inner);
                self.emit(Op::YieldDelegate);
            }
            Expr::Await(inner) => {
                self.compile_expr(inner);
                self.emit(Op::Await);
            }
            Expr::ClassExpr {
                name,
                super_class,
                body,
            } => {
                let sc = super_class.as_ref().map(|b| b.as_ref().clone());
                self.compile_class(name.as_ref(), &sc, body);
            }
            Expr::OptionalChain { object, property } => {
                self.compile_expr(object);
                self.emit(Op::Dup);
                let skip = self.emit(Op::JumpIfNullish(0));
                let ci = self.add_const(Constant::String(property.clone()));
                self.emit(Op::GetPropNamed(ci));
                let end = self.emit(Op::Jump(0));
                self.patch_jump(skip);
                self.emit(Op::Pop);
                self.emit(Op::LoadUndefined);
                self.patch_jump(end);
            }
            Expr::OptionalCall { callee, arguments } => {
                // expr?.(args) — if expr is nullish, short-circuit to undefined.
                self.compile_expr(callee);
                self.emit(Op::Dup);
                let skip = self.emit(Op::JumpIfNullish(0));
                for arg in arguments {
                    self.compile_expr(arg);
                }
                self.emit(Op::Call(arguments.len() as u8));
                let end = self.emit(Op::Jump(0));
                self.patch_jump(skip);
                self.emit(Op::Pop);
                self.emit(Op::LoadUndefined);
                self.patch_jump(end);
            }
            Expr::TaggedTemplate { tag, template } => {
                // Tagged template: tag`str0${expr1}str1${expr2}str2`
                // → tag(["str0", "str1", "str2"], expr1, expr2)
                // Parse the template into static parts and expression sources
                let mut static_parts: Vec<String> = Vec::new();
                let mut expr_sources: Vec<String> = Vec::new();
                let bytes = template.as_bytes();
                let mut i = 0;
                let mut current = String::new();
                while i < bytes.len() {
                    if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                        static_parts.push(current.clone());
                        current.clear();
                        i += 2;
                        let mut depth = 1u32;
                        let mut expr_src = String::new();
                        while i < bytes.len() && depth > 0 {
                            match bytes[i] {
                                b'{' => {
                                    depth += 1;
                                    expr_src.push(bytes[i] as char);
                                }
                                b'}' => {
                                    depth -= 1;
                                    if depth > 0 {
                                        expr_src.push(b'}' as char);
                                    }
                                }
                                _ => {
                                    if bytes[i] < 0x80 {
                                        expr_src.push(bytes[i] as char);
                                    } else {
                                        let start = i;
                                        while i + 1 < bytes.len() && (bytes[i + 1] & 0xC0) == 0x80 {
                                            i += 1;
                                        }
                                        if let Ok(s) = core::str::from_utf8(&bytes[start..=i]) {
                                            expr_src.push_str(s);
                                        }
                                    }
                                }
                            }
                            i += 1;
                        }
                        expr_sources.push(expr_src);
                    } else if bytes[i] < 0x80 {
                        current.push(bytes[i] as char);
                        i += 1;
                    } else {
                        let start = i;
                        i += 1;
                        while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 {
                            i += 1;
                        }
                        if let Ok(s) = core::str::from_utf8(&bytes[start..i]) {
                            current.push_str(s);
                        }
                    }
                }
                static_parts.push(current);

                // Compile: push tag function
                self.compile_expr(tag);

                // Build the strings array (first argument)
                for sp in &static_parts {
                    let ci = self.add_const(Constant::String(sp.clone()));
                    self.emit(Op::LoadConst(ci));
                }
                self.emit(Op::NewArray(static_parts.len() as u16));

                // Compile each expression as additional arguments
                let argc = 1 + expr_sources.len(); // strings array + each expression value
                for expr_src in &expr_sources {
                    let tokens = crate::lexer::Lexer::tokenize(expr_src);
                    let mut p = crate::parser::Parser::new(tokens);
                    let prog = p.parse_program();
                    if let Some(Stmt::Expr(inner)) = prog.body.into_iter().next() {
                        self.compile_expr(&inner);
                    } else {
                        self.emit(Op::LoadUndefined);
                    }
                }

                self.emit(Op::Call(argc as u8));
            }
            Expr::RegExp { pattern, flags } => {
                let pi = self.add_const(Constant::String(pattern.clone()));
                let fi = self.add_const(Constant::String(flags.clone()));
                self.emit(Op::NewRegExp(pi, fi));
            }
            Expr::NewTarget => {
                self.emit(Op::NewTarget);
            }
        }
    }

    /// Check if an assignment operator is a logical assignment (&&=, ||=, ??=).
    fn is_logical_assign(op: &AssignOp) -> bool {
        matches!(
            op,
            AssignOp::AndAssign | AssignOp::OrAssign | AssignOp::NullishAssign
        )
    }

    /// Emit the short-circuit jump for a logical assignment.
    /// JumpIfFalse/JumpIfTrue/JumpIfNullish all PEEK (don't pop).
    /// For &&=: skip RHS if current value is falsy.
    /// For ||=: skip RHS if current value is truthy.
    /// For ??=: skip RHS if current value is NOT nullish.
    /// Returns (skip_label, needs_invert):
    ///   - For &&= and ||=: simple jump, needs_invert=false
    ///   - For ??=: jump if nullish to eval_rhs, then Jump(skip) for non-nullish
    fn emit_logical_skip(&mut self, op: &AssignOp) -> usize {
        match op {
            AssignOp::AndAssign => self.emit(Op::JumpIfFalse(0)),
            AssignOp::OrAssign => self.emit(Op::JumpIfTrue(0)),
            AssignOp::NullishAssign => {
                // JumpIfNullish jumps when IS nullish → need to invert:
                // if nullish → fall through to eval RHS
                // if NOT nullish → skip RHS
                let eval_rhs = self.emit(Op::JumpIfNullish(0));
                let skip = self.emit(Op::Jump(0)); // not nullish → skip
                self.patch_jump(eval_rhs);
                skip
            }
            _ => unreachable!(),
        }
    }

    fn compile_assignment(&mut self, op: &AssignOp, left: &Expr, right: &Expr) {
        match left {
            Expr::Ident(name) => {
                let name = name.clone();
                if Self::is_logical_assign(op) {
                    // x &&= expr: if x is falsy, leave x on stack; else eval expr, assign, leave new val.
                    // JumpIfFalse/True/Nullish all PEEK (value stays on stack).
                    self.emit_load_name(&name); // [old_val]
                    let skip = self.emit_logical_skip(op); // [old_val] — jumps if should skip RHS
                    self.emit(Op::Pop); // [] — discard old_val
                    self.compile_expr(right); // [new_val]
                    self.emit_store_name(&name); // [new_val]
                    self.patch_jump(skip);
                    // At skip: [old_val] still on stack as result
                } else if *op != AssignOp::Assign {
                    self.emit_load_name(&name);
                    self.compile_expr(right);
                    self.emit_compound_op(op);
                    self.emit_store_name(&name);
                } else {
                    self.compile_expr(right);
                    self.emit_store_name(&name);
                }
            }
            Expr::Member {
                object, property, ..
            } => {
                self.compile_expr(object);
                if Self::is_logical_assign(op) {
                    // obj.prop &&= expr: short-circuit on property value
                    self.emit(Op::Dup); // [obj, obj]
                    let ci = self.add_const(Constant::String(property.clone()));
                    self.emit(Op::GetPropNamed(ci)); // [obj, prop_val] — GetPropNamed pops obj copy
                    let skip = self.emit_logical_skip(op); // [obj, prop_val]
                    self.emit(Op::Pop); // [obj]
                    self.compile_expr(right); // [obj, new_val]
                    let ci2 = self.add_const(Constant::String(property.clone()));
                    self.emit(Op::SetPropNamed(ci2)); // [new_val] — SetPropNamed pops obj+val, pushes val
                    let done = self.emit(Op::Jump(0));
                    self.patch_jump(skip);
                    // Short-circuited: stack is [obj, old_val] — need just [old_val]
                    // Pop old_val temporarily, pop obj, then re-read property
                    // Simpler: just pop old_val, then re-read from obj (value hasn't changed)
                    self.emit(Op::Pop); // [obj]
                    let ci3 = self.add_const(Constant::String(property.clone()));
                    self.emit(Op::GetPropNamed(ci3)); // [prop_val]
                    self.patch_jump(done);
                } else if *op != AssignOp::Assign {
                    self.emit(Op::Dup);
                    let ci = self.add_const(Constant::String(property.clone()));
                    self.emit(Op::GetPropNamed(ci));
                    self.compile_expr(right);
                    self.emit_compound_op(op);
                    let ci2 = self.add_const(Constant::String(property.clone()));
                    self.emit(Op::SetPropNamed(ci2));
                } else {
                    self.compile_expr(right);
                    let ci = self.add_const(Constant::String(property.clone()));
                    self.emit(Op::SetPropNamed(ci));
                }
            }
            Expr::Index { object, index } => {
                self.compile_expr(object);
                self.compile_expr(index);
                if *op != AssignOp::Assign {
                    self.emit(Op::Dup); // dup key
                                        // Complex: need to get current value... simplified
                    self.compile_expr(right);
                    self.emit_compound_op(op);
                } else {
                    self.compile_expr(right);
                }
                self.emit(Op::SetProp);
            }
            Expr::Array(elements) if *op == AssignOp::Assign => {
                // Array destructuring assignment: [x, y, z] = expr
                // ES2023: must use iterator protocol (Symbol.iterator)
                self.compile_expr(right);
                self.emit(Op::Dup); // keep RHS value as result of the assignment expression
                self.emit(Op::GetIterator); // convert to iterator
                                            // Stack: [..., rhs_value, iterator]
                                            // Note: IterNext peeks the iterator from stack.last() without popping.
                                            // After IterNext, stack is [..., iter, value, has_more].
                                            // After consuming value and has_more, iter is back on top for next call.
                for elem in elements.iter() {
                    match elem {
                        None => {
                            // Elision: [,] — advance iterator without binding
                            self.emit(Op::IterNext);
                            self.emit(Op::Pop); // pop has_more
                            self.emit(Op::Pop); // pop value
                        }
                        Some(expr) => {
                            match expr {
                                Expr::Spread(inner) => {
                                    // Rest element: [...x] — collect remaining into array
                                    let result_arr =
                                        self.scope_mut().add_local(String::from("__dstr_rest__"));
                                    self.emit(Op::NewArray(0));
                                    self.emit(Op::StoreLocal(result_arr));
                                    self.emit(Op::Pop);
                                    let loop_top = self.offset();
                                    self.emit(Op::IterNext); // value, has_more
                                    let exit = self.emit(Op::JumpIfFalse(0));
                                    // has_more=true: push value into array
                                    let tmp =
                                        self.scope_mut().add_local(String::from("__dstr_tmp__"));
                                    self.emit(Op::StoreLocal(tmp));
                                    self.emit(Op::Pop);
                                    self.emit(Op::LoadLocal(result_arr));
                                    self.emit(Op::LoadLocal(tmp));
                                    self.emit(Op::ArrayPush);
                                    self.emit(Op::Pop);
                                    self.emit(Op::Jump(loop_top as i32 - self.offset() as i32 - 1));
                                    self.patch_jump(exit);
                                    self.emit(Op::Pop); // pop value (undefined when done)
                                    self.emit(Op::LoadLocal(result_arr));
                                    self.compile_assign_target(inner);
                                }
                                Expr::Assign {
                                    op: AssignOp::Assign,
                                    left,
                                    right: default,
                                } => {
                                    // Element with default: [x = default] = arr
                                    self.emit(Op::IterNext); // value, has_more
                                    self.emit(Op::Pop); // pop has_more
                                                        // If undefined, use default
                                    self.emit(Op::Dup);
                                    self.emit(Op::LoadUndefined);
                                    self.emit(Op::StrictEq);
                                    let skip = self.emit(Op::JumpIfFalse(0));
                                    self.emit(Op::Pop);
                                    self.compile_expr(default);
                                    self.patch_jump(skip);
                                    self.compile_assign_target(left);
                                }
                                _ => {
                                    self.emit(Op::IterNext); // value, has_more
                                    self.emit(Op::Pop); // pop has_more
                                    self.compile_assign_target(expr);
                                }
                            }
                        }
                    }
                }
                self.emit(Op::Pop); // pop the iterator
                                    // Stack: [..., rhs_value] — original RHS is the result
            }
            Expr::Object(props) if *op == AssignOp::Assign => {
                // Object destructuring assignment: {a, b, ...rest} = expr
                // ES2023 §13.15.5.3: RequireObjectCoercible — throw TypeError for null/undefined
                self.compile_expr(right);
                self.emit(Op::Dup); // keep RHS value as result
                                    // Emit RequireObjectCoercible check
                self.emit(Op::Dup);
                self.emit(Op::RequireObjectCoercible);
                self.emit(Op::Pop);

                // Collect excluded keys for rest element
                let mut excluded_keys: Vec<String> = Vec::new();
                let mut has_rest = false;
                let mut rest_target: Option<&Expr> = None;
                for prop in props {
                    let key_name = match &prop.key {
                        PropKey::Ident(k) | PropKey::String(k) => k.clone(),
                        PropKey::Number(n) => format!("{}", n),
                        PropKey::Computed(_) => continue,
                    };
                    if key_name == "..." {
                        has_rest = true;
                        rest_target = Some(&prop.value);
                    } else {
                        excluded_keys.push(key_name);
                    }
                }

                // Emit normal property extractions
                for prop in props {
                    let key_name = match &prop.key {
                        PropKey::Ident(k) | PropKey::String(k) => k.clone(),
                        PropKey::Number(n) => format!("{}", n),
                        PropKey::Computed(_) => continue,
                    };
                    if key_name == "..." {
                        continue;
                    }
                    self.emit(Op::Dup);
                    let ci = self.add_const(Constant::String(key_name));
                    self.emit(Op::GetPropNamed(ci));
                    // Check if value has a default (prop.value is Assign expr)
                    if let Expr::Assign {
                        op: AssignOp::Assign,
                        left,
                        right: default,
                    } = &prop.value
                    {
                        self.emit(Op::Dup);
                        self.emit(Op::LoadUndefined);
                        self.emit(Op::StrictEq);
                        let skip = self.emit(Op::JumpIfFalse(0));
                        self.emit(Op::Pop);
                        self.compile_expr(default);
                        self.patch_jump(skip);
                        self.compile_assign_target(left);
                    } else {
                        self.compile_assign_target(&prop.value);
                    }
                }

                // Emit rest element if present
                if has_rest {
                    if let Some(target) = rest_target {
                        self.emit(Op::Dup);
                        for key in &excluded_keys {
                            let ki = self.add_const(Constant::String(key.clone()));
                            self.emit(Op::LoadConst(ki));
                        }
                        let n = excluded_keys.len() as u8;
                        self.emit(Op::ObjectRest(n));
                        self.compile_assign_target(target);
                    }
                }

                self.emit(Op::Pop); // pop dup'd object, leave RHS as result
            }
            _ => {
                // ES2023 §13.15.1: Invalid assignment target — emit SyntaxError.
                self.emit_throw_syntax_error("Invalid left-hand side in assignment");
            }
        }
    }

    /// Compile an assignment target (for destructuring assignment).
    /// The value to assign is on top of the stack. This method pops it and stores it.
    fn compile_assign_target(&mut self, target: &Expr) {
        match target {
            Expr::Ident(name) => {
                self.emit_store_name(name);
                self.emit(Op::Pop); // emit_store_name leaves value on stack
            }
            Expr::Member {
                object, property, ..
            } => {
                // obj.prop = value — Stack: [value]
                // Save value to temp, compile object, load value, SetPropNamed
                let tmp = self.scope_mut().add_local(String::from("__assign_tmp__"));
                self.emit(Op::StoreLocal(tmp)); // save value
                self.emit(Op::Pop); // StoreLocal peeks, pop value
                self.compile_expr(object); // [obj]
                self.emit(Op::LoadLocal(tmp)); // [obj, value]
                let ci = self.add_const(Constant::String(property.clone()));
                self.emit(Op::SetPropNamed(ci)); // [value]
                self.emit(Op::Pop); // pop result
            }
            Expr::Index { object, index } => {
                // obj[key] = value — Stack: [value]
                let tmp = self.scope_mut().add_local(String::from("__assign_tmp__"));
                self.emit(Op::StoreLocal(tmp));
                self.emit(Op::Pop);
                self.compile_expr(object); // [obj]
                self.compile_expr(index); // [obj, key]
                self.emit(Op::LoadLocal(tmp)); // [obj, key, value]
                self.emit(Op::SetProp); // [value]
                self.emit(Op::Pop);
            }
            Expr::Array(elements) => {
                // Nested array destructuring: [[a, b]] = [[1, 2]]
                // Use iterator protocol. IterNext peeks the iterator without popping.
                self.emit(Op::GetIterator);
                for elem in elements.iter() {
                    match elem {
                        None => {
                            self.emit(Op::IterNext);
                            self.emit(Op::Pop);
                            self.emit(Op::Pop);
                        }
                        Some(Expr::Spread(inner)) => {
                            let result_arr =
                                self.scope_mut().add_local(String::from("__dstr_rest__"));
                            self.emit(Op::NewArray(0));
                            self.emit(Op::StoreLocal(result_arr));
                            self.emit(Op::Pop);
                            let loop_top = self.offset();
                            self.emit(Op::IterNext);
                            let exit = self.emit(Op::JumpIfFalse(0));
                            let tmp = self.scope_mut().add_local(String::from("__dstr_tmp__"));
                            self.emit(Op::StoreLocal(tmp));
                            self.emit(Op::Pop);
                            self.emit(Op::LoadLocal(result_arr));
                            self.emit(Op::LoadLocal(tmp));
                            self.emit(Op::ArrayPush);
                            self.emit(Op::Pop);
                            self.emit(Op::Jump(loop_top as i32 - self.offset() as i32 - 1));
                            self.patch_jump(exit);
                            self.emit(Op::Pop);
                            self.emit(Op::LoadLocal(result_arr));
                            self.compile_assign_target(inner);
                        }
                        Some(Expr::Assign {
                            op: AssignOp::Assign,
                            left,
                            right: default,
                        }) => {
                            self.emit(Op::IterNext);
                            self.emit(Op::Pop);
                            self.emit(Op::Dup);
                            self.emit(Op::LoadUndefined);
                            self.emit(Op::StrictEq);
                            let skip = self.emit(Op::JumpIfFalse(0));
                            self.emit(Op::Pop);
                            self.compile_expr(default);
                            self.patch_jump(skip);
                            self.compile_assign_target(left);
                        }
                        Some(e) => {
                            self.emit(Op::IterNext);
                            self.emit(Op::Pop);
                            self.compile_assign_target(e);
                        }
                    }
                }
                self.emit(Op::Pop); // pop iterator
            }
            Expr::Object(props) => {
                // Nested object destructuring
                // Collect excluded keys for rest
                let mut excluded_keys: Vec<String> = Vec::new();
                let mut has_rest = false;
                let mut rest_idx = 0;
                for (i, prop) in props.iter().enumerate() {
                    let key_name = match &prop.key {
                        PropKey::Ident(k) | PropKey::String(k) => k.clone(),
                        PropKey::Number(n) => format!("{}", n),
                        PropKey::Computed(_) => continue,
                    };
                    if key_name == "..." {
                        has_rest = true;
                        rest_idx = i;
                    } else {
                        excluded_keys.push(key_name);
                    }
                }

                for prop in props {
                    let key_name = match &prop.key {
                        PropKey::Ident(k) | PropKey::String(k) => k.clone(),
                        PropKey::Number(n) => format!("{}", n),
                        PropKey::Computed(_) => continue,
                    };
                    if key_name == "..." {
                        continue;
                    }
                    self.emit(Op::Dup);
                    let ci = self.add_const(Constant::String(key_name));
                    self.emit(Op::GetPropNamed(ci));
                    if let Expr::Assign {
                        op: AssignOp::Assign,
                        left,
                        right: default,
                    } = &prop.value
                    {
                        self.emit(Op::Dup);
                        self.emit(Op::LoadUndefined);
                        self.emit(Op::StrictEq);
                        let skip = self.emit(Op::JumpIfFalse(0));
                        self.emit(Op::Pop);
                        self.compile_expr(default);
                        self.patch_jump(skip);
                        self.compile_assign_target(left);
                    } else {
                        self.compile_assign_target(&prop.value);
                    }
                }
                if has_rest {
                    if let Some(prop) = props.get(rest_idx) {
                        self.emit(Op::Dup);
                        for key in &excluded_keys {
                            let ki = self.add_const(Constant::String(key.clone()));
                            self.emit(Op::LoadConst(ki));
                        }
                        let n = excluded_keys.len() as u8;
                        self.emit(Op::ObjectRest(n));
                        self.compile_assign_target(&prop.value);
                    }
                }
                self.emit(Op::Pop);
            }
            _ => {
                self.emit(Op::Pop); // discard value for unsupported targets
            }
        }
    }

    fn emit_compound_op(&mut self, op: &AssignOp) {
        match op {
            AssignOp::AddAssign => {
                self.emit(Op::Add);
            }
            AssignOp::SubAssign => {
                self.emit(Op::Sub);
            }
            AssignOp::MulAssign => {
                self.emit(Op::Mul);
            }
            AssignOp::DivAssign => {
                self.emit(Op::Div);
            }
            AssignOp::ModAssign => {
                self.emit(Op::Mod);
            }
            AssignOp::ExpAssign => {
                self.emit(Op::Exp);
            }
            AssignOp::BitAndAssign => {
                self.emit(Op::BitAnd);
            }
            AssignOp::BitOrAssign => {
                self.emit(Op::BitOr);
            }
            AssignOp::BitXorAssign => {
                self.emit(Op::BitXor);
            }
            AssignOp::ShlAssign => {
                self.emit(Op::Shl);
            }
            AssignOp::ShrAssign => {
                self.emit(Op::Shr);
            }
            AssignOp::UShrAssign => {
                self.emit(Op::UShr);
            }
            _ => {} // Logical assignments handled differently
        }
    }

    fn compile_update(&mut self, op: &UpdateOp, argument: &Expr, prefix: bool) {
        match argument {
            Expr::Ident(name) => {
                let name = name.clone();
                let lookup = self.resolve_name(&name);
                match lookup {
                    NameLookup::Local(slot) => {
                        if !prefix {
                            self.emit(Op::LoadLocal(slot)); // push old value (post-increment)
                        }
                        self.emit(Op::LoadLocal(slot));
                        match op {
                            UpdateOp::Inc => {
                                self.emit(Op::Inc);
                            }
                            UpdateOp::Dec => {
                                self.emit(Op::Dec);
                            }
                        }
                        self.emit(Op::StoreLocal(slot));
                        if prefix {
                            self.emit(Op::LoadLocal(slot)); // reload after store
                        } else {
                            self.emit(Op::Pop); // pop stored value, old value remains
                        }
                    }
                    NameLookup::Upvalue(idx) => {
                        if !prefix {
                            self.emit(Op::LoadUpvalue(idx));
                        }
                        self.emit(Op::LoadUpvalue(idx));
                        match op {
                            UpdateOp::Inc => {
                                self.emit(Op::Inc);
                            }
                            UpdateOp::Dec => {
                                self.emit(Op::Dec);
                            }
                        }
                        self.emit(Op::StoreUpvalue(idx));
                        if prefix {
                            self.emit(Op::LoadUpvalue(idx));
                        } else {
                            self.emit(Op::Pop);
                        }
                    }
                    NameLookup::Global => {
                        let ci = self.add_const(Constant::String(name.clone()));
                        if !prefix {
                            self.emit(Op::LoadGlobal(ci));
                        }
                        self.emit(Op::LoadGlobal(ci));
                        match op {
                            UpdateOp::Inc => {
                                self.emit(Op::Inc);
                            }
                            UpdateOp::Dec => {
                                self.emit(Op::Dec);
                            }
                        }
                        self.emit(Op::StoreGlobal(ci));
                        if prefix {
                            self.emit(Op::LoadGlobal(ci));
                        } else {
                            self.emit(Op::Pop);
                        }
                    }
                }
            }
            Expr::Member {
                object, property, ..
            } => {
                // Read-modify-write: obj.prop++ / ++obj.prop
                // Stack: [obj] → Dup → [obj, obj] → GetPropNamed → [obj, old_val]
                //      → Inc/Dec → [obj, new_val] → SetPropNamed → [new_val]
                // Note: for postfix the result should be old_val; since this case
                // is almost always used as a statement expression (result discarded),
                // we emit pre-increment semantics (result = new_val) as a simplification.
                self.compile_expr(object);
                self.emit(Op::Dup);
                let ci = self.add_const(Constant::String(property.clone()));
                self.emit(Op::GetPropNamed(ci));
                match op {
                    UpdateOp::Inc => {
                        self.emit(Op::Inc);
                    }
                    UpdateOp::Dec => {
                        self.emit(Op::Dec);
                    }
                }
                let ci2 = self.add_const(Constant::String(property.clone()));
                self.emit(Op::SetPropNamed(ci2));
                // SetPropNamed pops [obj, new_val], pushes new_val — that is the expression result.
            }
            _ => {
                // Index and other cases: simplified (no store-back).
                self.compile_expr(argument);
                match op {
                    UpdateOp::Inc => {
                        self.emit(Op::Inc);
                    }
                    UpdateOp::Dec => {
                        self.emit(Op::Dec);
                    }
                }
            }
        }
    }

    /// Returns true if any argument in a call expression is a spread (`...expr`).
    fn args_have_spread(args: &[Expr]) -> bool {
        args.iter().any(|a| matches!(a, Expr::Spread(_)))
    }

    /// Compile a list of call arguments into an Array on the stack,
    /// correctly handling spread elements (`...expr`).
    fn compile_args_as_array(&mut self, args: &[Expr]) {
        // Spread/ArrayPush use pop-modify-push semantics: no Dup needed.
        self.emit(Op::NewArray(0));
        for arg in args {
            if let Expr::Spread(inner) = arg {
                self.compile_expr(inner);
                self.emit(Op::Spread);
            } else {
                self.compile_expr(arg);
                self.emit(Op::ArrayPush);
            }
        }
    }

    fn begin_scope(&mut self) {
        self.scope_mut().scope_depth += 1;
    }

    fn end_scope(&mut self) {
        let depth = self.scope().scope_depth;
        while self.scope().locals.last().map(|l| l.depth) == Some(depth) {
            self.scope_mut().locals.pop();
        }
        self.scope_mut().scope_depth -= 1;
    }

    /// Collect all binding names from a parameter pattern.
    fn collect_param_names(pat: &Pattern) -> Vec<String> {
        let mut names = Vec::new();
        Self::collect_names_inner(pat, &mut names);
        names
    }

    fn collect_names_inner(pat: &Pattern, out: &mut Vec<String>) {
        match pat {
            Pattern::Ident(n) => out.push(n.clone()),
            Pattern::Array(elems) => {
                for e in elems.iter().flatten() {
                    Self::collect_names_inner(e, out);
                }
            }
            Pattern::Object(props) => {
                for prop in props {
                    Self::collect_names_inner(&prop.value, out);
                }
            }
            Pattern::Assign(inner, _) => Self::collect_names_inner(inner, out),
            Pattern::Rest(inner) => Self::collect_names_inner(inner, out),
        }
    }

    /// Emit bytecode that creates and throws a SyntaxError.
    fn emit_throw_syntax_error(&mut self, msg: &str) {
        // Build: throw new SyntaxError("<msg>")
        let se_idx = self.add_const(Constant::String(String::from("SyntaxError")));
        self.emit(Op::LoadGlobal(se_idx));
        let msg_idx = self.add_const(Constant::String(String::from(msg)));
        self.emit(Op::LoadConst(msg_idx));
        self.emit(Op::New(1));
        self.emit(Op::Throw);
    }
}
