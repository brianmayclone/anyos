use alloc::string::{String, ToString};
use alloc::vec::Vec;
use libdb_client::Database;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::buffer::buffer_from_bytes;
use super::util::object;

const DB_HANDLE_KEY: &str = "__anyos_db_handle__";

static mut DATABASES: Option<Vec<Option<Database>>> = None;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("init"), native_fn("init", init));
    module.set(String::from("open"), native_fn("open", open));
    module.set(
        String::from("openMemory"),
        native_fn("openMemory", open_memory),
    );
    module.set(
        String::from("openInMemory"),
        native_fn("openInMemory", open_memory),
    );
    object(module)
}

fn init(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(libdb_client::init())
}

fn open(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let path = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    if path.is_empty() || path == "undefined" {
        vm.pending_exception = Some(vm.make_type_error("db.open requires a database path"));
        return JsValue::Undefined;
    }
    let _ = libdb_client::init();
    match Database::open(&path) {
        Some(database) => make_database(database),
        None => {
            vm.pending_exception = Some(vm.make_type_error("db.open failed"));
            JsValue::Undefined
        }
    }
}

fn open_memory(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let _ = libdb_client::init();
    match Database::open_in_memory() {
        Some(database) => make_database(database),
        None => {
            vm.pending_exception = Some(vm.make_type_error("db.openMemory failed"));
            JsValue::Undefined
        }
    }
}

fn make_database(database: Database) -> JsValue {
    let id = store_database(database);
    let mut out = JsObject::new();
    out.set_hidden(String::from(DB_HANDLE_KEY), JsValue::Number(id as f64));
    out.set(String::from("exec"), native_fn("exec", exec));
    out.set(String::from("query"), native_fn("query", query));
    out.set(String::from("flush"), native_fn("flush", flush));
    out.set(String::from("close"), native_fn("close", close));
    out.set(
        String::from("lastError"),
        native_fn("lastError", last_error),
    );
    object(out)
}

fn exec(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sql = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    if sql.is_empty() || sql == "undefined" {
        vm.pending_exception = Some(vm.make_type_error("db.exec requires SQL"));
        return JsValue::Undefined;
    }
    with_database(vm, |database| match database.exec(&sql) {
        Ok(rows) => JsValue::Number(rows as f64),
        Err(message) => {
            vm.pending_exception = Some(vm.make_type_error(&message));
            JsValue::Undefined
        }
    })
}

fn query(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sql = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    if sql.is_empty() || sql == "undefined" {
        vm.pending_exception = Some(vm.make_type_error("db.query requires SQL"));
        return JsValue::Undefined;
    }
    with_database(vm, |database| match database.query(&sql) {
        Ok(result) => query_result_to_js(&result),
        Err(message) => {
            vm.pending_exception = Some(vm.make_type_error(&message));
            JsValue::Undefined
        }
    })
}

fn flush(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    with_database(vm, |database| match database.flush() {
        Ok(()) => JsValue::Bool(true),
        Err(message) => {
            vm.pending_exception = Some(vm.make_type_error(&message));
            JsValue::Undefined
        }
    })
}

fn close(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let id = vm.current_this.get_property(DB_HANDLE_KEY).to_number() as usize;
    let closed = take_database(id).is_some();
    vm.current_this
        .set_property(String::from(DB_HANDLE_KEY), JsValue::Number(0.0));
    JsValue::Bool(closed)
}

fn last_error(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    with_database(vm, |database| JsValue::String(database.last_error()))
}

fn query_result_to_js(result: &libdb_client::QueryResult) -> JsValue {
    let columns = result.col_names();
    let row_count = result.row_count();
    let col_count = result.col_count();
    let mut rows = Vec::new();
    for row in 0..row_count {
        let row_object = JsValue::new_object();
        let row_array = JsValue::new_array(Vec::new());
        for col in 0..col_count {
            let value = cell_to_js(result, row, col);
            row_array.set_property(col.to_string(), value.clone());
            if let Some(name) = columns.get(col as usize) {
                row_object.set_property(name.clone(), value);
            }
        }
        row_object.set_property(String::from("values"), row_array);
        rows.push(row_object);
    }

    let mut out = JsObject::new();
    out.set(
        String::from("columns"),
        JsValue::new_array(columns.into_iter().map(JsValue::String).collect()),
    );
    out.set(String::from("rows"), JsValue::new_array(rows));
    out.set(String::from("rowCount"), JsValue::Number(row_count as f64));
    out.set(String::from("colCount"), JsValue::Number(col_count as f64));
    object(out)
}

fn cell_to_js(result: &libdb_client::QueryResult, row: u32, col: u32) -> JsValue {
    if result.is_null(row, col) {
        return JsValue::Null;
    }
    if let Some(text) = result.get_text(row, col) {
        if !text.is_empty() {
            return JsValue::String(text);
        }
    }
    if let Some(value) = result.get_int(row, col) {
        return JsValue::Number(value as f64);
    }
    if let Some(blob) = result.get_blob(row, col) {
        return buffer_from_bytes(blob);
    }
    JsValue::String(String::new())
}

fn with_database<F>(vm: &mut Vm, action: F) -> JsValue
where
    F: FnOnce(&Database) -> JsValue,
{
    let id = vm.current_this.get_property(DB_HANDLE_KEY).to_number() as usize;
    if id == 0 {
        vm.pending_exception = Some(vm.make_type_error("database is closed"));
        return JsValue::Undefined;
    }
    match get_database(id) {
        Some(database) => action(database),
        None => {
            vm.pending_exception = Some(vm.make_type_error("database handle is invalid"));
            JsValue::Undefined
        }
    }
}

fn store_database(database: Database) -> usize {
    unsafe {
        let databases = DATABASES.get_or_insert_with(Vec::new);
        databases.push(Some(database));
        databases.len()
    }
}

fn get_database(id: usize) -> Option<&'static Database> {
    unsafe {
        let idx = id.checked_sub(1)?;
        DATABASES.as_ref()?.get(idx)?.as_ref()
    }
}

fn take_database(id: usize) -> Option<Database> {
    unsafe {
        let idx = id.checked_sub(1)?;
        DATABASES.as_mut()?.get_mut(idx)?.take()
    }
}
