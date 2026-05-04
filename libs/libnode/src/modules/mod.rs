pub mod assert;
pub mod buffer;
pub mod child_process;
pub mod commonjs;
pub mod constants;
pub mod crypto;
pub mod dns;
pub mod events;
pub mod fs;
pub mod http;
pub mod native;
pub mod net;
pub mod node_module;
pub mod os;
pub mod path;
pub mod process;
pub mod querystring;
pub mod stream;
pub mod string_decoder;
pub mod timers;
pub mod tls;
pub mod tty;
pub mod url;
pub mod util;
pub mod uv;
pub mod web;
pub mod zlib;
mod zlib_codec;

pub use assert::{module as assert_module, strict_module as assert_strict_module};
pub use buffer::module as buffer_module;
pub use child_process::module as child_process_module;
pub use commonjs::{
    module_object as commonjs_module, require as node_require, resolve as node_require_resolve,
};
pub use constants::module as constants_module;
pub use crypto::module as crypto_module;
pub use dns::module as dns_module;
pub use dns::promises_module as dns_promises_module;
pub use events::module as events_module;
pub use fs::module as fs_module;
pub use fs::promises_module as fs_promises_module;
pub use http::module as http_module;
pub use native::{anyui_module, ffi_module, image_module};
pub use net::module as net_module;
pub use node_module::module as node_module_module;
pub use os::module as os_module;
pub use path::{
    module as path_module, posix_module as path_posix_module, win32_module as path_win32_module,
};
pub use process::module as process_module;
pub use querystring::module as querystring_module;
pub use stream::{
    consumers_module as stream_consumers_module, module as stream_module,
    promises_module as stream_promises_module, web_module as stream_web_module,
};
pub use string_decoder::module as string_decoder_module;
pub use timers::{module as timers_module, promises_module as timers_promises_module};
pub use tls::module as tls_module;
pub use tty::module as tty_module;
pub use url::module as url_module;
pub use util::{module as util_module, types_module as util_types_module};
pub use uv::module as uv_module;
pub use web::globals_module as web_globals_module;
pub use zlib::module as zlib_module;
