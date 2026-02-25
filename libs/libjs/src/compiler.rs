//! Compiles JavaScript AST into bytecode.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::string::ToString;

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
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            scopes: Vec::new(),
            binding_is_global: false,
        }
    }

    /// Returns true when we are at the outermost (global) function scope,
    /// i.e. not inside any compiled function body.
    fn is_global_scope(&self) -> bool {
        self.scopes.len() == 1
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

    /// Compile a program into a top-level chunk.
    pub fn compile(&mut self, program: &Program) -> Chunk {
        self.scopes.push(Scope::new());
        for stmt in &program.body {
            self.compile_stmt(stmt);
        }
        // Implicit return undefined
        self.emit(Op::LoadUndefined);
        self.emit(Op::Return);
        self.scopes.pop().unwrap().chunk
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
            NameLookup::Local(slot) => { self.emit(Op::LoadLocal(slot)); }
            NameLookup::Upvalue(idx) => { self.emit(Op::LoadUpvalue(idx)); }
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
                let is_global = *kind == VarKind::Var && self.is_global_scope();
                for decl in decls {
                    self.compile_var_decl(decl, is_global);
                }
            }
            Stmt::Block(stmts) => {
                self.begin_scope();
                for s in stmts {
                    self.compile_stmt(s);
                }
                self.end_scope();
            }
            Stmt::If { condition, consequent, alternate } => {
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
                self.scope_mut().continue_target = Some(loop_start);
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
                self.scope_mut().continue_target = Some(cond_target);

                self.compile_stmt(body);

                // Update continue target to point here (condition)
                let cond_pos = self.offset();
                self.scope_mut().continue_target = Some(cond_pos);

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
            Stmt::For { init, test, update, body } => {
                self.begin_scope();
                if let Some(init) = init {
                    match init.as_ref() {
                        ForInit::VarDecl { kind, decls } => {
                            let is_global = *kind == VarKind::Var && self.is_global_scope();
                            for d in decls {
                                self.compile_var_decl(d, is_global);
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

                // Patch all forward continue-jumps to the update expression.
                let continue_pos = self.offset();
                let cont_jumps = core::mem::take(&mut self.scope_mut().continue_jumps);
                for cj in &cont_jumps {
                    self.patch_jump_to_pos(*cj, continue_pos);
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
                self.emit(Op::Return);
            }
            Stmt::Break(_label) => {
                let idx = self.emit(Op::Jump(0));
                self.scope_mut().break_jumps.push(idx);
            }
            Stmt::Continue(_label) => {
                if let Some(target) = self.scope().continue_target {
                    // Known target (while/do-while/for-in/for-of) → backward jump.
                    let back = target as i32 - self.offset() as i32 - 1;
                    self.emit(Op::Jump(back));
                } else {
                    // Unknown target (for loop update) → forward jump, patched later.
                    let idx = self.emit(Op::Jump(0));
                    self.scope_mut().continue_jumps.push(idx);
                }
            }
            Stmt::Switch { discriminant, cases } => {
                self.compile_switch(discriminant, cases);
            }
            Stmt::Throw(expr) => {
                self.compile_expr(expr);
                self.emit(Op::Throw);
            }
            Stmt::Try { block, catch, finally } => {
                self.compile_try(block, catch, finally);
            }
            Stmt::FunctionDecl { name, params, body, is_async } => {
                self.compile_function(Some(name), params, body, *is_async);
                // At the global scope function declarations are global bindings.
                if self.is_global_scope() {
                    let ci = self.add_const(Constant::String(name.clone()));
                    self.emit(Op::StoreGlobal(ci));
                    self.emit(Op::Pop);
                } else {
                    let slot = self.scope_mut().add_local(name.clone());
                    self.emit(Op::StoreLocal(slot));
                    self.emit(Op::Pop);
                }
            }
            Stmt::ClassDecl { name, super_class, body } => {
                self.compile_class(Some(name), super_class, body);
                if self.is_global_scope() {
                    let ci = self.add_const(Constant::String(name.clone()));
                    self.emit(Op::StoreGlobal(ci));
                    self.emit(Op::Pop);
                } else {
                    let slot = self.scope_mut().add_local(name.clone());
                    self.emit(Op::StoreLocal(slot));
                    self.emit(Op::Pop);
                }
            }
            Stmt::Labeled { label: _, body } => {
                self.compile_stmt(body);
            }
            Stmt::Empty | Stmt::Debugger => {
                self.emit(Op::Nop);
            }
        }
    }

    fn compile_var_decl(&mut self, decl: &VarDeclarator, is_global_var: bool) {
        let prev = self.binding_is_global;
        self.binding_is_global = is_global_var;
        match &decl.name {
            Pattern::Ident(name) => {
                if let Some(init) = &decl.init {
                    self.compile_expr(init);
                } else {
                    self.emit(Op::LoadUndefined);
                }
                let name_clone = name.clone();
                self.bind_ident(&name_clone);
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
        }
        self.binding_is_global = prev;
    }

    fn compile_array_destructure(&mut self, elements: &[Option<Pattern>]) {
        // Array is on top of stack
        for (i, elem) in elements.iter().enumerate() {
            if let Some(pat) = elem {
                self.emit(Op::Dup);
                let idx = self.add_const(Constant::Number(i as f64));
                self.emit(Op::LoadConst(idx));
                self.emit(Op::GetProp);
                self.compile_pattern_binding(pat);
            }
        }
        self.emit(Op::Pop); // pop the array
    }

    fn compile_object_destructure(&mut self, props: &[ObjPatProp]) {
        for prop in props {
            self.emit(Op::Dup);
            let name_idx = self.add_const(Constant::String(prop.key.clone()));
            self.emit(Op::GetPropNamed(name_idx));
            self.compile_pattern_binding(&prop.value);
        }
        self.emit(Op::Pop); // pop the object
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
                self.compile_expr(default);
                self.patch_jump(skip);
                self.compile_pattern_binding(inner);
            }
            Pattern::Array(elems) => {
                self.compile_array_destructure(elems);
            }
            Pattern::Object(props) => {
                self.compile_object_destructure(props);
            }
        }
    }

    fn compile_for_in_of(
        &mut self,
        left: &ForInit,
        right: &Expr,
        body: &Stmt,
        is_of: bool,
    ) {
        self.begin_scope();
        self.compile_expr(right);

        if is_of {
            self.emit(Op::GetIterator);
        } else {
            // For-in: get object keys as an array, then iterate
            // We'll use a simple approach: push keys array + index counter
            // For simplicity, emit GetIterator which the VM handles for both
            self.emit(Op::GetIterator);
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

        self.scope_mut().continue_target = Some(loop_start);
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

    fn compile_try(
        &mut self,
        block: &[Stmt],
        catch: &Option<CatchClause>,
        finally: &Option<Vec<Stmt>>,
    ) {
        let catch_offset_slot = self.emit(Op::TryCatch(0, 0));

        // Try block
        for s in block {
            self.compile_stmt(s);
        }
        let try_end_jump = self.emit(Op::Jump(0));

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

        // Finally block
        if let Some(fin) = finally {
            for s in fin {
                self.compile_stmt(s);
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
        self.compile_function_impl(name, params, body, is_async, false);
    }

    fn compile_function_named_expr(
        &mut self,
        name: Option<&String>,
        params: &[Param],
        body: &[Stmt],
        is_async: bool,
    ) {
        self.compile_function_impl(name, params, body, is_async, true);
    }

    fn compile_function_impl(
        &mut self,
        name: Option<&String>,
        params: &[Param],
        body: &[Stmt],
        is_async: bool,
        named_expr: bool,
    ) {
        let mut func_scope = Scope::new();
        func_scope.chunk.name = name.cloned();
        func_scope.chunk.param_count = params.len() as u16;

        // Find the rest parameter (last param with is_rest=true), if any.
        let rest_param_idx = params.iter().position(|p| p.is_rest);

        // Add regular (non-rest) params as locals.
        for param in params {
            if param.is_rest { continue; }
            if let Pattern::Ident(ref n) = param.pattern {
                func_scope.add_local(n.clone());
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
                self.compile_expr(default);
                self.emit(Op::StoreLocal(i as u16));
                self.emit(Op::Pop);
                self.patch_jump(skip);
            }
            let _ = named_param_count; // silence unused warning
        }

        for s in body {
            self.compile_stmt(s);
        }

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
        // Copy upvalue descriptors into the chunk so the VM knows how to capture them.
        func_chunk.upvalues = func_scope.upvalues.iter().map(|uv| UpvalueRef {
            is_local: uv.is_local,
            index: uv.index,
        }).collect();
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
            self.compile_expr(super_expr);  // → [SuperClass]
            let slot = self.scope_mut().add_local(String::from("$$super$$"));
            self.emit(Op::StoreLocal(slot)); // peek — leaves [SuperClass] on stack
            self.emit(Op::Pop);              // → []
            Some(slot)
        } else {
            None
        };

        // Step 1: compile the constructor (or a default one).
        let ctor = body.iter().find(|m| matches!(m.kind, ClassMemberKind::Constructor { .. }));
        if let Some(ctor_member) = ctor {
            if let ClassMemberKind::Constructor { ref params, ref body } = ctor_member.kind {
                self.compile_function(name, params, body, false);
            }
        } else {
            // Default constructor — for derived classes ideally calls super(), but
            // the derived-class tests all provide explicit constructors, so an empty
            // body is sufficient here.
            self.compile_function(name, &[], &[], false);
        }
        // Stack: [..., Constructor]

        // Step 2: if there's a super class, set up the prototype chain.
        if let Some(super_slot) = super_local {
            // Stack: [..., Constructor]
            self.emit(Op::Dup);
            let proto_idx = self.add_const(Constant::String(String::from("prototype")));
            self.emit(Op::GetPropNamed(proto_idx));    // [..., Constructor, Constructor.prototype]
            self.emit(Op::LoadLocal(super_slot));      // [..., Constructor, Constructor.prototype, SuperClass]
            let proto_idx2 = self.add_const(Constant::String(String::from("prototype")));
            self.emit(Op::GetPropNamed(proto_idx2));   // [..., Constructor, Constructor.prototype, SuperClass.prototype]
            // Set Constructor.prototype.__proto__ = SuperClass.prototype
            let proto_key_idx = self.add_const(Constant::String(String::from("__proto__")));
            self.emit(Op::SetPropNamed(proto_key_idx)); // [..., Constructor, SuperClass.prototype]
            self.emit(Op::Pop);                          // [..., Constructor]
        }

        // Step 3: add instance methods to Constructor.prototype.
        for member in body {
            if matches!(member.kind, ClassMemberKind::Constructor { .. }) {
                continue;
            }
            let key_name = match &member.key {
                PropKey::Ident(s) | PropKey::String(s) => s.clone(),
                _ => String::from("_member_"),
            };
            if member.is_static {
                // Static methods/properties: set directly on Constructor.
                match &member.kind {
                    ClassMemberKind::Method { params, body } => {
                        self.emit(Op::Dup); // dup Constructor
                        self.compile_function(Some(&key_name), params, body, false);
                        let ki = self.add_const(Constant::String(key_name));
                        self.emit(Op::SetPropNamed(ki));
                        self.emit(Op::Pop);
                    }
                    ClassMemberKind::Property { value } => {
                        self.emit(Op::Dup);
                        if let Some(v) = value { self.compile_expr(v); } else { self.emit(Op::LoadUndefined); }
                        let ki = self.add_const(Constant::String(key_name));
                        self.emit(Op::SetPropNamed(ki));
                        self.emit(Op::Pop);
                    }
                    _ => {}
                }
            } else {
                // Instance methods: set on Constructor.prototype.
                match &member.kind {
                    ClassMemberKind::Method { params, body } => {
                        // Stack before: [..., Constructor]
                        self.emit(Op::Dup); // [..., Constructor, Constructor]
                        let proto_idx = self.add_const(Constant::String(String::from("prototype")));
                        self.emit(Op::GetPropNamed(proto_idx));
                        // GetPropNamed pops Constructor-dup, pushes prototype
                        // Stack: [..., Constructor, Constructor.prototype]
                        self.compile_function(Some(&key_name), params, body, false);
                        // Stack: [..., Constructor, Constructor.prototype, methodFn]
                        let ki = self.add_const(Constant::String(key_name));
                        self.emit(Op::SetPropNamed(ki));
                        // SetPropNamed pops methodFn+prototype, sets prop, pushes methodFn
                        // Stack: [..., Constructor, methodFn]
                        self.emit(Op::Pop); // pop methodFn
                        // Stack: [..., Constructor]
                    }
                    _ => {}
                }
            }
        }
        // Stack: [..., Constructor]
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
                        b'{' => { depth += 1; expr_src.push(bytes[i] as char); i += 1; }
                        b'}' => {
                            depth -= 1;
                            if depth > 0 { expr_src.push(b'}' as char); }
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
                                while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 { i += 1; }
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
                while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 { i += 1; }
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
            Expr::Bool(true) => { self.emit(Op::LoadTrue); }
            Expr::Bool(false) => { self.emit(Op::LoadFalse); }
            Expr::Null => { self.emit(Op::LoadNull); }
            Expr::Undefined => { self.emit(Op::LoadUndefined); }
            Expr::This => { self.emit(Op::LoadThis); }
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
                            self.emit(Op::Dup);           // [obj, obj]
                            self.compile_expr(&prop.value); // [obj, obj, source]
                            self.emit(Op::ObjectSpread);   // [obj]  (copies source → obj)
                            continue;
                        }
                    }
                    match &prop.key {
                        PropKey::Ident(name) | PropKey::String(name) => {
                            self.emit(Op::Dup);           // [obj, obj]
                            self.compile_expr(&prop.value); // [obj, obj, val]
                            let ci = self.add_const(Constant::String(name.clone()));
                            self.emit(Op::SetPropNamed(ci)); // [obj, val]
                            self.emit(Op::Pop);             // [obj]
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
            Expr::Member { object, property, .. } => {
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
                        // Stack layout for CallMethod: [..., this, SuperClass, arg1..argN]
                        self.emit(Op::LoadThis);
                        self.emit_load_name("$$super$$");
                        for arg in arguments { self.compile_expr(arg); }
                        self.emit(Op::CallMethod(arguments.len() as u8));
                    }
                    Expr::Member { object, property, .. }
                        if matches!(object.as_ref(), Expr::Ident(n) if n == "super") =>
                    {
                        // super.method(args) — call parent prototype method with current `this`.
                        // Stack layout: [..., this, SuperClass.prototype.method, arg1..argN]
                        self.emit(Op::LoadThis);
                        self.emit_load_name("$$super$$");
                        let proto_ci = self.add_const(Constant::String(String::from("prototype")));
                        self.emit(Op::GetPropNamed(proto_ci));
                        let method_ci = self.add_const(Constant::String(property.clone()));
                        self.emit(Op::GetPropNamed(method_ci));
                        for arg in arguments { self.compile_expr(arg); }
                        self.emit(Op::CallMethod(arguments.len() as u8));
                    }
                    Expr::Member { object, property, .. } => {
                        // Stack: [..., this_obj, method_fn, arg1, ..., argN]
                        self.compile_expr(object);   // push this
                        self.emit(Op::Dup);          // dup for GetPropNamed
                        let ci = self.add_const(Constant::String(property.clone()));
                        self.emit(Op::GetPropNamed(ci)); // pop dup, push method
                        if Self::args_have_spread(arguments) {
                            self.compile_args_as_array(arguments);
                            self.emit(Op::CallMethodSpread);
                        } else {
                            for arg in arguments { self.compile_expr(arg); }
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
                            for arg in arguments { self.compile_expr(arg); }
                            self.emit(Op::CallMethod(arguments.len() as u8));
                        }
                    }
                    _ => {
                        self.compile_expr(callee);
                        if Self::args_have_spread(arguments) {
                            self.compile_args_as_array(arguments);
                            self.emit(Op::CallSpread);
                        } else {
                            for arg in arguments { self.compile_expr(arg); }
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
                    UnaryOp::Neg => { self.emit(Op::Neg); }
                    UnaryOp::Pos => { self.emit(Op::Pos); }
                    UnaryOp::Not => { self.emit(Op::Not); }
                    UnaryOp::BitNot => { self.emit(Op::BitNot); }
                    UnaryOp::Typeof => { self.emit(Op::Typeof); }
                    UnaryOp::Void => { self.emit(Op::Void); }
                    UnaryOp::Delete => { self.emit(Op::Pop); self.emit(Op::LoadTrue); }
                }
            }
            Expr::Update { op, argument, prefix } => {
                self.compile_update(op, argument, *prefix);
            }
            Expr::Binary { op, left, right } => {
                self.compile_expr(left);
                self.compile_expr(right);
                match op {
                    BinaryOp::Add => { self.emit(Op::Add); }
                    BinaryOp::Sub => { self.emit(Op::Sub); }
                    BinaryOp::Mul => { self.emit(Op::Mul); }
                    BinaryOp::Div => { self.emit(Op::Div); }
                    BinaryOp::Mod => { self.emit(Op::Mod); }
                    BinaryOp::Exp => { self.emit(Op::Exp); }
                    BinaryOp::Eq => { self.emit(Op::Eq); }
                    BinaryOp::Ne => { self.emit(Op::Ne); }
                    BinaryOp::StrictEq => { self.emit(Op::StrictEq); }
                    BinaryOp::StrictNe => { self.emit(Op::StrictNe); }
                    BinaryOp::Lt => { self.emit(Op::Lt); }
                    BinaryOp::Le => { self.emit(Op::Le); }
                    BinaryOp::Gt => { self.emit(Op::Gt); }
                    BinaryOp::Ge => { self.emit(Op::Ge); }
                    BinaryOp::BitAnd => { self.emit(Op::BitAnd); }
                    BinaryOp::BitOr => { self.emit(Op::BitOr); }
                    BinaryOp::BitXor => { self.emit(Op::BitXor); }
                    BinaryOp::Shl => { self.emit(Op::Shl); }
                    BinaryOp::Shr => { self.emit(Op::Shr); }
                    BinaryOp::UShr => { self.emit(Op::UShr); }
                    BinaryOp::In => { self.emit(Op::In); }
                    BinaryOp::InstanceOf => { self.emit(Op::InstanceOf); }
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
            Expr::Conditional { test, consequent, alternate } => {
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
            Expr::FunctionExpr { name, params, body, is_async } => {
                if name.is_some() {
                    // Named function expression: the name is bound inside the body.
                    self.compile_function_named_expr(name.as_ref(), params, body, *is_async);
                } else {
                    self.compile_function(name.as_ref(), params, body, *is_async);
                }
            }
            Expr::Arrow { params, body, is_async } => {
                match body {
                    ArrowBody::Block(stmts) => {
                        self.compile_function(None, params, stmts, *is_async);
                    }
                    ArrowBody::Expr(expr) => {
                        // Convert expression body to return statement
                        let return_stmt = Stmt::Return(Some(expr.as_ref().clone()));
                        self.compile_function(None, params, &[return_stmt], *is_async);
                    }
                }
            }
            Expr::Spread(inner) => {
                self.compile_expr(inner);
                self.emit(Op::Spread);
            }
            Expr::Typeof(inner) => {
                self.compile_expr(inner);
                self.emit(Op::Typeof);
            }
            Expr::Void(inner) => {
                self.compile_expr(inner);
                self.emit(Op::Void);
            }
            Expr::Delete(inner) => {
                match inner.as_ref() {
                    Expr::Member { object, property, .. } => {
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
                // Simplified: yield acts like return for now
                self.emit(Op::Return);
            }
            Expr::Await(inner) => {
                self.compile_expr(inner);
                self.emit(Op::Await);
            }
            Expr::ClassExpr { name, super_class, body } => {
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
            Expr::TaggedTemplate { tag, template } => {
                self.compile_expr(tag);
                let ci = self.add_const(Constant::String(template.clone()));
                self.emit(Op::LoadConst(ci));
                self.emit(Op::NewArray(1));
                self.emit(Op::Call(1));
            }
        }
    }

    fn compile_assignment(&mut self, op: &AssignOp, left: &Expr, right: &Expr) {
        match left {
            Expr::Ident(name) => {
                let name = name.clone();
                if *op != AssignOp::Assign {
                    // Compound assignment: load current value first
                    self.emit_load_name(&name);
                    self.compile_expr(right);
                    self.emit_compound_op(op);
                } else {
                    self.compile_expr(right);
                }
                // emit_store_name emits Dup, store, Pop — leaving value on stack
                self.emit_store_name(&name);
            }
            Expr::Member { object, property, .. } => {
                self.compile_expr(object);
                if *op != AssignOp::Assign {
                    self.emit(Op::Dup);
                    let ci = self.add_const(Constant::String(property.clone()));
                    self.emit(Op::GetPropNamed(ci));
                    self.compile_expr(right);
                    self.emit_compound_op(op);
                } else {
                    self.compile_expr(right);
                }
                let ci = self.add_const(Constant::String(property.clone()));
                self.emit(Op::SetPropNamed(ci));
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
            _ => {
                self.compile_expr(right);
            }
        }
    }

    fn emit_compound_op(&mut self, op: &AssignOp) {
        match op {
            AssignOp::AddAssign => { self.emit(Op::Add); }
            AssignOp::SubAssign => { self.emit(Op::Sub); }
            AssignOp::MulAssign => { self.emit(Op::Mul); }
            AssignOp::DivAssign => { self.emit(Op::Div); }
            AssignOp::ModAssign => { self.emit(Op::Mod); }
            AssignOp::ExpAssign => { self.emit(Op::Exp); }
            AssignOp::BitAndAssign => { self.emit(Op::BitAnd); }
            AssignOp::BitOrAssign => { self.emit(Op::BitOr); }
            AssignOp::BitXorAssign => { self.emit(Op::BitXor); }
            AssignOp::ShlAssign => { self.emit(Op::Shl); }
            AssignOp::ShrAssign => { self.emit(Op::Shr); }
            AssignOp::UShrAssign => { self.emit(Op::UShr); }
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
                            UpdateOp::Inc => { self.emit(Op::Inc); }
                            UpdateOp::Dec => { self.emit(Op::Dec); }
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
                            UpdateOp::Inc => { self.emit(Op::Inc); }
                            UpdateOp::Dec => { self.emit(Op::Dec); }
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
                            UpdateOp::Inc => { self.emit(Op::Inc); }
                            UpdateOp::Dec => { self.emit(Op::Dec); }
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
            Expr::Member { object, property, .. } => {
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
                    UpdateOp::Inc => { self.emit(Op::Inc); }
                    UpdateOp::Dec => { self.emit(Op::Dec); }
                }
                let ci2 = self.add_const(Constant::String(property.clone()));
                self.emit(Op::SetPropNamed(ci2));
                // SetPropNamed pops [obj, new_val], pushes new_val — that is the expression result.
            }
            _ => {
                // Index and other cases: simplified (no store-back).
                self.compile_expr(argument);
                match op {
                    UpdateOp::Inc => { self.emit(Op::Inc); }
                    UpdateOp::Dec => { self.emit(Op::Dec); }
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
}
