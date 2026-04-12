fn register_layer_name(full_name: &str, layer_order: &mut Vec<String>) {
    if full_name.is_empty() {
        return;
    }
    if !layer_order.iter().any(|name| name == full_name) {
        layer_order.push(String::from(full_name));
    }
}

fn qualify_layer_name(name: &str, parent: Option<&str>) -> String {
    let name = name.trim();
    if name.is_empty() {
        return String::new();
    }
    if let Some(parent_name) = parent {
        let mut full = String::from(parent_name);
        full.push('.');
        full.push_str(name);
        full
    } else {
        String::from(name)
    }
}

fn register_layer_statement(name_text: &str, parent: Option<&str>, layer_order: &mut Vec<String>) {
    for raw_name in name_text.split(',') {
        let full_name = qualify_layer_name(raw_name.trim(), parent);
        register_layer_name(&full_name, layer_order);
    }
}

fn resolve_layer_block_name(
    name_text: &str,
    parent: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> String {
    let trimmed = name_text.trim();
    if trimmed.is_empty() {
        *anon_layer_counter += 1;
        let mut full = String::from("__anon_layer_");
        full.push_str(&anon_layer_counter.to_string());
        register_layer_name(&full, layer_order);
        return full;
    }

    let full_name = qualify_layer_name(trimmed, parent);
    register_layer_name(&full_name, layer_order);
    full_name
}
