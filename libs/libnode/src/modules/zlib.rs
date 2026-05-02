use alloc::string::String;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::buffer::{buffer_from_bytes, buffer_to_bytes, is_buffer_like};
use super::util::object;
use super::zlib_codec;

const STREAM_OP_KEY: &str = "__node_zlib_stream_op__";
const STREAM_CHUNKS_KEY: &str = "__node_zlib_stream_chunks__";

pub fn module() -> JsValue {
    let _ = libzip_client::init();
    let mut module = JsObject::new();
    module.set(String::from("gzip"), native_fn("gzip", gzip_callback));
    module.set(String::from("gunzip"), native_fn("gunzip", gunzip_callback));
    module.set(
        String::from("deflate"),
        native_fn("deflate", deflate_callback),
    );
    module.set(
        String::from("inflate"),
        native_fn("inflate", inflate_callback),
    );
    module.set(
        String::from("deflateRaw"),
        native_fn("deflateRaw", deflate_raw_callback),
    );
    module.set(
        String::from("inflateRaw"),
        native_fn("inflateRaw", inflate_raw_callback),
    );
    module.set(String::from("unzip"), native_fn("unzip", unzip_callback));
    module.set(
        String::from("brotliCompress"),
        native_fn("brotliCompress", callback_passthrough),
    );
    module.set(
        String::from("brotliDecompress"),
        native_fn("brotliDecompress", callback_passthrough),
    );
    module.set(String::from("gzipSync"), native_fn("gzipSync", gzip_sync));
    module.set(
        String::from("gunzipSync"),
        native_fn("gunzipSync", gunzip_sync),
    );
    module.set(
        String::from("deflateSync"),
        native_fn("deflateSync", deflate_sync),
    );
    module.set(
        String::from("inflateSync"),
        native_fn("inflateSync", inflate_sync),
    );
    module.set(
        String::from("deflateRawSync"),
        native_fn("deflateRawSync", deflate_raw_sync),
    );
    module.set(
        String::from("inflateRawSync"),
        native_fn("inflateRawSync", inflate_raw_sync),
    );
    module.set(
        String::from("unzipSync"),
        native_fn("unzipSync", unzip_sync),
    );
    module.set(
        String::from("brotliCompressSync"),
        native_fn("brotliCompressSync", sync_passthrough),
    );
    module.set(
        String::from("brotliDecompressSync"),
        native_fn("brotliDecompressSync", sync_passthrough),
    );
    module.set(
        String::from("createGzip"),
        native_fn("createGzip", create_gzip_stream),
    );
    module.set(
        String::from("createGunzip"),
        native_fn("createGunzip", create_gunzip_stream),
    );
    module.set(
        String::from("createDeflate"),
        native_fn("createDeflate", create_deflate_stream),
    );
    module.set(
        String::from("createInflate"),
        native_fn("createInflate", create_inflate_stream),
    );
    module.set(
        String::from("createDeflateRaw"),
        native_fn("createDeflateRaw", create_deflate_raw_stream),
    );
    module.set(
        String::from("createInflateRaw"),
        native_fn("createInflateRaw", create_inflate_raw_stream),
    );
    module.set(
        String::from("createUnzip"),
        native_fn("createUnzip", create_unzip_stream),
    );
    module.set(String::from("constants"), constants_object());
    object(module)
}

fn callback_passthrough(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let input = args.first().cloned().unwrap_or(JsValue::Undefined);
    let out = bytes_value(&input);
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[JsValue::Null, out.clone()], JsValue::Undefined);
        return JsValue::Undefined;
    }
    out
}

fn gzip_callback(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    callback_result(vm, args, op_gzip)
}

fn gunzip_callback(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    callback_result(vm, args, zlib_codec::gunzip)
}

fn deflate_callback(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    callback_result(vm, args, op_deflate)
}

fn inflate_callback(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    callback_result(vm, args, zlib_codec::inflate_zlib)
}

fn deflate_raw_callback(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    callback_result(vm, args, op_deflate_raw)
}

fn inflate_raw_callback(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    callback_result(vm, args, zlib_codec::inflate_raw)
}

fn unzip_callback(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    callback_result(vm, args, zlib_codec::unzip)
}

fn callback_result(vm: &mut Vm, args: &[JsValue], op: fn(&[u8]) -> Option<Vec<u8>>) -> JsValue {
    let bytes = bytes_from(args.first().unwrap_or(&JsValue::Undefined));
    let result = op(&bytes).map(buffer_from_bytes).unwrap_or_else(zlib_error);
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        if is_zlib_error(&result) {
            vm.call_value(callback, &[result, JsValue::Undefined], JsValue::Undefined);
        } else {
            vm.call_value(callback, &[JsValue::Null, result], JsValue::Undefined);
        }
        return JsValue::Undefined;
    }
    result
}

fn sync_passthrough(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    bytes_value(args.first().unwrap_or(&JsValue::Undefined))
}

fn gzip_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    buffer_from_bytes(zlib_codec::gzip(&bytes_from(
        args.first().unwrap_or(&JsValue::Undefined),
    )))
}

fn gunzip_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    result_value(zlib_codec::gunzip(&bytes_from(
        args.first().unwrap_or(&JsValue::Undefined),
    )))
}

fn deflate_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    buffer_from_bytes(zlib_codec::deflate_zlib(&bytes_from(
        args.first().unwrap_or(&JsValue::Undefined),
    )))
}

fn inflate_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    result_value(zlib_codec::inflate_zlib(&bytes_from(
        args.first().unwrap_or(&JsValue::Undefined),
    )))
}

fn deflate_raw_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    buffer_from_bytes(zlib_codec::deflate_raw(&bytes_from(
        args.first().unwrap_or(&JsValue::Undefined),
    )))
}

fn inflate_raw_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    result_value(zlib_codec::inflate_raw(&bytes_from(
        args.first().unwrap_or(&JsValue::Undefined),
    )))
}

fn unzip_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    result_value(zlib_codec::unzip(&bytes_from(
        args.first().unwrap_or(&JsValue::Undefined),
    )))
}

fn create_gzip_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    create_transform("gzip")
}

fn create_gunzip_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    create_transform("gunzip")
}

fn create_deflate_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    create_transform("deflate")
}

fn create_inflate_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    create_transform("inflate")
}

fn create_deflate_raw_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    create_transform("deflateRaw")
}

fn create_inflate_raw_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    create_transform("inflateRaw")
}

fn create_unzip_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    create_transform("unzip")
}

fn create_transform(op: &str) -> JsValue {
    let stream = super::stream::make_passthrough_stream();
    stream.set_property(
        String::from(STREAM_OP_KEY),
        JsValue::String(String::from(op)),
    );
    stream.set_property(
        String::from(STREAM_CHUNKS_KEY),
        JsValue::new_array(Vec::new()),
    );
    stream.set_property(String::from("write"), native_fn("write", zlib_stream_write));
    stream.set_property(String::from("end"), native_fn("end", zlib_stream_end));
    stream
}

fn zlib_stream_write(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let chunk = args.first().cloned().unwrap_or(JsValue::Undefined);
    if !matches!(chunk, JsValue::Undefined | JsValue::Null) {
        let mut chunks = stream_chunks(&vm.current_this);
        chunks.push(chunk);
        vm.current_this
            .set_property(String::from(STREAM_CHUNKS_KEY), JsValue::new_array(chunks));
    }
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], vm.current_this.clone());
    }
    JsValue::Bool(true)
}

fn zlib_stream_end(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !args.is_empty() && !matches!(args[0], JsValue::Function(_)) {
        let _ = zlib_stream_write(vm, &args[..1]);
    }

    let mut input = Vec::new();
    for chunk in stream_chunks(&vm.current_this) {
        input.extend_from_slice(&bytes_from(&chunk));
    }

    let result = match vm
        .current_this
        .get_property(STREAM_OP_KEY)
        .to_js_string()
        .as_str()
    {
        "gzip" => Some(zlib_codec::gzip(&input)),
        "gunzip" => zlib_codec::gunzip(&input),
        "deflate" => Some(zlib_codec::deflate_zlib(&input)),
        "inflate" => zlib_codec::inflate_zlib(&input),
        "deflateRaw" => Some(zlib_codec::deflate_raw(&input)),
        "inflateRaw" => zlib_codec::inflate_raw(&input),
        "unzip" => zlib_codec::unzip(&input),
        _ => Some(input),
    };

    match result {
        Some(bytes) => {
            emit(vm, "data", &[buffer_from_bytes(bytes)]);
            emit(vm, "finish", &[]);
            emit(vm, "end", &[]);
        }
        None => {
            emit(vm, "error", &[zlib_error()]);
        }
    }

    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], vm.current_this.clone());
    }
    vm.current_this.clone()
}

fn stream_chunks(stream: &JsValue) -> Vec<JsValue> {
    match stream.get_property(STREAM_CHUNKS_KEY) {
        JsValue::Array(array) => array.borrow().to_dense_vec(),
        _ => Vec::new(),
    }
}

fn emit(vm: &mut Vm, event: &str, args: &[JsValue]) {
    let emit = vm.current_this.get_property("emit");
    if matches!(emit, JsValue::Function(_)) {
        let mut call_args = Vec::with_capacity(args.len() + 1);
        call_args.push(JsValue::String(String::from(event)));
        call_args.extend_from_slice(args);
        vm.call_value(&emit, &call_args, vm.current_this.clone());
    }
}

fn op_gzip(bytes: &[u8]) -> Option<Vec<u8>> {
    Some(zlib_codec::gzip(bytes))
}

fn op_deflate(bytes: &[u8]) -> Option<Vec<u8>> {
    Some(zlib_codec::deflate_zlib(bytes))
}

fn op_deflate_raw(bytes: &[u8]) -> Option<Vec<u8>> {
    Some(zlib_codec::deflate_raw(bytes))
}

fn result_value(result: Option<Vec<u8>>) -> JsValue {
    result.map(buffer_from_bytes).unwrap_or_else(zlib_error)
}

fn zlib_error() -> JsValue {
    let error = JsValue::new_object();
    error.set_property(String::from("name"), JsValue::String(String::from("Error")));
    error.set_property(
        String::from("message"),
        JsValue::String(String::from("incorrect header check")),
    );
    error.set_property(
        String::from("code"),
        JsValue::String(String::from("Z_DATA_ERROR")),
    );
    error
}

fn is_zlib_error(value: &JsValue) -> bool {
    matches!(
        value.get_property("code"),
        JsValue::String(code) if code == "Z_DATA_ERROR"
    )
}

fn bytes_from(value: &JsValue) -> Vec<u8> {
    if is_buffer_like(value) {
        return buffer_to_bytes(value);
    }
    match value {
        JsValue::String(text) => text.as_bytes().to_vec(),
        JsValue::Array(array) => array
            .borrow()
            .to_dense_vec()
            .into_iter()
            .map(|value| value.to_number().clamp(0.0, 255.0) as u8)
            .collect::<Vec<u8>>(),
        value => value.to_js_string().into_bytes(),
    }
}

fn bytes_value(value: &JsValue) -> JsValue {
    if is_buffer_like(value) {
        return buffer_from_bytes(buffer_to_bytes(value));
    }
    match value {
        JsValue::String(text) => buffer_from_bytes(text.as_bytes().to_vec()),
        JsValue::Array(array) => buffer_from_bytes(
            array
                .borrow()
                .to_dense_vec()
                .into_iter()
                .map(|value| value.to_number().clamp(0.0, 255.0) as u8)
                .collect::<Vec<u8>>(),
        ),
        value => buffer_from_bytes(value.to_js_string().into_bytes()),
    }
}

fn constants_object() -> JsValue {
    let constants = JsValue::new_object();
    constants.set_property(String::from("Z_NO_FLUSH"), JsValue::Number(0.0));
    constants.set_property(String::from("Z_SYNC_FLUSH"), JsValue::Number(2.0));
    constants.set_property(String::from("Z_FULL_FLUSH"), JsValue::Number(3.0));
    constants.set_property(String::from("Z_FINISH"), JsValue::Number(4.0));
    constants.set_property(String::from("Z_OK"), JsValue::Number(0.0));
    constants.set_property(String::from("Z_STREAM_END"), JsValue::Number(1.0));
    constants.set_property(String::from("Z_DEFAULT_COMPRESSION"), JsValue::Number(-1.0));
    constants
}
