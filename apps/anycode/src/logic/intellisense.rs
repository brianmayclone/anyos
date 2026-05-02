use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::language::{self, LanguageId};
use crate::logic::node_packages;
use crate::logic::project::Project;
use crate::logic::symbol_index::{IndexedSymbol, SymbolIndex};
use crate::logic::symbols::SymbolKind;
use crate::util::path;

const MAX_COMPLETIONS: usize = 64;

#[derive(Clone)]
pub struct CompletionItem {
    pub label: String,
    pub insert_text: String,
    pub detail: String,
}

pub struct CompletionSet {
    pub prefix: String,
    pub items: Vec<CompletionItem>,
}

pub fn completions_for_cursor(
    file_path: &str,
    text: &str,
    row: u32,
    col: u32,
    index: &SymbolIndex,
    project: Option<&Project>,
) -> CompletionSet {
    let lang = language::language_for_filename(path::basename(file_path)).id;
    let prefix = prefix_at(text, row as usize, col as usize);
    let mut items = Vec::new();

    if matches!(lang, LanguageId::JavaScript | LanguageId::TypeScript) {
        if let Some(module_prefix) = module_string_prefix_at(text, row as usize, col as usize) {
            push_node_module_completions(&mut items, &module_prefix, project);
            return CompletionSet {
                prefix: module_prefix,
                items,
            };
        }

        if let Some((receiver, member_prefix)) = member_access_at(text, row as usize, col as usize)
        {
            if is_anyui_alias(text, &receiver) {
                let mut member_items = Vec::new();
                push_anyui_members(&mut member_items, &member_prefix);
                return CompletionSet {
                    prefix: member_prefix,
                    items: member_items,
                };
            }
            if let Some(kind) = js_receiver_kind(text, &receiver) {
                let mut member_items = Vec::new();
                push_js_receiver_members(&mut member_items, &member_prefix, kind);
                return CompletionSet {
                    prefix: member_prefix,
                    items: member_items,
                };
            }
        }
        push_js_node_globals(&mut items, &prefix, project);
        push_js_anyos_completions(&mut items, &prefix);
    }

    if let Some(info) = language::info_for_id(lang) {
        for &(trigger, body) in info.snippets {
            push_completion(&mut items, &prefix, trigger, body, "snippet");
        }
        for keyword in info.keywords {
            push_completion(&mut items, &prefix, keyword, keyword, lang.display_name());
        }
    }

    for symbol in &index.symbols {
        if !symbol_matches_file(lang, symbol) {
            continue;
        }
        push_completion(
            &mut items,
            &prefix,
            &symbol.name,
            &symbol.name,
            &format!(
                "{} · {}",
                symbol.kind.label(),
                path::basename(&symbol.file_path)
            ),
        );
        if items.len() >= MAX_COMPLETIONS {
            break;
        }
    }

    CompletionSet { prefix, items }
}

pub fn hover_for_cursor(
    file_path: &str,
    text: &str,
    row: u32,
    col: u32,
    index: &SymbolIndex,
) -> String {
    let word = word_at(text, row as usize, col as usize);
    if word.is_empty() {
        return String::new();
    }

    if let Some(symbol) = best_symbol_for_word(file_path, &word, index) {
        return format!(
            "{} {}\n{}\n{}:{}",
            symbol.kind.label(),
            symbol.name,
            symbol.detail,
            path::basename(&symbol.file_path),
            symbol.line + 1
        );
    }

    let lang = language::language_for_filename(path::basename(file_path)).id;
    if let Some(info) = language::info_for_id(lang) {
        if info.keywords.iter().any(|kw| *kw == word) {
            return format!("{} keyword: {}", lang.display_name(), word);
        }
    }

    String::new()
}

pub fn word_at_cursor(text: &str, row: u32, col: u32) -> String {
    word_at(text, row as usize, col as usize)
}

pub fn should_auto_popup(file_path: &str, text: &str, row: u32, col: u32) -> bool {
    let lang = language::language_for_filename(path::basename(file_path)).id;
    if !matches!(lang, LanguageId::JavaScript | LanguageId::TypeScript) {
        return false;
    }
    if module_string_prefix_at(text, row as usize, col as usize).is_some() {
        return true;
    }
    previous_byte_at(text, row as usize, col as usize) == Some(b'.')
}

pub fn best_symbol_for_word<'a>(
    file_path: &str,
    word: &str,
    index: &'a SymbolIndex,
) -> Option<&'a IndexedSymbol> {
    let mut fallback = None;
    for symbol in &index.symbols {
        if symbol.name != word {
            continue;
        }
        if symbol.file_path == file_path {
            return Some(symbol);
        }
        if fallback.is_none() {
            fallback = Some(symbol);
        }
    }
    fallback
}

pub fn completion_list_text(items: &[CompletionItem]) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push('|');
        }
        out.push_str(&item.label);
        if !item.detail.is_empty() {
            out.push_str("    ");
            out.push_str(&item.detail);
        }
    }
    out
}

fn push_completion(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    label: &str,
    insert_text: &str,
    detail: &str,
) {
    if items.len() >= MAX_COMPLETIONS || label.is_empty() {
        return;
    }
    if !prefix.is_empty() && !starts_with_ascii_ci(label, prefix) {
        return;
    }
    if items.iter().any(|item| item.label == label) {
        return;
    }
    items.push(CompletionItem {
        label: String::from(label),
        insert_text: strip_placeholders(insert_text),
        detail: String::from(detail),
    });
}

fn push_js_anyos_completions(items: &mut Vec<CompletionItem>, prefix: &str) {
    for &(label, insert, detail) in JS_ANYOS_COMPLETIONS {
        push_completion(items, prefix, label, insert, detail);
    }
}

fn push_anyui_members(items: &mut Vec<CompletionItem>, prefix: &str) {
    for &(label, insert, detail) in ANYUI_MEMBER_COMPLETIONS {
        push_completion(items, prefix, label, insert, detail);
    }
}

fn push_node_module_completions(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    project: Option<&Project>,
) {
    for &(module, detail) in NODE_CORE_MODULES {
        push_completion(items, prefix, module, module, detail);
    }
    if let Some(project) = project {
        for package in node_packages::packages_for_project(project) {
            push_completion(
                items,
                prefix,
                &package.name,
                &package.name,
                package.kind.display_name(),
            );
        }
    }
}

fn push_js_node_globals(items: &mut Vec<CompletionItem>, prefix: &str, project: Option<&Project>) {
    for &(label, insert, detail) in JS_NODE_GLOBALS {
        push_completion(items, prefix, label, insert, detail);
    }
    push_node_module_completions(items, prefix, project);
}

#[derive(Clone, Copy, PartialEq)]
enum JsReceiverKind {
    Console,
    Process,
    Buffer,
    Json,
    Math,
    Promise,
    Fs,
    Path,
    Http,
    Https,
    Url,
    Events,
    Stream,
    ChildProcess,
    AnyUiControl,
}

fn push_js_receiver_members(items: &mut Vec<CompletionItem>, prefix: &str, kind: JsReceiverKind) {
    let members: &[(&str, &str, &str)] = match kind {
        JsReceiverKind::Console => CONSOLE_MEMBERS,
        JsReceiverKind::Process => PROCESS_MEMBERS,
        JsReceiverKind::Buffer => BUFFER_MEMBERS,
        JsReceiverKind::Json => JSON_MEMBERS,
        JsReceiverKind::Math => MATH_MEMBERS,
        JsReceiverKind::Promise => PROMISE_MEMBERS,
        JsReceiverKind::Fs => FS_MEMBERS,
        JsReceiverKind::Path => PATH_MEMBERS,
        JsReceiverKind::Http => HTTP_MEMBERS,
        JsReceiverKind::Https => HTTP_MEMBERS,
        JsReceiverKind::Url => URL_MEMBERS,
        JsReceiverKind::Events => EVENTS_MEMBERS,
        JsReceiverKind::Stream => STREAM_MEMBERS,
        JsReceiverKind::ChildProcess => CHILD_PROCESS_MEMBERS,
        JsReceiverKind::AnyUiControl => ANYUI_CONTROL_MEMBERS,
    };
    for &(label, insert, detail) in members {
        push_completion(items, prefix, label, insert, detail);
    }
}

fn js_receiver_kind(text: &str, receiver: &str) -> Option<JsReceiverKind> {
    match receiver {
        "console" => return Some(JsReceiverKind::Console),
        "process" => return Some(JsReceiverKind::Process),
        "Buffer" => return Some(JsReceiverKind::Buffer),
        "JSON" => return Some(JsReceiverKind::Json),
        "Math" => return Some(JsReceiverKind::Math),
        "Promise" => return Some(JsReceiverKind::Promise),
        _ => {}
    }

    if is_anyui_instance(text, receiver) {
        return Some(JsReceiverKind::AnyUiControl);
    }

    for line in text.split('\n') {
        if !line.contains(receiver) {
            continue;
        }
        let compact = without_spaces(line);
        if let Some(module) = required_module_for_receiver(&compact, receiver) {
            return module_receiver_kind(&module);
        }
        if let Some(module) = imported_module_for_receiver(&compact, receiver) {
            return module_receiver_kind(&module);
        }
    }
    None
}

fn required_module_for_receiver(compact_line: &str, receiver: &str) -> Option<String> {
    let assign = format!("{}=require(", receiver);
    let pos = compact_line.find(&assign)?;
    let rest = &compact_line[pos + assign.len()..];
    quoted_prefix(rest)
}

fn imported_module_for_receiver(compact_line: &str, receiver: &str) -> Option<String> {
    let star_single = format!("*as{}from'", receiver);
    if let Some(pos) = compact_line.find(&star_single) {
        return quoted_prefix(&compact_line[pos + star_single.len() - 1..]);
    }
    let star_double = format!("*as{}from\"", receiver);
    if let Some(pos) = compact_line.find(&star_double) {
        return quoted_prefix(&compact_line[pos + star_double.len() - 1..]);
    }
    let default_single = format!("import{}from'", receiver);
    if let Some(pos) = compact_line.find(&default_single) {
        return quoted_prefix(&compact_line[pos + default_single.len() - 1..]);
    }
    let default_double = format!("import{}from\"", receiver);
    if let Some(pos) = compact_line.find(&default_double) {
        return quoted_prefix(&compact_line[pos + default_double.len() - 1..]);
    }
    None
}

fn module_receiver_kind(module: &str) -> Option<JsReceiverKind> {
    match module {
        "fs" | "node:fs" | "fs/promises" | "node:fs/promises" => Some(JsReceiverKind::Fs),
        "path" | "node:path" => Some(JsReceiverKind::Path),
        "http" | "node:http" => Some(JsReceiverKind::Http),
        "https" | "node:https" => Some(JsReceiverKind::Https),
        "url" | "node:url" => Some(JsReceiverKind::Url),
        "events" | "node:events" => Some(JsReceiverKind::Events),
        "stream" | "node:stream" => Some(JsReceiverKind::Stream),
        "child_process" | "node:child_process" => Some(JsReceiverKind::ChildProcess),
        _ => None,
    }
}

fn is_anyui_instance(text: &str, receiver: &str) -> bool {
    for line in text.split('\n') {
        if !line.contains(receiver) || !line.contains("new ") {
            continue;
        }
        let compact = without_spaces(line);
        let lhs = format!("{}=new", receiver);
        if compact.contains(&lhs)
            && (compact.contains("newui.")
                || compact.contains("newanyui.")
                || compact.contains("newrequire('@anyos/anyui').")
                || compact.contains("newrequire(\"@anyos/anyui\")."))
        {
            return true;
        }
    }
    false
}

fn module_string_prefix_at(text: &str, row: usize, col: usize) -> Option<String> {
    let line = nth_line(text, row);
    let col = col.min(line.len());
    let before = &line[..col];
    let quote_pos = before
        .rfind('\'')
        .or_else(|| before.rfind('"'))
        .or_else(|| before.rfind('`'))?;
    let prefix = &before[quote_pos + 1..];
    if prefix.contains('\'') || prefix.contains('"') || prefix.contains('`') {
        return None;
    }
    let context = without_spaces(&before[..quote_pos]);
    if context.ends_with("require(")
        || context.ends_with("import(")
        || context.ends_with("from")
        || context.ends_with("import")
    {
        return Some(String::from(prefix));
    }
    None
}

fn quoted_prefix(value: &str) -> Option<String> {
    let quote = value.chars().next()?;
    if quote != '\'' && quote != '"' && quote != '`' {
        return None;
    }
    let rest = &value[quote.len_utf8()..];
    let end = rest.find(quote).unwrap_or(rest.len());
    Some(String::from(&rest[..end]))
}

const JS_ANYOS_COMPLETIONS: &[(&str, &str, &str)] = &[
    ("@anyos/anyui", "@anyos/anyui", "native anyOS UI module"),
    (
        "requireAnyUI",
        "const ui = require('@anyos/anyui');",
        "import native anyOS UI",
    ),
    (
        "createWindow",
        "const win = new ui.Window('Main', 120, 80, 800, 520);",
        "anyOS UI window",
    ),
    (
        "anyuiButton",
        "const button = new ui.Button('OK');",
        "anyOS UI control",
    ),
    (
        "anyuiLabel",
        "const label = new ui.Label('Label');",
        "anyOS UI control",
    ),
];

const JS_NODE_GLOBALS: &[(&str, &str, &str)] = &[
    (
        "require",
        "require('${1:module}')",
        "CommonJS module import",
    ),
    ("module", "module", "CommonJS current module"),
    ("exports", "exports", "CommonJS exports object"),
    ("__dirname", "__dirname", "current module directory"),
    ("__filename", "__filename", "current module file"),
    ("process", "process", "Node.js process object"),
    ("console", "console", "console logging API"),
    ("Buffer", "Buffer", "binary buffer API"),
    ("setTimeout", "setTimeout(() => {\n    \n}, 0)", "timer"),
    (
        "setInterval",
        "setInterval(() => {\n    \n}, 1000)",
        "timer",
    ),
    ("clearTimeout", "clearTimeout", "timer cleanup"),
    ("clearInterval", "clearInterval", "timer cleanup"),
    ("Promise", "Promise", "Promise constructor"),
    ("async", "async", "async function keyword"),
    ("await", "await", "await expression"),
];

const NODE_CORE_MODULES: &[(&str, &str)] = &[
    ("assert", "Node.js core module"),
    ("buffer", "Node.js core module"),
    ("child_process", "Node.js process spawning"),
    ("crypto", "Node.js core module"),
    ("dns", "Node.js DNS module"),
    ("events", "Node.js events module"),
    ("fs", "Node.js filesystem module"),
    ("fs/promises", "Node.js promise filesystem module"),
    ("http", "Node.js HTTP module"),
    ("https", "Node.js HTTPS module"),
    ("net", "Node.js TCP module"),
    ("os", "Node.js OS module"),
    ("path", "Node.js path module"),
    ("process", "Node.js process module"),
    ("querystring", "Node.js querystring module"),
    ("stream", "Node.js stream module"),
    ("timers", "Node.js timers module"),
    ("url", "Node.js URL module"),
    ("util", "Node.js utilities module"),
    ("zlib", "Node.js compression module"),
    ("node:fs", "Node.js filesystem module"),
    ("node:http", "Node.js HTTP module"),
    ("node:path", "Node.js path module"),
];

const CONSOLE_MEMBERS: &[(&str, &str, &str)] = &[
    ("log", "log", "console.log(...args)"),
    ("info", "info", "console.info(...args)"),
    ("warn", "warn", "console.warn(...args)"),
    ("error", "error", "console.error(...args)"),
    ("debug", "debug", "console.debug(...args)"),
    ("trace", "trace", "console.trace(...args)"),
    ("time", "time", "console.time(label)"),
    ("timeEnd", "timeEnd", "console.timeEnd(label)"),
    ("dir", "dir", "console.dir(value)"),
];

const PROCESS_MEMBERS: &[(&str, &str, &str)] = &[
    ("argv", "argv", "process arguments"),
    ("env", "env", "environment variables"),
    ("cwd", "cwd", "process.cwd()"),
    ("exit", "exit", "process.exit(code)"),
    ("nextTick", "nextTick", "process.nextTick(callback)"),
    ("platform", "platform", "platform string"),
    ("version", "version", "Node.js version"),
    ("versions", "versions", "runtime component versions"),
    ("stdin", "stdin", "standard input stream"),
    ("stdout", "stdout", "standard output stream"),
    ("stderr", "stderr", "standard error stream"),
    ("on", "on", "event listener"),
];

const BUFFER_MEMBERS: &[(&str, &str, &str)] = &[
    ("from", "from", "Buffer.from(value)"),
    ("alloc", "alloc", "Buffer.alloc(size)"),
    ("allocUnsafe", "allocUnsafe", "Buffer.allocUnsafe(size)"),
    ("byteLength", "byteLength", "Buffer.byteLength(value)"),
    ("concat", "concat", "Buffer.concat(list)"),
    ("isBuffer", "isBuffer", "Buffer.isBuffer(value)"),
];

const JSON_MEMBERS: &[(&str, &str, &str)] = &[
    ("parse", "parse", "JSON.parse(text)"),
    ("stringify", "stringify", "JSON.stringify(value)"),
];

const MATH_MEMBERS: &[(&str, &str, &str)] = &[
    ("abs", "abs", "Math.abs(x)"),
    ("ceil", "ceil", "Math.ceil(x)"),
    ("floor", "floor", "Math.floor(x)"),
    ("max", "max", "Math.max(...values)"),
    ("min", "min", "Math.min(...values)"),
    ("random", "random", "Math.random()"),
    ("round", "round", "Math.round(x)"),
    ("trunc", "trunc", "Math.trunc(x)"),
];

const PROMISE_MEMBERS: &[(&str, &str, &str)] = &[
    ("all", "all", "Promise.all(iterable)"),
    ("allSettled", "allSettled", "Promise.allSettled(iterable)"),
    ("race", "race", "Promise.race(iterable)"),
    ("resolve", "resolve", "Promise.resolve(value)"),
    ("reject", "reject", "Promise.reject(error)"),
];

const FS_MEMBERS: &[(&str, &str, &str)] = &[
    ("readFile", "readFile", "fs.readFile(path, callback)"),
    (
        "readFileSync",
        "readFileSync",
        "fs.readFileSync(path, encoding)",
    ),
    (
        "writeFile",
        "writeFile",
        "fs.writeFile(path, data, callback)",
    ),
    (
        "writeFileSync",
        "writeFileSync",
        "fs.writeFileSync(path, data)",
    ),
    ("existsSync", "existsSync", "fs.existsSync(path)"),
    ("mkdir", "mkdir", "fs.mkdir(path, options, callback)"),
    ("mkdirSync", "mkdirSync", "fs.mkdirSync(path, options)"),
    ("readdir", "readdir", "fs.readdir(path, callback)"),
    ("readdirSync", "readdirSync", "fs.readdirSync(path)"),
    ("stat", "stat", "fs.stat(path, callback)"),
    ("statSync", "statSync", "fs.statSync(path)"),
    ("unlink", "unlink", "fs.unlink(path, callback)"),
    ("unlinkSync", "unlinkSync", "fs.unlinkSync(path)"),
    (
        "createReadStream",
        "createReadStream",
        "fs.createReadStream(path)",
    ),
    (
        "createWriteStream",
        "createWriteStream",
        "fs.createWriteStream(path)",
    ),
    ("promises", "promises", "fs.promises API"),
];

const PATH_MEMBERS: &[(&str, &str, &str)] = &[
    ("join", "join", "path.join(...parts)"),
    ("resolve", "resolve", "path.resolve(...parts)"),
    ("dirname", "dirname", "path.dirname(path)"),
    ("basename", "basename", "path.basename(path)"),
    ("extname", "extname", "path.extname(path)"),
    ("normalize", "normalize", "path.normalize(path)"),
    ("relative", "relative", "path.relative(from, to)"),
    ("isAbsolute", "isAbsolute", "path.isAbsolute(path)"),
    ("sep", "sep", "path separator"),
];

const HTTP_MEMBERS: &[(&str, &str, &str)] = &[
    ("createServer", "createServer", "http.createServer(handler)"),
    ("request", "request", "http.request(options, callback)"),
    ("get", "get", "http.get(options, callback)"),
    ("Server", "Server", "HTTP server class"),
    (
        "IncomingMessage",
        "IncomingMessage",
        "HTTP request/response message",
    ),
    ("ServerResponse", "ServerResponse", "HTTP server response"),
    ("STATUS_CODES", "STATUS_CODES", "HTTP status code map"),
];

const URL_MEMBERS: &[(&str, &str, &str)] = &[
    ("URL", "URL", "URL class"),
    (
        "URLSearchParams",
        "URLSearchParams",
        "URLSearchParams class",
    ),
    ("parse", "parse", "url.parse(input)"),
    ("format", "format", "url.format(urlObject)"),
    ("pathToFileURL", "pathToFileURL", "url.pathToFileURL(path)"),
    ("fileURLToPath", "fileURLToPath", "url.fileURLToPath(url)"),
];

const EVENTS_MEMBERS: &[(&str, &str, &str)] = &[
    ("EventEmitter", "EventEmitter", "EventEmitter class"),
    ("once", "once", "events.once(emitter, name)"),
    ("on", "on", "events.on(emitter, name)"),
];

const STREAM_MEMBERS: &[(&str, &str, &str)] = &[
    ("Readable", "Readable", "Readable stream class"),
    ("Writable", "Writable", "Writable stream class"),
    ("Duplex", "Duplex", "Duplex stream class"),
    ("Transform", "Transform", "Transform stream class"),
    ("pipeline", "pipeline", "stream.pipeline(...)"),
    ("finished", "finished", "stream.finished(stream, callback)"),
];

const CHILD_PROCESS_MEMBERS: &[(&str, &str, &str)] = &[
    ("spawn", "spawn", "child_process.spawn(command, args)"),
    ("exec", "exec", "child_process.exec(command, callback)"),
    ("execFile", "execFile", "child_process.execFile(file, args)"),
    ("fork", "fork", "child_process.fork(modulePath)"),
    (
        "spawnSync",
        "spawnSync",
        "child_process.spawnSync(command, args)",
    ),
    ("execSync", "execSync", "child_process.execSync(command)"),
];

const ANYUI_MEMBER_COMPLETIONS: &[(&str, &str, &str)] = &[
    ("Window", "Window", "@anyos/anyui class"),
    ("View", "View", "@anyos/anyui class"),
    ("Button", "Button", "@anyos/anyui class"),
    ("PlainButton", "PlainButton", "@anyos/anyui class"),
    ("IconButton", "IconButton", "@anyos/anyui class"),
    ("ImageButton", "ImageButton", "@anyos/anyui class"),
    ("Label", "Label", "@anyos/anyui class"),
    ("LinkLabel", "LinkLabel", "@anyos/anyui class"),
    ("TextField", "TextField", "@anyos/anyui class"),
    ("TextArea", "TextArea", "@anyos/anyui class"),
    (
        "AutoCompleteTextField",
        "AutoCompleteTextField",
        "@anyos/anyui class",
    ),
    ("SearchField", "SearchField", "@anyos/anyui class"),
    ("CheckBox", "CheckBox", "@anyos/anyui class"),
    ("RadioButton", "RadioButton", "@anyos/anyui class"),
    ("Toggle", "Toggle", "@anyos/anyui class"),
    ("DropDown", "DropDown", "@anyos/anyui class"),
    ("ComboBox", "ComboBox", "@anyos/anyui class"),
    ("ListBox", "ListBox", "@anyos/anyui class"),
    ("TreeView", "TreeView", "@anyos/anyui class"),
    ("DataGrid", "DataGrid", "@anyos/anyui class"),
    ("TableView", "TableView", "@anyos/anyui class"),
    ("TabBar", "TabBar", "@anyos/anyui class"),
    ("SegmentedControl", "SegmentedControl", "@anyos/anyui class"),
    ("Toolbar", "Toolbar", "@anyos/anyui class"),
    ("NavigationBar", "NavigationBar", "@anyos/anyui class"),
    ("GroupBox", "GroupBox", "@anyos/anyui class"),
    ("Panel", "Panel", "@anyos/anyui class"),
    ("FlowPanel", "FlowPanel", "@anyos/anyui class"),
    ("StackPanel", "StackPanel", "@anyos/anyui class"),
    ("SplitView", "SplitView", "@anyos/anyui class"),
    ("ScrollView", "ScrollView", "@anyos/anyui class"),
    ("Canvas", "Canvas", "@anyos/anyui class"),
    ("ImageView", "ImageView", "@anyos/anyui class"),
    ("ColorWell", "ColorWell", "@anyos/anyui class"),
    ("DatePicker", "DatePicker", "@anyos/anyui class"),
    ("TimePicker", "TimePicker", "@anyos/anyui class"),
    ("ProgressBar", "ProgressBar", "@anyos/anyui class"),
    ("Slider", "Slider", "@anyos/anyui class"),
    ("Stepper", "Stepper", "@anyos/anyui class"),
    ("Spinner", "Spinner", "@anyos/anyui class"),
    ("StatusIndicator", "StatusIndicator", "@anyos/anyui class"),
    ("Alert", "Alert", "@anyos/anyui class"),
    ("Badge", "Badge", "@anyos/anyui class"),
    ("Tooltip", "Tooltip", "@anyos/anyui class"),
];

const ANYUI_CONTROL_MEMBERS: &[(&str, &str, &str)] = &[
    ("add", "add", "add child control"),
    ("remove", "remove", "remove child control"),
    ("setText", "setText", "set control text"),
    ("getText", "getText", "read control text"),
    ("setPosition", "setPosition", "set absolute position"),
    ("setSize", "setSize", "set control size"),
    ("setDock", "setDock", "set dock layout"),
    ("setMargin", "setMargin", "set margin"),
    ("setPadding", "setPadding", "set padding"),
    ("setColor", "setColor", "set #AARRGGBB color"),
    ("setTextColor", "setTextColor", "set #AARRGGBB text color"),
    ("setVisible", "setVisible", "show or hide control"),
    ("setEnabled", "setEnabled", "enable or disable control"),
    ("setTooltip", "setTooltip", "set tooltip"),
    ("focus", "focus", "focus control"),
    ("onClick", "onClick", "click event"),
    ("onChanged", "onChanged", "changed event"),
    ("onTextChanged", "onTextChanged", "text changed event"),
    ("onSubmit", "onSubmit", "submit event"),
];

fn symbol_matches_file(lang: LanguageId, symbol: &IndexedSymbol) -> bool {
    match lang {
        LanguageId::Rust => matches!(
            symbol.kind,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::Struct
                | SymbolKind::Enum
                | SymbolKind::Trait
                | SymbolKind::Module
                | SymbolKind::Macro
                | SymbolKind::TypeAlias
                | SymbolKind::Constant
        ),
        _ => true,
    }
}

fn member_access_at(text: &str, row: usize, col: usize) -> Option<(String, String)> {
    let line = nth_line(text, row);
    let bytes = line.as_bytes();
    let col = col.min(bytes.len());
    let mut dot = col;
    while dot > 0 && is_ident_byte(bytes[dot - 1]) {
        dot -= 1;
    }
    if dot == 0 || bytes[dot - 1] != b'.' {
        return None;
    }
    let member = core::str::from_utf8(&bytes[dot..col]).unwrap_or("");
    let mut receiver_end = dot - 1;
    while receiver_end > 0 && bytes[receiver_end - 1].is_ascii_whitespace() {
        receiver_end -= 1;
    }
    let mut receiver_start = receiver_end;
    while receiver_start > 0 && is_ident_byte(bytes[receiver_start - 1]) {
        receiver_start -= 1;
    }
    if receiver_start == receiver_end {
        return None;
    }
    Some((
        String::from(core::str::from_utf8(&bytes[receiver_start..receiver_end]).unwrap_or("")),
        String::from(member),
    ))
}

fn is_anyui_alias(text: &str, receiver: &str) -> bool {
    if receiver == "ui" || receiver == "anyui" {
        return true;
    }
    for line in text.split('\n') {
        if !line.contains("@anyos/anyui") || !line.contains(receiver) {
            continue;
        }
        let compact = without_spaces(line);
        if compact.contains(&format!("{}=require('@anyos/anyui')", receiver))
            || compact.contains(&format!("{}=require(\"@anyos/anyui\")", receiver))
            || compact.contains(&format!("*as{}from'@anyos/anyui'", receiver))
            || compact.contains(&format!("*as{}from\"@anyos/anyui\"", receiver))
        {
            return true;
        }
    }
    false
}

fn without_spaces(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if !ch.is_whitespace() {
            out.push(ch);
        }
    }
    out
}

fn prefix_at(text: &str, row: usize, col: usize) -> String {
    let line = nth_line(text, row);
    let bytes = line.as_bytes();
    let mut end = col.min(bytes.len());
    while end > 0 && !is_ident_byte(bytes[end - 1]) {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    String::from(core::str::from_utf8(&bytes[start..end]).unwrap_or(""))
}

fn word_at(text: &str, row: usize, col: usize) -> String {
    let line = nth_line(text, row);
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return String::new();
    }
    let mut pos = col.min(bytes.len().saturating_sub(1));
    if !is_ident_byte(bytes[pos]) && pos > 0 {
        pos -= 1;
    }
    if !is_ident_byte(bytes[pos]) {
        return String::new();
    }
    let mut start = pos;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = pos + 1;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    String::from(core::str::from_utf8(&bytes[start..end]).unwrap_or(""))
}

fn nth_line(text: &str, row: usize) -> &str {
    text.split('\n').nth(row).unwrap_or("")
}

fn previous_byte_at(text: &str, row: usize, col: usize) -> Option<u8> {
    let line = nth_line(text, row);
    let bytes = line.as_bytes();
    let col = col.min(bytes.len());
    if col == 0 {
        return None;
    }
    Some(bytes[col - 1])
}

fn is_ident_byte(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

fn starts_with_ascii_ci(value: &str, prefix: &str) -> bool {
    value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn strip_placeholders(template: &str) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next();
            while let Some(inner) = chars.next() {
                if inner == ':' {
                    break;
                }
                if inner == '}' {
                    break;
                }
            }
            while let Some(inner) = chars.next() {
                if inner == '}' {
                    break;
                }
                out.push(inner);
            }
        } else {
            out.push(ch);
        }
    }
    out
}
