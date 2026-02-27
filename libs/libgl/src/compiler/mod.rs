//! GLSL ES 1.00 compiler.
//!
//! Pipeline: Source → [`lexer`] → Tokens → [`parser`] → AST → [`lower`] → IR.
//! The IR is executed by [`backend_sw`] (software interpreter) or translated
//! to DX9 SM 2.0 bytecode by [`backend_dx9`] for SVGA3D GPU acceleration.

pub mod lexer;
pub mod ast;
pub mod parser;
pub mod ir;
pub mod lower;
pub mod backend_sw;
pub mod backend_dx9;
pub mod backend_jit;

use alloc::string::String;
use crate::types::*;

/// Compile GLSL source into IR.
pub fn compile(source: &str, shader_type: GLenum) -> Result<ir::Program, String> {
    let tokens = lexer::tokenize(source)?;
    let ast = parser::parse(&tokens, shader_type)?;
    let program = lower::lower(&ast, shader_type)?;
    Ok(program)
}
