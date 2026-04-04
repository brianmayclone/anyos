//! ArrayBuffer, DataView, and TypedArray implementations.
//!
//! Provides: ArrayBuffer, DataView, Int8Array, Uint8Array, Uint8ClampedArray,
//! Int16Array, Uint16Array, Int32Array, Uint32Array, Float32Array, Float64Array.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use alloc::rc::Rc;
use alloc::format;
use core::cell::RefCell;

use crate::value::*;
use super::Vm;

// ── Internal tags ──

pub const ARRAYBUFFER_TAG: &str = "__arraybuffer__";
pub const DATAVIEW_TAG: &str = "__dataview__";
pub const TYPED_ARRAY_TAG: &str = "__typedarray__";

const BUF_KEY: &str = "__buf__";
const BYTE_OFF_KEY: &str = "__byteOffset__";
const BYTE_LEN_KEY: &str = "__byteLength__";
const ELEM_SIZE_KEY: &str = "__elemSize__";
const KIND_KEY: &str = "__kind__";

/// TypedArray element kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TypedArrayKind {
    Int8, Uint8, Uint8Clamped,
    Int16, Uint16,
    Int32, Uint32,
    Float32, Float64,
}

impl TypedArrayKind {
    pub fn bytes_per_element(self) -> usize {
        match self {
            Self::Int8 | Self::Uint8 | Self::Uint8Clamped => 1,
            Self::Int16 | Self::Uint16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::Float64 => 8,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Int8 => "Int8Array", Self::Uint8 => "Uint8Array",
            Self::Uint8Clamped => "Uint8ClampedArray",
            Self::Int16 => "Int16Array", Self::Uint16 => "Uint16Array",
            Self::Int32 => "Int32Array", Self::Uint32 => "Uint32Array",
            Self::Float32 => "Float32Array", Self::Float64 => "Float64Array",
        }
    }
    fn to_u8(self) -> u8 {
        match self {
            Self::Int8 => 0, Self::Uint8 => 1, Self::Uint8Clamped => 2,
            Self::Int16 => 3, Self::Uint16 => 4,
            Self::Int32 => 5, Self::Uint32 => 6,
            Self::Float32 => 7, Self::Float64 => 8,
        }
    }
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Int8, 1 => Self::Uint8, 2 => Self::Uint8Clamped,
            3 => Self::Int16, 4 => Self::Uint16,
            5 => Self::Int32, 6 => Self::Uint32,
            7 => Self::Float32, _ => Self::Float64,
        }
    }
}

// ── Shared buffer storage ──
// We store the raw bytes in a `Vec<u8>` wrapped in `Rc<RefCell<>>` so that
// multiple TypedArray views and DataView can share the same backing buffer.

/// Create an ArrayBuffer object.
pub fn create_arraybuffer(vm: &Vm, byte_length: usize) -> JsValue {
    let buf: Vec<u8> = vec![0u8; byte_length];
    let buf_rc = Rc::new(RefCell::new(buf));

    let mut obj = JsObject::with_tag(ARRAYBUFFER_TAG);
    obj.prototype = Some(vm.object_proto.clone());
    // Store buf_rc as a pointer in a hidden number field (cast Rc to raw pointer → usize → f64)
    // This is safe in our no_std single-threaded environment.
    let ptr = Rc::into_raw(buf_rc) as usize;
    obj.set_hidden(String::from(BUF_KEY), JsValue::Number(ptr as f64));
    obj.set(String::from("byteLength"), JsValue::Number(byte_length as f64));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// Get the raw buffer pointer from an ArrayBuffer-tagged object.
fn get_buf_rc(obj: &JsObject) -> Option<Rc<RefCell<Vec<u8>>>> {
    if let JsValue::Number(ptr_val) = obj.get(BUF_KEY) {
        let ptr = ptr_val as usize;
        if ptr == 0 { return None; }
        // Reconstruct Rc without dropping
        let rc = unsafe { Rc::from_raw(ptr as *const RefCell<Vec<u8>>) };
        let cloned = rc.clone();
        // Don't drop the original Rc — leak it back
        core::mem::forget(rc);
        Some(cloned)
    } else {
        None
    }
}

/// Get buffer from an ArrayBuffer JsValue.
pub fn get_buffer(val: &JsValue) -> Option<Rc<RefCell<Vec<u8>>>> {
    if let JsValue::Object(obj) = val {
        let o = obj.borrow();
        if o.internal_tag.as_deref() == Some(ARRAYBUFFER_TAG) {
            return get_buf_rc(&o);
        }
    }
    None
}

/// Get buffer from a TypedArray JsValue.
fn get_typed_array_info(val: &JsValue) -> Option<(Rc<RefCell<Vec<u8>>>, usize, usize, TypedArrayKind)> {
    if let JsValue::Object(obj) = val {
        let o = obj.borrow();
        if o.internal_tag.as_deref() != Some(TYPED_ARRAY_TAG) { return None; }
        let byte_offset = match o.get(BYTE_OFF_KEY) { JsValue::Number(n) => n as usize, _ => 0 };
        let byte_length = match o.get(BYTE_LEN_KEY) { JsValue::Number(n) => n as usize, _ => 0 };
        let kind = match o.get(KIND_KEY) { JsValue::Number(n) => TypedArrayKind::from_u8(n as u8), _ => TypedArrayKind::Uint8 };
        let buf = get_buf_rc(&o)?;
        Some((buf, byte_offset, byte_length, kind))
    } else {
        None
    }
}

// ── Read/Write helpers for typed arrays ──

fn read_element(buf: &[u8], offset: usize, kind: TypedArrayKind) -> f64 {
    let b = &buf[offset..];
    match kind {
        TypedArrayKind::Int8 => b[0] as i8 as f64,
        TypedArrayKind::Uint8 | TypedArrayKind::Uint8Clamped => b[0] as f64,
        TypedArrayKind::Int16 => i16::from_le_bytes([b[0], b[1]]) as f64,
        TypedArrayKind::Uint16 => u16::from_le_bytes([b[0], b[1]]) as f64,
        TypedArrayKind::Int32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
        TypedArrayKind::Uint32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
        TypedArrayKind::Float32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
        TypedArrayKind::Float64 => f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
    }
}

fn write_element(buf: &mut [u8], offset: usize, kind: TypedArrayKind, val: f64) {
    let b = &mut buf[offset..];
    match kind {
        TypedArrayKind::Int8 => { b[0] = val as i8 as u8; }
        TypedArrayKind::Uint8 => { b[0] = val as u8; }
        TypedArrayKind::Uint8Clamped => {
            b[0] = if val < 0.0 { 0 } else if val > 255.0 { 255 } else { val as u8 };
        }
        TypedArrayKind::Int16 => { let bytes = (val as i16).to_le_bytes(); b[..2].copy_from_slice(&bytes); }
        TypedArrayKind::Uint16 => { let bytes = (val as u16).to_le_bytes(); b[..2].copy_from_slice(&bytes); }
        TypedArrayKind::Int32 => { let bytes = (val as i32).to_le_bytes(); b[..4].copy_from_slice(&bytes); }
        TypedArrayKind::Uint32 => { let bytes = (val as u32).to_le_bytes(); b[..4].copy_from_slice(&bytes); }
        TypedArrayKind::Float32 => { let bytes = (val as f32).to_le_bytes(); b[..4].copy_from_slice(&bytes); }
        TypedArrayKind::Float64 => { let bytes = val.to_le_bytes(); b[..8].copy_from_slice(&bytes); }
    }
}

// ── Create TypedArray ──

pub fn create_typed_array(vm: &Vm, kind: TypedArrayKind, args: &[JsValue]) -> JsValue {
    let elem_size = kind.bytes_per_element();

    // Determine the buffer and dimensions
    let (buf_val, byte_offset, length) = match args.first() {
        Some(JsValue::Number(n)) => {
            // new Uint8Array(length)
            let len = *n as usize;
            let buf = create_arraybuffer(vm, len * elem_size);
            (buf, 0usize, len)
        }
        Some(JsValue::Object(obj)) if obj.borrow().internal_tag.as_deref() == Some(ARRAYBUFFER_TAG) => {
            // new Uint8Array(buffer, byteOffset?, length?)
            let buf_val = JsValue::Object(obj.clone());
            let byte_off = args.get(1).map(|v| v.to_number() as usize).unwrap_or(0);
            let buf_len = match obj.borrow().get("byteLength") { JsValue::Number(n) => n as usize, _ => 0 };
            let len = args.get(2).map(|v| v.to_number() as usize)
                .unwrap_or((buf_len - byte_off) / elem_size);
            (buf_val, byte_off, len)
        }
        Some(JsValue::Array(arr)) => {
            // new Uint8Array([1, 2, 3])
            let dense = arr.borrow().to_dense_vec();
            let len = dense.len();
            let buf = create_arraybuffer(vm, len * elem_size);
            if let Some(buf_rc) = get_buffer(&buf) {
                let mut b = buf_rc.borrow_mut();
                for (i, el) in dense.iter().enumerate() {
                    write_element(&mut b, i * elem_size, kind, el.to_number());
                }
            }
            (buf, 0, len)
        }
        _ => {
            // new Uint8Array() — empty
            let buf = create_arraybuffer(vm, 0);
            (buf, 0, 0)
        }
    };

    // Get buf pointer from the ArrayBuffer
    let buf_ptr = if let JsValue::Object(obj) = &buf_val {
        match obj.borrow().get(BUF_KEY) { JsValue::Number(n) => n, _ => 0.0 }
    } else { 0.0 };

    let byte_length = length * elem_size;
    let mut obj = JsObject::with_tag(TYPED_ARRAY_TAG);
    obj.prototype = Some(vm.typed_array_proto.clone());
    obj.set_hidden(String::from(BUF_KEY), JsValue::Number(buf_ptr));
    obj.set_hidden(String::from(BYTE_OFF_KEY), JsValue::Number(byte_offset as f64));
    obj.set_hidden(String::from(BYTE_LEN_KEY), JsValue::Number(byte_length as f64));
    obj.set_hidden(String::from(KIND_KEY), JsValue::Number(kind.to_u8() as f64));
    obj.set(String::from("length"), JsValue::Number(length as f64));
    obj.set(String::from("byteLength"), JsValue::Number(byte_length as f64));
    obj.set(String::from("byteOffset"), JsValue::Number(byte_offset as f64));
    obj.set(String::from("buffer"), buf_val);
    obj.set_hidden(String::from("BYTES_PER_ELEMENT"), JsValue::Number(elem_size as f64));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

// ── Constructors ──

fn make_ctor(kind: TypedArrayKind) -> fn(&mut Vm, &[JsValue]) -> JsValue {
    match kind {
        TypedArrayKind::Int8 => ctor_int8array,
        TypedArrayKind::Uint8 => ctor_uint8array,
        TypedArrayKind::Uint8Clamped => ctor_uint8clampedarray,
        TypedArrayKind::Int16 => ctor_int16array,
        TypedArrayKind::Uint16 => ctor_uint16array,
        TypedArrayKind::Int32 => ctor_int32array,
        TypedArrayKind::Uint32 => ctor_uint32array,
        TypedArrayKind::Float32 => ctor_float32array,
        TypedArrayKind::Float64 => ctor_float64array,
    }
}

pub fn ctor_arraybuffer(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let len = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    create_arraybuffer(vm, len)
}

pub fn ctor_int8array(vm: &mut Vm, args: &[JsValue]) -> JsValue { create_typed_array(vm, TypedArrayKind::Int8, args) }
pub fn ctor_uint8array(vm: &mut Vm, args: &[JsValue]) -> JsValue { create_typed_array(vm, TypedArrayKind::Uint8, args) }
pub fn ctor_uint8clampedarray(vm: &mut Vm, args: &[JsValue]) -> JsValue { create_typed_array(vm, TypedArrayKind::Uint8Clamped, args) }
pub fn ctor_int16array(vm: &mut Vm, args: &[JsValue]) -> JsValue { create_typed_array(vm, TypedArrayKind::Int16, args) }
pub fn ctor_uint16array(vm: &mut Vm, args: &[JsValue]) -> JsValue { create_typed_array(vm, TypedArrayKind::Uint16, args) }
pub fn ctor_int32array(vm: &mut Vm, args: &[JsValue]) -> JsValue { create_typed_array(vm, TypedArrayKind::Int32, args) }
pub fn ctor_uint32array(vm: &mut Vm, args: &[JsValue]) -> JsValue { create_typed_array(vm, TypedArrayKind::Uint32, args) }
pub fn ctor_float32array(vm: &mut Vm, args: &[JsValue]) -> JsValue { create_typed_array(vm, TypedArrayKind::Float32, args) }
pub fn ctor_float64array(vm: &mut Vm, args: &[JsValue]) -> JsValue { create_typed_array(vm, TypedArrayKind::Float64, args) }

pub fn ctor_dataview(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let buf_val = args.first().cloned().unwrap_or(JsValue::Undefined);
    let byte_offset = args.get(1).map(|v| v.to_number() as usize).unwrap_or(0);
    let byte_length = args.get(2).map(|v| v.to_number() as usize).unwrap_or_else(|| {
        if let JsValue::Object(obj) = &buf_val {
            match obj.borrow().get("byteLength") { JsValue::Number(n) => n as usize, _ => 0 }
        } else { 0 }
    });

    let buf_ptr = if let JsValue::Object(obj) = &buf_val {
        match obj.borrow().get(BUF_KEY) { JsValue::Number(n) => n, _ => 0.0 }
    } else { 0.0 };

    let mut obj = JsObject::with_tag(DATAVIEW_TAG);
    obj.prototype = Some(vm.object_proto.clone());
    obj.set_hidden(String::from(BUF_KEY), JsValue::Number(buf_ptr));
    obj.set(String::from("byteOffset"), JsValue::Number(byte_offset as f64));
    obj.set(String::from("byteLength"), JsValue::Number(byte_length as f64));
    obj.set(String::from("buffer"), buf_val);

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

// ── TypedArray.prototype methods ──

/// Index access: `arr[i]` — handled by the VM's property access via numeric keys.
/// We override get_property in the VM for typed arrays instead.

/// `TypedArray.prototype.set(array, offset?)`
pub fn typed_array_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, byte_off, _byte_len, kind) = match get_typed_array_info(&this) {
        Some(info) => info,
        None => return JsValue::Undefined,
    };
    let offset = args.get(1).map(|v| v.to_number() as usize).unwrap_or(0);
    let elem_size = kind.bytes_per_element();

    if let Some(JsValue::Array(src)) = args.first() {
        let dense = src.borrow().to_dense_vec();
        let mut b = buf.borrow_mut();
        for (i, el) in dense.iter().enumerate() {
            let idx = offset + i;
            let pos = byte_off + idx * elem_size;
            if pos + elem_size <= b.len() {
                write_element(&mut b, pos, kind, el.to_number());
            }
        }
    }
    JsValue::Undefined
}

/// `TypedArray.prototype.subarray(begin, end?)`
pub fn typed_array_subarray(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, byte_off, byte_len, kind) = match get_typed_array_info(&this) {
        Some(info) => info,
        None => return JsValue::Undefined,
    };
    let elem_size = kind.bytes_per_element();
    let length = byte_len / elem_size;
    let begin = args.first().map(|v| {
        let n = v.to_number() as i64;
        if n < 0 { (length as i64 + n).max(0) as usize } else { (n as usize).min(length) }
    }).unwrap_or(0);
    let end = args.get(1).map(|v| {
        let n = v.to_number() as i64;
        if n < 0 { (length as i64 + n).max(0) as usize } else { (n as usize).min(length) }
    }).unwrap_or(length);

    let new_offset = byte_off + begin * elem_size;
    let new_len = if end > begin { end - begin } else { 0 };
    let new_byte_len = new_len * elem_size;

    // Get buf pointer
    let buf_ptr_val = if let JsValue::Object(obj) = &this {
        obj.borrow().get(BUF_KEY)
    } else { JsValue::Number(0.0) };

    let mut obj = JsObject::with_tag(TYPED_ARRAY_TAG);
    obj.prototype = Some(vm.typed_array_proto.clone());
    obj.set_hidden(String::from(BUF_KEY), buf_ptr_val);
    obj.set_hidden(String::from(BYTE_OFF_KEY), JsValue::Number(new_offset as f64));
    obj.set_hidden(String::from(BYTE_LEN_KEY), JsValue::Number(new_byte_len as f64));
    obj.set_hidden(String::from(KIND_KEY), JsValue::Number(kind.to_u8() as f64));
    obj.set(String::from("length"), JsValue::Number(new_len as f64));
    obj.set(String::from("byteLength"), JsValue::Number(new_byte_len as f64));
    obj.set(String::from("byteOffset"), JsValue::Number(new_offset as f64));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// `TypedArray.prototype.slice(begin, end?)`
pub fn typed_array_slice(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, byte_off, byte_len, kind) = match get_typed_array_info(&this) {
        Some(info) => info,
        None => return JsValue::Undefined,
    };
    let elem_size = kind.bytes_per_element();
    let length = byte_len / elem_size;
    let begin = args.first().map(|v| {
        let n = v.to_number() as i64;
        if n < 0 { (length as i64 + n).max(0) as usize } else { (n as usize).min(length) }
    }).unwrap_or(0);
    let end = args.get(1).map(|v| {
        let n = v.to_number() as i64;
        if n < 0 { (length as i64 + n).max(0) as usize } else { (n as usize).min(length) }
    }).unwrap_or(length);

    let new_len = if end > begin { end - begin } else { 0 };
    let new_buf = create_arraybuffer(vm, new_len * elem_size);
    if let Some(new_buf_rc) = get_buffer(&new_buf) {
        let src = buf.borrow();
        let mut dst = new_buf_rc.borrow_mut();
        for i in 0..new_len {
            let src_off = byte_off + (begin + i) * elem_size;
            let dst_off = i * elem_size;
            if src_off + elem_size <= src.len() && dst_off + elem_size <= dst.len() {
                dst[dst_off..dst_off + elem_size].copy_from_slice(&src[src_off..src_off + elem_size]);
            }
        }
    }

    // Build result array from new buffer
    let mut result_args = Vec::new();
    result_args.push(JsValue::Number(new_len as f64));
    create_typed_array(vm, kind, &[JsValue::Number(new_len as f64)])
}

/// `TypedArray.prototype.fill(value, start?, end?)`
pub fn typed_array_fill(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, byte_off, byte_len, kind) = match get_typed_array_info(&this) {
        Some(info) => info,
        None => return this,
    };
    let elem_size = kind.bytes_per_element();
    let length = byte_len / elem_size;
    let value = args.first().map(|v| v.to_number()).unwrap_or(0.0);
    let start = args.get(1).map(|v| (v.to_number() as usize).min(length)).unwrap_or(0);
    let end = args.get(2).map(|v| (v.to_number() as usize).min(length)).unwrap_or(length);

    let mut b = buf.borrow_mut();
    for i in start..end {
        write_element(&mut b, byte_off + i * elem_size, kind, value);
    }
    this
}

/// `TypedArray.prototype.indexOf(value, fromIndex?)`
pub fn typed_array_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, byte_off, byte_len, kind) = match get_typed_array_info(&this) {
        Some(info) => info,
        None => return JsValue::Number(-1.0),
    };
    let elem_size = kind.bytes_per_element();
    let length = byte_len / elem_size;
    let search = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    let from = args.get(1).map(|v| v.to_number() as usize).unwrap_or(0);

    let b = buf.borrow();
    for i in from..length {
        let v = read_element(&b, byte_off + i * elem_size, kind);
        if v == search { return JsValue::Number(i as f64); }
    }
    JsValue::Number(-1.0)
}

/// `TypedArray.prototype.forEach(callback)`
pub fn typed_array_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let (buf, byte_off, byte_len, kind) = match get_typed_array_info(&this) {
        Some(info) => info,
        None => return JsValue::Undefined,
    };
    let elem_size = kind.bytes_per_element();
    let length = byte_len / elem_size;

    for i in 0..length {
        let v = {
            let b = buf.borrow();
            read_element(&b, byte_off + i * elem_size, kind)
        };
        vm.invoke_function(&callback, &[JsValue::Number(v), JsValue::Number(i as f64), this.clone()], this.clone());
    }
    JsValue::Undefined
}

// ── DataView methods ──

fn get_dataview_info(val: &JsValue) -> Option<(Rc<RefCell<Vec<u8>>>, usize)> {
    if let JsValue::Object(obj) = val {
        let o = obj.borrow();
        if o.internal_tag.as_deref() != Some(DATAVIEW_TAG) { return None; }
        let byte_offset = match o.get("byteOffset") { JsValue::Number(n) => n as usize, _ => 0 };
        let buf = get_buf_rc(&o)?;
        Some((buf, byte_offset))
    } else {
        None
    }
}

pub fn dataview_get_int8(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, base) = match get_dataview_info(&this) { Some(i) => i, None => return JsValue::Undefined };
    let off = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let b = buf.borrow();
    let pos = base + off;
    if pos < b.len() { JsValue::Number(b[pos] as i8 as f64) } else { JsValue::Undefined }
}

pub fn dataview_get_uint8(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, base) = match get_dataview_info(&this) { Some(i) => i, None => return JsValue::Undefined };
    let off = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let b = buf.borrow();
    let pos = base + off;
    if pos < b.len() { JsValue::Number(b[pos] as f64) } else { JsValue::Undefined }
}

pub fn dataview_get_int16(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, base) = match get_dataview_info(&this) { Some(i) => i, None => return JsValue::Undefined };
    let off = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let le = args.get(1).map(|v| v.to_boolean()).unwrap_or(false);
    let b = buf.borrow();
    let pos = base + off;
    if pos + 1 < b.len() {
        let val = if le { i16::from_le_bytes([b[pos], b[pos+1]]) } else { i16::from_be_bytes([b[pos], b[pos+1]]) };
        JsValue::Number(val as f64)
    } else { JsValue::Undefined }
}

pub fn dataview_get_uint16(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, base) = match get_dataview_info(&this) { Some(i) => i, None => return JsValue::Undefined };
    let off = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let le = args.get(1).map(|v| v.to_boolean()).unwrap_or(false);
    let b = buf.borrow();
    let pos = base + off;
    if pos + 1 < b.len() {
        let val = if le { u16::from_le_bytes([b[pos], b[pos+1]]) } else { u16::from_be_bytes([b[pos], b[pos+1]]) };
        JsValue::Number(val as f64)
    } else { JsValue::Undefined }
}

pub fn dataview_get_int32(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, base) = match get_dataview_info(&this) { Some(i) => i, None => return JsValue::Undefined };
    let off = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let le = args.get(1).map(|v| v.to_boolean()).unwrap_or(false);
    let b = buf.borrow();
    let pos = base + off;
    if pos + 3 < b.len() {
        let val = if le { i32::from_le_bytes([b[pos], b[pos+1], b[pos+2], b[pos+3]]) }
                  else { i32::from_be_bytes([b[pos], b[pos+1], b[pos+2], b[pos+3]]) };
        JsValue::Number(val as f64)
    } else { JsValue::Undefined }
}

pub fn dataview_get_float32(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, base) = match get_dataview_info(&this) { Some(i) => i, None => return JsValue::Undefined };
    let off = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let le = args.get(1).map(|v| v.to_boolean()).unwrap_or(false);
    let b = buf.borrow();
    let pos = base + off;
    if pos + 3 < b.len() {
        let val = if le { f32::from_le_bytes([b[pos], b[pos+1], b[pos+2], b[pos+3]]) }
                  else { f32::from_be_bytes([b[pos], b[pos+1], b[pos+2], b[pos+3]]) };
        JsValue::Number(val as f64)
    } else { JsValue::Undefined }
}

pub fn dataview_get_float64(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let (buf, base) = match get_dataview_info(&this) { Some(i) => i, None => return JsValue::Undefined };
    let off = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let le = args.get(1).map(|v| v.to_boolean()).unwrap_or(false);
    let b = buf.borrow();
    let pos = base + off;
    if pos + 7 < b.len() {
        let val = if le { f64::from_le_bytes([b[pos],b[pos+1],b[pos+2],b[pos+3],b[pos+4],b[pos+5],b[pos+6],b[pos+7]]) }
                  else { f64::from_be_bytes([b[pos],b[pos+1],b[pos+2],b[pos+3],b[pos+4],b[pos+5],b[pos+6],b[pos+7]]) };
        JsValue::Number(val)
    } else { JsValue::Undefined }
}

// ── ArrayBuffer.prototype.slice ──

pub fn arraybuffer_slice(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let buf_rc = match get_buffer(&this) { Some(b) => b, None => return JsValue::Undefined };
    let buf = buf_rc.borrow();
    let len = buf.len();
    let begin = args.first().map(|v| {
        let n = v.to_number() as i64;
        if n < 0 { (len as i64 + n).max(0) as usize } else { (n as usize).min(len) }
    }).unwrap_or(0);
    let end = args.get(1).map(|v| {
        let n = v.to_number() as i64;
        if n < 0 { (len as i64 + n).max(0) as usize } else { (n as usize).min(len) }
    }).unwrap_or(len);

    let new_len = if end > begin { end - begin } else { 0 };
    let new_buf = create_arraybuffer(vm, new_len);
    if let Some(new_rc) = get_buffer(&new_buf) {
        let mut dst = new_rc.borrow_mut();
        if new_len > 0 {
            dst[..new_len].copy_from_slice(&buf[begin..begin + new_len]);
        }
    }
    new_buf
}

/// Handle numeric index access on TypedArrays.
/// Called from the VM's `get_property_with_proto` for typed array objects.
pub fn typed_array_get_index(obj: &JsObject, index: usize) -> Option<JsValue> {
    if obj.internal_tag.as_deref() != Some(TYPED_ARRAY_TAG) { return None; }
    let byte_off = match obj.get(BYTE_OFF_KEY) { JsValue::Number(n) => n as usize, _ => return None };
    let kind = match obj.get(KIND_KEY) { JsValue::Number(n) => TypedArrayKind::from_u8(n as u8), _ => return None };
    let elem_size = kind.bytes_per_element();
    let buf_rc = get_buf_rc(obj)?;
    let buf = buf_rc.borrow();
    let pos = byte_off + index * elem_size;
    if pos + elem_size <= buf.len() {
        Some(JsValue::Number(read_element(&buf, pos, kind)))
    } else {
        None
    }
}

/// Handle numeric index write on TypedArrays.
pub fn typed_array_set_index(obj: &JsObject, index: usize, val: f64) -> bool {
    if obj.internal_tag.as_deref() != Some(TYPED_ARRAY_TAG) { return false; }
    let byte_off = match obj.get(BYTE_OFF_KEY) { JsValue::Number(n) => n as usize, _ => return false };
    let kind = match obj.get(KIND_KEY) { JsValue::Number(n) => TypedArrayKind::from_u8(n as u8), _ => return false };
    let elem_size = kind.bytes_per_element();
    let buf_rc = match get_buf_rc(obj) { Some(b) => b, None => return false };
    let mut buf = buf_rc.borrow_mut();
    let pos = byte_off + index * elem_size;
    if pos + elem_size <= buf.len() {
        write_element(&mut buf, pos, kind, val);
        true
    } else {
        false
    }
}
