use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::project::Project;

#[derive(Clone, Debug)]
pub struct ServiceOperation {
    pub name: String,
    pub method: String,
    pub path: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnectedServiceKind {
    OpenApiRest,
    WsdlSoap,
    Grpc,
    RestEndpoint,
}

impl ConnectedServiceKind {
    pub fn from_index(index: u32) -> Self {
        match index {
            1 => Self::WsdlSoap,
            2 => Self::Grpc,
            3 => Self::RestEndpoint,
            _ => Self::OpenApiRest,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenApiRest => "openapi-rest",
            Self::WsdlSoap => "wsdl-soap",
            Self::Grpc => "grpc",
            Self::RestEndpoint => "rest-endpoint",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::OpenApiRest => "OpenAPI / REST",
            Self::WsdlSoap => "WSDL / SOAP",
            Self::Grpc => "gRPC / Protobuf",
            Self::RestEndpoint => "REST Endpoint",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConnectedService {
    pub name: String,
    pub kind: ConnectedServiceKind,
    pub endpoint: String,
    pub module_name: String,
    pub output_dir: String,
}

pub fn services_for_project(project: &Project) -> Vec<ConnectedService> {
    let manifest = manifest_path(project);
    let Ok(text) = anyos_std::fs::read_to_string(&manifest) else {
        return Vec::new();
    };
    parse_manifest(&text, project)
}

pub fn add_service(
    project: &Project,
    name: &str,
    endpoint: &str,
    module_name: &str,
    kind: ConnectedServiceKind,
) -> Result<ConnectedService, &'static str> {
    let name = sanitize_display_name(name);
    if name.is_empty() {
        return Err("Enter a service name");
    }
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err("Enter a WSDL/OpenAPI/protobuf path or endpoint URL");
    }
    let module_name = sanitize_module_name(module_name);
    if module_name.is_empty() {
        return Err("Enter a valid Rust module name");
    }

    let output_dir = format!("{}/src/generated/services/{}", project.root, module_name);
    ensure_dir(&format!("{}/src/generated", project.root))?;
    ensure_dir(&format!("{}/src/generated/services", project.root))?;
    ensure_dir(&output_dir)?;

    let service = ConnectedService {
        name,
        kind,
        endpoint: String::from(endpoint),
        module_name,
        output_dir,
    };
    generate_service_files(project, &service)?;
    write_manifest(project, &service)?;
    refresh_generated_roots(project)?;
    Ok(service)
}

pub fn generated_mod_path(project: &Project) -> String {
    format!("{}/src/generated/mod.rs", project.root)
}

fn generate_service_files(
    project: &Project,
    service: &ConnectedService,
) -> Result<(), &'static str> {
    let operations = discover_operations(project, service);
    anyos_std::fs::write_bytes(
        &format!("{}/mod.rs", service.output_dir),
        generated_mod_rs(service).as_bytes(),
    )
    .map_err(|_| "Could not write connected service module")?;
    anyos_std::fs::write_bytes(
        &format!("{}/client.rs", service.output_dir),
        generated_client_rs(service, &operations).as_bytes(),
    )
    .map_err(|_| "Could not write connected service client")?;
    anyos_std::fs::write_bytes(
        &format!("{}/models.rs", service.output_dir),
        generated_models_rs(service, &operations).as_bytes(),
    )
    .map_err(|_| "Could not write connected service models")?;
    anyos_std::fs::write_bytes(
        &format!("{}/README.md", service.output_dir),
        generated_readme(service, &operations).as_bytes(),
    )
    .map_err(|_| "Could not write connected service readme")?;
    Ok(())
}

fn write_manifest(project: &Project, service: &ConnectedService) -> Result<(), &'static str> {
    let manifest = manifest_path(project);
    let mut services = services_for_project(project);
    services.retain(|existing| existing.module_name != service.module_name);
    services.push(service.clone());

    let mut out = String::from("anycode-connected-services-v1\n");
    for service in &services {
        out.push_str(&format!(
            "service name=\"{}\" kind=\"{}\" endpoint=\"{}\" module=\"{}\" output=\"{}\"\n",
            escape(&service.name),
            service.kind.as_str(),
            escape(&service.endpoint),
            escape(&service.module_name),
            escape(&service.output_dir)
        ));
    }
    anyos_std::fs::write_bytes(&manifest, out.as_bytes())
        .map_err(|_| "Could not write connected services manifest")
}

fn refresh_generated_roots(project: &Project) -> Result<(), &'static str> {
    let services = services_for_project(project);
    let services_dir = format!("{}/src/generated/services", project.root);
    ensure_dir(&format!("{}/src/generated", project.root))?;
    ensure_dir(&services_dir)?;

    let mut services_mod = String::new();
    for service in &services {
        services_mod.push_str(&format!("pub mod {};\n", service.module_name));
    }
    anyos_std::fs::write_bytes(&format!("{}/mod.rs", services_dir), services_mod.as_bytes())
        .map_err(|_| "Could not write services module")?;

    let root_mod = "pub mod services;\n";
    anyos_std::fs::write_bytes(&generated_mod_path(project), root_mod.as_bytes())
        .map_err(|_| "Could not write generated module")
}

fn generated_mod_rs(service: &ConnectedService) -> String {
    format!(
        "pub mod client;\npub mod models;\n\npub use client::{}Client;\npub use models::*;\n",
        rust_type_name(&service.name)
    )
}

fn generated_client_rs(service: &ConnectedService, operations: &[ServiceOperation]) -> String {
    let type_name = rust_type_name(&service.name);
    let method = match service.kind {
        ConnectedServiceKind::OpenApiRest | ConnectedServiceKind::RestEndpoint => "request",
        ConnectedServiceKind::WsdlSoap => "soap_action",
        ConnectedServiceKind::Grpc => "call",
    };
    let mut operation_methods = String::new();
    for operation in operations {
        operation_methods.push_str(&format!(
            "\n    pub fn {}(&self) -> Result<String, ServiceError> {{\n        self.{}(\"{}\")\n    }}\n",
            sanitize_method_name(&operation.name),
            method,
            escape_rust_string(&operation.name)
        ));
    }
    format!(
        "use alloc::string::String;\n\n#[derive(Clone, Debug)]\npub struct {type_name}Client {{\n    pub endpoint: String,\n}}\n\nimpl {type_name}Client {{\n    pub fn new(endpoint: &str) -> Self {{\n        Self {{ endpoint: String::from(endpoint) }}\n    }}\n\n    pub fn default() -> Self {{\n        Self::new(\"{endpoint}\")\n    }}\n\n    pub fn {method}(&self, operation: &str) -> Result<String, ServiceError> {{\n        let _ = operation;\n        Err(ServiceError::TransportNotConnected)\n    }}\n{operation_methods}}}\n\n#[derive(Clone, Debug)]\npub enum ServiceError {{\n    TransportNotConnected,\n    InvalidResponse,\n    ServiceFault(String),\n}}\n",
        type_name = type_name,
        endpoint = escape_rust_string(&service.endpoint),
        method = method,
        operation_methods = operation_methods
    )
}

fn generated_models_rs(service: &ConnectedService, operations: &[ServiceOperation]) -> String {
    let mut operation_rows = String::new();
    for operation in operations {
        operation_rows.push_str(&format!(
            "    ServiceOperation {{ name: \"{}\", method: \"{}\", path: \"{}\" }},\n",
            escape_rust_string(&operation.name),
            escape_rust_string(&operation.method),
            escape_rust_string(&operation.path)
        ));
    }
    format!(
        "#[derive(Clone, Debug)]\npub struct ServiceInfo {{\n    pub name: &'static str,\n    pub kind: &'static str,\n}}\n\n#[derive(Clone, Debug)]\npub struct ServiceOperation {{\n    pub name: &'static str,\n    pub method: &'static str,\n    pub path: &'static str,\n}}\n\npub const SERVICE_INFO: ServiceInfo = ServiceInfo {{\n    name: \"{}\",\n    kind: \"{}\",\n}};\n\npub const SERVICE_OPERATIONS: &[ServiceOperation] = &[\n{}];\n",
        escape_rust_string(&service.name),
        service.kind.as_str(),
        operation_rows
    )
}

fn generated_readme(service: &ConnectedService, operations: &[ServiceOperation]) -> String {
    let mut ops = String::new();
    if operations.is_empty() {
        ops.push_str("\nNo operations were discovered yet. The client still contains the generic call method.\n");
    } else {
        ops.push_str("\nDiscovered operations:\n");
        for operation in operations {
            ops.push_str(&format!(
                "- `{}` {} `{}`\n",
                operation.name, operation.method, operation.path
            ));
        }
    }
    format!(
        "# {}\n\nGenerated by anyCode Connected Services.\n\nKind: `{}`\nEndpoint/spec: `{}`\n{}\nThe generated client is intentionally transport-neutral for now. Wire it to the platform HTTP/SOAP/gRPC transport layer, then regenerate from the Connected Services dialog when the service contract changes.\n",
        service.name,
        service.kind.display_name(),
        service.endpoint,
        ops
    )
}

fn discover_operations(project: &Project, service: &ConnectedService) -> Vec<ServiceOperation> {
    match service.kind {
        ConnectedServiceKind::OpenApiRest => discover_openapi_operations(project, service),
        ConnectedServiceKind::WsdlSoap => discover_wsdl_operations(project, service),
        ConnectedServiceKind::Grpc => discover_grpc_operations(project, service),
        ConnectedServiceKind::RestEndpoint => vec![ServiceOperation {
            name: String::from("request"),
            method: String::from("GET"),
            path: service.endpoint.clone(),
        }],
    }
}

fn discover_openapi_operations(
    project: &Project,
    service: &ConnectedService,
) -> Vec<ServiceOperation> {
    let Some(text) = read_local_contract(project, &service.endpoint) else {
        return Vec::new();
    };
    let Ok(value) = anyos_std::json::Value::parse(&text) else {
        return Vec::new();
    };
    let Some(root) = value.as_object() else {
        return Vec::new();
    };
    let Some(paths) = root.get("paths").and_then(|value| value.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (path_name, path_value) in paths.iter() {
        let Some(path_obj) = path_value.as_object() else {
            continue;
        };
        for (method, operation_value) in path_obj.iter() {
            if !is_http_method(method) {
                continue;
            }
            let operation_name = operation_value
                .as_object()
                .and_then(|obj| obj.get("operationId"))
                .and_then(|value| value.as_str())
                .map(String::from)
                .unwrap_or_else(|| format!("{}_{}", method, path_to_operation_name(path_name)));
            out.push(ServiceOperation {
                name: operation_name,
                method: ascii_upper(method),
                path: String::from(path_name),
            });
        }
    }
    out
}

fn discover_wsdl_operations(
    project: &Project,
    service: &ConnectedService,
) -> Vec<ServiceOperation> {
    let Some(text) = read_local_contract(project, &service.endpoint) else {
        return Vec::new();
    };
    let Ok(doc) = anyos_std::xml::Document::parse(&text) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    collect_wsdl_operation_names(&doc.root, &mut names);
    let mut out = Vec::new();
    for name in names {
        if !out.iter().any(|op: &ServiceOperation| op.name == name) {
            out.push(ServiceOperation {
                name,
                method: String::from("SOAP"),
                path: service.endpoint.clone(),
            });
        }
    }
    out
}

fn discover_grpc_operations(
    project: &Project,
    service: &ConnectedService,
) -> Vec<ServiceOperation> {
    let Some(text) = read_local_contract(project, &service.endpoint) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.split('\n') {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("rpc ") else {
            continue;
        };
        let name = rest
            .split(|ch: char| ch == '(' || ch.is_whitespace())
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty() {
            out.push(ServiceOperation {
                name: String::from(name),
                method: String::from("RPC"),
                path: service.endpoint.clone(),
            });
        }
    }
    out
}

fn parse_manifest(text: &str, project: &Project) -> Vec<ConnectedService> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let trimmed = line.trim();
        if !trimmed.starts_with("service ") {
            continue;
        }
        let module_name = attr(trimmed, "module").unwrap_or_default();
        if module_name.is_empty() {
            continue;
        }
        let kind = match attr(trimmed, "kind").unwrap_or_default().as_str() {
            "wsdl-soap" => ConnectedServiceKind::WsdlSoap,
            "grpc" => ConnectedServiceKind::Grpc,
            "rest-endpoint" => ConnectedServiceKind::RestEndpoint,
            _ => ConnectedServiceKind::OpenApiRest,
        };
        out.push(ConnectedService {
            name: attr(trimmed, "name").unwrap_or_else(|| module_name.clone()),
            kind,
            endpoint: attr(trimmed, "endpoint").unwrap_or_default(),
            module_name: module_name.clone(),
            output_dir: attr(trimmed, "output").unwrap_or_else(|| {
                format!("{}/src/generated/services/{}", project.root, module_name)
            }),
        });
    }
    out
}

fn manifest_path(project: &Project) -> String {
    format!("{}/.anycode-connected-services", project.root)
}

fn ensure_dir(path: &str) -> Result<(), &'static str> {
    if crate::util::path::exists(path) {
        return Ok(());
    }
    if anyos_std::fs::mkdir(path) == 0 {
        Ok(())
    } else {
        Err("Could not create connected services directory")
    }
}

fn read_local_contract(project: &Project, endpoint: &str) -> Option<String> {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return None;
    }
    let path = if endpoint.starts_with('/') {
        String::from(endpoint)
    } else {
        format!("{}/{}", project.root, endpoint)
    };
    anyos_std::fs::read_to_string(&path).ok()
}

fn is_http_method(value: &str) -> bool {
    matches!(
        value,
        "get" | "put" | "post" | "delete" | "patch" | "options" | "head" | "trace"
    )
}

fn ascii_upper(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        out.push(ch.to_ascii_uppercase());
    }
    out
}

fn path_to_operation_name(path: &str) -> String {
    let mut out = String::new();
    for ch in path.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        String::from("root")
    } else {
        out
    }
}

fn collect_wsdl_operation_names(element: &anyos_std::xml::Element, out: &mut Vec<String>) {
    if local_xml_name(element.name()) == "operation" {
        if let Some(name) = element.attr("name") {
            if !name.is_empty() {
                out.push(String::from(name));
            }
        }
    }
    for child in element.child_elements() {
        collect_wsdl_operation_names(child, out);
    }
}

fn local_xml_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn sanitize_display_name(value: &str) -> String {
    value.trim().chars().filter(|ch| *ch != '"').collect()
}

fn sanitize_module_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.trim().chars() {
        let valid = ch.is_ascii_alphanumeric() || ch == '_';
        if valid {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == ' ' {
            out.push('_');
        }
    }
    if out
        .chars()
        .next()
        .map(|ch| ch.is_ascii_digit())
        .unwrap_or(false)
    {
        out.insert(0, '_');
    }
    out
}

fn rust_type_name(value: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper_next {
                out.push(ch.to_ascii_uppercase());
                upper_next = false;
            } else {
                out.push(ch);
            }
        } else {
            upper_next = true;
        }
    }
    if out.is_empty() {
        String::from("Service")
    } else {
        out
    }
}

fn sanitize_method_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("operation");
    }
    if out
        .chars()
        .next()
        .map(|ch| ch.is_ascii_digit())
        .unwrap_or(false)
    {
        out.insert(0, '_');
    }
    if is_rust_keyword(&out) {
        out.push('_');
    }
    out
}

fn is_rust_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
    )
}

fn attr(line: &str, name: &str) -> Option<String> {
    let needle = format!("{}=\"", name);
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(unescape(&rest[..end]))
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    out
}

fn escape_rust_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
