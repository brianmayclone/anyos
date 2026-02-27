//! Shader and program object management.
//!
//! Handles `glCreateShader`, `glShaderSource`, `glCompileShader`, `glCreateProgram`,
//! `glAttachShader`, `glLinkProgram`, `glUseProgram`, and uniform/attribute binding.

use alloc::string::String;
use alloc::vec::Vec;
use crate::types::*;
use crate::compiler;
use crate::compiler::backend_jit::{JitCode, JitFn};

/// Maximum uniforms per program.
pub const MAX_UNIFORMS: usize = 64;

/// Maximum varyings per program.
pub const MAX_VARYINGS: usize = 16;

/// A compiled shader object.
pub struct GlShader {
    pub shader_type: GLenum,
    pub source: String,
    pub compiled: bool,
    pub info_log: String,
    pub ir: Option<compiler::ir::Program>,
}

/// A uniform variable in a linked program.
#[derive(Clone)]
pub struct UniformInfo {
    pub name: String,
    /// Location index (0-based).
    pub location: i32,
    /// Number of float components (1, 2, 3, 4, 9, 16).
    pub size: u32,
    /// Current value stored as up to 16 floats.
    pub value: [f32; 16],
    /// For sampler uniforms: texture unit index.
    pub sampler_unit: i32,
}

/// An attribute variable in a linked program.
#[derive(Clone)]
pub struct AttribInfo {
    pub name: String,
    /// Attribute location (matches vertex attrib array index).
    pub location: i32,
}

/// A varying variable passed from vertex to fragment shader.
#[derive(Clone)]
pub struct VaryingInfo {
    pub name: String,
    /// Number of components (1..4).
    pub components: u32,
    /// Index into the varying array.
    pub index: usize,
}

/// A linked shader program.
pub struct GlProgram {
    pub vertex_shader: u32,
    pub fragment_shader: u32,
    pub linked: bool,
    pub info_log: String,
    pub uniforms: Vec<UniformInfo>,
    pub attributes: Vec<AttribInfo>,
    pub varyings: Vec<VaryingInfo>,
    /// Total number of varying float components.
    pub varying_count: usize,
    /// Compiled vertex shader IR.
    pub vs_ir: Option<compiler::ir::Program>,
    /// Compiled fragment shader IR.
    pub fs_ir: Option<compiler::ir::Program>,
    /// Manual attribute location bindings (glBindAttribLocation).
    pub attrib_bindings: Vec<(String, i32)>,
    /// JIT-compiled vertex shader (cached, compiled on first draw).
    pub vs_jit: Option<JitCode>,
    /// JIT-compiled fragment shader (cached, compiled on first draw).
    pub fs_jit: Option<JitCode>,
}

/// Storage for shader and program objects.
pub struct ShaderStore {
    shaders: Vec<Option<GlShader>>,
    programs: Vec<Option<GlProgram>>,
    next_shader_id: u32,
    next_program_id: u32,
}

impl ShaderStore {
    /// Create an empty shader store.
    pub fn new() -> Self {
        Self {
            shaders: Vec::new(),
            programs: Vec::new(),
            next_shader_id: 1,
            next_program_id: 1,
        }
    }

    /// Create a shader object, returns its id.
    pub fn create_shader(&mut self, shader_type: GLenum) -> u32 {
        let id = self.next_shader_id;
        self.next_shader_id += 1;
        while self.shaders.len() <= id as usize {
            self.shaders.push(None);
        }
        self.shaders[id as usize] = Some(GlShader {
            shader_type,
            source: String::new(),
            compiled: false,
            info_log: String::new(),
            ir: None,
        });
        id
    }

    /// Delete a shader object.
    pub fn delete_shader(&mut self, id: u32) {
        if (id as usize) < self.shaders.len() {
            self.shaders[id as usize] = None;
        }
    }

    /// Get a reference to a shader.
    pub fn get_shader(&self, id: u32) -> Option<&GlShader> {
        if id == 0 { return None; }
        self.shaders.get(id as usize).and_then(|s| s.as_ref())
    }

    /// Get a mutable reference to a shader.
    pub fn get_shader_mut(&mut self, id: u32) -> Option<&mut GlShader> {
        if id == 0 { return None; }
        self.shaders.get_mut(id as usize).and_then(|s| s.as_mut())
    }

    /// Create a program object, returns its id.
    pub fn create_program(&mut self) -> u32 {
        let id = self.next_program_id;
        self.next_program_id += 1;
        while self.programs.len() <= id as usize {
            self.programs.push(None);
        }
        self.programs[id as usize] = Some(GlProgram {
            vertex_shader: 0,
            fragment_shader: 0,
            linked: false,
            info_log: String::new(),
            uniforms: Vec::new(),
            attributes: Vec::new(),
            varyings: Vec::new(),
            varying_count: 0,
            vs_ir: None,
            fs_ir: None,
            attrib_bindings: Vec::new(),
            vs_jit: None,
            fs_jit: None,
        });
        id
    }

    /// Delete a program object.
    pub fn delete_program(&mut self, id: u32) {
        if (id as usize) < self.programs.len() {
            self.programs[id as usize] = None;
        }
    }

    /// Get a reference to a program.
    pub fn get_program(&self, id: u32) -> Option<&GlProgram> {
        if id == 0 { return None; }
        self.programs.get(id as usize).and_then(|s| s.as_ref())
    }

    /// Get a mutable reference to a program.
    pub fn get_program_mut(&mut self, id: u32) -> Option<&mut GlProgram> {
        if id == 0 { return None; }
        self.programs.get_mut(id as usize).and_then(|s| s.as_mut())
    }

    /// Compile a shader from its source.
    pub fn compile_shader(&mut self, id: u32) {
        let shader = match self.get_shader_mut(id) {
            Some(s) => s,
            None => return,
        };

        match compiler::compile(&shader.source, shader.shader_type) {
            Ok(ir) => {
                shader.compiled = true;
                shader.info_log.clear();
                shader.ir = Some(ir);
            }
            Err(msg) => {
                shader.compiled = false;
                shader.info_log = msg;
                shader.ir = None;
            }
        }
    }

    /// Link a program from its attached shaders.
    pub fn link_program(&mut self, program_id: u32) {
        // Gather shader IR
        let (vs_id, fs_id, bindings) = {
            let prog = match self.get_program(program_id) {
                Some(p) => p,
                None => return,
            };
            (prog.vertex_shader, prog.fragment_shader, prog.attrib_bindings.clone())
        };

        let vs_ir = match self.get_shader(vs_id).and_then(|s| s.ir.clone()) {
            Some(ir) => ir,
            None => {
                if let Some(prog) = self.get_program_mut(program_id) {
                    prog.linked = false;
                    prog.info_log = String::from("Vertex shader not compiled");
                }
                return;
            }
        };

        let fs_ir = match self.get_shader(fs_id).and_then(|s| s.ir.clone()) {
            Some(ir) => ir,
            None => {
                if let Some(prog) = self.get_program_mut(program_id) {
                    prog.linked = false;
                    prog.info_log = String::from("Fragment shader not compiled");
                }
                return;
            }
        };

        // Collect uniforms from both shaders
        let mut uniforms = Vec::new();
        let mut loc = 0i32;
        for u in vs_ir.uniforms.iter().chain(fs_ir.uniforms.iter()) {
            if uniforms.iter().any(|existing: &UniformInfo| existing.name == u.name) {
                continue;
            }
            uniforms.push(UniformInfo {
                name: u.name.clone(),
                location: loc,
                size: u.components,
                value: [0.0; 16],
                sampler_unit: 0,
            });
            loc += 1;
        }

        // Collect attributes from vertex shader
        let mut attributes = Vec::new();
        for (i, a) in vs_ir.attributes.iter().enumerate() {
            let bound_loc = bindings.iter()
                .find(|(n, _)| n == &a.name)
                .map(|(_, l)| *l);
            attributes.push(AttribInfo {
                name: a.name.clone(),
                location: bound_loc.unwrap_or(i as i32),
            });
        }

        // Collect varyings from vertex shader
        let mut varyings = Vec::new();
        let mut varying_offset = 0usize;
        for v in vs_ir.varyings.iter() {
            varyings.push(VaryingInfo {
                name: v.name.clone(),
                components: v.components,
                index: varying_offset,
            });
            varying_offset += v.components as usize;
        }

        let prog = match self.get_program_mut(program_id) {
            Some(p) => p,
            None => return,
        };
        // JIT-compile both shaders for fast execution
        let vs_jit = compiler::backend_jit::compile_jit(&vs_ir);
        let fs_jit = compiler::backend_jit::compile_jit(&fs_ir);
        crate::serial_println!(
            "[libgl] JIT: VS={} ({} bytes), FS={} ({} bytes)",
            vs_jit.is_some(),
            vs_jit.as_ref().map_or(0, |j| j.code_len()),
            fs_jit.is_some(),
            fs_jit.as_ref().map_or(0, |j| j.code_len()),
        );

        prog.linked = true;
        prog.info_log.clear();
        prog.uniforms = uniforms;
        prog.attributes = attributes;
        prog.varyings = varyings;
        prog.varying_count = varying_offset;
        prog.vs_jit = vs_jit;
        prog.fs_jit = fs_jit;
        prog.vs_ir = Some(vs_ir);
        prog.fs_ir = Some(fs_ir);
    }
}
