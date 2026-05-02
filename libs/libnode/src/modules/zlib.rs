use alloc::string::String;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::buffer::{buffer_from_bytes, buffer_to_bytes, is_buffer_like};
use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("gzip"),
        native_fn("gzip", callback_passthrough),
    );
    module.set(
        String::from("gunzip"),
        native_fn("gunzip", callback_passthrough),
    );
    module.set(
        String::from("deflate"),
        native_fn("deflate", callback_passthrough),
    );
    module.set(
        String::from("inflate"),
        native_fn("inflate", callback_passthrough),
    );
    module.set(
        String::from("unzip"),
        native_fn("unzip", callback_passthrough),
    );
    module.set(
        String::from("brotliCompress"),
        native_fn("brotliCompress", callback_passthrough),
    );
    module.set(
        String::from("brotliDecompress"),
        native_fn("brotliDecompress", callback_passthrough),
    );
    module.set(
        String::from("gzipSync"),
        native_fn("gzipSync", sync_passthrough),
    );
    module.set(
        String::from("gunzipSync"),
        native_fn("gunzipSync", sync_passthrough),
    );
    module.set(
        String::from("deflateSync"),
        native_fn("deflateSync", sync_passthrough),
    );
    module.set(
        String::from("inflateSync"),
        native_fn("inflateSync", sync_passthrough),
    );
    module.set(
        String::from("unzipSync"),
        native_fn("unzipSync", sync_passthrough),
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
        native_fn("createGzip", create_transform),
    );
    module.set(
        String::from("createGunzip"),
        native_fn("createGunzip", create_transform),
    );
    module.set(
        String::from("createDeflate"),
        native_fn("createDeflate", create_transform),
    );
    module.set(
        String::from("createInflate"),
        native_fn("createInflate", create_transform),
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

fn sync_passthrough(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    bytes_value(args.first().unwrap_or(&JsValue::Undefined))
}

fn create_transform(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let stream = super::stream::make_passthrough_stream();
    stream
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
