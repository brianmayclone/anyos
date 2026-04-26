use anyrc::linker::link::TargetAbi;
use anyrc::runtime::runtime_stubs;

#[test]
fn runtime_does_not_export_fake_stdlib_methods() {
    let stubs = runtime_stubs(TargetAbi::AnyOs);
    let exported: Vec<&str> = stubs.iter().map(|(name, _)| name.as_str()).collect();

    for forbidden in [
        "String::drain",
        "String::splitn",
        "String::trim_matches",
        "Vec::drain",
        "Vec::retain",
        "Option::map",
        "Result::map_err",
        "HashMap::insert",
    ] {
        assert!(
            !exported.contains(&forbidden),
            "{forbidden} must be compiled from Rust stdlib code or implemented as a real intrinsic, not linked as a placeholder runtime stub"
        );
    }
}

#[test]
fn runtime_keeps_only_abi_and_memory_helpers() {
    let stubs = runtime_stubs(TargetAbi::AnyOs);
    let exported: Vec<&str> = stubs.iter().map(|(name, _)| name.as_str()).collect();

    for required in [
        "__anyrc_alloc",
        "__anyrc_realloc",
        "__anyrc_vec_push",
        "__anyrc_vec_pop",
        "memcpy",
        "memmove",
        "memset",
        "memcmp",
    ] {
        assert!(
            exported.contains(&required),
            "{required} is a real low-level runtime helper and must remain available"
        );
    }
}
