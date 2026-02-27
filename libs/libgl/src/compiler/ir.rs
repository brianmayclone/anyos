//! Intermediate Representation (IR) for compiled GLSL shaders.
//!
//! SSA-inspired register-based IR. Each register holds a `[f32; 4]` vector.
//! The software backend interprets these instructions directly; a future HW
//! backend could translate them to GPU bytecode.

use alloc::string::String;
use alloc::vec::Vec;

/// A compiled shader program in IR form.
#[derive(Debug, Clone)]
pub struct Program {
    /// IR instructions.
    pub instructions: Vec<Inst>,
    /// Number of registers needed.
    pub num_regs: u32,
    /// Declared attributes (vertex shader inputs).
    pub attributes: Vec<VarInfo>,
    /// Declared varyings (VS outputs / FS inputs).
    pub varyings: Vec<VarInfo>,
    /// Declared uniforms.
    pub uniforms: Vec<VarInfo>,
    /// Declared local variables.
    pub locals: Vec<VarInfo>,
}

/// Variable metadata (name + type info).
#[derive(Debug, Clone)]
pub struct VarInfo {
    pub name: String,
    /// Number of float components (1, 2, 3, 4, 9, 16).
    pub components: u32,
    /// Register index where this variable is stored.
    pub reg: u32,
}

/// Register index (into the register file).
pub type Reg = u32;

/// IR instruction set.
#[derive(Debug, Clone)]
pub enum Inst {
    /// Load immediate float value into register component.
    LoadConst(Reg, [f32; 4]),

    /// Copy: dst = src
    Mov(Reg, Reg),

    /// Component-wise add: dst = a + b
    Add(Reg, Reg, Reg),
    /// Component-wise subtract: dst = a - b
    Sub(Reg, Reg, Reg),
    /// Component-wise multiply: dst = a * b
    Mul(Reg, Reg, Reg),
    /// Component-wise divide: dst = a / b
    Div(Reg, Reg, Reg),
    /// Negate: dst = -src
    Neg(Reg, Reg),

    /// Dot product of 3 components: dst.x = a.xyz · b.xyz
    Dp3(Reg, Reg, Reg),
    /// Dot product of 4 components: dst.x = a.xyzw · b.xyzw
    Dp4(Reg, Reg, Reg),

    /// Cross product: dst.xyz = a.xyz × b.xyz
    Cross(Reg, Reg, Reg),

    /// Normalize: dst = normalize(src)
    Normalize(Reg, Reg),
    /// Length: dst.x = length(src)
    Length(Reg, Reg),

    /// Component-wise min: dst = min(a, b)
    Min(Reg, Reg, Reg),
    /// Component-wise max: dst = max(a, b)
    Max(Reg, Reg, Reg),
    /// Clamp: dst = clamp(x, min, max)
    Clamp(Reg, Reg, Reg, Reg),
    /// Mix/lerp: dst = mix(a, b, t)
    Mix(Reg, Reg, Reg, Reg),

    /// Abs: dst = abs(src)
    Abs(Reg, Reg),
    /// Floor: dst = floor(src)
    Floor(Reg, Reg),
    /// Fract: dst = fract(src)
    Fract(Reg, Reg),

    /// Power: dst = pow(base, exp)
    Pow(Reg, Reg, Reg),
    /// Square root: dst = sqrt(src)
    Sqrt(Reg, Reg),
    /// Reciprocal square root: dst = inversesqrt(src)
    Rsqrt(Reg, Reg),

    /// Sine: dst = sin(src)
    Sin(Reg, Reg),
    /// Cosine: dst = cos(src)
    Cos(Reg, Reg),

    /// Reflect: dst = reflect(I, N)
    Reflect(Reg, Reg, Reg),

    /// Texture sample: dst = texture2D(sampler_reg, coord_reg)
    TexSample(Reg, Reg, Reg),

    /// Matrix-vector multiply (4x4 * vec4): dst = mat * vec
    /// mat is stored in 4 consecutive registers (columns).
    MatMul4(Reg, Reg, Reg),

    /// Matrix-vector multiply (3x3 * vec3): dst = mat * vec
    MatMul3(Reg, Reg, Reg),

    /// Swizzle: dst = swizzle(src, [component indices])
    /// Each index is 0-3 mapping to x/y/z/w.
    Swizzle(Reg, Reg, [u8; 4], u8),

    /// Write specific components from src into dst, preserving others.
    /// mask: bit 0=x, 1=y, 2=z, 3=w.
    WriteMask(Reg, Reg, u8),

    /// Compare less than: dst = (a < b) ? 1.0 : 0.0 (component-wise).
    CmpLt(Reg, Reg, Reg),
    /// Compare equal: dst = (a == b) ? 1.0 : 0.0.
    CmpEq(Reg, Reg, Reg),

    /// Select: dst = (cond.x != 0) ? a : b
    Select(Reg, Reg, Reg, Reg),

    /// Convert int to float: dst = float(src)
    IntToFloat(Reg, Reg),
    /// Convert float to int: dst.x = int(src.x)
    FloatToInt(Reg, Reg),

    /// Store gl_Position (vertex shader output).
    StorePosition(Reg),
    /// Store gl_FragColor (fragment shader output).
    StoreFragColor(Reg),
    /// Store gl_PointSize (vertex shader output).
    StorePointSize(Reg),

    /// Load a varying by index.
    LoadVarying(Reg, u32),
    /// Store a varying by index.
    StoreVarying(u32, Reg),

    /// Load a uniform by index.
    LoadUniform(Reg, u32),

    /// Load an attribute by index.
    LoadAttribute(Reg, u32),
}
