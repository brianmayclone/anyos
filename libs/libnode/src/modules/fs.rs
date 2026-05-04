use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, native_promise, Vm};

use super::util::object;

const STAT_TYPE_KEY: &str = "__node_stat_type__";
const STAT_SIZE_KEY: &str = "__node_stat_size__";
const DIRENT_TYPE_KEY: &str = "__node_dirent_type__";

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("readFileSync"),
        native_fn("readFileSync", read_file_sync),
    );
    module.set(
        String::from("writeFileSync"),
        native_fn("writeFileSync", write_file_sync),
    );
    module.set(
        String::from("existsSync"),
        native_fn("existsSync", exists_sync),
    );
    module.set(
        String::from("mkdirSync"),
        native_fn("mkdirSync", mkdir_sync),
    );
    module.set(
        String::from("readdirSync"),
        native_fn("readdirSync", readdir_sync),
    );
    module.set(String::from("statSync"), native_fn("statSync", stat_sync));
    module.set(
        String::from("lstatSync"),
        native_fn("lstatSync", lstat_sync),
    );
    module.set(
        String::from("unlinkSync"),
        native_fn("unlinkSync", unlink_sync),
    );
    module.set(
        String::from("rmdirSync"),
        native_fn("rmdirSync", unlink_sync),
    );
    module.set(String::from("rmSync"), native_fn("rmSync", rm_sync));
    module.set(String::from("promises"), promises_module());
    object(module)
}

pub fn promises_module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("readFile"), native_fn("readFile", read_file));
    module.set(
        String::from("writeFile"),
        native_fn("writeFile", write_file),
    );
    module.set(String::from("mkdir"), native_fn("mkdir", mkdir));
    module.set(String::from("readdir"), native_fn("readdir", readdir));
    module.set(String::from("stat"), native_fn("stat", stat));
    module.set(String::from("lstat"), native_fn("lstat", lstat));
    module.set(String::from("unlink"), native_fn("unlink", unlink));
    module.set(String::from("rmdir"), native_fn("rmdir", rmdir));
    module.set(String::from("rm"), native_fn("rm", rm));
    module.set(String::from("access"), native_fn("access", access));
    object(module)
}

fn read_file(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = read_file_sync(vm, args);
    promise_from_result(vm, value)
}

fn write_file(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = write_file_sync(vm, args);
    promise_from_result(vm, value)
}

fn mkdir(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = mkdir_sync(vm, args);
    promise_from_result(vm, value)
}

fn readdir(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = readdir_sync(vm, args);
    promise_from_result(vm, value)
}

fn stat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = stat_sync(vm, args);
    promise_from_result(vm, value)
}

fn lstat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = lstat_sync(vm, args);
    promise_from_result(vm, value)
}

fn unlink(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = unlink_sync(vm, args);
    promise_from_result(vm, value)
}

fn rmdir(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = unlink_sync(vm, args);
    promise_from_result(vm, value)
}

fn rm(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = rm_sync(vm, args);
    promise_from_result(vm, value)
}

fn access(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let path = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let mut stat = [0u32; 7];
    if anyos_std::fs::stat(&path, &mut stat) == u32::MAX {
        let err = vm.make_type_error(&format!("ENOENT: {}", path));
        return native_promise::promise_reject(vm, &[err]);
    }
    native_promise::promise_resolve(vm, &[JsValue::Undefined])
}

fn promise_from_result(vm: &mut Vm, value: JsValue) -> JsValue {
    if let Some(err) = vm
        .pending_exception
        .take()
        .or_else(|| vm.last_exception.take())
    {
        native_promise::promise_reject(vm, &[err])
    } else {
        native_promise::promise_resolve(vm, &[value])
    }
}

fn read_file_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.readFileSync requires a path"));
        return JsValue::Undefined;
    };
    match anyos_std::fs::read_to_string(&path) {
        Ok(data) => JsValue::String(data),
        Err(_) => {
            vm.pending_exception = Some(vm.make_type_error(&format!("ENOENT: {}", path)));
            JsValue::Undefined
        }
    }
}

fn write_file_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.writeFileSync requires a path"));
        return JsValue::Undefined;
    };
    let data = args
        .get(1)
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    if anyos_std::fs::write_bytes(&path, data.as_bytes()).is_err() {
        vm.pending_exception = Some(vm.make_type_error(&format!("EIO: {}", path)));
    }
    JsValue::Undefined
}

fn exists_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let path = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let mut stat = [0u32; 7];
    JsValue::Bool(anyos_std::fs::stat(&path, &mut stat) != u32::MAX)
}

fn mkdir_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.mkdirSync requires a path"));
        return JsValue::Undefined;
    };
    let ok = if option_bool(args.get(1), "recursive") {
        mkdir_recursive(&path)
    } else {
        anyos_std::fs::mkdir(&path) != u32::MAX
    };
    if !ok {
        vm.pending_exception = Some(vm.make_type_error(&format!("EIO: {}", path)));
    }
    JsValue::Undefined
}

fn readdir_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.readdirSync requires a path"));
        return JsValue::Undefined;
    };
    let mut buf = alloc::vec![0u8; 8192];
    let count = anyos_std::fs::readdir(&path, &mut buf);
    if count == u32::MAX {
        vm.pending_exception = Some(vm.make_type_error(&format!("ENOENT: {}", path)));
        return JsValue::Undefined;
    }
    let with_file_types = option_bool(args.get(1), "withFileTypes");
    let mut entries = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let base = index * 64;
        if base + 64 > buf.len() {
            break;
        }
        let entry_type = buf[base] as u32;
        let name_len = buf[base + 1] as usize;
        let name_start = base + 8;
        let name_end = (name_start + name_len).min(base + 64);
        if let Ok(name) = core::str::from_utf8(&buf[name_start..name_end]) {
            if with_file_types {
                entries.push(make_dirent(String::from(name), entry_type));
            } else {
                entries.push(JsValue::String(String::from(name)));
            }
        }
    }
    JsValue::new_array(entries)
}

fn stat_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    stat_common(vm, args, false)
}

fn lstat_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    stat_common(vm, args, true)
}

fn stat_common(vm: &mut Vm, args: &[JsValue], no_follow: bool) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.statSync requires a path"));
        return JsValue::Undefined;
    };
    let mut stat = [0u32; 7];
    let ret = if no_follow {
        anyos_std::fs::lstat(&path, &mut stat)
    } else {
        anyos_std::fs::stat(&path, &mut stat)
    };
    if ret == u32::MAX {
        vm.pending_exception = Some(vm.make_type_error(&format!("ENOENT: {}", path)));
        return JsValue::Undefined;
    }
    make_stats_object(stat)
}

fn unlink_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.unlinkSync requires a path"));
        return JsValue::Undefined;
    };
    if anyos_std::fs::unlink(&path) == u32::MAX {
        vm.pending_exception = Some(vm.make_type_error(&format!("ENOENT: {}", path)));
    }
    JsValue::Undefined
}

fn rm_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.rmSync requires a path"));
        return JsValue::Undefined;
    };
    let recursive = option_bool(args.get(1), "recursive");
    let force = option_bool(args.get(1), "force");
    let ok = if recursive {
        remove_recursive(&path, force)
    } else {
        anyos_std::fs::unlink(&path) != u32::MAX || force
    };
    if !ok {
        vm.pending_exception = Some(vm.make_type_error(&format!("ENOENT: {}", path)));
    }
    JsValue::Undefined
}

fn make_stats_object(stat: [u32; 7]) -> JsValue {
    let mut out = JsObject::new();
    out.set(String::from("size"), JsValue::Number(stat[1] as f64));
    out.set(String::from("mode"), JsValue::Number(stat[5] as f64));
    out.set(String::from("mtimeMs"), JsValue::Number(stat[6] as f64));
    out.set(String::from("isFile"), native_fn("isFile", stat_is_file));
    out.set(
        String::from("isDirectory"),
        native_fn("isDirectory", stat_is_directory),
    );
    out.set(
        String::from("isSymbolicLink"),
        native_fn("isSymbolicLink", stat_is_symlink),
    );
    out.set_hidden(String::from(STAT_TYPE_KEY), JsValue::Number(stat[0] as f64));
    out.set_hidden(String::from(STAT_SIZE_KEY), JsValue::Number(stat[1] as f64));
    object(out)
}

fn stat_is_file(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(vm.current_this.get_property(STAT_TYPE_KEY).to_number() as u32 == 0)
}

fn stat_is_directory(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(vm.current_this.get_property(STAT_TYPE_KEY).to_number() as u32 == 1)
}

fn stat_is_symlink(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let ty = vm.current_this.get_property(STAT_TYPE_KEY).to_number() as u32;
    JsValue::Bool(ty == 2)
}

fn make_dirent(name: String, entry_type: u32) -> JsValue {
    let mut out = JsObject::new();
    out.set(String::from("name"), JsValue::String(name));
    out.set(String::from("isFile"), native_fn("isFile", dirent_is_file));
    out.set(
        String::from("isDirectory"),
        native_fn("isDirectory", dirent_is_directory),
    );
    out.set(
        String::from("isSymbolicLink"),
        native_fn("isSymbolicLink", dirent_is_symlink),
    );
    out.set_hidden(
        String::from(DIRENT_TYPE_KEY),
        JsValue::Number(entry_type as f64),
    );
    object(out)
}

fn dirent_is_file(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(vm.current_this.get_property(DIRENT_TYPE_KEY).to_number() as u32 == 0)
}

fn dirent_is_directory(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(vm.current_this.get_property(DIRENT_TYPE_KEY).to_number() as u32 == 1)
}

fn dirent_is_symlink(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(vm.current_this.get_property(DIRENT_TYPE_KEY).to_number() as u32 == 2)
}

fn option_bool(options: Option<&JsValue>, key: &str) -> bool {
    options
        .map(|options| options.get_property(key).to_boolean())
        .unwrap_or(false)
}

fn mkdir_recursive(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let mut built = if path.starts_with('/') {
        String::from("/")
    } else {
        String::new()
    };
    for part in path.split('/').filter(|part| !part.is_empty()) {
        if !built.is_empty() && !built.ends_with('/') {
            built.push('/');
        }
        built.push_str(part);
        let mut stat = [0u32; 7];
        if anyos_std::fs::stat(&built, &mut stat) != u32::MAX {
            if stat[0] != 1 {
                return false;
            }
            continue;
        }
        if anyos_std::fs::mkdir(&built) == u32::MAX {
            return false;
        }
    }
    true
}

fn remove_recursive(path: &str, force: bool) -> bool {
    let mut stat = [0u32; 7];
    if anyos_std::fs::lstat(path, &mut stat) == u32::MAX {
        return force;
    }
    if stat[0] == 1 {
        let mut buf = alloc::vec![0u8; 8192];
        let count = anyos_std::fs::readdir(path, &mut buf);
        if count == u32::MAX {
            return false;
        }
        for index in 0..count as usize {
            let base = index * 64;
            if base + 64 > buf.len() {
                break;
            }
            let name_len = buf[base + 1] as usize;
            let name_start = base + 8;
            let name_end = (name_start + name_len).min(base + 64);
            let Ok(name) = core::str::from_utf8(&buf[name_start..name_end]) else {
                return false;
            };
            if name == "." || name == ".." {
                continue;
            }
            if !remove_recursive(&join_path(path, name), force) {
                return false;
            }
        }
    }
    anyos_std::fs::unlink(path) != u32::MAX || force
}

fn join_path(parent: &str, child: &str) -> String {
    if parent.ends_with('/') {
        format!("{}{}", parent, child)
    } else {
        format!("{}/{}", parent, child)
    }
}
