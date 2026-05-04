use alloc::string::String;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::buffer::buffer_to_bytes;
use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    install_constants(&mut module);
    module.set(String::from("init"), native_fn("init", init));
    module.set(String::from("resize"), native_fn("resize", resize));
    module.set(
        String::from("clearColor"),
        native_fn("clearColor", clear_color),
    );
    module.set(String::from("clear"), native_fn("clear", clear));
    module.set(String::from("viewport"), native_fn("viewport", viewport));
    module.set(String::from("enable"), native_fn("enable", enable));
    module.set(String::from("disable"), native_fn("disable", disable));
    module.set(String::from("getError"), native_fn("getError", get_error));
    module.set(String::from("flush"), native_fn("flush", flush));
    module.set(String::from("finish"), native_fn("finish", finish));
    module.set(
        String::from("genBuffers"),
        native_fn("genBuffers", gen_buffers),
    );
    module.set(
        String::from("deleteBuffers"),
        native_fn("deleteBuffers", delete_buffers),
    );
    module.set(
        String::from("bindBuffer"),
        native_fn("bindBuffer", bind_buffer),
    );
    module.set(
        String::from("bufferData"),
        native_fn("bufferData", buffer_data),
    );
    module.set(
        String::from("genTextures"),
        native_fn("genTextures", gen_textures),
    );
    module.set(
        String::from("bindTexture"),
        native_fn("bindTexture", bind_texture),
    );
    module.set(
        String::from("texParameteri"),
        native_fn("texParameteri", tex_parameteri),
    );
    module.set(
        String::from("createShader"),
        native_fn("createShader", create_shader),
    );
    module.set(
        String::from("shaderSource"),
        native_fn("shaderSource", shader_source),
    );
    module.set(
        String::from("compileShader"),
        native_fn("compileShader", compile_shader),
    );
    module.set(
        String::from("getShaderCompileStatus"),
        native_fn("getShaderCompileStatus", get_shader_compile_status),
    );
    module.set(
        String::from("getShaderInfoLog"),
        native_fn("getShaderInfoLog", get_shader_info_log),
    );
    module.set(
        String::from("createProgram"),
        native_fn("createProgram", create_program),
    );
    module.set(
        String::from("attachShader"),
        native_fn("attachShader", attach_shader),
    );
    module.set(
        String::from("linkProgram"),
        native_fn("linkProgram", link_program),
    );
    module.set(
        String::from("useProgram"),
        native_fn("useProgram", use_program),
    );
    module.set(
        String::from("getProgramLinkStatus"),
        native_fn("getProgramLinkStatus", get_program_link_status),
    );
    module.set(
        String::from("getUniformLocation"),
        native_fn("getUniformLocation", get_uniform_location),
    );
    module.set(String::from("uniform1i"), native_fn("uniform1i", uniform1i));
    module.set(String::from("uniform1f"), native_fn("uniform1f", uniform1f));
    module.set(String::from("uniform2f"), native_fn("uniform2f", uniform2f));
    module.set(String::from("uniform3f"), native_fn("uniform3f", uniform3f));
    module.set(String::from("uniform4f"), native_fn("uniform4f", uniform4f));
    module.set(
        String::from("uniformMatrix4fv"),
        native_fn("uniformMatrix4fv", uniform_matrix4fv),
    );
    module.set(
        String::from("drawArrays"),
        native_fn("drawArrays", draw_arrays),
    );
    module.set(String::from("setFXAA"), native_fn("setFXAA", set_fxaa));
    module.set(String::from("math"), math_module());
    object(module)
}

fn install_constants(module: &mut JsObject) {
    for (name, value) in [
        ("NO_ERROR", libgl_client::GL_NO_ERROR),
        ("FALSE", libgl_client::GL_FALSE as u32),
        ("TRUE", libgl_client::GL_TRUE as u32),
        ("DEPTH_TEST", libgl_client::GL_DEPTH_TEST),
        ("BLEND", libgl_client::GL_BLEND),
        ("CULL_FACE", libgl_client::GL_CULL_FACE),
        ("COLOR_BUFFER_BIT", libgl_client::GL_COLOR_BUFFER_BIT),
        ("DEPTH_BUFFER_BIT", libgl_client::GL_DEPTH_BUFFER_BIT),
        ("TRIANGLES", libgl_client::GL_TRIANGLES),
        ("TRIANGLE_STRIP", libgl_client::GL_TRIANGLE_STRIP),
        ("TRIANGLE_FAN", libgl_client::GL_TRIANGLE_FAN),
        ("ARRAY_BUFFER", libgl_client::GL_ARRAY_BUFFER),
        (
            "ELEMENT_ARRAY_BUFFER",
            libgl_client::GL_ELEMENT_ARRAY_BUFFER,
        ),
        ("STATIC_DRAW", libgl_client::GL_STATIC_DRAW),
        ("FLOAT", libgl_client::GL_FLOAT),
        ("UNSIGNED_SHORT", libgl_client::GL_UNSIGNED_SHORT),
        ("UNSIGNED_INT", libgl_client::GL_UNSIGNED_INT),
        ("UNSIGNED_BYTE", libgl_client::GL_UNSIGNED_BYTE),
        ("TEXTURE_2D", libgl_client::GL_TEXTURE_2D),
        ("TEXTURE0", libgl_client::GL_TEXTURE0),
        ("RGBA", libgl_client::GL_RGBA),
        ("RGB", libgl_client::GL_RGB),
        ("NEAREST", libgl_client::GL_NEAREST),
        ("LINEAR", libgl_client::GL_LINEAR),
        ("VERTEX_SHADER", libgl_client::GL_VERTEX_SHADER),
        ("FRAGMENT_SHADER", libgl_client::GL_FRAGMENT_SHADER),
        ("COMPILE_STATUS", libgl_client::GL_COMPILE_STATUS),
        ("LINK_STATUS", libgl_client::GL_LINK_STATUS),
        ("FRAMEBUFFER", libgl_client::GL_FRAMEBUFFER),
        (
            "FRAMEBUFFER_COMPLETE",
            libgl_client::GL_FRAMEBUFFER_COMPLETE,
        ),
    ] {
        module.set(String::from(name), JsValue::Number(value as f64));
    }
}

fn math_module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("PI"), JsValue::Number(libgl_client::PI as f64));
    module.set(String::from("sin"), native_fn("sin", math_sin));
    module.set(String::from("cos"), native_fn("cos", math_cos));
    module.set(String::from("tan"), native_fn("tan", math_tan));
    module.set(String::from("sqrt"), native_fn("sqrt", math_sqrt));
    module.set(String::from("abs"), native_fn("abs", math_abs));
    module.set(String::from("pow"), native_fn("pow", math_pow));
    module.set(String::from("clamp"), native_fn("clamp", math_clamp));
    module.set(String::from("lerp"), native_fn("lerp", math_lerp));
    object(module)
}

fn init(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !libgl_client::init() {
        return JsValue::Bool(false);
    }
    let width = arg_u32(args, 0, 640);
    let height = arg_u32(args, 1, 480);
    libgl_client::gl_init(width, height);
    JsValue::Bool(true)
}

macro_rules! gl_void {
    ($name:ident, $body:block) => {
        fn $name(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
            if !ensure_loaded(vm) {
                return JsValue::Undefined;
            }
            $body
            JsValue::Undefined
        }
    };
}

fn resize(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::gl_resize(arg_u32(args, 0, 640), arg_u32(args, 1, 480));
    JsValue::Undefined
}

fn clear_color(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::clear_color(
        arg_f32(args, 0, 0.0),
        arg_f32(args, 1, 0.0),
        arg_f32(args, 2, 0.0),
        arg_f32(args, 3, 1.0),
    );
    JsValue::Undefined
}

fn clear(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::clear(arg_u32(args, 0, libgl_client::GL_COLOR_BUFFER_BIT));
    JsValue::Undefined
}

fn viewport(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::viewport(
        arg_i32(args, 0, 0),
        arg_i32(args, 1, 0),
        arg_i32(args, 2, 640),
        arg_i32(args, 3, 480),
    );
    JsValue::Undefined
}

fn enable(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::enable(arg_u32(args, 0, 0));
    JsValue::Undefined
}

fn disable(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::disable(arg_u32(args, 0, 0));
    JsValue::Undefined
}

fn get_error(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    JsValue::Number(libgl_client::get_error() as f64)
}

gl_void!(flush, {
    libgl_client::flush();
});
gl_void!(finish, {
    libgl_client::finish();
});

fn gen_buffers(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    let count = arg_i32(args, 0, 1).max(0) as usize;
    let mut ids = alloc::vec![0u32; count];
    libgl_client::gen_buffers(count as i32, &mut ids);
    JsValue::new_array(
        ids.into_iter()
            .map(|id| JsValue::Number(id as f64))
            .collect(),
    )
}

fn delete_buffers(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::delete_buffers(&u32_array(args.first()));
    JsValue::Undefined
}

fn bind_buffer(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::bind_buffer(
        arg_u32(args, 0, libgl_client::GL_ARRAY_BUFFER),
        arg_u32(args, 1, 0),
    );
    JsValue::Undefined
}

fn buffer_data(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    let data = bytes_from(args.get(1));
    libgl_client::buffer_data(
        arg_u32(args, 0, libgl_client::GL_ARRAY_BUFFER),
        &data,
        arg_u32(args, 2, libgl_client::GL_STATIC_DRAW),
    );
    JsValue::Undefined
}

fn gen_textures(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    let count = arg_i32(args, 0, 1).max(0) as usize;
    let mut ids = alloc::vec![0u32; count];
    libgl_client::gen_textures(count as i32, &mut ids);
    JsValue::new_array(
        ids.into_iter()
            .map(|id| JsValue::Number(id as f64))
            .collect(),
    )
}

fn bind_texture(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::bind_texture(
        arg_u32(args, 0, libgl_client::GL_TEXTURE_2D),
        arg_u32(args, 1, 0),
    );
    JsValue::Undefined
}

fn tex_parameteri(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::tex_parameteri(
        arg_u32(args, 0, libgl_client::GL_TEXTURE_2D),
        arg_u32(args, 1, 0),
        arg_i32(args, 2, 0),
    );
    JsValue::Undefined
}

fn create_shader(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    JsValue::Number(
        libgl_client::create_shader(arg_u32(args, 0, libgl_client::GL_VERTEX_SHADER)) as f64,
    )
}

fn shader_source(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::shader_source(arg_u32(args, 0, 0), &arg_string(args, 1, ""));
    JsValue::Undefined
}

fn compile_shader(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::compile_shader(arg_u32(args, 0, 0));
    JsValue::Undefined
}

fn get_shader_compile_status(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    JsValue::Bool(libgl_client::get_shader_compile_status(arg_u32(args, 0, 0)))
}

fn get_shader_info_log(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    JsValue::String(libgl_client::get_shader_info_log(arg_u32(args, 0, 0)))
}

fn create_program(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    JsValue::Number(libgl_client::create_program() as f64)
}

fn attach_shader(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::attach_shader(arg_u32(args, 0, 0), arg_u32(args, 1, 0));
    JsValue::Undefined
}

fn link_program(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::link_program(arg_u32(args, 0, 0));
    JsValue::Undefined
}

fn use_program(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::use_program(arg_u32(args, 0, 0));
    JsValue::Undefined
}

fn get_program_link_status(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    JsValue::Bool(libgl_client::get_program_link_status(arg_u32(args, 0, 0)))
}

fn get_uniform_location(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    JsValue::Number(libgl_client::get_uniform_location(
        arg_u32(args, 0, 0),
        &arg_string(args, 1, ""),
    ) as f64)
}

fn uniform1i(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if ensure_loaded(vm) {
        libgl_client::uniform1i(arg_i32(args, 0, -1), arg_i32(args, 1, 0));
    }
    JsValue::Undefined
}
fn uniform1f(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if ensure_loaded(vm) {
        libgl_client::uniform1f(arg_i32(args, 0, -1), arg_f32(args, 1, 0.0));
    }
    JsValue::Undefined
}
fn uniform2f(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if ensure_loaded(vm) {
        libgl_client::uniform2f(
            arg_i32(args, 0, -1),
            arg_f32(args, 1, 0.0),
            arg_f32(args, 2, 0.0),
        );
    }
    JsValue::Undefined
}
fn uniform3f(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if ensure_loaded(vm) {
        libgl_client::uniform3f(
            arg_i32(args, 0, -1),
            arg_f32(args, 1, 0.0),
            arg_f32(args, 2, 0.0),
            arg_f32(args, 3, 0.0),
        );
    }
    JsValue::Undefined
}
fn uniform4f(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if ensure_loaded(vm) {
        libgl_client::uniform4f(
            arg_i32(args, 0, -1),
            arg_f32(args, 1, 0.0),
            arg_f32(args, 2, 0.0),
            arg_f32(args, 3, 0.0),
            arg_f32(args, 4, 0.0),
        );
    }
    JsValue::Undefined
}

fn uniform_matrix4fv(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    let mut values = [0.0f32; 16];
    for (idx, value) in f32_array(args.get(2)).into_iter().take(16).enumerate() {
        values[idx] = value;
    }
    libgl_client::uniform_matrix4fv(
        arg_i32(args, 0, -1),
        args.get(1).map(|v| v.to_boolean()).unwrap_or(false),
        &values,
    );
    JsValue::Undefined
}

fn draw_arrays(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::draw_arrays(
        arg_u32(args, 0, libgl_client::GL_TRIANGLES),
        arg_i32(args, 1, 0),
        arg_i32(args, 2, 0),
    );
    JsValue::Undefined
}

fn set_fxaa(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    libgl_client::set_fxaa(args.first().map(|v| v.to_boolean()).unwrap_or(false));
    JsValue::Undefined
}

fn math_sin(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    math1(vm, args, libgl_client::sin)
}
fn math_cos(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    math1(vm, args, libgl_client::cos)
}
fn math_tan(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    math1(vm, args, libgl_client::tan)
}
fn math_sqrt(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    math1(vm, args, libgl_client::sqrt)
}
fn math_abs(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    math1(vm, args, libgl_client::abs)
}
fn math_pow(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if ensure_loaded(vm) {
        JsValue::Number(libgl_client::pow(arg_f32(args, 0, 0.0), arg_f32(args, 1, 0.0)) as f64)
    } else {
        JsValue::Undefined
    }
}
fn math_clamp(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if ensure_loaded(vm) {
        JsValue::Number(libgl_client::clamp(
            arg_f32(args, 0, 0.0),
            arg_f32(args, 1, 0.0),
            arg_f32(args, 2, 1.0),
        ) as f64)
    } else {
        JsValue::Undefined
    }
}
fn math_lerp(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if ensure_loaded(vm) {
        JsValue::Number(libgl_client::lerp(
            arg_f32(args, 0, 0.0),
            arg_f32(args, 1, 0.0),
            arg_f32(args, 2, 0.0),
        ) as f64)
    } else {
        JsValue::Undefined
    }
}

fn math1(vm: &mut Vm, args: &[JsValue], f: fn(f32) -> f32) -> JsValue {
    if !ensure_loaded(vm) {
        return JsValue::Undefined;
    }
    JsValue::Number(f(arg_f32(args, 0, 0.0)) as f64)
}

fn ensure_loaded(vm: &mut Vm) -> bool {
    if libgl_client::init() {
        true
    } else {
        vm.pending_exception = Some(vm.make_type_error("libgl is not available"));
        false
    }
}

fn arg_string(args: &[JsValue], index: usize, default: &str) -> String {
    args.get(index)
        .map(|value| value.to_js_string())
        .filter(|value| value != "undefined")
        .unwrap_or_else(|| String::from(default))
}

fn arg_u32(args: &[JsValue], index: usize, default: u32) -> u32 {
    args.get(index)
        .map(|value| value.to_number() as u32)
        .unwrap_or(default)
}

fn arg_i32(args: &[JsValue], index: usize, default: i32) -> i32 {
    args.get(index)
        .map(|value| value.to_number() as i32)
        .unwrap_or(default)
}

fn arg_f32(args: &[JsValue], index: usize, default: f32) -> f32 {
    args.get(index)
        .map(|value| value.to_number() as f32)
        .unwrap_or(default)
}

fn bytes_from(value: Option<&JsValue>) -> Vec<u8> {
    match value {
        Some(value) if super::buffer::is_buffer_like(value) => buffer_to_bytes(value),
        Some(JsValue::Array(array)) => array
            .borrow()
            .to_dense_vec()
            .into_iter()
            .map(|value| value.to_number().clamp(0.0, 255.0) as u8)
            .collect(),
        Some(JsValue::String(text)) => text.as_bytes().to_vec(),
        _ => Vec::new(),
    }
}

fn u32_array(value: Option<&JsValue>) -> Vec<u32> {
    match value {
        Some(JsValue::Array(array)) => array
            .borrow()
            .to_dense_vec()
            .into_iter()
            .map(|value| value.to_number() as u32)
            .collect(),
        Some(value) => alloc::vec![value.to_number() as u32],
        None => Vec::new(),
    }
}

fn f32_array(value: Option<&JsValue>) -> Vec<f32> {
    match value {
        Some(JsValue::Array(array)) => array
            .borrow()
            .to_dense_vec()
            .into_iter()
            .map(|value| value.to_number() as f32)
            .collect(),
        _ => Vec::new(),
    }
}
