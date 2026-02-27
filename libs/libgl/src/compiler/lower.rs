//! AST → IR lowering pass.
//!
//! Walks the AST and emits IR instructions, allocating registers for variables,
//! temporaries, and built-in outputs. Handles type constructors, swizzles,
//! built-in function calls, and matrix operations.

use alloc::string::String;
use alloc::vec::Vec;
use super::ast::*;
use super::ir::*;
use crate::types::*;

/// Lowering context tracking register allocation and variable mappings.
struct LowerCtx {
    insts: Vec<Inst>,
    next_reg: u32,
    /// Variable name → register mapping.
    vars: Vec<(String, u32, u32)>, // (name, reg, components)
    /// Shader type.
    shader_type: GLenum,
    /// Collected declarations.
    attributes: Vec<VarInfo>,
    varyings: Vec<VarInfo>,
    uniforms: Vec<VarInfo>,
}

impl LowerCtx {
    fn alloc_reg(&mut self) -> u32 {
        let r = self.next_reg;
        self.next_reg += 1;
        r
    }

    fn alloc_regs(&mut self, n: u32) -> u32 {
        let r = self.next_reg;
        self.next_reg += n;
        r
    }

    fn find_var(&self, name: &str) -> Option<(u32, u32)> {
        for (n, reg, comp) in self.vars.iter().rev() {
            if n == name { return Some((*reg, *comp)); }
        }
        None
    }
}

/// Lower an AST translation unit to IR.
pub fn lower(ast: &TranslationUnit, shader_type: GLenum) -> Result<Program, String> {
    let mut ctx = LowerCtx {
        insts: Vec::new(),
        next_reg: 0,
        vars: Vec::new(),
        shader_type,
        attributes: Vec::new(),
        varyings: Vec::new(),
        uniforms: Vec::new(),
    };

    // First pass: collect global declarations and allocate registers
    for decl in &ast.declarations {
        match decl {
            Declaration::Precision(_, _) => {} // ignored
            Declaration::Variable(var) => {
                let reg = if var.type_spec == TypeSpec::Mat4 {
                    ctx.alloc_regs(4) // 4 registers for mat4 columns
                } else if var.type_spec == TypeSpec::Mat3 {
                    ctx.alloc_regs(3)
                } else {
                    ctx.alloc_reg()
                };
                let comp = var.type_spec.components();

                let info = VarInfo {
                    name: var.name.clone(),
                    components: comp,
                    reg,
                };

                match var.qualifier {
                    StorageQualifier::Attribute => {
                        ctx.attributes.push(info);
                    }
                    StorageQualifier::Varying => {
                        ctx.varyings.push(info);
                    }
                    StorageQualifier::Uniform => {
                        ctx.uniforms.push(info);
                    }
                    _ => {}
                }

                ctx.vars.push((var.name.clone(), reg, comp));
            }
            Declaration::Function(_) => {} // handled in second pass
        }
    }

    // Emit load instructions for inputs
    for (i, attr) in ctx.attributes.iter().enumerate() {
        ctx.insts.push(Inst::LoadAttribute(attr.reg, i as u32));
    }
    for (i, uni) in ctx.uniforms.iter().enumerate() {
        if uni.components == 16 {
            // mat4: load into 4 consecutive registers
            for col in 0..4 {
                ctx.insts.push(Inst::LoadUniform(uni.reg + col, i as u32 * 4 + col));
            }
        } else {
            ctx.insts.push(Inst::LoadUniform(uni.reg, i as u32));
        }
    }
    if shader_type == GL_FRAGMENT_SHADER {
        for (i, vary) in ctx.varyings.iter().enumerate() {
            ctx.insts.push(Inst::LoadVarying(vary.reg, i as u32));
        }
    }

    // Second pass: lower function bodies (primarily 'main')
    for decl in &ast.declarations {
        if let Declaration::Function(func) = decl {
            lower_function_body(&mut ctx, func)?;
        }
    }

    Ok(Program {
        instructions: ctx.insts,
        num_regs: ctx.next_reg,
        attributes: ctx.attributes,
        varyings: ctx.varyings,
        uniforms: ctx.uniforms,
        locals: Vec::new(),
    })
}

fn lower_function_body(ctx: &mut LowerCtx, func: &FunctionDef) -> Result<(), String> {
    for stmt in &func.body {
        lower_stmt(ctx, stmt)?;
    }
    Ok(())
}

fn lower_stmt(ctx: &mut LowerCtx, stmt: &Stmt) -> Result<(), String> {
    match stmt {
        Stmt::VarDecl(var) => {
            let reg = if var.type_spec == TypeSpec::Mat4 {
                ctx.alloc_regs(4)
            } else if var.type_spec == TypeSpec::Mat3 {
                ctx.alloc_regs(3)
            } else {
                ctx.alloc_reg()
            };
            let comp = var.type_spec.components();
            ctx.vars.push((var.name.clone(), reg, comp));
            if let Some(init) = &var.initializer {
                let src = lower_expr(ctx, init)?;
                if comp == 16 {
                    for c in 0..4u32 {
                        ctx.insts.push(Inst::Mov(reg + c, src + c));
                    }
                } else {
                    ctx.insts.push(Inst::Mov(reg, src));
                }
            }
        }
        Stmt::Assign(lhs, rhs) => {
            let src = lower_expr(ctx, rhs)?;
            lower_assign(ctx, lhs, src)?;
        }
        Stmt::CompoundAssign(lhs, op, rhs) => {
            let lhs_val = lower_expr(ctx, lhs)?;
            let rhs_val = lower_expr(ctx, rhs)?;
            let result = ctx.alloc_reg();
            let inst = match op {
                CompoundOp::Add => Inst::Add(result, lhs_val, rhs_val),
                CompoundOp::Sub => Inst::Sub(result, lhs_val, rhs_val),
                CompoundOp::Mul => Inst::Mul(result, lhs_val, rhs_val),
                CompoundOp::Div => Inst::Div(result, lhs_val, rhs_val),
            };
            ctx.insts.push(inst);
            lower_assign(ctx, lhs, result)?;
        }
        Stmt::Return(Some(expr)) => {
            let val = lower_expr(ctx, expr)?;
            // In main(), return is implicit — but store output anyway
            if ctx.shader_type == GL_VERTEX_SHADER {
                ctx.insts.push(Inst::StorePosition(val));
            } else {
                ctx.insts.push(Inst::StoreFragColor(val));
            }
        }
        Stmt::Return(None) => {}
        Stmt::Expr(expr) => {
            lower_expr(ctx, expr)?;
        }
        Stmt::If(cond, then_body, else_body) => {
            // Simple flattened execution: evaluate both branches, select based on condition.
            // For Phase 1 this is sufficient; proper control flow in Phase 3.
            let cond_reg = lower_expr(ctx, cond)?;
            for s in then_body {
                lower_stmt(ctx, s)?;
            }
            if let Some(else_stmts) = else_body {
                for s in else_stmts {
                    lower_stmt(ctx, s)?;
                }
            }
            let _ = cond_reg; // TODO: proper conditional execution
        }
        Stmt::For(init, cond, inc, body) => {
            // Unrolled loop support would go here; for Phase 1 just execute body once
            lower_stmt(ctx, init)?;
            let _ = lower_expr(ctx, cond)?;
            for s in body {
                lower_stmt(ctx, s)?;
            }
            lower_stmt(ctx, inc)?;
        }
        Stmt::Discard => {} // Fragment-only, Phase 3
    }
    Ok(())
}

fn lower_assign(ctx: &mut LowerCtx, lhs: &Expr, src: u32) -> Result<(), String> {
    match lhs {
        Expr::Ident(name) => {
            if name == "gl_Position" {
                ctx.insts.push(Inst::StorePosition(src));
            } else if name == "gl_FragColor" {
                ctx.insts.push(Inst::StoreFragColor(src));
            } else if name == "gl_PointSize" {
                ctx.insts.push(Inst::StorePointSize(src));
            } else if let Some((reg, comp)) = ctx.find_var(name) {
                // Check if this is a varying being written in vertex shader
                if ctx.shader_type == GL_VERTEX_SHADER {
                    for (i, v) in ctx.varyings.iter().enumerate() {
                        if v.name == *name {
                            ctx.insts.push(Inst::StoreVarying(i as u32, src));
                            return Ok(());
                        }
                    }
                }
                if comp == 16 {
                    for c in 0..4u32 {
                        ctx.insts.push(Inst::Mov(reg + c, src + c));
                    }
                } else {
                    ctx.insts.push(Inst::Mov(reg, src));
                }
            }
        }
        Expr::Swizzle(base_expr, swiz) => {
            if let Expr::Ident(name) = base_expr.as_ref() {
                if let Some((reg, _)) = ctx.find_var(&name) {
                    // Write-masked swizzle assignment
                    let mask = swizzle_write_mask(swiz);
                    ctx.insts.push(Inst::WriteMask(reg, src, mask));
                    return Ok(());
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Estimate the component count of an expression without lowering it.
/// Used to detect mat4*vec4 → MatMul4 in binary multiply.
fn expr_component_hint(ctx: &LowerCtx, expr: &Expr) -> u32 {
    match expr {
        Expr::Ident(name) => {
            if let Some((_, comp)) = ctx.find_var(name) {
                comp
            } else {
                4
            }
        }
        Expr::Call(name, _) => match name.as_str() {
            "float" => 1,
            "vec2" => 2,
            "vec3" => 3,
            "vec4" => 4,
            "mat3" => 9,
            "mat4" => 16,
            _ => 4,
        },
        Expr::FloatLit(_) | Expr::IntLit(_) | Expr::BoolLit(_) => 1,
        Expr::Binary(lhs, _, _) => expr_component_hint(ctx, lhs),
        _ => 4,
    }
}

fn lower_expr(ctx: &mut LowerCtx, expr: &Expr) -> Result<u32, String> {
    match expr {
        Expr::FloatLit(v) => {
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::LoadConst(r, [*v, *v, *v, *v]));
            Ok(r)
        }
        Expr::IntLit(v) => {
            let f = *v as f32;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::LoadConst(r, [f, f, f, f]));
            Ok(r)
        }
        Expr::BoolLit(v) => {
            let f = if *v { 1.0 } else { 0.0 };
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::LoadConst(r, [f, f, f, f]));
            Ok(r)
        }
        Expr::Ident(name) => {
            if let Some((reg, _)) = ctx.find_var(name) {
                Ok(reg)
            } else {
                // Could be gl_Position etc. — return a temp
                let r = ctx.alloc_reg();
                ctx.insts.push(Inst::LoadConst(r, [0.0; 4]));
                Ok(r)
            }
        }
        Expr::Binary(lhs, op, rhs) => {
            // Detect mat*vec for matrix multiply before lowering operands
            let left_comp = expr_component_hint(ctx, lhs);
            let right_comp = expr_component_hint(ctx, rhs);

            let a = lower_expr(ctx, lhs)?;
            let b = lower_expr(ctx, rhs)?;
            let r = ctx.alloc_reg();
            let inst = match op {
                BinOp::Add => Inst::Add(r, a, b),
                BinOp::Sub => Inst::Sub(r, a, b),
                BinOp::Mul => {
                    if left_comp == 16 && right_comp <= 4 {
                        // mat4 * vec4 → matrix-vector multiply
                        Inst::MatMul4(r, a, b)
                    } else if left_comp == 9 && right_comp <= 3 {
                        // mat3 * vec3 → matrix-vector multiply
                        Inst::MatMul3(r, a, b)
                    } else {
                        Inst::Mul(r, a, b)
                    }
                }
                BinOp::Div => Inst::Div(r, a, b),
                BinOp::Less => Inst::CmpLt(r, a, b),
                BinOp::Greater => Inst::CmpLt(r, b, a),
                BinOp::Eq => Inst::CmpEq(r, a, b),
                BinOp::NotEq => {
                    ctx.insts.push(Inst::CmpEq(r, a, b));
                    let neg = ctx.alloc_reg();
                    let one = ctx.alloc_reg();
                    ctx.insts.push(Inst::LoadConst(one, [1.0; 4]));
                    ctx.insts.push(Inst::Sub(neg, one, r));
                    return Ok(neg);
                }
                BinOp::LessEq | BinOp::GreaterEq => {
                    // a <= b  ↔  !(b < a)
                    let (la, lb) = if matches!(op, BinOp::LessEq) { (b, a) } else { (a, b) };
                    ctx.insts.push(Inst::CmpLt(r, la, lb));
                    let neg = ctx.alloc_reg();
                    let one = ctx.alloc_reg();
                    ctx.insts.push(Inst::LoadConst(one, [1.0; 4]));
                    ctx.insts.push(Inst::Sub(neg, one, r));
                    return Ok(neg);
                }
                BinOp::And => Inst::Mul(r, a, b), // logical and via multiply
                BinOp::Or => {
                    // a || b = clamp(a + b, 0, 1)
                    ctx.insts.push(Inst::Add(r, a, b));
                    let zero = ctx.alloc_reg();
                    let one = ctx.alloc_reg();
                    ctx.insts.push(Inst::LoadConst(zero, [0.0; 4]));
                    ctx.insts.push(Inst::LoadConst(one, [1.0; 4]));
                    let clamped = ctx.alloc_reg();
                    ctx.insts.push(Inst::Clamp(clamped, r, zero, one));
                    return Ok(clamped);
                }
            };
            ctx.insts.push(inst);
            Ok(r)
        }
        Expr::Unary(UnaryOp::Neg, sub) => {
            let a = lower_expr(ctx, sub)?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Neg(r, a));
            Ok(r)
        }
        Expr::Unary(UnaryOp::Not, sub) => {
            let a = lower_expr(ctx, sub)?;
            let r = ctx.alloc_reg();
            let one = ctx.alloc_reg();
            ctx.insts.push(Inst::LoadConst(one, [1.0; 4]));
            ctx.insts.push(Inst::Sub(r, one, a));
            Ok(r)
        }
        Expr::Ternary(cond, then_expr, else_expr) => {
            let c = lower_expr(ctx, cond)?;
            let a = lower_expr(ctx, then_expr)?;
            let b = lower_expr(ctx, else_expr)?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Select(r, c, a, b));
            Ok(r)
        }
        Expr::Call(name, args) => lower_call(ctx, name, args),
        Expr::Swizzle(base, swiz) => {
            let src = lower_expr(ctx, base)?;
            let r = ctx.alloc_reg();
            let (indices, count) = parse_swizzle(swiz);
            ctx.insts.push(Inst::Swizzle(r, src, indices, count));
            Ok(r)
        }
        Expr::Field(base, field) => {
            // Treat as swizzle if single component
            let src = lower_expr(ctx, base)?;
            let r = ctx.alloc_reg();
            let (indices, count) = parse_swizzle(field);
            ctx.insts.push(Inst::Swizzle(r, src, indices, count));
            Ok(r)
        }
        Expr::Index(base, index) => {
            // For mat4[i] → access column register
            let base_reg = lower_expr(ctx, base)?;
            let _ = lower_expr(ctx, index)?;
            // Simplified: just return base for now (full indexing in Phase 3)
            Ok(base_reg)
        }
    }
}

fn lower_call(ctx: &mut LowerCtx, name: &str, args: &[Expr]) -> Result<u32, String> {
    match name {
        // ── Type constructors ───────────────────────────────────────────
        "float" => {
            let a = if args.is_empty() {
                let r = ctx.alloc_reg();
                ctx.insts.push(Inst::LoadConst(r, [0.0; 4]));
                r
            } else {
                lower_expr(ctx, &args[0])?
            };
            Ok(a)
        }
        "vec2" => {
            let r = ctx.alloc_reg();
            match args.len() {
                1 => {
                    let a = lower_expr(ctx, &args[0])?;
                    ctx.insts.push(Inst::Swizzle(r, a, [0, 0, 0, 0], 2));
                }
                2 => {
                    let a = lower_expr(ctx, &args[0])?;
                    let b = lower_expr(ctx, &args[1])?;
                    // Combine x from a, y from b
                    ctx.insts.push(Inst::LoadConst(r, [0.0; 4]));
                    ctx.insts.push(Inst::WriteMask(r, a, 0x1));
                    let tmp = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(tmp, b, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r, tmp, 0x2));
                }
                _ => { ctx.insts.push(Inst::LoadConst(r, [0.0; 4])); }
            }
            Ok(r)
        }
        "vec3" => {
            let r = ctx.alloc_reg();
            match args.len() {
                1 => {
                    let a = lower_expr(ctx, &args[0])?;
                    ctx.insts.push(Inst::Swizzle(r, a, [0, 0, 0, 0], 3));
                }
                3 => {
                    let x = lower_expr(ctx, &args[0])?;
                    let y = lower_expr(ctx, &args[1])?;
                    let z = lower_expr(ctx, &args[2])?;
                    ctx.insts.push(Inst::LoadConst(r, [0.0; 4]));
                    ctx.insts.push(Inst::WriteMask(r, x, 0x1));
                    let tmp_y = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(tmp_y, y, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r, tmp_y, 0x2));
                    let tmp_z = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(tmp_z, z, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r, tmp_z, 0x4));
                }
                _ => {
                    // vec3(vec2, float) or other combos — simplified
                    let a = lower_expr(ctx, &args[0])?;
                    ctx.insts.push(Inst::Mov(r, a));
                    if args.len() > 1 {
                        let b = lower_expr(ctx, &args[1])?;
                        let tmp = ctx.alloc_reg();
                        ctx.insts.push(Inst::Swizzle(tmp, b, [0, 0, 0, 0], 1));
                        ctx.insts.push(Inst::WriteMask(r, tmp, 0x4));
                    }
                }
            }
            Ok(r)
        }
        "vec4" => {
            let r = ctx.alloc_reg();
            match args.len() {
                1 => {
                    let a = lower_expr(ctx, &args[0])?;
                    ctx.insts.push(Inst::Swizzle(r, a, [0, 0, 0, 0], 4));
                }
                4 => {
                    let x = lower_expr(ctx, &args[0])?;
                    let y = lower_expr(ctx, &args[1])?;
                    let z = lower_expr(ctx, &args[2])?;
                    let w = lower_expr(ctx, &args[3])?;
                    ctx.insts.push(Inst::LoadConst(r, [0.0; 4]));
                    ctx.insts.push(Inst::WriteMask(r, x, 0x1));
                    let ty = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(ty, y, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r, ty, 0x2));
                    let tz = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(tz, z, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r, tz, 0x4));
                    let tw = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(tw, w, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r, tw, 0x8));
                }
                2 => {
                    // vec4(vec3, float) or vec4(vec2, vec2)
                    let a = lower_expr(ctx, &args[0])?;
                    let b = lower_expr(ctx, &args[1])?;
                    ctx.insts.push(Inst::Mov(r, a));
                    let tw = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(tw, b, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r, tw, 0x8));
                }
                3 => {
                    // vec4(vec2, float, float)
                    let a = lower_expr(ctx, &args[0])?;
                    let b = lower_expr(ctx, &args[1])?;
                    let c = lower_expr(ctx, &args[2])?;
                    ctx.insts.push(Inst::Mov(r, a));
                    let tz = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(tz, b, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r, tz, 0x4));
                    let tw = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(tw, c, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r, tw, 0x8));
                }
                _ => { ctx.insts.push(Inst::LoadConst(r, [0.0; 4])); }
            }
            Ok(r)
        }
        "mat4" => {
            let r = ctx.alloc_regs(4);
            if args.len() == 1 {
                // mat4(float) — diagonal matrix
                let diag = lower_expr(ctx, &args[0])?;
                let zero = ctx.alloc_reg();
                ctx.insts.push(Inst::LoadConst(zero, [0.0; 4]));
                for col in 0..4u32 {
                    ctx.insts.push(Inst::Mov(r + col, zero));
                    // Set diagonal element
                    let mask = 1u8 << col;
                    ctx.insts.push(Inst::WriteMask(r + col, diag, mask));
                }
            } else if args.len() == 16 {
                // mat4(16 floats) — column-major
                for col in 0..4u32 {
                    let c0 = lower_expr(ctx, &args[(col * 4) as usize])?;
                    let c1 = lower_expr(ctx, &args[(col * 4 + 1) as usize])?;
                    let c2 = lower_expr(ctx, &args[(col * 4 + 2) as usize])?;
                    let c3 = lower_expr(ctx, &args[(col * 4 + 3) as usize])?;
                    ctx.insts.push(Inst::LoadConst(r + col, [0.0; 4]));
                    ctx.insts.push(Inst::WriteMask(r + col, c0, 0x1));
                    let t1 = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(t1, c1, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r + col, t1, 0x2));
                    let t2 = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(t2, c2, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r + col, t2, 0x4));
                    let t3 = ctx.alloc_reg();
                    ctx.insts.push(Inst::Swizzle(t3, c3, [0, 0, 0, 0], 1));
                    ctx.insts.push(Inst::WriteMask(r + col, t3, 0x8));
                }
            }
            Ok(r)
        }
        "mat3" => {
            let r = ctx.alloc_regs(3);
            if args.len() == 1 {
                let diag = lower_expr(ctx, &args[0])?;
                let zero = ctx.alloc_reg();
                ctx.insts.push(Inst::LoadConst(zero, [0.0; 4]));
                for col in 0..3u32 {
                    ctx.insts.push(Inst::Mov(r + col, zero));
                    let mask = 1u8 << col;
                    ctx.insts.push(Inst::WriteMask(r + col, diag, mask));
                }
            }
            Ok(r)
        }

        // ── Built-in functions ──────────────────────────────────────────
        "texture2D" => {
            if args.len() < 2 { return Err(String::from("texture2D requires 2 args")); }
            let sampler = lower_expr(ctx, &args[0])?;
            let coord = lower_expr(ctx, &args[1])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::TexSample(r, sampler, coord));
            Ok(r)
        }
        "normalize" => {
            if args.is_empty() { return Err(String::from("normalize requires 1 arg")); }
            let a = lower_expr(ctx, &args[0])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Normalize(r, a));
            Ok(r)
        }
        "dot" => {
            if args.len() < 2 { return Err(String::from("dot requires 2 args")); }
            let a = lower_expr(ctx, &args[0])?;
            let b = lower_expr(ctx, &args[1])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Dp3(r, a, b));
            Ok(r)
        }
        "cross" => {
            if args.len() < 2 { return Err(String::from("cross requires 2 args")); }
            let a = lower_expr(ctx, &args[0])?;
            let b = lower_expr(ctx, &args[1])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Cross(r, a, b));
            Ok(r)
        }
        "length" => {
            if args.is_empty() { return Err(String::from("length requires 1 arg")); }
            let a = lower_expr(ctx, &args[0])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Length(r, a));
            Ok(r)
        }
        "clamp" => {
            if args.len() < 3 { return Err(String::from("clamp requires 3 args")); }
            let x = lower_expr(ctx, &args[0])?;
            let lo = lower_expr(ctx, &args[1])?;
            let hi = lower_expr(ctx, &args[2])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Clamp(r, x, lo, hi));
            Ok(r)
        }
        "mix" => {
            if args.len() < 3 { return Err(String::from("mix requires 3 args")); }
            let a = lower_expr(ctx, &args[0])?;
            let b = lower_expr(ctx, &args[1])?;
            let t = lower_expr(ctx, &args[2])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Mix(r, a, b, t));
            Ok(r)
        }
        "min" => {
            if args.len() < 2 { return Err(String::from("min requires 2 args")); }
            let a = lower_expr(ctx, &args[0])?;
            let b = lower_expr(ctx, &args[1])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Min(r, a, b));
            Ok(r)
        }
        "max" => {
            if args.len() < 2 { return Err(String::from("max requires 2 args")); }
            let a = lower_expr(ctx, &args[0])?;
            let b = lower_expr(ctx, &args[1])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Max(r, a, b));
            Ok(r)
        }
        "abs" => {
            if args.is_empty() { return Err(String::from("abs requires 1 arg")); }
            let a = lower_expr(ctx, &args[0])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Abs(r, a));
            Ok(r)
        }
        "pow" => {
            if args.len() < 2 { return Err(String::from("pow requires 2 args")); }
            let a = lower_expr(ctx, &args[0])?;
            let b = lower_expr(ctx, &args[1])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Pow(r, a, b));
            Ok(r)
        }
        "sqrt" => {
            if args.is_empty() { return Err(String::from("sqrt requires 1 arg")); }
            let a = lower_expr(ctx, &args[0])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Sqrt(r, a));
            Ok(r)
        }
        "inversesqrt" => {
            if args.is_empty() { return Err(String::from("inversesqrt requires 1 arg")); }
            let a = lower_expr(ctx, &args[0])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Rsqrt(r, a));
            Ok(r)
        }
        "sin" => {
            if args.is_empty() { return Err(String::from("sin requires 1 arg")); }
            let a = lower_expr(ctx, &args[0])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Sin(r, a));
            Ok(r)
        }
        "cos" => {
            if args.is_empty() { return Err(String::from("cos requires 1 arg")); }
            let a = lower_expr(ctx, &args[0])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Cos(r, a));
            Ok(r)
        }
        "reflect" => {
            if args.len() < 2 { return Err(String::from("reflect requires 2 args")); }
            let i = lower_expr(ctx, &args[0])?;
            let n = lower_expr(ctx, &args[1])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Reflect(r, i, n));
            Ok(r)
        }
        "floor" => {
            if args.is_empty() { return Err(String::from("floor requires 1 arg")); }
            let a = lower_expr(ctx, &args[0])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Floor(r, a));
            Ok(r)
        }
        "fract" => {
            if args.is_empty() { return Err(String::from("fract requires 1 arg")); }
            let a = lower_expr(ctx, &args[0])?;
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::Fract(r, a));
            Ok(r)
        }
        _ => {
            // Unknown function — return zero
            let r = ctx.alloc_reg();
            ctx.insts.push(Inst::LoadConst(r, [0.0; 4]));
            Ok(r)
        }
    }
}

/// Parse a swizzle string into component indices.
fn parse_swizzle(s: &str) -> ([u8; 4], u8) {
    let mut indices = [0u8; 4];
    let mut count = 0u8;
    for (i, c) in s.bytes().enumerate() {
        if i >= 4 { break; }
        indices[i] = match c {
            b'x' | b'r' | b's' => 0,
            b'y' | b'g' | b't' => 1,
            b'z' | b'b' | b'p' => 2,
            b'w' | b'a' | b'q' => 3,
            _ => 0,
        };
        count += 1;
    }
    (indices, count)
}

/// Compute a write mask from a swizzle string.
fn swizzle_write_mask(s: &str) -> u8 {
    let mut mask = 0u8;
    for c in s.bytes() {
        match c {
            b'x' | b'r' | b's' => mask |= 0x1,
            b'y' | b'g' | b't' => mask |= 0x2,
            b'z' | b'b' | b'p' => mask |= 0x4,
            b'w' | b'a' | b'q' => mask |= 0x8,
            _ => {}
        }
    }
    mask
}
